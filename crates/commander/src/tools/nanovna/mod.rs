//! NanoVNA V2 on the Tools tab. Issue 59.
//! The device is optional. Nothing is polled until Connect.

mod device;
mod cal;
mod display;
mod layout;
mod limits;
mod markers;
mod proto;
mod rf;
mod keys;
#[cfg(test)]
mod a11y;
mod calstd;
mod search;
mod sweep;
mod traces;

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, vec2};
use rustcars::{LcarsFrame, LcarsTheme, Side, action_button, palette, readout};
use serde::{Deserialize, Serialize};

use device::{ListedPort, Session};
use rf::{Cx, Sample};

const Z0_DEFAULT: f64 = 50.0;
const START_DEFAULT: f64 = 50_000_000.0;
const STOP_DEFAULT: f64 = 900_000_000.0;
const POINTS_DEFAULT: u16 = 101;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Smith,
    /// Admittance Smith chart (Y grid). Issue 64 / 58.
    SmithY,
    Polar,
    Swr,
    ReturnLoss,
    Impedance,
    S21,
    Phase,
    Unwrap,
    GroupDelay,
    Tdr,
    LinMag,
    ReIm,
    /// |Z| from S11. Issue 64.
    Zabs,
    PhaseS11,
    UnwrapS11,
    PolarS21,
    ReImS21,
    LinMagS21,
}

/// Every format with its key label. FORMAT shows the ones valid for the
/// selected parameter (S11 or S21). Issue 64.
const VIEWS: [(View, &str); 19] = [
    (View::Smith, "Smith"),
    (View::SmithY, "Smith Y"),
    (View::Polar, "Polar"),
    (View::Swr, "SWR"),
    (View::ReturnLoss, "Return loss"),
    (View::LinMag, "Lin mag"),
    (View::ReIm, "Real + imag"),
    (View::Impedance, "R + X"),
    (View::Zabs, "|Z|"),
    (View::PhaseS11, "Phase"),
    (View::UnwrapS11, "Unwrap"),
    (View::Tdr, "TDR"),
    (View::S21, "dB mag"),
    (View::Phase, "Phase"),
    (View::Unwrap, "Unwrap"),
    (View::GroupDelay, "Group delay"),
    (View::PolarS21, "Polar"),
    (View::ReImS21, "Real + imag"),
    (View::LinMagS21, "Lin mag"),
];

/// True when the format reads S21; the rest read S11.
fn uses_s21(view: View) -> bool {
    matches!(
        view,
        View::S21 | View::Phase | View::Unwrap | View::GroupDelay | View::PolarS21 | View::ReImS21 | View::LinMagS21
    )
}

/// Polar and Smith draw on a disk, not a Cartesian grid.
fn is_disk(view: View) -> bool {
    matches!(view, View::Smith | View::SmithY | View::Polar | View::PolarS21)
}

/// The parameter a format reads from one sample.
fn param_of(view: View, sample: &Sample) -> Cx {
    if uses_s21(view) { sample.s21 } else { sample.s11 }
}

