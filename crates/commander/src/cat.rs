//! Radios the face can keep open. Hamlib is the first driver. A later driver
//! is another variant of [`RadioDriver`], not a second manager.
//!
//! The program starts `rigctld` itself. It does not attach to a daemon it
//! did not spawn. The listen address is always 127.0.0.1, and the argv never
//! includes `-o`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::os::fd::RawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use serde::{Deserialize, Serialize};

use rigctl::{Command as CatCommand, LiveRig, Rig};

const PORT_START: u16 = 4532;
const PORT_TRIES: u16 = 64;
/// A busy serial port still exists, so inotify never fires. The daemon has
/// already exited. This is the gap before it is started again.
const RETRY_AFTER: Duration = Duration::from_secs(2);
/// Idle TCP sessions get no push from hamlib. The light read is how a dead
/// daemon is noticed. The selected radio is polled by the drive loop instead.
const IDLE_READ: Duration = Duration::from_secs(2);

/// What the drive loop is doing with the selected radio. The hub writes
/// `connect`. The drive loop writes the port it actually holds.
#[derive(Debug)]
pub struct FaceGate {
    pub demo: bool,
    pub face_port: Option<u16>,
    pub face_down: bool,
    pub connect: Option<u16>,
    /// The handed port speaks GQRX short commands.
    pub connect_classic: bool,
    pub connect_failed: bool,
    /// The selected radio is under Gpredict. The drive loop reads both VFOs
    /// and does not call `set_vfo` on each poll.
    pub follow: bool,
}

impl FaceGate {
    pub fn new() -> Self {
        Self {
            demo: false,
            face_port: None,
            face_down: true,
            connect: None,
            connect_classic: false,
            connect_failed: false,
            follow: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RadioDriver {
    Hamlib,
    Gqrx,
}

pub fn driver_name(driver: RadioDriver) -> &'static str {
    match driver {
        RadioDriver::Hamlib => "hamlib",
        RadioDriver::Gqrx => "gqrx",
    }
}

fn default_true() -> bool {
    true
}

/// One Gqrx QSettings document. Outer keys are INI groups. An empty
/// `input/device` is omitted so Gqrx opens the I/O device chooser.
pub type GqrxConfig = BTreeMap<String, BTreeMap<String, String>>;

fn gqrx_config_is_empty(config: &GqrxConfig) -> bool {
    config.is_empty()
}

/// Computer-side devices for one radio, and whether its card should exist.
/// `rx` and `tx` stay true until that node disappears after Commander has seen it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadioAudio {
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub output: String,
    #[serde(default = "default_true")]
    pub rx: bool,
    #[serde(default = "default_true")]
    pub tx: bool,
}

impl Default for RadioAudio {
    fn default() -> Self {
        Self {
            input: String::new(),
            output: String::new(),
            rx: true,
            tx: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadioSpec {
    pub driver: RadioDriver,
    pub name: String,
    pub model: i32,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub serial_speed: String,
    #[serde(default)]
    pub civ: String,
    #[serde(default)]
    pub ptt_type: String,
    #[serde(default)]
    pub ptt_file: String,
    #[serde(default)]
    pub conf: String,
    /// Park the hamlib dummy dials the first time this process answers.
    #[serde(default)]
    pub park: bool,
    /// Tune stops sending frequency commands and the poll follows Gpredict.
    #[serde(default)]
    pub follow: bool,
    /// Speaker and mic this radio's card is wired to.
    #[serde(default)]
    pub audio: RadioAudio,
    /// Gqrx settings for this radio. Written again as a native INI file
    /// under `~/.config/hamspot-commander/gqrx/` when the process starts.
    /// Absent on a hamlib row and on a station file from before this field.
    #[serde(default, skip_serializing_if = "gqrx_config_is_empty")]
    pub gqrx: GqrxConfig,
}

impl RadioSpec {
    pub fn dummy() -> Self {
        Self {
            driver: RadioDriver::Hamlib,
            name: "Dummy".into(),
            model: 1,
            device: String::new(),
            serial_speed: String::new(),
            civ: String::new(),
            ptt_type: String::new(),
            ptt_file: String::new(),
            conf: String::new(),
            park: true,
            follow: false,
            audio: RadioAudio::default(),
            gqrx: GqrxConfig::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RadioRow {
    pub id: u64,
    pub spec: RadioSpec,
    pub port: Option<u16>,
    pub status: String,
    pub selected: bool,
    pub make: String,
    pub model_label: String,
    pub audio_status: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HubView {
    pub rows: Vec<RadioRow>,
    pub serial: Vec<String>,
    pub audio: crate::audio::AudioGraph,
    pub playback: Vec<crate::audio::AudioNode>,
    pub capture: Vec<crate::audio::AudioNode>,
    /// Connectable ports from the latest full graph, keyed by node name.
    pub connect_ports: std::collections::BTreeMap<String, Vec<String>>,
    /// A dump has arrived and every card that should exist has been seen,
    /// or has been removed after it was seen. The window waits for this
    /// before it writes the open station file.
    pub audio_ready: bool,
    pub audio_note: String,
}

enum HubCmd {
    Add(RadioSpec),
    Update(u64, RadioSpec),
    Remove(u64),
    Select(u64),
    Replace(Vec<RadioSpec>),
    /// The station file's saved graph. Missing radio cards are created from it.
    ApplyAudio(crate::audio::AudioGraph),
    /// One `pw-dump` snapshot whose hamspot slice changed.
    AudioSnap(crate::audio::AudioLook),
    /// The Audio frame picked a computer-side source and sink.
    SetEndpoint(u64, String, String),
    /// Stop every child this program started and leave the supervisor loop.
    Shutdown,
    /// Gpredict follow for one radio. Does not restart `rigctld`.
    SetFollow(u64, bool),
}

pub struct Hub {
    cmds: Mutex<Vec<HubCmd>>,
    view: Mutex<HubView>,
    /// Process groups currently running. Shutdown signals these if the
    /// supervisor has not finished stopping them itself.
    pids: Mutex<Vec<i32>>,
    /// `pw-dump -m` process group. Shutdown signals it if the watcher is still up.
    audio_pid: Mutex<Option<i32>>,
    wake: RawFd,
    done: Mutex<bool>,
    done_wake: Condvar,
}

struct Slot {
    id: u64,
    spec: RadioSpec,
    port: Option<u16>,
    status: String,
    status_dirty: bool,
    child: Option<Child>,
    idle: Option<LiveRig>,
    retry_at: Option<Instant>,
    next_idle: Option<Instant>,
    park_pending: bool,
    handed: Option<u16>,
    /// Port written into this Gqrx config. `port` stays empty until the
    /// process accepts TCP, which is after the I/O chooser closes.
    assigned: Option<u16>,
    /// Next TCP probe. Gqrx does not signal that remote control is listening.
    probe_at: Option<Instant>,
    rx_audio: Option<Child>,
    tx_audio: Option<Child>,
    audio_wired: bool,
    saw_rx: bool,
    saw_tx: bool,
    make: String,
    model_label: String,
    audio_status: String,
    /// TCP port reserved in [`CLAIMED`] for this slot's daemon.
    held: Option<u16>,
}

pub fn validate_spec(spec: &RadioSpec) -> Result<(), String> {
    if spec.name.trim().is_empty() {
        return Err("radio name is empty".into());
    }
    if spec.driver == RadioDriver::Hamlib && spec.model <= 0 {
        return Err("hamlib model must be greater than zero".into());
    }
    if !spec.serial_speed.trim().is_empty() && spec.serial_speed.trim().parse::<u32>().is_err() {
        return Err("serial speed is not a number".into());
    }
    if !spec.civ.trim().is_empty() && spec.civ.trim().parse::<u32>().is_err() {
        return Err("CI-V address is not a number".into());
    }
    Ok(())
}

/// `rigctld` argv. `-o` is never included. An empty device omits `-r`.
pub fn hamlib_argv(spec: &RadioSpec, port: u16) -> Vec<String> {
    let mut args = vec![
        "-m".into(),
        spec.model.to_string(),
        "-T".into(),
        "127.0.0.1".into(),
        "-t".into(),
        port.to_string(),
    ];
    push_opt(&mut args, "-r", &spec.device);
    push_opt(&mut args, "-s", &spec.serial_speed);
    push_opt(&mut args, "-c", &spec.civ);
    push_opt(&mut args, "-P", &spec.ptt_type);
    push_opt(&mut args, "-p", &spec.ptt_file);
    push_opt(&mut args, "-C", &spec.conf);
    args
}

fn push_opt(args: &mut Vec<String>, flag: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    args.push(flag.into());
    args.push(value.into());
}

/// Ports a radio or a rotator has reserved. The listener is dropped so the
/// daemon can bind, and the set keeps the other supervisor from taking it
/// during that gap.
static CLAIMED: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

/// First port at or above `start` that this process can bind, up to 64 tries.
/// The port stays reserved until [`release_port`].
pub fn claim_port(start: u16) -> Result<u16, String> {
    let mut claimed = CLAIMED.lock().expect("ports");
    let last = start.saturating_add(PORT_TRIES - 1);
    for port in start..=last {
        if claimed.contains(&port) {
            continue;
        }
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            claimed.insert(port);
            return Ok(port);
        }
    }
    Err(format!("no free TCP port from {start} through {last}"))
}

pub fn release_port(port: u16) {
    CLAIMED.lock().expect("ports").remove(&port);
}

/// First port at or above `start` that this process can bind, up to 64 tries.
/// The listener is dropped before the function returns so `rigctld` can bind it.
/// The reservation is released too. A daemon that keeps the port uses [`claim_port`].
#[cfg(test)]
pub fn free_port(start: u16) -> Result<u16, String> {
    let port = claim_port(start)?;
    release_port(port);
    Ok(port)
}

/// USB serial names under `root`. `root/serial/by-id` wins when it has entries.
/// Otherwise `ttyUSB*` and `ttyACM*` directly in `root`.
pub fn serial_ports_in(root: &Path) -> Vec<String> {
    let by_id = root.join("serial").join("by-id");
    let mut named = dir_names(&by_id);
    if !named.is_empty() {
        named.sort();
        return named;
    }
    let mut tty = Vec::new();
    let Ok(read) = fs::read_dir(root) else {
        return tty;
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("ttyUSB") || name.starts_with("ttyACM") {
            tty.push(entry.path().display().to_string());
        }
    }
    tty.sort();
    tty
}

fn dir_names(dir: &Path) -> Vec<String> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    read.flatten()
        .map(|entry| entry.path().display().to_string())
        .collect()
}

/// One row of `rigctld -l`. The menu draws these in fixed monospace columns.
#[derive(Clone, Debug, PartialEq)]
pub struct HamlibModel {
    pub id: i32,
    pub mfg: String,
    pub model: String,
    pub version: String,
    pub status: String,
    pub macro_name: String,
}

/// Column starts in `rigctld -l` on hamlib 4.6: Mfg, Model, Version, Status, Macro.
const COL_MFG: usize = 8;
const COL_MODEL: usize = 31;
const COL_VERSION: usize = 55;
const COL_STATUS: usize = 71;
const COL_MACRO: usize = 83;

pub fn parse_model_list(text: &str) -> Vec<HamlibModel> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let Some((id, _)) = line.trim().split_once(|ch: char| ch.is_whitespace()) else {
            continue;
        };
        let Ok(id) = id.parse::<i32>() else {
            continue;
        };
        if id <= 0 {
            continue;
        }
        rows.push(HamlibModel {
            id,
            mfg: column(line, COL_MFG, Some(COL_MODEL)),
            model: column(line, COL_MODEL, Some(COL_VERSION)),
            version: column(line, COL_VERSION, Some(COL_STATUS)),
            status: column(line, COL_STATUS, Some(COL_MACRO)),
            macro_name: column(line, COL_MACRO, None),
        });
    }
    rows
}

fn column(line: &str, start: usize, end: Option<usize>) -> String {
    let end = end.unwrap_or(line.len()).min(line.len());
    // `rigctld -l` is ASCII. A short line or a bad index is an empty field.
    if start >= end || !line.is_char_boundary(start) || !line.is_char_boundary(end) {
        return String::new();
    }
    line[start..end].trim().to_string()
}

/// Left-aligned fields. The same widths are used for the header and every row.
pub fn format_model_row(row: &HamlibModel) -> String {
    format!(
        "{:<7}{:<22}{:<24}{:<16}{:<12}{}",
        row.id, row.mfg, row.model, row.version, row.status, row.macro_name
    )
}

pub fn model_menu_header() -> String {
    format!(
        "{:<7}{:<22}{:<24}{:<16}{:<12}{}",
        "Rig #", "Mfg", "Model", "Version", "Status", "Macro"
    )
}

/// Closed combo button. Padding stays in the menu, not on the button.
pub fn model_button_label(row: &HamlibModel) -> String {
    match (row.mfg.is_empty(), row.model.is_empty()) {
        (true, true) => row.id.to_string(),
        (true, false) => format!("{}  {}", row.id, row.model),
        (false, true) => format!("{}  {}", row.id, row.mfg),
        (false, false) => format!("{}  {} {}", row.id, row.mfg, row.model),
    }
}

pub fn model_list() -> Result<Vec<HamlibModel>, String> {
    let output = Command::new("rigctld")
        .arg("-l")
        .output()
        .map_err(|err| format!("rigctld -l: {err}"))?;
    if !output.status.success() {
        return Err(format!("rigctld -l exited {}", output.status));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_model_list(&text))
}

pub fn spawn_rigctld(spec: &RadioSpec, port: u16) -> Result<Child, String> {
    validate_spec(spec)?;
    let mut cmd = Command::new("rigctld");
    cmd.args(hamlib_argv(spec, port))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    cmd.spawn().map_err(|err| format!("rigctld: {err}"))
}

fn config_home() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".config")
}

fn sanitize_radio_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('_');
        }
    }
    if out.is_empty() {
        "gqrx".into()
    } else {
        out
    }
}

