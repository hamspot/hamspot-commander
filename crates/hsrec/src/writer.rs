//! Background recorder. Runs on its own low-priority thread; the HackRF
//! worker hands it channelised IQ with `try_send` (never blocks live decode;
//! a full queue drops the chunk and counts it), and the UI opens/closes
//! segments. Writes are buffered sequential chunks, no per-chunk fsync, and
//! DB writes happen only at segment open/close.
use crate::db::Db;
use crate::meta::Meta;
use crate::storage::{self, CapDecision};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PAD_MS: u32 = 250;
pub const WRITE_BUF: usize = 128 * 1024;
pub const QUEUE: usize = 512;
/// Check the cap roughly every this many bytes.
const CAP_CHECK_BYTES: i64 = 4 * 1024 * 1024;

pub struct Chunk {
    pub key: u64,
    /// Frequency the samples are centred on; a change resets the pre-roll.
    pub hz: u64,
    /// Interleaved I,Q (raw IQ) or mono samples (demod audio).
    pub data: Vec<i16>,
}

pub enum Cmd {
    Configure { rec_dir: PathBuf, db_path: PathBuf },
    Cap { gb: f64, overwrite_oldest: bool },
    Open { key: u64, meta: Meta },
    /// Close after the post-roll. `keep = false` deletes it (e.g. encrypted).
    Close { key: u64, keep: bool },
    Shutdown,
}

#[derive(Default)]
pub struct Status {
    /// A segment is being written right now (bright red icon).
    pub writing: AtomicBool,
    pub cap_hit: AtomicBool,
    pub dropped: AtomicU64,
    /// Bumped whenever rows change, so the UI can re-query.
    pub generation: AtomicU64,
    pub log: Mutex<Vec<String>>,
    /// The writer has an open database and folder and can record.
    pub ready: AtomicBool,
    /// Why the writer cannot record (empty when fine).
    pub error: Mutex<String>,
}

impl Status {
    pub fn note(&self, line: impl Into<String>) {
        let mut log = self.log.lock().expect("rec log");
        log.push(line.into());
        let n = log.len();
        if n > 200 {
            log.drain(..n - 200);
        }
    }
    pub fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.lock().expect("rec log"))
    }
}

#[derive(Clone)]
pub struct Recorder {
    samples: SyncSender<Chunk>,
    control: Sender<Cmd>,
    pub status: Arc<Status>,
}

impl Recorder {
    pub fn spawn() -> Self {
        let (stx, srx) = mpsc::sync_channel(QUEUE);
        let (ctx, crx) = mpsc::channel();
        let status = Arc::new(Status::default());
        let st = Arc::clone(&status);
        std::thread::Builder::new()
            .name("rec-writer".into())
            .spawn(move || {
                lower_priority();
                Worker::new(st).run(srx, crx)
            })
            .expect("rec writer thread");
        Self { samples: stx, control: ctx, status }
    }

    /// Never blocks. A full queue drops the chunk and counts it.
    pub fn push(&self, chunk: Chunk) {
        let n = chunk.data.len() as u64;
        match self.samples.try_send(chunk) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.status.dropped.fetch_add(n, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.control.send(cmd);
    }
}

/// nice 19 and idle I/O class for this thread only (Linux).
fn lower_priority() {
    #[cfg(target_os = "linux")]
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 19);
        // IOPRIO_WHO_PROCESS = 1, class idle = 3 << 13.
        libc::syscall(libc::SYS_ioprio_set, 1, tid as libc::c_int, 3 << 13);
    }
}

struct Seg {
    id: i64,
    meta: Meta,
    raw: PathBuf,
    out: std::io::BufWriter<std::fs::File>,
    started: chrono::DateTime<chrono::Utc>,
    values: u64,
    since_cap: i64,
    /// Values still to write after Close (post-roll), then finish.
    closing: Option<(u64, bool, Instant)>,
}

pub(crate) struct Worker {
    status: Arc<Status>,
    rec_dir: Option<PathBuf>,
    db: Option<Db>,
    cap_gb: f64,
    overwrite: bool,
    pre: HashMap<u64, (u64, VecDeque<i16>)>,
    segs: HashMap<u64, Seg>,
}

