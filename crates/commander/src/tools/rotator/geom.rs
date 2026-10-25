//! Rotator geometry (docs/rotator/G-1000-notes.md, G-5500-notes.md).
//!
//! Azimuth here is the value `rotctld` reports. Travel limits come from hamlib
//! (`\dump_state`) when it reports sane ones, and otherwise from a model preset.
//! When the azimuth range is wider than 360, the part past one full turn from
//! the CCW stop is the overlap (G-1000DXA manual p.2 and p.8; G-5500DC p.6).
//! No clocks and no egui: everything is unit tested.

/// Rotation range tolerance (G-1000DXA manual p.15: 450 +/-3).
pub const RANGE_SLOP: f64 = 3.0;
/// The lamp threshold sits on the 360 mark. The half degree keeps a rotator
/// parked on the mark from flickering.
pub const OVERLAP_SLOP: f64 = 0.5;
/// SPEED control range, seconds per 360 (G-1000DXA manual p.15).
pub const SLOWEST_S: f64 = 100.0;
pub const FASTEST_S: f64 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub az_lo: f64,
    pub az_hi: f64,
    pub el_lo: f64,
    pub el_hi: f64,
}

impl Limits {
    pub fn az(self) -> (f64, f64) {
        (self.az_lo, self.az_hi)
    }

    pub fn el(self) -> (f64, f64) {
        (self.el_lo, self.el_hi)
    }

    /// Bearings where the overlap starts and ends on the dial. None when the
    /// travel is one turn or less.
    pub fn overlap_bearings(self) -> Option<(f64, f64)> {
        (self.az_hi - self.az_lo > 360.0 + OVERLAP_SLOP)
            .then(|| (bearing(self.az_lo), bearing(self.az_hi)))
    }

