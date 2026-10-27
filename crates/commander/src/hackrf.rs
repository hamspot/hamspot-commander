//! Direct HackRF capture. One `hackrf_transfer` feeds every control channel
//! in the cluster and the voice channel. A voice grant inside the 6 MHz
//! window only moves the voice discriminator. A clear selected grant outside
//! that window retunes onto the voice channel until the call ends, then the
//! control cluster is restored.
//!
//! The baseband filter at 8 Msps is 6 MHz. Gain flags are omitted until a
//! stage is stored. The Qt blob in a GQRX document is not a confirmed scale.

use std::f64::consts::PI;
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use crate::follow::CLEAR_ALG;

pub const SAMPLE_RATE: u32 = 8_000_000;
/// Baseband filter. `hackrf_transfer` caps this at 0.75 times the sample rate.
pub const FILTER_HZ: f64 = 6_000_000.0;
/// Controls kept from one capture. More than this is a statewide list, not
/// a set of adjacent towers.
pub const MAX_CONTROLS: usize = 8;
const BOX: usize = 500;
const MID_RATE: f64 = SAMPLE_RATE as f64 / BOX as f64;
const SCALE_HZ: f64 = 4_000.0;
const SCALE_I16: f64 = 12_000.0;
/// Leaky-mean step. At 8 Msps this is about a 20 Hz cutoff, under C4FM
/// deviation, so a control parked on the LO keeps its modulation.
const DC_ALPHA: f32 = 1.0 / 65_536.0;

/// Running mean of I and Q. A HackRF ADC sits off the origin. One control
/// puts that bin on the LO, and the discriminator then has nothing to follow.
struct Dc {
    i: f32,
    q: f32,
}

impl Dc {
    fn new() -> Self {
        Self { i: 0.0, q: 0.0 }
    }

    fn apply(&mut self, i: f32, q: f32) -> (f32, f32) {
        self.i += DC_ALPHA * (i - self.i);
        self.q += DC_ALPHA * (q - self.q);
        (i - self.i, q - self.q)
    }
}

/// Largest run of frequencies that fits in the filter, and the midpoint.
/// An empty list has no capture. Equal frequencies count once.
pub fn place(freqs: &[f64]) -> Option<(f64, Vec<f64>)> {
    let mut ordered: Vec<f64> = freqs
        .iter()
        .copied()
        .filter(|hz| hz.is_finite() && *hz > 0.0)
        .collect();
    ordered.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    ordered.dedup_by(|left, right| (*left - *right).abs() < 1.0);
    if ordered.is_empty() {
        return None;
    }
    let mut best_i = 0;
    let mut best_j = 0;
    let mut i = 0;
    for j in 0..ordered.len() {
        while ordered[j] - ordered[i] > FILTER_HZ {
            i += 1;
        }
        if j - i > best_j - best_i {
            best_i = i;
            best_j = j;
        }
    }
    let mut kept = ordered[best_i..=best_j].to_vec();
    if kept.len() > MAX_CONTROLS {
        let mid = (kept[0] + kept[kept.len() - 1]) / 2.0;
        kept.sort_by(|left, right| {
            let dl = (left - mid).abs();
            let dr = (right - mid).abs();
            dl.partial_cmp(&dr).unwrap_or(std::cmp::Ordering::Equal)
        });
        kept.truncate(MAX_CONTROLS);
        kept.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    }
    let center = (kept[0] + kept[kept.len() - 1]) / 2.0;
    let kept: Vec<f64> = kept
        .into_iter()
        .filter(|hz| (*hz - center).abs() <= FILTER_HZ / 2.0)
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some((center, kept))
    }
}

pub fn inside(center: f64, hz: f64) -> bool {
    hz.is_finite() && (hz - center).abs() <= FILTER_HZ / 2.0
}

/// Where SCAN listens. A memory already inside this capture stays here.
/// Its level is one bin, and moving the LO would restart the dongle and
/// drop the other control decoders. A memory outside the capture is a
/// capture of its own.
pub fn scan_capture(center: f64, controls: &[f64], memory_hz: f64) -> (f64, Vec<f64>) {
    if inside(center, memory_hz) || !memory_hz.is_finite() || memory_hz < 1_000_000.0 {
        (center, controls.to_vec())
    } else {
        (memory_hz, vec![memory_hz])
    }
}

/// Dial frequency for one scanner sample. The HackRF dial is the LO. A
/// memory inside the capture is already on frequency, so the sample is
/// reported at that memory. A memory outside stays on the LO until a
/// retune lands, which is what the scanner's match is waiting for.
pub fn scan_freq(center_hz: f64, memory_hz: f64, strength_db: f64) -> f64 {
    if strength_db.is_finite() && inside(center_hz, memory_hz) {
        memory_hz
    } else {
        center_hz
    }
}

/// Bins in one displayed capture. The window is the sample rate, centered
/// on the LO. Index 0 is the low edge.
pub const SPECTRUM_BINS: usize = 1024;

/// One published frame. `seq` starts at 1 and wraps past 0. An empty `db`
/// means the capture has not produced a frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpectrumSnap {
    pub seq: u64,
    pub center_hz: f64,
    pub db: Vec<f32>,
}

/// Power of the newest `SPECTRUM_BINS` complex samples in `iq`, with no
/// mean removed. Short blocks are skipped. The bins are rotated so index 0
/// is the low edge.
#[cfg(test)]
fn spectrum_db(iq: &[u8]) -> Option<Vec<f32>> {
    spectrum_corrected(iq, 0.0, 0.0)
}

/// Same frame as [`spectrum_db`], with a measured I/Q mean taken off first.
fn spectrum_corrected(iq: &[u8], mean_i: f32, mean_q: f32) -> Option<Vec<f32>> {
    let pairs = iq.len() / 2;
    if pairs < SPECTRUM_BINS {
        return None;
    }
    let start = (pairs - SPECTRUM_BINS) * 2;
    let iq = &iq[start..start + SPECTRUM_BINS * 2];
    let mut re = vec![0.0; SPECTRUM_BINS];
    let mut im = vec![0.0; SPECTRUM_BINS];
    let denom = (SPECTRUM_BINS - 1) as f64;
    let mean_i = mean_i as f64;
    let mean_q = mean_q as f64;
    for i in 0..SPECTRUM_BINS {
        let window = 0.5 * (1.0 - (2.0 * PI * i as f64 / denom).cos());
        re[i] = (iq[i * 2] as i8 as f64 - mean_i) * window;
        im[i] = (iq[i * 2 + 1] as i8 as f64 - mean_q) * window;
    }
    fft(&mut re, &mut im);
    let mut db = vec![0.0f32; SPECTRUM_BINS];
    for i in 0..SPECTRUM_BINS {
        let src = (i + SPECTRUM_BINS / 2) % SPECTRUM_BINS;
        let power = re[src] * re[src] + im[src] * im[src];
        let value = 10.0 * power.max(1e-20).log10();
        if !value.is_finite() {
            return None;
        }
        db[i] = value as f32;
    }
    Some(db)
}

/// dB of the bin nearest `hz` above the median bin inside the baseband filter.
/// A quiet channel sits near zero. Empty or out-of-span input is not a sample.
pub fn channel_db(db: &[f32], center_hz: f64, hz: f64) -> f64 {
    if db.is_empty() || !center_hz.is_finite() || !hz.is_finite() || !inside(center_hz, hz) {
        return f64::NAN;
    }
    let n = db.len();
    let span = SAMPLE_RATE as f64;
    let bin_hz = span / n as f64;
    let low = center_hz - span / 2.0;
    let idx = ((hz - low) / bin_hz).round() as isize;
    if idx < 0 || idx >= n as isize {
        return f64::NAN;
    }
    let mut usable = Vec::new();
    for (i, value) in db.iter().enumerate() {
        let freq = low + (i as f64 + 0.5) * bin_hz;
        if (freq - center_hz).abs() <= FILTER_HZ / 2.0 && value.is_finite() {
            usable.push(*value);
        }
    }
    if usable.is_empty() {
        return f64::NAN;
    }
    usable.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let mid = usable[usable.len() / 2] as f64;
    db[idx as usize] as f64 - mid
}

