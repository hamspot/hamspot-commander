//! Painter and pointer handling for the RF patch bay (#92, #93).

use super::geom::{self, JACK_R, SNAP, TITLE_H};
use super::bind::{self, Binding};
use super::model::{self, Cable, Dir, Jack, Kind, PatchBay, Role, Signal};
use crate::amp::{self, AmpModel, AmplifierRow, AmplifierSpec, Control, Power};
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};

#[derive(Default)]
pub struct UiState {
    drag: Option<Drag>,
    selected_cable: Option<usize>,
    selected_dev: Option<u32>,
    note: String,
    amp_pick: usize,
    actions: Vec<Action>,
}

impl UiState {
    /// Operator requests from this frame, for the caller to carry out.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }
}

/// What the patch bay asks the app to do. Each comes from one click.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// "Add to Automation list": a new configured amplifier row.
    AddAmplifier(AmplifierSpec),
    /// Standby / Operate key on a bound box, by amplifier id. The caller
    /// resolves the id at send time and sends nothing if it is missing.
    Control(String, Control),
}

/// Configured amplifiers (live rows) and the single-sourced model list.
pub struct AmpLink<'a> {
    pub rows: &'a [AmplifierRow],
    pub models: &'a [AmpModel],
}

fn power_led(power: Option<Power>) -> Color32 {
    match power {
        Some(Power::Operate) => TX,
        Some(Power::Standby) => Color32::from_rgb(90, 200, 110),
        Some(Power::On) => Color32::from_rgb(150, 190, 255),
        _ => Color32::from_rgb(70, 70, 76),
    }
}

enum Drag {
    Box { id: u32, grab: [f32; 2] },
    /// Cable being drawn from `fixed`. `lifted` is a cable taken off a jack.
    Cable { fixed: Jack, lifted: Option<Cable> },
}

const TX: Color32 = Color32::from_rgb(255, 176, 0);
const RX: Color32 = Color32::from_rgb(0, 206, 196);
const CTL: Color32 = Color32::from_rgb(200, 200, 210);
const LABEL: Color32 = Color32::from_rgb(226, 226, 226);
const PANEL: Color32 = Color32::from_rgb(14, 14, 16);
const BODY: Color32 = Color32::from_rgb(30, 30, 34);

fn kind_color(kind: Kind) -> Color32 {
    match kind {
        Kind::Radio => Color32::from_rgb(255, 153, 0),
        Kind::Amplifier => Color32::from_rgb(255, 120, 90),
        Kind::Switch => Color32::from_rgb(204, 153, 204),
        Kind::Tuner => Color32::from_rgb(153, 153, 255),
        Kind::Meter => Color32::from_rgb(255, 204, 153),
        Kind::Load => Color32::from_rgb(170, 170, 170),
        Kind::Antenna => Color32::from_rgb(153, 204, 255),
    }
}

/// Draw the frame. `radios` are the station's radio rows (name, can transmit).
/// Returns true when the stored layout changed.
pub fn show(ui: &mut egui::Ui, bay: &mut PatchBay, st: &mut UiState, radios: &[(String, bool)], link: &AmpLink) -> bool {
    let before = bay.clone();
    bay.sync_radios(radios);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("RF patch");
            ui.label(
                egui::RichText::new("VISUAL ONLY: cables here do not switch, key or tune any hardware")
                    .color(TX)
                    .strong(),
            );
        });
        toolbar(ui, bay, st, link);
        canvas(ui, bay, st, link);
        status(ui, bay, st, link);
    });
    bay.prune();
    *bay != before
}

