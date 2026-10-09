//! Hamspot Commander, phase 1: the rig-control face.
//!
//! The window follows the Ham Radio Deluxe 5.21 rig screen (frequency, meters,
//! VFO cluster, band scales, sliders). Painting follows the RT-950 CPS: dark
//! egui, group frames, and the bandspread dial in `dial`.

mod dial;

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use eframe::egui;
use rigctl::{Command, DemoRig, LinkKind, LiveRig, Rig, RigError, Snapshot};

use dial::{
    band_of, digit_steps, format_hz, meter, parse_freq, scale, strength_frac, strength_label,
    BANDS, HAM,
};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([960.0, 640.0])
            .with_title("Hamspot Commander"),
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
    local_mem: Option<f64>,
    hold_freq: Option<f64>,
    hold_levels: HashMap<String, f64>,
    hold_pass: Option<i32>,
    fine_lock: Option<(f64, f64)>,
    fullscreen: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let snap = DemoRig::new().poll().expect("demo rig polls");
        let shared = Arc::new(Mutex::new(Shared {
            snap,
            cmds: Vec::new(),
            switch: None,
        }));
        let wake = Arc::new(Condvar::new());
        let ctx = cc.egui_ctx.clone();
        let shared_thread = Arc::clone(&shared);
        let wake_thread = Arc::clone(&wake);
        thread::spawn(move || drive(shared_thread, wake_thread, ctx));
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
            local_mem: None,
            hold_freq: None,
            hold_levels: HashMap::new(),
            hold_pass: None,
            fine_lock: None,
            fullscreen: false,
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

    fn tune(&mut self, hz: f64) {
        self.hold_freq = Some(hz);
        self.freq_text = format_hz(hz);
        self.send(Command::SetFreq(hz));
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut snap = self.snap();
        if snap.online() {
            if let Some(band) = band_of(snap.freq_hz) {
                self.band_mem.insert(band.id.to_string(), snap.freq_hz);
            }
        }
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.pointer.any_released()) {
            self.fine_lock = None;
        }

        egui::Panel::top("bar").show_inside(ui, |ui| {
            self.bar(ui, &snap);
        });
        egui::Panel::bottom("sliders").show_inside(ui, |ui| {
            self.sliders(ui, &snap);
        });
        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.face(ui, &mut snap);
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
    fn bar(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
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
            ui.separator();
            let link = match snap.kind {
                LinkKind::Demo => "Demo".to_string(),
                LinkKind::Live => format!("{}  {}", snap.model, snap.endpoint),
                LinkKind::Down => format!("Disconnected  {}", snap.message),
            };
            ui.label(link);
            if !snap.message.is_empty() && snap.kind != LinkKind::Down {
                ui.label(
                    egui::RichText::new(&snap.message).color(egui::Color32::from_rgb(220, 160, 80)),
                );
            }
        });
    }

    fn face(&mut self, ui: &mut egui::Ui, snap: &mut Snapshot) {
        let freq = self.shown_freq(snap);
        ui.horizontal_top(|ui| {
            self.status_column(ui, snap);
            ui.separator();
            self.vfo_column(ui, snap);
            ui.separator();
            ui.vertical(|ui| {
                self.frequency(ui, snap, freq);
                ui.add_space(4.0);
                let frac = strength_frac(snap.strength_db);
                meter(ui, &strength_label(snap.strength_db), frac, "");
            });
            ui.separator();
            self.buttons(ui, snap);
        });
        ui.add_space(8.0);
        let (lo, hi) = self.fine_lock.unwrap_or((freq - 1_200.0, freq + 1_200.0));
        if let Some(hz) = scale(ui, "Fine", freq, lo, hi, HAM, 36.0) {
            if self.fine_lock.is_none() {
                self.fine_lock = Some((lo, hi));
            }
            if snap.caps.can_set_freq {
                self.tune(hz);
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("160 m – 10 m").weak());
            let alt = ui.button("ALT");
            if alt.clicked() && snap.caps.vfo_ops.contains("XCHG") {
                self.send(Command::VfoOp("XCHG".into()));
            }
            let bsp = ui.selectable_label(self.show_spread, "BSP");
            if bsp.clicked() {
                self.show_spread = !self.show_spread;
            }
            for band in BANDS {
                let on = band_of(freq).is_some_and(|current| current.id == band.id);
                if ui.selectable_label(on, band.chip).clicked() && snap.caps.can_set_freq {
                    let hz = self.band_mem.get(band.id).copied().unwrap_or(band.home);
                    self.tune(hz);
                }
            }
        });
        if self.show_spread {
            ui.add_space(4.0);
            for band in BANDS.iter().rev() {
                let active = band_of(freq).is_some_and(|current| current.id == band.id);
                let color = if active {
                    HAM
                } else {
                    HAM.gamma_multiply(0.35)
                };
                if let Some(hz) = scale(ui, band.chip, freq, band.lo, band.hi, color, 28.0) {
                    if snap.caps.can_set_freq {
                        self.tune(hz);
                    }
                }
            }
        }
    }

    fn status_column(&self, ui: &mut egui::Ui, snap: &Snapshot) {
        ui.vertical(|ui| {
            ui.set_width(150.0);
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
            ui.set_width(168.0);
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
                        self.tune(hz);
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

    fn frequency(&mut self, ui: &mut egui::Ui, snap: &Snapshot, freq: f64) {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(&snap.mode)
                    .size(18.0)
                    .strong()
                    .color(HAM),
            );
            if snap.rit_hz != 0 {
                ui.label(format!("RIT {:+} Hz", snap.rit_hz));
            }
            ui.horizontal(|ui| {
                for (ch, step) in digit_steps(freq) {
                    if ch == '.' {
                        ui.label(egui::RichText::new(".").size(48.0).strong().monospace());
                        continue;
                    }
                    let step = step.unwrap_or(1.0);
                    let selected = (step - self.step_hz).abs() < 0.1;
                    let text = egui::RichText::new(ch.to_string())
                        .size(48.0)
                        .strong()
                        .monospace();
                    if ui
                        .add_enabled(
                            snap.caps.can_set_freq,
                            egui::Button::new(text).selected(selected),
                        )
                        .clicked()
                    {
                        self.step_hz = step;
                    }
                }
            });
            let wheel = ui.interact(
                ui.min_rect(),
                ui.id().with("freq-wheel"),
                egui::Sense::hover(),
            );
            if wheel.hovered() && snap.caps.can_set_freq {
                let dy = ui.input(|i| i.smooth_scroll_delta.y);
                if dy.abs() > 0.0 {
                    let dir = if dy > 0.0 { 1.0 } else { -1.0 };
                    self.tune(freq + dir * self.step_hz);
                }
            }
            ui.horizontal(|ui| {
                let edit = egui::TextEdit::singleline(&mut self.freq_text).desired_width(160.0);
                let resp = ui.add_enabled(snap.caps.can_set_freq, edit);
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if let Some(hz) = parse_freq(&self.freq_text) {
                        self.tune(hz);
                    }
                }
                ui.label(egui::RichText::new(format!("step {}", step_label(self.step_hz))).weak());
            });
        });
    }

    fn buttons(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        ui.vertical(|ui| {
            egui::Grid::new("rig-buttons")
                .spacing([4.0, 4.0])
                .show(ui, |ui| {
                    self.func_btn(ui, snap, "ATU", "TUNER");
                    self.band_step(ui, snap, "Band -", -1);
                    self.dead(ui, "Dimmer");
                    self.func_btn(ui, snap, "NR", "NR");
                    self.preamp(ui, snap, "Pre 1", 10.0);
                    ui.end_row();

                    self.ant(ui, snap, "Ant 1", 1);
                    self.func_btn(ui, snap, "Bk-in", "SBKIN");
                    self.func_btn(ui, snap, "Fast", "FAGC");
                    self.dead(ui, "NR sub");
                    self.preamp(ui, snap, "Pre 2", 20.0);
                    ui.end_row();

                    self.ant(ui, snap, "Ant 2", 2);
                    self.dead(ui, "CW Spot");
                    self.dead(ui, "Keyer");
                    self.dead(ui, "Nar");
                    self.func_btn(ui, snap, "Proc", "COMP");
                    ui.end_row();

                    self.dead(ui, "Ant Rx");
                    self.dead(ui, "Chan +");
                    self.func_btn(ui, snap, "Lock", "LOCK");
                    self.func_btn(ui, snap, "Notch", "ANF");
                    self.dead(ui, "QMR");
                    ui.end_row();

                    self.dead(ui, "BC");
                    self.dead(ui, "Chan -");
                    self.ptt(ui, snap, "MOX");
                    self.dead(ui, "PWR");
                    self.dead(ui, "QMS");
                    ui.end_row();

                    self.band_step(ui, snap, "Band +", 1);
                    self.dead(ui, "Contour");
                    self.func_btn(ui, snap, "Moni", "MON");
                    self.dead(ui, "Scan");
                    self.dead(ui, "Scan Dn");
                    ui.end_row();
                });
            ui.add_space(6.0);
            self.mode_box(ui, snap, "Main mode", true);
            self.mode_box(ui, snap, "Sub mode", false);
            self.receiver_box(ui, snap);
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

    fn ptt(&self, ui: &mut egui::Ui, snap: &Snapshot, label: &str) {
        let enabled = snap.online() && snap.caps.can_set_ptt;
        if ui
            .add_enabled(enabled, egui::Button::new(label).selected(snap.ptt))
            .clicked()
        {
            self.send(Command::SetPtt(!snap.ptt));
        }
    }

    fn band_step(&mut self, ui: &mut egui::Ui, snap: &Snapshot, label: &str, dir: isize) {
        let enabled = snap.online() && snap.caps.can_set_freq;
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            let freq = self.shown_freq(snap);
            let idx = BANDS
                .iter()
                .position(|band| band_of(freq).is_some_and(|c| c.id == band.id))
                .unwrap_or(5);
            let next = idx as isize + dir;
            if (0..BANDS.len() as isize).contains(&next) {
                let band = &BANDS[next as usize];
                let hz = self.band_mem.get(band.id).copied().unwrap_or(band.home);
                self.tune(hz);
            }
        }
    }

    fn mode_box(&self, ui: &mut egui::Ui, snap: &Snapshot, label: &str, main: bool) {
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
                                Command::SetMode {
                                    mode: mode.clone(),
                                    passband_hz: snap.passband_hz,
                                }
                            } else {
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
        egui::Grid::new("levels")
            .num_columns(4)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                self.level_slider(ui, snap, "AF", "AF gain (main)");
                self.level_slider(ui, snap, "RF", "RF gain (main)");
                self.agc(ui, snap);
                self.level_slider(ui, snap, "RFPOWER", "RF power");
                ui.end_row();

                fixed_slider(ui, "AF gain (sub)", 0.10);
                fixed_slider(ui, "RF gain (sub)", 1.0);
                ui.add_enabled(false, egui::Button::new("AGC (sub): Auto Fast"));
                self.level_slider(ui, snap, "NR", "Noise reduction");
                ui.end_row();

                self.level_slider(ui, snap, "IF", "IF shift");
                self.width_slider(ui, snap);
                ui.add_enabled(false, egui::Button::new("Contour: 20 Hz"));
                ui.label("");
                ui.end_row();

                self.level_slider(ui, snap, "MICGAIN", "Mic gain");
                self.level_slider(ui, snap, "VOXDELAY", "VOX delay");
                self.level_slider(ui, snap, "VOXGAIN", "VOX gain");
                ui.label("");
                ui.end_row();
            });
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
                ui.label("A dummy rig is `rigctld -m 1 -t 4532`.");
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
                        self.tune(hz);
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
    let mut session = Session::Demo(DemoRig::new());
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

        let mut remote = None;
        for cmd in &cmds {
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
        } else {
            last = snap.clone();
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
            freq_hz: 0.0,
            mode: String::new(),
            passband_hz: 0,
            sub_mode: String::new(),
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
