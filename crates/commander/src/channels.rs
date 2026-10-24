//! Memory channels. A zone here is a named group of memories. It is not a
//! band-plan zone, and this file is not a band plan.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    pub name: String,
    pub hz: f64,
    #[serde(default)]
    pub mode: String,
    /// Park a control-channel follower here. The flag is not specific to one
    /// air interface. Old files load with it clear.
    #[serde(default)]
    pub control: bool,
    /// Marked encrypted in the listing. Scan skips it, and a click does not
    /// tune it. Follow plays a grant on this frequency only when a key file
    /// has a filled key.
    #[serde(default)]
    pub encrypted: bool,
    /// Receive demodulator for this memory. Empty is analog. A stored kind is
    /// inserted while the memory is selected. It does not tune.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vocoder: String,
    /// Ask the decoder for inverted polarity. Only DMR, NXDN, and dPMR have
    /// a switch. Old files load with it clear, and a clear flag is omitted.
    #[serde(default, skip_serializing_if = "is_false")]
    pub invert: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// DMR (`-xr`), NXDN48/NXDN96 (`-xn`), and dPMR (`-xd`). Other kinds have
/// no polarity switch in this decoder.
pub fn vocoder_can_invert(kind: &str) -> bool {
    matches!(kind, "dmr" | "nxdn48" | "nxdn96" | "dpmr")
}

/// Closed dropdown text, in menu order. The stored value is the first column.
pub const VOCODER_CHOICES: &[(&str, &str)] = &[
    ("", "None"),
    ("p25", "P25"),
    ("p25p2", "P25 Phase 2"),
    ("dmr", "DMR"),
    ("dstar", "D-STAR"),
    ("ysf", "Fusion"),
    ("nxdn48", "NXDN48"),
    ("nxdn96", "NXDN96"),
    ("dpmr", "dPMR"),
    ("m17", "M17"),
    ("provoice", "ProVoice"),
    ("edacs", "EDACS"),
    ("edacs-esk", "EDACS ESK"),
    ("edacs-344", "EDACS 344"),
    ("edacs-434", "EDACS 434"),
    ("edacs-ea", "EDACS EA"),
    ("edacs-ea-esk", "EDACS EA ESK"),
    ("x2tdma", "X2-TDMA"),
    ("freedv1600", "FreeDV 1600"),
    ("freedv700c", "FreeDV 700C"),
    ("freedv700d", "FreeDV 700D"),
    ("freedv700e", "FreeDV 700E"),
    ("freedv800xa", "FreeDV 800XA"),
    ("freedv2400a", "FreeDV 2400A"),
    ("freedv2400b", "FreeDV 2400B"),
    ("drm", "DRM"),
];

/// A [`VOCODER_CHOICES`] value, or empty. Anything else is analog.
pub fn vocoder_kind(value: &str) -> &'static str {
    let value = value.trim();
    VOCODER_CHOICES
        .iter()
        .find(|(stored, _)| *stored == value)
        .map(|(stored, _)| *stored)
        .unwrap_or("")
}

pub fn vocoder_label(value: &str) -> &'static str {
    let kind = vocoder_kind(value);
    VOCODER_CHOICES
        .iter()
        .find(|(stored, _)| *stored == kind)
        .map(|(_, label)| *label)
        .unwrap_or("None")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelZone {
    pub name: String,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelFile {
    pub version: u32,
    pub zones: Vec<ChannelZone>,
}

pub fn empty() -> ChannelFile {
    ChannelFile {
        version: 1,
        zones: vec![ChannelZone {
            name: "Memories".into(),
            channels: Vec::new(),
        }],
    }
}

pub fn parse(text: &str) -> Result<ChannelFile, String> {
    let file: ChannelFile = serde_json::from_str(text).map_err(|err| err.to_string())?;
    validate(&file)?;
    Ok(file)
}

pub fn load_file(path: &Path) -> Result<ChannelFile, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    parse(&text)
}

