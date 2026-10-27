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
}

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
        let (tx, rx) = mpsc::channel();
        let live = Arc::new(Mutex::new(Live::default()));
        let shared = Arc::clone(&live);
        let thread = thread::Builder::new()
            .name("nanovna-v2".into())
            .spawn(move || worker(rx, shared, ctx))
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

fn worker(rx: Receiver<Cmd>, live: Arc<Mutex<Live>>, ctx: egui::Context) {
    let mut port: Option<Box<dyn serialport::SerialPort>> = None;
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
                    ctx.request_repaint_of(super::super::viewport_id());
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if matches!(cmd, Cmd::Stop) {
            break;
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
                match open_port(&path).and_then(|mut opened| {
                    let id = identify(&mut *opened)?;
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
                    match read_sweep(port.as_mut(), identity, start_hz, step_hz, points) {
                        Ok(sweep) => {
                            let mut guard = live.lock().expect("nanovna");
                            guard.generation = guard.generation.wrapping_add(1);
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
                            live.lock().expect("nanovna").sweep_errors += 1;
                            Err(err)
                        }
                    }
                } else {
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
        ctx.request_repaint_of(super::super::viewport_id());
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
        format!("SN {}", proto::device_id_sn(id))
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

fn read_device_id_once(port: &mut dyn serialport::SerialPort) -> Result<[u8; 12], String> {
    // Drop anything stale before and after the resync so the 12 bytes line up.
    resync(port)?;
    std::thread::sleep(Duration::from_millis(20));
    let _ = port.clear(serialport::ClearBuffer::Input);
    write_cmd(port, &proto::read_device_id())?;
    let mut raw = [0u8; 12];
    read_exact(port, &mut raw, Duration::from_millis(400))?;
    proto::parse_device_id(&raw)
}

fn identify(port: &mut dyn serialport::SerialPort) -> Result<(Identity, IdRead), String> {
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
    port: &mut dyn serialport::SerialPort,
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
    port: &mut dyn serialport::SerialPort,
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

fn read_capture(port: &mut dyn serialport::SerialPort) -> Result<Vec<u8>, String> {
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

fn resync(port: &mut dyn serialport::SerialPort) -> Result<(), String> {
    let _ = port.clear(serialport::ClearBuffer::Input);
    write_cmd(port, &proto::resync())
}

fn write_cmd(port: &mut dyn serialport::SerialPort, bytes: &[u8]) -> Result<(), String> {
    port.write_all(bytes).map_err(|err| err.to_string())
}

fn read_exact(
    port: &mut dyn serialport::SerialPort,
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
        assert_eq!(id_text(&id, true), "SN 12345678-00000001-deadbeef");
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