fn toolbar(ui: &mut egui::Ui, bay: &mut PatchBay, st: &mut UiState, link: &AmpLink) {
    let models = link.models;
    let at = [220.0 + 30.0 * (bay.devices.len() % 6) as f32, 20.0 + 24.0 * (bay.devices.len() % 6) as f32];
    ui.horizontal_wrapped(|ui| {
        ui.label("Add:");
        egui::ComboBox::from_id_salt("patchbay_amp")
            .selected_text(models.get(st.amp_pick).map(model_name).unwrap_or_else(|| "Other amplifier".into()))
            .show_ui(ui, |ui| {
                for (i, m) in models.iter().enumerate() {
                    ui.selectable_value(&mut st.amp_pick, i, amp::model_button_label(m));
                }
                ui.selectable_value(&mut st.amp_pick, models.len(), "Other amplifier");
            });
        if ui.button("Amplifier").clicked() {
            let (model, name) = models.get(st.amp_pick).map(|m| (m.id, model_name(m))).unwrap_or((0, "Amplifier".into()));
            let id = bay.add(Kind::Amplifier, &name, at);
            if let Some(d) = bay.devices.iter_mut().find(|d| d.id == id) {
                d.model = model;
            }
            st.selected_dev = Some(id);
        }
        for kind in [Kind::Switch, Kind::Tuner, Kind::Meter, Kind::Load, Kind::Antenna] {
            if ui.button(kind.label()).clicked() {
                let n = bay.devices.iter().filter(|d| d.kind == kind).count() + 1;
                st.selected_dev = Some(bay.add(kind, &format!("{} {n}", kind.label()), at));
            }
        }
    });
    let Some(id) = st.selected_dev else { return };
    let Some(dev) = bay.devices.iter_mut().find(|d| d.id == id) else {
        st.selected_dev = None;
        return;
    };
    let mut remove = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{}:", dev.kind.label()));
        if dev.kind == Kind::Radio {
            ui.label(&dev.name);
            ui.label(if dev.tx { "can transmit" } else { "receive only" });
        } else {
            ui.add(egui::TextEdit::singleline(&mut dev.name).desired_width(140.0));
            ui.label("Bands");
            ui.add(egui::TextEdit::singleline(&mut dev.bands).hint_text("e.g. 2m/70cm").desired_width(100.0));
        }
        if dev.kind == Kind::Amplifier && dev.model != 0 {
            ui.label(format!("hamlib model {}", dev.model));
        }
        if dev.kind == Kind::Amplifier {
            amp_controls(ui, dev, st, link);
        }
        if dev.kind == Kind::Switch {
            ui.label("Ways");
            ui.add(egui::DragValue::new(&mut dev.ways).range(1..=8));
            ui.label("Position");
            for n in 1..=dev.ways.clamp(1, 8) {
                if ui.selectable_label(dev.sel == n, n.to_string()).clicked() {
                    dev.sel = n;
                }
            }
        }
        if dev.kind != Kind::Radio && ui.button("Remove").clicked() {
            remove = true;
        }
    });
    if remove {
        bay.remove_device(id);
        st.selected_dev = None;
        st.selected_cable = None;
    }
}

fn model_name(m: &AmpModel) -> String {
    format!("{} {}", m.mfg, m.model).trim().to_string()
}

/// Binding picker, Add to Automation list, and the explicit power keys for
/// the selected amplifier box. Nothing here sends on its own.
fn amp_controls(ui: &mut egui::Ui, dev: &mut model::Device, st: &mut UiState, link: &AmpLink) {
    ui.label("Configured amplifier");
    let label = |row: &AmplifierRow| format!("{}  [{}]", row.spec.name, &row.spec.id[..row.spec.id.len().min(6)]);
    let shown = match bind::binding(dev, link.rows) {
        Binding::Unbound => "unbound (visual only)".to_string(),
        Binding::Missing => "amplifier missing".to_string(),
        Binding::Bound(row) => label(row),
    };
    let before = dev.amp_id.clone();
    egui::ComboBox::from_id_salt(("patchbay_bind", dev.id)).selected_text(shown).show_ui(ui, |ui| {
        ui.selectable_value(&mut dev.amp_id, String::new(), "unbound (visual only)");
        for row in link.rows {
            ui.selectable_value(&mut dev.amp_id, row.spec.id.clone(), label(row))
                .on_hover_text(format!("amplifier id {}", row.spec.id));
        }
    });
    if dev.amp_id != before {
        dev.note.clear();
    }
    if !dev.note.is_empty() {
        ui.label(egui::RichText::new(format!("⚠ {}", dev.note)).color(TX));
    }
    match bind::binding(dev, link.rows) {
        Binding::Unbound => {
            if ui
                .button("Add to Automation list")
                .on_hover_text("Adds this amplifier to Config → Automation → Amplifiers and binds this box to it. Nothing is sent to the amplifier.")
                .clicked()
            {
                let taken: Vec<String> = link.rows.iter().map(|r| r.spec.name.clone()).collect();
                let spec = bind::spec_for(dev, &taken, amp::new_id());
                dev.amp_id = spec.id.clone();
                dev.note.clear();
                st.actions.push(Action::AddAmplifier(spec));
            }
        }
        Binding::Missing => {
            ui.label(egui::RichText::new("⚠ amplifier missing: its configured amplifier was removed. Nothing is sent.").color(TX));
        }
        Binding::Bound(row) => {
            let live = row.port.is_some();
            if ui.add_enabled(live, egui::Button::new("Standby")).clicked() {
                st.actions.push(Action::Control(row.spec.id.clone(), Control::SetPower(Power::Standby)));
            }
            if ui
                .add_enabled(live, egui::Button::new(egui::RichText::new("Operate").color(TX)))
                .on_hover_text("Sends set_powerstat Operate to this amplifier. Only on this click.")
                .clicked()
            {
                st.actions.push(Action::Control(row.spec.id.clone(), Control::SetPower(Power::Operate)));
            }
        }
    }
}

