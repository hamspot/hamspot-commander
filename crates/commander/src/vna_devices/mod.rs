//! Scan any RF device on the patch panel with the VNA and keep its scans
//! (#219). Pure logic here; the wizard window is in `ui`.

pub mod kinds;
pub mod ui;

use crate::patchbay::model::{Device, Kind, PatchBay, Signal};
use crate::tools::nanovna::StoredScan;
use crate::tools::nanovna::rf::{self, Cx, Sample};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- plan

/// One connection step: `ch0` on VNA port 1 (CH0, S11), `ch1` on VNA port 2
/// (CH1, S21) when the step measures transmission, every port in `loads`
/// terminated in a 50 Ω load.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairStep {
    pub ch0: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ch1: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loads: Vec<String>,
}

impl PairStep {
    /// The text equivalent of the wiring picture.
    pub fn text(&self) -> String {
        let mut s = format!("Connect {} to VNA PORT 1 (CH0)", self.ch0);
        if let Some(b) = &self.ch1 {
            s += &format!("; connect {b} to VNA PORT 2 (CH1)");
        } else {
            s += "; VNA PORT 2 (CH1) unused";
        }
        if !self.loads.is_empty() {
            s += &format!("; terminate {} with 50 Ω loads", self.loads.join(", "));
        }
        s + "."
    }

