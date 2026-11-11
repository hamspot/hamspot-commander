//! libhackrf, loaded at run time with `dlopen`, for the direct HackRF
//! capture. One open handle carries the sample stream and every control
//! call: retune, gain, the MAX2837 registers and the M0 shortfall count.
//! The samples are the same signed 8-bit IQ `hackrf_transfer -r -` writes.
//! No link-time dependency: a missing library is an error at start, and the
//! capture then falls back to `hackrf_transfer` (decisions.md, "libhackrf
//! capture in-process").

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

type Dev = *mut c_void;

/// `hackrf_transfer` from `hackrf.h`.
#[repr(C)]
struct Transfer {
    device: Dev,
    buffer: *mut u8,
    buffer_length: c_int,
    valid_length: c_int,
    rx_ctx: *mut c_void,
    tx_ctx: *mut c_void,
}

/// `hackrf_m0_state` from `hackrf.h`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct M0State {
    requested_mode: u16,
    request_flag: u16,
    active_mode: u32,
    m0_count: u32,
    m4_count: u32,
    num_shortfalls: u32,
    longest_shortfall: u32,
    shortfall_limit: u32,
    threshold: u32,
    next_mode: u32,
    error: u32,
}

type RxCb = extern "C" fn(*mut Transfer) -> c_int;

struct Api {
    open_by_serial: unsafe extern "C" fn(*const c_char, *mut Dev) -> c_int,
    close: unsafe extern "C" fn(Dev) -> c_int,
    start_rx: unsafe extern "C" fn(Dev, RxCb, *mut c_void) -> c_int,
    stop_rx: unsafe extern "C" fn(Dev) -> c_int,
    is_streaming: unsafe extern "C" fn(Dev) -> c_int,
    set_freq: unsafe extern "C" fn(Dev, u64) -> c_int,
    set_sample_rate: unsafe extern "C" fn(Dev, f64) -> c_int,
    set_baseband_filter_bandwidth: unsafe extern "C" fn(Dev, u32) -> c_int,
    set_lna_gain: unsafe extern "C" fn(Dev, u32) -> c_int,
    set_vga_gain: unsafe extern "C" fn(Dev, u32) -> c_int,
    set_amp_enable: unsafe extern "C" fn(Dev, u8) -> c_int,
    error_name: unsafe extern "C" fn(c_int) -> *const c_char,
    board_id_read: unsafe extern "C" fn(Dev, *mut u8) -> c_int,
    usb_api_version_read: Option<unsafe extern "C" fn(Dev, *mut u16) -> c_int>,
    get_m0_state: Option<unsafe extern "C" fn(Dev, *mut M0State) -> c_int>,
    max2837_read: unsafe extern "C" fn(Dev, u8, *mut u16) -> c_int,
    max2837_write: unsafe extern "C" fn(Dev, u8, u16) -> c_int,
}

const LIB_NAMES: [&str; 2] = ["libhackrf.so.0", "libhackrf.so"];

fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(load).as_ref().map_err(Clone::clone)
}

/// Whether libhackrf can be used in this process.
pub fn loaded() -> Result<(), String> {
    api().map(|_| ())
}

