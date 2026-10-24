//! One station file. The band plan and the channel file are objects in the
//! document. The extension is `.HSCommander`. Live meter needles are not stored.
//! A torn-off meter's screen position is stored when the window system reports it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::audio::AudioGraph;
use crate::channels::ChannelFile;
use crate::plan::{self, BandPlan};
use crate::talkgroups::TalkgroupFile;

/// Station files use this extension, including the capital letters.
pub const STATION_EXTENSION: &str = "HSCommander";
/// The station opened when Commander is started with no file argument.
pub const DEFAULT_STATION_NAME: &str = "Default.HSCommander";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub version: u32,
    pub bsp_name: String,
    pub bsp: BandPlan,
    pub channels: ChannelFile,
    /// Talkgroup directory for the control-channel follower. Absent in a
    /// station written before that page existed.
    #[serde(default = "TalkgroupFile::empty_default")]
    pub talkgroups: TalkgroupFile,
    /// Radios named on a control channel. Absent in a station written before
    /// that page existed. This is not the CAT radio list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<crate::units::Unit>,
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
    /// Index into `radios` of the row the Tune page drives. Absent means the first row.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub selected: usize,
    /// Page that was open. Absent means Tune.
    #[serde(default, skip_serializing_if = "is_tune")]
    pub page: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub channel_zone: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub channel_sel: usize,
    /// Talkgroup zone and row that were open. Absent means the first row.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub talkgroup_zone: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub talkgroup_sel: usize,
    /// Radio-id row that was open. Absent means the first row.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unit_sel: usize,
    /// Indexes into `radios` whose talkgroup Follow box is on.
    /// Absent in a station written before that box was stored.
    /// This is not the Gpredict `follow` flag on a radio.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_follow: Vec<usize>,
    /// Trunking job of a receiver. Absent means every followed radio is
    /// both the control receiver and the voice receiver.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trunk_receivers: Vec<TrunkReceiver>,
    /// Channel zones marked for the purple scan list, in zone-list order.
    /// Absent in a station written before that list was stored.
    /// SCAN and DWELL themselves are not stored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scan_zones: Vec<usize>,
    /// Talkgroup zones marked for HackRF monitoring. Indexes into
    /// `talkgroups.zones`. Absent in an older station file.
    /// SCAN and DWELL are not stored here either.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub monitor_zones: Vec<usize>,
    /// Talkgroup numbers marked for HackRF monitoring. A number can be
    /// stored before that talkgroup is in the file. 0 is not a talkgroup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub monitor_talkgroups: Vec<u32>,
    /// Meter order, lock, and the torn-off window when its screen place is known.
    #[serde(default, skip_serializing_if = "MetersFace::is_default")]
    pub meters: MetersFace,
}

/// What one receiver does while Follow is on. A missing entry means Both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrunkRole {
    /// Stay on the control channel. Do not play voice.
    Control,
    /// Play voice. A far call may leave the control channel.
    Voice,
    /// Decode the control channel and play voice. A far call may leave.
    Both,
}

/// One radio in the trunking receiver list. `radio` is an index into
/// [`ConfigFile::radios`], the same index `control_follow` uses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrunkReceiver {
    pub radio: usize,
    pub role: TrunkRole,
}

/// Where a torn-off window was. Every field is optional because Wayland does
/// not report a window position. A missing field is left unset on open.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowPlace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixels_per_point: Option<f32>,
}

impl WindowPlace {
    pub fn is_empty(&self) -> bool {
        self.x.is_none()
            && self.y.is_none()
            && self.width.is_none()
            && self.height.is_none()
            && self.monitor_width.is_none()
            && self.monitor_height.is_none()
            && self.pixels_per_point.is_none()
    }
}

/// Meter grid and its torn-off window. Needle positions are readings, not settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetersFace {
    #[serde(default = "default_locked")]
    pub locked: bool,
    #[serde(default)]
    pub detached: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    #[serde(default, skip_serializing_if = "WindowPlace::is_empty")]
    pub window: WindowPlace,
}

impl Default for MetersFace {
    fn default() -> Self {
        Self {
            locked: true,
            detached: false,
            order: Vec::new(),
            window: WindowPlace::default(),
        }
    }
}

