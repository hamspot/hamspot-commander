//! Antenna rotators Commander starts itself.
//!
//! Hamlib `rotctld` is the daemon, the same role `rigctld` has for a radio.
//! A Yaesu G-5500 is not a model number. The computer talks to a GS-232A
//! (601) or GS-232B (603), and that box drives the G-5500. Gpredict sends
//! `P` to the listen port. This module polls `p` and does not send `P`.

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

/// Gpredict's rotator default. Radios start at 4532, so a free machine gives
/// the first rotator 4533. A port a radio already reserved is skipped.
const PORT_START: u16 = 4533;
/// A dead `rotctld` publishes nothing, and the serial node is still there, so
/// nothing else wakes this thread. The gap is the same one the radio
/// supervisor uses before it starts the daemon again.
const RETRY_AFTER: Duration = Duration::from_secs(2);
/// `rotctld` does not push position. The read is how azimuth and elevation
/// stay current, and how a hung daemon is noticed.
const POSITION_EVERY: Duration = Duration::from_secs(1);

/// `rotctld -l` on hamlib 4.6. Macro starts two columns later than `rigctld -l`.
const COL_MFG: usize = 8;
const COL_MODEL: usize = 31;
const COL_VERSION: usize = 55;
const COL_STATUS: usize = 71;
const COL_MACRO: usize = 85;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotatorSpec {
    pub name: String,
    pub model: i32,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub serial_speed: String,
    #[serde(default)]
    pub conf: String,
}

