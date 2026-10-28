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

/// Top of every SWR chart: 10:1. Points above it are clipped at the top edge.
pub const SWR_MAX: f64 = 10.0;
/// SWR graticule labels, taken from an analog SWR meter face.
pub const SWR_TICKS: [f64; 7] = [1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 10.0];

/// Height of an SWR value on a chart: 0 at the bottom (1:1), 1 at the top
/// (10:1). Spaced like a meter face, linear in |Γ| = (s−1)/(s+1), so 2:1 sits
/// about 40 % up and 3:1 about 60 %. Anything above 10:1, infinite SWR
/// included, clips to the top; below 1 clamps to the bottom; NaN has no place.
pub fn swr_height(swr: f64) -> Option<f64> {
    if swr.is_nan() {
        return None;
    }
    if swr >= SWR_MAX {
        return Some(1.0);
    }
    if swr <= 1.0 {
        return Some(0.0);
    }
    let g = |s: f64| (s - 1.0) / (s + 1.0);
    Some(g(swr) / g(SWR_MAX))
}

/// Red the SWR trace turns above 3:1 (DANGER).
pub const SWR_RED: (u8, u8, u8) = (0xD9, 0x42, 0x3A);
/// SWR where the trace starts to fade to red, and where it is fully red.
pub const SWR_WARN: f64 = 3.0;
pub const SWR_RED_AT: f64 = 4.0;

/// Trace colour for an SWR value: `base` at or below 3:1, blending to red
/// by 4:1, red above (clipped and infinite included). NaN keeps `base`.
pub fn swr_color(base: (u8, u8, u8), swr: f64) -> (u8, u8, u8) {
    if swr.is_nan() || swr <= SWR_WARN {
        return base;
    }
    let t = ((swr - SWR_WARN) / (SWR_RED_AT - SWR_WARN)).clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round() as u8;
    (mix(base.0, SWR_RED.0), mix(base.1, SWR_RED.1), mix(base.2, SWR_RED.2))
}

/// Inverse of `swr_height` on 0..=1 (1 maps to 10:1).
pub fn swr_at_height(h: f64) -> f64 {
    let g = h.clamp(0.0, 1.0) * (SWR_MAX - 1.0) / (SWR_MAX + 1.0);
    (1.0 + g) / (1.0 - g)
}

/// True when an SWR value is off the top of the chart (infinite included).
pub fn swr_clipped(swr: f64) -> bool {
    swr > SWR_MAX
}

