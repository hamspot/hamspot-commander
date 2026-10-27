//! Replay: every playback re-runs the raw IQ through the current
//! discriminator (and decoder for digital modes). Saved audio is never read
//! for raw_iq recordings. Archived `.zst` files decompress on the fly.
use crate::dsp::Replayer;
use crate::meta::{Meta, KIND_DEMOD_AUDIO};
use std::io::{BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const OUT_RATE: u32 = 48_000;

pub fn open_raw(path: &Path) -> std::io::Result<Box<dyn Read + Send>> {
    let f = BufReader::with_capacity(256 * 1024, std::fs::File::open(path)?);
    if path.extension().is_some_and(|e| e == "zst") {
        Ok(Box::new(zstd::stream::read::Decoder::with_buffer(f)?))
    } else {
        Ok(Box::new(f))
    }
}

/// Raw file to the 48 kHz s16 stream the live decoders and speaker read.
pub struct Stream {
    reader: Box<dyn Read + Send>,
    replayer: Replayer,
    raw: bool,
    eight_bit: bool,
    buf: Vec<u8>,
}

impl Stream {
    pub fn open(path: &Path, meta: &Meta) -> std::io::Result<Self> {
        Ok(Self {
            reader: open_raw(path)?,
            replayer: Replayer::default(),
            raw: meta.source_kind != KIND_DEMOD_AUDIO,
            eight_bit: meta.raw_format == "cs8",
            buf: vec![0u8; 64 * 1024],
        })
    }

    /// Skip `ms` of input (seek). Returns false at end of file.
    pub fn skip_ms(&mut self, ms: u64, meta: &Meta) -> std::io::Result<bool> {
        let ch = if self.raw { 2 } else { 1 };
        let width = if self.eight_bit { 1 } else { 2 };
        let mut left = meta.sample_rate as u64 * ms / 1000 * ch * width;
        while left > 0 {
            let want = (left as usize).min(self.buf.len());
            let n = self.reader.read(&mut self.buf[..want])?;
            if n == 0 {
                return Ok(false);
            }
            left -= n as u64;
        }
        Ok(true)
    }

    /// Next block of 48 kHz audio; empty at end.
    pub fn next(&mut self, out: &mut Vec<i16>) -> std::io::Result<()> {
        out.clear();
        let n = read_some(&mut self.reader, &mut self.buf)?;
        if n == 0 {
            return Ok(());
        }
        let n = n - n % 4;
        let samples: Vec<i16> = if self.eight_bit {
            self.buf[..n].iter().map(|b| (*b as i8 as i16) * 256).collect()
        } else {
            self.buf[..n].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
        };
        if self.raw {
            self.replayer.push(&samples, out);
        } else {
            out.extend_from_slice(&samples);
        }
        Ok(())
    }
}

fn read_some(r: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
    // Fill a multiple of 4 bytes so IQ pairs never split.
    let mut got = 0;
    while got < buf.len() {
        let n = r.read(&mut buf[got..])?;
        if n == 0 {
            break;
        }
        got += n;
        if got % 4 == 0 && got >= 4096 {
            break;
        }
    }
    Ok(got)
}

/// Faster-than-realtime decode of the whole file (thumbnails, WAV export).
pub fn decode_all(path: &Path, meta: &Meta) -> std::io::Result<Vec<i16>> {
    let mut st = Stream::open(path, meta)?;
    let mut all = Vec::new();
    let mut block = Vec::new();
    loop {
        st.next(&mut block)?;
        if block.is_empty() {
            return Ok(all);
        }
        all.extend_from_slice(&block);
    }
}

/// Peak loudness per bucket, 0..=255 (waveform thumbnail).
pub fn waveform(samples: &[i16], buckets: usize) -> Vec<u8> {
    if samples.is_empty() || buckets == 0 {
        return Vec::new();
    }
    let per = samples.len().div_ceil(buckets).max(1);
    samples
        .chunks(per)
        .map(|c| {
            let mean = c.iter().map(|s| *s as i64).sum::<i64>() / c.len() as i64;
            let peak = c.iter().map(|s| (*s as i64 - mean).unsigned_abs()).max().unwrap_or(0);
            (peak * 255 / 32768).min(255) as u8
        })
        .collect()
}

pub fn write_wav(path: &Path, samples: &[i16], rate: u32) -> std::io::Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let data = (samples.len() * 2) as u32;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data.to_le_bytes())?;
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()
}

