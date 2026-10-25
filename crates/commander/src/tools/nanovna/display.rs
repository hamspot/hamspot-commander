//! Pure display helpers for the VNA panes: graticule scales, tick labels,
//! and marker search. Issues 62, 63, 64, and 67. Nothing here touches egui.

/// A Cartesian pane scale in the R&S style: a reference value on the top
/// line and a fixed number of divisions of `per_div` each going down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    /// Value of the top graticule line.
    pub top: f64,
    /// Units per division.
    pub per_div: f64,
    pub divs: u32,
}

impl Scale {
    pub fn bottom(&self) -> f64 {
        self.top - self.per_div * self.divs as f64
    }

    /// Value of each horizontal graticule line, top first.
    pub fn lines(&self) -> Vec<f64> {
        (0..=self.divs)
            .map(|i| self.top - self.per_div * i as f64)
            .collect()
    }

    /// 0 at the top line, 1 at the bottom line.
    pub fn fraction(&self, value: f64) -> f64 {
        (self.top - value) / (self.per_div * self.divs as f64)
    }
}

/// The 1-2-5 step at or above `raw`.
pub fn nice_step(raw: f64) -> f64 {
    if !(raw.is_finite() && raw > 0.0) {
        return 1.0;
    }
    let exp = raw.log10().floor();
    let base = 10f64.powf(exp);
    let frac = raw / base;
    let nice = if frac <= 1.0 + 1e-9 {
        1.0
    } else if frac <= 2.0 + 1e-9 {
        2.0
    } else if frac <= 5.0 + 1e-9 {
        5.0
    } else {
        10.0
    };
    nice * base
}

/// Fit `divs` divisions of a 1-2-5 size around every finite value.
/// Empty or flat data still gets a usable scale.
pub fn auto_scale(values: &[f64], divs: u32) -> Scale {
    let divs = divs.max(1);
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for v in values.iter().copied().filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if !lo.is_finite() {
        return Scale { top: divs as f64, per_div: 1.0, divs };
    }
    if hi - lo < 1e-12 {
        let pad = if lo.abs() > 1e-12 { lo.abs() * 0.1 } else { 1.0 };
        lo -= pad;
        hi += pad;
    }
    let mut per_div = nice_step((hi - lo) / divs as f64);
    loop {
        let top = (hi / per_div).ceil() * per_div;
        if top - per_div * divs as f64 <= lo + 1e-12 * per_div.max(1.0) {
            return Scale { top, per_div, divs };
        }
        per_div = nice_step(per_div * 1.000_001);
    }
}

/// Evenly spaced 1-2-5 ticks inside `lo..=hi`, about `want` of them.
pub fn ticks(lo: f64, hi: f64, want: usize) -> Vec<f64> {
    if !(lo.is_finite() && hi.is_finite() && hi > lo) || want == 0 {
        return Vec::new();
    }
    let step = nice_step((hi - lo) / want as f64);
    let first = (lo / step).ceil() * step;
    let mut out = Vec::new();
    let mut v = first;
    while v <= hi + step * 1e-9 {
        out.push(if v.abs() < step * 1e-9 { 0.0 } else { v });
        v += step;
        if out.len() > 200 {
            break;
        }
    }
    out
}

/// A frequency in the unit an instrument would print: Hz, kHz, MHz, or GHz.
pub fn fmt_hz(hz: f64) -> String {
    let a = hz.abs();
    let (v, unit) = if a >= 1e9 {
        (hz / 1e9, "GHz")
    } else if a >= 1e6 {
        (hz / 1e6, "MHz")
    } else if a >= 1e3 {
        (hz / 1e3, "kHz")
    } else {
        (hz, "Hz")
    };
    format!("{} {unit}", trim(v, 6))
}

/// A short axis label with no more digits than the step needs.
pub fn fmt_tick(value: f64, step: f64) -> String {
    let places = if step >= 1.0 || step <= 0.0 || !step.is_finite() {
        0
    } else {
        (-step.log10().floor()) as usize
    };
    let text = format!("{value:.places$}");
    if text.starts_with('-') && text.trim_start_matches(['-', '0', '.']).is_empty() {
        text[1..].to_string()
    } else {
        text
    }
}

