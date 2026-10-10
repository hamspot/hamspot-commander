//! Loadable band plan. A zone is a named list of bands. Each band carries a
//! few mode segments (cw, digital, voice, mixed) from the ARRL voluntary plan.
//! https://www.arrl.org/band-plan

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_JSON: &str = include_str!("../../../bsp/default.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Use {
    Cw,
    Digital,
    Voice,
    Mixed,
}

impl Use {
    pub fn label(self) -> &'static str {
        match self {
            Use::Cw => "CW",
            Use::Digital => "Digital",
            Use::Voice => "Voice",
            Use::Mixed => "Mixed",
        }
    }

    pub fn rgb(self) -> [u8; 3] {
        match self {
            Use::Cw => [214, 168, 72],
            Use::Digital => [90, 150, 210],
            Use::Voice => [92, 184, 120],
            Use::Mixed => [160, 160, 160],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub lo_hz: f64,
    pub hi_hz: f64,
    #[serde(rename = "use")]
    pub use_kind: Use,
    /// Listening service, when this file is a service map rather than a mode plan.
    /// Absent on the amateur plan. Known names are in [`SERVICES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
}

/// Legend order for a service map. Colors follow the RT-950 CPS swatches.
pub const SERVICES: &[&str] = &[
    "broadcast",
    "military",
    "marine",
    "aero",
    "ham",
    "cb",
    "government",
    "time",
    "utility",
];

pub fn service_label(name: &str) -> String {
    match name {
        "broadcast" => "Broadcast".into(),
        "military" => "Military".into(),
        "marine" => "Marine".into(),
        "aero" => "Aero".into(),
        "ham" => "Ham".into(),
        "cb" => "CB".into(),
        "government" => "Government".into(),
        "time" => "Time".into(),
        "utility" => "Utility".into(),
        other => other.to_string(),
    }
}

pub fn service_rgb(name: &str) -> [u8; 3] {
    match name {
        "ham" => [92, 184, 120],
        "broadcast" => [90, 150, 210],
        "aero" => [214, 168, 72],
        "marine" => [72, 168, 184],
        "government" => [130, 136, 150],
        "cb" => [196, 112, 72],
        "military" => [176, 96, 96],
        "time" => [176, 140, 220],
        "utility" => [110, 110, 110],
        _ => [160, 160, 160],
    }
}

/// Ham covers a shared edge with broadcast, matching the RT-950 dial.
pub fn service_rank(name: &str) -> u8 {
    match name {
        "ham" => 80,
        "cb" => 70,
        "military" => 60,
        "aero" => 50,
        "marine" => 40,
        "time" => 35,
        "government" => 30,
        "utility" => 20,
        "broadcast" => 10,
        _ => 0,
    }
}

pub fn segment_rgb(seg: &Segment) -> [u8; 3] {
    match seg.service.as_deref().filter(|name| !name.is_empty()) {
        Some(name) => service_rgb(name),
        None => seg.use_kind.rgb(),
    }
}

pub fn segment_title(seg: &Segment) -> String {
    match seg.service.as_deref().filter(|name| !name.is_empty()) {
        Some(name) => service_label(name),
        None => seg.use_kind.label().to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BandLimit {
    pub name: String,
    pub chip: String,
    pub lo_hz: f64,
    pub hi_hz: f64,
    pub home_hz: f64,
    pub mode: String,
    pub segments: Vec<Segment>,
    /// Session-only transmit frequency. Not written to the file.
    #[serde(default, skip_serializing)]
    pub tx_hz: Option<f64>,
}

/// How a zone is drawn on the tune-page bandspread.
/// `bands` is one row per band (the ham stack). `spans` is one row, and every
/// band in the zone is a subband on that row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowLayout {
    #[default]
    Bands,
    Spans,
}

fn is_bands(layout: &RowLayout) -> bool {
    *layout == RowLayout::Bands
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    pub name: String,
    pub color: [u8; 3],
    #[serde(default, skip_serializing_if = "is_bands")]
    pub layout: RowLayout,
    pub bands: Vec<BandLimit>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BandPlan {
    pub version: u32,
    pub zones: Vec<Zone>,
}

pub fn parse(text: &str) -> Result<BandPlan, String> {
    let mut plan: BandPlan = serde_json::from_str(text).map_err(|err| err.to_string())?;
    validate(&plan)?;
    for zone in &mut plan.zones {
        for band in &mut zone.bands {
            if band.tx_hz.is_none() {
                band.tx_hz = Some(band.home_hz);
            }
        }
    }
    Ok(plan)
}

pub fn load_file(path: &Path) -> Result<BandPlan, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    parse(&text)
}

pub fn save_file(path: &Path, plan: &BandPlan) -> Result<(), String> {
    validate(plan)?;
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|err| err.to_string())?;
        }
    }
    let text = serde_json::to_string_pretty(plan).map_err(|err| err.to_string())?;
    fs::write(path, text + "\n").map_err(|err| err.to_string())
}

/// Directory of band plans installed next to the binary, else the source tree.
pub fn bsp_dir() -> Option<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join("bsp"));
            // The test executable lives in `target/<profile>/deps`.
            dirs.push(dir.join("../bsp"));
        }
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bsp"));
    dirs.push(PathBuf::from("bsp"));
    dirs.into_iter().find_map(|dir| {
        if dir.join("default.json").is_file() {
            Some(dir.canonicalize().unwrap_or(dir))
        } else {
            None
        }
    })
}