thread_local! {
    /// Group-delay aperture in points, set from the face each frame. Issue 64.
    static GD_APERTURE: std::cell::Cell<usize> = const { std::cell::Cell::new(4) };
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Standard {
    Open,
    Short,
    Load,
    Thru,
    Isol,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    Start,
    Stop,
    Points,
    Z0,
    Velocity,
    Extension,
    HostAvg,
    Smooth,
}

impl Edit {
    fn get(self, ui: &NanoVna) -> String {
        match self {
            Edit::Start => ui.start.clone(),
            Edit::Stop => ui.stop.clone(),
            Edit::Points => ui.points.clone(),
            Edit::Z0 => ui.z0.clone(),
            Edit::Velocity => ui.velocity.clone(),
            Edit::Extension => ui.extension.clone(),
            Edit::HostAvg => ui.host_avg.clone(),
            Edit::Smooth => ui.smooth.clone(),
        }
    }

    fn set(self, ui: &mut NanoVna, text: String) {
        match self {
            Edit::Start => ui.start = text,
            Edit::Stop => ui.stop = text,
            Edit::Points => ui.points = text,
            Edit::Z0 => ui.z0 = text,
            Edit::Velocity => ui.velocity = text,
            Edit::Extension => ui.extension = text,
            Edit::HostAvg => ui.host_avg = text,
            Edit::Smooth => ui.smooth = text,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Run {
    Idle,
    Queued,
    Running,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalSet {
    pub id: String,
    /// Zone name from the station band plan. Empty on a calibration stored
    /// before bands were kept.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub zone: String,
    /// Band or subband name. A subband is `band / segment`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub band: String,
    /// Operator's name for the set. Empty shows the zone. Issue 65.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub start_hz: f64,
    pub step_hz: f64,
    pub z0: f64,
    pub open: Vec<[f64; 2]>,
    pub short: Vec<[f64; 2]>,
    pub load: Vec<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thru: Vec<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub isolation: Vec<[f64; 2]>,
    /// When the set was last measured: ISO 8601 UTC, e.g. `2026-10-25T22:40:00Z`.
    /// Sets saved before this field load as None and show "no date".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<String>,
}

/// ISO 8601 UTC stamp for a calibration set, to the second.
pub fn cal_stamp_now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Short form for badges and lists, in UTC to the minute: `2026-10-25T22:40Z`.
/// Accepts any RFC 3339 / ISO 8601 date-time with Z or an offset.
pub fn cal_date_text(saved_at: Option<&str>) -> String {
    match saved_at {
        None => "no date".into(),
        Some(s) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%MZ").to_string())
            .unwrap_or_else(|_| s.to_string()),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Face {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub port: String,
    #[serde(default = "default_start", skip_serializing_if = "is_default_start")]
    pub start_hz: f64,
    #[serde(default = "default_stop", skip_serializing_if = "is_default_stop")]
    pub stop_hz: f64,
    #[serde(default = "default_points", skip_serializing_if = "is_default_points")]
    pub points: u16,
    #[serde(default = "default_z0", skip_serializing_if = "is_default_z0")]
    pub z0: f64,
    #[serde(default = "default_velocity", skip_serializing_if = "is_default_velocity")]
    pub velocity: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub extension_m: f64,
    #[serde(default = "default_one", skip_serializing_if = "is_one")]
    pub host_average: u32,
    #[serde(default = "default_one", skip_serializing_if = "is_one")]
    pub smooth: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cals: Vec<CalSet>,
    /// Manual scales by trace format key. A format with no entry auto-scales. Issue 69.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub scales: std::collections::BTreeMap<String, display::ManualScale>,
    /// Group-delay aperture in points. Issue 64.
    #[serde(default = "default_aperture", skip_serializing_if = "is_default_aperture")]
    pub gd_aperture: u32,
    /// Pane grid and trace assignment. Issues 62 and 63.
    #[serde(default, skip_serializing_if = "layout::Layout::is_default")]
    pub layout: layout::Layout,
    /// Limit lines by trace format key. Issue 71.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub limits: std::collections::BTreeMap<String, limits::LimitSet>,
}

fn default_aperture() -> u32 {
    4
}
fn is_default_aperture(v: &u32) -> bool {
    *v == 4
}

fn default_start() -> f64 {
    START_DEFAULT
}
fn default_stop() -> f64 {
    STOP_DEFAULT
}
fn default_points() -> u16 {
    POINTS_DEFAULT
}
fn default_z0() -> f64 {
    Z0_DEFAULT
}
fn default_velocity() -> f64 {
    0.66
}
fn default_one() -> u32 {
    1
}
fn is_default_start(v: &f64) -> bool {
    (*v - START_DEFAULT).abs() < 1.0
}
fn is_default_stop(v: &f64) -> bool {
    (*v - STOP_DEFAULT).abs() < 1.0
}
fn is_default_points(v: &u16) -> bool {
    *v == POINTS_DEFAULT
}
fn is_default_z0(v: &f64) -> bool {
    (*v - Z0_DEFAULT).abs() < 1e-9
}
fn is_default_velocity(v: &f64) -> bool {
    (*v - 0.66).abs() < 1e-9
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}
fn is_one(v: &u32) -> bool {
    *v <= 1
}

impl Default for Face {
    fn default() -> Self {
        Self {
            port: String::new(),
            start_hz: START_DEFAULT,
            stop_hz: STOP_DEFAULT,
            points: POINTS_DEFAULT,
            z0: Z0_DEFAULT,
            velocity: 0.66,
            extension_m: 0.0,
            host_average: 1,
            smooth: 1,
            cals: Vec::new(),
            scales: std::collections::BTreeMap::new(),
            gd_aperture: 4,
            limits: std::collections::BTreeMap::new(),
            layout: layout::Layout::default(),
        }
    }
}

impl Face {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// Port and sweep settings, which stay in the station file. Calibration
    /// and scans live in the `.HSVNA` file beside it.
    pub fn for_station(&self) -> Self {
        let mut face = self.clone();
        face.cals.clear();
        face
    }
}

/// One stored sweep for a zone. The pairs are the raw S11 and S21 samples,
/// in the same `[real, imag]` order as a calibration standard.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredScan {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub zone: String,
    pub start_hz: f64,
    pub step_hz: f64,
    pub s11: Vec<[f64; 2]>,
    pub s21: Vec<[f64; 2]>,
}

/// Calibration sets and zone scans for one station. `City.HSCommander`
/// keeps this in `City.HSVNA`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VnaFile {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cals: Vec<CalSet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scans: Vec<StoredScan>,
}

impl VnaFile {
    pub fn empty() -> Self {
        Self {
            version: 1,
            cals: Vec::new(),
            scans: Vec::new(),
        }
    }
}

/// What a click on a zone tab does. A double-click always measures.
/// A single click measures only when that zone has no stored scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ZoneClick {
    Recall,
    Measure,
}

fn zone_click(has_scan: bool, double: bool) -> ZoneClick {
    if double || !has_scan {
        ZoneClick::Measure
    } else {
        ZoneClick::Recall
    }
}

/// Dark red, the same fill as the calibration measure button while it is held.
const SCAN_TAB: Color32 = Color32::from_rgb(0x3A, 0x0C, 0x10);

pub fn path_beside(station: &Path) -> PathBuf {
    station.with_extension("HSVNA")
}

pub fn validate_archive(file: &VnaFile) -> Result<(), String> {
    if file.version != 1 {
        return Err("This VNA file is not version 1.".into());
    }
    let mut face = Face::default();
    face.cals = file.cals.clone();
    validate(&face)?;
    for scan in &file.scans {
        if scan.id.is_empty() {
            return Err("nanovna scan is missing a device id".into());
        }
        if scan.s11.len() < 2 || scan.s11.len() != scan.s21.len() {
            return Err("nanovna scan is not a sweep".into());
        }
        if !scan.start_hz.is_finite() || !scan.step_hz.is_finite() || scan.step_hz <= 0.0 {
            return Err("nanovna scan frequency grid is not usable".into());
        }
    }
    Ok(())
}

pub fn load_file(path: &Path) -> Result<VnaFile, String> {
    if !path.exists() {
        return Ok(VnaFile::empty());
    }
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let file: VnaFile = serde_json::from_str(&text).map_err(|err| err.to_string())?;
    validate_archive(&file)?;
    Ok(file)
}

pub fn save_file(path: &Path, file: &VnaFile) -> Result<(), String> {
    validate_archive(file)?;
    // A station that never stored a calibration does not gain an empty file.
    if file.cals.is_empty() && file.scans.is_empty() && !path.exists() {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(file).map_err(|err| err.to_string())? + "\n";
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|err| err.to_string())?;
        }
    }
    // Temp file and rename, so a crash mid-write never leaves half a file.
    let tmp = path.with_extension("HSVNA.tmp");
    fs::write(&tmp, text).map_err(|err| format!("{}: {err}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|err| format!("{}: {err}", path.display()))
}

/// Read the VNA file. An older station that still carries `cals` donates
/// them when the VNA file has none. `stripped_station` means those `cals`
/// were removed from `face` and the station should be written again.
pub struct LoadedVna {
    pub archive: VnaFile,
    pub stripped_station: bool,
    /// Set when the VNA file could not be read and was moved aside.
    pub note: Option<String>,
}

pub fn load_beside(station: &Path, face: &mut Face) -> Result<LoadedVna, String> {
    let path = path_beside(station);
    let mut note = None;
    let mut archive = match load_file(&path) {
        Ok(file) => file,
        // A damaged file is moved aside, never overwritten by the next save.
        Err(err) if path.exists() => {
            let aside = path.with_extension(format!("HSVNA.corrupt-{}", cal_stamp_now().replace(':', "")));
            fs::rename(&path, &aside).map_err(|e| format!("{}: {e}", path.display()))?;
            note = Some(format!("{} could not be read ({err}); moved to {} and started empty.", path.display(), aside.display()));
            VnaFile::empty()
        }
        Err(err) => return Err(err),
    };
    let station_had_cals = !face.cals.is_empty();
    if archive.cals.is_empty() && station_had_cals {
        archive.cals = face.cals.clone();
        // Leave the station copy in place if the new file cannot be written.
        save_file(&path, &archive)?;
    }
    if station_had_cals {
        face.cals.clear();
    }
    Ok(LoadedVna {
        archive,
        stripped_station: station_had_cals,
        note,
    })
}

pub fn validate(face: &Face) -> Result<(), String> {
    if !face.port.chars().all(|ch| ch != '\0') {
        return Err("nanovna port is not a path".into());
    }
    if !face.start_hz.is_finite() || face.start_hz < proto::FREQ_MIN_HZ as f64 {
        return Err("nanovna start frequency is below 10 kHz".into());
    }
    if !face.stop_hz.is_finite() || face.stop_hz <= face.start_hz {
        return Err("nanovna stop frequency is not above the start".into());
    }
    if face.points < 2 || face.points as usize > proto::POINTS_MAX {
        return Err("nanovna point count is outside 2..=1024".into());
    }
    if !face.z0.is_finite() || face.z0 <= 0.0 {
        return Err("nanovna Z0 must be greater than zero".into());
    }
    if !face.velocity.is_finite() || face.velocity <= 0.0 || face.velocity > 1.0 {
        return Err("nanovna velocity factor must be between 0 and 1".into());
    }
    if !face.extension_m.is_finite() {
        return Err("nanovna port extension is not a length".into());
    }
    if face.host_average == 0 || face.smooth == 0 {
        return Err("nanovna averaging and smoothing start at 1".into());
    }
    for cal in &face.cals {
        if cal.id.is_empty() {
            return Err("nanovna calibration is missing a device id".into());
        }
        let lengths: Vec<usize> = [
            cal.open.len(),
            cal.short.len(),
            cal.load.len(),
            cal.thru.len(),
            cal.isolation.len(),
        ]
        .into_iter()
        .filter(|len| *len > 0)
        .collect();
        if lengths.is_empty() {
            return Err("nanovna calibration has no standards".into());
        }
        let n = lengths[0];
        if n < 2 || lengths.iter().any(|len| *len != n) {
            return Err("nanovna calibration standards are not the same sweep".into());
        }
        if !cal.start_hz.is_finite() || !cal.step_hz.is_finite() || cal.step_hz <= 0.0 {
            return Err("nanovna calibration frequency grid is not usable".into());
        }
        if !cal.z0.is_finite() || cal.z0 <= 0.0 {
            return Err("nanovna calibration Z0 must be greater than zero".into());
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WizStep {
    Open,
    Short,
    Load,
    Thru,
    Isol,
    Finish,
}

impl WizStep {
    fn next(self) -> Self {
        match self {
            WizStep::Open => WizStep::Short,
            WizStep::Short => WizStep::Load,
            WizStep::Load => WizStep::Thru,
            WizStep::Thru => WizStep::Isol,
            WizStep::Isol | WizStep::Finish => WizStep::Finish,
        }
    }

    fn standard(self) -> Option<Standard> {
        match self {
            WizStep::Open => Some(Standard::Open),
            WizStep::Short => Some(Standard::Short),
            WizStep::Load => Some(Standard::Load),
            WizStep::Thru => Some(Standard::Thru),
            WizStep::Isol => Some(Standard::Isol),
            WizStep::Finish => None,
        }
    }

    fn heading(self, count: usize) -> String {
        let tail = if count > 1 {
            format!("{count} zones, one fitting.")
        } else {
            "One zone.".into()
        };
        match self {
            WizStep::Open => format!("Step 1 of 3. S11 open. {tail}"),
            WizStep::Short => format!("Step 2 of 3. Short on S11. {tail}"),
            WizStep::Load => format!("Step 3 of 3. 50 ohm load on S11. {tail}"),
            WizStep::Thru => format!("Optional. Cable from S11 to S21. {tail}"),
            WizStep::Isol => format!("Optional. 50 ohm load on S11 and on S21. {tail}"),
            WizStep::Finish => format!("The three pieces are stored. {tail}"),
        }
    }

    /// What to fit once. The same piece stays on while every zone is swept.
    fn instruction(self) -> &'static str {
        match self {
            WizStep::Open => {
                "Leave S11 empty. Leave S21 empty. Press Measure open. Every zone is swept before you fit the short."
            }
            WizStep::Short => {
                "Screw the short onto S11 once. Leave it there. Leave S21 empty. Press Measure short. Every zone is swept before you take it off."
            }
            WizStep::Load => {
                "Screw the 50 ohm load onto S11 once. Leave it there. Leave S21 empty. Press Measure load. Every zone is swept before you take it off."
            }
            WizStep::Thru => {
                "Fit one cable from S11 to S21. Leave it there. Press Measure cable. Every zone is swept before you take it off. Skip cable skips every zone."
            }
            WizStep::Isol => {
                "Fit a 50 ohm load on S11 and another on S21. Leave them there. Press Measure both. Every zone is swept. Skip loads skips every zone."
            }
            WizStep::Finish => {
                "Take every load and cable off. Connect the antenna or the device you want to measure, then press the Scan button in this step."
            }
        }
    }

    fn piece_saved(self) -> &'static str {
        match self {
            WizStep::Open => "Open saved.",
            WizStep::Short => "Short saved.",
            WizStep::Load => "50 ohm load saved.",
            WizStep::Thru => "Cable saved.",
            WizStep::Isol | WizStep::Finish => "Both loads saved.",
        }
    }

    fn done_note(self) -> &'static str {
        match self {
            WizStep::Open => "Open saved for every zone in this pass. Fit the short on S11.",
            WizStep::Short => "Short saved for every zone in this pass. Fit the 50 ohm load on S11.",
            WizStep::Load => {
                "50 ohm load saved for every zone in this pass. Take it off. The cable is optional."
            }
            WizStep::Thru => "Cable saved for every zone in this pass.",
            WizStep::Isol | WizStep::Finish => {
                "Both loads saved for every zone in this pass. Take them off."
            }
        }
    }

    fn step_name(self) -> &'static str {
        match self {
            WizStep::Open => "Open",
            WizStep::Short => "Short",
            WizStep::Load => "50 ohm",
            WizStep::Thru => "Cable",
            WizStep::Isol => "Both",
            WizStep::Finish => "Scan",
        }
    }

    fn action(self, zone: &str) -> String {
        match self {
            WizStep::Open => "Measure open".into(),
            WizStep::Short => "Measure short".into(),
            WizStep::Load => "Measure load".into(),
            WizStep::Thru => "Measure cable".into(),
            WizStep::Isol => "Measure both".into(),
            WizStep::Finish => format!("Scan {zone}"),
        }
    }

}

#[derive(Clone, PartialEq)]
struct ZoneSpan {
    zone: String,
    lo: f64,
    hi: f64,
}

#[derive(Clone, PartialEq)]
struct Wizard {
    /// Zones swept while the current piece stays fitted.
    spans: Vec<ZoneSpan>,
    /// Which span the current sweep belongs to.
    at: usize,
    step: WizStep,
    /// A rescan of one step. Storing it does not walk on to the next piece.
    only_step: bool,
}

impl Wizard {
    fn current(&self) -> Option<&ZoneSpan> {
        self.spans.get(self.at)
    }
}

/// Stay on this piece until every zone is stored, then move to the next piece.
fn next_batch(step: WizStep, at: usize, count: usize) -> (WizStep, usize) {
    if count > 0 && at + 1 < count {
        (step, at + 1)
    } else {
        (step.next(), 0)
    }
}

fn measure_note(step: WizStep, zone: &str, at: usize, count: usize) -> String {
    let piece = match step {
        WizStep::Open => "the open",
        WizStep::Short => "the short",
        WizStep::Load => "the 50 ohm load",
        WizStep::Thru => "the cable",
        WizStep::Isol => "both loads",
        WizStep::Finish => "the scan",
    };
    if count <= 1 {
        format!("Measuring {piece} for {zone}.")
    } else {
        format!(
            "Measuring {piece} for {zone}. {} of {count}. Leave the ports as they are.",
            at + 1
        )
    }
}

pub struct NanoVna {
    face: Face,
    session: Session,
    ports: Vec<ListedPort>,
    start: String,
    stop: String,
    points: String,
    z0: String,
    velocity: String,
    extension: String,
    host_avg: String,
    smooth: String,
    dev_avg: String,
    view: View,
    run: bool,
    /// Stop after the next sweep (SINGLE). Issue 68.
    single: bool,
    run_state: Run,
    /// Marker frequencies in Hz: M1..M8 then R. Issue 66.
    markers: [Option<f64>; markers::SLOTS],
    marker_on: usize,
    readout_fmt: markers::Readout,
    marker_freq: String,
    /// MEM1..MEM4 frozen traces and the selected slot. Issue 70.
    mems: [Option<Vec<Sample>>; traces::MEMS],
    mem_sel: usize,
    math: traces::Math,
    show_memory: bool,
    history: VecDeque<Vec<Sample>>,
    seen: u64,
    pending: Option<Standard>,
    note: String,
    dirty: bool,
    setup_open: bool,
    plan: crate::plan::BandPlan,
    zone_sel: usize,
    /// The Cal tab, drawn before the zones so its place does not move.
    cal_page: bool,
    /// Zone scan waiting until the NanoVNA has reported its id.
    await_scan: Option<(String, f64, f64)>,
    wizard: Option<Wizard>,
    arm_connect: bool,
    user_closed: bool,
    scans: Vec<StoredScan>,
    /// Raw samples of the scan a zone click recalled. Empty history shows this
    /// instead of the device's last sweep.
    shown_scan: Option<Vec<Sample>>,
    shown_id: String,
    /// Zone whose sweep is in flight. The tab stays dark red while it runs.
    sweep_zone: Option<String>,
    /// Ignore the device's previous sweep until the measurement just started arrives.
    blank_until_sweep: bool,
    archive_dirty: bool,
    /// Time between the last two sweeps that arrived while running.
    sweep_ms: Option<f64>,
    /// Sweeps received since connect, and the device details popover. Issue 72.
    sweep_count: u64,
    /// Correction off (CAL OFF) and the state of the shown trace. Issue 65.
    cal_off: bool,
    limits_edit: bool,
    /// Active trace, the pane being drawn, the open trace menu, marker coupling. Issues 62, 63, 66.
    active_trc: usize,
    cur_pane: u8,
    trace_menu: Option<usize>,
    couple: bool,
    /// TARGET value for the target search, in the trace unit. Issue 67.
    target_text: String,
    /// Repeat this search each sweep (TRACK): 0 max, 1 min.
    track: Option<u8>,
    /// Band-filter readout on S21 panes, SWR-bandwidth readout on S11 panes.
    filter_on: bool,
    notch_on: bool,
    swr_bw_on: bool,
    cal_state: cal::CalState,
    details_open: bool,
    last_sweep: Option<std::time::Instant>,
    /// A stored calibration standard shown in its own window. Issue 75.
    cal_graph: Option<(CalSet, WizStep)>,
    /// Format, compare set (index into the stored sets) and error-term view of that window.
    cal_graph_view: View,
    /// Keyboard click-mode presses waiting for their key group this frame. Issue 73.
    kbd: Vec<(keys::Group, u8)>,
    /// Arrow-key marker step waiting for the chart: -1 / +1.
    kbd_step: i32,
    cal_compare: Option<usize>,
    /// ISO 8601 time this screen was created; sets saved earlier show RESTORED.
    run_started: String,
    /// Markers placed on that window's graph (point indices; M1 first). Issue 75.
    cal_graph_marks: Vec<usize>,
    cal_terms: bool,
}

impl NanoVna {
    pub fn new(ctx: &egui::Context) -> Self {
        let face = Face::default();
        let mut ui = Self::from_face(ctx, face);
        ui.ports = device::list_ports();
        ui
    }

    fn from_face(ctx: &egui::Context, face: Face) -> Self {
        Self {
            start: hz_field(face.start_hz),
            stop: hz_field(face.stop_hz),
            points: face.points.to_string(),
            z0: trim_num(face.z0),
            velocity: trim_num(face.velocity),
            extension: trim_num(face.extension_m),
            host_avg: face.host_average.to_string(),
            smooth: face.smooth.to_string(),
            dev_avg: "1".into(),
            face,
            session: Session::start(ctx.clone()),
            ports: Vec::new(),
            view: View::Smith,
            run: false,
            single: false,
            run_state: Run::Idle,
            markers: [None; markers::SLOTS],
            marker_on: 0,
            readout_fmt: markers::Readout::Trace,
            marker_freq: String::new(),
            mems: Default::default(),
            mem_sel: 0,
            math: traces::Math::Off,
            show_memory: false,
            history: VecDeque::new(),
            seen: 0,
            pending: None,
            note: String::new(),
            dirty: false,
            setup_open: false,
            plan: crate::plan::BandPlan {
                version: 1,
                zones: Vec::new(),
            },
            zone_sel: 0,
            cal_page: false,
            await_scan: None,
            wizard: None,
            arm_connect: false,
            user_closed: false,
            scans: Vec::new(),
            shown_scan: None,
            shown_id: String::new(),
            sweep_zone: None,
            blank_until_sweep: false,
            archive_dirty: false,
            sweep_ms: None,
            sweep_count: 0,
            cal_off: false,
            limits_edit: false,
            active_trc: 0,
            cur_pane: 0,
            trace_menu: None,
            couple: true,
            target_text: "-3".into(),
            track: None,
            filter_on: false,
            notch_on: false,
            swr_bw_on: false,
            cal_state: cal::CalState::Uncal,
            details_open: false,
            last_sweep: None,
            cal_graph_view: View::Smith,
            kbd: Vec::new(),
            kbd_step: 0,
            cal_compare: None,
            run_started: cal_stamp_now(),
            cal_graph_marks: Vec::new(),
            cal_terms: false,
            cal_graph: None,
        }
    }

    /// The NanoVNA screen just became visible.
    pub fn shown(&mut self) {
        if !self.user_closed {
            self.arm_connect = true;
        }
    }

    /// The NanoVNA screen just closed. The next visit may connect again.
    pub fn hidden(&mut self) {
        self.user_closed = false;
    }

    pub fn set_plan(&mut self, plan: &crate::plan::BandPlan) {
        self.plan = plan.clone();
        if self.zone_sel >= self.plan.zones.len() {
            self.zone_sel = 0;
        }
    }

    pub fn apply_face(&mut self, face: Face) {
        self.start = hz_field(face.start_hz);
        self.stop = hz_field(face.stop_hz);
        self.points = face.points.to_string();
        self.z0 = trim_num(face.z0);
        self.velocity = trim_num(face.velocity);
        self.extension = trim_num(face.extension_m);
        self.host_avg = face.host_average.to_string();
        self.smooth = face.smooth.to_string();
        self.face = face;
        self.active_trc = 0;
        if let Some(v) = self.face.layout.traces.first().and_then(|t| view_from_id(&t.view)) {
            self.view = v;
        }
        self.history.clear();
        self.mems = Default::default();
        self.dirty = false;
    }

    pub fn station_face(&self) -> Face {
        self.face.for_station()
    }

    pub fn archive(&self) -> VnaFile {
        VnaFile {
            version: 1,
            cals: self.face.cals.clone(),
            scans: self.scans.clone(),
        }
    }

    pub fn set_archive(&mut self, archive: VnaFile) {
        self.face.cals = archive.cals;
        self.scans = archive.scans;
        self.shown_scan = None;
        self.shown_id.clear();
        self.history.clear();
        self.archive_dirty = false;
    }

    pub fn take_dirty(&mut self) -> bool {
        let dirty = self.dirty;
        self.dirty = false;
        dirty
    }

    pub fn take_archive_dirty(&mut self) -> bool {
        let dirty = self.archive_dirty;
        self.archive_dirty = false;
        dirty
    }

    pub fn mark_archive_dirty(&mut self) {
        self.archive_dirty = true;
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = LcarsTheme::default();
        let live = self.session.snapshot();
        GD_APERTURE.with(|a| a.set(self.face.gd_aperture.clamp(2, 200) as usize));
        self.read_keys(ui);
        if live.generation != self.seen && !live.sweep.is_empty() {
            self.ingest(&live.sweep);
            self.seen = live.generation;
            self.sweep_count += 1;
            let now = std::time::Instant::now();
            if let (true, Some(was)) = (self.run, self.last_sweep) {
                self.sweep_ms = Some(now.duration_since(was).as_secs_f64() * 1000.0);
            }
            self.last_sweep = Some(now);
        }
        self.pump(&live);
        if self.arm_connect
            && !self.user_closed
            && !live.connected
            && !live.busy
            && !self.face.port.is_empty()
        {
            self.arm_connect = false;
            self.session.connect(self.face.port.clone());
        }
        if self.await_scan.is_some() && live.device_id.is_some() && !live.busy {
            let (zone, lo, hi) = self.await_scan.take().expect("await scan");
            self.choose_span(&live, &zone, lo, hi);
        }
        if let Some(png) = self.session.take_capture() {
            self.save_bytes("Save screenshot", "nanovna-screen.png", &png);
        }
        if live.connected {
            // Keeps the status bar honest about a lost board within a second.
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
        self.chart(ui, &theme, &live);
        self.details_dialog(ui, &theme, &live);
        self.limits_dialog(ui, &theme);
        let shown_trace = self.display_trace(&live);
        self.trace_menu_dialog(ui, &theme, shown_trace.as_deref());
        self.setup_dialog(ui, &theme, &live);
        self.wizard_dialog(ui, &theme, &live);
    }

    fn setup_dialog(&mut self, ui: &mut Ui, theme: &LcarsTheme, live: &device::Live) {
        if !self.setup_open {
            return;
        }
        let mut open = true;
        egui::Window::new("NanoVNA setup")
            .open(&mut open)
            .resizable(true)
            .default_width(440.0)
            .show(ui.ctx(), |ui| {
                ui.set_min_width(400.0);
                ui.set_max_width(520.0);
                egui::ScrollArea::vertical()
                    .id_salt("nanovna-setup")
                    .max_height(560.0)
                    .show(ui, |ui| {
                        self.controls(ui, theme, live);
                    });
            });
        self.setup_open = open;
    }

    /// Device details read from the identity and device-id registers. Issue 72.
    fn details_dialog(&mut self, ui: &mut Ui, theme: &LcarsTheme, live: &device::Live) {
        if !self.details_open {
            return;
        }
        let mut open = true;
        let text = self.device_details(live);
        egui::Window::new("NanoVNA device")
            .open(&mut open)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.label(egui::RichText::new(&text).monospace().size(theme.data_size * 0.85));
                ui.horizontal(|ui| {
                    if action_button(ui, theme, "Copy", vec2(80.0, 32.0)).clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                    if action_button(ui, theme, "Reconnect", vec2(120.0, 32.0)).clicked() {
                        self.reconnect();
                    }
                });
            });
        self.details_open = open;
    }

    fn device_details(&self, live: &device::Live) -> String {
        let mut out = String::new();
        let line = |out: &mut String, k: &str, v: String| out.push_str(&format!("{k:<14}{v}\n"));
        line(&mut out, "STATE", if live.connected { "CONNECTED".into() } else if live.lost { "DISCONNECTED (LOST)".into() } else { "NOT CONNECTED".into() });
        line(&mut out, "PORT", if self.face.port.is_empty() { "-".into() } else { self.face.port.clone() });
        if let Some(id) = live.identity {
            line(&mut out, "FIRMWARE", id.version_label());
            line(&mut out, "BOARD", id.board.to_string());
            line(&mut out, "VARIANT", id.variant.to_string());
            line(&mut out, "PROTOCOL", id.protocol.to_string());
            line(&mut out, "MAX FREQ", display::fmt_hz(id.max_hz()));
            line(&mut out, "DEV AVG/POWER", if id.power_and_average() { "yes".into() } else { "needs FW 1.0.2".into() });
        }
        if let Some(dev) = live.device_id {
            if live.id_reliable {
                line(&mut out, "DEVICE SN", proto::device_id_sn_compact(&dev));
                line(&mut out, "SN (FULL)", proto::device_id_sn(&dev));
            } else {
                line(&mut out, "DEVICE ID", "ID unreliable (reads disagreed)".into());
                line(&mut out, "NOTE", device::POWER_NOTE.into());
            }
        }
        line(&mut out, "MAX POINTS", proto::POINTS_MAX.to_string());
        line(&mut out, "SWEEPS", self.sweep_count.to_string());
        line(&mut out, "SWEEP ERRORS", live.sweep_errors.to_string());
        if let Some(ms) = self.sweep_ms {
            line(&mut out, "SWEEP TIME", format!("{} ms", ms.round()));
        }
        line(&mut out, "CALIBRATION", self.cal_state.label().to_string());
        let warn = status_warnings(live);
        if !warn.is_empty() {
            line(&mut out, "WARNINGS", warn.join(" · "));
        }
        line(&mut out, "STATUS", live.status.clone());
        out.trim_end().to_string()
    }

    fn reconnect(&mut self) {
        if self.face.port.is_empty() {
            self.note = "Choose a USB port in Setup first.".into();
            return;
        }
        self.user_closed = false;
        self.run = false;
        self.run_state = Run::Idle;
        self.sweep_count = 0;
        self.session.disconnect();
        self.session.connect(self.face.port.clone());
        self.note = format!("Reconnecting to {}.", self.face.port);
    }

    fn pump(&mut self, live: &device::Live) {
        match self.run_state {
            Run::Queued if live.busy => self.run_state = Run::Running,
            Run::Running if !live.busy => self.run_state = Run::Idle,
            Run::Queued if !live.busy && live.generation != self.seen => self.run_state = Run::Idle,
            _ => {}
        }
        if self.run
            && self.wizard.is_none()
            && self.run_state == Run::Idle
            && live.connected
            && !live.busy
        {
            if self.setup_has_cal(live) {
                if self.send_sweep() {
                    self.run_state = Run::Queued;
                    if self.single {
                        self.single = false;
                        self.run = false;
                    }
                }
            } else {
                self.run = false;
                self.request_zone(live);
            }
        }
    }

    fn controls(&mut self, ui: &mut Ui, theme: &LcarsTheme, live: &device::Live) {
        ui.add(
            egui::Label::new(egui::RichText::new(&live.status).color(palette::TEXT_MUTED)).wrap(),
        );
        ui.horizontal(|ui| {
            if action_button(ui, theme, "Ports", vec2(88.0, 32.0)).clicked() {
                self.ports = device::list_ports();
            }
            if action_button(ui, theme, "Connect", vec2(100.0, 32.0)).clicked() {
                self.user_closed = false;
                if !self.face.port.is_empty() {
                    self.session.connect(self.face.port.clone());
                }
            }
            if action_button(ui, theme, "Close", vec2(80.0, 32.0)).clicked() {
                self.user_closed = true;
                self.arm_connect = false;
                self.run = false;
                self.run_state = Run::Idle;
                self.session.disconnect();
            }
        });
        self.control_body(ui, theme, live);
    }

    fn control_body(&mut self, ui: &mut Ui, theme: &LcarsTheme, live: &device::Live) {
        if self.ports.is_empty() {
            ui.label(
                egui::RichText::new("No USB serial ports.").color(palette::TEXT_MUTED),
            );
        }
        for port in &self.ports.clone() {
            let title = if port.nanovna {
                format!("{}  V2", port.name)
            } else {
                port.name.clone()
            };
            if ui.selectable_label(self.face.port == port.name, title).clicked() {
                self.face.port = port.name.clone();
                self.dirty = true;
            }
        }
        self.edit(ui, "Start Hz", Edit::Start);
        self.edit(ui, "Stop Hz", Edit::Stop);
        self.edit(ui, "Points", Edit::Points);
        ui.horizontal_wrapped(|ui| {
            for count in [11u16, 51, 101, 201, 301, 501, 1023] {
                if action_button(ui, theme, &count.to_string(), vec2(52.0, 28.0)).clicked() {
                    self.points = count.to_string();
                    if self.face.points != count {
                        self.face.points = count;
                        self.dirty = true;
                    }
                }
            }
        });
        self.edit(ui, "Z0", Edit::Z0);
        ui.label(
            egui::RichText::new("Each zone is calibrated from the Cal tab.")
                .color(palette::TEXT_MUTED),
        );
        ui.horizontal(|ui| {
            if action_button(ui, theme, "Clear cal", vec2(100.0, 32.0)).clicked() {
                self.clear_cal(live);
            }
            if action_button(ui, theme, "Backup", vec2(100.0, 32.0)).clicked() {
                self.backup_cal();
            }
            if action_button(ui, theme, "Restore", vec2(100.0, 32.0)).clicked() {
                self.restore_cal();
            }
        });
        self.edit(ui, "VF", Edit::Velocity);
        self.edit(ui, "Extension m", Edit::Extension);
        self.edit(ui, "Host avg", Edit::HostAvg);
        self.edit(ui, "Smooth", Edit::Smooth);
        let power_ok = live.identity.is_some_and(|id| id.power_and_average());
        let above = self
            .parse_setup()
            .is_ok_and(|(start, _, _)| start >= 140_000_000);
        if power_ok && above {
            ui.horizontal(|ui| {
                if action_button(ui, theme, "Avg on device", vec2(140.0, 32.0)).clicked() {
                    if let Ok(count) = self.dev_avg.trim().parse::<u8>() {
                        if count >= 1 {
                            self.session.average(count);
                        }
                    }
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.dev_avg)
                        .desired_width(48.0)
                        .font(egui::TextStyle::Monospace),
                );
            });
            ui.horizontal(|ui| {
                for (level, name) in [(3u8, "Max"), (2, "-3 dB"), (1, "-6 dB"), (0, "-9 dB")] {
                    if action_button(ui, theme, name, vec2(70.0, 32.0)).clicked() {
                        self.session.power(level);
                    }
                }
            });
        } else if live.connected {
            readout(
                ui,
                theme,
                "TX power and device averaging need firmware 1.0.2 or later, and a sweep that starts at or above 140 MHz.",
                false,
            );
        }
        ui.horizontal(|ui| {
            if action_button(ui, theme, "Memory", vec2(88.0, 32.0)).clicked() {
                if let Some(trace) = self.display_trace(live) {
                    self.mems[self.mem_sel] = Some(trace);
                    self.show_memory = true;
                }
            }
            if action_button(ui, theme, "Mem on", vec2(80.0, 32.0)).clicked() {
                self.show_memory = !self.show_memory;
            }
            if action_button(ui, theme, "Math", vec2(70.0, 32.0)).clicked() {
                self.math = next_math(self.math);
            }
        });
        ui.horizontal(|ui| {
            if action_button(ui, theme, "CSV", vec2(64.0, 32.0)).clicked() {
                self.export_text(live, "csv", "nanovna.csv", true);
            }
            if action_button(ui, theme, "s1p", vec2(64.0, 32.0)).clicked() {
                self.export_text(live, "s1p", "nanovna.s1p", false);
            }
            if action_button(ui, theme, "s2p", vec2(64.0, 32.0)).clicked() {
                self.export_text(live, "s2p", "nanovna.s2p", false);
            }
            if action_button(ui, theme, "PNG", vec2(64.0, 32.0)).clicked() {
                self.export_png(live);
            }
            if action_button(ui, theme, "Screen", vec2(80.0, 32.0)).clicked() {
                self.session.capture();
            }
        });
        if !self.note.is_empty() {
            readout(ui, theme, &self.note, true);
        }
        if let Some(samples) = self.display_trace(live) {
            self.finder_readout(ui, theme, &samples);
        }
    }

    fn face_caption(&self, live: &device::Live, samples: Option<&[Sample]>) -> String {
        let Some(samples) = samples else {
            if !live.sweep.is_empty() && self.wizard.is_none() && live.connected {
                return "Not valid. This tab has no open, short, and load calibration.".into();
            }
            if live.connected || live.status != "Not connected." {
                return live.status.clone();
            }
            return if self.face.port.is_empty() {
                "Not connected. Setup chooses the USB port.".into()
            } else {
                format!("Not connected to {}.", self.face.port)
            };
        };
        let Some(index) = self.marker_index(self.marker_on, samples) else {
            return self.cal_label(live);
        };
        let Some(sample) = samples.get(index) else {
            return self.cal_label(live);
        };
        let d = rf::derive(sample, self.z0_value());
        format!(
            "{}  {} Hz  SWR {}  RL {} dB  |Z| {} Ω  {} {:+}j Ω  Q {}  S21 {} dB  series {}  parallel {}",
            markers::slot_name(self.marker_on),
            sample.hz.round(),
            num(rf::swr(sample.s11)),
            num(rf::return_loss_db(sample.s11)),
            num(d.z_abs),
            num(d.r),
            d.x,
            num(d.q),
            num(rf::db(sample.s21)),
            reactance_part(d.series_l_h, d.series_c_f),
            reactance_part(d.parallel_l_h, d.parallel_c_f),
        )
    }

    fn edit(&mut self, ui: &mut Ui, label: &str, which: Edit) {
        let mut text = which.get(self);
        let changed = ui
            .horizontal(|ui| {
                ui.label(egui::RichText::new(label).color(palette::TEXT_MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .desired_width(140.0)
                        .font(egui::TextStyle::Monospace),
                )
                .changed()
            })
            .inner;
        if changed {
            which.set(self, text);
            self.commit_fields();
        }
    }

    /// Smith, Polar, SWR, and the other chart views. They sit at the top of
    /// the right frame. The zone tabs stay above both frames.
    fn chart(&mut self, ui: &mut Ui, theme: &LcarsTheme, live: &device::Live) {
        self.band_strip(ui, theme, live);
        if self.cal_page {
            let (left, right) = body_split(ui, theme, 280.0);
            show_frame(ui, theme, left, Side::Left, |ui| {
                readout(
                    ui,
                    theme,
                    "Each zone stores open, short, 50 ohm, the cable, and both loads.",
                    false,
                );
            });
            show_frame(ui, theme, right, Side::Right, |ui| {
                self.cal_body(ui, theme, live);
            });
            self.cal_graph_dialog(ui, theme);
            return;
        }
        let mut trace = self.display_trace(live);
        let mut memory = if self.show_memory {
            self.memory().cloned()
        } else {
            None
        };
        // With no live trace, a shown memory (for example an imported file)
        // becomes the trace so markers and readouts work on it. Issue 53.
        if trace.is_none() && memory.is_some() {
            trace = memory.take();
        }
        let math = match (&trace, &memory) {
            (Some(t), Some(m)) => traces::trace_math(t, m, self.math),
            _ => None,
        };
        if self.math != traces::Math::Off && memory.is_some() && math.is_none() && self.wizard.is_none() {
            self.note = "Math needs the memory on the same frequency grid as the trace.".into();
        }
        let (left, right) = body_split(ui, theme, 440.0);
        let mut caption = self.face_caption(live, trace.as_deref());
        if self.await_scan.is_some() && self.wizard.is_none() {
            caption = format!("Waiting for the NanoVNA. {caption}");
        } else if !self.note.is_empty() && self.wizard.is_none() {
            caption = format!("{}  {caption}", self.note);
        }
        show_frame(ui, theme, left, Side::Left, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("nanovna-keys")
                .auto_shrink([false, false])
                .max_height(ui.available_height())
                .show(ui, |ui| {
                    self.softkeys(ui, theme, live, trace.as_deref());
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&caption)
                                .monospace()
                                .size(theme.data_size * 0.85)
                                .color(palette::TEXT_MUTED),
                        )
                        .wrap(),
                    );
                    ui.add_space(6.0);
                    self.marker_block(ui, theme, trace.as_deref());
                });
        });
        if let Some(code) = self.track {
            self.marker_search(code, trace.as_deref());
        }
        if let Some(samples) = trace.as_deref().filter(|s| !s.is_empty()) {
            // M / Shift+M / Ctrl+M: active marker, next marker, reference marker; ← → step it. Issue 73.
            if let Some(op) = self.key_op(keys::Group::Marker) {
                self.marker_on = match op {
                    1 => self.marker_on,
                    2 => (self.marker_on + 1) % markers::NORMAL,
                    _ => markers::REF,
                };
                if self.markers[self.marker_on].is_none() {
                    self.markers[self.marker_on] = Some(samples[samples.len() / 2].hz);
                }
            }
            if self.kbd_step != 0 {
                let at = self.marker_index(self.marker_on, samples).unwrap_or(samples.len() / 2) as i64;
                let to = (at + self.kbd_step as i64).clamp(0, samples.len() as i64 - 1) as usize;
                self.markers[self.marker_on] = Some(samples[to].hz);
            }
        }
        let placed = self.placed(trace.as_deref());
        let (cal, state) = match trace.as_deref() {
            Some(t) => self.cal_for(t, None),
            None => (None, if self.cal_off { cal::CalState::Off } else { cal::CalState::Uncal }),
        };
        self.cal_state = state;
        self.sync_traces();
        show_frame(ui, theme, right, Side::Right, |ui| {
            let table_h = if placed.is_empty() { 0.0 } else { 22.0 * (placed.len() as f32 + 1.0) + 6.0 };
            let status_h = 26.0;
            let grid_h = (ui.available_height() - table_h - status_h - 8.0).max(160.0);
            let area = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), grid_h));
            ui.allocate_rect(area, Sense::hover());
            self.draw_panes(ui, theme, area, trace.as_deref(), memory.as_deref(), math.as_deref(), &placed, cal.as_ref());
            if let Some(samples) = trace.as_deref() {
                self.marker_table(ui, theme, samples, &placed);
            }
            self.status_bar(ui, theme, live, trace.as_deref(), cal.as_ref());
        });
    }

    /// The trace whose settings the keys edit, kept in the layout. Issue 62.
    fn sync_traces(&mut self) {
        if self.face.layout.traces.is_empty() {
            self.face.layout.traces.push(layout::TraceCfg { view: view_id(self.view), pane: 0, hidden: false });
            self.active_trc = 0;
        }
        self.active_trc = self.active_trc.min(self.face.layout.traces.len() - 1);
        let id = view_id(self.view);
        let t = &mut self.face.layout.traces[self.active_trc];
        if t.view != id {
            t.view = id;
            self.dirty = true;
        }
        self.cur_pane = t.pane;
    }

    fn trace_view(&self, t: usize) -> View {
        self.face.layout.traces.get(t).and_then(|c| view_from_id(&c.view)).unwrap_or(self.view)
    }

    /// Make trace `t` the active one: the keys and marker table follow it.
    fn activate(&mut self, t: usize) {
        if t < self.face.layout.traces.len() {
            self.active_trc = t;
            self.view = self.trace_view(t);
            self.cur_pane = self.face.layout.traces[t].pane;
        }
    }

    /// Draw every shown pane: its header lines and its chart. Issues 62 and 63.
    #[allow(clippy::too_many_arguments)]
    fn draw_panes(
        &mut self,
        ui: &mut Ui,
        theme: &LcarsTheme,
        area: Rect,
        trace: Option<&[Sample]>,
        memory: Option<&[Sample]>,
        math: Option<&[Sample]>,
        placed: &[(usize, usize)],
        cal: Option<&CalSet>,
    ) {
        let shown = self.face.layout.shown();
        let (cols, rows) = layout::grid(shown.len());
        let gap = 6.0;
        let cw = (area.width() - gap * (cols as f32 - 1.0)) / cols as f32;
        let ch = (area.height() - gap * (rows as f32 - 1.0)) / rows as f32;
        let active_pane = self.cur_pane;
        let mut new_active: Option<usize> = None;
        let mut max_toggle: Option<u8> = None;
        let mut clicked: Option<(usize, u8)> = None;
        for (k, pane) in shown.iter().copied().enumerate() {
            let (cx, cy) = (k % cols, k / cols);
            let rect = Rect::from_min_size(
                Pos2::new(area.left() + cx as f32 * (cw + gap), area.top() + cy as f32 * (ch + gap)),
                vec2(cw, ch),
            );
            let all = self.face.layout.in_pane(pane);
            let visible: Vec<usize> = all.iter().copied().filter(|t| !self.face.layout.traces[*t].hidden).collect();
            let primary = visible.first().copied();
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                self.cur_pane = pane;
                if all.is_empty() {
                    let resp = ui.allocate_exact_size(vec2(rect.width(), 26.0), Sense::click()).1;
                    ui.painter().text(
                        resp.rect.left_center() + vec2(8.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("PANE {} · NO TRACE · +TRACE OR MOVE ONE HERE", pane + 1),
                        egui::FontId::monospace(theme.data_size * 0.8),
                        palette::TEXT_MUTED,
                    );
                    if resp.double_clicked() {
                        max_toggle = Some(pane);
                    }
                }
                for &t in &all {
                    self.view = self.trace_view(t);
                    let color = if Some(t) == primary { TRACE1 } else { TRACE_COLORS[t % TRACE_COLORS.len()] };
                    let resp = self.trace_header(ui, theme, trace, cal, t, t == self.active_trc, color);
                    if resp.double_clicked() {
                        max_toggle = Some(pane);
                    } else if resp.clicked() {
                        new_active = Some(t);
                        self.trace_menu = Some(t);
                    }
                }
                let Some(p) = primary else {
                    return;
                };
                let pv = self.trace_view(p);
                self.view = pv;
                let z0 = self.z0_value();
                let vf = self.velocity_value();
                // Other visible Cartesian traces in the pane draw over the first one.
                let extra: Vec<(Vec<f64>, Color32)> = match trace {
                    Some(samples) if !is_disk(pv) && pv != View::Tdr => visible[1..]
                        .iter()
                        .filter_map(|t| {
                            let v = self.trace_view(*t);
                            (!is_disk(v) && v != View::Tdr).then(|| (series(v, samples, z0, vf).1, TRACE_COLORS[*t % TRACE_COLORS.len()]))
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                // Coupled markers show on every pane; uncoupled only on the active one. Issue 66.
                let marks: Vec<(usize, usize)> = if self.couple || pane == active_pane { placed.to_vec() } else { Vec::new() };
                let mut info = self.marker_info(trace, &marks);
                info.extend(self.analysis_lines(trace));
                let manual = self.face.scales.get(self.key().as_str()).copied();
                let overlay = self.overlay(trace);
                let (hit, _, gesture) = paint_chart(
                    ui,
                    pv,
                    trace,
                    memory,
                    math,
                    z0,
                    vf,
                    false,
                    theme.data_size,
                    &marks,
                    &info,
                    manual,
                    &overlay,
                    &extra,
                );
                if gesture.ref_divs != 0.0 {
                    let by = gesture.ref_divs as f64;
                    self.edit_scale(trace, |m| m.shifted(by, 10));
                    new_active = Some(p);
                }
                if gesture.wheel != 0 {
                    let up = gesture.wheel > 0;
                    self.edit_scale(trace, |m| m.stepped(up));
                    new_active = Some(p);
                }
                if hit.is_some() {
                    clicked = hit;
                    if new_active.is_none() {
                        new_active = Some(p);
                    }
                }
            });
        }
        if let Some(pane) = max_toggle {
            self.face.layout.toggle_max(pane);
            self.dirty = true;
        }
        let active = new_active.unwrap_or(self.active_trc);
        self.activate(active);
        if let (Some((index, clicks)), Some(samples)) = (clicked, trace) {
            let slot = markers::click_slot(self.marker_on, clicks);
            if let Some(sample) = samples.get(index) {
                self.markers[slot] = Some(sample.hz);
                if clicks == 2 {
                    self.marker_on = slot;
                }
            }
        }
    }

    /// Scale and limit key for the shown format in the pane being drawn.
    /// Pane 1 keeps the plain format name so older files keep their scales.
    fn key(&self) -> String {
        let base = scale_key(self.view);
        if self.cur_pane == 0 { base.to_string() } else { format!("P{} {base}", self.cur_pane + 1) }
    }

    /// Per-trace menu opened from its header line. Issue 63.
    /// Plain / Shift / Ctrl + letter = single / double / triple click on that key
    /// group; ← → move the active marker. Ignored while a text field has focus. Issue 73.
    fn read_keys(&mut self, ui: &Ui) {
        self.kbd.clear();
        self.kbd_step = 0;
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let events = ui.input(|i| i.events.clone());
        for e in events {
            if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
                if modifiers.alt {
                    continue;
                }
                match key {
                    egui::Key::ArrowLeft => self.kbd_step = -1,
                    egui::Key::ArrowRight => self.kbd_step = 1,
                    _ => {
                        let name = key.name();
                        if let (1, Some(c)) = (name.len(), name.chars().next()) {
                            if let Some(g) = keys::group_of(c) {
                                self.kbd.push((g, keys::mode(modifiers.shift, modifiers.ctrl || modifiers.command)));
                            }
                        }
                    }
                }
            }
        }
    }

    /// Take a pending keyboard press for this key group.
    fn key_op(&mut self, group: keys::Group) -> Option<u8> {
        let at = self.kbd.iter().position(|(g, _)| *g == group)?;
        Some(self.kbd.remove(at).1)
    }

    fn trace_menu_dialog(&mut self, ui: &mut Ui, theme: &LcarsTheme, samples: Option<&[Sample]>) {
        let Some(t) = self.trace_menu else {
            return;
        };
        if t >= self.face.layout.traces.len() {
            self.trace_menu = None;
            return;
        }
        let mut open = true;
        let mut pick: Option<View> = None;
        let mut hide = false;
        let mut auto = false;
        let mut store = false;
        let mut move_next = false;
        let mut remove = false;
        let cur = self.trace_view(t);
        let hidden = self.face.layout.traces[t].hidden;
        egui::Window::new(format!("TRC{} settings", t + 1))
            .open(&mut open)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                for s21 in [false, true] {
                    ui.label(egui::RichText::new(if s21 { "S21 FORMAT" } else { "S11 FORMAT" }).color(palette::TEXT_MUTED));
                    ui.horizontal_wrapped(|ui| {
                        for (v, label) in VIEWS.iter().filter(|(v, _)| uses_s21(*v) == s21) {
                            if ui.selectable_label(*v == cur, *label).clicked() {
                                pick = Some(*v);
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    hide = action_button(ui, theme, if hidden { "Show" } else { "Hide" }, vec2(80.0, 30.0)).clicked();
                    auto = action_button(ui, theme, "Auto scale", vec2(110.0, 30.0)).clicked();
                    store = action_button(ui, theme, "Store mem", vec2(110.0, 30.0)).clicked();
                });
                ui.horizontal(|ui| {
                    move_next = action_button(ui, theme, "Next pane", vec2(110.0, 30.0)).clicked();
                    remove = self.face.layout.traces.len() > 1
                        && action_button(ui, theme, "Remove", vec2(90.0, 30.0)).clicked();
                });
            });
        self.activate(t);
        if let Some(v) = pick {
            self.view = v;
        }
        if hide {
            let h = &mut self.face.layout.traces[t].hidden;
            *h = !*h;
            self.dirty = true;
        }
        if auto {
            if self.face.scales.remove(self.key().as_str()).is_some() {
                self.dirty = true;
            }
        }
        if store {
            if let Some(trace) = samples {
                self.mems[self.mem_sel] = Some(trace.to_vec());
                self.show_memory = true;
            }
        }
        if move_next {
            self.face.layout.move_next(t);
            self.dirty = true;
        }
        if remove {
            self.face.layout.traces.remove(t);
            self.activate(0);
            self.dirty = true;
            open = false;
        }
        self.sync_traces();
        if !open {
            self.trace_menu = None;
        }
    }

    /// The scale the Cartesian pane uses: (scale, reference value, manual). Issue 69.
    fn shown_scale(&self, samples: Option<&[Sample]>) -> Option<(display::Scale, f64, bool)> {
        if is_disk(self.view) {
            return None;
        }
        if let Some(m) = self.face.scales.get(self.key().as_str()) {
            return Some((m.to_scale(10), m.ref_value, true));
        }
        let samples = samples?;
        let (_, mut ys) = series(self.view, samples, self.z0_value(), self.velocity_value());
        ys.extend(second_series(self.view, samples, self.z0_value()));
        let scale = display::auto_scale(&ys, 10);
        Some((scale, display::auto_ref(&scale), false))
    }

    /// Change the manual scale of the shown format, starting from what is on screen.
    fn edit_scale(&mut self, samples: Option<&[Sample]>, f: impl FnOnce(display::ManualScale) -> display::ManualScale) {
        let Some((scale, ref_value, _)) = self.shown_scale(samples) else {
            return;
        };
        let key = self.key();
        let base = self
            .face
            .scales
            .get(&key)
            .copied()
            .unwrap_or_else(|| display::ManualScale::from_scale(&scale, ref_value));
        let next = f(base);
        if next.ref_value.is_finite() && next.per_div.is_finite() && next.per_div > 0.0 {
            self.face.scales.insert(key, next);
            self.dirty = true;
        }
    }

    /// Sweep index of a marker slot on this trace. Issue 66.
    fn marker_index(&self, slot: usize, samples: &[Sample]) -> Option<usize> {
        markers::nearest_index(samples, self.markers.get(slot).copied().flatten()?)
    }

    /// (slot, index) for every placed marker, R last.
    fn placed(&self, samples: Option<&[Sample]>) -> Vec<(usize, usize)> {
        let Some(samples) = samples else {
            return Vec::new();
        };
        (0..markers::SLOTS)
            .filter_map(|slot| self.marker_index(slot, samples).map(|i| (slot, i)))
            .collect()
    }

    /// The small marker list painted in the top right of the pane.
    fn marker_info(&self, samples: Option<&[Sample]>, placed: &[(usize, usize)]) -> Vec<(String, Color32)> {
        let Some(samples) = samples else {
            return Vec::new();
        };
        let z0 = self.z0_value();
        placed
            .iter()
            .filter_map(|(slot, i)| {
                let sample = samples.get(*i)?;
                Some((
                    format!(
                        "{:<3}{:>14}  {}",
                        markers::slot_name(*slot),
                        display::fmt_hz(sample.hz),
                        self.value_text(self.view, sample, *i, z0, None)
                    ),
                    MARKER_COLORS[*slot % MARKER_COLORS.len()],
                ))
            })
            .collect()
    }

    /// VALUE column text: the chosen readout, or the trace format.
    fn value_text(&self, view: View, sample: &Sample, index: usize, z0: f64, ys: Option<&[f64]>) -> String {
        if self.readout_fmt != markers::Readout::Trace {
            let (param, _, _) = view_meta(view);
            return markers::readout_text(self.readout_fmt, sample, z0, index, param == "S21");
        }
        let (_, _, unit) = view_meta(view);
        match view {
            View::GroupDelay | View::Unwrap | View::UnwrapS11 => {
                let owned;
                let ys = match ys {
                    Some(ys) => ys,
                    None => {
                        owned = series(view, std::slice::from_ref(sample), z0, 0.66).1;
                        &owned
                    }
                };
                let v = if ys.len() == 1 { ys.first() } else { ys.get(index) };
                v.map(|v| format!("{} {unit}", num(*v))).unwrap_or_default()
            }
            View::Tdr => "-".into(),
            view => marker_value(view, sample, z0),
        }
    }

    /// One trace line over a pane: name, parameter chip, format, scale, REF,
    /// flags and the CAL badge. Sections drop from the right when narrow. Issue 63.
    #[allow(clippy::too_many_arguments)]
    fn trace_header(
        &mut self,
        ui: &mut Ui,
        theme: &LcarsTheme,
        samples: Option<&[Sample]>,
        cal: Option<&CalSet>,
        trc: usize,
        active: bool,
        color: Color32,
    ) -> egui::Response {
        let (param, format, unit) = view_meta(self.view);
        let shown = self.shown_scale(samples);
        let hidden = self.face.layout.traces.get(trc).is_some_and(|t| t.hidden);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, if active { palette::STRIP } else { rustcars::tint(palette::STRIP, -0.4) });
        if active {
            painter.rect_filled(Rect::from_min_size(rect.min, vec2(4.0, rect.height())), 0.0, palette::TAN);
        }
        let font = egui::FontId::monospace(theme.data_size * 0.85);
        // CAL, CAL INT, CAL OFF or UNCAL, with the kind and range of the set. Issue 65.
        let state = self.cal_state;
        let kind = match cal {
            Some(set) if set.thru.is_empty() => " OSL",
            Some(_) => " OSL+THRU",
            None => "",
        };
        let range = cal
            .map(|set| {
                let hi = set.start_hz + set.step_hz * (set.open.len().saturating_sub(1)) as f64;
                format!(" · {}-{}", display::fmt_hz(set.start_hz), display::fmt_hz(hi))
            })
            .unwrap_or_default();
        let label = if samples.is_none() && state != cal::CalState::Off { "UNCAL" } else { state.label() };
        let fill = match state {
            cal::CalState::Exact if samples.is_some() => palette::OLIVE,
            cal::CalState::Interp => palette::TAN,
            cal::CalState::Off => palette::SLATE,
            _ => palette::BRICK,
        };
        // The long badge first; the short label when the pane is narrow.
        let date = cal.map(|set| format!(" · {}", cal_date_text(set.saved_at.as_deref()))).unwrap_or_default();
        // RESTORED: loaded from the VNA file; OTHER ID: saved under another device id. Issue 65.
        let live_id = self.session.snapshot().device_id.map(|raw| proto::device_id_hex(&raw));
        let origin = match cal {
            Some(set) if other_device(set, live_id.as_deref()) => " · RESTORED · OTHER ID",
            Some(set) if restored(set, &self.run_started) => " · RESTORED",
            _ => "",
        };
        let long = format!("{label}{kind}{range}{date}{origin}");
        let mut g = painter.layout_no_wrap(long, font.clone(), rustcars::ink_on(fill));
        if g.size().x > rect.width() * 0.45 {
            g = painter.layout_no_wrap(label.to_string(), font.clone(), rustcars::ink_on(fill));
        }
        let chip = Rect::from_min_size(
            Pos2::new(rect.right() - g.size().x - 20.0, rect.top() + 3.0),
            vec2(g.size().x + 12.0, rect.height() - 6.0),
        );
        let limit = chip.left() - 8.0;
        let mut x = rect.left() + 10.0;
        let mut cut = false;
        let mut put = |x: &mut f32, text: &str, color: Color32| {
            if cut {
                return;
            }
            let g = painter.layout_no_wrap(text.to_string(), font.clone(), color);
            let w = g.size().x;
            if *x + w > limit {
                cut = true;
                return;
            }
            painter.galley(Pos2::new(*x, rect.center().y - g.size().y / 2.0), g, color);
            *x += w + 12.0;
        };
        put(&mut x, &format!("TRC{}", trc + 1), palette::TEXT);
        // Parameter chip in the trace colour with dark ink.
        let pg = painter.layout_no_wrap(param.to_string(), font.clone(), rustcars::ink_on(color));
        if x + pg.size().x + 12.0 < limit {
            let pchip = Rect::from_min_size(Pos2::new(x, rect.top() + 3.0), vec2(pg.size().x + 12.0, rect.height() - 6.0));
            painter.rect_filled(pchip, 4.0, color);
            painter.galley(Pos2::new(pchip.left() + 6.0, pchip.center().y - pg.size().y / 2.0), pg, rustcars::ink_on(color));
            x = pchip.right() + 12.0;
        }
        put(&mut x, format, palette::TEXT);
        if hidden {
            put(&mut x, "HIDDEN", palette::TEXT_MUTED);
        }
        if let Some((scale, ref_value, manual)) = shown {
            put(&mut x, &format!("{} {unit}/", display::fmt_tick(scale.per_div, scale.per_div / 10.0)), palette::TEXT);
            put(&mut x, &format!("REF {} {unit}", display::fmt_tick(ref_value, scale.per_div)), palette::TEXT);
            put(&mut x, if manual { "MAN" } else { "AUTO" }, palette::TEXT_MUTED);
        }
        if self.show_memory && self.memory().is_some() {
            put(&mut x, &format!("MEM{}", self.mem_sel + 1), MEM_COLOR);
        }
        match self.math {
            traces::Math::Sub if self.show_memory => put(&mut x, "MATH D−M", MATH_TEXT),
            traces::Math::Div if self.show_memory => put(&mut x, "MATH D/M", MATH_TEXT),
            _ => {}
        }
        // Averaging and limit verdict flags. Issue 63.
        let avg = self.host_average_value();
        if avg > 1 {
            put(&mut x, &format!("AVG {avg}"), palette::TEXT);
        }
        if let Some(v) = self.overlay(samples).verdict {
            let (text, color) = header_verdict(&v);
            put(&mut x, &text, color);
        }
        put(&mut x, "CH1", palette::TEXT_MUTED);
        // Badge: single = Cal tab (wizard), double = restore a saved set, triple = correction on/off.
        let badge_resp = ui.interact(chip, egui::Id::unique(("vna-cal-badge", trc)), Sense::click());
        if click_n(ui, &badge_resp) == 3 {
            self.cal_off = !self.cal_off;
            self.note = if self.cal_off { "Correction is off (CAL OFF).".into() } else { "Correction is on.".into() };
        } else if click_n(ui, &badge_resp) == 2 {
            self.restore_cal();
        } else if click_n(ui, &badge_resp) == 1 {
            self.cal_page = true;
        }
        painter.rect_filled(chip, 4.0, fill);
        painter.galley(
            Pos2::new(chip.left() + 6.0, chip.center().y - g.size().y / 2.0),
            g,
            rustcars::ink_on(fill),
        );
        resp.on_hover_text("Click: trace menu (format, scale, memory, hide, pane) · double: maximize or restore pane")
    }

    /// One row per placed marker under the chart, with delta to R. Issue 66.
    fn marker_table(&self, ui: &mut Ui, theme: &LcarsTheme, samples: &[Sample], placed: &[(usize, usize)]) {
        if placed.is_empty() {
            return;
        }
        let z0 = self.z0_value();
        let (hz, ys) = series(self.view, samples, z0, self.velocity_value());
        let (_, _, unit) = view_meta(self.view);
        let reference = placed.iter().find(|(slot, _)| *slot == markers::REF).map(|(_, i)| *i);
        let font = egui::FontId::monospace(theme.data_size * 0.85);
        let row = |ui: &mut Ui, cells: [String; 7], color: Color32, lit: bool| {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
            let painter = ui.painter_at(rect);
            if lit {
                painter.rect_filled(rect, 0.0, palette::STRIP);
            }
            let widths = [0.05, 0.04, 0.15, 0.2, 0.2, 0.1, 0.26];
            let mut x = rect.left() + 6.0;
            for (cell, w) in cells.iter().zip(widths) {
                painter.text(
                    Pos2::new(x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    cell,
                    font.clone(),
                    color,
                );
                x += rect.width() * w;
            }
        };
        row(
            ui,
            ["MKR".into(), "TRC".into(), "FREQ".into(), "VALUE".into(), "Z = R + jX".into(), "SWR".into(), "DELTA TO R".into()],
            palette::TEXT_MUTED,
            false,
        );
        let tdr = self.view == View::Tdr;
        for (slot, index) in placed {
            let Some(sample) = samples.get(*index) else {
                continue;
            };
            let d = rf::derive(sample, z0);
            let delta = match reference {
                _ if *slot == markers::REF || tdr => "-".to_string(),
                Some(r) => markers::delta(&ys, &hz, *index, r)
                    .map(|(df, dv)| markers::delta_text(df, dv, unit))
                    .unwrap_or_else(|| "-".into()),
                None => "no R".into(),
            };
            row(
                ui,
                [
                    markers::slot_name(*slot),
                    "1".into(),
                    display::fmt_hz(sample.hz),
                    self.value_text(self.view, sample, *index, z0, Some(&ys)),
                    format!("{} {:+}j Ω", num(d.r), num(d.x)),
                    format!("{} : 1", num(rf::swr(sample.s11))),
                    delta,
                ],
                MARKER_COLORS[*slot % MARKER_COLORS.len()],
                *slot == self.marker_on,
            );
        }
    }

    /// The channel and instrument line along the bottom. Issues 65 and 71.
    fn status_bar(&mut self, ui: &mut Ui, theme: &LcarsTheme, live: &device::Live, samples: Option<&[Sample]>, cal: Option<&CalSet>) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, palette::STRIP);
        // The summary follows the SWEEP fields as they are edited; the
        // shown sweep is used only when the fields do not parse. Issue 68.
        let (start, stop, points) = match (self.parse_setup(), samples) {
            (Ok((a, b, n)), _) => (a as f64, b as f64, n as usize),
            (Err(_), Some(s)) if !s.is_empty() => (s[0].hz, s[s.len() - 1].hz, s.len()),
            _ => (self.face.start_hz, self.face.stop_hz, self.face.points as usize),
        };
        let avg = self.host_average_value().max(1);
        let mode = if !self.run { "HOLD" } else if self.single { "SINGLE" } else { "CONT" };
        let summary = format!(
            "CH1  START {} · STOP {} · {} PTS",
            display::fmt_hz(start),
            display::fmt_hz(stop),
            points,
        );
        let mut cal_text = self.cal_state.label().to_string();
        if let Some(set) = cal {
            let name = if set.name.is_empty() { &set.zone } else { &set.name };
            if !name.is_empty() {
                cal_text.push_str(&format!(" {name}"));
            }
        }
        // Warnings carry text, never colour alone.
        let warn = status_warnings(live);
        let id_bad = device::power_note(live).is_some();
        // Priority: warnings are never dropped, then the sweep summary,
        // then mode and cal, then AVG. Issue 68 (status line overlap).
        let mut segs: Vec<(String, u8)> = vec![
            (summary, 3),
            (format!("AVG {avg}"), 1),
            (mode.to_string(), 2),
            (cal_text, 2),
        ];
        if !warn.is_empty() {
            segs.push((format!("⚠ {}", warn.join(" · ")), u8::MAX));
        }
        let device = device_short(live, &self.face.port);
        let leds = status_leds(live, self.cal_state, cal.is_some_and(|set| restored(set, &self.run_started)), self.run && live.connected);
        // The full line, for copying: everything the bar and popup know.
        let fw = live
            .identity
            .as_ref()
            .map(|id| format!(" · FW {}.{}", id.fw_major, id.fw_minor))
            .unwrap_or_default();
        let mut time = self
            .sweep_ms
            .map(|ms| format!(" · {} ms/SWEEP", ms.round()))
            .unwrap_or_default();
        if live.connected {
            time.push_str(&format!(" · #{}", self.sweep_count));
        }
        let all: Vec<&str> = segs.iter().map(|(t, _)| t.as_str()).collect();
        let status_line = format!("{}  {device}{fw}{time}", all.join("  "));
        if resp.hovered() {
            painter.rect_stroke(rect, 0.0, Stroke::new(1.0, palette::TAN), egui::StrokeKind::Inside);
        }
        // single = details, double = reconnect, triple = copy the status line.
        // A click on an LED is a click on the bar: it opens the device details.
        let key = self.key_op(keys::Group::Status);
        if click_n(ui, &resp) == 3 || key == Some(3) {
            ui.ctx().copy_text(status_line.clone());
            self.note = "Status line copied.".into();
        } else if click_n(ui, &resp) == 2 || key == Some(2) {
            self.details_open = false;
            self.reconnect();
        } else if click_n(ui, &resp) == 1 || key == Some(1) {
            self.details_open = !self.details_open;
        }
        let font = egui::FontId::monospace(theme.data_size * 0.8);
        let right_color = if live.connected { palette::TEXT } else if live.lost { LOST_TEXT } else { palette::TEXT_MUTED };
        let right_galley = painter.layout_no_wrap(device.clone(), font.clone(), right_color);
        let char_w = painter.layout_no_wrap("0".into(), font.clone(), palette::TEXT).size().x.max(1.0);
        let text_x = rect.right() - 8.0 - right_galley.size().x;
        painter.galley(Pos2::new(text_x, rect.center().y - right_galley.size().y / 2.0), right_galley, right_color);
        // LEDs just left of the device text.
        let led_r = 4.0;
        let led_step = 13.0;
        let mut led_x = text_x - 10.0 - led_r;
        let mut hover_tip: Option<String> = None;
        let hover = resp.hover_pos();
        for led in leds.iter().rev() {
            let c = Pos2::new(led_x, rect.center().y);
            painter.circle_filled(c, led_r, led.color);
            painter.circle_stroke(c, led_r, Stroke::new(1.0, Color32::from_black_alpha(160)));
            if hover.is_some_and(|p| (p - c).length() <= led_step / 2.0 + 1.0) {
                hover_tip = Some(led.tip.clone());
            }
            led_x -= led_step;
        }
        let right_start = led_x + led_step - led_r;
        let left_x = rect.left() + 8.0;
        let left_w = (right_start - 12.0 - left_x).max(0.0);
        let left = fit_status(&segs, (left_w / char_w).floor() as usize);
        let left_rect = Rect::from_min_max(Pos2::new(left_x, rect.top()), Pos2::new(left_x + left_w, rect.bottom()));
        ui.painter_at(left_rect).text(
            Pos2::new(left_x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            left,
            font,
            if id_bad { LOST_TEXT } else { palette::TEXT },
        );
        let tip = match hover_tip {
            Some(t) => format!("{t}\n\n{}", keys::hint(keys::Group::Status)),
            None => format!("{status_line}\n\n{}", keys::hint(keys::Group::Status)),
        };
        let _ = resp.on_hover_text(tip);
    }

    /// The instrument key panel down the left: FORMAT, DISPLAY, MARKER, SWEEP · CAL.
    fn softkeys(&mut self, ui: &mut Ui, theme: &LcarsTheme, live: &device::Live, samples: Option<&[Sample]>) {
        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
        // Leave room on the right for the scrollbar so it never covers a key.
        let width = (ui.available_width() - 18.0).clamp(120.0, 380.0);
        let col3 = ((width - 16.0) / 3.0).floor();
        let col4 = ((width - 24.0) / 4.0).floor();

        key_caption(ui, theme, "SWEEP · CAL", width);
        let mut scan = false;
        let mut mode: Option<u8> = None;
        let mut setup = false;
        ui.horizontal(|ui| {
            scan = softkey(ui, theme, "Scan", false, col4);
            // RUN: single = continuous, double = single sweep, triple = hold.
            let resp = softkey_response(ui, theme, "Run", self.run && !self.single, col4, true)
                .on_hover_text(keys::hint(keys::Group::Run));
            if click_n(ui, &resp) == 3 {
                mode = Some(2);
            } else if click_n(ui, &resp) == 2 {
                mode = Some(1);
            } else if click_n(ui, &resp) == 1 {
                mode = Some(0);
            } else if let Some(m) = self.key_op(keys::Group::Run) {
                mode = Some(m - 1);
            }
            if softkey(ui, theme, "Hold", !self.run, col4) {
                mode = Some(2);
            }
            if softkey(ui, theme, "Single", self.run && self.single, col4) {
                mode = Some(1);
            }
        });
        if scan {
            self.begin_scan(live);
        }
        match mode {
            Some(2) => {
                self.run = false;
                self.single = false;
                self.run_state = Run::Idle;
            }
            Some(m) => {
                if !self.run {
                    self.start_run(live);
                }
                self.single = m == 1 && self.run;
            }
            None => {}
        }
        self.sweep_panel(ui, theme, live, width, col4);
        ui.horizontal(|ui| {
            setup = softkey(ui, theme, "Setup", self.setup_open, col4);
        });
        if setup {
            self.ports = device::list_ports();
            self.setup_open = true;
        }

        key_caption(ui, theme, "LAYOUT · TRACE", width);
        let mut lay: Option<(u8, u8)> = None;
        ui.horizontal(|ui| {
            // LAYOUT: single = choose, double = swap pane order, triple = default four-pane set.
            for n in [1u8, 2, 4] {
                let resp = softkey_response(ui, theme, &format!("Layout·{n}"), self.face.layout.panes == n, col3, true)
                    .on_hover_text(keys::hint(keys::Group::Layout));
                if click_n(ui, &resp) == 3 {
                    lay = Some((n, 3));
                } else if click_n(ui, &resp) == 2 {
                    lay = Some((n, 2));
                } else if click_n(ui, &resp) == 1 {
                    lay = Some((n, 1));
                }
            }
        });
        if lay.is_none() {
            if let Some(op) = self.key_op(keys::Group::Layout) {
                let next = match self.face.layout.panes { 1 => 2, 2 => 4, _ => 1 };
                lay = Some((next, op));
            }
        }
        match lay {
            Some((n, 1)) => {
                if n == 4 && self.face.layout.traces.len() <= 1 {
                    self.face.layout = layout::Layout::default_four();
                    self.activate(0);
                } else {
                    self.face.layout.set_panes(n);
                }
                self.dirty = true;
            }
            Some((_, 2)) => {
                self.face.layout.swap_order();
                self.dirty = true;
            }
            Some((_, 3)) => {
                self.face.layout = layout::Layout::default_four();
                self.activate(0);
                self.dirty = true;
            }
            _ => {}
        }
        let n_tr = self.face.layout.traces.len().max(1);
        let mut tr_op: Option<(usize, u8)> = None;
        ui.horizontal(|ui| {
            // TRC keys: single = select (again: show/hide), double = trace menu, triple = store memory.
            for t in 0..n_tr {
                let hidden = self.face.layout.traces.get(t).is_some_and(|c| c.hidden);
                let label = if hidden { format!("TRC{} off", t + 1) } else { format!("TRC{}", t + 1) };
                let resp = softkey_response(ui, theme, &label, t == self.active_trc, col4, true)
                    .on_hover_text(keys::hint(keys::Group::Trace));
                if click_n(ui, &resp) == 3 {
                    tr_op = Some((t, 3));
                } else if click_n(ui, &resp) == 2 {
                    tr_op = Some((t, 2));
                } else if click_n(ui, &resp) == 1 {
                    tr_op = Some((t, 1));
                }
            }
            if n_tr < layout::MAX_TRACES && softkey(ui, theme, "+Trace", false, col4) {
                tr_op = Some((usize::MAX, 0));
            }
        });
        if tr_op.is_none() {
            if let Some(op) = self.key_op(keys::Group::Trace) {
                let n = self.face.layout.traces.len().max(1);
                let t = if op == 1 { (self.active_trc + 1) % n } else { self.active_trc };
                tr_op = Some((t, op));
            }
        }
        match tr_op {
            Some((usize::MAX, _)) => {
                let pane = (self.face.layout.traces.len() as u8) % self.face.layout.panes.max(1);
                self.face.layout.traces.push(layout::TraceCfg { view: view_id(View::S21), pane, hidden: false });
                let t = self.face.layout.traces.len() - 1;
                self.activate(t);
                self.dirty = true;
            }
            Some((t, 1)) => {
                if t == self.active_trc {
                    if let Some(c) = self.face.layout.traces.get_mut(t) {
                        c.hidden = !c.hidden;
                        self.dirty = true;
                    }
                } else {
                    self.activate(t);
                }
            }
            Some((t, 2)) => {
                self.activate(t);
                self.trace_menu = Some(t);
            }
            Some((t, 3)) => {
                self.activate(t);
                if let Some(trace) = samples {
                    self.mems[self.mem_sel] = Some(trace.to_vec());
                    self.show_memory = true;
                }
            }
            _ => {}
        }
        ui.horizontal(|ui| {
            if softkey_enabled(ui, theme, "Pane →", false, col4, self.face.layout.panes > 1) {
                self.face.layout.move_next(self.active_trc);
                self.dirty = true;
            }
            // MAX: single = maximize/restore the active trace's pane, double = next pane, triple = grid. Issue 62.
            let multi = self.face.layout.panes > 1;
            let lit = self.face.layout.maximized.is_some();
            let resp = softkey_response(ui, theme, if lit { "Restore" } else { "Max" }, lit, col4, multi)
                .on_hover_text(keys::hint(keys::Group::Max));
            let mut op = if multi { click_n(ui, &resp) } else { 0 };
            if op == 0 {
                op = self.key_op(keys::Group::Max).unwrap_or(0);
            }
            if op > 0 {
                let pane = self.face.layout.traces.get(self.active_trc).map(|t| t.pane).unwrap_or(0);
                let l = &mut self.face.layout;
                l.maximized = keys::max_op(op, l.panes, l.maximized, pane);
                self.dirty = true;
            }
            if softkey(ui, theme, if self.couple { "Couple on" } else { "Couple off" }, self.couple, col4) {
                self.couple = !self.couple;
            }
        });

        key_caption(ui, theme, "FORMAT", width);
        // Parameter first; only the formats valid for it are offered. Issue 64.
        let s21 = uses_s21(self.view);
        let mut pick_param = None;
        ui.horizontal(|ui| {
            if softkey(ui, theme, "S11", !s21, col4) {
                pick_param = Some(false);
            }
            if softkey(ui, theme, "S21", s21, col4) {
                pick_param = Some(true);
            }
        });
        if let Some(want) = pick_param {
            if want != s21 {
                self.view = if want { View::S21 } else { View::Smith };
            }
        }
        let valid: Vec<(View, &str)> = VIEWS.iter().copied().filter(|(v, _)| uses_s21(*v) == uses_s21(self.view)).collect();
        for triple in valid.chunks(3) {
            ui.horizontal(|ui| {
                for (view, label) in triple {
                    if softkey(ui, theme, label, self.view == *view, col3) {
                        self.view = *view;
                    }
                }
            });
        }
        if self.view == View::GroupDelay {
            ui.horizontal(|ui| {
                let ap = self.face.gd_aperture;
                if softkey_enabled(ui, theme, "Apert ▼", false, col4, ap > 2) {
                    self.face.gd_aperture = ap.saturating_sub(2).max(2);
                    self.dirty = true;
                }
                if softkey_enabled(ui, theme, "Apert ▲", false, col4, ap < 200) {
                    self.face.gd_aperture = (ap + 2).min(200);
                    self.dirty = true;
                }
                ui.label(
                    egui::RichText::new(format!("APERTURE {} PTS", self.face.gd_aperture))
                        .size(theme.data_size * 0.8)
                        .color(palette::TEXT_MUTED),
                );
            });
        }

        key_caption(ui, theme, "DISPLAY", width);
        ui.horizontal(|ui| {
            // STORE: single = store, double = toggle memory display, triple = cycle math.
            let resp = softkey_response(ui, theme, "Store", false, col4, samples.is_some())
                .on_hover_text(keys::hint(keys::Group::Store));
            let key = if samples.is_some() { self.key_op(keys::Group::Store) } else { None };
            if click_n(ui, &resp) == 3 || key == Some(3) {
                self.math = next_math(self.math);
            } else if click_n(ui, &resp) == 2 || key == Some(2) {
                self.show_memory = !self.show_memory;
            } else if click_n(ui, &resp) == 1 || key == Some(1) {
                if let Some(trace) = samples {
                    self.mems[self.mem_sel] = Some(trace.to_vec());
                    self.show_memory = true;
                }
            }
            let has_mem = self.memory().is_some();
            if softkey_enabled(ui, theme, "Mem", self.show_memory && has_mem, col4, has_mem) {
                self.show_memory = !self.show_memory;
            }
            let math_label = match self.math {
                traces::Math::Off => "Math off",
                traces::Math::Sub => "D − M",
                traces::Math::Div => "D / M",
            };
            if softkey_enabled(ui, theme, math_label, self.math != traces::Math::Off, col4, has_mem) {
                self.math = next_math(self.math);
            }
        });
        ui.horizontal(|ui| {
            for slot in 0..traces::MEMS {
                let label = if self.mems[slot].is_some() { format!("MEM{}", slot + 1) } else { format!("MEM{}·", slot + 1) };
                if softkey(ui, theme, &label, self.mem_sel == slot, col4) {
                    self.mem_sel = slot;
                }
            }
        });
        let mut import = false;
        let mut save = false;
        let mut clear = false;
        ui.horizontal(|ui| {
            import = softkey(ui, theme, "Import", false, col4);
            save = softkey_enabled(ui, theme, "Save mem", false, col4, self.memory().is_some());
            clear = softkey_enabled(ui, theme, "Clr mem", false, col4, self.memory().is_some());
        });
        if import {
            self.import_memory();
        }
        if save {
            if let Some(mem) = self.memory().cloned() {
                let text = rf::touchstone_s2p(&mem, self.z0_value());
                self.save_bytes("Save memory", &format!("nanovna-mem{}.s2p", self.mem_sel + 1), text.as_bytes());
            }
        }
        if clear {
            self.mems[self.mem_sel] = None;
        }

        key_caption(ui, theme, "SCALE", width);
        let cart = !is_disk(self.view);
        let shown = self.shown_scale(samples);
        let manual = shown.is_some_and(|s| s.2);
        let mut scale_op: Option<u8> = None;
        ui.horizontal(|ui| {
            // AUTO: single = auto-scale this trace, double = all traces in the
            // pane (one trace today), triple = reset every saved scale.
            let resp = softkey_response(ui, theme, "Auto", cart && !manual, col4, cart)
                .on_hover_text(keys::hint(keys::Group::Auto));
            if click_n(ui, &resp) == 3 {
                scale_op = Some(3);
            } else if click_n(ui, &resp) == 2 {
                scale_op = Some(2);
            } else if click_n(ui, &resp) == 1 {
                scale_op = Some(1);
            } else if cart {
                scale_op = self.key_op(keys::Group::Auto);
            }
            if softkey_enabled(ui, theme, "/div ▲", false, col4, cart && shown.is_some()) {
                scale_op = Some(10);
            }
            if softkey_enabled(ui, theme, "/div ▼", false, col4, cart && shown.is_some()) {
                scale_op = Some(11);
            }
            if softkey_enabled(ui, theme, "Hold", manual, col4, cart && shown.is_some()) {
                scale_op = Some(12);
            }
        });
        if let Some((scale, ref_value, _)) = shown.filter(|_| cart) {
            let (_, _, unit) = view_meta(self.view);
            let mut r = ref_value;
            let mut pos = self.face.scales.get(self.key().as_str()).map(|m| m.ref_pos).unwrap_or_else(|| display::ManualScale::from_scale(&scale, ref_value).ref_pos);
            let mut per = scale.per_div;
            let (mut cr, mut cp, mut cd) = (false, false, false);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("REF").color(palette::TEXT_MUTED));
                cr = ui.add(egui::DragValue::new(&mut r).speed(scale.per_div / 10.0).suffix(format!(" {unit}"))).changed();
                ui.label(egui::RichText::new("POS").color(palette::TEXT_MUTED));
                cp = ui.add(egui::DragValue::new(&mut pos).range(0.0..=10.0).speed(0.1).suffix(" div")).changed();
                ui.label(egui::RichText::new("/DIV").color(palette::TEXT_MUTED));
                cd = ui.add(egui::DragValue::new(&mut per).range(1e-12..=1e12).speed(scale.per_div / 20.0)).changed();
            });
            if cr || cp || cd {
                self.edit_scale(samples, |m| display::ManualScale {
                    ref_value: if cr { r } else { m.ref_value },
                    ref_pos: if cp { pos } else { m.ref_pos },
                    per_div: if cd { per } else { m.per_div },
                });
            }
        }
        match scale_op {
            Some(1) | Some(2) => {
                if self.face.scales.remove(self.key().as_str()).is_some() {
                    self.dirty = true;
                }
            }
            Some(3) => {
                if !self.face.scales.is_empty() {
                    self.face.scales.clear();
                    self.dirty = true;
                }
                self.note = "Every trace format is back on auto-scale.".into();
            }
            Some(10) => self.edit_scale(samples, |m| m.stepped(true)),
            Some(11) => self.edit_scale(samples, |m| m.stepped(false)),
            Some(12) => self.edit_scale(samples, |m| m),
            _ => {}
        }

        key_caption(ui, theme, "LIMITS", width);
        let cart_lim = !is_disk(self.view) && self.view != View::Tdr;
        let key = self.key();
        let lim_on = self.face.limits.get(&key).is_some_and(|s| s.on);
        let mut lim_op = None;
        ui.horizontal(|ui| {
            // LIMITS: single = on/off, double = edit, triple = SWR 1.5 preset on this band.
            let resp = softkey_response(ui, theme, if lim_on { "Limits on" } else { "Limits off" }, lim_on, col4, cart_lim)
                .on_hover_text(keys::hint(keys::Group::Limits));
            if click_n(ui, &resp) == 3 {
                lim_op = Some(3);
            } else if click_n(ui, &resp) == 2 {
                lim_op = Some(2);
            } else if click_n(ui, &resp) == 1 {
                lim_op = Some(1);
            } else if cart_lim {
                lim_op = self.key_op(keys::Group::Limits);
            }
            if softkey_enabled(ui, theme, "Edit", self.limits_edit, col4, cart_lim) {
                lim_op = Some(2);
            }
            if softkey(ui, theme, "SWR 1.5", false, col4) {
                lim_op = Some(3);
            }
            if softkey(ui, theme, "SWR 2", false, col4) {
                lim_op = Some(4);
            }
        });
        match lim_op {
            Some(1) => {
                let set = self.face.limits.entry(key).or_default();
                set.on = !set.on;
                if set.segs.is_empty() {
                    self.limits_edit = true;
                }
                self.dirty = true;
            }
            Some(2) => self.limits_edit = true,
            Some(3) => self.limit_preset(1.5),
            Some(4) => self.limit_preset(2.0),
            _ => {}
        }

        key_caption(ui, theme, "MARKER", width);
        for row in [[0usize, 1, 2, 3], [4, 5, 6, 7]] {
            ui.horizontal(|ui| {
                for slot in row {
                    let placed = self.markers[slot].is_some();
                    let title = if placed { markers::slot_name(slot) } else { format!("{}·", markers::slot_name(slot)) };
                    if softkey(ui, theme, &title, self.marker_on == slot, col4) {
                        self.marker_on = slot;
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            if softkey(ui, theme, "Ref R", self.marker_on == markers::REF, col4) {
                self.marker_on = markers::REF;
            }
            if softkey(ui, theme, "Clr all", false, col4) {
                self.markers = [None; markers::SLOTS];
            }
            // Typed exact frequency for the active marker. Enter applies it.
            let resp = ui.add_sized(
                vec2(col4 * 2.0 + 8.0, KEY_H),
                egui::TextEdit::singleline(&mut self.marker_freq)
                    .hint_text("FREQ MHz")
                    .font(egui::TextStyle::Monospace),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                match parse_marker_mhz(&self.marker_freq) {
                    Some(hz) => {
                        self.markers[self.marker_on] = Some(hz);
                        self.note.clear();
                    }
                    None => self.note = "Marker frequency is a number in MHz, for example 14.2.".into(),
                }
            }
        });
        key_caption(ui, theme, "MKR READOUT", width);
        for row in markers::READOUTS.chunks(4) {
            ui.horizontal(|ui| {
                for (fmt, label) in row {
                    if softkey(ui, theme, label, self.readout_fmt == *fmt, col4) {
                        self.readout_fmt = *fmt;
                    }
                }
            });
        }
        let mut search: Option<u8> = None;
        ui.horizontal(|ui| {
            let ok = samples.is_some();
            for (code, label) in [(0u8, "Max"), (1, "Min"), (2, "Next pk"), (3, "Clear")] {
                let enabled = ok || code == 3;
                if softkey_enabled(ui, theme, label, false, col4, enabled) {
                    search = Some(code);
                }
            }
        });
        if let Some(code) = search {
            self.marker_search(code, samples);
        }
        // TARGET search, TRACK, peaks to markers, filter and SWR readouts. Issue 67.
        let ok = samples.is_some();
        let mut find: Option<i32> = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("TARGET").monospace().color(palette::TEXT_MUTED))
                .on_hover_text("Value to search for, in the active trace's unit");
            ui.add(egui::TextEdit::singleline(&mut self.target_text).desired_width(col4).font(egui::TextStyle::Monospace));
            if softkey_enabled(ui, theme, "◀ Tgt", false, col4, ok) {
                find = Some(-1);
            }
            if softkey_enabled(ui, theme, "Tgt ▶", false, col4, ok) {
                find = Some(1);
            }
        });
        if let Some(dir) = find {
            self.target_search(dir, samples);
        }
        ui.horizontal(|ui| {
            let label = match self.track {
                Some(0) => "Track max",
                Some(_) => "Track min",
                None => "Track off",
            };
            if softkey(ui, theme, label, self.track.is_some(), col4) {
                self.track = match self.track {
                    None => Some(0),
                    Some(0) => Some(1),
                    _ => None,
                };
            }
            if softkey_enabled(ui, theme, "Peaks→M", false, col4, ok) {
                self.peaks_to_markers(samples);
            }
            // FILTER: single = band-pass analysis, double = notch analysis, triple = both off.
            let resp = softkey_response(ui, theme, if self.notch_on { "Notch" } else { "Filter" }, self.filter_on || self.notch_on, col4, true)
                .on_hover_text("Click: band-pass analysis on/off\nDouble-click [Shift+click]: notch analysis on/off\nTriple-click [Ctrl+click]: analysis off");
            match click_n(ui, &resp) {
                1 => self.filter_on = !self.filter_on,
                2 => self.notch_on = !self.notch_on,
                3 => {
                    self.filter_on = false;
                    self.notch_on = false;
                }
                _ => {}
            }
            if softkey(ui, theme, "SWR BW", self.swr_bw_on, col4) {
                self.swr_bw_on = !self.swr_bw_on;
            }
        });
    }

    /// Move the active marker to the next TARGET crossing left or right. Issue 67.
    fn target_search(&mut self, dir: i32, samples: Option<&[Sample]>) {
        let Some(samples) = samples else { return };
        let Some(target) = self.target_text.trim().parse::<f64>().ok() else {
            self.note = "TARGET is a number in the trace unit, for example -3.".into();
            return;
        };
        if is_disk(self.view) || self.view == View::Tdr {
            self.note = "Target search works on Cartesian frequency traces.".into();
            return;
        }
        let (xs, ys) = series(self.view, samples, self.z0_value(), self.velocity_value());
        let from = self.marker_index(self.marker_on, samples);
        match search::target(&xs, &ys, target, from, dir) {
            Some(hz) => {
                self.markers[self.marker_on] = Some(hz);
                self.note.clear();
            }
            None => self.note = format!("No crossing of {target} in that direction."),
        }
    }

    /// Put M1..M8 on the highest peaks of the active trace. Issue 67.
    fn peaks_to_markers(&mut self, samples: Option<&[Sample]>) {
        let Some(samples) = samples else { return };
        if is_disk(self.view) || self.view == View::Tdr {
            self.note = "Peak search works on Cartesian frequency traces.".into();
            return;
        }
        let (_, ys) = series(self.view, samples, self.z0_value(), self.velocity_value());
        let finite: Vec<f64> = ys.iter().copied().filter(|v| v.is_finite()).collect();
        let span = finite.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - finite.iter().cloned().fold(f64::INFINITY, f64::min);
        let found = search::peaks(&ys, markers::NORMAL, (span * 0.05).max(1e-12));
        for slot in 0..markers::NORMAL {
            self.markers[slot] = found.get(slot).and_then(|i| samples.get(*i)).map(|s| s.hz);
        }
        self.note = format!("{} peak(s) placed on markers.", found.len());
    }

    /// Filter / SWR-bandwidth lines painted with the marker list. Issue 67.
    fn analysis_lines(&self, samples: Option<&[Sample]>) -> Vec<(String, Color32)> {
        let mut out = Vec::new();
        let Some(samples) = samples else { return out };
        let xs: Vec<f64> = samples.iter().map(|s| s.hz).collect();
        let s21 = uses_s21(self.view);
        if self.filter_on && s21 {
            let db: Vec<f64> = samples.iter().map(|s| 20.0 * s.s21.norm().max(1e-12).log10()).collect();
            match search::bandfilter(&xs, &db) {
                Some(f) => {
                    out.push((format!("FLT  CF {}  IL {:.2} dB  Q {:.1}", display::fmt_hz(f.center_hz), f.loss_db, f.q()), palette::TEXT));
                    out.push((format!("     3dB {}–{} ({})", display::fmt_hz(f.lo3), display::fmt_hz(f.hi3), display::fmt_hz(f.bw3())), palette::TEXT));
                    out.push((format!("     6dB {}–{} ({})", display::fmt_hz(f.lo6), display::fmt_hz(f.hi6), display::fmt_hz(f.bw6())), palette::TEXT));
                }
                None => out.push(("FLT  edges outside the sweep".into(), palette::TEXT_MUTED)),
            }
        }
        if self.notch_on && s21 {
            let db: Vec<f64> = samples.iter().map(|s| 20.0 * s.s21.norm().max(1e-12).log10()).collect();
            match search::notch(&xs, &db) {
                Some(n) => {
                    out.push((format!("NOTCH {}  DEPTH {:.1} dB  Q {:.1}", display::fmt_hz(n.hz), n.depth_db, n.q()), palette::TEXT));
                    out.push((format!("     3dB {}–{} ({})", display::fmt_hz(n.lo3), display::fmt_hz(n.hi3), display::fmt_hz(n.bw3())), palette::TEXT));
                    if let (Some(lo), Some(hi)) = (n.lo_half, n.hi_half) {
                        out.push((format!("     half-depth {}–{} ({})", display::fmt_hz(lo), display::fmt_hz(hi), display::fmt_hz(hi - lo)), palette::TEXT));
                    }
                }
                None => out.push(("NOTCH none (or edges outside the sweep)".into(), palette::TEXT_MUTED)),
            }
        }
        if self.swr_bw_on && !s21 {
            let swr: Vec<f64> = samples.iter().map(|s| rf::swr(s.s11)).collect();
            if let Some(b) = search::swr_band(&xs, &swr, 2.0) {
                out.push((format!("SWR  min {:.2} @ {}", b.min_swr, display::fmt_hz(b.min_hz)), palette::TEXT));
                out.push(match b.edges {
                    Some((lo, hi)) => (format!("     2:1 {}–{} ({})", display::fmt_hz(lo), display::fmt_hz(hi), display::fmt_hz(hi - lo)), palette::TEXT),
                    None => ("     never under 2:1".into(), palette::TEXT_MUTED),
                });
            }
        }
        out
    }

    /// Start continuous sweeps, or ask for a zone scan when there is no calibration.
    fn start_run(&mut self, live: &device::Live) {
        if self.wizard.is_some() {
            self.note = "Finish the calibration before Run.".into();
        } else if self.setup_has_cal(live) {
            self.sweep_zone = self.zone_name();
            self.shown_scan = None;
            self.shown_id.clear();
            self.history.clear();
            self.blank_until_sweep = true;
            self.run = true;
        } else {
            self.begin_scan(live);
        }
    }

    /// START, STOP, CENTER, SPAN, POINTS, AVG and band presets beside the
    /// chart, so the Setup dialog is not needed for a sweep change. Issue 68.
    fn sweep_panel(&mut self, ui: &mut Ui, theme: &LcarsTheme, live: &device::Live, width: f32, col4: f32) {
        let max_hz = live.identity.map(|id| id.max_hz()).unwrap_or(3.0e9);
        let floor = proto::FREQ_FLOOR_HZ as f64;
        let start = parse_hz(&self.start).map(|v| v as f64).unwrap_or(self.face.start_hz);
        let stop = parse_hz(&self.stop).map(|v| v as f64).unwrap_or(self.face.stop_hz);
        let points: i64 = self.points.trim().parse().unwrap_or(self.face.points as i64);
        let (center, span) = sweep::center_span(start, stop);
        let field_w = col4 * 2.0 + 8.0;
        let mut apply: Option<(f64, f64, i64)> = None;
        let field = |ui: &mut Ui, label: &str, value: String| -> Option<String> {
            let mut out = None;
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(label).size(theme.data_size * 0.75).color(palette::TEXT_MUTED));
                let id = egui::Id::unique(("vna-sweep", label));
                let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or(value.clone());
                let resp = ui.add_sized(
                    vec2(field_w, KEY_H - 6.0),
                    egui::TextEdit::singleline(&mut text).font(egui::TextStyle::Monospace).id(id.with("edit")),
                );
                if resp.has_focus() {
                    ui.data_mut(|d| d.insert_temp(id, text.clone()));
                } else {
                    ui.data_mut(|d| d.remove::<String>(id));
                }
                if resp.lost_focus() && text.trim() != value.trim() {
                    out = Some(text);
                }
            });
            out
        };
        let mut bad = false;
        ui.horizontal(|ui| {
            if let Some(t) = field(ui, "START", display::fmt_hz(start)) {
                match sweep::parse_freq(&t) {
                    Some(v) => apply = Some((v, stop, points)),
                    None => bad = true,
                }
            }
            if let Some(t) = field(ui, "STOP", display::fmt_hz(stop)) {
                match sweep::parse_freq(&t) {
                    Some(v) => apply = Some((start, v, points)),
                    None => bad = true,
                }
            }
        });
        ui.horizontal(|ui| {
            if let Some(t) = field(ui, "CENTER", display::fmt_hz(center)) {
                match sweep::parse_freq(&t) {
                    Some(v) => {
                        let (a, b) = sweep::start_stop(v, span);
                        apply = Some((a, b, points));
                    }
                    None => bad = true,
                }
            }
            if let Some(t) = field(ui, "SPAN", display::fmt_hz(span)) {
                match sweep::parse_freq(&t) {
                    Some(v) => {
                        let (a, b) = sweep::start_stop(center, v);
                        apply = Some((a, b, points));
                    }
                    None => bad = true,
                }
            }
        });
        let mut avg_text: Option<String> = None;
        ui.horizontal(|ui| {
            if let Some(t) = field(ui, "POINTS", points.to_string()) {
                match t.trim().parse::<i64>() {
                    Ok(n) => apply = Some((start, stop, n)),
                    Err(_) => bad = true,
                }
            }
            avg_text = field(ui, "AVG (HOST)", self.host_avg.clone());
        });
        if bad {
            self.note = "Frequencies take k, M or G (a bare number is MHz). Points is a whole number.".into();
        }
        if let Some(t) = avg_text {
            match t.trim().parse::<u32>() {
                Ok(n) if n >= 1 => {
                    self.host_avg = n.to_string();
                    self.commit_fields();
                }
                _ => self.note = "AVG is a whole number of sweeps, 1 or more.".into(),
            }
        }
        if let Some((a, b, n)) = apply {
            let c = sweep::clamp(a, b, n, floor, max_hz, proto::POINTS_MAX as u16);
            self.start = hz_field(c.start);
            self.stop = hz_field(c.stop);
            self.points = c.points.to_string();
            self.note = if c.notes.is_empty() { String::new() } else { format!("Clamped: {}.", c.notes.join("; ")) };
        }
        // Device averaging needs firmware 1.0.2 and a start at or above 140 MHz.
        let fw_ok = live.identity.is_some_and(|id| id.power_and_average());
        ui.horizontal(|ui| {
            let ok = fw_ok && start >= 140e6;
            if softkey_enabled(ui, theme, "Dev avg", false, col4, ok) {
                if let Ok(count) = self.dev_avg.trim().parse::<u8>() {
                    if count >= 1 {
                        self.session.average(count);
                    }
                }
            }
            ui.add_enabled(
                ok,
                egui::TextEdit::singleline(&mut self.dev_avg).desired_width(40.0).font(egui::TextStyle::Monospace),
            );
            let why = if !live.connected {
                "connect first"
            } else if !fw_ok {
                "needs FW 1.0.2"
            } else if start < 140e6 {
                "start ≥ 140 MHz"
            } else {
                "on device"
            };
            ui.label(egui::RichText::new(why).size(theme.data_size * 0.75).color(palette::TEXT_MUTED));
        });
        let presets = sweep::band_presets(floor, max_hz);
        let mut pick: Option<(f64, f64)> = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("BAND").size(theme.data_size * 0.75).color(palette::TEXT_MUTED));
            egui::ComboBox::from_id_salt("vna-band-preset")
                .width((width - 60.0).max(80.0))
                .selected_text("Ham band preset")
                .show_ui(ui, |ui| {
                    for (name, lo, hi) in &presets {
                        let text = format!("{name}  {} – {}", display::fmt_hz(*lo), display::fmt_hz(*hi));
                        if ui.selectable_label(false, text).clicked() {
                            pick = Some((*lo, *hi));
                        }
                    }
                });
        });
        if let Some((lo, hi)) = pick {
            self.apply_edges(lo, hi);
            self.note.clear();
        }
    }

    /// Move the active marker: 0 max, 1 min, 2 next peak, 3 clear. Issue 67.
    fn marker_search(&mut self, code: u8, samples: Option<&[Sample]>) {
        if code == 3 {
            self.markers[self.marker_on] = None;
            return;
        }
        let Some(samples) = samples else {
            return;
        };
        if self.view == View::Tdr {
            self.note = "Marker search works on frequency traces.".into();
            return;
        }
        let (_, ys) = series(self.view, samples, self.z0_value(), self.velocity_value());
        let at = self.marker_index(self.marker_on, samples);
        let found = match code {
            0 => display::search_max(&ys),
            1 => display::search_min(&ys),
            _ => display::next_peak(&ys, at),
        };
        match found {
            Some(index) => self.markers[self.marker_on] = samples.get(index).map(|s| s.hz),
            None => self.note = "No further peak on this trace.".into(),
        }
    }

    /// Gamma, SWR, return loss, mismatch loss, and impedance for the selected marker.
    fn marker_block(&self, ui: &mut Ui, theme: &LcarsTheme, samples: Option<&[Sample]>) {
        let Some(samples) = samples else {
            readout(ui, theme, "No calibrated sweep yet.", false);
            return;
        };
        let Some(index) = self.marker_index(self.marker_on, samples) else {
            readout(ui, theme, "Click the chart to place the selected marker.", false);
            return;
        };
        let Some(sample) = samples.get(index) else {
            readout(ui, theme, "The selected marker is off this sweep.", false);
            return;
        };
        let z0 = self.z0_value();
        let gamma = sample.s11;
        let mag = gamma.abs();
        let z = rf::impedance(gamma, z0).unwrap_or(Cx::new(f64::INFINITY, f64::INFINITY));
        let zn = if z0 > 0.0 {
            z.scale(1.0 / z0)
        } else {
            Cx::new(f64::INFINITY, f64::INFINITY)
        };
        let y = {
            let n = zn.norm();
            if n == 0.0 || !n.is_finite() {
                Cx::new(f64::INFINITY, f64::INFINITY)
            } else {
                Cx::new(zn.re / n, -zn.im / n)
            }
        };
        let swr = if mag >= 1.0 {
            "INF : 1".into()
        } else {
            format!("{} : 1", fmt_g((1.0 + mag) / (1.0 - mag)))
        };
        let rl = if mag <= 0.0 {
            "INF dB".into()
        } else {
            format!("{} dB", fmt_g(-20.0 * mag.log10()))
        };
        let ml = if mag >= 1.0 {
            "INF dB".into()
        } else {
            format!("{} dB", fmt_g(-10.0 * (1.0 - mag * mag).log10()))
        };
        readout(
            ui,
            theme,
            &format!("{}  {} MHz", markers::slot_name(self.marker_on), fmt_g(sample.hz / 1.0e6)),
            true,
        );
        for (name, value) in [
            (
                "GAMMA",
                format!(
                    "{} at {} deg",
                    fmt_g(mag),
                    fmt_g(gamma.arg().to_degrees())
                ),
            ),
            ("SWR", swr),
            ("RETURN LOSS", rl),
            ("MISMATCH LOSS", ml),
            ("z = Z / Z0", fmt_cx(crate::tools::smith::math::cx(zn.re, zn.im))),
            ("y = 1 / z", fmt_cx(crate::tools::smith::math::cx(y.re, y.im))),
            ("Z (ohm)", fmt_cx(crate::tools::smith::math::cx(z.re, z.im))),
        ] {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(name).color(palette::TEXT_MUTED));
                readout(ui, theme, &value, false);
            });
        }
    }

    fn finder_readout(&self, ui: &mut Ui, theme: &LcarsTheme, samples: &[Sample]) {
        if let (Some(a), Some(b)) = (self.marker_index(0, samples), self.marker_index(1, samples)) {
            if let (Some(left), Some(right)) = (samples.get(a), samples.get(b)) {
                readout(
                    ui,
                    theme,
                    &format!(
                        "Δ M1−M2  {} Hz  Δ SWR {}  Δ RL {} dB",
                        (right.hz - left.hz).round(),
                        num(rf::swr(right.s11) - rf::swr(left.s11)),
                        num(rf::return_loss_db(right.s11) - rf::return_loss_db(left.s11)),
                    ),
                    false,
                );
            }
        }
        let z0 = self.z0_value();
        if let Some(i) = display::search_min(&samples.iter().map(|s| rf::swr(s.s11)).collect::<Vec<_>>()) {
            let s = &samples[i];
            readout(ui, theme, &format!("Lowest SWR {} : 1 at {}", num(rf::swr(s.s11)), display::fmt_hz(s.hz)), false);
        }
        for item in rf::x_resonances(samples, z0).into_iter().take(4) {
            readout(
                ui,
                theme,
                &format!("{} resonance {} Hz", item.kind, item.hz.round()),
                false,
            );
        }
        if let Some(peak) = rf::s21_peak(samples) {
            readout(
                ui,
                theme,
                &format!(
                    "S21 peak {} Hz  3 dB {} Hz  6 dB {} Hz  Q {}",
                    peak.hz.round(),
                    num(peak.bw3_hz),
                    num(peak.bw6_hz),
                    num(peak.q),
                ),
                false,
            );
        }
        if self.view == View::Tdr {
            if let Ok((limit, trace)) = rf::distance_to_fault(samples, self.velocity_value()) {
                if let Some(peak) = trace
                    .iter()
                    .max_by(|a, b| a.magnitude.partial_cmp(&b.magnitude).unwrap_or(std::cmp::Ordering::Equal))
                {
                    readout(
                        ui,
                        theme,
                        &format!(
                            "Fault {} m  range limit {} m  cable loss {} dB",
                            num(peak.metres),
                            num(limit),
                            num(cable_loss(samples)),
                        ),
                        false,
                    );
                }
            }
        }
    }

    fn ingest(&mut self, raw: &[Sample]) {
        let keep = self.host_average_value().max(1);
        let sweep_for = self.sweep_zone.clone();
        let selected = self.zone_name();
        // A calibration piece is drawn as it arrives. A zone sweep updates the
        // chart only when that zone is the one on screen.
        let show_this = self.pending.is_some()
            || match &sweep_for {
                Some(zone) => selected.as_deref() == Some(zone.as_str()),
                None => true,
            };
        if show_this {
            self.shown_scan = None;
            self.shown_id.clear();
            self.blank_until_sweep = false;
            self.history.push_back(raw.to_vec());
            while self.history.len() > keep {
                self.history.pop_front();
            }
        }
        if let Some(standard) = self.pending.take() {
            self.store_standard(standard, raw);
        } else if let Some(zone) = sweep_for {
            self.store_scan(&zone, raw);
            if !self.run {
                self.sweep_zone = None;
            }
        }
    }

    fn store_scan(&mut self, zone: &str, raw: &[Sample]) {
        let Some(id) = self.session.snapshot().device_id else {
            return;
        };
        if raw.len() < 2 {
            return;
        }
        let id = proto::device_id_hex(&id);
        let scan = StoredScan {
            id: id.clone(),
            zone: zone.to_string(),
            start_hz: raw[0].hz,
            step_hz: raw[1].hz - raw[0].hz,
            s11: raw.iter().map(|sample| [sample.s11.re, sample.s11.im]).collect(),
            s21: raw.iter().map(|sample| [sample.s21.re, sample.s21.im]).collect(),
        };
        if let Some(found) = self
            .scans
            .iter_mut()
            .find(|stored| stored.id == scan.id && stored.zone == scan.zone)
        {
            *found = scan;
        } else {
            self.scans.push(scan);
        }
        self.archive_dirty = true;
    }

    fn zone_name(&self) -> Option<String> {
        self.plan.zones.get(self.zone_sel).map(|zone| zone.name.clone())
    }

    /// The zone tab that is measuring right now.
    fn scanning_zone<'a>(&'a self, live: &'a device::Live) -> Option<&'a str> {
        if let Some((zone, _, _)) = &self.await_scan {
            return Some(zone.as_str());
        }
        let active = self.run
            || self.run_state != Run::Idle
            || live.busy
            || self.pending.is_some();
        if self.wizard.is_some() {
            if self.pending.is_some() || self.run_state != Run::Idle || live.busy {
                return self.wizard.as_ref().and_then(|wiz| wiz.current()).map(|span| span.zone.as_str());
            }
            return None;
        }
        if active {
            return self.sweep_zone.as_deref();
        }
        None
    }

    fn scan_for(&self, zone: &str, device: Option<&str>) -> Option<&StoredScan> {
        if let Some(id) = device {
            if let Some(scan) = self.scans.iter().rev().find(|scan| scan.zone == zone && scan.id == id)
            {
                return Some(scan);
            }
        }
        self.scans.iter().rev().find(|scan| scan.zone == zone)
    }

    fn recall_selected(&mut self) {
        let Some(name) = self.zone_name() else {
            return;
        };
        let id = self
            .session
            .snapshot()
            .device_id
            .map(|bytes| proto::device_id_hex(&bytes));
        let Some(scan) = self.scan_for(&name, id.as_deref()).cloned() else {
            let live = self.session.snapshot();
            self.request_zone(&live);
            return;
        };
        self.history.clear();
        self.blank_until_sweep = false;
        self.shown_id = scan.id.clone();
        self.shown_scan = Some(samples_of(&scan));
        self.note = format!("Showing the stored scan for {name}.");
    }

    fn store_standard(&mut self, standard: Standard, raw: &[Sample]) {
        let Some(id) = self.session.snapshot().device_id else {
            self.note = "Connect the NanoVNA before storing a calibration.".into();
            return;
        };
        let id = proto::device_id_hex(&id);
        if raw.len() < 2 {
            return;
        }
        let start = raw[0].hz;
        let step = raw[1].hz - raw[0].hz;
        let n = raw.len();
        let z0 = self.z0_value();
        let band = self
            .wizard
            .as_ref()
            .and_then(|wiz| wiz.current().map(|span| (span.zone.clone(), String::new())));
        let found = self.face.cals.iter_mut().position(|set| {
            set.id == id
                && match &band {
                    Some((zone, name)) => set.zone == *zone && set.band == *name,
                    None => {
                        (set.start_hz - start).abs() < 1.0
                            && (set.step_hz - step).abs() < 1.0
                            && (set.open.len() == n || set.open.is_empty())
                    }
                }
        });
        if let Some(index) = found {
            let set = &mut self.face.cals[index];
            // A new grid cannot keep standards measured on the old one.
            // Leaving them would fail the station-file length check.
            if !same_grid(set, start, step, n) {
                set.open.clear();
                set.short.clear();
                set.load.clear();
                set.thru.clear();
                set.isolation.clear();
            }
            set.start_hz = start;
            set.step_hz = step;
            set.z0 = z0;
            if let Some((zone, name)) = &band {
                set.zone = zone.clone();
                set.band = name.clone();
            }
            fill_standard(set, standard, raw);
            set.saved_at = Some(cal_stamp_now());
        } else {
            let (zone, name) = band.clone().unwrap_or_default();
            let mut set = CalSet {
                id,
                zone,
                band: name,
                name: String::new(),
                start_hz: start,
                step_hz: step,
                z0,
                open: Vec::new(),
                short: Vec::new(),
                load: Vec::new(),
                thru: Vec::new(),
                isolation: Vec::new(),
                saved_at: Some(cal_stamp_now()),
            };
            fill_standard(&mut set, standard, raw);
            self.face.cals.push(set);
        }
        self.archive_dirty = true;
        self.advance_batch(standard);
    }

    /// The piece stays fitted. The next zone is swept immediately. The next
    /// piece waits until every zone in this pass has stored this one.
    fn advance_batch(&mut self, standard: Standard) {
        let Some(wiz) = self.wizard.clone() else {
            self.note = "Saved in the VNA file.".into();
            return;
        };
        if wiz.only_step && wiz.at + 1 >= wiz.spans.len() {
            let saved = wiz.step.piece_saved();
            self.wizard = None;
            self.note = saved.into();
            return;
        }
        let (step, at) = next_batch(wiz.step, wiz.at, wiz.spans.len());
        let same_piece = step == wiz.step;
        if let Some(w) = self.wizard.as_mut() {
            w.step = step;
            w.at = at;
        }
        if !same_piece {
            self.note = wiz.step.done_note().into();
            // The last piece has no further sweep. Leave the dialog instead of
            // a Scan button that does not measure a calibration piece.
            if step == WizStep::Finish {
                self.wizard = None;
            }
            return;
        }
        let Some(span) = self.wizard.as_ref().and_then(|w| w.current().cloned()) else {
            return;
        };
        self.sweep_zone = Some(span.zone.clone());
        self.apply_edges(span.lo, span.hi);
        self.pending = Some(standard);
        let count = wiz.spans.len();
        if self.send_sweep() {
            self.run_state = Run::Queued;
            self.note = measure_note(wiz.step, &span.zone, at, count);
        }
    }

    fn setup_has_cal(&self, live: &device::Live) -> bool {
        let Some(raw_id) = live.device_id else {
            return false;
        };
        let id = proto::device_id_hex(&raw_id);
        let Ok((start, stop, points)) = self.parse_setup() else {
            return false;
        };
        let step = ((stop - start) as f64 / f64::from(points - 1))
            .round()
            .max(1.0);
        if self.cal_off {
            return true;
        }
        let _ = &id;
        self.face.cals.iter().any(|set| {
            cal_complete(set)
                && ((set.open.len() == points as usize && same_grid(set, start as f64, step, points as usize))
                    || cal::covers(set, start as f64, stop as f64))
        })
    }

    fn request_zone(&mut self, live: &device::Live) {
        let Some(zone) = self.plan.zones.get(self.zone_sel).cloned() else {
            self.note = "This station has no band plan to calibrate.".into();
            return;
        };
        let Some((lo, hi)) = zone_limits(&zone) else {
            self.note = format!("{} has no frequency limits.", zone.name);
            return;
        };
        self.choose_span(live, &zone.name, lo, hi);
    }

    /// Scan the selected tab. Without a stored calibration this opens the
    /// wizard and does not sweep the device under test.
    fn begin_scan(&mut self, live: &device::Live) {
        self.run = false;
        self.run_state = Run::Idle;
        if let Some(wiz) = self.wizard.clone() {
            let name = if wiz.step == WizStep::Finish {
                "Scan".to_string()
            } else {
                wiz.step.action("")
            };
            self.note = format!(
                "Use {name} in the calibration dialog. Scan on this row waits until the open, the short, and the 50 ohm load are stored."
            );
            return;
        }
        if self.plan.zones.is_empty() {
            if self.setup_has_cal(live) && self.send_sweep() {
                self.run_state = Run::Queued;
            } else {
                self.note = "This sweep has no calibration. A scan needs open, short, and load.".into();
            }
            return;
        }
        self.request_zone(live);
    }

    fn apply_edges(&mut self, lo: f64, hi: f64) {
        let max = self
            .session
            .snapshot()
            .identity
            .map(|id| id.max_hz())
            .unwrap_or(3.0e9);
        let lo = lo.max(proto::FREQ_FLOOR_HZ as f64).min(max - 1.0);
        let hi = hi.min(max).max(lo + 1.0);
        self.start = format!("{:.0}", lo.round());
        self.stop = format!("{:.0}", hi.round());
    }

    fn apply_grid(&mut self, cal: &CalSet) {
        let n = cal
            .open
            .len()
            .max(cal.short.len())
            .max(cal.load.len())
            .clamp(2, proto::POINTS_MAX);
        let stop = cal.start_hz + cal.step_hz * (n as f64 - 1.0);
        self.start = format!("{:.0}", cal.start_hz.round());
        self.stop = format!("{:.0}", stop.round());
        self.points = n.to_string();
    }

    fn programmed_edges(&self, lo: f64, hi: f64) -> (f64, f64) {
        let max = self
            .session
            .snapshot()
            .identity
            .map(|id| id.max_hz())
            .unwrap_or(3.0e9);
        let lo = lo.max(proto::FREQ_FLOOR_HZ as f64).min(max - 1.0);
        let hi = hi.min(max).max(lo + 1.0);
        (lo.round(), hi.round())
    }

    fn choose_span(&mut self, live: &device::Live, zone: &str, lo: f64, hi: f64) {
        self.run = false;
        self.run_state = Run::Idle;
        let (lo, hi) = self.programmed_edges(lo, hi);
        let Some(id) = live.device_id else {
            self.wizard = None;
            if self.face.port.is_empty() {
                self.await_scan = None;
                self.note = "Choose the NanoVNA port in Setup, then click the zone.".into();
                return;
            }
            // A double-click is a request to use the saved port, including
            // after Close on this visit.
            self.user_closed = false;
            self.await_scan = Some((zone.to_string(), lo, hi));
            if !live.connected && !live.busy {
                self.session.connect(self.face.port.clone());
            }
            self.note = format!("Connecting. {zone} calibrates when the NanoVNA answers.");
            return;
        };
        self.await_scan = None;
        let id = proto::device_id_hex(&id);
        if let Some(cal) = find_zone_cal(&self.face.cals, &id, zone, lo, hi).cloned() {
            self.wizard = None;
            self.apply_grid(&cal);
            self.sweep_zone = Some(zone.to_string());
            self.shown_scan = None;
            self.shown_id.clear();
            self.history.clear();
            self.blank_until_sweep = true;
            self.note = if other_device(&cal, Some(&id)) {
                format!(
                    "Scanning {zone} with the calibration saved {} under device id {} (this connection reports {}). The V2.2 id is not stable; check a known load if in doubt.",
                    cal_date_text(cal.saved_at.as_deref()),
                    cal.id,
                    id
                )
            } else if restored(&cal, &self.run_started) {
                format!("Scanning {zone} with the restored calibration saved {}.", cal_date_text(cal.saved_at.as_deref()))
            } else {
                format!("Scanning {zone} with the stored calibration.")
            };
            if self.send_sweep() {
                self.run_state = Run::Queued;
            } else {
                self.sweep_zone = None;
                self.blank_until_sweep = false;
            }
            return;
        }
        let spans = self.missing_zones(&id);
        if spans.is_empty() {
            self.note = format!("{zone} has no frequency limits to calibrate.");
            return;
        }
        self.note.clear();
        self.wizard = Some(Wizard {
            spans,
            at: 0,
            step: WizStep::Open,
            only_step: false,
        });
    }

    fn all_zone_spans(&self) -> Vec<ZoneSpan> {
        let mut spans = Vec::new();
        for zone in &self.plan.zones {
            let Some((lo, hi)) = zone_limits(zone) else {
                continue;
            };
            let (lo, hi) = self.programmed_edges(lo, hi);
            spans.push(ZoneSpan {
                zone: zone.name.clone(),
                lo,
                hi,
            });
        }
        spans
    }

    fn start_wizard(&mut self, live: &device::Live, spans: Vec<ZoneSpan>, step: WizStep, only_step: bool) {
        if live.device_id.is_none() {
            self.note = "Connect the NanoVNA before a rescan.".into();
            return;
        }
        if spans.is_empty() {
            self.note = "This station has no band plan to calibrate.".into();
            return;
        }
        self.note.clear();
        self.wizard = Some(Wizard {
            spans,
            at: 0,
            step,
            only_step,
        });
    }

    /// Every plan zone that does not yet have open, short, and load for this device.
    fn missing_zones(&self, id: &str) -> Vec<ZoneSpan> {
        let mut spans = Vec::new();
        for zone in &self.plan.zones {
            let Some((lo, hi)) = zone_limits(zone) else {
                continue;
            };
            let (lo, hi) = self.programmed_edges(lo, hi);
            if find_zone_cal(&self.face.cals, id, &zone.name, lo, hi).is_some() {
                continue;
            }
            spans.push(ZoneSpan {
                zone: zone.name.clone(),
                lo,
                hi,
            });
        }
        spans
    }

    fn wizard_measure(&mut self) {
        let Some(wiz) = self.wizard.clone() else {
            return;
        };
        let Some(standard) = wiz.step.standard() else {
            self.wizard = None;
            self.pending = None;
            let live = self.session.snapshot();
            self.request_zone(&live);
            return;
        };
        if let Some(w) = self.wizard.as_mut() {
            w.at = 0;
        }
        let Some(span) = wiz.spans.first().cloned() else {
            self.note = "This station has no band plan to calibrate.".into();
            return;
        };
        self.sweep_zone = Some(span.zone.clone());
        self.apply_edges(span.lo, span.hi);
        self.pending = Some(standard);
        self.run = false;
        self.run_state = Run::Idle;
        if self.send_sweep() {
            self.run_state = Run::Queued;
            self.note = measure_note(wiz.step, &span.zone, 0, wiz.spans.len());
        }
    }

    fn wizard_skip(&mut self) {
        if self.wizard.as_ref().is_some_and(|wiz| wiz.only_step) {
            self.wizard = None;
            self.pending = None;
            self.note = "Rescan skipped.".into();
            return;
        }
        let outcome = {
            let Some(wiz) = self.wizard.as_mut() else {
                return;
            };
            // Each optional step has its own skip. The cable does not drop the
            // two-load step, and neither skip drops the open, short, or load.
            // Skipping the last piece closes the dialog.
            match wiz.step {
                WizStep::Thru => {
                    wiz.step = WizStep::Isol;
                    wiz.at = 0;
                    ("Cable skipped for every zone.", false)
                }
                WizStep::Isol => ("Loads skipped. Calibration stored.", true),
                _ => return,
            }
        };
        // A sweep already in flight is not stored as this piece.
        self.pending = None;
        if outcome.1 {
            self.wizard = None;
        }
        self.note = outcome.0.into();
    }

    fn band_strip(&mut self, ui: &mut Ui, theme: &LcarsTheme, live: &device::Live) {
        let mut measure = false;
        let mut recall = false;
        let names: Vec<String> = self.plan.zones.iter().map(|zone| zone.name.clone()).collect();
        let scanning = self.scanning_zone(live).map(str::to_string);
        let device_id = live.device_id.map(|bytes| proto::device_id_hex(&bytes));
        ui.horizontal_wrapped(|ui| {
                let cal = crate::folder_tab(ui, self.cal_page, 5, "Cal", true);
                if cal.clicked() {
                    self.cal_page = true;
                }
                for (idx, name) in names.iter().enumerate() {
                    let label = if name.trim().is_empty() { "Zone" } else { name };
                    let hot = scanning.as_deref() == Some(name.as_str());
                    let resp = crate::folder_tab_colored(
                        ui,
                        !self.cal_page && self.zone_sel == idx,
                        idx,
                        label,
                        false,
                        hot.then_some(SCAN_TAB),
                        None,
                    );
                    if resp.clicked() || resp.double_clicked() {
                        self.cal_page = false;
                        self.zone_sel = idx;
                        let has_scan = self.scan_for(name, device_id.as_deref()).is_some();
                        if zone_click(has_scan, resp.double_clicked()) == ZoneClick::Measure {
                            measure = true;
                            recall = false;
                        } else {
                            measure = false;
                            recall = true;
                        }
                    }
                }
        });
        // The zone tabs and the elbow under them used to share an edge.
        ui.add_space(theme.frame_gap);
        if measure {
            self.request_zone(live);
        } else if recall {
            self.recall_selected();
        }
    }

    fn cal_body(&mut self, ui: &mut Ui, theme: &LcarsTheme, live: &device::Live) {
        let id = live.device_id.map(|bytes| proto::device_id_hex(&bytes));
        if let Some(note) = device::power_note(live) {
            ui.label(
                egui::RichText::new(note)
                    .monospace()
                    .strong()
                    .size(theme.data_size)
                    .color(LOST_TEXT),
            );
        }
        if !self.note.is_empty() {
            ui.label(
                egui::RichText::new(&self.note)
                    .monospace()
                    .size(theme.data_size)
                    .color(palette::TEXT_MUTED),
            );
        }
        ui.horizontal(|ui| {
            if action_button(ui, theme, "Rescan all", vec2(140.0, 32.0)).clicked() {
                let spans = self.all_zone_spans();
                self.start_wizard(live, spans, WizStep::Open, false);
            }
            let label = if self.cal_off { "Correction: OFF" } else { "Correction: ON" };
            if action_button(ui, theme, label, vec2(170.0, 32.0)).clicked() {
                self.cal_off = !self.cal_off;
            }
        });
        let rows: Vec<(String, f64, f64)> = self
            .plan
            .zones
            .iter()
            .filter_map(|zone| zone_limits(zone).map(|(lo, hi)| (zone.name.clone(), lo, hi)))
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("nanovna-cal")
            .auto_shrink([false, false])
            .max_height(ui.available_height())
            .show(ui, |ui| {
                if rows.is_empty() {
                    readout(ui, theme, "This station has no band plan.", false);
                }
                for (name, lo, hi) in &rows {
                    let (lo, hi) = self.programmed_edges(*lo, *hi);
                    ui.label(
                        egui::RichText::new(format!("{name}  {:.3}-{:.3} MHz", lo / 1e6, hi / 1e6))
                            .monospace()
                            .size(theme.data_size)
                            .color(theme.text_hi),
                    );
                    let stored = self.zone_cal(id.as_deref(), name).cloned();
                    if let Some(set) = &stored {
                        // Rename and delete the stored set. Delete needs a double click. Issue 65.
                        let at = self.face.cals.iter().rposition(|c| c == set);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("NAME").color(palette::TEXT_MUTED));
                            let mut text = set.name.clone();
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut text)
                                    .hint_text(name.as_str())
                                    .desired_width(180.0)
                                    .font(egui::TextStyle::Monospace),
                            );
                            if resp.changed() {
                                if let Some(i) = at {
                                    self.face.cals[i].name = text.trim().to_string();
                                    self.archive_dirty = true;
                                }
                            }
                            ui.label(
                                egui::RichText::new(format!("SAVED {}", cal_date_text(set.saved_at.as_deref())))
                                    .monospace()
                                    .color(palette::TEXT_MUTED),
                            );
                            let del = action_button(ui, theme, "Delete", vec2(90.0, 26.0))
                                .on_hover_text("Double-click to delete this calibration set");
                            if del.double_clicked() {
                                if let Some(i) = at {
                                    self.face.cals.remove(i);
                                    self.archive_dirty = true;
                                    self.note = format!("Deleted the {name} calibration.");
                                }
                            }
                        });
                    }
                    for step in [WizStep::Open, WizStep::Short, WizStep::Load, WizStep::Thru, WizStep::Isol]
                    {
                        let summary = step_summary(stored.as_ref(), step);
                        ui.horizontal(|ui| {
                            ui.set_min_height(super::LIST_ROW);
                            let has = stored
                                .as_ref()
                                .is_some_and(|set| !step_samples(set, step).is_empty());
                            let graph = rustcars::lcars_button(
                                ui,
                                theme,
                                if has { "Graph" } else { "not stored" },
                                palette::SLATE,
                                vec2(64.0, 26.0),
                                rustcars::Cap::None,
                                has,
                            )
                            .on_hover_text(format!(
                                "View graph of the {} standard in {name}.\nClick: Smith (polar for S21) · Double: return loss (S21 dB) · Triple: phase",
                                step.step_name()
                            ));
                            // Single, double and triple click pick the first view. Issue 75.
                            let pick = if click_n(ui, &graph) == 3 {
                                Some(2)
                            } else if click_n(ui, &graph) == 2 {
                                Some(1)
                            } else if click_n(ui, &graph) == 1 {
                                Some(0)
                            } else {
                                None
                            };
                            if let (Some(mode), Some(set)) = (pick, stored.clone()) {
                                let s21 = matches!(step, WizStep::Thru | WizStep::Isol);
                                self.cal_graph_view = match (mode, s21) {
                                    (0, false) => View::Smith,
                                    (0, true) => View::PolarS21,
                                    (1, false) => View::ReturnLoss,
                                    (1, true) => View::S21,
                                    (_, false) => View::PhaseS11,
                                    (_, true) => View::Phase,
                                };
                                self.cal_graph = Some((set, step));
                                self.cal_compare = None;
                                self.cal_graph_marks.clear();
                            }
                            if reload_button(ui, theme).clicked() {
                                self.start_wizard(
                                    live,
                                    vec![ZoneSpan {
                                        zone: name.clone(),
                                        lo,
                                        hi,
                                    }],
                                    step,
                                    true,
                                );
                            }
                            ui.label(
                                egui::RichText::new(format!(
                                    "{:<8} {summary}",
                                    step.step_name()
                                ))
                                .monospace()
                                .size(theme.data_size)
                                .color(palette::TEXT),
                            );
                        });
                    }
                }
            });
    }

    /// One stored standard: the chosen format with an ideal reference, the worst
    /// point, an optional compare set, the solved error terms, PNG and CSV. Issue 75.
    fn cal_graph_dialog(&mut self, ui: &mut Ui, theme: &LcarsTheme) {
        let Some((set, step)) = self.cal_graph.clone() else {
            return;
        };
        let kind = match step {
            WizStep::Open => calstd::Kind::Open,
            WizStep::Short => calstd::Kind::Short,
            WizStep::Load => calstd::Kind::Load,
            WizStep::Thru => calstd::Kind::Thru,
            _ => calstd::Kind::Isol,
        };
        let s21 = kind.is_s21();
        let values = pairs(step_samples(&set, step));
        let to_samples = |vals: &[Cx]| -> Vec<Sample> {
            vals.iter()
                .enumerate()
                .map(|(i, v)| Sample { hz: calstd::hz(set.start_hz, set.step_hz, i), s11: *v, s21: *v })
                .collect()
        };
        let pts = to_samples(&values);
        // The ideal null is drawn at the floor, not at minus infinity.
        let ideal = if kind.ideal().abs() == 0.0 { Cx::new(10f64.powf(calstd::NULL_DB / 20.0), 0.0) } else { kind.ideal() };
        let ideal_pts = to_samples(&vec![ideal; values.len()]);
        let others: Vec<(usize, String)> = self
            .face
            .cals
            .iter()
            .enumerate()
            .filter(|(_, c)| **c != set && !step_samples(c, step).is_empty())
            .map(|(i, c)| (i, format!("{} {} {}", if c.name.is_empty() { &c.zone } else { &c.name }, c.band, c.id)))
            .collect();
        let compare: Option<Vec<Sample>> = self
            .cal_compare
            .and_then(|i| self.face.cals.get(i))
            .map(|c| to_samples(&pairs(step_samples(c, step))));
        let worst = calstd::worst(kind, &values);
        let views: &[View] = if s21 {
            &[View::PolarS21, View::S21, View::Phase, View::Unwrap, View::GroupDelay]
        } else {
            &[View::Smith, View::ReturnLoss, View::PhaseS11, View::UnwrapS11]
        };
        if !views.contains(&self.cal_graph_view) {
            self.cal_graph_view = views[0];
        }
        let terms = rf::solve_osl(&pairs(&set.open), &pairs(&set.short), &pairs(&set.load)).ok();
        let mut open = true;
        let mut export: Option<bool> = None;
        let title = format!(
            "{} · {} · {}",
            if set.zone.is_empty() { "Calibration" } else { set.zone.as_str() },
            step.step_name(),
            "raw standard",
        );
        egui::Window::new(title)
            .open(&mut open)
            .resizable(true)
            .default_size(vec2(760.0, 480.0))
            .show(ui.ctx(), |ui| {
                let col = 96.0;
                ui.horizontal_wrapped(|ui| {
                    for v in views {
                        if softkey(ui, theme, view_name(*v), !self.cal_terms && self.cal_graph_view == *v, col) {
                            self.cal_graph_view = *v;
                            self.cal_terms = false;
                        }
                    }
                    if softkey_enabled(ui, theme, "Err terms", self.cal_terms, col, terms.is_some()) {
                        self.cal_terms = !self.cal_terms;
                    }
                });
                ui.horizontal(|ui| {
                    let current = self
                        .cal_compare
                        .and_then(|i| others.iter().find(|(j, _)| *j == i))
                        .map(|(_, n)| n.clone())
                        .unwrap_or_else(|| "Compare: off".into());
                    egui::ComboBox::from_id_salt("vna-cal-compare").selected_text(current).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.cal_compare, None, "off");
                        for (i, n) in &others {
                            ui.selectable_value(&mut self.cal_compare, Some(*i), n);
                        }
                    });
                    if softkey(ui, theme, "PNG", false, col) {
                        export = Some(true);
                    }
                    if softkey(ui, theme, "CSV", false, col) {
                        export = Some(false);
                    }
                });
                let worst_text = worst
                    .map(|w| {
                        format!(
                            " · worst {} at {}",
                            if matches!(kind, calstd::Kind::Load | calstd::Kind::Isol) {
                                format!("{:.1} dB", w.db)
                            } else {
                                format!("{:.2} dB off ideal", w.db)
                            },
                            display::fmt_hz(calstd::hz(set.start_hz, set.step_hz, w.index))
                        )
                    })
                    .unwrap_or_default();
                ui.label(
                    egui::RichText::new(format!(
                        "{} points · {} to {} · Z0 {} Ω · device {} · saved {}{worst_text} · uncorrected ratio as stored",
                        values.len(),
                        display::fmt_hz(pts.first().map(|s| s.hz).unwrap_or(0.0)),
                        display::fmt_hz(pts.last().map(|s| s.hz).unwrap_or(0.0)),
                        trim_num(set.z0),
                        if set.id.is_empty() { "-" } else { set.id.as_str() },
                        cal_date_text(set.saved_at.as_deref()),
                    ))
                    .monospace()
                    .size(theme.data_size * 0.85)
                    .color(palette::TEXT_MUTED),
                );
                let legend = if self.cal_terms {
                    "Directivity (gold) · Source match (teal) · Reflection tracking (plum), |term| dB".to_string()
                } else {
                    format!(
                        "Measured (gold, saved {}) · Ideal {} (grey){} · R = worst point",
                        cal_date_text(set.saved_at.as_deref()),
                        step.step_name().to_lowercase(),
                        if compare.is_some() { " · Compare (teal)" } else { "" }
                    )
                };
                ui.label(egui::RichText::new(legend).size(theme.data_size * 0.8).color(palette::TEXT_MUTED));
                // Marker readout first, so the graph keeps the rest of the height.
                if !self.cal_terms && !self.cal_graph_marks.is_empty() {
                    let view = self.cal_graph_view;
                    let text = self
                        .cal_graph_marks
                        .iter()
                        .enumerate()
                        .filter_map(|(k, i)| pts.get(*i).map(|p| format!("M{} {} {}", k + 1, display::fmt_hz(p.hz), marker_value(view, p, set.z0))))
                        .collect::<Vec<_>>()
                        .join("  ·  ");
                    ui.label(egui::RichText::new(text).monospace().size(theme.data_size * 0.85).color(palette::TEXT));
                }
                let h = (ui.available_height() - 4.0).max(240.0);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(320.0), h), Sense::click());
                let resp = resp.on_hover_text("Click: place M1 · Double-click or Shift+click: add a marker · Triple-click or Ctrl+click: clear markers");
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, PANE_BG);
                let xs: Vec<f64> = pts.iter().map(|s| s.hz).collect();
                // Clicking the graph places markers as on the live sweep. Issue 75.
                let op = click_n(ui, &resp);
                if op > 0 && !self.cal_terms {
                    let at = resp.interact_pointer_pos();
                    let index = at.and_then(|p| match self.cal_graph_view {
                        View::Smith => {
                            let d = smith_disk(rect);
                            calstd::nearest_gamma(&values, Cx::new(((p.x - d.center.x) / d.rad) as f64, (-(p.y - d.center.y) / d.rad) as f64))
                        }
                        View::PolarS21 => {
                            let plot = rect.shrink(16.0);
                            let r = plot.width().min(plot.height()) * 0.45;
                            let c = plot.center();
                            calstd::nearest_gamma(&values, Cx::new(((p.x - c.x) / r) as f64, (-(p.y - c.y) / r) as f64))
                        }
                        _ => {
                            let plot = cart_plot(rect);
                            let (lo, hi) = (xs.first().copied()?, xs.last().copied()?);
                            let f = ((p.x - plot.left()) / plot.width()).clamp(0.0, 1.0) as f64;
                            calstd::nearest_x(&xs, lo + f * (hi - lo))
                        }
                    });
                    calstd::place_marker(&mut self.cal_graph_marks, op, index, markers::NORMAL);
                }
                let placed: Vec<(usize, usize)> = self.cal_graph_marks.iter().enumerate().map(|(k, i)| (k, *i)).collect();
                if self.cal_terms {
                    if let Some(t) = &terms {
                        let d = calstd::term_db(&t.directivity);
                        let m = calstd::term_db(&t.source_match);
                        let r = calstd::term_db(&t.reflection_tracking);
                        let n = d.len().min(xs.len());
                        paint_cartesian(
                            &painter,
                            rect,
                            View::S21,
                            &xs[..n],
                            &[(&d[..n], TRACE1), (&m[..n], MEM_COLOR), (&r[..n], palette::PLUM)],
                            &[],
                            theme.data_size,
                            None,
                            &Overlay::default(),
                        );
                    }
                } else if self.cal_graph_view == View::Smith {
                    paint_smith(&painter, rect, false, Some(&pts), compare.as_deref(), set.z0, None, theme.data_size);
                    let disk = smith_disk(rect);
                    let at = disk.center + vec2(kind.ideal().re as f32 * disk.rad, -(kind.ideal().im as f32) * disk.rad);
                    painter.circle_stroke(at, 7.0, egui::Stroke::new(2.0, palette::TEXT));
                    painter.text(at + vec2(0.0, -12.0), egui::Align2::CENTER_BOTTOM, "ideal", egui::FontId::monospace(theme.data_size * 0.8), palette::TEXT);
                    if let Some(w) = worst.and_then(|w| pts.get(w.index)) {
                        let p = disk.center + vec2(w.s11.re as f32 * disk.rad, -(w.s11.im as f32) * disk.rad);
                        paint_marker(&painter, p, markers::REF, theme.data_size);
                    }
                    for (slot, i) in &placed {
                        if let Some(s) = pts.get(*i) {
                            paint_marker(&painter, disk.center + vec2(s.s11.re as f32 * disk.rad, -(s.s11.im as f32) * disk.rad), *slot, theme.data_size);
                        }
                    }
                } else if self.cal_graph_view == View::PolarS21 {
                    let plot = rect.shrink(16.0);
                    paint_polar(&painter, plot, theme.data_size);
                    stroke_polar(&painter, plot, &ideal_pts, IDEAL_COLOR);
                    if let Some(c) = &compare {
                        stroke_polar(&painter, plot, c, MEM_COLOR);
                    }
                    stroke_polar(&painter, plot, &pts, TRACE1);
                    let (c, r) = (plot.center(), plot.width().min(plot.height()) * 0.45);
                    for (slot, i) in &placed {
                        if let Some(s) = pts.get(*i) {
                            let m = s.s11.re.hypot(s.s11.im);
                            let g = if m > 1.0 { Cx::new(s.s11.re / m, s.s11.im / m) } else { s.s11 };
                            paint_marker(&painter, c + vec2(g.re as f32 * r, -(g.im as f32) * r), *slot, theme.data_size);
                        }
                    }
                } else {
                    let view = self.cal_graph_view;
                    let ys = series(view, &pts, set.z0, 0.66).1;
                    let ideal_ys = series(view, &ideal_pts, set.z0, 0.66).1;
                    let cmp_ys = compare.as_ref().map(|c| series(view, c, set.z0, 0.66).1);
                    let mut traces: Vec<(&[f64], Color32)> = vec![(&ys, TRACE1), (&ideal_ys, IDEAL_COLOR)];
                    if let Some(c) = &cmp_ys {
                        let n = c.len().min(xs.len());
                        traces.push((&c[..n], MEM_COLOR));
                    }
                    let mut marks: Vec<(usize, usize)> = worst.map(|w| vec![(markers::REF, w.index)]).unwrap_or_default();
                    marks.extend(placed.iter().copied());
                    paint_cartesian(&painter, rect, view, &xs, &traces, &marks, theme.data_size, None, &Overlay::default());
                }
            });
        if let Some(png) = export {
            let name = format!("cal-{}-{}", if set.zone.is_empty() { "set" } else { set.zone.as_str() }, step.step_name().to_lowercase());
            let cmp: Option<Vec<Cx>> = self.cal_compare.and_then(|i| self.face.cals.get(i)).map(|c| pairs(step_samples(c, step)));
            if png {
                let xs: Vec<f64> = pts.iter().map(|s| s.hz).collect();
                let view = if is_disk(self.cal_graph_view) { if s21 { View::S21 } else { View::ReturnLoss } } else { self.cal_graph_view };
                let ys = series(view, &pts, set.z0, 0.66).1;
                let iy = series(view, &ideal_pts, set.z0, 0.66).1;
                match rf::chart_png_traces(&format!("{} {}", step.step_name(), view_name(view)), &xs, &[&ys, &iy]) {
                    Ok(bytes) => self.save_bytes("Save chart", &format!("{name}.png"), &bytes),
                    Err(err) => self.note = err,
                }
            } else {
                let text = calstd::csv(set.start_hz, set.step_hz, &values, cmp.as_deref());
                self.save_bytes("Save CSV", &format!("{name}.csv"), text.as_bytes());
            }
        }
        if !open {
            self.cal_graph = None;
        }
    }

    fn zone_cal(&self, id: Option<&str>, zone: &str) -> Option<&CalSet> {
        pick_cal(&self.face.cals, id, |set| set.zone == zone && set.band.is_empty())
    }

    fn wizard_dialog(&mut self, ui: &mut Ui, theme: &LcarsTheme, live: &device::Live) {
        let Some(wiz) = self.wizard.clone() else {
            return;
        };
        let mut open = true;
        let note = self.note.clone();
        egui::Window::new("Calibrate NanoVNA")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(720.0)
            .max_width(720.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                // The words change every step. Their boxes do not, so the
                // buttons stay on the same row for the whole pass.
                ui.set_min_width(680.0);
                ui.set_max_width(680.0);
                let busy = live.busy || self.pending.is_some();
                let mut heading = wiz.step.heading(wiz.spans.len());
                if busy {
                    heading.push_str(" Measuring. Leave the ports as they are.");
                } else if !note.is_empty() {
                    heading.push(' ');
                    heading.push_str(&note);
                }
                fixed_readout(ui, theme, &heading, 3, true);
                ui.horizontal(|ui| {
                    let spare = (ui.available_width() - 460.0).max(0.0);
                    ui.add_space(spare * 0.5);
                    paint_wiring(ui, wiz.step);
                });
                fixed_readout(ui, theme, wiz.step.instruction(), 3, true);
                let names = wiz
                    .spans
                    .iter()
                    .map(|span| span.zone.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                fixed_readout(
                    ui,
                    theme,
                    &format!("One fitting, then these zones: {names}."),
                    2,
                    false,
                );
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let scan_zone = self
                        .plan
                        .zones
                        .get(self.zone_sel)
                        .map(|zone| zone.name.as_str())
                        .unwrap_or("zone");
                    let label = wiz.step.action(scan_zone);
                    let size = vec2(220.0, 32.0);
                    if busy {
                        // The click that started this step is finished. The same
                        // control stays dark and ignores clicks until the next
                        // step draws a new one.
                        let _ = held_button(ui, theme, &label, size);
                    } else if action_button(ui, theme, &label, size).clicked() {
                        self.wizard_measure();
                    }
                    let skip = match wiz.step {
                        WizStep::Thru => Some("Skip cable"),
                        WizStep::Isol => Some("Skip loads"),
                        _ => None,
                    };
                    if let Some(skip) = skip {
                        if action_button(ui, theme, skip, vec2(130.0, 32.0)).clicked() {
                            self.wizard_skip();
                        }
                    } else {
                        ui.allocate_exact_size(vec2(130.0, 32.0), Sense::hover());
                    }
                    if action_button(ui, theme, "Cancel", vec2(88.0, 32.0)).clicked() {
                        self.wizard = None;
                        self.pending = None;
                        self.note =
                            "Calibration cancelled. Nothing new was kept from this step.".into();
                    }
                });
                ui.add_space(8.0);
            });
        if !open {
            self.wizard = None;
            self.pending = None;
            self.note = "Calibration cancelled. Nothing new was kept from this step.".into();
        }
    }

    fn backup_cal(&mut self) {
        let text = match encode_backup(&self.face.cals) {
            Ok(text) => text,
            Err(err) => {
                self.note = err;
                return;
            }
        };
        let Some(path) = pick_named("Back up calibration", "nanovna-cal.json") else {
            return;
        };
        self.note = match fs::write(&path, text) {
            Ok(()) => format!(
                "Wrote {}. These sets stay in the VNA file.",
                path.display()
            ),
            Err(err) => format!("{}: {err}", path.display()),
        };
    }

    fn restore_cal(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Restore calibration")
            .pick_file()
        else {
            return;
        };
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                self.note = format!("{}: {err}", path.display());
                return;
            }
        };
        match decode_backup(&text) {
            Ok(cals) => {
                let count = cals.len();
                self.face.cals = cals;
                self.archive_dirty = true;
                self.note = format!("Restored {count} calibration sets into the VNA file.");
            }
            Err(err) => self.note = err,
        }
    }

    fn clear_cal(&mut self, live: &device::Live) {
        let Some(id) = live.device_id else {
            self.face.cals.clear();
            self.archive_dirty = true;
            return;
        };
        let id = proto::device_id_hex(&id);
        self.face.cals.retain(|set| set.id != id);
        self.archive_dirty = true;
        self.note = "Calibration for this device was cleared.".into();
    }

    fn send_sweep(&mut self) -> bool {
        match self.parse_setup() {
            Ok((start, stop, points)) => {
                let step = ((stop - start) as f64 / f64::from(points - 1))
                    .round()
                    .max(1.0) as u64;
                let same = (self.face.start_hz - start as f64).abs() < 1.0
                    && (self.face.stop_hz - stop as f64).abs() < 1.0
                    && self.face.points == points;
                self.face.start_hz = start as f64;
                self.face.stop_hz = stop as f64;
                self.face.points = points;
                if !same {
                    self.dirty = true;
                }
                self.session.sweep(start, step, points);
                self.note.clear();
                true
            }
            Err(err) => {
                // Numbers that are only out of range are clamped with a note,
                // like the SWEEP panel. Text that is not a number still errors. Issue 68.
                if let (Ok(a), Ok(b), Ok(n)) = (parse_hz(&self.start), parse_hz(&self.stop), self.points.trim().parse::<i64>()) {
                    let max = self.session.snapshot().identity.map(|id| id.max_hz()).unwrap_or(3.0e9);
                    let c = sweep::clamp(a as f64, b as f64, n, proto::FREQ_FLOOR_HZ as f64, max, proto::POINTS_MAX as u16);
                    if !c.notes.is_empty() {
                        self.start = hz_field(c.start);
                        self.stop = hz_field(c.stop);
                        self.points = c.points.to_string();
                        if self.parse_setup().is_ok() {
                            let ok = self.send_sweep();
                            self.note = format!("Clamped: {}.", c.notes.join("; "));
                            return ok;
                        }
                    }
                }
                self.note = err;
                false
            }
        }
    }

    fn parse_setup(&self) -> Result<(u64, u64, u16), String> {
        let start = parse_hz(&self.start)?;
        let stop = parse_hz(&self.stop)?;
        let points: u16 = self
            .points
            .trim()
            .parse()
            .map_err(|_| "points is not a whole number".to_string())?;
        if start < proto::FREQ_FLOOR_HZ {
            return Err("start is below 50 kHz".into());
        }
        if stop <= start {
            return Err("stop must be above start".into());
        }
        let max = self
            .session
            .snapshot()
            .identity
            .map(|id| id.max_hz())
            .unwrap_or(3.0e9);
        if stop as f64 > max {
            return Err(format!("stop is above {} Hz for this board", max));
        }
        if points < 2 || points as usize > proto::POINTS_MAX {
            return Err("points must be from 2 to 1024".into());
        }
        Ok((start, stop, points))
    }

    fn commit_fields(&mut self) {
        let before = self.face.clone();
        if let Ok(z0) = self.z0.trim().parse::<f64>() {
            if z0 > 0.0 {
                self.face.z0 = z0;
            }
        }
        if let Ok(vf) = self.velocity.trim().parse::<f64>() {
            if vf > 0.0 && vf <= 1.0 {
                self.face.velocity = vf;
            }
        }
        if let Ok(ext) = self.extension.trim().parse::<f64>() {
            if ext.is_finite() {
                self.face.extension_m = ext;
            }
        }
        if let Ok(avg) = self.host_avg.trim().parse::<u32>() {
            if avg >= 1 {
                self.face.host_average = avg;
            }
        }
        if let Ok(smooth) = self.smooth.trim().parse::<u32>() {
            if smooth >= 1 {
                self.face.smooth = smooth;
            }
        }
        if self.face != before {
            self.dirty = true;
        }
    }

    fn display_trace(&self, live: &device::Live) -> Option<Vec<Sample>> {
        let (raw, stored_id) = self.trace_source(live)?;
        let n = raw[0].len();
        if raw.iter().any(|row| row.len() != n) {
            return None;
        }
        let s11 = rf::mean_complex(
            &raw.iter()
                .map(|row| row.iter().map(|s| s.s11).collect())
                .collect::<Vec<_>>(),
        );
        let s21 = rf::mean_complex(
            &raw.iter()
                .map(|row| row.iter().map(|s| s.s21).collect())
                .collect::<Vec<_>>(),
        );
        let mut samples = Vec::with_capacity(n);
        for index in 0..n {
            samples.push(Sample {
                hz: raw[0][index].hz,
                s11: s11[index],
                s21: s21[index],
            });
        }
        let (found, state) = self.cal_for(&samples, stored_id.as_deref());
        if found.is_none() && state != cal::CalState::Off && self.wizard.is_none() {
            return None;
        }
        if let Some(set) = found.as_ref() {
            if let Ok(terms) = rf::solve_osl(&pairs(&set.open), &pairs(&set.short), &pairs(&set.load))
            {
                let n = samples.len();
                let thru_len = set.thru.len();
                let iso_len = set.isolation.len();
                for (index, sample) in samples.iter_mut().enumerate() {
                    if let Some(s11) = rf::apply_osl(sample.s11, &terms, index) {
                        sample.s11 = s11;
                    }
                    if thru_len == n {
                        let iso = if iso_len == n {
                            Some(pair(set.isolation[index]))
                        } else {
                            None
                        };
                        if let Some(s21) = rf::apply_thru(sample.s21, pair(set.thru[index]), iso) {
                            sample.s21 = s21;
                        }
                    }
                }
            }
        }
        let metres = self.face.extension_m;
        let vf = self.velocity_value();
        if metres != 0.0 {
            for sample in &mut samples {
                *sample = rf::port_extend(*sample, metres, vf);
            }
        }
        let width = self.face.smooth;
        if width > 1 {
            samples = rf::smooth_samples(&samples, width);
        }
        Some(samples)
    }

    fn trace_source(&self, live: &device::Live) -> Option<(Vec<Vec<Sample>>, Option<String>)> {
        if !self.history.is_empty() {
            return Some((self.history.iter().cloned().collect(), None));
        }
        if let Some(shown) = &self.shown_scan {
            return Some((vec![shown.clone()], Some(self.shown_id.clone())));
        }
        if self.blank_until_sweep || live.sweep.is_empty() {
            return None;
        }
        Some((vec![live.sweep.clone()], None))
    }

    /// The calibration for a sweep: exact grid, else interpolated from a
    /// covering set, else none. CAL OFF wins over both. Issue 65.
    fn cal_for(&self, samples: &[Sample], stored_id: Option<&str>) -> (Option<CalSet>, cal::CalState) {
        if self.cal_off {
            return (None, cal::CalState::Off);
        }
        if let Some(set) = self.matched_cal_for(samples, stored_id) {
            return (Some(set.clone()), cal::CalState::Exact);
        }
        if samples.len() >= 2 {
            if let Some(id) = self.cal_device_id(stored_id) {
                let hz: Vec<f64> = samples.iter().map(|s| s.hz).collect();
                // Narrowest covering set first: it has the finest grid.
                let covering = |set: &&CalSet| cal_complete(set) && cal::covers(set, hz[0], hz[hz.len() - 1]);
                let mut sets: Vec<&CalSet> = self.face.cals.iter().filter(|set| set.id == id).filter(covering).collect();
                if sets.is_empty() {
                    sets = self.face.cals.iter().filter(covering).collect();
                }
                sets.sort_by(|a, b| a.step_hz.partial_cmp(&b.step_hz).unwrap_or(std::cmp::Ordering::Equal));
                if let Some(set) = sets.first().and_then(|set| cal::interpolate(set, &hz)) {
                    return (Some(set), cal::CalState::Interp);
                }
            }
        }
        (None, cal::CalState::Uncal)
    }

    fn cal_device_id(&self, stored_id: Option<&str>) -> Option<String> {
        let live_id = self
            .session
            .snapshot()
            .device_id
            .map(|raw| proto::device_id_hex(&raw));
        let prefer_stored = self.shown_scan.is_some() && self.history.is_empty();
        if prefer_stored {
            stored_id.map(str::to_string).or(live_id)
        } else {
            live_id.or_else(|| stored_id.map(str::to_string))
        }
    }

    fn matched_cal(&self, samples: &[Sample]) -> Option<&CalSet> {
        self.matched_cal_for(samples, None)
    }

    fn matched_cal_for(&self, samples: &[Sample], stored_id: Option<&str>) -> Option<&CalSet> {
        let live_id = self
            .session
            .snapshot()
            .device_id
            .map(|raw| proto::device_id_hex(&raw));
        // A recalled scan keeps the calibration of the device that measured it.
        let prefer_stored = self.shown_scan.is_some() && self.history.is_empty();
        let id = if prefer_stored {
            stored_id.map(str::to_string).or(live_id)
        } else {
            live_id.or_else(|| stored_id.map(str::to_string))
        }?;
        if samples.len() < 2 {
            return None;
        }
        let start = samples[0].hz;
        let step = samples[1].hz - samples[0].hz;
        pick_cal(&self.face.cals, Some(&id), |set| {
            set.open.len() == samples.len()
                && (set.start_hz - start).abs() < 1.0
                && (set.step_hz - step).abs() < 1.0
                && set.short.len() == samples.len()
                && set.load.len() == samples.len()
        })
    }

    fn cal_label(&self, live: &device::Live) -> String {
        let Some(samples) = live.sweep.first().map(|_| &live.sweep) else {
            return "No sweep yet. The trace is empty until a device answers.".into();
        };
        let Some(id) = live.device_id else {
            return "Uncalibrated. Connect a NanoVNA V2 to store open, short, and load.".into();
        };
        let id = proto::device_id_hex(&id);
        let any = self.face.cals.iter().any(|set| set.id == id);
        if self.cal_off {
            "Correction is off (CAL OFF). The trace is raw.".into()
        } else if self.matched_cal(samples).is_some() {
            "Calibrated for this device and this sweep.".into()
        } else if self.cal_for(samples, None).1 == cal::CalState::Interp {
            "CAL INT: the sweep grid differs, so a covering calibration is interpolated.".into()
        } else if any {
            "Outside the calibrated range. The trace is uncalibrated.".into()
        } else {
            "Uncalibrated. Open, short, and load have not been stored for this sweep.".into()
        }
    }

    fn export_text(&mut self, live: &device::Live, ext: &str, name: &str, csv: bool) {
        let Some(samples) = self.display_trace(live) else {
            self.note = "There is no sweep to export.".into();
            return;
        };
        let z0 = self.z0_value();
        let cal = self.matched_cal(&samples).is_some();
        let text = if csv {
            rf::csv_table(&samples, z0, cal)
        } else if ext == "s1p" {
            rf::touchstone_s1p(&samples, z0)
        } else {
            rf::touchstone_s2p(&samples, z0)
        };
        if let Some(path) = pick_save(name) {
            self.note = match fs::write(&path, text) {
                Ok(()) => format!("Wrote {}", path.display()),
                Err(err) => err.to_string(),
            };
        }
    }

    fn export_png(&mut self, live: &device::Live) {
        let Some(samples) = self.display_trace(live) else {
            self.note = "There is no sweep to export.".into();
            return;
        };
        let z0 = self.z0_value();
        let (xs, ys) = series(self.view, &samples, z0, self.velocity_value());
        let react: Vec<f64> = if self.view == View::Impedance {
            samples.iter().map(|sample| rf::derive(sample, z0).x).collect()
        } else {
            Vec::new()
        };
        let traces: Vec<&[f64]> = if react.is_empty() {
            vec![&ys]
        } else {
            vec![&ys, &react]
        };
        match rf::chart_png_traces(view_name(self.view), &xs, &traces) {
            Ok(bytes) => self.save_bytes("Save chart", "nanovna.png", &bytes),
            Err(err) => self.note = err,
        }
    }

    fn save_bytes(&mut self, title: &str, name: &str, bytes: &[u8]) {
        if let Some(path) = pick_save(name).map(|path| {
            let _ = title;
            path
        }) {
            self.note = match fs::write(&path, bytes) {
                Ok(()) => format!("Wrote {}", path.display()),
                Err(err) => err.to_string(),
            };
        }
    }

    /// Band edges plus this format's limit lines and verdict. Issue 71.
    fn overlay(&self, samples: Option<&[Sample]>) -> Overlay {
        let mut o = Overlay { edges: self.band_edges(), ..Default::default() };
        if is_disk(self.view) || self.view == View::Tdr {
            return o;
        }
        if let Some(samples) = samples.filter(|_| uses_s21(self.view) && (self.filter_on || self.notch_on)) {
            let xs: Vec<f64> = samples.iter().map(|s| s.hz).collect();
            let db: Vec<f64> = samples.iter().map(|s| 20.0 * s.s21.norm().max(1e-12).log10()).collect();
            o.analysis = analysis_marks(&xs, &db, self.filter_on, self.notch_on);
        }
        if let Some(set) = self.face.limits.get(self.key().as_str()).filter(|s| s.on && !s.segs.is_empty()) {
            o.limits = set.segs.clone();
            if let Some(samples) = samples {
                let (xs, ys) = series(self.view, samples, self.z0_value(), self.velocity_value());
                o.verdict = Some(limits::check(&xs, &ys, &set.segs));
            }
        }
        o
    }

    /// A ham band preset limit for the shown format: the band holding the
    /// sweep centre, SWR `swr`:1 on the SWR format. Issue 71.
    fn limit_preset(&mut self, swr: f64) {
        let center = (self.face.start_hz + self.face.stop_hz) / 2.0;
        let start = parse_hz(&self.start).map(|v| v as f64).unwrap_or(self.face.start_hz);
        let stop = parse_hz(&self.stop).map(|v| v as f64).unwrap_or(self.face.stop_hz);
        let mid = if stop > start { (start + stop) / 2.0 } else { center };
        let Some(band) = crate::tools::ham_rf::bands::find_band(mid / 1e6) else {
            self.note = "The sweep centre is not in a ham band, so there is no band preset.".into();
            return;
        };
        self.view = View::Swr;
        let k = self.key();
        let set = self.face.limits.entry(k).or_default();
        set.segs = vec![limits::swr_preset(band.lo_mhz * 1e6, band.hi_mhz * 1e6, swr)];
        set.on = true;
        self.dirty = true;
        self.note = format!("Limit: SWR {swr}:1 across {}.", band.name);
    }

    /// The limit line editor for the shown format. Issue 71.
    fn limits_dialog(&mut self, ui: &mut Ui, theme: &LcarsTheme) {
        if !self.limits_edit {
            return;
        }
        let key = self.key();
        let (_, _, unit) = view_meta(self.view);
        let mut open = true;
        let mut set = self.face.limits.get(&key).cloned().unwrap_or_default();
        let before = set.clone();
        egui::Window::new(format!("Limit lines · {key}"))
            .open(&mut open)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                if crate::toggle::add_toggle(ui, true, set.on, "Limit test on").clicked() {
                    set.on = !set.on;
                }
                let mut remove = None;
                egui::Grid::new("vna-limits").striped(true).show(ui, |ui| {
                    for h in ["START MHz", "STOP MHz", &format!("START {unit}"), &format!("STOP {unit}"), "TYPE", ""] {
                        ui.label(egui::RichText::new(h).color(palette::TEXT_MUTED));
                    }
                    ui.end_row();
                    for (i, seg) in set.segs.iter_mut().enumerate() {
                        let mut a = seg.start_hz / 1e6;
                        let mut b = seg.stop_hz / 1e6;
                        if ui.add(egui::DragValue::new(&mut a).speed(0.01).max_decimals(6)).changed() {
                            seg.start_hz = a * 1e6;
                        }
                        if ui.add(egui::DragValue::new(&mut b).speed(0.01).max_decimals(6)).changed() {
                            seg.stop_hz = b * 1e6;
                        }
                        ui.add(egui::DragValue::new(&mut seg.start_v).speed(0.05));
                        ui.add(egui::DragValue::new(&mut seg.stop_v).speed(0.05));
                        let label = if seg.upper { "UPPER" } else { "LOWER" };
                        if ui.button(label).clicked() {
                            seg.upper = !seg.upper;
                        }
                        if ui.button("Delete").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
                if let Some(i) = remove {
                    set.segs.remove(i);
                }
                ui.horizontal(|ui| {
                    if action_button(ui, theme, "Add segment", vec2(140.0, 30.0)).clicked() {
                        let v = self.shown_scale(None).map(|s| s.1).unwrap_or(0.0);
                        let v = if self.view == View::Swr { 2.0 } else { v };
                        set.segs.push(limits::Segment {
                            start_hz: self.face.start_hz,
                            stop_hz: self.face.stop_hz,
                            start_v: v,
                            stop_v: v,
                            upper: true,
                        });
                    }
                    if action_button(ui, theme, "Clear", vec2(90.0, 30.0)).clicked() {
                        set.segs.clear();
                    }
                });
            });
        if set != before {
            if set == limits::LimitSet::default() {
                self.face.limits.remove(&key);
            } else {
                self.face.limits.insert(key, set);
            }
            self.dirty = true;
        }
        self.limits_edit = open;
    }

    /// Every zone edge of the station band plan, in Hz. Issue 53.
    fn band_edges(&self) -> Vec<f64> {
        let mut out: Vec<f64> = self
            .plan
            .zones
            .iter()
            .filter_map(zone_limits)
            .flat_map(|(lo, hi)| [lo, hi])
            .collect();
        out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        out.dedup_by(|a, b| (*a - *b).abs() < 1.0);
        out
    }

    /// The selected memory trace. Issue 70.
    fn memory(&self) -> Option<&Vec<Sample>> {
        self.mems[self.mem_sel].as_ref()
    }

    /// Load a Touchstone (.s1p/.s2p) or CSV sweep into the selected memory. Issues 70 and 53.
    fn import_memory(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import a sweep into memory")
            .add_filter("Touchstone or CSV", &["s1p", "s2p", "csv", "S1P", "S2P", "CSV"])
            .pick_file()
        else {
            return;
        };
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(err) => {
                self.note = err.to_string();
                return;
            }
        };
        let z0 = self.z0_value();
        let csv = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv"));
        let parsed = if csv { traces::parse_csv(&text, z0) } else { traces::parse_touchstone(&text) };
        match parsed {
            Ok(mut imp) => {
                traces::renormalize(&mut imp.samples, imp.z0, z0);
                let n = imp.samples.len();
                self.mems[self.mem_sel] = Some(imp.samples);
                self.show_memory = true;
                self.note = format!(
                    "MEM{} ← {} ({n} points{}).",
                    self.mem_sel + 1,
                    path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
                    if imp.has_s21 { ", S11 and S21" } else { ", S11 only" }
                );
            }
            Err(err) => self.note = format!("Import failed: {err}"),
        }
    }

    fn z0_value(&self) -> f64 {
        self.face.z0
    }

    fn velocity_value(&self) -> f64 {
        self.face.velocity
    }

    fn host_average_value(&self) -> usize {
        self.face.host_average.max(1) as usize
    }
}

