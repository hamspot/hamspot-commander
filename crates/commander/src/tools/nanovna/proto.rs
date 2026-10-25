//! NanoVNA V2 binary protocol. Little-endian. No serial port here.
//!
//! The record layout and checksum are the ones in NanoVNA2-firmware
//! `cmdReadFIFO`: six little-endian i32 values, a frequency index, five
//! reserved bytes, and a checksum over the first 31 bytes.

pub const NOP: u8 = 0x00;
pub const INDICATE: u8 = 0x0d;
pub const READ1: u8 = 0x10;
/// Two-byte read. The host uses one-byte reads, four-byte reads, and the FIFO.
#[allow(dead_code)]
pub const READ2: u8 = 0x11;
pub const READ4: u8 = 0x12;
pub const READ_FIFO: u8 = 0x18;
pub const WRITE1: u8 = 0x20;
pub const WRITE2: u8 = 0x21;
pub const WRITE8: u8 = 0x23;

pub const ADDR_SWEEP_START: u8 = 0x00;
pub const ADDR_SWEEP_STEP: u8 = 0x10;
pub const ADDR_SWEEP_POINTS: u8 = 0x20;
pub const ADDR_VALUES_PER_FREQ: u8 = 0x22;
pub const ADDR_FIFO: u8 = 0x30;
pub const ADDR_AVERAGE: u8 = 0x40;
pub const ADDR_TX_POWER: u8 = 0x42;
pub const ADDR_DEVICE_ID: u8 = 0xd0;
pub const ADDR_CAPTURE: u8 = 0xee;
pub const ADDR_VARIANT: u8 = 0xf0;
pub const ADDR_PROTOCOL: u8 = 0xf1;
pub const ADDR_BOARD: u8 = 0xf2;
pub const ADDR_FW_MAJOR: u8 = 0xf3;
pub const ADDR_FW_MINOR: u8 = 0xf4;

pub const RECORD_LEN: usize = 32;
pub const FIFO_CHUNK: usize = 255;
/// Firmware `USB_POINTS_MAX` on a V2.2 with its ecal buffer compiled in.
pub const POINTS_MAX: usize = 1024;
pub const FREQ_MIN_HZ: u64 = 10_000;
pub const FREQ_FLOOR_HZ: u64 = 50_000;

/// Eight NOPs put the parser back at the start of a command.
pub fn resync() -> Vec<u8> {
    vec![NOP; 8]
}

pub fn read1(addr: u8) -> [u8; 2] {
    [READ1, addr]
}

pub fn read4(addr: u8) -> [u8; 2] {
    [READ4, addr]
}

pub fn write1(addr: u8, value: u8) -> [u8; 3] {
    [WRITE1, addr, value]
}

pub fn write2(addr: u8, value: u16) -> [u8; 4] {
    let b = value.to_le_bytes();
    [WRITE2, addr, b[0], b[1]]
}

pub fn write8(addr: u8, value: u64) -> [u8; 10] {
    let b = value.to_le_bytes();
    [
        WRITE8, addr, b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]
}

pub fn read_fifo(count: u8) -> [u8; 3] {
    [READ_FIFO, ADDR_FIFO, count]
}

/// Clear the measured-value FIFO. A write of any byte to 0x30 does that.
pub fn clear_fifo() -> [u8; 3] {
    write1(ADDR_FIFO, 0)
}

pub fn indicate() -> [u8; 1] {
    [INDICATE]
}

/// Registers 0xf0 through 0xf4, one byte each.
pub fn read_identity() -> Vec<u8> {
    let mut out = Vec::with_capacity(10);
    for addr in [
        ADDR_VARIANT,
        ADDR_PROTOCOL,
        ADDR_BOARD,
        ADDR_FW_MAJOR,
        ADDR_FW_MINOR,
    ] {
        out.extend(read1(addr));
    }
    out
}

/// Twelve-byte unique id at 0xd0, as three 4-byte reads.
pub fn read_device_id() -> Vec<u8> {
    let mut out = Vec::with_capacity(6);
    out.extend(read4(ADDR_DEVICE_ID));
    out.extend(read4(ADDR_DEVICE_ID.wrapping_add(4)));
    out.extend(read4(ADDR_DEVICE_ID.wrapping_add(8)));
    out
}

