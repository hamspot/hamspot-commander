//! Storage layout and the disk cap.
//!
//! `<station dir>/Name.recordings/<source>/<YYYY-MM-DD>/<HHMMSS>_<lane>_<freq>.cs16`
//! (`.s16` for demod audio) plus a `.json` sidecar. Archived files gain `.zst`.
use crate::db::Recording;
use std::path::{Path, PathBuf};

pub const DEFAULT_CAP_GB: f64 = 20.0;
pub const WARN_LEVELS: [f64; 2] = [0.80, 0.95];
/// Long calls are split so one file never grows past ~5 minutes.
pub const SPLIT_MS: i64 = 5 * 60 * 1000;

pub fn recordings_dir_beside(station: &Path) -> PathBuf {
    let stem = station
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "station".into());
    station.with_file_name(format!("{stem}.recordings"))
}

pub fn cache_dir(rec_dir: &Path) -> PathBuf {
    rec_dir.join(".cache")
}

fn clean(text: &str) -> String {
    let s: String = text
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "x".into() } else { s.chars().take(32).collect() }
}

/// Relative path (no extension) for a new recording.
pub fn new_stem(source: &str, start: chrono::DateTime<chrono::Utc>, lane: &str, hz: f64, part: u32) -> String {
    let day = start.format("%Y-%m-%d");
    let time = start.format("%H%M%S");
    let suffix = if part > 0 { format!("_p{part}") } else { String::new() };
    format!("{}/{day}/{time}_{}_{:.4}{suffix}", clean(source), clean(lane), hz / 1e6)
}

pub fn relative(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir).unwrap_or(path).to_string_lossy().to_string()
}

/// Sidecar is `stem.json`; raw is `stem.cs16|.cs8|.s16` with optional `.zst`.
pub fn sidecar_for(raw: &Path) -> PathBuf {
    let mut s = raw.to_string_lossy().to_string();
    if let Some(x) = s.strip_suffix(".zst") {
        s = x.to_string();
    }
    let p = PathBuf::from(s);
    p.with_extension("json")
}