fn same_grid(set: &CalSet, start: f64, step: f64, n: usize) -> bool {
    (set.start_hz - start).abs() < 1.0
        && (set.step_hz - step).abs() < 1.0
        && [
            set.open.len(),
            set.short.len(),
            set.load.len(),
            set.thru.len(),
            set.isolation.len(),
        ]
        .into_iter()
        .filter(|len| *len > 0)
        .all(|len| len == n)
}

fn cal_complete(set: &CalSet) -> bool {
    let n = set.open.len();
    n >= 2 && set.short.len() == n && set.load.len() == n
}

fn zone_limits(zone: &crate::plan::Zone) -> Option<(f64, f64)> {
    let mut lo = f64::MAX;
    let mut hi = 0.0_f64;
    for band in &zone.bands {
        if band.lo_hz.is_finite() && band.hi_hz.is_finite() && band.hi_hz > band.lo_hz {
            lo = lo.min(band.lo_hz);
            hi = hi.max(band.hi_hz);
        }
    }
    if lo.is_finite() && hi > lo {
        Some((lo, hi))
    } else {
        None
    }
}

fn step_samples(set: &CalSet, step: WizStep) -> &[[f64; 2]] {
    match step {
        WizStep::Open => &set.open,
        WizStep::Short => &set.short,
        WizStep::Load => &set.load,
        WizStep::Thru => &set.thru,
        WizStep::Isol => &set.isolation,
        WizStep::Finish => &[],
    }
}

