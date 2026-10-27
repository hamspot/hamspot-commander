//! UTC slot timing for slotted HF data modes.
//!
//! Slots are counted from the Unix epoch. Every supported length divides a
//! day exactly, so slot boundaries line up with UTC midnight and the even/odd
//! parity used by FT8-style modes is the same every day.

pub const DAY_MS: i64 = 86_400_000;

/// Slot length in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotLen(i64);

impl SlotLen {
    pub const FT8: SlotLen = SlotLen(15_000);
    pub const FT4: SlotLen = SlotLen(7_500);
    pub const WSPR: SlotLen = SlotLen(120_000);

    /// A slot length must be positive and divide a day exactly.
    pub fn new(ms: i64) -> Result<SlotLen, String> {
        if ms <= 0 || DAY_MS % ms != 0 {
            return Err(format!("slot length {ms} ms does not divide a UTC day"));
        }
        Ok(SlotLen(ms))
    }

    pub fn ms(self) -> i64 {
        self.0
    }

    /// `15 s`, `7.5 s`, `2 min`.
    pub fn label(self) -> String {
        let ms = self.0;
        if ms >= 60_000 && ms % 60_000 == 0 {
            format!("{} min", ms / 60_000)
        } else if ms % 1000 == 0 {
            format!("{} s", ms / 1000)
        } else {
            format!("{:.1} s", ms as f64 / 1000.0)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub start_ms: i64,
    pub len_ms: i64,
}

impl Slot {
    pub fn end_ms(&self) -> i64 {
        self.start_ms + self.len_ms
    }
    /// Slot number since UTC midnight (0-based).
    pub fn index_in_day(&self) -> i64 {
        self.start_ms.rem_euclid(DAY_MS) / self.len_ms
    }
    /// FT8-style parity: the first slot of each UTC minute/day is even.
    pub fn even(&self) -> bool {
        self.start_ms.div_euclid(self.len_ms) % 2 == 0
    }
    pub fn elapsed_ms(&self, now_ms: i64) -> i64 {
        (now_ms - self.start_ms).clamp(0, self.len_ms)
    }
    pub fn remaining_ms(&self, now_ms: i64) -> i64 {
        self.len_ms - self.elapsed_ms(now_ms)
    }
    pub fn fraction(&self, now_ms: i64) -> f32 {
        self.elapsed_ms(now_ms) as f32 / self.len_ms as f32
    }
}

/// The slot holding `utc_ms`. A time exactly on a boundary starts the new slot.
pub fn slot_at(utc_ms: i64, len: SlotLen) -> Slot {
    let l = len.ms();
    Slot {
        start_ms: utc_ms.div_euclid(l) * l,
        len_ms: l,
    }
}

/// `HHMMSS` or `HHMM` as decoders print it.
pub fn parse_hhmmss(text: &str) -> Option<(u32, u32, u32)> {
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m, s) = match text.len() {
        6 => (&text[0..2], &text[2..4], &text[4..6]),
        4 => (&text[0..2], &text[2..4], "00"),
        _ => return None,
    };
    let (h, m, s) = (h.parse().ok()?, m.parse().ok()?, s.parse().ok()?);
    (h < 24 && m < 60 && s < 60).then_some((h, m, s))
}

/// Allowed lead of a decoder timestamp over this clock before it counts as future.
pub const FUTURE_SLACK_MS: i64 = 2_000;

/// Turn a time of day into a full UTC instant near `now_ms`: the latest
/// candidate (yesterday, today, tomorrow) that is not in the future. A decode
/// stamped 23:59:45 that arrives at 00:00:02 belongs to yesterday.
pub fn resolve_time_of_day(h: u32, m: u32, s: u32, now_ms: i64) -> i64 {
    let tod = ((h as i64 * 60 + m as i64) * 60 + s as i64) * 1000;
    let day0 = now_ms.div_euclid(DAY_MS) * DAY_MS;
    [day0 + DAY_MS + tod, day0 + tod, day0 - DAY_MS + tod]
        .into_iter()
        .find(|t| *t <= now_ms + FUTURE_SLACK_MS)
        .unwrap_or(day0 - DAY_MS + tod)
}

/// `HH:MM:SS` of a UTC instant.
pub fn format_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIDNIGHT: i64 = 1_793_059_200_000; // 2026-10-28T00:00:00Z