/// Live readout under a bound box's jacks: LED and words for power state,
/// connection, frequency, SWR, forward power and fault.
fn draw_amp_status(p: &egui::Painter, dev: &model::Device, r: Rect, link: &AmpLink) {
    let ports = dev.ports();
    let ins = ports.iter().filter(|p| p.dir == Dir::In).count();
    let rows = ins.max(ports.len() - ins).max(1) as f32;
    let top = r.min.y + TITLE_H + geom::ROW_H * rows + 6.0;
    let font = FontId::monospace(10.5);
    let (line1, line2, power) = match bind::binding(dev, link.rows) {
        Binding::Bound(row) => {
            let st = &row.state;
            let power = if row.port.is_some() { st.power } else { None };
            let word = power.map(|p| p.label()).unwrap_or("—");
            let freq = st.freq_hz.map(|hz| format!("{:.3}M", hz / 1e6)).unwrap_or_else(|| "—".into());
            let lv = |n: &str| st.level(n).map(amp::format_level).unwrap_or_else(|| "—".into());
            let fault = match st.level("FAULT").map(str::trim) {
                Some(f) if !f.is_empty() && f != "0" && f != "0.000000" => format!(" FAULT {f}"),
                _ => String::new(),
            };
            (
                format!("{word:<4} {freq}  {}", bind::link_text(row)),
                format!("SWR {}  Fwd {}{fault}", lv("SWR"), lv("PWRFORWARD")),
                power,
            )
        }
        Binding::Missing => ("?    amplifier missing".to_string(), "sends nothing".to_string(), None),
        Binding::Unbound => return,
    };
    let led = pos2(r.min.x + 12.0, top + 7.0);
    p.circle_filled(led, 5.0, power_led(power));
    p.circle_stroke(led, 5.0, Stroke::new(1.0, Color32::from_rgb(10, 10, 10)));
    p.text(pos2(r.min.x + 22.0, top + 7.0), Align2::LEFT_CENTER, line1, font.clone(), LABEL);
    p.text(pos2(r.min.x + 8.0, top + 7.0 + geom::ROW_H), Align2::LEFT_CENTER, line2, font, LABEL);
}

