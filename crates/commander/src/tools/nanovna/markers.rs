//! Pure marker logic for the VNA page: nearest point, reference and delta
//! values, and the readout formats a marker can show independent of the
//! trace format. Issue 66. Nothing here touches egui.

use std::f64::consts::PI;

use super::rf::{self, Cx, Sample};

/// Normal markers M1..M8 plus the reference marker R.
pub const NORMAL: usize = 8;
/// Slot index of the reference marker R.
pub const REF: usize = NORMAL;
pub const SLOTS: usize = NORMAL + 1;

pub fn slot_name(slot: usize) -> String {
    if slot == REF { "R".into() } else { format!("M{}", slot + 1) }
}

/// Index of the sweep point closest to `hz`. Markers are stored as a
/// frequency so they stay put when a new sweep or a new point count arrives.
pub fn nearest_index(samples: &[Sample], hz: f64) -> Option<usize> {
    if !hz.is_finite() {
        return None;
    }
    samples
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (a.1.hz - hz)
                .abs()
                .partial_cmp(&(b.1.hz - hz).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
}

/// Frequency and value difference of a marker against R: (Δf, Δvalue).
pub fn delta(ys: &[f64], hz: &[f64], at: usize, reference: usize) -> Option<(f64, f64)> {
    let (f, r) = (hz.get(at)?, hz.get(reference)?);
    let (y, ry) = (ys.get(at)?, ys.get(reference)?);
    Some((f - r, y - ry))
}

/// "+200 kHz / -6.1 dB" the way the table prints a delta.
pub fn delta_text(df: f64, dv: f64, unit: &str) -> String {
    let sign = if df < 0.0 { "-" } else { "+" };
    let unit = if unit == "U" { "" } else { unit };
    let value = if dv.is_finite() { format!("{dv:+.3}") } else { "INF".into() };
    format!("{sign}{} / {value} {unit}", super::display::fmt_hz(df.abs())).trim_end().to_string()
}

/// What the marker VALUE column shows. Trace follows the trace format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readout {
    Trace,
    Db,
    RjX,
    LC,
    Zabs,
    GjB,
    Swr,
    Index,
}

pub const READOUTS: [(Readout, &str); 8] = [
    (Readout::Trace, "Trace"),
    (Readout::Db, "dB"),
    (Readout::RjX, "R+jX"),
    (Readout::LC, "L / C"),
    (Readout::Zabs, "|Z|"),
    (Readout::GjB, "G+jB"),
    (Readout::Swr, "SWR"),
    (Readout::Index, "Point"),
];

/// Admittance Y = 1/Z in siemens. None at a short.
pub fn admittance(z: Cx) -> Option<Cx> {
    Cx::new(1.0, 0.0).div(z)
}

/// Equivalent series inductance (positive) or capacitance (negative X) at `hz`.
/// Returns (henries, farads); the one that does not apply is NaN.
pub fn equivalent_lc(x: f64, hz: f64) -> (f64, f64) {
    let w = 2.0 * PI * hz;
    if !x.is_finite() || w == 0.0 {
        (f64::NAN, f64::NAN)
    } else if x >= 0.0 {
        (x / w, f64::NAN)
    } else {
        (f64::NAN, -1.0 / (w * x))
    }
}

fn si(value: f64, unit: &str) -> String {
    if !value.is_finite() {
        return "INF".into();
    }
    let a = value.abs();
    let (k, p) = if a >= 1.0 {
        (1.0, "")
    } else if a >= 1e-3 {
        (1e3, "m")
    } else if a >= 1e-6 {
        (1e6, "µ")
    } else if a >= 1e-9 {
        (1e9, "n")
    } else {
        (1e12, "p")
    };
    format!("{:.3} {p}{unit}", value * k)
}

