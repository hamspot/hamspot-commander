//! commander-rec: archive and maintain HamSpot Commander REC recordings
//! from the command line (cron-friendly). Exit codes: 0 ok, 1 error,
//! 2 partial / verify failure, 3 another run holds the lock.
use hsrec::{archive, db::Db, export, lock, storage};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

const USAGE: &str = "usage: commander-rec <archive|unarchive|list|verify|stats|export> --station PATH [options]
  archive   [--older-than 7d] [--level 1-19] [--nice] [--dry-run]
  unarchive [--older-than AGE] [--dry-run]
  list      [--source hackrf_tg]
  verify    [--archived-only]
  stats     [--cap-gb 20]
  export    --to DIR [--label NAME] [--move] [--older-than AGE] [--dry-run]
  render-wav RAW OUT.wav [--decoder SCRIPT] [--param HAMSPOT_K=V]...   (no --station; re-demod one file)
example crontab:
  15 3 * * * nice -n 19 ionice -c3 commander-rec archive --older-than 7d --station ~/stations/SAFE-T.HSCommander";

#[derive(Default)]
struct Args {
    cmd: String,
    station: Option<PathBuf>,
    older: Option<f64>,
    level: i32,
    nice: bool,
    dry: bool,
    source: Option<String>,
    archived_only: bool,
    cap_gb: f64,
    to: Option<PathBuf>,
    label: Option<String>,
    mv: bool,
}

fn parse(argv: &[String]) -> Result<Args, String> {
    let mut a = Args { level: archive::DEFAULT_LEVEL, cap_gb: storage::DEFAULT_CAP_GB, ..Default::default() };
    let mut it = argv.iter();
    a.cmd = it.next().cloned().ok_or("no command")?;
    while let Some(x) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("{x} needs a value"));
        match x.as_str() {
            "--station" => a.station = Some(PathBuf::from(expand(&val()?))),
            "--older-than" => a.older = Some(archive::parse_age(&val()?).ok_or("bad --older-than (e.g. 7d)")?),
            "--level" => a.level = val()?.parse::<i32>().map_err(|_| "bad --level")?.clamp(1, 19),
            "--nice" => a.nice = true,
            "--dry-run" | "-n" => a.dry = true,
            "--source" => a.source = Some(val()?),
            "--archived-only" => a.archived_only = true,
            "--cap-gb" => a.cap_gb = val()?.parse().map_err(|_| "bad --cap-gb")?,
            "--to" => a.to = Some(PathBuf::from(expand(&val()?))),
            "--label" => a.label = Some(val()?),
            "--move" => a.mv = true,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(a)
}

fn expand(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => p.to_string(),
    }
}

