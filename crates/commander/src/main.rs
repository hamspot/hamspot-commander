//! Hamspot Commander, phase 1: the rig-control face.
//!
//! The window follows the RT-950 CPS shell: a top action bar, a page tab row,
//! grouped frames in the body, and a status line. The controls are the rig
//! face (frequency, meters, VFO, band scales, levels).

mod channels;
mod dial;
mod plan;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use eframe::egui;
use rigctl::{Command, DemoRig, LinkKind, LiveRig, Rig, RigError, Snapshot};

use dial::{
    channel_dial, channel_window, digit_steps, format_hz, meter, parse_freq, scale, strength_frac,
    strength_label, swatch, vfo_label, wheel_notches, zone_scale, PaintSpan, SubBand, HAM,
};
use plan::{BandPlan, Use};

/// The hamspot.com favicon, scaled with nearest-neighbor so the window list
/// and the dock stay sharp. The site file is 16×17, so the result is padded
/// out to a square.
fn hamspot_icon() -> egui::IconData {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/hamspot-favicon.png"))
        .expect("embedded hamspot favicon");
    square_icon(&scale_nearest(&icon, 8))
}

fn scale_nearest(icon: &egui::IconData, factor: u32) -> egui::IconData {
    let width = icon.width * factor;
    let height = icon.height * factor;
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let src = (((y / factor) * icon.width + (x / factor)) * 4) as usize;
            let dst = ((y * width + x) * 4) as usize;
            rgba[dst..dst + 4].copy_from_slice(&icon.rgba[src..src + 4]);
        }
    }
    egui::IconData {
        rgba,
        width,
        height,
    }
}

fn square_icon(icon: &egui::IconData) -> egui::IconData {
    let side = icon.width.max(icon.height);
    if icon.width == side && icon.height == side {
        return icon.clone();
    }
    let mut rgba = vec![0u8; (side * side * 4) as usize];
    let x0 = (side - icon.width) / 2;
    let y0 = (side - icon.height) / 2;
    for y in 0..icon.height {
        for x in 0..icon.width {
            let src = ((y * icon.width + x) * 4) as usize;
            let dst = (((y + y0) * side + (x + x0)) * 4) as usize;
            rgba[dst..dst + 4].copy_from_slice(&icon.rgba[src..src + 4]);
        }
    }
    egui::IconData {
        rgba,
        width: side,
        height: side,
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([960.0, 640.0])
            .with_title("Hamspot Commander")
            // Wayland uses this id to find hamspotcommander.desktop, which is
            // what puts the icon in the window list and on the dock.
            .with_app_id("hamspotcommander")
            .with_icon(hamspot_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Hamspot Commander",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

struct Favorite {
    hz: f64,
    mode: String,
    passband_hz: i32,
}

struct Shared {
    snap: Snapshot,
    cmds: Vec<Command>,
    switch: Option<Switch>,
}

enum Switch {
    Demo,
    Connect { host: String, port: u16 },
    Disconnect,
}

struct App {
    shared: Arc<Mutex<Shared>>,
    wake: Arc<Condvar>,
    host: String,
    port: String,
    connect_open: bool,
    favorite: Option<Favorite>,
    fav_open: bool,
    show_spread: bool,
    band_mem: HashMap<String, f64>,
    step_hz: f64,
    freq_text: String,
    sub_freq_text: String,
    local_mem: Option<f64>,
    hold_freq: Option<f64>,
    hold_levels: HashMap<String, f64>,
    hold_pass: Option<i32>,
    fine_lock: Option<(f64, f64)>,
    fullscreen: bool,
    page: Page,
    plan: BandPlan,
    zone_sel: usize,
    band_sel: usize,
    plan_path: String,
    /// Last folder chosen in the band-plan browser. Absent means the working directory.
    plan_dir: Option<PathBuf>,
    plan_note: String,
    /// True when `plan_note` is a transmit refusal. Load and save errors stay
    /// until the next load or save.
    plan_note_ptt: bool,
    /// Text in the open zone-tab rename field. The plan keeps the previous
    /// name while this is empty or only spaces.
    zone_rename: Option<(usize, String)>,
    /// Mode last requested from a bandspread click. A drag across one segment
    /// does not send the same mode again until the rig reports it or the
    /// operator picks another mode.
    mode_hold: Option<String>,
    /// Last split frequency and mode requested by a right-click on the bandspread.
    sub_freq_hold: Option<f64>,
    /// Heights of the Tune VFO row from the previous frame. A horizontal row
    /// pins a shorter neighbor to the top, so each piece is padded to the
    /// tallest of the current VFO, the VFO button frame, and the paired VFO.
    main_cluster_height: f32,
    vfo_frame_height: f32,
    sub_vfo_height: f32,
    sub_mode_hold: Option<String>,
    channels: channels::ChannelFile,
    channel_zone: usize,
    channel_sel: usize,
    channel_path: String,
    /// Last folder chosen in the channel browser. Absent means the working directory.
    channel_dir: Option<PathBuf>,
    channel_note: String,
}

fn rx_pairs(snap: &Snapshot) -> Vec<(f64, f64)> {
    snap.caps
        .rx_ranges
        .iter()
        .map(|range| (range.min, range.max))
        .collect()
}

fn rx_fallback(snap: &Snapshot) -> (f64, f64) {
    if snap.caps.rx_ranges.is_empty() {
        return (1_800_000.0, 29_700_000.0);
    }
    let min = snap
        .caps
        .rx_ranges
        .iter()
        .map(|range| range.min)
        .fold(f64::MAX, f64::min);
    let max = snap
        .caps
        .rx_ranges
        .iter()
        .map(|range| range.max)
        .fold(f64::MIN, f64::max);
    (min, max)
}

fn mode_choices(snap: &Snapshot) -> Vec<String> {
    if snap.caps.modes.is_empty() {
        ["USB", "LSB", "CW", "AM", "FM"]
            .into_iter()
            .map(str::to_string)
            .collect()
    } else {
        snap.caps.modes.clone()
    }
}

#[cfg(test)]
mod icon_tests {
    #[test]
    fn favicon_becomes_a_square_window_icon() {
        let icon = super::hamspot_icon();
        assert_eq!(icon.width, icon.height);
        assert!(icon.width >= 128);
        assert_eq!(
            icon.rgba.len(),
            icon.width as usize * icon.height as usize * 4
        );
    }
}

fn file_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string())
}

fn remembered_dir(dir: &Option<PathBuf>) -> PathBuf {
    dir.clone()
        .filter(|dir| dir.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn mhz_drag(ui: &mut egui::Ui, label: &str, mhz: &mut f64) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::DragValue::new(mhz)
                .speed(0.001)
                .range(0.0..=1500.0)
                .clamp_existing_to_range(false)
                .max_decimals(6)
                .suffix(" MHz"),
        )
    })
    .inner
    .changed()
}

fn paint_spans(zone: &plan::Zone) -> Vec<PaintSpan> {
    let mut spans: Vec<(u8, PaintSpan)> = zone
        .bands
        .iter()
        .flat_map(|band| band.segments.iter())
        .map(|seg| {
            let rank = seg.service.as_deref().map(plan::service_rank).unwrap_or(0);
            let [r, g, b] = plan::segment_rgb(seg);
            (
                rank,
                PaintSpan {
                    lo: seg.lo_hz,
                    hi: seg.hi_hz,
                    color: egui::Color32::from_rgb(r, g, b),
                },
            )
        })
        .collect();
    // Later paint covers an overlap. Ham sits above broadcast.
    spans.sort_by_key(|(rank, span)| (*rank, span.lo as i64));
    spans.into_iter().map(|(_, span)| span).collect()
}

fn legend_items(zone: &plan::Zone, lo: f64, hi: f64) -> Vec<(egui::Color32, String, bool)> {
    let tagged = zone.bands.iter().any(|band| {
        band.segments
            .iter()
            .any(|seg| seg.service.as_ref().is_some_and(|name| !name.is_empty()))
    });
    if !tagged {
        return [Use::Cw, Use::Digital, Use::Voice, Use::Mixed]
            .into_iter()
            .map(|kind| {
                let drawn = use_in_window(zone, kind, lo, hi);
                let [r, g, b] = kind.rgb();
                (
                    egui::Color32::from_rgb(r, g, b),
                    kind.label().to_string(),
                    drawn,
                )
            })
            .collect();
    }
    plan::SERVICES
        .iter()
        .filter(|name| {
            zone.bands.iter().any(|band| {
                band.segments
                    .iter()
                    .any(|seg| seg.service.as_deref() == Some(**name))
            })
        })
        .map(|name| {
            let drawn = zone.bands.iter().any(|band| {
                band.segments.iter().any(|seg| {
                    seg.service.as_deref() == Some(*name) && seg.hi_hz >= lo && seg.lo_hz <= hi
                })
            });
            let [r, g, b] = plan::service_rgb(name);
            (
                egui::Color32::from_rgb(r, g, b),
                plan::service_label(name),
                drawn,
            )
        })
        .collect()
}

