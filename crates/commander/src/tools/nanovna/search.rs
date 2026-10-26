//! Marker search helpers: target crossing, multi-peak, band-filter and
//! SWR-bandwidth analysis. Pure functions over (frequency, value) series. Issue 67.

/// Linear interpolation of where `ys` crosses `target` between i and i+1.
fn cross(xs: &[f64], ys: &[f64], i: usize, target: f64) -> f64 {
    let (y0, y1) = (ys[i], ys[i + 1]);
    if (y1 - y0).abs() < f64::EPSILON {
        return xs[i];
    }
    xs[i] + (target - y0) / (y1 - y0) * (xs[i + 1] - xs[i])
}

/// The next frequency after (dir > 0) or before (dir < 0) index `from` where the
/// trace crosses `target`. Starts at the edge when `from` is None.
pub fn target(xs: &[f64], ys: &[f64], target: f64, from: Option<usize>, dir: i32) -> Option<f64> {
    let n = xs.len().min(ys.len());
    if n < 2 {
        return None;
    }
    let hit = |i: usize| {
        let (a, b) = (ys[i] - target, ys[i + 1] - target);
        a.is_finite() && b.is_finite() && (a == 0.0 || a.signum() != b.signum())
    };
    if dir >= 0 {
        let start = from.unwrap_or(0);
        (start..n - 1).find(|&i| hit(i)).map(|i| cross(xs, ys, i, target))
    } else {
        let end = from.unwrap_or(n).min(n - 1);
        (0..end).rev().find(|&i| hit(i)).map(|i| cross(xs, ys, i, target))
    }
}

