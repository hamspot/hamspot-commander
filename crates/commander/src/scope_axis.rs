//! Vertical axis of the Tune spectrum scope. With AUTO FLOOR the bottom sits a
//! few dB under a robust noise-floor estimate and the top just above the
//! strongest signal, so the plot spends its height on signals, not noise.
//! The axis moves slowly (hysteresis + rate limit) so it does not jitter.

/// dB under the estimated noise floor for the bottom of the axis.
pub const FLOOR_MARGIN: f32 = 8.0;
/// dB above the strongest bin for the top.
pub const TOP_MARGIN: f32 = 6.0;
/// The axis never spans less than this.
pub const MIN_SPAN: f32 = 30.0;
/// Fraction of bins under the noise estimate (robust against carriers).
pub const NOISE_PERCENTILE: f32 = 0.2;
/// Changes smaller than this are ignored (hysteresis).
pub const DEADBAND: f32 = 2.0;
/// Most the axis edges move per frame.
pub const MAX_STEP: f32 = 1.0;
/// Fixed span when auto is off (the original behaviour).
pub const MANUAL_SPAN: f32 = 70.0;

/// Noise floor (low percentile) and peak of the finite bins, or None.
pub fn estimate(trace: &[f32]) -> Option<(f32, f32)> {
    let mut v: Vec<f32> = trace.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    let i = ((v.len() - 1) as f32 * NOISE_PERCENTILE).round() as usize;
    Some((v[i], v[v.len() - 1]))
}

/// Target (bottom, top) before smoothing.
pub fn target(noise: f32, peak: f32) -> (f32, f32) {
    let bottom = noise - FLOOR_MARGIN;
    (bottom, (peak + TOP_MARGIN).max(bottom + MIN_SPAN))
}

/// Range drawn on the axis: edges, tick step, all clean multiples of `step`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub bottom: f32,
    pub top: f32,
    pub step: f32,
}

impl Axis {
    /// Round outwards to a 5 or 10 dB grid (at most ~8 divisions).
    pub fn clean(bottom: f32, top: f32) -> Self {
        let span = (top - bottom).max(1.0);
        let step = if span <= 40.0 { 5.0 } else if span <= 80.0 { 10.0 } else { 20.0 };
        let b = (bottom / step).floor() * step;
        let t = ((top / step).ceil() * step).max(b + step);
        Axis { bottom: b, top: t, step }
    }

    /// The original axis: the strongest bin at the top, 70 dB down, 7 x 10 dB.
    pub fn manual(trace: &[f32]) -> Self {
        let top = trace.iter().copied().filter(|v| v.is_finite()).fold(f32::MIN, f32::max);
        let top = if top > f32::MIN / 2.0 { top } else { 0.0 };
        Axis { bottom: top - MANUAL_SPAN, top, step: 10.0 }
    }
}

/// Smoothed auto-floor state, one per scope.
#[derive(Default, Clone, Debug)]
pub struct AutoFloor {
    edges: Option<(f32, f32)>,
}

impl AutoFloor {
    pub fn reset(&mut self) {
        self.edges = None;
    }

    /// Feed one frame's trace; returns the axis to draw (None until data).
    pub fn update(&mut self, trace: &[f32]) -> Option<Axis> {
        let Some((noise, peak)) = estimate(trace) else {
            return self.edges.map(|(b, t)| Axis::clean(b, t));
        };
        let (tb, tt) = target(noise, peak);
        let (b, t) = match self.edges {
            None => (tb, tt),
            Some((b, t)) => (follow(b, tb), follow(t, tt)),
        };
        let t = t.max(b + MIN_SPAN);
        self.edges = Some((b, t));
        Some(Axis::clean(b, t))
    }
}

fn follow(cur: f32, want: f32) -> f32 {
    let d = want - cur;
    if d.abs() <= DEADBAND {
        cur
    } else {
        cur + d.clamp(-MAX_STEP, MAX_STEP)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn carrier() -> Vec<f32> {
        (0..1024).map(|i| if (500..504).contains(&i) { -30.0 } else { -80.0 + ((i * 7919) % 13) as f32 * 0.3 - 1.8 }).collect()
    }

    #[test]
    fn noise_and_carrier() {
        let (n, p) = estimate(&carrier()).unwrap();
        assert!((n + 80.0).abs() < 2.5, "noise {n}");
        assert_eq!(p, -30.0);
        let mut a = AutoFloor::default();
        let ax = a.update(&carrier()).unwrap();
        assert!(ax.bottom <= n - FLOOR_MARGIN && ax.bottom > n - FLOOR_MARGIN - ax.step - 0.01, "{ax:?}");
        assert!(ax.top >= -30.0 + TOP_MARGIN && ax.top < -30.0 + TOP_MARGIN + ax.step + 0.01, "{ax:?}");
    }

    #[test]
    fn flat_trace_gets_the_minimum_span() {
        let mut a = AutoFloor::default();
        let ax = a.update(&[-90.0; 256]).unwrap();
        assert!(ax.top - ax.bottom >= MIN_SPAN);
        assert!(ax.bottom <= -98.0);
    }

    #[test]
    fn nan_inf_and_empty_never_panic() {
        let mut a = AutoFloor::default();
        assert_eq!(a.update(&[]), None);
        assert_eq!(a.update(&[f32::NAN, f32::INFINITY, f32::NEG_INFINITY]), None);
        let ax = a.update(&[f32::NAN, -70.0, f32::NEG_INFINITY, -60.0]).unwrap();
        assert!(ax.bottom.is_finite() && ax.top.is_finite());
        // Data drops out: keep the last axis.
        assert_eq!(a.update(&[f32::NAN]), Some(ax));
        let m = Axis::manual(&[f32::NAN]);
        assert!(m.bottom.is_finite() && m.top.is_finite());
    }

    #[test]
    fn slightly_varying_noise_holds_the_axis_and_big_moves_are_rate_limited() {
        let mut a = AutoFloor::default();
        let first = a.update(&carrier()).unwrap();
        for k in 0..200 {
            let wobble = ((k * 37) % 7) as f32 * 0.5 - 1.5; // +-1.5 dB
            let t: Vec<f32> = carrier().iter().map(|v| v + wobble).collect();
            assert_eq!(a.update(&t).unwrap(), first, "axis moved on small wobble at frame {k}");
        }
        // Noise jumps 20 dB: edges move at most MAX_STEP a frame.
        let up: Vec<f32> = carrier().iter().map(|v| v + 20.0).collect();
        let mut last = a.edges.unwrap();
        for _ in 0..40 {
            a.update(&up);
            let e = a.edges.unwrap();
            assert!((e.0 - last.0).abs() <= MAX_STEP + 1e-4 && (e.1 - last.1).abs() <= MAX_STEP + 1e-4);
            last = e;
        }
        assert!(last.0 > -80.0, "floor followed the jump");
    }

    #[test]
    fn ticks_are_clean_multiples() {
        for (b, t) in [(-88.3, -24.0), (-98.0, -68.0), (-133.0, 2.0), (-5.0, -4.0)] {
            let ax = Axis::clean(b, t);
            assert!([5.0, 10.0, 20.0].contains(&ax.step));
            assert_eq!(ax.bottom % ax.step, 0.0, "{ax:?}");
            assert_eq!(ax.top % ax.step, 0.0, "{ax:?}");
            assert!(ax.bottom <= b && ax.top >= t);
        }
    }

    #[test]
    fn manual_axis_is_the_original() {
        let ax = Axis::manual(&[-80.0, -24.0, -70.0]);
        assert_eq!(ax, Axis { bottom: -94.0, top: -24.0, step: 10.0 });
    }
}
