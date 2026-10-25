//! Limit lines and the PASS/FAIL verdict for a trace. Issue 71. Nothing
//! here touches egui.

use serde::{Deserialize, Serialize};

/// One straight limit segment from (start_hz, start_v) to (stop_hz, stop_v).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start_hz: f64,
    pub stop_hz: f64,
    pub start_v: f64,
    pub stop_v: f64,
    /// Upper limit: a point above it fails. Lower: a point below it fails.
    pub upper: bool,
}

/// The limit set saved for one trace format.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LimitSet {
    #[serde(default)]
    pub on: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segs: Vec<Segment>,
}

impl Segment {
    /// The limit value at `hz`, or None outside the segment.
    pub fn at(&self, hz: f64) -> Option<f64> {
        let (lo, hi) = (self.start_hz.min(self.stop_hz), self.start_hz.max(self.stop_hz));
        if !(hz >= lo && hz <= hi) {
            return None;
        }
        if (self.stop_hz - self.start_hz).abs() < f64::EPSILON {
            return Some(if self.upper { self.start_v.min(self.stop_v) } else { self.start_v.max(self.stop_v) });
        }
        let t = (hz - self.start_hz) / (self.stop_hz - self.start_hz);
        Some(self.start_v + (self.stop_v - self.start_v) * t)
    }

    pub fn fails(&self, hz: f64, value: f64) -> bool {
        match self.at(hz) {
            Some(limit) if value.is_finite() => if self.upper { value > limit } else { value < limit },
            // An infinite reading (open, SWR ∞) fails an upper limit.
            Some(_) => self.upper && value.is_infinite() && value > 0.0,
            None => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    /// Points that fall under at least one segment.
    pub tested: usize,
    pub failing: Vec<usize>,
}

impl Verdict {
    pub fn pass(&self) -> bool {
        self.tested > 0 && self.failing.is_empty()
    }

    /// "PASS", "FAIL" or "NO DATA" — always text, never colour alone.
    pub fn label(&self) -> &'static str {
        if self.tested == 0 { "LIMIT: NO POINTS" } else if self.failing.is_empty() { "PASS" } else { "FAIL" }
    }
}

pub fn check(xs: &[f64], ys: &[f64], segs: &[Segment]) -> Verdict {
    let mut tested = 0;
    let mut failing = Vec::new();
    for (i, (x, y)) in xs.iter().zip(ys).enumerate() {
        let mut covered = false;
        let mut bad = false;
        for s in segs {
            if s.at(*x).is_some() {
                covered = true;
                bad |= s.fails(*x, *y);
            }
        }
        if covered {
            tested += 1;
        }
        if bad {
            failing.push(i);
        }
    }
    Verdict { tested, failing }
}

/// An upper SWR limit across a band, for example 2:1 on 20 m.
pub fn swr_preset(lo_hz: f64, hi_hz: f64, swr: f64) -> Segment {
    Segment { start_hz: lo_hz, stop_hz: hi_hz, start_v: swr, stop_v: swr, upper: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fail_shows_when_a_point_crosses_a_limit() {
        let xs = [14.0e6, 14.1e6, 14.2e6, 14.3e6, 15.0e6];
        let segs = [swr_preset(14.0e6, 14.35e6, 2.0)];
        let ok = check(&xs, &[1.2, 1.5, 1.9, 2.0, 9.0], &segs);
        assert!(ok.pass());
        assert_eq!(ok.tested, 4);
        assert_eq!(ok.label(), "PASS");
        let bad = check(&xs, &[1.2, 2.1, 1.9, f64::INFINITY, 9.0], &segs);
        assert!(!bad.pass());
        assert_eq!(bad.failing, vec![1, 3]);
        assert_eq!(bad.label(), "FAIL");
        assert_eq!(check(&xs, &[1.0; 5], &[swr_preset(1e6, 2e6, 2.0)]).label(), "LIMIT: NO POINTS");
    }

    #[test]
    fn sloped_upper_and_lower_segments_interpolate() {
        let up = Segment { start_hz: 0.0, stop_hz: 10.0, start_v: -10.0, stop_v: -20.0, upper: true };
        assert_eq!(up.at(5.0), Some(-15.0));
        assert!(up.fails(5.0, -14.0));
        assert!(!up.fails(5.0, -16.0));
        let low = Segment { upper: false, ..up };
        assert!(low.fails(5.0, -16.0));
        assert!(!low.fails(11.0, -100.0));
    }

    #[test]
    fn limit_sets_round_trip() {
        let set = LimitSet { on: true, segs: vec![swr_preset(7.0e6, 7.3e6, 1.5)] };
        let text = serde_json::to_string(&set).unwrap();
        assert_eq!(serde_json::from_str::<LimitSet>(&text).unwrap(), set);
        assert_eq!(serde_json::from_str::<LimitSet>("{}").unwrap(), LimitSet::default());
    }
}