pub fn gqrx_conf_path(config_home: &Path, name: &str) -> PathBuf {
    config_home
        .join("hamspot-commander")
        .join("gqrx")
        .join(format!("{}.conf", sanitize_radio_name(name)))
}

pub fn gqrx_conf_for_name(name: &str) -> PathBuf {
    gqrx_conf_path(&config_home(), name)
}

/// Settings for a Gqrx that has never had an input device. `loadConfig`
/// returns false when `input/device` is missing, and the window then opens
/// the I/O device chooser. `crashed` stays false so the crash guard does not
/// replace that chooser.
pub fn gqrx_chooser_config() -> GqrxConfig {
    let mut groups = GqrxConfig::new();
    let mut general = BTreeMap::new();
    general.insert("configversion".into(), "4".into());
    general.insert("crashed".into(), "false".into());
    groups.insert("General".into(), general);
    let mut remote = BTreeMap::new();
    remote.insert("allowed_hosts".into(), "127.0.0.1".into());
    remote.insert("enabled".into(), "true".into());
    groups.insert("remote_control".into(), remote);
    groups
}

pub fn gqrx_needs_chooser(config: &GqrxConfig) -> bool {
    match config.get("input").and_then(|group| group.get("device")) {
        Some(device) => device.trim().is_empty(),
        None => true,
    }
}

fn normalize_gqrx(config: &mut GqrxConfig) {
    let general = config.entry("General".into()).or_default();
    general.insert("crashed".into(), "false".into());
    if !general.contains_key("configversion") {
        general.insert("configversion".into(), "4".into());
    }
    if let Some(input) = config.get_mut("input") {
        if input
            .get("device")
            .is_some_and(|device| device.trim().is_empty())
        {
            input.remove("device");
        }
    }
}

/// QSettings INI. Values stay strings, which is how Gqrx writes them.
pub fn parse_gqrx_native(text: &str) -> GqrxConfig {
    let mut groups = GqrxConfig::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() > 2 {
            let name = trimmed[1..trimmed.len() - 1].trim().to_string();
            if name.is_empty() {
                current = None;
                continue;
            }
            groups.entry(name.clone()).or_default();
            current = Some(name);
            continue;
        }
        let Some(group) = current.clone() else {
            continue;
        };
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        groups
            .entry(group)
            .or_default()
            .insert(key.to_string(), value.trim().to_string());
    }
    groups
}

pub fn render_gqrx_native(config: &GqrxConfig, port: u16) -> String {
    let mut text = String::new();
    for (group, keys) in config {
        let mut body = String::new();
        for (key, value) in keys {
            if group.eq_ignore_ascii_case("input")
                && key.eq_ignore_ascii_case("device")
                && value.trim().is_empty()
            {
                continue;
            }
            if group.eq_ignore_ascii_case("General") && key.eq_ignore_ascii_case("crashed") {
                body.push_str("crashed=false\n");
                continue;
            }
            body.push_str(key);
            body.push('=');
            body.push_str(value);
            body.push('\n');
        }
        if body.is_empty() {
            continue;
        }
        text.push('[');
        text.push_str(group);
        text.push_str("]\n");
        text.push_str(&body);
        text.push('\n');
    }
    patch_remote_control(&text, port)
}

fn read_gqrx_file(path: &Path) -> Result<Option<GqrxConfig>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(Some(parse_gqrx_native(&text)))
}

