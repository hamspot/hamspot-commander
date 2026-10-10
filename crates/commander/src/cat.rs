//! Radios the face can keep open. Hamlib is the first driver. A later driver
//! is another variant of [`RadioDriver`], not a second manager.
//!
//! The program starts `rigctld` itself. It does not attach to a daemon it
//! did not spawn. The listen address is always 127.0.0.1, and the argv never
//! includes `-o`.

use std::fs;
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
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HubView {
    pub rows: Vec<RadioRow>,
    pub serial: Vec<String>,
}

enum HubCmd {
    Add(RadioSpec),
    Update(u64, RadioSpec),
    Remove(u64),
    Select(u64),
    Replace(Vec<RadioSpec>),
    /// Stop every child this program started and leave the supervisor loop.
    Shutdown,
}

pub struct Hub {
    cmds: Mutex<Vec<HubCmd>>,
    view: Mutex<HubView>,
    /// Process groups currently running. Shutdown signals these if the
    /// supervisor has not finished stopping them itself.
    pids: Mutex<Vec<i32>>,
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

/// First port at or above `start` that this process can bind, up to 64 tries.
/// The listener is dropped before the function returns so `rigctld` can bind it.
pub fn free_port(start: u16) -> Result<u16, String> {
    let last = start.saturating_add(PORT_TRIES - 1);
    for port in start..=last {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err(format!("no free TCP port from {start} through {last}"))
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

pub fn prepare_gqrx_conf(path: &Path, port: u16, default_conf: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    if !path.is_file() {
        if default_conf.is_file() {
            fs::copy(default_conf, path).map_err(|err| {
                format!(
                    "copy {} to {}: {err}",
                    default_conf.display(),
                    path.display()
                )
            })?;
        } else {
            fs::write(path, "").map_err(|err| format!("{}: {err}", path.display()))?;
        }
    }
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let patched = patch_remote_control(&text, port);
    fs::write(path, patched).map_err(|err| format!("{}: {err}", path.display()))
}

pub fn spawn_gqrx(spec: &RadioSpec, port: u16) -> Result<Child, String> {
    validate_spec(spec)?;
    let home = config_home();
    let path = gqrx_conf_path(&home, &spec.name);
    let default_conf = home.join("gqrx").join("default.conf");
    prepare_gqrx_conf(&path, port, &default_conf)?;
    let mut cmd = Command::new("gqrx");
    cmd.arg("-c")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    cmd.spawn().map_err(|err| format!("gqrx: {err}"))
}

fn spawn_radio(spec: &RadioSpec, port: u16) -> Result<Child, String> {
    match spec.driver {
        RadioDriver::Hamlib => spawn_rigctld(spec, port),
        RadioDriver::Gqrx => spawn_gqrx(spec, port),
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
    let mut slots = vec![new_slot(1, RadioSpec::dummy())];
    let mut next_id = 2u64;
    let mut selected = 1u64;
    let mut serial = serial_ports_in(Path::new("/dev"));
    let watch = DevWatch::open();
    publish(&hub, &slots, selected, &serial);
    ctx.request_repaint();
    loop {
        let cmds = std::mem::take(&mut *hub.cmds.lock().expect("cat cmds"));
        let mut changed = !cmds.is_empty();
        let mut stop = false;
        for cmd in cmds {
            changed = true;
            match cmd {
                HubCmd::Shutdown => stop = true,
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
            publish(&hub, &slots, selected, &serial);
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
        for slot in &mut slots {
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
        if changed || slots.iter().any(|slot| slot.status_dirty) {
            publish(&hub, &slots, selected, &serial);
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
                slot.child = None;
                slot.idle = None;
                slot.port = None;
                slot.handed = None;
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
                slot.port = None;
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
        slot.port = None;
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
        match free_port(PORT_START)
            .and_then(|port| spawn_radio(&slot.spec, port).map(|child| (port, child)))
        {
            Ok((port, mut child)) => match wait_ready(port, &mut child) {
                Ok(()) => {
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

fn next_wait(slots: &[Slot], now: Instant) -> Duration {
    let mut wait = IDLE_READ;
    for slot in slots {
        for at in [slot.retry_at, slot.next_idle].into_iter().flatten() {
            if at <= now {
                return Duration::from_millis(1);
            }
            wait = wait.min(at.saturating_duration_since(now));
        }
    }
    wait
}

fn publish(hub: &Hub, slots: &[Slot], selected: u64, serial: &[String]) {
    let rows = slots
        .iter()
        .map(|slot| RadioRow {
            id: slot.id,
            spec: slot.spec.clone(),
            port: slot.port,
            status: slot.status.clone(),
            selected: slot.id == selected,
        })
        .collect();
    let pids = slots
        .iter()
        .filter_map(|slot| slot.child.as_ref().map(|child| child.id() as i32))
        .collect();
    *hub.pids.lock().expect("cat pids") = pids;
    *hub.view.lock().expect("cat view") = HubView {
        rows,
        serial: serial.to_vec(),
    };
}

fn stop_slot(slot: &mut Slot) {
    slot.idle = None;
    if let Some(child) = slot.child.as_mut() {
        stop_child(child);
    }
    slot.child = None;
    slot.port = None;
    slot.handed = None;
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
        let port = match free_port(PORT_START) {
            Ok(port) => port,
            Err(_) => return,
        };
        let mut child = match spawn_rigctld(&spec, port) {
            Ok(child) => child,
            Err(_) => return,
        };
        let ready = wait_ready(port, &mut child);
        if ready.is_err() {
            stop_child(&mut child);
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
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err());
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
    fn gqrx_conf_copies_a_missing_file_then_patches_it() {
        let root = std::env::temp_dir().join(format!("hamspot-gqrx-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let default_conf = root.join("default.conf");
        fs::create_dir_all(&root).unwrap();
        fs::write(&default_conf, "[input]\ndevice=hackrf\n").unwrap();
        let path = root.join("radio.conf");
        prepare_gqrx_conf(&path, 4536, &default_conf).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("device=hackrf"));
        assert!(text.contains("enabled=true"));
        assert!(text.contains("port=4536"));
        fs::write(&default_conf, "[input]\ndevice=replaced\n").unwrap();
        prepare_gqrx_conf(&path, 4537, &default_conf).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("device=hackrf"));
        assert!(text.contains("port=4537"));
        assert!(!text.contains("replaced"));
        let _ = fs::remove_dir_all(&root);
    }
}
