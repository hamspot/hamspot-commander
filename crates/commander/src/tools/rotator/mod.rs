//! Rotator screen, after the Yaesu G-1000DXA controller: amber azimuth dial
//! over an elevation globe, red LED AZ/EL readouts, OVERLAP lamp, SPEED,
//! PRESET + START, LEFT/RIGHT and UP/DOWN, and separate AZ and EL manual
//! tracking rates (an operator feature, not in the Yaesu manuals).
//!
//! It drives the rotator selected in Config Automation through `rot::Hub`
//! (`rotctld`). Travel limits come from hamlib when it reports sane ones, and
//! otherwise from the model preset. Issues #77-#86.
pub mod geom;
mod horizon;
mod led;

use std::sync::Arc;

use geom::{Dir, Limits, Model, Source, Tick, TrackEnd};
use rustcars::{Cap, LcarsTheme, lcars_button, palette};
use serde::{Deserialize, Serialize};

use crate::rot::{self, Spin};

const AMBER: egui::Color32 = egui::Color32::from_rgb(0xFF, 0xA8, 0x2E);
const AMBER_DIM: egui::Color32 = egui::Color32::from_rgb(0x8A, 0x5A, 0x18);
const FACE: egui::Color32 = egui::Color32::from_rgb(0x10, 0x10, 0x12);
const BEZEL: egui::Color32 = egui::Color32::from_rgb(0x3A, 0x3A, 0x3E);
const INDEX_ORANGE: egui::Color32 = egui::Color32::from_rgb(0xFF, 0x60, 0x10);
const MARK_BLUE: egui::Color32 = egui::Color32::from_rgb(0x30, 0x90, 0xFF);
const LAMP_ON: egui::Color32 = egui::Color32::from_rgb(0xFF, 0x20, 0x20);
const LAMP_OFF: egui::Color32 = egui::Color32::from_rgb(0x40, 0x10, 0x10);
const PRESET_GREEN: egui::Color32 = egui::Color32::from_rgb(0x50, 0xD0, 0x70);

/// Send a new tracking target once it moves this far.
const TRACK_STEP: f64 = 0.5;

/// One axis of manual tracking.
struct Track {
    start: f64,
    t0: f64,
    rate_dps: f64,
    dir: Dir,
    preset: f64,
    last_sent: f64,
}

/// Rate setting for one axis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rate {
    pub value: f64,
    pub per_minute: bool,
    pub dir: Dir,
}

impl Rate {
    fn dps(self) -> f64 {
        if self.per_minute { self.value / 60.0 } else { self.value }
    }
}

/// What the tab saves between runs (eframe storage, key `rotator_settings`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub model: Model,
    pub show_travel: bool,
    pub speed_s: f64,
    pub preset_az: f64,
    pub preset_el: f64,
    pub az_rate: Rate,
    pub el_rate: Rate,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: Model::G1000South,
            show_travel: Model::G1000South.default_show_travel(),
            speed_s: 70.0,
            preset_az: 0.0,
            preset_el: 0.0,
            az_rate: Rate { value: 15.0, per_minute: true, dir: Dir::Plus },
            el_rate: Rate { value: 5.0, per_minute: true, dir: Dir::Plus },
        }
    }
}

pub struct Rotator {
    theme: LcarsTheme,
    hub: Option<Arc<rot::Hub>>,
    pub settings: Settings,
    az_track: Option<Track>,
    el_track: Option<Track>,
    jog: Option<Spin>,
    note: String,
}

fn hamlib_limits(row: &rot::RotatorRow) -> Option<Limits> {
    row.limits.map(|l| Limits { az_lo: l.min_az, az_hi: l.max_az, el_lo: l.min_el, el_hi: l.max_el })
}

impl Rotator {
    pub fn new() -> Self {
        Self {
            theme: LcarsTheme::default(),
            hub: None,
            settings: Settings::default(),
            az_track: None,
            el_track: None,
            jog: None,
            note: String::new(),
        }
    }