pub fn set_sweep(start_hz: u64, step_hz: u64, points: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(26);
    out.extend(write8(ADDR_SWEEP_START, start_hz));
    out.extend(write8(ADDR_SWEEP_STEP, step_hz));
    out.extend(write2(ADDR_SWEEP_POINTS, points));
    out.extend(write2(ADDR_VALUES_PER_FREQ, 1));
    out
}

pub fn set_average(count: u8) -> [u8; 3] {
    write1(ADDR_AVERAGE, count)
}

pub fn set_tx_power(level: u8) -> [u8; 3] {
    write1(ADDR_TX_POWER, level)
}

pub fn capture() -> [u8; 3] {
    write1(ADDR_CAPTURE, 0)
}

/// Checksum the firmware writes into byte 31.
///
/// The shift is the C integer promotion of a `uint8_t`, so the low 8 bits
/// are kept only when the result is stored.
pub fn record_checksum(body: &[u8]) -> u8 {
    let mut sum: u32 = 0b0100_0110;
    let n = body.len().min(31);
    for &byte in &body[..n] {
        sum = (sum ^ ((sum << 1) | 1) ^ u32::from(byte)) & 0xff;
    }
    sum as u8
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawRecord {
    pub fwd_re: i32,
    pub fwd_im: i32,
    pub rev0_re: i32,
    pub rev0_im: i32,
    pub rev1_re: i32,
    pub rev1_im: i32,
    pub freq_index: u16,
}

#[cfg(test)]
pub fn encode_record(rec: &RawRecord) -> [u8; RECORD_LEN] {
    let mut out = [0u8; RECORD_LEN];
    out[0..4].copy_from_slice(&rec.fwd_re.to_le_bytes());
    out[4..8].copy_from_slice(&rec.fwd_im.to_le_bytes());
    out[8..12].copy_from_slice(&rec.rev0_re.to_le_bytes());
    out[12..16].copy_from_slice(&rec.rev0_im.to_le_bytes());
    out[16..20].copy_from_slice(&rec.rev1_re.to_le_bytes());
    out[20..24].copy_from_slice(&rec.rev1_im.to_le_bytes());
    out[24..26].copy_from_slice(&rec.freq_index.to_le_bytes());
    out[31] = record_checksum(&out);
    out
}

pub fn decode_record(bytes: &[u8]) -> Result<RawRecord, String> {
    if bytes.len() != RECORD_LEN {
        return Err(format!("record is {} bytes", bytes.len()));
    }
    let expect = record_checksum(bytes);
    if bytes[31] != expect {
        return Err("record checksum does not match".into());
    }
    let i32_at = |off: usize| i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
    Ok(RawRecord {
        fwd_re: i32_at(0),
        fwd_im: i32_at(4),
        rev0_re: i32_at(8),
        rev0_im: i32_at(12),
        rev1_re: i32_at(16),
        rev1_im: i32_at(20),
        freq_index: u16::from_le_bytes(bytes[24..26].try_into().unwrap()),
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ratio {
    pub s11_re: f64,
    pub s11_im: f64,
    pub s21_re: f64,
    pub s21_im: f64,
}

/// S11 = rev0/fwd and S21 = rev1/fwd.
/// A zero forward wave is not a reflection coefficient.
pub fn ratios(rec: &RawRecord) -> Result<Ratio, String> {
    let fwd_re = rec.fwd_re as f64;
    let fwd_im = rec.fwd_im as f64;
    let den = fwd_re * fwd_re + fwd_im * fwd_im;
    if den == 0.0 {
        return Err(format!("frequency index {} has no forward wave", rec.freq_index));
    }
    let div = |re: i32, im: i32| {
        let re = re as f64;
        let im = im as f64;
        (
            (re * fwd_re + im * fwd_im) / den,
            (im * fwd_re - re * fwd_im) / den,
        )
    };
    let (s11_re, s11_im) = div(rec.rev0_re, rec.rev0_im);
    let (s21_re, s21_im) = div(rec.rev1_re, rec.rev1_im);
    Ok(Ratio {
        s11_re,
        s11_im,
        s21_re,
        s21_im,
    })
}

/// Place each record at its frequency index. A bad checksum, a repeated
/// index, or an index outside `points` rejects the chunk.
pub fn assemble(chunk: &[u8], points: usize) -> Result<Vec<Option<Ratio>>, String> {
    if points == 0 || points > POINTS_MAX {
        return Err(format!("point count {points} is outside 1..={POINTS_MAX}"));
    }
    if chunk.len() % RECORD_LEN != 0 {
        return Err("fifo chunk is not a whole number of records".into());
    }
    let mut out = vec![None; points];
    for frame in chunk.chunks(RECORD_LEN) {
        let rec = decode_record(frame)?;
        let index = rec.freq_index as usize;
        if index >= points {
            return Err(format!("frequency index {index} is outside 0..{points}"));
        }
        if out[index].is_some() {
            return Err(format!("frequency index {index} arrived twice"));
        }
        out[index] = Some(ratios(&rec)?);
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub variant: u8,
    pub protocol: u8,
    pub board: u8,
    pub fw_major: u8,
    pub fw_minor: u8,
}

impl Identity {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != 5 {
            return Err(format!("identity is {} bytes", bytes.len()));
        }
        Ok(Self {
            variant: bytes[0],
            protocol: bytes[1],
            board: bytes[2],
            fw_major: bytes[3],
            fw_minor: bytes[4],
        })
    }

    pub fn bootloader(self) -> bool {
        self.fw_major == 0xff
    }

    /// Firmware 1.0.1 and older shifts the first S21 point.
    pub fn s21_hack(self) -> bool {
        !self.bootloader() && (self.fw_major == 0 || (self.fw_major == 1 && self.fw_minor <= 1))
    }

    /// TX power and device averaging exist on 1.0.2 and later.
    pub fn power_and_average(self) -> bool {
        !self.bootloader() && (self.fw_major > 1 || (self.fw_major == 1 && self.fw_minor >= 2))
    }

    pub fn max_hz(self) -> f64 {
        if self.board >= 4 {
            4_400_000_000.0
        } else {
            3_000_000_000.0
        }
    }

    pub fn version_label(self) -> String {
        if self.bootloader() {
            format!("bootloader {}", self.fw_minor)
        } else {
            format!("{}.{}", self.fw_major, self.fw_minor)
        }
    }
}

pub fn parse_device_id(bytes: &[u8]) -> Result<[u8; 12], String> {
    if bytes.len() != 12 {
        return Err(format!("device id is {} bytes", bytes.len()));
    }
    let mut id = [0u8; 12];
    id.copy_from_slice(bytes);
    Ok(id)
}

pub fn device_id_hex(id: &[u8; 12]) -> String {
    id.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureMeta {
    pub width: u16,
    pub height: u16,
    pub pixel_format: u8,
}

pub fn parse_capture_meta(bytes: &[u8]) -> Result<CaptureMeta, String> {
    if bytes.len() != 5 {
        return Err(format!("capture header is {} bytes", bytes.len()));
    }
    let width = u16::from_le_bytes([bytes[0], bytes[1]]);
    let height = u16::from_le_bytes([bytes[2], bytes[3]]);
    let pixel_format = bytes[4];
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return Err(format!("capture size {width}x{height} is not usable"));
    }
    if pixel_format != 16 {
        return Err(format!("capture pixel format {pixel_format} is not 16"));
    }
    Ok(CaptureMeta {
        width,
        height,
        pixel_format,
    })
}

/// Programmed start and point count for one sweep.
///
/// The S21 workaround asks for one extra point, starting one step earlier,
/// and the caller drops the first point. It is skipped when the start is
/// already at the bottom of the range, because the step would fall off.
pub fn programmed_sweep(start_hz: u64, step_hz: u64, points: u16, s21_hack: bool) -> (u64, u16) {
    if s21_hack && start_hz > step_hz && (points as usize) + 1 <= POINTS_MAX {
        (start_hz - step_hz, points + 1)
    } else {
        (start_hz, points)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_of_a_zero_body_is_the_firmware_loop() {
        assert_eq!(record_checksum(&[0u8; 31]), 0x3d);
    }

    #[test]
    fn a_record_round_trips_and_a_bad_checksum_is_rejected() {
        let rec = RawRecord {
            fwd_re: 1_073_741_824,
            fwd_im: 0,
            rev0_re: 536_870_912,
            rev0_im: 0,
            rev1_re: 268_435_456,
            rev1_im: 0,
            freq_index: 3,
        };
        let bytes = encode_record(&rec);
        assert_eq!(bytes[31], 0x12);
        let back = decode_record(&bytes).unwrap();
        assert_eq!(back, rec);
        let ratio = ratios(&back).unwrap();
        assert!((ratio.s11_re - 0.5).abs() < 1e-12);
        assert!((ratio.s21_re - 0.25).abs() < 1e-12);
        let mut bad = bytes;
        bad[31] ^= 0xff;
        assert!(decode_record(&bad).is_err());
    }

    #[test]
    fn assemble_orders_by_frequency_index() {
        let mut chunk = Vec::new();
        for index in [1u16, 0] {
            chunk.extend(encode_record(&RawRecord {
                fwd_re: 1000,
                fwd_im: 0,
                rev0_re: i32::from(index) * 100,
                rev0_im: 0,
                rev1_re: 0,
                rev1_im: 0,
                freq_index: index,
            }));
        }
        let rows = assemble(&chunk, 2).unwrap();
        assert!((rows[0].unwrap().s11_re).abs() < 1e-12);
        assert!((rows[1].unwrap().s11_re - 0.1).abs() < 1e-12);
    }

    #[test]
    fn an_index_past_the_sweep_is_rejected() {
        let bytes = encode_record(&RawRecord {
            fwd_re: 1,
            fwd_im: 0,
            rev0_re: 0,
            rev0_im: 0,
            rev1_re: 0,
            rev1_im: 0,
            freq_index: 4,
        });
        assert!(assemble(&bytes, 4).is_err());
    }

    #[test]
    fn identity_flags_follow_the_version_bytes() {
        let id = Identity::parse(&[2, 1, 2, 1, 0]).unwrap();
        assert!(id.s21_hack());
        assert!(!id.power_and_average());
        assert!((id.max_hz() - 3.0e9).abs() < 1.0);
        let later = Identity::parse(&[2, 1, 4, 1, 2]).unwrap();
        assert!(!later.s21_hack());
        assert!(later.power_and_average());
        assert!((later.max_hz() - 4.4e9).abs() < 1.0);
        assert!(Identity::parse(&[2, 1, 2, 0xff, 1]).unwrap().bootloader());
    }

    #[test]
    fn the_s21_workaround_adds_one_earlier_point() {
        assert_eq!(programmed_sweep(1_000_000, 10_000, 101, true), (990_000, 102));
        assert_eq!(programmed_sweep(1_000_000, 10_000, 101, false), (1_000_000, 101));
        assert_eq!(programmed_sweep(10_000, 20_000, 11, true), (10_000, 11));
    }

    #[test]
    fn sweep_and_identity_commands_match_the_opcodes() {
        let cmd = set_sweep(123, 10, 101);
        assert_eq!(cmd[0], WRITE8);
        assert_eq!(cmd[1], ADDR_SWEEP_START);
        assert_eq!(&cmd[2..10], &123u64.to_le_bytes());
        assert_eq!(cmd[10], WRITE8);
        assert_eq!(cmd[11], ADDR_SWEEP_STEP);
        let ident = read_identity();
        assert_eq!(ident, vec![0x10, 0xf0, 0x10, 0xf1, 0x10, 0xf2, 0x10, 0xf3, 0x10, 0xf4]);
        assert_eq!(clear_fifo(), [WRITE1, ADDR_FIFO, 0]);
        assert_eq!(read_fifo(3), [READ_FIFO, ADDR_FIFO, 3]);
        assert_eq!(READ2, 0x11);
    }
}
