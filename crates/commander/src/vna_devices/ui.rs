//! Device scan wizard (#219). Same step-by-step flow and wiring pictures as
//! the VNA calibration wizard: device → safety → calibration → one connect +
//! scan step per port pair → review → save.

use super::*;
use crate::tools::nanovna::{self, NanoVna};
use crate::tx_lockout::TxLockout;
use egui::{Color32, RichText};

const WARN: Color32 = Color32::from_rgb(255, 176, 0);
pub const DANGER: Color32 = Color32::from_rgb(0xD9, 0x42, 0x3A);

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Device,
    Safety,
    Calibration,
    Pair(usize),
    Review,
}

pub struct ScanWizard {
    station: PathBuf,
    station_name: String,
    pub dev: Device,
    tx_path: bool,
    needs_confirm: bool,
    jack: String,
    confirmed: bool,
    step: Step,
    plan: Vec<PairStep>,
    results: Vec<StepResult>,
    waiting: Option<u64>,
    error: String,
    info: String,
    history: Vec<Listed>,
    history_err: String,
    pick_a: Option<usize>,
    pick_b: Option<usize>,
    delete_armed: bool,
    scan_note: String,
    meta: Option<DeviceScanMeta>,
    pub open: bool,
}

struct DeviceScanMeta {
    start_hz: f64,
    stop_hz: f64,
    points: u16,
    vna_id: String,
    cal_note: String,
}

/// What the app should do after this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    None,
    ShowVnaPage,
}

/// Clear a manual (unknown-state) VNA hold: the "Release TX lockout" press.
pub fn release_by_user(lock: &mut TxLockout, station: &Path) {
    lock.release_by_user(LOCK_KEY);
    clear_marker(station);
}

impl ScanWizard {
    pub fn new(station: &Path, bay: &PatchBay, id: u32) -> Option<Self> {
        let dev = bay.device(id)?.clone();
        let ports = rf_ports(&dev);
        let jack = ports.first().cloned().unwrap_or_default();
        let mut w = ScanWizard {
            station: station.to_path_buf(),
            station_name: station.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            tx_path: in_tx_path(bay, id),
            needs_confirm: needs_off_confirm(&dev),
            plan: plan_for(&dev, Some(&jack)),
            dev,
            jack,
            confirmed: false,
            step: Step::Device,
            results: Vec::new(),
            waiting: None,
            error: String::new(),
            info: String::new(),
            history: Vec::new(),
            history_err: String::new(),
            pick_a: None,
            pick_b: None,
            delete_armed: false,
            scan_note: String::new(),
            meta: None,
            open: true,
        };
        w.reload_history();
        Some(w)
    }

    fn reload_history(&mut self) {
        match history(&self.station, &self.dev.uid) {
            Ok(h) => {
                self.history = h;
                self.history_err.clear();
            }
            Err(e) => {
                self.history.clear();
                self.history_err = format!("Saved scans could not be read: {e}");
            }
        }
        self.pick_a = self.pick_a.filter(|i| *i < self.history.len());
        self.pick_b = self.pick_b.filter(|i| *i < self.history.len());
    }

    fn rel_dir(&self) -> String {
        format!("{}.HSData/VNA/{}/", self.station_name, self.dev.uid)
    }

    /// Stop whatever is in flight and let go of an automatic lockout hold.
    pub fn cancel(&mut self, vna: &mut NanoVna, lock: &mut TxLockout) {
        if self.waiting.take().is_some() {
            vna.cancel_device_sweep();
        }
        lock.release(LOCK_KEY);
        if !lock.holds(LOCK_KEY) {
            clear_marker(&self.station);
        }
        self.results.clear();
        self.step = Step::Device;
    }

