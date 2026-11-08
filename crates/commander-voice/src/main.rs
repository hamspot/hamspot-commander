//! Voice between a decoder and the speaker.
//!
//! `play` reads the decoder's own signed 16-bit stream and queues it on a
//! PipeWire playback stream. Two slots are summed first. PipeWire resamples
//! to the sink. The 8 kHz to 48 kHz repeat remains only for a FreeDV speech
//! file that still asks for it. FreeDV's modem takes the other direction:
//! 48 kHz receiver audio is low-passed and stepped down to 8 kHz. DRM reads
//! a WAV stream; `--wav` writes the header and then copies the 48 kHz mono
//! samples through.

mod play;

use std::env;
use std::io::{self, Read, Write};
use std::process::ExitCode;

const UP: usize = 6;
/// Low-pass before the 48 kHz -> 8 kHz step. Cutoff 3.5 kHz, Hamming window,
/// DC gain 1. Newest sample is index 0.
const TAPS: [f32; 36] = [
    0.001431618, 0.001608754, 0.001634077, 0.001118938, -0.000446128, -0.003395075,
    -0.007547649, -0.011968816, -0.014954816, -0.014311187, -0.007891438, 0.005733507,
    0.026670475, 0.053365840, 0.082676033, 0.110377284, 0.132014818, 0.143883765,
    0.143883765, 0.132014818, 0.110377284, 0.082676033, 0.053365840, 0.026670475,
    0.005733507, -0.007891438, -0.014311187, -0.014954816, -0.011968816, -0.007547649,
    -0.003395075, -0.000446128, 0.001118938, 0.001634077, 0.001608754, 0.001431618,
];

/// The header sox writes when the WAV is a pipe and it cannot seek back to
/// fix the length. `decdrm` already reads that header from a fifo.
const WAV_HEADER: [u8; 44] = [
    b'R', b'I', b'F', b'F', 0x24, 0xf0, 0xff, 0x7f, b'W', b'A', b'V', b'E', b'f', b'm', b't', b' ',
    0x10, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x80, 0xbb, 0x00, 0x00, 0x00, 0x77, 0x01, 0x00,
    0x02, 0x00, 0x10, 0x00, b'd', b'a', b't', b'a', 0x00, 0xf0, 0xff, 0x7f,
];

#[derive(Clone, Debug, PartialEq, Eq)]
enum Job {
    /// 8 kHz to 48 kHz by repeating each sample. `channels` is 1, or 2 when
    /// the two slots are summed first.
    Up { channels: usize },
    /// 48 kHz mono to 8 kHz mono.
    Down,
    /// 48 kHz mono raw to a streaming WAV of the same samples.
    Wav,
    /// Play stdin on a PipeWire sink at the decoder's real rate.
    Play {
        rate: u32,
        channels: usize,
        target: Option<String>,
    },
}

struct Decimate {
    z: [f32; 36],
    n: usize,
}

impl Decimate {
    fn new() -> Self {
        Self { z: [0.0; 36], n: 0 }
    }

    fn push(&mut self, sample: i16) -> Option<i16> {
        self.z.copy_within(0..35, 1);
        self.z[0] = sample as f32;
        self.n += 1;
        if self.n < TAPS.len() || (self.n - TAPS.len()) % UP != 0 {
            return None;
        }
        let mut acc = 0.0f32;
        for (tap, coeff) in self.z.iter().zip(TAPS) {
            acc += tap * coeff;
        }
        Some(acc.round().clamp(-32768.0, 32767.0) as i16)
    }
}

fn mix(left: i16, right: i16) -> i16 {
    left.saturating_add(right)
}

