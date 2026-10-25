//! Rotator geometry, after the Yaesu G-1000 series (see docs/rotator/G-1000-notes.md).
//!
//! Azimuth here is the value `rotctld` reports. A south-stop rotator reports
//! -180..270 and a north-stop rotator 0..450. Travel is degrees from the CCW stop,
//! 0..450. Past 360 the OVERLAP lamp lights (2102B manual p.2, p.8).
//! No clocks and no egui: everything is unit tested.

/// Total travel, 360 plus 90 of overlap (2102B p.1).
pub const TRAVEL: f64 = 450.0;
/// Rotation range tolerance (2102B p.15: 450 +/-3).
pub const RANGE_SLOP: f64 = 3.0;
/// The lamp threshold sits on the 360 mark. The half degree keeps a rotator
/// parked on the mark from flickering.
pub const OVERLAP_SLOP: f64 = 0.5;
/// SPEED control range, seconds per 360 (2102B p.15, G-800DXA/G-1000DXA).
pub const SLOWEST_S: f64 = 100.0;
pub const FASTEST_S: f64 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopAt {
    /// Factory setting: the CCW stop is at 180 (South).
    South,
    /// Stopper moved to 0 (North).
    North,
}

impl StopAt {
    /// The `rotctld` azimuth at the CCW stop.
    pub fn start_az(self) -> f64 {
        match self {
            StopAt::South => -180.0,
            StopAt::North => 0.0,
        }
    }

    pub fn limits(self) -> (f64, f64) {
        let start = self.start_az();
        (start, start + TRAVEL)
    }

    /// Bearings where the overlap begins and ends on the dial.
    pub fn overlap_bearings(self) -> (f64, f64) {
        let start = bearing(self.start_az());
        (start, bearing(start + TRAVEL - 360.0))
    }
}

pub fn travel(az: f64, stop: StopAt) -> f64 {
    az - stop.start_az()
}

/// Compass bearing 0..360 for a reported azimuth.
pub fn bearing(az: f64) -> f64 {
    az.rem_euclid(360.0)
}

/// The OVERLAP lamp: more than a full turn from the CCW stop.
pub fn in_overlap(az: f64, stop: StopAt) -> bool {
    travel(az, stop) > 360.0 + OVERLAP_SLOP
}

/// A reported azimuth the G-1000 cannot reach. That usually means the stop
/// setting does not match the rotator, or the rotator is misreporting.
pub fn out_of_range(az: f64, stop: StopAt) -> bool {
    let t = travel(az, stop);
    t < -RANGE_SLOP || t > TRAVEL + RANGE_SLOP
}

/// Point on a dial of radius `r` for a bearing. 0 is up, clockwise, screen y down.
pub fn dial_xy(bearing_deg: f64, r: f64) -> (f64, f64) {
    let a = bearing_deg.to_radians();
    (r * a.sin(), -r * a.cos())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    Minor,
    Major,
}

/// One tick per 5 degrees. Every 30 is major and carries a label.
pub fn ticks() -> Vec<(f64, Tick)> {
    (0..72)
        .map(|i| {
            let b = i as f64 * 5.0;
            let kind = if i % 6 == 0 { Tick::Major } else { Tick::Minor };
            (b, kind)
        })
        .collect()
}

/// The azimuth to command for a bearing. Inside the overlap there are two;
/// take the one nearest the current azimuth so the antenna does not unwind.
pub fn az_for_bearing(bearing_deg: f64, current_az: f64, stop: StopAt) -> f64 {
    let (lo, hi) = stop.limits();
    let b = bearing(bearing_deg);
    (-2..=2)
        .map(|n| b + 360.0 * n as f64)
        .filter(|az| *az >= lo - 1e-9 && *az <= hi + 1e-9)
        .min_by(|a, b| {
            (a - current_az)
                .abs()
                .partial_cmp(&(b - current_az).abs())
                .expect("finite")
        })
        .unwrap_or_else(|| current_az.clamp(lo, hi))
}