fn use_in_window(zone: &plan::Zone, kind: Use, lo: f64, hi: f64) -> bool {
    zone.bands.iter().any(|band| {
        band.segments
            .iter()
            .any(|seg| seg.use_kind == kind && seg.hi_hz >= lo && seg.lo_hz <= hi)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Tune,
    Plan,
    Channels,
    Meters,
    Levels,
}

const PAGES: &[(Page, &str)] = &[
    (Page::Tune, "Tune"),
    (Page::Plan, "Plan"),
    (Page::Channels, "Channels"),
    (Page::Meters, "Meters"),
    (Page::Levels, "Levels"),
];

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        // The face talks to hamlib. The in-process demo is only the Demo button.
        let mut snap = empty_snap();
        snap.message = "connecting to 127.0.0.1:4532".into();
        let shared = Arc::new(Mutex::new(Shared {
            snap,
            cmds: Vec::new(),
            switch: Some(Switch::Connect {
                host: "127.0.0.1".into(),
                port: 4532,
            }),
        }));
        let wake = Arc::new(Condvar::new());
        let ctx = cc.egui_ctx.clone();
        let shared_thread = Arc::clone(&shared);
        let wake_thread = Arc::clone(&wake);
        thread::spawn(move || drive(shared_thread, wake_thread, ctx));
        let plan_path = plan::default_path();
        let plan = plan::load_file(&plan_path).unwrap_or_else(|_| plan::embedded());
        let plan_dir = cc
            .storage
            .and_then(|storage| storage.get_string("plan_dir"))
            .map(PathBuf::from)
            .filter(|dir| dir.is_dir())
            .or_else(plan::bsp_dir);
        let channel_dir = cc
            .storage
            .and_then(|storage| storage.get_string("channel_dir"))
            .map(PathBuf::from)
            .filter(|dir| dir.is_dir());
        let band_sel = plan
            .zones
            .first()
            .and_then(|zone| zone.bands.iter().position(|band| band.chip == "20m"))
            .unwrap_or(0);
        Self {
            shared,
            wake,
            host: "127.0.0.1".into(),
            port: "4532".into(),
            connect_open: false,
            favorite: None,
            fav_open: false,
            show_spread: true,
            band_mem: HashMap::new(),
            step_hz: 100.0,
            freq_text: format_hz(rigctl::DEMO_FREQ_HZ),
            sub_freq_text: format_hz(7_074_000.0),
            local_mem: None,
            hold_freq: None,
            hold_levels: HashMap::new(),
            hold_pass: None,
            fine_lock: None,
            fullscreen: false,
            page: Page::Tune,
            plan,
            zone_sel: 0,
            band_sel,
            plan_path: plan_path.display().to_string(),
            plan_dir,
            plan_note: String::new(),
            plan_note_ptt: false,
            zone_rename: None,
            mode_hold: None,
            sub_freq_hold: None,
            sub_mode_hold: None,
            main_cluster_height: 0.0,
            vfo_frame_height: 0.0,
            sub_vfo_height: 0.0,
            channels: channels::empty(),
            channel_zone: 0,
            channel_sel: 0,
            channel_path: String::new(),
            channel_dir,
            channel_note: String::new(),
        }
    }

    fn snap(&self) -> Snapshot {
        self.shared.lock().expect("rig thread").snap.clone()
    }

    fn send(&self, cmd: Command) {
        let mut guard = self.shared.lock().expect("rig thread");
        guard.cmds.push(cmd);
        self.wake.notify_one();
    }

    fn switch(&self, switch: Switch) {
        let mut guard = self.shared.lock().expect("rig thread");
        guard.switch = Some(switch);
        self.wake.notify_one();
    }

    fn shown_freq(&mut self, snap: &Snapshot) -> f64 {
        if let Some(held) = self.hold_freq {
            if (held - snap.freq_hz).abs() < 0.5 {
                self.hold_freq = None;
                snap.freq_hz
            } else {
                held
            }
        } else {
            snap.freq_hz
        }
    }

    /// Point the Tune chips and the bandspread at the band that contains `hz`.
    /// The current band wins when it still contains `hz`, so a shared
    /// frequency stays on the line the operator clicked. A gap leaves the
    /// selection alone.
    fn follow_band(&mut self, hz: f64, rx: &[(f64, f64)]) {
        if let Some(band) = self
            .plan
            .zones
            .get(self.zone_sel)
            .and_then(|zone| zone.bands.get(self.band_sel))
        {
            let inside = plan::pieces_inside(band.lo_hz, band.hi_hz, rx)
                .iter()
                .any(|(lo, hi)| hz >= *lo && hz <= *hi);
            if inside {
                return;
            }
        }
        let bands = plan::tunable_bands(&self.plan, rx);
        let Some(idx) = plan::band_at(&bands, hz) else {
            return;
        };
        self.zone_sel = bands[idx].zone;
        self.band_sel = bands[idx].band;
    }

    fn shown_sub_freq(&self, snap: &Snapshot) -> f64 {
        self.sub_freq_hold.unwrap_or(snap.sub_freq_hz)
    }

    fn send_plan_mode(&mut self, snap: &Snapshot, wanted: &str) {
        if !snap.online() || !snap.caps.can_set_mode {
            return;
        }
        let Some(mode) = plan::choose_mode(wanted, &snap.caps.modes) else {
            return;
        };
        if snap.mode.eq_ignore_ascii_case(&mode) {
            return;
        }
        if self
            .mode_hold
            .as_deref()
            .is_some_and(|held| held.eq_ignore_ascii_case(&mode))
        {
            return;
        }
        self.mode_hold = Some(mode.clone());
        self.send(Command::SetMode {
            mode,
            passband_hz: snap.passband_hz,
        });
    }

    /// Right-click path. Same frequency and plan mode as the main VFO, sent
    /// to the other VFO. The main frequency and mode are left alone.
    fn tune_sub(&mut self, snap: &Snapshot, hz: f64, wanted: Option<&str>) {
        let Some(hz) = snap.caps.allow_freq(hz) else {
            return;
        };
        if !snap.online() || !snap.caps.can_set_split {
            return;
        }
        self.sub_freq_text = format_hz(hz);
        let held = self
            .sub_freq_hold
            .is_some_and(|held| (held - hz).abs() < 0.5);
        let reported = snap.sub_freq_hz > 0.0 && (snap.sub_freq_hz - hz).abs() < 0.5;
        if !held && !reported {
            self.sub_freq_hold = Some(hz);
            self.send(Command::SetSplitFreq(hz));
        }
        let Some(wanted) = wanted else {
            return;
        };
        let Some(mode) = plan::choose_mode(wanted, &snap.caps.modes) else {
            return;
        };
        if snap.sub_mode.eq_ignore_ascii_case(&mode) {
            return;
        }
        if self
            .sub_mode_hold
            .as_deref()
            .is_some_and(|held| held.eq_ignore_ascii_case(&mode))
        {
            return;
        }
        self.sub_mode_hold = Some(mode.clone());
        self.send(Command::SetSubMode {
            mode,
            passband_hz: snap.passband_hz,
        });
    }

    fn tune(&mut self, snap: &Snapshot, hz: f64) {
        let Some(hz) = snap.caps.allow_freq(hz) else {
            let current = self.shown_freq(snap);
            self.freq_text = format_hz(current);
            return;
        };
        self.hold_and_send(hz);
    }

    /// Move one VFO by `delta` hertz. One digit click or one wheel notch
    /// passes a single place value. The step stops at the receive edge.
    fn step_digit(&mut self, snap: &Snapshot, current: f64, delta: f64, main: bool) {
        let target = current + delta;
        if main {
            self.nudge(snap, current, target);
        } else {
            self.nudge_sub(snap, current, target);
        }
    }

    /// Wheel and digit steps. Stops on the edge of the current receive
    /// segment instead of jumping a gap or going negative.
    fn nudge(&mut self, snap: &Snapshot, current: f64, target: f64) {
        let Some(hz) = snap.caps.step_freq(current, target) else {
            return;
        };
        if (hz - current).abs() < 0.5 {
            return;
        }
        self.hold_and_send(hz);
    }

    fn nudge_sub(&mut self, snap: &Snapshot, current: f64, target: f64) {
        let Some(hz) = snap.caps.step_freq(current, target) else {
            return;
        };
        if (hz - current).abs() < 0.5 {
            return;
        }
        self.tune_sub(snap, hz, None);
    }

    fn hold_and_send(&mut self, hz: f64) {
        self.hold_freq = Some(hz);
        self.freq_text = format_hz(hz);
        self.send(Command::SetFreq(hz));
    }

    /// ±1.2 kHz around `freq`, cut off at the receive segment so the fine
    /// scale cannot offer a frequency the radio will not tune.
    fn fine_window(&self, snap: &Snapshot, freq: f64) -> (f64, f64) {
        if let Some(locked) = self.fine_lock {
            return locked;
        }
        let mut lo = freq - 1_200.0;
        let mut hi = freq + 1_200.0;
        if let Some(seg) = snap.caps.rx_segment(freq) {
            lo = lo.max(seg.min);
            hi = hi.min(seg.max);
        }
        if lo > hi {
            (freq, freq)
        } else {
            (lo, hi)
        }
    }

    fn shown_level(&mut self, name: &str, snap: &Snapshot) -> f64 {
        let live = snap.levels.get(name).copied().unwrap_or(0.0);
        if let Some(held) = self.hold_levels.get(name).copied() {
            if (held - live).abs() < 1e-3 {
                self.hold_levels.remove(name);
                live
            } else {
                held
            }
        } else {
            live
        }
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(dir) = &self.plan_dir {
            storage.set_string("plan_dir", dir.display().to_string());
        }
        if let Some(dir) = &self.channel_dir {
            storage.set_string("channel_dir", dir.display().to_string());
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut snap = self.snap();
        // Tune follows the frequency the readout is showing, including a
        // set_freq that the radio has not confirmed yet. The Plan editor
        // keeps its own zone tab.
        let face_hz = if self.page == Page::Tune {
            let hz = self.shown_freq(&snap);
            self.follow_band(hz, &rx_pairs(&snap));
            hz
        } else {
            snap.freq_hz
        };
        if snap.online() {
            if let Some(band) = self
                .plan
                .zones
                .get(self.zone_sel)
                .and_then(|zone| zone.bands.get(self.band_sel))
            {
                if snap.freq_hz >= band.lo_hz && snap.freq_hz <= band.hi_hz {
                    self.band_mem.insert(band.name.clone(), snap.freq_hz);
                }
            }
        }
        if self
            .mode_hold
            .as_deref()
            .is_some_and(|held| snap.mode.eq_ignore_ascii_case(held))
        {
            self.mode_hold = None;
        }
        if self
            .sub_freq_hold
            .is_some_and(|held| (snap.sub_freq_hz - held).abs() < 0.5)
        {
            self.sub_freq_hold = None;
        }
        if self
            .sub_mode_hold
            .as_deref()
            .is_some_and(|held| snap.sub_mode.eq_ignore_ascii_case(held))
        {
            self.sub_mode_hold = None;
        }
        if self.plan_note_ptt
            && self
                .plan
                .transmit_block_on(self.zone_sel, self.band_sel, snap.freq_hz, &snap.mode)
                .is_none()
        {
            self.plan_note.clear();
            self.plan_note_ptt = false;
        }
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.pointer.any_released()) {
            self.fine_lock = None;
        }

        egui::Panel::top("bar").show_inside(ui, |ui| {
            self.bar(ui);
        });
        // One row. A later tab scrolls sideways instead of wrapping onto
        // a second line and pushing the page body down.
        egui::Panel::top("pages").show_inside(ui, |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("page-tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (page, label) in PAGES {
                            if ui
                                .add(egui::Button::selectable(self.page == *page, *label))
                                .clicked()
                            {
                                self.page = *page;
                            }
                        }
                    });
                });
        });
        egui::Panel::bottom("status").show_inside(ui, |ui| {
            self.status_line(ui, &snap, face_hz);
        });
        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Tune owns the wheel. Plan and Channels own a bandspread click.
            // A scroll area would take those pointers and move the page.
            if self.page == Page::Tune {
                self.page_tune(ui, &mut snap);
            } else if self.page == Page::Plan {
                self.page_plan(ui, &mut snap);
            } else if self.page == Page::Channels {
                self.page_channels(ui, &mut snap);
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("body")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.page_body(ui, &mut snap));
            }
        });

        if self.connect_open {
            self.connect_window(&ctx);
        }
        if self.fav_open {
            self.favorite_window(&ctx, &snap);
        }
    }
}

