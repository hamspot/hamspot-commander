//! Derived cache: waveform thumbnails (and any decoded audio) generated
//! from raw IQ. Disposable: deleting `.cache/` loses nothing, and a new
//! decoder version invalidates old entries.
use crate::db::{Db, Recording};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const DEFAULT_CACHE_BYTES: u64 = 256 * 1024 * 1024;
pub const WAVEFORM_BUCKETS: usize = 400;

/// Version of the replay chain. Changes whenever the decoder changes.
pub fn decoder_version() -> String {
    let mut h = Sha256::new();
    h.update(crate::dsp::CHAIN_ID.as_bytes());
    h.update(crate::meta::sw_version().as_bytes());
    hex(&h.finalize()[..6])
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn entry_path(rec_dir: &Path, id: i64, kind: &str, version: &str) -> PathBuf {
    crate::storage::cache_dir(rec_dir).join(format!("{id}-{kind}-{version}.bin"))
}

/// Waveform from the cache, or generate it by decoding the raw file.
pub fn waveform(db: &Db, rec_dir: &Path, rec: &Recording, version: &str) -> std::io::Result<Vec<u8>> {
    let path = entry_path(rec_dir, rec.id, "waveform", version);
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(bytes);
    }
    let audio = crate::replay::decode_all(&rec_dir.join(&rec.raw_path), &rec.meta)?;
    let wf = crate::replay::waveform(&audio, WAVEFORM_BUCKETS);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, &wf)?;
    let _ = db.add_derived(rec.id, "waveform", version, &crate::storage::relative(rec_dir, &path));
    Ok(wf)
}

pub fn size(rec_dir: &Path) -> u64 {
    files(rec_dir).iter().map(|(_, len, _)| *len).sum()
}

fn files(rec_dir: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let Ok(rd) = std::fs::read_dir(crate::storage::cache_dir(rec_dir)) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (e.path(), m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH)))
        })
        .collect()
}

/// "Clear cache" button.
pub fn clear(db: &Db, rec_dir: &Path) -> std::io::Result<()> {
    let _ = db.clear_derived();
    match std::fs::remove_dir_all(crate::storage::cache_dir(rec_dir)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Drop stale versions, then oldest entries until under `max_bytes`.
pub fn prune(rec_dir: &Path, max_bytes: u64, version: &str) -> usize {
    let mut removed = 0;
    let mut list = files(rec_dir);
    list.retain(|(p, _, _)| {
        let stale = !p.to_string_lossy().contains(&format!("-{version}."));
        if stale && std::fs::remove_file(p).is_ok() {
            removed += 1;
        }
        !stale
    });
    list.sort_by_key(|(_, _, t)| *t);
    let mut total: u64 = list.iter().map(|(_, l, _)| *l).sum();
    for (p, len, _) in list {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::tempdir;

    #[test]
    fn keying_and_pruning() {
        let dir = tempdir("cache1");
        let a = entry_path(&dir, 1, "waveform", "v1");
        let b = entry_path(&dir, 2, "waveform", "v2");
        assert_ne!(a, b);
        std::fs::create_dir_all(a.parent().unwrap()).unwrap();
        std::fs::write(&a, [0u8; 100]).unwrap();
        std::fs::write(&b, [0u8; 100]).unwrap();
        // Version change invalidates v1.
        assert_eq!(prune(&dir, u64::MAX, "v2"), 1);
        assert!(!a.exists() && b.exists());
        assert_eq!(prune(&dir, 10, "v2"), 1);
        assert_eq!(size(&dir), 0);
    }

    #[test]
    fn waveform_generated_then_cached_and_clear_loses_nothing() {
        let dir = tempdir("cache2");
        let db = Db::open_in_memory().unwrap();
        let (path, meta) = crate::replay::tests_support::fm_file(&dir, 700.0);
        let rel = crate::storage::relative(&dir, &path);
        let id = db.insert(&rel, &meta).unwrap();
        let rec = db.get(id).unwrap().unwrap();
        let wf = waveform(&db, &dir, &rec, "v1").unwrap();
        assert_eq!(wf.len(), WAVEFORM_BUCKETS);
        assert!(entry_path(&dir, id, "waveform", "v1").exists());
        clear(&db, &dir).unwrap();
        assert!(db.get(id).unwrap().is_some());
        assert_eq!(waveform(&db, &dir, &rec, "v1").unwrap(), wf);
    }
}
