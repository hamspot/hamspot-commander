//! Calibration state and interpolation of a stored calibration onto a
//! different sweep grid (CAL INT). Issue 65. Nothing here touches egui.

use super::CalSet;

/// What correction the shown trace has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalState {
    /// A stored set on exactly this grid.
    Exact,
    /// A stored set covering the sweep, interpolated onto this grid.
    Interp,
    /// The operator turned correction off.
    Off,
    Uncal,
}

impl CalState {
    pub fn label(self) -> &'static str {
        match self {
            CalState::Exact => "CAL",
            CalState::Interp => "CAL INT",
            CalState::Off => "CAL OFF",
            CalState::Uncal => "UNCAL",
        }
    }
}

fn last_hz(set: &CalSet) -> f64 {
    set.start_hz + set.step_hz * (set.open.len().saturating_sub(1)) as f64
}

/// True when `set` has OSL and its range holds `lo..=hi` (within 1 Hz).
pub fn covers(set: &CalSet, lo: f64, hi: f64) -> bool {
    let n = set.open.len();
    n >= 2
        && set.short.len() == n
        && set.load.len() == n
        && set.step_hz > 0.0
        && lo >= set.start_hz - 1.0
        && hi <= last_hz(set) + 1.0
}

fn interp_rows(rows: &[[f64; 2]], start: f64, step: f64, hz: &[f64]) -> Vec<[f64; 2]> {
    let n = rows.len();
    hz.iter()
        .map(|f| {
            let pos = ((f - start) / step).clamp(0.0, (n - 1) as f64);
            let i = (pos.floor() as usize).min(n - 2);
            let t = pos - i as f64;
            let (a, b) = (rows[i], rows[i + 1]);
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
        })
        .collect()
}

/// The stored raw standards linearly interpolated (real and imaginary
/// parts) onto `hz`, which must be evenly spaced and inside the set. The
/// error terms are then solved from these on the new grid as usual.
pub fn interpolate(set: &CalSet, hz: &[f64]) -> Option<CalSet> {
    if hz.len() < 2 || !covers(set, hz[0], hz[hz.len() - 1]) {
        return None;
    }
    let n = set.open.len();
    let rows = |r: &[[f64; 2]]| {
        if r.len() == n { interp_rows(r, set.start_hz, set.step_hz, hz) } else { Vec::new() }
    };
    Some(CalSet {
        start_hz: hz[0],
        step_hz: hz[1] - hz[0],
        open: rows(&set.open),
        short: rows(&set.short),
        load: rows(&set.load),
        thru: rows(&set.thru),
        isolation: rows(&set.isolation),
        ..set.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::super::rf::{self, Cx};
    use super::*;

    /// Raw standards through a frequency-dependent error box that is linear in f.
    fn boxed(n: usize, start: f64, step: f64) -> CalSet {
        let g = |f: f64, actual: Cx| {
            let k = f / 1e9;
            let ed = Cx::new(0.02 + 0.01 * k, -0.01);
            let es = Cx::new(0.05 * k, 0.02);
            let er = Cx::new(0.9 - 0.05 * k, 0.03 * k);
            let num = er.mul(actual);
            let den = Cx::new(1.0, 0.0).sub(es.mul(actual));
            ed.add(num.div(den).unwrap())
        };
        let rows = |actual: Cx| (0..n).map(|i| { let v = g(start + step * i as f64, actual); [v.re, v.im] }).collect();
        CalSet {
            id: "dev".into(),
            zone: "Z".into(),
            band: String::new(),
            name: String::new(),
            start_hz: start,
            step_hz: step,
            z0: 50.0,
            open: rows(Cx::new(1.0, 0.0)),
            short: rows(Cx::new(-1.0, 0.0)),
            load: rows(Cx::ZERO),
            thru: Vec::new(),
            isolation: Vec::new(),
            saved_at: None,
        }
    }

    #[test]
    fn an_interpolated_calibration_still_corrects_a_known_load() {
        let set = boxed(201, 100e6, 1e6);
        let hz: Vec<f64> = (0..51).map(|i| 150.5e6 + i as f64 * 1.7e6).collect();
        let int = interpolate(&set, &hz).expect("covered");
        assert_eq!(int.open.len(), 51);
        let terms = rf::solve_osl(
            &int.open.iter().map(|r| Cx::new(r[0], r[1])).collect::<Vec<_>>(),
            &int.short.iter().map(|r| Cx::new(r[0], r[1])).collect::<Vec<_>>(),
            &int.load.iter().map(|r| Cx::new(r[0], r[1])).collect::<Vec<_>>(),
        )
        .unwrap();
        // A 0.3 + j0.2 load measured through the same box at point 10.
        let truth = boxed(1, hz[10], 1.0);
        let k = hz[10] / 1e9;
        let ed = Cx::new(0.02 + 0.01 * k, -0.01);
        let es = Cx::new(0.05 * k, 0.02);
        let er = Cx::new(0.9 - 0.05 * k, 0.03 * k);
        let actual = Cx::new(0.3, 0.2);
        let measured = ed.add(er.mul(actual).div(Cx::new(1.0, 0.0).sub(es.mul(actual))).unwrap());
        let fixed = rf::apply_osl(measured, &terms, 10).unwrap();
        assert!(fixed.sub(actual).abs() < 1e-3, "{fixed:?}");
        let _ = truth;
    }

    #[test]
    fn a_sweep_outside_the_set_is_not_interpolated() {
        let set = boxed(11, 100e6, 1e6);
        assert!(covers(&set, 100e6, 110e6));
        assert!(!covers(&set, 99e6, 105e6));
        assert!(interpolate(&set, &[105e6, 111e6]).is_none());
        assert_eq!(CalState::Interp.label(), "CAL INT");
        assert_eq!(CalState::Off.label(), "CAL OFF");
    }
}
