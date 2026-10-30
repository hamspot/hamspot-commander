//! Station-wide software E-STOP (issue #198).
//!
//! One global latched state. While it is engaged every transmitter, amplifier
//! and motorized device Commander controls is told to stop, and new transmit,
//! operate and motion requests are refused at the device hubs (the choke
//! points), not only greyed out in the window. Releasing it returns to normal
//! operation; it does not itself key anything or start motion.
//!
//! This is a software stop. It is not a hardware or mains emergency stop.
//!
//! Lock safety: [`engaged`] is an atomic read, so it is safe anywhere,
//! including the device threads. The device list is a plain `Mutex` that is
//! never held across a device call or an egui call.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// DANGER red (#D9423A). Black text on it is 4.79:1.
pub const DANGER: egui::Color32 = egui::Color32::from_rgb(0xD9, 0x42, 0x3A);
/// How long a device has to confirm its stop before it is shown as failed.
/// It is still retried after that.
pub const CONFIRM_WITHIN: Duration = Duration::from_secs(3);
/// Gap between stop retries for a device that has not confirmed.
pub const RETRY_EVERY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Transmitter,
    Amplifier,
    Motor,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Transmitter => "transmitter",
            Kind::Amplifier => "amplifier",
            Kind::Motor => "motor",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Commanded, not confirmed yet.
    Pending,
    Stopped,
    /// The last try failed (or it did not confirm in time). Still retried.
    Failed(String),
    /// The device has no stop command Commander can send.
    NoStopSupport,
}

impl Outcome {
    pub fn label(&self) -> String {
        match self {
            Outcome::Pending => "stopping".into(),
            Outcome::Stopped => "stopped".into(),
            Outcome::Failed(why) => format!("FAILED: {why}"),
            Outcome::NoStopSupport => "no stop support".into(),
        }
    }
    pub fn settled(&self) -> bool {
        matches!(self, Outcome::Stopped | Outcome::NoStopSupport)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub kind: Kind,
    pub name: String,
    pub outcome: Outcome,
    /// When the stop was first commanded in this engagement.
    pub since: Instant,
}

/// The state machine. [`State::engage`] / [`State::release`] change it; the
/// device threads call [`State::report`]. Pure, so it is unit tested.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub engaged: bool,
    /// Bumps on every engage, so a report from an older engagement is ignored.
    pub generation: u64,
    pub devices: Vec<Device>,
}

impl State {
    pub fn engage(&mut self) {
        if self.engaged {
            return;
        }
        self.engaged = true;
        self.generation += 1;
        self.devices.clear();
    }

    /// Release (RE-ARM). Restarts nothing: it only lets new requests through.
    pub fn release(&mut self) {
        self.engaged = false;
        self.devices.clear();
    }

    /// One device's stop result for `generation`. Ignored when released or stale.
    pub fn report(&mut self, generation: u64, kind: Kind, name: &str, outcome: Outcome, now: Instant) {
        if !self.engaged || generation != self.generation {
            return;
        }
        match self.devices.iter_mut().find(|d| d.kind == kind && d.name == name) {
            Some(dev) => {
                // A confirmed stop stays confirmed until the next engage.
                if !dev.outcome.settled() || outcome.settled() {
                    dev.outcome = outcome;
                }
            }
            None => self.devices.push(Device { kind, name: name.to_string(), outcome, since: now }),
        }
    }

    /// Pending devices older than [`CONFIRM_WITHIN`] are shown as failed.
    /// The others are untouched, so one dead device cannot hold them up.
    pub fn expire(&mut self, now: Instant) {
        for dev in &mut self.devices {
            if dev.outcome == Outcome::Pending && now.duration_since(dev.since) >= CONFIRM_WITHIN {
                dev.outcome = Outcome::Failed("did not confirm the stop".into());
            }
        }
    }

    #[allow(dead_code)] // used by tests
    pub fn all_settled(&self) -> bool {
        self.devices.iter().all(|d| d.outcome.settled())
    }
}