fn step_summary(set: Option<&CalSet>, step: WizStep) -> String {
    let Some(set) = set else {
        return "not stored".into();
    };
    let samples = step_samples(set, step);
    if samples.len() < 2 {
        return "not stored".into();
    }
    let stop = set.start_hz + set.step_hz * (samples.len() - 1) as f64;
    let first = samples[0];
    format!(
        "{} pts  {:.3}-{:.3} MHz  first {:+.3}{:+.3}j",
        samples.len(),
        set.start_hz / 1_000_000.0,
        stop / 1_000_000.0,
        first[0],
        first[1]
    )
}

fn edges_match(set: &CalSet, lo: f64, hi: f64) -> bool {
    let n = set.open.len().max(2) as f64;
    let stop = set.start_hz + set.step_hz * (n - 1.0);
    let slop = set.step_hz.max(1.0);
    (set.start_hz - lo).abs() <= slop && (stop - hi).abs() <= slop
}

/// The NanoVNA V2.2 does not report a stable device id: register 0xd0 reads
/// differently from one connection to the next (often all zeros). So a set is
/// chosen for this device's id first, and otherwise the newest matching set
/// from any id. The caller shows the second case (OTHER ID). Issue 65.
fn pick_cal<'a>(cals: &'a [CalSet], id: Option<&str>, ok: impl Fn(&CalSet) -> bool) -> Option<&'a CalSet> {
    if let Some(id) = id {
        if let Some(set) = cals.iter().rev().find(|set| set.id == id && ok(set)) {
            return Some(set);
        }
    }
    // Newest saved first; sets without a date (older Commander) last, in file order.
    cals.iter()
        .enumerate()
        .filter(|(_, set)| ok(set))
        .max_by(|(i, a), (j, b)| a.saved_at.cmp(&b.saved_at).then(i.cmp(j)))
        .map(|(_, set)| set)
}