    /// Sane enough to drive the tab: finite, azimuth span 90..720, elevation
    /// inside -90..180 with a positive span.
    pub fn is_sane(self) -> bool {
        let all = [self.az_lo, self.az_hi, self.el_lo, self.el_hi];
        let span = self.az_hi - self.az_lo;
        all.iter().all(|v| v.is_finite())
            && (90.0..=720.0).contains(&span)
            && self.el_hi > self.el_lo
            && self.el_lo >= -90.0
            && self.el_hi <= 180.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Model {
    /// Factory setting: the CCW stop is at 180 (South), 450 of travel.
    G1000South,
    /// G-1000DXA with the stopper moved to 0 (North).
    G1000North,
    /// G-5500 / G-5500DC: az 0..450 from North, el 0..180 (G-5500DC p.2, p.6).
    G5500,
    /// G-5400B / G-5600B: assumed the same as the G-5500 (the manual is a scan).
    G5600B,
    /// Plain az/el rotator: 0..360, 0..90.
    Generic,
}

pub const MODELS: [Model; 5] = [
    Model::G1000South,
    Model::G1000North,
    Model::G5500,
    Model::G5600B,
    Model::Generic,
];

impl Model {
    pub fn label(self) -> &'static str {
        match self {
            Model::G1000South => "G-1000DXA (S stop)",
            Model::G1000North => "G-1000DXA (N stop)",
            Model::G5500 => "G-5500/DC",
            Model::G5600B => "G-5600B",
            Model::Generic => "Generic",
        }
    }

    pub fn limits(self) -> Limits {
        let (az_lo, az_hi, el_hi) = match self {
            Model::G1000South => (-180.0, 270.0, 90.0),
            Model::G1000North => (0.0, 450.0, 90.0),
            Model::G5500 | Model::G5600B => (0.0, 450.0, 180.0),
            Model::Generic => (0.0, 360.0, 90.0),
        };
        Limits { az_lo, az_hi, el_lo: 0.0, el_hi }
    }

    /// The AZ LED shows travel past 360 by default, except for a south stop,
    /// where it shows the compass bearing and OVL.
    pub fn default_show_travel(self) -> bool {
        self != Model::G1000South
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Hamlib,
    Preset,
}

/// Hamlib's limits win when they are sane. Otherwise the model preset.
pub fn effective(hamlib: Option<Limits>, preset: Limits) -> (Limits, Source) {
    match hamlib {
        Some(lim) if lim.is_sane() => (lim, Source::Hamlib),
        _ => (preset, Source::Preset),
    }
}

pub fn travel(az: f64, lim: Limits) -> f64 {
    az - lim.az_lo
}

/// Compass bearing 0..360 for a reported azimuth.
pub fn bearing(az: f64) -> f64 {
    az.rem_euclid(360.0)
}

/// The OVERLAP lamp: more than a full turn from the CCW stop.
pub fn in_overlap(az: f64, lim: Limits) -> bool {
    travel(az, lim) > 360.0 + OVERLAP_SLOP
}

/// A reported azimuth outside the travel limits (plus the tolerance).
pub fn out_of_range(az: f64, lim: Limits) -> bool {
    az < lim.az_lo - RANGE_SLOP || az > lim.az_hi + RANGE_SLOP
}

/// AZ readout value. `show_travel`: the first turn reads as the bearing and the
/// overlap pass reads past 360 (400.0). Otherwise always the bearing.
pub fn display_az(az: f64, lim: Limits, show_travel: bool) -> f64 {
    let b = bearing(az);
    if show_travel && in_overlap(az, lim) { b + 360.0 } else { b }
}

/// Past 90 elevation the antenna points over the top: the same direction as
/// az + 180, el 180 - el (an inference, see G-5500-notes.md).
pub fn flip_equiv(az: f64, el: f64) -> (f64, f64) {
    (bearing(az + 180.0), 180.0 - el)
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
pub fn az_for_bearing(bearing_deg: f64, current_az: f64, lim: Limits) -> f64 {
    let (lo, hi) = lim.az();
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
pub fn path_enters_overlap(from: f64, to: f64, lim: Limits) -> bool {
    in_overlap(from.max(to), lim)
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

/// Tracking direction on one axis. Minus is CCW or DOWN, Plus is CW or UP.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Dir {
    Minus,
    Plus,
    ToPreset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackEnd {
    Running,
    AtPreset,
    AtLimit,
}

/// Commanded position `elapsed_s` into a manual tracking run on one axis that
/// began at `start`. The run ends at the preset (ToPreset) or a limit.
pub fn track_target(
    start: f64,
    rate_dps: f64,
    elapsed_s: f64,
    dir: Dir,
    preset: f64,
    (lo, hi): (f64, f64),
) -> (f64, TrackEnd) {
    let step = rate_dps.max(0.0) * elapsed_s.max(0.0);
    let (target, goal) = match dir {
        Dir::Plus => (start + step, None),
        Dir::Minus => (start - step, None),
        Dir::ToPreset => {
            let sign = if preset >= start { 1.0 } else { -1.0 };
            (start + sign * step, Some(preset))
        }
    };
    if let Some(goal) = goal {
        let passed = if goal >= start { target >= goal } else { target <= goal };
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

/// Seconds until a run at `rate_dps` hits a limit. None when it never moves.
pub fn secs_to_limit(pos: f64, rate_dps: f64, dir: Dir, (lo, hi): (f64, f64)) -> Option<f64> {
    if rate_dps <= 0.0 {
        return None;
    }
    match dir {
        Dir::Plus => Some((hi - pos).max(0.0) / rate_dps),
        Dir::Minus => Some((pos - lo).max(0.0) / rate_dps),
        Dir::ToPreset => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    const S: Limits = Limits { az_lo: -180.0, az_hi: 270.0, el_lo: 0.0, el_hi: 90.0 };
    const N: Limits = Limits { az_lo: 0.0, az_hi: 450.0, el_lo: 0.0, el_hi: 180.0 };

    #[test]
    fn south_stop_overlap_matches_the_manual() {
        assert_eq!(Model::G1000South.limits(), S);
        assert_eq!(S.overlap_bearings(), Some((180.0, 270.0)));
        assert!(!in_overlap(-180.0, S));
        assert!(!in_overlap(180.0, S));
        assert!(in_overlap(200.0, S));
        assert!(close(bearing(-90.0), 270.0));
    }

    #[test]
    fn g5500_is_north_start_with_180_elevation() {
        assert_eq!(Model::G5500.limits(), N);
        assert_eq!(N.overlap_bearings(), Some((0.0, 90.0)));
        assert!(in_overlap(400.0, N));
        assert!(!in_overlap(359.0, N));
        assert_eq!(Model::Generic.limits().overlap_bearings(), None);
    }

    #[test]
    fn hamlib_limits_win_when_sane() {
        let dummy = Limits { az_lo: -180.0, az_hi: 450.0, el_lo: 0.0, el_hi: 90.0 };
        assert_eq!(effective(Some(dummy), N), (dummy, Source::Hamlib));
        assert_eq!(effective(None, N), (N, Source::Preset));
        let zero = Limits { az_lo: 0.0, az_hi: 0.0, el_lo: 0.0, el_hi: 0.0 };
        assert_eq!(effective(Some(zero), N), (N, Source::Preset));
        let nan = Limits { az_lo: f64::NAN, ..dummy };
        assert_eq!(effective(Some(nan), S), (S, Source::Preset));
        let wild = Limits { az_hi: 2000.0, ..dummy };
        assert_eq!(effective(Some(wild), S).1, Source::Preset);
        let el_bad = Limits { el_hi: 400.0, ..dummy };
        assert_eq!(effective(Some(el_bad), S).1, Source::Preset);
    }

    #[test]
    fn range_check_uses_the_effective_limits() {
        assert!(!out_of_range(272.0, S));
        assert!(out_of_range(300.0, S));
        assert!(out_of_range(-190.0, S));
        assert!(!out_of_range(450.0, N));
    }

    #[test]
    fn az_readout_modes() {
        assert!(close(display_az(400.0, N, true), 400.0));
        assert!(close(display_az(400.0, N, false), 40.0));
        assert!(close(display_az(200.0, S, true), 560.0));
        assert!(close(display_az(200.0, S, false), 200.0));
        assert!(close(display_az(-90.0, S, true), 270.0));
    }

    #[test]
    fn flip_points_the_same_way() {
        let (az, el) = flip_equiv(10.0, 120.0);
        assert!(close(az, 190.0) && close(el, 60.0));
        let (az, _) = flip_equiv(300.0, 95.0);
        assert!(close(az, 120.0));
    }

    #[test]
    fn dial_points_run_clockwise_from_up() {
        let (x, y) = dial_xy(0.0, 10.0);
        assert!(close(x, 0.0) && close(y, -10.0));
        let (x, y) = dial_xy(90.0, 10.0);
        assert!(close(x, 10.0) && close(y, 0.0));
    }

    #[test]
    fn ticks_every_five_with_labels_every_thirty() {
        let t = ticks();
        assert_eq!(t.len(), 72);
        assert_eq!(t.iter().filter(|(_, k)| *k == Tick::Major).count(), 12);
    }

    #[test]
    fn preset_in_the_overlap_takes_the_nearest_azimuth() {
        assert!(close(az_for_bearing(200.0, -170.0, S), -160.0));
        assert!(close(az_for_bearing(200.0, 150.0, S), 200.0));
        assert!(close(az_for_bearing(30.0, 420.0, N), 390.0));
        assert!(path_enters_overlap(150.0, 200.0, S));
        assert!(!path_enters_overlap(-160.0, 90.0, S));
    }

    #[test]
    fn speed_maps_to_slew_and_hamlib() {
        assert!(close(slew_dps(40.0), 9.0));
        assert!(close(slew_dps(100.0), 3.6));
        assert_eq!(hamlib_speed(100.0), 1);
        assert_eq!(hamlib_speed(40.0), 100);
        assert_eq!(clamp_rate(20.0, 100.0), (3.6, true));
    }

    #[test]
    fn tracking_steps_per_axis_and_stops_at_limits_or_preset() {
        let az = S.az();
        let (p, end) = track_target(0.0, 0.5, 10.0, Dir::Plus, 0.0, az);
        assert!(close(p, 5.0) && end == TrackEnd::Running);
        let (p, end) = track_target(260.0, 1.0, 30.0, Dir::Plus, 0.0, az);
        assert!(close(p, 270.0) && end == TrackEnd::AtLimit);
        let (p, end) = track_target(100.0, 1.0, 50.0, Dir::ToPreset, 90.0, az);
        assert!(close(p, 90.0) && end == TrackEnd::AtPreset);
        // Elevation clamps to its own range.
        let (p, end) = track_target(80.0, 1.0, 30.0, Dir::Plus, 0.0, S.el());
        assert!(close(p, 90.0) && end == TrackEnd::AtLimit);
        let (p, end) = track_target(80.0, 1.0, 30.0, Dir::Plus, 0.0, N.el());
        assert!(close(p, 110.0) && end == TrackEnd::Running);
        assert_eq!(secs_to_limit(260.0, 2.0, Dir::Plus, az), Some(5.0));
        assert_eq!(secs_to_limit(10.0, 2.0, Dir::Minus, N.el()), Some(5.0));
    }
}