fn load() -> Result<Api, String> {
    let mut handle = std::ptr::null_mut();
    for name in LIB_NAMES {
        let c = CString::new(name).expect("lib name");
        handle = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if !handle.is_null() {
            break;
        }
    }
    if handle.is_null() {
        return Err("libhackrf is not installed (libhackrf.so.0)".into());
    }
    let sym = |name: &str| -> Option<*mut c_void> {
        let c = CString::new(name).expect("symbol name");
        let p = unsafe { libc::dlsym(handle, c.as_ptr()) };
        (!p.is_null()).then_some(p)
    };
    macro_rules! need {
        ($name:literal) => {
            match sym($name) {
                Some(p) => unsafe { std::mem::transmute::<*mut c_void, _>(p) },
                None => return Err(format!("libhackrf has no {}", $name)),
            }
        };
    }
    macro_rules! maybe {
        ($name:literal) => {
            sym($name).map(|p| unsafe { std::mem::transmute::<*mut c_void, _>(p) })
        };
    }
    let init: unsafe extern "C" fn() -> c_int = need!("hackrf_init");
    let api = Api {
        open_by_serial: need!("hackrf_open_by_serial"),
        close: need!("hackrf_close"),
        start_rx: need!("hackrf_start_rx"),
        stop_rx: need!("hackrf_stop_rx"),
        is_streaming: need!("hackrf_is_streaming"),
        set_freq: need!("hackrf_set_freq"),
        set_sample_rate: need!("hackrf_set_sample_rate"),
        set_baseband_filter_bandwidth: need!("hackrf_set_baseband_filter_bandwidth"),
        set_lna_gain: need!("hackrf_set_lna_gain"),
        set_vga_gain: need!("hackrf_set_vga_gain"),
        set_amp_enable: need!("hackrf_set_amp_enable"),
        error_name: need!("hackrf_error_name"),
        board_id_read: need!("hackrf_board_id_read"),
        usb_api_version_read: maybe!("hackrf_usb_api_version_read"),
        get_m0_state: maybe!("hackrf_get_m0_state"),
        max2837_read: need!("hackrf_max2837_read"),
        max2837_write: need!("hackrf_max2837_write"),
    };
    // Once per process. hackrf_exit is never called: other handles may be open.
    let rc = unsafe { init() };
    if rc != 0 {
        return Err(format!("hackrf_init: {}", err_name(&api, rc)));
    }
    Ok(api)
}

fn err_name(api: &Api, rc: c_int) -> String {
    let p = unsafe { (api.error_name)(rc) };
    if p.is_null() {
        return format!("error {rc}");
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn check(api: &Api, what: &str, rc: c_int) -> Result<(), String> {
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("{what}: {}", err_name(api, rc)))
    }
}

/// The receive chip on this board, from `hackrf_board_id_read`.
/// Only HackRF One before r9 is known to carry the MAX2837.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RxChip {
    Max2837,
    /// HackRF One r9 and later.
    Max2839,
    /// Praline.
    Max2831,
    Unknown(u8),
}

pub fn chip_for_board(board_id: u8) -> RxChip {
    match board_id {
        // Jawbreaker and HackRF One r1-r8.
        1 | 2 => RxChip::Max2837,
        4 => RxChip::Max2839,
        5 => RxChip::Max2831,
        other => RxChip::Unknown(other),
    }
}

/// MAX2837 temperature ADC code (register 7, D4:D0) to degrees C.
/// Datasheet points: 1 = -40 C, 15 = 25 C, 29 = 85 C, about 4.4 C a step.
/// Codes outside 1..=29 are not readings and give `None`.
pub fn max2837_temp_c(code: u8) -> Option<f32> {
    if !(1..=29).contains(&code) {
        return None;
    }
    Some(25.0 + (code as f32 - 15.0) * 4.4)
}

/// The 5-bit code in a register 7 read.
pub fn temp_code(reg7: u16) -> u8 {
    (reg7 & 0x1f) as u8
}

/// One die-temperature attempt. `code` and `c` are `None` when there is no
/// trustworthy reading; `note` says why.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DieTemp {
    pub code: Option<u8>,
    pub c: Option<f32>,
    pub src: String,
    pub note: String,
}

struct RxCtx {
    tx: SyncSender<(Instant, Vec<u8>)>,
    /// IQ samples (byte pairs) lost because the reader was full.
    dropped: AtomicU64,
    stopping: AtomicBool,
}

