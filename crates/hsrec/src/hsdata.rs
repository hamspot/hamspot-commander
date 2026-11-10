//! `Foo.HSCommander` beside `Foo.HSData/`.
//!
//! Layout 0 is the flat names (`Foo.HSKeys`, `Foo.recordings`, …). They keep
//! working until the operator chooses Move. Layout 1 is `HSData.json` with
//! `"layout": 1`. VNA scans already live in `Foo.HSData/VNA/` and are not
//! part of this move.
//!
//! `resolve` is the only join for a path that came from stored data.

use crate::archive::sha256_file;
use crate::db::db_path_beside;
use crate::safepath::safe_rel;
use crate::storage::recordings_dir_beside;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const LAYOUT_V1: u32 = 1;
pub const MANIFEST_NAME: &str = "0001-flat-to-v1.json";

/// The dialog offers Later first. Tests assert this and never call Move on a live station.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Move,
    Later,
}

pub const DEFAULT_CHOICE: Choice = Choice::Later;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places {
    pub root: PathBuf,
    pub v1: bool,
    pub recordings: PathBuf,
    pub db: PathBuf,
    pub keys: PathBuf,
    pub log: PathBuf,
    pub vna_cal: PathBuf,
    pub files_link: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Index {
    pub layout: u32,
    #[serde(default)]
    pub station: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub migrated_from: String,
    /// `rec/files` is a symlink the operator created. No other link is followed.
    #[serde(default)]
    pub files_link: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Manifest {
    layout_from: u32,
    layout_to: u32,
    /// True when a copy was required. Those originals are never deleted.
    copied: bool,
    items: Vec<Item>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Item {
    from: String,
    to: String,
    bytes: u64,
    sha256: String,
}

fn stem(station: &Path) -> String {
    station
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "station".into())
}

/// `Foo.HSCommander` → `Foo.HSData`. Derived, never stored in the station file.
pub fn root(station: &Path) -> PathBuf {
    station.with_file_name(format!("{}.HSData", stem(station)))
}

fn index_path(station: &Path) -> PathBuf {
    root(station).join("HSData.json")
}

fn manifest_path(station: &Path) -> PathBuf {
    root(station).join("migration").join(MANIFEST_NAME)
}

pub fn read_index(station: &Path) -> Option<Index> {
    let text = fs::read_to_string(index_path(station)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn is_v1(station: &Path) -> bool {
    read_index(station).is_some_and(|i| i.layout == LAYOUT_V1)
}

/// Flat files are still beside the station and layout 1 has not been written.
pub fn needs_prompt(station: &Path) -> bool {
    if is_v1(station) {
        return false;
    }
    flat_items(station).into_iter().any(|(_, src)| src.exists())
}

fn flat_items(station: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        ("keys", station.with_extension("HSKeys")),
        ("vna", station.with_extension("HSVNA")),
        ("log", station.with_extension("HSLog")),
        ("db", db_path_beside(station)),
        ("db-wal", PathBuf::from(format!("{}-wal", db_path_beside(station).display()))),
        ("db-shm", PathBuf::from(format!("{}-shm", db_path_beside(station).display()))),
        ("db-lock", PathBuf::from(format!("{}.lock", db_path_beside(station).display()))),
        ("recordings", recordings_dir_beside(station)),
    ]
}

fn dest_for(station: &Path, kind: &str) -> Option<PathBuf> {
    let base = root(station);
    let name = stem(station);
    Some(match kind {
        "keys" => base.join("keys").join(format!("{name}.HSKeys")),
        "vna" => base.join("vna").join("calibration.HSVNA"),
        "log" => base.join("log").join("logbook.HSLog"),
        "db" => base.join("rec").join("rec.sqlite"),
        "db-wal" => base.join("rec").join("rec.sqlite-wal"),
        "db-shm" => base.join("rec").join("rec.sqlite-shm"),
        "db-lock" => base.join("rec").join("rec.sqlite.lock"),
        "recordings" => base.join("rec").join("files"),
        _ => return None,
    })
}

pub fn places(station: &Path) -> Places {
    let v1 = is_v1(station);
    let files_link = read_index(station).is_some_and(|i| i.files_link);
    let root = root(station);
    if v1 {
        Places {
            recordings: root.join("rec").join("files"),
            db: root.join("rec").join("rec.sqlite"),
            keys: root.join("keys").join(format!("{}.HSKeys", stem(station))),
            log: root.join("log").join("logbook.HSLog"),
            vna_cal: root.join("vna").join("calibration.HSVNA"),
            files_link,
            root,
            v1,
        }
    } else {
        Places {
            recordings: recordings_dir_beside(station),
            db: db_path_beside(station),
            keys: station.with_extension("HSKeys"),
            log: station.with_extension("HSLog"),
            vna_cal: station.with_extension("HSVNA"),
            files_link: false,
            root,
            v1,
        }
    }
}

pub fn recordings_dir(station: &Path) -> PathBuf {
    places(station).recordings
}

pub fn db_path(station: &Path) -> PathBuf {
    places(station).db
}

pub fn keys_path(station: &Path) -> PathBuf {
    places(station).keys
}

pub fn log_path(station: &Path) -> PathBuf {
    places(station).log
}

pub fn vna_cal_path(station: &Path) -> PathBuf {
    places(station).vna_cal
}

/// Join `rel` under `root`. Rejects absolute paths, `..`, and a symlink that
/// is not the operator's `rec/files` link (`files_link`).
pub fn resolve(root: &Path, rel: &str, files_link: bool) -> Result<PathBuf, String> {
    if !safe_rel(rel) {
        return Err(format!("path is not inside station data: {rel}"));
    }
    let files = root.join("rec").join("files");
    let mut cur = root.to_path_buf();
    let mut under_files = false;
    for c in Path::new(rel).components() {
        cur.push(c);
        under_files = cur == files || cur.starts_with(&files);
        if let Ok(md) = fs::symlink_metadata(&cur) {
            if md.file_type().is_symlink() {
                let allowed = files_link && cur == files;
                if !allowed {
                    return Err(format!("symlink is not allowed: {}", cur.display()));
                }
            }
        }
    }
    if under_files && files_link {
        let base = fs::canonicalize(&files).map_err(|e| format!("{}: {e}", files.display()))?;
        if cur.exists() {
            let got = fs::canonicalize(&cur).map_err(|e| e.to_string())?;
            if !got.starts_with(&base) {
                return Err(format!("path leaves the recordings folder: {}", got.display()));
            }
            return Ok(got);
        }
        return Ok(cur);
    }
    if cur.exists() {
        let base = fs::canonicalize(root).map_err(|e| e.to_string())?;
        let got = fs::canonicalize(&cur).map_err(|e| e.to_string())?;
        if !got.starts_with(&base) {
            return Err(format!("path leaves station data: {}", got.display()));
        }
        return Ok(got);
    }
    Ok(cur)
}

pub fn apply(station: &Path, choice: Choice) -> Result<String, String> {
    match choice {
        Choice::Later => Ok("left the old files where they are".into()),
        Choice::Move => migrate(station),
    }
}

pub fn migrate(station: &Path) -> Result<String, String> {
    if is_v1(station) {
        return Ok("already in HSData".into());
    }
    let db = db_path_beside(station);
    if db.is_file() {
        if let Ok(conn) = rusqlite::Connection::open(&db) {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
    }
    let data = root(station);
    fs::create_dir_all(data.join("migration")).map_err(|e| e.to_string())?;
    let mut copied = false;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut items = Vec::new();
    for (kind, src) in flat_items(station) {
        if !src.exists() {
            continue;
        }
        let Some(dst) = dest_for(station, kind) else { continue };
        if dst.exists() {
            undo(&moved);
            return Err(format!("{} already exists; nothing else was moved", dst.display()));
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        match transfer(&src, &dst, &mut copied) {
            Ok(()) => {
                record_tree(&src, &dst, &data, station.parent().unwrap_or(Path::new(".")), &mut items)?;
                moved.push((src, dst));
            }
            Err(e) => {
                undo(&moved);
                return Err(e.to_string());
            }
        }
    }
    if items.is_empty() {
        return Err("nothing to move".into());
    }
    let manifest = Manifest { layout_from: 0, layout_to: LAYOUT_V1, copied, items };
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    let man = manifest_path(station);
    write_file(&man, &text)?;
    write_file(&data.join("migration").join("rollback.sh"), &rollback_script(&manifest))?;
    let index = Index {
        layout: LAYOUT_V1,
        station: stem(station),
        created: stamp(),
        migrated_from: "flat".into(),
        files_link: data.join("rec").join("files").is_symlink(),
    };
    write_file(&index_path(station), &serde_json::to_string_pretty(&index).map_err(|e| e.to_string())?)?;
    let how = if copied { "copied (old files kept)" } else { "renamed" };
    Ok(format!("station data {how} into {}", data.display()))
}

/// Put a renamed layout back. A copy is left on disk; this only stops using it.
pub fn restore(station: &Path) -> Result<String, String> {
    let man = manifest_path(station);
    let text = fs::read_to_string(&man).map_err(|e| format!("{}: {e}", man.display()))?;
    let manifest: Manifest = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if manifest.copied {
        let _ = fs::remove_file(index_path(station));
        return Ok("old files were kept. HSData is not in use. The copies were not deleted.".into());
    }
    let data = root(station);
    for item in manifest.items.iter().rev() {
        let from = station.parent().unwrap_or(Path::new(".")).join(&item.from);
        let to = data.join(&item.to);
        if to.exists() && !from.exists() {
            if let Some(parent) = from.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::rename(&to, &from).map_err(|e| format!("{}: {e}", to.display()))?;
        }
    }
    let _ = fs::remove_file(index_path(station));
    Ok("restored the old file names".into())
}

fn rollback_script(manifest: &Manifest) -> String {
    let mut out = String::from("#!/bin/sh\n# Restore the flat names. Copies are not deleted.\nset -eu\nDATA=\"$(cd \"$(dirname \"$0\")/..\" && pwd)\"\nSTATION=\"$(cd \"$(dirname \"$0\")/../..\" && pwd)\"\n");
    if manifest.copied {
        out.push_str("echo \"originals were kept; remove HSData.json to stop using the copies\"\n");
        return out;
    }
    for item in manifest.items.iter().rev() {
        out.push_str(&format!("mkdir -p \"$(dirname \"$STATION/{}\")\"\n", item.from));
        out.push_str(&format!("mv -n \"$DATA/{}\" \"$STATION/{}\"\n", item.to, item.from));
    }
    out
}

fn undo(moved: &[(PathBuf, PathBuf)]) {
    for (src, dst) in moved.iter().rev() {
        if dst.exists() && !src.exists() {
            let _ = fs::rename(dst, src);
        }
    }
}

fn transfer(src: &Path, dst: &Path, copied: &mut bool) -> io::Result<()> {
    if same_device(src, dst) {
        fs::rename(src, dst)
    } else {
        *copied = true;
        copy_path(src, dst)
    }
}

fn same_device(src: &Path, dst: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let src_dev = fs::metadata(src).map(|m| m.dev()).ok();
    let parent = dst.parent().unwrap_or(dst);
    let dst_dev = fs::metadata(parent).map(|m| m.dev()).ok();
    match (src_dev, dst_dev) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

fn copy_path(src: &Path, dst: &Path) -> io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    if md.file_type().is_symlink() {
        return Err(io::Error::new(io::ErrorKind::Other, "refusing to copy a symlink"));
    }
    if md.is_dir() {
        fs::create_dir_all(dst)?;
        for ent in fs::read_dir(src)? {
            let ent = ent?;
            copy_path(&ent.path(), &dst.join(ent.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut input = File::open(src)?;
        let mut output = File::create(dst)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        Ok(())
    }
}

fn record_tree(src: &Path, dst: &Path, data: &Path, station_dir: &Path, items: &mut Vec<Item>) -> Result<(), String> {
    let before = items.len();
    if dst.is_file() {
        items.push(one_item(station_dir, src, data, dst)?);
    } else {
        let mut stack = vec![dst.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let rd = fs::read_dir(&dir).map_err(|e| e.to_string())?;
            for ent in rd {
                let ent = ent.map_err(|e| e.to_string())?;
                let path = ent.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    let rel = path.strip_prefix(dst).unwrap_or(&path);
                    items.push(one_item(station_dir, &src.join(rel), data, &path)?);
                }
            }
        }
    }
    if items.len() == before {
        items.push(Item { from: rel_of(station_dir, src), to: rel_of(data, dst), bytes: 0, sha256: String::new() });
    }
    Ok(())
}

fn one_item(station_dir: &Path, src: &Path, data: &Path, dst: &Path) -> Result<Item, String> {
    let bytes = fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
    let sha256 = sha256_file(dst).unwrap_or_default();
    Ok(Item { from: rel_of(station_dir, src), to: rel_of(data, dst), bytes, sha256 })
}

fn rel_of(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut f = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

fn stamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    secs.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station_in(dir: &Path) -> PathBuf {
        let p = dir.join("Demo.HSCommander");
        fs::write(&p, "{}\n").unwrap();
        p
    }

    #[test]
    fn later_is_the_default_and_moves_nothing() {
        assert_eq!(DEFAULT_CHOICE, Choice::Later);
        let dir = std::env::temp_dir().join(format!("hsdata-later-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let station = station_in(&dir);
        fs::write(station.with_extension("HSKeys"), "k").unwrap();
        assert!(needs_prompt(&station));
        apply(&station, DEFAULT_CHOICE).unwrap();
        assert!(station.with_extension("HSKeys").is_file());
        assert!(!index_path(&station).exists());
        assert_eq!(keys_path(&station), station.with_extension("HSKeys"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_renames_on_the_same_disk_and_restore_puts_them_back() {
        let dir = std::env::temp_dir().join(format!("hsdata-move-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let station = station_in(&dir);
        fs::write(station.with_extension("HSKeys"), "secret").unwrap();
        fs::create_dir_all(root(&station).join("VNA").join("dev")).unwrap();
        fs::write(root(&station).join("VNA").join("dev").join("a.HSVNAScan"), "scan").unwrap();
        let rec = recordings_dir_beside(&station);
        fs::create_dir_all(rec.join("hackrf_tg")).unwrap();
        fs::write(rec.join("hackrf_tg").join("x.cs16"), b"iq").unwrap();
        let note = apply(&station, Choice::Move).unwrap();
        assert!(note.contains("renamed"), "{note}");
        assert!(!station.with_extension("HSKeys").exists());
        assert_eq!(fs::read(keys_path(&station)).unwrap(), b"secret");
        assert!(root(&station).join("VNA").join("dev").join("a.HSVNAScan").is_file());
        assert_eq!(fs::read(recordings_dir(&station).join("hackrf_tg").join("x.cs16")).unwrap(), b"iq");
        assert!(manifest_path(&station).is_file());
        restore(&station).unwrap();
        assert!(station.with_extension("HSKeys").is_file());
        assert!(!is_v1(&station));
        assert!(root(&station).join("VNA").join("dev").join("a.HSVNAScan").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_rejects_escape_and_allows_the_files_link() {
        assert!(resolve(Path::new("/data"), "/etc/passwd", false).is_err());
        assert!(resolve(Path::new("/data"), "../x", false).is_err());
        assert!(resolve(Path::new("/data"), "rec/files/a.cs16", false).is_ok());
        let dir = std::env::temp_dir().join(format!("hsdata-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let root = dir.join("S.HSData");
        let outside = dir.join("other");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(root.join("rec")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("rec").join("files")).unwrap();
        fs::write(outside.join("a.cs16"), b"z").unwrap();
        assert!(resolve(&root, "rec/files/a.cs16", false).is_err());
        let got = resolve(&root, "rec/files/a.cs16", true).unwrap();
        assert_eq!(fs::read(&got).unwrap(), b"z");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scans_use_the_same_root() {
        let station = Path::new("/cfg/SAFE-T.HSCommander");
        assert_eq!(root(station), PathBuf::from("/cfg/SAFE-T.HSData"));
        assert_eq!(root(station).join("VNA"), PathBuf::from("/cfg/SAFE-T.HSData/VNA"));
    }
}