/// True when a set was stored under a different, real device id than `live`.
fn other_device(set: &CalSet, live: Option<&str>) -> bool {
    let real = |id: &str| !id.is_empty() && id.bytes().any(|b| b != b'0');
    live.is_some_and(|live| set.id != live && real(&set.id) && real(live))
}

/// Restored = saved before this run started (or saved by a Commander without dates).
fn restored(set: &CalSet, run_started: &str) -> bool {
    set.saved_at.as_deref().is_none_or(|at| at < run_started)
}

fn find_zone_cal<'a>(cals: &'a [CalSet], id: &str, zone: &str, lo: f64, hi: f64) -> Option<&'a CalSet> {
    pick_cal(cals, Some(id), |set| {
        set.zone == zone
            && set.band.is_empty()
            && cal_complete(set)
            && edges_match(set, lo, hi)
    })
}

fn fill_standard(set: &mut CalSet, standard: Standard, raw: &[Sample]) {
    let s11: Vec<[f64; 2]> = raw.iter().map(|s| [s.s11.re, s.s11.im]).collect();
    let s21: Vec<[f64; 2]> = raw.iter().map(|s| [s.s21.re, s.s21.im]).collect();
    match standard {
        Standard::Open => set.open = s11,
        Standard::Short => set.short = s11,
        Standard::Load => set.load = s11,
        Standard::Thru => set.thru = s21,
        Standard::Isol => set.isolation = s21,
    }
}