/// `commander-rec render-wav RAW OUT.wav [--decoder SCRIPT] [--param HAMSPOT_K=V]...`
/// Re-demodulates one recording (sidecar `RAW` with .json beside it) the way
/// playback does and writes the speaker audio to a WAV. Reads nothing else.
fn render_wav(args: &[String]) -> u8 {
    let mut pos = Vec::new();
    let mut decoder: Option<hsrec::replay::DecoderCmd> = None;
    let mut env = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--decoder" => {
                let Some(script) = it.next() else { eprintln!("--decoder needs a path"); return 2 };
                decoder = Some(hsrec::replay::DecoderCmd { argv: vec![script.clone(), "23999".into()], ..Default::default() });
            }
            "--env" | "--param" => {
                let Some(kv) = it.next().and_then(|kv| kv.split_once('=')) else { eprintln!("--env K=V"); return 2 };
                env.push((kv.0.to_string(), kv.1.to_string()));
            }
            _ => pos.push(a.clone()),
        }
    }
    let [raw, out] = pos.as_slice() else {
        eprintln!("usage: commander-rec render-wav RAW OUT.wav [--decoder SCRIPT] [--param HAMSPOT_K=V]...");
        return 2;
    };
    let raw = std::path::PathBuf::from(raw);
    let side = raw.with_extension("json");
    let meta: hsrec::meta::Meta = match std::fs::read_to_string(&side).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("commander-rec: {}: {e}", side.display());
            return 1;
        }
    };
    if let Some(d) = decoder.as_mut() {
        // Saved decoder_params, then --param overrides. Keys must be HAMSPOT_*.
        for (k, _) in &env {
            if !hsrec::replay::param_key_ok(k) {
                eprintln!("commander-rec: --param {k}: ignored (keys are HAMSPOT_[A-Z0-9_]+)");
            }
        }
        d.env = hsrec::replay::decoder_env(&meta, &env);
        for (k, v) in &d.env {
            println!("param {k}={v}");
        }
    }
    match hsrec::replay::render(&raw, &meta, decoder.as_ref()) {
        Ok(r) => {
            let _ = hsrec::replay::write_wav(std::path::Path::new(out), &r.audio, if decoder.is_some() { hsrec::replay::OUT_RATE } else { hsrec::replay::OUT_RATE });
            println!("disc_samples={} disc_peak={} audio_samples={} audio_peak={} voice={} decoder_status={:?}",
                r.disc.len(), hsrec::replay::peak(&r.disc), r.audio.len(), hsrec::replay::peak(&r.audio),
                hsrec::replay::peak(&r.audio) > hsrec::replay::VOICE_PEAK, r.decoder_status);
            if !r.decoder_stderr.is_empty() {
                println!("decoder stderr (tail):\n{}", r.decoder_stderr);
            }
            0
        }
        Err(e) => {
            eprintln!("commander-rec: render: {e}");
            1
        }
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("render-wav") {
        return ExitCode::from(render_wav(&argv[1..]));
    }
    let a = match parse(&argv) {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("commander-rec: {e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(if e.is_empty() { 0 } else { 1 });
        }
    };
    ExitCode::from(run(a))
}

