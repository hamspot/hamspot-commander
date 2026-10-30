//! Serial session for one NanoVNA V2. The UI thread only sees [`Live`].

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::proto::{self, Identity, Ratio};
use super::rf::{self, Sample};

pub const USB_VID: u16 = 0x04b4;
pub const USB_PID: u16 = 0x0008;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedPort {
    pub name: String,
    pub nanovna: bool,
}

pub fn list_ports() -> Vec<ListedPort> {
    let Ok(found) = serialport::available_ports() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for port in found {
        if let Some(listed) = listed_usb(port.port_name, &port.port_type) {
            out.push(listed);
        }
    }
    out.sort_by(|a, b| b.nanovna.cmp(&a.nanovna).then(a.name.cmp(&b.name)));
    out
}

/// Built-in UARTs, pseudoterminals, and ports with no USB identity are left out.
fn listed_usb(name: impl Into<String>, kind: &serialport::SerialPortType) -> Option<ListedPort> {
    let serialport::SerialPortType::UsbPort(info) = kind else {
        return None;
    };
    Some(ListedPort {
        name: name.into(),
        nanovna: info.vid == USB_VID && info.pid == USB_PID,
    })
}

#[derive(Clone, Debug)]
pub struct Live {
    pub status: String,
    pub connected: bool,
    pub identity: Option<Identity>,
    pub device_id: Option<[u8; 12]>,
    /// False when the id reads did not agree; `device_id` is then all zeros.
    pub id_reliable: bool,
    pub sweep: Vec<Sample>,
    pub generation: u64,
    pub busy: bool,
    pub capture_png: Option<Vec<u8>>,
    /// Sweeps that failed since connect. Issue 72.
    pub sweep_errors: u64,
    /// Set when the port vanished while connected, so the UI can warn.
    pub lost: bool,
    /// Sweep commands finished, with or without data. The UI re-arms Run on
    /// this, so a sweep that fails between two frames cannot stall Run.
    pub sweeps_done: u64,
    /// Why the last sweep command produced no data. Cleared by a good sweep.
    pub last_sweep_error: Option<String>,
    /// Consecutive good sweeps whose samples were bit-for-bit the same as the
    /// sweep before. A live NanoVNA always has some noise, so 1 or more is a fault.
    pub identical_run: u32,
    /// Hash of the last good sweep's samples.
    pub sweep_hash: u64,
    /// Wall-clock time (local, HH:MM:SS) of the last good sweep.
    pub sweep_time: Option<String>,
    /// Plain-text log of what was sent and what came back, newest last.
    pub log: Vec<String>,
}

/// Lines kept in [`Live::log`].
pub const LOG_LINES: usize = 12;

impl Live {
    fn note(&mut self, line: String) {
        let stamp = chrono::Local::now().format("%H:%M:%S%.3f");
        self.log.push(format!("{stamp} {line}"));
        let extra = self.log.len().saturating_sub(LOG_LINES);
        if extra > 0 {
            self.log.drain(..extra);
        }
    }
}

/// Hash of a sweep's samples, by bit pattern.
pub fn sweep_hash(sweep: &[Sample]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for s in sweep {
        for v in [s.hz, s.s11.re, s.s11.im, s.s21.re, s.s21.im] {
            v.to_bits().hash(&mut h);
        }
    }
    h.finish()
}

/// Byte link to a V2: the real serial port, or an in-memory fake in tests.
pub trait Link: std::io::Read + std::io::Write + Send {
    /// Drop unread input.
    fn clear_input(&mut self);
}

impl Link for Box<dyn serialport::SerialPort> {
    fn clear_input(&mut self) {
        let _ = self.clear(serialport::ClearBuffer::Input);
    }
}

type Opener = Box<dyn Fn(&str) -> Result<Box<dyn Link>, String> + Send>;

impl Default for Live {
    fn default() -> Self {
        Self {
            status: "Not connected.".into(),
            connected: false,
            identity: None,
            device_id: None,
            id_reliable: false,
            sweep: Vec::new(),
            generation: 0,
            busy: false,
            capture_png: None,
            sweep_errors: 0,
            lost: false,
            sweeps_done: 0,
            last_sweep_error: None,
            identical_run: 0,
            sweep_hash: 0,
            sweep_time: None,
            log: Vec::new(),
        }
    }
}

enum Cmd {
    Connect(String),
    Disconnect,
    Sweep { start_hz: u64, step_hz: u64, points: u16 },
    Average(u8),
    Power(u8),
    Capture,
    Stop,
}