    pub fn set_hub(&mut self, hub: Arc<rot::Hub>) {
        self.hub = Some(hub);
    }

    /// Settings as JSON for eframe storage.
    pub fn settings_json(&self) -> String {
        serde_json::to_string(&self.settings).unwrap_or_default()
    }

    /// Load saved settings. Bad JSON keeps the defaults.
    pub fn load_settings_json(&mut self, text: &str) {
        if let Ok(settings) = serde_json::from_str::<Settings>(text) {
            self.settings = settings;
        }
    }

    /// One status line for the Tools page: the selected rotator and its state.
    pub fn status_line(&self) -> String {
        let rows = self.hub.as_ref().map(|hub| hub.view().rows).unwrap_or_default();
        match rows.iter().find(|row| row.selected).or_else(|| rows.first()) {
            Some(row) => {
                let az = row.azimuth.map(|a| format!("   AZ {a:.0}°")).unwrap_or_default();
                let el = row.elevation.map(|e| format!("   EL {e:.0}°")).unwrap_or_default();
                format!("ROTATOR {}  {}  {}{az}{el}", row.spec.name, row.model_label, row.status)
            }
            None => "NO ROTATOR: ADD ONE IN CONFIG AUTOMATION (DUMMY IS MODEL 1)".to_owned(),
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let rows = self.hub.as_ref().map(|hub| hub.view().rows).unwrap_or_default();
        let row = rows
            .iter()
            .find(|row| row.selected)
            .or_else(|| rows.first())
            .cloned();
        let (lim, source) =
            geom::effective(row.as_ref().and_then(hamlib_limits), self.settings.model.limits());
        super::screen_header(ui, "rotator_header", &t, |ui| {
            ui.horizontal(|ui| {
                lcars_button(
                    ui,
                    &t,
                    "Rotator",
                    palette::TAN,
                    egui::vec2(120.0, t.bar_thin),
                    Cap::Left,
                    true,
                );
                ui.add_space(t.main_gap);
                let label = match &row {
                    Some(row) => format!("{}  {}  {}", row.spec.name, row.model_label, row.status),
                    None => "No rotator. Add one in Config Automation (Dummy is model 1).".into(),
                };
                ui.label(egui::RichText::new(label).color(t.text));
            });
        });
        let az = row.as_ref().and_then(|row| row.azimuth);
        let el = row.as_ref().and_then(|row| row.elevation);
        let id = row.as_ref().filter(|row| row.port.is_some()).map(|row| row.id);
        self.step_track(ui, id, az, el, lim);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.bg))
            .show(ui, |ui| {
                let full = ui.available_rect_before_wrap();
                let panel_w = 320.0_f32.min(full.width() * 0.45);
                let dial_rect = egui::Rect::from_min_max(
                    full.min,
                    egui::pos2(full.right() - panel_w - t.main_gap, full.bottom()),
                );
                let panel = egui::Rect::from_min_max(
                    egui::pos2(full.right() - panel_w, full.top()),
                    full.max,
                );
                let s = &self.settings;
                let preset_az = az.map(|az| geom::az_for_bearing(s.preset_az, az, lim));
                paint_dial(ui.painter(), dial_rect);
                paint_globe(ui.painter(), dial_rect, el, Some(s.preset_el), lim.el_hi);
                // The azimuth scale, needle and lamp go back on top of the globe.
                paint_dial_top(ui.painter(), dial_rect, az, Some(s.preset_az), lim, preset_az);
                ui.scope_builder(egui::UiBuilder::new().max_rect(panel), |ui| {
                    egui::ScrollArea::vertical().id_salt("rotator-panel").show(ui, |ui| {
                        self.controls(ui, id, az, el, lim, source);
                    });
                });
            });
    }

    fn step_track(
        &mut self,
        ui: &egui::Ui,
        id: Option<u64>,
        az: Option<f64>,
        el: Option<f64>,
        lim: Limits,
    ) {
        if self.az_track.is_none() && self.el_track.is_none() {
            return;
        }
        let (Some(id), Some(hub), Some(az), Some(el)) = (id, self.hub.clone(), az, el) else {
            self.az_track = None;
            self.el_track = None;
            self.note = "Tracking stopped: rotator not running.".into();
            return;
        };
        let now = ui.input(|input| input.time);
        let mut send = false;
        let mut step = |track: &mut Option<Track>, range: (f64, f64), name: &str, note: &mut String| -> Option<f64> {
            let tr = track.as_mut()?;
            let (target, end) =
                geom::track_target(tr.start, tr.rate_dps, now - tr.t0, tr.dir, tr.preset, range);
            if (target - tr.last_sent).abs() >= TRACK_STEP || end != TrackEnd::Running {
                tr.last_sent = target;
                send = true;
            }
            match end {
                TrackEnd::Running => {}
                TrackEnd::AtPreset => {
                    *track = None;
                    *note = format!("{name} tracking reached the preset.");
                }
                TrackEnd::AtLimit => {
                    *track = None;
                    *note = format!("{name} tracking stopped at the limit.");
                }
            }
            Some(target)
        };
        let az_cmd = step(&mut self.az_track, lim.az(), "AZ", &mut self.note).unwrap_or(az);
        let el_cmd = step(&mut self.el_track, lim.el(), "EL", &mut self.note).unwrap_or(el);
        if send {
            hub.set_pos(id, az_cmd, el_cmd);
        }
        if self.az_track.is_some() || self.el_track.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn jog_pair(
        &mut self,
        ui: &mut egui::Ui,
        id: Option<u64>,
        pair: [(Spin, &str, Cap); 2],
        color: egui::Color32,
    ) {
        let t = self.theme;
        let key = egui::vec2(92.0, t.bar_thin);
        let live = id.is_some();
        let hub = self.hub.clone();
        ui.horizontal(|ui| {
            for (spin, label, cap) in pair {
                let resp = key_button(ui, &t, label, color, key, cap, live);
                let held = live && resp.is_pointer_button_down_on();
                if let (Some(id), Some(hub)) = (id, hub.as_ref()) {
                    if held && self.jog != Some(spin) {
                        // As on the controller, a jog key aborts preset and tracking runs.
                        self.az_track = None;
                        self.el_track = None;
                        hub.turn(id, spin, geom::hamlib_speed(self.settings.speed_s));
                        self.jog = Some(spin);
                    } else if !held && self.jog == Some(spin) {
                        hub.stop(id);
                        self.jog = None;
                    }
                }
            }
        });
    }

    /// Rate, unit, direction and START/STOP for one axis.
    #[allow(clippy::too_many_arguments)]
    fn track_row(
        &mut self,
        ui: &mut egui::Ui,
        el_axis: bool,
        id: Option<u64>,
        pos: Option<f64>,
        lim: Limits,
        preset_cmd: Option<f64>,
    ) {
        let t = self.theme;
        let key = egui::vec2(110.0, t.bar_thin);
        let live = id.is_some() && pos.is_some();
        let (name, minus, plus) = if el_axis { ("EL", "DOWN", "UP") } else { ("AZ", "CCW", "CW") };
        let range = if el_axis { lim.el() } else { lim.az() };
        let speed_s = self.settings.speed_s;
        let rate = if el_axis { &mut self.settings.el_rate } else { &mut self.settings.az_rate };
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(name).color(AMBER));
            ui.add(egui::DragValue::new(&mut rate.value).range(0.0..=600.0).speed(0.1));
            ui.selectable_value(&mut rate.per_minute, true, "°/min");
            ui.selectable_value(&mut rate.per_minute, false, "°/s");
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut rate.dir, Dir::Minus, minus);
            ui.selectable_value(&mut rate.dir, Dir::Plus, plus);
            ui.selectable_value(&mut rate.dir, Dir::ToPreset, "To preset");
        });
        let r = *rate;
        let (dps, clamped) = geom::clamp_rate(r.dps(), speed_s);
        if clamped {
            ui.label(egui::RichText::new(format!("Rate clamped to {dps:.2}°/s by SPEED.")).color(AMBER));
        }
        if let Some(p) = pos {
            if let Some(secs) = geom::secs_to_limit(p, dps, r.dir, range) {
                ui.label(format!("{name} limit in {secs:.0} s at this rate."));
            }
        }
        let track = if el_axis { &mut self.el_track } else { &mut self.az_track };
        let running = track.is_some();
        let label = if running { format!("{name} STOP") } else { format!("{name} START") };
        let color = if running { palette::BRICK } else { palette::OLIVE };
        if key_button(ui, &t, &label, color, key, Cap::Both, live).clicked() {
            if running {
                *track = None;
                self.note = format!("{name} tracking stopped.");
            } else if let Some(p) = pos {
                let now = ui.input(|input| input.time);
                *track = Some(Track {
                    start: p,
                    t0: now,
                    rate_dps: dps,
                    dir: r.dir,
                    preset: preset_cmd.unwrap_or(p).clamp(range.0, range.1),
                    last_sent: p,
                });
                self.note = format!("{name} tracking at {dps:.3}°/s.");
            }
        }
    }

    fn controls(
        &mut self,
        ui: &mut egui::Ui,
        id: Option<u64>,
        az: Option<f64>,
        el: Option<f64>,
        lim: Limits,
        source: Source,
    ) {
        let t = self.theme;
        let key = egui::vec2(132.0, t.bar_thin);
        let live = id.is_some();
        let hub = self.hub.clone();
        let heading = |ui: &mut egui::Ui, text: &str| {
            ui.label(egui::RichText::new(text).color(AMBER).size(t.header_size * 0.6));
        };

        self.readouts(ui, az, el, lim);
        ui.checkbox(&mut self.settings.show_travel, "AZ shows travel past 360 (off: bearing + OVL)");
        if let Some(az) = az {
            if geom::out_of_range(az, lim) {
                ui.label(
                    egui::RichText::new(format!(
                        "Reported {az:.1}° is outside the travel limits {:.0}…{:.0}.",
                        lim.az_lo, lim.az_hi
                    ))
                    .color(LAMP_ON),
                );
            }
        }
        let old_model = self.settings.model;
        ui.horizontal_wrapped(|ui| {
            ui.label("Model:");
            for m in geom::MODELS {
                ui.selectable_value(&mut self.settings.model, m, m.label());
            }
        });
        if self.settings.model != old_model {
            self.settings.show_travel = self.settings.model.default_show_travel();
        }
        let src = match source {
            Source::Hamlib => "limits: hamlib",
            Source::Preset => "limits: preset",
        };
        ui.label(
            egui::RichText::new(format!(
                "{src}  az {:.0}…{:.0}  el {:.0}…{:.0}",
                lim.az_lo, lim.az_hi, lim.el_lo, lim.el_hi
            ))
            .color(t.text)
            .size(12.0),
        );
        ui.add_space(t.main_gap);

        heading(ui, "SPEED");
        ui.horizontal(|ui| {
            ui.label("HIGH");
            // Reversed so right is faster, as on the knob: LOW .. HIGH.
            let mut low_high = geom::SLOWEST_S + geom::FASTEST_S - self.settings.speed_s;
            ui.add(
                egui::Slider::new(&mut low_high, geom::FASTEST_S..=geom::SLOWEST_S)
                    .show_value(false),
            );
            self.settings.speed_s = geom::SLOWEST_S + geom::FASTEST_S - low_high;
            ui.label("LOW");
        });
        ui.label(format!(
            "{:.0} s per 360°  ({:.1}°/s)",
            self.settings.speed_s,
            geom::slew_dps(self.settings.speed_s)
        ));
        ui.add_space(t.main_gap);

        heading(ui, "LEFT (CCW) / (CW) RIGHT");
        self.jog_pair(ui, id, [(Spin::Ccw, "LEFT", Cap::Left), (Spin::Cw, "RIGHT", Cap::Right)], palette::SLATE);
        heading(ui, "UP / DOWN");
        self.jog_pair(ui, id, [(Spin::Up, "UP", Cap::Left), (Spin::Down, "DOWN", Cap::Right)], palette::TEAL);
        ui.add_space(t.main_gap);

        self.settings.preset_el = self.settings.preset_el.clamp(lim.el_lo, lim.el_hi);
        heading(ui, "PRESET  AZ / EL");
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.settings.preset_az)
                    .range(0.0..=359.9)
                    .speed(1.0)
                    .suffix("°"),
            );
            ui.add(
                egui::DragValue::new(&mut self.settings.preset_el)
                    .range(lim.el_lo..=lim.el_hi)
                    .speed(0.5)
                    .suffix("°"),
            );
            if key_button(ui, &t, "START", palette::TAN, key * egui::vec2(0.7, 1.0), Cap::None, live)
                .clicked()
            {
                if let (Some(id), Some(hub), Some(az)) = (id, hub.as_ref(), az) {
                    self.az_track = None;
                    self.el_track = None;
                    let s = &self.settings;
                    let target = geom::az_for_bearing(s.preset_az, az, lim);
                    hub.set_pos(id, target, s.preset_el);
                    let tail = if geom::path_enters_overlap(az, target, lim) {
                        ": path enters OVERLAP."
                    } else {
                        "."
                    };
                    self.note = format!(
                        "Preset {:.0}° -> az {target:.1} el {:.1}{tail}",
                        s.preset_az, s.preset_el
                    );
                }
            }
        });
        ui.add_space(t.main_gap);

        heading(ui, "MANUAL TRACKING");
        let preset_az = az.map(|az| geom::az_for_bearing(self.settings.preset_az, az, lim));
        self.track_row(ui, false, id, az, lim, preset_az);
        ui.add_space(t.main_gap * 0.5);
        let preset_el = Some(self.settings.preset_el);
        self.track_row(ui, true, id, el, lim, preset_el);
        ui.add_space(t.main_gap);
        if key_button(ui, &t, "STOP ALL", palette::BRICK, key, Cap::Both, live).clicked() {
            self.az_track = None;
            self.el_track = None;
            self.jog = None;
            if let (Some(id), Some(hub)) = (id, hub.as_ref()) {
                hub.stop(id);
            }
        }
        if !self.note.is_empty() {
            ui.label(egui::RichText::new(&self.note).color(t.text));
        }
    }
}