/// A marker value in the chosen readout format. `s21` picks the parameter
/// for dB. Impedance formats always come from S11.
pub fn readout_text(fmt: Readout, sample: &Sample, z0: f64, index: usize, s21: bool) -> String {
    let z = rf::impedance(sample.s11, z0);
    match fmt {
        Readout::Trace => String::new(),
        Readout::Db => {
            let v = if s21 { rf::db(sample.s21) } else { rf::db(sample.s11) };
            if v.is_finite() { format!("{v:.2} dB") } else { "-INF dB".into() }
        }
        Readout::RjX => match z {
            Some(z) if z.re.is_finite() => format!("{:.2} {:+.2}j Ω", z.re, z.im),
            _ => "OPEN".into(),
        },
        Readout::LC => match z {
            Some(z) => {
                let (l, c) = equivalent_lc(z.im, sample.hz);
                if l.is_finite() { si(l, "H") } else if c.is_finite() { si(c, "F") } else { "INF".into() }
            }
            None => "OPEN".into(),
        },
        Readout::Zabs => match z {
            Some(z) if z.re.is_finite() => format!("{:.2} Ω", z.abs()),
            _ => "INF Ω".into(),
        },
        Readout::GjB => match z.and_then(admittance) {
            Some(y) => format!("{:.3} {:+.3}j mS", y.re * 1e3, y.im * 1e3),
            None => "SHORT".into(),
        },
        Readout::Swr => {
            let v = rf::swr(sample.s11);
            format!("{} : 1", crate::tools::nanovna::display::fmt_swr(v, 3))
        }
        Readout::Index => format!("#{index}"),
    }
}

/// Where a plot click with `clicks` presses puts a marker: single = the
/// active marker, double = the next normal marker after it, triple = R.
pub fn click_slot(active: usize, clicks: u8) -> usize {
    match clicks {
        3 => REF,
        2 => {
            if active >= NORMAL { 0 } else { (active + 1) % NORMAL }
        }
        _ => active,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_z(r: f64, x: f64, hz: f64) -> Sample {
        let z = Cx::new(r, x);
        let g = z.sub(Cx::new(50.0, 0.0)).div(z.add(Cx::new(50.0, 0.0))).unwrap();
        Sample { hz, s11: g, s21: Cx::new(0.5, 0.0) }
    }

    #[test]
    fn markers_find_the_nearest_point_and_deltas_are_against_r() {
        let s: Vec<Sample> = (0..11).map(|i| at_z(50.0, 0.0, 14.0e6 + i as f64 * 20e3)).collect();
        assert_eq!(nearest_index(&s, 14.09e6), Some(4));
        assert_eq!(nearest_index(&s, 1.0), Some(0));
        assert_eq!(nearest_index(&[], 1.0), None);
        let hz: Vec<f64> = s.iter().map(|s| s.hz).collect();
        let ys = vec![-12.1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -18.2];
        let (df, dv) = delta(&ys, &hz, 10, 0).unwrap();
        assert!((df - 200e3).abs() < 1e-6);
        assert!((dv + 6.1).abs() < 1e-9);
        assert_eq!(delta_text(df, dv, "dB"), "+200 kHz / -6.100 dB");
    }

    #[test]
    fn readout_formats_convert_correctly() {
        let s = at_z(100.0, 50.0, 10.0e6);
        assert_eq!(readout_text(Readout::RjX, &s, 50.0, 3, false), "100.00 +50.00j Ω");
        assert_eq!(readout_text(Readout::Zabs, &s, 50.0, 3, false), "111.80 Ω");
        assert_eq!(readout_text(Readout::Swr, &s, 50.0, 3, false), "2.618 : 1");
        assert_eq!(readout_text(Readout::Index, &s, 50.0, 3, false), "#3");
        // Y = 1/(100+j50) = 8 - j4 mS.
        assert_eq!(readout_text(Readout::GjB, &s, 50.0, 3, false), "8.000 -4.000j mS");
        // X = +50 Ω at 10 MHz is 795.775 nH.
        assert_eq!(readout_text(Readout::LC, &s, 50.0, 3, false), "795.775 nH");
        // X = -50 Ω at 10 MHz is 318.310 pF.
        let c = at_z(50.0, -50.0, 10.0e6);
        assert_eq!(readout_text(Readout::LC, &c, 50.0, 0, false), "318.310 pF");
        // |S21| 0.5 is -6.02 dB.
        assert_eq!(readout_text(Readout::Db, &s, 50.0, 0, true), "-6.02 dB");
    }

    #[test]
    fn click_modes_place_active_next_and_reference() {
        assert_eq!(click_slot(0, 1), 0);
        assert_eq!(click_slot(0, 2), 1);
        assert_eq!(click_slot(7, 2), 0);
        assert_eq!(click_slot(2, 3), REF);
        assert_eq!(slot_name(REF), "R");
        assert_eq!(slot_name(0), "M1");
    }
}