/// Forward FFT. `re` and `im` are the same power-of-two length.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && n == im.len());
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f64;
        let wlen_re = ang.cos();
        let wlen_im = ang.sin();
        let mut i = 0;
        while i < n {
            let mut w_re = 1.0;
            let mut w_im = 0.0;
            for k in 0..len / 2 {
                let u_re = re[i + k];
                let u_im = im[i + k];
                let v_re = re[i + k + len / 2] * w_re - im[i + k + len / 2] * w_im;
                let v_im = re[i + k + len / 2] * w_im + im[i + k + len / 2] * w_re;
                re[i + k] = u_re + v_re;
                im[i + k] = u_im + v_im;
                re[i + k + len / 2] = u_re - v_re;
                im[i + k + len / 2] = u_im - v_im;
                let next_re = w_re * wlen_re - w_im * wlen_im;
                w_im = w_re * wlen_im + w_im * wlen_re;
                w_re = next_re;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// Publishes a spectrum frame about 15 times a second and wakes the hub.
/// The wake is the frame being ready. The capture thread does not sleep.
struct SpecPump {
    since: usize,
    seq: u64,
    slot: Arc<Mutex<SpectrumSnap>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl SpecPump {
    fn push(&mut self, iq: &[u8], center: f64, mean_i: f32, mean_q: f32) {
        self.since += iq.len() / 2;
        if self.since < (SAMPLE_RATE as usize) / 15 {
            return;
        }
        self.since = 0;
        let Some(db) = spectrum_corrected(iq, mean_i, mean_q) else {
            return;
        };
        self.seq = self.seq.wrapping_add(1);
        if self.seq == 0 {
            self.seq = 1;
        }
        {
            let mut slot = self.slot.lock().expect("hackrf spectrum");
            slot.seq = self.seq;
            slot.center_hz = center;
            slot.db = db;
        }
        (self.wake)();
    }
}

/// A talkgroup is heard when its zone is marked or its number is marked.
/// Until the operator marks either list, every talkgroup is heard. The
/// one-radio follower plays every clear grant the same way.
pub fn tg_selected(zone: Option<usize>, tg: u32, zones: &[usize], talkgroups: &[u32]) -> bool {
    if zones.is_empty() && talkgroups.is_empty() {
        return true;
    }
    zone.is_some_and(|index| zones.contains(&index)) || talkgroups.contains(&tg)
}

/// Voice frequency for one grant. `None` leaves the voice channel silent.
/// Encrypted grants and grants outside the window are not played.
pub fn grant_voice(center: f64, hz: f64, alg: u16, selected: bool) -> Option<f64> {
    if alg != CLEAR_ALG || !selected || !inside(center, hz) {
        return None;
    }
    Some(hz)
}

/// One voice call this radio left the control cluster to hear.
/// `synced` is the first voice-decoder sync on this trip. `searches` counts
/// voice-decoder search lines before that sync. Neither value is stored
/// in the station file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceAway {
    pub hz: f64,
    pub tg: u32,
    pub synced: bool,
    pub searches: u8,
}

/// Capture after a voice decision. `left` means the control cluster was
/// dropped for `voice_hz`. `open` is the calls still worth remembering,
/// newest `seq` last among equals.
#[derive(Clone, Debug, PartialEq)]
pub struct FollowCapture {
    pub center: f64,
    pub controls: Vec<f64>,
    pub left: bool,
    pub voice_hz: u64,
    pub call: Option<(f64, u32)>,
    pub open: Vec<(f64, u32, u64)>,
    pub away: Option<VoiceAway>,
    pub seq: u64,
}

/// Where the dongle listens for one voice frequency.
/// A grant inside the cluster keeps every control decoder. A grant outside
/// is a capture of its own, and only when this radio is allowed to leave.
/// The voice list is one frequency so the voice decoder still starts:
/// an empty control list starts no trunk scripts.
pub fn capture_for_voice(
    cluster_center: f64,
    controls: &[f64],
    voice_hz: Option<f64>,
    may_leave: bool,
) -> (f64, Vec<f64>, bool) {
    if may_leave && cluster_center >= 1_000_000.0 {
        if let Some(hz) = voice_hz {
            if hz.is_finite() && hz >= 1_000_000.0 && !inside(cluster_center, hz) {
                return (hz, vec![hz], true);
            }
        }
    }
    (cluster_center, controls.to_vec(), false)
}

/// Whether the voice decoder has finished the call this radio left for.
/// The first sync arms the trip. A search on that same update does not count.
/// Two searches before any sync, a sync loss after sync, or a terminator
/// finishes the trip. A sync loss before the first sync does not.
pub fn excursion_done(
    synced: bool,
    searches: u8,
    saw_sync: bool,
    saw_lost: bool,
    saw_end: bool,
    saw_search: bool,
) -> (bool, bool, u8) {
    if saw_end {
        return (true, synced || saw_sync, searches);
    }
    let mut synced = synced;
    let mut searches = searches;
    if saw_sync {
        synced = true;
    } else if saw_lost && synced {
        return (true, true, searches);
    } else if saw_search && !synced {
        searches = searches.saturating_add(1);
        if searches >= 2 {
            return (true, false, searches);
        }
    }
    (false, synced, searches)
}

/// Keep the held call in the remembered list so [`follow_capture`] keeps
/// choosing it while no new grant is offered.
pub fn keep_open(open: &mut Vec<(f64, u32, u64)>, call: (f64, u32), seq: u64) {
    if !open.iter().any(|row| row.1 == call.1) {
        open.push((call.0, call.1, seq));
    }
}

/// Inputs for [`follow_capture`] after the shared hold has decided.
/// A kept call offers no new grant and no end hint, so a butt-key, a dead
/// carrier or another talkgroup's grant cannot move it. A release forces the
/// end. A switch offers only the emergency or priority call.
pub fn hold_inputs(
    step: &crate::follow::WideHold,
    fresh: Vec<(f64, u32)>,
    seen: (bool, bool, bool, bool),
    open: &mut Vec<(f64, u32, u64)>,
    seq: u64,
) -> (Vec<(f64, u32)>, bool, bool, bool, bool) {
    use crate::follow::WideHold;
    match step {
        WideHold::Free => (fresh, seen.0, seen.1, seen.2, seen.3),
        WideHold::Keep(call) => {
            keep_open(open, *call, seq);
            (Vec::new(), false, false, false, false)
        }
        WideHold::Release => (fresh, false, false, true, false),
        WideHold::Switch(call) => (vec![*call], false, false, false, false),
    }
}

/// Pick the capture and the voice frequency for one radio.
/// `fresh` is newest-last and already limited to clear selected grants from
/// radios that are still on a control channel. `scan` wins over a voice
/// trip: memory scan keeps the dial it already chose.
/// `may_play` is false for a receiver that only watches the control channel.
pub fn follow_capture(
    cluster_center: f64,
    cluster_controls: &[f64],
    scan: Option<(f64, Vec<f64>)>,
    may_play: bool,
    may_leave: bool,
    mut open: Vec<(f64, u32, u64)>,
    mut seq: u64,
    previous: Option<(f64, u32)>,
    fresh: &[(f64, u32)],
    mut away: Option<VoiceAway>,
    saw_sync: bool,
    saw_lost: bool,
    saw_end: bool,
    saw_search: bool,
) -> FollowCapture {
    let may_leave = may_leave && scan.is_none();
    let fresh: Vec<(f64, u32)> = if may_play {
        fresh
            .iter()
            .copied()
            .filter(|(hz, _)| {
                may_leave || grant_voice(cluster_center, *hz, CLEAR_ALG, true).is_some()
            })
            .collect()
    } else {
        Vec::new()
    };

    let (done, synced, searches) = if let Some(state) = away {
        excursion_done(
            state.synced,
            state.searches,
            saw_sync,
            saw_lost,
            saw_end,
            saw_search,
        )
    } else {
        (false, false, 0)
    };
    if let Some(state) = away.as_mut() {
        state.synced = synced;
        state.searches = searches;
    }

    let mut replaced = false;
    for (hz, tg) in &fresh {
        seq = seq.wrapping_add(1);
        replaced = true;
        if let Some(row) = open.iter_mut().find(|row| row.1 == *tg) {
            row.0 = *hz;
            row.2 = seq;
        } else {
            open.push((*hz, *tg, seq));
        }
    }

    let ended = if away.is_some() { done } else { saw_end };
    if ended && !replaced {
        if let Some((_, tg)) = previous {
            open.retain(|row| row.1 != tg);
        }
        away = None;
    }

    let latch = away.map(|state| state.hz);
    open.retain(|row| {
        inside(cluster_center, row.0)
            || fresh.iter().any(|(_, tg)| row.1 == *tg)
            || latch.is_some_and(|hz| (row.0 - hz).abs() < 1.0)
    });

    let call = if !may_play {
        None
    } else if let Some((hz, tg)) = fresh.last().copied() {
        Some((hz, tg))
    } else if previous.is_some_and(|(_, tg)| open.iter().any(|row| row.1 == tg)) {
        previous
    } else {
        open.iter()
            .max_by_key(|row| row.2)
            .map(|row| (row.0, row.1))
    };

    let voice_for_capture = call.map(|(hz, _)| hz).filter(|_| may_play);
    let (center, controls, left) = if let Some((center, controls)) = scan {
        (center, controls, false)
    } else {
        capture_for_voice(
            cluster_center,
            cluster_controls,
            voice_for_capture,
            may_leave && may_play,
        )
    };

    let voice_hz = if !may_play {
        0
    } else if left {
        call.map(|(hz, _)| hz.round() as u64).unwrap_or(0)
    } else if let Some((hz, _)) = call {
        if inside(center, hz) {
            hz.round() as u64
        } else {
            0
        }
    } else {
        0
    };

    let away = if !left {
        None
    } else if let Some((hz, tg)) = call {
        let same = away.is_some_and(|state| (state.hz - hz).abs() < 1.0 && state.tg == tg);
        if same {
            Some(VoiceAway {
                hz,
                tg,
                synced,
                searches,
            })
        } else {
            Some(VoiceAway {
                hz,
                tg,
                synced: false,
                searches: 0,
            })
        }
    } else {
        None
    };

    let call = if may_play { call } else { None };
    if !may_play {
        open.clear();
    }

    FollowCapture {
        center,
        controls,
        left,
        voice_hz,
        call,
        open,
        away,
        seq,
    }
}

/// Gains already stored by GQRX. Each field is absent until that stage is
/// in the document. A stored zero is still sent: the tool's own default
/// is LNA 8 dB and VGA 20 dB, and a stored VGA of 0 is not that default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RxGain {
    pub lna_db: Option<u32>,
    pub vga_db: Option<u32>,
    pub amp: Option<bool>,
}

/// GQRX `input/gains`. The map values are dB times 10. An empty or broken
/// value leaves every stage unset.
pub fn parse_gqrx_gains(text: &str) -> RxGain {
    let bytes = variant_bytes(text);
    let mut pos = 0;
    let Some(kind) = read_be_u32(&bytes, &mut pos) else {
        return RxGain::default();
    };
    // QVariant::Map.
    if kind != 8 {
        return RxGain::default();
    }
    let Some(count) = read_be_u32(&bytes, &mut pos) else {
        return RxGain::default();
    };
    let mut gain = RxGain::default();
    for _ in 0..count {
        let Some(nbytes) = read_be_u32(&bytes, &mut pos) else {
            break;
        };
        let nbytes = nbytes as usize;
        if pos + nbytes > bytes.len() || nbytes % 2 != 0 {
            break;
        }
        let mut name = String::new();
        let raw = &bytes[pos..pos + nbytes];
        pos += nbytes;
        let mut i = 0;
        while i + 1 < raw.len() {
            let unit = u16::from_be_bytes([raw[i], raw[i + 1]]);
            if let Some(ch) = char::from_u32(u32::from(unit)) {
                name.push(ch);
            }
            i += 2;
        }
        let Some(vtype) = read_be_u32(&bytes, &mut pos) else {
            break;
        };
        // QVariant::Int. Anything else is not a gain slider.
        if vtype != 2 {
            break;
        }
        let Some(tenths) = read_be_u32(&bytes, &mut pos) else {
            break;
        };
        match name.trim().to_ascii_uppercase().as_str() {
            "LNA" => gain.lna_db = Some(step_down(tenths / 10, 8, 40)),
            "VGA" => gain.vga_db = Some(step_down(tenths / 10, 2, 62)),
            "AMP" => gain.amp = Some(tenths >= 70),
            _ => {}
        }
    }
    gain
}

/// `hackrf_transfer` LNA when `-l` is omitted.
pub const LNA_OMITTED: u32 = 8;
/// `hackrf_transfer` VGA when `-g` is omitted.
pub const VGA_OMITTED: u32 = 20;
pub const LNA_STEP: u32 = 8;
pub const LNA_MAX: u32 = 40;
pub const VGA_STEP: u32 = 2;
pub const VGA_MAX: u32 = 62;

fn step_down(value: u32, step: u32, max: u32) -> u32 {
    let value = value.min(max);
    if step == 0 {
        return value;
    }
    value - (value % step)
}

pub fn snap_lna(value: u32) -> u32 {
    step_down(value, LNA_STEP, LNA_MAX)
}

pub fn snap_vga(value: u32) -> u32 {
    step_down(value, VGA_STEP, VGA_MAX)
}

/// One wheel notch is one legal step. Past either end the stage stays put.
pub fn step_stage(value: u32, notches: i32, step: u32, max: u32) -> u32 {
    let snapped = step_down(value, step, max);
    if notches == 0 || step == 0 {
        return snapped;
    }
    let steps = (snapped / step) as i32 + notches;
    let max_steps = (max / step) as i32;
    (steps.clamp(0, max_steps) as u32) * step
}

fn variant_bytes(text: &str) -> Vec<u8> {
    let text = text.trim();
    let text = text
        .strip_prefix("@Variant(")
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(text);
    if text.contains('\\') && !text.contains('\0') {
        unescape_variant(text)
    } else {
        text.as_bytes().to_vec()
    }
}

fn unescape_variant(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        match bytes[i + 1] {
            b'0' => {
                out.push(0);
                i += 2;
            }
            b'b' => {
                out.push(8);
                i += 2;
            }
            b'n' => {
                out.push(b'\n');
                i += 2;
            }
            b'r' => {
                out.push(b'\r');
                i += 2;
            }
            b't' => {
                out.push(b'\t');
                i += 2;
            }
            b'\\' => {
                out.push(b'\\');
                i += 2;
            }
            b'x' => {
                let mut j = i + 2;
                let mut val = 0u8;
                let mut digits = 0;
                while digits < 2 && j < bytes.len() && bytes[j].is_ascii_hexdigit() {
                    val = val * 16
                        + match bytes[j] {
                            b'0'..=b'9' => bytes[j] - b'0',
                            b'a'..=b'f' => bytes[j] - b'a' + 10,
                            _ => bytes[j] - b'A' + 10,
                        };
                    j += 1;
                    digits += 1;
                }
                if digits == 0 {
                    out.push(b'\\');
                    i += 1;
                } else {
                    out.push(val);
                    i = j;
                }
            }
            _ => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    out
}

fn read_be_u32(buf: &[u8], pos: &mut usize) -> Option<u32> {
    let next = pos.checked_add(4)?;
    if next > buf.len() {
        return None;
    }
    let value = u32::from_be_bytes(buf[*pos..next].try_into().ok()?);
    *pos = next;
    Some(value)
}

/// `hackrf_transfer` argv. An empty serial omits `-d`. A gain stage is
/// omitted when the GQRX document does not name it.
pub fn transfer_args(serial: &str, center_hz: f64, gain: RxGain) -> Vec<String> {
    let mut args = vec![
        "-r".into(),
        "-".into(),
        "-f".into(),
        format!("{}", center_hz.round() as u64),
        "-s".into(),
        SAMPLE_RATE.to_string(),
        "-b".into(),
        format!("{}", FILTER_HZ.round() as u64),
    ];
    if let Some(amp) = gain.amp {
        args.push("-a".into());
        args.push(if amp { "1" } else { "0" }.into());
    }
    if let Some(lna) = gain.lna_db {
        args.push("-l".into());
        args.push(lna.to_string());
    }
    if let Some(vga) = gain.vga_db {
        args.push("-g".into());
        args.push(vga.to_string());
    }
    let serial = serial.trim();
    if !serial.is_empty() {
        args.push("-d".into());
        args.push(serial.to_string());
    }
    args
}

/// One dongle from `hackrf_info`. `board` is the name in parentheses, such as
/// `HackRF One`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dongle {
    pub index: Option<u32>,
    pub serial: String,
    pub board: String,
}

/// Serial lines from `hackrf_info`.
pub fn serials(text: &str) -> Vec<String> {
    dongles(text)
        .into_iter()
        .map(|dongle| dongle.serial)
        .collect()
}

/// Dongles printed by `hackrf_info`. A later `Index:` belongs to the next
/// serial, which is how the tool groups more than one board.
pub fn dongles(text: &str) -> Vec<Dongle> {
    let mut out = Vec::new();
    let mut serial: Option<String> = None;
    let mut board = String::new();
    let mut index: Option<u32> = None;
    let mut pending_index: Option<u32> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Index:") {
            pending_index = rest.trim().parse().ok();
            continue;
        }
        if let Some(rest) = line.strip_prefix("Serial number:") {
            push_dongle(&mut out, &mut serial, &mut board, &mut index);
            let serial_text = rest.trim();
            if !serial_text.is_empty() {
                serial = Some(serial_text.to_string());
                index = pending_index.take();
            }
            continue;
        }
        if serial.is_some() {
            if let Some(rest) = line.strip_prefix("Board ID Number:") {
                board = rest.trim().to_string();
            }
        }
    }
    push_dongle(&mut out, &mut serial, &mut board, &mut index);
    out
}