/// Space left for a label: the rect minus the round ends of its caps.
fn label_area(rect: egui::Rect, cap: Cap) -> egui::Rect {
    let r = rect.height() * 0.5;
    let (left, right) = match cap {
        Cap::Left => (r, 0.0),
        Cap::Right => (0.0, r),
        Cap::Both => (r, r),
        Cap::None => (0.0, 0.0),
    };
    egui::Rect::from_min_max(rect.min + egui::vec2(left, 0.0), rect.max - egui::vec2(right, 0.0))
}

/// Top-left of a galley of `size` centered in `area`.
fn label_pos(area: egui::Rect, size: egui::Vec2) -> egui::Pos2 {
    area.center() - size * 0.5
}

/// A rotator key: the lcars_button shape, but the label is centered both ways
/// with padding and shrinks to fit (lcars_button puts it bottom right).
fn key_button(
    ui: &mut egui::Ui,
    t: &LcarsTheme,
    label: &str,
    base: egui::Color32,
    size: egui::Vec2,
    cap: Cap,
    enabled: bool,
) -> egui::Response {
    const PAD: f32 = 8.0;
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let fill = if !enabled {
        rustcars::tint(base, -0.6)
    } else if resp.is_pointer_button_down_on() {
        rustcars::tint(base, -0.25)
    } else if resp.hovered() {
        rustcars::tint(base, 0.3)
    } else {
        base
    };
    let radius = rustcars::cap_radius(cap, rect.height());
    let painter = ui.painter();
    painter.rect_filled(rect, radius, fill);
    if resp.has_focus() {
        painter.rect_stroke(
            rect.shrink(2.0),
            radius,
            egui::Stroke::new(2.0, t.text_hi),
            egui::StrokeKind::Inside,
        );
    }
    let ink = rustcars::ink_on(fill);
    let area = label_area(rect, cap).shrink(PAD.min(rect.height() * 0.2));
    let text = label.to_uppercase();
    let mut font = t.header_size.min(rect.height() - 2.0 * PAD).max(9.0);
    let mut galley = painter.layout_no_wrap(text.clone(), egui::FontId::proportional(font), ink);
    while (galley.size().x > area.width() || galley.size().y > rect.height()) && font > 9.0 {
        font -= 1.0;
        galley = painter.layout_no_wrap(text.clone(), egui::FontId::proportional(font), ink);
    }
    painter.galley(label_pos(area, galley.size()), galley, ink);
    resp
}