fn parse(args: &[String]) -> Result<Job, String> {
    if args.first().map(String::as_str) == Some("play") {
        return parse_play(&args[1..]);
    }
    let mut channels: Option<usize> = None;
    let mut in_rate: Option<u32> = None;
    let mut out_rate: Option<u32> = None;
    let mut wav = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let value = || -> Result<&str, String> {
            args.get(i + 1).map(String::as_str).ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--channels" => {
                let text = value()?;
                channels = Some(match text {
                    "1" => 1,
                    "2" => 2,
                    _ => return Err("--channels must be 1 or 2".into()),
                });
                i += 2;
            }
            "--in-rate" => {
                in_rate = Some(value()?.parse().map_err(|_| "--in-rate must be a number")?);
                i += 2;
            }
            "--out-rate" => {
                out_rate = Some(value()?.parse().map_err(|_| "--out-rate must be a number")?);
                i += 2;
            }
            "--wav" => {
                wav = true;
                i += 1;
            }
            "--help" | "-h" => return Err(help()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if wav {
        if channels.is_some() || in_rate.is_some() || out_rate.is_some() {
            return Err("--wav reads 48 kHz mono and takes no other options".into());
        }
        return Ok(Job::Wav);
    }
    let channels = channels.unwrap_or(1);
    let in_rate = in_rate.unwrap_or(8_000);
    let out_rate = out_rate.unwrap_or(48_000);
    match (in_rate, out_rate, channels) {
        (8_000, 48_000, 1 | 2) => Ok(Job::Up { channels }),
        (48_000, 8_000, 1) => Ok(Job::Down),
        (48_000, 8_000, _) => Err("the 48 kHz to 8 kHz step is mono".into()),
        _ => Err("rates must be 8000 to 48000, or 48000 to 8000".into()),
    }
}

fn parse_play(args: &[String]) -> Result<Job, String> {
    let mut rate: Option<u32> = None;
    let mut channels: Option<usize> = None;
    let mut target: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let value = || -> Result<&str, String> {
            args.get(i + 1).map(String::as_str).ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--rate" => {
                let text = value()?;
                let parsed: u32 = text.parse().map_err(|_| "--rate must be a number")?;
                if !(8_000..=192_000).contains(&parsed) {
                    return Err("--rate must be from 8000 to 192000".into());
                }
                rate = Some(parsed);
                i += 2;
            }
            "--channels" => {
                let text = value()?;
                channels = Some(match text {
                    "1" => 1,
                    "2" => 2,
                    _ => return Err("--channels must be 1 or 2".into()),
                });
                i += 2;
            }
            "--target" => {
                let text = value()?;
                if text.is_empty() || text.starts_with('-') || text.contains('\0') || text.contains(' ') {
                    return Err("--target is not a sink name".into());
                }
                target = Some(text.to_string());
                i += 2;
            }
            "--help" | "-h" => return Err(help()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let rate = rate.ok_or("--rate is required with play")?;
    let channels = channels.ok_or("--channels is required with play")?;
    Ok(Job::Play { rate, channels, target })
}

fn help() -> String {
    "commander-voice: play decoder voice on a PipeWire sink\n\
     usage: commander-voice play --rate <hz> --channels <1|2> [--target <sink>]\n\
            commander-voice [--channels 1|2] [--in-rate 8000] [--out-rate 48000]\n\
            commander-voice --in-rate 48000 --out-rate 8000 --channels 1\n\
            commander-voice --wav\n"
        .into()
}

fn process(job: &Job, input: &[u8], decimate: &mut Decimate, output: &mut Vec<i16>) {
    output.clear();
    match job {
        Job::Up { channels } => {
            let channels = *channels;
            let step = channels * 2;
            let mut i = 0;
            while i + step <= input.len() {
                let sample = if channels == 1 {
                    i16::from_le_bytes([input[i], input[i + 1]])
                } else {
                    mix(
                        i16::from_le_bytes([input[i], input[i + 1]]),
                        i16::from_le_bytes([input[i + 2], input[i + 3]]),
                    )
                };
                for _ in 0..UP {
                    output.push(sample);
                }
                i += step;
            }
        }
        Job::Down => {
            let mut i = 0;
            while i + 2 <= input.len() {
                let sample = i16::from_le_bytes([input[i], input[i + 1]]);
                if let Some(out) = decimate.push(sample) {
                    output.push(out);
                }
                i += 2;
            }
        }
        Job::Wav | Job::Play { .. } => {
            let mut i = 0;
            while i + 2 <= input.len() {
                output.push(i16::from_le_bytes([input[i], input[i + 1]]));
                i += 2;
            }
        }
    }
}

/// `Ok(false)` is a closed reader. The decoder is then out of a place to
/// write, and the script exits with the decoder. No error line: the pipe
/// closing is the signal.
fn write_samples(out: &mut impl Write, samples: &[i16]) -> io::Result<bool> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    match out.write_all(&bytes).and_then(|()| out.flush()) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(err) => Err(err),
    }
}