pub fn raw_for_sidecar(side: &Path) -> Option<PathBuf> {
    for ext in ["cs16", "cs8", "s16", "cs16.zst", "cs8.zst", "s16.zst"] {
        let p = side.with_extension(ext);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn80,
    Warn95,
    Full,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Usage {
    pub used: i64,
    pub cap: i64,
}

impl Usage {
    pub fn from_rows(rows: &[Recording], cap_gb: f64) -> Self {
        Self { used: rows.iter().map(|r| r.disk_bytes()).sum(), cap: cap_bytes(cap_gb) }
    }
    pub fn fraction(&self) -> f64 {
        if self.cap <= 0 { 1.0 } else { self.used as f64 / self.cap as f64 }
    }
    pub fn level(&self) -> Level {
        let f = self.fraction();
        if f >= 1.0 {
            Level::Full
        } else if f >= WARN_LEVELS[1] {
            Level::Warn95
        } else if f >= WARN_LEVELS[0] {
            Level::Warn80
        } else {
            Level::Ok
        }
    }
    /// Text for the meter; warnings always carry words, not only colour.
    pub fn text(&self) -> String {
        let gb = |b: i64| b as f64 / 1e9;
        let base = format!("{:.2} / {:.1} GB ({:.0}%)", gb(self.used), gb(self.cap), self.fraction() * 100.0);
        match self.level() {
            Level::Ok => base,
            Level::Warn80 => format!("{base}  ⚠ OVER 80%"),
            Level::Warn95 => format!("{base}  ⚠ OVER 95%: ARCHIVE OR EXPORT SOON"),
            Level::Full => format!("{base}  ⚠ DISK CAP REACHED: RECORDING STOPPED"),
        }
    }
}

pub fn cap_bytes(cap_gb: f64) -> i64 {
    (cap_gb.max(0.0) * 1e9) as i64
}

/// What to do when `need` more bytes would cross the cap.
#[derive(Debug, PartialEq)]
pub enum CapDecision {
    Fits,
    /// Delete these (oldest unprotected local rows) first.
    Overwrite(Vec<i64>),
    /// Stop recording and warn. Never deletes.
    Stop,
}

pub fn cap_decision(rows: &[Recording], cap_gb: f64, need: i64, overwrite_oldest: bool) -> CapDecision {
    let usage = Usage::from_rows(rows, cap_gb);
    if usage.used + need <= usage.cap {
        return CapDecision::Fits;
    }
    if !overwrite_oldest {
        return CapDecision::Stop;
    }
    let mut over = usage.used + need - usage.cap;
    let mut cand: Vec<&Recording> = rows
        .iter()
        .filter(|r| !r.protected && !r.offline() && !r.missing && r.disk_bytes() > 0)
        .collect();
    cand.sort_by(|a, b| a.meta.start_utc.cmp(&b.meta.start_utc));
    let mut ids = Vec::new();
    for r in cand {
        if over <= 0 {
            break;
        }
        over -= r.disk_bytes();
        ids.push(r.id);
    }
    if over > 0 { CapDecision::Stop } else { CapDecision::Overwrite(ids) }
}

/// Remove a recording's files (raw, sidecar). Derived cache is separate.
pub fn remove_files(rec_dir: &Path, raw_path: &str) {
    let raw = rec_dir.join(raw_path);
    let _ = std::fs::remove_file(sidecar_for(&raw));
    let _ = std::fs::remove_file(&raw);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::meta;
    use crate::db::Db;

    fn rows(n: usize, bytes: i64) -> Vec<Recording> {
        let db = Db::open_in_memory().unwrap();
        for i in 0..n {
            let mut m = meta("hackrf_tg", Some(1), &format!("2026-01-01T00:00:{i:02}Z"), "");
            m.bytes = bytes;
            db.insert(&format!("{i}"), &m).unwrap();
        }
        db.all().unwrap()
    }

    #[test]
    fn layout() {
        let t = chrono::DateTime::parse_from_rfc3339("2026-10-26T21:05:09Z").unwrap().with_timezone(&chrono::Utc);
        assert_eq!(new_stem("hackrf_tg", t, "Fire Dispatch (101)", 851_012_500.0, 0),
            "hackrf_tg/2026-10-26/210509_Fire_Dispatch__101_851.0125");
        assert!(new_stem("hackrf_tg", t, "x", 1e6, 2).ends_with("_p2"));
        assert_eq!(recordings_dir_beside(Path::new("/s/SAFE-T.HSCommander")), PathBuf::from("/s/SAFE-T.recordings"));
        assert_eq!(sidecar_for(Path::new("/a/b.cs16.zst")), PathBuf::from("/a/b.json"));
    }

    #[test]
    fn meter_thresholds() {
        let u = |used| Usage { used, cap: 100 };
        assert_eq!(u(79).level(), Level::Ok);
        assert_eq!(u(80).level(), Level::Warn80);
        assert_eq!(u(95).level(), Level::Warn95);
        assert_eq!(u(100).level(), Level::Full);
        assert!(u(100).text().contains("RECORDING STOPPED"));
    }

    #[test]
    fn stop_at_cap_by_default() {
        let r = rows(3, 400_000_000);
        assert_eq!(cap_decision(&r, 2.0, 100, false), CapDecision::Fits);
        assert_eq!(cap_decision(&r, 1.2, 1, false), CapDecision::Stop);
    }

    #[test]
    fn overwrite_oldest_skips_protected() {
        let mut r = rows(3, 400_000_000);
        r[0].protected = true;
        match cap_decision(&r, 1.2, 1, true) {
            CapDecision::Overwrite(ids) => assert_eq!(ids, vec![r[1].id]),
            other => panic!("{other:?}"),
        }
        for x in r.iter_mut() {
            x.protected = true;
        }
        assert_eq!(cap_decision(&r, 1.2, 1, true), CapDecision::Stop);
    }

    #[test]
    fn archived_and_offline_count_right() {
        let mut r = rows(2, 1000);
        r[0].archived = true;
        r[0].compressed_bytes = 100;
        r[1].location = "offline:USB".into();
        assert_eq!(Usage::from_rows(&r, 1.0).used, 100);
    }
}