    /// Called every frame, before drawing, also while the window is hidden.
    pub fn pump(&mut self, vna: &mut NanoVna, lock: &mut TxLockout) {
        let Some(need) = self.waiting else { return };
        let view = vna.device_scan_view();
        let capture = vna.device_scan_capture();
        match poll(need, view.connected, view.generation, capture.is_some()) {
            Poll::Wait => {}
            Poll::Lost => {
                self.waiting = None;
                vna.cancel_device_sweep();
                if lock.holds(LOCK_KEY) {
                    lock.make_manual(LOCK_KEY, &format!("VNA scan on {} stopped unexpectedly; state unknown", self.dev.name));
                }
                self.error = "The VNA disconnected during the sweep. The scan was not saved. TX lockout stays engaged until you press Release TX lockout.".into();
            }
            Poll::Done => {
                self.waiting = None;
                let Step::Pair(i) = self.step else { return };
                let (raw, corrected) = capture.unwrap_or_default();
                let Some(raw) = stored(&raw, &view.vna_id) else {
                    self.error = "The sweep came back empty; scan this step again.".into();
                    return;
                };
                self.results.push(StepResult {
                    step: self.plan[i].clone(),
                    raw,
                    corrected: corrected.and_then(|c| stored(&c, &view.vna_id)),
                });
                if i + 1 < self.plan.len() {
                    self.step = Step::Pair(i + 1);
                    self.info = format!("Step {} of {} measured. Change the cables for the next step.", i + 1, self.plan.len());
                } else {
                    self.step = Step::Review;
                    lock.release(LOCK_KEY);
                    if !lock.holds(LOCK_KEY) {
                        clear_marker(&self.station);
                    }
                    self.info = "All steps measured. Review and save.".into();
                }
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, vna: &mut NanoVna, lock: &mut TxLockout) -> Out {
        self.pump(vna, lock);
        let mut out = Out::None;
        let mut open = self.open;
        let title = format!("VNA SCAN · {}", self.dev.name);
        egui::Window::new(title)
            .id(egui::Id::unique(("vna-device-scan", self.dev.uid.clone())))
            .open(&mut open)
            .default_size([660.0, 640.0])
            .max_height(760.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    out = self.body(ui, vna, lock);
                });
            });
        if !open {
            self.cancel(vna, lock);
        }
        self.open = open;
        out
    }

    fn body(&mut self, ui: &mut egui::Ui, vna: &mut NanoVna, lock: &mut TxLockout) -> Out {
        let mut out = Out::None;
        let view = vna.device_scan_view();
        if let Some(b) = lock.banner() {
            ui.label(RichText::new(b).color(WARN).strong());
            if lock.is_manual(LOCK_KEY) && ui.button("Release TX lockout").clicked() {
                release_by_user(lock, &self.station);
            }
        }
        ui.label(format!("{} · {} · {} port(s), {} Ω reference", self.dev.kind_label(), self.dev.name, rf_ports(&self.dev).len(), kinds::Z0_OHMS));
        if view.connected {
            ui.label(format!("VNA CONNECTED · {}", view.status));
        } else {
            ui.label(RichText::new("NO VNA CONNECTED. Saved scans can still be viewed. Connect on Tools · NanoVNA to scan.").color(WARN).strong());
        }
        if !self.error.is_empty() {
            ui.label(RichText::new(format!("✖ {}", self.error)).color(WARN));
        }
        if !self.info.is_empty() {
            ui.label(&self.info);
        }
        ui.separator();
        ui.label(RichText::new(self.step_title()).strong());
        match self.step.clone() {
            Step::Device => {
                let ports = rf_ports(&self.dev);
                if ports.is_empty() {
                    ui.label("This device has no RF ports to scan.");
                    return out;
                }
                if self.dev.kind == Kind::Radio {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Jack to scan (1-port S11):");
                        for p in &ports {
                            if crate::toggle::add_toggle(ui, true, self.jack == *p, p).clicked() {
                                self.jack = p.clone();
                            }
                        }
                    });
                    self.plan = plan_for(&self.dev, Some(&self.jack));
                }
                let note = if self.dev.kind == Kind::Rf { kinds::note_of(&self.dev.rf) } else { "" };
                if !note.is_empty() {
                    ui.label(note);
                }
                ui.label(format!("Scan plan: {} step(s). NanoVNA V2 measures S11 and S21 only; S22 comes from a reversed step where the plan has one.", self.plan.len()));
                for (i, s) in self.plan.iter().enumerate() {
                    ui.label(format!("{}. {} — {}", i + 1, s.measures(), s.text()));
                }
                ui.label(if self.tx_path {
                    "This device is in a transmit path: the station TX LOCKOUT is engaged before the first sweep and held until the scan finishes or is cancelled."
                } else {
                    "Not in a transmit path on the panel: no TX lockout needed."
                });
                ui.label("The NanoVNA sends a small stimulus signal out of PORT 1 while it sweeps.");
                if ui.button("Next: safety").clicked() {
                    self.step = Step::Safety;
                }
            }
            Step::Safety => {
                if self.needs_confirm {
                    ui.label(RichText::new(RADIO_WARNING).color(WARN).strong());
                    let label = if self.confirmed { "✔ Confirmed: this device is off / not transmitting" } else { "I confirm this device is off / not transmitting" };
                    if crate::toggle::add_toggle(ui, true, self.confirmed, label).clicked() {
                        self.confirmed = !self.confirmed;
                    }
                } else {
                    ui.label("Passive device: no power-off confirmation needed.");
                }
                ui.horizontal(|ui| {
                    if ui.button("Back").clicked() {
                        self.step = Step::Device;
                    }
                    if ui.add_enabled(!self.needs_confirm || self.confirmed, egui::Button::new("Next: calibration")).clicked() {
                        self.step = Step::Calibration;
                    }
                });
                if self.needs_confirm && !self.confirmed {
                    ui.label("Confirm above to continue.");
                }
            }
            Step::Calibration => {
                ui.label(format!(
                    "Sweep: {} – {}, {} points (change it on the NanoVNA page).",
                    mhz(view.start_hz),
                    mhz(view.stop_hz),
                    view.points
                ));
                ui.label(&view.cal_note);
                ui.label("Calibration is reused from the VNA page. Calibrate there (Cal tab) at the cable ends you will connect to the device.");
                ui.horizontal(|ui| {
                    if ui.button("Back").clicked() {
                        self.step = Step::Safety;
                    }
                    if ui.button("Open NanoVNA page").clicked() {
                        out = Out::ShowVnaPage;
                    }
                    if ui.button("Next: connect").clicked() {
                        self.results.clear();
                        self.meta = Some(DeviceScanMeta {
                            start_hz: view.start_hz,
                            stop_hz: view.stop_hz,
                            points: view.points,
                            vna_id: view.vna_id.clone(),
                            cal_note: view.cal_note.clone(),
                        });
                        self.step = Step::Pair(0);
                        out = Out::ShowVnaPage;
                    }
                });
            }
            Step::Pair(i) => {
                let s = self.plan[i].clone();
                ui.label(format!("Step {} of {}: {}", i + 1, self.plan.len(), s.measures()));
                nanovna::paint_device_wiring(ui, &self.dev.name, &rf_ports(&self.dev), &s.ch0, s.ch1.as_deref(), &s.loads);
                ui.label(RichText::new(s.text()).strong());
                if self.waiting.is_some() {
                    ui.label("Sweeping… keep the NanoVNA page open.");
                    if ui.button("Cancel scan").clicked() {
                        self.cancel(vna, lock);
                        self.info = "Scan cancelled; nothing saved.".into();
                    }
                } else {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(view.connected, egui::Button::new("Scan this step")).clicked() {
                            self.start_step(vna, lock);
                            out = Out::ShowVnaPage;
                        }
                        if ui.button("Cancel scan").clicked() {
                            self.cancel(vna, lock);
                            self.info = "Scan cancelled; nothing saved.".into();
                        }
                    });
                    if !view.connected {
                        ui.label(RichText::new("NO VNA CONNECTED: cannot scan.").color(WARN));
                    }
                }
            }
            Step::Review => {
                for r in &self.results {
                    for line in step_summary(r) {
                        ui.label(line);
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("Note");
                    ui.add(egui::TextEdit::singleline(&mut self.scan_note).hint_text("e.g. switch position 2, after rain").desired_width(320.0));
                });
                ui.horizontal(|ui| {
                    if ui.button("Save scan").clicked() {
                        self.save();
                    }
                    if ui.button("Discard").clicked() {
                        self.results.clear();
                        self.step = Step::Device;
                        self.info = "Scan discarded; nothing saved.".into();
                    }
                });
            }
        }
        ui.separator();
        self.history_ui(ui);
        out
    }

    fn step_title(&self) -> String {
        match &self.step {
            Step::Device => "1 · DEVICE".into(),
            Step::Safety => "2 · SAFETY".into(),
            Step::Calibration => "3 · CALIBRATION".into(),
            Step::Pair(i) => format!("4 · CONNECT AND SCAN ({}/{})", i + 1, self.plan.len()),
            Step::Review => "5 · REVIEW AND SAVE".into(),
        }
    }

    fn start_step(&mut self, vna: &mut NanoVna, lock: &mut TxLockout) {
        self.error.clear();
        let view = vna.device_scan_view();
        if let Err(e) = arm_sweep(lock, self.tx_path, &self.dev.name, view.connected) {
            self.error = e;
            return;
        }
        if self.tx_path {
            if let Err(e) = write_marker(&self.station, &self.dev.name) {
                self.error = format!("TX lockout is engaged but its restart marker could not be written ({e}). Scanning anyway; the lockout holds.");
            }
        }
        match vna.request_device_sweep() {
            Ok(need) => self.waiting = Some(need),
            Err(e) => {
                lock.release(LOCK_KEY);
                if !lock.holds(LOCK_KEY) {
                    clear_marker(&self.station);
                }
                self.error = e;
            }
        }
    }

    fn save(&mut self) {
        let Some(m) = &self.meta else { return };
        let set = ScanSet {
            format: FORMAT.into(),
            version: VERSION,
            device_uid: self.dev.uid.clone(),
            device_name: self.dev.name.clone(),
            device_kind: if self.dev.kind == Kind::Rf { self.dev.rf.clone() } else { format!("{:?}", self.dev.kind).to_lowercase() },
            ports: rf_ports(&self.dev),
            taken_at: nanovna::cal_stamp_now(),
            start_hz: m.start_hz,
            stop_hz: m.stop_hz,
            points: m.points,
            vna_id: m.vna_id.clone(),
            cal_note: m.cal_note.clone(),
            by: format!("HamSpot Commander {}", env!("CARGO_PKG_VERSION")),
            note: self.scan_note.clone(),
            length_m: self.dev.length_m,
            vf: self.dev.vf,
            steps: self.results.clone(),
        };
        match save_new(&self.station, &set) {
            Ok(p) => {
                let file = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                self.info = format!("Saved {}{file}", self.rel_dir());
                self.results.clear();
                self.scan_note.clear();
                self.step = Step::Device;
                self.reload_history();
            }
            Err(e) => self.error = format!("Scan NOT saved: {e}. The measurement is still here; try Save again."),
        }
    }

    fn history_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(format!("SCAN HISTORY · {} (newest first)", self.rel_dir())).strong());
        if !self.history_err.is_empty() {
            ui.label(RichText::new(&self.history_err).color(WARN));
        }
        if self.history.is_empty() {
            ui.label("No saved scans for this device yet.");
            return;
        }
        if ui.button("Reload").clicked() {
            self.reload_history();
        }
        for i in 0..self.history.len() {
            let (head, lines) = match &self.history[i].set {
                Ok(s) => (
                    format!("{} · {} · {} step(s){}", s.taken_at, self.history[i].file, s.steps.len(), if s.note.is_empty() { String::new() } else { format!(" · {}", s.note) }),
                    s.steps.iter().flat_map(step_summary).collect::<Vec<_>>(),
                ),
                Err(e) => (format!("{} · cannot read: {e}", self.history[i].file), Vec::new()),
            };
            ui.horizontal_wrapped(|ui| {
                if crate::toggle::add_toggle(ui, true, self.pick_a == Some(i), "A").clicked() {
                    self.pick_a = if self.pick_a == Some(i) { None } else { Some(i) };
                    self.delete_armed = false;
                }
                if crate::toggle::add_toggle(ui, true, self.pick_b == Some(i), "B").clicked() {
                    self.pick_b = if self.pick_b == Some(i) { None } else { Some(i) };
                }
                ui.label(head);
            });
            if let Some(first) = lines.first() {
                ui.label(format!("      {first}"));
            }
        }
        if let (Some(a), Some(b)) = (self.pick_a, self.pick_b) {
            if let (Ok(x), Ok(y)) = (&self.history[a].set, &self.history[b].set) {
                ui.separator();
                ui.label(RichText::new("COMPARE A · B").strong());
                ui.label(format!("A {}  ·  B {}", x.taken_at, y.taken_at));
                compare_lines(x, y).into_iter().for_each(|l| {
                    ui.label(l);
                });
            }
        }
        if let Some(a) = self.pick_a {
            let file = self.history[a].file.clone();
            ui.add_space(6.0);
            crate::group_box::group_box(ui, "Danger", DANGER, |ui| {
                if !self.delete_armed {
                    if ui.button(format!("Delete scan A ({file})")).clicked() {
                        self.delete_armed = true;
                    }
                } else {
                    ui.label(format!("Delete {file} for good? This can not be undone."));
                    ui.horizontal(|ui| {
                        if ui.button("Confirm delete").clicked() {
                            match delete_scan(&self.station, &self.dev.uid, &file) {
                                Ok(()) => self.info = format!("Deleted {file}."),
                                Err(e) => self.error = format!("Not deleted: {e}"),
                            }
                            self.delete_armed = false;
                            self.pick_a = None;
                            self.reload_history();
                        }
                        if ui.button("Keep it").clicked() {
                            self.delete_armed = false;
                        }
                    });
                }
            });
        }
    }
}

