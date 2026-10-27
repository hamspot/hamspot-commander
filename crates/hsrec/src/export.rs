//! Copy or move recordings to removable media. A move deletes the local
//! copy only after the copy on the target verifies by sha256. Moved rows
//! stay in the DB as `offline:<label>` and play again when the drive is back.
use crate::archive::content_sha;
use crate::db::{Db, Recording};
use crate::meta::Meta;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const EXPORT_DIR: &str = "HamSpot-REC";
pub const INDEX: &str = "index.json";

#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    pub mount: PathBuf,
    pub label: String,
    pub free: u64,
}

pub fn free_bytes(path: &Path) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return 0 };
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return 0;
    }
    st.f_bavail as u64 * st.f_frsize as u64
}

/// Mounted removable drives (under /media or /run/media), from /proc/mounts.
pub fn removable_drives() -> Vec<Drive> {
    let text = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    parse_mounts(&text).into_iter().map(|(mount, label)| {
        let free = free_bytes(&mount);
        Drive { mount, label, free }
    }).collect()
}

pub fn parse_mounts(text: &str) -> Vec<(PathBuf, String)> {
    text.lines()
        .filter_map(|l| {
            let mount = l.split_whitespace().nth(1)?.replace("\\040", " ");
            (mount.starts_with("/media/") || mount.starts_with("/run/media/")).then(|| {
                let p = PathBuf::from(&mount);
                let label = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                (p, label)
            })
        })
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IndexEntry {
    pub raw_path: String,
    pub sha256: String,
    pub archived: bool,
    pub meta: Meta,
}

#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub copied: usize,
    pub moved: usize,
    pub failed: Vec<String>,
    pub cancelled: bool,
}

fn copy_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = PathBuf::from(format!("{}.part", dst.display()));
    std::fs::copy(src, &tmp)?;
    std::fs::File::open(&tmp)?.sync_all()?;
    std::fs::rename(&tmp, dst)
}

