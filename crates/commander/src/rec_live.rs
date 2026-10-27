//! Process-wide REC recorder. The HackRF worker threads push channelised IQ
//! here; the UI thread opens and closes segments. Nothing here blocks.
use hsrec::writer::{Chunk, Recorder};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

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

/// From the HackRF worker: never blocks; a full queue drops and counts.
pub fn push(key: u64, hz: u64, data: Vec<i16>) {
    if let Some(rec) = RECORDER.get() {
        rec.push(Chunk { key, hz, data });
    }
}