fn run(a: Args) -> u8 {
    let Some(station) = a.station.clone() else {
        eprintln!("commander-rec: --station PATH is required");
        return 1;
    };
    if a.nice {
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS, 0, 19);
            libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13);
        }
    }
    let db_path = hsrec::db::db_path_beside(&station);
    let dir = storage::recordings_dir_beside(&station);
    // A missing database is created quietly, but only beside a real station.
    if !db_path.exists() && !station.exists() {
        eprintln!("commander-rec: no station file at {}", station.display());
        return 1;
    }
    let db = match Db::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("commander-rec: {e}");
            return 1;
        }
    };
    let rows = db.all().unwrap_or_default();
    let now = chrono::Utc::now();
    let pick = |rows: &[hsrec::db::Recording]| -> Vec<hsrec::db::Recording> {
        match a.older {
            Some(d) => archive::older_than(rows, d, now).into_iter().cloned().collect(),
            None => rows.iter().filter(|r| !r.offline() && !r.missing && !r.meta.end_utc.is_empty()).cloned().collect(),
        }
    };
    let needs_lock = matches!(a.cmd.as_str(), "archive" | "unarchive" | "verify" | "export") && !a.dry;
    let _lock = if needs_lock {
        match lock::try_lock(&db_path) {
            Ok(Some(l)) => Some(l),
            Ok(None) => {
                eprintln!("commander-rec: another archive/export run holds {}", lock::lock_path(&db_path).display());
                return 3;
            }
            Err(e) => {
                eprintln!("commander-rec: lock: {e}");
                return 1;
            }
        }
    } else {
        None
    };
    let cancel = AtomicBool::new(false);
    match a.cmd.as_str() {
        "list" => {
            for r in rows.iter().filter(|r| a.source.as_deref().is_none_or(|s| r.meta.source == s)) {
                println!(
                    "{:>6}  {}  {:>7.1}s  {:<16} {:<24} {:<12} {}{}{}",
                    r.id,
                    r.meta.start_utc,
                    r.meta.duration_ms as f64 / 1000.0,
                    r.meta.source,
                    r.meta.lane(),
                    r.meta.source_kind,
                    if r.archived { "archived " } else { "" },
                    if r.protected { "protected " } else { "" },
                    if r.offline() { r.location.as_str() } else if r.missing { "MISSING" } else { "" },
                );
            }
            0
        }
        "stats" => {
            let u = storage::Usage::from_rows(&rows, a.cap_gb);
            println!("usage: {}", u.text());
            let mut by: std::collections::BTreeMap<String, (usize, i64)> = Default::default();
            for r in &rows {
                let e = by.entry(r.meta.source.clone()).or_default();
                e.0 += 1;
                e.1 += r.disk_bytes();
            }
            for (s, (n, b)) in by {
                println!("{s:<18} {n:>6} recordings  {:>8.2} GB", b as f64 / 1e9);
            }
            println!("archived: {}  offline: {}  missing: {}",
                rows.iter().filter(|r| r.archived).count(),
                rows.iter().filter(|r| r.offline()).count(),
                rows.iter().filter(|r| r.missing).count());
            0
        }
        "archive" => {
            let todo: Vec<_> = pick(&rows).into_iter().filter(|r| !r.archived).collect();
            let mut failed = 0;
            for r in &todo {
                if a.dry {
                    println!("would archive {} ({} bytes)", r.raw_path, r.meta.bytes);
                    continue;
                }
                match archive::archive_one(&db, &dir, r, a.level, &cancel) {
                    archive::Outcome::Archived { saved } => println!("archived {} (saved {saved} bytes)", r.raw_path),
                    archive::Outcome::Skipped(why) => println!("skipped {}: {why}", r.raw_path),
                    archive::Outcome::Failed(e) => {
                        eprintln!("FAILED {e}");
                        failed += 1;
                    }
                    archive::Outcome::Cancelled => return 2,
                }
            }
            if failed > 0 { 2 } else { 0 }
        }
        "unarchive" => {
            let todo: Vec<_> = rows.iter().filter(|r| r.archived && !r.offline()).filter(|r| {
                a.older.is_none_or(|d| archive::older_than(&[(*r).clone()].map(|mut x| { x.archived = false; x }), d, now).len() == 1)
            }).collect();
            let mut failed = 0;
            for r in todo {
                if a.dry {
                    println!("would unarchive {}", r.raw_path);
                    continue;
                }
                match archive::unarchive_one(&db, &dir, r) {
                    Ok(()) => println!("unarchived {}", r.raw_path),
                    Err(e) => {
                        eprintln!("FAILED {e}");
                        failed += 1;
                    }
                }
            }
            if failed > 0 { 2 } else { 0 }
        }
        "verify" => {
            let mut bad = 0;
            for r in rows.iter().filter(|r| !a.archived_only || r.archived) {
                if r.offline() {
                    continue;
                }
                match archive::verify_one(&db, &dir, r) {
                    Ok(()) => println!("ok     {}", r.raw_path),
                    Err(e) => {
                        println!("BAD    {e}");
                        bad += 1;
                    }
                }
            }
            if bad > 0 { 2 } else { 0 }
        }
        "export" => {
            let Some(to) = a.to.clone() else {
                eprintln!("commander-rec: export needs --to DIR");
                return 1;
            };
            let label = a.label.clone().unwrap_or_else(|| {
                to.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "drive".into())
            });
            let todo = pick(&rows);
            if a.dry {
                for r in &todo {
                    println!("would {} {} -> {}", if a.mv { "move" } else { "copy" }, r.raw_path, to.join(export::EXPORT_DIR).join(&r.raw_path).display());
                }
                return 0;
            }
            let rep = export::export(&db, &dir, &todo, &to, &label, a.mv, &cancel, |n, of| {
                if of > 0 && n < of {
                    eprintln!("[{}/{of}]", n + 1);
                }
            });
            for f in &rep.failed {
                eprintln!("FAILED {f}");
            }
            println!("copied {} moved {} failed {}", rep.copied, rep.moved, rep.failed.len());
            println!("sync done: you may remove the drive.");
            if rep.failed.is_empty() { 0 } else { 2 }
        }
        other => {
            eprintln!("commander-rec: unknown command {other}\n{USAGE}");
            1
        }
    }
}