/// A digital decoder that reads the 48 kHz discriminator stream on stdin
/// and writes 48 kHz s16 mono speech on stdout (the station's voice script).
#[derive(Clone, Debug, Default)]
pub struct DecoderCmd {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// pw-cat to the system default output.
pub fn default_play_argv() -> Vec<String> {
    ["pw-cat", "-p", "--rate", "48000", "--channels", "1", "--format", "s16", "-"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Result of an offline render of one recording.
#[derive(Debug, Default)]
pub struct Render {
    /// 48 kHz discriminator samples fed to the decoder (or played for analog).
    pub disc: Vec<i16>,
    /// Speaker audio: the decoder's output for digital modes, else `disc`.
    pub audio: Vec<i16>,
    /// Decoder exit code, when a decoder ran.
    pub decoder_status: Option<i32>,
    /// Last lines of the decoder's stderr.
    pub decoder_stderr: String,
}

/// Peak-to-mean level a decoded voice stream must exceed to count as voice.
pub const VOICE_PEAK: i64 = 600;

/// Peak of `samples` around their mean (0 for silence).
pub fn peak(samples: &[i16]) -> i64 {
    if samples.is_empty() {
        return 0;
    }
    let mean = samples.iter().map(|s| *s as i64).sum::<i64>() / samples.len() as i64;
    samples.iter().map(|s| (*s as i64 - mean).abs()).max().unwrap_or(0)
}

/// Re-demodulate a recording offline, through the same chain playback uses,
/// and return the audio instead of playing it (`commander-rec render-wav`).
pub fn render(path: &Path, meta: &Meta, decoder: Option<&DecoderCmd>) -> std::io::Result<Render> {
    let disc = decode_all(path, meta)?;
    let digital = meta.is_digital() && meta.source_kind != KIND_DEMOD_AUDIO;
    let Some(cmd) = decoder.filter(|c| digital && !c.argv.is_empty()) else {
        return Ok(Render { audio: disc.clone(), disc, ..Default::default() });
    };
    let mut c = Command::new(&cmd.argv[0]);
    c.args(&cmd.argv[1..])
        .envs(cmd.env.iter().cloned())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = c.spawn()?;
    let mut so = child.stdout.take().ok_or_else(|| std::io::Error::other("decoder stdout"))?;
    let mut se = child.stderr.take().ok_or_else(|| std::io::Error::other("decoder stderr"))?;
    let out = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = so.read_to_end(&mut v);
        v
    });
    let err = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = se.read_to_end(&mut v);
        v
    });
    let mut stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("decoder stdin"))?;
    let feed = disc.clone();
    let writer = std::thread::spawn(move || {
        // Real-time-ish pacing so the UDP voice hop inside the script keeps up,
        // then two seconds of silence to flush the vocoder before EOF.
        let t0 = Instant::now();
        let mut sent = 0usize;
        let tail = vec![0i16; OUT_RATE as usize * 2];
        for chunk in feed.chunks(4800).chain(tail.chunks(4800)) {
            let mut bytes = Vec::with_capacity(chunk.len() * 2);
            for v in chunk {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            if stdin.write_all(&bytes).is_err() {
                break;
            }
            sent += chunk.len();
            // Up to 8x real time.
            let due = Duration::from_micros((sent as u64 * 1_000_000 / OUT_RATE as u64) / 8);
            if let Some(wait) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(wait);
            }
        }
    });
    let _ = writer.join();
    let status = child.wait()?;
    let bytes = out.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&err.join().unwrap_or_default()).to_string();
    let tail: Vec<&str> = stderr.lines().rev().take(12).collect();
    let audio = bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    Ok(Render {
        disc,
        audio,
        decoder_status: status.code(),
        decoder_stderr: tail.into_iter().rev().collect::<Vec<_>>().join("\n"),
    })
}

pub struct Player {
    pub stop: Arc<AtomicBool>,
    pub done: Arc<AtomicBool>,
    /// Playback position in ms from the start of the recording.
    pub pos_ms: Arc<AtomicU64>,
    pub error: Arc<std::sync::Mutex<String>>,
    /// Bytes and peak of what reached the speaker, and the decoder's exit.
    pub heard_bytes: Arc<AtomicU64>,
    pub heard_peak: Arc<AtomicU64>,
    pub decoder_ran: Arc<AtomicBool>,
    pub decoder_status: Arc<std::sync::Mutex<Option<i32>>>,
    pub decoder_stderr: Arc<std::sync::Mutex<String>>,
    pub played_ms: Arc<AtomicU64>,
}

/// What a finished playback produced, for the REC status line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub decoder_ran: bool,
    pub heard_bytes: u64,
    pub heard_peak: u64,
    pub played_ms: u64,
    pub decoder_status: Option<i32>,
    pub decoder_stderr: String,
}