fn trim(v: f64, places: usize) -> String {
    let text = format!("{v:.places$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

/// Index of the largest finite value.
pub fn search_max(ys: &[f64]) -> Option<usize> {
    ys.iter()
        .enumerate()
        .filter(|(_, v)| v.is_finite())
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
}

/// Index of the smallest finite value.
pub fn search_min(ys: &[f64]) -> Option<usize> {
    ys.iter()
        .enumerate()
        .filter(|(_, v)| v.is_finite())
        .min_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
}

/// Local maxima, tallest first. The ends count when they rise above their neighbour.
pub fn peaks(ys: &[f64]) -> Vec<usize> {
    let n = ys.len();
    let mut out: Vec<usize> = (0..n)
        .filter(|&i| {
            let v = ys[i];
            v.is_finite()
                && (i == 0 || !(ys[i - 1] >= v))
                && (i + 1 == n || !(ys[i + 1] >= v))
        })
        .collect();
    out.sort_by(|a, b| ys[*b].total_cmp(&ys[*a]).then(a.cmp(b)));
    out
}

/// The next lower peak after the one at `from`. Without a current peak it is the highest.
pub fn next_peak(ys: &[f64], from: Option<usize>) -> Option<usize> {
    let list = peaks(ys);
    let Some(from) = from else {
        return list.first().copied();
    };
    let current = ys.get(from).copied().filter(|v| v.is_finite());
    match current {
        None => list.first().copied(),
        Some(cur) => list
            .iter()
            .copied()
            .find(|&i| i != from && (ys[i] < cur || (ys[i] == cur && i > from))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_one_two_five() {
        assert_eq!(nice_step(0.7), 1.0);
        assert_eq!(nice_step(1.3), 2.0);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(7.0), 10.0);
        assert!((nice_step(0.013) - 0.02).abs() < 1e-12);
        assert_eq!(nice_step(-1.0), 1.0);
    }

    #[test]
    fn auto_scale_covers_the_data_on_round_lines() {
        let s = auto_scale(&[-31.2, -3.4, f64::NAN, -12.0], 10);
        assert!(s.top >= -3.4 && s.bottom() <= -31.2, "{s:?}");
        assert_eq!(s.per_div, 5.0);
        assert_eq!(s.top, 0.0);
        assert_eq!(s.lines().len(), 11);
        assert!((s.fraction(s.top)).abs() < 1e-12);
        assert!((s.fraction(s.bottom()) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn auto_scale_handles_flat_and_empty_traces() {
        let flat = auto_scale(&[1.0, 1.0], 10);
        assert!(flat.top > 1.0 && flat.bottom() < 1.0);
        let none = auto_scale(&[f64::INFINITY], 10);
        assert_eq!(none.per_div, 1.0);
    }

    #[test]
    fn ticks_land_on_round_frequencies() {
        let t = ticks(1.0e6, 30.0e6, 6);
        assert_eq!(t.first().copied(), Some(5.0e6));
        assert_eq!(t.last().copied(), Some(30.0e6));
        assert!(ticks(5.0, 1.0, 4).is_empty());
        assert_eq!(fmt_hz(14_200_000.0), "14.2 MHz");
        assert_eq!(fmt_hz(2.4e9), "2.4 GHz");
        assert_eq!(fmt_hz(500.0), "500 Hz");
        assert_eq!(fmt_tick(-0.0001, 0.5), "0.0");
        assert_eq!(fmt_tick(12.0, 5.0), "12");
    }

    #[test]
    fn marker_search_finds_max_min_and_next_peak() {
        let ys = [0.0, 3.0, 1.0, 5.0, 2.0, 4.0, f64::NAN, 1.0];
        assert_eq!(search_max(&ys), Some(3));
        assert_eq!(search_min(&ys), Some(0));
        assert_eq!(peaks(&ys), vec![3, 5, 1, 7]);
        assert_eq!(next_peak(&ys, None), Some(3));
        assert_eq!(next_peak(&ys, Some(3)), Some(5));
        assert_eq!(next_peak(&ys, Some(5)), Some(1));
        assert_eq!(next_peak(&ys, Some(1)), Some(7));
        assert_eq!(next_peak(&ys, Some(7)), None);
        assert!(search_max(&[]).is_none());
    }
}