impl App {
    fn bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Connect").clicked() {
                self.connect_open = true;
            }
            if ui.button("Demo").clicked() {
                self.hold_freq = None;
                self.hold_levels.clear();
                self.hold_pass = None;
                self.switch(Switch::Demo);
            }
            if ui.button("Favorite").clicked() {
                self.fav_open = true;
            }
            if ui.button("Full screen").clicked() {
                self.fullscreen = !self.fullscreen;
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
        });
    }

    fn status_line(&self, ui: &mut egui::Ui, snap: &Snapshot, face_hz: f64) {
        let link = match snap.kind {
            LinkKind::Demo => "Demo".to_string(),
            LinkKind::Live => format!("{}  {}", snap.model, snap.endpoint),
            LinkKind::Down => format!("Disconnected  {}", snap.message),
        };
        ui.horizontal(|ui| {
            ui.label(link);
            if !snap.message.is_empty() && snap.kind != LinkKind::Down {
                ui.separator();
                ui.label(
                    egui::RichText::new(&snap.message).color(egui::Color32::from_rgb(220, 160, 80)),
                );
            }
            if let Some(label) = self
                .plan
                .reference_label(self.zone_sel, self.band_sel, face_hz)
            {
                ui.separator();
                ui.label(label);
            }
            if !self.plan_note.is_empty() {
                ui.separator();
                ui.label(
                    egui::RichText::new(&self.plan_note)
                        .color(egui::Color32::from_rgb(220, 160, 80)),
                );
            }
            if !self.channel_note.is_empty() {
                ui.separator();
                ui.label(
                    egui::RichText::new(&self.channel_note)
                        .color(egui::Color32::from_rgb(220, 160, 80)),
                );
            }
            if !snap.last_cat.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&snap.last_cat).monospace());
                });
            }
        });
    }

    fn page_body(&mut self, ui: &mut egui::Ui, snap: &mut Snapshot) {
        match self.page {
            Page::Tune => self.page_tune(ui, snap),
            Page::Plan => self.page_plan(ui, snap),
            Page::Channels => self.page_channels(ui, snap),
            Page::Meters => self.page_meters(ui, snap),
            Page::Levels => self.page_levels(ui, snap),
        }
    }

    fn page_tune(&mut self, ui: &mut egui::Ui, snap: &mut Snapshot) {
        let freq = self.shown_freq(snap);
        let sub_freq = self.shown_sub_freq(snap);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            // Same width as the controls frame and the bandspread.
            ui.take_available_width();
            ui.horizontal(|ui| {
                let known_main = self.main_cluster_height;
                let known_frame = self.vfo_frame_height;
                let known_sub = self.sub_vfo_height;
                let show_sub = !snap.sub_vfo.is_empty();
                let row_h = known_main
                    .max(known_frame)
                    .max(if show_sub { known_sub } else { 0.0 });
                let main_h = place_centered(ui, row_h, known_main, |ui| {
                    self.vfo_cluster(ui, snap, true, freq);
                });
                ui.add_space(16.0);
                let frame_h = place_centered(ui, row_h, known_frame, |ui| {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        self.vfo_column(ui, snap);
                    });
                });
                let sub_h = if show_sub {
                    ui.add_space(16.0);
                    place_centered(ui, row_h, known_sub, |ui| {
                        self.vfo_cluster(ui, snap, false, sub_freq);
                    })
                } else {
                    known_sub
                };
                let moved = (main_h - known_main).abs() > 0.5
                    || (frame_h - known_frame).abs() > 0.5
                    || (show_sub && (sub_h - known_sub).abs() > 0.5);
                self.main_cluster_height = main_h;
                self.vfo_frame_height = frame_h;
                if show_sub {
                    self.sub_vfo_height = sub_h;
                }
                if moved {
                    ui.ctx()
                        .request_discard("center the VFO frame with the readouts");
                }
            });
            ui.add_space(6.0);
            let (lo, hi) = self.fine_window(snap, freq);
            if let Some(hz) = scale(ui, "Fine", freq, lo, hi, HAM, 36.0, &[]) {
                if self.fine_lock.is_none() {
                    self.fine_lock = Some((lo, hi));
                }
                if snap.caps.can_set_freq {
                    self.tune(snap, hz);
                }
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let alt = ui.button("ALT");
                if alt.clicked() && snap.caps.vfo_ops.contains("XCHG") {
                    self.send(Command::VfoOp("XCHG".into()));
                }
                let bsp = ui.selectable_label(self.show_spread, "BSP");
                if bsp.clicked() {
                    self.show_spread = !self.show_spread;
                }
                if ui.button("LOAD").clicked() {
                    self.browse_plan(false);
                }
                let mut chip_pick = None;
                // Memory is keyed by band name, the same key `band_step` uses.
                let rx = rx_pairs(snap);
                let listed: Vec<(usize, String, String, f64, Vec<(f64, f64)>)> = self
                    .plan
                    .zones
                    .get(self.zone_sel)
                    .map(|zone| {
                        zone.bands
                            .iter()
                            .enumerate()
                            .filter_map(|(idx, band)| {
                                let pieces = plan::pieces_inside(band.lo_hz, band.hi_hz, &rx);
                                if pieces.is_empty() {
                                    return None;
                                }
                                Some((
                                    idx,
                                    band.chip.clone(),
                                    band.name.clone(),
                                    band.home_hz,
                                    pieces,
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let chips: Vec<(usize, String, String, f64)> = listed
                    .into_iter()
                    .map(|(idx, chip, name, home, pieces)| {
                        let remembered = self.band_mem.get(&name).copied().unwrap_or(home);
                        let hz = if pieces
                            .iter()
                            .any(|(start, end)| remembered >= *start && remembered <= *end)
                        {
                            remembered
                        } else {
                            pieces[0].0
                        };
                        (idx, chip, name, hz)
                    })
                    .collect();
                for (idx, chip, name, hz) in &chips {
                    let on = self.band_sel == *idx;
                    if ui.selectable_label(on, chip).clicked() {
                        self.band_sel = *idx;
                        if snap.caps.can_set_freq {
                            let tuned = self.band_mem.get(name).copied().unwrap_or(*hz);
                            chip_pick = Some(tuned);
                        }
                    }
                }
                if let Some(hz) = chip_pick {
                    self.tune(snap, hz);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.receiver_box(ui, snap);
                    ui.add_space(8.0);
                    self.mode_box(ui, snap, "Sub", false);
                    ui.add_space(8.0);
                    self.mode_box(ui, snap, "Mode", true);
                });
            });
        });
        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.take_available_width();
            self.buttons(ui, snap);
        });
        if self.show_spread {
            ui.add_space(8.0);
            // One line per zone. Subbands share that line, with a small gap
            // between them. The frequency wheel stays above this scroll.
            let frame = egui::Frame::group(ui.style());
            let chrome =
                frame.inner_margin.sum().y + frame.stroke.width * 2.0 + frame.outer_margin.sum().y;
            let height = (ui.available_height() - chrome).max(80.0);
            let mut scale_pick: Option<(usize, usize, f64, bool)> = None;
            frame.show(ui, |ui| {
                ui.take_available_width();
                egui::ScrollArea::vertical()
                    .id_salt("tune-bsp")
                    .auto_shrink([false, true])
                    .max_height(height)
                    .show(ui, |ui| {
                        let rx = rx_pairs(snap);
                        for (zone_idx, zone) in self.plan.zones.iter().enumerate().rev() {
                            let mut subs = Vec::new();
                            let mut ids = Vec::new();
                            for (band_idx, band) in zone.bands.iter().enumerate() {
                                let spans: Vec<PaintSpan> = band
                                    .segments
                                    .iter()
                                    .map(|seg| {
                                        let [r, g, b] = plan::segment_rgb(seg);
                                        PaintSpan {
                                            lo: seg.lo_hz,
                                            hi: seg.hi_hz,
                                            color: egui::Color32::from_rgb(r, g, b),
                                        }
                                    })
                                    .collect();
                                for (lo, hi) in plan::pieces_inside(band.lo_hz, band.hi_hz, &rx) {
                                    ids.push(band_idx);
                                    subs.push(SubBand {
                                        lo,
                                        hi,
                                        chip: band.chip.clone(),
                                        spans: spans.clone(),
                                    });
                                }
                            }
                            if subs.is_empty() {
                                continue;
                            }
                            let picked = (zone_idx == self.zone_sel)
                                .then(|| {
                                    ids.iter()
                                        .enumerate()
                                        .find(|(i, id)| {
                                            **id == self.band_sel
                                                && freq >= subs[*i].lo
                                                && freq <= subs[*i].hi
                                        })
                                        .or_else(|| {
                                            ids.iter()
                                                .enumerate()
                                                .find(|(_, id)| **id == self.band_sel)
                                        })
                                        .map(|(i, _)| i)
                                })
                                .flatten();
                            if let Some((slot, hz, sub)) =
                                zone_scale(ui, &zone.name, freq, &subs, picked, 28.0)
                            {
                                // Left click needs set_freq. Right click needs split.
                                // The other button is ignored when its command is closed.
                                let allowed = if sub {
                                    snap.caps.can_set_split
                                } else {
                                    snap.caps.can_set_freq
                                };
                                if allowed {
                                    scale_pick = Some((zone_idx, ids[slot], hz, sub));
                                }
                            }
                        }
                    });
            });
            if let Some((zone_idx, band_idx, hz, sub)) = scale_pick {
                let mode = self
                    .plan
                    .zones
                    .get(zone_idx)
                    .and_then(|zone| zone.bands.get(band_idx))
                    .map(|band| plan::mode_at(band, hz));
                self.zone_sel = zone_idx;
                self.band_sel = band_idx;
                if sub {
                    self.tune_sub(snap, hz, mode.as_deref());
                } else if snap.caps.allow_freq(hz).is_some() {
                    self.tune(snap, hz);
                    if let Some(mode) = mode {
                        self.send_plan_mode(snap, &mode);
                    }
                }
            }
        }
    }

    fn page_plan(&mut self, ui: &mut egui::Ui, snap: &mut Snapshot) {
        if self.zone_sel >= self.plan.zones.len() {
            self.zone_sel = 0;
            self.band_sel = 0;
        }
        ui.add_space(6.0);
        self.zone_tabs(ui, snap);
        if self.plan.zones.is_empty() {
            ui.label("No zones in this plan.");
            return;
        }
        let band_count = self.plan.zones[self.zone_sel].bands.len();
        if self.band_sel >= band_count {
            self.band_sel = 0;
        }
        let selected = self.plan.zones[self.zone_sel]
            .bands
            .get(self.band_sel)
            .map(|band| (band.home_hz, band.name.clone(), band.mode.clone()));
        let homes: Vec<f64> = self.plan.zones[self.zone_sel]
            .bands
            .iter()
            .map(|band| band.home_hz)
            .collect();
        let (win_lo, win_hi) = channel_window(&homes, rx_fallback(snap));
        let spans = paint_spans(&self.plan.zones[self.zone_sel]);

        let mut clicked = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                let shown = match selected {
                    Some((hz, _, _)) => format!("{:.3}", hz / 1_000_000.0),
                    None => "—".to_string(),
                };
                ui.label(egui::RichText::new(shown).size(40.0).strong());
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("MHz").size(16.0));
                    if let Some((hz, _, _)) = selected {
                        ui.label(egui::RichText::new(format_hz(hz)).weak());
                    }
                });
                ui.add_space(18.0);
                match &selected {
                    Some((_, name, mode)) => {
                        ui.label(egui::RichText::new(name).size(22.0).color(HAM).strong());
                        ui.add_space(12.0);
                        if !mode.is_empty() {
                            ui.label(egui::RichText::new(mode).size(22.0));
                        }
                    }
                    None => {
                        ui.label(egui::RichText::new("—").size(22.0).weak());
                    }
                }
            });
            ui.add_space(6.0);
            let needle = selected.map(|(hz, _, _)| hz);
            clicked = channel_dial(ui, needle, win_lo, win_hi, &spans);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.set_min_height(18.0);
                let items = legend_items(&self.plan.zones[self.zone_sel], win_lo, win_hi);
                for (color, label, drawn) in items {
                    ui.scope(|ui| {
                        if !drawn {
                            ui.set_opacity(0.35);
                        }
                        swatch(ui, color, &label);
                    });
                    ui.add_space(10.0);
                }
            });
        });
        if let Some(hz) = clicked {
            self.dial_home(snap, hz);
        }

        ui.add_space(8.0);
        let gap = ui.spacing().item_spacing.x;
        let editor_w = 320.0;
        let total_w = ui.available_width();
        let left_w = (total_w - gap - editor_w).max(280.0);
        let right_w = (total_w - gap - left_w).max(0.0);
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(left_w);
                egui::ScrollArea::vertical()
                    .id_salt("plan-list")
                    .max_height(height.max(80.0))
                    .show(ui, |ui| {
                        let count = self
                            .plan
                            .zones
                            .get(self.zone_sel)
                            .map(|zone| zone.bands.len())
                            .unwrap_or(0);
                        for idx in 0..count {
                            if self.band_row(ui, idx) {
                                self.recall_band(snap, idx);
                            }
                        }
                    });
            });
            ui.vertical(|ui| {
                ui.set_width(right_w);
                egui::ScrollArea::vertical()
                    .id_salt("plan-editor")
                    .auto_shrink([false, true])
                    .max_height(height.max(80.0))
                    .show(ui, |ui| self.band_editor(ui, snap, right_w));
            });
        });
    }

    fn zone_tabs(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        let names: Vec<String> = self
            .plan
            .zones
            .iter()
            .map(|zone| zone.name.clone())
            .collect();
        ui.horizontal_wrapped(|ui| {
            if ui.button("LOAD").clicked() {
                self.browse_plan(false);
            }
            if ui.button("Save").clicked() {
                self.browse_plan(true);
            }
            if ui.button("ADD").clicked() {
                let color = self
                    .plan
                    .zones
                    .get(self.zone_sel)
                    .map(|zone| zone.color)
                    .unwrap_or([92, 184, 120]);
                let home = self.shown_freq(snap);
                self.zone_sel = plan::push_zone(&mut self.plan, color, home, &snap.mode);
                self.band_sel = 0;
                self.zone_rename = None;
            }
            for (idx, name) in names.iter().enumerate() {
                let response = ui.add(egui::Button::selectable(self.zone_sel == idx, name));
                if response.clicked() || response.secondary_clicked() {
                    self.zone_sel = idx;
                    self.band_sel = 0;
                }
                response.context_menu(|ui| self.zone_menu(ui, idx));
            }
        });
    }

    fn zone_menu(&mut self, ui: &mut egui::Ui, idx: usize) {
        let current = self
            .plan
            .zones
            .get(idx)
            .map(|zone| zone.name.clone())
            .unwrap_or_default();
        let mut text = self
            .zone_rename
            .as_ref()
            .filter(|(stored, _)| *stored == idx)
            .map(|(_, text)| text.clone())
            .unwrap_or(current);
        let edited = ui
            .add(
                egui::TextEdit::singleline(&mut text)
                    .id_salt(("zone-rename", idx))
                    .desired_width(160.0),
            )
            .changed();
        if edited {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                if let Some(zone) = self.plan.zones.get_mut(idx) {
                    plan::rename_zone(zone, trimmed);
                }
            }
            self.zone_rename = Some((idx, text));
        }
        if ui.button("Delete").clicked() {
            self.zone_sel = plan::delete_zone(&mut self.plan, idx, self.zone_sel);
            self.band_sel = 0;
            self.zone_rename = None;
            ui.close();
        }
    }

    fn plan_save_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                self.browse_plan(true);
            }
            ui.label(file_label(&self.plan_path))
                .on_hover_text(&self.plan_path);
        });
    }

    fn page_channels(&mut self, ui: &mut egui::Ui, snap: &mut Snapshot) {
        ui.horizontal(|ui| {
            if ui.button("Load").clicked() {
                self.browse_channels(false);
            }
            if ui.button("Save").clicked() {
                self.browse_channels(true);
            }
            let label = if self.channel_path.is_empty() {
                "No file".to_string()
            } else {
                file_label(&self.channel_path)
            };
            ui.label(label).on_hover_text(&self.channel_path);
        });
        if self.channels.zones.is_empty() {
            ui.label("No zones in this channel file.");
            return;
        }
        if self.channel_zone >= self.channels.zones.len() {
            self.channel_zone = 0;
            self.channel_sel = 0;
        }
        let count = self.channels.zones[self.channel_zone].channels.len();
        if count == 0 {
            self.channel_sel = 0;
        } else if self.channel_sel >= count {
            self.channel_sel = count - 1;
        }
        ui.add_space(6.0);
        let names: Vec<String> = self
            .channels
            .zones
            .iter()
            .map(|zone| zone.name.clone())
            .collect();
        ui.horizontal(|ui| {
            for (idx, name) in names.iter().enumerate() {
                if ui
                    .add(egui::Button::selectable(self.channel_zone == idx, name))
                    .clicked()
                {
                    self.channel_zone = idx;
                    self.channel_sel = 0;
                }
            }
        });
        let selected = self.channels.zones[self.channel_zone]
            .channels
            .get(self.channel_sel)
            .map(|channel| (channel.hz, channel.name.clone(), channel.mode.clone()));
        let freqs: Vec<f64> = self.channels.zones[self.channel_zone]
            .channels
            .iter()
            .map(|channel| channel.hz)
            .collect();
        let (win_lo, win_hi) = channel_window(&freqs, rx_fallback(snap));
        let marks = plan::marks_for_window(&self.plan, win_lo, win_hi);
        let spans: Vec<PaintSpan> = marks
            .iter()
            .map(|mark| {
                let [r, g, b] = mark.rgb;
                PaintSpan {
                    lo: mark.lo_hz,
                    hi: mark.hi_hz,
                    color: egui::Color32::from_rgb(r, g, b),
                }
            })
            .collect();
        let needle = selected.as_ref().map(|(hz, _, _)| *hz);
        let plan_title =
            needle.and_then(|hz| plan::label_for_window(&self.plan, win_lo, win_hi, hz));

        let mut clicked = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                let shown = match needle {
                    Some(hz) => format!("{:.3}", hz / 1_000_000.0),
                    None => "—".to_string(),
                };
                ui.label(egui::RichText::new(shown).size(40.0).strong());
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("MHz").size(16.0));
                    if let Some(hz) = needle {
                        ui.label(egui::RichText::new(format_hz(hz)).weak());
                    }
                });
                ui.add_space(18.0);
                match plan_title {
                    Some(title) => {
                        ui.label(egui::RichText::new(title).size(22.0).color(HAM).strong());
                    }
                    None => {
                        ui.label(egui::RichText::new("—").size(22.0).weak());
                    }
                }
                if let Some((_, name, mode)) = &selected {
                    if !name.is_empty() {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(name).size(22.0));
                    }
                    if !mode.is_empty() {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(mode).size(22.0));
                    }
                }
            });
            ui.add_space(6.0);
            clicked = channel_dial(ui, needle, win_lo, win_hi, &spans);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.set_min_height(18.0);
                let mut seen = Vec::new();
                for mark in &marks {
                    if seen.contains(&mark.legend) {
                        continue;
                    }
                    seen.push(mark.legend.clone());
                    let [r, g, b] = mark.rgb;
                    swatch(ui, egui::Color32::from_rgb(r, g, b), &mark.legend);
                    ui.add_space(10.0);
                }
            });
        });
        if let Some(hz) = clicked {
            self.set_channel_hz(snap, hz);
        }

        ui.add_space(8.0);
        let gap = ui.spacing().item_spacing.x;
        let editor_w = 320.0;
        let total_w = ui.available_width();
        let left_w = (total_w - gap - editor_w).max(280.0);
        let right_w = (total_w - gap - left_w).max(0.0);
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(left_w);
                egui::ScrollArea::vertical()
                    .id_salt("channel-list")
                    .max_height(height.max(80.0))
                    .show(ui, |ui| {
                        let count = self
                            .channels
                            .zones
                            .get(self.channel_zone)
                            .map(|zone| zone.channels.len())
                            .unwrap_or(0);
                        for idx in 0..count {
                            if self.channel_row(ui, idx) {
                                self.recall_channel(snap, idx);
                            }
                        }
                    });
            });
            ui.vertical(|ui| {
                ui.set_width(right_w);
                egui::ScrollArea::vertical()
                    .id_salt("channel-editor")
                    .auto_shrink([false, true])
                    .max_height(height.max(80.0))
                    .show(ui, |ui| self.channel_editor(ui, snap, right_w));
            });
        });
    }

    fn browse_plan(&mut self, save: bool) {
        let dir = remembered_dir(&self.plan_dir);
        let dialog = rfd::FileDialog::new()
            .set_title(if save {
                "Save band plan"
            } else {
                "Load band plan"
            })
            .add_filter("Band plan", &["json"])
            .set_directory(&dir);
        let picked = if save {
            let name = Path::new(&self.plan_path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "band-plan.json".into());
            dialog.set_file_name(name).save_file()
        } else {
            dialog.pick_file()
        };
        let Some(path) = picked else {
            return;
        };
        if let Some(parent) = path.parent() {
            if parent.is_dir() {
                self.plan_dir = Some(parent.to_path_buf());
            }
        }
        self.plan_path = path.display().to_string();
        if save {
            self.save_plan();
        } else {
            self.load_plan();
        }
    }

    fn load_plan(&mut self) {
        match plan::load_file(Path::new(&self.plan_path)) {
            Ok(plan) => {
                self.plan = plan;
                self.zone_sel = 0;
                self.band_sel = self
                    .plan
                    .zones
                    .first()
                    .and_then(|zone| zone.bands.iter().position(|band| band.chip == "20m"))
                    .unwrap_or(0);
                self.plan_note.clear();
                self.plan_note_ptt = false;
            }
            Err(err) => {
                self.plan_note = err;
                self.plan_note_ptt = false;
            }
        }
    }

    fn save_plan(&mut self) {
        match plan::save_file(Path::new(&self.plan_path), &self.plan) {
            Ok(()) => {
                if !self.plan_note_ptt {
                    self.plan_note.clear();
                }
            }
            Err(err) => {
                self.plan_note = err;
                self.plan_note_ptt = false;
            }
        }
    }

    fn browse_channels(&mut self, save: bool) {
        let dir = remembered_dir(&self.channel_dir);
        let dialog = rfd::FileDialog::new()
            .set_title(if save {
                "Save channels"
            } else {
                "Load channels"
            })
            .add_filter("Channels", &["json"])
            .set_directory(&dir);
        let picked = if save {
            let name = if self.channel_path.is_empty() {
                "channels.json".to_string()
            } else {
                file_label(&self.channel_path)
            };
            dialog.set_file_name(name).save_file()
        } else {
            dialog.pick_file()
        };
        let Some(path) = picked else {
            return;
        };
        if let Some(parent) = path.parent() {
            if parent.is_dir() {
                self.channel_dir = Some(parent.to_path_buf());
            }
        }
        self.channel_path = path.display().to_string();
        if save {
            self.save_channels();
        } else {
            self.load_channels();
        }
    }

    fn load_channels(&mut self) {
        match channels::load_file(Path::new(&self.channel_path)) {
            Ok(file) => {
                self.channels = file;
                self.channel_zone = 0;
                self.channel_sel = 0;
                self.channel_note.clear();
            }
            Err(err) => {
                self.channel_note = err;
            }
        }
    }

    fn save_channels(&mut self) {
        match channels::save_file(Path::new(&self.channel_path), &self.channels) {
            Ok(()) => self.channel_note.clear(),
            Err(err) => self.channel_note = err,
        }
    }

    fn channel_row(&self, ui: &mut egui::Ui, idx: usize) -> bool {
        let Some(channel) = self
            .channels
            .zones
            .get(self.channel_zone)
            .and_then(|zone| zone.channels.get(idx))
        else {
            return false;
        };
        let freq = format!("{:10.3}", channel.hz / 1_000_000.0);
        let slot = format!("{:>3}", idx + 1);
        let name = channel.name.clone();
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        let painter = ui.painter();
        if self.channel_sel == idx {
            painter.rect_filled(rect, egui::CornerRadius::same(4), HAM.gamma_multiply(0.28));
        } else if idx % 2 == 1 {
            painter.rect_filled(rect, egui::CornerRadius::ZERO, ui.visuals().faint_bg_color);
        }
        let text = format!("{slot}   {freq}   {name}");
        painter.text(
            rect.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::monospace(14.0),
            ui.visuals().text_color(),
        );
        response.clicked()
    }

    fn channel_editor(&mut self, ui: &mut egui::Ui, snap: &Snapshot, width: f32) {
        let frame = egui::Frame::group(ui.style());
        let chrome =
            frame.inner_margin.sum().x + frame.stroke.width * 2.0 + frame.outer_margin.sum().x;
        let inner = (width - chrome).max(0.0);
        let zone_idx = self.channel_zone;
        let sel = self.channel_sel;
        let Some(channel) = self
            .channels
            .zones
            .get(zone_idx)
            .and_then(|zone| zone.channels.get(sel))
        else {
            let mut add = false;
            frame.show(ui, |ui| {
                ui.set_min_width(inner);
                ui.label("No channel.");
                if ui.button("Add").clicked() {
                    add = true;
                }
            });
            if add {
                let hz = self.shown_freq(snap);
                let mode = snap.mode.clone();
                self.add_channel(hz, mode);
            }
            return;
        };
        let mut name = channel.name.clone();
        let mut mhz = channel.hz / 1_000_000.0;
        let current = channel.mode.clone();
        let modes = mode_choices(snap);
        let mut name_changed = false;
        let mut freq_changed = false;
        let mut picked = None;
        let mut add = false;
        let mut delete = false;
        frame.show(ui, |ui| {
            ui.set_min_width(inner);
            ui.label(egui::RichText::new(format!("Channel {}", sel + 1)).strong());
            name_changed = ui
                .horizontal(|ui| {
                    ui.label("Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut name)
                            .id_salt(("ch-name", zone_idx, sel))
                            .desired_width(180.0),
                    )
                })
                .inner
                .changed();
            freq_changed = mhz_drag(ui, "Frequency", &mut mhz);
            ui.horizontal(|ui| {
                ui.label("Mode");
                egui::ComboBox::from_id_salt(("ch-mode", zone_idx, sel))
                    .width(180.0)
                    .selected_text(if current.is_empty() {
                        "—".to_string()
                    } else {
                        current.clone()
                    })
                    .show_ui(ui, |ui| {
                        for mode in &modes {
                            if ui.selectable_label(current == *mode, mode).clicked() {
                                picked = Some(mode.clone());
                            }
                        }
                    });
            });
            ui.horizontal(|ui| {
                if ui.button("Add").clicked() {
                    add = true;
                }
                if ui.button("Delete").clicked() {
                    delete = true;
                }
            });
        });
        if name_changed {
            if let Some(channel) = self
                .channels
                .zones
                .get_mut(zone_idx)
                .and_then(|zone| zone.channels.get_mut(sel))
            {
                channel.name = name;
            }
        }
        if freq_changed {
            let hz = mhz * 1_000_000.0;
            if hz.is_finite() && hz > 0.0 {
                self.set_channel_hz(snap, hz);
            }
        }
        if let Some(mode) = picked {
            if let Some(channel) = self
                .channels
                .zones
                .get_mut(zone_idx)
                .and_then(|zone| zone.channels.get_mut(sel))
            {
                channel.mode = mode.clone();
            }
            if snap.online() && snap.caps.can_set_mode {
                self.send(Command::SetMode {
                    mode,
                    passband_hz: snap.passband_hz,
                });
            }
        }
        if delete {
            self.delete_channel();
        } else if add {
            let hz = self.shown_freq(snap);
            let mode = snap.mode.clone();
            self.add_channel(hz, mode);
        }
    }

    fn add_channel(&mut self, hz: f64, mode: String) {
        if !hz.is_finite() || hz <= 0.0 {
            self.channel_note = "Channel frequency must be greater than zero.".into();
            return;
        }
        let Some(zone) = self.channels.zones.get_mut(self.channel_zone) else {
            return;
        };
        zone.channels.push(channels::Channel {
            name: String::new(),
            hz,
            mode,
        });
        self.channel_sel = zone.channels.len() - 1;
        self.channel_note.clear();
    }

    fn delete_channel(&mut self) {
        let Some(zone) = self.channels.zones.get_mut(self.channel_zone) else {
            return;
        };
        if self.channel_sel < zone.channels.len() {
            zone.channels.remove(self.channel_sel);
            if self.channel_sel >= zone.channels.len() && self.channel_sel > 0 {
                self.channel_sel -= 1;
            }
        }
    }

    /// Stores `hz` on the selected memory. A frequency the rig cannot receive
    /// stays in the file and is not sent.
    fn set_channel_hz(&mut self, snap: &Snapshot, hz: f64) {
        if !hz.is_finite() || hz <= 0.0 {
            return;
        }
        let Some(channel) = self
            .channels
            .zones
            .get_mut(self.channel_zone)
            .and_then(|zone| zone.channels.get_mut(self.channel_sel))
        else {
            return;
        };
        channel.hz = hz;
        if snap.caps.allow_freq(hz).is_some() {
            self.tune(snap, hz);
        }
    }

    fn recall_channel(&mut self, snap: &Snapshot, idx: usize) {
        self.channel_sel = idx;
        let Some(channel) = self
            .channels
            .zones
            .get(self.channel_zone)
            .and_then(|zone| zone.channels.get(idx))
        else {
            return;
        };
        let hz = channel.hz;
        let mode = channel.mode.clone();
        if hz.is_finite() && hz > 0.0 && snap.caps.allow_freq(hz).is_some() {
            self.tune(snap, hz);
            if snap.online() && snap.caps.can_set_mode && !mode.is_empty() {
                self.send(Command::SetMode {
                    mode,
                    passband_hz: snap.passband_hz,
                });
            }
        }
    }

    fn band_row(&self, ui: &mut egui::Ui, idx: usize) -> bool {
        let Some(band) = self
            .plan
            .zones
            .get(self.zone_sel)
            .and_then(|zone| zone.bands.get(idx))
        else {
            return false;
        };
        let freq = format!("{:10.3}", band.home_hz / 1_000_000.0);
        let slot = format!("{:>3}", idx + 1);
        let name = band.name.clone();
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        let painter = ui.painter();
        if self.band_sel == idx {
            painter.rect_filled(rect, egui::CornerRadius::same(4), HAM.gamma_multiply(0.28));
        } else if idx % 2 == 1 {
            painter.rect_filled(rect, egui::CornerRadius::ZERO, ui.visuals().faint_bg_color);
        }
        let text = format!("{slot}   {freq}   {name}");
        painter.text(
            rect.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::monospace(14.0),
            ui.visuals().text_color(),
        );
        response.clicked()
    }

    fn band_editor(&mut self, ui: &mut egui::Ui, snap: &Snapshot, width: f32) {
        let frame = egui::Frame::group(ui.style());
        let chrome =
            frame.inner_margin.sum().x + frame.stroke.width * 2.0 + frame.outer_margin.sum().x;
        let inner = (width - chrome).max(0.0);
        let sel = self.band_sel;
        let Some(band) = self
            .plan
            .zones
            .get(self.zone_sel)
            .and_then(|zone| zone.bands.get(sel))
        else {
            frame.show(ui, |ui| {
                ui.set_min_width(inner);
                self.plan_save_row(ui);
                ui.label("No band.");
            });
            return;
        };
        let title = band.name.clone();
        let mut name = band.name.clone();
        let mut home_mhz = band.home_hz / 1_000_000.0;
        let mut lo_mhz = band.lo_hz / 1_000_000.0;
        let mut hi_mhz = band.hi_hz / 1_000_000.0;
        let current = band.mode.clone();
        let modes = mode_choices(snap);
        let mut name_changed = false;
        let mut home_changed = false;
        let mut lo_changed = false;
        let mut hi_changed = false;
        let mut picked = None;
        frame.show(ui, |ui| {
            ui.set_min_width(inner);
            self.plan_save_row(ui);
            ui.label(egui::RichText::new(title).strong());
            name_changed = ui
                .horizontal(|ui| {
                    ui.label("Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut name)
                            .id_salt(("band-name", self.zone_sel, sel))
                            .desired_width(180.0),
                    )
                })
                .inner
                .changed();
            home_changed = mhz_drag(ui, "Home", &mut home_mhz);
            lo_changed = mhz_drag(ui, "Low", &mut lo_mhz);
            hi_changed = mhz_drag(ui, "High", &mut hi_mhz);
            ui.horizontal(|ui| {
                ui.label("Mode");
                egui::ComboBox::from_id_salt(("band-mode", self.zone_sel, sel))
                    .width(180.0)
                    .selected_text(if current.is_empty() {
                        "—".to_string()
                    } else {
                        current.clone()
                    })
                    .show_ui(ui, |ui| {
                        for mode in &modes {
                            if ui.selectable_label(current == *mode, mode).clicked() {
                                picked = Some(mode.clone());
                            }
                        }
                    });
            });
        });
        if name_changed {
            if let Some(band) = self
                .plan
                .zones
                .get_mut(self.zone_sel)
                .and_then(|zone| zone.bands.get_mut(sel))
            {
                band.name = name;
            }
        }
        if lo_changed {
            if let Some(band) = self
                .plan
                .zones
                .get_mut(self.zone_sel)
                .and_then(|zone| zone.bands.get_mut(sel))
            {
                band.lo_hz = lo_mhz * 1_000_000.0;
            }
        }
        if hi_changed {
            if let Some(band) = self
                .plan
                .zones
                .get_mut(self.zone_sel)
                .and_then(|zone| zone.bands.get_mut(sel))
            {
                band.hi_hz = hi_mhz * 1_000_000.0;
            }
        }
        if home_changed {
            self.write_band_home(snap, sel, home_mhz * 1_000_000.0);
        }
        if let Some(mode) = picked {
            if let Some(band) = self
                .plan
                .zones
                .get_mut(self.zone_sel)
                .and_then(|zone| zone.bands.get_mut(sel))
            {
                band.mode = mode.clone();
            }
            if snap.online() && snap.caps.can_set_mode {
                self.send(Command::SetMode {
                    mode,
                    passband_hz: snap.passband_hz,
                });
            }
        }
    }

    /// A click inside a band moves that band's home. Clicks in a gap do nothing.
    fn dial_home(&mut self, snap: &Snapshot, hz: f64) {
        if !hz.is_finite() || hz < 0.0 {
            return;
        }
        let Some(idx) = self.plan.band_index_at(self.zone_sel, hz) else {
            return;
        };
        self.band_sel = idx;
        self.write_band_home(snap, idx, hz);
    }

    fn write_band_home(&mut self, snap: &Snapshot, idx: usize, hz: f64) {
        if !hz.is_finite() || hz < 0.0 {
            return;
        }
        let Some(band) = self
            .plan
            .zones
            .get_mut(self.zone_sel)
            .and_then(|zone| zone.bands.get_mut(idx))
        else {
            return;
        };
        if band.tx_hz.is_none() || band.tx_hz == Some(band.home_hz) {
            band.tx_hz = Some(hz);
        }
        band.home_hz = hz;
        if snap.caps.allow_freq(hz).is_some() {
            self.tune(snap, hz);
        }
    }

    fn recall_band(&mut self, snap: &Snapshot, idx: usize) {
        self.band_sel = idx;
        let Some(band) = self
            .plan
            .zones
            .get(self.zone_sel)
            .and_then(|zone| zone.bands.get(idx))
        else {
            return;
        };
        let hz = band.home_hz;
        let mode = band.mode.clone();
        if snap.caps.allow_freq(hz).is_some() {
            self.tune(snap, hz);
            if snap.online() && snap.caps.can_set_mode && !mode.is_empty() {
                self.send(Command::SetMode {
                    mode,
                    passband_hz: snap.passband_hz,
                });
            }
        }
    }

    fn page_meters(&self, ui: &mut egui::Ui, snap: &Snapshot) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Meters").strong());
            let frac = strength_frac(snap.strength_db);
            meter(ui, &strength_label(snap.strength_db), frac, "");
            self.status_column(ui, snap);
        });
    }

    fn page_levels(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            self.sliders(ui, snap);
        });
    }

    fn status_column(&self, ui: &mut egui::Ui, snap: &Snapshot) {
        ui.vertical(|ui| {
            let alc = snap.levels.get("ALC").copied().unwrap_or(0.0);
            let swr = snap.levels.get("SWR").copied().unwrap_or(1.0);
            let pwr = snap
                .levels
                .get("RFPOWER_METER")
                .or_else(|| snap.levels.get("RFPOWER"))
                .copied()
                .unwrap_or(0.0);
            meter(ui, "ALC", alc as f32, "");
            meter(
                ui,
                "SWR",
                ((swr - 1.0) / 4.0).clamp(0.0, 1.0) as f32,
                &format!("{swr:.1}"),
            );
            meter(
                ui,
                "PWR",
                pwr as f32,
                &format!("{}", (pwr * 100.0).round() as i32),
            );
            ui.add_space(6.0);
            if let Some(roof) = &snap.roofing {
                ui.label(format!("Roofing: {roof}"));
            }
            alarm(ui, "Busy", false);
            alarm(ui, "Mic Eq", false);
            alarm(ui, "PLL unlock", false);
            alarm(ui, "High SWR", swr >= 3.0);
        });
    }

    fn vfo_column(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        ui.vertical(|ui| {
            let can = snap.online();
            two(ui, |ui| {
                op_button(
                    ui,
                    can && snap.caps.vfo_ops.contains("XCHG"),
                    "A <> B",
                    || {
                        self.send(Command::VfoOp("XCHG".into()));
                    },
                );
                rit_button(ui, snap, "R-6kHz", -6_000, &|cmd| self.send(cmd));
            });
            two(ui, |ui| {
                op_button(
                    ui,
                    can && snap.caps.vfo_ops.contains("CPY"),
                    "A > B",
                    || {
                        self.send(Command::VfoOp("CPY".into()));
                    },
                );
                rit_button(ui, snap, "R-3kHz", -3_000, &|cmd| self.send(cmd));
            });
            two(ui, |ui| {
                if ui.add_enabled(can, egui::Button::new("A > Mem")).clicked() {
                    self.local_mem = Some(snap.freq_hz);
                }
                rit_button(ui, snap, "R-15kHz", -15_000, &|cmd| self.send(cmd));
            });
            two(ui, |ui| {
                if ui
                    .add_enabled(
                        can && self.local_mem.is_some(),
                        egui::Button::new("Mem > A"),
                    )
                    .clicked()
                {
                    if let Some(hz) = self.local_mem {
                        self.tune(snap, hz);
                    }
                }
                rit_button(ui, snap, "R-Auto", 0, &|cmd| self.send(cmd));
            });
            two(ui, |ui| {
                let main = !snap.tx_vfo.contains('B');
                if ui
                    .add_enabled(
                        can && snap.caps.can_set_split,
                        egui::Button::new("TX main").selected(main),
                    )
                    .clicked()
                {
                    self.send(Command::SetSplit {
                        on: false,
                        tx_vfo: snap.vfo.clone(),
                    });
                }
                if ui
                    .add_enabled(
                        can && snap.caps.can_set_split,
                        egui::Button::new("TX sub").selected(!main),
                    )
                    .clicked()
                {
                    self.send(Command::SetSplit {
                        on: true,
                        tx_vfo: "VFOB".into(),
                    });
                }
            });
            two(ui, |ui| {
                for name in ["VFOA", "VFOB"] {
                    let label = if name == "VFOA" { "VFO A" } else { "VFO B" };
                    let on = snap.vfo.eq_ignore_ascii_case(name);
                    if ui
                        .add_enabled(can, egui::Button::new(label).selected(on))
                        .clicked()
                    {
                        self.send(Command::SetVfo(name.into()));
                    }
                }
            });
        });
    }

    /// One VFO: the digit row, the typed frequency, and the hamlib name with
    /// its mode. `main` is the current VFO (`get_vfo`). The other cluster is
    /// `snap.sub_vfo`. The S-meter is the receiver, so it stays on the current
    /// VFO. Hamlib reports one strength.
    fn vfo_cluster(&mut self, ui: &mut egui::Ui, snap: &Snapshot, main: bool, freq: f64) {
        let name = if main {
            snap.vfo.as_str()
        } else {
            snap.sub_vfo.as_str()
        };
        let mode = if main {
            snap.mode.as_str()
        } else {
            snap.sub_mode.as_str()
        };
        let cluster = ui.horizontal(|ui| {
            // The sub VFO stays at 70% of the main. Both are then 90% so the
            // VFO button frame fits between them on the default window.
            let scale = if main { 0.9 } else { 0.7 * 0.9 };
            scale_vfo(ui, scale);
            let hovered_digit = self.frequency(ui, snap, freq, main, scale);
            ui.add_space(18.0 * scale);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(vfo_label(name)).strong());
                ui.label(
                    egui::RichText::new(mode)
                        .size(22.0 * scale)
                        .strong()
                        .color(HAM),
                );
                if main && snap.rit_hz != 0 {
                    ui.label(format!("RIT {:+} Hz", snap.rit_hz));
                }
                if main {
                    let frac = strength_frac(snap.strength_db);
                    meter(ui, &strength_label(snap.strength_db), frac, "");
                }
            });
            hovered_digit
        });
        let can_tune = snap.online()
            && if main {
                snap.caps.can_set_freq
            } else {
                snap.caps.can_set_split
            };
        // The cluster, not just the digit buttons. A digit under the pointer
        // wins. Anywhere else on this VFO, the wheel turns the selected digit.
        let over_cluster = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|pos| cluster.response.rect.contains(pos));
        if over_cluster && can_tune {
            let notches = ui.input(|input| {
                wheel_notches(input.events.iter().filter_map(|event| match event {
                    egui::Event::MouseWheel { delta, phase, .. } => match phase {
                        egui::TouchPhase::End | egui::TouchPhase::Cancel => None,
                        _ => Some(delta.y),
                    },
                    _ => None,
                }))
            });
            if notches != 0 {
                let step = cluster.inner.unwrap_or(self.step_hz);
                self.step_digit(snap, freq, f64::from(notches) * step, main);
            }
        }
    }

    /// The digit under the pointer, if the pointer is on one.
    fn frequency(
        &mut self,
        ui: &mut egui::Ui,
        snap: &Snapshot,
        freq: f64,
        main: bool,
        scale: f32,
    ) -> Option<f64> {
        let can_tune = snap.online()
            && if main {
                snap.caps.can_set_freq
            } else {
                snap.caps.can_set_split
            };
        ui.vertical(|ui| {
            let digits = ui.horizontal(|ui| {
                let pointer = ui.input(|i| i.pointer.hover_pos());
                let mut hovered_step = None;
                for (ch, step) in digit_steps(freq) {
                    let Some(step) = step else {
                        ui.label(
                            egui::RichText::new(ch.to_string())
                                .size(48.0 * scale)
                                .strong()
                                .monospace(),
                        );
                        continue;
                    };
                    let selected = (step - self.step_hz).abs() < 0.1;
                    let text = egui::RichText::new(ch.to_string())
                        .size(48.0 * scale)
                        .strong()
                        .monospace();
                    let response =
                        ui.add_enabled(can_tune, egui::Button::new(text).selected(selected));
                    // The pointer position, not widget hover: a later hover
                    // sense would sit on top of these buttons and steal it.
                    if pointer.is_some_and(|pos| response.rect.contains(pos)) {
                        hovered_step = Some(step);
                    }
                    if response.clicked() {
                        // The clicked digit becomes the step, and this click
                        // turns that digit by one.
                        self.step_hz = step;
                        self.step_digit(snap, freq, step, main);
                    }
                }
                hovered_step
            });
            ui.horizontal(|ui| {
                let salt = if main { "main-freq" } else { "sub-freq" };
                let id = ui.id().with(salt);
                if !ui.memory(|mem| mem.has_focus(id)) {
                    let shown = format_hz(freq);
                    if main {
                        self.freq_text = shown;
                    } else {
                        self.sub_freq_text = shown;
                    }
                }
                let width = 160.0 * scale;
                let resp = if main {
                    let edit = egui::TextEdit::singleline(&mut self.freq_text)
                        .id_salt(salt)
                        .desired_width(width);
                    ui.add_enabled(can_tune, edit)
                } else {
                    let edit = egui::TextEdit::singleline(&mut self.sub_freq_text)
                        .id_salt(salt)
                        .desired_width(width);
                    ui.add_enabled(can_tune, edit)
                };
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let raw = if main {
                        self.freq_text.clone()
                    } else {
                        self.sub_freq_text.clone()
                    };
                    if let Some(hz) = parse_freq(&raw) {
                        if main {
                            self.tune(snap, hz);
                        } else {
                            self.tune_sub(snap, hz, None);
                        }
                    }
                }
                ui.label(egui::RichText::new(format!("step {}", step_label(self.step_hz))).weak());
            });
            digits.inner
        })
        .inner
    }

    fn buttons(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        ui.horizontal_wrapped(|ui| {
            self.func_btn(ui, snap, "ATU", "TUNER");
            self.band_step(ui, snap, "Band -", -1);
            self.band_step(ui, snap, "Band +", 1);
            self.dead(ui, "Dimmer");
            self.func_btn(ui, snap, "NR", "NR");
            self.preamp(ui, snap, "Pre 1", 10.0);
            self.preamp(ui, snap, "Pre 2", 20.0);
            self.ant(ui, snap, "Ant 1", 1);
            self.ant(ui, snap, "Ant 2", 2);
            self.func_btn(ui, snap, "Bk-in", "SBKIN");
            self.func_btn(ui, snap, "Fast", "FAGC");
            self.dead(ui, "NR sub");
            self.dead(ui, "CW Spot");
            self.dead(ui, "Keyer");
            self.dead(ui, "Nar");
            self.func_btn(ui, snap, "Proc", "COMP");
            self.dead(ui, "Ant Rx");
            self.dead(ui, "Chan +");
            self.dead(ui, "Chan -");
            self.func_btn(ui, snap, "Lock", "LOCK");
            self.func_btn(ui, snap, "Notch", "ANF");
            self.dead(ui, "QMR");
            self.dead(ui, "BC");
            self.ptt(ui, snap, "MOX");
            self.dead(ui, "PWR");
            self.dead(ui, "QMS");
            self.dead(ui, "Contour");
            self.func_btn(ui, snap, "Moni", "MON");
            self.dead(ui, "Scan");
            self.dead(ui, "Scan Dn");
        });
    }

    fn func_btn(&self, ui: &mut egui::Ui, snap: &Snapshot, label: &str, name: &str) {
        let on = snap.funcs.get(name).copied().unwrap_or(false);
        let enabled = snap.online() && snap.caps.has_func(name);
        if ui
            .add_enabled(enabled, egui::Button::new(label).selected(on))
            .clicked()
        {
            self.send(Command::SetFunc {
                name: name.into(),
                on: !on,
            });
        }
    }

    fn dead(&self, ui: &mut egui::Ui, label: &str) {
        ui.add_enabled(false, egui::Button::new(label));
    }

    fn ant(&self, ui: &mut egui::Ui, snap: &Snapshot, label: &str, n: i32) {
        let enabled = snap.online() && snap.caps.can_set_ant;
        if ui
            .add_enabled(enabled, egui::Button::new(label).selected(snap.ant == n))
            .clicked()
        {
            self.send(Command::SetAnt(n));
        }
    }

    fn preamp(&self, ui: &mut egui::Ui, snap: &Snapshot, label: &str, db: f64) {
        let enabled = snap.online() && snap.caps.level_range("PREAMP").is_some();
        let on = snap
            .levels
            .get("PREAMP")
            .is_some_and(|v| (*v - db).abs() < 0.5);
        if ui
            .add_enabled(enabled, egui::Button::new(label).selected(on))
            .clicked()
        {
            self.send(Command::SetLevel {
                name: "PREAMP".into(),
                value: db,
            });
        }
    }

    fn ptt(&mut self, ui: &mut egui::Ui, snap: &Snapshot, label: &str) {
        let enabled = snap.online() && snap.caps.can_set_ptt;
        if ui
            .add_enabled(enabled, egui::Button::new(label).selected(snap.ptt))
            .clicked()
        {
            let next = !snap.ptt;
            if next {
                if let Some(why) = self.plan.transmit_block_on(
                    self.zone_sel,
                    self.band_sel,
                    snap.freq_hz,
                    &snap.mode,
                ) {
                    self.plan_note = why;
                    self.plan_note_ptt = true;
                    return;
                }
            }
            self.plan_note.clear();
            self.plan_note_ptt = false;
            self.send(Command::SetPtt(next));
        }
    }

    fn band_step(&mut self, ui: &mut egui::Ui, snap: &Snapshot, label: &str, dir: isize) {
        let enabled = snap.online() && snap.caps.can_set_freq;
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            let freq = self.shown_freq(snap);
            let bands = plan::tunable_bands(&self.plan, &rx_pairs(snap));
            let Some(idx) = plan::adjacent_band(&bands, freq, dir as i32) else {
                return;
            };
            let band = &bands[idx];
            let remembered = self.band_mem.get(&band.name).copied();
            let hz = plan::band_tune_hz(band, remembered);
            // Hamlib has no band-up command. set_freq is the CAT command.
            // A frequency the radio will refuse does not move the chips.
            if snap.caps.allow_freq(hz).is_none() {
                return;
            }
            self.zone_sel = band.zone;
            self.band_sel = band.band;
            self.tune(snap, hz);
        }
    }
}

