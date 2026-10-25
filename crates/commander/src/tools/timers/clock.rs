//! Pure time logic for the timers example: stopwatch, countdown, clock formats, zones.
//! Nothing here reads the system clock; callers pass `now` so every function is testable.
use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc};

/// Monotonic stopwatch driven by a caller-supplied time in seconds.
#[derive(Default, Debug, Clone)]
pub struct Stopwatch {
    accum: f64,
    started: Option<f64>,
    /// Cumulative elapsed time at each lap.
    pub laps: Vec<f64>,
}

impl Stopwatch {
    pub fn running(&self) -> bool {
        self.started.is_some()
    }
    pub fn elapsed(&self, now: f64) -> f64 {
        self.accum + self.started.map_or(0.0, |s| (now - s).max(0.0))
    }
    pub fn start(&mut self, now: f64) {
        if self.started.is_none() {
            self.started = Some(now);
        }
    }
    pub fn stop(&mut self, now: f64) {
        if let Some(s) = self.started.take() {
            self.accum += (now - s).max(0.0);
        }
    }
    pub fn toggle(&mut self, now: f64) {
        if self.running() {
            self.stop(now)
        } else {
            self.start(now)
        }
    }
    /// Records a lap (only while running) and returns its cumulative time.
    pub fn lap(&mut self, now: f64) -> Option<f64> {
        if !self.running() {
            return None;
        }
        let t = self.elapsed(now);
        self.laps.push(t);
        Some(t)
    }
    /// The duration of each lap (difference from the previous lap).
    pub fn lap_splits(&self) -> Vec<f64> {
        let mut prev = 0.0;
        self.laps
            .iter()
            .map(|t| {
                let d = t - prev;
                prev = *t;
                d
            })
            .collect()
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Countdown timer. Time can be added while it runs.
#[derive(Default, Debug, Clone)]
pub struct Countdown {
    left: f64,
    started: Option<f64>,
}

impl Countdown {
    pub fn running(&self) -> bool {
        self.started.is_some()
    }
    pub fn remaining(&self, now: f64) -> f64 {
        (self.left - self.started.map_or(0.0, |s| (now - s).max(0.0))).max(0.0)
    }
    /// True once a running countdown has reached zero.
    pub fn finished(&self, now: f64) -> bool {
        self.started.is_some() && self.remaining(now) <= 0.0
    }
    pub fn set(&mut self, secs: f64) {
        self.left = secs.max(0.0);
        self.started = None;
    }
    pub fn add(&mut self, now: f64, secs: f64) {
        let r = self.remaining(now);
        self.left = r + secs;
        if self.started.is_some() {
            self.started = Some(now);
        }
    }
    pub fn start(&mut self, now: f64) {
        if self.started.is_none() && self.left > 0.0 {
            self.started = Some(now);
        }
    }
    pub fn pause(&mut self, now: f64) {
        self.left = self.remaining(now);
        self.started = None;
    }
    pub fn toggle(&mut self, now: f64) {
        if self.running() {
            self.pause(now)
        } else {
            self.start(now)
        }
    }
}

/// `1:02:03` or `02:03` (no tenths) for countdowns.
pub fn fmt_hms(secs: f64) -> String {
    let s = secs.max(0.0).ceil() as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// `00:01:23.4` for the stopwatch (tenths truncated, never rounded up).
pub fn fmt_stopwatch(secs: f64) -> String {
    let tenths = (secs.max(0.0) * 10.0).floor() as u64;
    let (h, m, s, t) = (
        tenths / 36000,
        tenths / 600 % 60,
        tenths / 10 % 60,
        tenths % 10,
    );
    format!("{h:02}:{m:02}:{s:02}.{t}")
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Stamp {
    /// `2026-10-23 1948Z`, the usual log style.
    Utc,
    /// `2026-10-23 15:48 -04:00`
    Local,
    /// `2026-10-23T19:48:12Z`
    Iso,
}

pub fn stamp(utc: DateTime<Utc>, local: FixedOffset, kind: Stamp) -> String {
    match kind {
        Stamp::Utc => utc.format("%Y-%m-%d %H%MZ").to_string(),
        Stamp::Local => utc
            .with_timezone(&local)
            .format("%Y-%m-%d %H:%M %:z")
            .to_string(),
        Stamp::Iso => utc.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ClockFmt {
    H24,
    H12,
    Iso,
}

/// A clock face for a fixed offset.
pub fn face(utc: DateTime<Utc>, offset: FixedOffset, f: ClockFmt) -> String {
    let t = utc.with_timezone(&offset);
    match f {
        ClockFmt::H24 => t.format("%H:%M:%S").to_string(),
        ClockFmt::H12 => t.format("%I:%M:%S %p").to_string(),
        ClockFmt::Iso => t.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
    }
}

pub fn day_of_year(utc: DateTime<Utc>) -> u32 {
    utc.ordinal()
}

/// Seconds until the next multiple of `period` seconds of UTC time (e.g. 3600 = top of the hour).
pub fn secs_to_next(utc: DateTime<Utc>, period: u32) -> u32 {
    let into = utc.minute() * 60 + utc.second();
    let into = into % period.max(1);
    period - into
}

/// Fixed-offset zones for the world clock. There is no daylight saving here: it needs a
/// time-zone database, and the offset shown is the standard (winter) one.
pub const ZONES: [(&str, i32); 12] = [
    ("Pacific (standard)", -8 * 60),
    ("Mountain (standard)", -7 * 60),
    ("Central (standard)", -6 * 60),
    ("Eastern (standard)", -5 * 60),
    ("Atlantic (standard)", -4 * 60),
    ("UTC / Zulu", 0),
    ("Central Europe (standard)", 60),
    ("Eastern Europe (standard)", 2 * 60),
    ("Moscow", 3 * 60),
    ("China, Perth", 8 * 60),
    ("Japan, Korea", 9 * 60),
    ("Australia East (standard)", 10 * 60),
];

pub fn offset(minutes: i32) -> FixedOffset {
    FixedOffset::east_opt(minutes * 60).expect("offset in range")
}

pub fn offset_label(minutes: i32) -> String {
    if minutes == 0 {
        return "UTC".into();
    }
    let (sign, m) = if minutes < 0 {
        ('-', -minutes)
    } else {
        ('+', minutes)
    };
    if m % 60 == 0 {
        format!("UTC{sign}{}", m / 60)
    } else {
        format!("UTC{sign}{}:{:02}", m / 60, m % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 23, h, m, s).unwrap()
    }

    #[test]
    fn stopwatch_runs_pauses_and_laps() {
        let mut sw = Stopwatch::default();
        assert_eq!(sw.elapsed(5.0), 0.0);
        assert_eq!(sw.lap(1.0), None, "no laps while stopped");
        sw.start(10.0);
        sw.start(11.0); // second start is ignored
        assert_eq!(sw.elapsed(12.5), 2.5);
        assert_eq!(sw.lap(12.5), Some(2.5));
        assert_eq!(sw.lap(15.0), Some(5.0));
        sw.stop(16.0);
        assert_eq!(sw.elapsed(100.0), 6.0, "stopped time does not advance");
        sw.start(200.0);
        assert_eq!(sw.elapsed(201.0), 7.0, "resumes from where it stopped");
        assert_eq!(sw.lap_splits(), vec![2.5, 2.5]);
        sw.reset();
        assert!(!sw.running() && sw.laps.is_empty() && sw.elapsed(300.0) == 0.0);
        sw.toggle(1.0);
        assert!(sw.running());
        sw.toggle(2.0);
        assert_eq!(sw.elapsed(9.0), 1.0);
    }

    #[test]
    fn countdown_counts_down_and_finishes() {
        let mut c = Countdown::default();
        c.start(0.0);
        assert!(!c.running(), "nothing to count");
        c.set(60.0);
        c.start(100.0);
        assert_eq!(c.remaining(130.0), 30.0);
        assert!(!c.finished(130.0));
        assert!(c.finished(160.0));
        assert_eq!(c.remaining(999.0), 0.0, "never negative");
        // pause keeps the remainder
        let mut c = Countdown::default();
        c.set(60.0);
        c.start(0.0);
        c.pause(20.0);
        assert_eq!(c.remaining(500.0), 40.0);
        // adding time while running extends it from now
        c.start(500.0);
        c.add(510.0, 60.0);
        assert_eq!(c.remaining(510.0), 90.0);
        assert_eq!(c.remaining(540.0), 60.0);
        // adding while paused works too, set() stops it
        c.pause(540.0);
        c.add(600.0, 30.0);
        assert_eq!(c.remaining(600.0), 90.0);
        c.set(10.0);
        assert!(!c.running());
        assert!(
            !c.finished(1000.0),
            "a stopped countdown never reports finished"
        );
    }

    #[test]
    fn duration_formats() {
        assert_eq!(fmt_hms(600.0), "10:00");
        assert_eq!(
            fmt_hms(59.2),
            "01:00",
            "counts down: rounds up so 00:00 means done"
        );
        assert_eq!(fmt_hms(0.0), "00:00");
        assert_eq!(fmt_hms(3725.0), "1:02:05");
        assert_eq!(fmt_hms(-5.0), "00:00");
        assert_eq!(fmt_stopwatch(0.0), "00:00:00.0");
        assert_eq!(fmt_stopwatch(83.49), "00:01:23.4");
        assert_eq!(fmt_stopwatch(3661.99), "01:01:01.9");
    }

    #[test]
    fn log_stamps() {
        let u = at(19, 48, 12);
        let edt = offset(-240);
        assert_eq!(stamp(u, edt, Stamp::Utc), "2026-10-23 1948Z");
        assert_eq!(stamp(u, edt, Stamp::Local), "2026-10-23 15:48 -04:00");
        assert_eq!(stamp(u, edt, Stamp::Iso), "2026-10-23T19:48:12Z");
        // local date can differ from the UTC date
        assert_eq!(
            stamp(at(2, 5, 0), edt, Stamp::Local),
            "2026-10-22 22:05 -04:00"
        );
        assert_eq!(
            stamp(at(0, 0, 0), offset(0), Stamp::Utc),
            "2026-10-23 0000Z"
        );
    }

    #[test]
    fn clock_faces_and_zones() {
        let u = at(19, 48, 12);
        assert_eq!(face(u, offset(0), ClockFmt::H24), "19:48:12");
        assert_eq!(face(u, offset(-300), ClockFmt::H24), "14:48:12");
        assert_eq!(face(u, offset(-300), ClockFmt::H12), "02:48:12 PM");
        assert_eq!(face(u, offset(540), ClockFmt::H24), "04:48:12"); // next day in Japan
        assert_eq!(
            face(u, offset(60), ClockFmt::Iso),
            "2026-10-23T20:48:12+01:00"
        );
        assert_eq!(offset_label(0), "UTC");
        assert_eq!(offset_label(-300), "UTC-5");
        assert_eq!(offset_label(540), "UTC+9");
        assert_eq!(offset_label(330), "UTC+5:30");
        for (_, m) in ZONES {
            let _ = offset(m);
        }
        assert_eq!(day_of_year(at(0, 0, 0)), 296); // Oct 23 2026
    }

    #[test]
    fn top_of_hour() {
        assert_eq!(secs_to_next(at(19, 48, 12), 3600), 11 * 60 + 48);
        assert_eq!(
            secs_to_next(at(19, 0, 0), 3600),
            3600,
            "at the mark, wait a full period"
        );
        assert_eq!(secs_to_next(at(19, 59, 59), 3600), 1);
        assert_eq!(secs_to_next(at(19, 48, 12), 600), 108);
    }
}
