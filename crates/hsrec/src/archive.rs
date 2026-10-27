//! On-demand zstd archive. Per file: compress to a temp file, verify by
//! decompressing and matching sha256, atomic rename, then remove the
//! original. A failure at any step keeps the original untouched.
use crate::cache::hex;
use crate::db::{Db, Recording};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const DEFAULT_LEVEL: i32 = 9;

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Archived { saved: i64 },
    Skipped(&'static str),
    Failed(String),
    Cancelled,
}

pub fn sha256_reader(mut r: impl Read, cancel: &AtomicBool) -> std::io::Result<Option<String>> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let n = r.read(&mut buf)?;
        if n == 0 {
            return Ok(Some(hex(&h.finalize())));
        }
        h.update(&buf[..n]);
    }
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let f = std::fs::File::open(path)?;
    Ok(sha256_reader(f, &AtomicBool::new(false))?.unwrap_or_default())
}

/// Hash of the *uncompressed* content (what replay sees).
pub fn content_sha(path: &Path) -> std::io::Result<String> {
    Ok(sha256_reader(crate::replay::open_raw(path)?, &AtomicBool::new(false))?.unwrap_or_default())
}

pub fn archive_one(db: &Db, dir: &Path, rec: &Recording, level: i32, cancel: &AtomicBool) -> Outcome {
    if rec.archived {
        return Outcome::Skipped("already archived");
    }
    if rec.offline() || rec.missing {
        return Outcome::Skipped("not on this disk");
    }
    if rec.meta.end_utc.is_empty() {
        return Outcome::Skipped("still recording");
    }
    let Some(src) = crate::safepath::join_inside(dir, &rec.raw_path) else {
        return Outcome::Skipped("unsafe path");
    };
    let dst = PathBuf::from(format!("{}.zst", src.display()));
    let tmp = PathBuf::from(format!("{}.zst.tmp", src.display()));
    let run = || -> std::io::Result<Option<(String, i64)>> {
        // Compress while hashing the original.
        let mut input = std::io::BufReader::with_capacity(256 * 1024, std::fs::File::open(&src)?);
        let out = std::io::BufWriter::with_capacity(256 * 1024, std::fs::File::create(&tmp)?);
        let mut enc = zstd::stream::write::Encoder::new(out, level)?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            enc.write_all(&buf[..n])?;
        }
        let mut out = enc.finish()?;
        out.flush()?;
        out.get_ref().sync_all()?;
        let want = hex(&h.finalize());
        let got = sha256_reader(zstd::stream::read::Decoder::new(std::fs::File::open(&tmp)?)?, cancel)?;
        let Some(got) = got else { return Ok(None) };
        if got != want {
            return Err(std::io::Error::other("checksum mismatch after compression"));
        }
        let size = std::fs::metadata(&tmp)?.len() as i64;
        Ok(Some((want, size)))
    };
    match run() {
        Ok(None) => {
            let _ = std::fs::remove_file(&tmp);
            Outcome::Cancelled
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Outcome::Failed(format!("{}: {e} (original kept)", rec.raw_path))
        }
        Ok(Some((sha, size))) => {
            if let Err(e) = std::fs::rename(&tmp, &dst) {
                let _ = std::fs::remove_file(&tmp);
                return Outcome::Failed(format!("{}: rename: {e} (original kept)", rec.raw_path));
            }
            let rel = crate::storage::relative(dir, &dst);
            if let Err(e) = db.set_archived(rec.id, &rel, true, "zstd", level, size, &sha) {
                let _ = std::fs::remove_file(&dst);
                return Outcome::Failed(format!("{}: database: {e} (original kept)", rec.raw_path));
            }
            let _ = std::fs::remove_file(&src);
            Outcome::Archived { saved: rec.meta.bytes - size }
        }
    }
}

pub fn unarchive_one(db: &Db, dir: &Path, rec: &Recording) -> Result<(), String> {
    if !rec.archived {
        return Ok(());
    }
    let rel = rec.raw_path.trim_end_matches(".zst").to_string();
    let (Some(src), Some(dst)) = (crate::safepath::join_inside(dir, &rec.raw_path), crate::safepath::join_inside(dir, &rel)) else {
        return Err(format!("{}: unsafe path", rec.raw_path));
    };
    let tmp = PathBuf::from(format!("{}.tmp", dst.display()));
    let run = || -> std::io::Result<String> {
        let mut r = crate::replay::open_raw(&src)?;
        let mut w = std::io::BufWriter::with_capacity(256 * 1024, std::fs::File::create(&tmp)?);
        std::io::copy(&mut r, &mut w)?;
        w.flush()?;
        drop(w);
        sha256_file(&tmp)
    };
    match run() {
        Ok(sha) if rec.sha256.is_empty() || sha == rec.sha256 => {
            std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
            db.set_archived(rec.id, &rel, false, "", 0, 0, &sha).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&src);
            Ok(())
        }
        Ok(_) => {
            let _ = std::fs::remove_file(&tmp);
            Err(format!("{}: checksum mismatch, archive kept", rec.raw_path))
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(format!("{}: {e}", rec.raw_path))
        }
    }
}

/// Verify one row. Records a checksum the first time for raw files.
pub fn verify_one(db: &Db, dir: &Path, rec: &Recording) -> Result<(), String> {
    if rec.offline() {
        return Ok(());
    }
    let path = crate::safepath::join_inside(dir, &rec.raw_path).ok_or_else(|| format!("{}: unsafe path", rec.raw_path))?;
    let sha = content_sha(&path).map_err(|e| format!("{}: {e}", rec.raw_path))?;
    if rec.sha256.is_empty() {
        db.set_sha(rec.id, &sha).map_err(|e| e.to_string())?;
        return Ok(());
    }
    if sha != rec.sha256 {
        return Err(format!("{}: checksum mismatch", rec.raw_path));
    }
    Ok(())
}