impl App {
    fn mode_box(&mut self, ui: &mut egui::Ui, snap: &Snapshot, label: &str, main: bool) {
        let current = if main {
            snap.mode.as_str()
        } else {
            snap.sub_mode.as_str()
        };
        let enabled = snap.online()
            && if main {
                snap.caps.can_set_mode
            } else {
                snap.caps.can_set_split && !snap.caps.modes.is_empty()
            };
        ui.add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt(label)
                .selected_text(format!("{label}: {current}"))
                .show_ui(ui, |ui| {
                    for mode in &snap.caps.modes {
                        if ui.selectable_label(mode == current, mode).clicked() {
                            let cmd = if main {
                                self.mode_hold = Some(mode.clone());
                                Command::SetMode {
                                    mode: mode.clone(),
                                    passband_hz: snap.passband_hz,
                                }
                            } else {
                                self.sub_mode_hold = Some(mode.clone());
                                Command::SetSubMode {
                                    mode: mode.clone(),
                                    passband_hz: snap.passband_hz,
                                }
                            };
                            self.send(cmd);
                        }
                    }
                });
        });
    }

    fn receiver_box(&self, ui: &mut egui::Ui, snap: &Snapshot) {
        let label = if snap.power_on { "RX" } else { "Off" };
        let enabled = snap.online() && snap.caps.can_set_power;
        ui.add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt("receiver")
                .selected_text(format!("Receiver: {label}"))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(snap.power_on, "RX").clicked() {
                        self.send(Command::SetPower(true));
                    }
                    if ui.selectable_label(!snap.power_on, "Off").clicked() {
                        self.send(Command::SetPower(false));
                    }
                });
        });
    }

    fn sliders(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        ui.label(egui::RichText::new("Levels").strong());
        self.level_slider(ui, snap, "AF", "AF gain (main)");
        self.level_slider(ui, snap, "RF", "RF gain (main)");
        self.agc(ui, snap);
        self.level_slider(ui, snap, "RFPOWER", "RF power");
        ui.add_space(6.0);
        fixed_slider(ui, "AF gain (sub)", 0.10);
        fixed_slider(ui, "RF gain (sub)", 1.0);
        ui.add_enabled(false, egui::Button::new("AGC (sub): Auto Fast"));
        self.level_slider(ui, snap, "NR", "Noise reduction");
        ui.add_space(6.0);
        self.level_slider(ui, snap, "IF", "IF shift");
        self.width_slider(ui, snap);
        ui.add_enabled(false, egui::Button::new("Contour: 20 Hz"));
        ui.add_space(6.0);
        self.level_slider(ui, snap, "MICGAIN", "Mic gain");
        self.level_slider(ui, snap, "VOXDELAY", "VOX delay");
        self.level_slider(ui, snap, "VOXGAIN", "VOX gain");
    }

    fn level_slider(&mut self, ui: &mut egui::Ui, snap: &Snapshot, name: &str, label: &str) {
        let Some(range) = snap.caps.level_range(name) else {
            let mut value = 0.0;
            ui.add_enabled(
                false,
                egui::Slider::new(&mut value, 0.0..=1.0).text(format!("{label}: —")),
            );
            return;
        };
        let mut value = self.shown_level(name, snap);
        let text = format!("{label}: {}", readout(name, value, range));
        let resp = ui.add(egui::Slider::new(&mut value, range.min..=range.max).text(text));
        if resp.changed() {
            self.hold_levels.insert(name.to_string(), value);
            self.send(Command::SetLevel {
                name: name.into(),
                value,
            });
        }
    }

    fn width_slider(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        if !snap.caps.can_set_mode {
            let mut value = 0.0;
            ui.add_enabled(
                false,
                egui::Slider::new(&mut value, 0.0..=1.0).text("Filter width: —"),
            );
            return;
        }
        let live = snap.passband_hz;
        let mut value = self.hold_pass.unwrap_or(live);
        if self.hold_pass.is_some_and(|held| (held - live).abs() <= 1) {
            self.hold_pass = None;
            value = live;
        }
        let text = format!("Filter width: {value} Hz");
        let resp = ui.add(egui::Slider::new(&mut value, 50..=16_000).text(text));
        if resp.changed() {
            self.hold_pass = Some(value);
            self.send(Command::SetMode {
                mode: snap.mode.clone(),
                passband_hz: value,
            });
        }
    }

    fn agc(&self, ui: &mut egui::Ui, snap: &Snapshot) {
        let enabled =
            snap.online() && snap.caps.level_range("AGC").is_some() && !snap.caps.agc.is_empty();
        let current = snap.levels.get("AGC").copied().unwrap_or(0.0).round() as i32;
        let name = snap
            .caps
            .agc
            .iter()
            .find(|(id, _)| *id == current)
            .map(|(_, name)| name.as_str())
            .unwrap_or("—");
        ui.add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt("agc")
                .selected_text(format!("AGC (main): {name}"))
                .show_ui(ui, |ui| {
                    for (id, label) in &snap.caps.agc {
                        if ui.selectable_label(*id == current, label).clicked() {
                            self.send(Command::SetLevel {
                                name: "AGC".into(),
                                value: f64::from(*id),
                            });
                        }
                    }
                });
        });
    }

    fn connect_window(&mut self, ctx: &egui::Context) {
        let mut open = self.connect_open;
        egui::Window::new("Connect to rigctld")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Hamlib rigctld is already running. This does not start it.");
                ui.horizontal(|ui| {
                    ui.label("Host");
                    ui.text_edit_singleline(&mut self.host);
                });
                ui.horizontal(|ui| {
                    ui.label("Port");
                    ui.text_edit_singleline(&mut self.port);
                });
                ui.horizontal(|ui| {
                    if ui.button("Connect").clicked() {
                        match self.port.trim().parse::<u16>() {
                            Ok(port) if port > 0 => {
                                self.switch(Switch::Connect {
                                    host: self.host.trim().to_string(),
                                    port,
                                });
                                self.connect_open = false;
                            }
                            _ => {
                                self.port = "4532".into();
                            }
                        }
                    }
                    if ui.button("Disconnect").clicked() {
                        self.switch(Switch::Disconnect);
                        self.connect_open = false;
                    }
                });
                ui.label("The hamlib dummy is scripts/dummy-rig.sh, which is rigctld -m 1 on 127.0.0.1:4532.");
            });
        self.connect_open = open;
    }

    fn favorite_window(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        let mut open = self.fav_open;
        egui::Window::new("Favorite")
            .open(&mut open)
            .show(ctx, |ui| {
                if let Some(fav) = &self.favorite {
                    ui.label(format!("{}  {}", format_hz(fav.hz), fav.mode));
                } else {
                    ui.label("Nothing saved.");
                }
                if ui.button("Save current").clicked() && snap.online() {
                    self.favorite = Some(Favorite {
                        hz: snap.freq_hz,
                        mode: snap.mode.clone(),
                        passband_hz: snap.passband_hz,
                    });
                }
                if ui
                    .add_enabled(
                        self.favorite.is_some() && snap.caps.can_set_freq,
                        egui::Button::new("Recall"),
                    )
                    .clicked()
                {
                    if let Some(fav) = &self.favorite {
                        let hz = fav.hz;
                        let mode = fav.mode.clone();
                        let pass = fav.passband_hz;
                        self.tune(snap, hz);
                        if snap.caps.can_set_mode {
                            self.send(Command::SetMode {
                                mode,
                                passband_hz: pass,
                            });
                        }
                    }
                }
            });
        self.fav_open = open;
    }
}