impl MetersFace {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

fn default_locked() -> bool {
    true
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

fn is_tune(page: &str) -> bool {
    page.is_empty() || page == "Tune"
}

fn finite_place(name: &str, value: Option<f32>, positive: bool) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    if !value.is_finite() || (positive && value <= 0.0) {
        return Err(format!("meter window {name} is not a usable number"));
    }
    Ok(())
}

fn validate_window(window: &WindowPlace) -> Result<(), String> {
    finite_place("x", window.x, false)?;
    finite_place("y", window.y, false)?;
    finite_place("width", window.width, true)?;
    finite_place("height", window.height, true)?;
    finite_place("monitor_width", window.monitor_width, true)?;
    finite_place("monitor_height", window.monitor_height, true)?;
    finite_place("pixels_per_point", window.pixels_per_point, true)?;
    Ok(())
}

/// `~/.config/hamspot-commander`, or `$XDG_CONFIG_HOME/hamspot-commander`
/// when that variable is set. The portable launcher sets it to the tree's
/// `config` directory, so the default station travels with the folder.
pub fn station_dir() -> PathBuf {
    station_dir_in(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

pub fn station_dir_in(xdg: Option<&std::ffi::OsStr>, home: Option<&std::ffi::OsStr>) -> PathBuf {
    if let Some(config) = xdg {
        if !config.is_empty() {
            return PathBuf::from(config).join("hamspot-commander");
        }
    }
    if let Some(home) = home {
        if !home.is_empty() {
            return PathBuf::from(home)
                .join(".config")
                .join("hamspot-commander");
        }
    }
    PathBuf::from("hamspot-commander")
}

pub fn default_station_path() -> PathBuf {
    station_dir().join(DEFAULT_STATION_NAME)
}

/// The station a launch should open. A non-option argument is a clicked file.
/// Anything else is [`default_station_path`].
pub fn station_from_args<I, S>(args: I) -> PathBuf
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for arg in args {
        let arg = arg.as_ref().trim();
        if arg.is_empty() || arg.starts_with('-') {
            continue;
        }
        return PathBuf::from(arg);
    }
    default_station_path()
}

/// Keep `.HSCommander` and an older `.json` station. Anything else gains the extension.
pub fn with_station_extension(path: &Path) -> PathBuf {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.to_path_buf();
    };
    if name.ends_with(".HSCommander") || name.ends_with(".hscommander") || name.ends_with(".json") {
        return path.to_path_buf();
    }
    path.with_file_name(format!("{name}.{STATION_EXTENSION}"))
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
    /// The one-slot `A > Mem` frequency. Absent in a file from before it was stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_hz: Option<f64>,
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
    crate::talkgroups::validate(&file.talkgroups)?;
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
    if let Some(hz) = file.tune.mem_hz {
        if !hz.is_finite() || hz <= 0.0 {
            return Err("memory frequency must be greater than zero".into());
        }
    }
    validate_window(&file.meters.window)?;
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
    let mut seen_follow = Vec::new();
    for index in &file.control_follow {
        if *index >= file.radios.len() || seen_follow.contains(index) {
            return Err(format!(
                "control follow radio {index} is not in the radio list"
            ));
        }
        seen_follow.push(*index);
    }
    let mut seen_trunk = Vec::new();
    for entry in &file.trunk_receivers {
        if entry.radio >= file.radios.len() || seen_trunk.contains(&entry.radio) {
            return Err(format!(
                "trunk receiver {} is not in the radio list",
                entry.radio
            ));
        }
        seen_trunk.push(entry.radio);
    }
    let mut seen_zones = Vec::new();
    for index in &file.scan_zones {
        if *index >= file.channels.zones.len() || seen_zones.contains(index) {
            return Err(format!("scan zone {index} is not in the zone list"));
        }
        seen_zones.push(*index);
    }
    let mut seen_monitor = Vec::new();
    for index in &file.monitor_zones {
        if *index >= file.talkgroups.zones.len() || seen_monitor.contains(index) {
            return Err(format!(
                "monitor zone {index} is not in the talkgroup list"
            ));
        }
        seen_monitor.push(*index);
    }
    let mut seen_tg = Vec::new();
    for tg in &file.monitor_talkgroups {
        if *tg == 0 || seen_tg.contains(tg) {
            return Err(format!("monitor talkgroup {tg} is not a talkgroup number"));
        }
        seen_tg.push(*tg);
    }
    Ok(())
}

/// One flag per radio. `indexes` are the radios whose talkgroup Follow box is on.
pub fn control_follow_flags(radio_count: usize, indexes: &[usize]) -> Vec<bool> {
    let mut on = vec![false; radio_count];
    for index in indexes {
        if let Some(flag) = on.get_mut(*index) {
            *flag = true;
        }
    }
    on
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
            talkgroups: crate::talkgroups::empty(),
            units: Vec::new(),
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
                mem_hz: None,
            },
            levels: BTreeMap::from([("AF".into(), 0.25)]),
            radios: Vec::new(),
            audio: AudioGraph::default(),
            rotators: Vec::new(),
            selected: 0,
            page: String::new(),
            channel_zone: 0,
            channel_sel: 0,
            talkgroup_zone: 0,
            talkgroup_sel: 0,
            unit_sel: 0,
            control_follow: Vec::new(),
            trunk_receivers: Vec::new(),
            scan_zones: Vec::new(),
            monitor_zones: Vec::new(),
            monitor_talkgroups: Vec::new(),
            meters: MetersFace::default(),
        }
    }

    #[test]
    fn scan_zones_round_trip_and_an_old_file_leaves_them_clear() {
        let mut file = sample();
        file.scan_zones = vec![0];
        let text = to_string(&file).unwrap();
        assert!(text.contains("\"scan_zones\""));
        let back = parse(&text).unwrap();
        assert_eq!(back.scan_zones, vec![0]);
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value.as_object_mut().unwrap().remove("scan_zones");
        let loaded = parse(&value.to_string()).unwrap();
        assert!(loaded.scan_zones.is_empty());
        file.scan_zones = vec![4];
        assert!(to_string(&file).is_err());
        file.scan_zones = vec![0, 0];
        assert!(to_string(&file).is_err());
    }

    #[test]
    fn monitor_lists_round_trip_and_an_old_file_leaves_them_clear() {
        let mut file = sample();
        file.monitor_zones = vec![0];
        file.monitor_talkgroups = vec![10006];
        let text = to_string(&file).unwrap();
        assert!(text.contains("\"monitor_zones\""));
        assert!(text.contains("\"monitor_talkgroups\""));
        let back = parse(&text).unwrap();
        assert_eq!(back.monitor_zones, vec![0]);
        assert_eq!(back.monitor_talkgroups, vec![10006]);
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value.as_object_mut().unwrap().remove("monitor_zones");
        value.as_object_mut().unwrap().remove("monitor_talkgroups");
        let loaded = parse(&value.to_string()).unwrap();
        assert!(loaded.monitor_zones.is_empty());
        assert!(loaded.monitor_talkgroups.is_empty());
        file.monitor_zones = vec![1];
        assert!(to_string(&file).is_err());
        file.monitor_zones = vec![0, 0];
        assert!(to_string(&file).is_err());
        file.monitor_zones.clear();
        file.monitor_talkgroups = vec![0];
        assert!(to_string(&file).is_err());
        file.monitor_talkgroups = vec![10006, 10006];
        assert!(to_string(&file).is_err());
    }

    #[test]
    fn a_hackrf_radio_loads_and_an_older_file_stays_hamlib() {
        let mut file = sample();
        let mut spec = crate::cat::RadioSpec::dummy();
        spec.driver = crate::cat::RadioDriver::Hackrf;
        spec.name = "HackRF".into();
        spec.model = 0;
        spec.park = false;
        spec.device = "0000000000000000457863c8252d451f".into();
        file.radios.push(spec);
        let text = to_string(&file).unwrap();
        assert!(text.contains("\"hackrf\""));
        let back = parse(&text).unwrap();
        assert_eq!(back.radios[0].driver, crate::cat::RadioDriver::Hackrf);
        assert_eq!(back.radios[0].sample_rate, crate::hackrf::SAMPLE_RATE);
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["radios"][0]["driver"] = serde_json::json!("hamlib");
        value["radios"][0]["model"] = serde_json::json!(1);
        value["radios"][0].as_object_mut().unwrap().remove("sample_rate");
        let loaded = parse(&value.to_string()).unwrap();
        assert_eq!(loaded.radios[0].driver, crate::cat::RadioDriver::Hamlib);
        assert_eq!(loaded.radios[0].sample_rate, crate::hackrf::SAMPLE_RATE);
        value["radios"][0]["driver"] = serde_json::json!("gqrx");
        let gqrx = parse(&value.to_string()).unwrap();
        assert_eq!(gqrx.radios[0].driver, crate::cat::RadioDriver::Gqrx);
    }

    #[test]
    fn safet_station_file_loads_when_it_is_present() {
        let path = Path::new("/home/user/.config/hamspot-commander/SAFE-T.HSCommander");
        if !path.is_file() {
            return;
        }
        let file = load_file(path).expect("SAFE-T station");
        assert_eq!(file.channels.zones.len(), 158);
        let channels: Vec<_> = file
            .channels
            .zones
            .iter()
            .flat_map(|zone| zone.channels.iter())
            .collect();
        assert_eq!(channels.len(), 927);
        assert!(channels.iter().any(|channel| channel.control));
        assert!(file.talkgroups.zones.len() > 1);
        assert!(file
            .talkgroups
            .zones
            .iter()
            .flat_map(|zone| zone.talkgroups.iter())
            .any(|row| row.encrypted));
        assert!(file
            .radios
            .iter()
            .any(|radio| radio.rx_pipe.contains("commander-demod-p25.rxpipe")));
        assert!(file
            .radios
            .iter()
            .any(|radio| radio.tx_pipe.contains("commander-mod-p25.txpipe")));
    }

    #[test]
    fn station_file_embeds_talkgroups_channel_flags_and_pipes() {
        let mut file = sample();
        file.channels.zones[0].channels.push(crate::channels::Channel {
            name: "852.475c".into(),
            hz: 852_475_000.0,
            mode: "FM".into(),
            control: true,
            encrypted: false,
            vocoder: String::new(),
            invert: false,
        });
        file.talkgroups.zones[0].talkgroups.push(crate::talkgroups::Talkgroup {
            name: "01-LEDISP — Law: Dispatch (County)".into(),
            tg: 10006,
            encrypted: false,
        });
        file.radios.push(crate::cat::RadioSpec {
            driver: crate::cat::RadioDriver::Hamlib,
            name: "SAFE-T".into(),
            model: 1,
            device: String::new(),
            serial_speed: String::new(),
            civ: String::new(),
            ptt_type: String::new(),
            ptt_file: String::new(),
            conf: String::new(),
            park: false,
            follow: false,
            audio: crate::cat::RadioAudio::default(),
            gqrx: crate::cat::GqrxConfig::new(),
            rx_pipe: "commander-demod-p25.rxpipe".into(),
            tx_pipe: "commander-mod-p25.txpipe".into(),
            sample_rate: crate::hackrf::SAMPLE_RATE,
            lna_db: None,
            vga_db: None,
        });
        let text = to_string(&file).unwrap();
        assert!(text.contains("\"talkgroups\""));
        assert!(text.contains("\"control\": true"));
        assert!(text.contains("commander-demod-p25.rxpipe"));
        assert!(text.contains("commander-mod-p25.txpipe"));
        let back = parse(&text).unwrap();
        assert!(back.channels.zones[0].channels[0].control);
        assert!(!back.channels.zones[0].channels[0].encrypted);
        assert_eq!(back.talkgroups.zones[0].talkgroups[0].tg, 10006);
        assert_eq!(back.radios[0].rx_pipe, "commander-demod-p25.rxpipe");
        assert_eq!(back.radios[0].tx_pipe, "commander-mod-p25.txpipe");
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value.as_object_mut().unwrap().remove("talkgroups");
        let loaded = parse(&value.to_string()).unwrap();
        assert_eq!(loaded.talkgroups.zones.len(), 1);
        assert!(loaded.talkgroups.zones[0].talkgroups.is_empty());
        assert!(loaded.units.is_empty());
        assert!(!text.contains("\"units\""));
    }

    #[test]
    fn control_follow_round_trips_and_an_old_file_leaves_it_off() {
        let mut file = sample();
        let mut second = crate::cat::RadioSpec::dummy();
        second.name = "HackRF".into();
        file.radios = vec![crate::cat::RadioSpec::dummy(), second];
        file.control_follow = vec![1];
        file.unit_sel = 3;
        let text = to_string(&file).unwrap();
        assert!(text.contains("\"control_follow\""));
        assert!(text.contains("\"unit_sel\""));
        let back = parse(&text).unwrap();
        assert_eq!(back.control_follow, vec![1]);
        assert_eq!(control_follow_flags(back.radios.len(), &back.control_follow), vec![
            false, true
        ]);
        assert_eq!(back.unit_sel, 3);
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value.as_object_mut().unwrap().remove("control_follow");
        value.as_object_mut().unwrap().remove("unit_sel");
        let loaded = parse(&value.to_string()).unwrap();
        assert!(loaded.control_follow.is_empty());
        assert_eq!(loaded.unit_sel, 0);
        file.control_follow = vec![5];
        assert!(to_string(&file).is_err());
        file.control_follow = vec![1, 1];
        assert!(to_string(&file).is_err());
    }

    #[test]
    fn trunk_receivers_round_trip_and_an_old_file_omits_them() {
        let mut file = sample();
        let mut second = crate::cat::RadioSpec::dummy();
        second.name = "Voice".into();
        file.radios = vec![crate::cat::RadioSpec::dummy(), second];
        file.trunk_receivers = vec![
            TrunkReceiver {
                radio: 0,
                role: TrunkRole::Control,
            },
            TrunkReceiver {
                radio: 1,
                role: TrunkRole::Voice,
            },
        ];
        let text = to_string(&file).unwrap();
        assert!(text.contains("\"trunk_receivers\""));
        assert!(text.contains("\"control\""));
        assert!(text.contains("\"voice\""));
        let back = parse(&text).unwrap();
        assert_eq!(back.trunk_receivers, file.trunk_receivers);
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value.as_object_mut().unwrap().remove("trunk_receivers");
        let loaded = parse(&value.to_string()).unwrap();
        assert!(loaded.trunk_receivers.is_empty());
        file.trunk_receivers = vec![TrunkReceiver {
            radio: 4,
            role: TrunkRole::Both,
        }];
        assert!(to_string(&file).is_err());
        file.trunk_receivers = vec![
            TrunkReceiver {
                radio: 0,
                role: TrunkRole::Both,
            },
            TrunkReceiver {
                radio: 0,
                role: TrunkRole::Voice,
            },
        ];
        assert!(to_string(&file).is_err());
    }

    #[test]
    fn radio_ids_round_trip_and_an_old_file_omits_them() {
        let mut file = sample();
        file.units.push(crate::units::Unit {
            id: 42,
            description: "engine 42".into(),
            last_tg: Some(100),
            peer: None,
            emergency: true,
            encrypted: false,
            site: Some(0x0A),
            wacn: Some("BEE00".into()),
            sys: None,
            reg: Some("REG_ACCEPT".into()),
        });
        let text = to_string(&file).unwrap();
        assert!(text.contains("\"units\""));
        assert!(!text.contains("\"peer\""));
        assert!(!text.contains("\"encrypted\""));
        let back = parse(&text).unwrap();
        assert_eq!(back.units[0].description, "engine 42");
        assert_eq!(back.units[0].last_tg, Some(100));
        assert!(back.units[0].emergency);
        assert_eq!(back.units[0].site, Some(0x0A));
        assert_eq!(back.units[0].wacn.as_deref(), Some("BEE00"));
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value.as_object_mut().unwrap().remove("units");
        let loaded = parse(&value.to_string()).unwrap();
        assert!(loaded.units.is_empty());
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
        value["not_a_station_field"] = serde_json::json!({});
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
            tx_pipe: String::new(),
            sample_rate: crate::hackrf::SAMPLE_RATE,
            lna_db: None,
            vga_db: None,
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
        assert!(!to_string(&file).unwrap().contains("tx_pipe"));
        file.radios[0].tx_pipe = "/opt/modems/commander-mod-am.txpipe".into();
        let text = to_string(&file).unwrap();
        assert!(text.contains("commander-mod-am.txpipe"));
        let back = parse(&text).unwrap();
        assert_eq!(
            back.radios[0].tx_pipe,
            "/opt/modems/commander-mod-am.txpipe"
        );
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["radios"][0]
            .as_object_mut()
            .unwrap()
            .remove("tx_pipe");
        let loaded = parse(&value.to_string()).unwrap();
        assert!(loaded.radios[0].tx_pipe.is_empty());
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
    fn a_clicked_file_wins_and_the_default_name_is_fixed() {
        let home = Path::new("/home/operator");
        assert_eq!(
            station_dir_in(None, Some(home.as_os_str())),
            home.join(".config").join("hamspot-commander")
        );
        let portable = Path::new("/stick/HamSpotCommander/config");
        assert_eq!(
            station_dir_in(Some(portable.as_os_str()), Some(home.as_os_str())),
            portable.join("hamspot-commander")
        );
        assert_eq!(
            station_from_args(std::iter::empty::<&str>())
                .file_name()
                .and_then(|name| name.to_str()),
            Some(DEFAULT_STATION_NAME)
        );
        assert_eq!(
            station_from_args(["", "--gpredict-cleanup"])
                .file_name()
                .and_then(|name| name.to_str()),
            Some(DEFAULT_STATION_NAME)
        );
        assert_eq!(
            station_from_args(["/tmp/Field.HSCommander"]),
            PathBuf::from("/tmp/Field.HSCommander")
        );
        assert_eq!(
            with_station_extension(Path::new("/tmp/Field")),
            PathBuf::from("/tmp/Field.HSCommander")
        );
        assert_eq!(
            with_station_extension(Path::new("/tmp/old.json")),
            PathBuf::from("/tmp/old.json")
        );
    }

    #[test]
    fn a_torn_off_meter_window_round_trips_and_an_old_file_stays_locked() {
        let text = to_string(&sample()).unwrap();
        assert!(!text.contains("meters"));
        let loaded = parse(&text).unwrap();
        assert!(loaded.meters.locked);
        assert!(!loaded.meters.detached);
        assert!(loaded.meters.window.is_empty());
        let mut file = sample();
        file.page = "Meters".into();
        file.selected = 1;
        file.channel_zone = 2;
        file.tune.mem_hz = Some(14_230_000.0);
        file.meters.locked = false;
        file.meters.detached = true;
        file.meters.order = vec!["STR".into(), "SWR".into()];
        file.meters.window = WindowPlace {
            x: Some(40.0),
            y: Some(-20.0),
            width: Some(1000.0),
            height: Some(560.0),
            monitor_width: Some(1920.0),
            monitor_height: Some(1080.0),
            pixels_per_point: Some(2.0),
        };
        let back = parse(&to_string(&file).unwrap()).unwrap();
        assert_eq!(back.page, "Meters");
        assert_eq!(back.selected, 1);
        assert_eq!(back.channel_zone, 2);
        assert_eq!(back.tune.mem_hz, Some(14_230_000.0));
        assert!(!back.meters.locked);
        assert!(back.meters.detached);
        assert_eq!(
            back.meters.order,
            vec!["STR".to_string(), "SWR".to_string()]
        );
        assert_eq!(back.meters.window.x, Some(40.0));
        assert_eq!(back.meters.window.y, Some(-20.0));
        assert_eq!(back.meters.window.monitor_width, Some(1920.0));
        file.meters.window.width = Some(0.0);
        assert!(to_string(&file).is_err());
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value.as_object_mut().unwrap().remove("meters");
        value.as_object_mut().unwrap().remove("page");
        value.as_object_mut().unwrap().remove("selected");
        let old = parse(&value.to_string()).unwrap();
        assert!(old.meters.is_default());
        assert_eq!(old.selected, 0);
        assert!(old.page.is_empty());
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

    #[test]
    fn the_agent_guide_example_station_loads() {
        let guide = include_str!("../../../docs/AGENT-HSCommander.md");
        let at = guide.find("North-County.HSCommander").expect("heading");
        let json_at = guide[at..].find("```json").expect("station fence");
        let body = at + json_at + "```json".len();
        let end = guide[body..].find("```").expect("station end");
        let station = guide[body..body + end].trim();
        let file = parse(station).expect("station");
        assert_eq!(file.channels.zones.len(), 1);
        assert_eq!(file.channels.zones[0].channels.len(), 3);
        assert!(file.channels.zones[0].channels[0].control);
        assert_eq!(file.tune.freq_hz, 855_212_500.0);
        assert!(file.talkgroups.zones[0].talkgroups[1].encrypted);
        let rest = &guide[body + end..];
        let key_at = rest.find("```json").expect("key fence");
        let key_body = key_at + "```json".len();
        let key_end = rest[key_body..].find("```").expect("key end");
        let keys = crate::keys::parse(rest[key_body..key_body + key_end].trim()).expect("keys");
        assert_eq!(keys.keys.len(), 1);
        assert!(keys.keys[0].text.is_empty());
        assert_eq!(keys.keys[0].alg, "84");
    }
}
