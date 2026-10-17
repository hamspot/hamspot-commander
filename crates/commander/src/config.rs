//! One station file. The band plan and the channel file are objects in the
//! document. The Meters tab is live and is not stored.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::audio::AudioGraph;
use crate::channels::ChannelFile;
use crate::plan::{self, BandPlan};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub version: u32,
    pub bsp_name: String,
    pub bsp: BandPlan,
    pub channels: ChannelFile,
    pub tune: Tune,
    pub levels: BTreeMap<String, f64>,
    /// Radios the CAT manager starts. Absent in a file written before that
    /// frame existed, which still loads and starts the hamlib dummy.
    #[serde(default)]
    pub radios: Vec<crate::cat::RadioSpec>,
    /// Hamspot PipeWire devices and the wires other tools have left on them.
    /// Absent in a file written before the Audio frame existed.
    #[serde(default)]
    pub audio: AudioGraph,
    /// Rotators the Automation frame starts. Absent in a file written before
    /// that frame existed, which still loads and starts no rotator.
    #[serde(default)]
    pub rotators: Vec<crate::rot::RotatorSpec>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tune {
    pub show_spread: bool,
    pub step_hz: f64,
    pub zone_sel: usize,
    pub band_sel: usize,
    pub band_mem: BTreeMap<String, f64>,
    pub favorite: Option<FavoriteSlot>,
    pub host: String,
    pub port: String,
    pub freq_hz: f64,
    pub mode: String,
    pub passband_hz: i32,
    pub sub_freq_hz: f64,
    pub sub_mode: String,
    pub rit_hz: i32,
    pub split: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FavoriteSlot {
    pub hz: f64,
    pub mode: String,
    pub passband_hz: i32,
}

/// What happened to `bsp/<name>` while opening a config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BspInstall {
    /// The name was not already a file, so the embedded plan was written.
    Written(PathBuf),
    /// A file with that name was already there. Its bytes were left alone.
    Kept(PathBuf),
    /// The band-plan folder could not be found.
    NoFolder,
}

pub fn parse(text: &str) -> Result<ConfigFile, String> {
    let file: ConfigFile = serde_json::from_str(text).map_err(|err| err.to_string())?;
    validate(&file)?;
    Ok(file)
}

pub fn to_string(file: &ConfigFile) -> Result<String, String> {
    validate(file)?;
    let text = serde_json::to_string_pretty(file).map_err(|err| err.to_string())?;
    Ok(text + "\n")
}

pub fn save_file(path: &Path, file: &ConfigFile) -> Result<(), String> {
    let text = to_string(file)?;
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|err| err.to_string())?;
        }
    }
    fs::write(path, text).map_err(|err| format!("{}: {err}", path.display()))
}

pub fn load_file(path: &Path) -> Result<ConfigFile, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    parse(&text)
}

pub fn validate(file: &ConfigFile) -> Result<(), String> {
    if file.version != 1 {
        return Err(format!("config version {} is not 1", file.version));
    }
    if !bsp_name_ok(&file.bsp_name) {
        return Err(format!(
            "band plan name {} is not a single .json file name",
            file.bsp_name
        ));
    }
    plan::validate(&file.bsp)?;
    crate::channels::validate(&file.channels)?;
    if !file.tune.step_hz.is_finite() || file.tune.step_hz <= 0.0 {
        return Err("digit step must be greater than zero".into());
    }
    if !file.tune.freq_hz.is_finite() || file.tune.freq_hz <= 0.0 {
        return Err("frequency must be greater than zero".into());
    }
    if file.tune.passband_hz < 0 {
        return Err("passband must not be negative".into());
    }
    if file.tune.host.trim().is_empty() {
        return Err("host is empty".into());
    }
    if file.tune.port.trim().is_empty() {
        return Err("port is empty".into());
    }
    if !file.tune.sub_freq_hz.is_finite() || file.tune.sub_freq_hz < 0.0 {
        return Err("split frequency must not be negative".into());
    }
    for (name, hz) in &file.tune.band_mem {
        if name.trim().is_empty() || !hz.is_finite() || *hz <= 0.0 {
            return Err(format!("band memory {name} is not a frequency"));
        }
    }
    if let Some(fav) = &file.tune.favorite {
        if !fav.hz.is_finite() || fav.hz <= 0.0 {
            return Err("favorite frequency must be greater than zero".into());
        }
    }
    for (name, value) in &file.levels {
        if name.trim().is_empty() || !value.is_finite() {
            return Err(format!("level {name} is not a number"));
        }
    }
    for radio in &file.radios {
        crate::cat::validate_spec(radio)?;
    }
    for rotator in &file.rotators {
        crate::rot::validate_spec(rotator)?;
    }
    Ok(())
}

/// A band-plan file name with no directory. `../x.json` is refused.
pub fn bsp_name_ok(name: &str) -> bool {
    if name.len() <= ".json".len() || !name.ends_with(".json") {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return false;
    }
    if name.starts_with('.') {
        return false;
    }
    !name.contains("..")
}

