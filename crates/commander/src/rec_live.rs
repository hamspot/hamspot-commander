//! Process-wide REC recorder. The HackRF worker threads push channelised IQ
//! here, and the hamlib/GQRX receive pipe pushes the rig's demod audio.
//! The UI thread opens and closes segments. Nothing here blocks.
use hsrec::writer::{Chunk, Recorder};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

static RECORDER: OnceLock<Recorder> = OnceLock::new();
/// Any REC key armed. When clear the tap costs nothing.
static TAP: AtomicBool = AtomicBool::new(false);

pub fn recorder() -> &'static Recorder {
    RECORDER.get_or_init(Recorder::spawn)
}

pub fn tap_on() -> bool {
    TAP.load(Ordering::Relaxed)
}

pub fn set_tap(on: bool) {
    TAP.store(on, Ordering::Relaxed);
}

/// From the HackRF worker or the rig-audio tee: never blocks; a full queue drops and counts.
pub fn push(key: u64, hz: u64, data: Vec<i16>) {
    if let Some(rec) = RECORDER.get() {
        rec.push(Chunk { key, hz, data });
    }
}

static REC_HZ: OnceLock<Mutex<HashMap<u64, Arc<AtomicU64>>>> = OnceLock::new();

fn rec_hz_map() -> std::sync::MutexGuard<'static, HashMap<u64, Arc<AtomicU64>>> {
    REC_HZ.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Frequency (Hz) the rig-audio tee should file for this radio. `0` means
/// this radio is not on a call REC wants. The receive-pipe thread holds the
/// same atomic the UI stores into.
pub fn rec_hz_slot(id: u64) -> Arc<AtomicU64> {
    let mut map = rec_hz_map();
    map.entry(id).or_insert_with(|| Arc::new(AtomicU64::new(0))).clone()
}

pub fn set_rec_hz(id: u64, hz: u64) {
    rec_hz_slot(id).store(hz, Ordering::Relaxed);
}

/// One leftover byte when a read splits a little-endian sample.
#[derive(Default)]
pub struct S16Carry {
    odd: Option<u8>,
}

impl S16Carry {
    /// Next whole samples in `input`. When `keep` is false the alignment is
    /// still updated and nothing is allocated: REC is off, or this radio is
    /// not on a call.
    pub fn take(&mut self, input: &[u8], keep: bool) -> Vec<i16> {
        if !keep {
            let total = usize::from(self.odd.is_some()) + input.len();
            self.odd = if total % 2 == 1 {
                input.last().copied().or(self.odd)
            } else {
                None
            };
            return Vec::new();
        }
        let mut raw = Vec::with_capacity(usize::from(self.odd.is_some()) + input.len());
        if let Some(b) = self.odd.take() {
            raw.push(b);
        }
        raw.extend_from_slice(input);
        let n = raw.len() - raw.len() % 2;
        self.odd = if n < raw.len() { Some(raw[n]) } else { None };
        raw[..n].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s16_carry_splits_and_drops_when_not_kept() {
        let mut c = S16Carry::default();
        assert_eq!(c.take(&[0x34, 0x12], true), vec![0x1234]);
        // A read that ends mid-sample waits for the next byte.
        assert!(c.take(&[0x00], true).is_empty());
        assert_eq!(c.take(&[0x80], true), vec![i16::from_le_bytes([0x00, 0x80])]);
        // While REC is not taking this radio, samples are dropped. An odd
        // trailing byte is kept only so the next sample stays aligned.
        assert!(c.take(&[0x01, 0x02, 0x03], false).is_empty());
        assert_eq!(c.take(&[0x04], true), vec![i16::from_le_bytes([0x03, 0x04])]);
        assert!(c.take(&[], true).is_empty());
    }

    #[test]
    fn rec_hz_slot_is_shared() {
        let id = u64::MAX - 7;
        let slot = rec_hz_slot(id);
        assert_eq!(slot.load(Ordering::Relaxed), 0);
        set_rec_hz(id, 146_520_000);
        assert_eq!(slot.load(Ordering::Relaxed), 146_520_000);
        set_rec_hz(id, 0);
        assert_eq!(rec_hz_slot(id).load(Ordering::Relaxed), 0);
    }
}