fn pairs(rows: &[[f64; 2]]) -> Vec<Cx> {
    rows.iter().copied().map(pair).collect()
}

fn pair(row: [f64; 2]) -> Cx {
    Cx::new(row[0], row[1])
}

fn cable_loss(samples: &[Sample]) -> f64 {
    let mut sum = 0.0;
    let mut n = 0.0;
    for sample in samples {
        let loss = -rf::db(sample.s21);
        if loss.is_finite() {
            sum += loss;
            n += 1.0;
        }
    }
    if n == 0.0 { f64::NAN } else { sum / n }
}

fn parse_hz(text: &str) -> Result<u64, String> {
    let value: f64 = text
        .trim()
        .parse()
        .map_err(|_| format!("{} is not a frequency", text.trim()))?;
    if !value.is_finite() || value < 1.0 {
        return Err("frequency must be at least 1 Hz".into());
    }
    Ok(value.round() as u64)
}

fn hz_field(hz: f64) -> String {
    format!("{}", hz.round() as u64)
}

fn trim_num(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn reactance_part(henries: f64, farads: f64) -> String {
    if henries.is_finite() {
        format!("{} H", num(henries))
    } else if farads.is_finite() {
        format!("{} F", num(farads))
    } else {
        "INF".into()
    }
}

fn num(value: f64) -> String {
    if !value.is_finite() {
        "INF".into()
    } else if value.abs() >= 100.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.3}")
    }
}

fn samples_of(scan: &StoredScan) -> Vec<Sample> {
    scan.s11
        .iter()
        .zip(scan.s21.iter())
        .enumerate()
        .map(|(index, (s11, s21))| Sample {
            hz: scan.start_hz + scan.step_hz * index as f64,
            s11: Cx::new(s11[0], s11[1]),
            s21: Cx::new(s21[0], s21[1]),
        })
        .collect()
}

fn view_name(view: View) -> &'static str {
    VIEWS
        .iter()
        .find(|(id, _)| *id == view)
        .map(|(_, name)| *name)
        .unwrap_or("sweep")
}

fn pick_save(name: &str) -> Option<PathBuf> {
    pick_named("Save NanoVNA trace", name)
}

fn pick_named(title: &str, name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title(title)
        .set_file_name(name)
        .save_file()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CalBackup {
    version: u32,
    cals: Vec<CalSet>,
}

fn encode_backup(cals: &[CalSet]) -> Result<String, String> {
    if cals.is_empty() {
        return Err("There is no calibration to back up.".into());
    }
    let mut face = Face::default();
    face.cals = cals.to_vec();
    validate(&face)?;
    let file = CalBackup {
        version: 1,
        cals: cals.to_vec(),
    };
    serde_json::to_string_pretty(&file)
        .map(|text| text + "\n")
        .map_err(|err| err.to_string())
}

fn decode_backup(text: &str) -> Result<Vec<CalSet>, String> {
    let file: CalBackup = serde_json::from_str(text).map_err(|err| err.to_string())?;
    if file.version != 1 {
        return Err("This calibration backup is not version 1.".into());
    }
    let mut face = Face::default();
    face.cals = file.cals.clone();
    validate(&face)?;
    Ok(file.cals)
}

fn series(view: View, samples: &[Sample], z0: f64, velocity: f64) -> (Vec<f64>, Vec<f64>) {
    match view {
        View::Swr => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| rf::swr(s.s11)).collect(),
        ),
        View::ReturnLoss => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| rf::return_loss_db(s.s11)).collect(),
        ),
        View::Impedance => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| rf::derive(s, z0).r).collect(),
        ),
        View::S21 => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| rf::db(s.s21)).collect(),
        ),
        View::Phase | View::PhaseS11 => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| param_of(view, s).arg().to_degrees()).collect(),
        ),
        View::Zabs => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| rf::derive(s, z0).z_abs).collect(),
        ),
        View::Unwrap | View::UnwrapS11 => {
            let phase = rf::unwrap_phase(&samples.iter().map(|s| param_of(view, s)).collect::<Vec<_>>());
            (
                samples.iter().map(|s| s.hz).collect(),
                phase.into_iter().map(|rad| rad.to_degrees()).collect(),
            )
        }
        View::GroupDelay => {
            let phase = rf::unwrap_phase(&samples.iter().map(|s| s.s21).collect::<Vec<_>>());
            let hz: Vec<f64> = samples.iter().map(|s| s.hz).collect();
            let delay = rf::group_delay_aperture(&hz, &phase, GD_APERTURE.with(|a| a.get()));
            (hz, delay.into_iter().map(|s| s * 1e9).collect())
        }
        View::Tdr => {
            if let Ok((_limit, trace)) = rf::distance_to_fault(samples, velocity) {
                (
                    trace.iter().map(|p| p.metres).collect(),
                    trace.iter().map(|p| p.magnitude).collect(),
                )
            } else {
                (Vec::new(), Vec::new())
            }
        }
        View::LinMag | View::LinMagS21 => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| param_of(view, s).abs()).collect(),
        ),
        View::ReIm | View::ReImS21 => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| param_of(view, s).re).collect(),
        ),
        View::Smith | View::SmithY | View::Polar | View::PolarS21 => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| param_of(view, s).abs()).collect(),
        ),
    }
}

/// The second line some formats draw: X beside R, imaginary beside real.
fn second_series(view: View, samples: &[Sample], z0: f64) -> Vec<f64> {
    match view {
        View::Impedance => samples.iter().map(|s| rf::derive(s, z0).x).collect(),
        View::ReIm | View::ReImS21 => samples.iter().map(|s| param_of(view, s).im).collect(),
        _ => Vec::new(),
    }
}

/// Measured parameter, format name, and unit for the trace header. Issue 63.
fn view_meta(view: View) -> (&'static str, &'static str, &'static str) {
    match view {
        View::Smith => ("S11", "SMITH", "U"),
        View::SmithY => ("S11", "SMITH Y", "U"),
        View::Polar => ("S11", "POLAR", "U"),
        View::Swr => ("S11", "SWR", "U"),
        View::ReturnLoss => ("S11", "RETURN LOSS", "dB"),
        View::LinMag => ("S11", "LIN MAG", "U"),
        View::ReIm => ("S11", "REAL + IMAG", "U"),
        View::Impedance => ("S11", "R + jX", "Ω"),
        View::S21 => ("S21", "DB MAG", "dB"),
        View::Phase => ("S21", "PHASE", "°"),
        View::Unwrap => ("S21", "UNWR PHASE", "°"),
        View::GroupDelay => ("S21", "DELAY", "ns"),
        View::Tdr => ("S11", "TDR", "U"),
        View::Zabs => ("S11", "|Z|", "Ω"),
        View::PhaseS11 => ("S11", "PHASE", "°"),
        View::UnwrapS11 => ("S11", "UNWR PHASE", "°"),
        View::PolarS21 => ("S21", "POLAR", "U"),
        View::ReImS21 => ("S21", "REAL + IMAG", "U"),
        View::LinMagS21 => ("S21", "LIN MAG", "U"),
    }
}

/// Primary marker value in the trace's own format.
fn marker_value(view: View, sample: &Sample, z0: f64) -> String {
    let (_, _, unit) = view_meta(view);
    match view {
        View::Smith | View::SmithY | View::Impedance => {
            let d = rf::derive(sample, z0);
            format!("{} {:+}j Ω", num(d.r), num(d.x))
        }
        View::Polar | View::PolarS21 => format!(
            "{} ∠ {}°",
            fmt_g(param_of(view, sample).abs()),
            fmt_g(param_of(view, sample).arg().to_degrees())
        ),
        View::Zabs => format!("{} Ω", num(rf::derive(sample, z0).z_abs)),
        View::PhaseS11 => format!("{} °", num(sample.s11.arg().to_degrees())),
        View::LinMagS21 => fmt_g(sample.s21.abs()),
        View::ReImS21 => format!("{} {:+}j", fmt_g(sample.s21.re), fmt_g(sample.s21.im)),
        View::Swr => format!("{} : 1", num(rf::swr(sample.s11))),
        View::ReturnLoss => format!("{} dB", num(rf::return_loss_db(sample.s11))),
        View::LinMag => fmt_g(sample.s11.abs()),
        View::ReIm => format!("{} {:+}j", fmt_g(sample.s11.re), fmt_g(sample.s11.im)),
        View::S21 => format!("{} dB", num(rf::db(sample.s21))),
        View::Phase => format!("{} °", num(sample.s21.arg().to_degrees())),
        _ => format!("{unit}"),
    }
}

/// Where Cartesian panes draw: room on the left for tick labels and below for frequencies.
fn cart_plot(rect: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(rect.left() + 58.0, rect.top() + 10.0),
        Pos2::new(rect.right() - 16.0, rect.bottom() - 22.0),
    )
}

/// M1..M8 then R (white). Every marker also carries its name as text.
const MARKER_COLORS: [Color32; markers::SLOTS] = [
    Color32::from_rgb(0xFF, 0x99, 0x00),
    Color32::from_rgb(0x99, 0xCC, 0xFF),
    Color32::from_rgb(0xCC, 0x99, 0xCC),
    Color32::from_rgb(0x99, 0xDD, 0x88),
    Color32::from_rgb(0xFF, 0xCC, 0x66),
    Color32::from_rgb(0x66, 0xDD, 0xDD),
    Color32::from_rgb(0xFF, 0x88, 0x88),
    Color32::from_rgb(0xCC, 0xCC, 0x66),
    Color32::from_rgb(0xF0, 0xF0, 0xF0),
];
/// Major grid: 3:1 on the pane (WCAG 1.4.11). Issue 73.
const GRID: Color32 = Color32::from_rgb(0x58, 0x64, 0x76);
const GRID_FAINT: Color32 = Color32::from_rgb(0x2C, 0x33, 0x3D);
const TRACE1: Color32 = Color32::from_rgb(0xF2, 0xC2, 0x6B);
const TRACE2: Color32 = Color32::from_rgb(0x78, 0xAA, 0xD2);
/// TRC1..TRC4 colours for extra traces sharing a pane and their header chips.
const TRACE_COLORS: [Color32; 4] = [
    TRACE1,
    Color32::from_rgb(0x8C, 0xC8, 0xFF),
    Color32::from_rgb(0xE0, 0xA0, 0xE0),
    Color32::from_rgb(0xA0, 0xE0, 0x90),
];
/// MATH label text: brick is too dark for text on the strip.
/// Disconnected / lost text: brick fails 4.5:1 as text, this light red passes. Issue 73.
/// One status LED: its colour and the text that says what it means.
#[derive(Clone, Debug, PartialEq)]
struct Led {
    color: Color32,
    tip: String,
}

const LED_OK: Color32 = Color32::from_rgb(0x5E, 0xC2, 0x5A);
const LED_WARN: Color32 = Color32::from_rgb(0xE8, 0xA8, 0x30);
const LED_BAD: Color32 = Color32::from_rgb(0xE0, 0x4A, 0x3C);
const LED_OFF: Color32 = Color32::from_rgb(0x55, 0x55, 0x55);

/// The status-line warnings, also listed in the device details.
fn status_warnings(live: &device::Live) -> Vec<&'static str> {
    let mut warn = Vec::new();
    if live.identity.is_some_and(|id| !id.power_and_average()) {
        warn.push("FW < 1.0.2: NO DEV AVG/POWER");
    }
    if live.sweep_errors > 0 && live.connected {
        warn.push("SWEEP ERRORS");
    }
    if live.lost {
        warn.push("CHECK USB");
    }
    if device::power_note(live).is_some() {
        warn.push("ID UNRELIABLE: POWER CYCLE VNA");
    }
    warn
}

/// Device type and port only, e.g. `NANOVNA V2 · ttyACM0`.
fn device_short(live: &device::Live, port: &str) -> String {
    if live.connected {
        let name = Path::new(port).file_name().and_then(|n| n.to_str()).unwrap_or(port);
        if name.is_empty() { "NANOVNA V2".into() } else { format!("NANOVNA V2 · {name}") }
    } else if live.lost {
        "DISCONNECTED".into()
    } else {
        "NOT CONNECTED".into()
    }
}

/// LINK, ID, CAL, SWEEP lamps, left to right.
fn status_leds(live: &device::Live, cal: cal::CalState, restored: bool, sweeping: bool) -> [Led; 4] {
    let led = |color, tip: &str| Led { color, tip: tip.to_string() };
    let link = if live.connected {
        led(LED_OK, "LINK: connected")
    } else if live.lost {
        led(LED_BAD, "LINK: disconnected (lost), check USB")
    } else {
        led(LED_OFF, "LINK: not connected")
    };
    let id = match (live.connected, live.device_id.is_some(), live.id_reliable) {
        (true, true, true) => led(LED_OK, "ID: serial number read and checked"),
        (true, true, false) => led(LED_BAD, "ID UNRELIABLE: POWER CYCLE VNA"),
        _ => led(LED_OFF, "ID: no device id"),
    };
    let calibrated = match cal {
        cal::CalState::Exact if restored => led(LED_OK, "CAL: restored calibration applied"),
        cal::CalState::Exact => led(LED_OK, "CAL: calibration applied"),
        cal::CalState::Interp => led(LED_WARN, "CAL INT: calibration interpolated onto this grid"),
        cal::CalState::Off => led(LED_WARN, "CAL OFF: correction turned off"),
        cal::CalState::Uncal => led(LED_OFF, "UNCAL: no calibration for this sweep"),
    };
    let sweep = if !live.connected {
        led(LED_OFF, "SWEEP: idle, not connected")
    } else if live.sweep_errors > 0 {
        led(LED_WARN, "SWEEP: running with sweep errors")
    } else if sweeping || live.busy {
        led(LED_OK, "SWEEP: sweeping")
    } else {
        led(LED_OFF, "SWEEP: held")
    };
    [link, id, calibrated, sweep]
}

/// Joins the status segments with two spaces in their order, dropping the
/// lowest-priority segment (rightmost first on a tie) until the text fits
/// in `max` characters. Priority `u8::MAX` is never dropped; if that alone
/// is too long it is cut with an ellipsis. Never returns more than `max`.
fn fit_status(segs: &[(String, u8)], max: usize) -> String {
    let mut keep: Vec<bool> = vec![true; segs.len()];
    let join = |keep: &[bool]| {
        segs.iter().zip(keep).filter(|(_, k)| **k).map(|((t, _), _)| t.as_str()).collect::<Vec<_>>().join("  ")
    };
    loop {
        let text = join(&keep);
        if text.chars().count() <= max {
            return text;
        }
        let drop = segs
            .iter()
            .enumerate()
            .filter(|(i, (_, p))| keep[*i] && *p != u8::MAX)
            .min_by_key(|(i, (_, p))| (*p, usize::MAX - *i))
            .map(|(i, _)| i);
        match drop {
            Some(i) => keep[i] = false,
            None => {
                if max == 0 {
                    return String::new();
                }
                let mut cut: String = text.chars().take(max - 1).collect();
                cut.push('…');
                return cut;
            }
        }
    }
}

const LOST_TEXT: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x7A);
const MATH_TEXT: Color32 = Color32::from_rgb(0xFF, 0x99, 0x88);
/// Memory traces. Drawn dashed on Cartesian panes.
const MEM_COLOR: Color32 = palette::TEAL;
/// Ideal reference behind a calibration standard: light grey, above 3:1 on the pane. Issue 75.
const IDEAL_COLOR: Color32 = Color32::from_rgb(0x9A, 0x9C, 0xA8);
const PANE_BG: Color32 = Color32::from_rgb(12, 16, 22);

/// Graticule, labelled ticks, reference line, and traces on one Cartesian pane.
fn paint_cartesian(
    painter: &egui::Painter,
    rect: Rect,
    view: View,
    xs: &[f64],
    traces: &[(&[f64], Color32)],
    markers: &[(usize, usize)],
    label_px: f32,
    manual: Option<display::ManualScale>,
    overlay: &Overlay,
) {
    let plot = cart_plot(rect);
    let font = egui::FontId::monospace((label_px * 0.8).max(10.0));
    let mut all = Vec::new();
    for (ys, _) in traces {
        all.extend_from_slice(ys);
    }
    let scale = manual.map(|m| m.to_scale(10)).unwrap_or_else(|| display::auto_scale(&all, 10));
    for (i, value) in scale.lines().into_iter().enumerate() {
        let y = plot.top() + plot.height() * i as f32 / scale.divs as f32;
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0, if i % 5 == 0 { GRID } else { GRID_FAINT }),
        );
        if i % 2 == 0 {
            painter.text(
                Pos2::new(plot.left() - 6.0, y),
                egui::Align2::RIGHT_CENTER,
                display::fmt_tick(value, scale.per_div),
                font.clone(),
                palette::TEXT_MUTED,
            );
        }
    }
    let xmin = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let xmax = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if xmax > xmin {
        let tdr = view == View::Tdr;
        for tick in display::ticks(xmin, xmax, 6) {
            let x = plot.left() + ((tick - xmin) / (xmax - xmin)) as f32 * plot.width();
            painter.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(1.0, GRID_FAINT),
            );
            let text = if tdr {
                format!("{} m", trim_num(tick))
            } else {
                display::fmt_hz(tick)
            };
            painter.text(
                Pos2::new(x, plot.bottom() + 3.0),
                egui::Align2::CENTER_TOP,
                text,
                font.clone(),
                palette::TEXT_MUTED,
            );
        }
    }
    painter.rect_stroke(plot, 0.0, Stroke::new(1.0, GRID), egui::StrokeKind::Inside);
    // Reference line and its triangle on the right edge, like the R&S ref marker.
    let ref_value = manual.map(|m| m.ref_value).unwrap_or_else(|| display::auto_ref(&scale));
    let ry = plot.top() + scale.fraction(ref_value) as f32 * plot.height();
    let ref_stroke = Stroke::new(1.0, TRACE1);
    let mut x = plot.left();
    while x < plot.right() {
        let end = (x + 6.0).min(plot.right());
        painter.line_segment([Pos2::new(x, ry), Pos2::new(end, ry)], ref_stroke);
        x += 10.0;
    }
    painter.add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(plot.right(), ry),
            Pos2::new(plot.right() + 9.0, ry - 6.0),
            Pos2::new(plot.right() + 9.0, ry + 6.0),
        ],
        TRACE1,
        Stroke::NONE,
    ));
    if !(xmax > xmin) {
        return;
    }
    // Band-plan zone edges as dashed verticals, labelled with the frequency. Issue 53.
    if view != View::Tdr {
        let edge = Stroke::new(1.0, palette::TAN);
        for hz in overlay.edges.iter().filter(|hz| **hz > xmin && **hz < xmax) {
            let x = plot.left() + ((hz - xmin) / (xmax - xmin)) as f32 * plot.width();
            let mut y = plot.top();
            while y < plot.bottom() {
                painter.line_segment([Pos2::new(x, y), Pos2::new(x, (y + 4.0).min(plot.bottom()))], edge);
                y += 8.0;
            }
            painter.text(
                Pos2::new(x + 2.0, plot.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format!("EDGE {}", display::fmt_hz(*hz)),
                egui::FontId::monospace((label_px * 0.65).max(9.0)),
                palette::TAN,
            );
        }
    }
    // Filter / notch edges as solid verticals with a label. Issue 67.
    if view != View::Tdr {
        let st = Stroke::new(1.5, palette::PLUM.gamma_multiply(1.6));
        for (k, (hz, label)) in overlay.analysis.iter().enumerate().filter(|(_, (hz, _))| *hz > xmin && *hz < xmax) {
            let x = plot.left() + ((hz - xmin) / (xmax - xmin)) as f32 * plot.width();
            painter.line_segment([Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())], st);
            painter.text(
                Pos2::new(x + 2.0, plot.bottom() - 14.0 - (k % 2) as f32 * 12.0),
                egui::Align2::LEFT_TOP,
                *label,
                egui::FontId::monospace((label_px * 0.65).max(9.0)),
                palette::TEXT,
            );
        }
    }
    let map = |x: f64, y: f64| {
        Pos2::new(
            plot.left() + ((x - xmin) / (xmax - xmin)) as f32 * plot.width(),
            plot.top() + (scale.fraction(y) as f32).clamp(-0.02, 1.02) * plot.height(),
        )
    };
    let clip = painter.with_clip_rect(plot.expand(2.0));
    for (ys, color) in traces {
        if ys.len() != xs.len() {
            continue;
        }
        let mut prev: Option<Pos2> = None;
        // Memory traces are drawn dashed so they read apart from live data
        // without relying on colour. Issue 70.
        let dashed = *color == MEM_COLOR;
        let mut run = 0.0f32;
        for (x, y) in xs.iter().zip(ys.iter()) {
            if !y.is_finite() {
                prev = None;
                continue;
            }
            let pos = map(*x, *y);
            if let Some(was) = prev {
                let len: f32 = was.distance(pos);
                if !dashed || (run % 12.0) < 7.0 {
                    clip.line_segment([was, pos], Stroke::new(2.0, *color));
                }
                run += len.max(1.0);
            }
            prev = Some(pos);
        }
    }
    // Limit lines, failing points and the PASS/FAIL chip. Issue 71.
    let limit_stroke = Stroke::new(2.0, palette::BRICK);
    for seg in &overlay.limits {
        let (a, b) = (seg.start_hz.max(xmin), seg.stop_hz.min(xmax));
        if !(b >= a) {
            continue;
        }
        let (va, vb) = (seg.at(a).unwrap_or(seg.start_v), seg.at(b).unwrap_or(seg.stop_v));
        let (pa, pb) = (map(a, va), map(b, vb));
        clip.line_segment([pa, pb], limit_stroke);
        // Small ticks point to the failing side: up for a lower limit, down for an upper.
        let dir = if seg.upper { -6.0 } else { 6.0 };
        for t in [0.0f32, 0.5, 1.0] {
            let p = pa + (pb - pa) * t;
            clip.line_segment([p, Pos2::new(p.x, p.y + dir)], limit_stroke);
        }
    }
    if let (Some(verdict), Some((ys, _))) = (&overlay.verdict, traces.first()) {
        for i in &verdict.failing {
            if let (Some(x), Some(y)) = (xs.get(*i), ys.get(*i)) {
                if y.is_finite() {
                    clip.circle_stroke(map(*x, *y), 4.0, Stroke::new(2.0, palette::BRICK));
                }
            }
        }
        let text = format!("LIMIT {}", verdict.label());
        let fill = if verdict.pass() { palette::OLIVE } else { palette::BRICK };
        let g = painter.layout_no_wrap(text, egui::FontId::monospace((label_px * 0.9).max(11.0)), rustcars::ink_on(fill));
        let chip = Rect::from_min_size(Pos2::new(plot.left() + 8.0, plot.top() + 8.0), g.size() + vec2(16.0, 8.0));
        painter.rect_filled(chip, 4.0, fill);
        painter.galley(chip.min + vec2(8.0, 4.0), g, rustcars::ink_on(fill));
    }
    if let Some((ys, _)) = traces.first() {
        for (slot, index) in markers {
            if let (Some(x), Some(y)) = (xs.get(*index), ys.get(*index)) {
                if y.is_finite() {
                    paint_marker(painter, map(*x, *y), *slot, label_px);
                }
            }
        }
    }
}

