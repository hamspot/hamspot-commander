//! Integration tests for commander-rec on temp dirs.
use hsrec::db::Db;
use hsrec::meta::*;
use std::path::{Path, PathBuf};
use std::process::Command;

fn temp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("hsrec-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Station with one 10-day-old and one fresh recording.
fn station(tag: &str) -> (PathBuf, PathBuf) {
    let base = temp(tag);
    let station = base.join("S.HSCommander");
    std::fs::write(&station, "{}").unwrap();
    let dir = hsrec::storage::recordings_dir_beside(&station);
    let db = Db::open(&hsrec::db::db_path_beside(&station)).unwrap();
    let now = chrono::Utc::now();
    for (i, age) in [10i64, 0].iter().enumerate() {
        let start = now - chrono::Duration::days(*age) - chrono::Duration::seconds(5);
        let rel = format!("hackrf_tg/x{i}.cs16");
        let data: Vec<u8> = (0..64_000u32).flat_map(|k| ((k % 200) as i16).to_le_bytes()).collect();
        std::fs::create_dir_all(dir.join("hackrf_tg")).unwrap();
        std::fs::write(dir.join(&rel), &data).unwrap();
        let m = Meta {
            source: SOURCE_HACKRF_TG.into(), source_kind: KIND_RAW_IQ.into(), talkgroup: Some(100 + i as u32),
            raw_format: "cs16".into(), sample_rate: 32_000,
            start_utc: start.to_rfc3339(), end_utc: (start + chrono::Duration::seconds(1)).to_rfc3339(),
            duration_ms: 1000, bytes: data.len() as i64, ..Default::default()
        };
        db.insert(&rel, &m).unwrap();
    }
    (station, dir)
}

fn run(args: &[&str], station: &Path) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_commander-rec"))
        .args(args)
        .arg("--station")
        .arg(station)
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr))
}

#[test]
fn dry_run_changes_nothing() {
    let (st, dir) = station("dry");
    let (code, out) = run(&["archive", "--older-than", "7d", "--dry-run"], &st);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("would archive hackrf_tg/x0.cs16"));
    assert!(!out.contains("x1"));
    assert!(dir.join("hackrf_tg/x0.cs16").exists());
    assert!(!dir.join("hackrf_tg/x0.cs16.zst").exists());
}

#[test]
fn archive_verify_list_stats_unarchive() {
    let (st, dir) = station("arch");
    let (code, out) = run(&["archive", "--older-than", "7d", "--level", "3", "--nice"], &st);
    assert_eq!(code, 0, "{out}");
    assert!(dir.join("hackrf_tg/x0.cs16.zst").exists());
    assert!(dir.join("hackrf_tg/x1.cs16").exists());
    let (code, out) = run(&["verify"], &st);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.matches("ok ").count(), 2);
    let (code, out) = run(&["list"], &st);
    assert_eq!(code, 0);
    assert!(out.contains("archived"));
    let (code, out) = run(&["stats"], &st);
    assert_eq!(code, 0);
    assert!(out.contains("hackrf_tg") && out.contains("archived: 1"));
    let (code, out) = run(&["unarchive"], &st);
    assert_eq!(code, 0, "{out}");
    assert!(dir.join("hackrf_tg/x0.cs16").exists());
    // Corrupt a file: verify reports partial failure (2).
    std::fs::write(dir.join("hackrf_tg/x0.cs16"), [9u8; 8]).unwrap();
    assert_eq!(run(&["verify"], &st).0, 2);
}

#[test]
fn lock_contention_returns_3() {
    let (st, _) = station("lock");
    let _held = hsrec::lock::try_lock(&hsrec::db::db_path_beside(&st)).unwrap().unwrap();
    let (code, out) = run(&["archive", "--older-than", "7d"], &st);
    assert_eq!(code, 3, "{out}");
}

#[test]
fn export_move_to_fake_mount() {
    let (st, dir) = station("export");
    let usb = st.parent().unwrap().join("media/USB");
    std::fs::create_dir_all(&usb).unwrap();
    let (code, out) = run(&["export", "--to", usb.to_str().unwrap(), "--move", "--older-than", "7d"], &st);
    assert_eq!(code, 0, "{out}");
    assert!(usb.join("HamSpot-REC/hackrf_tg/x0.cs16").exists());
    assert!(!dir.join("hackrf_tg/x0.cs16").exists());
    let (_, out) = run(&["list"], &st);
    assert!(out.contains("offline:USB"));
}

#[test]
fn errors_exit_1() {
    let base = temp("err");
    assert_eq!(run(&["stats"], &base.join("none.HSCommander")).0, 1);
    let (st, _) = station("err2");
    assert_eq!(run(&["bogus"], &st).0, 1);
}