impl Rotator {
    /// Red LED readouts. AZ shows travel past 360 or the bearing (checkbox),
    /// with OVL in the overlap. EL shows up to 180.0, and FLIP past 90 with the
    /// equivalent az/el.
    fn readouts(&self, ui: &mut egui::Ui, az: Option<f64>, el: Option<f64>, lim: Limits) {
        let t = self.theme;
        let show_travel = self.settings.show_travel;
        let overlap = az.is_some_and(|az| geom::in_overlap(az, lim));
        let az_text = led::format_az(az.map(|az| geom::display_az(az, lim, show_travel)), !show_travel);
        let el_text = led::format_el(el);
        let flipped = el.is_some_and(led::el_flipped);
        let digit_h = 34.0;
        let small = egui::FontId::proportional(11.0);
        let dim = egui::Color32::from_rgb(0x90, 0x30, 0x28);
        for (name, text, cells, flag_on, flag) in [
            ("AZ", az_text, 4.0, overlap, "OVL"),
            ("EL", el_text, 4.0, flipped, "FLIP"),
        ] {
            let width = 250.0_f32.min(ui.available_width());
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, digit_h + 6.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.text(
                rect.left_center(),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(14.0),
                AMBER,
            );
            let n = led::cells(&text).len().max(cells as usize) as f32;
            let led_w = digit_h * 0.62 * n + 10.0;
            let led_rect = egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(28.0, 3.0),
                egui::vec2(led_w, digit_h),
            );
            led::paint(painter, led_rect, &text, led::LIT);
            painter.text(
                egui::pos2(led_rect.right() + 6.0, led_rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                flag,
                small.clone(),
                if flag_on { led::LIT } else { led::GHOST },
            );
            let secondary = match (name, az, el) {
                ("AZ", Some(az), _) if overlap && show_travel => {
                    Some(format!("BRG {}", led::format_az(Some(geom::bearing(az)), true).trim()))
                }
                ("EL", Some(az), Some(e)) if flipped => {
                    let (a, b) = geom::flip_equiv(az, e);
                    Some(format!("= {a:.0}/{b:.0}"))
                }
                _ => None,
            };
            if let Some(text) = secondary {
                painter.text(
                    egui::pos2(led_rect.right() + 6.0, led_rect.bottom() - 2.0),
                    egui::Align2::LEFT_BOTTOM,
                    text,
                    small.clone(),
                    dim,
                );
            }
        }
        if let Some(az) = az {
            ui.label(
                egui::RichText::new(format!(
                    "travel {:.0}° of {:.0}",
                    geom::travel(az, lim),
                    lim.az_hi - lim.az_lo
                ))
                .color(t.text)
                .size(12.0),
            );
        }
    }
}

