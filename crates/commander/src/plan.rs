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

/// Segment title at `hz` from the same plan [`marks_for_window`] would paint.
pub fn label_for_window(loaded: &BandPlan, lo: f64, hi: f64, hz: f64) -> Option<String> {
    if loaded.marks_in(lo, hi).is_empty() {
        embedded().label_at(hz)
    } else {
        loaded.label_at(hz)
    }
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

pub fn family_allowed(segment: Use, family: Use) -> bool {
    match family {
        Use::Cw => true,
        Use::Voice => matches!(segment, Use::Voice | Use::Mixed),
        Use::Digital => matches!(segment, Use::Digital | Use::Mixed),
        Use::Mixed => true,
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
    /// Why PTT must stay off. `None` means the mode is allowed, or the
    /// frequency is not inside a segment we know.
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

    pub fn segment_label(&self, zone: usize, hz: f64) -> Option<String> {
        let zone = self.zones.get(zone)?;
        let band = band_covering(zone, hz)?;
        let seg = segment_at(&band.segments, hz)?;
        Some(format!("{} {}", band.name, segment_title(seg)))
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
    fn plans_are_installed_beside_the_binary() {
        let dir = bsp_dir().expect("bsp directory");
        assert!(dir.join("default.json").is_file(), "{}", dir.display());
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
}
