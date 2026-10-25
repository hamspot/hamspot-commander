//! Busy page-tab pulse. Issue 61.
//!
//! A tab is marked while voice is actually up: a scanner hold on a
//! conventional memory, or a clear voice grant. A control channel, a data
//! grant, and noise are not marks. The brightness is a sine of elapsed
//! time, faded in at the start and faded out across the hang after the
//! last report. Reduced motion and the Config switch leave the speaker
//! and hold the fill still.
//!
//! Channel and talkgroup pages stay in the main window. The page tab is
//! that page's title. The speaker sits on the left of that tab's text.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::Color32;

/// One brightness cycle when the station does not say otherwise.
pub const PERIOD_DEFAULT: f64 = 2.0;
/// How long the speaker takes to fade after the last voice report.
pub const HANG: Duration = Duration::from_secs(2);
/// Rise at the start of a mark, so the fill does not pop on.
pub const FADE_IN: Duration = Duration::from_millis(200);
/// Toward-white tint at the bright peak. The same for every tab.
/// White-ink tabs sit near the ink threshold, so a larger step flips
/// the label to black and drops it under 4.5:1.
pub const PEAK: f32 = 0.06;
/// Steady mark. It is always drawn. This is how dim it is while nobody is talking.
pub const GHOST: f32 = 0.32;
pub const SPEAKER: &str = "\u{1F50A}";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VoiceTab {
    /// Conventional memory the scanner is holding. Not a control channel.
    Channel { zone: usize, index: usize },
    /// One radio's talkgroup page while a voice grant is up.
    Talkgroup { radio: u64 },
}

struct Slot {
    started: Instant,
    heard: Instant,
    phase: f64,
}

pub struct Pulse {
    born: Instant,
    live: HashMap<VoiceTab, Slot>,
}

impl Pulse {
    pub fn new() -> Self {
        Self {
            born: Instant::now(),
            live: HashMap::new(),
        }
    }

    pub fn hear_channel(&mut self, zone: usize, index: usize, now: Instant) {
        self.hear(VoiceTab::Channel { zone, index }, now);
    }

    pub fn hear_talkgroup(&mut self, radio: u64, _tg: u32, now: Instant) {
        self.hear(VoiceTab::Talkgroup { radio }, now);
    }

    fn hear(&mut self, tab: VoiceTab, now: Instant) {
        if let Some(slot) = self.live.get_mut(&tab) {
            slot.heard = now;
        } else {
            self.live.insert(
                tab,
                Slot {
                    started: now,
                    heard: now,
                    phase: phase_of(tab),
                },
            );
        }
    }

    /// Drop marks whose hang has finished.
    pub fn prune(&mut self, now: Instant) {
        self.live
            .retain(|_, slot| now.saturating_duration_since(slot.heard) <= HANG);
    }

    pub fn busy(&self) -> bool {
        !self.live.is_empty()
    }

    pub fn marker(&self, tab: &VoiceTab, now: Instant) -> bool {
        self.live
            .get(tab)
            .is_some_and(|slot| now.saturating_duration_since(slot.heard) <= HANG)
    }

    pub fn showing_channel(&self, now: Instant) -> Option<VoiceTab> {
        self.live.iter().find_map(|(tab, slot)| match tab {
            VoiceTab::Channel { .. } if fresh(slot, now) => Some(*tab),
            _ => None,
        })
    }

    /// 1 while voice was just reported, 0 after [`HANG`] of silence.
    pub fn presence(&self, tab: &VoiceTab, now: Instant) -> f32 {
        let Some(slot) = self.live.get(tab) else {
            return 0.0;
        };
        let age = now.saturating_duration_since(slot.heard);
        if age > HANG {
            return 0.0;
        }
        1.0 - (age.as_secs_f64() / HANG.as_secs_f64()).clamp(0.0, 1.0) as f32
    }

    pub fn channel_presence(&self, now: Instant) -> f32 {
        self.showing_channel(now)
            .map(|tab| self.presence(&tab, now))
            .unwrap_or(0.0)
    }

    /// Tint to add to the resting tab fill. Zero when the pulse is off,
    /// motion is reduced, or the mark has faded out.
    pub fn lift(
        &self,
        tab: &VoiceTab,
        now: Instant,
        period: f64,
        enabled: bool,
        reduced: bool,
    ) -> f32 {
        if !enabled || reduced {
            return 0.0;
        }
        let Some(slot) = self.live.get(tab) else {
            return 0.0;
        };
        let env = envelope(
            now.saturating_duration_since(slot.started),
            now.saturating_duration_since(slot.heard),
        );
        if env <= 0.0 {
            return 0.0;
        }
        let elapsed = now.saturating_duration_since(self.born).as_secs_f64();
        wave(elapsed, slot.phase, period) * env * PEAK
    }
}

fn fresh(slot: &Slot, now: Instant) -> bool {
    now.saturating_duration_since(slot.heard) <= HANG
}

/// 0 at the start of a mark, 1 after [`FADE_IN`], then down to 0 across [`HANG`].
pub fn envelope(since_start: Duration, since_heard: Duration) -> f32 {
    if since_heard > HANG {
        return 0.0;
    }
    let rise = if FADE_IN.is_zero() {
        1.0
    } else {
        (since_start.as_secs_f64() / FADE_IN.as_secs_f64()).clamp(0.0, 1.0) as f32
    };
    let fall = 1.0 - (since_heard.as_secs_f64() / HANG.as_secs_f64()).clamp(0.0, 1.0) as f32;
    rise * fall
}