/// Finished, local, unarchived rows that started more than `days` ago.
pub fn older_than<'a>(rows: &'a [Recording], days: f64, now: chrono::DateTime<chrono::Utc>) -> Vec<&'a Recording> {
    let cut = now - chrono::Duration::milliseconds((days * 86_400_000.0) as i64);
    rows.iter()
        .filter(|r| !r.archived && !r.offline() && !r.missing && !r.meta.end_utc.is_empty())
        .filter(|r| {
            chrono::DateTime::parse_from_rfc3339(&r.meta.start_utc).map(|t| t.with_timezone(&chrono::Utc) < cut).unwrap_or(false)
        })
        .collect()
}

/// "7d", "12h", "30m", or a bare number of days.
pub fn parse_age(text: &str) -> Option<f64> {
    let t = text.trim();
    let (num, unit) = match t.chars().last()? {
        'd' | 'h' | 'm' | 'w' => (&t[..t.len() - 1], t.chars().last()?),
        _ => (t, 'd'),
    };
    let v: f64 = num.parse().ok()?;
    Some(match unit {
        'w' => v * 7.0,
        'h' => v / 24.0,
        'm' => v / 1440.0,
        _ => v,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::{meta, tempdir};

    fn setup(tag: &str) -> (PathBuf, Db, i64, Vec<i16>) {
        let dir = tempdir(tag);
        let (path, m) = crate::replay::tests_support::fm_file(&dir, 900.0);
        let db = Db::open_in_memory().unwrap();
        let mut m = m;
        m.start_utc = "2026-01-01T00:00:00Z".into();
        m.end_utc = "2026-01-01T00:00:01Z".into();
        m.bytes = std::fs::metadata(&path).unwrap().len() as i64;
        let id = db.insert(&crate::storage::relative(&dir, &path), &m).unwrap();
        let audio = crate::replay::decode_all(&path, &m).unwrap();
        (dir, db, id, audio)
    }

    #[test]
    fn round_trip_archive_replay_unarchive() {
        let (dir, db, id, audio) = setup("arch1");
        let rec = db.get(id).unwrap().unwrap();
        let out = archive_one(&db, &dir, &rec, 3, &AtomicBool::new(false));
        assert!(matches!(out, Outcome::Archived { .. }), "{out:?}");
        let rec = db.get(id).unwrap().unwrap();
        assert!(rec.archived && rec.raw_path.ends_with(".zst"));
        assert!(!dir.join("t.cs16").exists());
        assert!(rec.compressed_bytes > 0 && rec.sha256.len() == 64);
        assert_eq!(crate::replay::decode_all(&dir.join(&rec.raw_path), &rec.meta).unwrap(), audio);
        verify_one(&db, &dir, &rec).unwrap();
        assert_eq!(archive_one(&db, &dir, &rec, 3, &AtomicBool::new(false)), Outcome::Skipped("already archived"));
        unarchive_one(&db, &dir, &rec).unwrap();
        let rec = db.get(id).unwrap().unwrap();
        assert!(!rec.archived && dir.join("t.cs16").exists());
        assert_eq!(crate::replay::decode_all(&dir.join(&rec.raw_path), &rec.meta).unwrap(), audio);
    }

    #[test]
    fn cancelled_or_failed_keeps_original() {
        let (dir, db, id, _) = setup("arch2");
        let rec = db.get(id).unwrap().unwrap();
        assert_eq!(archive_one(&db, &dir, &rec, 3, &AtomicBool::new(true)), Outcome::Cancelled);
        assert!(dir.join("t.cs16").exists());
        assert!(!dir.join("t.cs16.zst.tmp").exists());
        assert!(!db.get(id).unwrap().unwrap().archived);
        // Missing source -> failure, nothing changes.
        let mut bad = rec.clone();
        bad.raw_path = "nope.cs16".into();
        assert!(matches!(archive_one(&db, &dir, &bad, 3, &AtomicBool::new(false)), Outcome::Failed(_)));
    }

    #[test]
    fn verify_detects_corruption() {
        let (dir, db, id, _) = setup("arch3");
        let rec = db.get(id).unwrap().unwrap();
        verify_one(&db, &dir, &rec).unwrap();
        let rec = db.get(id).unwrap().unwrap();
        std::fs::write(dir.join("t.cs16"), [1u8; 16]).unwrap();
        assert!(verify_one(&db, &dir, &rec).is_err());
    }

    #[test]
    fn ages() {
        assert_eq!(parse_age("7d"), Some(7.0));
        assert_eq!(parse_age("2w"), Some(14.0));
        assert_eq!(parse_age("12h"), Some(0.5));
        assert_eq!(parse_age("3"), Some(3.0));
        assert_eq!(parse_age("x"), None);
        let db = Db::open_in_memory().unwrap();
        db.insert("a", &meta("hackrf_tg", None, "2026-01-01T00:00:00Z", "2026-01-01T00:00:01Z")).unwrap();
        db.insert("b", &meta("hackrf_tg", None, "2026-01-09T00:00:00Z", "2026-01-09T00:00:01Z")).unwrap();
        let rows = db.all().unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-01-10T00:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let old = older_than(&rows, 7.0, now);
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].raw_path, "a");
    }
}