/// A numbered marker: a downward triangle on the trace with its number above.
fn paint_marker(painter: &egui::Painter, at: Pos2, slot: usize, label_px: f32) {
    let color = MARKER_COLORS[slot % MARKER_COLORS.len()];
    painter.add(egui::Shape::convex_polygon(
        vec![
            at,
            Pos2::new(at.x - 6.0, at.y - 10.0),
            Pos2::new(at.x + 6.0, at.y - 10.0),
        ],
        color,
        Stroke::new(1.0, Color32::BLACK),
    ));
    painter.text(
        Pos2::new(at.x, at.y - 12.0),
        egui::Align2::CENTER_BOTTOM,
        if slot == markers::REF { "R".to_string() } else { format!("{}", slot + 1) },
        egui::FontId::monospace((label_px * 0.8).max(10.0)),
        color,
    );
}


/// Left and right rects for the elbow frames. The left side stays at `left_w`
/// until the window is too narrow to keep a chart beside it.
fn body_split(ui: &mut Ui, theme: &LcarsTheme, left_w: f32) -> (Rect, Rect) {
    let full = ui.available_rect_before_wrap();
    let gap = theme.main_gap;
    let left_w = left_w.min((full.width() - gap).max(0.0) * 0.5);
    let split = full.left() + left_w;
    let left = Rect::from_min_max(full.min, Pos2::new(split, full.bottom()));
    let right = Rect::from_min_max(Pos2::new((split + gap).min(full.right()), full.top()), full.max);
    ui.allocate_rect(full, Sense::hover());
    (left, right)
}

fn show_frame(ui: &mut Ui, theme: &LcarsTheme, rect: Rect, side: Side, add: impl FnOnce(&mut Ui)) {
    if rect.width() < 8.0 || rect.height() < 8.0 {
        return;
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        let mut frame = LcarsFrame::new(theme, side, 0.6);
        if side == Side::Right {
            frame.top_color = palette::PLUM;
            frame.bottom_color = palette::SLATE;
        }
        frame.min_height = rect.height();
        frame.show(ui, theme, add);
    });
}

/// A small group caption over a block of keys, with a rule like a front-panel silkscreen.
fn key_caption(ui: &mut Ui, theme: &LcarsTheme, text: &str, width: f32) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 16.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let g = painter.layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(theme.data_size * 0.8),
        palette::TEXT_MUTED,
    );
    let tw = g.size().x;
    painter.galley(Pos2::new(rect.left(), rect.center().y - g.size().y / 2.0), g, palette::TEXT_MUTED);
    painter.line_segment(
        [
            Pos2::new(rect.left() + tw + 8.0, rect.center().y),
            Pos2::new(rect.right(), rect.center().y),
        ],
        Stroke::new(1.0, palette::TAN),
    );
}

/// A uniform key cap. Lit keys are tan, idle keys slate.
/// Click mode 1 / 2 / 3 for a key or plot click, 0 = none. Shift+click is a
/// double click and Ctrl+click a triple click (rustcars convention). Issue 73.
/// Light green / light red on the header strip; the words carry the state.
const PASS_TEXT: Color32 = Color32::from_rgb(0xA8, 0xE0, 0x8C);
const FAIL_TEXT: Color32 = Color32::from_rgb(0xFF, 0xA0, 0x90);

/// "LIMIT PASS" / "LIMIT FAIL" / "LIMIT: NO POINTS" for the trace header. Issue 63.
fn header_verdict(v: &limits::Verdict) -> (String, Color32) {
    let label = v.label();
    let text = if label.starts_with("LIMIT") { label.to_string() } else { format!("LIMIT {label}") };
    let color = if v.tested == 0 { palette::TEXT_MUTED } else if v.pass() { PASS_TEXT } else { FAIL_TEXT };
    (text, color)
}

fn click_n(ui: &Ui, resp: &egui::Response) -> u8 {
    let m = ui.input(|i| i.modifiers);
    keys::click_mode(resp.clicked(), resp.double_clicked(), resp.triple_clicked(), m.shift, m.ctrl || m.command)
}

fn softkey(ui: &mut Ui, theme: &LcarsTheme, label: &str, lit: bool, width: f32) -> bool {
    softkey_enabled(ui, theme, label, lit, width, true)
}

fn softkey_enabled(ui: &mut Ui, theme: &LcarsTheme, label: &str, lit: bool, width: f32, enabled: bool) -> bool {
    let resp = key_cap(ui, theme, label, lit, width, enabled);
    enabled && resp.clicked()
}

/// A key cap that hands back its response, for keys with double and triple click
/// modes. Three pips down the right edge in the shared click-mode colours
/// (green single, orange double, purple triple) mark it as a multi-mode key. Issue 73.
fn softkey_response(ui: &mut Ui, theme: &LcarsTheme, label: &str, lit: bool, width: f32, enabled: bool) -> egui::Response {
    let resp = key_cap(ui, theme, label, lit, width, enabled);
    if enabled {
        let x = resp.rect.right() - 5.0;
        let y = resp.rect.center().y;
        for (k, mode) in [rustcars::Mode::Primary, rustcars::Mode::Alt1, rustcars::Mode::Alt2].into_iter().enumerate() {
            ui.painter().circle_filled(Pos2::new(x, y + (k as f32 - 1.0) * 6.0), 2.0, mode.colors().border);
        }
    }
    resp
}

fn key_cap(ui: &mut Ui, theme: &LcarsTheme, label: &str, lit: bool, width: f32, enabled: bool) -> egui::Response {
    // A key cap: centered label with padding on every side. The label steps
    // down in size before it is allowed to touch the edge of the cap.
    const PAD: f32 = 8.0;
    let base = if lit { palette::TAN } else { palette::SLATE };
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, KEY_H), sense);
    let fill = if !enabled {
        rustcars::tint(base, -0.6)
    } else if resp.is_pointer_button_down_on() {
        rustcars::tint(base, -0.25)
    } else if resp.hovered() {
        rustcars::tint(base, 0.3)
    } else {
        base
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, fill);
    if resp.has_focus() {
        painter.rect_stroke(
            rect.shrink(2.0),
            4.0,
            Stroke::new(2.0, theme.text_hi),
            egui::StrokeKind::Inside,
        );
    }
    let ink = rustcars::ink_on(fill);
    let text = label.to_uppercase();
    let mut size = (theme.header_size * 0.85).min(17.0);
    let mut galley = painter.layout_no_wrap(text.clone(), egui::FontId::proportional(size), ink);
    while galley.size().x > width - 2.0 * PAD && size > 9.0 {
        size -= 1.0;
        galley = painter.layout_no_wrap(text.clone(), egui::FontId::proportional(size), ink);
    }
    let at = rect.center() - galley.size() * 0.5;
    painter.galley(at, galley, ink);
    resp
}

/// Height of one key cap in the left key panel.
const KEY_H: f32 = 32.0;

fn paint_chart(
    ui: &mut Ui,
    view: View,
    samples: Option<&[Sample]>,
    memory: Option<&[Sample]>,
    math: Option<&[Sample]>,
    z0: f64,
    velocity: f64,
    admittance: bool,
    label_px: f32,
    markers: &[(usize, usize)],
    info: &[(String, Color32)],
    manual: Option<display::ManualScale>,
    overlay: &Overlay,
    extra: &[(Vec<f64>, Color32)],
) -> (Option<(usize, u8)>, Option<String>, ScaleGesture) {
    // S21 polar draws the S21 points through the S11 polar painter.
    let as_s11 = |s: &[Sample]| -> Vec<Sample> { s.iter().map(|x| Sample { s11: x.s21, ..*x }).collect() };
    let (mapped, mapped_mem);
    // The admittance Smith format is the Smith painter with its Y grid.
    let (view, admittance) = if view == View::SmithY { (View::Smith, true) } else { (view, admittance) };
    let (view, samples, memory) = if view == View::PolarS21 {
        mapped = samples.map(as_s11);
        mapped_mem = memory.map(as_s11);
        (View::Polar, mapped.as_deref(), mapped_mem.as_deref())
    } else {
        (view, samples, memory)
    };
    let size = vec2(
        ui.available_width().max(240.0),
        ui.available_height().max(160.0),
    );
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    if view != View::Smith {
        painter.rect_filled(rect, 0.0, PANE_BG);
    }
    let plot = rect.shrink(16.0);
    let hover = match view {
        View::Smith => paint_smith(
            &painter,
            rect,
            admittance,
            samples,
            memory,
            z0,
            resp.hover_pos(),
            label_px,
        ),
        View::Polar => {
            paint_polar(&painter, plot, label_px);
            if let Some(samples) = samples {
                stroke_polar(&painter, plot, samples, palette::TAN);
            }
            if let Some(memory) = memory {
                stroke_polar(&painter, plot, memory, palette::TEAL);
            }
            // Constant-|Γ| circle through the pointer and its readout. Issue 52.
            let center = plot.center();
            let radius = plot.width().min(plot.height()) * 0.45;
            resp.hover_pos().and_then(|p| {
                let re = ((p.x - center.x) / radius) as f64;
                let im = (-(p.y - center.y) / radius) as f64;
                let r = polar_readout(re, im)?;
                painter.circle_stroke(center, radius * r.mag as f32, Stroke::new(1.0, palette::TAN));
                Some(format!(
                    "|Γ| {:.3}  ∠ {:.1}°  SWR {}  RL {} dB",
                    r.mag,
                    r.angle_deg,
                    if r.swr.is_finite() { format!("{:.2}", r.swr) } else { "INF".into() },
                    if r.rl_db.is_finite() { format!("{:.2}", r.rl_db) } else { "INF".into() },
                ))
            })
        }
        _ => {
            let (xs, ys) = samples
                .map(|samples| series(view, samples, z0, velocity))
                .unwrap_or_default();
            let second = samples
                .map(|samples| second_series(view, samples, z0))
                .unwrap_or_default();
            let (mx, my) = memory
                .map(|memory| series(view, memory, z0, velocity))
                .unwrap_or_default();
            let delta: Vec<f64> = math.map(|m| series(view, m, z0, velocity).1).unwrap_or_default();
            // A memory on another grid is resampled onto the trace's frequencies.
            let my: Vec<f64> = if !my.is_empty() && mx != xs {
                traces::resample(&mx, &my, &xs)
            } else {
                my
            };
            let mut traces: Vec<(&[f64], Color32)> = vec![(&ys, TRACE1)];
            if !second.is_empty() {
                traces.push((&second, TRACE2));
            }
            if my.len() == ys.len() && !my.is_empty() {
                traces.push((&my, MEM_COLOR));
            }
            if delta.len() == ys.len() && !delta.is_empty() {
                traces.push((&delta, palette::BRICK));
            }
            for (e, c) in extra {
                if e.len() == ys.len() {
                    traces.push((e, *c));
                }
            }
            paint_cartesian(&painter, rect, view, &xs, &traces, markers, label_px, manual, overlay);
            None
        }
    };
    if is_disk(view) {
        if let Some(samples) = samples {
            let (center, radius) = if view == View::Smith {
                let disk = smith_disk(rect);
                (disk.center, disk.rad)
            } else {
                let plot = rect.shrink(16.0);
                (plot.center(), plot.width().min(plot.height()) * 0.45)
            };
            for (slot, index) in markers {
                if let Some(sample) = samples.get(*index) {
                    let (mut re, mut im) = (sample.s11.re, sample.s11.im);
                    if !(re.is_finite() && im.is_finite()) {
                        continue;
                    }
                    let mag = re.hypot(im);
                    if mag > 1.0 {
                        re /= mag;
                        im /= mag;
                    }
                    let at = Pos2::new(center.x + re as f32 * radius, center.y - im as f32 * radius);
                    paint_marker(&painter, at, *slot, label_px);
                }
            }
        }
    }
    paint_marker_info(&painter, rect, info, label_px);
    // single = active marker, double = next marker, triple = R. A drag on a
    // Cartesian pane moves the active marker along the trace.
    // Reference triangle drag and mouse wheel over the value axis. Issue 69.
    let mut gesture = ScaleGesture::default();
    let cart = !is_disk(view);
    let plot_c = cart_plot(rect);
    let grab_id = resp.id.with("ref-grab");
    if cart && resp.drag_started() {
        let near = resp
            .interact_pointer_pos()
            .is_some_and(|p| p.x >= plot_c.right() - 4.0 && p.x <= rect.right());
        ui.data_mut(|d| d.insert_temp(grab_id, near));
    }
    let grabbing = cart && resp.dragged() && ui.data(|d| d.get_temp::<bool>(grab_id)).unwrap_or(false);
    if grabbing && plot_c.height() > 0.0 {
        gesture.ref_divs = -resp.drag_delta().y / plot_c.height() * 10.0;
    }
    if cart {
        if let Some(p) = resp.hover_pos() {
            if p.x < plot_c.left() {
                let dy: f32 = ui.input(|i| {
                    i.raw
                        .events
                        .iter()
                        .map(|e| match e {
                            egui::Event::MouseWheel { delta, .. } => delta.y,
                            _ => 0.0,
                        })
                        .sum()
                });
                if dy > 0.0 {
                    gesture.wheel = -1;
                } else if dy < 0.0 {
                    gesture.wheel = 1;
                }
            }
        }
    }
    let clicks = if grabbing {
        None
    } else if click_n(ui, &resp) == 3 {
        Some(3)
    } else if click_n(ui, &resp) == 2 {
        Some(2)
    } else if click_n(ui, &resp) == 1 || (resp.dragged() && !is_disk(view)) {
        Some(1)
    } else {
        None
    };
    let clicked = clicks.and_then(|n| {
        samples
            .and_then(|samples| hit(view, rect, samples, resp.interact_pointer_pos()?))
            .map(|i| (i, n))
    });
    // Cursor readout on Cartesian panes: SWR, RL, R and X under the pointer. Issue 53.
    let hover = match (hover, samples, resp.hover_pos()) {
        (None, Some(samples), Some(pos)) if !is_disk(view) && view != View::Tdr => {
            hit(view, rect, samples, pos).and_then(|i| samples.get(i)).map(|s| {
                let d = rf::derive(s, z0);
                format!(
                    "{}  SWR {}  RL {} dB  R {} Ω  X {} Ω",
                    display::fmt_hz(s.hz),
                    num(rf::swr(s.s11)),
                    num(rf::return_loss_db(s.s11)),
                    num(d.r),
                    num(d.x)
                )
            })
        }
        (h, _, _) => h,
    };
    if let Some(text) = &hover {
        resp.on_hover_text(text);
    }
    (clicked, hover, gesture)
}

/// Band edges and limit lines drawn over a Cartesian pane. Issues 53 and 71.
#[derive(Default)]
struct Overlay {
    edges: Vec<f64>,
    /// Filter / notch analysis verticals: frequency and short label. Issue 67.
    analysis: Vec<(f64, &'static str)>,
    limits: Vec<limits::Segment>,
    verdict: Option<limits::Verdict>,
}

/// Vertical lines for the filter and notch analysis. Issue 67.
fn analysis_marks(xs: &[f64], db: &[f64], filter: bool, notch: bool) -> Vec<(f64, &'static str)> {
    let mut v = Vec::new();
    if filter {
        if let Some(f) = search::bandfilter(xs, db) {
            v.extend([(f.lo3, "-3dB"), (f.center_hz, "CF"), (f.hi3, "-3dB"), (f.lo6, "-6dB"), (f.hi6, "-6dB")]);
        }
    }
    if notch {
        if let Some(n) = search::notch(xs, db) {
            v.extend([(n.lo3, "N-3dB"), (n.hz, "NOTCH"), (n.hi3, "N-3dB")]);
        }
    }
    v
}

/// What the operator did to the scale on the pane this frame. Issue 69.
#[derive(Clone, Copy, Default)]
struct ScaleGesture {
    /// Divisions the reference triangle was dragged up (positive) or down.
    ref_divs: f32,
    /// +1 coarser, -1 finer per division, from the wheel over the value axis.
    wheel: i32,
}

/// Stable id of a format for the layout: "S11 SMITH", "S21 DB MAG". Issue 62.
fn view_id(view: View) -> String {
    let (param, format, _) = view_meta(view);
    format!("{param} {format}")
}

fn view_from_id(id: &str) -> Option<View> {
    VIEWS.iter().map(|(v, _)| *v).find(|v| view_id(*v) == id)
}

/// Key under which a trace format's manual scale is saved.
fn scale_key(view: View) -> &'static str {
    view_meta(view).1
}

/// The marker list in the top right of a pane, each line in its marker colour.
fn paint_marker_info(painter: &egui::Painter, rect: Rect, info: &[(String, Color32)], label_px: f32) {
    if info.is_empty() {
        return;
    }
    let font = egui::FontId::monospace((label_px * 0.75).max(10.0));
    let line_h = font.size + 3.0;
    let galleys: Vec<_> = info
        .iter()
        .map(|(text, color)| painter.layout_no_wrap(text.clone(), font.clone(), *color))
        .collect();
    let w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 12.0;
    let h = line_h * galleys.len() as f32 + 8.0;
    let boxr = Rect::from_min_size(Pos2::new(rect.right() - w - 20.0, rect.top() + 12.0), vec2(w, h));
    painter.rect_filled(boxr, 3.0, Color32::from_rgba_unmultiplied(0, 0, 0, 200));
    painter.rect_stroke(boxr, 3.0, Stroke::new(1.0, GRID), egui::StrokeKind::Inside);
    for (i, g) in galleys.into_iter().enumerate() {
        let color = info[i].1;
        painter.galley(Pos2::new(boxr.left() + 6.0, boxr.top() + 4.0 + line_h * i as f32), g, color);
    }
}

/// Off → D−M → D/M → off. Issue 70.
fn next_math(m: traces::Math) -> traces::Math {
    match m {
        traces::Math::Off => traces::Math::Sub,
        traces::Math::Sub => traces::Math::Div,
        traces::Math::Div => traces::Math::Off,
    }
}

/// "14.2" or "14.2 MHz" as Hz. Issue 66.
fn parse_marker_mhz(text: &str) -> Option<f64> {
    let t = text.trim().trim_end_matches("MHz").trim_end_matches("mhz").trim();
    let mhz: f64 = t.parse().ok()?;
    (mhz.is_finite() && mhz > 0.0).then_some(mhz * 1e6)
}

/// The two jacks, labeled the way the V2 marks them: S11 and S21.
fn paint_wiring(ui: &mut Ui, step: WizStep) {
    let (rect, _) = ui.allocate_exact_size(vec2(460.0, 214.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(12, 16, 22));
    let body = Rect::from_min_max(
        Pos2::new(rect.left() + 28.0, rect.top() + 86.0),
        Pos2::new(rect.right() - 28.0, rect.bottom() - 12.0),
    );
    painter.rect_filled(body, 6.0, Color32::from_rgb(32, 42, 56));
    painter.rect_stroke(
        body,
        6.0,
        Stroke::new(1.5, palette::SLATE),
        egui::StrokeKind::Inside,
    );
    painter.text(
        Pos2::new(body.center().x, body.bottom() - 16.0),
        egui::Align2::CENTER_CENTER,
        "NanoVNA V2",
        egui::FontId::proportional(14.0),
        palette::TEXT_MUTED,
    );
    let s11 = Pos2::new(body.left() + body.width() * 0.28, body.top());
    let s21 = Pos2::new(body.left() + body.width() * 0.72, body.top());
    paint_jack(&painter, s11, "S11");
    paint_jack(&painter, s21, "S21");
    match step {
        WizStep::Open => {
            paint_open(&painter, s11);
            paint_bare(&painter, s21);
        }
        WizStep::Short => {
            paint_short(&painter, s11);
            paint_bare(&painter, s21);
        }
        WizStep::Load => {
            paint_load(&painter, s11);
            paint_bare(&painter, s21);
        }
        WizStep::Thru => paint_cable(&painter, s11, s21),
        WizStep::Isol => {
            paint_load(&painter, s11);
            paint_load(&painter, s21);
        }
        WizStep::Finish => {
            paint_bare(&painter, s11);
            paint_bare(&painter, s21);
        }
    }
}

/// The measure control after it has been pressed. Clicks are taken and dropped.
fn held_button(ui: &mut Ui, theme: &LcarsTheme, label: &str, size: egui::Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = Color32::from_rgb(0x3A, 0x0C, 0x10);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, fill);
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, Color32::from_rgb(0x6A, 0x18, 0x1C)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label.to_uppercase(),
        egui::FontId::monospace(theme.data_size),
        Color32::from_rgb(0xA8, 0x78, 0x74),
    );
    resp
}

/// A text block with a reserved height. A longer sentence wraps inside it
/// and does not push the rows below.
fn fixed_readout(ui: &mut Ui, theme: &LcarsTheme, text: &str, lines: u32, highlight: bool) {
    let width = ui.available_width().max(1.0);
    let line = theme.data_size + 5.0;
    let height = line * lines as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let color = if highlight { theme.text_hi } else { theme.text };
    let galley = ui.painter().layout(
        text.to_owned(),
        egui::FontId::proportional(theme.data_size),
        color,
        rect.width(),
    );
    ui.painter_at(rect)
        .galley(rect.left_top(), galley, color);
}

fn reload_button(ui: &mut Ui, theme: &LcarsTheme) -> egui::Response {
    let resp = action_button(ui, theme, "", vec2(super::LIST_ROW, super::LIST_ROW));
    paint_reload(ui.painter_at(resp.rect), resp.rect);
    resp.on_hover_text("Rescan")
}

/// A circular arrow. The square is the rescan control on a calibration row.
fn paint_reload(painter: egui::Painter, rect: Rect) {
    let center = rect.center();
    let radius = rect.width() * 0.26;
    let color = Color32::from_rgb(0xE8, 0xE4, 0xD8);
    let start = 0.6_f32;
    let end = 5.5_f32;
    let mut previous = None;
    for step in 0..=18 {
        let angle = start + (end - start) * (step as f32 / 18.0);
        let point = Pos2::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        );
        if let Some(previous) = previous {
            painter.line_segment([previous, point], Stroke::new(2.0, color));
        }
        previous = Some(point);
    }
    let tip = Pos2::new(center.x + radius * end.cos(), center.y + radius * end.sin());
    let tangent = egui::vec2(-end.sin(), end.cos());
    let normal = egui::vec2(-tangent.y, tangent.x);
    painter.line_segment(
        [tip, tip - tangent * 5.0 + normal * 3.5],
        Stroke::new(2.0, color),
    );
    painter.line_segment(
        [tip, tip - tangent * 5.0 - normal * 3.5],
        Stroke::new(2.0, color),
    );
}

fn paint_jack(painter: &egui::Painter, at: Pos2, name: &str) {
    painter.circle_filled(at, 12.0, palette::TAN);
    painter.circle_filled(at, 4.5, Color32::from_rgb(18, 14, 10));
    painter.text(
        Pos2::new(at.x, at.y + 18.0),
        egui::Align2::CENTER_TOP,
        name,
        egui::FontId::proportional(22.0),
        palette::TEXT,
    );
}

fn paint_caption(painter: &egui::Painter, at: Pos2, text: &str) {
    painter.text(
        Pos2::new(at.x, at.y - 62.0),
        egui::Align2::CENTER_BOTTOM,
        text,
        egui::FontId::proportional(14.0),
        palette::TEXT,
    );
}

fn paint_stem(painter: &egui::Painter, at: Pos2, up: f32) {
    painter.line_segment(
        [at, Pos2::new(at.x, at.y - up)],
        Stroke::new(2.0, palette::TEAL),
    );
}

fn paint_open(painter: &egui::Painter, at: Pos2) {
    paint_stem(painter, at, 28.0);
    painter.circle_stroke(
        Pos2::new(at.x, at.y - 40.0),
        10.0,
        Stroke::new(2.0, palette::TEAL),
    );
    paint_caption(painter, at, "open");
}

fn paint_short(painter: &egui::Painter, at: Pos2) {
    paint_stem(painter, at, 22.0);
    painter.line_segment(
        [Pos2::new(at.x - 16.0, at.y - 22.0), Pos2::new(at.x + 16.0, at.y - 22.0)],
        Stroke::new(4.0, palette::TEAL),
    );
    paint_caption(painter, at, "short");
}

fn paint_load(painter: &egui::Painter, at: Pos2) {
    paint_stem(painter, at, 16.0);
    let body = Rect::from_center_size(Pos2::new(at.x, at.y - 36.0), vec2(48.0, 26.0));
    painter.rect_filled(body, 3.0, palette::TEAL);
    painter.text(
        body.center(),
        egui::Align2::CENTER_CENTER,
        "50",
        egui::FontId::monospace(16.0),
        Color32::from_rgb(12, 16, 22),
    );
    paint_caption(painter, at, "50 ohm");
}

fn paint_bare(painter: &egui::Painter, at: Pos2) {
    painter.text(
        Pos2::new(at.x, at.y - 20.0),
        egui::Align2::CENTER_BOTTOM,
        "empty",
        egui::FontId::proportional(13.0),
        palette::TEXT_MUTED,
    );
}

fn paint_cable(painter: &egui::Painter, left: Pos2, right: Pos2) {
    let crown = Pos2::new((left.x + right.x) * 0.5, left.y - 52.0);
    let mut prev = left;
    for step in 1..=18 {
        let t = step as f32 / 18.0;
        let u = 1.0 - t;
        let point = Pos2::new(
            u * u * left.x + 2.0 * u * t * crown.x + t * t * right.x,
            u * u * left.y + 2.0 * u * t * crown.y + t * t * right.y,
        );
        painter.line_segment([prev, point], Stroke::new(3.0, palette::TAN));
        prev = point;
    }
    painter.text(
        Pos2::new(crown.x, crown.y - 6.0),
        egui::Align2::CENTER_BOTTOM,
        "cable",
        egui::FontId::proportional(14.0),
        palette::TEXT,
    );
}