/// SWR number for display: ">10" above the chart top instead of a huge
/// number, "N/A" when unknown, otherwise with `decimals` places.
pub fn fmt_swr(swr: f64, decimals: usize) -> String {
    if swr.is_nan() {
        "N/A".into()
    } else if swr > SWR_MAX {
        ">10".into()
    } else {
        format!("{:.*}", decimals, swr.max(1.0))
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

/// A fixed, operator-set scale for one trace format. Issue 69.
/// `ref_pos` counts divisions up from the bottom graticule line, like the
/// R&S reference position; the reference value sits on that line.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ManualScale {
    pub ref_value: f64,
    pub ref_pos: f64,
    pub per_div: f64,
}

impl ManualScale {
    pub fn to_scale(&self, divs: u32) -> Scale {
        let per_div = if self.per_div.is_finite() && self.per_div > 0.0 { self.per_div } else { 1.0 };
        let pos = self.ref_pos.clamp(0.0, divs as f64);
        Scale { top: self.ref_value + (divs as f64 - pos) * per_div, per_div, divs }
    }

    /// Freeze what auto-scale shows, keeping `ref_value` on its line.
    pub fn from_scale(scale: &Scale, ref_value: f64) -> Self {
        let pos = scale.divs as f64 - (scale.top - ref_value) / scale.per_div;
        Self { ref_value, ref_pos: pos.clamp(0.0, scale.divs as f64), per_div: scale.per_div }
    }

    /// Move the reference line (and the trace with it) by `divs` divisions.
    pub fn shifted(self, by_divs: f64, divs: u32) -> Self {
        Self { ref_pos: (self.ref_pos + by_divs).clamp(0.0, divs as f64), ..self }
    }

    /// One 1-2-5 step coarser (`up`) or finer per division.
    pub fn stepped(self, up: bool) -> Self {
        let per_div = if up {
            nice_step(self.per_div * 1.000_001)
        } else {
            // The largest 1-2-5 value below the current one.
            let mut s = nice_step(self.per_div / 2.6);
            while nice_step(s * 1.000_001) < self.per_div * 0.999_999 {
                s = nice_step(s * 1.000_001);
            }
            s
        };
        Self { per_div, ..self }
    }
}

/// The value the reference triangle marks when auto-scale chose the scale:
/// zero when the grid spans it, otherwise the top line.
pub fn auto_ref(scale: &Scale) -> f64 {
    if scale.top >= 0.0 && scale.bottom() <= 0.0 { 0.0 } else { scale.top }
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
    fn swr_color_fades_to_red_between_three_and_four() {
        let b = (10, 200, 30);
        assert_eq!(swr_color(b, 1.0), b);
        assert_eq!(swr_color(b, 3.0), b);
        assert_eq!(swr_color(b, f64::NAN), b);
        for v in [4.0, 6.0, 10.0, 150_000.0, f64::INFINITY] {
            assert_eq!(swr_color(b, v), SWR_RED);
        }
        let steps: Vec<(u8, u8, u8)> = (0..=10).map(|i| swr_color(b, 3.0 + i as f64 / 10.0)).collect();
        assert!(steps.windows(2).all(|w| w[1].0 >= w[0].0 && w[1].1 <= w[0].1), "{steps:?}");
        assert!((swr_at_height(swr_height(3.0).unwrap()) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn swr_height_is_fixed_one_to_ten_and_clips_spikes() {
        assert_eq!(swr_height(1.0), Some(0.0));
        assert_eq!(swr_height(0.5), Some(0.0));
        assert_eq!(swr_height(10.0), Some(1.0));
        assert_eq!(swr_height(150_000.0), Some(1.0));
        assert_eq!(swr_height(f64::INFINITY), Some(1.0));
        assert_eq!(swr_height(f64::NAN), None);
        let h: Vec<f64> = SWR_TICKS.iter().map(|t| swr_height(*t).unwrap()).collect();
        assert!(h.windows(2).all(|w| w[0] < w[1]), "{h:?}");
        assert!((swr_height(2.0).unwrap() - 0.4074).abs() < 1e-3);
        assert!((swr_height(3.0).unwrap() - 0.6111).abs() < 1e-3);
        assert!(swr_clipped(f64::INFINITY) && swr_clipped(10.01) && !swr_clipped(10.0) && !swr_clipped(f64::NAN));
    }

    #[test]
    fn swr_text_shows_over_ten_instead_of_huge_numbers() {
        assert_eq!(fmt_swr(150_000.0, 2), ">10");
        assert_eq!(fmt_swr(f64::INFINITY, 3), ">10");
        assert_eq!(fmt_swr(f64::NAN, 2), "N/A");
        assert_eq!(fmt_swr(2.618, 3), "2.618");
        assert_eq!(fmt_swr(10.0, 2), "10.00");
    }

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
    fn a_manual_scale_holds_its_reference_and_steps() {
        let m = ManualScale { ref_value: 0.0, ref_pos: 10.0, per_div: 10.0 };
        let s = m.to_scale(10);
        assert_eq!((s.top, s.bottom()), (0.0, -100.0));
        let moved = m.shifted(-5.0, 10).to_scale(10);
        assert_eq!((moved.top, moved.bottom()), (50.0, -50.0));
        assert!((moved.fraction(0.0) - 0.5).abs() < 1e-12);
        assert_eq!(m.shifted(-50.0, 10).ref_pos, 0.0);
        assert_eq!(m.stepped(true).per_div, 20.0);
        assert_eq!(m.stepped(false).per_div, 5.0);
        assert_eq!(ManualScale { per_div: 2.0, ..m }.stepped(false).per_div, 1.0);
        let auto = auto_scale(&[-31.0, -3.0], 10);
        let frozen = ManualScale::from_scale(&auto, auto_ref(&auto));
        assert_eq!(frozen.to_scale(10), auto);
    }

    #[test]
    fn auto_scale_fits_every_point_inside_the_grid() {
        for data in [vec![-42.0, -0.3, -17.0], vec![1.02, 9.7], vec![1e-9, 3e-9], vec![-180.0, 179.0]] {
            let s = auto_scale(&data, 10);
            for v in &data {
                let f = s.fraction(*v);
                assert!((0.0..=1.0).contains(&f), "{v} outside {s:?}");
            }
        }
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