extern "C" fn rx_callback(transfer: *mut Transfer) -> c_int {
    // SAFETY: libhackrf passes a valid transfer for the length of the call,
    // and rx_ctx is the RxCtx boxed in Device, alive until after stop_rx.
    let transfer = unsafe { &*transfer };
    let ctx = unsafe { &*(transfer.rx_ctx as *const RxCtx) };
    if ctx.stopping.load(Ordering::Relaxed) {
        return -1;
    }
    let n = transfer.valid_length.max(0) as usize;
    if n == 0 || transfer.buffer.is_null() {
        return 0;
    }
    let data = unsafe { std::slice::from_raw_parts(transfer.buffer, n) }.to_vec();
    match ctx.tx.try_send((Instant::now(), data)) {
        Ok(()) => 0,
        Err(TrySendError::Full((_, data))) => {
            ctx.dropped.fetch_add(data.len() as u64 / 2, Ordering::Relaxed);
            0
        }
        Err(TrySendError::Disconnected(_)) => -1,
    }
}

/// USB buffers waiting for the channelizer: 64 x 256 KiB is about 1 s.
const RX_QUEUE: usize = 64;

/// One open HackRF, streaming. Control calls are serialized.
pub struct Device {
    api: &'static Api,
    dev: Dev,
    ctx: *mut RxCtx,
    lock: Mutex<()>,
    pub board_id: u8,
    pub usb_api: Option<u16>,
    closed: AtomicBool,
}

// SAFETY: libhackrf handles are used from several threads by hackrf_transfer
// itself (the callback runs on libusb's thread). Control calls here hold `lock`.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

/// Gains to apply at open and on change. `None` leaves that stage alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gains {
    pub amp: Option<bool>,
    pub lna_db: Option<u32>,
    pub vga_db: Option<u32>,
}

impl Device {
    /// Open by serial (empty = first), set rate, filter, frequency and gain,
    /// and start receiving. Samples arrive on the returned reader.
    pub fn open(
        serial: &str,
        sample_rate: f64,
        filter_hz: u32,
        center_hz: u64,
        gains: Gains,
    ) -> Result<(Arc<Device>, RxReader), String> {
        let api = api()?;
        let mut dev: Dev = std::ptr::null_mut();
        let serial = serial.trim();
        let c_serial = CString::new(serial).map_err(|_| "bad HackRF serial".to_string())?;
        let ptr = if serial.is_empty() { std::ptr::null() } else { c_serial.as_ptr() };
        check(api, "hackrf open", unsafe { (api.open_by_serial)(ptr, &mut dev) })?;
        let (tx, rx) = sync_channel(RX_QUEUE);
        let ctx = Box::into_raw(Box::new(RxCtx {
            tx,
            dropped: AtomicU64::new(0),
            stopping: AtomicBool::new(false),
        }));
        let mut board_id = 0xFFu8;
        unsafe { (api.board_id_read)(dev, &mut board_id) };
        let usb_api = api.usb_api_version_read.and_then(|f| {
            let mut v = 0u16;
            (unsafe { f(dev, &mut v) } == 0).then_some(v)
        });
        let device = Arc::new(Device {
            api,
            dev,
            ctx,
            lock: Mutex::new(()),
            board_id,
            usb_api,
            closed: AtomicBool::new(false),
        });
        let setup = || -> Result<(), String> {
            check(api, "hackrf sample rate", unsafe { (api.set_sample_rate)(dev, sample_rate) })?;
            check(api, "hackrf filter", unsafe { (api.set_baseband_filter_bandwidth)(dev, filter_hz) })?;
            check(api, "hackrf frequency", unsafe { (api.set_freq)(dev, center_hz) })?;
            device.set_gains(gains)?;
            check(api, "hackrf start", unsafe { (api.start_rx)(dev, rx_callback, ctx as *mut c_void) })
        };
        if let Err(err) = setup() {
            device.close();
            return Err(err);
        }
        Ok((
            Arc::clone(&device),
            RxReader {
                rx,
                cur: Vec::new(),
                off: 0,
                cutoff: None,
            },
        ))
    }

    pub fn set_freq(&self, hz: u64) -> Result<(), String> {
        let _g = self.lock.lock().expect("hackrf lock");
        check(self.api, "hackrf retune", unsafe { (self.api.set_freq)(self.dev, hz) })
    }