fn paint_globe(
    painter: &egui::Painter,
    rect: egui::Rect,
    el: Option<f64>,
    preset_el: Option<f64>,
    max_el: f64,
) {
    let r = dial_radius(rect);
    horizon::paint(painter, rect.center(), r * GLOBE, el, preset_el, max_el);
}

/// Globe radius as a share of the dial radius. The 30-degree labels sit at
/// 0.63 with their inner edge near 0.58, so 0.52 leaves a dark gap.
const GLOBE: f32 = 0.52;

fn dial_radius(rect: egui::Rect) -> f32 {
    (rect.width().min(rect.height()) * 0.5 - 8.0).max(40.0)
}

fn at(center: egui::Pos2, bearing: f64, r: f32) -> egui::Pos2 {
    let (x, y) = geom::dial_xy(bearing, r as f64);
    center + egui::vec2(x as f32, y as f32)
}

/// The G-1000DXA face. `preset_az` is the commanded azimuth for the preset,
/// used to say whether the preset lands in the overlap.
fn paint_dial(painter: &egui::Painter, rect: egui::Rect) {
    let center = rect.center();
    let r = dial_radius(rect);
    painter.circle_filled(center, r, BEZEL);
    painter.circle_filled(center, r * 0.93, FACE);
    painter.circle_stroke(center, r * 0.93, egui::Stroke::new(1.5, AMBER_DIM));
}