fn push_dongle(
    out: &mut Vec<Dongle>,
    serial: &mut Option<String>,
    board: &mut String,
    index: &mut Option<u32>,
) {
    let Some(serial) = serial.take() else {
        *board = String::new();
        *index = None;
        return;
    };
    out.push(Dongle {
        index: index.take(),
        serial,
        board: board_name(std::mem::take(board)),
    });
}

fn board_name(raw: String) -> String {
    let Some(open) = raw.find('(') else {
        return raw;
    };
    let Some(close) = raw[open + 1..].find(')') else {
        return raw;
    };
    let name = raw[open + 1..open + 1 + close].trim();
    if name.is_empty() {
        raw
    } else {
        name.to_string()
    }
}

/// Boards `hackrf_info` can see. Failure is a missing tool or a command
/// error that named no board. An open capture can still be listed.
pub fn list_dongles() -> Result<Vec<Dongle>, String> {
    let output = std::process::Command::new("hackrf_info")
        .output()
        .map_err(|err| format!("hackrf_info: {err}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut found = dongles(&stdout);
    if found.is_empty() {
        found = dongles(&stderr);
    }
    if found.is_empty() && !output.status.success() {
        let msg = stderr.trim();
        if msg.is_empty() {
            return Err("hackrf_info failed".into());
        }
        return Err(format!("hackrf_info: {msg}"));
    }
    Ok(found)
}

pub fn dongle_label(dongle: &Dongle) -> String {
    let board = if dongle.board.is_empty() {
        "HackRF".to_string()
    } else {
        dongle.board.clone()
    };
    match dongle.index {
        Some(index) => format!("{index}  {board}  {}", dongle.serial),
        None => format!("{board}  {}", dongle.serial),
    }
}

/// The device field, or the only dongle `hackrf_info` listed.
pub fn resolve_serial(device: &str, listed: &[String]) -> Result<String, String> {
    let device = device.trim();
    if !device.is_empty() {
        return Ok(device.to_string());
    }
    match listed {
        [one] => Ok(one.clone()),
        [] => Err("no HackRF".into()),
        _ => Err("set a HackRF serial".into()),
    }
}

struct Ddc {
    offset: f64,
    /// One-sample mixer rotation, cached from `offset`.
    rot_re: f32,
    rot_im: f32,
    osc_re: f32,
    osc_im: f32,
    acc_i: f32,
    acc_q: f32,
    filled: usize,
    /// Samples since the oscillator was pulled back to unit length.
    norm: usize,
    prev_i: f32,
    prev_q: f32,
    have_prev: bool,
    /// REC tap: when on, each half of the boxcar (250 samples, 32 ksps) is
    /// appended to `tap_out` as cs16. The discriminator is unchanged.
    tap_on: bool,
    half: (f32, f32),
    tap_out: Vec<i16>,
}

impl Ddc {
    fn new(offset: f64) -> Self {
        let mut ddc = Self {
            offset: f64::NAN,
            rot_re: 1.0,
            rot_im: 0.0,
            osc_re: 1.0,
            osc_im: 0.0,
            acc_i: 0.0,
            acc_q: 0.0,
            filled: 0,
            norm: 0,
            prev_i: 0.0,
            prev_q: 0.0,
            have_prev: false,
            tap_on: false,
            half: (0.0, 0.0),
            tap_out: Vec::new(),
        };
        ddc.set_offset(offset);
        ddc
    }

    fn set_offset(&mut self, offset: f64) {
        if self.offset.is_finite() && (self.offset - offset).abs() < 0.5 {
            return;
        }
        self.offset = offset;
        let step = -2.0 * PI * offset / SAMPLE_RATE as f64;
        self.rot_re = step.cos() as f32;
        self.rot_im = step.sin() as f32;
    }

    /// One baseband sample, or nothing until the boxcar fills.
    /// The mixer is a complex multiply. `sin` on every sample at 8 Msps
    /// fell behind the dongle and the stream died.
    fn push(&mut self, i: f32, q: f32) -> Option<f32> {
        let bi = i * self.osc_re - q * self.osc_im;
        let bq = i * self.osc_im + q * self.osc_re;
        let next_re = self.osc_re * self.rot_re - self.osc_im * self.rot_im;
        let next_im = self.osc_re * self.rot_im + self.osc_im * self.rot_re;
        self.osc_re = next_re;
        self.osc_im = next_im;
        self.norm += 1;
        if self.norm >= BOX {
            let mag = (self.osc_re * self.osc_re + self.osc_im * self.osc_im).sqrt();
            if mag > 1.0e-12 {
                self.osc_re /= mag;
                self.osc_im /= mag;
            } else {
                self.osc_re = 1.0;
                self.osc_im = 0.0;
            }
            self.norm = 0;
        }
        self.acc_i += bi;
        self.acc_q += bq;
        self.filled += 1;
        if self.tap_on && self.filled == hsrec::dsp::REC_BOX {
            self.half = (self.acc_i, self.acc_q);
            let n = hsrec::dsp::REC_BOX as f32;
            self.tap_out.push(hsrec::dsp::store_i16(self.acc_i / n));
            self.tap_out.push(hsrec::dsp::store_i16(self.acc_q / n));
        }
        if self.filled < BOX {
            return None;
        }
        if self.tap_on {
            let n = (BOX - hsrec::dsp::REC_BOX) as f32;
            self.tap_out.push(hsrec::dsp::store_i16((self.acc_i - self.half.0) / n));
            self.tap_out.push(hsrec::dsp::store_i16((self.acc_q - self.half.1) / n));
        }
        let mi = self.acc_i / BOX as f32;
        let mq = self.acc_q / BOX as f32;
        self.acc_i = 0.0;
        self.acc_q = 0.0;
        self.filled = 0;
        if !self.have_prev {
            self.prev_i = mi;
            self.prev_q = mq;
            self.have_prev = true;
            return None;
        }
        let cross = self.prev_i * mq - self.prev_q * mi;
        let dot = self.prev_i * mi + self.prev_q * mq;
        self.prev_i = mi;
        self.prev_q = mq;
        let hz = cross.atan2(dot) * (MID_RATE as f32) / (2.0 * PI as f32);
        Some(hz)
    }
}

fn to_i16(hz: f32) -> i16 {
    let sample = (hz / SCALE_HZ as f32 * SCALE_I16 as f32).round();
    sample.clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

/// Repeat each 16 kHz discriminator sample three times so the pipe is 48 kHz.
fn emit(hz: f32, out: &mut Vec<i16>) {
    let sample = to_i16(hz);
    out.push(sample);
    out.push(sample);
    out.push(sample);
}

/// One capture's discriminators. Voice output is silence while `voice` is none.
pub struct Bank {
    center: f64,
    dc: Dc,
    controls: Vec<Ddc>,
    voice: Ddc,
    voice_hz: Option<f64>,
    voice_steps: Vec<f32>,
    control_steps: Vec<Vec<f32>>,
}

impl Bank {
    pub fn new(center: f64, controls: &[f64]) -> Self {
        Self {
            center,
            dc: Dc::new(),
            controls: controls
                .iter()
                .map(|hz| Ddc::new(hz - center))
                .collect(),
            voice: Ddc::new(0.0),
            voice_hz: None,
            voice_steps: Vec::new(),
            control_steps: vec![Vec::new(); controls.len()],
        }
    }

    fn dc_mean(&self) -> (f32, f32) {
        (self.dc.i, self.dc.q)
    }

    /// Turn the REC tap on the voice mixer on or off.
    pub fn set_tap(&mut self, on: bool) {
        self.voice.tap_on = on;
        if !on {
            self.voice.tap_out.clear();
        }
    }

    /// Channelised IQ since the last call (32 ksps cs16 at the voice frequency).
    pub fn take_tap(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.voice.tap_out)
    }

    pub fn set_voice(&mut self, hz: Option<f64>) {
        self.voice_hz = hz.filter(|hz| inside(self.center, *hz));
        let offset = self.voice_hz.unwrap_or(self.center) - self.center;
        self.voice.set_offset(offset);
    }

    /// Signed 8-bit interleaved IQ. Each control gets its own buffer.
    /// The voice buffer is the same length, zeros when no voice frequency is set.
    pub fn push(&mut self, iq: &[u8], voice: &mut Vec<i16>, controls: &mut [Vec<i16>]) {
        let n = iq.len() / 2;
        self.voice_steps.clear();
        for steps in &mut self.control_steps {
            steps.clear();
        }
        for k in 0..n {
            let i = iq[k * 2] as i8 as f32;
            let q = iq[k * 2 + 1] as i8 as f32;
            let (i, q) = self.dc.apply(i, q);
            if let Some(hz) = self.voice.push(i, q) {
                self.voice_steps.push(hz);
            }
            for (index, ddc) in self.controls.iter_mut().enumerate() {
                if let Some(hz) = ddc.push(i, q) {
                    self.control_steps[index].push(hz);
                }
            }
        }
        let silent = self.voice_hz.is_none();
        for hz in &self.voice_steps {
            if silent {
                voice.extend_from_slice(&[0, 0, 0]);
            } else {
                emit(*hz, voice);
            }
        }
        for (index, steps) in self.control_steps.iter().enumerate() {
            if let Some(out) = controls.get_mut(index) {
                for hz in steps {
                    emit(*hz, out);
                }
            }
        }
    }
}

struct Decoder {
    child: Child,
    port: u16,
    lines: Arc<Mutex<Vec<String>>>,
}

pub struct Session {
    transfer: Child,
    /// Absent until the radio's receive node is in the graph. The capture
    /// still runs, so a control channel can be read before the speaker is up.
    play: Option<Child>,
    /// Synthesized voice played to the radio's speaker. Control decoders are
    /// not played. Absent when that speaker has no name. The capture still runs.
    speech: Option<Speech>,
    controls: Vec<Decoder>,
    voice_dec: Option<Decoder>,
    /// Voice decoders replaced after the site word arrived. Reaped without
    /// waiting, so a new site does not stall the capture.
    voice_retired: Vec<Decoder>,
    /// Next voice-decoder stdin. The worker takes this after the previous
    /// pipe closes. Closing that pipe must not stop the USB capture.
    voice_next: Arc<Mutex<Option<std::io::PipeWriter>>>,
    /// Trunk script path. Empty when this capture has no control list.
    script: Option<String>,
    /// Site word already given to the voice decoder. A repeat is not a restart.
    p2_word: Option<String>,
    /// Key CSV already given to the voice decoder. Empty means no `-K`.
    /// A change replaces the voice process and leaves the control decoders.
    key_csv: Option<String>,
    voice_hz: Arc<AtomicU64>,
    /// Radio id the REC tap files its IQ under.
    rec_key: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    transfer_log: Arc<Mutex<Vec<String>>>,
    transfer_reader: Option<JoinHandle<()>>,
    pub center: f64,
    pub control_hz: Vec<f64>,
    pub serial: String,
    pub gain: RxGain,
    spec: Arc<Mutex<SpectrumSnap>>,
}

impl Session {
    pub fn set_rec_key(&self, id: u64) {
        self.rec_key.store(id, Ordering::Relaxed);
    }

    pub fn set_voice(&self, hz: u64) {
        self.voice_hz.store(hz, Ordering::Relaxed);
    }

    /// Give the voice decoder the site word. Control decoders are left as
    /// they are: `-X` would freeze their site, and restarting them drops
    /// the identifier table. The voice process never hears the control
    /// channel, so it cannot learn WACN, system id, and NAC on its own.
    pub fn set_p2_site(&mut self, word: &str) -> Result<(), String> {
        self.reap_retired();
        let word = word.trim();
        if word.is_empty() || self.p2_word.as_deref() == Some(word) {
            return Ok(());
        }
        if !word.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err("site parameter is not hex".into());
        }
        let Some(script) = self.script.clone() else {
            return Ok(());
        };
        let (dec, write, audio) = spawn_decoder(
            &script,
            Some(word),
            self.speech.is_some(),
            self.key_csv.as_deref(),
        )?;
        if let Some(audio) = audio {
            if let Some(speech) = &self.speech {
                speech.push(audio);
            } else {
                thread::spawn(move || discard_audio(audio));
            }
        }
        {
            let mut pending = self.voice_next.lock().expect("hackrf voice");
            drop(pending.replace(write));
        }
        if let Some(mut old) = self.voice_dec.take() {
            signal(&mut old.child);
            self.voice_retired.push(old);
        }
        self.voice_dec = Some(dec);
        self.p2_word = Some(word.to_string());
        Ok(())
    }

    /// Give the voice decoder a new key file. Control decoders stay up and
    /// read the file the next time this capture starts. An empty path drops
    /// `-K`. The same path is not a restart.
    pub fn set_keys(&mut self, path: &str) -> Result<(), String> {
        self.reap_retired();
        let next = {
            let path = path.trim();
            if path.is_empty() {
                None
            } else {
                Some(path.to_string())
            }
        };
        if self.key_csv == next {
            return Ok(());
        }
        let Some(script) = self.script.clone() else {
            self.key_csv = next;
            return Ok(());
        };
        let site = self.p2_word.clone();
        let (dec, write, audio) = spawn_decoder(
            &script,
            site.as_deref(),
            self.speech.is_some(),
            next.as_deref(),
        )?;
        if let Some(audio) = audio {
            if let Some(speech) = &self.speech {
                speech.push(audio);
            } else {
                thread::spawn(move || discard_audio(audio));
            }
        }
        {
            let mut pending = self.voice_next.lock().expect("hackrf voice");
            drop(pending.replace(write));
        }
        if let Some(mut old) = self.voice_dec.take() {
            signal(&mut old.child);
            self.voice_retired.push(old);
        }
        self.voice_dec = Some(dec);
        self.key_csv = next;
        Ok(())
    }

    fn reap_retired(&mut self) {
        let mut still = Vec::new();
        for mut dec in self.voice_retired.drain(..) {
            match dec.child.try_wait() {
                Ok(None) => still.push(dec),
                _ => crate::rx_pipe::release_voice_port(dec.port),
            }
        }
        self.voice_retired = still;
    }

    /// Latest spectrum frame. Empty until the capture has produced one.
    pub fn spectrum(&self) -> SpectrumSnap {
        self.spec.lock().expect("hackrf spectrum").clone()
    }

    /// One buffer per control decoder. Lines from different towers stay apart
    /// so a grant header on one tower cannot take a frequency from another.
    pub fn drain_controls(&self) -> Vec<Vec<String>> {
        self.controls
            .iter()
            .map(|dec| drain_one(&dec.lines))
            .collect()
    }

    pub fn drain_voice(&self) -> Vec<String> {
        self.voice_dec
            .as_ref()
            .map(|dec| drain_one(&dec.lines))
            .unwrap_or_default()
    }

    pub fn playing(&self) -> bool {
        self.play.is_some()
    }

    pub fn child_ids(&self) -> Vec<u32> {
        let mut ids = vec![self.transfer.id()];
        if let Some(play) = &self.play {
            ids.push(play.id());
        }
        if let Some(speech) = &self.speech {
            ids.push(speech.play.id());
        }
        for dec in &self.controls {
            ids.push(dec.child.id());
        }
        if let Some(dec) = &self.voice_dec {
            ids.push(dec.child.id());
        }
        for dec in &self.voice_retired {
            ids.push(dec.child.id());
        }
        ids
    }

    pub fn alive(&mut self) -> bool {
        self.reap_retired();
        if self.failed.load(Ordering::Relaxed) {
            return false;
        }
        matches!(self.transfer.try_wait(), Ok(None))
    }

    /// Stop the capture. The returned text is what `hackrf_transfer` printed.
    pub fn stop(&mut self) -> String {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(speech) = &self.speech {
            speech.wake();
        }
        signal(&mut self.transfer);
        if let Some(play) = &mut self.play {
            signal(play);
        }
        for dec in &mut self.controls {
            signal(&mut dec.child);
        }
        if let Some(dec) = &mut self.voice_dec {
            signal(&mut dec.child);
        }
        for dec in &mut self.voice_retired {
            signal(&mut dec.child);
        }
        if let Some(speech) = &mut self.speech {
            speech.shutdown();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        drop(self.voice_next.lock().expect("hackrf voice").take());
        wait_child(&mut self.transfer);
        if let Some(play) = &mut self.play {
            wait_child(play);
        }
        for dec in &mut self.controls {
            wait_child(&mut dec.child);
            crate::rx_pipe::release_voice_port(dec.port);
        }
        if let Some(dec) = &mut self.voice_dec {
            wait_child(&mut dec.child);
            crate::rx_pipe::release_voice_port(dec.port);
        }
        for dec in &mut self.voice_retired {
            wait_child(&mut dec.child);
            crate::rx_pipe::release_voice_port(dec.port);
        }
        if let Some(reader) = self.transfer_reader.take() {
            let _ = reader.join();
        }
        drain_one(&self.transfer_log).join(" ")
    }
}

fn drain_one(lines: &Mutex<Vec<String>>) -> Vec<String> {
    std::mem::take(&mut *lines.lock().expect("hackrf lines"))
}

fn signal(child: &mut Child) {
    let pid = child.id() as i32;
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
}

fn wait_child(child: &mut Child) {
    crate::cat::stop_child(child);
}

/// Start the capture. `controls` empty means one discriminator at `center`
/// and no trunk scripts. A non-empty list also starts one P25 script per
/// control and one for the voice channel. The voice script stays up when
/// its frequency changes. `p2` is the site word for that voice process only.
/// Control processes are started without it so they can still learn a new site.
/// `speaker` receives the voice decoder's synthesized audio. Control audio
/// stays off that node. An empty speaker leaves the voice script's audio
/// unread by a player; the decoder still runs.
pub fn start(
    serial: &str,
    center: f64,
    controls: &[f64],
    play_node: Option<&str>,
    speaker: Option<&str>,
    script: Option<&str>,
    p2: Option<&str>,
    key_csv: Option<&str>,
    gain: RxGain,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<Session, String> {
    if !center.is_finite() || center < 1_000_000.0 {
        return Err("tune a frequency".into());
    }
    let args = transfer_args(serial, center, gain);
    let mut transfer_cmd = Command::new("hackrf_transfer");
    transfer_cmd
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut transfer = transfer_cmd
        .spawn()
        .map_err(|err| format!("hackrf_transfer: {err}"))?;
    let iq = match transfer.stdout.take() {
        Some(iq) => iq,
        None => {
            fail_transfer(&mut transfer);
            return Err("hackrf_transfer has no stdout".into());
        }
    };
    let transfer_log = Arc::new(Mutex::new(Vec::new()));
    let transfer_reader = transfer.stderr.take().map(|stderr| {
        let sink = Arc::clone(&transfer_log);
        thread::spawn(move || collect(stderr, sink))
    });

    let mut play = None;
    let mut play_write = None;
    if let Some(play_node) = play_node {
        let (play_read, write) = match std::io::pipe() {
            Ok(pair) => pair,
            Err(err) => {
                fail_transfer(&mut transfer);
                return Err(format!("hackrf play: {err}"));
            }
        };
        let mut play_cmd = Command::new("pw-cat");
        // FL is the radio card. MONO attaches the USB microphone.
        play_cmd
            .args(crate::rx_pipe::routed_play_args(play_node))
            .stdin(Stdio::from(play_read))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        match play_cmd.spawn() {
            Ok(child) => {
                play = Some(child);
                play_write = Some(write);
            }
            Err(err) => {
                fail_transfer(&mut transfer);
                return Err(format!("hackrf play: {err}"));
            }
        }
    }

    let follow = !controls.is_empty();
    let mut decoders = Vec::new();
    let mut control_writes = Vec::new();
    let mut voice_dec = None;
    let mut voice_write = None;
    let mut speech = None;
    let voice_hz = Arc::new(AtomicU64::new(0));
    let site = p2
        .map(str::trim)
        .filter(|word| !word.is_empty() && word.chars().all(|ch| ch.is_ascii_hexdigit()));
    let key_csv = key_csv
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string);
    let script_owned = if follow { script.map(str::to_string) } else { None };
    let speaker = speaker.map(str::trim).filter(|name| !name.is_empty());
    if let Some(script) = script_owned.as_deref() {
        for _ in controls {
            match spawn_decoder(script, None, false, key_csv.as_deref()) {
                Ok((dec, write, _)) => {
                    decoders.push(dec);
                    control_writes.push(write);
                }
                Err(err) => {
                    fail_partial(&mut transfer, play.as_mut(), &mut decoders);
                    return Err(err);
                }
            }
        }
        match spawn_decoder(script, site, speaker.is_some(), key_csv.as_deref()) {
            Ok((dec, write, audio)) => {
                voice_dec = Some(dec);
                voice_write = Some(write);
                if let Some(audio) = audio {
                    if let Some(name) = speaker {
                        if let Ok(player) = Speech::spawn(name, audio, Arc::clone(&voice_hz)) {
                            speech = Some(player);
                        }
                    } else {
                        thread::spawn(move || discard_audio(audio));
                    }
                }
            }
            Err(err) => {
                fail_partial(&mut transfer, play.as_mut(), &mut decoders);
                return Err(err);
            }
        }
    }

    let stop = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let voice_flag = Arc::clone(&voice_hz);
    let stop_flag = Arc::clone(&stop);
    let failed_flag = Arc::clone(&failed);
    let control_hz = controls.to_vec();
    let spec = Arc::new(Mutex::new(SpectrumSnap::default()));
    let pump = SpecPump {
        since: 0,
        seq: 0,
        slot: Arc::clone(&spec),
        wake,
    };
    let voice_next = Arc::new(Mutex::new(None));
    let voice_slot = Arc::clone(&voice_next);
    let rec_key = Arc::new(AtomicU64::new(0));
    let rec_flag = Arc::clone(&rec_key);
    let worker = thread::spawn(move || {
        run_bank(
            iq,
            play_write,
            control_writes,
            voice_write,
            voice_slot,
            center,
            &control_hz,
            voice_flag,
            stop_flag,
            failed_flag,
            pump,
            rec_flag,
        )
    });
    Ok(Session {
        transfer,
        play,
        speech,
        controls: decoders,
        voice_dec,
        voice_retired: Vec::new(),
        voice_next,
        script: script_owned,
        p2_word: site.map(str::to_string),
        key_csv,
        voice_hz,
        rec_key,
        stop,
        failed,
        worker: Some(worker),
        transfer_log,
        transfer_reader,
        center,
        control_hz: controls.to_vec(),
        serial: serial.trim().to_string(),
        gain,
        spec,
    })
}

fn fail_transfer(transfer: &mut Child) {
    signal(transfer);
    wait_child(transfer);
}

fn fail_partial(transfer: &mut Child, mut play: Option<&mut Child>, decoders: &mut [Decoder]) {
    signal(transfer);
    if let Some(play) = play.as_mut() {
        signal(play);
    }
    for dec in decoders.iter_mut() {
        signal(&mut dec.child);
    }
    wait_child(transfer);
    if let Some(play) = play {
        wait_child(play);
    }
    for dec in decoders.iter_mut() {
        wait_child(&mut dec.child);
        crate::rx_pipe::release_voice_port(dec.port);
    }
}

fn run_bank(
    mut iq: impl Read,
    mut play: Option<std::io::PipeWriter>,
    mut controls: Vec<std::io::PipeWriter>,
    mut voice: Option<std::io::PipeWriter>,
    voice_next: Arc<Mutex<Option<std::io::PipeWriter>>>,
    center: f64,
    control_hz: &[f64],
    voice_hz: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    mut spec: SpecPump,
    rec_key: Arc<AtomicU64>,
) {
    let mut bank = Bank::new(center, control_hz);
    // A full playback pipe must not stall this read. hackrf_transfer stops
    // the USB stream when its stdout blocks, and the scope then sticks on
    // the last frame.
    if let Some(play) = play.as_ref() {
        unblock(play);
    }
    if let Some(write) = voice.as_ref() {
        unblock(write);
    }
    for write in &controls {
        unblock(write);
    }
    let mut buf = [0u8; 16_384];
    let mut voice_out = Vec::new();
    let mut control_out: Vec<Vec<i16>> = vec![Vec::new(); control_hz.len()];
    while !stop.load(Ordering::Relaxed) {
        let n = match iq.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let hz = voice_hz.load(Ordering::Relaxed);
        bank.set_voice(if hz == 0 { None } else { Some(hz as f64) });
        voice_out.clear();
        for out in &mut control_out {
            out.clear();
        }
        let tap = hz != 0 && crate::rec_live::tap_on();
        bank.set_tap(tap);
        bank.push(&buf[..n], &mut voice_out, &mut control_out);
        if tap {
            let data = bank.take_tap();
            if !data.is_empty() {
                crate::rec_live::push(rec_key.load(Ordering::Relaxed), hz, data);
            }
        }
        let (mean_i, mean_q) = bank.dc_mean();
        spec.push(&buf[..n], center, mean_i, mean_q);
        if let Some(play) = play.as_mut() {
            if write_pcm(play, &voice_out).is_err() {
                break;
            }
        }
        // A closed voice pipe is the site-word replace. The control pipes
        // and the USB capture stay up. The identifier table lives there.
        if let Some(write) = voice.as_mut() {
            if write_pcm(write, &voice_out).is_err() {
                voice = None;
            }
        }
        if voice.is_none() {
            if let Some(next) = voice_next.lock().expect("hackrf voice").take() {
                unblock(&next);
                voice = Some(next);
            }
        }
        let mut broken = false;
        for (write, samples) in controls.iter_mut().zip(control_out.iter()) {
            if write_pcm(write, samples).is_err() {
                broken = true;
                break;
            }
        }
        if broken {
            break;
        }
    }
    if !stop.load(Ordering::Relaxed) {
        failed.store(true, Ordering::Relaxed);
    }
}

fn unblock(writer: &impl AsFd) {
    let fd = writer.as_fd().as_raw_fd();
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

/// Write one PCM chunk. `WouldBlock` drops the chunk and leaves the capture
/// running. A closed pipe is still a failure.
fn write_pcm(out: &mut impl Write, samples: &[i16]) -> std::io::Result<()> {
    if samples.is_empty() {
        return Ok(());
    }
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    let mut off = 0;
    while off < bytes.len() {
        match out.write(&bytes[off..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "pcm closed",
                ));
            }
            Ok(n) => off += n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn spawn_decoder(
    script: &str,
    site: Option<&str>,
    capture_audio: bool,
    key_csv: Option<&str>,
) -> Result<(Decoder, std::io::PipeWriter, Option<ChildStdout>), String> {
    let port = crate::rx_pipe::claim_voice_port()?;
    let (read, write) = match std::io::pipe() {
        Ok(pair) => pair,
        Err(err) => {
            crate::rx_pipe::release_voice_port(port);
            return Err(format!("hackrf decoder: {err}"));
        }
    };
    let argv = match crate::rx_pipe::command_argv(script, port) {
        Ok(argv) => argv,
        Err(err) => {
            crate::rx_pipe::release_voice_port(port);
            return Err(format!("hackrf decoder: {err}"));
        }
    };
    let lines = Arc::new(Mutex::new(Vec::new()));
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .env("HAMSPOT_CC_EVENTS", "1")
        .stdin(Stdio::from(read))
        .stdout(if capture_audio {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(site) = site {
        // dsd-fme -X. WACN, system id, and NAC. Not a key. Control
        // decoders pass None so a later site can still update them.
        cmd.env("HAMSPOT_P25_X", site);
    }
    if let Some(path) = key_csv.map(str::trim).filter(|path| !path.is_empty()) {
        cmd.env("HAMSPOT_P25_K", path);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            crate::rx_pipe::release_voice_port(port);
            return Err(format!("hackrf decoder: {err}"));
        }
    };
    if let Some(stderr) = child.stderr.take() {
        let sink = Arc::clone(&lines);
        thread::spawn(move || collect(stderr, sink));
    }
    let audio = if capture_audio {
        match child.stdout.take() {
            Some(audio) => Some(audio),
            None => {
                // Piped stdout is how this process was spawned. Missing it
                // means the pipe was not created. Leave no orphan decoder.
                signal(&mut child);
                wait_child(&mut child);
                crate::rx_pipe::release_voice_port(port);
                return Err("hackrf voice has no stdout".into());
            }
        }
    } else {
        None
    };
    Ok((
        Decoder {
            child,
            port,
            lines,
        },
        write,
        audio,
    ))
}

/// Voice script stdout copied to the speaker. A site-word swap hands in the
/// replacement stdout. The USB worker is not this thread.
struct Speech {
    play: Child,
    stop: Arc<AtomicBool>,
    slot: Arc<SpeechSlot>,
    worker: Option<JoinHandle<()>>,
}

struct SpeechSlot {
    next: Mutex<Option<ChildStdout>>,
    cv: Condvar,
}

impl Speech {
    fn spawn(speaker: &str, audio: ChildStdout, voice_hz: Arc<AtomicU64>) -> Result<Self, String> {
        let (read, write) = match std::io::pipe() {
            Ok(pair) => pair,
            Err(err) => {
                thread::spawn(move || discard_audio(audio));
                return Err(format!("hackrf voice play: {err}"));
            }
        };
        let mut play_cmd = Command::new("pw-cat");
        play_cmd
            .args(crate::rx_pipe::speaker_play_args(speaker))
            .stdin(Stdio::from(read))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        let play = match play_cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                thread::spawn(move || discard_audio(audio));
                return Err(format!("hackrf voice play: {err}"));
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let slot = Arc::new(SpeechSlot {
            next: Mutex::new(Some(audio)),
            cv: Condvar::new(),
        });
        let stop_flag = Arc::clone(&stop);
        let slot_flag = Arc::clone(&slot);
        let worker = thread::spawn(move || pump_speech(write, voice_hz, stop_flag, slot_flag));
        Ok(Self {
            play,
            stop,
            slot,
            worker: Some(worker),
        })
    }

    fn push(&self, audio: ChildStdout) {
        let mut guard = self.slot.next.lock().expect("hackrf speech");
        *guard = Some(audio);
        self.slot.cv.notify_all();
    }

    fn wake(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.slot.cv.notify_all();
    }

    /// The voice decoder has already been signaled, so a blocked read ends.
    fn shutdown(&mut self) {
        self.wake();
        signal(&mut self.play);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        wait_child(&mut self.play);
    }
}

fn pump_speech(
    mut write: std::io::PipeWriter,
    voice_hz: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    slot: Arc<SpeechSlot>,
) {
    let mut current: Option<ChildStdout> = None;
    // A closed speaker must not close the decoder pipe. Dropping that pipe
    // makes sox exit, and the voice script exits with it.
    let mut live = true;
    let mut buf = [0u8; 4096];
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if current.is_none() {
            let mut guard = slot.next.lock().expect("hackrf speech");
            while guard.is_none() && !stop.load(Ordering::Relaxed) {
                guard = slot.cv.wait(guard).expect("hackrf speech");
            }
            if stop.load(Ordering::Relaxed) {
                return;
            }
            current = guard.take();
        }
        let Some(src) = current.as_mut() else {
            return;
        };
        match src.read(&mut buf) {
            Ok(0) => current = None,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => current = None,
            Ok(n) => {
                if !live || voice_hz.load(Ordering::Relaxed) == 0 {
                    continue;
                }
                if write.write_all(&buf[..n]).is_err() {
                    live = false;
                }
            }
        }
    }
}

fn discard_audio(mut audio: ChildStdout) {
    let mut buf = [0u8; 4096];
    loop {
        match audio.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
    }
}

fn collect(stderr: impl Read, sink: Arc<Mutex<Vec<String>>>) {
    let mut reader = std::io::BufReader::new(stderr);
    let mut line = String::new();
    loop {
        line.clear();
        match std::io::BufRead::read_line(&mut reader, &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let text = line.trim_end();
                if crate::keys::is_key_dump(text) {
                    continue;
                }
                let mut guard = sink.lock().expect("hackrf lines");
                guard.push(text.to_string());
                if guard.len() > 400 {
                    let extra = guard.len() - 400;
                    guard.drain(0..extra);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rec_tap_replays_like_live_voice() {
        // Synthetic FM 1 kHz tone, 2.5 kHz deviation, 60 kHz above centre.
        let center = 851_000_000.0;
        let cs8 = hsrec::dsp::synth_fm_cs8(60_000.0, 1_000.0, 2_500.0, 0.2, 100.0);
        let mut bank = Bank::new(center, &[]);
        bank.set_voice(Some(center + 60_000.0));
        bank.set_tap(true);
        let mut live = Vec::new();
        let mut tap = Vec::new();
        for block in cs8.chunks(16_384) {
            bank.push(block, &mut live, &mut []);
            tap.extend(bank.take_tap());
        }
        // 32 ksps complex: two halves per live 16 kHz sample.
        assert_eq!(tap.len() / 2, live.len() / 3 * 2 + 2);
        let mut replay = Vec::new();
        hsrec::dsp::Replayer::default().push(&tap, &mut replay);
        let skip = 4_800;
        let n = replay.len().min(live.len()) - skip;
        let worst = live[skip..skip + n]
            .iter()
            .zip(&replay[skip..skip + n])
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .max()
            .unwrap();
        // Same discriminator; only cs16 rounding differs.
        assert!(worst < 120, "max diff {worst}");
        let hz = hsrec::dsp::zero_cross_hz(&replay[skip..], 48_000.0);
        assert!((hz - 1_000.0).abs() < 30.0, "{hz}");
        bank.set_tap(false);
        bank.push(&cs8[..16_384], &mut live, &mut []);
        assert!(bank.take_tap().is_empty());
    }

    use super::*;

    #[test]
    fn the_widest_cluster_that_fits_sets_the_center() {
        let (center, kept) = place(&[
            851_000_000.0,
            852_000_000.0,
            870_000_000.0,
            853_000_000.0,
        ])
        .expect("cluster");
        assert_eq!(kept, vec![851_000_000.0, 852_000_000.0, 853_000_000.0]);
        assert!((center - 852_000_000.0).abs() < 1.0);
        assert!(!kept.contains(&870_000_000.0));
    }

    #[test]
    fn an_empty_list_has_no_capture() {
        assert!(place(&[]).is_none());
        assert!(place(&[f64::NAN, -1.0]).is_none());
    }

    #[test]
    fn a_gain_stage_steps_by_one_notch_and_stops_at_the_end() {
        assert_eq!(step_stage(40, 1, LNA_STEP, LNA_MAX), 40);
        assert_eq!(step_stage(40, -1, LNA_STEP, LNA_MAX), 32);
        assert_eq!(step_stage(0, -1, LNA_STEP, LNA_MAX), 0);
        assert_eq!(step_stage(0, 1, VGA_STEP, VGA_MAX), 2);
        assert_eq!(step_stage(62, 1, VGA_STEP, VGA_MAX), 62);
        assert_eq!(step_stage(39, 0, LNA_STEP, LNA_MAX), 32);
        assert_eq!(step_stage(39, -1, LNA_STEP, LNA_MAX), 24);
        assert_eq!(step_stage(3, 0, VGA_STEP, VGA_MAX), 2);
        assert_eq!(snap_lna(40), 40);
        assert_eq!(snap_vga(0), 0);
    }

    #[test]
    fn a_talkgroup_is_on_when_its_zone_or_its_number_is() {
        assert!(tg_selected(Some(1), 100, &[1], &[]));
        assert!(tg_selected(Some(2), 100, &[], &[100]));
        assert!(!tg_selected(Some(2), 100, &[1], &[200]));
        assert!(!tg_selected(None, 100, &[0], &[]));
        assert!(tg_selected(Some(2), 100, &[], &[]));
        assert!(tg_selected(None, 100, &[], &[]));
    }

    #[test]
    fn voice_follows_a_clear_selected_grant_inside_the_window() {
        let center = 852_000_000.0;
        assert_eq!(
            grant_voice(center, 853_000_000.0, CLEAR_ALG, true),
            Some(853_000_000.0)
        );
        assert!(grant_voice(center, 853_000_000.0, 0x84, true).is_none());
        assert!(grant_voice(center, 853_000_000.0, CLEAR_ALG, false).is_none());
        assert!(grant_voice(center, 860_000_000.0, CLEAR_ALG, true).is_none());
    }

    fn cluster() -> (f64, Vec<f64>) {
        let controls = vec![854_437_500.0, 859_962_500.0];
        place(&controls).expect("cluster")
    }

    /// One HackRF update the way `service_hackrf` runs it.
    fn wide_update(
        hold: &mut crate::follow::VoiceHold,
        state: &mut (Option<(f64, u32)>, Vec<(f64, u32, u64)>, Option<VoiceAway>, u64),
        fresh: &[(f64, u32)],
        emergencies: &[(f64, u32)],
        events: &[crate::follow::CcEvent],
        now: std::time::Instant,
    ) -> FollowCapture {
        use crate::follow::CcEvent;
        let (center, controls) = cluster();
        let mut candidates: Vec<(f64, u32, bool)> =
            fresh.iter().map(|(hz, tg)| (*hz, *tg, false)).collect();
        candidates.extend(emergencies.iter().map(|(hz, tg)| (*hz, *tg, true)));
        let step = crate::follow::wide_hold_step(
            hold, state.0, events, false, &candidates, &|_| true, now,
        );
        let saw = |f: fn(&CcEvent) -> bool| events.iter().any(f);
        let mut open = std::mem::take(&mut state.1);
        let (fresh, a, b, c, d) = hold_inputs(
            &step,
            fresh.to_vec(),
            (
                saw(|e| matches!(e, CcEvent::Sync)),
                saw(|e| matches!(e, CcEvent::SyncLost)),
                saw(|e| matches!(e, CcEvent::End)),
                saw(|e| matches!(e, CcEvent::Search)),
            ),
            &mut open,
            state.3,
        );
        let picked = follow_capture(
            center, &controls, None, true, true, open, state.3, state.0, &fresh, state.2, a, b, c, d,
        );
        if picked.call != state.0 {
            if picked.call.is_some() { hold.begin(now); } else { hold.clear(); }
        }
        state.0 = picked.call;
        state.1 = picked.open.clone();
        state.2 = picked.away;
        state.3 = picked.seq;
        picked
    }

    fn ms(n: u64) -> std::time::Duration {
        std::time::Duration::from_millis(n)
    }

    #[test]
    fn wide_follow_holds_a_call_against_new_grants() {
        use crate::follow::{CcEvent, VoiceHold};
        let t = std::time::Instant::now();
        let mut hold = VoiceHold::default();
        let mut state = (None, Vec::new(), None, 0);
        let a = 858_512_500.0;
        let b = 857_000_000.0;
        wide_update(&mut hold, &mut state, &[(a, 100)], &[], &[], t);
        assert_eq!(state.0, Some((a, 100)));
        let picked = wide_update(&mut hold, &mut state, &[(b, 200)], &[], &[CcEvent::Voice], t + ms(100));
        assert_eq!(picked.call, Some((a, 100)));
        assert_eq!(picked.voice_hz, a.round() as u64);
        // Control decoders keep running: the cluster capture is unchanged.
        let (_, controls) = cluster();
        assert_eq!(picked.controls, controls);
    }

    #[test]
    fn wide_follow_holds_through_a_butt_key_and_a_dead_carrier() {
        use crate::follow::{CcEvent, VoiceHold};
        let t = std::time::Instant::now();
        let mut hold = VoiceHold::default();
        let mut state = (None, Vec::new(), None, 0);
        let a = 858_512_500.0;
        wide_update(&mut hold, &mut state, &[(a, 100)], &[], &[], t);
        wide_update(&mut hold, &mut state, &[], &[], &[CcEvent::Voice], t + ms(50));
        let p = wide_update(&mut hold, &mut state, &[], &[], &[CcEvent::End, CcEvent::SyncLost], t + ms(100));
        assert_eq!(p.call, Some((a, 100)));
        assert!(hold.in_hang());
        let p = wide_update(&mut hold, &mut state, &[], &[], &[CcEvent::Voice], t + ms(400));
        assert_eq!(p.call, Some((a, 100)));
        assert!(!hold.in_hang());
        let p = wide_update(&mut hold, &mut state, &[], &[], &[CcEvent::Search], t + ms(500));
        assert_eq!(p.call, Some((a, 100)));
        let p = wide_update(&mut hold, &mut state, &[], &[], &[], t + ms(999));
        assert_eq!(p.call, Some((a, 100)));
        let p = wide_update(&mut hold, &mut state, &[], &[], &[], t + ms(1000));
        assert_eq!(p.call, None);
        assert_eq!(p.voice_hz, 0);
    }

    #[test]
    fn wide_follow_releases_sync_without_voice_after_the_limit() {
        use crate::follow::{CcEvent, VoiceHold};
        let t = std::time::Instant::now();
        let mut hold = VoiceHold::default();
        let mut state = (None, Vec::new(), None, 0);
        let a = 858_512_500.0;
        wide_update(&mut hold, &mut state, &[(a, 100)], &[], &[], t);
        let p = wide_update(&mut hold, &mut state, &[], &[], &[CcEvent::Sync], t + ms(5900));
        assert_eq!(p.call, Some((a, 100)));
        let p = wide_update(&mut hold, &mut state, &[], &[], &[CcEvent::Sync], t + ms(6000));
        assert_eq!(p.call, None);
    }

    #[test]
    fn wide_follow_switches_for_an_emergency_and_logs_it() {
        use crate::follow::{CcEvent, VoiceHold};
        let t = std::time::Instant::now();
        let mut hold = VoiceHold::default();
        let mut state = (None, Vec::new(), None, 0);
        let a = 858_512_500.0;
        let e = 857_000_000.0;
        wide_update(&mut hold, &mut state, &[(a, 100)], &[], &[], t);
        // The emergency talkgroup is not monitored: it is only in the
        // emergency list, not the selected grants.
        let p = wide_update(&mut hold, &mut state, &[], &[(e, 777)], &[CcEvent::Voice], t + ms(10));
        assert_eq!(p.call, Some((e, 777)));
        assert_eq!(p.voice_hz, e.round() as u64);
        let notes = hold.take_overrides();
        assert!(notes.len() == 1 && notes[0].contains("emergency override"));
        let p = wide_update(&mut hold, &mut state, &[], &[], &[], t + ms(20));
        assert_eq!(p.call, Some((e, 777)));
    }

    #[test]
    fn wide_follow_priority_overrides_and_the_option_off_blocks_it() {
        use crate::follow::VoiceHold;
        let t = std::time::Instant::now();
        let a = 858_512_500.0;
        let pr = 857_000_000.0;
        let mut hold = VoiceHold::default();
        hold.set_override(true, vec![555]);
        let mut state = (None, Vec::new(), None, 0);
        wide_update(&mut hold, &mut state, &[(a, 100)], &[], &[], t);
        let p = wide_update(&mut hold, &mut state, &[(pr, 555)], &[], &[], t + ms(10));
        assert_eq!(p.call, Some((pr, 555)));

        let mut hold = VoiceHold::default();
        hold.set_override(false, vec![555]);
        let mut state = (None, Vec::new(), None, 0);
        wide_update(&mut hold, &mut state, &[(a, 100)], &[], &[], t);
        let p = wide_update(&mut hold, &mut state, &[(pr, 555)], &[(pr, 777)], &[], t + ms(10));
        assert_eq!(p.call, Some((a, 100)));
        assert!(hold.take_overrides().is_empty());
    }

    #[test]
    fn an_in_window_grant_keeps_the_control_cluster() {
        let (center, controls) = cluster();
        let voice = 858_512_500.0;
        assert!(inside(center, voice));
        let picked = follow_capture(
            center,
            &controls,
            None,
            true,
            true,
            Vec::new(),
            0,
            None,
            &[(voice, 100)],
            None,
            false,
            false,
            false,
            false,
        );
        assert!(!picked.left);
        assert_eq!(picked.controls, controls);
        assert!((picked.center - center).abs() < 1.0);
        assert_eq!(picked.voice_hz, voice.round() as u64);
        assert!(picked.away.is_none());
    }

    #[test]
    fn a_far_grant_drops_the_control_channel_until_the_call_ends() {
        let (center, controls) = cluster();
        let voice = 851_975_000.0;
        assert!(!inside(center, voice));
        let picked = follow_capture(
            center,
            &controls,
            None,
            true,
            true,
            Vec::new(),
            0,
            None,
            &[(voice, 100)],
            None,
            false,
            false,
            false,
            false,
        );
        assert!(picked.left);
        assert_eq!(picked.center, voice);
        assert_eq!(picked.controls, vec![voice]);
        assert_eq!(picked.voice_hz, voice.round() as u64);
        let away = picked.away.expect("away");
        assert_eq!(away.tg, 100);
        assert!(!away.synced);

        let back = follow_capture(
            center,
            &controls,
            None,
            true,
            true,
            picked.open,
            picked.seq,
            picked.call,
            &[],
            picked.away,
            false,
            false,
            true,
            false,
        );
        assert!(!back.left);
        assert!((back.center - center).abs() < 1.0);
        assert_eq!(back.controls, controls);
        assert_eq!(back.voice_hz, 0);
        assert!(back.away.is_none());
    }

    #[test]
    fn a_far_grant_stays_put_when_the_radio_may_not_leave() {
        let (center, controls) = cluster();
        let voice = 851_975_000.0;
        let picked = follow_capture(
            center,
            &controls,
            None,
            true,
            false,
            Vec::new(),
            0,
            None,
            &[(voice, 100)],
            None,
            false,
            false,
            false,
            false,
        );
        assert!(!picked.left);
        assert_eq!(picked.controls, controls);
        assert_eq!(picked.voice_hz, 0);
        assert!(picked.away.is_none());
    }

    #[test]
    fn a_control_receiver_does_not_leave_or_play() {
        let (center, controls) = cluster();
        let picked = follow_capture(
            center,
            &controls,
            None,
            false,
            false,
            vec![(858_512_500.0, 7, 1)],
            4,
            Some((858_512_500.0, 7)),
            &[(851_975_000.0, 100)],
            None,
            false,
            false,
            false,
            false,
        );
        assert!(!picked.left);
        assert_eq!(picked.controls, controls);
        assert_eq!(picked.voice_hz, 0);
        assert!(picked.call.is_none());
        assert!(picked.open.is_empty());
        assert!(picked.away.is_none());
        assert_eq!(picked.seq, 4);
    }

    #[test]
    fn two_searches_before_sync_end_the_trip_and_sync_then_loss_does_too() {
        assert_eq!(excursion_done(false, 0, false, false, false, true), (false, false, 1));
        assert_eq!(excursion_done(false, 1, false, false, false, true), (true, false, 2));
        assert_eq!(excursion_done(false, 0, true, false, false, true), (false, true, 0));
        assert_eq!(excursion_done(true, 0, false, false, false, true), (false, true, 0));
        assert_eq!(excursion_done(true, 0, false, true, false, false), (true, true, 0));
        assert_eq!(excursion_done(false, 0, false, true, false, false), (false, false, 0));
        assert_eq!(excursion_done(false, 0, false, false, true, false), (true, false, 0));
    }

    #[test]
    fn a_newer_far_grant_replaces_an_in_window_call() {
        let (center, controls) = cluster();
        let near = 858_512_500.0;
        let far = 851_975_000.0;
        let first = follow_capture(
            center,
            &controls,
            None,
            true,
            true,
            Vec::new(),
            0,
            None,
            &[(near, 1)],
            None,
            false,
            false,
            false,
            false,
        );
        assert!(!first.left);
        let second = follow_capture(
            center,
            &controls,
            None,
            true,
            true,
            first.open,
            first.seq,
            first.call,
            &[(far, 2)],
            first.away,
            false,
            false,
            false,
            false,
        );
        assert!(second.left);
        assert_eq!(second.call, Some((far, 2)));
        assert_eq!(second.voice_hz, far.round() as u64);
    }

    #[test]
    fn transfer_args_omit_gain_and_an_empty_serial() {
        let args = transfer_args("", 852_012_500.0, RxGain::default());
        assert!(args.contains(&"-r".into()));
        assert!(args.contains(&"-".into()));
        assert!(!args.iter().any(|arg| arg == "-l" || arg == "-g" || arg == "-a" || arg == "-d"));
        let named = transfer_args("abc", 852_012_500.0, RxGain::default());
        assert!(named.windows(2).any(|pair| pair[0] == "-d" && pair[1] == "abc"));
    }

    #[test]
    fn a_gqrx_gain_map_is_lna_vga_and_amp() {
        // dB times 10, the way GQRX writes input/gains. LNA 400 is 40 dB.
        let text = r"@Variant(\0\0\0\b\0\0\0\x3\0\0\0\x6\0\x41\0M\0P\0\0\0\x2\0\0\0\0\0\0\0\x6\0L\0N\0\x41\0\0\0\x2\0\0\x1\x90\0\0\0\x6\0V\0G\0\x41\0\0\0\x2\0\0\0\0)";
        let gain = parse_gqrx_gains(text);
        assert_eq!(
            gain,
            RxGain {
                lna_db: Some(40),
                vga_db: Some(0),
                amp: Some(false),
            }
        );
        let args = transfer_args("abc", 857_200_000.0, gain);
        assert!(args.windows(2).any(|pair| pair[0] == "-l" && pair[1] == "40"));
        assert!(args.windows(2).any(|pair| pair[0] == "-g" && pair[1] == "0"));
        assert!(args.windows(2).any(|pair| pair[0] == "-a" && pair[1] == "0"));
        assert!(parse_gqrx_gains("").lna_db.is_none());
    }

    #[test]
    fn hackrf_info_text_lists_each_board() {
        let text = "\
Found HackRF
Index: 0
Serial number: 0000000000000000457863c8252d451f
Board ID Number: 2 (HackRF One)
Found HackRF
Index: 1
Serial number: abc
Board ID Number: 4 (HackRF Jawbreaker)
";
        let listed = dongles(text);
        assert_eq!(
            listed,
            vec![
                Dongle {
                    index: Some(0),
                    serial: "0000000000000000457863c8252d451f".into(),
                    board: "HackRF One".into(),
                },
                Dongle {
                    index: Some(1),
                    serial: "abc".into(),
                    board: "HackRF Jawbreaker".into(),
                },
            ]
        );
        assert_eq!(
            serials(text),
            vec![
                "0000000000000000457863c8252d451f".to_string(),
                "abc".to_string()
            ]
        );
        assert_eq!(
            dongle_label(&listed[0]),
            "0  HackRF One  0000000000000000457863c8252d451f"
        );
    }

    #[test]
    fn one_listed_dongle_fills_an_empty_device() {
        let text = "Found HackRF\nSerial number: deadbeef\n";
        assert_eq!(serials(text), vec!["deadbeef".to_string()]);
        assert_eq!(resolve_serial("", &serials(text)).unwrap(), "deadbeef");
        assert_eq!(resolve_serial("abc", &[]).unwrap(), "abc");
        assert!(resolve_serial("", &[]).is_err());
        assert!(resolve_serial("", &["a".into(), "b".into()]).is_err());
    }

    #[test]
    fn a_tone_one_kilohertz_above_the_channel_comes_out_positive() {
        let center = 100_000_000.0;
        let mut bank = Bank::new(center, &[center]);
        bank.set_voice(Some(center));
        let n = BOX * 40;
        let mut iq = Vec::with_capacity(n * 2);
        let tone = center + 1_000.0;
        for k in 0..n {
            let phase = 2.0 * PI * tone * k as f64 / SAMPLE_RATE as f64;
            let i = (90.0 * phase.cos()).round() as i8;
            let q = (90.0 * phase.sin()).round() as i8;
            iq.push(i as u8);
            iq.push(q as u8);
        }
        let mut voice = Vec::new();
        let mut controls = vec![Vec::new()];
        bank.push(&iq, &mut voice, &mut controls);
        let mean = |samples: &[i16]| {
            let sum: i32 = samples.iter().map(|sample| *sample as i32).sum();
            sum as f64 / samples.len() as f64
        };
        assert!(!voice.is_empty());
        let hz = mean(&voice) / SCALE_I16 * SCALE_HZ;
        assert!(hz > 600.0 && hz < 1_400.0, "voice {hz}");
        let control = mean(&controls[0]) / SCALE_I16 * SCALE_HZ;
        assert!(control > 600.0 && control < 1_400.0, "control {control}");
    }

    #[test]
    fn the_bank_keeps_up_with_the_sample_rate() {
        let center = 859_000_000.0;
        let controls: Vec<f64> = (0..8).map(|i| center - 2_000_000.0 + i as f64 * 500_000.0).collect();
        let mut bank = Bank::new(center, &controls);
        bank.set_voice(Some(center + 25_000.0));
        let n = 400_000;
        let iq = vec![10u8; n * 2];
        let mut voice = Vec::new();
        let mut outs = vec![Vec::new(); controls.len()];
        let start = std::time::Instant::now();
        bank.push(&iq, &mut voice, &mut outs);
        let elapsed = start.elapsed();
        // Debug builds are not the running radio. The capture stalls when this
        // block takes longer than the air time. DC tracking sits in the same loop.
        if cfg!(debug_assertions) {
            return;
        }
        let budget = std::time::Duration::from_secs_f64(n as f64 / SAMPLE_RATE as f64);
        assert!(
            elapsed < budget,
            "demod took {elapsed:?} for {n} samples; the capture budget is {budget:?}"
        );
    }

    #[test]
    fn a_tone_peaks_on_its_spectrum_bin() {
        let n = SPECTRUM_BINS;
        let rate = SAMPLE_RATE as f64;
        let mut iq = Vec::with_capacity(n * 2);
        let tone = rate / 16.0;
        for k in 0..n {
            let phase = 2.0 * PI * tone * k as f64 / rate;
            let i = (100.0 * phase.cos()).round() as i8;
            let q = (100.0 * phase.sin()).round() as i8;
            iq.push(i as u8);
            iq.push(q as u8);
        }
        let db = spectrum_db(&iq).expect("spectrum");
        assert_eq!(db.len(), n);
        let peak = db
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.partial_cmp(right.1).unwrap())
            .map(|(idx, _)| idx)
            .unwrap();
        let expect = n / 2 + n / 16;
        assert!(
            (peak as isize - expect as isize).abs() <= 1,
            "peak {peak} expected {expect}"
        );
        assert!(spectrum_db(&iq[..10]).is_none());
    }

    #[test]
    fn a_dc_offset_does_not_move_the_tone() {
        let center = 100_000_000.0;
        let mut bank = Bank::new(center, &[center]);
        // Baseband. A tone written at `center + 1000` aliases to fs/2 and the
        // boxcar cancels it, which is not the control channel sitting on the LO.
        let n = 400_000;
        let mut iq = Vec::with_capacity(n * 2);
        for k in 0..n {
            let phase = 2.0 * PI * 1_000.0 * k as f64 / SAMPLE_RATE as f64;
            let i = (40.0 * phase.cos()).round() as i32 + 80;
            let q = (40.0 * phase.sin()).round() as i32 - 60;
            iq.push(i as i8 as u8);
            iq.push(q as i8 as u8);
        }
        let mut voice = Vec::new();
        let mut controls = vec![Vec::new()];
        bank.push(&iq, &mut voice, &mut controls);
        let samples = &controls[0];
        assert!(samples.len() > 8, "control samples {}", samples.len());
        let tail = &samples[samples.len() * 3 / 4..];
        let sum: i32 = tail.iter().map(|sample| *sample as i32).sum();
        let hz = sum as f64 / tail.len() as f64 / SCALE_I16 * SCALE_HZ;
        assert!(hz > 600.0 && hz < 1_400.0, "control {hz}");
    }

    #[test]
    fn a_dc_offset_is_not_the_center_bin() {
        let mut iq = Vec::with_capacity(SPECTRUM_BINS * 2);
        for _ in 0..SPECTRUM_BINS {
            iq.push(40i8 as u8);
            iq.push((-30i8) as u8);
        }
        let raw = spectrum_db(&iq).expect("raw");
        let center = SPECTRUM_BINS / 2;
        let raw_peak = raw
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.partial_cmp(right.1).unwrap())
            .map(|(idx, _)| idx)
            .unwrap();
        assert_eq!(raw_peak, center);
        let mut dc = Dc::new();
        for _ in 0..(65_536 * 8) {
            let _ = dc.apply(40.0, -30.0);
        }
        let corrected = spectrum_corrected(&iq, dc.i, dc.q).expect("corrected");
        assert!(
            corrected[center] < raw[center] - 40.0,
            "raw {} corrected {}",
            raw[center], corrected[center]
        );
    }

    #[test]
    fn a_tone_is_louder_than_an_empty_channel() {
        let n = SPECTRUM_BINS;
        let rate = SAMPLE_RATE as f64;
        let mut iq = Vec::with_capacity(n * 2);
        let tone = rate / 16.0;
        let center = 852_000_000.0;
        for k in 0..n {
            let phase = 2.0 * PI * tone * k as f64 / rate;
            let i = (100.0 * phase.cos()).round() as i8;
            let q = (100.0 * phase.sin()).round() as i8;
            iq.push(i as u8);
            iq.push(q as u8);
        }
        let db = spectrum_db(&iq).expect("spectrum");
        let hot = channel_db(&db, center, center + tone);
        let cold = channel_db(&db, center, center + rate / 4.0);
        assert!(hot.is_finite() && cold.is_finite(), "hot {hot} cold {cold}");
        assert!(hot > cold + 20.0, "hot {hot} cold {cold}");
        assert!(channel_db(&db, center, center + FILTER_HZ).is_nan());
        assert!(channel_db(&[], center, center).is_nan());
    }

    #[test]
    fn scan_keeps_a_control_that_is_already_in_the_window() {
        let center = 857_200_000.0;
        let controls = vec![854_437_500.0, 858_512_500.0, 859_962_500.0];
        let (got, kept) = scan_capture(center, &controls, 858_512_500.0);
        assert_eq!(got, center);
        assert_eq!(kept, controls);
        assert_eq!(scan_freq(center, 858_512_500.0, 18.0), 858_512_500.0);
        assert_eq!(scan_freq(center, 858_512_500.0, f64::NAN), center);
    }

    #[test]
    fn scan_parks_on_a_control_outside_the_window() {
        let center = 857_200_000.0;
        let controls = vec![858_512_500.0];
        let outside = 852_162_500.0;
        let (got, kept) = scan_capture(center, &controls, outside);
        assert_eq!(got, outside);
        assert_eq!(kept, vec![outside]);
        assert_eq!(scan_freq(center, outside, 18.0), center);
    }
}