pub fn default_path() -> PathBuf {
    bsp_dir()
        .map(|dir| dir.join("default.json"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("bsp/default.json"))
}

pub fn embedded() -> BandPlan {
    parse(DEFAULT_JSON).expect("embedded band plan is valid")
}

/// One colored piece of a channel bandspread, taken from a band plan.
#[derive(Clone, Debug, PartialEq)]
pub struct DialMark {
    pub lo_hz: f64,
    pub hi_hz: f64,
    pub rank: u8,
    pub rgb: [u8; 3],
    pub legend: String,
}

/// Spans for the channel bandspread. The loaded plan wins when it covers the
/// window. Otherwise the embedded Ham plan is used. A window outside both is
/// an empty list.
pub fn marks_for_window(loaded: &BandPlan, lo: f64, hi: f64) -> Vec<DialMark> {
    let marks = loaded.marks_in(lo, hi);
    if marks.is_empty() {
        embedded().marks_in(lo, hi)
    } else {
        marks
    }
}

/// Parts of `lo..=hi` the radio can actually tune. Disjoint receive
/// segments stay separate so a slot never offers a frequency in a gap.
/// An empty range list is not unlimited.
pub fn pieces_inside(lo: f64, hi: f64, ranges: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if !lo.is_finite() || !hi.is_finite() || hi <= lo {
        return Vec::new();
    }
    let mut pieces: Vec<(f64, f64)> = ranges
        .iter()
        .filter_map(|(min, max)| {
            if !min.is_finite() || !max.is_finite() || max <= min {
                return None;
            }
            let start = lo.max(*min);
            let end = hi.min(*max);
            (end > start).then_some((start, end))
        })
        .collect();
    pieces.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in pieces {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

/// One band the radio can tune, from any zone. `default.json` keeps the meter
/// bands in one zone. `arrl.json` puts each meter band in its own zone and
/// the mode slices in `bands`. Band+ walks this list, not the open zone.
#[derive(Clone, Debug, PartialEq)]
pub struct BandRef {
    pub zone: usize,
    pub band: usize,
    pub name: String,
    pub home_hz: f64,
    pub pieces: Vec<(f64, f64)>,
}

/// Bands that intersect `rx`, lowest frequency first.
pub fn tunable_bands(plan: &BandPlan, rx: &[(f64, f64)]) -> Vec<BandRef> {
    let mut out = Vec::new();
    for (zone_idx, zone) in plan.zones.iter().enumerate() {
        for (band_idx, band) in zone.bands.iter().enumerate() {
            let pieces = pieces_inside(band.lo_hz, band.hi_hz, rx);
            if pieces.is_empty() {
                continue;
            }
            out.push(BandRef {
                zone: zone_idx,
                band: band_idx,
                name: band.name.clone(),
                home_hz: band.home_hz,
                pieces,
            });
        }
    }
    out.sort_by(|a, b| {
        a.pieces[0]
            .0
            .total_cmp(&b.pieces[0].0)
            .then(a.zone.cmp(&b.zone))
            .then(a.band.cmp(&b.band))
    });
    out
}

fn covers(band: &BandRef, hz: f64) -> bool {
    band.pieces.iter().any(|(lo, hi)| hz >= *lo && hz <= *hi)
}

/// Low edge of the piece that contains `hz`.
fn cover_lo(band: &BandRef, hz: f64) -> Option<f64> {
    band.pieces
        .iter()
        .find(|(lo, hi)| hz >= *lo && hz <= *hi)
        .map(|(lo, _)| *lo)
}

/// Band that contains `hz`. A shared edge belongs to the band that starts
/// there, the same rule as [`segment_at`].
pub fn band_at(bands: &[BandRef], hz: f64) -> Option<usize> {
    if !hz.is_finite() {
        return None;
    }
    bands
        .iter()
        .enumerate()
        .filter(|(_, band)| covers(band, hz))
        .max_by(|(_, a), (_, b)| {
            cover_lo(a, hz)
                .unwrap_or(0.0)
                .total_cmp(&cover_lo(b, hz).unwrap_or(0.0))
                .then(a.zone.cmp(&b.zone))
                .then(a.band.cmp(&b.band))
        })
        .map(|(idx, _)| idx)
}

/// Index of the band `dir` steps from the one containing `hz`.
/// `dir` is `1` or `-1`. When `hz` sits in a gap, `1` is the next band above
/// and `-1` is the next band below. `None` at either end.
pub fn adjacent_band(bands: &[BandRef], hz: f64, dir: i32) -> Option<usize> {
    if bands.is_empty() || dir == 0 {
        return None;
    }
    if let Some(idx) = band_at(bands, hz) {
        let next = idx as isize + dir as isize;
        return (next >= 0 && (next as usize) < bands.len()).then_some(next as usize);
    }
    if dir > 0 {
        bands.iter().position(|band| band.pieces[0].0 > hz)
    } else {
        bands
            .iter()
            .rposition(|band| band.pieces.last().is_some_and(|(_, hi)| *hi < hz))
    }
}

/// Remembered frequency when it still falls in the band, otherwise the start
/// of the first receive piece.
pub fn band_tune_hz(band: &BandRef, remembered: Option<f64>) -> f64 {
    let want = remembered.unwrap_or(band.home_hz);
    if covers(band, want) {
        want
    } else {
        band.pieces[0].0
    }
}

/// Segment title at `hz` from the same plan [`marks_for_window`] would paint.
pub fn label_for_window(loaded: &BandPlan, lo: f64, hi: f64, hz: f64) -> Option<String> {
    if loaded.marks_in(lo, hi).is_empty() {
        embedded().label_at(hz)
    } else {
        loaded.label_at(hz)
    }
}

/// Next tab title. `New`, then `New 2`, `New 3`, skipping names already used.
pub fn fresh_zone_name(zones: &[Zone]) -> String {
    let taken = |candidate: &str| zones.iter().any(|zone| zone.name == candidate);
    if !taken("New") {
        return "New".into();
    }
    for n in 2..10_000 {
        let candidate = format!("New {n}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    // 2..=9999 are all taken. This last title is still distinct from those.
    "New 10000".into()
}

/// Append one zone and return its index. Home is `home_hz` when that value is
/// finite and positive, otherwise 14.200 MHz. The single band covers 50 kHz
/// either side of home and validates.
pub fn push_zone(plan: &mut BandPlan, color: [u8; 3], home_hz: f64, mode: &str) -> usize {
    let name = fresh_zone_name(&plan.zones);
    let mut home = if home_hz.is_finite() && home_hz > 0.0 {
        home_hz
    } else {
        14_200_000.0
    };
    let mut lo = (home - 50_000.0).max(0.0);
    let mut hi = home + 50_000.0;
    if !lo.is_finite() || !hi.is_finite() || hi <= lo {
        home = 14_200_000.0;
        lo = home - 50_000.0;
        hi = home + 50_000.0;
    }
    let mode_owned = if mode_family(mode).is_some() {
        mode.trim().to_string()
    } else {
        "USB".into()
    };
    let use_kind = match mode_family(&mode_owned).unwrap_or(Use::Voice) {
        Use::Cw => Use::Cw,
        Use::Digital => Use::Digital,
        Use::Voice | Use::Mixed => Use::Voice,
    };
    plan.zones.push(Zone {
        name: name.clone(),
        color,
        layout: RowLayout::Bands,
        bands: vec![BandLimit {
            name,
            chip: "new".into(),
            lo_hz: lo,
            hi_hz: hi,
            home_hz: home,
            mode: mode_owned,
            segments: vec![Segment {
                lo_hz: lo,
                hi_hz: hi,
                use_kind,
                service: None,
            }],
            tx_hz: Some(home),
        }],
    });
    plan.zones.len() - 1
}

/// Store a trimmed name. An empty name leaves the zone unchanged.
pub fn rename_zone(zone: &mut Zone, name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return false;
    }
    zone.name = trimmed.to_string();
    true
}

/// Remove one zone. Deleting the selected tab selects the previous tab, or
/// the first tab when the removed one was first. An empty plan selects 0.
pub fn delete_zone(plan: &mut BandPlan, idx: usize, selected: usize) -> usize {
    if idx >= plan.zones.len() {
        return selected.min(plan.zones.len().saturating_sub(1));
    }
    plan.zones.remove(idx);
    if plan.zones.is_empty() {
        return 0;
    }
    let next = if idx < selected {
        selected - 1
    } else if idx == selected {
        if idx == 0 {
            0
        } else {
            idx - 1
        }
    } else {
        selected
    };
    next.min(plan.zones.len() - 1)
}

pub fn validate(plan: &BandPlan) -> Result<(), String> {
    if plan.version != 1 {
        return Err(format!("band plan version {} is not 1", plan.version));
    }
    if plan.zones.is_empty() {
        return Err("band plan has no zones".into());
    }
    for zone in &plan.zones {
        if zone.name.trim().is_empty() {
            return Err("a zone has no name".into());
        }
        for band in &zone.bands {
            check_band(&zone.name, band)?;
        }
    }
    Ok(())
}

fn check_band(zone: &str, band: &BandLimit) -> Result<(), String> {
    if band.name.trim().is_empty() {
        return Err(format!("{zone} has a band with no name"));
    }
    if !finite_span(band.lo_hz, band.hi_hz) {
        return Err(format!("{zone} {} has a bad limit", band.name));
    }
    if !band.home_hz.is_finite() || band.home_hz < band.lo_hz || band.home_hz > band.hi_hz {
        return Err(format!("{zone} {} home is outside the band", band.name));
    }
    let family = mode_family(&band.mode).ok_or_else(|| {
        format!(
            "{zone} {} mode {} is not cw, voice, or digital",
            band.name, band.mode
        )
    })?;
    let mut edges: Vec<&Segment> = band.segments.iter().collect();
    edges.sort_by(|a, b| {
        a.lo_hz
            .partial_cmp(&b.lo_hz)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut prev_hi = band.lo_hz;
    for seg in &edges {
        if !finite_span(seg.lo_hz, seg.hi_hz) {
            return Err(format!("{zone} {} has a bad segment", band.name));
        }
        if seg.lo_hz < band.lo_hz || seg.hi_hz > band.hi_hz {
            return Err(format!(
                "{zone} {} segment sits outside the band",
                band.name
            ));
        }
        if seg.lo_hz < prev_hi {
            return Err(format!("{zone} {} segments overlap", band.name));
        }
        prev_hi = seg.hi_hz;
    }
    if let Some(seg) = segment_at(&band.segments, band.home_hz) {
        if !family_allowed(seg.use_kind, family) {
            return Err(format!(
                "{zone} {} home is in a {} segment",
                band.name,
                seg.use_kind.label()
            ));
        }
    }
    Ok(())
}

fn finite_span(lo: f64, hi: f64) -> bool {
    lo.is_finite() && hi.is_finite() && lo >= 0.0 && hi > lo
}

pub fn mode_family(mode: &str) -> Option<Use> {
    match mode.trim().to_ascii_uppercase().as_str() {
        "CW" | "CWR" => Some(Use::Cw),
        "USB" | "LSB" | "AM" | "FM" | "WFM" => Some(Use::Voice),
        "RTTY" | "RTTYR" | "PKT" | "PKTFM" | "DATA" | "DIG" | "DIGU" | "DIGL" => Some(Use::Digital),
        _ => None,
    }
}

/// Mode for a bandspread click at `hz`. The band's stored mode is kept when
/// it is the same kind of mode as the segment under the click (LSB on a voice
/// segment, RTTY on a digital segment). Otherwise the segment picks CW, RTTY,
/// or USB. CW is a legal mode on every segment, so it must not stick when the
/// click is on voice or digital.
pub fn mode_at(band: &BandLimit, hz: f64) -> String {
    let stored = band.mode.trim();
    let Some(seg) = segment_at(&band.segments, hz) else {
        return if stored.is_empty() {
            "USB".into()
        } else {
            stored.to_string()
        };
    };
    let same_kind = match seg.use_kind {
        Use::Mixed => mode_family(stored).is_some(),
        Use::Cw => mode_family(stored) == Some(Use::Cw),
        Use::Digital => mode_family(stored) == Some(Use::Digital),
        Use::Voice => mode_family(stored) == Some(Use::Voice),
    };
    if same_kind && !stored.is_empty() {
        return stored.to_string();
    }
    match seg.use_kind {
        Use::Cw => "CW".into(),
        Use::Digital => "RTTY".into(),
        Use::Voice | Use::Mixed => "USB".into(),
    }
}

/// Pick a mode the radio lists. An empty list means the rig did not say, so
/// the plan mode is kept. A missing name falls back to another mode of the
/// same family.
pub fn choose_mode(wanted: &str, available: &[String]) -> Option<String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    if available.is_empty() {
        return Some(wanted.to_string());
    }
    if let Some(found) = available
        .iter()
        .find(|mode| mode.eq_ignore_ascii_case(wanted))
    {
        return Some(found.clone());
    }
    let family = mode_family(wanted)?;
    available
        .iter()
        .find(|mode| mode_family(mode) == Some(family))
        .cloned()
}

pub fn family_allowed(segment: Use, family: Use) -> bool {
    match family {
        Use::Cw => true,
        Use::Voice => matches!(segment, Use::Voice | Use::Mixed),
        Use::Digital => matches!(segment, Use::Digital | Use::Mixed),
        Use::Mixed => true,
    }
}

/// `20 m` plus Voice is `20 m Voice`. A name that already carries that use
/// (`20 m Voice`) stays as it is.
fn label_with_use(name: &str, title: &str) -> String {
    let already = name
        .split_whitespace()
        .any(|word| word.eq_ignore_ascii_case(title));
    if already {
        name.to_string()
    } else {
        format!("{name} {title}")
    }
}

pub fn segment_at(segments: &[Segment], hz: f64) -> Option<&Segment> {
    segments
        .iter()
        .filter(|seg| hz >= seg.lo_hz && hz <= seg.hi_hz)
        .max_by(|a, b| {
            a.lo_hz
                .partial_cmp(&b.lo_hz)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

impl BandPlan {
    /// Why PTT must stay off when any band in the zone covers `hz`.
    /// The face uses [`Self::transmit_block_on`] for the clicked band.
    #[cfg(test)]
    pub fn transmit_block(&self, zone: usize, hz: f64, mode: &str) -> Option<String> {
        let family = mode_family(mode)?;
        let zone = self.zones.get(zone)?;
        let band = band_covering(zone, hz)?;
        let seg = segment_at(&band.segments, hz)?;
        if family_allowed(seg.use_kind, family) {
            return None;
        }
        Some(format!(
            "{} is {} on {}. {} is {}.",
            mhz(hz),
            seg.use_kind.label().to_ascii_lowercase(),
            band.name,
            mode.trim().to_ascii_uppercase(),
            family.label().to_ascii_lowercase()
        ))
    }

    /// Zone-wide label. The face uses [`Self::reference_label`] so a shared
    /// frequency stays on the clicked band.
    #[cfg(test)]
    pub fn segment_label(&self, zone: usize, hz: f64) -> Option<String> {
        let zone = self.zones.get(zone)?;
        let band = band_covering(zone, hz)?;
        let seg = segment_at(&band.segments, hz)?;
        Some(format!("{} {}", band.name, segment_title(seg)))
    }

    /// Label for the band the operator clicked. A shared frequency does not
    /// switch to another band. Inside the band but outside every segment, the
    /// band name stands alone.
    pub fn reference_label(&self, zone: usize, band: usize, hz: f64) -> Option<String> {
        let band = self.zones.get(zone)?.bands.get(band)?;
        if !hz.is_finite() || hz < band.lo_hz || hz > band.hi_hz {
            return None;
        }
        match segment_at(&band.segments, hz) {
            Some(seg) => Some(label_with_use(&band.name, &segment_title(seg))),
            None => Some(band.name.clone()),
        }
    }

    /// Transmit check for the clicked band only. A frequency outside that
    /// band is not judged by a different band that happens to cover it.
    pub fn transmit_block_on(
        &self,
        zone: usize,
        band: usize,
        hz: f64,
        mode: &str,
    ) -> Option<String> {
        let family = mode_family(mode)?;
        let band = self.zones.get(zone)?.bands.get(band)?;
        if hz < band.lo_hz || hz > band.hi_hz {
            return None;
        }
        let seg = segment_at(&band.segments, hz)?;
        if family_allowed(seg.use_kind, family) {
            return None;
        }
        Some(format!(
            "{} is {} on {}. {} is {}.",
            mhz(hz),
            seg.use_kind.label().to_ascii_lowercase(),
            band.name,
            mode.trim().to_ascii_uppercase(),
            family.label().to_ascii_lowercase()
        ))
    }

    /// Segments that overlap `lo..=hi`, from every zone. Higher service rank
    /// is later so it paints on top.
    pub fn marks_in(&self, lo: f64, hi: f64) -> Vec<DialMark> {
        if !lo.is_finite() || !hi.is_finite() || hi < lo {
            return Vec::new();
        }
        let mut marks = Vec::new();
        for zone in &self.zones {
            for band in &zone.bands {
                for seg in &band.segments {
                    if seg.hi_hz >= lo && seg.lo_hz <= hi && seg.hi_hz > seg.lo_hz {
                        let rank = seg.service.as_deref().map(service_rank).unwrap_or(0);
                        marks.push(DialMark {
                            lo_hz: seg.lo_hz,
                            hi_hz: seg.hi_hz,
                            rank,
                            rgb: segment_rgb(seg),
                            legend: segment_title(seg),
                        });
                    }
                }
            }
        }
        marks.sort_by(|a, b| {
            a.rank.cmp(&b.rank).then_with(|| {
                a.lo_hz
                    .partial_cmp(&b.lo_hz)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        marks
    }

    /// Band and segment title at `hz`, searching every zone. The highest
    /// service rank wins, then the segment that starts later.
    pub fn label_at(&self, hz: f64) -> Option<String> {
        if !hz.is_finite() {
            return None;
        }
        let (band, seg) = self
            .zones
            .iter()
            .filter_map(|zone| {
                let band = band_covering(zone, hz)?;
                let seg = segment_at(&band.segments, hz)?;
                Some((band, seg))
            })
            .max_by(|(_, a), (_, b)| {
                let rank = |seg: &Segment| seg.service.as_deref().map(service_rank).unwrap_or(0);
                rank(a).cmp(&rank(b)).then_with(|| {
                    a.lo_hz
                        .partial_cmp(&b.lo_hz)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            })?;
        Some(format!("{} {}", band.name, segment_title(seg)))
    }

    /// Index of the band to tune when several allocations share a frequency.
    pub fn band_index_at(&self, zone: usize, hz: f64) -> Option<usize> {
        let zone = self.zones.get(zone)?;
        zone.bands
            .iter()
            .enumerate()
            .filter(|(_, band)| hz >= band.lo_hz && hz <= band.hi_hz)
            .max_by(|(_, a), (_, b)| {
                cover_rank(a, hz).cmp(&cover_rank(b, hz)).then_with(|| {
                    a.lo_hz
                        .partial_cmp(&b.lo_hz)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            })
            .map(|(idx, _)| idx)
    }
}

fn band_covering(zone: &Zone, hz: f64) -> Option<&BandLimit> {
    zone.bands
        .iter()
        .filter(|band| hz >= band.lo_hz && hz <= band.hi_hz)
        .max_by(|a, b| {
            cover_rank(a, hz).cmp(&cover_rank(b, hz)).then_with(|| {
                a.lo_hz
                    .partial_cmp(&b.lo_hz)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        })
}

fn cover_rank(band: &BandLimit, hz: f64) -> u8 {
    segment_at(&band.segments, hz)
        .and_then(|seg| seg.service.as_deref())
        .map(service_rank)
        .unwrap_or(0)
}

fn mhz(hz: f64) -> String {
    format!("{:.3} MHz", hz / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_plan_puts_20m_home_on_voice() {
        let plan = embedded();
        let ham = plan.zones.first().unwrap();
        assert_eq!(ham.name, "Ham");
        let band = ham.bands.iter().find(|band| band.chip == "20m").unwrap();
        assert!((band.home_hz - 14_153_608.0).abs() < 0.5);
        assert_eq!(band.mode, "USB");
        let seg = segment_at(&band.segments, band.home_hz).unwrap();
        assert_eq!(seg.use_kind, Use::Voice);
        let text = serde_json::to_string(&plan).unwrap();
        let again = parse(&text).unwrap();
        assert_eq!(again.zones[0].bands.len(), plan.zones[0].bands.len());
        assert!(
            (again.zones[0]
                .bands
                .iter()
                .find(|b| b.chip == "20m")
                .unwrap()
                .home_hz
                - 14_153_608.0)
                .abs()
                < 0.5
        );
    }

    #[test]
    fn overlap_and_forbidden_home_are_refused() {
        let overlap = r#"{
            "version": 1,
            "zones": [{
                "name": "Ham",
                "color": [1, 2, 3],
                "bands": [{
                    "name": "20 m",
                    "chip": "20m",
                    "lo_hz": 14000000,
                    "hi_hz": 14350000,
                    "home_hz": 14153608,
                    "mode": "USB",
                    "segments": [
                        {"lo_hz": 14000000, "hi_hz": 14160000, "use": "cw"},
                        {"lo_hz": 14150000, "hi_hz": 14350000, "use": "voice"}
                    ]
                }]
            }]
        }"#;
        assert!(parse(overlap).is_err());

        let voice_on_digital = r#"{
            "version": 1,
            "zones": [{
                "name": "Ham",
                "color": [1, 2, 3],
                "bands": [{
                    "name": "20 m",
                    "chip": "20m",
                    "lo_hz": 14000000,
                    "hi_hz": 14350000,
                    "home_hz": 14080000,
                    "mode": "USB",
                    "segments": [
                        {"lo_hz": 14000000, "hi_hz": 14150000, "use": "digital"},
                        {"lo_hz": 14150000, "hi_hz": 14350000, "use": "voice"}
                    ]
                }]
            }]
        }"#;
        assert!(parse(voice_on_digital).is_err());
    }

    #[test]
    fn voice_ptt_stops_on_digital_and_cw_does_not() {
        let plan = embedded();
        assert!(plan.transmit_block(0, 14_080_000.0, "USB").is_some());
        assert!(plan.transmit_block(0, 14_080_000.0, "CW").is_none());
        assert!(plan.transmit_block(0, 14_153_608.0, "USB").is_none());
        assert!(plan.transmit_block(0, 14_080_000.0, "RTTY").is_none());
        assert!(plan.transmit_block(0, 14_200_000.0, "RTTY").is_some());
    }

    #[test]
    fn another_zone_keeps_its_homes() {
        let mut plan = embedded();
        plan.zones.push(Zone {
            name: "Aero".into(),
            color: [214, 168, 72],
            layout: RowLayout::Bands,
            bands: vec![BandLimit {
                name: "Aero".into(),
                chip: "aero".into(),
                lo_hz: 2_850_000.0,
                hi_hz: 3_155_000.0,
                home_hz: 3_000_000.0,
                mode: "USB".into(),
                segments: vec![Segment {
                    lo_hz: 2_850_000.0,
                    hi_hz: 3_155_000.0,
                    use_kind: Use::Voice,
                    service: None,
                }],
                tx_hz: None,
            }],
        });
        let ham_home = plan.zones[0].bands[5].home_hz;
        plan.zones[1].bands[0].home_hz = 2_900_000.0;
        assert!((plan.zones[0].bands[5].home_hz - ham_home).abs() < 0.5);
        assert_eq!(plan.zones[0].bands.len(), 13);
        assert_eq!(plan.zones[1].bands.len(), 1);
        assert_ne!(plan.zones[0].bands[0].chip, plan.zones[1].bands[0].chip);
    }

    #[test]
    fn shortwave_example_names_the_service_under_the_dial() {
        let plan = parse(include_str!("../../../bsp/shortwave.json")).unwrap();
        let zone = |name: &str| {
            plan.zones
                .iter()
                .position(|zone| zone.name == name)
                .unwrap_or_else(|| panic!("missing {name}"))
        };
        let label = |name, hz| plan.segment_label(zone(name), hz).unwrap();
        assert_eq!(label("Broadcast", 1_000_000.0), "MW Broadcast");
        assert_eq!(label("Ham", 14_153_608.0), "20 m Ham");
        assert_eq!(label("Ham", 7_200_000.0), "40 m Ham");
        assert_eq!(label("Broadcast", 7_400_000.0), "41 m Broadcast");
        assert_eq!(label("Marine", 2_182_000.0), "2 MHz distress Marine");
        assert_eq!(label("Aero", 3_000_000.0), "Aero 3 MHz Aero");
        assert_eq!(label("Military", 3_080_000.0), "Military 3 MHz Military");
        assert_eq!(label("CB", 27_185_000.0), "11 m CB");
        assert_eq!(label("Time", 5_000_000.0), "Time 5 MHz Time");
        assert_eq!(label("Government", 5_100_000.0), "Government Government");
        assert!(plan
            .segment_label(zone("Government"), 4_500_000.0)
            .is_none());
        let government = zone("Government");
        let sixty = plan.zones[zone("Ham")]
            .bands
            .iter()
            .position(|band| band.name == "60 m")
            .unwrap();
        let shared = 5_350_000.0;
        assert_eq!(
            plan.reference_label(zone("Ham"), sixty, shared).as_deref(),
            Some("60 m Ham")
        );
        assert_eq!(
            plan.reference_label(government, 0, shared).as_deref(),
            Some("Government")
        );
        assert!(plan
            .transmit_block_on(government, 0, shared, "USB")
            .is_none());

        let aero = &plan.zones[zone("Aero")];
        assert_eq!(aero.layout, RowLayout::Spans);
        assert!(aero.bands.len() > 1, "{}", aero.bands.len());
        let government = &plan.zones[zone("Government")];
        assert_eq!(government.bands.len(), 1);
        assert!(plan.zones.iter().all(|zone| zone.name != "Utility"));
        let ham = &plan.zones[zone("Ham")];
        assert_eq!(ham.layout, RowLayout::Bands);
        assert!(ham.bands.len() > 1);
    }

    #[test]
    fn bandspread_click_uses_the_plan_mode() {
        let plan = embedded();
        let band = |chip: &str| {
            plan.zones[0]
                .bands
                .iter()
                .find(|band| band.chip == chip)
                .unwrap()
        };
        let twenty = band("20m");
        assert_eq!(mode_at(twenty, 14_153_608.0), "USB");
        assert_eq!(mode_at(twenty, 14_050_000.0), "CW");
        assert_eq!(mode_at(twenty, 14_100_000.0), "RTTY");
        assert_eq!(mode_at(band("40m"), 7_150_000.0), "LSB");
        assert_eq!(mode_at(band("30m"), 10_110_000.0), "CW");
        assert_eq!(mode_at(band("30m"), 10_136_000.0), "RTTY");
        assert_eq!(mode_at(band("2m"), 146_520_000.0), "FM");
        assert_eq!(mode_at(band("2m"), 144_050_000.0), "CW");
        assert_eq!(
            choose_mode("RTTY", &["USB".into(), "DATA".into()]).as_deref(),
            Some("DATA")
        );
        assert_eq!(
            choose_mode("USB", &["LSB".into(), "USB".into()]).as_deref(),
            Some("USB")
        );
        assert!(choose_mode("RTTY", &["USB".into(), "LSB".into()]).is_none());
        assert_eq!(choose_mode("CW", &[]).as_deref(), Some("CW"));
    }

    #[test]
    fn zone_tabs_add_rename_and_delete() {
        let mut plan = embedded();
        let before = plan.zones.len();
        let idx = push_zone(&mut plan, [92, 184, 120], 14_153_608.0, "USB");
        assert_eq!(idx, before);
        assert_eq!(plan.zones[idx].name, "New");
        assert_eq!(plan.zones[idx].bands[0].chip, "new");
        assert_eq!(plan.zones[idx].bands[0].mode, "USB");
        assert!((plan.zones[idx].bands[0].home_hz - 14_153_608.0).abs() < 0.5);
        validate(&plan).unwrap();
        let again = push_zone(&mut plan, [1, 2, 3], 0.0, "nope");
        assert_eq!(plan.zones[again].name, "New 2");
        assert_eq!(plan.zones[again].bands[0].mode, "USB");
        assert!((plan.zones[again].bands[0].home_hz - 14_200_000.0).abs() < 0.5);
        let cw = push_zone(&mut plan, [1, 2, 3], 7_050_000.0, "CW");
        assert_eq!(plan.zones[cw].bands[0].segments[0].use_kind, Use::Cw);
        assert!(rename_zone(&mut plan.zones[idx], "  40 m  "));
        assert_eq!(plan.zones[idx].name, "40 m");
        assert_eq!(plan.zones[idx].bands[0].name, "New");
        assert!(!rename_zone(&mut plan.zones[idx], "   "));
        assert_eq!(plan.zones[idx].name, "40 m");

        let selected = delete_zone(&mut plan, cw, cw);
        assert_eq!(selected, cw - 1);
        assert!(plan.zones.iter().all(|zone| zone.name != "New 3"));
        let first = delete_zone(&mut plan, 0, 0);
        assert_eq!(first, 0);
        assert!(!plan.zones.is_empty());
        plan.zones.clear();
        assert_eq!(delete_zone(&mut plan, 0, 3), 0);
        assert!(plan.zones.is_empty());
        let added = push_zone(&mut plan, [92, 184, 120], f64::NAN, "FM");
        assert_eq!(added, 0);
        assert_eq!(plan.zones[0].name, "New");
        assert_eq!(plan.zones[0].bands[0].mode, "FM");
        assert_eq!(plan.zones[0].bands[0].segments[0].use_kind, Use::Voice);
        validate(&plan).unwrap();
    }

    #[test]
    fn arrl_example_is_one_zone_per_named_band() {
        let plan = parse(include_str!("../../../bsp/arrl.json")).unwrap();
        let zones: Vec<&str> = plan.zones.iter().map(|zone| zone.name.as_str()).collect();
        assert_eq!(
            zones,
            [
                "2200 m", "630 m", "160 m", "80 m", "60 m", "40 m", "30 m", "20 m", "17 m", "15 m",
                "12 m", "10 m", "6 m", "2 m", "1.25 m", "70 cm", "33 cm", "23 cm", "13 cm",
                "13 cm hi", "9 cm", "5 cm", "3 cm",
            ]
        );
        let zone = |name: &str| plan.zones.iter().find(|zone| zone.name == name).unwrap();
        let twenty = zone("20 m");
        assert_eq!(
            twenty
                .bands
                .iter()
                .map(|band| band.chip.as_str())
                .collect::<Vec<_>>(),
            ["CW", "Digital", "Voice"]
        );
        let voice = twenty
            .bands
            .iter()
            .find(|band| band.chip == "Voice")
            .unwrap();
        assert!((voice.home_hz - 14_153_608.0).abs() < 0.5);
        assert_eq!(voice.mode, "USB");
        assert_eq!(voice.segments.len(), 1);
        assert_eq!(voice.segments[0].use_kind, Use::Voice);
        let twenty_idx = plan
            .zones
            .iter()
            .position(|zone| zone.name == "20 m")
            .unwrap();
        let voice_idx = twenty
            .bands
            .iter()
            .position(|band| band.chip == "Voice")
            .unwrap();
        assert_eq!(
            plan.reference_label(twenty_idx, voice_idx, 14_153_608.0)
                .as_deref(),
            Some("20 m Voice")
        );
        for zone in &plan.zones {
            assert!(
                zone.bands.iter().all(|band| band.segments.len() == 1),
                "{}",
                zone.name
            );
        }
        let covers = |hz: f64| {
            plan.zones.iter().any(|zone| {
                zone.bands
                    .iter()
                    .any(|band| hz >= band.lo_hz && hz <= band.hi_hz)
            })
        };
        // 2310–2390 MHz sits outside the amateur allocation.
        assert!(!covers(2_350_000_000.0));
        // Amateur use of 3.45–3.5 GHz ended 2022-04-14.
        assert!(!covers(3_470_000_000.0));
        assert!(!covers(24_000_000_000.0));
        assert!((zone("13 cm").bands.last().unwrap().hi_hz - 2_310_000_000.0).abs() < 0.5);
        assert!((zone("13 cm hi").bands[0].lo_hz - 2_390_000_000.0).abs() < 0.5);
        let nine = zone("9 cm");
        assert!((nine.bands[0].lo_hz - 3_300_000_000.0).abs() < 0.5);
        assert!((nine.bands.last().unwrap().hi_hz - 3_450_000_000.0).abs() < 0.5);
        let calling = zone("1.25 m")
            .bands
            .iter()
            .find(|band| (band.home_hz - 222_100_000.0).abs() < 0.5)
            .unwrap();
        assert_eq!(calling.mode, "USB");
        assert_eq!(calling.segments[0].use_kind, Use::Voice);
        let five = zone("5 cm");
        assert!(five.bands.iter().any(|band| {
            (band.home_hz - 5_760_100_000.0).abs() < 0.5 && band.segments[0].use_kind == Use::Mixed
        }));
        let three = zone("3 cm");
        assert!((three.bands.last().unwrap().hi_hz - 10_500_000_000.0).abs() < 0.5);
        assert!(three.bands.iter().any(|band| {
            (band.home_hz - 10_368_100_000.0).abs() < 0.5 && band.segments[0].use_kind == Use::Mixed
        }));
    }

    #[test]
    fn plans_are_installed_beside_the_binary() {
        let dir = bsp_dir().expect("bsp directory");
        assert!(dir.join("default.json").is_file(), "{}", dir.display());
        assert!(dir.join("arrl.json").is_file(), "{}", dir.display());
        assert!(dir.join("shortwave.json").is_file(), "{}", dir.display());
        let plan = load_file(&dir.join("shortwave.json")).unwrap();
        let government = plan
            .zones
            .iter()
            .find(|zone| zone.name == "Government")
            .unwrap();
        assert_eq!(government.bands.len(), 1);
        let aero = plan.zones.iter().find(|zone| zone.name == "Aero").unwrap();
        assert_eq!(aero.layout, RowLayout::Spans);
    }

    #[test]
    fn matching_window_uses_the_loaded_plan() {
        let mut plan = embedded();
        let band = plan.zones[0]
            .bands
            .iter_mut()
            .find(|band| band.name == "160 m")
            .unwrap();
        band.segments = vec![Segment {
            lo_hz: 1_800_000.0,
            hi_hz: 2_000_000.0,
            use_kind: Use::Digital,
            service: None,
        }];
        let marks = marks_for_window(&plan, 1_850_000.0, 1_950_000.0);
        assert!(!marks.is_empty());
        assert!(marks.iter().all(|mark| mark.rgb == Use::Digital.rgb()));
        assert_eq!(
            label_for_window(&plan, 1_850_000.0, 1_950_000.0, 1_900_000.0).as_deref(),
            Some("160 m Digital")
        );
    }

    #[test]
    fn unmatched_window_uses_the_embedded_default() {
        let mut plan = embedded();
        plan.zones[0].bands.retain(|band| band.name == "160 m");
        let marks = marks_for_window(&plan, 14_000_000.0, 14_350_000.0);
        assert!(marks.iter().any(|mark| {
            mark.rgb == Use::Voice.rgb() && mark.lo_hz <= 14_153_608.0 && mark.hi_hz >= 14_153_608.0
        }));
        assert_eq!(
            label_for_window(&plan, 14_000_000.0, 14_350_000.0, 14_153_608.0).as_deref(),
            Some("20 m Voice")
        );
    }

    #[test]
    fn window_outside_both_plans_is_unmarked() {
        let plan = embedded();
        assert!(marks_for_window(&plan, 1_000.0, 2_000.0).is_empty());
        assert!(label_for_window(&plan, 1_000.0, 2_000.0, 1_500.0).is_none());
    }

    #[test]
    fn bands_outside_the_radio_are_hidden() {
        let dummy = [(150_000.0, 1_500_000_000.0)];
        let hf = [(1_800_000.0, 30_000_000.0)];
        let uhf = (420_000_000.0, 450_000_000.0);
        assert!(pieces_inside(uhf.0, uhf.1, &dummy).len() == 1);
        assert!(pieces_inside(uhf.0, uhf.1, &hf).is_empty());
        assert!(pieces_inside(uhf.0, uhf.1, &[]).is_empty());
        let government = pieces_inside(5_060_000.0, 5_450_000.0, &[(5_250_000.0, 5_450_000.0)]);
        assert_eq!(government, vec![(5_250_000.0, 5_450_000.0)]);
        let full = pieces_inside(5_060_000.0, 5_450_000.0, &dummy);
        assert_eq!(full, vec![(5_060_000.0, 5_450_000.0)]);
    }

    #[test]
    fn band_buttons_follow_meter_bands_in_the_default_plan() {
        let plan = parse(include_str!("../../../bsp/default.json")).unwrap();
        let rx = [(150_000.0, 1_500_000_000.0)];
        let bands = tunable_bands(&plan, &rx);
        let up = adjacent_band(&bands, 14_153_608.0, 1).unwrap();
        let down = adjacent_band(&bands, 14_153_608.0, -1).unwrap();
        assert_eq!(bands[up].name, "17 m");
        assert_eq!(bands[down].name, "30 m");
        assert!((band_tune_hz(&bands[up], None) - 18_110_000.0).abs() < 0.5);
    }

    #[test]
    fn band_buttons_cross_arrl_zones() {
        let plan = parse(include_str!("../../../bsp/arrl.json")).unwrap();
        let rx = [(150_000.0, 1_500_000_000.0)];
        let bands = tunable_bands(&plan, &rx);
        // 14.153608 sits in 20 m Voice. The open zone after a load is 2200 m,
        // which does not contain that frequency, so a zone-local step stops.
        let here = band_at(&bands, 14_153_608.0).unwrap();
        assert_eq!(bands[here].name, "20 m Voice");
        let up = adjacent_band(&bands, 14_153_608.0, 1).unwrap();
        let down = adjacent_band(&bands, 14_153_608.0, -1).unwrap();
        assert_eq!(bands[up].name, "17 m CW");
        assert_eq!(
            bands[up].zone,
            plan.zones.iter().position(|z| z.name == "17 m").unwrap()
        );
        assert_eq!(bands[down].name, "20 m Digital");
        // Dummy receive starts at 150 kHz, so 2200 m (135.7–137.8 kHz) is not
        // a step. The next band the radio can hear is 630 m.
        let below = adjacent_band(&bands, 100_000.0, 1).unwrap();
        assert_eq!(bands[below].name, "630 m CW");
        assert!(adjacent_band(&bands, 100_000.0, -1).is_none());
        let wide = tunable_bands(&plan, &[(100_000.0, 1_500_000_000.0)]);
        let lf = adjacent_band(&wide, 100_000.0, 1).unwrap();
        assert_eq!(wide[lf].name, "2200 m CW");
    }

    #[test]
    fn band_plus_steps_off_the_17m_voice_edge() {
        let plan = parse(include_str!("../../../bsp/arrl.json")).unwrap();
        let rx = [(150_000.0, 1_500_000_000.0)];
        let bands = tunable_bands(&plan, &rx);
        // 18.110 MHz is both the top of 17 m Digital and the 17 m Voice home.
        let voice = band_at(&bands, 18_110_000.0).unwrap();
        assert_eq!(bands[voice].name, "17 m Voice");
        let next = adjacent_band(&bands, 18_110_000.0, 1).unwrap();
        assert_eq!(bands[next].name, "15 m CW");
        let mut hz = 100_000.0;
        let mut seen = Vec::new();
        for _ in 0..bands.len() {
            let Some(idx) = adjacent_band(&bands, hz, 1) else {
                break;
            };
            let tuned = band_tune_hz(&bands[idx], None);
            assert!(
                tuned > hz,
                "{} retunes {} at {}",
                bands[idx].name,
                tuned,
                hz
            );
            seen.push(bands[idx].name.clone());
            hz = tuned;
        }
        assert_eq!(seen.len(), bands.len());
        assert!(seen
            .windows(2)
            .any(|pair| pair == ["17 m Voice", "15 m CW"]));
    }
}