pub fn save_file(path: &Path, file: &ChannelFile) -> Result<(), String> {
    validate(file)?;
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|err| err.to_string())?;
        }
    }
    let text = serde_json::to_string_pretty(file).map_err(|err| err.to_string())?;
    fs::write(path, text + "\n").map_err(|err| format!("{}: {err}", path.display()))
}

/// Add one control memory to `zone`. A frequency already marked control is
/// left as it is. A memory at the same frequency that was not a control
/// becomes one, so a neighbor announcement joins the control scan. The name
/// matches the listing form `859.9625c`. The mode is the mode of a control
/// memory already in the zone, or `FM`.
pub fn add_control(zone: &mut ChannelZone, hz: f64, rfss: u32, site: u32) -> bool {
    if !hz.is_finite() || hz <= 0.0 {
        return false;
    }
    if let Some(channel) = zone
        .channels
        .iter_mut()
        .find(|channel| (channel.hz - hz).abs() < 1.0)
    {
        if channel.control {
            return false;
        }
        channel.control = true;
        return true;
    }
    let mode = zone
        .channels
        .iter()
        .find(|channel| channel.control && !channel.mode.is_empty())
        .map(|channel| channel.mode.clone())
        .unwrap_or_else(|| "FM".into());
    let mhz = hz / 1_000_000.0;
    let mut name = format!("{mhz:.4}c");
    if zone.channels.iter().any(|channel| channel.name == name) {
        name = format!("{name} {rfss}-{site}");
    }
    zone.channels.push(Channel {
        name,
        hz,
        mode,
        control: true,
        encrypted: false,
        vocoder: String::new(),
            invert: false,
    });
    true
}

/// True when a memory marked encrypted is within 500 Hz of `hz`.
/// That is the same window the scanner uses for "the radio is on this channel".
pub fn hz_encrypted(file: &ChannelFile, hz: f64) -> bool {
    if !hz.is_finite() {
        return false;
    }
    file.zones.iter().any(|zone| {
        zone.channels.iter().any(|channel| {
            channel.encrypted && channel.hz.is_finite() && (channel.hz - hz).abs() < 500.0
        })
    })
}

/// Control memories in one zone. An encrypted control is left out. The
/// follower uses this list and no other zone. `hz`, then mode, then name.
pub fn zone_controls(file: &ChannelFile, zone: usize) -> Vec<(f64, String, String)> {
    let Some(zone) = file.zones.get(zone) else {
        return Vec::new();
    };
    zone.channels.iter().filter(|channel| {
        channel.control
            && !channel.encrypted
            && channel.hz.is_finite()
            && channel.hz > 0.0
    })
        .map(|channel| (channel.hz, channel.mode.clone(), channel.name.clone()))
        .collect()
}

