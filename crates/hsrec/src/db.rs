//! `Name.HSREC.sqlite`: recordings index. Same migration pattern as the
//! Logbook store (`MIGRATIONS` + `PRAGMA user_version`). The raw files and
//! their JSON sidecars are the source of truth; [`Db::rescan`] rebuilds rows.
use crate::meta::Meta;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

const MIGRATIONS: &[&str] = &["CREATE TABLE recordings (
        id INTEGER PRIMARY KEY,
        source TEXT NOT NULL,
        source_kind TEXT NOT NULL DEFAULT 'raw_iq',
        radio_id INTEGER NOT NULL DEFAULT 0,
        system TEXT NOT NULL DEFAULT '',
        site TEXT NOT NULL DEFAULT '',
        talkgroup INTEGER,
        channel_name TEXT NOT NULL DEFAULT '',
        radio_unit_id INTEGER,
        raw_path TEXT NOT NULL UNIQUE,
        raw_format TEXT NOT NULL,
        sample_rate INTEGER NOT NULL,
        centre_hz REAL NOT NULL DEFAULT 0,
        channel_hz REAL NOT NULL DEFAULT 0,
        offset_hz REAL NOT NULL DEFAULT 0,
        bandwidth_hz REAL NOT NULL DEFAULT 0,
        gains_json TEXT NOT NULL DEFAULT 'null',
        modulation TEXT NOT NULL DEFAULT '',
        decoder_json TEXT NOT NULL DEFAULT 'null',
        sw_version TEXT NOT NULL DEFAULT '',
        start_utc TEXT NOT NULL,
        end_utc TEXT NOT NULL DEFAULT '',
        duration_ms INTEGER NOT NULL DEFAULT 0,
        bytes INTEGER NOT NULL DEFAULT 0,
        emergency INTEGER NOT NULL DEFAULT 0,
        priority INTEGER NOT NULL DEFAULT 0,
        protected INTEGER NOT NULL DEFAULT 0,
        star INTEGER NOT NULL DEFAULT 0,
        notes TEXT NOT NULL DEFAULT '',
        archived INTEGER NOT NULL DEFAULT 0,
        codec TEXT NOT NULL DEFAULT '',
        level INTEGER NOT NULL DEFAULT 0,
        compressed_bytes INTEGER NOT NULL DEFAULT 0,
        sha256 TEXT NOT NULL DEFAULT '',
        location TEXT NOT NULL DEFAULT 'local',
        missing INTEGER NOT NULL DEFAULT 0,
        meta_json TEXT NOT NULL DEFAULT '{}'
    );
    CREATE INDEX rec_source_start ON recordings(source, start_utc);
    CREATE INDEX rec_tg_start ON recordings(talkgroup, start_utc);
    CREATE INDEX rec_channel_start ON recordings(channel_name, start_utc);
    CREATE TABLE derived (
        recording_id INTEGER NOT NULL,
        kind TEXT NOT NULL,
        decoder_version TEXT NOT NULL,
        path TEXT NOT NULL,
        PRIMARY KEY (recording_id, kind, decoder_version)
    );"];

#[derive(Clone, Debug, PartialEq)]
pub struct Recording {
    pub id: i64,
    pub meta: Meta,
    /// Relative to the recordings directory.
    pub raw_path: String,
    pub protected: bool,
    pub star: bool,
    pub notes: String,
    pub archived: bool,
    pub codec: String,
    pub level: i32,
    pub compressed_bytes: i64,
    pub sha256: String,
    /// `local` or `offline:<volume label>`.
    pub location: String,
    pub missing: bool,
}

impl Recording {
    pub fn offline(&self) -> bool {
        self.location.starts_with("offline:")
    }
    /// Bytes this row costs against the disk cap.
    pub fn disk_bytes(&self) -> i64 {
        if self.offline() || self.missing {
            0
        } else if self.archived {
            self.compressed_bytes
        } else {
            self.meta.bytes
        }
    }
}

const COLS: &str = "id, raw_path, protected, star, notes, archived, codec, level, compressed_bytes, sha256, location, missing, meta_json, end_utc, duration_ms, bytes";