/// A move from `from` to `to` passes through or ends in the overlap.
pub fn path_enters_overlap(from: f64, to: f64, stop: StopAt) -> bool {
    in_overlap(from.max(to), stop)
}

/// Slew rate at a SPEED setting, degrees per second.
pub fn slew_dps(secs_per_rev: f64) -> f64 {
    360.0 / secs_per_rev.clamp(FASTEST_S, SLOWEST_S)
}

/// Hamlib `move` speed 1..100 for a SPEED setting: 100 s is 1, 40 s is 100.
pub fn hamlib_speed(secs_per_rev: f64) -> i32 {
    let s = secs_per_rev.clamp(FASTEST_S, SLOWEST_S);
    let frac = (SLOWEST_S - s) / (SLOWEST_S - FASTEST_S);
    (1.0 + frac * 99.0).round() as i32
}

/// A tracking rate the rotator cannot keep up with at this SPEED is clamped.
/// The flag is true when it was clamped.
pub fn clamp_rate(rate_dps: f64, secs_per_rev: f64) -> (f64, bool) {
    let max = slew_dps(secs_per_rev);
    let rate = rate_dps.max(0.0);
    if rate > max { (max, true) } else { (rate, false) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackDir {
    Ccw,
    Cw,
    ToPreset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackEnd {
    Running,
    AtPreset,
    AtLimit,
}

/// Commanded azimuth `elapsed_s` into a manual tracking run that began at
/// `start_az`. The run ends at the preset (ToPreset) or a travel stop.
pub fn track_target(
    start_az: f64,
    rate_dps: f64,
    elapsed_s: f64,
    dir: TrackDir,
    preset_az: f64,
    stop: StopAt,
) -> (f64, TrackEnd) {
    let (lo, hi) = stop.limits();
    let step = rate_dps.max(0.0) * elapsed_s.max(0.0);
    let (target, goal) = match dir {
        TrackDir::Cw => (start_az + step, None),
        TrackDir::Ccw => (start_az - step, None),
        TrackDir::ToPreset => {
            let sign = if preset_az >= start_az { 1.0 } else { -1.0 };
            (start_az + sign * step, Some(preset_az))
        }
    };
    if let Some(goal) = goal {
        let passed = if goal >= start_az { target >= goal } else { target <= goal };
        if passed {
            return (goal.clamp(lo, hi), TrackEnd::AtPreset);
        }
    }
    if target >= hi {
        (hi, TrackEnd::AtLimit)
    } else if target <= lo {
        (lo, TrackEnd::AtLimit)
    } else {
        (target, TrackEnd::Running)
    }
}

/// Seconds until a run at `rate_dps` hits a travel stop. None when it never moves.
pub fn secs_to_limit(az: f64, rate_dps: f64, dir: TrackDir, stop: StopAt) -> Option<f64> {
    if rate_dps <= 0.0 {
        return None;
    }
    let (lo, hi) = stop.limits();
    match dir {
        TrackDir::Cw => Some((hi - az).max(0.0) / rate_dps),
        TrackDir::Ccw => Some((az - lo).max(0.0) / rate_dps),
        TrackDir::ToPreset => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn south_stop_overlap_matches_the_manual() {
        let s = StopAt::South;
        assert_eq!(s.limits(), (-180.0, 270.0));
        assert_eq!(s.overlap_bearings(), (180.0, 270.0));
        assert!(!in_overlap(-180.0, s));
        assert!(!in_overlap(180.0, s));
        assert!(in_overlap(200.0, s));
        assert!(in_overlap(270.0, s));
        assert!(close(bearing(-90.0), 270.0));
        assert!(close(bearing(200.0), 200.0));
    }

    #[test]
    fn north_stop_overlap_is_the_last_ninety() {
        let n = StopAt::North;
        assert_eq!(n.limits(), (0.0, 450.0));
        assert_eq!(n.overlap_bearings(), (0.0, 90.0));
        assert!(!in_overlap(359.0, n));
        assert!(in_overlap(400.0, n));
        assert!(close(bearing(400.0), 40.0));
    }

    #[test]
    fn range_check_allows_the_spec_tolerance() {
        let s = StopAt::South;
        assert!(!out_of_range(272.0, s));
        assert!(out_of_range(300.0, s));
        assert!(out_of_range(-190.0, s));
        assert!(out_of_range(450.0, s));
        assert!(!out_of_range(450.0, StopAt::North));
    }

    #[test]
    fn dial_points_run_clockwise_from_up() {
        let (x, y) = dial_xy(0.0, 10.0);
        assert!(close(x, 0.0) && close(y, -10.0));
        let (x, y) = dial_xy(90.0, 10.0);
        assert!(close(x, 10.0) && close(y, 0.0));
        let (x, y) = dial_xy(180.0, 10.0);
        assert!(close(x, 0.0) && close(y, 10.0));
    }

    #[test]
    fn ticks_every_five_with_labels_every_thirty() {
        let t = ticks();
        assert_eq!(t.len(), 72);
        assert_eq!(t.iter().filter(|(_, k)| *k == Tick::Major).count(), 12);
        assert_eq!(t[6], (30.0, Tick::Major));
        assert_eq!(t[1], (5.0, Tick::Minor));
    }

    #[test]
    fn preset_in_the_overlap_takes_the_nearest_azimuth() {
        let s = StopAt::South;
        // Bearing 200 is -160 or 200. From -170, go to -160, not around.
        assert!(close(az_for_bearing(200.0, -170.0, s), -160.0));
        assert!(close(az_for_bearing(200.0, 150.0, s), 200.0));
        // Outside the overlap there is one choice.
        assert!(close(az_for_bearing(90.0, 260.0, s), 90.0));
        assert!(close(az_for_bearing(30.0, 0.0, StopAt::North), 30.0));
        assert!(close(az_for_bearing(30.0, 420.0, StopAt::North), 390.0));
        assert!(path_enters_overlap(150.0, 200.0, s));
        assert!(!path_enters_overlap(-160.0, 90.0, s));
    }

    #[test]
    fn speed_maps_to_slew_and_hamlib() {
        assert!(close(slew_dps(40.0), 9.0));
        assert!(close(slew_dps(100.0), 3.6));
        assert!(close(slew_dps(10.0), 9.0));
        assert_eq!(hamlib_speed(100.0), 1);
        assert_eq!(hamlib_speed(40.0), 100);
        assert_eq!(hamlib_speed(70.0), 51);
        assert_eq!(clamp_rate(1.0, 100.0), (1.0, false));
        assert_eq!(clamp_rate(20.0, 100.0), (3.6, true));
    }

    #[test]
    fn tracking_steps_at_the_rate_and_stops_at_limits_or_preset() {
        let s = StopAt::South;
        let (az, end) = track_target(0.0, 0.5, 10.0, TrackDir::Cw, 0.0, s);
        assert!(close(az, 5.0) && end == TrackEnd::Running);
        let (az, end) = track_target(0.0, 0.5, 10.0, TrackDir::Ccw, 0.0, s);
        assert!(close(az, -5.0) && end == TrackEnd::Running);
        let (az, end) = track_target(260.0, 1.0, 30.0, TrackDir::Cw, 0.0, s);
        assert!(close(az, 270.0) && end == TrackEnd::AtLimit);
        let (az, end) = track_target(-170.0, 1.0, 30.0, TrackDir::Ccw, 0.0, s);
        assert!(close(az, -180.0) && end == TrackEnd::AtLimit);
        let (az, end) = track_target(100.0, 1.0, 5.0, TrackDir::ToPreset, 90.0, s);
        assert!(close(az, 95.0) && end == TrackEnd::Running);
        let (az, end) = track_target(100.0, 1.0, 50.0, TrackDir::ToPreset, 90.0, s);
        assert!(close(az, 90.0) && end == TrackEnd::AtPreset);
        assert_eq!(secs_to_limit(260.0, 2.0, TrackDir::Cw, s), Some(5.0));
        assert_eq!(secs_to_limit(260.0, 0.0, TrackDir::Cw, s), None);
    }
}