impl Outcome {
    /// Text when a digital replay ran but decoded no voice; None otherwise.
    pub fn no_voice_text(&self) -> Option<String> {
        if !self.decoder_ran || self.heard_peak as i64 > VOICE_PEAK {
            return None;
        }
        let secs = self.played_ms as f64 / 1000.0;
        let mut t = format!(
            "No voice decoded: the replay re-demodulated {secs:.1} s of this recording and the decoder produced {}.",
            if self.heard_bytes == 0 { "no audio".to_string() } else { "only silence".to_string() }
        );
        if let Some(code) = self.decoder_status.filter(|c| *c != 0) {
            t.push_str(&format!(" Decoder exit {code}."));
        }
        if let Some(line) = self.decoder_stderr.lines().rev().find(|l| !l.trim().is_empty() && !l.contains("exiting on signal")) {
            t.push_str(&format!(" Last decoder message: {}", line.trim()));
        }
        Some(t)
    }
}

impl Player {
    pub fn outcome(&self) -> Outcome {
        Outcome {
            decoder_ran: self.decoder_ran.load(Ordering::Relaxed),
            heard_bytes: self.heard_bytes.load(Ordering::Relaxed),
            heard_peak: self.heard_peak.load(Ordering::Relaxed),
            played_ms: self.played_ms.load(Ordering::Relaxed),
            decoder_status: *self.decoder_status.lock().expect("status"),
            decoder_stderr: self.decoder_stderr.lock().expect("stderr").clone(),
        }
    }