/// Step-by-step comparison of two scan sets of one device, as text.
pub fn compare_lines(a: &ScanSet, b: &ScanSet) -> Vec<String> {
    let mut out = Vec::new();
    for sa in &a.steps {
        let Some(sb) = b.steps.iter().find(|s| s.step == sa.step) else {
            out.push(format!("{}: only in A", sa.step.measures()));
            continue;
        };
        let pa = samples_of(sa.corrected.as_ref().unwrap_or(&sa.raw));
        let pb = samples_of(sb.corrected.as_ref().unwrap_or(&sb.raw));
        if let (Some(x), Some(y)) = (s11_stats(&pa), s11_stats(&pb)) {
            out.push(format!(
                "S11 at {}: resonance A {} / B {} (Δ {:+.1} kHz) · min SWR A {:.2} / B {:.2}",
                sa.step.ch0,
                mhz(x.at_hz),
                mhz(y.at_hz),
                (y.at_hz - x.at_hz) / 1e3,
                x.min_swr,
                y.min_swr
            ));
        }
        if let (Some(ch1), Some(x), Some(y)) = (&sa.step.ch1, s21_stats(&pa), s21_stats(&pb)) {
            out.push(format!("S21 {} → {ch1}: best A {:.2} dB / B {:.2} dB (Δ {:+.2} dB)", sa.step.ch0, x.best_db, y.best_db, y.best_db - x.best_db));
        }
    }
    if a.cal_note != b.cal_note {
        out.push("Calibration differs between A and B; compare with care.".into());
    }
    out
}