/// Write `config` in Gqrx's INI format. The assigned listen port replaces
/// `remote_control/port`. A missing parent directory is created. An existing
/// file is replaced, so a new instance does not keep another radio's device.
pub fn write_gqrx_native(path: &Path, config: &GqrxConfig, port: u16) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    let text = render_gqrx_native(config, port);
    fs::write(path, text).map_err(|err| format!("{}: {err}", path.display()))
}

/// Use the embedded document. A radio saved before that field existed keeps
/// the native file already on disk when there is one.
fn prepare_gqrx_document(spec: &mut RadioSpec) -> Result<PathBuf, String> {
    prepare_gqrx_document_in(&config_home(), spec)
}

fn prepare_gqrx_document_in(config_home: &Path, spec: &mut RadioSpec) -> Result<PathBuf, String> {
    let path = gqrx_conf_path(config_home, &spec.name);
    if spec.gqrx.is_empty() {
        match read_gqrx_file(&path)? {
            Some(found) => spec.gqrx = found,
            None => spec.gqrx = gqrx_chooser_config(),
        }
    }
    normalize_gqrx(&mut spec.gqrx);
    Ok(path)
}

/// Rewrite only the `remote_control` group. Every other group stays.
pub fn patch_remote_control(text: &str, port: u16) -> String {
    let mut out = String::new();
    let mut in_group = false;
    let mut saw_group = false;
    let mut saw_enabled = false;
    let mut saw_port = false;
    let flush = |out: &mut String, saw_enabled: bool, saw_port: bool| {
        if !saw_enabled {
            out.push_str("enabled=true\n");
        }
        if !saw_port {
            out.push_str(&format!("port={port}\n"));
        }
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_group {
                flush(&mut out, saw_enabled, saw_port);
            }
            in_group = trimmed.eq_ignore_ascii_case("[remote_control]");
            if in_group {
                saw_group = true;
                saw_enabled = false;
                saw_port = false;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_group {
            let key = trimmed.split_once('=').map(|(key, _)| key.trim());
            if key.is_some_and(|key| key.eq_ignore_ascii_case("enabled")) {
                out.push_str("enabled=true\n");
                saw_enabled = true;
                continue;
            }
            if key.is_some_and(|key| key.eq_ignore_ascii_case("port")) {
                out.push_str(&format!("port={port}\n"));
                saw_port = true;
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if in_group {
        flush(&mut out, saw_enabled, saw_port);
    }
    if !saw_group {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("[remote_control]\n");
        out.push_str("enabled=true\n");
        out.push_str(&format!("port={port}\n"));
    }
    out
}

pub fn spawn_gqrx(spec: &mut RadioSpec, port: u16, id: u64) -> Result<Child, String> {
    validate_spec(spec)?;
    let path = prepare_gqrx_document(spec)?;
    write_gqrx_native(&path, &spec.gqrx, port)?;
    let mut cmd = Command::new("gqrx");
    // The card's node names are the Pulse devices this process should open.
    cmd.env("PULSE_SINK", crate::audio::rx_name(id))
        .env("PULSE_SOURCE", crate::audio::tx_name(id))
        .arg("-c")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    cmd.spawn().map_err(|err| format!("gqrx: {err}"))
}

fn spawn_radio(spec: &mut RadioSpec, port: u16, id: u64) -> Result<Child, String> {
    match spec.driver {
        RadioDriver::Hamlib => spawn_rigctld(spec, port),
        RadioDriver::Gqrx => spawn_gqrx(spec, port, id),
    }
}

fn open_rig(spec: &RadioSpec, port: u16) -> Result<LiveRig, rigctl::RigError> {
    match spec.driver {
        RadioDriver::Hamlib => LiveRig::connect("127.0.0.1", port),
        RadioDriver::Gqrx => LiveRig::connect_classic("127.0.0.1", port),
    }
}

/// TCP connect succeeds, or the child has exited. A refused connect returns
/// at once and `rigctld` publishes no listen event, so the gap between tries
/// is a short wait.
pub fn wait_ready(port: u16, child: &mut Child) -> Result<(), String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(format!(
                    "rigctld exited before it accepted a connection ({status})"
                ));
            }
            Ok(None) => {}
            Err(err) => return Err(err.to_string()),
        }
        match TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
            Ok(stream) => {
                drop(stream);
                return Ok(());
            }
            Err(_) if start.elapsed() < Duration::from_secs(3) => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(format!("127.0.0.1:{port}: {err}")),
        }
    }
}

pub fn park_dummy(rig: &mut LiveRig) -> Result<(), String> {
    let steps = [
        ("VFOA", 14_153_608.0, "USB", 2400),
        ("VFOB", 146_520_000.0, "FM", 15000),
        ("VFOC", 446_000_000.0, "FM", 15000),
        ("VFOA", 14_153_608.0, "USB", 2400),
    ];
    for (vfo, hz, mode, pass) in steps {
        rig.apply(&CatCommand::SetVfo(vfo.into()))
            .map_err(|err| err.to_string())?;
        rig.apply(&CatCommand::SetFreq(hz))
            .map_err(|err| err.to_string())?;
        rig.apply(&CatCommand::SetMode {
            mode: mode.into(),
            passband_hz: pass,
        })
        .map_err(|err| err.to_string())?;
    }
    Ok(())
}

pub fn stop_child(child: &mut Child) {
    let pid = child.id() as i32;
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    let start = Instant::now();
    loop {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        if start.elapsed() > Duration::from_secs(1) {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.wait();
            return;
        }
        // The child has been signaled. try_wait is the completion check.
        thread::sleep(Duration::from_millis(20));
    }
}

impl Hub {
    pub fn spawn(
        face: Arc<Mutex<FaceGate>>,
        kick: Arc<dyn Fn() + Send + Sync>,
        ctx: egui::Context,
    ) -> Arc<Self> {
        let wake = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        let hub = Arc::new(Self {
            cmds: Mutex::new(Vec::new()),
            view: Mutex::new(HubView::default()),
            pids: Mutex::new(Vec::new()),
            audio_pid: Mutex::new(None),
            wake,
            done: Mutex::new(false),
            done_wake: Condvar::new(),
        });
        let thread_hub = Arc::clone(&hub);
        thread::spawn(move || supervise(thread_hub, face, kick, ctx));
        hub
    }

    pub fn view(&self) -> HubView {
        self.view.lock().expect("cat view").clone()
    }

    pub fn add(&self, spec: RadioSpec) {
        self.send(HubCmd::Add(spec));
    }

    pub fn update(&self, id: u64, spec: RadioSpec) {
        self.send(HubCmd::Update(id, spec));
    }

    pub fn remove(&self, id: u64) {
        self.send(HubCmd::Remove(id));
    }

    pub fn select(&self, id: u64) {
        self.send(HubCmd::Select(id));
    }

    pub fn replace(&self, specs: Vec<RadioSpec>) {
        self.send(HubCmd::Replace(specs));
    }

    pub fn apply_audio(&self, graph: crate::audio::AudioGraph) {
        self.send(HubCmd::ApplyAudio(graph));
    }

    pub fn set_endpoint(&self, id: u64, input: String, output: String) {
        self.send(HubCmd::SetEndpoint(id, input, output));
    }

    /// Record Gpredict follow. The running `rigctld` stays up.
    pub fn set_follow(&self, id: u64, follow: bool) {
        self.send(HubCmd::SetFollow(id, follow));
    }

    /// Wake the supervisor without a command. Leaving Demo uses this so the
    /// selected radio is handed back without waiting out the idle read.
    pub fn wake(&self) {
        self.poke();
    }

    /// Stop every `rigctld` this program started. Returns when the supervisor
    /// has stopped them, or after the backstop below.
    ///
    /// The supervisor's `stop_child` is the completion signal. The wait is
    /// bounded because a supervisor that already died cannot set `done`, and
    /// a window close must not hang. Eight seconds covers one in-flight
    /// `wait_ready` (three seconds) plus stopping the children. Anything still
    /// listed in `pids` is signaled after that.
    pub fn shutdown(&self) {
        self.send(HubCmd::Shutdown);
        let guard = self.done.lock().expect("cat done");
        let (guard, _) = self
            .done_wake
            .wait_timeout(guard, Duration::from_secs(8))
            .expect("cat done");
        if *guard {
            return;
        }
        drop(guard);
        if let Some(pid) = *self.audio_pid.lock().expect("audio pid") {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
        }
        let pids = self.pids.lock().expect("cat pids").clone();
        for pid in pids {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
        }
    }

    fn send(&self, cmd: HubCmd) {
        self.cmds.lock().expect("cat cmds").push(cmd);
        self.poke();
    }

