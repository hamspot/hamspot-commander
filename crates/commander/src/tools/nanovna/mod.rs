//! NanoVNA V2 on the Tools tab. Issue 59.
//! The device is optional. Nothing is polled until Connect.

mod device;
mod display;
mod proto;
mod rf;

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Smith,
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
}

const VIEWS: [(View, &str); 12] = [
    (View::Smith, "Smith"),
    (View::Polar, "Polar"),
    (View::Swr, "SWR"),
    (View::ReturnLoss, "Return loss"),
    (View::LinMag, "Lin mag"),
    (View::ReIm, "Real + imag"),
    (View::Impedance, "R + X"),
    (View::S21, "S21 dB"),
    (View::Phase, "Phase"),
    (View::Unwrap, "Unwrap"),
    (View::GroupDelay, "Group delay"),
    (View::Tdr, "TDR"),
];

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
    fs::write(path, text).map_err(|err| format!("{}: {err}", path.display()))
}

/// Read the VNA file. An older station that still carries `cals` donates
/// them when the VNA file has none. `stripped_station` means those `cals`
/// were removed from `face` and the station should be written again.
pub struct LoadedVna {
    pub archive: VnaFile,
    pub stripped_station: bool,
}

pub fn load_beside(station: &Path, face: &mut Face) -> Result<LoadedVna, String> {
    let path = path_beside(station);
    let mut archive = load_file(&path)?;
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
    run_state: Run,
    markers: [Option<usize>; 4],
    marker_on: usize,
    memory: Option<Vec<Sample>>,
    show_memory: bool,
    show_diff: bool,
    admittance: bool,
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
    last_sweep: Option<std::time::Instant>,
    /// A stored calibration standard shown in its own window. Issue 75.
    cal_graph: Option<(CalSet, WizStep)>,
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
            run_state: Run::Idle,
            markers: [None; 4],
            marker_on: 0,
            memory: None,
            show_memory: false,
            show_diff: false,
            admittance: false,
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
            last_sweep: None,
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
        self.history.clear();
        self.memory = None;
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
        if live.generation != self.seen && !live.sweep.is_empty() {
            self.ingest(&live.sweep);
            self.seen = live.generation;
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
        self.chart(ui, &theme, &live);
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
                    self.memory = Some(trace);
                    self.show_memory = true;
                }
            }
            if action_button(ui, theme, "Mem on", vec2(80.0, 32.0)).clicked() {
                self.show_memory = !self.show_memory;
            }
            if action_button(ui, theme, "Diff", vec2(70.0, 32.0)).clicked() {
                self.show_diff = !self.show_diff;
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
        let Some(index) = self.markers[self.marker_on] else {
            return self.cal_label(live);
        };
        let Some(sample) = samples.get(index) else {
            return self.cal_label(live);
        };
        let d = rf::derive(sample, self.z0_value());
        format!(
            "M{}  {} Hz  SWR {}  RL {} dB  |Z| {} Ω  {} {:+}j Ω  Q {}  S21 {} dB  series {}  parallel {}",
            self.marker_on + 1,
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
        let trace = self.display_trace(live);
        let memory = if self.show_memory {
            self.memory.clone()
        } else {
            None
        };
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
        let mut clicked = None;
        let placed: Vec<(usize, usize)> = self
            .markers
            .iter()
            .enumerate()
            .filter_map(|(slot, at)| at.map(|at| (slot, at)))
            .collect();
        let cal = trace.as_deref().and_then(|t| self.matched_cal(t)).cloned();
        show_frame(ui, theme, right, Side::Right, |ui| {
            self.trace_header(ui, theme, trace.as_deref(), cal.as_ref());
            let table_h = if placed.is_empty() { 0.0 } else { 22.0 * (placed.len() as f32 + 1.0) + 6.0 };
            let status_h = 26.0;
            let chart_h = (ui.available_height() - table_h - status_h - 8.0).max(120.0);
            ui.allocate_ui(vec2(ui.available_width(), chart_h), |ui| {
                let (hit, _) = paint_chart(
                    ui,
                    self.view,
                    trace.as_deref(),
                    memory.as_deref(),
                    self.show_diff,
                    self.z0_value(),
                    self.velocity_value(),
                    self.admittance,
                    theme.data_size,
                    &placed,
                );
                clicked = hit;
            });
            if let Some(samples) = trace.as_deref() {
                self.marker_table(ui, theme, samples, &placed);
            }
            self.status_bar(ui, theme, live, trace.as_deref(), cal.as_ref());
        });
        if let Some(index) = clicked {
            self.markers[self.marker_on] = Some(index);
        }
    }

    /// The R&S trace-list line over the pane: name, parameter chip, format, scale, channel, cal.
    fn trace_header(&self, ui: &mut Ui, theme: &LcarsTheme, samples: Option<&[Sample]>, cal: Option<&CalSet>) {
        let (param, format, unit) = view_meta(self.view);
        let scale = match (self.view, samples) {
            (View::Smith | View::Polar, _) | (_, None) => None,
            (view, Some(samples)) => {
                let (_, mut ys) = series(view, samples, self.z0_value(), self.velocity_value());
                ys.extend(second_series(view, samples, self.z0_value()));
                Some(display::auto_scale(&ys, 10))
            }
        };
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, palette::STRIP);
        let font = egui::FontId::monospace(theme.data_size * 0.9);
        let mut x = rect.left() + 8.0;
        let put = |x: &mut f32, text: &str, color: Color32| {
            let g = painter.layout_no_wrap(text.to_string(), font.clone(), color);
            let w = g.size().x;
            painter.galley(Pos2::new(*x, rect.center().y - g.size().y / 2.0), g, color);
            *x += w + 14.0;
        };
        put(&mut x, "TRC1", palette::TEXT);
        // Parameter chip in the trace colour with dark ink.
        let g = painter.layout_no_wrap(param.to_string(), font.clone(), rustcars::ink_on(TRACE1));
        let chip = Rect::from_min_size(
            Pos2::new(x, rect.top() + 3.0),
            vec2(g.size().x + 12.0, rect.height() - 6.0),
        );
        painter.rect_filled(chip, 4.0, TRACE1);
        painter.galley(
            Pos2::new(chip.left() + 6.0, chip.center().y - g.size().y / 2.0),
            g,
            rustcars::ink_on(TRACE1),
        );
        x = chip.right() + 14.0;
        put(&mut x, format, palette::TEXT);
        if let Some(scale) = scale {
            let ref_value = if scale.top >= 0.0 && scale.bottom() <= 0.0 { 0.0 } else { scale.top };
            put(&mut x, &format!("{} {unit}/", display::fmt_tick(scale.per_div, scale.per_div / 10.0)), palette::TEXT);
            put(&mut x, &format!("REF {} {unit}", display::fmt_tick(ref_value, scale.per_div)), palette::TEXT);
        }
        put(&mut x, "CH1", palette::TEXT_MUTED);
        if self.show_memory && self.memory.is_some() {
            put(&mut x, "MEM", palette::TEXT_MUTED);
        }
        let (badge, fill) = match cal {
            Some(set) if set.thru.is_empty() => ("CAL OSL".to_string(), palette::OLIVE),
            Some(_) => ("CAL OSL+THRU".to_string(), palette::OLIVE),
            None if samples.is_some() => ("CAL ON".to_string(), palette::OLIVE),
            None => ("UNCAL".to_string(), palette::BRICK),
        };
        let g = painter.layout_no_wrap(badge, font.clone(), rustcars::ink_on(fill));
        let chip = Rect::from_min_size(
            Pos2::new(rect.right() - g.size().x - 20.0, rect.top() + 3.0),
            vec2(g.size().x + 12.0, rect.height() - 6.0),
        );
        if chip.left() > x {
            painter.rect_filled(chip, 4.0, fill);
            painter.galley(
                Pos2::new(chip.left() + 6.0, chip.center().y - g.size().y / 2.0),
                g,
                rustcars::ink_on(fill),
            );
        }
    }

    /// One row per placed marker under the chart. Issue 68.
    fn marker_table(&self, ui: &mut Ui, theme: &LcarsTheme, samples: &[Sample], placed: &[(usize, usize)]) {
        if placed.is_empty() {
            return;
        }
        let z0 = self.z0_value();
        let (_, ys) = series(self.view, samples, z0, self.velocity_value());
        let (_, _, unit) = view_meta(self.view);
        let font = egui::FontId::monospace(theme.data_size * 0.85);
        let row = |ui: &mut Ui, cells: [String; 5], color: Color32, lit: bool| {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
            let painter = ui.painter_at(rect);
            if lit {
                painter.rect_filled(rect, 0.0, palette::STRIP);
            }
            let widths = [0.07, 0.2, 0.25, 0.28, 0.2];
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
            ["MKR".into(), "STIMULUS".into(), "VALUE".into(), "Z = R + jX".into(), "SWR".into()],
            palette::TEXT_MUTED,
            false,
        );
        for (slot, index) in placed {
            let Some(sample) = samples.get(*index) else {
                continue;
            };
            let value = match self.view {
                View::GroupDelay | View::Unwrap => ys
                    .get(*index)
                    .map(|v| format!("{} {unit}", num(*v)))
                    .unwrap_or_default(),
                View::Tdr => "-".into(),
                view => marker_value(view, sample, z0),
            };
            let d = rf::derive(sample, z0);
            row(
                ui,
                [
                    format!("M{}", slot + 1),
                    display::fmt_hz(sample.hz),
                    value,
                    format!("{} {:+}j Ω", num(d.r), num(d.x)),
                    format!("{} : 1", num(rf::swr(sample.s11))),
                ],
                MARKER_COLORS[*slot % MARKER_COLORS.len()],
                *slot == self.marker_on,
            );
        }
    }

    /// The channel and instrument line along the bottom. Issues 65 and 71.
    fn status_bar(&self, ui: &mut Ui, theme: &LcarsTheme, live: &device::Live, samples: Option<&[Sample]>, cal: Option<&CalSet>) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, palette::STRIP);
        let (start, stop, points) = match samples {
            Some(s) if !s.is_empty() => (s[0].hz, s[s.len() - 1].hz, s.len()),
            _ => (self.face.start_hz, self.face.stop_hz, self.face.points as usize),
        };
        let avg = self.host_average_value().max(1);
        let mut left = format!(
            "CH1  START {}  STOP {}  {} PTS  AVG {}",
            display::fmt_hz(start),
            display::fmt_hz(stop),
            points,
            avg,
        );
        if let Some(set) = cal {
            if !set.zone.is_empty() {
                left.push_str(&format!("  CAL {}", set.zone));
            }
        }
        let device = if live.connected {
            let fw = live
                .identity
                .as_ref()
                .map(|id| format!("  FW {}.{}", id.fw_major, id.fw_minor))
                .unwrap_or_default();
            format!("NANOVNA V2 · {}{fw}", self.face.port)
        } else {
            "NOT CONNECTED".to_string()
        };
        let time = self
            .sweep_ms
            .map(|ms| format!(" · {} ms/SWEEP", ms.round()))
            .unwrap_or_default();
        let font = egui::FontId::monospace(theme.data_size * 0.8);
        painter.text(
            Pos2::new(rect.left() + 8.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            left,
            font.clone(),
            palette::TEXT,
        );
        painter.text(
            Pos2::new(rect.right() - 8.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            format!("{device}{time}"),
            font,
            if live.connected { palette::TEXT } else { palette::TEXT_MUTED },
        );
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
        let mut toggle = false;
        let mut setup = false;
        ui.horizontal(|ui| {
            scan = softkey(ui, theme, "Scan", false, col4 * 2.0 + 8.0);
            let run_label = if self.run { "Stop" } else { "Run" };
            toggle = softkey(ui, theme, run_label, self.run, col4);
            setup = softkey(ui, theme, "Setup", self.setup_open, col4);
        });
        if scan {
            self.begin_scan(live);
        }
        if toggle {
            if self.run {
                self.run = false;
                self.run_state = Run::Idle;
            } else if self.wizard.is_some() {
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
        if setup {
            self.ports = device::list_ports();
            self.setup_open = true;
        }

        key_caption(ui, theme, "FORMAT", width);
        for triple in VIEWS.chunks(3) {
            ui.horizontal(|ui| {
                for (view, label) in triple {
                    if softkey(ui, theme, label, self.view == *view, col3) {
                        self.view = *view;
                    }
                }
            });
        }

        key_caption(ui, theme, "DISPLAY", width);
        ui.horizontal(|ui| {
            let smith = self.view == View::Smith;
            if softkey_enabled(ui, theme, "Y adm", self.admittance && smith, col4, smith) {
                self.admittance = !self.admittance;
            }
            if softkey(ui, theme, "Store", false, col4) {
                if let Some(trace) = samples {
                    self.memory = Some(trace.to_vec());
                    self.show_memory = true;
                }
            }
            let has_mem = self.memory.is_some();
            if softkey_enabled(ui, theme, "Mem", self.show_memory && has_mem, col4, has_mem) {
                self.show_memory = !self.show_memory;
            }
            if softkey_enabled(ui, theme, "Diff", self.show_diff && has_mem, col4, has_mem) {
                self.show_diff = !self.show_diff;
            }
        });

        key_caption(ui, theme, "MARKER", width);
        ui.horizontal(|ui| {
            for slot in 0..4 {
                let title = format!("M{}", slot + 1);
                if softkey(ui, theme, &title, self.marker_on == slot, col4) {
                    self.marker_on = slot;
                }
            }
        });
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
        let at = self.markers[self.marker_on];
        let found = match code {
            0 => display::search_max(&ys),
            1 => display::search_min(&ys),
            _ => display::next_peak(&ys, at),
        };
        match found {
            Some(index) => self.markers[self.marker_on] = Some(index),
            None => self.note = "No further peak on this trace.".into(),
        }
    }

    /// Gamma, SWR, return loss, mismatch loss, and impedance for the selected marker.
    fn marker_block(&self, ui: &mut Ui, theme: &LcarsTheme, samples: Option<&[Sample]>) {
        let Some(samples) = samples else {
            readout(ui, theme, "No calibrated sweep yet.", false);
            return;
        };
        let Some(index) = self.markers[self.marker_on] else {
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
            &format!("M{}  {} MHz", self.marker_on + 1, fmt_g(sample.hz / 1.0e6)),
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
        if let (Some(a), Some(b)) = (self.markers[0], self.markers[1]) {
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
        } else {
            let (zone, name) = band.clone().unwrap_or_default();
            let mut set = CalSet {
                id,
                zone,
                band: name,
                start_hz: start,
                step_hz: step,
                z0,
                open: Vec::new(),
                short: Vec::new(),
                load: Vec::new(),
                thru: Vec::new(),
                isolation: Vec::new(),
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
        self.face.cals.iter().any(|set| {
            set.id == id
                && cal_complete(set)
                && set.open.len() == points as usize
                && same_grid(set, start as f64, step, points as usize)
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
            self.note = format!("Scanning {zone} with the stored calibration.");
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
        if !self.note.is_empty() {
            ui.label(
                egui::RichText::new(&self.note)
                    .monospace()
                    .size(theme.data_size)
                    .color(palette::TEXT_MUTED),
            );
        }
        if action_button(ui, theme, "Rescan all", vec2(140.0, 32.0)).clicked() {
            let spans = self.all_zone_spans();
            self.start_wizard(live, spans, WizStep::Open, false);
        }
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
                    for step in [WizStep::Open, WizStep::Short, WizStep::Load, WizStep::Thru, WizStep::Isol]
                    {
                        let summary = step_summary(stored.as_ref(), step);
                        ui.horizontal(|ui| {
                            ui.set_min_height(super::LIST_ROW);
                            let has = stored
                                .as_ref()
                                .is_some_and(|set| !step_samples(set, step).is_empty());
                            if rustcars::lcars_button(
                                ui,
                                theme,
                                "Graph",
                                palette::SLATE,
                                vec2(64.0, 26.0),
                                rustcars::Cap::None,
                                has,
                            )
                            .on_hover_text("View graph of this stored standard")
                            .clicked()
                            {
                                if let Some(set) = stored.clone() {
                                    self.cal_graph = Some((set, step));
                                }
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

    /// The raw curve of one stored standard: magnitude against frequency and the polar locus.
    fn cal_graph_dialog(&mut self, ui: &mut Ui, theme: &LcarsTheme) {
        let Some((set, step)) = self.cal_graph.clone() else {
            return;
        };
        let rows = step_samples(&set, step);
        let xs: Vec<f64> = (0..rows.len()).map(|i| set.start_hz + set.step_hz * i as f64).collect();
        let mag: Vec<f64> = rows.iter().map(|r| rf::db(pair(*r))).collect();
        let pts: Vec<Sample> = rows
            .iter()
            .zip(&xs)
            .map(|(r, hz)| Sample { hz: *hz, s11: pair(*r), s21: Cx::new(0.0, 0.0) })
            .collect();
        let mut open = true;
        let title = format!(
            "{} · {} · {}",
            if set.zone.is_empty() { "Calibration" } else { set.zone.as_str() },
            step.step_name(),
            "raw standard",
        );
        egui::Window::new(title)
            .open(&mut open)
            .resizable(true)
            .default_size(vec2(720.0, 420.0))
            .show(ui.ctx(), |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} points · {} to {} · Z0 {} Ω · uncorrected ratio as stored",
                        rows.len(),
                        display::fmt_hz(xs.first().copied().unwrap_or(0.0)),
                        display::fmt_hz(xs.last().copied().unwrap_or(0.0)),
                        trim_num(set.z0),
                    ))
                    .monospace()
                    .size(theme.data_size * 0.85)
                    .color(palette::TEXT_MUTED),
                );
                ui.label(
                    egui::RichText::new("Magnitude dB (gold) · Phase on the polar plot")
                        .size(theme.data_size * 0.8)
                        .color(palette::TEXT_MUTED),
                );
                let h = (ui.available_height() - 4.0).max(220.0);
                let w = ui.available_width();
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2((w * 0.6).max(240.0), h), Sense::hover());
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, 0.0, PANE_BG);
                    paint_cartesian(&painter, rect, View::S21, &xs, &[(&mag, TRACE1)], &[], theme.data_size);
                    let (rect, _) = ui.allocate_exact_size(vec2((w * 0.38).max(160.0), h), Sense::hover());
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, 0.0, PANE_BG);
                    let plot = rect.shrink(16.0);
                    paint_polar(&painter, plot);
                    stroke_polar(&painter, plot, &pts, TRACE1);
                });
            });
        if !open {
            self.cal_graph = None;
        }
    }

    fn zone_cal(&self, id: Option<&str>, zone: &str) -> Option<&CalSet> {
        self.face.cals.iter().rev().find(|set| {
            set.zone == zone
                && set.band.is_empty()
                && id.is_none_or(|id| set.id == id)
        })
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
        if self.matched_cal_for(&samples, stored_id.as_deref()).is_none() && self.wizard.is_none() {
            return None;
        }
        if let Some(set) = self.matched_cal_for(&samples, stored_id.as_deref()) {
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
        self.face.cals.iter().find(|set| {
            set.id == id
                && set.open.len() == samples.len()
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
        if self.matched_cal(samples).is_some() {
            "Calibrated for this device and this sweep.".into()
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

fn find_zone_cal<'a>(cals: &'a [CalSet], id: &str, zone: &str, lo: f64, hi: f64) -> Option<&'a CalSet> {
    cals.iter().rev().find(|set| {
        set.id == id
            && set.zone == zone
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
        View::Phase => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| s.s21.arg().to_degrees()).collect(),
        ),
        View::Unwrap => {
            let phase = rf::unwrap_phase(&samples.iter().map(|s| s.s21).collect::<Vec<_>>());
            (
                samples.iter().map(|s| s.hz).collect(),
                phase.into_iter().map(|rad| rad.to_degrees()).collect(),
            )
        }
        View::GroupDelay => {
            let phase = rf::unwrap_phase(&samples.iter().map(|s| s.s21).collect::<Vec<_>>());
            let hz: Vec<f64> = samples.iter().map(|s| s.hz).collect();
            let delay = rf::group_delay(&hz, &phase);
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
        View::LinMag => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| s.s11.abs()).collect(),
        ),
        View::ReIm => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| s.s11.re).collect(),
        ),
        View::Smith | View::Polar => (
            samples.iter().map(|s| s.hz).collect(),
            samples.iter().map(|s| s.s11.abs()).collect(),
        ),
    }
}

/// The second line some formats draw: X beside R, imaginary beside real.
fn second_series(view: View, samples: &[Sample], z0: f64) -> Vec<f64> {
    match view {
        View::Impedance => samples.iter().map(|s| rf::derive(s, z0).x).collect(),
        View::ReIm => samples.iter().map(|s| s.s11.im).collect(),
        _ => Vec::new(),
    }
}

/// Measured parameter, format name, and unit for the trace header. Issue 63.
fn view_meta(view: View) -> (&'static str, &'static str, &'static str) {
    match view {
        View::Smith => ("S11", "SMITH", "U"),
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
    }
}

/// Primary marker value in the trace's own format.
fn marker_value(view: View, sample: &Sample, z0: f64) -> String {
    let (_, _, unit) = view_meta(view);
    match view {
        View::Smith | View::Impedance => {
            let d = rf::derive(sample, z0);
            format!("{} {:+}j Ω", num(d.r), num(d.x))
        }
        View::Polar => format!(
            "{} ∠ {}°",
            fmt_g(sample.s11.abs()),
            fmt_g(sample.s11.arg().to_degrees())
        ),
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

const MARKER_COLORS: [Color32; 4] = [
    Color32::from_rgb(0xFF, 0x99, 0x00),
    Color32::from_rgb(0x99, 0xCC, 0xFF),
    Color32::from_rgb(0xCC, 0x99, 0xCC),
    Color32::from_rgb(0x99, 0xDD, 0x88),
];
const GRID: Color32 = Color32::from_rgb(0x4A, 0x55, 0x66);
const GRID_FAINT: Color32 = Color32::from_rgb(0x2C, 0x33, 0x3D);
const TRACE1: Color32 = Color32::from_rgb(0xF2, 0xC2, 0x6B);
const TRACE2: Color32 = Color32::from_rgb(0x78, 0xAA, 0xD2);
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
) {
    let plot = cart_plot(rect);
    let font = egui::FontId::monospace((label_px * 0.8).max(10.0));
    let mut all = Vec::new();
    for (ys, _) in traces {
        all.extend_from_slice(ys);
    }
    let scale = display::auto_scale(&all, 10);
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
    let ref_value = if scale.top >= 0.0 && scale.bottom() <= 0.0 { 0.0 } else { scale.top };
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
        let mut prev = None;
        for (x, y) in xs.iter().zip(ys.iter()) {
            if !y.is_finite() {
                prev = None;
                continue;
            }
            let pos = map(*x, *y);
            if let Some(was) = prev {
                clip.line_segment([was, pos], Stroke::new(2.0, *color));
            }
            prev = Some(pos);
        }
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
        format!("{}", slot + 1),
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
fn softkey(ui: &mut Ui, theme: &LcarsTheme, label: &str, lit: bool, width: f32) -> bool {
    softkey_enabled(ui, theme, label, lit, width, true)
}

fn softkey_enabled(ui: &mut Ui, theme: &LcarsTheme, label: &str, lit: bool, width: f32, enabled: bool) -> bool {
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
    enabled && resp.clicked()
}

/// Height of one key cap in the left key panel.
const KEY_H: f32 = 32.0;

fn paint_chart(
    ui: &mut Ui,
    view: View,
    samples: Option<&[Sample]>,
    memory: Option<&[Sample]>,
    diff: bool,
    z0: f64,
    velocity: f64,
    admittance: bool,
    label_px: f32,
    markers: &[(usize, usize)],
) -> (Option<usize>, Option<String>) {
    let size = vec2(
        ui.available_width().max(240.0),
        ui.available_height().max(160.0),
    );
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
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
            paint_polar(&painter, plot);
            if let Some(samples) = samples {
                stroke_polar(&painter, plot, samples, palette::TAN);
            }
            if let Some(memory) = memory {
                stroke_polar(&painter, plot, memory, palette::TEAL);
            }
            None
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
            let delta: Vec<f64> = if diff && my.len() == ys.len() {
                ys.iter().zip(&my).map(|(a, b)| a - b).collect()
            } else {
                Vec::new()
            };
            let mut traces: Vec<(&[f64], Color32)> = vec![(&ys, TRACE1)];
            if !second.is_empty() {
                traces.push((&second, TRACE2));
            }
            if my.len() == ys.len() && mx.len() == xs.len() && !my.is_empty() {
                traces.push((&my, palette::TEAL));
            }
            if !delta.is_empty() {
                traces.push((&delta, palette::BRICK));
            }
            paint_cartesian(&painter, rect, view, &xs, &traces, markers, label_px);
            None
        }
    };
    if matches!(view, View::Smith | View::Polar) {
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
    let clicked = if resp.clicked() {
        samples.and_then(|samples| hit(view, rect, samples, resp.interact_pointer_pos()?))
    } else {
        None
    };
    if let Some(text) = &hover {
        resp.on_hover_text(text);
    }
    (clicked, hover)
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

fn paint_polar(painter: &egui::Painter, plot: Rect) {
    let center = plot.center();
    let radius = plot.width().min(plot.height()) * 0.45;
    let ring = Stroke::new(1.0, Color32::from_rgb(70, 84, 102));
    painter.circle_stroke(center, radius, Stroke::new(1.5, Color32::from_rgb(150, 164, 180)));
    painter.line_segment(
        [Pos2::new(center.x - radius, center.y), Pos2::new(center.x + radius, center.y)],
        ring,
    );
    for swr in [1.5_f32, 2.0, 3.0] {
        let gamma = (swr - 1.0) / (swr + 1.0);
        painter.circle_stroke(center, radius * gamma, ring);
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
            start_hz: 1e6,
            step_hz: 1e6,
            z0: 50.0,
            open: vec![[0.0, 0.0], [0.1, 0.0]],
            short: vec![[-1.0, 0.0]],
            load: vec![[0.0, 0.0], [0.0, 0.0]],
            thru: Vec::new(),
            isolation: Vec::new(),
        });
        assert!(validate(&face).is_err());
    }

    fn one_standard() -> CalSet {
        CalSet {
            id: "abc".into(),
            zone: String::new(),
            band: String::new(),
            start_hz: 1e6,
            step_hz: 1e6,
            z0: 50.0,
            open: vec![[1.0, 0.0], [0.9, 0.1]],
            short: Vec::new(),
            load: Vec::new(),
            thru: Vec::new(),
            isolation: Vec::new(),
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
        assert!(find_zone_cal(&cals, "missing", "VHF", 25_000_000.0, hi).is_none());
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