fn row_rec(r: &rusqlite::Row<'_>) -> rusqlite::Result<Recording> {
    let json: String = r.get(12)?;
    let mut meta: Meta = serde_json::from_str(&json).unwrap_or_default();
    // Old rows may carry an absolute decoder path; show and use only the portable name.
    crate::safepath::portable_meta(&mut meta);
    meta.end_utc = r.get(13)?;
    meta.duration_ms = r.get(14)?;
    meta.bytes = r.get(15)?;
    Ok(Recording {
        id: r.get(0)?,
        raw_path: r.get(1)?,
        protected: r.get::<_, i64>(2)? != 0,
        star: r.get::<_, i64>(3)? != 0,
        notes: r.get(4)?,
        archived: r.get::<_, i64>(5)? != 0,
        codec: r.get(6)?,
        level: r.get(7)?,
        compressed_bytes: r.get(8)?,
        sha256: r.get(9)?,
        location: r.get(10)?,
        missing: r.get::<_, i64>(11)? != 0,
        meta,
    })
}

pub struct Db {
    pub conn: Connection,
}

/// `Name.HSREC.sqlite` beside `Name.HSCommander` (or any station file).
pub fn db_path_beside(station: &Path) -> std::path::PathBuf {
    let stem = station
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "station".into());
    station.with_file_name(format!("{stem}.HSREC.sqlite"))
}

/// An error with our own text, for cases SQLite itself would not report.
fn fail(msg: String) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR), Some(msg))
}