fn drive(shared: Arc<Mutex<Shared>>, wake: Arc<Condvar>, ctx: egui::Context) {
    let mut session = Session::Down {
        message: "connecting to 127.0.0.1:4532".into(),
    };
    let mut last = shared.lock().expect("ui").snap.clone();
    loop {
        let (cmds, switch) = {
            let guard = shared.lock().expect("ui");
            let pending = !guard.cmds.is_empty() || guard.switch.is_some();
            let mut guard = if pending {
                guard
            } else if session.is_down() {
                wake.wait(guard).expect("ui")
            } else {
                // The radio's own dial can move. Hamlib's default protocol does
                // not push, so the listen socket is polled on this timeout and
                // woken at once when the window queues a command.
                let (guard, _) = wake
                    .wait_timeout(guard, Duration::from_millis(200))
                    .expect("ui");
                guard
            };
            let cmds = std::mem::take(&mut guard.cmds);
            let switch = guard.switch.take();
            (cmds, switch)
        };

        if let Some(switch) = switch {
            session = match switch {
                Switch::Demo => Session::Demo(DemoRig::new()),
                Switch::Disconnect => Session::Down {
                    message: "disconnected".into(),
                },
                Switch::Connect { host, port } => match LiveRig::connect(&host, port) {
                    Ok(rig) => Session::Live(rig),
                    Err(err) => Session::Down {
                        message: err.to_string(),
                    },
                },
            };
        }

        let mut sent = None;
        let mut remote = None;
        for cmd in &cmds {
            sent = Some(cmd.cat_line());
            match session.apply(cmd) {
                Ok(()) => {}
                Err(RigError::Remote(msg)) => remote = Some(msg),
                Err(RigError::Io(msg)) => {
                    session = Session::Down { message: msg };
                    break;
                }
            }
        }

        let mut snap = match session.poll() {
            Ok(snap) => snap,
            Err(RigError::Io(msg)) => {
                session = Session::Down {
                    message: msg.clone(),
                };
                offline(&last, &msg)
            }
            Err(RigError::Remote(msg)) => {
                remote = Some(msg);
                last.clone()
            }
        };
        if let Some(msg) = remote {
            snap.message = msg;
        }
        if snap.kind == LinkKind::Down {
            let mut parked = last.clone();
            parked.kind = LinkKind::Down;
            parked.message = snap.message.clone();
            parked.caps = rigctl::Caps::empty();
            snap = parked;
        }
        // Polls replace the snapshot. Keep the last operator command, and
        // record a command from this pass after that replacement.
        snap.last_cat = sent.unwrap_or_else(|| last.last_cat.clone());
        if snap.kind != LinkKind::Down {
            last = snap.clone();
        } else {
            last.last_cat.clone_from(&snap.last_cat);
        }
        {
            let mut guard = shared.lock().expect("ui");
            guard.snap = snap;
        }
        ctx.request_repaint();
    }
}

