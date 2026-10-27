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

pub struct Player {
    pub stop: Arc<AtomicBool>,
    pub done: Arc<AtomicBool>,
    /// Playback position in ms from the start of the recording.
    pub pos_ms: Arc<AtomicU64>,
    pub error: Arc<std::sync::Mutex<String>>,
}

impl Player {
    /// Start playback at `from_ms`. Runs in its own process group and never
    /// touches the radio RX sinks.
    pub fn start(path: &Path, meta: &Meta, from_ms: u64, decoder: Option<DecoderCmd>, play_argv: Vec<String>) -> std::io::Result<Self> {
        let mut stream = Stream::open(path, meta)?;
        let meta = meta.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let pos = Arc::new(AtomicU64::new(from_ms));
        let error = Arc::new(std::sync::Mutex::new(String::new()));
        let (s, d, p, e) = (stop.clone(), done.clone(), pos.clone(), error.clone());
        std::thread::Builder::new().name("rec-replay".into()).spawn(move || {
            let result = (|| -> std::io::Result<()> {
                if !stream.skip_ms(from_ms, &meta)? {
                    return Ok(());
                }
                let mut player = spawn(&play_argv, Stdio::piped(), None)?;
                let mut dec: Option<Child> = None;
                let mut sink: Box<dyn Write> = match &decoder {
                    Some(cmd) if meta.is_digital() && meta.source_kind != KIND_DEMOD_AUDIO && !cmd.argv.is_empty() => {
                        let mut c = Command::new(&cmd.argv[0]);
                        c.args(&cmd.argv[1..])
                            .envs(cmd.env.iter().cloned())
                            .stdin(Stdio::piped())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::null())
                            .process_group(0);
                        let mut child = c.spawn()?;
                        // Copy decoder speech to the player.
                        if let (Some(mut so), Some(mut pin)) = (child.stdout.take(), player.stdin.take()) {
                            std::thread::spawn(move || {
                                let _ = std::io::copy(&mut so, &mut pin);
                            });
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
                    let _ = c.wait();
                }
                if s.load(Ordering::Relaxed) { kill(&mut player) }
                let _ = player.wait();
                Ok(())
            })();
            if let Err(err) = result {
                *e.lock().expect("replay err") = err.to_string();
            }
            d.store(true, Ordering::Relaxed);
        })?;
        Ok(Self { stop, done, pos_ms: pos, error })
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
    fn wav_header() {
        let dir = tempdir("replay5");
        let p = dir.join("x.wav");
        write_wav(&p, &[1, 2, 3], 48_000).unwrap();
        let b = std::fs::read(&p).unwrap();
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(b.len(), 44 + 6);
    }
}