    /// Start playback at `from_ms`. Runs in its own process group and never
    /// touches the radio RX sinks.
    pub fn start(path: &Path, meta: &Meta, from_ms: u64, decoder: Option<DecoderCmd>, play_argv: Vec<String>) -> std::io::Result<Self> {
        let mut stream = Stream::open(path, meta)?;
        let meta = meta.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let pos = Arc::new(AtomicU64::new(from_ms));
        let error = Arc::new(std::sync::Mutex::new(String::new()));
        let heard_bytes = Arc::new(AtomicU64::new(0));
        let heard_peak = Arc::new(AtomicU64::new(0));
        let decoder_ran = Arc::new(AtomicBool::new(false));
        let decoder_status = Arc::new(std::sync::Mutex::new(None));
        let decoder_stderr = Arc::new(std::sync::Mutex::new(String::new()));
        let played_ms = Arc::new(AtomicU64::new(0));
        let (hb, hp, dr, ds, dse, pm) = (heard_bytes.clone(), heard_peak.clone(), decoder_ran.clone(), decoder_status.clone(), decoder_stderr.clone(), played_ms.clone());
        let (s, d, p, e) = (stop.clone(), done.clone(), pos.clone(), error.clone());
        std::thread::Builder::new().name("rec-replay".into()).spawn(move || {
            let result = (|| -> std::io::Result<()> {
                if !stream.skip_ms(from_ms, &meta)? {
                    return Ok(());
                }
                let mut player = spawn(&play_argv, Stdio::piped(), None)
                    .map_err(|err| std::io::Error::other(format!("audio output {:?}: {err}", play_argv.first().cloned().unwrap_or_default())))?;
                let mut copier: Option<std::thread::JoinHandle<()>> = None;
                let mut err_reader: Option<std::thread::JoinHandle<String>> = None;
                let mut dec: Option<Child> = None;
                let mut sink: Box<dyn Write> = match &decoder {
                    Some(cmd) if meta.is_digital() && meta.source_kind != KIND_DEMOD_AUDIO && !cmd.argv.is_empty() => {
                        let mut c = Command::new(&cmd.argv[0]);
                        c.args(&cmd.argv[1..])
                            .envs(cmd.env.iter().cloned())
                            .stdin(Stdio::piped())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .process_group(0);
                        let mut child = c.spawn().map_err(|err| std::io::Error::other(format!("decoder {}: {err}", cmd.argv[0])))?;
                        dr.store(true, Ordering::Relaxed);
                        if let Some(mut se) = child.stderr.take() {
                            err_reader = Some(std::thread::spawn(move || {
                                let mut v = Vec::new();
                                let _ = se.read_to_end(&mut v);
                                let text = String::from_utf8_lossy(&v).to_string();
                                let tail: Vec<&str> = text.lines().rev().take(8).collect();
                                tail.into_iter().rev().collect::<Vec<_>>().join("\n")
                            }));
                        }
                        // Copy decoder speech to the player, measuring what is heard.
                        if let (Some(mut so), Some(mut pin)) = (child.stdout.take(), player.stdin.take()) {
                            let (hb, hp) = (hb.clone(), hp.clone());
                            copier = Some(std::thread::spawn(move || {
                                let mut buf = vec![0u8; 8192];
                                loop {
                                    let n = match so.read(&mut buf) { Ok(0) | Err(_) => break, Ok(n) => n };
                                    let samples: Vec<i16> = buf[..n - n % 2].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
                                    hb.fetch_add(n as u64, Ordering::Relaxed);
                                    hp.fetch_max(peak(&samples) as u64, Ordering::Relaxed);
                                    if pin.write_all(&buf[..n]).is_err() { break; }
                                }
                            }));
                        }
                        let w = child.stdin.take().ok_or_else(|| std::io::Error::other("decoder stdin"))?;
                        dec = Some(child);
                        Box::new(w)
                    }
                    _ => Box::new(player.stdin.take().ok_or_else(|| std::io::Error::other("player stdin"))?),
                };
                let t0 = Instant::now();
                let mut sent: u64 = 0;
                let mut block = Vec::new();
                while !s.load(Ordering::Relaxed) {
                    stream.next(&mut block)?;
                    if block.is_empty() {
                        break;
                    }
                    let mut bytes = Vec::with_capacity(block.len() * 2);
                    for v in &block {
                        bytes.extend_from_slice(&v.to_le_bytes());
                    }
                    if sink.write_all(&bytes).is_err() {
                        break;
                    }
                    sent += block.len() as u64;
                    let ahead_ms = sent * 1000 / OUT_RATE as u64;
                    pm.store(ahead_ms, Ordering::Relaxed);
                    p.store(from_ms + ahead_ms, Ordering::Relaxed);
                    // Pace to real time, about 200 ms ahead of the speaker.
                    let el = t0.elapsed().as_millis() as u64;
                    if ahead_ms > el + 200 {
                        std::thread::sleep(Duration::from_millis(ahead_ms - el - 200));
                    }
                }
                drop(sink);
                if let Some(mut c) = dec {
                    if s.load(Ordering::Relaxed) { kill(&mut c) }
                    let st = c.wait().ok().and_then(|st| st.code());
                    *ds.lock().expect("status") = st;
                }
                if let Some(h) = copier { let _ = h.join(); }
                if let Some(h) = err_reader { *dse.lock().expect("stderr") = h.join().unwrap_or_default(); }
                if s.load(Ordering::Relaxed) { kill(&mut player) }
                let _ = player.wait();
                Ok(())
            })();
            if let Err(err) = result {
                *e.lock().expect("replay err") = err.to_string();
            }
            d.store(true, Ordering::Relaxed);
        })?;
        Ok(Self { stop, done, pos_ms: pos, error, heard_bytes, heard_peak, decoder_ran, decoder_status, decoder_stderr, played_ms })
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn finished(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spawn(argv: &[String], stdin: Stdio, stdout: Option<Stdio>) -> std::io::Result<Child> {
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..]).stdin(stdin).stdout(stdout.unwrap_or(Stdio::null())).stderr(Stdio::null()).process_group(0);
    c.spawn()
}

fn kill(child: &mut Child) {
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::dsp::*;
    use crate::meta::*;

    pub fn fm_file(dir: &Path, tone: f64) -> (std::path::PathBuf, Meta) {
        let cs8 = synth_fm_cs8(50_000.0, tone, 2_500.0, 0.3, 100.0);
        let mut ch = Channelizer::new(50_000.0);
        let mut iq = Vec::new();
        for p in cs8.chunks_exact(2) {
            ch.push(p[0] as i8 as f32, p[1] as i8 as f32, &mut iq);
        }
        let path = dir.join("t.cs16");
        let bytes: Vec<u8> = iq.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(&path, bytes).unwrap();
        let meta = Meta { source: SOURCE_HACKRF_CHANNELS.into(), source_kind: KIND_RAW_IQ.into(),
            raw_format: "cs16".into(), sample_rate: REC_RATE, modulation: "fm".into(), ..Default::default() };
        (path, meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::tests_support::fm_file;
    use crate::db::tests::tempdir;
    use crate::dsp::*;
    use crate::meta::*;

    #[test]
    fn synthetic_fm_replays_to_tone() {
        let dir = tempdir("replay1");
        let (path, meta) = fm_file(&dir, 800.0);
        let audio = decode_all(&path, &meta).unwrap();
        assert!(audio.len() > 12_000);
        let hz = zero_cross_hz(&audio[2400..], OUT_RATE as f32);
        assert!((hz - 800.0).abs() < 30.0, "{hz}");
        let wf = waveform(&audio, 50);
        assert_eq!(wf.len(), 50);
        assert!(wf.iter().skip(2).all(|v| *v > 20));
    }

    #[test]
    fn zst_replays_same_samples() {
        let dir = tempdir("replay2");
        let (path, meta) = fm_file(&dir, 600.0);
        let a = decode_all(&path, &meta).unwrap();
        let z = dir.join("t.cs16.zst");
        let data = std::fs::read(&path).unwrap();
        std::fs::write(&z, zstd::encode_all(&data[..], 3).unwrap()).unwrap();
        assert_eq!(decode_all(&z, &meta).unwrap(), a);
    }

    #[test]
    fn seek_skips() {
        let dir = tempdir("replay3");
        let (path, meta) = fm_file(&dir, 600.0);
        let mut st = Stream::open(&path, &meta).unwrap();
        assert!(st.skip_ms(100, &meta).unwrap());
        let mut n = 0;
        let mut b = Vec::new();
        loop { st.next(&mut b).unwrap(); if b.is_empty() { break } n += b.len(); }
        // 0.3 s total minus 0.1 s skipped, at 48 kHz.
        assert!((9_000..10_000).contains(&n), "{n}");
        assert!(!st.skip_ms(1000, &meta).unwrap());
    }

    #[test]
    fn demod_audio_passes_through() {
        let dir = tempdir("replay4");
        let path = dir.join("a.s16");
        let samples: Vec<i16> = (0..4800).map(|i| (i % 100) as i16).collect();
        std::fs::write(&path, samples.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        let meta = Meta { source_kind: KIND_DEMOD_AUDIO.into(), raw_format: "s16le".into(), sample_rate: 48_000, ..Default::default() };
        assert_eq!(decode_all(&path, &meta).unwrap(), samples);
    }

    #[test]
    fn outcome_text() {
        let silent = Outcome { decoder_ran: true, played_ms: 2000, decoder_stderr: "Sync: no sync\nsocat W exiting on signal 15".into(), ..Default::default() };
        let t = silent.no_voice_text().unwrap();
        assert!(t.starts_with("No voice decoded") && t.contains("2.0 s") && t.contains("no audio") && t.contains("Sync: no sync"));
        let voiced = Outcome { decoder_ran: true, heard_bytes: 1000, heard_peak: 9000, ..Default::default() };
        assert_eq!(voiced.no_voice_text(), None);
        assert_eq!(Outcome::default().no_voice_text(), None); // analog: no decoder
        assert_eq!(peak(&[0, 0, 0]), 0);
    }

    #[test]
    fn missing_audio_output_is_an_error() {
        let dir = tempdir("replay-noout");
        let (path, meta) = fm_file(&dir, 1000.0);
        let p = Player::start(&path, &meta, 0, None, vec!["/nonexistent/pw-cat".into()]).unwrap();
        while !p.finished() { std::thread::sleep(Duration::from_millis(10)); }
        assert!(p.error.lock().unwrap().contains("audio output"));
    }

    #[test]
    fn wav_header() {
        let dir = tempdir("replay5");
        let p = dir.join("x.wav");
        write_wav(&p, &[1, 2, 3], 48_000).unwrap();
        let b = std::fs::read(&p).unwrap();
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(b.len(), 44 + 6);
    }
}

#[cfg(test)]
mod live_player {
    use super::*;
    /// Runs the real Player (as the GUI does) on a recording, with the
    /// speaker replaced by a file. `HSREC_RAW=... HSREC_SCRIPT=... cargo test -- --ignored`
    #[test]
    #[ignore]
    fn player_on_real_recording() {
        let (Ok(raw), Ok(script)) = (std::env::var("HSREC_RAW"), std::env::var("HSREC_SCRIPT")) else { return };
        let raw = std::path::PathBuf::from(raw);
        let meta: Meta = serde_json::from_str(&std::fs::read_to_string(raw.with_extension("json")).unwrap()).unwrap();
        let out = std::env::temp_dir().join("hsrec-player-out.raw");
        let play = vec!["sh".into(), "-c".into(), format!("cat > {}", out.display())];
        let dec = DecoderCmd { argv: vec![script, "23998".into()], env: Vec::new() };
        let p = Player::start(&raw, &meta, 0, Some(dec), play).unwrap();
        while !p.finished() {
            std::thread::sleep(Duration::from_millis(100));
        }
        let n = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        eprintln!("player wrote {n} bytes, error={:?}", p.error.lock().unwrap());
    }
}