/// Up to `count` local maxima, highest first, each at least `prominence` above
/// the lower of the minima to either side.
pub fn peaks(ys: &[f64], count: usize, prominence: f64) -> Vec<usize> {
    let n = ys.len();
    let mut found: Vec<usize> = (1..n.saturating_sub(1))
        .filter(|&i| ys[i].is_finite() && ys[i] >= ys[i - 1] && ys[i] > ys[i + 1])
        .filter(|&i| {
            let left = ys[..i].iter().rev().take_while(|v| **v <= ys[i]).fold(f64::INFINITY, |m, v| m.min(*v));
            let right = ys[i + 1..].iter().take_while(|v| **v <= ys[i]).fold(f64::INFINITY, |m, v| m.min(*v));
            ys[i] - left.max(right).min(ys[i]) >= prominence || ys[i] - left.min(right) >= prominence && (left.is_infinite() || right.is_infinite())
        })
        .collect();
    found.sort_by(|a, b| ys[*b].total_cmp(&ys[*a]));
    found.truncate(count);
    found
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filter {
    pub center_hz: f64,
    pub loss_db: f64,
    pub lo3: f64,
    pub hi3: f64,
    pub lo6: f64,
    pub hi6: f64,
}

impl Filter {
    pub fn bw3(&self) -> f64 {
        self.hi3 - self.lo3
    }
    pub fn bw6(&self) -> f64 {
        self.hi6 - self.lo6
    }
    pub fn q(&self) -> f64 {
        self.center_hz / self.bw3()
    }
}

/// Band-pass analysis of an S21 dB trace: the peak, its insertion loss and the
/// 3 dB / 6 dB edges. None when an edge falls outside the sweep.
pub fn bandfilter(xs: &[f64], db: &[f64]) -> Option<Filter> {
    let n = xs.len().min(db.len());
    let peak = (0..n).filter(|&i| db[i].is_finite()).max_by(|a, b| db[*a].total_cmp(&db[*b]))?;
    let top = db[peak];
    let edge = |drop: f64| -> Option<(f64, f64)> {
        let level = top - drop;
        let lo = (0..peak).rev().find(|&i| db[i] <= level).map(|i| cross(xs, db, i, level))?;
        let hi = (peak..n - 1).find(|&i| db[i + 1] <= level).map(|i| cross(xs, db, i, level))?;
        Some((lo, hi))
    };
    let (lo3, hi3) = edge(3.0)?;
    let (lo6, hi6) = edge(6.0)?;
    Some(Filter { center_hz: (lo3 * hi3).sqrt(), loss_db: -top, lo3, hi3, lo6, hi6 })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Notch {
    /// Frequency of the deepest point.
    pub hz: f64,
    /// Reference (pass-band) level minus the minimum, dB, positive.
    pub depth_db: f64,
    /// Pass-band reference level, dB: the higher of the two sweep ends.
    pub ref_db: f64,
    /// Where the trace falls 3 dB below the reference.
    pub lo3: f64,
    pub hi3: f64,
    /// Where the trace crosses half the depth; None when outside the sweep.
    pub lo_half: Option<f64>,
    pub hi_half: Option<f64>,
}

impl Notch {
    pub fn bw3(&self) -> f64 {
        self.hi3 - self.lo3
    }
    pub fn q(&self) -> f64 {
        self.hz / self.bw3()
    }
}

/// Notch (band-stop) analysis of an S21 dB trace: the deepest point, its depth
/// below the pass-band (the higher sweep end), and the 3 dB and half-depth widths.
/// None when the dip is under 3 dB or a 3 dB edge falls outside the sweep.
pub fn notch(xs: &[f64], db: &[f64]) -> Option<Notch> {
    let n = xs.len().min(db.len());
    if n < 3 {
        return None;
    }
    let at = (0..n).filter(|&i| db[i].is_finite()).min_by(|a, b| db[*a].total_cmp(&db[*b]))?;
    let ref_db = db[0].max(db[n - 1]);
    let depth = ref_db - db[at];
    if !(depth > 3.0) {
        return None;
    }
    let edge = |level: f64| -> (Option<f64>, Option<f64>) {
        let lo = (0..at).rev().find(|&i| db[i] >= level).map(|i| cross(xs, db, i, level));
        let hi = (at..n - 1).find(|&i| db[i + 1] >= level).map(|i| cross(xs, db, i, level));
        (lo, hi)
    };
    let (lo3, hi3) = edge(ref_db - 3.0);
    let (lo_half, hi_half) = edge(ref_db - depth / 2.0);
    Some(Notch { hz: xs[at], depth_db: depth, ref_db, lo3: lo3?, hi3: hi3?, lo_half, hi_half })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwrBand {
    pub min_hz: f64,
    pub min_swr: f64,
    /// Edges where SWR crosses the limit; None when the trace never dips under it,
    /// and an edge equals the sweep end when it is still under the limit there.
    pub edges: Option<(f64, f64)>,
}

/// Minimum SWR and the bandwidth around it under `limit` (2:1 usually).
pub fn swr_band(xs: &[f64], swr: &[f64], limit: f64) -> Option<SwrBand> {
    let n = xs.len().min(swr.len());
    let at = (0..n).filter(|&i| swr[i].is_finite()).min_by(|a, b| swr[*a].total_cmp(&swr[*b]))?;
    let edges = (swr[at] <= limit).then(|| {
        let lo = (0..at).rev().find(|&i| swr[i] > limit).map(|i| cross(xs, swr, i, limit)).unwrap_or(xs[0]);
        let hi = (at..n - 1).find(|&i| swr[i + 1] > limit).map(|i| cross(xs, swr, i, limit)).unwrap_or(xs[n - 1]);
        (lo, hi)
    });
    Some(SwrBand { min_hz: xs[at], min_swr: swr[at], edges })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(n: usize) -> Vec<f64> {
        (0..n).map(|i| 1e6 + i as f64 * 1e3).collect()
    }

    #[test]
    fn target_interpolates_both_directions() {
        let xs = axis(5);
        let ys = [0.0, 2.0, 4.0, 2.0, 0.0];
        assert_eq!(target(&xs, &ys, 3.0, None, 1), Some(1_001_500.0));
        assert_eq!(target(&xs, &ys, 3.0, Some(2), 1), Some(1_002_500.0));
        assert_eq!(target(&xs, &ys, 3.0, Some(2), -1), Some(1_001_500.0));
        assert_eq!(target(&xs, &ys, 9.0, None, 1), None);
    }

    #[test]
    fn peaks_skip_ripple_and_sort_highest_first() {
        let ys = [0.0, 5.0, 0.0, 0.2, 0.1, 9.0, 0.0, 3.0, 0.0];
        assert_eq!(peaks(&ys, 5, 1.0), vec![5, 1, 7]);
        assert_eq!(peaks(&ys, 2, 1.0), vec![5, 1]);
    }

    #[test]
    fn bandfilter_finds_edges_q_and_loss() {
        let xs = axis(11);
        let db: Vec<f64> = (0..11).map(|i| -2.0 - (i as f64 - 5.0).powi(2)).collect();
        let f = bandfilter(&xs, &db).unwrap();
        assert!((f.loss_db - 2.0).abs() < 1e-9);
        assert!((f.bw3() - 3_333.3).abs() < 1.0, "{f:?}");
        assert!(f.bw6() > f.bw3());
        assert!(f.q() > 100.0);
        assert!(bandfilter(&xs, &vec![-1.0; 11]).is_none());
    }

    #[test]
    fn notch_finds_depth_frequency_and_widths() {
        // Synthetic notch: 0 dB pass-band, -30 dB at 100 Hz, 1 dB per Hz each side.
        let xs: Vec<f64> = (0..201).map(|i| i as f64).collect();
        let db: Vec<f64> = xs.iter().map(|x| -(30.0 - (x - 100.0).abs()).max(0.0)).collect();
        let n = notch(&xs, &db).unwrap();
        assert_eq!(n.hz, 100.0);
        assert!((n.depth_db - 30.0).abs() < 1e-9);
        assert!((n.lo3 - 73.0).abs() < 1e-6 && (n.hi3 - 127.0).abs() < 1e-6, "{n:?}");
        assert!((n.bw3() - 54.0).abs() < 1e-6);
        assert!((n.lo_half.unwrap() - 85.0).abs() < 1e-6 && (n.hi_half.unwrap() - 115.0).abs() < 1e-6);
        assert!((n.q() - 100.0 / 54.0).abs() < 1e-9);
        // A flat trace has no notch.
        assert!(notch(&xs, &vec![-1.0; 201]).is_none());
        // Edge outside the sweep.
        let half: Vec<f64> = (0..20).map(|i| -(30.0 - i as f64)).collect();
        assert!(notch(&xs[..20], &half).is_none());
    }

    #[test]
    fn swr_band_reports_min_and_two_to_one_edges() {
        let xs = axis(5);
        let b = swr_band(&xs, &[3.0, 1.5, 1.1, 1.5, 3.0], 2.0).unwrap();
        assert_eq!((b.min_hz, b.min_swr), (1_002_000.0, 1.1));
        let (lo, hi) = b.edges.unwrap();
        assert!((lo - 1_000_666.6).abs() < 1.0 && (hi - 1_003_333.3).abs() < 1.0);
        assert!(swr_band(&xs, &[3.0; 5], 2.0).unwrap().edges.is_none());
    }
}