enum Session {
    Demo(DemoRig),
    Live(LiveRig),
    Down { message: String },
}

impl Session {
    fn is_down(&self) -> bool {
        matches!(self, Session::Down { .. })
    }

    fn apply(&mut self, cmd: &Command) -> Result<(), RigError> {
        match self {
            Session::Demo(rig) => rig.apply(cmd),
            Session::Live(rig) => rig.apply(cmd),
            Session::Down { message } => Err(RigError::Io(message.clone())),
        }
    }

    fn poll(&mut self) -> Result<Snapshot, RigError> {
        match self {
            Session::Demo(rig) => rig.poll(),
            Session::Live(rig) => rig.poll(),
            Session::Down { message } => Ok(offline(&empty_snap(), message)),
        }
    }
}

/// Scale a VFO cluster's type, button padding, and gaps. Child rows copy this
/// style when they are created, so it has to be set on the cluster before
/// those rows exist.
fn scale_vfo(ui: &mut egui::Ui, scale: f32) {
    let style = ui.style_mut();
    style.spacing.item_spacing *= scale;
    style.spacing.button_padding *= scale;
    style.spacing.interact_size *= scale;
    style.spacing.icon_width *= scale;
    style.spacing.icon_width_inner *= scale;
    for font in style.text_styles.values_mut() {
        font.size *= scale;
    }
}