/// Export `rows` to `<target>/HamSpot-REC/`. `label` names the drive.
pub fn export(
    db: &Db,
    dir: &Path,
    rows: &[Recording],
    target: &Path,
    label: &str,
    move_files: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> Report {
    let mut report = Report::default();
    let root = target.join(EXPORT_DIR);
    let rows: Vec<&Recording> = rows.iter().filter(|r| !r.offline() && !r.missing && !r.meta.end_utc.is_empty()).collect();
    let need: u64 = rows.iter().map(|r| r.disk_bytes().max(0) as u64 + 4096).sum();
    if free_bytes(target) < need {
        report.failed.push(format!("not enough space on {label}: need {:.2} GB", need as f64 / 1e9));
        return report;
    }
    let index_path = root.join(INDEX);
    let mut index: Vec<IndexEntry> = std::fs::read_to_string(&index_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    for (n, rec) in rows.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        progress(n, rows.len());
        let (Some(src), Some(dst)) = (crate::safepath::join_inside(dir, &rec.raw_path), crate::safepath::join_inside(&root, &rec.raw_path)) else {
            report.failed.push(format!("{}: unsafe path, skipped", rec.raw_path));
            continue;
        };
        let side = crate::storage::sidecar_for(&src);
        let run = || -> std::io::Result<String> {
            copy_file(&src, &dst)?;
            if side.exists() {
                copy_file(&side, &crate::storage::sidecar_for(&dst))?;
            }
            let a = content_sha(&src)?;
            let b = content_sha(&dst)?;
            if a != b {
                return Err(std::io::Error::other("checksum mismatch on target"));
            }
            Ok(a)
        };
        match run() {
            Ok(sha) => {
                if rec.sha256.is_empty() {
                    let _ = db.set_sha(rec.id, &sha);
                }
                index.retain(|e| e.raw_path != rec.raw_path);
                let mut meta = rec.meta.clone();
                crate::safepath::portable_meta(&mut meta);
                index.push(IndexEntry { raw_path: rec.raw_path.clone(), sha256: sha, archived: rec.archived, meta });
                if move_files {
                    if db.set_location(rec.id, &format!("offline:{label}")).is_ok() {
                        crate::storage::remove_files(dir, &rec.raw_path);
                        report.moved += 1;
                    }
                } else {
                    report.copied += 1;
                }
            }
            Err(e) => report.failed.push(format!("{}: {e} (local copy kept)", rec.raw_path)),
        }
    }
    if let Some(p) = index_path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let _ = std::fs::write(&index_path, serde_json::to_vec_pretty(&index).unwrap_or_default());
    unsafe { libc::sync() };
    progress(rows.len(), rows.len());
    report
}

/// Where an offline row's file is right now, if its drive is mounted and
/// the file there still matches.
pub fn locate(rec: &Recording, drives: &[Drive]) -> Option<PathBuf> {
    let label = rec.location.strip_prefix("offline:")?;
    let drive = drives.iter().find(|d| d.label == label)?;
    let p = crate::safepath::join_inside(&drive.mount.join(EXPORT_DIR), &rec.raw_path)?;
    p.exists().then_some(p)
}

/// Copy offline rows back from a re-attached drive (verified by sha256).
pub fn reattach(db: &Db, dir: &Path, drive: &Drive) -> Report {
    let mut report = Report::default();
    let label = format!("offline:{}", drive.label);
    for rec in db.all().unwrap_or_default().into_iter().filter(|r| r.location == label) {
        let (Some(src), Some(dst)) = (crate::safepath::join_inside(&drive.mount.join(EXPORT_DIR), &rec.raw_path), crate::safepath::join_inside(dir, &rec.raw_path)) else {
            report.failed.push(format!("{}: unsafe path, skipped", rec.raw_path));
            continue;
        };
        let run = || -> std::io::Result<()> {
            if !rec.sha256.is_empty() && content_sha(&src)? != rec.sha256 {
                return Err(std::io::Error::other("checksum mismatch on drive"));
            }
            copy_file(&src, &dst)?;
            let side = crate::storage::sidecar_for(&src);
            if side.exists() {
                copy_file(&side, &crate::storage::sidecar_for(&dst))?;
            }
            Ok(())
        };
        match run() {
            Ok(()) => {
                let _ = db.set_location(rec.id, "local");
                report.copied += 1;
            }
            Err(e) => report.failed.push(format!("{}: {e}", rec.raw_path)),
        }
    }
    report
}

/// Import a foreign export (another Commander's `index.json`), copying files in.
pub fn import(db: &Db, dir: &Path, export_root: &Path) -> Report {
    let mut report = Report::default();
    let Ok(text) = std::fs::read_to_string(export_root.join(INDEX)) else {
        report.failed.push("no index.json".into());
        return report;
    };
    let entries: Vec<IndexEntry> = serde_json::from_str(&text).unwrap_or_default();
    for mut e in entries {
        // A foreign index is untrusted: its paths must stay inside the drive
        // folder and our recordings folder, and its decoder must be portable.
        let (Some(src), Some(dst)) = (crate::safepath::join_inside(export_root, &e.raw_path), crate::safepath::join_inside(dir, &e.raw_path)) else {
            report.failed.push(format!("{}: unsafe path in index.json, skipped", e.raw_path));
            continue;
        };
        if std::fs::symlink_metadata(&src).map(|m| !m.file_type().is_file()).unwrap_or(true) {
            report.failed.push(format!("{}: not a regular file, skipped", e.raw_path));
            continue;
        }
        crate::safepath::portable_meta(&mut e.meta);
        if db.by_path(&e.raw_path).ok().flatten().is_some() {
            continue;
        }
        match copy_file(&src, &dst).and_then(|_| content_sha(&dst)) {
            Ok(sha) if e.sha256.is_empty() || sha == e.sha256 => {
                let _ = std::fs::write(crate::storage::sidecar_for(&dst), serde_json::to_vec_pretty(&e.meta).unwrap_or_default());
                if let Ok(id) = db.insert(&e.raw_path, &e.meta) {
                    let size = std::fs::metadata(&dst).map(|m| m.len() as i64).unwrap_or(0);
                    let _ = db.set_archived(id, &e.raw_path, e.archived, if e.archived { "zstd" } else { "" }, 0, if e.archived { size } else { 0 }, &sha);
                }
                report.copied += 1;
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&dst);
                report.failed.push(format!("{}: checksum mismatch", e.raw_path));
            }
            Err(err) => report.failed.push(format!("{}: {err}", e.raw_path)),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::tempdir;

    fn setup(tag: &str) -> (PathBuf, PathBuf, Db, i64) {
        let base = tempdir(tag);
        let dir = base.join("S.recordings");
        std::fs::create_dir_all(&dir).unwrap();
        let (path, mut m) = crate::replay::tests_support::fm_file(&dir, 500.0);
        m.start_utc = "2026-01-01T00:00:00Z".into();
        m.end_utc = "2026-01-01T00:00:01Z".into();
        std::fs::write(crate::storage::sidecar_for(&path), serde_json::to_vec(&m).unwrap()).unwrap();
        let db = Db::open_in_memory().unwrap();
        let id = db.insert("t.cs16", &m).unwrap();
        let usb = base.join("media/user/USB");
        std::fs::create_dir_all(&usb).unwrap();
        (dir, usb, db, id)
    }

    #[test]
    fn mounts() {
        let m = parse_mounts("/dev/sda1 / ext4 rw 0 0\n/dev/sdb1 /media/user/MY\\040USB vfat rw 0 0\n/dev/sdc1 /run/media/d/K vfat rw 0 0\n");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].1, "MY USB");
    }

    #[test]
    fn copy_keeps_local_move_goes_offline_and_reattach_restores() {
        let (dir, usb, db, id) = setup("exp1");
        let rows = db.all().unwrap();
        let r = export(&db, &dir, &rows, &usb, "USB", false, &AtomicBool::new(false), |_, _| {});
        assert_eq!(r.copied, 1, "{r:?}");
        assert!(dir.join("t.cs16").exists());
        assert!(usb.join("HamSpot-REC/t.cs16").exists());
        assert!(usb.join("HamSpot-REC/index.json").exists());
        let r = export(&db, &dir, &db.all().unwrap(), &usb, "USB", true, &AtomicBool::new(false), |_, _| {});
        assert_eq!(r.moved, 1);
        assert!(!dir.join("t.cs16").exists());
        let rec = db.get(id).unwrap().unwrap();
        assert_eq!(rec.location, "offline:USB");
        let drive = Drive { mount: usb.clone(), label: "USB".into(), free: 1 };
        assert!(locate(&rec, std::slice::from_ref(&drive)).is_some());
        let r = reattach(&db, &dir, &drive);
        assert_eq!(r.copied, 1);
        let rec = db.get(id).unwrap().unwrap();
        assert_eq!(rec.location, "local");
        assert!(crate::replay::decode_all(&dir.join(&rec.raw_path), &rec.meta).unwrap().len() > 1000);
    }

    #[test]
    fn failed_move_keeps_source() {
        let (dir, usb, db, id) = setup("exp2");
        let mut rows = db.all().unwrap();
        rows[0].raw_path = "missing.cs16".into();
        let r = export(&db, &dir, &rows, &usb, "USB", true, &AtomicBool::new(false), |_, _| {});
        assert_eq!(r.moved, 0);
        assert_eq!(r.failed.len(), 1);
        assert!(dir.join("t.cs16").exists());
        assert_eq!(db.get(id).unwrap().unwrap().location, "local");
        // Cancelled before start: nothing moves.
        let r = export(&db, &dir, &db.all().unwrap(), &usb, "USB", true, &AtomicBool::new(true), |_, _| {});
        assert!(r.cancelled);
        assert!(dir.join("t.cs16").exists());
    }

    #[test]
    fn import_rejects_traversal_and_absolute_and_migrates_decoder() {
        let (_dir, usb, _db, _id) = setup("exp-evil");
        let root = usb.join(EXPORT_DIR);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("ok.cs16"), [0u8; 64]).unwrap();
        let mut m = crate::db::tests::meta("hackrf_tg", Some(1), "2026-10-27T00:00:00Z", "2026-10-27T00:00:01Z");
        m.decoder = serde_json::json!({"script": "/home/user/.local/share/hamspot-commander/modems/commander-demod-p25.rxpipe"});
        let e = |p: &str| IndexEntry { raw_path: p.into(), sha256: String::new(), archived: false, meta: m.clone() };
        std::fs::write(root.join(INDEX), serde_json::to_vec(&vec![e("../../escape.cs16"), e("/tmp/abs.cs16"), e("ok.cs16")]).unwrap()).unwrap();
        let other = usb.parent().unwrap().join("Other.recordings");
        std::fs::create_dir_all(&other).unwrap();
        let db2 = Db::open_in_memory().unwrap();
        let r = import(&db2, &other, &root);
        assert_eq!(r.copied, 1);
        assert_eq!(r.failed.len(), 2);
        let rows = db2.all().unwrap();
        assert_eq!(rows[0].meta.decoder["script"], "modems/commander-demod-p25.rxpipe");
        let side = std::fs::read_to_string(other.join("ok.json")).unwrap();
        assert!(!side.contains("/home/"));
    }

    #[test]
    fn import_foreign_index() {
        let (dir, usb, db, _) = setup("exp3");
        export(&db, &dir, &db.all().unwrap(), &usb, "USB", false, &AtomicBool::new(false), |_, _| {});
        let other = tempdir("exp3b");
        let db2 = Db::open_in_memory().unwrap();
        let r = import(&db2, &other, &usb.join(EXPORT_DIR));
        assert_eq!(r.copied, 1, "{r:?}");
        assert_eq!(db2.all().unwrap().len(), 1);
        assert!(other.join("t.cs16").exists());
    }
}
