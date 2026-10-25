//! Rotator screen, after the Yaesu G-1000DXA controller: amber azimuth dial,
//! OVERLAP lamp, SPEED, PRESET + START, LEFT (CCW) / RIGHT (CW), and a manual
//! tracking rate with START/STOP (an operator feature, not in the Yaesu manuals).
//!
//! It drives the rotator selected in Config Automation through `rot::Hub`
//! (`rotctld`). Test it with the hamlib Dummy (model 1). Issues #77-#83.
pub mod geom;

use std::sync::Arc;

use geom::{StopAt, Tick, TrackDir, TrackEnd};
use rustcars::{Cap, LcarsTheme, lcars_button, palette};

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

struct Track {
    start_az: f64,
    t0: f64,
    rate_dps: f64,
    dir: TrackDir,
    preset_az: f64,
    last_sent: f64,
}

pub struct Rotator {
    theme: LcarsTheme,
    hub: Option<Arc<rot::Hub>>,
    stop_at: StopAt,
    /// SPEED, seconds per 360.
    speed_s: f64,
    preset: f64,
    rate: f64,
    per_minute: bool,
    dir: TrackDir,
    track: Option<Track>,
    jog: Option<Spin>,
    note: String,
}

impl Rotator {
    pub fn new() -> Self {
        Self {
            theme: LcarsTheme::default(),
            hub: None,
            stop_at: StopAt::South,
            speed_s: 70.0,
            preset: 0.0,
            rate: 15.0,
            per_minute: true,
            dir: TrackDir::Cw,
            track: None,
            jog: None,
            note: String::new(),
        }
    }

    pub fn set_hub(&mut self, hub: Arc<rot::Hub>) {
        self.hub = Some(hub);
    }