    #[test]
    fn midnight_is_a_day_boundary() {
        assert_eq!(MIDNIGHT % DAY_MS, 0);
    }

    #[test]
    fn ft8_boundaries_and_edges() {
        let s = slot_at(MIDNIGHT + 14_999, SlotLen::FT8);
        assert_eq!(s.start_ms, MIDNIGHT);
        assert!(s.even());
        assert_eq!(s.remaining_ms(MIDNIGHT + 14_999), 1);
        let s = slot_at(MIDNIGHT + 15_000, SlotLen::FT8);
        assert_eq!(s.start_ms, MIDNIGHT + 15_000);
        assert!(!s.even());
        assert_eq!(s.index_in_day(), 1);
        assert_eq!(s.elapsed_ms(MIDNIGHT + 15_000), 0);
    }

    #[test]
    fn ft4_half_second_slots() {
        let s = slot_at(MIDNIGHT + 7_499, SlotLen::FT4);
        assert_eq!(s.start_ms, MIDNIGHT);
        let s = slot_at(MIDNIGHT + 7_500, SlotLen::FT4);
        assert_eq!(s.start_ms, MIDNIGHT + 7_500);
        assert_eq!(s.end_ms(), MIDNIGHT + 15_000);
        assert_eq!(SlotLen::FT4.label(), "7.5 s");
    }

    #[test]
    fn wspr_even_minutes() {
        let s = slot_at(MIDNIGHT + 3 * 60_000 + 5, SlotLen::WSPR);
        assert_eq!(s.start_ms, MIDNIGHT + 2 * 60_000);
        assert_eq!(SlotLen::WSPR.label(), "2 min");
        assert_eq!(SlotLen::FT8.label(), "15 s");
    }

    #[test]
    fn day_rollover() {
        for len in [SlotLen::FT8, SlotLen::FT4, SlotLen::WSPR] {
            let last = slot_at(MIDNIGHT - 1, len);
            assert_eq!(last.end_ms(), MIDNIGHT);
            assert!(!last.even(), "last slot of a day is odd");
            assert_eq!(last.index_in_day(), DAY_MS / len.ms() - 1);
            let first = slot_at(MIDNIGHT, len);
            assert_eq!(first.index_in_day(), 0);
            assert!(first.even());
        }
    }

    #[test]
    fn before_epoch_still_floors() {
        let s = slot_at(-1, SlotLen::FT8);
        assert_eq!(s.start_ms, -15_000);
    }

    #[test]
    fn slot_lengths_must_divide_a_day() {
        assert!(SlotLen::new(15_000).is_ok());
        assert!(SlotLen::new(7_000).is_err());
        assert!(SlotLen::new(0).is_err());
        assert!(SlotLen::new(-15_000).is_err());
    }

    #[test]
    fn time_of_day_parsing() {
        assert_eq!(parse_hhmmss("235945"), Some((23, 59, 45)));
        assert_eq!(parse_hhmmss("1204"), Some((12, 4, 0)));
        assert_eq!(parse_hhmmss("246000"), None);
        assert_eq!(parse_hhmmss("12a400"), None);
        assert_eq!(parse_hhmmss("12345"), None);
    }

    #[test]
    fn time_of_day_resolves_across_midnight_and_never_future() {
        // Stamped just before midnight, arriving just after: yesterday.
        let t = resolve_time_of_day(23, 59, 45, MIDNIGHT + 2_000);
        assert_eq!(t, MIDNIGHT - 15_000);
        // Same day.
        let t = resolve_time_of_day(0, 0, 15, MIDNIGHT + 20_000);
        assert_eq!(t, MIDNIGHT + 15_000);
        // A stamp slightly ahead (inside the slack) stays today.
        let t = resolve_time_of_day(0, 0, 30, MIDNIGHT + 29_000);
        assert_eq!(t, MIDNIGHT + 30_000);
        // Far ahead of the clock: resolved to yesterday, never the future.
        let t = resolve_time_of_day(12, 0, 0, MIDNIGHT + 1_000);
        assert!(t <= MIDNIGHT + 1_000);
        assert_eq!(format_utc(MIDNIGHT - 15_000), "23:59:45");
    }
}
