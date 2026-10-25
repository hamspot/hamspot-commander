//! Pane grid and trace assignment for the VNA page. Issues 62 and 63.
//! Nothing here touches egui. Traces name their format by a stable id
//! ("S11 SMITH", "S21 DB MAG"), so the station file stays readable.

use serde::{Deserialize, Serialize};

/// Up to four traces, TRC1..TRC4.
pub const MAX_TRACES: usize = 4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceCfg {
    pub view: String,
    #[serde(default)]
    pub pane: u8,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// 1, 2 or 4 panes.
    #[serde(default = "one")]
    pub panes: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traces: Vec<TraceCfg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximized: Option<u8>,
}

fn one() -> u8 {
    1
}

impl Default for Layout {
    fn default() -> Self {
        Self { panes: 1, traces: Vec::new(), maximized: None }
    }
}

impl Layout {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// Panes shown now: one when a pane is maximized.
    pub fn shown(&self) -> Vec<u8> {
        match self.maximized {
            Some(p) if p < self.panes => vec![p],
            _ => (0..self.panes.max(1)).collect(),
        }
    }

    /// Choose 1, 2 or 4 panes. Traces in panes that no longer exist move to the last pane.
    pub fn set_panes(&mut self, n: u8) {
        let n = match n {
            0 | 1 => 1,
            2 | 3 => 2,
            _ => 4,
        };
        self.panes = n;
        self.maximized = None;
        for t in &mut self.traces {
            if t.pane >= n {
                t.pane = n - 1;
            }
        }
    }

    /// Move a trace to the next pane, wrapping.
    pub fn move_next(&mut self, trace: usize) {
        let n = self.panes.max(1);
        if let Some(t) = self.traces.get_mut(trace) {
            t.pane = (t.pane + 1) % n;
        }
    }

    /// Swap the contents of pane order: reverse which pane each trace sits in.
    pub fn swap_order(&mut self) {
        let n = self.panes.max(1);
        for t in &mut self.traces {
            t.pane = n - 1 - t.pane.min(n - 1);
        }
    }

    /// Double click on a pane header: maximize it, or restore the grid.
    pub fn toggle_max(&mut self, pane: u8) {
        self.maximized = if self.maximized == Some(pane) { None } else { Some(pane) };
    }

    /// Visible traces in a pane, in trace order.
    pub fn in_pane(&self, pane: u8) -> Vec<usize> {
        self.traces.iter().enumerate().filter(|(_, t)| t.pane == pane).map(|(i, _)| i).collect()
    }

    /// The default four-pane set from the issue.
    pub fn default_four() -> Self {
        let t = |v: &str, p: u8| TraceCfg { view: v.into(), pane: p, hidden: false };
        Self {
            panes: 4,
            traces: vec![t("S11 RETURN LOSS", 0), t("S21 DB MAG", 1), t("S11 SMITH", 2), t("S21 DELAY", 3)],
            maximized: None,
        }
    }
}

/// Columns and rows for a pane count.
pub fn grid(n: usize) -> (usize, usize) {
    match n {
        0 | 1 => (1, 1),
        2 => (1, 2),
        _ => (2, 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_persist_and_traces_move_between_panes() {
        let mut l = Layout::default_four();
        let text = serde_json::to_string(&l).unwrap();
        assert_eq!(serde_json::from_str::<Layout>(&text).unwrap(), l);
        assert_eq!(serde_json::from_str::<Layout>("{}").unwrap(), Layout::default());
        l.move_next(3);
        assert_eq!(l.traces[3].pane, 0);
        assert_eq!(l.in_pane(0), vec![0, 3]);
        l.set_panes(2);
        assert!(l.traces.iter().all(|t| t.pane < 2));
        l.swap_order();
        assert_eq!(l.traces[1].pane, 0);
    }

    #[test]
    fn maximize_and_restore() {
        let mut l = Layout::default_four();
        assert_eq!(l.shown(), vec![0, 1, 2, 3]);
        l.toggle_max(2);
        assert_eq!(l.shown(), vec![2]);
        l.toggle_max(2);
        assert_eq!(l.shown().len(), 4);
        assert_eq!(grid(1), (1, 1));
        assert_eq!(grid(2), (1, 2));
        assert_eq!(grid(4), (2, 2));
    }
}