static ENGAGED: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static STATE: Mutex<State> = Mutex::new(State { engaged: false, generation: 0, devices: Vec::new() });
static PERSIST: Mutex<Option<PathBuf>> = Mutex::new(None);
type Waker = Box<dyn Fn() + Send>;
static WAKERS: Mutex<Vec<Waker>> = Mutex::new(Vec::new());

/// Register a call made right after every engage (wakes a device hub).
/// It must not take the E-STOP lock or an egui lock.
pub fn on_engage(f: Waker) {
    WAKERS.lock().expect("estop wakers").push(f);
}

/// True while the E-STOP is engaged. Lock-free.
pub fn engaged() -> bool {
    ENGAGED.load(Ordering::SeqCst)
}

/// Current engagement number (for [`report`]).
pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

pub fn engage() {
    let mut st = STATE.lock().expect("estop");
    st.engage();
    GENERATION.store(st.generation, Ordering::SeqCst);
    ENGAGED.store(true, Ordering::SeqCst);
    drop(st);
    save(true);
    for w in WAKERS.lock().expect("estop wakers").iter() {
        w();
    }
}

pub fn release() {
    let mut st = STATE.lock().expect("estop");
    st.release();
    ENGAGED.store(false, Ordering::SeqCst);
    drop(st);
    save(false);
}

pub fn report(generation: u64, kind: Kind, name: &str, outcome: Outcome) {
    STATE.lock().expect("estop").report(generation, kind, name, outcome, Instant::now());
}

/// Copy of the state for drawing, with timeouts applied.
pub fn snapshot() -> State {
    let mut st = STATE.lock().expect("estop");
    st.expire(Instant::now());
    st.clone()
}

/// Remember the engaged flag in `path` and start engaged if it was engaged
/// when Commander last ran, so a restart does not silently re-enable transmit.
pub fn load_persisted(path: PathBuf) {
    let was = persisted_engaged(&path);
    *PERSIST.lock().expect("estop persist") = Some(path);
    if was {
        engage();
    }
}

pub fn persisted_engaged(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| text.trim() == "engaged")
}

fn save(on: bool) {
    let path = PERSIST.lock().expect("estop persist").clone();
    if let Some(path) = path {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, if on { "engaged\n" } else { "armed\n" });
    }
}

// ---- Choke-point rules (pure, unit tested) ----

/// May the radio get this CAT command? Only keying is refused; unkeying and
/// everything else (tuning, mode) still goes through.
pub fn allows_rig(cmd: &rigctl::Command, engaged: bool) -> bool {
    !(engaged && matches!(cmd, rigctl::Command::SetPtt(true)))
}

/// May the rotator get this control? Only `stop` while engaged.
pub fn allows_rot(cmd: &crate::rot::Control, engaged: bool) -> bool {
    !engaged || matches!(cmd, crate::rot::Control::Stop)
}

/// May the amplifier get this control? Operate and On are refused while engaged.
pub fn allows_amp(cmd: &crate::amp::Control, engaged: bool) -> bool {
    use crate::amp::{Control, Power};
    !(engaged && matches!(cmd, Control::SetPower(Power::Operate | Power::On)))
}

/// Per-device retry clock for the device threads.
#[derive(Clone, Copy, Debug, Default)]
pub struct Retry {
    /// Engagement this device last confirmed, 0 = none.
    pub confirmed: u64,
    pub next_try: Option<Instant>,
}

impl Retry {
    /// Should the device thread send (or resend) its stop now?
    pub fn due(&self, engaged: bool, generation: u64, now: Instant) -> bool {
        engaged && self.confirmed != generation && self.next_try.is_none_or(|at| now >= at)
    }
    pub fn tried(&mut self, generation: u64, ok: bool, now: Instant) {
        if ok {
            self.confirmed = generation;
            self.next_try = None;
        } else {
            self.next_try = Some(now + RETRY_EVERY);
        }
    }
}

// ---- Keyboard chord ----