    fn poke(&self) {
        if self.wake < 0 {
            return;
        }
        let one = 1u64;
        unsafe {
            libc::write(
                self.wake,
                &one as *const u64 as *const libc::c_void,
                std::mem::size_of::<u64>(),
            );
        }
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        if self.wake >= 0 {
            unsafe { libc::close(self.wake) };
        }
    }
}

fn supervise(
    hub: Arc<Hub>,
    face: Arc<Mutex<FaceGate>>,
    kick: Arc<dyn Fn() + Send + Sync>,
    ctx: egui::Context,
) {
    let models = model_list().unwrap_or_default();
    let mut slots = vec![new_slot(1, RadioSpec::dummy())];
    let mut next_id = 2u64;
    let mut selected = 1u64;
    let mut serial = serial_ports_in(Path::new("/dev"));
    let mut look = crate::audio::AudioLook::default();
    let mut wanted = crate::audio::AudioGraph::default();
    let mut links_pending = false;
    let mut audio_note = String::new();
    match watch_dump(Arc::clone(&hub)) {
        Some(pid) => *hub.audio_pid.lock().expect("audio pid") = Some(pid),
        None => audio_note = "pw-dump is not available".into(),
    }
    let watch = DevWatch::open();
    publish(&hub, &slots, selected, &serial, &look, false, &audio_note);
    ctx.request_repaint();
    loop {
        let cmds = std::mem::take(&mut *hub.cmds.lock().expect("cat cmds"));
        let mut changed = !cmds.is_empty();
        let mut stop = false;
        for cmd in cmds {
            changed = true;
            match cmd {
                HubCmd::Shutdown => stop = true,
                HubCmd::SetFollow(id, follow) => {
                    if let Some(slot) = slots.iter_mut().find(|slot| slot.id == id) {
                        slot.spec.follow = follow;
                        slot.status_dirty = true;
                    }
                }
                HubCmd::Add(spec) => {
                    if validate_spec(&spec).is_err() {
                        continue;
                    }
                    let id = next_id;
                    next_id += 1;
                    slots.push(new_slot(id, spec));
                    selected = id;
                    clear_hand(&mut slots);
                }
                HubCmd::Update(id, spec) => {
                    if validate_spec(&spec).is_err() {
                        continue;
                    }
                    if let Some(slot) = slots.iter_mut().find(|slot| slot.id == id) {
                        stop_slot(slot);
                        // An edit restarts the daemon. Park only if this slot
                        // had not parked yet. A dummy the operator already
                        // used keeps whatever the new process starts with,
                        // and a later reconnect of that process does not
                        // rewrite the dials either.
                        let park = spec.park && slot.park_pending;
                        let mut spec = spec;
                        // Stop flushed Gqrx's QSettings. The edit dialog does
                        // not change that document, so the device chosen in
                        // the I/O chooser stays with this radio.
                        if spec.driver == RadioDriver::Gqrx && !gqrx_needs_chooser(&slot.spec.gqrx)
                        {
                            spec.gqrx = slot.spec.gqrx.clone();
                        }
                        slot.spec = spec;
                        slot.park_pending = park;
                        slot.status = "starting".into();
                        slot.status_dirty = true;
                    }
                }
                HubCmd::Remove(id) => {
                    if let Some(pos) = slots.iter().position(|slot| slot.id == id) {
                        let mut slot = slots.remove(pos);
                        stop_slot(&mut slot);
                        if selected == id {
                            selected = slots.first().map(|slot| slot.id).unwrap_or(0);
                        }
                        clear_hand(&mut slots);
                    }
                }
                HubCmd::Select(id) => {
                    if slots.iter().any(|slot| slot.id == id) && selected != id {
                        selected = id;
                        clear_hand(&mut slots);
                        if let Some(slot) = slots.iter_mut().find(|slot| slot.id == id) {
                            slot.idle = None;
                        }
                    }
                }
                HubCmd::ApplyAudio(graph) => {
                    wanted = graph;
                    links_pending = !wanted.links.is_empty();
                    for slot in &mut slots {
                        slot.audio_wired = false;
                    }
                    let live = crate::audio::dump_once().unwrap_or_else(|| look.graph.clone());
                    restore_extra_devices(&wanted, &live, &slots, &mut audio_note);
                }
                HubCmd::AudioSnap(snap) => {
                    note_audio(&mut slots, &snap.graph);
                    look = snap;
                }
                HubCmd::SetEndpoint(id, input, output) => {
                    if let Some(slot) = slots.iter_mut().find(|slot| slot.id == id) {
                        retarget(slot, &input, &output, &look.graph);
                        slot.status_dirty = true;
                    }
                }
                HubCmd::Replace(specs) => {
                    for slot in &mut slots {
                        stop_slot(slot);
                    }
                    slots.clear();
                    selected = 0;
                    let specs = if specs.is_empty() {
                        vec![RadioSpec::dummy()]
                    } else {
                        specs
                    };
                    for spec in specs {
                        if validate_spec(&spec).is_err() {
                            continue;
                        }
                        let id = next_id;
                        next_id += 1;
                        if selected == 0 {
                            selected = id;
                        }
                        slots.push(new_slot(id, spec));
                    }
                    if slots.is_empty() {
                        selected = next_id;
                        next_id += 1;
                        slots.push(new_slot(selected, RadioSpec::dummy()));
                    }
                    {
                        let mut gate = face.lock().expect("face");
                        gate.face_port = None;
                        gate.face_down = true;
                        gate.connect = None;
                        gate.connect_failed = false;
                    }
                }
            }
        }
        if stop {
            for slot in &mut slots {
                stop_slot(slot);
            }
            publish(&hub, &slots, selected, &serial, &look, false, &audio_note);
            break;
        }
        if let Some(watch) = &watch {
            if watch.drain() {
                serial = serial_ports_in(Path::new("/dev"));
                changed = true;
            }
        } else {
            serial = serial_ports_in(Path::new("/dev"));
        }
        let (demo, face_port, face_down, failed) = {
            let mut gate = face.lock().expect("face");
            let failed = gate.connect_failed;
            if failed {
                gate.connect_failed = false;
            }
            (gate.demo, gate.face_port, gate.face_down, failed)
        };
        let now = Instant::now();
        if failed {
            clear_hand(&mut slots);
        }
        if links_pending {
            links_pending = !restore_links(&wanted, &slots, &mut audio_note);
        }
        for slot in &mut slots {
            service_audio(slot, &models, &wanted, &look.graph);
            tick_slot(
                slot,
                slot.id == selected,
                demo,
                face_port,
                face_down || failed,
                &face,
                kick.as_ref(),
                now,
            );
        }
        let ready = audio_settled(&slots, &look.graph);
        if changed || slots.iter().any(|slot| slot.status_dirty) {
            publish(&hub, &slots, selected, &serial, &look, ready, &audio_note);
            for slot in &mut slots {
                slot.status_dirty = false;
            }
            ctx.request_repaint();
        }
        park_until(&hub, &watch, next_wait(&slots, Instant::now()));
    }
    *hub.done.lock().expect("cat done") = true;
    hub.done_wake.notify_all();
}

fn clear_hand(slots: &mut [Slot]) {
    for slot in slots {
        slot.handed = None;
    }
}

fn new_slot(id: u64, spec: RadioSpec) -> Slot {
    let park_pending = spec.park;
    Slot {
        id,
        spec,
        port: None,
        status: "starting".into(),
        status_dirty: true,
        child: None,
        idle: None,
        retry_at: None,
        next_idle: None,
        park_pending,
        handed: None,
        assigned: None,
        probe_at: None,
        rx_audio: None,
        tx_audio: None,
        audio_wired: false,
        saw_rx: false,
        saw_tx: false,
        make: String::new(),
        model_label: String::new(),
        audio_status: String::new(),
        held: None,
    }
}

fn release_held(slot: &mut Slot) {
    if let Some(port) = slot.held.take() {
        release_port(port);
    }
}

