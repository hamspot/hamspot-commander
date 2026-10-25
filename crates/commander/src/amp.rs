//! Linear amplifiers Commander starts itself.
//!
//! Hamlib `ampctld` is the daemon, the same role `rigctld` has for a radio
//! and `rotctld` has for a rotator. This module polls frequency, power
//! status and the levels the backend lists. It never sends operate on its
//! own: `set_powerstat` goes out only when the operator clicks a key.
//!
//! `AmplifierSpec` is plain station-file data. The patch bay can reuse it
//! without the daemon code.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::fd::RawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::cat::{self, claim_port, release_port};

/// Past the rig (4532) and rotator (4533) defaults. A taken port is skipped.
const PORT_START: u16 = 4534;
const RETRY_AFTER: Duration = Duration::from_secs(2);
/// `ampctld` does not push state. The read keeps the readouts current and
/// notices a hung daemon.
const STATE_EVERY: Duration = Duration::from_secs(1);

/// `ampctld -l` on hamlib 4.6.5 uses the `rotctld -l` columns.
const COL_MFG: usize = 8;
const COL_MODEL: usize = 31;
const COL_VERSION: usize = 55;
const COL_STATUS: usize = 71;
const COL_MACRO: usize = 85;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmplifierSpec {
    /// Stable unique id (32 hex digits, random at creation, never derived
    /// from the name). Patch bay boxes bind to this. Old files without it
    /// get one on load (`repair_ids`).
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub model: i32,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub serial_speed: String,
    #[serde(default)]
    pub conf: String,
    /// Mirror the active rig's frequency to this amplifier with `set_freq`.
    /// Off unless the operator ticks it (#103).
    #[serde(default, skip_serializing_if = "is_false")]
    pub follow_rig: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

impl AmplifierSpec {
    #[cfg(test)]
    pub fn dummy() -> Self {
        Self {
            id: "0123456789abcdef0123456789abcdef".into(),
            name: "Dummy".into(),
            model: 1,
            device: String::new(),
            serial_speed: String::new(),
            conf: String::new(),
            follow_rig: false,
        }
    }
}

/// Hamlib `powerstat_t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Power {
    Off,
    On,
    Standby,
    Operate,
    Unknown,
}

impl Power {
    pub fn hamlib_code(self) -> i32 {
        match self {
            Power::Off => 0,
            Power::On => 1,
            Power::Standby => 2,
            Power::Operate => 4,
            Power::Unknown => 8,
        }
    }

    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Power::Off,
            1 => Power::On,
            2 => Power::Standby,
            4 => Power::Operate,
            _ => Power::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Power::Off => "OFF",
            Power::On => "ON",
            Power::Standby => "STBY",
            Power::Operate => "OPER",
            Power::Unknown => "?",
        }
    }
}

/// One poll of a running amplifier.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AmpState {
    pub freq_hz: Option<f64>,
    pub power: Option<Power>,
    /// Level name and value, in the order the backend lists them.
    pub levels: Vec<(String, String)>,
    pub info: String,
}