/// egui keeps a shorter widget on the top edge of a horizontal row. Pad it so
/// its middle lines up with `row_h`. `known_h` is last frame's measurement;
/// the returned height is this frame's.
fn place_centered(
    ui: &mut egui::Ui,
    row_h: f32,
    known_h: f32,
    add: impl FnOnce(&mut egui::Ui),
) -> f32 {
    let pad = ((row_h - known_h) * 0.5).max(0.0);
    let mut measured = 0.0;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if pad > 0.0 {
            ui.add_space(pad);
        }
        let piece = ui.scope(add);
        measured = piece.response.rect.height();
    });
    measured
}

fn offline(last: &Snapshot, message: &str) -> Snapshot {
    let mut snap = last.clone();
    snap.kind = LinkKind::Down;
    snap.message = message.to_string();
    snap.caps = rigctl::Caps::empty();
    snap
}

fn empty_snap() -> Snapshot {
    offline(
        &Snapshot {
            kind: LinkKind::Down,
            model: String::new(),
            endpoint: String::new(),
            message: String::new(),
            last_cat: String::new(),
            freq_hz: 0.0,
            mode: String::new(),
            passband_hz: 0,
            sub_mode: String::new(),
            sub_freq_hz: 0.0,
            sub_vfo: String::new(),
            vfo: String::new(),
            tx_vfo: String::new(),
            rit_hz: 0,
            ptt: false,
            power_on: false,
            ant: 1,
            strength_db: -54.0,
            levels: Default::default(),
            funcs: Default::default(),
            caps: rigctl::Caps::empty(),
            roofing: None,
        },
        "",
    )
}