fn tick_slot(
    slot: &mut Slot,
    selected: bool,
    demo: bool,
    face_port: Option<u16>,
    face_down: bool,
    face: &Mutex<FaceGate>,
    kick: &dyn Fn(),
    now: Instant,
) {
    if let Some(child) = slot.child.as_mut() {
        match child.try_wait() {
            Ok(Some(status)) => {
                // A Gqrx that accepted a connection has written its settings,
                // including the device chosen in the I/O dialog. Read that
                // file before the next start replaces it.
                let connected = slot.port.is_some();
                slot.child = None;
                slot.idle = None;
                release_held(slot);
                slot.port = None;
                slot.assigned = None;
                slot.probe_at = None;
                slot.handed = None;
                if connected {
                    adopt_gqrx_file(slot);
                }
                slot.status = format!("retrying ({status})");
                slot.status_dirty = true;
                // The device node is still there when the port is busy.
                // inotify does not fire. Start again after RETRY_AFTER.
                slot.retry_at = Some(now + RETRY_AFTER);
            }
            Ok(None) => {}
            Err(_) => {
                slot.child = None;
                slot.idle = None;
                release_held(slot);
                slot.port = None;
                slot.assigned = None;
                slot.probe_at = None;
            }
        }
    }
    let device = slot.spec.device.trim().to_string();
    if !device.is_empty() && !Path::new(&device).exists() {
        if let Some(child) = slot.child.as_mut() {
            stop_child(child);
        }
        slot.child = None;
        slot.idle = None;
        release_held(slot);
        slot.port = None;
        slot.assigned = None;
        slot.probe_at = None;
        slot.handed = None;
        if slot.status != "device missing" {
            slot.status = "device missing".into();
            slot.status_dirty = true;
        }
        slot.retry_at = None;
        return;
    }
    if slot.child.is_none() {
        if slot.retry_at.is_some_and(|at| now < at) {
            return;
        }
        match claim_port(PORT_START).and_then(|port| {
            spawn_radio(&mut slot.spec, port, slot.id)
                .map(|child| (port, child))
                .map_err(|err| {
                    release_port(port);
                    err
                })
        }) {
            Ok((port, child)) if slot.spec.driver == RadioDriver::Gqrx => {
                slot.held = Some(port);
                // Leave the process up while the I/O chooser is open. Remote
                // control listens only after that dialog is accepted.
                slot.child = Some(child);
                slot.assigned = Some(port);
                slot.port = None;
                slot.retry_at = None;
                slot.probe_at = Some(now);
                slot.status = if gqrx_needs_chooser(&slot.spec.gqrx) {
                    "choose I/O device".into()
                } else {
                    format!("starting  127.0.0.1:{port}")
                };
                slot.status_dirty = true;
            }
            Ok((port, mut child)) => match wait_ready(port, &mut child) {
                Ok(()) => {
                    slot.held = Some(port);
                    if slot.park_pending && slot.spec.driver == RadioDriver::Hamlib {
                        if let Ok(mut rig) = LiveRig::connect("127.0.0.1", port) {
                            if park_dummy(&mut rig).is_ok() {
                                slot.park_pending = false;
                            }
                        }
                    }
                    slot.child = Some(child);
                    slot.port = Some(port);
                    slot.retry_at = None;
                    slot.handed = None;
                    slot.status = format!("up  127.0.0.1:{port}");
                    slot.status_dirty = true;
                }
                Err(err) => {
                    release_port(port);
                    stop_child(&mut child);
                    slot.status = err;
                    slot.status_dirty = true;
                    slot.retry_at = Some(now + RETRY_AFTER);
                }
            },
            Err(err) => {
                slot.status = err;
                slot.status_dirty = true;
                slot.retry_at = Some(now + RETRY_AFTER);
            }
        }
    }
    if slot.spec.driver == RadioDriver::Gqrx && slot.child.is_some() && slot.port.is_none() {
        probe_gqrx(slot, now);
    }
    let Some(port) = slot.port else {
        return;
    };
    if selected && !demo {
        slot.idle = None;
        let need = face_down || face_port != Some(port);
        let cooling = slot.retry_at.is_some_and(|at| now < at);
        if need && slot.handed != Some(port) && !cooling {
            let mut gate = face.lock().expect("face");
            gate.connect = Some(port);
            gate.connect_classic = slot.spec.driver == RadioDriver::Gqrx;
            gate.face_down = false;
            drop(gate);
            // `kick` notifies the drive condvar while holding the mutex that
            // condvar is waited on, so the port is visible before the wake.
            kick();
            slot.handed = Some(port);
            slot.retry_at = Some(now + RETRY_AFTER);
        }
        if face_port == Some(port) && !face_down {
            slot.handed = None;
            slot.retry_at = None;
            if !slot.status.starts_with("up") {
                slot.status = format!("up  127.0.0.1:{port}");
                slot.status_dirty = true;
            }
        }
        return;
    }
    // The drive loop still has this port. A second client would poll the
    // same radio. Idle starts after the drive loop clears the port.
    if face_port == Some(port) {
        slot.idle = None;
        return;
    }
    if slot.idle.is_none() {
        if slot.next_idle.is_some_and(|at| now < at) {
            return;
        }
        match open_rig(&slot.spec, port) {
            Ok(rig) => {
                slot.idle = Some(rig);
                slot.next_idle = Some(now + IDLE_READ);
            }
            Err(_) => slot.next_idle = Some(now + IDLE_READ),
        }
        return;
    }
    if slot.next_idle.is_some_and(|at| now < at) {
        return;
    }
    let failed = slot.idle.as_mut().is_some_and(|rig| rig.ping().is_err());
    slot.next_idle = Some(now + IDLE_READ);
    if failed {
        slot.idle = None;
        if slot.status.starts_with("up") {
            slot.status = "retrying".into();
            slot.status_dirty = true;
        }
    }
}

/// One connect attempt. Gqrx publishes no listen event, so the accept is
/// the completion signal and this probe is only how soon it is noticed.
fn probe_gqrx(slot: &mut Slot, now: Instant) {
    if slot.probe_at.is_some_and(|at| now < at) {
        return;
    }
    let Some(port) = slot.assigned else {
        return;
    };
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    match TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
        Ok(stream) => {
            drop(stream);
            slot.port = Some(port);
            slot.probe_at = None;
            slot.retry_at = None;
            adopt_gqrx_file(slot);
            slot.status = format!("up  127.0.0.1:{port}");
            slot.status_dirty = true;
        }
        Err(_) => {
            slot.probe_at = Some(now + Duration::from_millis(250));
            let status = if gqrx_needs_chooser(&slot.spec.gqrx) {
                "choose I/O device".to_string()
            } else {
                format!("starting  127.0.0.1:{port}")
            };
            if slot.status != status {
                slot.status = status;
                slot.status_dirty = true;
            }
        }
    }
}

/// Fold the native file back into the station row. A missing or unreadable
/// file leaves the embedded copy unchanged.
fn adopt_gqrx_file(slot: &mut Slot) {
    if slot.spec.driver != RadioDriver::Gqrx {
        return;
    }
    let path = gqrx_conf_for_name(&slot.spec.name);
    let Ok(Some(mut found)) = read_gqrx_file(&path) else {
        return;
    };
    normalize_gqrx(&mut found);
    if found != slot.spec.gqrx {
        slot.spec.gqrx = found;
        slot.status_dirty = true;
    }
}

fn next_wait(slots: &[Slot], now: Instant) -> Duration {
    let mut wait = IDLE_READ;
    for slot in slots {
        for at in [slot.retry_at, slot.next_idle, slot.probe_at]
            .into_iter()
            .flatten()
        {
            if at <= now {
                return Duration::from_millis(1);
            }
            wait = wait.min(at.saturating_duration_since(now));
        }
    }
    wait
}

fn publish(
    hub: &Hub,
    slots: &[Slot],
    selected: u64,
    serial: &[String],
    look: &crate::audio::AudioLook,
    audio_ready: bool,
    audio_note: &str,
) {
    let rows = slots
        .iter()
        .map(|slot| RadioRow {
            id: slot.id,
            spec: slot.spec.clone(),
            port: slot.port,
            status: slot.status.clone(),
            selected: slot.id == selected,
            make: slot.make.clone(),
            model_label: slot.model_label.clone(),
            audio_status: slot.audio_status.clone(),
        })
        .collect();
    let mut pids = Vec::new();
    for slot in slots {
        if let Some(child) = &slot.child {
            pids.push(child.id() as i32);
        }
        if let Some(child) = &slot.rx_audio {
            pids.push(child.id() as i32);
        }
        if let Some(child) = &slot.tx_audio {
            pids.push(child.id() as i32);
        }
    }
    *hub.pids.lock().expect("cat pids") = pids;
    *hub.view.lock().expect("cat view") = HubView {
        rows,
        serial: serial.to_vec(),
        audio: look.graph.clone(),
        playback: look.playback.clone(),
        capture: look.capture.clone(),
        connect_ports: look.connect_ports.clone(),
        audio_ready,
        audio_note: audio_note.to_string(),
    };
}

fn stop_slot(slot: &mut Slot) {
    let adopt = slot.port.is_some();
    slot.idle = None;
    if let Some(child) = slot.child.as_mut() {
        stop_child(child);
    }
    slot.child = None;
    release_held(slot);
    slot.port = None;
    slot.assigned = None;
    slot.probe_at = None;
    slot.handed = None;
    if adopt {
        // The process has exited, so its destructor has flushed QSettings.
        adopt_gqrx_file(slot);
    }
    stop_audio(slot);
}