fn canvas(ui: &mut egui::Ui, bay: &mut PatchBay, st: &mut UiState, link: &AmpLink) {
    let width = ui.available_width().max(400.0);
    let bottom = bay
        .devices
        .iter()
        .map(|d| d.pos[1] + geom::box_size(d)[1])
        .fold(0.0, f32::max);
    let height = (bottom + 60.0).max(420.0);
    let (resp, painter) = ui.allocate_painter(vec2(width, height), Sense::click_and_drag());
    let o = resp.rect.min;
    let to_screen = |p: [f32; 2]| pos2(o.x + p[0], o.y + p[1]);
    let pointer = resp.hover_pos().or_else(|| ui.input(|i| i.pointer.interact_pos()));
    let at = pointer.map(|p| [p.x - o.x, p.y - o.y]);

    // Pointer handling.
    if resp.drag_started() {
        if let Some(at) = at {
            if let Some(jack) = geom::nearest_jack(bay, at, JACK_R + 4.0, |_| true) {
                if let Some(ci) = bay.cable_at(&jack) {
                    let cable = bay.cables.remove(ci);
                    let fixed = if cable.from == jack { cable.to.clone() } else { cable.from.clone() };
                    st.drag = Some(Drag::Cable { fixed, lifted: Some(cable) });
                } else {
                    st.drag = Some(Drag::Cable { fixed: jack, lifted: None });
                }
                st.selected_cable = None;
            } else if let Some(id) = geom::title_at(bay, at) {
                let d = bay.device(id).map(|d| d.pos).unwrap_or_default();
                st.drag = Some(Drag::Box { id, grab: [at[0] - d[0], at[1] - d[1]] });
                st.selected_dev = Some(id);
            }
        }
    }
    if let (Some(Drag::Box { id, grab }), Some(at)) = (&st.drag, at) {
        if let Some(d) = bay.devices.iter_mut().find(|d| d.id == *id) {
            d.pos = [(at[0] - grab[0]).clamp(0.0, (width - geom::BOX_W).max(0.0)), (at[1] - grab[1]).max(0.0)];
        }
    }
    if resp.drag_stopped() {
        if let Some(Drag::Cable { fixed, lifted }) = st.drag.take() {
            let drop = at.and_then(|at| {
                geom::nearest_jack(bay, at, SNAP, |j| bay.check(&fixed, j, None).is_ok())
                    .map(Ok)
                    .or_else(|| geom::nearest_jack(bay, at, SNAP, |j| *j != fixed).map(Err))
            });
            match drop {
                Some(Ok(j)) => {
                    let _ = bay.connect(&fixed, &j);
                    st.note.clear();
                }
                Some(Err(j)) => {
                    st.note = bay.check(&fixed, &j, None).err().unwrap_or_default();
                    if let Some(c) = lifted {
                        bay.cables.push(c);
                    }
                }
                None => {
                    st.note = if lifted.is_some() { "cable unplugged".into() } else { String::new() };
                }
            }
        } else {
            st.drag = None;
        }
    }
    if resp.clicked() {
        if let Some(at) = at {
            st.selected_cable = geom::cable_at(bay, at, 6.0);
            if st.selected_cable.is_none() {
                st.selected_dev = geom::box_at(bay, at);
            }
        }
    }
    if resp.secondary_clicked() {
        if let Some(ci) = at.and_then(|at| geom::cable_at(bay, at, 6.0)) {
            bay.cables.remove(ci);
            st.selected_cable = None;
            st.note = "cable removed".into();
        }
    }
    if let Some(ci) = st.selected_cable {
        if ci >= bay.cables.len() {
            st.selected_cable = None;
        } else if ui.memory(|m| m.focused().is_none()) && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
            bay.cables.remove(ci);
            st.selected_cable = None;
            st.note = "cable removed".into();
        }
    }

    // Painting.
    painter.rect_filled(resp.rect, 6.0, PANEL);
    let mut y = 8.0;
    while y < height {
        painter.line_segment([to_screen([4.0, y]), to_screen([width - 4.0, y])], Stroke::new(1.0, Color32::from_rgb(24, 24, 28)));
        y += 44.0;
    }
    let hover_jack = at.and_then(|at| geom::nearest_jack(bay, at, JACK_R + 4.0, |_| true));
    let drag_ok = |j: &Jack| match &st.drag {
        Some(Drag::Cable { fixed, .. }) => bay.check(fixed, j, None).is_ok(),
        _ => false,
    };
    for dev in &bay.devices {
        let size = geom::box_size(dev);
        let r = Rect::from_min_size(to_screen(dev.pos), vec2(size[0], size[1]));
        let selected = st.selected_dev == Some(dev.id);
        if selected {
            painter.rect_filled(r.expand(2.0), 8.0, LABEL);
        }
        painter.rect_filled(r, 7.0, BODY);
        let title = Rect::from_min_size(r.min, vec2(size[0], TITLE_H));
        painter.rect_filled(title, 7.0, kind_color(dev.kind));
        painter.text(title.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, &dev.name, FontId::proportional(13.0), Color32::BLACK);
        let tag = match dev.kind {
            Kind::Radio if !dev.tx => "RX ONLY".to_string(),
            Kind::Amplifier if dev.model != 0 => format!("#{}", dev.model),
            Kind::Switch => format!("POS {}", dev.sel),
            _ => dev.bands.clone(),
        };
        painter.text(title.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, tag, FontId::proportional(11.0), Color32::BLACK);
        for (name, dir, p) in geom::jacks(dev) {
            let jack = Jack::new(dev.id, &name);
            let signal = dev.port(&name).map(|p| p.signal).unwrap_or(Signal::Coax);
            let c = to_screen(p);
            let hot = hover_jack.as_ref() == Some(&jack) || drag_ok(&jack);
            draw_jack(&painter, c, signal, hot);
            let (dx, align) = if dir == Dir::In { (JACK_R + 5.0, Align2::LEFT_CENTER) } else { (-JACK_R - 5.0, Align2::RIGHT_CENTER) };
            painter.text(c + vec2(dx, 0.0), align, &name, FontId::proportional(11.0), LABEL);
        }
        if dev.is_bound_amp() {
            draw_amp_status(&painter, dev, r, link);
        }
    }
    let rep = model::trace(bay);
    let roles = model::roles(bay, &rep);
    let time = ui.input(|i| i.time) as f32;
    let hover_cable = at.and_then(|at| geom::cable_at(bay, at, 6.0));
    for (i, c) in bay.cables.iter().enumerate() {
        let (Some(a), Some(b)) = (geom::jack_pos(bay, &c.from), geom::jack_pos(bay, &c.to)) else { continue };
        let role = roles.get(i).copied().unwrap_or(Role::Idle);
        let lit = hover_cable == Some(i) || st.selected_cable == Some(i);
        draw_cable(&painter, geom::cable_curve(a, b), role, lit, time, &to_screen);
    }
    if let (Some(Drag::Cable { fixed, .. }), Some(at)) = (&st.drag, at) {
        if let Some(a) = geom::jack_pos(bay, fixed) {
            let snap = geom::nearest_jack(bay, at, SNAP, |j| bay.check(fixed, j, None).is_ok())
                .and_then(|j| geom::jack_pos(bay, &j))
                .unwrap_or(at);
            let out = bay.device(fixed.device).and_then(|d| d.port(&fixed.port)).map(|p| p.dir) == Some(Dir::Out);
            let curve = if out { geom::cable_curve(a, snap) } else { geom::cable_curve(snap, a) };
            draw_cable(&painter, curve, Role::Idle, true, time, &to_screen);
        }
    }
    if !bay.cables.is_empty() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
}