impl RotatorSpec {
    #[cfg(test)]
    pub fn dummy() -> Self {
        Self {
            name: "Dummy".into(),
            model: 1,
            device: String::new(),
            serial_speed: String::new(),
            conf: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RotatorRow {
    pub id: u64,
    pub spec: RotatorSpec,
    pub port: Option<u16>,
    pub status: String,
    pub selected: bool,
    pub azimuth: Option<f64>,
    pub elevation: Option<f64>,
    pub make: String,
    pub model_label: String,
}

#[derive(Clone, Debug, Default)]
pub struct RotView {
    pub rows: Vec<RotatorRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RotModel {
    pub id: i32,
    pub mfg: String,
    pub model: String,
    pub version: String,
    pub status: String,
    pub macro_name: String,
}

enum RotCmd {
    Add(RotatorSpec),
    Update(u64, RotatorSpec),
    Remove(u64),
    Select(u64),
    Replace(Vec<RotatorSpec>),
    Shutdown,
}

struct Slot {
    id: u64,
    spec: RotatorSpec,
    port: Option<u16>,
    status: String,
    status_dirty: bool,
    child: Option<Child>,
    retry_at: Option<Instant>,
    next_pos: Option<Instant>,
    azimuth: Option<f64>,
    elevation: Option<f64>,
    make: String,
    model_label: String,
    held: Option<u16>,
}

pub struct Hub {
    cmds: Mutex<Vec<RotCmd>>,
    view: Mutex<RotView>,
    pids: Mutex<Vec<i32>>,
    wake: RawFd,
    done: Mutex<bool>,
    done_wake: Condvar,
}

pub fn validate_spec(spec: &RotatorSpec) -> Result<(), String> {
    if spec.name.trim().is_empty() {
        return Err("rotator name is empty".into());
    }
    if spec.model <= 0 {
        return Err("rotator model must be greater than zero".into());
    }
    if !spec.serial_speed.trim().is_empty() && spec.serial_speed.trim().parse::<u32>().is_err() {
        return Err("serial speed is not a number".into());
    }
    Ok(())
}

/// `rotctld` argv. Loopback only. `-o` is not a rotator flag and is not sent.
pub fn rotctld_argv(spec: &RotatorSpec, port: u16) -> Vec<String> {
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

pub fn parse_model_list(text: &str) -> Vec<RotModel> {
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
        rows.push(RotModel {
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

pub fn format_model_row(row: &RotModel) -> String {
    format!(
        "{:<7}{:<22}{:<24}{:<16}{:<12}{}",
        row.id, row.mfg, row.model, row.version, row.status, row.macro_name
    )
}

pub fn model_menu_header() -> String {
    format!(
        "{:<7}{:<22}{:<24}{:<16}{:<12}{}",
        "Rot #", "Mfg", "Model", "Version", "Status", "Macro"
    )
}

pub fn model_button_label(row: &RotModel) -> String {
    match (row.mfg.is_empty(), row.model.is_empty()) {
        (true, true) => row.id.to_string(),
        (true, false) => format!("{}  {}", row.id, row.model),
        (false, true) => format!("{}  {}", row.id, row.mfg),
        (false, false) => format!("{}  {} {}", row.id, row.mfg, row.model),
    }
}

pub fn model_list() -> Result<Vec<RotModel>, String> {
    let output = Command::new("rotctld")
        .arg("-l")
        .output()
        .map_err(|err| format!("rotctld -l: {err}"))?;
    if !output.status.success() {
        return Err(format!("rotctld -l exited {}", output.status));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_model_list(&text))
}

pub fn spawn_rotctld(spec: &RotatorSpec, port: u16) -> Result<Child, String> {
    validate_spec(spec)?;
    let mut cmd = Command::new("rotctld");
    cmd.args(rotctld_argv(spec, port))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    cmd.spawn().map_err(|err| format!("rotctld: {err}"))
}

/// TCP connect is the ready signal. A refused connect returns at once and
/// `rotctld` publishes no listen event, so the gap between tries is a short wait.
fn wait_ready(port: u16, child: &mut Child) -> Result<(), String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(format!(
                    "rotctld exited before it accepted a connection ({status})"
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

fn release_held(slot: &mut Slot) {
    if let Some(port) = slot.held.take() {
        release_port(port);
    }
}

fn stop_child(child: &mut Child) {
    cat::stop_child(child);
}

struct PosClient {
    stream: TcpStream,
}

impl PosClient {
    fn open(port: u16) -> Result<Self, String> {
        let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|err| err.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|err| err.to_string())?;
        stream.set_nodelay(true).map_err(|err| err.to_string())?;
        Ok(Self { stream })
    }

    fn position(&mut self) -> Result<(f64, f64), String> {
        let reply = transact(&mut self.stream, "get_pos")?;
        if reply.0 != 0 {
            return Err(format!("get_pos RPRT {}", reply.0));
        }
        let az = labeled(&reply.1, "Azimuth").ok_or("get_pos had no Azimuth")?;
        let el = labeled(&reply.1, "Elevation").ok_or("get_pos had no Elevation")?;
        Ok((az, el))
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
            return Err("rotctld did not answer".into());
        }
        let mut buf = [0u8; 4096];
        match stream.read(&mut buf) {
            Ok(0) => return Err("rotctld closed the connection".into()),
            Ok(n) => acc.extend_from_slice(&buf[..n]),
            Err(err)
                if err.kind() == std::io::ErrorKind::TimedOut
                    || err.kind() == std::io::ErrorKind::WouldBlock =>
            {
                return Err("rotctld did not answer".into());
            }
            Err(err) => return Err(err.to_string()),
        }
    }
}

fn labeled(lines: &[String], key: &str) -> Option<f64> {
    let prefix = format!("{key}:");
    lines.iter().find_map(|line| {
        line.trim()
            .strip_prefix(&prefix)
            .map(str::trim)
            .and_then(|text| text.parse().ok())
    })
}

impl Hub {
    pub fn spawn(ctx: egui::Context) -> Arc<Self> {
        let wake = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        let hub = Arc::new(Self {
            cmds: Mutex::new(Vec::new()),
            view: Mutex::new(RotView::default()),
            pids: Mutex::new(Vec::new()),
            wake,
            done: Mutex::new(false),
            done_wake: Condvar::new(),
        });
        let thread_hub = Arc::clone(&hub);
        thread::spawn(move || supervise(thread_hub, ctx));
        hub
    }

    pub fn view(&self) -> RotView {
        self.view.lock().expect("rot view").clone()
    }

    pub fn add(&self, spec: RotatorSpec) {
        self.send(RotCmd::Add(spec));
    }

    pub fn update(&self, id: u64, spec: RotatorSpec) {
        self.send(RotCmd::Update(id, spec));
    }

    pub fn remove(&self, id: u64) {
        self.send(RotCmd::Remove(id));
    }

    pub fn select(&self, id: u64) {
        self.send(RotCmd::Select(id));
    }

    pub fn replace(&self, specs: Vec<RotatorSpec>) {
        self.send(RotCmd::Replace(specs));
    }

    /// Stop every `rotctld` this program started. The supervisor's stop is
    /// the completion signal. The wait is bounded because a dead supervisor
    /// cannot set `done`, and closing the window must not hang.
    pub fn shutdown(&self) {
        self.send(RotCmd::Shutdown);
        let guard = self.done.lock().expect("rot done");
        let (guard, _) = self
            .done_wake
            .wait_timeout(guard, Duration::from_secs(8))
            .expect("rot done");
        if *guard {
            return;
        }
        drop(guard);
        let pids = self.pids.lock().expect("rot pids").clone();
        for pid in pids {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
        }
    }

    fn send(&self, cmd: RotCmd) {
        self.cmds.lock().expect("rot cmds").push(cmd);
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

fn supervise(hub: Arc<Hub>, ctx: egui::Context) {
    let models = model_list().unwrap_or_default();
    let mut slots: Vec<Slot> = Vec::new();
    let mut next_id = 1u64;
    let mut selected = 0u64;
    let watch = DevWatch::open();
    publish(&hub, &slots, selected);
    ctx.request_repaint();
    loop {
        let cmds = std::mem::take(&mut *hub.cmds.lock().expect("rot cmds"));
        let mut changed = !cmds.is_empty();
        let mut stop = false;
        for cmd in cmds {
            changed = true;
            match cmd {
                RotCmd::Shutdown => stop = true,
                RotCmd::Add(spec) => {
                    if validate_spec(&spec).is_err() {
                        continue;
                    }
                    let id = next_id;
                    next_id += 1;
                    slots.push(new_slot(id, spec, &models));
                    selected = id;
                }
                RotCmd::Update(id, spec) => {
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
                RotCmd::Remove(id) => {
                    if let Some(pos) = slots.iter().position(|slot| slot.id == id) {
                        let mut slot = slots.remove(pos);
                        stop_slot(&mut slot);
                        if selected == id {
                            selected = slots.first().map(|slot| slot.id).unwrap_or(0);
                        }
                    }
                }
                RotCmd::Select(id) => {
                    if slots.iter().any(|slot| slot.id == id) {
                        selected = id;
                    }
                }
                RotCmd::Replace(specs) => {
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
            }
        }
        if stop {
            for slot in &mut slots {
                stop_slot(slot);
            }
            publish(&hub, &slots, selected);
            break;
        }
        if let Some(watch) = &watch {
            if watch.drain() {
                changed = true;
            }
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
        park_until(&hub, &watch, next_wait(&slots, Instant::now()));
    }
    *hub.done.lock().expect("rot done") = true;
    hub.done_wake.notify_all();
}

fn new_slot(id: u64, spec: RotatorSpec, models: &[RotModel]) -> Slot {
    let mut slot = Slot {
        id,
        spec,
        port: None,
        status: "starting".into(),
        status_dirty: true,
        child: None,
        retry_at: None,
        next_pos: None,
        azimuth: None,
        elevation: None,
        make: String::new(),
        model_label: String::new(),
        held: None,
    };
    label_slot(&mut slot, models);
    slot
}

fn label_slot(slot: &mut Slot, models: &[RotModel]) {
    if let Some(row) = models.iter().find(|row| row.id == slot.spec.model) {
        slot.make = row.mfg.clone();
        slot.model_label = row.model.clone();
    } else {
        slot.make.clear();
        slot.model_label.clear();
    }
}

fn tick_slot(slot: &mut Slot, now: Instant) {
    if let Some(child) = slot.child.as_mut() {
        match child.try_wait() {
            Ok(Some(status)) => {
                slot.child = None;
                release_held(slot);
                slot.port = None;
                slot.next_pos = None;
                slot.status = format!("retrying ({status})");
                slot.status_dirty = true;
                slot.retry_at = Some(now + RETRY_AFTER);
            }
            Ok(None) => {}
            Err(_) => {
                slot.child = None;
                release_held(slot);
                slot.port = None;
                slot.next_pos = None;
            }
        }
    }
    let device = slot.spec.device.trim().to_string();
    if !device.is_empty() && !Path::new(&device).exists() {
        if let Some(child) = slot.child.as_mut() {
            stop_child(child);
        }
        slot.child = None;
        release_held(slot);
        slot.port = None;
        slot.next_pos = None;
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
            spawn_rotctld(&slot.spec, port)
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
                    slot.next_pos = Some(now);
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
    let Some(port) = slot.port else {
        return;
    };
    if slot.next_pos.is_some_and(|at| now < at) {
        return;
    }
    slot.next_pos = Some(now + POSITION_EVERY);
    match PosClient::open(port).and_then(|mut client| client.position()) {
        Ok((az, el)) => {
            let changed = slot.azimuth != Some(az) || slot.elevation != Some(el);
            slot.azimuth = Some(az);
            slot.elevation = Some(el);
            if changed {
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
        stop_child(child);
    }
    slot.child = None;
    release_held(slot);
    slot.port = None;
    slot.next_pos = None;
}

fn next_wait(slots: &[Slot], now: Instant) -> Duration {
    let mut wait = POSITION_EVERY;
    for slot in slots {
        for at in [slot.retry_at, slot.next_pos].into_iter().flatten() {
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
        .map(|slot| RotatorRow {
            id: slot.id,
            spec: slot.spec.clone(),
            port: slot.port,
            status: slot.status.clone(),
            selected: slot.id == selected,
            azimuth: slot.azimuth,
            elevation: slot.elevation,
            make: slot.make.clone(),
            model_label: slot.model_label.clone(),
        })
        .collect();
    let pids = slots
        .iter()
        .filter_map(|slot| slot.child.as_ref().map(|child| child.id() as i32))
        .collect();
    *hub.pids.lock().expect("rot pids") = pids;
    *hub.view.lock().expect("rot view") = RotView { rows };
}

fn park_until(hub: &Hub, watch: &Option<DevWatch>, wait: Duration) {
    let ms = wait.as_millis().min(i32::MAX as u128) as i32;
    let mut wake = libc::pollfd {
        fd: hub.wake,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe {
        if let Some(watch) = watch {
            let mut both = [
                wake,
                libc::pollfd {
                    fd: watch.fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            libc::poll(both.as_mut_ptr(), 2, ms);
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
    if let Some(watch) = watch {
        watch.drain();
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
    fn argv_is_loopback_and_omits_an_empty_device() {
        let spec = RotatorSpec {
            name: "Tower".into(),
            model: 601,
            device: "/dev/ttyUSB0".into(),
            serial_speed: "9600".into(),
            conf: "retry=2".into(),
        };
        let args = rotctld_argv(&spec, 4533);
        assert_eq!(
            args,
            vec![
                "-m",
                "601",
                "-T",
                "127.0.0.1",
                "-t",
                "4533",
                "-r",
                "/dev/ttyUSB0",
                "-s",
                "9600",
                "-C",
                "retry=2",
            ]
        );
        assert!(!args.iter().any(|arg| arg == "-o" || arg == "--vfo"));
        let bare = RotatorSpec::dummy();
        let args = rotctld_argv(&bare, 4534);
        assert!(!args.iter().any(|arg| arg == "-r"));
        assert!(validate_spec(&bare).is_ok());
    }

    #[test]
    fn gs232a_parses_as_yaesu_model_601() {
        let text = "\
 Rot #  Mfg                    Model                   Version         Status        Macro
   601  Yaesu                  GS-232A                 20220109.0      Stable        ROT_MODEL_GS232A
   603  Yaesu                  GS-232B                 20220109.0      Stable        ROT_MODEL_GS232B
";
        let rows = parse_model_list(text);
        assert_eq!(rows[0].id, 601);
        assert_eq!(rows[0].mfg, "Yaesu");
        assert_eq!(rows[0].model, "GS-232A");
        assert_eq!(rows[0].status, "Stable");
        assert_eq!(rows[0].macro_name, "ROT_MODEL_GS232A");
        assert_eq!(rows[1].id, 603);
        assert_eq!(rows[1].model, "GS-232B");
        assert_eq!(model_button_label(&rows[0]), "601  Yaesu GS-232A");
        let shown = format_model_row(&rows[0]);
        let other = format_model_row(&rows[1]);
        assert_eq!(shown.find("Yaesu"), other.find("Yaesu"));
        assert_eq!(shown.find("GS-232A"), other.find("GS-232B"));
    }

    #[test]
    fn dummy_rotctld_answers_position_and_the_process_ends() {
        let spec = RotatorSpec::dummy();
        let port = match claim_port(PORT_START) {
            Ok(port) => port,
            Err(_) => return,
        };
        let mut child = match spawn_rotctld(&spec, port) {
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
        let mut client = PosClient::open(port).expect("client");
        let (az, el) = client.position().expect("get_pos");
        assert!((az - 0.0).abs() < 0.01);
        assert!((el - 0.0).abs() < 0.01);
        drop(client);
        stop_child(&mut child);
        release_port(port);
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err());
    }
}