fn park_until(hub: &Hub, watch: &Option<DevWatch>, wait: Duration) {
    let ms = wait.as_millis().min(i32::MAX as u128) as i32;
    let mut dev = watch.as_ref().map(|watch| libc::pollfd {
        fd: watch.fd,
        events: libc::POLLIN,
        revents: 0,
    });
    let mut wake = libc::pollfd {
        fd: hub.wake,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe {
        if let Some(dev) = dev.as_mut() {
            if hub.wake >= 0 {
                let mut both = [wake, *dev];
                libc::poll(both.as_mut_ptr(), 2, ms);
            } else {
                libc::poll(dev, 1, ms);
            }
        } else if hub.wake >= 0 {
            libc::poll(&mut wake, 1, ms);
        } else {
            thread::sleep(wait);
        }
    }
    if hub.wake >= 0 {
        let mut buf = [0u8; 16];
        unsafe {
            libc::read(hub.wake, buf.as_mut_ptr() as *mut libc::c_void, buf.len());
        }
    }
}

struct DevWatch {
    fd: RawFd,
}

impl DevWatch {
    fn open() -> Option<Self> {
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let mask = libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO;
        for path in ["/dev", "/dev/serial"] {
            let Ok(c) = std::ffi::CString::new(path) else {
                continue;
            };
            unsafe {
                libc::inotify_add_watch(fd, c.as_ptr(), mask);
            }
        }
        Some(Self { fd })
    }

    fn drain(&self) -> bool {
        let mut buf = [0u8; 4096];
        let mut saw = false;
        loop {
            let n =
                unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            saw = true;
        }
        saw
    }
}

impl Drop for DevWatch {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

fn radio_make_model(
    driver: RadioDriver,
    name: &str,
    model: i32,
    rows: &[HamlibModel],
) -> (String, String) {
    match driver {
        RadioDriver::Gqrx => ("GQRX".into(), name.trim().to_string()),
        RadioDriver::Hamlib => {
            if let Some(row) = rows.iter().find(|row| row.id == model) {
                (row.mfg.clone(), row.model.clone())
            } else {
                ("Hamlib".into(), model.to_string())
            }
        }
    }
}

fn watch_dump(hub: Arc<Hub>) -> Option<i32> {
    let mut child = Command::new("pw-dump")
        .args(["-m", "-N", "-R", "-i", "0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .ok()?;
    let pid = child.id() as i32;
    let stdout = child.stdout.take()?;
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let mut merge = crate::audio::DumpMerge::default();
        let mut last = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    // A later line is only the objects that changed. Merging
                    // keeps the radio card when PipeWire reports a port update.
                    if let Some(look) = merge.apply(&line) {
                        let key = crate::audio::look_key(&look);
                        if key != last {
                            last = key;
                            hub.send(HubCmd::AudioSnap(look));
                        }
                    }
                }
            }
        }
        let _ = child.wait();
    });
    Some(pid)
}

fn note_audio(slots: &mut [Slot], graph: &crate::audio::AudioGraph) {
    for slot in slots {
        let rx = crate::audio::rx_name(slot.id);
        let tx = crate::audio::tx_name(slot.id);
        let seen_rx = crate::audio::has_device(graph, &rx);
        let seen_tx = crate::audio::has_device(graph, &tx);
        if seen_rx {
            slot.saw_rx = true;
            slot.spec.audio.rx = true;
        } else if slot.saw_rx {
            slot.spec.audio.rx = false;
        }
        if seen_tx {
            slot.saw_tx = true;
            slot.spec.audio.tx = true;
        } else if slot.saw_tx {
            slot.spec.audio.tx = false;
        }
        if let Some(node) = crate::audio::peer_node(graph, &crate::audio::rx_stream(slot.id), true)
        {
            slot.spec.audio.output = node;
        }
        if let Some(node) = crate::audio::peer_node(graph, &crate::audio::tx_stream(slot.id), false)
        {
            slot.spec.audio.input = node;
        }
        slot.status_dirty = true;
    }
}

fn audio_settled(slots: &[Slot], graph: &crate::audio::AudioGraph) -> bool {
    slots.iter().all(|slot| {
        let rx_ok = !slot.spec.audio.rx
            || slot.saw_rx
            || crate::audio::has_device(graph, &crate::audio::rx_name(slot.id));
        let tx_ok = !slot.spec.audio.tx
            || slot.saw_tx
            || crate::audio::has_device(graph, &crate::audio::tx_name(slot.id));
        rx_ok && tx_ok
    })
}

fn service_audio(
    slot: &mut Slot,
    models: &[HamlibModel],
    wanted: &crate::audio::AudioGraph,
    live: &crate::audio::AudioGraph,
) {
    let (make, model_label) =
        radio_make_model(slot.spec.driver, &slot.spec.name, slot.spec.model, models);
    if slot.make != make || slot.model_label != model_label {
        slot.make = make;
        slot.model_label = model_label;
        slot.status_dirty = true;
    }
    reap_audio(&mut slot.rx_audio);
    reap_audio(&mut slot.tx_audio);
    if !slot.spec.audio.rx {
        if slot.rx_audio.is_some() {
            stop_audio_end(&mut slot.rx_audio);
        }
    } else if slot.rx_audio.is_none()
        && !crate::audio::has_device(live, &crate::audio::rx_name(slot.id))
    {
        let description = crate::audio::describe(&slot.make, &slot.model_label, slot.id, "RX");
        match spawn_loopback(&crate::audio::rx_loopback_args(slot.id, &description)) {
            Ok(child) => {
                slot.rx_audio = Some(child);
                slot.audio_wired = false;
                if slot.audio_status.starts_with("pw-loopback") {
                    slot.audio_status.clear();
                    slot.status_dirty = true;
                }
            }
            Err(err) => {
                if slot.audio_status != err {
                    slot.audio_status = err;
                    slot.status_dirty = true;
                }
            }
        }
    }
    if !slot.spec.audio.tx {
        if slot.tx_audio.is_some() {
            stop_audio_end(&mut slot.tx_audio);
        }
    } else if slot.tx_audio.is_none()
        && !crate::audio::has_device(live, &crate::audio::tx_name(slot.id))
    {
        let description = crate::audio::describe(&slot.make, &slot.model_label, slot.id, "TX");
        match spawn_loopback(&crate::audio::tx_loopback_args(slot.id, &description)) {
            Ok(child) => {
                slot.tx_audio = Some(child);
                slot.audio_wired = false;
            }
            Err(err) => {
                if slot.audio_status != err {
                    slot.audio_status = err;
                    slot.status_dirty = true;
                }
            }
        }
    }
    if !slot.audio_wired {
        wire_slot(slot, wanted);
    }
}

fn reap_audio(child: &mut Option<Child>) {
    let finished = match child.as_mut() {
        Some(child) => match child.try_wait() {
            Ok(Some(_)) | Err(_) => true,
            Ok(None) => false,
        },
        None => false,
    };
    if finished {
        *child = None;
    }
}

fn stop_audio_end(child: &mut Option<Child>) {
    if let Some(child) = child.as_mut() {
        stop_child(child);
    }
    *child = None;
}

fn stop_audio(slot: &mut Slot) {
    stop_audio_end(&mut slot.rx_audio);
    stop_audio_end(&mut slot.tx_audio);
}

fn spawn_loopback(args: &[String]) -> Result<Child, String> {
    let mut cmd = Command::new("pw-loopback");
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    cmd.spawn().map_err(|err| format!("pw-loopback: {err}"))
}

fn wire_slot(slot: &mut Slot, wanted: &crate::audio::AudioGraph) {
    let outputs = crate::audio::pw_ports(true).unwrap_or_default();
    let inputs = crate::audio::pw_ports(false).unwrap_or_default();
    if slot.spec.audio.rx
        && crate::audio::ports_of(&outputs, &crate::audio::rx_stream(slot.id)).is_empty()
    {
        return;
    }
    if slot.spec.audio.tx
        && crate::audio::ports_of(&inputs, &crate::audio::tx_stream(slot.id)).is_empty()
    {
        return;
    }
    let saved = wanted_links(slot.id, wanted);
    if saved.is_empty() {
        wire_default(slot, &outputs, &inputs);
    } else {
        for link in saved {
            if let Err(err) = crate::audio::pw_link(&link.output, &link.input) {
                slot.audio_status = err;
                slot.status_dirty = true;
            }
        }
    }
    slot.audio_wired = true;
    slot.status_dirty = true;
}

fn wanted_links(id: u64, graph: &crate::audio::AudioGraph) -> Vec<crate::audio::AudioLink> {
    graph
        .links
        .iter()
        .filter(|link| mentions(id, &link.output) || mentions(id, &link.input))
        .cloned()
        .collect()
}