    pub fn set_gains(&self, gains: Gains) -> Result<(), String> {
        let _g = self.lock.lock().expect("hackrf lock");
        if let Some(amp) = gains.amp {
            check(self.api, "hackrf amp", unsafe { (self.api.set_amp_enable)(self.dev, amp as u8) })?;
        }
        if let Some(lna) = gains.lna_db {
            check(self.api, "hackrf LNA", unsafe { (self.api.set_lna_gain)(self.dev, lna) })?;
        }
        if let Some(vga) = gains.vga_db {
            check(self.api, "hackrf VGA", unsafe { (self.api.set_vga_gain)(self.dev, vga) })?;
        }
        Ok(())
    }

    pub fn streaming(&self) -> bool {
        if self.closed.load(Ordering::Relaxed) {
            return false;
        }
        let _g = self.lock.lock().expect("hackrf lock");
        unsafe { (self.api.is_streaming)(self.dev) == 1 }
    }

    /// IQ samples lost between the USB thread and the channelizer.
    pub fn handoff_dropped(&self) -> u64 {
        unsafe { &*self.ctx }.dropped.load(Ordering::Relaxed)
    }

    /// M0 shortfalls: times the USB host did not take a buffer in time.
    /// `None` when the firmware does not report it.
    pub fn usb_overruns(&self) -> Option<u64> {
        let f = self.api.get_m0_state?;
        let _g = self.lock.lock().expect("hackrf lock");
        let mut st = M0State::default();
        (unsafe { f(self.dev, &mut st) } == 0).then_some(st.num_shortfalls as u64)
    }

    pub fn chip(&self) -> RxChip {
        chip_for_board(self.board_id)
    }

    /// Die temperature through the MAX2837 temperature ADC. Writes register
    /// 9 (TS_EN, then the ADC trigger) during receive and restores it.
    /// Only on a board known to carry the MAX2837, and only when the caller
    /// has opted in (decisions.md, "HackRF die temperature").
    pub fn die_temp(&self) -> DieTemp {
        let mut out = DieTemp {
            src: "max2837-die".into(),
            ..DieTemp::default()
        };
        if self.chip() != RxChip::Max2837 {
            out.src = "none".into();
            out.note = format!("receive chip {:?} has no supported sensor", self.chip());
            return out;
        }
        let _g = self.lock.lock().expect("hackrf lock");
        let (read, write) = (self.api.max2837_read, self.api.max2837_write);
        let mut r9 = 0u16;
        if unsafe { read(self.dev, 9, &mut r9) } != 0 {
            out.note = "register 9 read failed".into();
            return out;
        }
        let restore = r9 & !0b11;
        let steps = [restore | 0b10, restore | 0b11];
        let mut ok = unsafe { write(self.dev, 9, steps[0]) } == 0;
        if ok {
            std::thread::sleep(Duration::from_millis(1));
            ok = unsafe { write(self.dev, 9, steps[1]) } == 0;
            std::thread::sleep(Duration::from_micros(50));
        }
        let mut r7 = 0u16;
        let read_ok = ok && unsafe { read(self.dev, 7, &mut r7) } == 0;
        let restored = unsafe { write(self.dev, 9, r9) } == 0;
        if !restored {
            out.note = "register 9 restore failed".into();
            return out;
        }
        if !read_ok {
            out.note = "register 7 read failed".into();
            return out;
        }
        let code = temp_code(r7);
        out.code = Some(code);
        out.c = max2837_temp_c(code);
        if out.c.is_none() {
            out.note = format!("code {code} is not a reading (firmware may return its cached register)");
        }
        out
    }

    /// Stop the stream and close. Safe to call twice.
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let _g = self.lock.lock().expect("hackrf lock");
        unsafe {
            (*self.ctx).stopping.store(true, Ordering::Relaxed);
            (self.api.stop_rx)(self.dev);
            (self.api.close)(self.dev);
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        self.close();
        // SAFETY: the stream is stopped and the handle closed, so the
        // callback no longer holds this pointer.
        drop(unsafe { Box::from_raw(self.ctx) });
    }
}