/// 0 at the bottom of the cycle, 1 at the bright peak. `phase` is turns, 0 to 1.
pub fn wave(elapsed_secs: f64, phase: f64, period: f64) -> f32 {
    let period = if period.is_finite() && period >= 0.4 {
        period
    } else {
        PERIOD_DEFAULT
    };
    let turns = elapsed_secs / period + phase;
    let s = (turns * std::f64::consts::TAU).sin();
    ((s + 1.0) * 0.5) as f32
}

pub fn brighten(base: Color32, lift: f32) -> Color32 {
    rustcars::tint(base, lift.clamp(0.0, PEAK))
}

/// Full while someone is talking, [`GHOST`] when the hang has finished.
pub fn speaker_alpha(presence: f32) -> f32 {
    let presence = presence.clamp(0.0, 1.0);
    GHOST + (1.0 - GHOST) * presence
}

/// GNOME stores reduced motion as `enable-animations` = false.
/// A missing schema leaves motion on. The Config switch still stops the sine.
pub fn system_reduced_motion() -> bool {
    let Ok(out) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    animations_disabled(&String::from_utf8_lossy(&out.stdout))
}

pub fn animations_disabled(text: &str) -> bool {
    text.split_whitespace().next() == Some("false")
}

fn phase_of(tab: VoiceTab) -> f64 {
    let n = match tab {
        VoiceTab::Channel { zone, index } => ((zone as u64) << 32) ^ (index as u64),
        VoiceTab::Talkgroup { radio } => radio ^ 0xA5A5_A5A5_A5A5_A5A5,
    };
    let mixed = n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (mixed % 10_000) as f64 / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors() -> [Color32; 6] {
        use rustcars::palette::{BRICK, OLIVE, PLUM, SLATE, TAN, TEAL};
        [TAN, SLATE, OLIVE, BRICK, TEAL, PLUM]
    }

    #[test]
    fn a_mark_fades_in_and_then_out_across_the_hang() {
        assert!(envelope(Duration::ZERO, Duration::ZERO) < 0.01);
        let mid = envelope(FADE_IN / 2, Duration::ZERO);
        assert!((mid - 0.5).abs() < 0.02);
        assert!((envelope(FADE_IN, Duration::ZERO) - 1.0).abs() < 0.01);
        let half = envelope(FADE_IN + HANG, HANG / 2);
        assert!((half - 0.5).abs() < 0.02);
        assert!(envelope(FADE_IN + HANG, HANG + Duration::from_millis(1)) < 0.01);
    }

    #[test]
    fn the_hang_keeps_the_speaker_and_reduced_motion_holds_the_fill() {
        let mut pulse = Pulse::new();
        let start = Instant::now();
        let tab = VoiceTab::Talkgroup { radio: 7 };
        pulse.hear_talkgroup(7, 1216, start);
        assert!(pulse.marker(&tab, start));
        assert!((pulse.presence(&tab, start) - 1.0).abs() < 0.01);
        assert!((speaker_alpha(0.0) - GHOST).abs() < 0.01);
        assert!((speaker_alpha(1.0) - 1.0).abs() < 0.01);
        let mid = start + HANG / 2;
        assert!((pulse.presence(&tab, mid) - 0.5).abs() < 0.05);
        assert_eq!(pulse.lift(&tab, start, PERIOD_DEFAULT, false, false), 0.0);
        assert_eq!(pulse.lift(&tab, start, PERIOD_DEFAULT, true, true), 0.0);
        let later = start + FADE_IN + Duration::from_millis(500);
        assert!(pulse.marker(&tab, later));
        assert!(pulse.lift(&tab, later, PERIOD_DEFAULT, true, false) > 0.0);
        assert_eq!(pulse.lift(&tab, later, PERIOD_DEFAULT, true, true), 0.0);
        pulse.prune(start + HANG + Duration::from_millis(1));
        assert!(!pulse.marker(&tab, start + HANG + Duration::from_millis(1)));
        assert!(!pulse.busy());
    }

    #[test]
    fn two_tabs_do_not_share_a_phase() {
        let a = phase_of(VoiceTab::Channel { zone: 0, index: 0 });
        let b = phase_of(VoiceTab::Channel { zone: 0, index: 1 });
        let c = phase_of(VoiceTab::Talkgroup { radio: 3 });
        assert!(a >= 0.0 && a < 1.0);
        assert_ne!(a, b);
        assert_ne!(a, c);
        // A quarter cycle with no offset is the bright peak. The bottom is three quarters in.
        assert!((wave(PERIOD_DEFAULT / 4.0, 0.0, PERIOD_DEFAULT) - 1.0).abs() < 0.001);
        assert!(wave(PERIOD_DEFAULT * 0.75, 0.0, PERIOD_DEFAULT) < 0.001);
    }

    #[test]
    fn the_brightest_frame_keeps_text_and_border_contrast() {
        let bg = rustcars::LcarsTheme::default().bg;
        for base in colors() {
            for resting in [base, rustcars::tint(base, 0.35)] {
                let bright = brighten(resting, PEAK);
                let ink = rustcars::ink_on(resting);
                assert_eq!(rustcars::ink_on(bright), ink);
                assert!(rustcars::contrast(ink, bright) >= 4.5);
                assert!(rustcars::contrast(bright, bg) >= 3.0);
            }
        }
    }

    #[test]
    fn gsettings_false_is_reduced_motion() {
        assert!(animations_disabled("false\n"));
        assert!(!animations_disabled("true\n"));
        assert!(!animations_disabled(""));
    }
}