fn mentions(id: u64, port: &str) -> bool {
    let node = crate::audio::port_node(port);
    node == crate::audio::rx_name(id)
        || node == crate::audio::tx_name(id)
        || node == crate::audio::rx_stream(id)
        || node == crate::audio::tx_stream(id)
}

fn wire_default(slot: &mut Slot, outputs: &[String], inputs: &[String]) {
    if slot.spec.audio.rx {
        let sink = if slot.spec.audio.output.trim().is_empty() {
            crate::audio::default_sink().unwrap_or_default()
        } else {
            slot.spec.audio.output.clone()
        };
        if sink.is_empty() {
            slot.audio_status = "no default output".into();
        } else {
            let from = crate::audio::ports_of(outputs, &crate::audio::rx_stream(slot.id));
            let to = crate::audio::ports_of(inputs, &sink);
            for (output, input) in crate::audio::pair_ports(&from, &to) {
                if let Err(err) = crate::audio::pw_link(&output, &input) {
                    slot.audio_status = err;
                }
            }
            slot.spec.audio.output = sink;
        }
    }
    if slot.spec.audio.tx {
        let source = if slot.spec.audio.input.trim().is_empty() {
            crate::audio::default_source().unwrap_or_default()
        } else {
            slot.spec.audio.input.clone()
        };
        if source.is_empty() {
            slot.audio_status = "no default input".into();
        } else {
            let from = crate::audio::ports_of(outputs, &source);
            let to = crate::audio::ports_of(inputs, &crate::audio::tx_stream(slot.id));
            for (output, input) in crate::audio::pair_ports(&from, &to) {
                if let Err(err) = crate::audio::pw_link(&output, &input) {
                    slot.audio_status = err;
                }
            }
            slot.spec.audio.input = source;
        }
    }
}

fn retarget(slot: &mut Slot, input: &str, output: &str, live: &crate::audio::AudioGraph) {
    let outputs = crate::audio::pw_ports(true).unwrap_or_default();
    let inputs = crate::audio::pw_ports(false).unwrap_or_default();
    if output != slot.spec.audio.output {
        unlink_stream(&crate::audio::rx_stream(slot.id), true, live);
        // An empty name is the bay dropping a wire on bare canvas.
        if !output.is_empty() {
            let from = crate::audio::ports_of(&outputs, &crate::audio::rx_stream(slot.id));
            let to = crate::audio::ports_of(&inputs, output);
            for (left, right) in crate::audio::pair_ports(&from, &to) {
                if let Err(err) = crate::audio::pw_link(&left, &right) {
                    slot.audio_status = err;
                }
            }
        }
        slot.spec.audio.output = output.to_string();
    }
    if input != slot.spec.audio.input {
        unlink_stream(&crate::audio::tx_stream(slot.id), false, live);
        if !input.is_empty() {
            let from = crate::audio::ports_of(&outputs, input);
            let to = crate::audio::ports_of(&inputs, &crate::audio::tx_stream(slot.id));
            for (left, right) in crate::audio::pair_ports(&from, &to) {
                if let Err(err) = crate::audio::pw_link(&left, &right) {
                    slot.audio_status = err;
                }
            }
        }
        slot.spec.audio.input = input.to_string();
    }
}

fn unlink_stream(stream: &str, stream_is_output: bool, graph: &crate::audio::AudioGraph) {
    for link in &graph.links {
        let ours = if stream_is_output {
            &link.output
        } else {
            &link.input
        };
        let peer = if stream_is_output {
            &link.input
        } else {
            &link.output
        };
        if crate::audio::port_node(ours) != stream {
            continue;
        }
        if crate::audio::watched_node(crate::audio::port_node(peer)) {
            continue;
        }
        let _ = crate::audio::pw_unlink(&link.output, &link.input);
    }
}

/// Saved wires whose ports both exist are put back. Returns true when every
/// saved wire was applied. A missing port keeps the caller trying.
fn restore_links(wanted: &crate::audio::AudioGraph, slots: &[Slot], note: &mut String) -> bool {
    let outputs = match crate::audio::pw_ports(true) {
        Ok(ports) => ports,
        Err(err) => {
            *note = err;
            return false;
        }
    };
    let inputs = match crate::audio::pw_ports(false) {
        Ok(ports) => ports,
        Err(err) => {
            *note = err;
            return false;
        }
    };
    let mut complete = true;
    for link in &wanted.links {
        let output_ready = outputs.iter().any(|port| port == &link.output);
        let input_ready = inputs.iter().any(|port| port == &link.input);
        if output_ready && input_ready {
            if let Err(err) = crate::audio::pw_link(&link.output, &link.input) {
                *note = err;
            }
            continue;
        }
        let waiting_on_radio = slots.iter().any(|slot| {
            (slot.spec.audio.rx && mentions(slot.id, &link.output))
                || (slot.spec.audio.tx && mentions(slot.id, &link.input))
                || (slot.spec.audio.rx && mentions(slot.id, &link.input))
                || (slot.spec.audio.tx && mentions(slot.id, &link.output))
        });
        if waiting_on_radio {
            complete = false;
        }
    }
    complete
}