/// `Read` over the USB buffers, in the byte order `hackrf_transfer` writes.
/// A quiet stream returns `TimedOut` so the reader can check its stop flag.
pub struct RxReader {
    rx: Receiver<(Instant, Vec<u8>)>,
    cur: Vec<u8>,
    off: usize,
    cutoff: Option<Instant>,
}

impl RxReader {
    /// Discard buffers that arrived before `at` (samples from the old
    /// frequency still in flight after a retune).
    pub fn drop_before(&mut self, at: Instant) {
        self.cutoff = Some(at);
        self.cur.clear();
        self.off = 0;
    }
}

impl Read for RxReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.off >= self.cur.len() {
            match self.rx.recv_timeout(Duration::from_millis(250)) {
                Ok((at, data)) => {
                    if self.cutoff.is_some_and(|cut| at < cut) {
                        continue;
                    }
                    self.cutoff = None;
                    self.cur = data;
                    self.off = 0;
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "no samples"));
                }
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
        let n = buf.len().min(self.cur.len() - self.off);
        buf[..n].copy_from_slice(&self.cur[self.off..self.off + n]);
        self.off += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_codes_follow_the_datasheet_points() {
        assert_eq!(max2837_temp_c(1), Some(25.0 - 14.0 * 4.4));
        assert_eq!(max2837_temp_c(15), Some(25.0));
        assert!((max2837_temp_c(29).unwrap() - 86.6).abs() < 0.01);
        assert_eq!(max2837_temp_c(0), None);
        assert_eq!(max2837_temp_c(30), None);
        assert_eq!(max2837_temp_c(31), None);
        // Only D4:D0 of register 7 are the code.
        assert_eq!(temp_code(0x3ef), 15);
        assert_eq!(temp_code(0x000), 0);
        assert_eq!(temp_code(0x01f), 31);
    }

    /// Every symbol this file uses resolves in the installed library.
    /// Loading does not open a dongle (hackrf_init only starts libusb).
    #[test]
    fn the_installed_library_has_every_call() {
        let installed = ["/usr/lib/x86_64-linux-gnu/libhackrf.so.0", "/usr/lib/libhackrf.so.0", "/usr/local/lib/libhackrf.so.0"]
            .iter()
            .any(|p| std::path::Path::new(p).exists());
        match loaded() {
            Ok(()) => {}
            Err(err) => assert!(!installed, "libhackrf is installed but did not load: {err}"),
        }
    }

    #[test]
    fn only_a_known_max2837_board_reads_its_die() {
        assert_eq!(chip_for_board(2), RxChip::Max2837);
        assert_eq!(chip_for_board(1), RxChip::Max2837);
        assert_eq!(chip_for_board(4), RxChip::Max2839);
        assert_eq!(chip_for_board(5), RxChip::Max2831);
        assert_eq!(chip_for_board(0xFE), RxChip::Unknown(0xFE));
        assert_eq!(chip_for_board(0xFF), RxChip::Unknown(0xFF));
    }

    #[test]
    fn the_reader_gives_the_bytes_in_order_and_drops_stale_buffers() {
        let (tx, rx) = sync_channel(8);
        let mut reader = RxReader { rx, cur: Vec::new(), off: 0, cutoff: None };
        let t0 = Instant::now();
        tx.send((t0, vec![1, 2, 3])).unwrap();
        let mut buf = [0u8; 2];
        assert_eq!(reader.read(&mut buf).unwrap(), 2);
        assert_eq!(buf, [1, 2]);
        assert_eq!(reader.read(&mut buf).unwrap(), 1);
        assert_eq!(buf[0], 3);
        tx.send((t0, vec![9, 9])).unwrap();
        let later = t0 + Duration::from_millis(5);
        tx.send((later, vec![7, 8])).unwrap();
        reader.drop_before(later);
        assert_eq!(reader.read(&mut buf).unwrap(), 2);
        assert_eq!(buf, [7, 8]);
        drop(tx);
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
    }
}