impl Db {
    /// Open the recordings database, creating it (and its folder) quietly
    /// when missing or zero bytes. Safe when the UI and `commander-rec`
    /// open it at once: the schema is applied inside one IMMEDIATE
    /// transaction behind a busy timeout. A file that is not a database, or
    /// one from a newer Commander, is refused and never written to.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .map_err(|e| fail(format!("cannot create the folder for the recordings database: {e}")))?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        // Reading the header first refuses a damaged or newer file before any write.
        let v = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?;
        Self::check_version(v)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    fn check_version(v: i64) -> rusqlite::Result<()> {
        if v as usize > MIGRATIONS.len() {
            return Err(fail(format!(
                "recordings database is from a newer Commander (schema {v}, this build knows {})",
                MIGRATIONS.len()
            )));
        }
        Ok(())
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        let version = |c: &Connection| c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0));
        let v = version(&conn)?;
        Self::check_version(v)?;
        if (v as usize) < MIGRATIONS.len() {
            conn.execute_batch("BEGIN IMMEDIATE")?;
            let run = || -> rusqlite::Result<()> {
                // Another process may have finished the schema while we waited.
                let v = version(&conn)? as usize;
                for (i, sql) in MIGRATIONS.iter().enumerate().skip(v) {
                    conn.execute_batch(sql)?;
                    conn.execute_batch(&format!("PRAGMA user_version = {}", i + 1))?;
                }
                Ok(())
            };
            match run() {
                Ok(()) => conn.execute_batch("COMMIT")?,
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(e);
                }
            }
        }
        Ok(Self { conn })
    }

    pub fn insert(&self, raw_path: &str, meta: &Meta) -> rusqlite::Result<i64> {
        let mut owned = meta.clone();
        crate::safepath::portable_meta(&mut owned);
        let meta = &owned;
        self.conn.execute(
            "INSERT INTO recordings (source, source_kind, radio_id, system, site, talkgroup, channel_name,
              radio_unit_id, raw_path, raw_format, sample_rate, centre_hz, channel_hz, offset_hz,
              bandwidth_hz, gains_json, modulation, decoder_json, sw_version, start_utc, end_utc,
              duration_ms, bytes, emergency, priority, meta_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26)",
            params![
                meta.source,
                meta.source_kind,
                meta.radio_id as i64,
                meta.system,
                meta.site,
                meta.talkgroup,
                meta.channel_name,
                meta.radio_unit_id,
                raw_path,
                meta.raw_format,
                meta.sample_rate,
                meta.centre_hz,
                meta.channel_hz,
                meta.offset_hz,
                meta.bandwidth_hz,
                meta.gains.to_string(),
                meta.modulation,
                meta.decoder.to_string(),
                meta.sw_version,
                meta.start_utc,
                meta.end_utc,
                meta.duration_ms,
                meta.bytes,
                meta.emergency as i64,
                meta.priority as i64,
                serde_json::to_string(meta).unwrap_or_default(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get(&self, id: i64) -> rusqlite::Result<Option<Recording>> {
        self.conn
            .query_row(&format!("SELECT {COLS} FROM recordings WHERE id = ?1"), [id], row_rec)
            .optional()
    }

    pub fn by_path(&self, raw_path: &str) -> rusqlite::Result<Option<Recording>> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM recordings WHERE raw_path = ?1"),
                [raw_path],
                row_rec,
            )
            .optional()
    }

    /// Rows of `source` overlapping [from, to) (UTC ISO strings), oldest first.
    pub fn range(&self, source: &str, from: &str, to: &str) -> rusqlite::Result<Vec<Recording>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {COLS} FROM recordings WHERE source = ?1 AND start_utc < ?3
             AND (end_utc >= ?2 OR end_utc = '') ORDER BY start_utc"
        ))?;
        let rows = st.query_map(params![source, from, to], row_rec)?;
        rows.collect()
    }

    pub fn all(&self) -> rusqlite::Result<Vec<Recording>> {
        let mut st = self.conn.prepare(&format!("SELECT {COLS} FROM recordings ORDER BY start_utc"))?;
        let rows = st.query_map([], row_rec)?;
        rows.collect()
    }

    pub fn sources(&self) -> rusqlite::Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT DISTINCT source FROM recordings ORDER BY source")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    pub fn by_talkgroup(&self, tg: u32) -> rusqlite::Result<Vec<Recording>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {COLS} FROM recordings WHERE talkgroup = ?1 ORDER BY start_utc"
        ))?;
        let rows = st.query_map([tg], row_rec)?;
        rows.collect()
    }

    pub fn finish(&self, id: i64, meta: &Meta) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE recordings SET end_utc = ?2, duration_ms = ?3, bytes = ?4, meta_json = ?5 WHERE id = ?1",
            params![id, meta.end_utc, meta.duration_ms, meta.bytes, serde_json::to_string(meta).unwrap_or_default()],
        )?;
        Ok(())
    }

    pub fn set_flag(&self, id: i64, column: &str, on: bool) -> rusqlite::Result<()> {
        assert!(matches!(column, "protected" | "star" | "missing"));
        self.conn
            .execute(&format!("UPDATE recordings SET {column} = ?2 WHERE id = ?1"), params![id, on as i64])?;
        Ok(())
    }

    pub fn set_notes(&self, id: i64, notes: &str) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE recordings SET notes = ?2 WHERE id = ?1", params![id, notes])?;
        Ok(())
    }

    pub fn set_archived(
        &self,
        id: i64,
        raw_path: &str,
        archived: bool,
        codec: &str,
        level: i32,
        compressed_bytes: i64,
        sha256: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE recordings SET raw_path = ?2, archived = ?3, codec = ?4, level = ?5,
             compressed_bytes = ?6, sha256 = ?7 WHERE id = ?1",
            params![id, raw_path, archived as i64, codec, level, compressed_bytes, sha256],
        )?;
        Ok(())
    }

    pub fn set_sha(&self, id: i64, sha256: &str) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE recordings SET sha256 = ?2 WHERE id = ?1", params![id, sha256])?;
        Ok(())
    }

    pub fn set_location(&self, id: i64, location: &str) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE recordings SET location = ?2 WHERE id = ?1", params![id, location])?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM derived WHERE recording_id = ?1", [id])?;
        self.conn.execute("DELETE FROM recordings WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Re-insert a deleted row (undo). Keeps the old id.
    pub fn restore(&self, rec: &Recording) -> rusqlite::Result<()> {
        let id = self.insert(&rec.raw_path, &rec.meta)?;
        self.conn.execute("UPDATE recordings SET id = ?2 WHERE id = ?1", params![id, rec.id])?;
        self.conn.execute(
            "UPDATE recordings SET protected=?2, star=?3, notes=?4, archived=?5, codec=?6, level=?7,
             compressed_bytes=?8, sha256=?9, location=?10 WHERE id = ?1",
            params![rec.id, rec.protected as i64, rec.star as i64, rec.notes, rec.archived as i64,
                rec.codec, rec.level, rec.compressed_bytes, rec.sha256, rec.location],
        )?;
        Ok(())
    }

    pub fn add_derived(&self, id: i64, kind: &str, version: &str, path: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO derived (recording_id, kind, decoder_version, path) VALUES (?1,?2,?3,?4)",
            params![id, kind, version, path],
        )?;
        Ok(())
    }

    pub fn derived(&self, id: i64, kind: &str, version: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT path FROM derived WHERE recording_id=?1 AND kind=?2 AND decoder_version=?3",
                params![id, kind, version],
                |r| r.get(0),
            )
            .optional()
    }

    pub fn clear_derived(&self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM derived", [])?;
        Ok(())
    }

    /// Walk `dir` for sidecars (`*.json`), add rows for any not indexed, and
    /// flag rows whose raw file is gone (local rows only). Returns (added, missing).
    pub fn rescan(&self, dir: &Path) -> rusqlite::Result<(usize, usize)> {
        let mut added = 0;
        let mut sidecars = Vec::new();
        walk(dir, &mut sidecars);
        for side in sidecars {
            let Ok(text) = std::fs::read_to_string(&side) else { continue };
            let Ok(mut meta) = serde_json::from_str::<Meta>(&text) else { continue };
            crate::safepath::portable_meta(&mut meta);
            let Some(raw) = crate::storage::raw_for_sidecar(&side) else { continue };
            let rel = crate::storage::relative(dir, &raw);
            if self.by_path(&rel)?.is_none() {
                let id = self.insert(&rel, &meta)?;
                if rel.ends_with(".zst") {
                    let size = std::fs::metadata(&raw).map(|m| m.len() as i64).unwrap_or(0);
                    self.set_archived(id, &rel, true, "zstd", 0, size, "")?;
                }
                added += 1;
            }
        }
        let mut missing = 0;
        for rec in self.all()? {
            if rec.offline() {
                continue;
            }
            let gone = !crate::safepath::join_inside(dir, &rec.raw_path).is_some_and(|p| p.exists());
            if gone != rec.missing {
                self.set_flag(rec.id, "missing", gone)?;
            }
            if gone {
                missing += 1;
            }
        }
        Ok((added, missing))
    }
}

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else if name.ends_with(".json") {
            out.push(p);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::meta::*;

    pub fn meta(source: &str, tg: Option<u32>, start: &str, end: &str) -> Meta {
        Meta {
            source: source.into(),
            source_kind: KIND_RAW_IQ.into(),
            talkgroup: tg,
            raw_format: "cs16".into(),
            sample_rate: 32_000,
            start_utc: start.into(),
            end_utc: end.into(),
            duration_ms: 1000,
            bytes: 128_000,
            ..Default::default()
        }
    }

    #[test]
    fn migrations_from_empty_and_reopen() {
        let dir = tempdir("db_mig");
        let p = dir.join("S.HSREC.sqlite");
        {
            let db = Db::open(&p).unwrap();
            db.insert("a.cs16", &meta(SOURCE_HACKRF_TG, Some(1), "2026-01-01T00:00:00Z", "2026-01-01T00:00:01Z")).unwrap();
        }
        let db = Db::open(&p).unwrap();
        let v: i64 = db.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
        assert_eq!(db.all().unwrap().len(), 1);
    }

    #[test]
    fn range_and_lane_queries() {
        let db = Db::open_in_memory().unwrap();
        db.insert("1", &meta(SOURCE_HACKRF_TG, Some(10), "2026-01-01T10:00:00Z", "2026-01-01T10:00:05Z")).unwrap();
        db.insert("2", &meta(SOURCE_HACKRF_TG, Some(11), "2026-01-01T11:00:00Z", "2026-01-01T11:00:05Z")).unwrap();
        db.insert("3", &meta(SOURCE_HACKRF_CHANNELS, None, "2026-01-01T10:30:00Z", "2026-01-01T10:30:05Z")).unwrap();
        let r = db.range(SOURCE_HACKRF_TG, "2026-01-01T09:00:00Z", "2026-01-01T10:59:00Z").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].meta.talkgroup, Some(10));
        // A recording that started before the window but ends inside it counts.
        let r = db.range(SOURCE_HACKRF_TG, "2026-01-01T10:00:03Z", "2026-01-01T12:00:00Z").unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(db.by_talkgroup(11).unwrap().len(), 1);
        assert_eq!(db.sources().unwrap(), vec!["hackrf_channels", "hackrf_tg"]);
    }

    #[test]
    fn dropping_derived_loses_nothing() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert("1", &meta(SOURCE_HACKRF_TG, Some(10), "2026-01-01T10:00:00Z", "x")).unwrap();
        db.add_derived(id, "waveform", "v1", ".cache/1.wf").unwrap();
        assert!(db.derived(id, "waveform", "v1").unwrap().is_some());
        db.conn.execute_batch("DROP TABLE derived").unwrap();
        assert_eq!(db.get(id).unwrap().unwrap().meta.talkgroup, Some(10));
    }

    #[test]
    fn delete_and_undo() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert("1", &meta(SOURCE_HACKRF_TG, Some(10), "2026-01-01T10:00:00Z", "x")).unwrap();
        db.set_flag(id, "protected", true).unwrap();
        let rec = db.get(id).unwrap().unwrap();
        db.delete(id).unwrap();
        assert!(db.get(id).unwrap().is_none());
        db.restore(&rec).unwrap();
        let back = db.get(id).unwrap().unwrap();
        assert!(back.protected);
        assert_eq!(back.meta, rec.meta);
    }

    #[test]
    fn rescan_adds_from_sidecars_and_flags_missing() {
        let dir = tempdir("db_rescan");
        let sub = dir.join("hackrf_tg/2026-01-01");
        std::fs::create_dir_all(&sub).unwrap();
        let m = meta(SOURCE_HACKRF_TG, Some(5), "2026-01-01T00:00:00Z", "2026-01-01T00:00:01Z");
        std::fs::write(sub.join("000000_tg5_851.cs16"), [0u8; 8]).unwrap();
        std::fs::write(sub.join("000000_tg5_851.json"), serde_json::to_string(&m).unwrap()).unwrap();
        let db = Db::open_in_memory().unwrap();
        db.insert("hackrf_tg/gone.cs16", &m).unwrap();
        let (added, missing) = db.rescan(&dir).unwrap();
        assert_eq!((added, missing), (1, 1));
        assert_eq!(db.rescan(&dir).unwrap(), (0, 1));
    }

    pub fn tempdir(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("hsrec-{tag}-{}-{}", std::process::id(), rand_tag()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn rand_tag() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }
}

