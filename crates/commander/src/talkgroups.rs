//! Talkgroup directory for the control-channel follower.
//!
//! A zone here is a named group of talkgroups. It is not a band-plan zone
//! and not a channel-file zone. The file does not decide which grant is
//! tuned. The follower still takes the newest clear voice grant.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Talkgroup {
    pub name: String,
    pub tg: u32,
    /// Transcribed from a listing. With no filled key, follow does not play it.
    #[serde(default)]
    pub encrypted: bool,
    /// A priority talkgroup may take a receiver off a held call when the
    /// station allows emergency/priority overrides. Absent means not priority.
    #[serde(default, skip_serializing_if = "is_false")]
    pub priority: bool,
    /// Marked for recording (right-click). Absent means not marked.
    #[serde(default, skip_serializing_if = "is_false")]
    pub record: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Talkgroups marked priority, in every zone.
pub fn priority_tgs(file: &TalkgroupFile) -> Vec<u32> {
    let mut out: Vec<u32> = file
        .zones
        .iter()
        .flat_map(|zone| zone.talkgroups.iter())
        .filter(|row| row.priority)
        .map(|row| row.tg)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TalkgroupZone {
    pub name: String,
    pub talkgroups: Vec<Talkgroup>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TalkgroupFile {
    pub version: u32,
    pub zones: Vec<TalkgroupZone>,
}

impl TalkgroupFile {
    /// `serde(default = ...)` on the station file. An old station has no key.
    pub fn empty_default() -> Self {
        empty()
    }
}

pub fn empty() -> TalkgroupFile {
    TalkgroupFile {
        version: 1,
        zones: vec![TalkgroupZone {
            name: "Talkgroups".into(),
            talkgroups: Vec::new(),
        }],
    }
}

pub fn parse(text: &str) -> Result<TalkgroupFile, String> {
    let file: TalkgroupFile = serde_json::from_str(text).map_err(|err| err.to_string())?;
    validate(&file)?;
    Ok(file)
}

pub fn load_file(path: &Path) -> Result<TalkgroupFile, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    parse(&text)
}

pub fn save_file(path: &Path, file: &TalkgroupFile) -> Result<(), String> {
    validate(file)?;
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|err| err.to_string())?;
        }
    }
    let text = serde_json::to_string_pretty(file).map_err(|err| err.to_string())?;
    fs::write(path, text + "\n").map_err(|err| format!("{}: {err}", path.display()))
}

pub fn validate(file: &TalkgroupFile) -> Result<(), String> {
    if file.version != 1 {
        return Err(format!("talkgroup file version {} is not 1", file.version));
    }
    if file.zones.is_empty() {
        return Err("talkgroup file has no zones".into());
    }
    for zone in &file.zones {
        if zone.name.trim().is_empty() {
            return Err("a zone has no name".into());
        }
    }
    Ok(())
}

/// Zone that holds talkgroups heard on the air and missing from the station.
pub const DISCOVERED: &str = "Discovered";

/// Record `tg` when no zone already has it. The new row goes in [`DISCOVERED`],
/// which is created at the end of the file so the open zone stays put.
/// `true` means a row was inserted.
pub fn note_seen(file: &mut TalkgroupFile, tg: u32, encrypted: bool) -> bool {
    if tg == 0 || file.zones.iter().any(|zone| zone.talkgroups.iter().any(|row| row.tg == tg))
    {
        return false;
    }
    if !file.zones.iter().any(|zone| zone.name == DISCOVERED) {
        file.zones.push(TalkgroupZone {
            name: DISCOVERED.into(),
            talkgroups: Vec::new(),
        });
    }
    let Some(zone) = file.zones.iter_mut().find(|zone| zone.name == DISCOVERED) else {
        return false;
    };
    zone.talkgroups.push(Talkgroup {
        record: false,
        name: tg.to_string(),
        tg,
        encrypted,
        priority: false,
    });
    true
}

/// True when any zone row for this talkgroup is marked encrypted.
/// A talkgroup that appears in more than one zone is encrypted when any
/// of those rows says so. The caller decides whether a filled key plays it.
pub fn is_encrypted(file: &TalkgroupFile, tg: u32) -> bool {
    file.zones.iter().any(|zone| {
        zone.talkgroups
            .iter()
            .any(|row| row.tg == tg && row.encrypted)
    })
}

/// Name stored for this talkgroup, if the directory has one.
pub fn name_of(file: &TalkgroupFile, tg: u32) -> Option<String> {
    for zone in &file.zones {
        for row in &zone.talkgroups {
            if row.tg == tg && !row.name.trim().is_empty() {
                return Some(row.name.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_version_is_refused() {
        let text = r#"{"version":2,"zones":[{"name":"Talkgroups","talkgroups":[]}]}"#;
        let err = parse(text).unwrap_err();
        assert!(err.contains("version"), "{err}");
    }

    #[test]
    fn round_trip() {
        let file = TalkgroupFile {
            version: 1,
            zones: vec![TalkgroupZone {
                name: "Police".into(),
                talkgroups: vec![Talkgroup { record: false,
                    name: "Dispatch".into(),
                    tg: 100,
                    encrypted: false,
                    priority: false,
                }],
            }],
        };
        let again = parse(&serde_json::to_string_pretty(&file).unwrap()).unwrap();
        assert_eq!(again, file);
    }

    #[test]
    fn an_unknown_talkgroup_lands_in_discovered() {
        let mut file = empty();
        assert!(note_seen(&mut file, 4400, false));
        assert_eq!(file.zones.len(), 2);
        assert_eq!(file.zones[0].name, "Talkgroups");
        assert!(file.zones[0].talkgroups.is_empty());
        assert_eq!(file.zones[1].name, DISCOVERED);
        assert_eq!(file.zones[1].talkgroups.len(), 1);
        assert_eq!(file.zones[1].talkgroups[0].tg, 4400);
        assert_eq!(file.zones[1].talkgroups[0].name, "4400");
        assert!(!file.zones[1].talkgroups[0].encrypted);
        assert!(!note_seen(&mut file, 4400, true));
        assert_eq!(file.zones[1].talkgroups.len(), 1);
        assert!(!file.zones[1].talkgroups[0].encrypted);
        file.zones[0].talkgroups.push(Talkgroup { record: false,
            name: "Dispatch".into(),
            tg: 100,
            encrypted: false,
            priority: false,
        });
        assert!(!note_seen(&mut file, 100, true));
        assert!(!is_encrypted(&file, 100));
        assert!(!is_encrypted(&file, 4400));
        file.zones[1].talkgroups[0].encrypted = true;
        assert!(is_encrypted(&file, 4400));
        assert!(note_seen(&mut file, 77, true));
        assert_eq!(file.zones[1].talkgroups.len(), 2);
        assert!(file.zones[1].talkgroups[1].encrypted);
        assert!(!note_seen(&mut file, 0, false));
    }
}