fn alarm(ui: &mut egui::Ui, text: &str, hot: bool) {
    let color = if hot {
        egui::Color32::from_rgb(220, 80, 80)
    } else {
        egui::Color32::from_rgb(110, 60, 60)
    };
    ui.label(egui::RichText::new(text).color(color).size(12.0));
}

fn two(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(body);
}

fn op_button(ui: &mut egui::Ui, enabled: bool, label: &str, on_click: impl FnOnce()) {
    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
        on_click();
    }
}

fn rit_button(ui: &mut egui::Ui, snap: &Snapshot, label: &str, hz: i32, send: &impl Fn(Command)) {
    // R-Auto clears RIT. Leaving it selected whenever RIT is zero would light
    // the button for the whole session, so only the offset buttons select.
    let selected = hz != 0 && snap.rit_hz == hz;
    if ui
        .add_enabled(
            snap.online() && snap.caps.can_set_rit,
            egui::Button::new(label).selected(selected),
        )
        .clicked()
    {
        send(Command::SetRit(hz));
    }
}

fn step_label(step: f64) -> String {
    if step >= 1_000_000.0 {
        format!("{} MHz", step / 1_000_000.0)
    } else if step >= 1_000.0 {
        format!("{} kHz", step / 1_000.0)
    } else {
        format!("{step:.0} Hz")
    }
}

fn readout(name: &str, value: f64, range: rigctl::Range) -> String {
    match name {
        "IF" => format!("{value:.0} Hz"),
        "VOXDELAY" if range.max > 20.0 => format!("{value:.0} ms"),
        "SWR" => format!("{value:.1}"),
        "AGC" | "PREAMP" => format!("{value:.0}"),
        _ if range.max <= 1.0 && range.min >= 0.0 => format!("{}", (value * 100.0).round() as i32),
        _ => format!("{value:.0}"),
    }
}

/// Sub-receiver and contour have no hamlib level. They stay on the face at the
/// reference numbers and never send a command.
fn fixed_slider(ui: &mut egui::Ui, label: &str, value: f64) {
    let shown = (value * 100.0).round() as i32;
    let mut value = value;
    ui.add_enabled(
        false,
        egui::Slider::new(&mut value, 0.0..=1.0).text(format!("{label}: {shown}")),
    );
}