/// Everything that draws over the elevation globe: the overlap arc, the
/// amber scale, crosshair, text, preset marker, needle and OVERLAP lamp.
fn paint_dial_top(
    painter: &egui::Painter,
    rect: egui::Rect,
    az: Option<f64>,
    preset: Option<f64>,
    lim: Limits,
    preset_az: Option<f64>,
) {
    let center = rect.center();
    let r = dial_radius(rect);
    let overlap = az.is_some_and(|az| geom::in_overlap(az, lim));

    // Overlap sector: the bearings the rotator can reach twice.
    if let Some((a, b)) = lim.overlap_bearings() {
        let span = (b - a).rem_euclid(360.0);
        let arc: Vec<egui::Pos2> = (0..=45)
            .map(|i| at(center, a + span * i as f64 / 45.0, r * 0.89))
            .collect();
        let arc_color = if overlap { LAMP_ON } else { LAMP_OFF };
        painter.add(egui::Shape::line(arc, egui::Stroke::new(3.0, arc_color)));
    }

    let font = |size: f32| egui::FontId::proportional(size);
    for (bearing, kind) in geom::ticks() {
        let (inner, width) = match kind {
            Tick::Major => (0.74, 2.5),
            Tick::Minor => (0.80, 1.5),
        };
        painter.line_segment(
            [at(center, bearing, r * inner), at(center, bearing, r * 0.86)],
            egui::Stroke::new(width, AMBER),
        );
        if kind == Tick::Major {
            let text = if bearing == 0.0 { "0°".to_string() } else { format!("{bearing:.0}") };
            painter.text(
                at(center, bearing, r * 0.63),
                egui::Align2::CENTER_CENTER,
                text,
                font(r * 0.085),
                AMBER,
            );
        }
    }
    // Crosshair.
    let thin = egui::Stroke::new(1.0, AMBER_DIM);
    painter.line_segment([at(center, 0.0, r * 0.55), at(center, 180.0, r * 0.55)], thin);
    painter.line_segment([at(center, 90.0, r * 0.55), at(center, 270.0, r * 0.55)], thin);
    // Orange index at 0/360 and blue mark at 90, on the bezel.
    painter.line_segment(
        [at(center, 0.0, r * 0.94), at(center, 0.0, r * 1.0)],
        egui::Stroke::new(3.0, INDEX_ORANGE),
    );
    painter.line_segment(
        [at(center, 90.0, r * 0.94), at(center, 90.0, r * 1.0)],
        egui::Stroke::new(3.0, MARK_BLUE),
    );
    painter.text(
        center + egui::vec2(0.0, r * 0.30),
        egui::Align2::CENTER_CENTER,
        "AZIMUTH INDICATOR",
        font(r * 0.065),
        AMBER,
    );
    painter.text(
        center + egui::vec2(0.0, r * 0.39),
        egui::Align2::CENTER_CENTER,
        "EACH DIVISION 5°",
        font(r * 0.045),
        AMBER,
    );
    // Preset marker on the scale.
    if let Some(p) = preset {
        let tip = at(center, p, r * 0.87);
        let base = r * 0.95;
        let left = at(center, p - 2.5, base);
        let right = at(center, p + 2.5, base);
        let color = if preset_az.is_some_and(|az| geom::in_overlap(az, lim)) {
            LAMP_ON
        } else {
            PRESET_GREEN
        };
        painter.add(egui::Shape::convex_polygon(vec![tip, left, right], color, egui::Stroke::NONE));
    }
    // Needle.
    if let Some(az) = az {
        let b = geom::bearing(az);
        let needle = egui::Color32::from_rgb(0xE8, 0xE8, 0xE8);
        painter.line_segment([at(center, b + 180.0, r * 0.12), at(center, b, r * 0.84)], egui::Stroke::new(3.0, needle));
        painter.circle_filled(center, r * 0.05, egui::Color32::from_gray(0x30));
    }
    // OVERLAP lamp, top right of the face as on the controller.
    let lamp_c = egui::pos2(rect.right() - r * 0.18 - 4.0, rect.top() + r * 0.1 + 4.0);
    let lamp = egui::Rect::from_center_size(lamp_c, egui::vec2(r * 0.12, r * 0.07));
    painter.rect_filled(lamp, egui::CornerRadius::same(2), if overlap { LAMP_ON } else { LAMP_OFF });
    painter.text(
        lamp_c + egui::vec2(0.0, r * 0.09),
        egui::Align2::CENTER_CENTER,
        "OVERLAP",
        font((r * 0.05).max(9.0)),
        egui::Color32::from_gray(0xC8),
    );
    if az.is_none() {
        // Below the globe's OFF flag, clear of the wings.
        painter.text(center + egui::vec2(0.0, r * 0.18), egui::Align2::CENTER_CENTER, "NO POSITION", font(r * 0.06), AMBER_DIM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_bad_json_keeps_defaults() {
        let mut r = Rotator::new();
        r.settings.model = Model::G5500;
        r.settings.el_rate.value = 2.5;
        r.settings.az_rate.dir = Dir::ToPreset;
        let text = r.settings_json();
        let mut back = Rotator::new();
        back.load_settings_json(&text);
        assert_eq!(back.settings, r.settings);
        let mut bad = Rotator::new();
        bad.load_settings_json("{nope");
        assert_eq!(bad.settings, Settings::default());
        let mut partial = Rotator::new();
        partial.load_settings_json(r#"{"model":"Generic"}"#);
        assert_eq!(partial.settings.model, Model::Generic);
        assert_eq!(partial.settings.speed_s, 70.0);
    }

    #[test]
    fn labels_center_in_the_flat_part_of_the_key() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(100.0, 40.0));
        let size = egui::vec2(30.0, 10.0);
        assert_eq!(label_pos(rect, size), egui::pos2(45.0, 35.0));
        assert_eq!(label_area(rect, Cap::None), rect);
        assert_eq!(label_area(rect, Cap::Both).center(), rect.center());
        assert_eq!(label_area(rect, Cap::Left).left(), 30.0);
        assert_eq!(label_area(rect, Cap::Right).right(), 90.0);
    }
}