pub fn validate(file: &ChannelFile) -> Result<(), String> {
    if file.version != 1 {
        return Err(format!("channel file version {} is not 1", file.version));
    }
    if file.zones.is_empty() {
        return Err("channel file has no zones".into());
    }
    for zone in &file.zones {
        if zone.name.trim().is_empty() {
            return Err("a zone has no name".into());
        }
        for (idx, channel) in zone.channels.iter().enumerate() {
            if !channel.hz.is_finite() || channel.hz <= 0.0 {
                return Err(format!(
                    "{} channel {} frequency must be greater than zero",
                    zone.name,
                    idx + 1
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_adjacent_site_becomes_a_control_memory() {
        let mut zone = ChannelZone {
            name: "1.027 Newport".into(),
            channels: vec![Channel {
                name: "859.9625c".into(),
                hz: 859_962_500.0,
                mode: "FM".into(),
                control: true,
                encrypted: false,
                vocoder: String::new(),
            invert: false,
            }],
        };
        assert!(add_control(&mut zone, 858_962_500.0, 1, 27));
        assert!(!add_control(&mut zone, 858_962_500.4, 1, 27));
        assert_eq!(zone.channels.len(), 2);
        assert_eq!(zone.channels[1].name, "858.9625c");
        assert!(zone.channels[1].control);
        assert_eq!(zone.channels[1].mode, "FM");
        assert!(!zone.channels[1].encrypted);
    }

    #[test]
    fn an_adjacent_site_marks_an_existing_memory_as_control() {
        let mut zone = ChannelZone {
            name: "1.027 Newport".into(),
            channels: vec![Channel {
                name: "voice".into(),
                hz: 858_962_500.0,
                mode: "FM".into(),
                control: false,
                encrypted: false,
                vocoder: String::new(),
                invert: false,
            }],
        };
        assert!(add_control(&mut zone, 858_962_500.0, 1, 27));
        assert_eq!(zone.channels.len(), 1);
        assert!(zone.channels[0].control);
        assert_eq!(zone.channels[0].name, "voice");
        assert!(!add_control(&mut zone, 858_962_500.0, 1, 27));
    }

    #[test]
    fn round_trip() {
        let file = ChannelFile {
            version: 1,
            zones: vec![ChannelZone {
                name: "Memories".into(),
                channels: vec![Channel {
                    name: "20 m".into(),
                    hz: 14_153_608.0,
                    mode: "USB".into(),
                    control: false,
                    encrypted: false,
                    vocoder: String::new(),
            invert: false,
                }],
            }],
        };
        let again = parse(&serde_json::to_string_pretty(&file).unwrap()).unwrap();
        assert_eq!(again, file);
    }

    #[test]
    fn negative_frequency_is_refused() {
        let text = r#"{"version":1,"zones":[{"name":"Memories","channels":[{"name":"x","hz":-1.0,"mode":"USB"}]}]}"#;
        let err = parse(text).unwrap_err();
        assert!(err.contains("greater than zero"), "{err}");
    }

    #[test]
    fn band_plan_is_not_a_channel_file() {
        let err = parse(crate::plan::DEFAULT_JSON).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn missing_control_flag_loads_clear() {
        let text = r#"{"version":1,"zones":[{"name":"Memories","channels":[{"name":"cc","hz":851012500.0,"mode":"FM"}]}]}"#;
        let file = parse(text).unwrap();
        assert!(!file.zones[0].channels[0].control);
        assert!(!file.zones[0].channels[0].encrypted);
        assert!(file.zones[0].channels[0].vocoder.is_empty());
    }

    #[test]
    fn vocoder_round_trip_omits_an_empty_value() {
        let text = r#"{"version":1,"zones":[{"name":"Memories","channels":[{"name":"dmr","hz":441000000.0,"mode":"FM","vocoder":"dmr"}]}]}"#;
        let file = parse(text).unwrap();
        assert_eq!(file.zones[0].channels[0].vocoder, "dmr");
        assert_eq!(vocoder_kind("dmr"), "dmr");
        assert_eq!(vocoder_label("p25"), "P25");
        assert_eq!(vocoder_kind("ysf"), "ysf");
        assert_eq!(vocoder_label("dstar"), "D-STAR");
        assert_eq!(vocoder_label("nxdn48"), "NXDN48");
        assert_eq!(vocoder_label("m17"), "M17");
        assert_eq!(vocoder_label("p25p2"), "P25 Phase 2");
        assert_eq!(vocoder_kind("edacs-ea-esk"), "edacs-ea-esk");
        assert_eq!(vocoder_label("x2tdma"), "X2-TDMA");
        assert!(!vocoder_can_invert("p25p2"));
        assert!(!vocoder_can_invert("provoice"));
        assert!(!vocoder_can_invert("edacs"));
        assert!(!vocoder_can_invert("x2tdma"));
        assert_eq!(vocoder_label("freedv700d"), "FreeDV 700D");
        assert_eq!(vocoder_kind("freedv2400b"), "freedv2400b");
        assert_eq!(vocoder_label("drm"), "DRM");
        assert!(!vocoder_can_invert("freedv1600"));
        assert!(!vocoder_can_invert("freedv800xa"));
        assert!(!vocoder_can_invert("drm"));
        assert_eq!(vocoder_kind("nxdn"), "");
        assert_eq!(vocoder_kind("typo"), "");
        assert_eq!(vocoder_label(""), "None");
        let mut stored = file.clone();
        stored.zones[0].channels[0].vocoder = "p25".into();
        let again = parse(&serde_json::to_string_pretty(&stored).unwrap()).unwrap();
        assert_eq!(again.zones[0].channels[0].vocoder, "p25");
        let json = serde_json::to_string(&again.zones[0].channels[0]).unwrap();
        assert!(json.contains("\"vocoder\":\"p25\""), "{json}");
        stored.zones[0].channels[0].vocoder.clear();
        let cleared = serde_json::to_string(&stored.zones[0].channels[0]).unwrap();
        assert!(!cleared.contains("vocoder"), "{cleared}");
        assert!(!cleared.contains("invert"), "{cleared}");
        stored.zones[0].channels[0].invert = true;
        let marked = serde_json::to_string(&stored.zones[0].channels[0]).unwrap();
        assert!(marked.contains("\"invert\":true"), "{marked}");
        let loaded = parse(&format!(
            r#"{{"version":1,"zones":[{{"name":"Memories","channels":[{marked}]}}]}}"#
        ))
        .unwrap();
        assert!(loaded.zones[0].channels[0].invert);
        assert!(!vocoder_can_invert("p25"));
        assert!(!vocoder_can_invert("dstar"));
        assert!(!vocoder_can_invert("ysf"));
        assert!(!vocoder_can_invert("m17"));
        assert!(!vocoder_can_invert(""));
        assert!(vocoder_can_invert("dmr"));
        assert!(vocoder_can_invert("nxdn48"));
        assert!(vocoder_can_invert("nxdn96"));
        assert!(vocoder_can_invert("dpmr"));
    }

    #[test]
    fn control_list_is_the_selected_zone_only() {
        let mut file = ChannelFile {
            version: 1,
            zones: vec![
                ChannelZone {
                    name: "Other site".into(),
                    channels: vec![Channel {
                        name: "other".into(),
                        hz: 851_012_500.0,
                        mode: "FM".into(),
                        control: true,
                        encrypted: false,
                        vocoder: String::new(),
            invert: false,
                    }],
                },
                ChannelZone {
                    name: "This site".into(),
                    channels: vec![
                        Channel {
                            name: "voice".into(),
                            hz: 852_000_000.0,
                            mode: "FM".into(),
                            control: false,
                            encrypted: false,
                            vocoder: String::new(),
            invert: false,
                        },
                        Channel {
                            name: "cc".into(),
                            hz: 858_512_500.0,
                            mode: "FM".into(),
                            control: true,
                            encrypted: false,
                            vocoder: String::new(),
            invert: false,
                        },
                    ],
                },
            ],
        };
        let listed = zone_controls(&file, 1);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, 858_512_500.0);
        assert_eq!(listed[0].2, "cc");
        assert!(zone_controls(&file, 4).is_empty());
        file.zones[1].channels.push(Channel {
            name: "locked".into(),
            hz: 859_000_000.0,
            mode: "FM".into(),
            control: true,
            encrypted: true,
            vocoder: String::new(),
            invert: false,
        });
        let listed = zone_controls(&file, 1);
        assert_eq!(listed.len(), 1);
        assert!(!hz_encrypted(&file, 858_512_500.0));
        assert!(hz_encrypted(&file, 859_000_100.0));
        assert!(!hz_encrypted(&file, 859_012_500.0));
    }

    #[test]
    fn missing_file_names_the_path() {
        let path = Path::new("/no/such/channels.json");
        let err = load_file(path).unwrap_err();
        assert!(err.contains("/no/such/channels.json"), "{err}");
    }
}
