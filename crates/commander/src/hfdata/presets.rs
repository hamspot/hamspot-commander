//! Dial presets per HF data mode, loaded from `share/hfdata/presets.json`
//! (built in). Each preset is checked against the band plan at run time and
//! the result is shown as text beside it.

use serde::Deserialize;

use crate::plan::{self, BandPlan, Use};

pub const PRESETS_JSON: &str = include_str!("../../../../share/hfdata/presets.json");

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetFile {
    pub version: u32,
    #[serde(default)]
    pub note: String,
    pub presets: Vec<Preset>,
}

fn usb() -> String {
    "USB".into()
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub mode: String,
    pub band: String,
    pub dial_hz: f64,
    #[serde(default = "usb")]
    pub sideband: String,
    /// True only once the frequency is checked against a published list.
    #[serde(default)]
    pub checked: bool,
}

impl Preset {
    pub fn mhz_label(&self) -> String {
        let mhz = self.dial_hz / 1e6;
        let s = format!("{mhz:.4}");
        let s = s.trim_end_matches('0');
        let s = if s.ends_with('.') { format!("{s}0") } else { s.to_string() };
        format!("{} {s}", self.band)
    }
}

pub fn parse(text: &str) -> Result<PresetFile, String> {
    let file: PresetFile = serde_json::from_str(text).map_err(|e| format!("preset file: {e}"))?;
    if file.version != 1 {
        return Err(format!("preset file version {} is not supported (want 1)", file.version));
    }
    let mut seen = std::collections::HashSet::new();
    for p in &file.presets {
        if p.mode.trim().is_empty() || p.band.trim().is_empty() {
            return Err("preset with an empty mode or band".into());
        }
        if !p.dial_hz.is_finite() || p.dial_hz < 100_000.0 || p.dial_hz > 60_000_000.0 {
            return Err(format!("{} {}: dial {} Hz is outside HF/6 m", p.mode, p.band, p.dial_hz));
        }
        if !matches!(p.sideband.as_str(), "USB" | "LSB") {
            return Err(format!("{} {}: sideband must be USB or LSB", p.mode, p.band));
        }
        if !seen.insert((p.mode.clone(), p.dial_hz as i64)) {
            return Err(format!("{} {}: duplicate preset", p.mode, p.band));
        }
    }
    Ok(file)
}

pub fn embedded() -> Result<PresetFile, String> {
    parse(PRESETS_JSON)
}

pub fn for_mode<'a>(file: &'a PresetFile, mode: &str) -> Vec<&'a Preset> {
    file.presets.iter().filter(|p| p.mode.eq_ignore_ascii_case(mode)).collect()
}

/// Where a dial frequency falls in a band plan.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanFit {
    Digital { band: String },
    OtherSegment { band: String, segment: &'static str },
    InBand { band: String },
    Outside,
}

pub fn plan_fit(bp: &BandPlan, hz: f64) -> PlanFit {
    let mut best = PlanFit::Outside;
    for zone in &bp.zones {
        for band in &zone.bands {
            if hz < band.lo_hz || hz > band.hi_hz {
                continue;
            }
            match plan::segment_at(&band.segments, hz) {
                Some(seg) if seg.use_kind == Use::Digital => {
                    return PlanFit::Digital { band: band.name.clone() };
                }
                Some(seg) => {
                    if !matches!(best, PlanFit::OtherSegment { .. }) {
                        best = PlanFit::OtherSegment {
                            band: band.name.clone(),
                            segment: seg.use_kind.label(),
                        };
                    }
                }
                None => {
                    if best == PlanFit::Outside {
                        best = PlanFit::InBand { band: band.name.clone() };
                    }
                }
            }
        }
    }
    best
}

pub fn fit_text(fit: &PlanFit) -> String {
    match fit {
        PlanFit::Digital { band } => format!("in plan: {band}, digital segment"),
        PlanFit::OtherSegment { band, segment } => {
            format!("in plan: {band}, {segment} segment (not marked digital)")
        }
        PlanFit::InBand { band } => format!("in plan: {band}, no segment at this frequency"),
        PlanFit::Outside => "NOT in the loaded band plan".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrl() -> BandPlan {
        plan::parse(include_str!("../../../../bsp/arrl.json")).expect("arrl plan")
    }

    #[test]
    fn embedded_presets_load_and_are_marked_unchecked() {
        let f = embedded().expect("built-in presets parse");
        assert!(!f.presets.is_empty());
        for m in ["FT8", "FT4", "WSPR", "JS8"] {
            assert!(!for_mode(&f, m).is_empty(), "{m} has presets");
        }
        // Nothing has been checked against a published list yet.
        assert!(f.presets.iter().all(|p| !p.checked));
        assert!(!PRESETS_JSON.contains("/home") && !PRESETS_JSON.contains(":\\\\"));
    }

    #[test]
    fn every_hf_preset_is_inside_an_arrl_band() {
        let bp = arrl();
        let f = embedded().unwrap();
        for p in f.presets.iter().filter(|p| p.dial_hz < 30e6) {
            assert_ne!(plan_fit(&bp, p.dial_hz), PlanFit::Outside, "{} {}", p.mode, p.band);
        }
    }

    #[test]
    fn plan_fit_reports_segments() {
        let bp = arrl();
        assert!(matches!(plan_fit(&bp, 7_100_000.0), PlanFit::Digital { .. }));
        assert_eq!(plan_fit(&bp, 5_000_000.0), PlanFit::Outside);
        assert!(fit_text(&PlanFit::Outside).contains("NOT"));
    }

    #[test]
    fn bad_files_are_rejected() {
        assert!(parse(r#"{"version":2,"presets":[]}"#).is_err());
        assert!(parse(r#"{"version":1,"presets":[{"mode":"FT8","band":"20 m","dial_hz":-1}]}"#).is_err());
        assert!(parse(r#"{"version":1,"presets":[{"mode":"FT8","band":"20 m","dial_hz":14074000,"sideband":"AM"}]}"#).is_err());
        assert!(parse(r#"{"version":1,"presets":[{"mode":"FT8","band":"20 m","dial_hz":14074000,"path":"/x"}]}"#).is_err());
        let dup = r#"{"version":1,"presets":[{"mode":"FT8","band":"20 m","dial_hz":14074000},{"mode":"FT8","band":"20 m","dial_hz":14074000}]}"#;
        assert!(parse(dup).is_err());
    }

    #[test]
    fn labels() {
        let p = Preset { mode: "FT4".into(), band: "40 m".into(), dial_hz: 7_047_500.0, sideband: usb(), checked: false };
        assert_eq!(p.mhz_label(), "40 m 7.0475");
        let p = Preset { dial_hz: 14_074_000.0, band: "20 m".into(), ..p };
        assert_eq!(p.mhz_label(), "20 m 14.074");
    }
}