/// The four keys that engage the E-STOP when all are held at once.
pub const CHORD: [egui::Key; 4] = [egui::Key::F9, egui::Key::F10, egui::Key::F11, egui::Key::F12];

/// True when every chord key is down in this viewport's input. One `input`
/// closure, nothing else called inside it. egui clears its held keys when the
/// window loses focus, so a key left "down" by a focus change does not count.
pub fn chord_down(ctx: &egui::Context) -> bool {
    ctx.input(|i| CHORD.iter().all(|k| i.key_down(*k)))
}

/// Edge trigger for the chord: fires once when the fourth key goes down.
/// Holding (or key repeat) does not fire again; letting go releases nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Chord {
    held: bool,
}

impl Chord {
    /// Feed this frame's [`chord_down`]. True on the frame it should engage.
    pub fn update(&mut self, down: bool) -> bool {
        let fire = down && !self.held;
        self.held = down;
        fire
    }

    /// Read the keys once for `ctx` and engage on the press edge. Only ever
    /// engages; the chord never releases.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let down = chord_down(ctx);
        if self.update(down) && !engaged() {
            engage();
        }
    }
}

// ---- Widget ----

/// The font of the app's standard buttons: the LCARS tab/nav buttons
/// (`rustcars::lcars_button`) draw their labels in Antonio `header_size`
/// (22 pt). E-STOP and RELEASE use exactly this font (#200).
pub fn std_button_font(_ui: &egui::Ui) -> egui::FontId {
    egui::FontId::proportional(rustcars::LcarsTheme::default().header_size)
}

/// THE height of the E-STOP and RELEASE buttons (#200): the height of the
/// app's standard buttons, the LCARS tabs, which are `bar_thin` (40 px) tall,
/// the same as the tan top bar the E-STOP sits in. One value for both buttons.
/// (The previous attempt used a plain `ui.button` height, 25-27 px, which on
/// screen looked tiny next to the 40 px tabs.)
pub fn std_button_height(_ui: &egui::Ui) -> f32 {
    rustcars::LcarsTheme::default().bar_thin
}

/// Corner radius of the E-STOP and RELEASE buttons: square, as David asked.
pub const CORNER: egui::CornerRadius = egui::CornerRadius::ZERO;
/// Border of both buttons (same in armed and engaged states).
pub const BORDER: f32 = 1.5;
const RELEASE_FILL: egui::Color32 = egui::Color32::from_rgb(0xF2, 0xC1, 0x4E);

/// Octagon radius for a button `h` tall: about the text height, never
/// touching the border (2 px clear top and bottom).
pub fn octagon_radius(h: f32, text_h: f32) -> f32 {
    (text_h / 2.0).min(h / 2.0 - 2.0).max(2.0)
}

/// Paint a stop-sign octagon (no font glyph) centred at `c`, radius `r`.
pub fn paint_octagon(painter: &egui::Painter, c: egui::Pos2, r: f32, color: egui::Color32) {
    painter.add(egui::Shape::convex_polygon(octagon_points(c, r), color, egui::Stroke::NONE));
}

