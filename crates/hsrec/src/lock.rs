//! Lockfile beside the DB so archive/export runs never overlap (CLI with
//! CLI, or CLI with the GUI archiver).
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

pub struct Lock {
    _file: std::fs::File,
    pub path: PathBuf,
}

pub fn lock_path(db_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.lock", db_path.display()))
}

/// None when another run holds it.
pub fn try_lock(db_path: &Path) -> std::io::Result<Option<Lock>> {
    let path = lock_path(db_path);
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path)?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        return Ok(None);
    }
    Ok(Some(Lock { _file: file, path }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn second_lock_fails_until_released() {
        let dir = crate::db::tests::tempdir("lock");
        let db = dir.join("S.HSREC.sqlite");
        let a = try_lock(&db).unwrap();
        assert!(a.is_some());
        assert!(try_lock(&db).unwrap().is_none());
        drop(a);
        assert!(try_lock(&db).unwrap().is_some());
    }
}
