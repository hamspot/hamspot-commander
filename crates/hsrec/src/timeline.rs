//! Security-camera style timeline layout, free of UI code.
use crate::db::Recording;

pub fn parse_ms(utc: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(utc).ok().map(|t| t.timestamp_millis())
}

pub fn ms_to_utc(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub const MIN_SPAN_MS: i64 = 10_000;
pub const MAX_SPAN_MS: i64 = 7 * 86_400_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Left edge, unix ms.
    pub start: i64,
    pub span: i64,
    /// Follow "now" at the right edge.
    pub live: bool,
}

impl View {
    pub fn live(now: i64, span: i64) -> Self {
        Self { start: now - span, span, live: true }
    }
    pub fn end(&self) -> i64 {
        self.start + self.span
    }
    pub fn x(&self, t: i64, width: f32) -> f32 {
        ((t - self.start) as f64 / self.span as f64 * width as f64) as f32
    }
    pub fn t(&self, x: f32, width: f32) -> i64 {
        self.start + (x as f64 / width.max(1.0) as f64 * self.span as f64) as i64
    }
    /// Zoom by `factor` (<1 zooms in) keeping time under `anchor_x` fixed.
    pub fn zoom(&mut self, factor: f64, anchor_x: f32, width: f32) {
        let at = self.t(anchor_x, width);
        let span = ((self.span as f64 * factor) as i64).clamp(MIN_SPAN_MS, MAX_SPAN_MS);
        let frac = anchor_x as f64 / width.max(1.0) as f64;
        self.start = at - (span as f64 * frac) as i64;
        self.span = span;
    }
    pub fn pan_px(&mut self, dx: f32, width: f32) {
        self.start -= (dx as f64 / width.max(1.0) as f64 * self.span as f64) as i64;
        self.live = false;
    }
    /// Jump to the local day containing `t` (+/- days).
    pub fn day(&mut self, t: i64, days: i64) {
        use chrono::{Local, TimeZone};
        let local = Local.timestamp_millis_opt(t).single().unwrap_or_else(Local::now);
        let midnight = local.date_naive().and_hms_opt(0, 0, 0).expect("midnight");
        let start = Local.from_local_datetime(&midnight).earliest().map(|d| d.timestamp_millis()).unwrap_or(t);
        self.start = start + days * 86_400_000;
        self.span = 86_400_000;
        self.live = false;
    }
    /// Keep the right edge on `now` while live.
    pub fn follow(&mut self, now: i64) {
        if self.live {
            self.start = now - self.span;
        }
    }
}

/// Tick step for the axis so labels are about 90 px apart.
pub fn tick_step(span: i64, width: f32) -> i64 {
    const STEPS: [i64; 14] = [1_000, 5_000, 10_000, 30_000, 60_000, 300_000, 600_000, 1_800_000, 3_600_000,
        3 * 3_600_000, 6 * 3_600_000, 12 * 3_600_000, 86_400_000, 7 * 86_400_000];
    let want = span as f64 * 90.0 / width.max(1.0) as f64;
    STEPS.iter().copied().find(|s| *s as f64 >= want).unwrap_or(7 * 86_400_000)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub id: i64,
    pub lane: usize,
    /// Row inside the lane when blocks overlap.
    pub sub: usize,
    pub x0: f32,
    pub x1: f32,
    pub live: bool,
}

/// Lane names in first-seen order of the given recordings (sorted by lane name).
pub fn lanes(recs: &[Recording]) -> Vec<String> {
    let mut v: Vec<String> = recs.iter().map(|r| r.meta.lane()).collect();
    v.sort();
    v.dedup();
    v
}

