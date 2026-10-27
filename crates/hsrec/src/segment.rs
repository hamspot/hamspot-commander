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

/// Default longest segment. A call still going at this length is closed
/// and a new segment opened at once, so nothing is lost; the timeline shows
/// two blocks. Most P25 radios time out a transmission at 60 s, so 120 s
/// only splits real marathons and the stuck segments this guards against.
pub const MAX_SEGMENT_DEFAULT_S: u32 = 120;
/// Smallest and largest maximum segment length a station file may set.
pub const MAX_SEGMENT_MIN_S: u32 = 10;
pub const MAX_SEGMENT_MAX_S: u32 = 3600;

/// How long a new talkgroup call may go without a decoded voice frame
/// before REC treats it as silent: the tune and first sync take this long.
pub const VOICE_START_GRACE: Duration = Duration::from_millis(3000);
/// Shortest decoder silence that closes a call. The decoder's lines arrive
/// in bursts, so a hang shorter than this is stretched to it.
pub const VOICE_SILENCE_MIN: Duration = Duration::from_millis(1000);

/// Gates a talkgroup call on the voice decoder: the follower may keep a call
/// picked while the voice channel is silent (a control channel repeating its
/// grant update), but REC only records while voice frames are decoded.
#[derive(Default)]
pub struct VoiceGate {
    key: Option<CallKey>,
    since: Option<Instant>,
}

impl VoiceGate {
    /// `want` is the call the follower holds now. `last_voice` is when this
    /// radio's voice decoder last printed a voice frame (LDU / 4V / 2V).
    /// Returns `want` while the call is alive, `None` once it has been
    /// silent for `silence` (or never spoke within [`VOICE_START_GRACE`]).
    pub fn pass(&mut self, want: Option<CallKey>, last_voice: Option<Instant>, silence: Duration, now: Instant) -> Option<CallKey> {
        if want != self.key {
            self.key = want.clone();
            self.since = want.as_ref().map(|_| now);
        }
        let since = self.since?;
        let silence = silence.max(VOICE_SILENCE_MIN);
        let alive = match last_voice.filter(|at| *at >= since) {
            Some(at) => now.saturating_duration_since(at) < silence,
            None => now.saturating_duration_since(since) < VOICE_START_GRACE,
        };
        if alive { want } else { None }
    }
}

/// One radio's segmenter.
#[derive(Default)]
pub struct Segmenter {
    current: Option<CallKey>,
    gone_at: Option<Instant>,
    opened_at: Option<Instant>,
}

impl Segmenter {
    pub fn recording(&self) -> Option<&CallKey> {
        self.current.as_ref()
    }

    /// `want` is the call to record now (already filtered by mode/marks).
    /// `encrypted` aborts and deletes the current segment.
    pub fn step(&mut self, want: Option<CallKey>, encrypted: bool, hang: Duration, now: Instant) -> Vec<Action> {
        self.step_max(want, encrypted, hang, None, now)
    }

    /// [`Self::step`] with a longest segment. A call still wanted at `max`
    /// is closed and re-opened in the same step.
    pub fn step_max(&mut self, want: Option<CallKey>, encrypted: bool, hang: Duration, max: Option<Duration>, now: Instant) -> Vec<Action> {
        let mut out = self.step_inner(want, encrypted, hang, now);
        if out.iter().any(|a| matches!(a, Action::Open(_))) {
            self.opened_at = Some(now);
        }
        if self.current.is_none() {
            self.opened_at = None;
        }
        if let (Some(max), Some(at), Some(cur)) = (max, self.opened_at, self.current.clone()) {
            if out.is_empty() && self.gone_at.is_none() && now.saturating_duration_since(at) >= max {
                out.push(Action::Close { keep: true });
                out.push(Action::Open(cur));
                self.opened_at = Some(now);
            }
        }
        out
    }

    fn step_inner(&mut self, want: Option<CallKey>, encrypted: bool, hang: Duration, now: Instant) -> Vec<Action> {
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
        self.opened_at = None;
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
    fn long_call_splits_at_max_without_a_gap() {
        let t0 = Instant::now();
        let hang = Duration::from_millis(500);
        let max = Some(Duration::from_secs(120));
        let mut s = Segmenter::default();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        assert_eq!(s.step_max(Some(k(1)), false, hang, max, ms(0)), vec![Action::Open(k(1))]);
        let mut opens = 1;
        for n in (100..=300_000).step_by(100) {
            let a = s.step_max(Some(k(1)), false, hang, max, ms(n));
            if !a.is_empty() {
                assert_eq!(a, vec![Action::Close { keep: true }, Action::Open(k(1))], "at {n} ms");
                assert!(n == 120_000 || n == 240_000, "split at {n} ms");
                opens += 1;
            }
        }
        assert_eq!(opens, 3);
        // A short call never splits, and no max means no split.
        let mut s = Segmenter::default();
        s.step_max(Some(k(2)), false, hang, None, ms(0));
        assert!(s.step_max(Some(k(2)), false, hang, None, ms(500_000)).is_empty());
    }

    #[test]
    fn silent_call_closes_on_decoder_silence() {
        // The follower never lets the call go (no terminator, the control
        // channel repeats the grant), but voice frames stop at 2 s.
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let hang = Duration::from_millis(500);
        let mut gate = VoiceGate::default();
        let mut s = Segmenter::default();
        let mut closed_at = None;
        let mut last_voice = None;
        for n in (0..=160_000u64).step_by(100) {
            if (1000..=2000).contains(&n) && n % 200 == 0 {
                last_voice = Some(ms(n));
            }
            let want = gate.pass(Some(k(7)), last_voice, hang, ms(n));
            for a in s.step_max(want, false, hang, Some(Duration::from_secs(120)), ms(n)) {
                if a == (Action::Close { keep: true }) && closed_at.is_none() {
                    closed_at = Some(n);
                }
            }
        }
        // Last voice 2.0 s + 1.0 s silence + 0.5 s hang.
        assert_eq!(closed_at, Some(3500));
        assert!(s.recording().is_none());
    }

    #[test]
    fn call_that_never_speaks_closes_after_start_grace() {
        let t0 = Instant::now();
        let mut gate = VoiceGate::default();
        assert_eq!(gate.pass(Some(k(1)), None, Duration::ZERO, t0), Some(k(1)));
        assert_eq!(gate.pass(Some(k(1)), None, Duration::ZERO, t0 + Duration::from_millis(2900)), Some(k(1)));
        assert_eq!(gate.pass(Some(k(1)), None, Duration::ZERO, t0 + VOICE_START_GRACE), None);
        // Voice from before this call does not count for it.
        let old = Some(t0);
        assert_eq!(gate.pass(Some(k(2)), old, Duration::ZERO, t0 + Duration::from_secs(4)), Some(k(2)));
        assert_eq!(gate.pass(Some(k(2)), old, Duration::ZERO, t0 + Duration::from_secs(8)), None);
        // Speech resumes: the call is wanted again (a new segment opens).
        let back = t0 + Duration::from_secs(9);
        assert_eq!(gate.pass(Some(k(2)), Some(back), Duration::ZERO, back), Some(k(2)));
        assert_eq!(gate.pass(None, Some(back), Duration::ZERO, back), None);
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