fn draw_jack(p: &egui::Painter, c: Pos2, signal: Signal, hot: bool) {
    if hot {
        p.circle_filled(c, JACK_R + 4.0, Color32::from_rgba_unmultiplied(255, 220, 120, 70));
    }
    match signal {
        // SO-239 / N style: knurled silver shell, dark dielectric, gold pin.
        Signal::Coax => {
            p.circle_filled(c, JACK_R, Color32::from_rgb(168, 170, 176));
            p.circle_stroke(c, JACK_R, Stroke::new(1.0, Color32::from_rgb(90, 92, 98)));
            p.circle_filled(c, JACK_R - 2.5, Color32::from_rgb(40, 40, 44));
            p.circle_filled(c, 1.8, Color32::from_rgb(235, 190, 70));
        }
        // Control jack (PTT/ALC): small square RCA-like.
        Signal::Control => {
            let r = Rect::from_center_size(c, vec2(10.0, 10.0));
            p.rect_filled(r, 2.0, Color32::from_rgb(120, 120, 128));
            p.circle_filled(c, 2.2, Color32::from_rgb(20, 20, 20));
        }
    }
}

fn draw_cable(p: &egui::Painter, curve: [[f32; 2]; 4], role: Role, lit: bool, time: f32, to_screen: &impl Fn([f32; 2]) -> Pos2) {
    let pts: Vec<Pos2> = geom::sample(curve, 40).into_iter().map(to_screen).collect();
    let control = role == Role::Control;
    let w = if control { 3.0 } else { 6.0 };
    if lit {
        p.line(pts.clone(), Stroke::new(w + 6.0, Color32::from_rgba_unmultiplied(255, 230, 150, 60)));
    }
    // Metallic gold: dark edge, body, bright core, then a thin specular line.
    p.line(pts.clone(), Stroke::new(w + 2.0, Color32::from_rgb(110, 78, 12)));
    p.line(pts.clone(), Stroke::new(w, Color32::from_rgb(196, 148, 40)));
    p.line(pts.clone(), Stroke::new(w * 0.45, Color32::from_rgb(242, 206, 104)));
    let spec: Vec<Pos2> = pts.iter().map(|q| *q - vec2(0.0, w * 0.22)).collect();
    p.line(spec, Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 250, 220, 170)));
    for end in [curve[0], curve[3]] {
        let c = to_screen(end);
        p.circle_filled(c, JACK_R + 1.0, Color32::from_rgb(150, 112, 30));
        p.circle_filled(c, JACK_R - 1.5, Color32::from_rgb(222, 180, 74));
        p.circle_stroke(c, JACK_R + 1.0, Stroke::new(1.0, Color32::from_rgb(90, 64, 10)));
    }
    let tint = match role {
        Role::Tx => TX,
        Role::Rx => RX,
        Role::Control => CTL,
        Role::Idle => return,
    };
    // Chevrons point in the direction the signal moves: TX toward the
    // antenna, RX toward the radio.
    let forward = role != Role::Rx;
    for k in 0..4 {
        let mut t = (k as f32 / 4.0 + time * 0.25).fract();
        if !forward {
            t = 1.0 - t;
        }
        let a = geom::bezier(curve, t);
        let b = geom::bezier(curve, (t + if forward { 0.01 } else { -0.01 }).clamp(0.0, 1.0));
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (ux, uy) = (dx / len, dy / len);
        let c = to_screen(a);
        let s = 5.0;
        let tip = c + vec2(ux * s, uy * s);
        let l = c + vec2(-ux * s - uy * s, -uy * s + ux * s);
        let r = c + vec2(-ux * s + uy * s, -uy * s - ux * s);
        p.line(vec![l, tip, r], Stroke::new(2.0, tint));
    }
    // A text tag so the role is not shown by colour alone.
    let mid = to_screen(geom::bezier(curve, 0.5)) + vec2(0.0, 12.0);
    let tag = match role {
        Role::Tx => "TX",
        Role::Rx => "RX",
        _ => "CTL",
    };
    let r = Rect::from_center_size(mid, vec2(26.0, 14.0));
    p.rect_filled(r, 3.0, Color32::from_rgb(10, 10, 10));
    p.text(mid, Align2::CENTER_CENTER, tag, FontId::monospace(10.0), tint);
}

fn status(ui: &mut egui::Ui, bay: &PatchBay, st: &UiState, link: &AmpLink) {
    let rep = model::trace(bay);
    let power = |id: &str| bind::resolve(id, link.rows).filter(|r| r.port.is_some()).and_then(|r| r.state.power);
    let amp_warn = bind::amp_warnings(bay, &rep, power);
    if rep.lines.is_empty() {
        ui.label("Drag from a jack to another jack to patch a cable. Drag a plugged end to move or unplug it. Right-click a cable, or click it and press Delete, to remove it.");
    }
    for line in &rep.lines {
        ui.label(egui::RichText::new(line).color(LABEL));
    }
    let notes: Vec<String> = bay
        .devices
        .iter()
        .filter(|d| !d.note.is_empty())
        .map(|d| format!("{}: {}", d.name, d.note))
        .collect();
    for w in rep.warnings.iter().chain(&amp_warn).chain(&notes) {
        ui.label(egui::RichText::new(format!("⚠ {w}")).color(TX));
    }
    if !st.note.is_empty() {
        ui.label(egui::RichText::new(format!("✖ {}", st.note)).color(TX));
    }
}
