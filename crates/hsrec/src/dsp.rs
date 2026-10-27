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
/// Discriminator rate: 16 kHz.
pub const MID_RATE: f32 = CAPTURE_RATE as f32 / LIVE_BOX as f32;
/// Scale from mean of i8 samples to stored i16.
pub const STORE_SCALE: f32 = 256.0;
/// Same numbers as the live voice path (`hackrf::SCALE_HZ`, `SCALE_I16`).
pub const SCALE_HZ: f32 = 4_000.0;
pub const SCALE_I16: f32 = 12_000.0;
/// Identifies the chain above, stored per recording.
pub const CHAIN_ID: &str = "hackrf8M-dc-mix-box250-cs16@32k;replay:pair-avg-disc16k-x3@48k";

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
#[derive(Default)]
pub struct Replayer {
    half: Option<(f32, f32)>,
    prev: Option<(f32, f32)>,
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
                emit3(discriminate(prev, mean, MID_RATE), out);
            }
            self.prev = Some(mean);
        }
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
}
