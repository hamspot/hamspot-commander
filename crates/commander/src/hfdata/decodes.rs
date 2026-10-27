//! The decode list shared by every HF data mode: newest slot first, then by
//! audio offset; duplicates folded; capped; rows stamped in the future are
//! refused and counted so the window can say so.

use super::slots::FUTURE_SLACK_MS;

#[derive(Clone, Debug, PartialEq)]
pub struct Decode {
    pub utc_ms: i64,
    /// Audio offset above the dial, Hz.
    pub offset_hz: f64,
    pub snr_db: Option<f32>,
    /// Time offset reported by the decoder, seconds.
    pub dt_s: Option<f32>,
    pub mode: String,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pushed {
    Added,
    Duplicate,
    Future,
}

pub const DEFAULT_CAP: usize = 500;
/// Two rows within this many Hz and ms with the same text are one decode.
const DUP_HZ: f64 = 10.0;
const DUP_MS: i64 = 1_000;

#[derive(Clone, Debug)]
pub struct DecodeList {
    rows: Vec<Decode>,
    cap: usize,
    pub future_refused: u64,
    pub duplicates: u64,
}

impl Default for DecodeList {
    fn default() -> Self {
        Self::new(DEFAULT_CAP)
    }
}

impl DecodeList {
    pub fn new(cap: usize) -> Self {
        Self { rows: Vec::new(), cap: cap.max(1), future_refused: 0, duplicates: 0 }
    }

    pub fn rows(&self) -> &[Decode] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.future_refused = 0;
        self.duplicates = 0;
    }

    pub fn push(&mut self, d: Decode, now_ms: i64) -> Pushed {
        if d.utc_ms > now_ms + FUTURE_SLACK_MS {
            self.future_refused += 1;
            return Pushed::Future;
        }
        if let Some(old) = self.rows.iter_mut().find(|r| {
            r.mode == d.mode
                && r.text == d.text
                && (r.offset_hz - d.offset_hz).abs() <= DUP_HZ
                && (r.utc_ms - d.utc_ms).abs() <= DUP_MS
        }) {
            if d.snr_db.unwrap_or(f32::MIN) > old.snr_db.unwrap_or(f32::MIN) {
                old.snr_db = d.snr_db;
            }
            self.duplicates += 1;
            return Pushed::Duplicate;
        }
        let at = self
            .rows
            .iter()
            .position(|r| r.utc_ms < d.utc_ms || (r.utc_ms == d.utc_ms && r.offset_hz > d.offset_hz))
            .unwrap_or(self.rows.len());
        self.rows.insert(at, d);
        self.rows.truncate(self.cap);
        Pushed::Added
    }

    /// Rows not after `now_ms` (the window never shows the future).
    pub fn visible(&self, now_ms: i64) -> impl Iterator<Item = &Decode> {
        self.rows.iter().filter(move |r| r.utc_ms <= now_ms + FUTURE_SLACK_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(t: i64, hz: f64, text: &str) -> Decode {
        Decode { utc_ms: t, offset_hz: hz, snr_db: Some(-10.0), dt_s: Some(0.1), mode: "FT8".into(), text: text.into() }
    }

    #[test]
    fn newest_first_then_offset() {
        let mut l = DecodeList::new(10);
        l.push(d(15_000, 1200.0, "A"), 100_000);
        l.push(d(30_000, 900.0, "B"), 100_000);
        l.push(d(30_000, 500.0, "C"), 100_000);
        l.push(d(0, 100.0, "D"), 100_000);
        let t: Vec<_> = l.rows().iter().map(|r| r.text.as_str()).collect();
        assert_eq!(t, ["C", "B", "A", "D"]);
    }

    #[test]
    fn duplicates_fold_and_keep_best_snr() {
        let mut l = DecodeList::new(10);
        assert_eq!(l.push(d(15_000, 1200.0, "CQ K1ABC FN42"), 50_000), Pushed::Added);
        let mut again = d(15_000, 1204.0, "CQ K1ABC FN42");
        again.snr_db = Some(-3.0);
        assert_eq!(l.push(again, 50_000), Pushed::Duplicate);
        assert_eq!(l.len(), 1);
        assert_eq!(l.rows()[0].snr_db, Some(-3.0));
        // Same text next slot is a new decode.
        assert_eq!(l.push(d(30_000, 1200.0, "CQ K1ABC FN42"), 50_000), Pushed::Added);
        // Same text, far offset: a different station.
        assert_eq!(l.push(d(30_000, 2200.0, "CQ K1ABC FN42"), 50_000), Pushed::Added);
        assert_eq!(l.duplicates, 1);
    }

    #[test]
    fn cap_drops_the_oldest() {
        let mut l = DecodeList::new(3);
        for i in 0..5 {
            l.push(d(i * 15_000, 1000.0, &format!("m{i}")), 1_000_000);
        }
        let t: Vec<_> = l.rows().iter().map(|r| r.text.as_str()).collect();
        assert_eq!(t, ["m4", "m3", "m2"]);
    }

    #[test]
    fn future_rows_are_refused_and_counted() {
        let mut l = DecodeList::new(10);
        assert_eq!(l.push(d(60_000, 1000.0, "x"), 10_000), Pushed::Future);
        assert_eq!(l.future_refused, 1);
        assert_eq!(l.len(), 0);
        assert_eq!(l.visible(10_000).count(), 0);
        l.clear();
        assert_eq!(l.future_refused, 0);
    }
}