pub fn octagon_points(c: egui::Pos2, r: f32) -> Vec<egui::Pos2> {
    (0..8)
        .map(|i| {
            let a = std::f32::consts::PI / 8.0 + i as f32 * std::f32::consts::PI / 4.0;
            egui::pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}

fn galley_w(ui: &egui::Ui, text: &str, font: egui::FontId) -> f32 {
    ui.painter().layout_no_wrap(text.into(), font, egui::Color32::BLACK).size().x
}

fn square_w(ui: &egui::Ui, label: &str, octagon: bool) -> f32 {
    let font = std_button_font(ui);
    let text_h = ui.painter().layout_no_wrap("E".into(), font.clone(), egui::Color32::BLACK).size().y;
    let r = octagon_radius(std_button_height(ui), text_h);
    let icon = if octagon { 2.0 * r + 4.0 } else { 0.0 };
    galley_w(ui, label, font) + icon + 2.0 * ui.spacing().button_padding.x
}

/// Width the control needs (for reserving a slot in a bar), measured with
/// the real fonts. Engaged includes RELEASE. Nothing else sits in the bar.
pub fn button_width(ui: &egui::Ui, engaged: bool) -> f32 {
    if engaged {
        square_w(ui, "E-STOP ENGAGED", true) + square_w(ui, "RELEASE", false) + 4.0
    } else {
        square_w(ui, "E-STOP", true)
    }
}

/// One square, bordered button of [`std_button_height`] with the theme's
/// Button text; `octagon` adds the stop sign before the text. Used by both
/// E-STOP and RELEASE so their font, height, corners and border are identical.
fn square_button(ui: &mut egui::Ui, label: &str, fill: egui::Color32, octagon: bool) -> egui::Response {
    let font = std_button_font(ui);
    let galley = ui.painter().layout_no_wrap(label.into(), font, egui::Color32::BLACK);
    let h = std_button_height(ui);
    let text_h = galley.size().y;
    let pad_x = ui.spacing().button_padding.x;
    let r = octagon_radius(h, text_h);
    let icon = if octagon { 2.0 * r + 4.0 } else { 0.0 };
    let w = galley.size().x + icon + 2.0 * pad_x;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
    let fill = if resp.hovered() { fill.gamma_multiply(0.85) } else { fill };
    let p = ui.painter();
    p.rect_filled(rect, CORNER, fill);
    p.rect_stroke(rect, CORNER, egui::Stroke::new(BORDER, egui::Color32::BLACK), egui::StrokeKind::Inside);
    if octagon {
        paint_octagon(p, egui::pos2(rect.left() + pad_x + r, rect.center().y), r, egui::Color32::BLACK);
    }
    p.galley(
        egui::pos2(rect.left() + pad_x + icon, rect.center().y - text_h / 2.0),
        galley,
        egui::Color32::BLACK,
    );
    let l = label.to_owned();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &l));
    resp
}

/// The shared E-STOP control. Every copy reads and changes the one global
/// state. Armed: one `E-STOP` button. Engaged: an `E-STOP ENGAGED` button
/// (pressing it again does nothing harmful) and a `RELEASE` button.
/// Returns true on the frame it was engaged, so the caller can wake the
/// device hubs at once (they also poll [`engaged`] on their own).
pub fn button(ui: &mut egui::Ui) -> bool {
    let on = engaged();
    let (stop, rel) = buttons(ui, on);
    let stop = stop.on_hover_text(if on {
        "E-STOP is engaged: transmit, amplifier operate and motion are blocked. Press RELEASE to re-arm."
    } else {
        "Emergency stop: unkey every transmitter, put amplifiers in standby and stop every rotator."
    });
    let pressed = stop.clicked() && !on;
    if pressed {
        engage();
    }
    if let Some(rel) = rel {
        let st = snapshot();
        let rel = rel.on_hover_ui(|ui| {
            ui.label("Release: back to normal operation. Nothing is keyed or moved by it.");
            if st.devices.is_empty() {
                ui.label("No devices were commanded.");
            }
            for d in &st.devices {
                ui.label(format!("{} {}: {}", d.kind.label(), d.name, d.outcome.label()));
            }
        });
        if rel.clicked() {
            release();
        }
    }
    pressed
}

