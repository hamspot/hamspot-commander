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
    /// Transcribed from a listing. The follower does not read this flag.
    #[serde(default)]
    pub encrypted: bool,
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

/// Add one control memory to `zone`. A frequency already in the zone is left
/// as it is. The name matches the listing form `859.9625c`. The mode is the
/// mode of a control memory already in the zone, or `FM`.
pub fn add_control(zone: &mut ChannelZone, hz: f64, rfss: u32, site: u32) -> bool {
    if !hz.is_finite() || hz <= 0.0 {
        return false;
    }
    if zone
        .channels
        .iter()
        .any(|channel| (channel.hz - hz).abs() < 1.0)
    {
        return false;
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
    });
    true
}

/// Control memories in one zone. The follower uses this list and no other
/// zone. `hz`, then mode, then name.
pub fn zone_controls(file: &ChannelFile, zone: usize) -> Vec<(f64, String, String)> {
    let Some(zone) = file.zones.get(zone) else {
        return Vec::new();
    };
    zone.channels
        .iter()
        .filter(|channel| channel.control && channel.hz.is_finite() && channel.hz > 0.0)
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
    }

    #[test]
    fn control_list_is_the_selected_zone_only() {
        let file = ChannelFile {
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
                        },
                        Channel {
                            name: "cc".into(),
                            hz: 858_512_500.0,
                            mode: "FM".into(),
                            control: true,
                            encrypted: false,
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
    }

    #[test]
    fn missing_file_names_the_path() {
        let path = Path::new("/no/such/channels.json");
        let err = load_file(path).unwrap_err();
        assert!(err.contains("/no/such/channels.json"), "{err}");
    }
}
