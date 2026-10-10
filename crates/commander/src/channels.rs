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
    fn round_trip() {
        let file = ChannelFile {
            version: 1,
            zones: vec![ChannelZone {
                name: "Memories".into(),
                channels: vec![Channel {
                    name: "20 m".into(),
                    hz: 14_153_608.0,
                    mode: "USB".into(),
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
    fn missing_file_names_the_path() {
        let path = Path::new("/no/such/channels.json");
        let err = load_file(path).unwrap_err();
        assert!(err.contains("/no/such/channels.json"), "{err}");
    }
}