/// Lay out the E-STOP (and, when `on`, RELEASE) in one row, vertically
/// centred in the height the caller's bar slot gives it (equal margins).
/// Pure layout: no global state is read or changed (headless tests use it).
pub fn buttons(ui: &mut egui::Ui, on: bool) -> (egui::Response, Option<egui::Response>) {
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let label = if on { "E-STOP ENGAGED" } else { "E-STOP" };
        let stop = square_button(ui, label, DANGER, true);
        let rel = on.then(|| square_button(ui, "RELEASE", RELEASE_FILL, false));
        (stop, rel)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engage_confirm_release() {
        let now = Instant::now();
        let mut st = State::default();
        st.engage();
        assert!(st.engaged);
        let g = st.generation;
        st.report(g, Kind::Motor, "rot", Outcome::Pending, now);
        st.report(g, Kind::Amplifier, "amp", Outcome::Stopped, now);
        assert!(!st.all_settled());
        st.report(g, Kind::Motor, "rot", Outcome::Stopped, now);
        assert!(st.all_settled());
        // A late failure does not undo a confirmed stop.
        st.report(g, Kind::Motor, "rot", Outcome::Failed("x".into()), now);
        assert_eq!(st.devices[0].outcome, Outcome::Stopped);
        st.release();
        assert!(!st.engaged && st.devices.is_empty());
        // Reports from the old engagement after release are ignored.
        st.report(g, Kind::Motor, "rot", Outcome::Pending, now);
        assert!(st.devices.is_empty());
        st.engage();
        assert_eq!(st.generation, g + 1);
        st.report(g, Kind::Motor, "rot", Outcome::Pending, now);
        assert!(st.devices.is_empty(), "stale generation ignored");
    }

    #[test]
    fn requests_refused_while_engaged() {
        use crate::amp::{Control as A, Power};
        use crate::rot::{Control as R, Spin};
        use rigctl::Command as C;
        assert!(!allows_rig(&C::SetPtt(true), true));
        assert!(allows_rig(&C::SetPtt(false), true));
        assert!(allows_rig(&C::SetPtt(true), false));
        assert!(!allows_rot(&R::SetPos(10.0, 0.0), true));
        assert!(!allows_rot(&R::Move(Spin::Cw, 50), true));
        assert!(allows_rot(&R::Stop, true));
        assert!(allows_rot(&R::Move(Spin::Cw, 50), false));
        assert!(!allows_amp(&A::SetPower(Power::Operate), true));
        assert!(!allows_amp(&A::SetPower(Power::On), true));
        assert!(allows_amp(&A::SetPower(Power::Standby), true));
        assert!(allows_amp(&A::SetPower(Power::Operate), false));
    }

    #[test]
    fn release_returns_to_normal_without_keying() {
        // Release clears the latch: requests are allowed again (normal
        // operation), no new stop is asked for, and nothing is keyed by it.
        let now = Instant::now();
        let mut st = State::default();
        st.engage();
        let g = st.generation;
        let mut r = Retry::default();
        assert!(r.due(true, g, now));
        r.tried(g, true, now);
        assert!(!r.due(true, g, now));
        st.release();
        assert!(!r.due(false, g, now));
        assert!(st.devices.is_empty());
        assert!(allows_rig(&rigctl::Command::SetPtt(true), st.engaged));
        assert!(allows_rot(&crate::rot::Control::SetPos(1.0, 0.0), st.engaged));
        assert!(allows_amp(&crate::amp::Control::SetPower(crate::amp::Power::Operate), st.engaged));
    }

    #[test]
    fn timed_out_device_does_not_block_others() {
        let t0 = Instant::now();
        let mut st = State::default();
        st.engage();
        let g = st.generation;
        st.report(g, Kind::Motor, "dead", Outcome::Pending, t0);
        st.report(g, Kind::Transmitter, "rig", Outcome::Stopped, t0);
        st.report(g, Kind::Amplifier, "amp", Outcome::Pending, t0);
        st.report(g, Kind::Amplifier, "amp", Outcome::Stopped, t0);
        st.expire(t0 + CONFIRM_WITHIN);
        let get = |n: &str| st.devices.iter().find(|d| d.name == n).unwrap().outcome.clone();
        assert!(matches!(get("dead"), Outcome::Failed(_)));
        assert_eq!(get("rig"), Outcome::Stopped);
        assert_eq!(get("amp"), Outcome::Stopped);
        // The dead one keeps being retried on its own clock.
        let mut dead = Retry::default();
        dead.tried(g, false, t0);
        assert!(!dead.due(true, g, t0));
        assert!(dead.due(true, g, t0 + RETRY_EVERY));
        // It can still confirm later.
        st.report(g, Kind::Motor, "dead", Outcome::Stopped, t0);
        assert!(st.all_settled());
    }

    #[test]
    fn persisted_flag_round_trips() {
        let dir = std::env::temp_dir().join(format!("estop-test-{}", std::process::id()));
        let path = dir.join("estop");
        assert!(!persisted_engaged(&path));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "engaged\n").unwrap();
        assert!(persisted_engaged(&path));
        std::fs::write(&path, "armed\n").unwrap();
        assert!(!persisted_engaged(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>) -> bool {
        let input = egui::RawInput { events, ..Default::default() };
        let mut down = false;
        let mut out = ctx.run_ui(input, |ui| {
            down = chord_down(ui.ctx());
        });
        out.textures_delta.clear();
        down
    }

    fn key(k: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key: k,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn chord_needs_all_four_and_fires_once() {
        let ctx = egui::Context::default();
        let mut chord = Chord::default();
        // Three keys: nothing.
        let three = CHORD[..3].iter().map(|k| key(*k, true)).collect();
        assert!(!chord.update(frame(&ctx, three)));
        // Fourth key: fires once.
        assert!(chord.update(frame(&ctx, vec![key(egui::Key::F12, true)])));
        // Still held (with a repeat event): no second fire.
        let rep = egui::Event::Key {
            key: egui::Key::F12,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: egui::Modifiers::NONE,
        };
        assert!(!chord.update(frame(&ctx, vec![rep])));
        assert!(!chord.update(frame(&ctx, vec![])));
        // Let one go, press it again: fires again (a new press).
        assert!(!chord.update(frame(&ctx, vec![key(egui::Key::F10, false)])));
        assert!(chord.update(frame(&ctx, vec![key(egui::Key::F10, true)])));
        // Focus loss clears held keys: no fire, and no stuck "down".
        assert!(!chord.update(frame(&ctx, vec![egui::Event::WindowFocused(false)])));
        assert!(!chord.update(frame(&ctx, vec![egui::Event::WindowFocused(true)])));
    }

    #[test]
    fn octagon_fits_the_tab_height() {
        let t = rustcars::LcarsTheme::default();
        let h = t.bar_thin;
        let r = octagon_radius(h, 30.0);
        assert!(2.0 * r <= h - 4.0, "octagon not clipped");
        assert!(r >= 2.0);
    }

    /// Headless frame with the app's real theme (rustcars Antonio font +
    /// `apply_theme`), as `Tools::new` installs it.
    fn themed_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        rustcars::install_lcars_font(&ctx);
        rustcars::apply_theme(&ctx, &rustcars::LcarsTheme::default());
        ctx
    }

    fn run(ctx: &egui::Context, f: &mut dyn FnMut(&mut egui::Ui)) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 400.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| f(ui));
        out.textures_delta.clear();
    }

    /// Font sizes of every text shape painted in `rect` this frame.
    fn text_sizes_in(shapes: &[egui::epaint::ClippedShape], rect: egui::Rect) -> Vec<f32> {
        let mut v = Vec::new();
        for c in shapes {
            if let egui::Shape::Text(t) = &c.shape {
                if rect.contains(t.pos + egui::vec2(1.0, 1.0)) {
                    for row in &t.galley.rows {
                        for g in &row.glyphs {
                            v.push(g.font_ascent + 0.0 * g.font_height);
                        }
                    }
                }
            }
        }
        v
    }

    #[test]
    fn estop_and_release_match_the_tab_buttons_headless() {
        // The app's standard button is the LCARS tab (CONFIG, KEYS…):
        // rustcars::lcars_button, bar_thin tall, header_size text.
        let ctx = themed_ctx();
        let t = rustcars::LcarsTheme::default();
        let mut got = None;
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 400.0))),
                ..Default::default()
            };
            let mut rects = None;
            let mut out = ctx.run_ui(input, |ui| {
                let tab = rustcars::lcars_button(ui, &t, "CONFIG", rustcars::palette::TAN, egui::vec2(120.0, t.bar_thin), rustcars::Cap::None, true).rect;
                let (stop, rel) = buttons(ui, true);
                let armed = buttons(ui, false).0.rect;
                rects = Some((tab, stop.rect, rel.unwrap().rect, armed));
            });
            let (tab, stop, rel, armed) = rects.unwrap();
            let sizes = |r| text_sizes_in(&out.shapes, r);
            out.textures_delta.clear();
            got = Some((tab, stop, rel, armed, sizes(tab), sizes(stop), sizes(rel), sizes(armed)));
        }
        let (tab, stop, rel, armed, ft, fs, fr, fa) = got.unwrap();
        eprintln!("tab {:?} stop {:?} release {:?} armed {:?}", tab.size(), stop.size(), rel.size(), armed.size());
        eprintln!("ascent tab {:?} stop {:?} release {:?} armed {:?}", ft.first(), fs.first(), fr.first(), fa.first());
        for r in [stop, rel, armed] {
            assert!((r.height() - tab.height()).abs() < 0.01, "{} vs tab {}", r.height(), tab.height());
        }
        assert!(!ft.is_empty() && !fs.is_empty() && !fr.is_empty() && !fa.is_empty());
        for f in fs.iter().chain(&fr).chain(&fa) {
            assert!((f - ft[0]).abs() < 0.01, "label font differs from tab font");
        }
        assert!((stop.center().y - rel.center().y).abs() < 0.01);
    }

    #[test]
    fn estop_buttons_are_centred_in_a_taller_bar() {
        let ctx = themed_ctx();
        let mut got = None;
        for _ in 0..2 {
            run(&ctx, &mut |ui| {
                let bar = egui::Rect::from_min_size(egui::pos2(0.0, 100.0), egui::vec2(600.0, 40.0));
                let r = ui
                    .scope_builder(
                        egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)),
                        |ui| buttons(ui, true),
                    )
                    .inner;
                got = Some((bar, r.0.rect, r.1.unwrap().rect));
            });
        }
        let (bar, stop, rel) = got.unwrap();
        for r in [stop, rel] {
            let (above, below) = (r.top() - bar.top(), bar.bottom() - r.bottom());
            assert!((above - below).abs() < 0.6, "above {above} below {below}");
        }
    }

    #[test]
    fn estop_buttons_are_square() {
        assert_eq!(CORNER, egui::CornerRadius::ZERO);
    }

    #[test]
    fn octagon_has_eight_points_on_the_circle() {
        let pts = octagon_points(egui::pos2(10.0, 10.0), 8.0);
        assert_eq!(pts.len(), 8);
        for p in pts {
            assert!(((p - egui::pos2(10.0, 10.0)).length() - 8.0).abs() < 1e-3);
        }
    }

    #[test]
    fn danger_text_contrast() {
        assert!(rustcars::contrast(DANGER, egui::Color32::BLACK) >= 4.5);
    }

    #[test]
    fn every_main_page_has_exactly_one_estop() {
        // main.rs draws the E-STOP in the 40 px page-bar slot shared by
        // every page (Config included), and once more at the right of a
        // detached page window's top row (not Tools: its window body draws
        // its own, in tools/mod.rs); group bars never.
        let call = ["estop::", "button(ui)"].concat();
        let main = include_str!("main.rs");
        assert_eq!(main.matches(&call).count(), 2, "page bar + detached page window");
        let at = main.find(&call).unwrap();
        assert!(main[..at].rfind("id_salt(\"estop-page-bar\")").is_some());
        let body = &main[main.find("fn detached_page_body(").unwrap()..];
        let body = &body[..body.find("\n    fn ").unwrap()];
        assert_eq!(body.matches(&call).count(), 1, "detached page windows carry it once");
        assert!(body.contains("if page != Page::Tools"), "Tools draws its own");
        assert_eq!(include_str!("tools/mod.rs").matches(&call).count(), 1);
        assert_eq!(include_str!("group_box.rs").matches(&call).count(), 0);
    }
}
