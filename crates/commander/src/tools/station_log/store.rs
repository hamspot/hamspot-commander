//! SQLite storage for the station log. Timestamps are stored as UTC ISO-8601
//! (`2026-10-22T12:34:56Z`) and converted to local time only for display.
//! A row may also stamp the dial: frequency in hertz, mode, and radio name.
use chrono::{DateTime, Local, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: i64,
    pub created_utc: String,
    pub text: String,
    /// Dial frequency in hertz. Absent when the operator did not press Log.
    pub hz: Option<f64>,
    pub mode: String,
    pub radio: String,
}

/// Dial fields written with a new entry. Empty mode and radio are stored as NULL-equivalent blanks.
#[derive(Clone, Debug, PartialEq)]
pub struct Stamp {
    pub hz: f64,
    pub mode: String,
    pub radio: String,
}

impl Entry {
    /// Local display time, e.g. "2026-10-22 08:34".
    pub fn local(&self) -> String {
        DateTime::parse_from_rfc3339(&self.created_utc)
            .map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|_| self.created_utc.clone())
    }
}

fn row_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: r.get(0)?,
        created_utc: r.get(1)?,
        text: r.get(2)?,
        hz: r.get(3)?,
        mode: r.get(4)?,
        radio: r.get(5)?,
    })
}

pub struct Store {
    conn: Connection,
}

/// Schema migrations, applied in order; `PRAGMA user_version` = number applied.
const MIGRATIONS: &[&str] = &["CREATE TABLE entries (
        id INTEGER PRIMARY KEY,
        created_utc TEXT NOT NULL,
        text TEXT NOT NULL,
        hz REAL,
        mode TEXT NOT NULL DEFAULT '',
        radio TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX entries_created ON entries(created_utc);"];

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    /// Tests open a database with no file. The running face uses [`Store::open`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        let v: usize = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(v) {
            conn.execute_batch(sql)?;
            conn.execute_batch(&format!("PRAGMA user_version = {}", i + 1))?;
        }
        Ok(Self { conn })
    }

    /// Tests check that the migration ran. The face does not read this.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn schema_version(&self) -> rusqlite::Result<i64> {
        self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))
    }

    pub fn now_utc() -> String {
        Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }

    pub fn add(&self, text: &str, created_utc: &str, stamp: Option<&Stamp>) -> rusqlite::Result<Entry> {
        let (hz, mode, radio) = match stamp {
            Some(stamp) => (Some(stamp.hz), stamp.mode.as_str(), stamp.radio.as_str()),
            None => (None, "", ""),
        };
        self.conn.execute(
            "INSERT INTO entries (created_utc, text, hz, mode, radio) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![created_utc, text, hz, mode, radio],
        )?;
        Ok(Entry {
            id: self.conn.last_insert_rowid(),
            created_utc: created_utc.into(),
            text: text.into(),
            hz,
            mode: mode.into(),
            radio: radio.into(),
        })
    }

    pub fn update(&self, id: i64, text: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE entries SET text = ?1 WHERE id = ?2",
            params![text, id],
        )?;
        Ok(())
    }

    /// Deletes and returns the entry so the caller can offer undo via [`Store::restore`].
    pub fn delete(&self, id: i64) -> rusqlite::Result<Option<Entry>> {
        let e = self
            .conn
            .query_row(
                "SELECT id, created_utc, text, hz, mode, radio FROM entries WHERE id = ?1",
                [id],
                |r| row_entry(r),
            )
            .optional()?;
        self.conn
            .execute("DELETE FROM entries WHERE id = ?1", [id])?;
        Ok(e)
    }

    /// Undo a delete: re-inserts with the original id and timestamp.
    pub fn restore(&self, e: &Entry) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO entries (id, created_utc, text, hz, mode, radio) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![e.id, e.created_utc, e.text, e.hz, e.mode, e.radio],
        )?;
        Ok(())
    }

    /// Newest first. `text` filters case-insensitively by substring; `date` (local
    /// "YYYY-MM-DD" prefix, e.g. "2026-10" or "2026-10-22") filters by local date.
    pub fn list(&self, text: &str, date: &str) -> rusqlite::Result<Vec<Entry>> {
        let mut st = self.conn.prepare(
            "SELECT id, created_utc, text, hz, mode, radio FROM entries
             WHERE text LIKE '%' || ?1 || '%' ESCAPE '\\'
             ORDER BY created_utc DESC, id DESC",
        )?;
        let esc = text
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let rows = st.query_map([esc], row_entry)?;
        let mut out = vec![];
        for e in rows {
            let e = e?;
            if date.is_empty() || e.local().starts_with(date.trim()) {
                out.push(e);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_add_list_filter() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as i64);
        s.add("first coffee", "2026-01-02T08:00:00Z", None).unwrap();
        s.add("Second entry about 100% effort", "2026-03-04T09:00:00Z", None)
            .unwrap();
        let all = s.list("", "").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].text, "Second entry about 100% effort"); // newest first
        assert_eq!(s.list("COFFEE", "").unwrap().len(), 1); // case-insensitive (ASCII)
        assert_eq!(s.list("100%", "").unwrap().len(), 1); // % is literal
        assert_eq!(s.list("_", "").unwrap().len(), 0); // _ is literal
        assert_eq!(s.list("", "2026-03").unwrap().len(), 1);
    }

    #[test]
    fn edit_delete_undo() {
        let s = Store::open_in_memory().unwrap();
        let e = s.add("typo", &Store::now_utc(), None).unwrap();
        s.update(e.id, "fixed").unwrap();
        assert_eq!(s.list("", "").unwrap()[0].text, "fixed");
        let gone = s.delete(e.id).unwrap().unwrap();
        assert!(s.list("", "").unwrap().is_empty());
        s.restore(&gone).unwrap();
        assert_eq!(s.list("", "").unwrap(), vec![gone]);
    }

    #[test]
    fn a_stamp_round_trips_and_delete_restores_it() {
        let s = Store::open_in_memory().unwrap();
        let stamp = Stamp {
            hz: 14_153_608.0,
            mode: "USB".into(),
            radio: "Dummy".into(),
        };
        let e = s
            .add("cq", "2026-10-24T18:00:00Z", Some(&stamp))
            .unwrap();
        let listed = s.list("", "").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].hz, Some(14_153_608.0));
        assert_eq!(listed[0].mode, "USB");
        assert_eq!(listed[0].radio, "Dummy");
        let gone = s.delete(e.id).unwrap().unwrap();
        s.restore(&gone).unwrap();
        assert_eq!(s.list("", "").unwrap()[0], gone);
    }

    #[test]
    fn timestamps_are_utc_iso8601() {
        let t = Store::now_utc();
        assert!(t.ends_with('Z') && DateTime::parse_from_rfc3339(&t).is_ok());
    }

    #[test]
    fn reopen_keeps_schema() {
        let dir = std::env::temp_dir().join(format!("rustcars-diary-test-{}", std::process::id()));
        let path = dir.join("d.sqlite");
        {
            let s = Store::open(&path).unwrap();
            s.add("persist", "2026-01-01T00:00:00Z", None).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.list("", "").unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
