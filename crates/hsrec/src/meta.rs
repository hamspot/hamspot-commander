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
    /// Everything the live decoder got from its environment, saved so replay
    /// re-demods the same way. Keys are env names (`HAMSPOT_[A-Z0-9_]+`);
    /// replay passes them to the decoder script. Any decoder may add keys.
    /// Absent in recordings made before this field existed.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub decoder_params: std::collections::BTreeMap<String, String>,
    /// Every radio heard talking on this call, first-heard first. The first
    /// one is also `radio_unit_id`. Absent in older recordings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<UnitSeen>,
}

/// One radio unit heard on a call, with when it was first and last seen
/// (UTC ISO strings, as the control channel reported it).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitSeen {
    pub id: u32,
    #[serde(default)]
    pub first_utc: String,
    #[serde(default)]
    pub last_utc: String,
}

/// Add or refresh `id` in `units` (first-heard order kept). `true` if the
/// list changed.
pub fn note_unit(units: &mut Vec<UnitSeen>, id: u32, at_utc: &str) -> bool {
    if id == 0 {
        return false;
    }
    if let Some(u) = units.iter_mut().find(|u| u.id == id) {
        if u.last_utc.as_str() < at_utc {
            u.last_utc = at_utc.to_string();
            return true;
        }
        return false;
    }
    units.push(UnitSeen { id, first_utc: at_utc.to_string(), last_utc: at_utc.to_string() });
    true
}

/// Merge `more` into `units`: new radios appended, times widened.
pub fn merge_units(units: &mut Vec<UnitSeen>, more: &[UnitSeen]) {
    for m in more {
        match units.iter_mut().find(|u| u.id == m.id) {
            Some(u) => {
                if !m.first_utc.is_empty() && (u.first_utc.is_empty() || m.first_utc < u.first_utc) {
                    u.first_utc = m.first_utc.clone();
                }
                if m.last_utc > u.last_utc {
                    u.last_utc = m.last_utc.clone();
                }
            }
            None => units.push(m.clone()),
        }
    }
}

impl Meta {
    /// Every radio ID on this call: `units`, or the lone `radio_unit_id`
    /// of an older recording.
    pub fn unit_ids(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.units.iter().map(|u| u.id).collect();
        if let Some(p) = self.radio_unit_id {
            if !v.contains(&p) {
                v.insert(0, p);
            }
        }
        v
    }
    /// Set `units` and keep `radio_unit_id` = the first-heard radio.
    pub fn set_units(&mut self, units: Vec<UnitSeen>) {
        self.units = units;
        if let Some(first) = self.units.first() {
            self.radio_unit_id = Some(first.id);
        }
    }
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

    #[test]
    fn units_note_merge_and_round_trip() {
        let mut u = Vec::new();
        assert!(note_unit(&mut u, 5, "2026-01-01T00:00:01Z"));
        assert!(!note_unit(&mut u, 5, "2026-01-01T00:00:01Z"));
        assert!(note_unit(&mut u, 5, "2026-01-01T00:00:09Z"));
        assert!(note_unit(&mut u, 9, "2026-01-01T00:00:02Z"));
        assert!(!note_unit(&mut u, 0, "2026-01-01T00:00:02Z"));
        assert_eq!(u[0], UnitSeen { id: 5, first_utc: "2026-01-01T00:00:01Z".into(), last_utc: "2026-01-01T00:00:09Z".into() });
        let mut a = vec![UnitSeen { id: 9, first_utc: "2026-01-01T00:00:05Z".into(), last_utc: "2026-01-01T00:00:05Z".into() }];
        merge_units(&mut a, &u);
        assert_eq!(a.iter().map(|x| x.id).collect::<Vec<_>>(), vec![9, 5]);
        assert_eq!(a[0].first_utc, "2026-01-01T00:00:02Z");
        let mut m = Meta::default();
        m.set_units(a);
        assert_eq!(m.radio_unit_id, Some(9));
        let back: Meta = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
        // Old JSON without `units` still reads, and no empty list is written.
        let old: Meta = serde_json::from_str(r#"{"source":"hackrf_tg","source_kind":"raw_iq","radio_unit_id":3,"raw_format":"cs16","sample_rate":1,"start_utc":""}"#).unwrap();
        assert_eq!(old.unit_ids(), vec![3]);
        assert!(!serde_json::to_string(&old).unwrap().contains("\"units\""));
    }

    #[test]
    fn decoder_params_round_trip_and_old_json_defaults() {
        let mut m = Meta { source: "hackrf_tg".into(), source_kind: KIND_RAW_IQ.into(), raw_format: "cs16".into(), sample_rate: 1, start_utc: "x".into(), ..Default::default() };
        let text = serde_json::to_string(&m).unwrap();
        assert!(!text.contains("decoder_params"));
        let old: Meta = serde_json::from_str(&text).unwrap();
        assert!(old.decoder_params.is_empty());
        m.decoder_params.insert("HAMSPOT_P25_X".into(), "BEE006BD6B5".into());
        m.decoder_params.insert("HAMSPOT_GAIN_LNA".into(), "32".into());
        let back: Meta = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back.decoder_params, m.decoder_params);
        assert_eq!(back.decoder_params.keys().next().unwrap(), "HAMSPOT_GAIN_LNA");
    }
}