fn run(job: Job) -> io::Result<()> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if job == Job::Wav {
        output.write_all(&WAV_HEADER)?;
        output.flush()?;
    }
    let mut buf = [0u8; 8192];
    let mut pending = Vec::new();
    let mut decoded = Vec::new();
    let mut decimate = Decimate::new();
    let frame = match &job {
        Job::Up { channels } => channels * 2,
        Job::Down | Job::Wav | Job::Play { .. } => 2,
    };
    loop {
        let n = match input.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        pending.extend_from_slice(&buf[..n]);
        let whole = pending.len() - pending.len() % frame;
        if whole == 0 {
            continue;
        }
        process(&job, &pending[..whole], &mut decimate, &mut decoded);
        pending.drain(..whole);
        if !write_samples(&mut output, &decoded)? {
            break;
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match parse(&args) {
        Ok(Job::Play { rate, channels, target }) => match play::run(rate, channels, target.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("commander-voice: {err}");
                ExitCode::from(1)
            }
        },
        Ok(job) => match run(job) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("commander-voice: {err}");
                ExitCode::from(1)
            }
        },
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(samples: &[i16]) -> Vec<u8> {
        let mut out = Vec::new();
        for sample in samples {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }

    #[test]
    fn one_channel_is_repeated_six_times() {
        let mut out = Vec::new();
        process(&Job::Up { channels: 1 }, &bytes(&[1000, -1000]), &mut Decimate::new(), &mut out);
        assert_eq!(out, vec![1000; 6].into_iter().chain(vec![-1000; 6]).collect::<Vec<_>>());
    }

    #[test]
    fn two_slots_are_summed_and_a_full_scale_sum_saturates() {
        let mut out = Vec::new();
        process(
            &Job::Up { channels: 2 },
            &bytes(&[20_000, 20_000, -20_000, -20_000, 3, 4]),
            &mut Decimate::new(),
            &mut out,
        );
        assert_eq!(&out[..6], &[32767; 6]);
        assert_eq!(&out[6..12], &[-32768; 6]);
        assert_eq!(&out[12..], &[7; 6]);
    }

    #[test]
    fn a_constant_level_survives_the_downsample() {
        let mut decimate = Decimate::new();
        let mut out = Vec::new();
        process(&Job::Down, &bytes(&vec![1000; 480]), &mut decimate, &mut out);
        assert!(!out.is_empty());
        assert!(out.iter().all(|sample| (*sample - 1000).abs() <= 1));
    }

    #[test]
    fn a_tone_above_the_new_nyquist_is_rejected() {
        let mut decimate = Decimate::new();
        let mut out = Vec::new();
        // 12 kHz at 48 kHz sample rate: four samples per cycle.
        let tone: Vec<i16> = (0..4800).map(|n| if n % 4 < 2 { 8000 } else { -8000 }).collect();
        process(&Job::Down, &bytes(&tone), &mut decimate, &mut out);
        let peak = out.iter().map(|sample| sample.unsigned_abs()).max().unwrap_or(0);
        assert!(peak < 800, "12 kHz leaked at {peak}");
    }

    #[test]
    fn wav_header_matches_the_pipe_header_sox_writes() {
        assert_eq!(&WAV_HEADER[..4], b"RIFF");
        assert_eq!(&WAV_HEADER[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(WAV_HEADER[24..28].try_into().unwrap()), 48_000);
    }

    #[test]
    fn an_odd_trailing_byte_is_kept_until_its_pair_arrives() {
        let mut decimate = Decimate::new();
        let mut out = Vec::new();
        let mut pending = vec![0xe8, 0x03, 0x01];
        let frame = 2;
        let whole = pending.len() - pending.len() % frame;
        process(&Job::Up { channels: 1 }, &pending[..whole], &mut decimate, &mut out);
        pending.drain(..whole);
        assert_eq!(out, vec![1000; 6]);
        assert_eq!(pending, vec![0x01]);
    }

    #[test]
    fn unknown_rates_and_a_stereo_downsample_are_refused() {
        assert!(parse(&[ "--in-rate".into(), "16000".into() ]).is_err());
        assert!(parse(&[
            "--in-rate".into(),
            "48000".into(),
            "--out-rate".into(),
            "8000".into(),
            "--channels".into(),
            "2".into(),
        ])
        .is_err());
        assert_eq!(
            parse(&["--channels".into(), "2".into()]).unwrap(),
            Job::Up { channels: 2 }
        );
        assert_eq!(parse(&["--wav".into()]).unwrap(), Job::Wav);
    }

    #[test]
    fn play_requires_a_rate_and_a_channel_count() {
        assert!(parse(&["play".into()]).is_err());
        assert!(parse(&["play".into(), "--rate".into(), "0".into(), "--channels".into(), "1".into()]).is_err());
        assert!(parse(&["play".into(), "--rate".into(), "8000".into(), "--channels".into(), "3".into()]).is_err());
        assert!(parse(&["play".into(), "--rate".into(), "8000".into(), "--channels".into(), "1".into(), "--target".into(), "-bad".into()]).is_err());
        match parse(&[
            "play".into(),
            "--rate".into(),
            "8000".into(),
            "--channels".into(),
            "2".into(),
            "--target".into(),
            "bluez_output.example".into(),
        ])
        .unwrap()
        {
            Job::Play { rate, channels, target } => {
                assert_eq!(rate, 8000);
                assert_eq!(channels, 2);
                assert_eq!(target.as_deref(), Some("bluez_output.example"));
            }
            other => panic!("expected play, got {other:?}"),
        }
    }
}