pub struct Session {
    tx: Sender<Cmd>,
    live: Arc<Mutex<Live>>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    pub fn start(ctx: egui::Context) -> Self {
        Self::start_with(ctx, Box::new(|path: &str| open_port(path).map(|p| Box::new(p) as Box<dyn Link>)))
    }

    /// A session over any [`Link`]. Tests pass an in-memory V2.
    pub fn start_with(ctx: egui::Context, opener: Opener) -> Self {
        let (tx, rx) = mpsc::channel();
        let live = Arc::new(Mutex::new(Live::default()));
        let shared = Arc::clone(&live);
        let thread = thread::Builder::new()
            .name("nanovna-v2".into())
            .spawn(move || worker(rx, shared, ctx, opener))
            .ok();
        Self {
            tx,
            live,
            thread,
        }
    }

    pub fn snapshot(&self) -> Live {
        self.live.lock().expect("nanovna").clone()
    }

    pub fn connect(&self, port: String) {
        let _ = self.tx.send(Cmd::Connect(port));
    }

    pub fn disconnect(&self) {
        let _ = self.tx.send(Cmd::Disconnect);
    }

    pub fn sweep(&self, start_hz: u64, step_hz: u64, points: u16) {
        let _ = self.tx.send(Cmd::Sweep {
            start_hz,
            step_hz,
            points,
        });
    }

    pub fn average(&self, count: u8) {
        let _ = self.tx.send(Cmd::Average(count));
    }

    pub fn power(&self, level: u8) {
        let _ = self.tx.send(Cmd::Power(level));
    }

    pub fn capture(&self) {
        let _ = self.tx.send(Cmd::Capture);
    }

