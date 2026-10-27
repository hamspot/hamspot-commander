//! Call segmentation and the REC key modes, kept free of UI code so it can
//! be tested with fake follower events.
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// REC key state for one tab. Persisted in the station file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecMode {
    #[default]
    Off,
    /// Red: record marked talkgroups / channels only.
    Marked,
    /// Amber: record all traffic.
    All,
}

impl RecMode {
    pub fn is_off(&self) -> bool {
        *self == RecMode::Off
    }
    pub fn armed(&self) -> bool {
        !self.is_off()
    }
    pub fn label(&self) -> &'static str {
        match self {
            RecMode::Off => "off",
            RecMode::Marked => "marked only",
            RecMode::All => "all traffic",
        }
    }
}

/// What the REC key does on a press (rustcars: click, Shift = double,
/// Ctrl = triple which is reserved). A plain double click is also "all".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Click,
    ShiftClick,
    CtrlClick,
    DoubleClick,
}

pub fn press(mode: RecMode, p: Press) -> RecMode {
    match p {
        Press::CtrlClick => mode,
        Press::Click => {
            if mode.is_off() { RecMode::Marked } else { RecMode::Off }
        }
        // egui reports the first click of a double click as a click, so a
        // double click from Marked (set by that first click) goes to All.
        Press::ShiftClick | Press::DoubleClick => {
            if mode == RecMode::All && p == Press::ShiftClick { RecMode::Off } else { RecMode::All }
        }
    }
}

/// Should this call be recorded under `mode`? Encrypted is never recorded.
pub fn wanted(mode: RecMode, marked: bool, encrypted: bool) -> bool {
    !encrypted && match mode {
        RecMode::Off => false,
        RecMode::Marked => marked,
        RecMode::All => true,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CallKey {
    pub talkgroup: Option<u32>,
    pub channel: String,
    pub hz: u64,
    /// Capture centre the IQ is tapped from. A retune starts a new segment,
    /// so the stored offset always matches the samples.
    pub centre: u64,
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Open(CallKey),
    Close { keep: bool },
}

/// One radio's segmenter.
#[derive(Default)]
pub struct Segmenter {
    current: Option<CallKey>,
    gone_at: Option<Instant>,
}

impl Segmenter {
    pub fn recording(&self) -> Option<&CallKey> {
        self.current.as_ref()
    }

    /// `want` is the call to record now (already filtered by mode/marks).
    /// `encrypted` aborts and deletes the current segment.
    pub fn step(&mut self, want: Option<CallKey>, encrypted: bool, hang: Duration, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        if encrypted && self.current.is_some() {
            self.current = None;
            self.gone_at = None;
            out.push(Action::Close { keep: false });
            return out;
        }
        match (&self.current, want) {
            (None, None) => {}
            (None, Some(k)) => {
                self.current = Some(k.clone());
                out.push(Action::Open(k));
            }
            (Some(cur), Some(k)) if *cur == k => self.gone_at = None,
            (Some(_), Some(k)) => {
                out.push(Action::Close { keep: true });
                self.current = Some(k.clone());
                self.gone_at = None;
                out.push(Action::Open(k));
            }
            (Some(_), None) => {
                let at = *self.gone_at.get_or_insert(now);
                if now.duration_since(at) >= hang {
                    self.current = None;
                    self.gone_at = None;
                    out.push(Action::Close { keep: true });
                }
            }
        }
        out
    }

    /// REC turned off or the radio went away.
    pub fn stop(&mut self) -> Vec<Action> {
        self.gone_at = None;
        if self.current.take().is_some() { vec![Action::Close { keep: true }] } else { Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(tg: u32) -> CallKey {
        CallKey { talkgroup: Some(tg), channel: String::new(), hz: 851_000_000, centre: 851_500_000 }
    }

    #[test]
    fn key_presses_follow_rustcars() {
        use RecMode::*;
        assert_eq!(press(Off, Press::Click), Marked);
        assert_eq!(press(Marked, Press::Click), Off);
        assert_eq!(press(All, Press::Click), Off);
        assert_eq!(press(Off, Press::ShiftClick), All);
        assert_eq!(press(Marked, Press::ShiftClick), All);
        assert_eq!(press(All, Press::ShiftClick), Off);
        assert_eq!(press(Marked, Press::DoubleClick), All); // first click made it Marked
        assert_eq!(press(All, Press::DoubleClick), All);
        assert_eq!(press(Marked, Press::CtrlClick), Marked);
    }

    #[test]
    fn filter() {
        assert!(!wanted(RecMode::Off, true, false));
        assert!(wanted(RecMode::Marked, true, false));
        assert!(!wanted(RecMode::Marked, false, false));
        assert!(wanted(RecMode::All, false, false));
        assert!(!wanted(RecMode::All, true, true));
    }

    #[test]
    fn split_and_merge() {
        let t0 = Instant::now();
        let hang = Duration::from_millis(500);
        let mut s = Segmenter::default();
        assert_eq!(s.step(Some(k(1)), false, hang, t0), vec![Action::Open(k(1))]);
        assert!(s.step(Some(k(1)), false, hang, t0 + Duration::from_millis(100)).is_empty());
        // Gap shorter than hang, same TG: one segment.
        assert!(s.step(None, false, hang, t0 + Duration::from_millis(200)).is_empty());
        assert!(s.step(Some(k(1)), false, hang, t0 + Duration::from_millis(600)).is_empty());
        // A different TG splits.
        assert_eq!(
            s.step(Some(k(2)), false, hang, t0 + Duration::from_millis(700)),
            vec![Action::Close { keep: true }, Action::Open(k(2))]
        );
        // Gap longer than hang closes.
        assert!(s.step(None, false, hang, t0 + Duration::from_millis(800)).is_empty());
        assert_eq!(s.step(None, false, hang, t0 + Duration::from_millis(1400)), vec![Action::Close { keep: true }]);
        assert!(s.recording().is_none());
    }

    #[test]
    fn encrypted_aborts() {
        let t0 = Instant::now();
        let mut s = Segmenter::default();
        s.step(Some(k(1)), false, Duration::ZERO, t0);
        assert_eq!(s.step(Some(k(1)), true, Duration::ZERO, t0), vec![Action::Close { keep: false }]);
        assert!(s.stop().is_empty());
    }
}