fn values_per_sec(meta: &Meta) -> u64 {
    let ch = if meta.raw_format.starts_with("cs") { 2 } else { 1 };
    meta.sample_rate as u64 * ch
}

impl Worker {
    pub(crate) fn new(status: Arc<Status>) -> Self {
        Self {
            status,
            rec_dir: None,
            db: None,
            cap_gb: storage::DEFAULT_CAP_GB,
            overwrite: false,
            pre: HashMap::new(),
            segs: HashMap::new(),
        }
    }

    fn run(mut self, samples: Receiver<Chunk>, control: Receiver<Cmd>) {
        loop {
            loop {
                match control.try_recv() {
                    Ok(Cmd::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => {
                        self.finish_all();
                        return;
                    }
                    Ok(cmd) => self.command(cmd),
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            match samples.recv_timeout(Duration::from_millis(50)) {
                Ok(chunk) => self.chunk(chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.finish_all();
                    return;
                }
            }
            self.tick();
        }
    }

    pub(crate) fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Configure { rec_dir, db_path } => {
                self.finish_all();
                match Db::open(&db_path) {
                    Ok(db) => {
                        match recover(&db, &rec_dir) {
                            Ok(n) if n > 0 => self.status.note(format!("REC: recovered {n} unfinished recording(s)")),
                            Ok(_) => {}
                            Err(e) => self.status.note(format!("REC: recovery failed: {e}")),
                        }
                        if let Err(e) = std::fs::create_dir_all(&rec_dir) {
                            self.status.ready.store(false, Ordering::Relaxed);
                            *self.status.error.lock().expect("rec error") = format!("cannot create the recordings folder: {e}");
                            return;
                        }
                        self.db = Some(db);
                        self.rec_dir = Some(rec_dir);
                        self.status.ready.store(true, Ordering::Relaxed);
                        self.status.error.lock().expect("rec error").clear();
                        self.status.generation.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        self.status.ready.store(false, Ordering::Relaxed);
                        *self.status.error.lock().expect("rec error") = format!("cannot open the recordings database: {e}");
                        self.status.note(format!("REC: cannot open database: {e}"));
                    }
                }
            }
            Cmd::Cap { gb, overwrite_oldest } => {
                self.cap_gb = gb;
                self.overwrite = overwrite_oldest;
                self.status.cap_hit.store(false, Ordering::Relaxed);
            }
            Cmd::Open { key, meta } => {
                if let Some(seg) = self.segs.get_mut(&key) {
                    // Re-key inside the post-roll continues the same segment.
                    if seg.closing.is_some() && same_call(&seg.meta, &meta) {
                        seg.closing = None;
                        return;
                    }
                    self.finish(key);
                }
                self.open(key, meta, 0);
            }
            Cmd::Close { key, keep } => {
                if let Some(seg) = self.segs.get_mut(&key) {
                    let post = values_per_sec(&seg.meta) * PAD_MS as u64 / 1000;
                    if keep {
                        seg.closing = Some((post, true, Instant::now()));
                    } else {
                        seg.closing = Some((0, false, Instant::now()));
                        self.finish(key);
                    }
                }
            }
            Cmd::Shutdown => self.finish_all(),
        }
        self.update_writing();
    }

    fn open(&mut self, key: u64, mut meta: Meta, part: u32) {
        if self.rec_dir.is_none() || self.db.is_none() {
            self.status.note("REC: no station open; recording skipped");
            return;
        }
        if !self.check_cap(0) {
            return;
        }
        let dir = self.rec_dir.clone().unwrap_or_default();
        let Some(db) = self.db.as_ref() else { return };
        let now = chrono::Utc::now();
        meta.part = part;
        if part == 0 {
            // Start time includes the pre-roll.
            let pre_values = self.pre.get(&key).map(|(_, q)| q.len() as u64).unwrap_or(0);
            let ms = pre_values * 1000 / values_per_sec(&meta).max(1);
            meta.start_utc = (now - chrono::Duration::milliseconds(ms as i64)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        } else {
            meta.start_utc = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        }
        let started = chrono::DateTime::parse_from_rfc3339(&meta.start_utc).map(|t| t.with_timezone(&chrono::Utc)).unwrap_or(now);
        let ext = if meta.raw_format.starts_with("cs") { meta.raw_format.clone() } else { "s16".into() };
        let mut stem = storage::new_stem(&meta.source, started, &meta.lane(), meta.channel_hz, part);
        let mut n = 1;
        while dir.join(format!("{stem}.{ext}")).exists() {
            stem = format!("{}_{n}", storage::new_stem(&meta.source, started, &meta.lane(), meta.channel_hz, part));
            n += 1;
        }
        let rel = format!("{stem}.{ext}");
        let raw = dir.join(&rel);
        if let Some(parent) = raw.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = match std::fs::File::create(&raw) {
            Ok(f) => f,
            Err(e) => {
                self.status.note(format!("REC: cannot create {}: {e}", raw.display()));
                return;
            }
        };
        let _ = std::fs::write(storage::sidecar_for(&raw), serde_json::to_vec_pretty(&meta).unwrap_or_default());
        let id = match db.insert(&rel, &meta) {
            Ok(id) => id,
            Err(e) => {
                self.status.note(format!("REC: database insert failed: {e}"));
                let _ = std::fs::remove_file(&raw);
                return;
            }
        };
        let mut seg = Seg {
            id,
            meta,
            raw,
            out: std::io::BufWriter::with_capacity(WRITE_BUF, file),
            started,
            values: 0,
            since_cap: 0,
            closing: None,
        };
        if part == 0 {
            if let Some((_, pre)) = self.pre.remove(&key) {
                let (a, b) = pre.as_slices();
                let _ = write_i16(&mut seg.out, a);
                let _ = write_i16(&mut seg.out, b);
                seg.values += pre.len() as u64;
            }
        }
        self.segs.insert(key, seg);
        self.status.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// True when recording may continue.
    fn check_cap(&mut self, need: i64) -> bool {
        let Some(db) = self.db.as_ref() else { return false };
        let rows = db.all().unwrap_or_default();
        match storage::cap_decision(&rows, self.cap_gb, need, self.overwrite) {
            CapDecision::Fits => true,
            CapDecision::Overwrite(ids) => {
                let dir = self.rec_dir.clone().unwrap_or_default();
                for id in ids {
                    if let Some(rec) = rows.iter().find(|r| r.id == id) {
                        storage::remove_files(&dir, &rec.raw_path);
                        let _ = db.delete(id);
                        self.status.note(format!("REC: overwrite-oldest removed {}", rec.raw_path));
                    }
                }
                self.status.generation.fetch_add(1, Ordering::Relaxed);
                true
            }
            CapDecision::Stop => {
                if !self.status.cap_hit.swap(true, Ordering::Relaxed) {
                    self.status.note("REC: ⚠ DISK CAP REACHED. Recording stopped; nothing was deleted. Archive, export, or raise the cap.");
                }
                false
            }
        }
    }

    pub(crate) fn chunk(&mut self, chunk: Chunk) {
        if self.segs.contains_key(&chunk.key) {
            let key = chunk.key;
            let dropped = self.status.dropped.swap(0, Ordering::Relaxed);
            let (done, split, check, used) = {
                let seg = self.segs.get_mut(&key).expect("seg");
                let mut data: &[i16] = &chunk.data;
                if let Some((left, _, _)) = seg.closing.as_mut() {
                    let take = (*left as usize).min(data.len());
                    data = &data[..take];
                    *left -= take as u64;
                }
                seg.meta.dropped += dropped;
                if write_i16(&mut seg.out, data).is_err() {
                    self.status.note(format!("REC: write failed for {}", seg.raw.display()));
                }
                seg.values += data.len() as u64;
                seg.since_cap += data.len() as i64 * 2;
                let done = matches!(seg.closing, Some((0, _, _)));
                let split = seg.values * 1000 / values_per_sec(&seg.meta).max(1) >= storage::SPLIT_MS as u64;
                let check = seg.since_cap >= CAP_CHECK_BYTES;
                if check {
                    seg.since_cap = 0;
                }
                (done, split, check, seg.values as i64 * 2)
            };
            if done {
                self.finish(key);
            } else if split {
                let meta = self.segs[&key].meta.clone();
                let part = meta.part + 1;
                self.finish(key);
                self.open(key, meta, part);
            } else if check && !self.check_cap(used) {
                self.finish(key);
            }
            self.update_writing();
            return;
        }
        // Not recording: keep the pre-roll.
        let per_sec = 2 * crate::dsp::REC_RATE as u64;
        let cap = (per_sec * PAD_MS as u64 / 1000) as usize;
        let entry = self.pre.entry(chunk.key).or_insert_with(|| (chunk.hz, VecDeque::new()));
        if entry.0 != chunk.hz {
            entry.0 = chunk.hz;
            entry.1.clear();
        }
        entry.1.extend(chunk.data.iter().copied());
        while entry.1.len() > cap {
            entry.1.pop_front();
        }
    }

    pub(crate) fn tick(&mut self) {
        // A closing segment whose samples stopped arriving finishes anyway.
        let stale: Vec<u64> = self
            .segs
            .iter()
            .filter(|(_, s)| s.closing.is_some_and(|(_, _, at)| at.elapsed() > Duration::from_secs(1)))
            .map(|(k, _)| *k)
            .collect();
        for key in stale {
            self.finish(key);
        }
        self.update_writing();
    }

    fn update_writing(&self) {
        self.status.writing.store(!self.segs.is_empty(), Ordering::Relaxed);
    }

    fn finish(&mut self, key: u64) {
        let Some(mut seg) = self.segs.remove(&key) else { return };
        let keep = seg.closing.map(|(_, k, _)| k).unwrap_or(true);
        let _ = seg.out.flush();
        drop(seg.out);
        let db = self.db.as_ref();
        if !keep {
            let _ = std::fs::remove_file(&seg.raw);
            let _ = std::fs::remove_file(storage::sidecar_for(&seg.raw));
            if let Some(db) = db {
                let _ = db.delete(seg.id);
            }
            self.status.generation.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let ms = seg.values * 1000 / values_per_sec(&seg.meta).max(1);
        seg.meta.duration_ms = ms as i64;
        seg.meta.bytes = seg.values as i64 * 2;
        seg.meta.end_utc = (seg.started + chrono::Duration::milliseconds(ms as i64))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let _ = std::fs::write(storage::sidecar_for(&seg.raw), serde_json::to_vec_pretty(&seg.meta).unwrap_or_default());
        if let Some(db) = db {
            let _ = db.finish(seg.id, &seg.meta);
        }
        if seg.meta.dropped > 0 {
            self.status.note(format!("REC: ⚠ {} samples dropped (disk slow) in {}", seg.meta.dropped, seg.raw.display()));
        }
        self.status.generation.fetch_add(1, Ordering::Relaxed);
    }

    fn finish_all(&mut self) {
        let keys: Vec<u64> = self.segs.keys().copied().collect();
        for k in keys {
            self.finish(k);
        }
        self.update_writing();
    }
}

fn same_call(a: &Meta, b: &Meta) -> bool {
    a.source == b.source && a.talkgroup == b.talkgroup && a.channel_name == b.channel_name
        && (a.channel_hz - b.channel_hz).abs() < 1.0
}

fn write_i16(out: &mut impl Write, data: &[i16]) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(data.len() * 2);
    for v in data {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    out.write_all(&bytes)
}

/// Rows left without an end (crash, power loss): fill end/duration/bytes
/// from the file on disk. Returns how many were fixed.
pub fn recover(db: &Db, dir: &Path) -> rusqlite::Result<usize> {
    let mut n = 0;
    for rec in db.all()? {
        if !rec.meta.end_utc.is_empty() {
            continue;
        }
        let raw = dir.join(&rec.raw_path);
        let bytes = std::fs::metadata(&raw).map(|m| m.len() as i64).unwrap_or(0);
        let mut meta = rec.meta.clone();
        let ms = (bytes / 2) as u64 * 1000 / values_per_sec(&meta).max(1);
        meta.bytes = bytes;
        meta.duration_ms = ms as i64;
        let start = chrono::DateTime::parse_from_rfc3339(&meta.start_utc)
            .map(|t| t.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());
        meta.end_utc = (start + chrono::Duration::milliseconds(ms as i64)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        db.finish(rec.id, &meta)?;
        let _ = std::fs::write(storage::sidecar_for(&raw), serde_json::to_vec_pretty(&meta).unwrap_or_default());
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::{meta, tempdir};

    fn worker(dir: &Path) -> Worker {
        let mut w = Worker::new(Arc::new(Status::default()));
        w.command(Cmd::Configure { rec_dir: dir.join("S.recordings"), db_path: dir.join("S.HSREC.sqlite") });
        w
    }

    fn m() -> Meta {
        let mut x = meta("hackrf_tg", Some(7), "", "");
        x.channel_hz = 851e6;
        x
    }

    #[test]
    fn missing_database_is_created_and_recording_starts() {
        let dir = tempdir("writer-autocreate").join("new").join("station");
        let mut w = worker(&dir);
        assert!(w.status.ready.load(Ordering::Relaxed));
        assert!(w.status.error.lock().unwrap().is_empty());
        assert!(dir.join("S.HSREC.sqlite").exists());
        w.command(Cmd::Open { key: 1, meta: m() });
        assert!(w.status.writing.load(Ordering::Relaxed));
        w.chunk(Chunk { key: 1, hz: 851, data: vec![2; 64_000] });
        w.command(Cmd::Close { key: 1, keep: true });
        assert_eq!(w.db.as_ref().unwrap().all().unwrap().len(), 1);
    }

    #[test]
    fn corrupt_database_is_reported_and_nothing_records() {
        let dir = tempdir("writer-corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("S.HSREC.sqlite"), vec![0x5au8; 8192]).unwrap();
        let mut w = worker(&dir);
        assert!(!w.status.ready.load(Ordering::Relaxed));
        assert!(w.status.error.lock().unwrap().contains("recordings database"));
        w.command(Cmd::Open { key: 1, meta: m() });
        assert!(!w.status.writing.load(Ordering::Relaxed));
        assert_eq!(std::fs::read(dir.join("S.HSREC.sqlite")).unwrap(), vec![0x5au8; 8192]);
    }

    #[test]
    fn preroll_segment_postroll() {
        let dir = tempdir("writer1");
        let mut w = worker(&dir);
        // 1 s of pre-roll at 32 ksps: only the last 250 ms is kept.
        w.chunk(Chunk { key: 1, hz: 851, data: vec![1; 64_000] });
        w.command(Cmd::Open { key: 1, meta: m() });
        assert!(w.status.writing.load(Ordering::Relaxed));
        w.chunk(Chunk { key: 1, hz: 851, data: vec![2; 64_000] });
        w.command(Cmd::Close { key: 1, keep: true });
        w.chunk(Chunk { key: 1, hz: 851, data: vec![3; 64_000] });
        assert!(!w.status.writing.load(Ordering::Relaxed));
        let db = w.db.as_ref().unwrap();
        let rows = db.all().unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        // 250 ms pre + 1 s + 250 ms post = 1.5 s.
        assert_eq!(r.meta.duration_ms, 1500);
        assert_eq!(r.meta.bytes, 96_000 * 2);
        let raw = std::fs::read(dir.join("S.recordings").join(&r.raw_path)).unwrap();
        assert_eq!(raw.len(), 192_000);
        assert!(dir.join("S.recordings").join(&r.raw_path).with_extension("json").exists());
    }

    #[test]
    fn rekey_in_postroll_continues() {
        let dir = tempdir("writer2");
        let mut w = worker(&dir);
        w.command(Cmd::Open { key: 1, meta: m() });
        w.chunk(Chunk { key: 1, hz: 851, data: vec![2; 6_400] });
        w.command(Cmd::Close { key: 1, keep: true });
        w.command(Cmd::Open { key: 1, meta: m() });
        w.chunk(Chunk { key: 1, hz: 851, data: vec![2; 64_000] });
        w.command(Cmd::Close { key: 1, keep: true });
        w.chunk(Chunk { key: 1, hz: 851, data: vec![2; 64_000] });
        assert_eq!(w.db.as_ref().unwrap().all().unwrap().len(), 1);
    }

    #[test]
    fn encrypted_close_deletes() {
        let dir = tempdir("writer3");
        let mut w = worker(&dir);
        w.command(Cmd::Open { key: 1, meta: m() });
        w.chunk(Chunk { key: 1, hz: 851, data: vec![2; 6_400] });
        w.command(Cmd::Close { key: 1, keep: false });
        assert!(w.db.as_ref().unwrap().all().unwrap().is_empty());
        let mut files = Vec::new();
        fn walk(p: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(p).unwrap().flatten() {
                if e.path().is_dir() { walk(&e.path(), out) } else { out.push(e.path()) }
            }
        }
        walk(&dir.join("S.recordings"), &mut files);
        assert!(files.is_empty(), "{files:?}");
    }

    #[test]
    fn stops_at_cap_and_never_deletes() {
        let dir = tempdir("writer4");
        let mut w = worker(&dir);
        w.command(Cmd::Cap { gb: 0.000_1, overwrite_oldest: false }); // 100 kB
        w.command(Cmd::Open { key: 1, meta: m() });
        w.chunk(Chunk { key: 1, hz: 851, data: vec![2; 64_000] });
        w.command(Cmd::Close { key: 1, keep: true });
        w.tick();
        std::thread::sleep(Duration::from_millis(1100));
        w.tick();
        w.command(Cmd::Open { key: 1, meta: m() });
        assert!(!w.status.writing.load(Ordering::Relaxed));
        assert!(w.status.cap_hit.load(Ordering::Relaxed));
        assert_eq!(w.db.as_ref().unwrap().all().unwrap().len(), 1);
        assert!(w.status.take_log().iter().any(|l| l.contains("DISK CAP")));
    }

    #[test]
    fn recovers_unfinished() {
        let dir = tempdir("writer5");
        let db = Db::open(&dir.join("S.HSREC.sqlite")).unwrap();
        let rd = dir.join("S.recordings");
        std::fs::create_dir_all(rd.join("a")).unwrap();
        std::fs::write(rd.join("a/x.cs16"), vec![0u8; 128_000]).unwrap();
        let mut mm = m();
        mm.start_utc = "2026-01-01T00:00:00.000Z".into();
        db.insert("a/x.cs16", &mm).unwrap();
        assert_eq!(recover(&db, &rd).unwrap(), 1);
        let r = &db.all().unwrap()[0];
        assert_eq!(r.meta.duration_ms, 1000);
        assert_eq!(r.meta.end_utc, "2026-01-01T00:00:01.000Z");
    }

    #[test]
    fn synthetic_fm_record_db_replay_round_trip() {
        let dir = tempdir("writer6");
        let mut w = worker(&dir);
        let cs8 = crate::dsp::synth_fm_cs8(-40_000.0, 1_200.0, 2_500.0, 0.5, 100.0);
        let mut ch = crate::dsp::Channelizer::new(-40_000.0);
        let mut m = m();
        m.modulation = "fm".into();
        w.command(Cmd::Open { key: 3, meta: m });
        for block in cs8.chunks(16_384) {
            let mut iq = Vec::new();
            for p in block.chunks_exact(2) {
                ch.push(p[0] as i8 as f32, p[1] as i8 as f32, &mut iq);
            }
            w.chunk(Chunk { key: 3, hz: 1, data: iq });
        }
        w.command(Cmd::Close { key: 3, keep: true });
        w.tick();
        std::thread::sleep(Duration::from_millis(1100));
        w.tick();
        let db = w.db.as_ref().unwrap();
        let rec = db.all().unwrap().pop().unwrap();
        assert_eq!(rec.meta.duration_ms, 500);
        let path = dir.join("S.recordings").join(&rec.raw_path);
        let audio = crate::replay::decode_all(&path, &rec.meta).unwrap();
        let hz = crate::dsp::zero_cross_hz(&audio[2_400..], 48_000.0);
        assert!((hz - 1_200.0).abs() < 30.0, "{hz}");
    }

    #[test]
    fn full_queue_drops_and_counts() {
        let (stx, _srx) = mpsc::sync_channel(1);
        let (ctx, _crx) = mpsc::channel();
        let r = Recorder { samples: stx, control: ctx, status: Arc::new(Status::default()) };
        r.push(Chunk { key: 1, hz: 1, data: vec![0; 10] });
        r.push(Chunk { key: 1, hz: 1, data: vec![0; 10] });
        assert_eq!(r.status.dropped.load(Ordering::Relaxed), 10);
    }
}