impl AmpState {
    pub fn level(&self, name: &str) -> Option<&str> {
        self.levels
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AmplifierRow {
    pub id: u64,
    pub spec: AmplifierSpec,
    pub port: Option<u16>,
    pub status: String,
    pub selected: bool,
    pub state: AmpState,
    pub make: String,
    pub model_label: String,
    /// Levels `set_level ?` lists. Empty when the backend sets none.
    pub settable: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct AmpView {
    pub rows: Vec<AmplifierRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AmpModel {
    pub id: i32,
    pub mfg: String,
    pub model: String,
    pub version: String,
    pub status: String,
    pub macro_name: String,
}

/// One-shot requests. Each is an operator action.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    SetFreq(f64),
    SetPower(Power),
    SetLevel(String, String),
    Reset,
}

/// The `ampctld` long-name command line for a control request.
pub fn control_line(cmd: &Control) -> String {
    match cmd {
        Control::SetFreq(hz) => format!("set_freq {:.0}", hz.max(0.0)),
        Control::SetPower(power) => format!("set_powerstat {}", power.hamlib_code()),
        Control::SetLevel(name, value) => format!("set_level {name} {value}"),
        // Hamlib reset_t 1 is a soft reset. The Dummy accepts it.
        Control::Reset => "reset 1".into(),
    }
}

enum AmpCmd {
    Add(AmplifierSpec),
    Update(u64, AmplifierSpec),
    Remove(u64),
    Select(u64),
    Replace(Vec<AmplifierSpec>),
    Control(u64, Control),
    Shutdown,
}

struct Slot {
    id: u64,
    spec: AmplifierSpec,
    port: Option<u16>,
    status: String,
    status_dirty: bool,
    child: Option<Child>,
    retry_at: Option<Instant>,
    next_poll: Option<Instant>,
    state: AmpState,
    level_names: Vec<String>,
    settable: Vec<String>,
    make: String,
    model_label: String,
    held: Option<u16>,
}

pub struct Hub {
    cmds: Mutex<Vec<AmpCmd>>,
    view: Mutex<AmpView>,
    pids: Mutex<Vec<i32>>,
    wake: RawFd,
    done: Mutex<bool>,
    done_wake: Condvar,
}

pub fn validate_spec(spec: &AmplifierSpec) -> Result<(), String> {
    if spec.name.trim().is_empty() {
        return Err("amplifier name is empty".into());
    }
    if spec.model <= 0 {
        return Err("amplifier model must be greater than zero".into());
    }
    if !spec.serial_speed.trim().is_empty() && spec.serial_speed.trim().parse::<u32>().is_err() {
        return Err("serial speed is not a number".into());
    }
    Ok(())
}

/// `ampctld` argv. Loopback only.
pub fn ampctld_argv(spec: &AmplifierSpec, port: u16) -> Vec<String> {
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

pub fn parse_model_list(text: &str) -> Vec<AmpModel> {
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
        rows.push(AmpModel {
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
    if start >= end || !line.is_char_boundary(start) || !line.is_char_boundary(end) {
        return String::new();
    }
    line[start..end].trim().to_string()
}

/// Hamlib 4.6.5 `ampctl -l`, used when the binary is missing or prints nothing.
pub fn static_models() -> Vec<AmpModel> {
    [
        (1, "Hamlib", "Dummy", "20200112.0", "Stable", "AMP_MODEL_DUMMY"),
        (2, "Hamlib", "NET ampctl", "20200112.0", "Stable", "AMP_MODEL_NETAMPCTL"),
        (201, "Elecraft", "KPA1500", "20220710.0", "Beta", "AMP_MODEL_ELECRAFT_KPA1500"),
        (301, "Gemini", "DX1200/HF-1K", "20230318.0", "Stable", "AMP_MODEL_GEMINI_DX1200"),
        (401, "Expert", "1.3K-FA/1.5K-FA/2K-FA", "20240709.0", "Beta", "AMP_MODEL_EXPERT_FA"),
    ]
    .into_iter()
    .map(|(id, mfg, model, version, status, macro_name)| AmpModel {
        id,
        mfg: mfg.into(),
        model: model.into(),
        version: version.into(),
        status: status.into(),
        macro_name: macro_name.into(),
    })
    .collect()
}

pub fn format_model_row(row: &AmpModel) -> String {
    format!(
        "{:<7}{:<22}{:<24}{:<16}{:<12}{}",
        row.id, row.mfg, row.model, row.version, row.status, row.macro_name
    )
}

pub fn model_menu_header() -> String {
    format!(
        "{:<7}{:<22}{:<24}{:<16}{:<12}{}",
        "Amp #", "Mfg", "Model", "Version", "Status", "Macro"
    )
}

pub fn model_button_label(row: &AmpModel) -> String {
    match (row.mfg.is_empty(), row.model.is_empty()) {
        (true, true) => row.id.to_string(),
        (true, false) => format!("{}  {}", row.id, row.model),
        (false, true) => format!("{}  {}", row.id, row.mfg),
        (false, false) => format!("{}  {} {}", row.id, row.mfg, row.model),
    }
}

/// `ampctld -l`, or the static list when that fails.
pub fn model_list() -> Vec<AmpModel> {
    let parsed = Command::new("ampctld")
        .arg("-l")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| parse_model_list(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_default();
    if parsed.is_empty() {
        static_models()
    } else {
        parsed
    }
}

pub fn spawn_ampctld(spec: &AmplifierSpec, port: u16) -> Result<Child, String> {
    validate_spec(spec)?;
    let mut cmd = Command::new("ampctld");
    cmd.args(ampctld_argv(spec, port))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    cmd.spawn().map_err(|err| format!("ampctld: {err}"))
}

fn wait_ready(port: u16, child: &mut Child) -> Result<(), String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(format!(
                    "ampctld exited before it accepted a connection ({status})"
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

/// The value lines of an extended reply, without the `cmd:` echo line.
fn values(lines: &[String], cmd: &str) -> Vec<String> {
    let echo = format!("{cmd}:");
    lines
        .iter()
        .filter(|line| !line.starts_with(&echo))
        .cloned()
        .collect()
}

fn labeled(lines: &[String], key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    lines
        .iter()
        .find_map(|line| line.trim().strip_prefix(&prefix).map(|v| v.trim().to_string()))
}

/// `get_level ?` answers one line of space-separated names.
pub fn parse_level_names(lines: &[String]) -> Vec<String> {
    values(lines, "get_level")
        .iter()
        .flat_map(|line| line.split_whitespace())
        .filter(|name| *name != "?")
        .map(str::to_string)
        .collect()
}

/// `set_level ?` answers the same way. The Dummy lists none.
pub fn parse_set_level_names(lines: &[String]) -> Vec<String> {
    values(lines, "set_level")
        .iter()
        .flat_map(|line| line.split_whitespace())
        .filter(|name| *name != "?")
        .map(str::to_string)
        .collect()
}

/// A fresh 128-bit random id as 32 lowercase hex digits.
pub fn new_id() -> String {
    let mut buf = [0u8; 16];
    let ok = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).is_ok();
    if !ok {
        // Fallback: time and a counter through the std hasher. Still unique
        // per process run; /dev/urandom is always there on Linux.
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        for half in buf.chunks_mut(8) {
            let mut h = RandomState::new().build_hasher();
            h.write_u64(N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
            half.copy_from_slice(&h.finish().to_le_bytes());
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Give every amplifier row a unique id. Rows with no id get one; a row
/// whose id repeats an earlier row's gets a new one. Returns the ids that
/// were duplicated, so bindings to them can be dropped.
pub fn repair_ids(specs: &mut [AmplifierSpec]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut dup = Vec::new();
    for spec in specs.iter_mut() {
        let id = spec.id.trim().to_string();
        if id.is_empty() {
            spec.id = new_id();
        } else if !seen.insert(id.clone()) {
            if !dup.contains(&id) {
                dup.push(id);
            }
            spec.id = new_id();
        }
        seen.insert(spec.id.clone());
    }
    dup
}

/// Displayed level step 1..=10 to Hamlib's 0.0..=1.0 (10 is 1.0).
pub fn level_from_step(n: u8) -> f64 {
    f64::from(n.clamp(1, 10)) / 10.0
}

/// Hamlib 0.0..=1.0 to the displayed step, rounded and held to 1..=10.
pub fn step_from_level(value: f64) -> u8 {
    if !value.is_finite() {
        return 1;
    }
    (value * 10.0).round().clamp(1.0, 10.0) as u8
}

/// A typed frequency in MHz, as Hz. Blank, zero, negative or junk is None.
pub fn parse_mhz(text: &str) -> Option<f64> {
    let mhz: f64 = text.trim().trim_end_matches("MHz").trim().parse().ok()?;
    (mhz.is_finite() && mhz > 0.0 && mhz < 1.0e6).then(|| (mhz * 1.0e6).round())
}

/// Wait this long on a steady rig frequency before mirroring it, so a
/// spinning VFO knob does not flood the amplifier with `set_freq`.
pub const FOLLOW_SETTLE: Duration = Duration::from_millis(400);

/// Follow-rig state for one amplifier (#103). Pure: the caller sends what
/// `step` returns.
#[derive(Clone, Debug, Default)]
pub struct FollowRig {
    sent: Option<f64>,
    pending: Option<(f64, Instant)>,
}

impl FollowRig {
    /// `rig_hz` is the active rig's frequency, None when no rig is online.
    /// Returns a frequency to send, at most once per settled change.
    pub fn step(&mut self, enabled: bool, rig_hz: Option<f64>, now: Instant) -> Option<f64> {
        let Some(hz) = rig_hz.filter(|hz| enabled && *hz > 0.0) else {
            self.pending = None;
            if !enabled {
                // Ticking it again sends the current frequency once.
                self.sent = None;
            }
            return None;
        };
        let hz = hz.round();
        if self.sent.is_some_and(|s| (s - hz).abs() < 1.0) {
            self.pending = None;
            return None;
        }
        match self.pending {
            Some((p, since)) if (p - hz).abs() < 1.0 => {
                if now.duration_since(since) >= FOLLOW_SETTLE {
                    self.pending = None;
                    self.sent = Some(hz);
                    return Some(hz);
                }
                None
            }
            _ => {
                self.pending = Some((hz, now));
                None
            }
        }
    }
}

/// A level value as shown: floats trimmed to two places.
pub fn format_level(raw: &str) -> String {
    match raw.trim().parse::<f64>() {
        Ok(v) if v.fract() == 0.0 => format!("{v:.0}"),
        Ok(v) => format!("{v:.2}"),
        Err(_) => raw.trim().to_string(),
    }
}

struct AmpClient {
    stream: TcpStream,
}

impl AmpClient {
    fn open(port: u16) -> Result<Self, String> {
        let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|err| err.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|err| err.to_string())?;
        stream.set_nodelay(true).map_err(|err| err.to_string())?;
        Ok(Self { stream })
    }

    fn ask(&mut self, body: &str) -> Result<Vec<String>, String> {
        let (rprt, lines) = transact(&mut self.stream, body)?;
        if rprt != 0 {
            return Err(format!("{body}: RPRT {rprt}"));
        }
        Ok(lines)
    }

    fn control(&mut self, cmd: &Control) -> Result<(), String> {
        self.ask(&control_line(cmd)).map(|_| ())
    }

    fn info(&mut self) -> Result<String, String> {
        let lines = self.ask("get_info")?;
        Ok(labeled(&lines, "Info").unwrap_or_default())
    }

    fn level_names(&mut self) -> Result<Vec<String>, String> {
        Ok(parse_level_names(&self.ask("get_level ?")?))
    }

    fn settable_levels(&mut self) -> Result<Vec<String>, String> {
        Ok(parse_set_level_names(&self.ask("set_level ?")?))
    }

    fn freq(&mut self) -> Result<f64, String> {
        let lines = self.ask("get_freq")?;
        labeled(&lines, "Frequency(Hz)")
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| "get_freq had no frequency".into())
    }

    fn power(&mut self) -> Result<Power, String> {
        let lines = self.ask("get_powerstat")?;
        labeled(&lines, "Power Status")
            .and_then(|v| v.parse().ok())
            .map(Power::from_code)
            .ok_or_else(|| "get_powerstat had no status".into())
    }

    fn level(&mut self, name: &str) -> Result<String, String> {
        let lines = self.ask(&format!("get_level {name}"))?;
        values(&lines, "get_level")
            .into_iter()
            .next()
            .ok_or_else(|| format!("get_level {name} had no value"))
    }

    /// One poll. Freq, power and levels a backend refuses are left empty.
    fn state(&mut self, names: &[String], info: &str) -> Result<AmpState, String> {
        let freq_hz = match self.freq() {
            Ok(hz) => Some(hz),
            Err(err) if err.contains("RPRT") => None,
            Err(err) => return Err(err),
        };
        let power = self.power().ok();
        let mut levels = Vec::new();
        for name in names {
            if let Ok(value) = self.level(name) {
                levels.push((name.clone(), value));
            }
        }
        Ok(AmpState { freq_hz, power, levels, info: info.to_string() })
    }
}

fn transact(stream: &mut TcpStream, body: &str) -> Result<(i32, Vec<String>), String> {
    stream
        .write_all(format!("+\\{body}\n").as_bytes())
        .map_err(|err| err.to_string())?;
    let mut acc = Vec::new();
    let mut lines = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        while let Some(pos) = acc.iter().position(|byte| *byte == b'\n') {
            let line = String::from_utf8_lossy(&acc[..pos]).trim().to_string();
            acc.drain(..=pos);
            if let Some(code) = line.strip_prefix("RPRT") {
                let rprt = code.trim().parse::<i32>().unwrap_or(-1);
                return Ok((rprt, lines));
            }
            if !line.is_empty() {
                lines.push(line);
            }
        }
        if Instant::now() >= deadline {
            return Err("ampctld did not answer".into());
        }
        let mut buf = [0u8; 4096];
        match stream.read(&mut buf) {
            Ok(0) => return Err("ampctld closed the connection".into()),
            Ok(n) => acc.extend_from_slice(&buf[..n]),
            Err(err)
                if err.kind() == std::io::ErrorKind::TimedOut
                    || err.kind() == std::io::ErrorKind::WouldBlock =>
            {
                return Err("ampctld did not answer".into());
            }
            Err(err) => return Err(err.to_string()),
        }
    }
}

impl Hub {
    pub fn spawn(ctx: egui::Context) -> Arc<Self> {
        let wake = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        let hub = Arc::new(Self {
            cmds: Mutex::new(Vec::new()),
            view: Mutex::new(AmpView::default()),
            pids: Mutex::new(Vec::new()),
            wake,
            done: Mutex::new(false),
            done_wake: Condvar::new(),
        });
        let thread_hub = Arc::clone(&hub);
        thread::spawn(move || supervise(thread_hub, ctx));
        hub
    }

    pub fn view(&self) -> AmpView {
        self.view.lock().expect("amp view").clone()
    }

    pub fn add(&self, spec: AmplifierSpec) {
        self.send(AmpCmd::Add(spec));
    }

    pub fn update(&self, id: u64, spec: AmplifierSpec) {
        self.send(AmpCmd::Update(id, spec));
    }

    pub fn remove(&self, id: u64) {
        self.send(AmpCmd::Remove(id));
    }

    pub fn select(&self, id: u64) {
        self.send(AmpCmd::Select(id));
    }

    pub fn replace(&self, specs: Vec<AmplifierSpec>) {
        self.send(AmpCmd::Replace(specs));
    }

    /// Operator action only. Nothing in Commander calls this on a timer or a load.
    pub fn control(&self, id: u64, cmd: Control) {
        self.send(AmpCmd::Control(id, cmd));
    }

    /// Stop every `ampctld` this program started. Bounded like the rotator hub.
    pub fn shutdown(&self) {
        self.send(AmpCmd::Shutdown);
        let guard = self.done.lock().expect("amp done");
        let (guard, _) = self
            .done_wake
            .wait_timeout(guard, Duration::from_secs(8))
            .expect("amp done");
        if *guard {
            return;
        }
        drop(guard);
        let pids = self.pids.lock().expect("amp pids").clone();
        for pid in pids {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
        }
    }

    fn send(&self, cmd: AmpCmd) {
        self.cmds.lock().expect("amp cmds").push(cmd);
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

fn supervise(hub: Arc<Hub>, ctx: egui::Context) {
    let models = model_list();
    let mut slots: Vec<Slot> = Vec::new();
    let mut next_id = 1u64;
    let mut selected = 0u64;
    publish(&hub, &slots, selected);
    ctx.request_repaint();
    loop {
        let cmds = std::mem::take(&mut *hub.cmds.lock().expect("amp cmds"));
        let changed = !cmds.is_empty();
        let mut stop = false;
        for cmd in cmds {
            match cmd {
                AmpCmd::Shutdown => stop = true,
                AmpCmd::Add(spec) => {
                    if validate_spec(&spec).is_err() {
                        continue;
                    }
                    let id = next_id;
                    next_id += 1;
                    slots.push(new_slot(id, spec, &models));
                    selected = id;
                }
                AmpCmd::Update(id, spec) => {
                    if validate_spec(&spec).is_err() {
                        continue;
                    }
                    if let Some(slot) = slots.iter_mut().find(|slot| slot.id == id) {
                        stop_slot(slot);
                        slot.spec = spec;
                        label_slot(slot, &models);
                        slot.status = "starting".into();
                        slot.status_dirty = true;
                    }
                }
                AmpCmd::Remove(id) => {
                    if let Some(pos) = slots.iter().position(|slot| slot.id == id) {
                        let mut slot = slots.remove(pos);
                        stop_slot(&mut slot);
                        if selected == id {
                            selected = slots.first().map(|slot| slot.id).unwrap_or(0);
                        }
                    }
                }
                AmpCmd::Select(id) => {
                    if slots.iter().any(|slot| slot.id == id) {
                        selected = id;
                    }
                }
                AmpCmd::Replace(specs) => {
                    for slot in &mut slots {
                        stop_slot(slot);
                    }
                    slots.clear();
                    selected = 0;
                    for spec in specs {
                        if validate_spec(&spec).is_err() {
                            continue;
                        }
                        let id = next_id;
                        next_id += 1;
                        if selected == 0 {
                            selected = id;
                        }
                        slots.push(new_slot(id, spec, &models));
                    }
                }
                AmpCmd::Control(id, cmd) => control_slot(&mut slots, id, &cmd),
            }
        }
        if stop {
            for slot in &mut slots {
                stop_slot(slot);
            }
            publish(&hub, &slots, selected);
            break;
        }
        let now = Instant::now();
        for slot in &mut slots {
            tick_slot(slot, now);
        }
        if changed || slots.iter().any(|slot| slot.status_dirty) {
            publish(&hub, &slots, selected);
            for slot in &mut slots {
                slot.status_dirty = false;
            }
            ctx.request_repaint();
        }
        park(&hub, next_wait(&slots, Instant::now()));
    }
    *hub.done.lock().expect("amp done") = true;
    hub.done_wake.notify_all();
}

fn control_slot(slots: &mut [Slot], id: u64, cmd: &Control) {
    let Some(slot) = slots.iter_mut().find(|slot| slot.id == id) else {
        return;
    };
    let Some(port) = slot.port else {
        return;
    };
    if let Err(err) = AmpClient::open(port).and_then(|mut client| client.control(cmd)) {
        slot.status = err;
        slot.status_dirty = true;
    }
    slot.next_poll = Some(Instant::now());
}

fn new_slot(id: u64, spec: AmplifierSpec, models: &[AmpModel]) -> Slot {
    let mut slot = Slot {
        id,
        spec,
        port: None,
        status: "starting".into(),
        status_dirty: true,
        child: None,
        retry_at: None,
        next_poll: None,
        state: AmpState::default(),
        level_names: Vec::new(),
        settable: Vec::new(),
        make: String::new(),
        model_label: String::new(),
        held: None,
    };
    label_slot(&mut slot, models);
    slot
}

fn label_slot(slot: &mut Slot, models: &[AmpModel]) {
    if let Some(row) = models.iter().find(|row| row.id == slot.spec.model) {
        slot.make = row.mfg.clone();
        slot.model_label = row.model.clone();
    } else {
        slot.make.clear();
        slot.model_label.clear();
    }
}

fn drop_child(slot: &mut Slot) {
    slot.child = None;
    if let Some(port) = slot.held.take() {
        release_port(port);
    }
    slot.port = None;
    slot.next_poll = None;
    slot.state = AmpState::default();
}

fn tick_slot(slot: &mut Slot, now: Instant) {
    if let Some(child) = slot.child.as_mut() {
        match child.try_wait() {
            Ok(Some(status)) => {
                drop_child(slot);
                slot.status = format!("retrying ({status})");
                slot.status_dirty = true;
                slot.retry_at = Some(now + RETRY_AFTER);
            }
            Ok(None) => {}
            Err(_) => drop_child(slot),
        }
    }
    let device = slot.spec.device.trim().to_string();
    if !device.is_empty() && !Path::new(&device).exists() {
        if let Some(child) = slot.child.as_mut() {
            cat::stop_child(child);
        }
        drop_child(slot);
        if slot.status != "device missing" {
            slot.status = "device missing".into();
            slot.status_dirty = true;
        }
        // The node can come back. Look again on the next retry tick.
        slot.retry_at = Some(now + RETRY_AFTER);
        return;
    }
    if slot.child.is_none() {
        if slot.retry_at.is_some_and(|at| now < at) {
            return;
        }
        match claim_port(PORT_START).and_then(|port| {
            spawn_ampctld(&slot.spec, port)
                .map(|child| (port, child))
                .map_err(|err| {
                    release_port(port);
                    err
                })
        }) {
            Ok((port, mut child)) => match wait_ready(port, &mut child) {
                Ok(()) => {
                    slot.held = Some(port);
                    slot.child = Some(child);
                    slot.port = Some(port);
                    slot.retry_at = None;
                    slot.next_poll = Some(now);
                    let mut info = String::new();
                    if let Ok(mut client) = AmpClient::open(port) {
                        slot.level_names = client.level_names().unwrap_or_default();
                        slot.settable = client.settable_levels().unwrap_or_default();
                        info = client.info().unwrap_or_default();
                    }
                    slot.state.info = info;
                    slot.status = format!("up  127.0.0.1:{port}");
                    slot.status_dirty = true;
                }
                Err(err) => {
                    release_port(port);
                    cat::stop_child(&mut child);
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
    let Some(port) = slot.port else {
        return;
    };
    if slot.next_poll.is_some_and(|at| now < at) {
        return;
    }
    slot.next_poll = Some(now + STATE_EVERY);
    let info = slot.state.info.clone();
    match AmpClient::open(port).and_then(|mut client| client.state(&slot.level_names, &info)) {
        Ok(state) => {
            if state != slot.state {
                slot.state = state;
                slot.status_dirty = true;
            }
            let up = format!("up  127.0.0.1:{port}");
            if slot.status != up {
                slot.status = up;
                slot.status_dirty = true;
            }
        }
        Err(err) => {
            slot.status = err;
            slot.status_dirty = true;
        }
    }
}

fn stop_slot(slot: &mut Slot) {
    if let Some(child) = slot.child.as_mut() {
        cat::stop_child(child);
    }
    drop_child(slot);
}

fn next_wait(slots: &[Slot], now: Instant) -> Duration {
    let mut wait = STATE_EVERY;
    for slot in slots {
        for at in [slot.retry_at, slot.next_poll].into_iter().flatten() {
            if at <= now {
                return Duration::from_millis(1);
            }
            wait = wait.min(at.saturating_duration_since(now));
        }
    }
    wait
}

fn publish(hub: &Hub, slots: &[Slot], selected: u64) {
    let rows = slots
        .iter()
        .map(|slot| AmplifierRow {
            id: slot.id,
            spec: slot.spec.clone(),
            port: slot.port,
            status: slot.status.clone(),
            selected: slot.id == selected,
            state: slot.state.clone(),
            make: slot.make.clone(),
            model_label: slot.model_label.clone(),
            settable: slot.settable.clone(),
        })
        .collect();
    let pids = slots
        .iter()
        .filter_map(|slot| slot.child.as_ref().map(|child| child.id() as i32))
        .collect();
    *hub.pids.lock().expect("amp pids") = pids;
    *hub.view.lock().expect("amp view") = AmpView { rows };
}

fn park(hub: &Hub, wait: Duration) {
    if hub.wake < 0 {
        thread::sleep(wait);
        return;
    }
    let ms = wait.as_millis().min(i32::MAX as u128) as i32;
    let mut fd = libc::pollfd { fd: hub.wake, events: libc::POLLIN, revents: 0 };
    let mut buf = [0u8; 16];
    unsafe {
        libc::poll(&mut fd, 1, ms);
        libc::read(hub.wake, buf.as_mut_ptr() as *mut libc::c_void, buf.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn argv_is_loopback_and_omits_empty_options() {
        let spec = AmplifierSpec {
            id: new_id(),
            name: "KPA".into(),
            model: 201,
            device: "/dev/ttyUSB1".into(),
            serial_speed: "38400".into(),
            conf: "retry=2".into(),
            follow_rig: false,
        };
        assert_eq!(
            ampctld_argv(&spec, 4534),
            vec![
                "-m", "201", "-T", "127.0.0.1", "-t", "4534", "-r", "/dev/ttyUSB1", "-s",
                "38400", "-C", "retry=2",
            ]
        );
        let bare = AmplifierSpec::dummy();
        assert!(!ampctld_argv(&bare, 4535).iter().any(|arg| arg == "-r" || arg == "-s"));
        assert!(validate_spec(&bare).is_ok());
        assert!(validate_spec(&AmplifierSpec { model: 0, ..bare.clone() }).is_err());
        assert!(validate_spec(&AmplifierSpec { serial_speed: "fast".into(), ..bare }).is_err());
    }

    #[test]
    fn ampctl_list_parses_and_static_list_matches_4_6_5() {
        let text = "\
 Amp #  Mfg                    Model                   Version         Status        Macro
     1  Hamlib                 Dummy                   20200112.0      Stable        AMP_MODEL_DUMMY
     2  Hamlib                 NET ampctl              20200112.0      Stable        AMP_MODEL_NETAMPCTL
   201  Elecraft               KPA1500                 20220710.0      Beta          AMP_MODEL_ELECRAFT_KPA1500
   301  Gemini                 DX1200/HF-1K            20230318.0      Stable        AMP_MODEL_GEMINI_DX1200
   401  Expert                 1.3K-FA/1.5K-FA/2K-FA   20240709.0      Beta          AMP_MODEL_EXPERT_FA
";
        let rows = parse_model_list(text);
        assert_eq!(rows, static_models());
        assert_eq!(model_button_label(&rows[2]), "201  Elecraft KPA1500");
        assert_eq!(format_model_row(&rows[0]).find("Hamlib"), model_menu_header().find("Mfg"));
    }

    #[test]
    fn control_lines_and_powerstat_codes() {
        assert_eq!(control_line(&Control::SetFreq(14_200_000.0)), "set_freq 14200000");
        assert_eq!(control_line(&Control::SetPower(Power::Standby)), "set_powerstat 2");
        assert_eq!(control_line(&Control::SetPower(Power::Operate)), "set_powerstat 4");
        assert_eq!(control_line(&Control::SetPower(Power::Off)), "set_powerstat 0");
        assert_eq!(control_line(&Control::Reset), "reset 1");
        assert_eq!(
            control_line(&Control::SetLevel("NH".into(), "1".into())),
            "set_level NH 1"
        );
        for power in [Power::Off, Power::On, Power::Standby, Power::Operate, Power::Unknown] {
            assert_eq!(Power::from_code(power.hamlib_code()), power);
        }
        assert_eq!(Power::from_code(3), Power::Unknown);
    }

    #[test]
    fn replies_parse_without_the_echo_line() {
        let names = parse_level_names(&lines(&[
            "get_level: ?",
            "SWR NH PF PWRINPUT PWRFORWARD PWRREFLECTED PWRPEAK FAULT",
        ]));
        assert_eq!(names.len(), 8);
        assert_eq!(names[0], "SWR");
        assert_eq!(values(&lines(&["get_level: SWR", "1.000000"]), "get_level"), ["1.000000"]);
        assert_eq!(
            labeled(&lines(&["get_freq:", "Frequency(Hz): 14200000"]), "Frequency(Hz)"),
            Some("14200000".into())
        );
        assert_eq!(format_level("1.000000"), "1");
        assert_eq!(format_level("1.250000"), "1.25");
        assert_eq!(format_level("SWR HIGH"), "SWR HIGH");
    }

    /// The Dummy only. No test talks to a real amplifier.
    #[test]
    fn dummy_ampctld_answers_and_the_process_ends() {
        let spec = AmplifierSpec::dummy();
        let Ok(port) = claim_port(PORT_START) else {
            return;
        };
        let mut child = match spawn_ampctld(&spec, port) {
            Ok(child) => child,
            Err(_) => {
                release_port(port);
                return;
            }
        };
        if wait_ready(port, &mut child).is_err() {
            cat::stop_child(&mut child);
            release_port(port);
            return;
        }
        let mut client = AmpClient::open(port).expect("client");
        assert_eq!(client.info().expect("get_info"), "Dummy amplifier");
        let names = client.level_names().expect("get_level ?");
        // The Dummy's levels are read-only meters; set_level ? lists none
        // and a refused set_level comes back as an error, not a panic.
        assert!(client.settable_levels().expect("set_level ?").is_empty());
        assert!(client.control(&Control::SetLevel("SWR".into(), "1.5".into())).is_err());
        assert!(names.iter().any(|name| name == "SWR"));
        // A fresh Dummy is off. Nothing here was sent on its own.
        assert_eq!(client.power().expect("get_powerstat"), Power::Off);
        assert!(client.control(&Control::SetFreq(14_200_000.0)).is_ok());
        assert_eq!(client.freq().expect("get_freq"), 14_200_000.0);
        assert!(client.control(&Control::SetPower(Power::Standby)).is_ok());
        assert_eq!(client.power().unwrap(), Power::Standby);
        assert!(client.control(&Control::SetPower(Power::Operate)).is_ok());
        assert_eq!(client.power().unwrap(), Power::Operate);
        assert!(client.control(&Control::Reset).is_ok());
        let state = client.state(&names, "Dummy amplifier").expect("state");
        assert_eq!(state.freq_hz, Some(14_200_000.0));
        assert!(state.level("SWR").is_some());
        drop(client);
        cat::stop_child(&mut child);
        release_port(port);
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err());
    }

    #[test]
    fn level_steps_convert_both_ways() {
        assert_eq!(level_from_step(10), 1.0);
        assert_eq!(level_from_step(1), 0.1);
        assert_eq!(level_from_step(5), 0.5);
        assert_eq!(level_from_step(0), 0.1);
        assert_eq!(level_from_step(42), 1.0);
        assert_eq!(step_from_level(1.0), 10);
        assert_eq!(step_from_level(0.0), 1);
        assert_eq!(step_from_level(0.34), 3);
        assert_eq!(step_from_level(0.35), 4);
        assert_eq!(step_from_level(2.0), 10);
        assert_eq!(step_from_level(-1.0), 1);
        assert_eq!(step_from_level(f64::NAN), 1);
        for n in 1..=10 {
            assert_eq!(step_from_level(level_from_step(n)), n);
        }
    }

    #[test]
    fn ids_are_random_and_repaired() {
        let a = new_id();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, new_id());
        let mut specs = vec![
            AmplifierSpec { id: String::new(), ..AmplifierSpec::dummy() },
            AmplifierSpec { id: "x".into(), ..AmplifierSpec::dummy() },
            AmplifierSpec { id: "x".into(), name: "B".into(), ..AmplifierSpec::dummy() },
        ];
        assert_eq!(repair_ids(&mut specs), ["x"]);
        assert_eq!(specs[0].id.len(), 32);
        assert_eq!(specs[1].id, "x", "the first holder keeps its id");
        assert_ne!(specs[2].id, "x", "the later row is regenerated");
        let mut again = specs.clone();
        assert!(repair_ids(&mut again).is_empty());
        assert_eq!(again, specs);
        // Old file without an id loads.
        let old: AmplifierSpec = serde_json::from_str(r#"{"name":"A","model":1}"#).unwrap();
        assert!(old.id.is_empty());
    }

    #[test]
    fn set_level_list_and_mhz_parse() {
        assert!(parse_set_level_names(&lines(&["set_level: ?", ""])).is_empty());
        assert_eq!(parse_set_level_names(&lines(&["set_level: ?", "PWR NH"])), ["PWR", "NH"]);
        assert_eq!(parse_mhz("14.2"), Some(14_200_000.0));
        assert_eq!(parse_mhz(" 144.390 MHz"), Some(144_390_000.0));
        assert_eq!(parse_mhz(""), None);
        assert_eq!(parse_mhz("0"), None);
        assert_eq!(parse_mhz("-7"), None);
        assert_eq!(parse_mhz("abc"), None);
    }

    #[test]
    fn follow_rig_waits_for_a_settled_frequency_and_never_sends_when_off() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut f = FollowRig::default();
        // Off: nothing, however long the rig sits still.
        for n in [0, 500, 5000] {
            assert_eq!(f.step(false, Some(14_200_000.0), ms(n)), None);
        }
        // On: first sight starts the settle timer.
        assert_eq!(f.step(true, Some(14_200_000.0), ms(6000)), None);
        assert_eq!(f.step(true, Some(14_200_000.0), ms(6200)), None);
        assert_eq!(f.step(true, Some(14_200_000.0), ms(6400)), Some(14_200_000.0));
        // Same frequency: not sent again.
        assert_eq!(f.step(true, Some(14_200_000.0), ms(9000)), None);
        // Spinning the knob restarts the timer each step.
        assert_eq!(f.step(true, Some(14_201_000.0), ms(9100)), None);
        assert_eq!(f.step(true, Some(14_202_000.0), ms(9300)), None);
        assert_eq!(f.step(true, Some(14_203_000.0), ms(9500)), None);
        assert_eq!(f.step(true, Some(14_203_000.0), ms(9900)), Some(14_203_000.0));
        // Rig offline: nothing.
        assert_eq!(f.step(true, None, ms(20000)), None);
        // Turned off then on: the current frequency goes once after settling.
        assert_eq!(f.step(false, Some(14_203_000.0), ms(21000)), None);
        assert_eq!(f.step(true, Some(14_203_000.0), ms(21100)), None);
        assert_eq!(f.step(true, Some(14_203_000.0), ms(21600)), Some(14_203_000.0));
    }

    #[test]
    fn follow_rig_is_left_out_of_the_file_when_off() {
        let text = serde_json::to_string(&AmplifierSpec::dummy()).unwrap();
        assert!(!text.contains("follow_rig"));
        let old: AmplifierSpec = serde_json::from_str(r#"{"name":"A","model":1}"#).unwrap();
        assert!(!old.follow_rig);
        let on = AmplifierSpec { follow_rig: true, ..AmplifierSpec::dummy() };
        let back: AmplifierSpec = serde_json::from_str(&serde_json::to_string(&on).unwrap()).unwrap();
        assert!(back.follow_rig);
    }
}