/// Blocks for recordings visible in `view`. Unfinished recordings grow to `now`.
/// Every block is at least 2 px wide so short calls stay clickable.
pub fn layout(recs: &[Recording], lanes: &[String], view: &View, width: f32, now: i64) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut ends: Vec<Vec<f32>> = vec![Vec::new(); lanes.len()];
    let mut sorted: Vec<&Recording> = recs.iter().collect();
    sorted.sort_by_key(|r| parse_ms(&r.meta.start_utc).unwrap_or(0));
    for r in sorted {
        let Some(t0) = parse_ms(&r.meta.start_utc) else { continue };
        let live = r.meta.end_utc.is_empty();
        let t1 = if live { now } else { parse_ms(&r.meta.end_utc).unwrap_or(t0 + r.meta.duration_ms) };
        if t1 < view.start || t0 > view.end() {
            continue;
        }
        let Some(lane) = lanes.iter().position(|l| *l == r.meta.lane()) else { continue };
        let x0 = view.x(t0, width);
        let x1 = view.x(t1, width).max(x0 + 2.0);
        let rows = &mut ends[lane];
        let sub = match rows.iter().position(|e| *e <= x0) {
            Some(i) => i,
            None => {
                rows.push(f32::MIN);
                rows.len() - 1
            }
        };
        rows[sub] = x1;
        out.push(Block { id: r.id, lane, sub, x0, x1, live });
    }
    out
}

/// Case-insensitive filter on lane, channel, TG number, radio id, notes.
pub fn matches(r: &Recording, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {} {} {} {}",
        r.meta.lane(),
        r.meta.channel_name,
        r.meta.talkgroup.map(|t| t.to_string()).unwrap_or_default(),
        r.meta.radio_unit_id.map(|t| t.to_string()).unwrap_or_default(),
        r.notes,
        r.meta.radio_name
    )
    .to_lowercase();
    hay.contains(&q)
}

/// Next recording after `id` in time (play-next).
pub fn next_after(recs: &[Recording], id: i64) -> Option<i64> {
    let mut v: Vec<&Recording> = recs.iter().collect();
    v.sort_by(|a, b| a.meta.start_utc.cmp(&b.meta.start_utc));
    let i = v.iter().position(|r| r.id == id)?;
    v.get(i + 1).map(|r| r.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::meta;
    use crate::db::Db;

    fn recs() -> Vec<Recording> {
        let db = Db::open_in_memory().unwrap();
        db.insert("a", &meta("hackrf_tg", Some(1), "2026-01-01T00:00:10Z", "2026-01-01T00:00:20Z")).unwrap();
        db.insert("b", &meta("hackrf_tg", Some(1), "2026-01-01T00:00:15Z", "2026-01-01T00:00:30Z")).unwrap();
        db.insert("c", &meta("hackrf_tg", Some(2), "2026-01-01T00:00:40Z", "")).unwrap();
        db.all().unwrap()
    }

    #[test]
    fn time_to_x_and_back() {
        let v = View { start: 1000, span: 10_000, live: false };
        assert_eq!(v.x(6000, 100.0), 50.0);
        assert_eq!(v.t(50.0, 100.0), 6000);
        let mut z = v;
        z.zoom(0.5, 50.0, 100.0);
        assert_eq!(z.span, MIN_SPAN_MS.max(5000));
        assert_eq!(z.t(50.0, 100.0), 6000);
        let mut p = v;
        p.pan_px(10.0, 100.0);
        assert_eq!(p.start, 0);
        assert!(!p.live);
    }

    #[test]
    fn lanes_and_overlaps() {
        let r = recs();
        let l = lanes(&r);
        assert_eq!(l, vec!["TG 1", "TG 2"]);
        let t0 = parse_ms("2026-01-01T00:00:00Z").unwrap();
        let v = View { start: t0, span: 60_000, live: false };
        let now = t0 + 50_000;
        let b = layout(&r, &l, &v, 600.0, now);
        assert_eq!(b.len(), 3);
        assert_eq!((b[0].lane, b[0].sub), (0, 0));
        assert_eq!((b[1].lane, b[1].sub), (0, 1)); // overlaps a
        assert_eq!((b[2].lane, b[2].sub), (1, 0));
        assert!(b[2].live);
        assert_eq!(b[2].x1, 500.0); // grows to now
        assert_eq!(b[0].x0, 100.0);
    }

    #[test]
    fn filter_next_ticks() {
        let r = recs();
        assert!(matches(&r[0], "tg 1"));
        assert!(!matches(&r[2], "tg 1"));
        assert_eq!(next_after(&r, r[0].id), Some(r[1].id));
        assert_eq!(next_after(&r, r[2].id), None);
        assert_eq!(tick_step(3_600_000, 900.0), 600_000);
    }
}