    pub fn measures(&self) -> String {
        match &self.ch1 {
            Some(b) => format!("S11 at {} and S21 {} → {}", self.ch0, self.ch0, b),
            None => format!("S11 at {}", self.ch0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanKind {
    S11,
    /// Every unordered pair, then S11 at any port never on CH0.
    AllPairs,
    /// Every ordered pair (non-reciprocal devices).
    AllOrdered,
    /// The first port to each of the others (switches, multicouplers).
    FromFirst,
}

impl PlanKind {
    pub fn parse(s: &str) -> PlanKind {
        match s {
            "s11" => PlanKind::S11,
            "all_ordered" => PlanKind::AllOrdered,
            "from_first" => PlanKind::FromFirst,
            _ => PlanKind::AllPairs,
        }
    }
}

pub fn plan(ports: &[String], kind: PlanKind) -> Vec<PairStep> {
    let others = |keep: &[&String]| -> Vec<String> { ports.iter().filter(|p| !keep.contains(p)).cloned().collect() };
    let pair = |a: &String, b: &String| PairStep { ch0: a.clone(), ch1: Some(b.clone()), loads: others(&[a, b]) };
    let mut out = Vec::new();
    if ports.len() < 2 || kind == PlanKind::S11 {
        for p in ports {
            out.push(PairStep { ch0: p.clone(), ch1: None, loads: others(&[p]) });
        }
        return out;
    }
    match kind {
        PlanKind::S11 => {}
        PlanKind::AllPairs => {
            for i in 0..ports.len() {
                for j in i + 1..ports.len() {
                    out.push(pair(&ports[i], &ports[j]));
                }
            }
            for p in ports {
                if !out.iter().any(|s| &s.ch0 == p) {
                    out.push(PairStep { ch0: p.clone(), ch1: None, loads: others(&[p]) });
                }
            }
        }
        PlanKind::AllOrdered => {
            for a in ports {
                for b in ports {
                    if a != b {
                        out.push(pair(a, b));
                    }
                }
            }
        }
        PlanKind::FromFirst => {
            for b in &ports[1..] {
                out.push(pair(&ports[0], b));
            }
        }
    }
    out
}

/// Coax ports a scan of this device can use.
pub fn rf_ports(dev: &Device) -> Vec<String> {
    dev.ports().into_iter().filter(|p| p.signal == Signal::Coax).map(|p| p.name).collect()
}

/// The scan plan for one device. A radio is scanned one jack at a time
/// (`jack`, default its first), as a 1-port.
pub fn plan_for(dev: &Device, jack: Option<&str>) -> Vec<PairStep> {
    let ports = rf_ports(dev);
    match dev.kind {
        Kind::Radio => {
            let j = jack.map(str::to_string).or_else(|| ports.first().cloned());
            j.into_iter().map(|j| PairStep { ch0: j, ch1: None, loads: Vec::new() }).collect()
        }
        Kind::Antenna | Kind::Load => plan(&ports, PlanKind::S11),
        Kind::Switch => plan(&ports, PlanKind::FromFirst),
        Kind::Rf => plan(&ports, kinds::plan_of(&dev.rf)),
        _ => plan(&ports, PlanKind::AllPairs),
    }
}

// ---------------------------------------------------------------- safety

/// Radios, amplifiers and table kinds marked `tx` can put out power by
/// themselves: an explicit "off / not transmitting" confirmation is needed.
pub fn needs_off_confirm(dev: &Device) -> bool {
    matches!(dev.kind, Kind::Radio | Kind::Amplifier) || (dev.kind == Kind::Rf && kinds::is_tx(&dev.rf))
}

/// True when the device can transmit or the panel's coax cables connect it,
/// through any chain of devices, to a transmitting radio, an amplifier or a
/// transmit-capable device. Scans of these hold the TX lockout.
pub fn in_tx_path(bay: &PatchBay, id: u32) -> bool {
    let is_tx = |d: &Device| (d.kind == Kind::Radio && d.tx) || d.kind == Kind::Amplifier || (d.kind == Kind::Rf && kinds::is_tx(&d.rf));
    let Some(start) = bay.device(id) else { return false };
    if needs_off_confirm(start) {
        return true;
    }
    let mut seen = vec![id];
    let mut todo = vec![id];
    while let Some(cur) = todo.pop() {
        for c in &bay.cables {
            let coax = bay.device(c.from.device).and_then(|d| d.port(&c.from.port)).map(|p| p.signal) == Some(Signal::Coax);
            if !coax {
                continue;
            }
            let next = if c.from.device == cur {
                c.to.device
            } else if c.to.device == cur {
                c.from.device
            } else {
                continue;
            };
            if seen.contains(&next) {
                continue;
            }
            if bay.device(next).is_some_and(is_tx) {
                return true;
            }
            seen.push(next);
            todo.push(next);
        }
    }
    false
}

pub const RADIO_WARNING: &str = "Radio must be OFF or not transmitting; transmitting into a VNA port can damage it. Scanning a powered-off radio jack shows the impedance its input presents.";

pub const LOCK_KEY: &str = "vna-scan";

/// Engage the lockout (when needed) before any sweep is asked for. The sweep
/// may start only on Ok.
pub fn arm_sweep(lock: &mut crate::tx_lockout::TxLockout, tx_path: bool, device: &str, connected: bool) -> Result<(), String> {
    if !connected {
        return Err("NO VNA CONNECTED. Connect the NanoVNA on the Tools · NanoVNA page, then scan.".into());
    }
    if tx_path {
        lock.engage(LOCK_KEY, &format!("VNA scan in progress on {device}"));
        if !lock.holds(LOCK_KEY) {
            return Err("TX lockout could not be engaged; scan not started.".into());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- sweep wait

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poll {
    Wait,
    Done,
    /// The VNA went away mid-sweep: state unknown.
    Lost,
}

/// `need` is the first sweep generation that started after the request.
pub fn poll(need: u64, connected: bool, generation: u64, has_sweep: bool) -> Poll {
    if !connected {
        Poll::Lost
    } else if generation >= need && has_sweep {
        Poll::Done
    } else {
        Poll::Wait
    }
}

// ---------------------------------------------------------------- triple click

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxClick {
    None,
    Select,
    OpenVna,
}

/// A device box: single click selects, triple click opens the VNA scan.
/// A double click has no meaning on a box, so it only selects.
pub fn box_click(clicked: bool, double: bool, triple: bool) -> BoxClick {
    if triple {
        BoxClick::OpenVna
    } else if clicked || double {
        BoxClick::Select
    } else {
        BoxClick::None
    }
}

// ---------------------------------------------------------------- files

pub const FORMAT: &str = "HSVNAScan";
pub const VERSION: u32 = 1;
pub const EXT: &str = "HSVNAScan";

/// `Foo.HSCommander` → `Foo.HSData/VNA`. Derived from the station file name
/// only; nothing stored refers to this path.
pub fn vna_dir(station: &Path) -> PathBuf {
    station.with_extension("HSData").join("VNA")
}

fn check_uid(uid: &str) -> Result<(), String> {
    if uid.is_empty() || !uid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("device id {uid:?} is not usable as a folder name"));
    }
    Ok(())
}

pub fn device_dir(station: &Path, uid: &str) -> Result<PathBuf, String> {
    check_uid(uid)?;
    Ok(vna_dir(station).join(uid))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepResult {
    pub step: PairStep,
    /// Uncorrected S11/S21 pairs, as the NanoVNA reported them.
    pub raw: StoredScan,
    /// Calibration-corrected pairs, when a calibration applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corrected: Option<StoredScan>,
}

/// One scan set of one device: every connection step, saved together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanSet {
    pub format: String,
    pub version: u32,
    pub device_uid: String,
    pub device_name: String,
    pub device_kind: String,
    pub ports: Vec<String>,
    /// UTC, RFC 3339, to the second.
    pub taken_at: String,
    pub start_hz: f64,
    pub stop_hz: f64,
    pub points: u16,
    #[serde(default)]
    pub vna_id: String,
    #[serde(default)]
    pub cal_note: String,
    /// Who/what took it: "HamSpot Commander <version>".
    #[serde(default)]
    pub by: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub length_m: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub vf: f64,
    pub steps: Vec<StepResult>,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

pub fn file_stem(taken_at: &str) -> String {
    taken_at.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
}

/// Write a new scan file. Never replaces an existing file: a clash gets
/// `-2`, `-3`… The text goes to a temp file first and is hard-linked into
/// place, so a crash never leaves half a scan and a link can not overwrite.
pub fn save_new(station: &Path, set: &ScanSet) -> Result<PathBuf, String> {
    if set.version != VERSION || set.format != FORMAT {
        return Err("refusing to write a scan of another format version".into());
    }
    if set.steps.is_empty() {
        return Err("scan has no measured steps; nothing saved".into());
    }
    let dir = device_dir(station, &set.device_uid)?;
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(set).map_err(|e| e.to_string())? + "\n";
    let tmp = dir.join(format!(".{}.{}.tmp", file_stem(&set.taken_at), std::process::id()));
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let stem = file_stem(&set.taken_at);
    let mut result = Err("no free file name".to_string());
    for n in 1..1000 {
        let name = if n == 1 { format!("{stem}.{EXT}") } else { format!("{stem}-{n}.{EXT}") };
        let path = dir.join(name);
        match fs::hard_link(&tmp, &path) {
            Ok(()) => {
                result = Ok(path);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                result = Err(format!("{}: {e}", path.display()));
                break;
            }
        }
    }
    let _ = fs::remove_file(&tmp);
    result
}

/// Read one scan file. A newer version is refused in plain words and the file
/// is left as it is.
pub fn parse(text: &str) -> Result<ScanSet, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("not a scan file: {e}"))?;
    if v.get("format").and_then(|f| f.as_str()) != Some(FORMAT) {
        return Err("not an HSVNAScan file".into());
    }
    let ver = v.get("version").and_then(|f| f.as_u64()).unwrap_or(0);
    if ver > VERSION as u64 {
        return Err(format!("made by a newer Commander (format version {ver}); left unchanged"));
    }
    if ver != VERSION as u64 {
        return Err(format!("unknown format version {ver}; left unchanged"));
    }
    serde_json::from_value(v).map_err(|e| format!("damaged scan file: {e}"))
}

pub struct Listed {
    pub file: String,
    pub set: Result<ScanSet, String>,
}

/// Every scan file of one device, newest first. Unreadable files are listed
/// with their error, after the readable ones.
pub fn history(station: &Path, uid: &str) -> Result<Vec<Listed>, String> {
    let dir = device_dir(station, uid)?;
    let rd = match fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || !name.ends_with(&format!(".{EXT}")) {
            continue;
        }
        let set = fs::read_to_string(ent.path()).map_err(|e| e.to_string()).and_then(|t| parse(&t));
        out.push(Listed { file: name, set });
    }
    out.sort_by(|a, b| match (&a.set, &b.set) {
        (Ok(x), Ok(y)) => y.taken_at.cmp(&x.taken_at).then(b.file.cmp(&a.file)),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        _ => a.file.cmp(&b.file),
    });
    Ok(out)
}

pub fn delete_scan(station: &Path, uid: &str, file: &str) -> Result<(), String> {
    if file.contains('/') || file.contains('\\') || file.starts_with('.') || !file.ends_with(&format!(".{EXT}")) {
        return Err(format!("{file:?} is not a scan file name"));
    }
    let path = device_dir(station, uid)?.join(file);
    fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Marker kept while a lockout-holding scan runs. Found at start-up, it means
/// the last scan did not finish cleanly.
pub fn lock_marker(station: &Path) -> PathBuf {
    vna_dir(station).join("tx-lockout.json")
}

pub fn write_marker(station: &Path, device: &str) -> Result<(), String> {
    let p = lock_marker(station);
    if let Some(d) = p.parent() {
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let text = serde_json::json!({"format": "HSVNALock", "version": 1, "device": device, "since": crate::tools::nanovna::cal_stamp_now()}).to_string();
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &p).map_err(|e| format!("{}: {e}", p.display()))
}

pub fn clear_marker(station: &Path) {
    let _ = fs::remove_file(lock_marker(station));
}

/// The device named in a left-over marker.
pub fn read_marker(station: &Path) -> Option<String> {
    let t = fs::read_to_string(lock_marker(station)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&t).unwrap_or_default();
    Some(v.get("device").and_then(|d| d.as_str()).unwrap_or("an unknown device").to_string())
}

// ---------------------------------------------------------------- summary

pub fn samples_of(scan: &StoredScan) -> Vec<Sample> {
    scan.s11
        .iter()
        .zip(scan.s21.iter())
        .enumerate()
        .map(|(i, (a, b))| Sample { hz: scan.start_hz + scan.step_hz * i as f64, s11: Cx::new(a[0], a[1]), s21: Cx::new(b[0], b[1]) })
        .collect()
}

pub fn stored(raw: &[Sample], vna_id: &str) -> Option<StoredScan> {
    if raw.len() < 2 {
        return None;
    }
    Some(StoredScan {
        id: if vna_id.is_empty() { "unknown".into() } else { vna_id.into() },
        zone: String::new(),
        start_hz: raw[0].hz,
        step_hz: raw[1].hz - raw[0].hz,
        s11: raw.iter().map(|s| [s.s11.re, s.s11.im]).collect(),
        s21: raw.iter().map(|s| [s.s21.re, s.s21.im]).collect(),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct S11Stats {
    pub min_swr: f64,
    pub at_hz: f64,
    /// Contiguous SWR < 2 span around the minimum.
    pub under2: Option<(f64, f64)>,
}

pub fn s11_stats(s: &[Sample]) -> Option<S11Stats> {
    let swr: Vec<f64> = s.iter().map(|x| rf::swr(x.s11)).collect();
    let (i, &min) = swr.iter().enumerate().filter(|(_, v)| v.is_finite()).min_by(|a, b| a.1.total_cmp(b.1))?;
    let under2 = (min < 2.0).then(|| {
        let mut lo = i;
        while lo > 0 && swr[lo - 1] < 2.0 {
            lo -= 1;
        }
        let mut hi = i;
        while hi + 1 < swr.len() && swr[hi + 1] < 2.0 {
            hi += 1;
        }
        (s[lo].hz, s[hi].hz)
    });
    Some(S11Stats { min_swr: min, at_hz: s[i].hz, under2 })
}

#[derive(Clone, Debug, PartialEq)]
pub struct S21Stats {
    pub best_db: f64,
    pub at_hz: f64,
    /// Contiguous span within 3 dB of the best point.
    pub bw3: (f64, f64),
}

pub fn s21_stats(s: &[Sample]) -> Option<S21Stats> {
    let db: Vec<f64> = s.iter().map(|x| rf::db(x.s21)).collect();
    let (i, &best) = db.iter().enumerate().filter(|(_, v)| v.is_finite()).max_by(|a, b| a.1.total_cmp(b.1))?;
    let mut lo = i;
    while lo > 0 && db[lo - 1] >= best - 3.0 {
        lo -= 1;
    }
    let mut hi = i;
    while hi + 1 < db.len() && db[hi + 1] >= best - 3.0 {
        hi += 1;
    }
    Some(S21Stats { best_db: best, at_hz: s[i].hz, bw3: (s[lo].hz, s[hi].hz) })
}

pub fn mhz(hz: f64) -> String {
    format!("{:.4} MHz", hz / 1e6)
}

/// Plain-text summary of one step (corrected values when present).
pub fn step_summary(r: &StepResult) -> Vec<String> {
    let s = samples_of(r.corrected.as_ref().unwrap_or(&r.raw));
    let tag = if r.corrected.is_some() { "" } else { " (UNCALIBRATED)" };
    let mut out = Vec::new();
    match s11_stats(&s) {
        Some(st) => out.push(format!(
            "S11 at {}{tag}: resonance (min SWR) {}:1 at {} · SWR<2 {}",
            r.step.ch0,
            crate::tools::nanovna::display::fmt_swr(st.min_swr, 2),
            mhz(st.at_hz),
            match st.under2 {
                Some((lo, hi)) => format!("{} – {} ({})", mhz(lo), mhz(hi), mhz(hi - lo)),
                None => "never under 2:1".into(),
            }
        )),
        None => out.push(format!("S11 at {}: no usable points", r.step.ch0)),
    }
    if let Some(b) = &r.step.ch1 {
        if let Some(st) = s21_stats(&s) {
            out.push(format!(
                "S21 {} → {b}{tag}: best {:.2} dB at {} · −3 dB span {} – {}",
                r.step.ch0,
                st.best_db,
                mhz(st.at_hz),
                mhz(st.bw3.0),
                mhz(st.bw3.1)
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests;
