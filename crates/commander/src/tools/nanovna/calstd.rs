//! Ideal references, deviation and CSV for one stored calibration standard.
//! Pure functions so the cal-standard graph can be tested. Issue 75.

use super::rf::Cx;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Open,
    Short,
    Load,
    Thru,
    Isol,
}

impl Kind {
    /// The value a perfect standard reads (S11 for O/S/L, S21 for thru/isolation).
    pub fn ideal(self) -> Cx {
        match self {
            Kind::Open | Kind::Thru => Cx::new(1.0, 0.0),
            Kind::Short => Cx::new(-1.0, 0.0),
            Kind::Load | Kind::Isol => Cx::new(0.0, 0.0),
        }
    }

    /// True when the stored pairs are S21 (thru, isolation).
    pub fn is_s21(self) -> bool {
        matches!(self, Kind::Thru | Kind::Isol)
    }
}

/// Floor used for the ideal null of a load or isolation, and for |Γ| = 0.
pub const NULL_DB: f64 = -80.0;

fn mag_db(v: Cx) -> f64 {
    (20.0 * v.abs().log10()).max(NULL_DB)
}

/// Worst point: index, |measured − ideal|, and the readout in dB. For open,
/// short and thru the readout is the magnitude error |20·log|Γ|| (0 = perfect);
/// for load and isolation it is the level 20·log|Γ| (lower is better, so the
/// worst point is the highest one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Worst {
    pub index: usize,
    pub delta: f64,
    pub db: f64,
}

pub fn worst(kind: Kind, values: &[Cx]) -> Option<Worst> {
    let ideal = kind.ideal();
    values
        .iter()
        .enumerate()
        .map(|(index, v)| {
            let delta = v.sub(ideal).abs();
            let db = match kind {
                Kind::Load | Kind::Isol => mag_db(*v),
                _ => mag_db(*v).abs(),
            };
            Worst { index, delta, db }
        })
        .max_by(|a, b| a.delta.total_cmp(&b.delta))
}

/// Frequency of point `i` in a stored set.
pub fn hz(start: f64, step: f64, i: usize) -> f64 {
    start + step * i as f64
}

/// `hz,re,im` lines with a header, optionally with a compare set's columns.
pub fn csv(start: f64, step: f64, values: &[Cx], compare: Option<&[Cx]>) -> String {
    let mut out = String::from(if compare.is_some() { "hz,re,im,compare_re,compare_im\n" } else { "hz,re,im\n" });
    for (i, v) in values.iter().enumerate() {
        out.push_str(&format!("{},{},{}", hz(start, step, i), v.re, v.im));
        if let Some(c) = compare {
            match c.get(i) {
                Some(c) => out.push_str(&format!(",{},{}", c.re, c.im)),
                None => out.push_str(",,"),
            }
        }
        out.push('\n');
    }
    out
}

/// Error-term magnitudes in dB (directivity, source match, reflection tracking).
pub fn term_db(values: &[Cx]) -> Vec<f64> {
    values.iter().map(|v| mag_db(*v)).collect()
}

/// Index of the point nearest a frequency. Issue 75.
pub fn nearest_x(xs: &[f64], hz: f64) -> Option<usize> {
    (0..xs.len()).filter(|&i| xs[i].is_finite()).min_by(|a, b| (xs[*a] - hz).abs().total_cmp(&(xs[*b] - hz).abs()))
}

/// Index of the point nearest a clicked reflection coefficient (Smith or polar). Issue 75.
pub fn nearest_gamma(values: &[Cx], at: Cx) -> Option<usize> {
    (0..values.len())
        .filter(|&i| values[i].re.is_finite() && values[i].im.is_finite())
        .min_by(|a, b| {
            let d = |c: Cx| (c.re - at.re).hypot(c.im - at.im);
            d(values[*a]).total_cmp(&d(values[*b]))
        })
}

/// Markers on the calibration graph after a click of mode `op` at point `index`:
/// 1 (click) moves M1, 2 (double / Shift) adds the next marker up to `max`
/// (the last one moves when all are placed), 3 (triple / Ctrl) clears them. Issue 75.
pub fn place_marker(marks: &mut Vec<usize>, op: u8, index: Option<usize>, max: usize) {
    match (op, index) {
        (3, _) => marks.clear(),
        (1, Some(i)) => {
            if marks.is_empty() {
                marks.push(i);
            } else {
                marks[0] = i;
            }
        }
        (2, Some(i)) => {
            if marks.len() < max {
                marks.push(i);
            } else if let Some(last) = marks.last_mut() {
                *last = i;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_standards_have_zero_deviation() {
        for kind in [Kind::Open, Kind::Short, Kind::Load, Kind::Thru, Kind::Isol] {
            let w = worst(kind, &vec![kind.ideal(); 5]).unwrap();
            assert_eq!(w.delta, 0.0, "{kind:?}");
            assert!(w.db == 0.0 || w.db == NULL_DB, "{kind:?} {}", w.db);
        }
    }

    #[test]
    fn a_75_ohm_load_reads_14_db_return_loss_at_the_worst_point() {
        // Γ = (75 − 50) / (75 + 50) = 0.2 → −13.98 dB.
        let values = [Cx::new(0.01, 0.0), Cx::new(0.2, 0.0), Cx::new(0.05, 0.0)];
        let w = worst(Kind::Load, &values).unwrap();
        assert_eq!(w.index, 1);
        assert!((w.db + 13.979).abs() < 0.01, "{}", w.db);
    }

    #[test]
    fn a_lossy_open_reports_its_magnitude_error() {
        let values = [Cx::new(1.0, 0.0), Cx::new(0.0, 0.891)];
        let w = worst(Kind::Open, &values).unwrap();
        assert_eq!(w.index, 1);
        assert!((w.db - 1.0).abs() < 0.01);
    }

    #[test]
    fn csv_has_frequency_and_both_sets() {
        let a = [Cx::new(1.0, 0.5), Cx::new(0.25, -1.0)];
        let text = csv(1e6, 1e3, &a, Some(&a[..1]));
        assert_eq!(text, "hz,re,im,compare_re,compare_im\n1000000,1,0.5,1,0.5\n1001000,0.25,-1,,\n");
        assert_eq!(csv(1e6, 1e3, &a[..1], None), "hz,re,im\n1000000,1,0.5\n");
    }

    #[test]
    fn graph_clicks_place_add_and_clear_markers() {
        let xs = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(nearest_x(&xs, 2.4), Some(1));
        assert_eq!(nearest_x(&xs, 9.0), Some(3));
        assert_eq!(nearest_x(&[], 1.0), None);
        let g = [Cx::new(1.0, 0.0), Cx::new(-1.0, 0.0), Cx::new(0.0, 0.1)];
        assert_eq!(nearest_gamma(&g, Cx::new(0.1, 0.0)), Some(2));
        assert_eq!(nearest_gamma(&g, Cx::new(-0.8, 0.1)), Some(1));
        let mut m = Vec::new();
        place_marker(&mut m, 1, Some(2), 3);
        assert_eq!(m, vec![2]);
        place_marker(&mut m, 1, Some(0), 3);
        assert_eq!(m, vec![0]);
        place_marker(&mut m, 2, Some(1), 3);
        place_marker(&mut m, 2, Some(3), 3);
        assert_eq!(m, vec![0, 1, 3]);
        place_marker(&mut m, 2, Some(2), 3);
        assert_eq!(m, vec![0, 1, 2]);
        place_marker(&mut m, 1, None, 3);
        assert_eq!(m, vec![0, 1, 2]);
        place_marker(&mut m, 3, None, 3);
        assert!(m.is_empty());
    }
}