#[cfg(test)]
mod autocreate_tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("hsrec-autocreate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }
    fn count(db: &Db) -> i64 {
        db.conn.query_row("SELECT COUNT(*) FROM recordings", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn missing_database_and_folder_are_created() {
        let d = tmp("missing");
        let p = d.join("a").join("b").join("S.HSREC.sqlite");
        let db = Db::open(&p).unwrap();
        assert_eq!(count(&db), 0);
        assert!(p.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn zero_byte_file_is_initialised() {
        let d = tmp("zero");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("S.HSREC.sqlite");
        std::fs::write(&p, b"").unwrap();
        assert_eq!(count(&Db::open(&p).unwrap()), 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn existing_database_is_kept() {
        let d = tmp("keep");
        let p = d.join("S.HSREC.sqlite");
        {
            let db = Db::open(&p).unwrap();
            db.insert("x.raw", &Meta::default()).unwrap();
        }
        assert_eq!(count(&Db::open(&p).unwrap()), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn corrupt_database_is_refused_and_untouched() {
        let d = tmp("corrupt");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("S.HSREC.sqlite");
        let junk = vec![0x5au8; 8192];
        std::fs::write(&p, &junk).unwrap();
        assert!(Db::open(&p).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), junk);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn newer_schema_is_refused_and_untouched() {
        let d = tmp("newer");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("S.HSREC.sqlite");
        {
            let c = Connection::open(&p).unwrap();
            c.execute_batch("CREATE TABLE future (x); PRAGMA user_version = 99;").unwrap();
        }
        let before = std::fs::read(&p).unwrap();
        let e = Db::open(&p).err().expect("refused").to_string();
        assert!(e.contains("newer"), "{e}");
        assert_eq!(std::fs::read(&p).unwrap(), before);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn two_threads_create_at_once() {
        let d = tmp("race");
        let p = d.join("sub").join("S.HSREC.sqlite");
        let hs: Vec<_> = (0..4)
            .map(|_| {
                let p = p.clone();
                std::thread::spawn(move || Db::open(&p).map(|db| count(&db)))
            })
            .collect();
        for h in hs {
            assert_eq!(h.join().unwrap().unwrap(), 0);
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
