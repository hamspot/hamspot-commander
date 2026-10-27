//! Per-recording metadata. Written to the DB and to a JSON sidecar beside
//! the raw file, so the DB can always be rebuilt from disk.
use serde::{Deserialize, Serialize};

pub const SOURCE_HACKRF_TG: &str = "hackrf_tg";
pub const SOURCE_HACKRF_CHANNELS: &str = "hackrf_channels";
pub const KIND_RAW_IQ: &str = "raw_iq";
pub const KIND_DEMOD_AUDIO: &str = "demod_audio";

/// Display name for a source key (REC page tabs).
pub fn source_label(source: &str) -> String {
    match source {
        SOURCE_HACKRF_TG => "HackRF TG".into(),
        SOURCE_HACKRF_CHANNELS => "HackRF Channels".into(),
        other => other
            .split('_')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Built-in tabs, always shown even when empty, then any other source in the DB.
pub fn source_tabs(in_db: &[String]) -> Vec<String> {
    let mut out = vec![SOURCE_HACKRF_TG.to_string(), SOURCE_HACKRF_CHANNELS.to_string()];
    for s in in_db {
        if !out.contains(s) {
            out.push(s.clone());
        }
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub source: String,
    /// `raw_iq` (HackRF channelised IQ) or `demod_audio` (rig's own audio).
    pub source_kind: String,
    #[serde(default)]
    pub radio_id: u64,
    #[serde(default)]
    pub radio_name: String,
    #[serde(default)]
    pub system: String,
    #[serde(default)]
    pub site: String,
    #[serde(default)]
    pub talkgroup: Option<u32>,
    #[serde(default)]
    pub talkgroup_name: String,
    #[serde(default)]
    pub channel_name: String,
    #[serde(default)]
    pub radio_unit_id: Option<u32>,
    /// `cs16` (raw IQ) or `s16le` (demod audio, mono).
    pub raw_format: String,
    pub sample_rate: u32,
    #[serde(default)]
    pub centre_hz: f64,
    #[serde(default)]
    pub channel_hz: f64,
    #[serde(default)]
    pub offset_hz: f64,
    #[serde(default)]
    pub bandwidth_hz: f64,
    #[serde(default)]
    pub chain: String,
    #[serde(default)]
    pub gains: serde_json::Value,
    /// p25, dmr, fm, ... (the channel's vocoder; empty or `fm` is analog).
    #[serde(default)]
    pub modulation: String,
    #[serde(default)]
    pub decoder: serde_json::Value,
    #[serde(default)]
    pub sw_version: String,
    pub start_utc: String,
    #[serde(default)]
    pub end_utc: String,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub bytes: i64,
    #[serde(default)]
    pub emergency: bool,
    #[serde(default)]
    pub priority: bool,
    /// Samples dropped because the writer fell behind. Never silent.
    #[serde(default)]
    pub dropped: u64,
    /// Segment number when a long call was split (0 = first).
    #[serde(default)]
    pub part: u32,
}

impl Meta {
    pub fn is_digital(&self) -> bool {
        !matches!(self.modulation.as_str(), "" | "fm" | "nfm" | "am" | "analog")
    }

    /// Lane key on the timeline: TG number or channel name.
    pub fn lane(&self) -> String {
        match self.talkgroup {
            Some(tg) if self.talkgroup_name.is_empty() => format!("TG {tg}"),
            Some(tg) => format!("{} ({tg})", self.talkgroup_name),
            None if !self.channel_name.is_empty() => self.channel_name.clone(),
            None => format!("{:.4} MHz", self.channel_hz / 1e6),
        }
    }
}

pub fn sw_version() -> String {
    format!(
        "{} {}",
        env!("CARGO_PKG_VERSION"),
        option_env!("HSREC_GIT_SHA").unwrap_or("")
    )
    .trim()
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_and_tabs() {
        assert_eq!(source_label("hackrf_tg"), "HackRF TG");
        assert_eq!(source_label("hackrf_channels"), "HackRF Channels");
        assert_eq!(source_label("rig_audio"), "Rig Audio");
        let tabs = source_tabs(&["rig_audio".into(), "hackrf_tg".into()]);
        assert_eq!(tabs, vec!["hackrf_tg", "hackrf_channels", "rig_audio"]);
        assert_eq!(source_tabs(&[]).len(), 2);
    }
}
