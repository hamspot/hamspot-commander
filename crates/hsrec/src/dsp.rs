//! Shared DSP for the REC tap and replay.
//!
//! The live HackRF voice path mixes the 8 Msps capture down to the call and
//! boxcar-averages 500 samples (16 kHz), then runs a phase discriminator.
//! The recorder stores each *half* of that boxcar (250 samples, 32 ksps
//! complex) as cs16, so replay can rebuild the exact 16 kHz complex stream
//! by averaging pairs and run the same discriminator as live.
use std::f32::consts::PI;

/// HackRF capture rate (must match `hackrf::SAMPLE_RATE`).
pub const CAPTURE_RATE: u32 = 8_000_000;
/// Live boxcar length (must match `hackrf::BOX`).
pub const LIVE_BOX: usize = 500;
/// Recorder boxcar: half of the live one.
pub const REC_BOX: usize = LIVE_BOX / 2;
/// Recorded complex sample rate: 32 ksps.
pub const REC_RATE: u32 = CAPTURE_RATE / REC_BOX as u32;

/// Usable width of the HackRF capture (its baseband filter; must match
/// `hackrf::FILTER_HZ`). A channel outside it is not in the samples at all.
pub const USABLE_HZ: f64 = 6_000_000.0;

/// Whether a channel `offset_hz` from the capture centre lies inside the
/// usable capture with its whole recorder bandwidth.
pub fn in_capture(offset_hz: f64) -> bool {
    offset_hz.is_finite() && offset_hz.abs() + REC_RATE as f64 / 2.0 <= USABLE_HZ / 2.0
}
/// Discriminator rate: 16 kHz.
pub const MID_RATE: f32 = CAPTURE_RATE as f32 / LIVE_BOX as f32;
/// Scale from mean of i8 samples to stored i16.
pub const STORE_SCALE: f32 = 256.0;
/// Same numbers as the live voice path (`hackrf::SCALE_HZ`, `SCALE_I16`).
pub const SCALE_HZ: f32 = 4_000.0;
pub const SCALE_I16: f32 = 12_000.0;
/// Identifies the chain above, stored per recording.
pub const CHAIN_ID: &str = "hackrf8M-dc-mix-box250-cs16@32k;replay:pair-avg-disc16k-afc-x3@48k";