    pub fn take_capture(&self) -> Option<Vec<u8>> {
        self.live.lock().expect("nanovna").capture_png.take()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn worker(rx: Receiver<Cmd>, live: Arc<Mutex<Live>>, ctx: egui::Context, opener: Opener) {
    let mut port: Option<Box<dyn Link>> = None;
    let mut identity: Option<Identity> = None;
    let mut path_open = String::new();
    loop {
        // Wake a few times a second so an unplugged board shows as
        // disconnected within a second even when no sweep is running. Issue 72.
        let cmd = match rx.recv_timeout(Duration::from_millis(300)) {
            Ok(cmd) => cmd,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if port.is_some() && unplugged(&path_open) {
                    port = None;
                    identity = None;
                    let mut guard = live.lock().expect("nanovna");
                    guard.connected = false;
                    guard.identity = None;
                    guard.device_id = None;
                    guard.lost = true;
                    guard.status = format!("Disconnected. {path_open} went away.");
                    drop(guard);
                    ctx.request_repaint_of(egui::ViewportId::ROOT);
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if matches!(cmd, Cmd::Stop) {
            break;
        }
        // E-STOP choke point: no sweep and no power change while it is
        // engaged, so Commander asks the V2 for no RF stimulus. Connect,
        // disconnect and capture still work. After RELEASE sweeps resume.
        if crate::estop::engaged() && matches!(cmd, Cmd::Sweep { .. } | Cmd::Power(_)) {
            let mut guard = live.lock().expect("nanovna");
            guard.status = "E-STOP engaged: sweep blocked".into();
            if matches!(cmd, Cmd::Sweep { .. }) {
                guard.sweeps_done += 1;
                guard.last_sweep_error = Some("E-STOP engaged: sweep blocked".into());
                guard.note("sweep blocked: E-STOP engaged".into());
            }
            drop(guard);
            crate::estop::report(
                crate::estop::generation(),
                crate::estop::Kind::Transmitter,
                "NanoVNA sweep",
                crate::estop::Outcome::Stopped,
            );
            ctx.request_repaint_of(egui::ViewportId::ROOT);
            continue;
        }
        set_busy(&live, true);
        let result = match cmd {
            Cmd::Connect(path) => {
                port = None;
                identity = None;
                path_open = path.clone();
                {
                    let mut guard = live.lock().expect("nanovna");
                    guard.lost = false;
                    guard.sweep_errors = 0;
                }
                match opener(&path).and_then(|mut opened| {
                    let id = identify(opened.as_mut())?;
                    Ok((opened, id.0, id.1))
                }) {
                    Ok((opened, id, id_read)) => {
                        let device_id = id_read.id;
                        if let Err(err) = accept_v2(id) {
                            let _ = opened;
                            Err(err)
                        } else {
                            identity = Some(id);
                            port = Some(opened);
                            let mut guard = live.lock().expect("nanovna");
                            guard.identity = Some(id);
                            guard.device_id = Some(device_id);
                            guard.id_reliable = id_read.reliable;
                            guard.connected = true;
                            if let Some(note) = &id_read.note {
                                eprintln!("nanovna: {note}");
                            }
                            guard.status = format!(
                                "NanoVNA V2  firmware {}  board {}  {}",
                                id.version_label(),
                                id.board,
                                id_text(&device_id, id_read.reliable)
                            );
                            Ok(())
                        }
                    }
                    Err(err) => Err(err),
                }
            }
            Cmd::Disconnect => {
                port = None;
                identity = None;
                let mut guard = live.lock().expect("nanovna");
                guard.connected = false;
                guard.identity = None;
                guard.device_id = None;
                guard.id_reliable = false;
                guard.status = "Not connected.".into();
                Ok(())
            }
            Cmd::Sweep {
                start_hz,
                step_hz,
                points,
            } => {
                if let Some(port) = port.as_mut() {
                    live.lock().expect("nanovna").note(format!(
                        "sent sweep start {start_hz} Hz step {step_hz} Hz {points} pts"
                    ));
                    let got = read_sweep(port.as_mut(), identity, start_hz, step_hz, points);
                    let mut guard = live.lock().expect("nanovna");
                    guard.sweeps_done += 1;
                    drop(guard);
                    match got {
                        Ok(sweep) => {
                            let mut guard = live.lock().expect("nanovna");
                            guard.generation = guard.generation.wrapping_add(1);
                            let hash = sweep_hash(&sweep);
                            if guard.generation > 1 && hash == guard.sweep_hash && !guard.sweep.is_empty() {
                                guard.identical_run += 1;
                            } else {
                                guard.identical_run = 0;
                            }
                            guard.sweep_hash = hash;
                            guard.last_sweep_error = None;
                            guard.sweep_time = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                            let line = format!(
                                "sweep #{} ok: {} pts, {} bytes, hash {:016x}{}",
                                guard.generation,
                                sweep.len(),
                                sweep.len() * proto::RECORD_LEN,
                                hash,
                                if guard.identical_run > 0 {
                                    format!(" IDENTICAL to previous ({} in a row)", guard.identical_run)
                                } else {
                                    String::new()
                                }
                            );
                            guard.note(line);
                            guard.sweep = sweep;
                            let points = guard.sweep.len();
                            let id_hex = guard
                                .device_id
                                .map(|bytes| id_text(&bytes, guard.id_reliable))
                                .unwrap_or_default();
                            if let Some(id) = guard.identity {
                                guard.status = format!(
                                    "NanoVNA V2  firmware {}  board {}  {}  {points} points",
                                    id.version_label(),
                                    id.board,
                                    id_hex
                                );
                            } else {
                                guard.status = format!("{points} points");
                            }
                            Ok(())
                        }
                        Err(err) => {
                            // Re-sync so the next sweep starts on a command boundary.
                            let synced = resync(port.as_mut()).is_ok();
                            let mut guard = live.lock().expect("nanovna");
                            guard.sweep_errors += 1;
                            guard.last_sweep_error = Some(err.clone());
                            guard.note(format!(
                                "sweep failed: {err}; {}",
                                if synced { "port re-synced" } else { "re-sync failed" }
                            ));
                            Err(format!("Sweep failed: {err}. The graph is NOT showing new data."))
                        }
                    }
                } else {
                    let mut guard = live.lock().expect("nanovna");
                    guard.sweeps_done += 1;
                    guard.last_sweep_error = Some("Not connected.".into());
                    Err("Not connected.".into())
                }
            }
            Cmd::Average(count) => {
                if let Some(port) = port.as_mut() {
                    write_cmd(port.as_mut(), &proto::set_average(count))
                } else {
                    Err("Not connected.".into())
                }
            }
            Cmd::Power(level) => {
                if let Some(port) = port.as_mut() {
                    write_cmd(port.as_mut(), &proto::set_tx_power(level))
                } else {
                    Err("Not connected.".into())
                }
            }
            Cmd::Capture => {
                if let Some(port) = port.as_mut() {
                    match read_capture(port.as_mut()) {
                        Ok(png) => {
                            live.lock().expect("nanovna").capture_png = Some(png);
                            Ok(())
                        }
                        Err(err) => Err(err),
                    }
                } else {
                    Err("Not connected.".into())
                }
            }
            Cmd::Stop => break,
        };
        if let Err(err) = result {
            let mut guard = live.lock().expect("nanovna");
            if port.is_none() {
                guard.connected = false;
                guard.identity = None;
                guard.device_id = None;
            }
            guard.status = err;
        }
        set_busy(&live, false);
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
}

/// True when a Unix device node for an open port no longer exists. Other
/// systems name ports without a path, so they rely on I/O errors instead.
pub fn unplugged(path: &str) -> bool {
    #[cfg(unix)]
    {
        !path.is_empty() && path.starts_with('/') && !std::path::Path::new(path).exists()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

fn set_busy(live: &Mutex<Live>, busy: bool) {
    live.lock().expect("nanovna").busy = busy;
}

/// Indicate already returned `2`. Protocol 1 is the V2 binary protocol.
/// This board reports variant 0 with protocol 1, board 2, and firmware 1.0.
/// Requiring variant 2 closed the port after every connect.
fn accept_v2(id: Identity) -> Result<(), String> {
    if id.bootloader() {
        Err("The device is in the bootloader. Commander does not flash it. Use bootload_firmware.py or NanoVNA-QT, then power it on normally.".into())
    } else if id.protocol != 1 {
        Err(format!(
            "This port is not a NanoVNA V2 (variant {}, protocol {}).",
            id.variant, id.protocol
        ))
    } else {
        Ok(())
    }
}

fn open_port(path: &str) -> Result<Box<dyn serialport::SerialPort>, String> {
    serialport::new(path, 115_200)
        .timeout(Duration::from_millis(50))
        .open()
        .map_err(|err| format!("{path}: {err}"))
}

/// Status text for an id: the About-screen SN, or a clear unreliable flag.
pub fn id_text(id: &[u8; 12], reliable: bool) -> String {
    if reliable {
        format!("SN {}", proto::device_id_sn_compact(id))
    } else {
        "ID unreliable".into()
    }
}

/// Red note when the id reads disagreed or all timed out. A NanoVNA on a
/// marginal battery and a weak USB hub tends to power cycle mid-read.
pub const POWER_NOTE: &str = "Device ID unreliable. The NanoVNA may be power cycling: check battery charge and USB hub power, then power cycle the device and reconnect.";

pub fn power_note(live: &Live) -> Option<&'static str> {
    (live.connected && live.device_id.is_some() && !live.id_reliable).then_some(POWER_NOTE)
}

/// Result of the voted device id read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdRead {
    pub id: [u8; 12],
    pub reliable: bool,
    /// Log line for odd outcomes.
    pub note: Option<String>,
}

/// Two reads; a third only when they disagree. All-zero or all-0xff ids
/// are accepted only when all three reads give the same value. A failed
/// (timed out) read counts as a read that matches nothing. With no
/// agreement the id is all zeros and flagged unreliable.
pub fn vote_device_id(mut read: impl FnMut() -> Result<[u8; 12], String>) -> IdRead {
    let a = read().ok();
    let b = read().ok();
    if let (Some(a), Some(b)) = (a, b) {
        if a == b && !proto::device_id_suspect(&a) {
            return IdRead { id: a, reliable: true, note: None };
        }
    }
    let c = read().ok();
    if let Some(c) = c {
        if proto::device_id_suspect(&c) {
            if a == Some(c) && b == Some(c) {
                return IdRead {
                    id: c,
                    reliable: true,
                    note: Some(format!(
                        "device id read {} three times; the firmware may not report an id",
                        proto::device_id_hex(&c)
                    )),
                };
            }
        } else if a == Some(c) || b == Some(c) {
            return IdRead {
                id: c,
                reliable: true,
                note: Some("device id reads disagreed; the third read settled it".into()),
            };
        }
    }
    IdRead {
        id: [0u8; 12],
        reliable: false,
        note: Some("device id reads did not agree; the id is unreliable".into()),
    }
}

fn read_device_id_once(port: &mut dyn Link) -> Result<[u8; 12], String> {
    // Drop anything stale before and after the resync so the 12 bytes line up.
    resync(port)?;
    std::thread::sleep(Duration::from_millis(20));
    port.clear_input();
    write_cmd(port, &proto::read_device_id())?;
    let mut raw = [0u8; 12];
    read_exact(port, &mut raw, Duration::from_millis(400))?;
    proto::parse_device_id(&raw)
}

fn identify(port: &mut dyn Link) -> Result<(Identity, IdRead), String> {
    resync(port)?;
    write_cmd(port, &proto::indicate())?;
    let mut mark = [0u8; 1];
    read_exact(port, &mut mark, Duration::from_millis(200))?;
    if mark[0] != b'2' {
        return Err("The port did not answer the NanoVNA V2 indicate command.".into());
    }
    write_cmd(port, &proto::read_identity())?;
    let mut ident = [0u8; 5];
    read_exact(port, &mut ident, Duration::from_millis(400))?;
    let id = Identity::parse(&ident)?;
    let ids = vote_device_id(|| read_device_id_once(port));
    Ok((id, ids))
}

fn read_sweep(
    port: &mut dyn Link,
    identity: Option<Identity>,
    start_hz: u64,
    step_hz: u64,
    points: u16,
) -> Result<Vec<Sample>, String> {
    if points < 2 || points as usize > proto::POINTS_MAX {
        return Err(format!("point count {points} is outside 2..={}", proto::POINTS_MAX));
    }
    if step_hz == 0 {
        return Err("the sweep step is zero".into());
    }
    let hack = identity.is_some_and(|id| id.s21_hack());
    let (prog_start, prog_points) = proto::programmed_sweep(start_hz, step_hz, points, hack);
    let slots = match read_fifo_slots(port, prog_start, step_hz, prog_points) {
        Ok(slots) => slots,
        Err(err) => {
            resync(port)?;
            read_fifo_slots(port, prog_start, step_hz, prog_points).map_err(|_| err)?
        }
    };
    let skip = if hack && prog_points != points { 1 } else { 0 };
    let mut out = Vec::with_capacity(points as usize);
    for (index, slot) in slots.into_iter().enumerate().skip(skip) {
        let Some(ratio) = slot else {
            return Err(format!("frequency index {index} is missing"));
        };
        let hz = prog_start as f64 + index as f64 * step_hz as f64;
        out.push(rf::from_ratio(
            hz,
            ratio.s11_re,
            ratio.s11_im,
            ratio.s21_re,
            ratio.s21_im,
        ));
    }
    if out.len() != points as usize {
        return Err(format!(
            "sweep returned {} points, expected {points}",
            out.len()
        ));
    }
    Ok(out)
}

fn read_fifo_slots(
    port: &mut dyn Link,
    prog_start: u64,
    step_hz: u64,
    prog_points: u16,
) -> Result<Vec<Option<Ratio>>, String> {
    resync(port)?;
    write_cmd(port, &proto::clear_fifo())?;
    write_cmd(port, &proto::set_sweep(prog_start, step_hz, prog_points))?;
    let mut slots: Vec<Option<Ratio>> = vec![None; prog_points as usize];
    let mut left = prog_points as usize;
    while left > 0 {
        let count = left.min(proto::FIFO_CHUNK);
        write_cmd(port, &proto::read_fifo(count as u8))?;
        let mut buf = vec![0u8; count * proto::RECORD_LEN];
        // A point takes a few tens of milliseconds on the device. The read
        // returns as the FIFO fills; there is no separate ready flag.
        let budget = Duration::from_millis(100 + count as u64 * 40);
        read_exact(port, &mut buf, budget)?;
        let parsed = proto::assemble(&buf, prog_points as usize)?;
        for (index, ratio) in parsed.into_iter().enumerate() {
            if ratio.is_some() {
                slots[index] = ratio;
            }
        }
        let now = slots.iter().filter(|slot| slot.is_none()).count();
        if now == left {
            return Err(format!("{left} sweep points did not arrive"));
        }
        left = now;
    }
    Ok(slots)
}

fn read_capture(port: &mut dyn Link) -> Result<Vec<u8>, String> {
    resync(port)?;
    write_cmd(port, &proto::capture())?;
    let mut header = [0u8; 5];
    read_exact(port, &mut header, Duration::from_millis(500))?;
    let meta = proto::parse_capture_meta(&header)?;
    let nbytes = usize::from(meta.width) * usize::from(meta.height) * 2;
    let mut pixels = vec![0u8; nbytes];
    read_exact(port, &mut pixels, Duration::from_secs(8))?;
    rf::rgb565_png(meta.width, meta.height, &pixels)
}

fn resync(port: &mut dyn Link) -> Result<(), String> {
    port.clear_input();
    write_cmd(port, &proto::resync())
}

fn write_cmd(port: &mut dyn Link, bytes: &[u8]) -> Result<(), String> {
    port.write_all(bytes).map_err(|err| err.to_string())
}

fn read_exact(
    port: &mut dyn Link,
    buf: &mut [u8],
    budget: Duration,
) -> Result<(), String> {
    let start = Instant::now();
    let mut got = 0;
    while got < buf.len() {
        if start.elapsed() > budget {
            return Err(format!(
                "timed out after {} of {} bytes",
                got,
                buf.len()
            ));
        }
        match port.read(&mut buf[got..]) {
            Ok(0) => return Err("the port closed".into()),
            Ok(n) => got += n,
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(err) => return Err(err.to_string()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usb(vid: u16, pid: u16) -> serialport::SerialPortType {
        serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
            vid,
            pid,
            serial_number: None,
            manufacturer: None,
            product: None,
        })
    }

    #[test]
    fn a_v2_with_variant_zero_is_accepted_and_other_protocols_are_not() {
        let seen = Identity {
            variant: 0,
            protocol: 1,
            board: 2,
            fw_major: 1,
            fw_minor: 0,
        };
        assert!(accept_v2(seen).is_ok());
        assert!(accept_v2(Identity { variant: 2, ..seen }).is_ok());
        assert!(accept_v2(Identity { protocol: 2, ..seen }).is_err());
        assert!(accept_v2(Identity {
            fw_major: 0xff,
            ..seen
        })
        .is_err());
    }

    fn run(reads: Vec<Result<[u8; 12], String>>) -> (IdRead, usize) {
        let mut it = reads.into_iter();
        let mut n = 0;
        let got = vote_device_id(|| {
            n += 1;
            it.next().expect("too many reads")
        });
        (got, n)
    }

    const A: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    const B: [u8; 12] = [9; 12];
    const C: [u8; 12] = [7; 12];
    const Z: [u8; 12] = [0; 12];
    const F: [u8; 12] = [0xff; 12];
    fn t() -> Result<[u8; 12], String> {
        Err("timed out after 0 of 12 bytes".into())
    }

    #[test]
    fn two_matching_reads_are_accepted_without_a_third() {
        let (got, n) = run(vec![Ok(A), Ok(A)]);
        assert_eq!((got.id, got.reliable, n), (A, true, 2));
    }

    #[test]
    fn a_third_read_settles_a_disagreement() {
        let (got, n) = run(vec![Ok(A), Ok(B), Ok(A)]);
        assert_eq!((got.id, got.reliable, n), (A, true, 3));
        let (got, _) = run(vec![Ok(A), Ok(B), Ok(B)]);
        assert_eq!((got.id, got.reliable), (B, true));
    }

    #[test]
    fn three_different_reads_give_an_unreliable_zero_id() {
        let (got, _) = run(vec![Ok(A), Ok(B), Ok(C)]);
        assert_eq!((got.id, got.reliable), (Z, false));
    }

    #[test]
    fn timeouts_count_as_failed_reads() {
        let (got, _) = run(vec![t(), Ok(A), Ok(A)]);
        assert_eq!((got.id, got.reliable), (A, true));
        let (got, _) = run(vec![t(), t(), Ok(A)]);
        assert_eq!((got.id, got.reliable), (Z, false));
        let (got, _) = run(vec![t(), t(), t()]);
        assert_eq!((got.id, got.reliable), (Z, false));
    }

    #[test]
    fn all_zero_or_all_ff_needs_three_reads() {
        let (got, n) = run(vec![Ok(Z), Ok(Z), Ok(Z)]);
        assert_eq!((got.id, got.reliable, n), (Z, true, 3));
        assert!(got.note.is_some());
        let (got, _) = run(vec![Ok(Z), Ok(Z), Ok(A)]);
        assert_eq!((got.id, got.reliable), (Z, false));
        let (got, _) = run(vec![Ok(F), Ok(F), Ok(F)]);
        assert_eq!((got.id, got.reliable), (F, true));
        let (got, _) = run(vec![Ok(F), Ok(A), Ok(F)]);
        assert_eq!((got.id, got.reliable), (Z, false));
        let (got, _) = run(vec![Ok(Z), Ok(A), Ok(A)]);
        assert_eq!((got.id, got.reliable), (A, true));
    }

    #[test]
    fn the_power_note_shows_only_for_an_unreliable_connected_id() {
        let mut live = Live { connected: true, device_id: Some(Z), id_reliable: false, ..Live::default() };
        assert_eq!(power_note(&live), Some(POWER_NOTE));
        live.id_reliable = true;
        assert_eq!(power_note(&live), None);
        live = Live::default();
        assert_eq!(power_note(&live), None);
        // Three timeouts give an unreliable id, so the note shows.
        let (got, _) = run(vec![t(), t(), t()]);
        live = Live { connected: true, device_id: Some(got.id), id_reliable: got.reliable, ..Live::default() };
        assert!(power_note(&live).is_some());
    }

    #[test]
    fn id_text_uses_the_about_screen_format() {
        let id = [0x78, 0x56, 0x34, 0x12, 1, 0, 0, 0, 0xef, 0xbe, 0xad, 0xde];
        assert_eq!(id_text(&id, true), "SN 12345678-1-deadbeef");
        assert_eq!(id_text(&id, false), "ID unreliable");
    }

    #[test]
    fn only_a_usb_serial_port_is_listed() {
        let v2 = listed_usb("/dev/ttyACM0", &usb(USB_VID, USB_PID)).unwrap();
        assert!(v2.nanovna);
        let other = listed_usb("/dev/ttyUSB0", &usb(0x0403, 0x6001)).unwrap();
        assert!(!other.nanovna);
        assert!(listed_usb("/dev/ttyS0", &serialport::SerialPortType::PciPort).is_none());
        assert!(listed_usb("/dev/pts/1", &serialport::SerialPortType::Unknown).is_none());
        assert!(listed_usb("rfcomm0", &serialport::SerialPortType::BluetoothPort).is_none());
    }
}


/// In-memory NanoVNA V2 for headless tests: it parses the binary protocol and
/// answers like the firmware.
#[cfg(test)]
pub mod fake {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Mode {
        /// Every sweep has new values.
        Normal,
        /// Every sweep returns exactly the same values.
        Identical,
        /// FIFO reads return nothing.
        Silent,
    }

    pub struct State {
        pub mode: Mode,
        pub start: u64,
        pub step: u64,
        pub points: u16,
        pub sweeps: u64,
        pub resyncs: u64,
        pub sent: Vec<u8>,
    }

    pub struct FakeV2 {
        pub state: Arc<Mutex<State>>,
        input: Vec<u8>,
        out: VecDeque<u8>,
        nops: usize,
    }

    pub fn new(mode: Mode) -> (FakeV2, Arc<Mutex<State>>) {
        let state = Arc::new(Mutex::new(State { mode, start: 0, step: 0, points: 0, sweeps: 0, resyncs: 0, sent: Vec::new() }));
        (FakeV2 { state: Arc::clone(&state), input: Vec::new(), out: VecDeque::new(), nops: 0 }, state)
    }

    pub fn opener(state: Arc<Mutex<State>>) -> Opener {
        Box::new(move |_path: &str| {
            Ok(Box::new(FakeV2 { state: Arc::clone(&state), input: Vec::new(), out: VecDeque::new(), nops: 0 }) as Box<dyn Link>)
        })
    }

    impl FakeV2 {
        fn run(&mut self) {
            loop {
                let Some(&op) = self.input.first() else { return };
                let need = match op {
                    proto::NOP | proto::INDICATE => 1,
                    proto::READ1 | proto::READ4 => 2,
                    proto::WRITE1 | proto::READ_FIFO => 3,
                    proto::WRITE2 => 4,
                    proto::WRITE8 => 10,
                    _ => 1,
                };
                if self.input.len() < need {
                    return;
                }
                let cmd: Vec<u8> = self.input.drain(..need).collect();
                if op == proto::NOP {
                    self.nops += 1;
                    if self.nops == 8 {
                        self.state.lock().unwrap().resyncs += 1;
                    }
                    continue;
                }
                self.nops = 0;
                let mut st = self.state.lock().unwrap();
                match op {
                    proto::INDICATE => self.out.push_back(b'2'),
                    proto::READ1 => self.out.push_back(match cmd[1] {
                        proto::ADDR_VARIANT => 2,
                        proto::ADDR_PROTOCOL => 1,
                        proto::ADDR_BOARD => 4,
                        proto::ADDR_FW_MAJOR => 1,
                        proto::ADDR_FW_MINOR => 2,
                        _ => 0,
                    }),
                    proto::READ4 => self.out.extend([cmd[1], 0x11, 0x22, 0x33]),
                    proto::WRITE1 if cmd[1] == proto::ADDR_FIFO => {}
                    proto::WRITE8 => {
                        let v = u64::from_le_bytes(cmd[2..10].try_into().unwrap());
                        if cmd[1] == proto::ADDR_SWEEP_START { st.start = v } else { st.step = v }
                    }
                    proto::WRITE2 if cmd[1] == proto::ADDR_SWEEP_POINTS => {
                        st.points = u16::from_le_bytes([cmd[2], cmd[3]]);
                        st.sweeps += 1;
                    }
                    proto::READ_FIFO => {
                        if st.mode == Mode::Silent {
                            continue;
                        }
                        let salt = if st.mode == Mode::Identical { 1 } else { st.sweeps as i32 };
                        for index in 0..u16::from(cmd[2]).min(st.points) {
                            self.out.extend(proto::encode_record(&proto::RawRecord {
                                fwd_re: 1_000_000,
                                fwd_im: 0,
                                rev0_re: 1000 * salt + i32::from(index),
                                rev0_im: 100,
                                rev1_re: 500,
                                rev1_im: 0,
                                freq_index: index,
                            }));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    impl std::io::Write for FakeV2 {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.state.lock().unwrap().sent.extend_from_slice(buf);
            self.input.extend_from_slice(buf);
            self.run();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl std::io::Read for FakeV2 {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.out.is_empty() {
                std::thread::sleep(Duration::from_millis(2));
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            let n = buf.len().min(self.out.len());
            for b in buf.iter_mut().take(n) {
                *b = self.out.pop_front().unwrap();
            }
            Ok(n)
        }
    }

    impl Link for FakeV2 {
        fn clear_input(&mut self) {
            self.out.clear();
        }
    }
}

#[cfg(test)]
mod fake_tests {
    use super::fake::{self, Mode};
    use super::*;

    fn wait(session: &Session, f: impl Fn(&Live) -> bool) -> Live {
        let t = Instant::now();
        loop {
            let live = session.snapshot();
            if f(&live) {
                return live;
            }
            assert!(t.elapsed() < Duration::from_secs(10), "timed out: {:?} {:?}", live.status, live.log);
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn connected(mode: Mode) -> (Session, Arc<Mutex<fake::State>>) {
        let (_dev, state) = fake::new(mode);
        let session = Session::start_with(egui::Context::default(), fake::opener(Arc::clone(&state)));
        session.connect("fake".into());
        wait(&session, |l| l.connected && !l.busy);
        (session, state)
    }

    #[test]
    fn each_sweep_brings_new_data_and_counts() {
        let (s, st) = connected(Mode::Normal);
        let mut hashes = Vec::new();
        for n in 1..=3u64 {
            s.sweep(10_000_000, 1_000_000, 11);
            let live = wait(&s, |l| l.sweeps_done == n && !l.busy);
            assert_eq!(live.generation, n);
            assert_eq!(live.sweep.len(), 11);
            assert_eq!(live.identical_run, 0);
            assert!(live.sweep_time.is_some());
            hashes.push(live.sweep_hash);
        }
        hashes.dedup();
        assert_eq!(hashes.len(), 3, "every sweep differs");
        assert_eq!(st.lock().unwrap().sweeps, 3, "the device was asked to sweep each time");
        assert!(s.snapshot().log.iter().any(|l| l.contains("sweep #3 ok")));
    }

    #[test]
    fn identical_frames_are_counted_and_logged() {
        let (s, _) = connected(Mode::Identical);
        for n in 1..=3u64 {
            s.sweep(10_000_000, 1_000_000, 11);
            wait(&s, |l| l.sweeps_done == n && !l.busy);
        }
        let live = s.snapshot();
        assert_eq!(live.identical_run, 2);
        assert!(live.log.last().unwrap().contains("IDENTICAL to previous (2 in a row)"));
    }

    #[test]
    fn a_silent_device_reports_an_error_and_resyncs() {
        let (s, st) = connected(Mode::Silent);
        let before = st.lock().unwrap().resyncs;
        s.sweep(10_000_000, 1_000_000, 5);
        let live = wait(&s, |l| l.sweeps_done == 1 && !l.busy);
        assert_eq!(live.generation, 0, "no data, no new generation");
        assert!(live.last_sweep_error.as_deref().unwrap().contains("timed out"));
        assert!(live.status.contains("NOT showing new data"));
        assert_eq!(live.sweep_errors, 1);
        assert!(st.lock().unwrap().resyncs > before);
        assert!(live.log.iter().any(|l| l.contains("port re-synced")));
        // The device recovers: the next sweep clears the error.
        st.lock().unwrap().mode = Mode::Normal;
        s.sweep(10_000_000, 1_000_000, 5);
        let live = wait(&s, |l| l.sweeps_done == 2 && !l.busy);
        assert_eq!(live.generation, 1);
        assert!(live.last_sweep_error.is_none());
    }
}