/// Same disk, constant-R and constant-X circles, and rim labels as the Tools Smith screen.
/// Y draws the admittance grid mirrored through the centre. The sweep stays S11.
fn paint_smith(
    painter: &egui::Painter,
    rect: Rect,
    admittance: bool,
    samples: Option<&[Sample]>,
    memory: Option<&[Sample]>,
    z0: f64,
    hover: Option<Pos2>,
    label_px: f32,
) -> Option<String> {
    use crate::tools::smith::math::{self, cx};
    const GRID_Z: Color32 = Color32::from_rgb(0x4A, 0x5A, 0x6E);
    const GRID_Y: Color32 = Color32::from_rgb(0x6E, 0x4E, 0x48);
    let disk = smith_disk(rect);
    let to_px = |p: math::Cx| {
        Pos2::new(
            disk.center.x + (p.re as f32) * disk.rad,
            disk.center.y - (p.im as f32) * disk.rad,
        )
    };
    let poly = |pts: &[math::Cx], stroke: Stroke| {
        if pts.len() > 1 {
            painter.add(egui::Shape::line(
                pts.iter().copied().map(to_px).collect(),
                stroke,
            ));
        }
    };
    let circle = |c: math::Cx, r: f64, stroke: Stroke| {
        for run in math::clipped_circle(c, r, 720) {
            poly(&run, stroke);
        }
    };
    painter.circle_filled(disk.center, disk.rad, palette::SURFACE);
    if admittance {
        let s = Stroke::new(1.0, GRID_Y);
        for r in math::GRID_R.iter().skip(1) {
            let (c, rr) = math::r_circle(*r);
            circle(c.scale(-1.0), rr, s);
        }
        for x in math::GRID_X {
            for x in [x, -x] {
                let (c, rr) = math::x_circle(x);
                circle(c.scale(-1.0), rr, s);
            }
        }
    }
    let gz = Stroke::new(1.0, GRID_Z);
    let font = egui::FontId::monospace(label_px);
    for r in math::GRID_R {
        let (c, rr) = math::r_circle(r);
        circle(
            c,
            rr,
            if r == 0.0 {
                Stroke::new(1.5, palette::TEXT_MUTED)
            } else {
                gz
            },
        );
        if r > 0.0 {
            painter.text(
                to_px(cx((r - 1.0) / (r + 1.0), 0.0)) + vec2(2.0, -2.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{r}"),
                font.clone(),
                palette::TEXT_MUTED,
            );
        }
    }
    for x in math::GRID_X {
        for x in [x, -x] {
            let (c, rr) = math::x_circle(x);
            circle(c, rr, gz);
            let rim = math::gamma(cx(0.0, x));
            painter.text(
                to_px(rim.scale(1.08)),
                egui::Align2::CENTER_CENTER,
                format!("{}{}", if x < 0.0 { "-j" } else { "+j" }, x.abs()),
                font.clone(),
                palette::TEXT_MUTED,
            );
        }
    }
    painter.line_segment([to_px(cx(-1.0, 0.0)), to_px(cx(1.0, 0.0))], gz);
    painter.text(
        to_px(cx(-1.0, 0.0)) + vec2(-4.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        "0",
        font.clone(),
        palette::TEXT_MUTED,
    );
    painter.text(
        to_px(cx(1.0, 0.0)) + vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "INF",
        font.clone(),
        palette::TEXT_MUTED,
    );
    if let Some(memory) = memory {
        stroke_gamma(painter, disk, memory, palette::TEAL);
    }
    if let Some(samples) = samples {
        stroke_gamma(painter, disk, samples, palette::TAN);
    }
    let g = hover.map(|q| {
        cx(
            ((q.x - disk.center.x) / disk.rad) as f64,
            ((disk.center.y - q.y) / disk.rad) as f64,
        )
    });
    g.filter(|p| p.abs() <= 1.0).map(|p| {
        let z = math::z_of(p);
        format!(
            "POINTER  z = {}   Z = {} ohm   |GAMMA| {}",
            fmt_cx(z),
            fmt_cx(z.scale(z0)),
            fmt_g(p.abs())
        )
    })
}

fn fmt_g(v: f64) -> String {
    if !v.is_finite() {
        return "INF".into();
    }
    if v == 0.0 {
        return "0".into();
    }
    let dec = (2 - v.abs().log10().floor() as i32).clamp(0, 6) as usize;
    format!("{v:.dec$}")
}

fn fmt_cx(z: crate::tools::smith::math::Cx) -> String {
    if !z.re.is_finite() || !z.im.is_finite() {
        return "OPEN (INF)".into();
    }
    let sign = if z.im < 0.0 { '-' } else { '+' };
    format!("{} {sign} j{}", fmt_g(z.re), fmt_g(z.im.abs()))
}

#[derive(Clone, Copy)]
struct SmithDisk {
    center: Pos2,
    rad: f32,
}

/// Centre and radius of the Smith disk. Thirty pixels stay outside the rim for the labels.
fn smith_disk(rect: Rect) -> SmithDisk {
    let side = rect.width().min(rect.height());
    SmithDisk {
        center: rect.center(),
        rad: (side / 2.0 - 30.0).max(8.0),
    }
}

/// Keep the part of the segment that lies inside the unit disk.
/// Points outside the rim are not on a Smith chart, and a huge gamma
/// would draw a line across the whole window.
fn clip_gamma_segment(a: (f64, f64), b: (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
    let inside = |p: (f64, f64)| p.0 * p.0 + p.1 * p.1 <= 1.0;
    let a_in = inside(a);
    let b_in = inside(b);
    if a_in && b_in {
        return Some((a, b));
    }
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let a2 = dx * dx + dy * dy;
    if a2 < 1e-18 {
        return None;
    }
    let bq = 2.0 * (a.0 * dx + a.1 * dy);
    let cq = a.0 * a.0 + a.1 * a.1 - 1.0;
    let disc = bq * bq - 4.0 * a2 * cq;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let mut hits = [(-bq - root) / (2.0 * a2), (-bq + root) / (2.0 * a2)];
    hits.sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
    let at = |t: f64| (a.0 + t * dx, a.1 + t * dy);
    let on = hits.into_iter().filter(|t| (0.0..=1.0).contains(t)).map(at);
    let mut on = on.collect::<Vec<_>>();
    if a_in {
        on.insert(0, a);
    }
    if b_in {
        on.push(b);
    }
    if on.len() >= 2 {
        Some((on[0], on[on.len() - 1]))
    } else {
        None
    }
}

fn stroke_gamma(painter: &egui::Painter, disk: SmithDisk, samples: &[Sample], color: Color32) {
    let to_px = |p: (f64, f64)| {
        Pos2::new(
            disk.center.x + p.0 as f32 * disk.rad,
            disk.center.y - p.1 as f32 * disk.rad,
        )
    };
    let mut run: Vec<Pos2> = Vec::new();
    let mut prev: Option<(f64, f64)> = None;
    let mut last: Option<Pos2> = None;
    for sample in samples {
        let here = (sample.s11.re, sample.s11.im);
        if !here.0.is_finite() || !here.1.is_finite() {
            stroke_run(painter, &mut run, color);
            prev = None;
            last = None;
            continue;
        }
        if let Some(was) = prev {
            if let Some((start, end)) = clip_gamma_segment(was, here) {
                let start = to_px(start);
                let end = to_px(end);
                let broken = last.map(|p| p.distance(start) > 0.75).unwrap_or(true);
                if broken {
                    stroke_run(painter, &mut run, color);
                    run.push(start);
                }
                run.push(end);
                last = Some(end);
            } else {
                stroke_run(painter, &mut run, color);
                last = None;
            }
        }
        prev = Some(here);
    }
    stroke_run(painter, &mut run, color);
}

fn stroke_run(painter: &egui::Painter, run: &mut Vec<Pos2>, color: Color32) {
    if run.len() > 1 {
        painter.add(egui::Shape::line(
            std::mem::take(run),
            Stroke::new(2.0, color),
        ));
    } else {
        run.clear();
    }
}

/// Magnitude, angle, SWR and return loss of a point on the polar plot. Issue 52.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PolarReading {
    mag: f64,
    angle_deg: f64,
    swr: f64,
    rl_db: f64,
}

/// None outside the unit circle, where a passive load cannot be.
fn polar_readout(re: f64, im: f64) -> Option<PolarReading> {
    let mag = re.hypot(im);
    if !(mag <= 1.0) {
        return None;
    }
    let g = Cx::new(re, im);
    Some(PolarReading { mag, angle_deg: im.atan2(re).to_degrees(), swr: rf::swr(g), rl_db: rf::return_loss_db(g) })
}

/// |Γ| for an SWR ring.
fn gamma_for_swr(swr: f64) -> f64 {
    (swr - 1.0) / (swr + 1.0)
}

fn paint_polar(painter: &egui::Painter, plot: Rect, label_px: f32) {
    let center = plot.center();
    let radius = plot.width().min(plot.height()) * 0.45;
    let ring = Stroke::new(1.0, Color32::from_rgb(70, 84, 102));
    painter.circle_stroke(center, radius, Stroke::new(1.5, Color32::from_rgb(150, 164, 180)));
    painter.line_segment(
        [Pos2::new(center.x - radius, center.y), Pos2::new(center.x + radius, center.y)],
        ring,
    );
    painter.line_segment(
        [Pos2::new(center.x, center.y - radius), Pos2::new(center.x, center.y + radius)],
        ring,
    );
    let font = egui::FontId::monospace((label_px * 0.7).max(9.0));
    for swr in [1.5_f64, 2.0, 3.0] {
        let gamma = gamma_for_swr(swr) as f32;
        painter.circle_stroke(center, radius * gamma, ring);
        painter.text(
            Pos2::new(center.x + radius * gamma * 0.707 + 2.0, center.y - radius * gamma * 0.707),
            egui::Align2::LEFT_BOTTOM,
            format!("SWR {swr}"),
            font.clone(),
            palette::TEXT_MUTED,
        );
    }
    for (text, at, align) in [
        ("0°", Pos2::new(center.x + radius + 4.0, center.y), egui::Align2::LEFT_CENTER),
        ("180°", Pos2::new(center.x - radius - 4.0, center.y), egui::Align2::RIGHT_CENTER),
        ("+90°", Pos2::new(center.x, center.y - radius - 2.0), egui::Align2::CENTER_BOTTOM),
        ("−90°", Pos2::new(center.x, center.y + radius + 2.0), egui::Align2::CENTER_TOP),
        ("|Γ| = 1", Pos2::new(center.x - radius * 0.707 - 2.0, center.y - radius * 0.707), egui::Align2::RIGHT_BOTTOM),
    ] {
        painter.text(at, align, text, font.clone(), palette::TEXT_MUTED);
    }
}

fn stroke_polar(painter: &egui::Painter, plot: Rect, samples: &[Sample], color: Color32) {
    let center = plot.center();
    let radius = plot.width().min(plot.height()) * 0.45;
    let mut prev = None;
    for sample in samples {
        if !sample.s11.re.is_finite() || !sample.s11.im.is_finite() {
            prev = None;
            continue;
        }
        let mut re = sample.s11.re;
        let mut im = sample.s11.im;
        let mag = re.hypot(im);
        if mag > 1.0 {
            re /= mag;
            im /= mag;
        }
        let pos = Pos2::new(
            center.x + (re as f32) * radius,
            center.y - (im as f32) * radius,
        );
        if let Some(was) = prev {
            painter.line_segment([was, pos], Stroke::new(2.0, color));
        }
        prev = Some(pos);
    }
}

fn hit(view: View, rect: Rect, samples: &[Sample], pos: Pos2) -> Option<usize> {
    match view {
        View::Smith => {
            let disk = smith_disk(rect);
            nearest_gamma(samples, pos, disk.center, disk.rad)
        }
        View::Polar => {
            let plot = rect.shrink(16.0);
            let radius = plot.width().min(plot.height()) * 0.45;
            nearest_gamma(samples, pos, plot.center(), radius)
        }
        _ => {
            let plot = cart_plot(rect);
            if !plot.contains(pos) || samples.len() < 2 {
                return None;
            }
            let t = ((pos.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
            Some(((samples.len() - 1) as f32 * t).round() as usize)
        }
    }
}

fn nearest_gamma(samples: &[Sample], pos: Pos2, center: Pos2, radius: f32) -> Option<usize> {
    samples
        .iter()
        .enumerate()
        .filter(|(_, sample)| sample.s11.re.is_finite() && sample.s11.im.is_finite())
        .min_by(|(_, a), (_, b)| {
            let da = (center.x + a.s11.re as f32 * radius - pos.x)
                .hypot(center.y - a.s11.im as f32 * radius - pos.y);
            let db = (center.x + b.s11.re as f32 * radius - pos.x)
                .hypot(center.y - b.s11.im as f32 * radius - pos.y);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn header_flags_show_limit_verdict_with_readable_colours() {
        let pass = limits::Verdict { tested: 5, failing: vec![] };
        let fail = limits::Verdict { tested: 5, failing: vec![2] };
        let none = limits::Verdict { tested: 0, failing: vec![] };
        assert_eq!(header_verdict(&pass), ("LIMIT PASS".to_string(), PASS_TEXT));
        assert_eq!(header_verdict(&fail), ("LIMIT FAIL".to_string(), FAIL_TEXT));
        assert_eq!(header_verdict(&none).0, "LIMIT: NO POINTS");
        for bg in [palette::STRIP, rustcars::tint(palette::STRIP, -0.4)] {
            assert!(a11y::contrast(PASS_TEXT, bg) >= 4.5);
            assert!(a11y::contrast(FAIL_TEXT, bg) >= 4.5);
        }
    }

    #[test]
    fn analysis_marks_place_filter_and_notch_lines() {
        let xs: Vec<f64> = (0..201).map(|i| i as f64).collect();
        let notch_db: Vec<f64> = xs.iter().map(|x| -(30.0 - (x - 100.0).abs()).max(0.0)).collect();
        let m = analysis_marks(&xs, &notch_db, false, true);
        assert_eq!(m.len(), 3);
        assert_eq!(m[1], (100.0, "NOTCH"));
        assert!(analysis_marks(&xs, &notch_db, false, false).is_empty());
        let bp: Vec<f64> = xs.iter().map(|x| -((x - 100.0).abs() * 0.5)).collect();
        let f = analysis_marks(&xs, &bp, true, false);
        assert_eq!(f.len(), 5);
        assert!((f[0].0 - 94.0).abs() < 1e-6 && (f[2].0 - 106.0).abs() < 1e-6, "{f:?}");
    }

    #[test]
    fn the_polar_readout_gives_magnitude_angle_swr_and_return_loss() {
        let r = polar_readout(0.0, 0.5).unwrap();
        assert!((r.mag - 0.5).abs() < 1e-12 && (r.angle_deg - 90.0).abs() < 1e-9);
        assert!((r.swr - 3.0).abs() < 1e-9);
        assert!((r.rl_db - 6.0206).abs() < 1e-3);
        assert!(polar_readout(0.9, 0.9).is_none());
        assert!((gamma_for_swr(2.0) - 1.0 / 3.0).abs() < 1e-12);
        assert!((gamma_for_swr(1.5) - 0.2).abs() < 1e-12);
    }

    #[test]
    fn formats_are_offered_only_for_their_parameter_with_units() {
        for (view, _) in VIEWS {
            let (param, _, unit) = view_meta(view);
            assert_eq!(param == "S21", uses_s21(view));
            assert!(!unit.is_empty());
        }
        // SWR and impedance formats exist for S11 only.
        for v in [View::Swr, View::Impedance, View::Zabs, View::Smith, View::Tdr] {
            assert!(!uses_s21(v));
        }
        for v in [View::S21, View::Phase, View::Unwrap, View::GroupDelay, View::PolarS21, View::ReImS21] {
            assert!(uses_s21(v));
        }
        assert_eq!(view_meta(View::Zabs).2, "Ω");
        assert_eq!(view_meta(View::GroupDelay).2, "ns");
        assert_eq!(view_meta(View::ReturnLoss).2, "dB");
        // 100 + j50 on 50 Ω: |Z| 111.8 Ω and SWR 2.62.
        let z = Cx::new(100.0, 50.0);
        let g = z.sub(Cx::new(50.0, 0.0)).div(z.add(Cx::new(50.0, 0.0))).unwrap();
        let s = Sample { hz: 1e6, s11: g, s21: Cx::new(0.0, 1.0) };
        let (_, ys) = series(View::Zabs, &[s], 50.0, 0.66);
        assert!((ys[0] - 111.803).abs() < 1e-3);
        let (_, swr) = series(View::Swr, &[s], 50.0, 0.66);
        assert!((swr[0] - 2.618).abs() < 1e-3);
        let (_, ph) = series(View::Phase, &[s], 50.0, 0.66);
        assert!((ph[0] - 90.0).abs() < 1e-9);
    }

    #[cfg(unix)]
    #[test]
    fn a_vanished_device_node_reads_as_unplugged() {
        assert!(device::unplugged("/dev/hamspot-no-such-nanovna"));
        assert!(!device::unplugged("/dev/null"));
        assert!(!device::unplugged(""));
        assert!(!device::unplugged("COM3"));
    }

    /// WCAG 1.4.3 / 1.4.11: text 4.5:1, trace lines and marker glyphs 3:1. Issue 73.
    #[test]
    fn vna_colours_meet_wcag_contrast() {
        use a11y::contrast;
        let text = [("TEXT", palette::TEXT), ("TEXT_MUTED", palette::TEXT_MUTED), ("MATH_TEXT", MATH_TEXT), ("LOST_TEXT", LOST_TEXT)];
        for bg in [PANE_BG, palette::STRIP] {
            for (name, c) in text {
                assert!(contrast(c, bg) >= 4.5, "{name} {:.2}", contrast(c, bg));
            }
        }
        // Marker labels are text on the pane.
        for (i, c) in MARKER_COLORS.iter().enumerate() {
            assert!(contrast(*c, PANE_BG) >= 4.5, "marker {i} {:.2}", contrast(*c, PANE_BG));
        }
        let lines = TRACE_COLORS.iter().copied().chain([MEM_COLOR, IDEAL_COLOR, GRID]);
        for c in lines {
            assert!(contrast(c, PANE_BG) >= 3.0, "{c:?} {:.2}", contrast(c, PANE_BG));
        }
    }

    #[test]
    fn calibration_dates_round_trip_old_sets_load_and_format() {
        let mut set = one_standard();
        assert!(set.saved_at.is_none());
        let old = serde_json::to_string(&set).unwrap();
        assert!(!old.contains("saved_at"), "no field written when there is no date");
        let back: CalSet = serde_json::from_str(&old).unwrap();
        assert_eq!(back.saved_at, None);
        assert_eq!(cal_date_text(None), "no date");
        set.saved_at = Some("2026-10-25T22:40:17Z".into());
        let back: CalSet = serde_json::from_str(&serde_json::to_string(&set).unwrap()).unwrap();
        assert_eq!(back, set);
        assert_eq!(cal_date_text(back.saved_at.as_deref()), "2026-10-25T22:40Z");
        assert_eq!(cal_date_text(Some("2026-10-25T18:40:00-04:00")), "2026-10-25T22:40Z");
        let now = cal_stamp_now();
        assert!(chrono::DateTime::parse_from_rfc3339(&now).is_ok() && now.ends_with('Z'), "{now}");
    }

    #[test]
    fn every_format_has_a_unique_layout_id_and_the_default_panes_resolve() {
        let ids: Vec<String> = VIEWS.iter().map(|(v, _)| view_id(*v)).collect();
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[..i].contains(id), "duplicate {id}");
            assert!(view_from_id(id) == Some(VIEWS[i].0));
        }
        let four = layout::Layout::default_four();
        let views: Vec<View> = four.traces.iter().map(|t| view_from_id(&t.view).expect(&t.view)).collect();
        assert_eq!(views, vec![View::ReturnLoss, View::S21, View::Smith, View::GroupDelay]);
        let mut face = Face::default();
        face.layout = four;
        let back: Face = serde_json::from_str(&serde_json::to_string(&face).unwrap()).unwrap();
        assert_eq!(back.layout, face.layout);
    }

    #[test]
    fn an_older_nanovna_section_loads_with_every_new_field_defaulted() {
        let old = r#"{"port":"/dev/ttyACM0","start_hz":1000000.0,"stop_hz":30000000.0,"points":201}"#;
        let face: Face = serde_json::from_str(old).unwrap();
        assert!(face.scales.is_empty() && face.limits.is_empty());
        assert_eq!(face.gd_aperture, 4);
        assert!(validate(&face).is_ok());
        // A default face writes nothing new, so files without VNA settings stay unchanged.
        assert_eq!(serde_json::to_string(&Face::default().for_station()).unwrap(), "{}");
        let set: CalSet = serde_json::from_str(
            r#"{"id":"a","start_hz":1.0,"step_hz":1.0,"z0":50.0,"open":[],"short":[],"load":[]}"#,
        )
        .unwrap();
        assert!(set.name.is_empty());
    }

    #[test]
    fn a_manual_scale_is_saved_per_trace_format() {
        let mut face = Face::default();
        face.scales.insert(
            scale_key(View::ReturnLoss).into(),
            display::ManualScale { ref_value: 0.0, ref_pos: 10.0, per_div: 5.0 },
        );
        let text = serde_json::to_string(&face).unwrap();
        assert!(text.contains("RETURN LOSS"));
        let back: Face = serde_json::from_str(&text).unwrap();
        assert_eq!(back, face);
        assert!(back.scales.get(scale_key(View::Swr)).is_none());
        assert!(!serde_json::to_string(&Face::default()).unwrap().contains("scales"));
    }

    #[test]
    fn a_default_face_is_the_one_the_station_file_omits() {
        assert!(Face::default().is_default());
        assert!(validate(&Face::default()).is_ok());
    }

    #[test]
    fn a_calibration_with_uneven_standards_is_rejected() {
        let mut face = Face::default();
        face.cals.push(CalSet {
            id: "abc".into(),
            zone: String::new(),
            band: String::new(),
            name: String::new(),
            start_hz: 1e6,
            step_hz: 1e6,
            z0: 50.0,
            open: vec![[0.0, 0.0], [0.1, 0.0]],
            short: vec![[-1.0, 0.0]],
            load: vec![[0.0, 0.0], [0.0, 0.0]],
            thru: Vec::new(),
            isolation: Vec::new(),
            saved_at: None,
        });
        assert!(validate(&face).is_err());
    }

    fn dated(id: &str, zone: &str, at: Option<&str>) -> CalSet {
        let mut set = one_standard();
        set.id = id.into();
        set.zone = zone.into();
        set.band.clear();
        set.saved_at = at.map(str::to_string);
        set
    }

    #[test]
    fn pick_prefers_this_device_then_newest_saved_set() {
        let cals = vec![
            dated("aa", "HF", Some("2026-10-20T10:00:00Z")),
            dated("bb", "HF", Some("2026-10-26T22:48:35Z")),
            dated("cc", "HF", None),
            dated("aa", "VHF", Some("2026-10-27T00:00:00Z")),
        ];
        let hf = |s: &CalSet| s.zone == "HF";
        assert_eq!(pick_cal(&cals, Some("aa"), hf).unwrap().id, "aa");
        assert_eq!(pick_cal(&cals, Some("zz"), hf).unwrap().id, "bb");
        assert_eq!(pick_cal(&cals, None, hf).unwrap().id, "bb");
        assert!(pick_cal(&cals, Some("aa"), |s| s.zone == "UHF").is_none());
    }

        #[test]
    fn status_fit_keeps_everything_when_wide() {
        let segs = vec![("A".to_string(), 3), ("BB".to_string(), 1), ("C".to_string(), 2)];
        assert_eq!(fit_status(&segs, 100), "A  BB  C");
    }

    #[test]
    fn status_fit_drops_lowest_priority_first_and_never_overflows() {
        let segs = vec![
            ("CH1 START 2 MHz".to_string(), 3),
            ("AVG 1".to_string(), 1),
            ("CONT".to_string(), 2),
            ("CAL".to_string(), 2),
            ("⚠ CHECK USB".to_string(), u8::MAX),
        ];
        let full = fit_status(&segs, 1000);
        assert!(full.contains("AVG 1"));
        let t = fit_status(&segs, full.chars().count() - 1);
        assert!(!t.contains("AVG") && t.contains("CONT") && t.contains("CAL"));
        // CAL (rightmost of the tie) goes before CONT.
        let t = fit_status(&segs, "CH1 START 2 MHz  CONT  ⚠ CHECK USB".chars().count());
        assert_eq!(t, "CH1 START 2 MHz  CONT  ⚠ CHECK USB");
        let t = fit_status(&segs, 11);
        assert_eq!(t, "⚠ CHECK USB");
        for max in 0..60 {
            assert!(fit_status(&segs, max).chars().count() <= max, "max {max}");
        }
        assert_eq!(fit_status(&segs, 5), "⚠ CH…");
    }

    #[test]
    fn device_short_is_type_and_port_only() {
        let mut live = device::Live { connected: true, ..device::Live::default() };
        assert_eq!(device_short(&live, "/dev/ttyACM0"), "NANOVNA V2 · ttyACM0");
        live.connected = false;
        live.lost = true;
        assert_eq!(device_short(&live, "/dev/ttyACM0"), "DISCONNECTED");
    }

    #[test]
    fn status_leds_reflect_link_id_cal_and_warn_in_text() {
        let mut live = device::Live { connected: true, device_id: Some([0; 12]), id_reliable: false, ..device::Live::default() };
        let leds = status_leds(&live, cal::CalState::Exact, true, true);
        assert_eq!(leds[0].color, LED_OK);
        assert_eq!(leds[1].color, LED_BAD);
        assert!(leds[1].tip.contains("UNRELIABLE"));
        assert!(leds[2].tip.contains("restored"));
        assert_eq!(leds[3].color, LED_OK);
        live.id_reliable = true;
        assert_eq!(status_leds(&live, cal::CalState::Uncal, false, false)[1].color, LED_OK);
        assert_eq!(status_leds(&live, cal::CalState::Uncal, false, false)[2].color, LED_OFF);
        let lost = device::Live { lost: true, ..device::Live::default() };
        let leds = status_leds(&lost, cal::CalState::Uncal, false, true);
        assert_eq!(leds[0].color, LED_BAD);
        assert_eq!(leds[3].color, LED_OFF);
        assert!(leds.iter().all(|l| !l.tip.is_empty()));
    }

    #[test]
    fn device_mismatch_and_restored_flags() {
        let set = dated("20b381000000000000000000", "HF", Some("2026-10-26T22:48:35Z"));
        assert!(other_device(&set, Some("808b08000000000000000000")));
        assert!(!other_device(&set, Some("20b381000000000000000000")));
        // An all-zero id says nothing about the device.
        assert!(!other_device(&set, Some("000000000000000000000000")));
        assert!(!other_device(&dated("000000000000000000000000", "HF", None), Some("808b08000000000000000000")));
        assert!(!other_device(&set, None));
        assert!(restored(&set, "2026-10-26T23:40:00Z"));
        assert!(!restored(&set, "2026-10-26T22:00:00Z"));
        assert!(restored(&dated("aa", "HF", None), "2026-10-26T22:00:00Z"));
    }

    #[test]
    fn vna_file_round_trip_corrupt_file_and_old_station() {
        let dir = std::env::temp_dir().join(format!("hamspot-vna-persist-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let station = dir.join("SAFE-T.HSCommander");
        let path = path_beside(&station);
        // Missing file: empty, nothing written.
        let mut face = Face::default();
        let loaded = load_beside(&station, &mut face).unwrap();
        assert!(loaded.archive.cals.is_empty() && loaded.note.is_none() && !path.exists());
        // Round trip keeps saved_at; no temp file is left behind.
        let mut archive = VnaFile::empty();
        archive.cals.push(dated("aa", "HF", Some("2026-10-26T22:48:35Z")));
        save_file(&path, &archive).unwrap();
        assert!(!path.with_extension("HSVNA.tmp").exists());
        let back = load_beside(&station, &mut face).unwrap();
        assert_eq!(back.archive.cals, archive.cals);
        assert_eq!(back.archive.cals[0].saved_at.as_deref(), Some("2026-10-26T22:48:35Z"));
        // Corrupt file: moved aside, never overwritten, load still succeeds.
        fs::write(&path, "{ not json").unwrap();
        let bad = load_beside(&station, &mut face).unwrap();
        assert!(bad.archive.cals.is_empty());
        assert!(bad.note.as_deref().unwrap().contains("corrupt"));
        assert!(!path.exists());
        let aside: Vec<_> = fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().contains("corrupt")).collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(fs::read_to_string(aside[0].path()).unwrap(), "{ not json");
        // An old station with no nanovna fields still loads with defaults.
        let old: Face = serde_json::from_str("{}").unwrap();
        assert!(old.is_default());
        let _ = fs::remove_dir_all(&dir);
    }

    fn one_standard() -> CalSet {
        CalSet {
            id: "abc".into(),
            zone: String::new(),
            band: String::new(),
            name: String::new(),
            start_hz: 1e6,
            step_hz: 1e6,
            z0: 50.0,
            open: vec![[1.0, 0.0], [0.9, 0.1]],
            short: Vec::new(),
            load: Vec::new(),
            thru: Vec::new(),
            isolation: Vec::new(),
            saved_at: None,
        }
    }

    #[test]
    fn a_zone_calibration_matches_that_tabs_limits() {
        let mut complete = one_standard();
        complete.zone = "VHF".into();
        complete.band.clear();
        complete.start_hz = 25_000_000.0;
        complete.step_hz = 250_000.0;
        complete.short = vec![[-1.0, 0.0], [-0.9, 0.0]];
        complete.load = vec![[0.0, 0.0], [0.0, 0.0]];
        let hi = 25_250_000.0;
        let mut other = complete.clone();
        other.id = "zzz".into();
        let cals = vec![other, complete];
        let found = find_zone_cal(&cals, "abc", "VHF", 25_000_000.0, hi).unwrap();
        assert_eq!(found.id, "abc");
        // Another id falls back to the newest matching set (the V2.2 id is not stable).
        assert!(find_zone_cal(&cals, "missing", "VHF", 25_000_000.0, hi).is_some());
        assert!(find_zone_cal(&cals, "abc", "HF", 25_000_000.0, hi).is_none());
        assert!(find_zone_cal(&cals, "abc", "VHF", 30_000_000.0, 50_000_000.0).is_none());
        assert!(find_zone_cal(&[one_standard()], "abc", "", 1e6, 2e6).is_none());
        assert_eq!(WizStep::Open.action("HF"), "Measure open");
        assert_eq!(WizStep::Short.action("HF"), "Measure short");
        assert_eq!(WizStep::Load.action("HF"), "Measure load");
        assert_eq!(WizStep::Thru.action("HF"), "Measure cable");
        assert_eq!(WizStep::Isol.action("HF"), "Measure both");
        assert_eq!(WizStep::Finish.action("HF"), "Scan HF");
        assert!(!WizStep::Open.instruction().contains("standard"));
        assert!(!WizStep::Thru.instruction().contains("standard"));
        assert!(WizStep::Open.instruction().contains("S11"));
        assert!(WizStep::Open.instruction().contains("S21"));
        assert!(WizStep::Thru.instruction().contains("S11"));
        assert!(WizStep::Thru.instruction().contains("S21"));
        assert!(WizStep::Isol.instruction().contains("S21"));
        assert_eq!(next_batch(WizStep::Open, 0, 5), (WizStep::Open, 1));
        assert_eq!(next_batch(WizStep::Open, 4, 5), (WizStep::Short, 0));
        assert_eq!(next_batch(WizStep::Load, 4, 5), (WizStep::Thru, 0));
        assert_eq!(next_batch(WizStep::Short, 0, 1), (WizStep::Load, 0));
        assert_eq!(step_summary(None, WizStep::Open), "not stored");
        let stored = one_standard();
        assert!(step_summary(Some(&stored), WizStep::Open).starts_with("2 pts"));
        assert_eq!(step_summary(Some(&stored), WizStep::Load), "not stored");
        let old = r#"{"id":"abc","start_hz":1000000.0,"step_hz":1000000.0,"z0":50.0,"open":[[1.0,0.0],[0.9,0.1]],"short":[],"load":[]}"#;
        let set: CalSet = serde_json::from_str(old).unwrap();
        assert!(set.zone.is_empty());
        assert!(set.band.is_empty());
        let text = serde_json::to_string(&set).unwrap();
        assert!(!text.contains("\"zone\""));
        assert!(!text.contains("\"band\""));
    }

    #[test]
    fn a_partial_calibration_is_kept_and_a_backup_round_trips() {
        let mut face = Face::default();
        face.cals.push(one_standard());
        assert!(validate(&face).is_ok());
        let text = encode_backup(&face.cals).unwrap();
        let back = decode_backup(&text).unwrap();
        assert_eq!(back, face.cals);
        assert!(decode_backup("{\"version\":2,\"cals\":[]}").is_err());
        assert!(decode_backup("{\"version\":1,\"cals\":[],\"extra\":1}").is_err());
    }

    #[test]
    fn a_smith_trace_stops_at_the_rim() {
        let inside = clip_gamma_segment((0.0, 0.0), (0.5, 0.0)).unwrap();
        assert!((inside.0.0).abs() < 1e-9 && (inside.1.0 - 0.5).abs() < 1e-9);
        let rim = clip_gamma_segment((0.0, 0.0), (2.0, 0.0)).unwrap();
        assert!((rim.1.0 - 1.0).abs() < 1e-6);
        assert!(clip_gamma_segment((2.0, 0.0), (3.0, 0.0)).is_none());
    }

    fn at_gamma(re: f64) -> Sample {
        Sample {
            hz: 1.0,
            s11: Cx::new(re, 0.0),
            s21: Cx::ZERO,
        }
    }

    #[test]
    fn a_smith_click_uses_the_labeled_disk() {
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(800.0, 800.0));
        let disk = smith_disk(rect);
        assert!((disk.rad - 370.0).abs() < 0.01);
        let samples = [at_gamma(0.0), at_gamma(1.0)];
        // Midway for the Smith disk is past the polar-chart boundary, so the
        // two views pick different samples.
        let between = Pos2::new(578.0, 400.0);
        assert_eq!(hit(View::Smith, rect, &samples, between), Some(0));
        assert_eq!(hit(View::Polar, rect, &samples, between), Some(1));
        let open = math_z_at_open();
        assert!(open.contains("OPEN"));
    }

    fn math_z_at_open() -> String {
        fmt_cx(crate::tools::smith::math::z_of(
            crate::tools::smith::math::cx(1.0, 0.0),
        ))
    }

    #[test]
    fn a_zone_click_loads_a_stored_scan_and_a_double_click_measures() {
        assert_eq!(zone_click(true, false), ZoneClick::Recall);
        assert_eq!(zone_click(false, false), ZoneClick::Measure);
        assert_eq!(zone_click(true, true), ZoneClick::Measure);
        assert_eq!(zone_click(false, true), ZoneClick::Measure);
    }

    #[test]
    fn calibration_leaves_the_station_face_and_round_trips_beside_it() {
        let dir = std::env::temp_dir().join(format!("hamspot-vna-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let station = dir.join("City.HSCommander");
        let mut face = Face::default();
        face.port = "/dev/ttyACM0".into();
        face.cals.push(one_standard());
        let loaded = load_beside(&station, &mut face).unwrap();
        assert!(face.cals.is_empty());
        assert_eq!(face.port, "/dev/ttyACM0");
        assert!(loaded.stripped_station);
        assert_eq!(loaded.archive.cals.len(), 1);
        let path = path_beside(&station);
        assert_eq!(path.file_name().unwrap(), "City.HSVNA");
        assert!(path.is_file());
        let again = load_beside(&station, &mut face).unwrap();
        assert!(!again.stripped_station);
        assert_eq!(again.archive.cals, loaded.archive.cals);
        let mut archive = again.archive;
        archive.scans.push(StoredScan {
            id: "abc".into(),
            zone: "HF".into(),
            start_hz: 1_000_000.0,
            step_hz: 1_000_000.0,
            s11: vec![[1.0, 0.0], [0.5, 0.0]],
            s21: vec![[0.0, 0.0], [0.0, 0.1]],
        });
        save_file(&path, &archive).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"scans\""));
        assert!(!text.contains("\"port\""));
        let back = load_file(&path).unwrap();
        assert_eq!(back.scans[0].zone, "HF");
        let samples = samples_of(&back.scans[0]);
        assert_eq!(samples.len(), 2);
        assert!((samples[1].hz - 2_000_000.0).abs() < 1.0);
        let _ = fs::remove_dir_all(&dir);
    }
}