/// Discriminator output in Hz to the s16 sample the decoders read.
pub fn hz_to_i16(hz: f32) -> i16 {
    let sample = (hz / SCALE_HZ * SCALE_I16).round();
    sample.clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

/// Phase step between two complex samples, in Hz at `rate`.
pub fn discriminate(prev: (f32, f32), cur: (f32, f32), rate: f32) -> f32 {
    let cross = prev.0 * cur.1 - prev.1 * cur.0;
    let dot = prev.0 * cur.0 + prev.1 * cur.1;
    cross.atan2(dot) * rate / (2.0 * PI)
}

/// Repeat one 16 kHz sample three times (48 kHz pipe), as live does.
pub fn emit3(hz: f32, out: &mut Vec<i16>) {
    let s = hz_to_i16(hz);
    out.extend_from_slice(&[s, s, s]);
}

/// Mean of i8 samples to a stored i16.
pub fn store_i16(mean: f32) -> i16 {
    (mean * STORE_SCALE)
        .round()
        .clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

/// 16 kHz discriminator samples per AFC measurement block (100 ms).
pub const AFC_BLOCK: u32 = 1_600;
/// A block counts as a carrier when its mean magnitude (i8 counts) is at
/// least this. Below it, a flat envelope is the DC block's residual.
pub const AFC_FLOOR: f32 = 0.15;
/// Envelope spread (std / mean) below which a block is a carrier. Noise is
/// about 0.52; the SAFE-T C4FM control and voice carriers are 0.04-0.20.
pub const AFC_CV: f32 = 0.35;
/// Largest offset the tracker accepts. A 20 ppm HackRF crystal at 900 MHz
/// is 18 kHz off, but anything past this is outside the channel.
pub const AFC_MAX_HZ: f32 = 4_000.0;
/// Weight of each new carrier block once the tracker has locked.
pub const AFC_WEIGHT: f32 = 0.2;

/// A carrier must stand this far above the channel's learned noise floor
/// (envelope ratio 2.0 = 6 dB) to open the REC squelch.
pub const SQUELCH_SNR: f32 = 2.0;

/// REC squelch for one channel: is there an RF signal, or only noise?
/// Each [`AFC_BLOCK`] (100 ms) of boxcar magnitudes is judged as the AFC
/// probe judges it (mean at least [`AFC_FLOOR`], envelope spread under
/// [`AFC_CV`]: an FM or C4FM carrier is flat, noise is Rayleigh), and in
/// addition at least [`SQUELCH_SNR`] above the channel's noise floor. The
/// floor is the average of the blocks that were not a carrier: it follows
/// a quieter block quickly and a louder one slowly, so a carrier never
/// teaches it. A channel that has never been quiet has no floor yet and
/// is judged by the AFC rule alone (a control channel keyed all the time).
#[derive(Debug, Default, Clone)]
pub struct Squelch {
    n: u32,
    sum: f64,
    sum2: f64,
    floor: Option<f32>,
    open: bool,
    snr_db: f32,
}

impl Squelch {
    /// One boxcar magnitude. `Some(open)` at the end of each block.
    pub fn push(&mut self, mag: f32) -> Option<bool> {
        let mag = if mag.is_finite() && mag >= 0.0 { mag } else { 0.0 };
        self.n += 1;
        self.sum += mag as f64;
        self.sum2 += (mag as f64) * (mag as f64);
        if self.n < AFC_BLOCK {
            return None;
        }
        let n = self.n as f64;
        let mean = self.sum / n;
        let var = (self.sum2 / n - mean * mean).max(0.0);
        let cv = if mean > 1e-6 { (var.sqrt() / mean) as f32 } else { f32::MAX };
        let mean = mean as f32;
        self.n = 0;
        self.sum = 0.0;
        self.sum2 = 0.0;
        let above = self.floor.is_none_or(|f| mean >= f * SQUELCH_SNR);
        let carrier = mean >= AFC_FLOOR && cv < AFC_CV && above;
        if !carrier {
            self.floor = Some(match self.floor {
                None => mean,
                Some(f) if mean < f => f * 0.7 + mean * 0.3,
                Some(f) => f * 0.98 + mean * 0.02,
            });
        }
        self.snr_db = match self.floor {
            Some(f) if f > 1e-6 && mean > 1e-6 => 20.0 * (mean / f).log10(),
            _ => 0.0,
        };
        self.open = carrier;
        Some(carrier)
    }

    /// The last finished block was a signal.
    pub fn open(&self) -> bool {
        self.open
    }

    /// Last block above the noise floor, in dB (0 before a floor is known).
    pub fn snr_db(&self) -> f32 {
        self.snr_db
    }

    pub fn floor(&self) -> Option<f32> {
        self.floor
    }
}

/// Statistics of one channel's discriminator over [`AFC_BLOCK`] samples.
/// `push` returns the block's mean frequency when the block is a carrier.
#[derive(Debug, Default, Clone)]
pub struct AfcProbe {
    n: u32,
    sum_hz: f64,
    sum_mag: f64,
    sum_mag2: f64,
    /// Mean magnitude of the last finished carrier block, 0 when the last
    /// block was not a carrier.
    pub strength: f32,
}

impl AfcProbe {
    pub fn push(&mut self, hz: f32, mag: f32) -> Option<f32> {
        let hz = if hz.is_finite() { hz } else { 0.0 };
        let mag = if mag.is_finite() && mag >= 0.0 { mag } else { 0.0 };
        self.n += 1;
        self.sum_hz += hz as f64;
        self.sum_mag += mag as f64;
        self.sum_mag2 += (mag as f64) * (mag as f64);
        if self.n < AFC_BLOCK {
            return None;
        }
        let n = self.n as f64;
        let mean_hz = (self.sum_hz / n) as f32;
        let mean_mag = self.sum_mag / n;
        let var = (self.sum_mag2 / n - mean_mag * mean_mag).max(0.0);
        let cv = if mean_mag > 1e-6 { (var.sqrt() / mean_mag) as f32 } else { f32::MAX };
        self.n = 0;
        self.sum_hz = 0.0;
        self.sum_mag = 0.0;
        self.sum_mag2 = 0.0;
        let carrier = (mean_mag as f32) >= AFC_FLOOR && cv < AFC_CV && mean_hz.abs() <= AFC_MAX_HZ;
        self.strength = if carrier { mean_mag as f32 } else { 0.0 };
        carrier.then_some(mean_hz)
    }

    /// Forget a partial block, for example after the channel changed.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Automatic frequency control for the discriminator.
///
/// The HackRF has no TCXO. Its crystal drifts as the board warms: on the
/// SAFE-T recordings the carriers sat 0.4-0.6 kHz high when the capture
/// started and 1.3-2.0 kHz high an hour later. That offset is a DC level on
/// the discriminator, and dsd-fme stops finding P25 sync between 1.2 and
/// 1.5 kHz of it. Every carrier in one capture shares the crystal, so one
/// estimate, learned from whichever carriers are up, corrects all of them.
/// Only the discriminator output is corrected; the stored IQ stays raw.
#[derive(Debug, Default, Clone)]
pub struct Afc {
    err_hz: f32,
    locked: bool,
}

impl Afc {
    /// Fold in one carrier block's mean frequency.
    pub fn update(&mut self, block_mean_hz: f32) {
        if !block_mean_hz.is_finite() || block_mean_hz.abs() > AFC_MAX_HZ {
            return;
        }
        if self.locked {
            self.err_hz += AFC_WEIGHT * (block_mean_hz - self.err_hz);
        } else {
            self.err_hz = block_mean_hz;
            self.locked = true;
        }
    }

    /// Current frequency error in Hz (positive: carriers read high).
    pub fn error_hz(&self) -> f32 {
        self.err_hz
    }

    pub fn locked(&self) -> bool {
        self.locked
    }

    /// Discriminator sample with the frequency error removed.
    pub fn correct(&self, hz: f32) -> f32 {
        hz - self.err_hz
    }
}

/// Stand-alone channeliser for tests and tools: DC-free cs8 at 8 Msps in,
/// 32 ksps cs16 out at `offset` Hz from the capture centre. The live path
/// does the same inside its voice mixer so it costs nothing extra.
pub struct Channelizer {
    rot: (f32, f32),
    osc: (f32, f32),
    acc: (f32, f32),
    filled: usize,
    norm: usize,
}

impl Channelizer {
    pub fn new(offset_hz: f64) -> Self {
        let step = -2.0 * std::f64::consts::PI * offset_hz / CAPTURE_RATE as f64;
        Self {
            rot: (step.cos() as f32, step.sin() as f32),
            osc: (1.0, 0.0),
            acc: (0.0, 0.0),
            filled: 0,
            norm: 0,
        }
    }

    pub fn push(&mut self, i: f32, q: f32, out: &mut Vec<i16>) {
        let (or, oi) = self.osc;
        let bi = i * or - q * oi;
        let bq = i * oi + q * or;
        self.osc = (or * self.rot.0 - oi * self.rot.1, or * self.rot.1 + oi * self.rot.0);
        self.norm += 1;
        if self.norm >= LIVE_BOX {
            let mag = (self.osc.0 * self.osc.0 + self.osc.1 * self.osc.1).sqrt();
            self.osc = if mag > 1e-12 { (self.osc.0 / mag, self.osc.1 / mag) } else { (1.0, 0.0) };
            self.norm = 0;
        }
        self.acc.0 += bi;
        self.acc.1 += bq;
        self.filled += 1;
        if self.filled == REC_BOX {
            out.push(store_i16(self.acc.0 / REC_BOX as f32));
            out.push(store_i16(self.acc.1 / REC_BOX as f32));
            self.acc = (0.0, 0.0);
            self.filled = 0;
        }
    }
}

/// Replay side: 32 ksps cs16 in, the live 48 kHz s16 discriminator stream out.
/// The recording is raw IQ; the same [`Afc`] as live removes the HackRF's
/// frequency error from the discriminator. It learns from the recording
/// itself, so the first 100 ms of a call are not yet corrected.
#[derive(Default)]
pub struct Replayer {
    half: Option<(f32, f32)>,
    prev: Option<(f32, f32)>,
    probe: AfcProbe,
    afc: Afc,
}

impl Replayer {
    /// `iq` is interleaved I,Q at 32 ksps.
    pub fn push(&mut self, iq: &[i16], out: &mut Vec<i16>) {
        for pair in iq.chunks_exact(2) {
            let s = (pair[0] as f32 / STORE_SCALE, pair[1] as f32 / STORE_SCALE);
            let Some(h) = self.half.take() else {
                self.half = Some(s);
                continue;
            };
            let mean = ((h.0 + s.0) * 0.5, (h.1 + s.1) * 0.5);
            if let Some(prev) = self.prev {
                let hz = discriminate(prev, mean, MID_RATE);
                let mag = (mean.0 * mean.0 + mean.1 * mean.1).sqrt();
                if let Some(block) = self.probe.push(hz, mag) {
                    self.afc.update(block);
                }
                emit3(self.afc.correct(hz), out);
            }
            self.prev = Some(mean);
        }
    }

    /// Frequency error the replay has learned so far, in Hz.
    pub fn afc_hz(&self) -> f32 {
        self.afc.error_hz()
    }
}

/// Synthetic FM at `offset_hz` from centre with a `tone_hz` modulating tone,
/// cs8 at 8 Msps. Test vectors only.
pub fn synth_fm_cs8(offset_hz: f64, tone_hz: f64, dev_hz: f64, secs: f64, amp: f64) -> Vec<u8> {
    let n = (secs * CAPTURE_RATE as f64) as usize;
    let mut out = Vec::with_capacity(n * 2);
    let fs = CAPTURE_RATE as f64;
    let mut phase = 0.0f64;
    for k in 0..n {
        let t = k as f64 / fs;
        let inst = offset_hz + dev_hz * (2.0 * std::f64::consts::PI * tone_hz * t).sin();
        phase += 2.0 * std::f64::consts::PI * inst / fs;
        if phase > std::f64::consts::PI * 2.0 {
            phase -= std::f64::consts::PI * 2.0;
        }
        out.push((amp * phase.cos()).round() as i8 as u8);
        out.push((amp * phase.sin()).round() as i8 as u8);
    }
    out
}

/// P25 Phase 1 frame sync, 24 dibits (`0x5575F5FF77FF`).
pub const P25_FRAME_SYNC: u64 = 0x5575_F5FF_77FF;

/// C4FM deviation for one P25 dibit: 01 is +3, 00 is +1, 10 is -1, 11 is -3.
fn c4fm_dev(dibit: u8) -> f64 {
    match dibit & 3 {
        0b01 => 1_800.0,
        0b00 => 600.0,
        0b10 => -600.0,
        _ => -1_800.0,
    }
}

/// Instantaneous frequency of a P25-like C4FM stream at `rate`: frame sync
/// then 400 pseudo-random symbols, repeated, at 4800 symbols/s, smoothed over
/// half a symbol. Not a decodable frame; dsd-fme still reports `+P25p1` sync
/// on every frame sync. Test vectors only.
pub fn synth_c4fm_freq(rate: f64, secs: f64) -> Vec<f64> {
    let mut symbols = Vec::new();
    let mut lcg: u32 = 0x1234_5678;
    let want = (secs * 4_800.0).ceil() as usize + 1;
    while symbols.len() < want {
        for k in 0..24 {
            symbols.push(c4fm_dev(((P25_FRAME_SYNC >> (46 - 2 * k)) & 3) as u8));
        }
        for _ in 0..400 {
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            symbols.push(c4fm_dev((lcg >> 30) as u8));
        }
    }
    let n = (secs * rate) as usize;
    let sps = rate / 4_800.0;
    let raw: Vec<f64> = (0..n).map(|k| symbols[(k as f64 / sps) as usize]).collect();
    let half = ((sps / 2.0) as usize).max(1);
    let mut out = Vec::with_capacity(n);
    let mut sum = 0.0;
    for k in 0..n {
        sum += raw[k];
        if k >= half {
            sum -= raw[k - half];
        }
        out.push(sum / half.min(k + 1) as f64);
    }
    out
}

/// P25-like C4FM at `offset_hz` from the capture centre, cs8 at 8 Msps. The
/// carrier reads `err_hz` high, as a warm HackRF crystal does. Test vectors only.
pub fn synth_c4fm_cs8(offset_hz: f64, err_hz: f64, secs: f64, amp: f64) -> Vec<u8> {
    let fs = CAPTURE_RATE as f64;
    let freq = synth_c4fm_freq(fs, secs);
    let mut out = Vec::with_capacity(freq.len() * 2);
    let mut phase = 0.0f64;
    let tau = std::f64::consts::PI * 2.0;
    for dev in freq {
        phase += tau * (offset_hz + err_hz + dev) / fs;
        if phase > tau {
            phase -= tau;
        } else if phase < 0.0 {
            phase += tau;
        }
        out.push((amp * phase.cos()).round() as i8 as u8);
        out.push((amp * phase.sin()).round() as i8 as u8);
    }
    out
}

/// The same C4FM as recorded by the REC tap: cs16 at 32 ksps, `err_hz` high.
pub fn synth_c4fm_cs16(err_hz: f64, secs: f64, amp: f64) -> Vec<i16> {
    let fs = REC_RATE as f64;
    let freq = synth_c4fm_freq(fs, secs);
    let mut out = Vec::with_capacity(freq.len() * 2);
    let mut phase = 0.0f64;
    for dev in freq {
        phase += std::f64::consts::PI * 2.0 * (err_hz + dev) / fs;
        out.push(store_i16((amp * phase.cos()) as f32));
        out.push(store_i16((amp * phase.sin()) as f32));
    }
    out
}

/// Dominant frequency of `samples` at `rate` by zero crossings (test helper).
pub fn zero_cross_hz(samples: &[i16], rate: f32) -> f32 {
    let mean: f64 = samples.iter().map(|s| *s as f64).sum::<f64>() / samples.len().max(1) as f64;
    let mut crossings = 0usize;
    for w in samples.windows(2) {
        let a = w[0] as f64 - mean;
        let b = w[1] as f64 - mean;
        if a < 0.0 && b >= 0.0 {
            crossings += 1;
        }
    }
    crossings as f32 * rate / samples.len().max(1) as f32
}

#[cfg(test)]
mod tests {
    #[test]
    fn capture_window() {
        assert!(super::in_capture(-537_500.0));
        assert!(super::in_capture(2_900_000.0));
        // SAFE-T recordings that replayed to noise: grants 4.3 and 5.2 MHz away.
        assert!(!super::in_capture(-4_337_500.0));
        assert!(!super::in_capture(-5_212_500.0));
        assert!(!super::in_capture(f64::NAN));
    }

    use super::*;

    fn channelise(cs8: &[u8], offset: f64) -> Vec<i16> {
        let mut ch = Channelizer::new(offset);
        let mut out = Vec::new();
        for p in cs8.chunks_exact(2) {
            ch.push(p[0] as i8 as f32, p[1] as i8 as f32, &mut out);
        }
        out
    }

    #[test]
    fn rate_is_32k() {
        assert_eq!(REC_RATE, 32_000);
    }

    #[test]
    fn squelch_opens_on_a_carrier_and_stays_shut_on_noise() {
        // Noise: Rayleigh magnitudes around 0.3 (spread about 0.52).
        let mut sq = Squelch::default();
        let mut seed = 12345u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let mut noise = |sq: &mut Squelch, blocks: u32| {
            let mut opened = false;
            for _ in 0..blocks * AFC_BLOCK {
                let u = rnd().max(1e-6);
                let m = 0.3 * (-2.0 * u.ln()).sqrt();
                if sq.push(m) == Some(true) {
                    opened = true;
                }
            }
            opened
        };
        assert!(!noise(&mut sq, 20), "noise never opens it");
        let floor = sq.floor().expect("floor learned from noise");
        assert!((0.2..0.5).contains(&floor), "floor {floor}");
        // A flat carrier 10 dB above the noise opens it on the next block.
        let mut opened = false;
        for _ in 0..AFC_BLOCK {
            if sq.push(floor * 3.2) == Some(true) {
                opened = true;
            }
        }
        assert!(opened && sq.open());
        assert!(sq.snr_db() > 8.0, "snr {}", sq.snr_db());
        // A long carrier does not teach the floor.
        for _ in 0..600 * AFC_BLOCK {
            sq.push(floor * 3.2);
        }
        assert!(sq.open() && (sq.floor().unwrap() - floor).abs() < 1e-3);
        // A flat level only 3 dB over the floor is not a signal.
        for _ in 0..AFC_BLOCK {
            sq.push(floor * 1.4);
        }
        assert!(!sq.open());
        // A control channel keyed from the start (no floor yet) still opens.
        let mut cc = Squelch::default();
        for _ in 0..AFC_BLOCK {
            cc.push(5.0);
        }
        assert!(cc.open());
    }

    #[test]
    fn tone_at_offset_lands_at_dc() {
        // Unmodulated carrier 100 kHz off centre: after the shift, phase is constant.
        let cs8 = synth_fm_cs8(100_000.0, 1.0, 0.0, 0.02, 100.0);
        let iq = channelise(&cs8, 100_000.0);
        let mut steps = Vec::new();
        let pts: Vec<(f32, f32)> = iq.chunks_exact(2).map(|p| (p[0] as f32, p[1] as f32)).collect();
        for w in pts.windows(2).skip(4) {
            steps.push(discriminate(w[0], w[1], REC_RATE as f32).abs());
        }
        let worst = steps.iter().cloned().fold(0.0f32, f32::max);
        assert!(worst < 50.0, "residual {worst} Hz");
        // A carrier elsewhere is attenuated by the boxcar.
        let far = channelise(&synth_fm_cs8(300_000.0, 1.0, 0.0, 0.02, 100.0), 100_000.0);
        let p_far: f64 = far.iter().map(|s| (*s as f64).powi(2)).sum();
        let p_on: f64 = iq.iter().map(|s| (*s as f64).powi(2)).sum();
        assert!(p_far < p_on / 100.0);
    }

    #[test]
    fn fm_round_trip_recovers_tone() {
        let cs8 = synth_fm_cs8(-250_000.0, 1_000.0, 2_500.0, 0.25, 100.0);
        let iq = channelise(&cs8, -250_000.0);
        let mut rp = Replayer::default();
        let mut audio = Vec::new();
        rp.push(&iq, &mut audio);
        let hz = zero_cross_hz(&audio[4800..], 48_000.0);
        assert!((hz - 1_000.0).abs() < 30.0, "got {hz}");
        // Peak deviation 2.5 kHz -> about 7500 counts.
        let peak = audio[4800..].iter().map(|s| s.abs()).max().unwrap();
        assert!((6_500..8_500).contains(&peak), "peak {peak}");
    }

    fn replay_mean_hz(iq: &[i16], skip_secs: f32) -> (f32, Replayer) {
        let mut rp = Replayer::default();
        let mut audio = Vec::new();
        rp.push(iq, &mut audio);
        let skip = (skip_secs * 48_000.0) as usize;
        let tail = &audio[skip..];
        let mean = tail.iter().map(|s| *s as f64).sum::<f64>() / tail.len() as f64;
        ((mean as f32) * SCALE_HZ / SCALE_I16, rp)
    }

    #[test]
    fn afc_removes_a_warm_crystal_offset_from_c4fm() {
        // 1.75 kHz high, as the SAFE-T control channel was after an hour.
        let iq = synth_c4fm_cs16(1_750.0, 1.0, 40.0);
        let (mean, rp) = replay_mean_hz(&iq, 0.3);
        assert!((rp.afc_hz() - 1_750.0).abs() < 100.0, "afc {}", rp.afc_hz());
        assert!(mean.abs() < 100.0, "discriminator still {mean} Hz off");
    }

    #[test]
    fn afc_does_not_learn_from_noise() {
        let mut lcg: u32 = 7;
        let iq: Vec<i16> = (0..64_000)
            .map(|_| {
                lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((lcg >> 16) as i16) / 8
            })
            .collect();
        let mut rp = Replayer::default();
        let mut audio = Vec::new();
        rp.push(&iq, &mut audio);
        assert_eq!(rp.afc_hz(), 0.0);
    }

    #[test]
    fn afc_probe_reports_only_carrier_blocks() {
        let mut probe = AfcProbe::default();
        let mut got = None;
        for k in 0..AFC_BLOCK {
            got = probe.push(500.0 + (k % 7) as f32, 40.0);
        }
        assert!(got.is_some_and(|hz| (hz - 503.0).abs() < 0.1), "{got:?}");
        assert!(probe.strength > 0.0);
        for k in 0..AFC_BLOCK {
            got = probe.push(500.0, if k % 2 == 0 { 0.0 } else { 80.0 });
        }
        assert_eq!(got, None);
        assert_eq!(probe.strength, 0.0);
    }
}
