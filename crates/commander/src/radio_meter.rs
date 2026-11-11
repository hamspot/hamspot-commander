//! HackRF health readings for the Meters page and the drift log:
//! frequency error, software drops, USB overruns and die temperature.
//! The capture thread writes them; the page reads the latest copy.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Drift log line format. Bump on any field change (decisions.md,
/// "HackRF drift log").
pub const DRIFT_LOG_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RadioMeter {
    /// "libhackrf" or "hackrf_transfer".
    pub backend: String,
    /// Board and firmware as the radio reports them. Empty when unknown.
    pub radio: String,
    pub serial: String,
    pub center_hz: f64,
    /// Frequency error removed in software, Hz. 0 until `afc_locked`.
    pub afc_hz: f32,
    pub afc_locked: bool,
    /// 48 kHz decoder samples dropped because a decoder fell behind, total.
    pub decoder_dropped: u64,
    /// 8 Msps IQ samples dropped between USB and the channelizer, total.
    /// Always 0 on `hackrf_transfer`, which cannot report it.
    pub iq_dropped: u64,
    /// M0 shortfalls since the stream started. `None`: not reported.
    pub usb_overruns: Option<u64>,
    pub temp: crate::libhackrf::DieTemp,
    /// Last drift-log write failure. Empty when the last write worked.
    pub log_error: String,
}

impl RadioMeter {
    /// Frequency error in parts per million of the center.
    pub fn ppm(&self) -> Option<f64> {
        (self.afc_locked && self.center_hz > 0.0).then(|| self.afc_hz as f64 / self.center_hz * 1e6)
    }
}

fn table() -> &'static Mutex<HashMap<u64, RadioMeter>> {
    static T: OnceLock<Mutex<HashMap<u64, RadioMeter>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn note(radio: u64, meter: RadioMeter) {
    table().lock().expect("radio meters").insert(radio, meter);
}

pub fn get(radio: u64) -> Option<RadioMeter> {
    table().lock().expect("radio meters").get(&radio).cloned()
}

pub fn forget(radio: u64) {
    table().lock().expect("radio meters").remove(&radio);
}

fn drift_dir_slot() -> &'static Mutex<Option<PathBuf>> {
    static D: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(None))
}

/// `NAME.HSData/hackrf`, or none when no station file is open.
pub fn set_drift_dir(dir: Option<PathBuf>) {
    let mut slot = drift_dir_slot().lock().expect("drift dir");
    if *slot != dir {
        *slot = dir;
    }
}

pub fn drift_dir() -> Option<PathBuf> {
    drift_dir_slot().lock().expect("drift dir").clone()
}

/// File name for a UTC day: `drift-YYYY-MM-DD.jsonl`.
pub fn drift_file_name(utc: chrono::DateTime<chrono::Utc>) -> String {
    format!("drift-{}.jsonl", utc.format("%Y-%m-%d"))
}

/// One JSON line. No paths. `temp_c` is null until a reading is trusted.
pub fn drift_line(meter: &RadioMeter, utc: chrono::DateTime<chrono::Utc>) -> String {
    let value = serde_json::json!({
        "v": DRIFT_LOG_VERSION,
        "utc": utc.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "serial": meter.serial,
        "backend": meter.backend,
        "center_hz": meter.center_hz.round() as u64,
        "afc_hz": (meter.afc_hz as f64).round(),
        "afc_locked": meter.afc_locked,
        "ppm": meter.ppm().map(|p| (p * 1000.0).round() / 1000.0),
        "temp_code": meter.temp.code,
        "temp_c": meter.temp.c.map(|c| (c as f64 * 10.0).round() / 10.0),
        "temp_src": meter.temp.src,
        "decoder_dropped": meter.decoder_dropped,
        "iq_dropped": meter.iq_dropped,
        "usb_overruns": meter.usb_overruns,
    });
    value.to_string()
}

/// Append one line to the day's file in `dir`, creating both as needed.
pub fn append_drift(dir: &Path, meter: &RadioMeter, utc: chrono::DateTime<chrono::Utc>) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(drift_file_name(utc));
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    let mut line = drift_line(meter, utc);
    line.push('\n');
    file.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample() -> RadioMeter {
        RadioMeter {
            backend: "libhackrf".into(),
            radio: String::new(),
            serial: "0000000000000000a06063c8234e925f".into(),
            center_hz: 857_200_000.0,
            afc_hz: 2370.4,
            afc_locked: true,
            decoder_dropped: 12,
            iq_dropped: 0,
            usb_overruns: None,
            temp: crate::libhackrf::DieTemp {
                code: None,
                c: None,
                src: "off".into(),
                note: String::new(),
            },
            log_error: String::new(),
        }
    }

    #[test]
    fn a_drift_line_has_every_field_and_no_path() {
        let utc = chrono::Utc.with_ymd_and_hms(2026, 11, 10, 22, 5, 7).unwrap();
        assert_eq!(drift_file_name(utc), "drift-2026-11-10.jsonl");
        let line = drift_line(&sample(), utc);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["utc"], "2026-11-10T22:05:07Z");
        assert_eq!(v["serial"], "0000000000000000a06063c8234e925f");
        assert_eq!(v["center_hz"], 857_200_000u64);
        assert_eq!(v["afc_hz"], 2370.0);
        assert_eq!(v["afc_locked"], true);
        assert!((v["ppm"].as_f64().unwrap() - 2.765).abs() < 0.001);
        assert!(v["temp_code"].is_null());
        assert!(v["temp_c"].is_null());
        assert_eq!(v["temp_src"], "off");
        assert_eq!(v["decoder_dropped"], 12);
        assert_eq!(v["iq_dropped"], 0);
        assert!(v["usb_overruns"].is_null());
        assert!(!line.contains('/'), "no paths in the log: {line}");
        assert!(!line.contains('\n'));
    }

    #[test]
    fn ppm_is_empty_until_the_error_is_measured() {
        let mut m = sample();
        m.afc_locked = false;
        assert_eq!(m.ppm(), None);
        let utc = chrono::Utc.with_ymd_and_hms(2026, 11, 10, 0, 0, 0).unwrap();
        let v: serde_json::Value = serde_json::from_str(&drift_line(&m, utc)).unwrap();
        assert!(v["ppm"].is_null());
    }

    #[test]
    fn lines_append_to_the_day_file() {
        let dir = std::env::temp_dir().join(format!("hs-drift-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let utc = chrono::Utc.with_ymd_and_hms(2026, 11, 10, 1, 2, 3).unwrap();
        append_drift(&dir, &sample(), utc).unwrap();
        append_drift(&dir, &sample(), utc).unwrap();
        let text = std::fs::read_to_string(dir.join("drift-2026-11-10.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