    fn rate_dps(&self) -> f64 {
        if self.per_minute { self.rate / 60.0 } else { self.rate }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let rows = self.hub.as_ref().map(|hub| hub.view().rows).unwrap_or_default();
        let row = rows
            .iter()
            .find(|row| row.selected)
            .or_else(|| rows.first())
            .cloned();
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
        let el = row.as_ref().and_then(|row| row.elevation).unwrap_or(0.0);
        let id = row.as_ref().filter(|row| row.port.is_some()).map(|row| row.id);
        self.step_track(ui, id, el);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.bg))
            .show(ui, |ui| {
                let full = ui.available_rect_before_wrap();
                let panel_w = 300.0_f32.min(full.width() * 0.45);
                let dial_rect = egui::Rect::from_min_max(
                    full.min,
                    egui::pos2(full.right() - panel_w - t.main_gap, full.bottom()),
                );
                let panel = egui::Rect::from_min_max(
                    egui::pos2(full.right() - panel_w, full.top()),
                    full.max,
                );
                let preset_az = az.map(|az| geom::az_for_bearing(self.preset, az, self.stop_at));
                paint_dial(ui.painter(), dial_rect, az, Some(self.preset), self.stop_at, preset_az);
                ui.scope_builder(egui::UiBuilder::new().max_rect(panel), |ui| {
                    egui::ScrollArea::vertical().id_salt("rotator-panel").show(ui, |ui| {
                        self.controls(ui, id, az, el);
                    });
                });
            });
    }

    fn step_track(&mut self, ui: &egui::Ui, id: Option<u64>, el: f64) {
        let Some(track) = self.track.as_mut() else {
            return;
        };
        let (Some(id), Some(hub)) = (id, self.hub.as_ref()) else {
            self.track = None;
            self.note = "Tracking stopped: rotator not running.".into();
            return;
        };
        let now = ui.input(|input| input.time);
        let (target, end) = geom::track_target(
            track.start_az,
            track.rate_dps,
            now - track.t0,
            track.dir,
            track.preset_az,
            self.stop_at,
        );
        if (target - track.last_sent).abs() >= TRACK_STEP || end != TrackEnd::Running {
            hub.set_pos(id, target, el);
            track.last_sent = target;
        }
        match end {
            TrackEnd::Running => {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            TrackEnd::AtPreset => {
                self.track = None;
                self.note = "Tracking reached the preset.".into();
            }
            TrackEnd::AtLimit => {
                self.track = None;
                self.note = "Tracking stopped at the travel stop.".into();
            }
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui, id: Option<u64>, az: Option<f64>, el: f64) {
        let t = self.theme;
        let key = egui::vec2(132.0, t.bar_thin);
        let live = id.is_some();
        let hub = self.hub.clone();
        let heading = |ui: &mut egui::Ui, text: &str| {
            ui.label(egui::RichText::new(text).color(AMBER).size(t.header_size * 0.6));
        };

        heading(ui, "AZIMUTH");
        let text = match az {
            Some(az) => format!(
                "{:05.1}°  (travel {:.0}° of 450)",
                geom::bearing(az),
                geom::travel(az, self.stop_at)
            ),
            None => "---".into(),
        };
        ui.label(egui::RichText::new(text).color(t.text).size(t.data_size));
        if let Some(az) = az {
            if geom::out_of_range(az, self.stop_at) {
                ui.label(
                    egui::RichText::new(format!(
                        "Reported {az:.1}° is outside the G-1000 travel for this stop setting."
                    ))
                    .color(LAMP_ON),
                );
            }
        }
        ui.horizontal(|ui| {
            ui.label("Stopper:");
            ui.selectable_value(&mut self.stop_at, StopAt::South, "South (factory)");
            ui.selectable_value(&mut self.stop_at, StopAt::North, "North");
        });
        ui.add_space(t.main_gap);

        heading(ui, "SPEED");
        ui.horizontal(|ui| {
            ui.label("HIGH");
            // Reversed so right is faster, as on the knob: LOW .. HIGH.
            let mut low_high = geom::SLOWEST_S + geom::FASTEST_S - self.speed_s;
            ui.add(
                egui::Slider::new(&mut low_high, geom::FASTEST_S..=geom::SLOWEST_S)
                    .show_value(false),
            );
            self.speed_s = geom::SLOWEST_S + geom::FASTEST_S - low_high;
            ui.label("LOW");
        });
        ui.label(format!(
            "{:.0} s per 360°  ({:.1}°/s)",
            self.speed_s,
            geom::slew_dps(self.speed_s)
        ));
        ui.add_space(t.main_gap);

        heading(ui, "LEFT (CCW) / (CW) RIGHT");
        ui.horizontal(|ui| {
            for (spin, label, cap) in [(Spin::Ccw, "LEFT", Cap::Left), (Spin::Cw, "RIGHT", Cap::Right)] {
                let resp = key_button(ui, &t, label, palette::SLATE, key * egui::vec2(0.7, 1.0), cap, live);
                let held = live && resp.is_pointer_button_down_on();
                if let (Some(id), Some(hub)) = (id, hub.as_ref()) {
                    if held && self.jog != Some(spin) {
                        // As on the controller, LEFT/RIGHT aborts a preset run.
                        self.track = None;
                        hub.turn(id, spin, geom::hamlib_speed(self.speed_s));
                        self.jog = Some(spin);
                    } else if !held && self.jog == Some(spin) {
                        hub.stop(id);
                        self.jog = None;
                    }
                }
            }
        });
        ui.add_space(t.main_gap);

        heading(ui, "PRESET");
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.preset)
                    .range(0.0..=359.9)
                    .speed(1.0)
                    .suffix("°"),
            );
            if key_button(ui, &t, "START", palette::TAN, key * egui::vec2(0.7, 1.0), Cap::None, live)
                .clicked()
            {
                if let (Some(id), Some(hub), Some(az)) = (id, hub.as_ref(), az) {
                    self.track = None;
                    let target = geom::az_for_bearing(self.preset, az, self.stop_at);
                    hub.set_pos(id, target, el);
                    self.note = if geom::path_enters_overlap(az, target, self.stop_at) {
                        format!("Preset {:.0}° -> az {target:.1}: path enters OVERLAP.", self.preset)
                    } else {
                        format!("Preset {:.0}° -> az {target:.1}.", self.preset)
                    };
                }
            }
        });
        ui.add_space(t.main_gap);

        heading(ui, "MANUAL TRACKING");
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.rate).range(0.0..=600.0).speed(0.1));
            ui.selectable_value(&mut self.per_minute, true, "°/min");
            ui.selectable_value(&mut self.per_minute, false, "°/s");
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.dir, TrackDir::Ccw, "CCW");
            ui.selectable_value(&mut self.dir, TrackDir::Cw, "CW");
            ui.selectable_value(&mut self.dir, TrackDir::ToPreset, "To preset");
        });
        let (rate, clamped) = geom::clamp_rate(self.rate_dps(), self.speed_s);
        if clamped {
            ui.label(
                egui::RichText::new(format!("Rate clamped to {rate:.2}°/s by SPEED."))
                    .color(AMBER),
            );
        }
        if let Some(az) = az {
            if let Some(secs) = geom::secs_to_limit(az, rate, self.dir, self.stop_at) {
                ui.label(format!("Travel stop in {:.0} s at this rate.", secs));
            }
        }
        let running = self.track.is_some();
        let label = if running { "STOP" } else { "START" };
        let color = if running { palette::BRICK } else { palette::OLIVE };
        if key_button(ui, &t, label, color, key, Cap::Both, live).clicked() {
            if running {
                self.track = None;
                if let (Some(id), Some(hub)) = (id, hub.as_ref()) {
                    hub.stop(id);
                }
                self.note = "Tracking stopped.".into();
            } else if let Some(az) = az {
                let now = ui.input(|input| input.time);
                self.track = Some(Track {
                    start_az: az,
                    t0: now,
                    rate_dps: rate,
                    dir: self.dir,
                    preset_az: geom::az_for_bearing(self.preset, az, self.stop_at),
                    last_sent: az,
                });
                self.note = format!("Tracking at {rate:.3}°/s.");
            }
        }
        ui.add_space(t.main_gap);
        if key_button(ui, &t, "STOP ALL", palette::BRICK, key, Cap::Both, live).clicked() {
            self.track = None;
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

fn at(center: egui::Pos2, bearing: f64, r: f32) -> egui::Pos2 {
    let (x, y) = geom::dial_xy(bearing, r as f64);
    center + egui::vec2(x as f32, y as f32)
}

/// The G-1000DXA face. `preset_az` is the commanded azimuth for the preset,
/// used to say whether the preset lands in the overlap.
fn paint_dial(
    painter: &egui::Painter,
    rect: egui::Rect,
    az: Option<f64>,
    preset: Option<f64>,
    stop: StopAt,
    preset_az: Option<f64>,
) {
    let center = rect.center();
    let r = (rect.width().min(rect.height()) * 0.5 - 8.0).max(40.0);
    let overlap = az.is_some_and(|az| geom::in_overlap(az, stop));
    painter.circle_filled(center, r, BEZEL);
    painter.circle_filled(center, r * 0.93, FACE);
    painter.circle_stroke(center, r * 0.93, egui::Stroke::new(1.5, AMBER_DIM));

    // Overlap sector: the bearings the rotator can reach twice.
    let (a, b) = stop.overlap_bearings();
    let span = (b - a).rem_euclid(360.0);
    let arc: Vec<egui::Pos2> = (0..=45)
        .map(|i| at(center, a + span * i as f64 / 45.0, r * 0.89))
        .collect();
    let arc_color = if overlap { LAMP_ON } else { LAMP_OFF };
    painter.add(egui::Shape::line(arc, egui::Stroke::new(3.0, arc_color)));

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
        let color = if preset_az.is_some_and(|az| geom::in_overlap(az, stop)) {
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
        painter.text(center, egui::Align2::CENTER_CENTER, "NO POSITION", font(r * 0.08), AMBER_DIM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