pub fn bsp_name_from_path(path: &str) -> Result<String, String> {
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if bsp_name_ok(&name) {
        Ok(name)
    } else {
        Err(format!(
            "band plan name {name} is not a single .json file name"
        ))
    }
}

/// Write `plan` to `dir/name` when that file is not already there.
pub fn install_new_bsp(
    dir: Option<&Path>,
    name: &str,
    plan: &BandPlan,
) -> Result<BspInstall, String> {
    if !bsp_name_ok(name) {
        return Err(format!(
            "band plan name {name} is not a single .json file name"
        ));
    }
    let Some(dir) = dir else {
        return Ok(BspInstall::NoFolder);
    };
    let path = dir.join(name);
    if path.is_file() {
        return Ok(BspInstall::Kept(path));
    }
    plan::save_file(&path, plan)?;
    Ok(BspInstall::Written(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ConfigFile {
        let mut bsp = plan::embedded();
        bsp.zones[0].bands[0].tx_hz = Some(1_900_000.0);
        ConfigFile {
            version: 1,
            bsp_name: "arrl.json".into(),
            bsp,
            channels: crate::channels::empty(),
            tune: Tune {
                show_spread: true,
                step_hz: 100.0,
                zone_sel: 0,
                band_sel: 0,
                band_mem: BTreeMap::from([("20 m".into(), 14_153_608.0)]),
                favorite: None,
                host: "127.0.0.1".into(),
                port: "4532".into(),
                freq_hz: 14_153_608.0,
                mode: "USB".into(),
                passband_hz: 2400,
                sub_freq_hz: 0.0,
                sub_mode: String::new(),
                rit_hz: 0,
                split: false,
            },
            levels: BTreeMap::from([("AF".into(), 0.25)]),
            radios: Vec::new(),
            audio: AudioGraph::default(),
            rotators: Vec::new(),
        }
    }

    #[test]
    fn config_round_trips_without_session_tx_hz() {
        let text = to_string(&sample()).unwrap();
        assert!(!text.contains("tx_hz"));
        assert!(!text.contains("meters"));
        let back = parse(&text).unwrap();
        assert!(back.bsp.zones[0].bands[0].tx_hz.is_none());
        assert_eq!(back.bsp_name, "arrl.json");
        assert_eq!(back.levels.get("AF").copied(), Some(0.25));
        assert_eq!(back.tune.freq_hz, 14_153_608.0);
    }

    #[test]
    fn bad_version_missing_plan_and_parent_path_are_refused() {
        let text = to_string(&sample()).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["version"] = serde_json::json!(2);
        assert!(parse(&value.to_string()).is_err());

        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value.as_object_mut().unwrap().remove("bsp");
        assert!(parse(&value.to_string()).is_err());

        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["bsp_name"] = serde_json::json!("../x.json");
        assert!(parse(&value.to_string()).is_err());

        value["bsp_name"] = serde_json::json!("arrl.json");
        value["meters"] = serde_json::json!({});
        assert!(parse(&value.to_string()).is_err());

        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value.as_object_mut().unwrap().remove("radios");
        let loaded = parse(&value.to_string()).unwrap();
        assert!(loaded.radios.is_empty());
    }

    #[test]
    fn radios_round_trip_name_model_and_device() {
        let mut file = sample();
        file.radios.push(crate::cat::RadioSpec {
            driver: crate::cat::RadioDriver::Hamlib,
            name: "IC-7300".into(),
            model: 3073,
            device: "/dev/ttyUSB0".into(),
            serial_speed: "19200".into(),
            civ: String::new(),
            ptt_type: String::new(),
            ptt_file: String::new(),
            conf: String::new(),
            park: false,
            follow: false,
            audio: crate::cat::RadioAudio::default(),
            gqrx: crate::cat::GqrxConfig::new(),
            rx_pipe: String::new(),
        });
        let back = parse(&to_string(&file).unwrap()).unwrap();
        assert_eq!(back.radios.len(), 1);
        assert_eq!(back.radios[0].name, "IC-7300");
        assert_eq!(back.radios[0].model, 3073);
        assert_eq!(back.radios[0].device, "/dev/ttyUSB0");
        assert!(!back.radios[0].park);
        assert!(!back.radios[0].follow);
        assert!(back.radios[0].audio.rx);
        assert!(back.radios[0].audio.input.is_empty());
        let mut value: serde_json::Value =
            serde_json::from_str(&to_string(&file).unwrap()).unwrap();
        value["radios"][0].as_object_mut().unwrap().remove("audio");
        value.as_object_mut().unwrap().remove("audio");
        let loaded = parse(&value.to_string()).unwrap();
        assert!(loaded.audio.devices.is_empty());
        assert!(loaded.radios[0].audio.tx);
        file.audio.devices.push(crate::audio::AudioNode {
            name: "hamspot_custom".into(),
            description: "Extra".into(),
            class: "Audio/Sink".into(),
        });
        file.audio.links.push(crate::audio::AudioLink {
            output: "output.hamspot_1_rx:output_FL".into(),
            input: "alsa_output.speakers:playback_FL".into(),
        });
        let back = parse(&to_string(&file).unwrap()).unwrap();
        assert_eq!(back.audio.devices[0].name, "hamspot_custom");
        assert_eq!(back.audio.links[0].output, "output.hamspot_1_rx:output_FL");
        assert!(!to_string(&file).unwrap().contains("rx_pipe"));
        file.radios[0].rx_pipe = "dsdccx -i - -o -".into();
        let text = to_string(&file).unwrap();
        assert!(text.contains("dsdccx -i - -o -"));
        let back = parse(&text).unwrap();
        assert_eq!(back.radios[0].rx_pipe, "dsdccx -i - -o -");
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["radios"][0]
            .as_object_mut()
            .unwrap()
            .remove("rx_pipe");
        let loaded = parse(&value.to_string()).unwrap();
        assert!(loaded.radios[0].rx_pipe.is_empty());
        assert!(!to_string(&file).unwrap().contains("\"gqrx\""));
    }

    #[test]
    fn a_file_without_rotators_loads_and_one_rotator_round_trips() {
        let text = to_string(&sample()).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value.as_object_mut().unwrap().remove("rotators");
        let loaded = parse(&value.to_string()).unwrap();
        assert!(loaded.rotators.is_empty());
        let mut file = sample();
        file.rotators.push(crate::rot::RotatorSpec {
            name: "Tower".into(),
            model: 601,
            device: "/dev/ttyUSB0".into(),
            serial_speed: "9600".into(),
            conf: String::new(),
        });
        let back = parse(&to_string(&file).unwrap()).unwrap();
        assert_eq!(back.rotators.len(), 1);
        assert_eq!(back.rotators[0].name, "Tower");
        assert_eq!(back.rotators[0].model, 601);
        assert_eq!(back.rotators[0].device, "/dev/ttyUSB0");
        file.rotators[0].name.clear();
        assert!(to_string(&file).is_err());
    }

    #[test]
    fn gqrx_settings_round_trip_inside_the_station_file() {
        let mut file = sample();
        let mut spec = crate::cat::RadioSpec::dummy();
        spec.driver = crate::cat::RadioDriver::Gqrx;
        spec.name = "GQRX".into();
        spec.model = 0;
        spec.park = false;
        spec.gqrx = crate::cat::gqrx_chooser_config();
        file.radios.push(spec);
        let text = to_string(&file).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let gqrx = &value["radios"][0]["gqrx"];
        assert!(gqrx.get("General").is_some());
        assert_eq!(gqrx["General"]["crashed"], "false");
        assert!(gqrx
            .get("input")
            .and_then(|group| group.get("device"))
            .is_none());
        let back = parse(&text).unwrap();
        assert!(crate::cat::gqrx_needs_chooser(&back.radios[0].gqrx));
        assert_eq!(
            back.radios[0]
                .gqrx
                .get("remote_control")
                .unwrap()
                .get("enabled")
                .unwrap(),
            "true"
        );
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["radios"][0].as_object_mut().unwrap().remove("gqrx");
        let loaded = parse(&value.to_string()).unwrap();
        assert!(loaded.radios[0].gqrx.is_empty());
    }

    #[test]
    fn a_new_bsp_name_is_written_and_an_existing_file_is_kept() {
        let dir =
            std::env::temp_dir().join(format!("hamspot-config-{}-{}", std::process::id(), "bsp"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = sample();
        let kept = dir.join("arrl.json");
        fs::write(&kept, "already here\n").unwrap();
        match install_new_bsp(Some(&dir), "arrl.json", &file.bsp).unwrap() {
            BspInstall::Kept(path) => assert_eq!(path, kept),
            other => panic!("expected the existing file to stay, got {other:?}"),
        }
        assert_eq!(fs::read_to_string(&kept).unwrap(), "already here\n");

        match install_new_bsp(Some(&dir), "field.json", &file.bsp).unwrap() {
            BspInstall::Written(path) => {
                let written = plan::load_file(&path).unwrap();
                assert_eq!(written.zones[0].name, file.bsp.zones[0].name);
            }
            other => panic!("expected a new file, got {other:?}"),
        }
        let again = fs::read(&dir.join("field.json")).unwrap();
        install_new_bsp(Some(&dir), "field.json", &file.bsp).unwrap();
        assert_eq!(fs::read(&dir.join("field.json")).unwrap(), again);
        assert!(matches!(
            install_new_bsp(None, "field.json", &file.bsp).unwrap(),
            BspInstall::NoFolder
        ));
        let _ = fs::remove_dir_all(&dir);
    }
}