fn restore_extra_devices(
    wanted: &crate::audio::AudioGraph,
    live: &crate::audio::AudioGraph,
    slots: &[Slot],
    note: &mut String,
) {
    for device in &wanted.devices {
        if !crate::audio::safe_hamspot_name(&device.name) {
            continue;
        }
        if crate::audio::has_device(live, &device.name) {
            continue;
        }
        if slots.iter().any(|slot| {
            device.name == crate::audio::rx_name(slot.id)
                || device.name == crate::audio::tx_name(slot.id)
        }) {
            continue;
        }
        if let Err(err) =
            crate::audio::create_linger(&device.name, &device.description, &device.class)
        {
            *note = err;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_carries_the_dialog_fields_and_not_vfo_mode() {
        let spec = RadioSpec {
            driver: RadioDriver::Hamlib,
            name: "IC-7300".into(),
            model: 3073,
            device: "/dev/ttyUSB0".into(),
            serial_speed: "19200".into(),
            civ: "148".into(),
            ptt_type: "RTS".into(),
            ptt_file: "/dev/ttyUSB1".into(),
            conf: "rts_state=ON".into(),
            park: false,
            follow: false,
            audio: RadioAudio::default(),
            gqrx: GqrxConfig::new(),
        };
        let args = hamlib_argv(&spec, 4534);
        assert_eq!(
            args,
            vec![
                "-m",
                "3073",
                "-T",
                "127.0.0.1",
                "-t",
                "4534",
                "-r",
                "/dev/ttyUSB0",
                "-s",
                "19200",
                "-c",
                "148",
                "-P",
                "RTS",
                "-p",
                "/dev/ttyUSB1",
                "-C",
                "rts_state=ON",
            ]
        );
        assert!(!args.iter().any(|arg| arg == "-o" || arg == "--vfo"));
    }

    #[test]
    fn empty_device_omits_the_rig_file_and_model_one_is_valid() {
        let spec = RadioSpec::dummy();
        let args = hamlib_argv(&spec, 4532);
        assert!(!args.iter().any(|arg| arg == "-r"));
        assert!(validate_spec(&spec).is_ok());
        assert_eq!(args[args.len() - 1], "4532");
    }

    #[test]
    fn port_picking_skips_a_bound_port() {
        let _hold = TcpListener::bind(("127.0.0.1", PORT_START));
        let port = free_port(PORT_START).unwrap();
        assert!(port > PORT_START);
    }

    #[test]
    fn serial_names_come_from_the_given_directory() {
        let root = std::env::temp_dir().join(format!("hamspot-serial-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let by_id = root.join("serial").join("by-id");
        fs::create_dir_all(&by_id).unwrap();
        fs::write(by_id.join("usb-radio"), b"").unwrap();
        fs::write(root.join("ttyUSB0"), b"").unwrap();
        let listed = serial_ports_in(&root);
        assert_eq!(listed.len(), 1);
        assert!(listed[0].ends_with("usb-radio"));
        fs::remove_dir_all(&by_id).unwrap();
        let listed = serial_ports_in(&root);
        assert_eq!(listed.len(), 1);
        assert!(listed[0].ends_with("ttyUSB0"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn model_list_keeps_numbered_rows() {
        let text = "\
 Rig #  Mfg                    Model                   Version         Status      Macro
     1  Hamlib                 Dummy                   20240709.0      Stable      RIG_MODEL_DUMMY
     6  Hamlib                 Dummy No VFO            20240409.0      Stable      RIG_MODEL_DUMMY_NOVFO
  3073  Icom                   IC-7300                 20250517.14     Stable      RIG_MODEL_IC7300
";
        let rows = parse_model_list(text);
        assert_eq!(rows[0].id, 1);
        assert_eq!(rows[0].mfg, "Hamlib");
        assert_eq!(rows[0].model, "Dummy");
        assert_eq!(rows[1].id, 6);
        assert_eq!(rows[1].model, "Dummy No VFO");
        assert_eq!(rows[2].id, 3073);
        assert_eq!(rows[2].mfg, "Icom");
        assert_eq!(rows[2].model, "IC-7300");
        let shown = format_model_row(&rows[0]);
        let icom = format_model_row(&rows[2]);
        assert_eq!(shown.find("Hamlib"), icom.find("Icom"));
        assert_eq!(shown.find("Dummy"), icom.find("IC-7300"));
        assert_eq!(model_button_label(&rows[2]), "3073  Icom IC-7300");
    }

    #[test]
    fn dummy_parks_vfo_a_and_the_process_ends() {
        let spec = RadioSpec::dummy();
        let port = match claim_port(PORT_START) {
            Ok(port) => port,
            Err(_) => return,
        };
        let mut child = match spawn_rigctld(&spec, port) {
            Ok(child) => child,
            Err(_) => {
                release_port(port);
                return;
            }
        };
        let ready = wait_ready(port, &mut child);
        if ready.is_err() {
            stop_child(&mut child);
            release_port(port);
            return;
        }
        {
            let mut rig = LiveRig::connect("127.0.0.1", port).expect("client");
            park_dummy(&mut rig).expect("park");
        }
        let mut rig = LiveRig::connect("127.0.0.1", port).expect("client");
        let snap = rig.poll().expect("poll");
        assert!((snap.freq_hz - 14_153_608.0).abs() < 1.0);
        drop(rig);
        stop_child(&mut child);
        release_port(port);
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err());
    }

    #[test]
    fn follow_reads_vfo_b_without_leaving_vfo_a() {
        let spec = RadioSpec::dummy();
        let port = match claim_port(PORT_START) {
            Ok(port) => port,
            Err(_) => return,
        };
        let mut child = match spawn_rigctld(&spec, port) {
            Ok(child) => child,
            Err(_) => {
                release_port(port);
                return;
            }
        };
        let ready = wait_ready(port, &mut child);
        if ready.is_err() {
            stop_child(&mut child);
            release_port(port);
            return;
        }
        let mut rig = LiveRig::connect("127.0.0.1", port).expect("client");
        rig.apply(&CatCommand::SetVfo("VFOB".into())).unwrap();
        rig.apply(&CatCommand::SetFreq(145_800_000.0)).unwrap();
        rig.apply(&CatCommand::SetMode {
            mode: "FM".into(),
            passband_hz: 15000,
        })
        .unwrap();
        rig.apply(&CatCommand::SetVfo("VFOA".into())).unwrap();
        rig.apply(&CatCommand::SetFreq(14_153_608.0)).unwrap();
        rig.apply(&CatCommand::SetMode {
            mode: "USB".into(),
            passband_hz: 2400,
        })
        .unwrap();
        rig.set_follow(true);
        let snap = rig.poll().expect("follow poll");
        assert_eq!(snap.vfo, "VFOA");
        assert!((snap.freq_hz - 14_153_608.0).abs() < 1.0);
        assert_eq!(snap.mode, "USB");
        assert_eq!(snap.sub_vfo, "VFOB");
        assert!((snap.sub_freq_hz - 145_800_000.0).abs() < 1.0);
        assert_eq!(snap.sub_mode, "FM");
        assert!(!rig.follow_current_only());
        rig.set_follow(false);
        let snap = rig.poll().expect("plain poll");
        assert_eq!(snap.vfo, "VFOA");
        assert!((snap.freq_hz - 14_153_608.0).abs() < 1.0);
        assert_eq!(snap.sub_vfo, "VFOB");
        drop(rig);
        stop_child(&mut child);
        release_port(port);
    }

    #[test]
    fn gqrx_spec_does_not_need_a_model() {
        let spec = RadioSpec {
            driver: RadioDriver::Gqrx,
            name: "SDR".into(),
            model: 0,
            device: String::new(),
            serial_speed: String::new(),
            civ: String::new(),
            ptt_type: String::new(),
            ptt_file: String::new(),
            conf: String::new(),
            park: false,
            follow: false,
            audio: RadioAudio::default(),
            gqrx: gqrx_chooser_config(),
        };
        assert!(validate_spec(&spec).is_ok());
        assert_eq!(driver_name(RadioDriver::Gqrx), "gqrx");
    }

    #[test]
    fn gqrx_conf_sets_the_port_and_keeps_other_groups() {
        let text = "[audio]\ngain=1\n\n[remote_control]\nenabled=false\nport=7356\nallowed_hosts=127.0.0.1\n";
        let out = patch_remote_control(text, 4534);
        assert!(out.contains("[audio]\n"));
        assert!(out.contains("gain=1\n"));
        assert!(out.contains("enabled=true\n"));
        assert!(out.contains("port=4534\n"));
        assert!(out.contains("allowed_hosts=127.0.0.1\n"));
        assert!(!out.contains("port=7356"));
        assert!(!out.contains("enabled=false"));

        let bare = patch_remote_control("[input]\ndevice=rtl\n", 4535);
        assert!(bare.contains("[input]\n"));
        assert!(bare.contains("device=rtl\n"));
        assert!(bare.contains("[remote_control]\n"));
        assert!(bare.contains("enabled=true\n"));
        assert!(bare.contains("port=4535\n"));
    }

    #[test]
    fn gqrx_chooser_writes_no_device_over_an_old_file() {
        let root = std::env::temp_dir().join(format!("hamspot-gqrx-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let path = root.join("radio.conf");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, "[input]\ndevice=hackrf\ncrashed=true\n").unwrap();
        let config = gqrx_chooser_config();
        assert!(gqrx_needs_chooser(&config));
        write_gqrx_native(&path, &config, 4536).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("device="));
        assert!(!text.contains("hackrf"));
        assert!(text.contains("crashed=false"));
        assert!(text.contains("enabled=true"));
        assert!(text.contains("port=4536"));
        let parsed = parse_gqrx_native(&text);
        assert!(gqrx_needs_chooser(&parsed));
        assert_eq!(
            parsed.get("remote_control").unwrap().get("port").unwrap(),
            "4536"
        );

        let mut kept = config;
        kept.entry("input".into())
            .or_default()
            .insert("frequency".into(), "14236000".into());
        kept.entry("input".into())
            .or_default()
            .insert("device".into(), "rtl=0".into());
        let text = render_gqrx_native(&kept, 4537);
        assert!(text.contains("frequency=14236000\n"));
        assert!(text.contains("device=rtl=0\n"));
        assert!(text.contains("port=4537\n"));
        assert!(!text.contains("port=4536"));
        assert!(!gqrx_needs_chooser(&parse_gqrx_native(&text)));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_gqrx_document_adopts_the_native_file_without_copying_default() {
        let root = std::env::temp_dir().join(format!("hamspot-gqrx-adopt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("config");
        let dir = home.join("hamspot-commander").join("gqrx");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SDR.conf"),
            "[General]\nconfigversion=4\ncrashed=true\n\n[input]\nfrequency=14236000\n\n[receiver]\ndemod=AM\n",
        )
        .unwrap();
        // A poisoned default.conf must not be copied into a new instance.
        fs::create_dir_all(home.join("gqrx")).unwrap();
        fs::write(
            home.join("gqrx").join("default.conf"),
            "[input]\ndevice=hackrf\n",
        )
        .unwrap();
        let mut spec = RadioSpec {
            driver: RadioDriver::Gqrx,
            name: "SDR".into(),
            model: 0,
            device: String::new(),
            serial_speed: String::new(),
            civ: String::new(),
            ptt_type: String::new(),
            ptt_file: String::new(),
            conf: String::new(),
            park: false,
            follow: false,
            audio: RadioAudio::default(),
            gqrx: GqrxConfig::new(),
        };
        let path = prepare_gqrx_document_in(&home, &mut spec).unwrap();
        assert!(gqrx_needs_chooser(&spec.gqrx));
        assert_eq!(
            spec.gqrx.get("input").unwrap().get("frequency").unwrap(),
            "14236000"
        );
        assert_eq!(
            spec.gqrx.get("General").unwrap().get("crashed").unwrap(),
            "false"
        );
        assert_eq!(
            spec.gqrx.get("receiver").unwrap().get("demod").unwrap(),
            "AM"
        );
        write_gqrx_native(&path, &spec.gqrx, 4538).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("device="));
        assert!(!text.contains("hackrf"));
        assert!(text.contains("frequency=14236000"));
        assert!(text.contains("port=4538"));
        let _ = fs::remove_dir_all(&root);
    }
}
