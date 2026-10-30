//! Scientific calculator with graphing, built only from rustcars widgets.
//! Run: `cargo run --example calculator` (desktop only).
//!
//! Click modes (AGENTS.md P2), used as *variants of one key*:
//! * function keys: click = function, double = inverse, triple = hyperbolic
//!   (SIN: sin / asin / sinh, LN: ln / exp / log10, SQRT: sqrt / ^2 / cbrt, PI: pi / e / ans).
//!   Shift = Alt 1 and Ctrl = Alt 2 work too, and right-click lists all variants.
//! * history rows: click = insert the result, double = load the expression for editing,
//!   triple = plot it (stays purple while plotted; triple-click again to remove).
//!
//! Digits and operators use `action_button` (no modes) so fast typing is never merged.
mod eval;

use eval::{Angle, Env};
use rustcars::{
    Cap, LcarsFrame, LcarsTheme, Mode, Side, action_button, apply_theme, install_lcars_font,
    lcars_button, mode_button, mode_list, palette, readout,
};

/// (label, [primary, alt1 (inverse), alt2 (hyperbolic/other)]) inserted text
const FN_KEYS: [(&str, [&str; 3]); 6] = [
    ("sin", ["sin(", "asin(", "sinh("]),
    ("cos", ["cos(", "acos(", "cosh("]),
    ("tan", ["tan(", "atan(", "tanh("]),
    ("ln", ["ln(", "exp(", "log("]),
    ("sqrt", ["sqrt(", "^2", "cbrt("]),
    ("pi", ["pi", "e", "ans"]),
];

const KEYS: [[&str; 5]; 4] = [
    ["7", "8", "9", "/", "("],
    ["4", "5", "6", "*", ")"],
    ["1", "2", "3", "-", "^"],
    ["0", ".", "x", "+", "!"],
];

pub struct Graph {
    pub cx: f64,
    pub cy: f64,
    pub ppu: f64, // pixels per unit
}

pub struct Calc {
    pub theme: LcarsTheme,
    pub input: String,
    pub result: Result<f64, String>,
    pub env: Env,
    pub history: Vec<(String, Mode)>,
    pub exprs: Vec<String>,
    pub graph: Graph,
}

impl Calc {
    pub fn new(ctx: &egui::Context) -> Self {
        let theme = LcarsTheme::default();
        install_lcars_font(ctx);
        apply_theme(ctx, &theme);
        Self {
            theme,
            input: String::new(),
            result: Ok(0.0),
            env: Env::default(),
            history: vec![],
            exprs: vec![],
            graph: Graph {
                cx: 0.0,
                cy: 0.0,
                ppu: 40.0,
            },
        }
    }

    pub fn evaluate(&mut self) {
        if self.input.trim().is_empty() {
            return;
        }
        self.result = eval::eval(&self.input, &self.env);
        if let Ok(v) = self.result {
            self.env.ans = v;
            let plot = if eval::uses_x(&self.input) {
                Mode::Alt2
            } else {
                Mode::Inactive
            };
            self.history
                .insert(0, (format!("{} = {}", self.input, fmt(v)), plot));
            self.exprs.insert(0, self.input.clone());
            self.input.clear();
        }
    }

    fn keyboard(&mut self, ui: &egui::Ui) {
        if ui.ctx().memory(|m| m.focused().is_some()) {
            return; // a widget has keyboard focus: Enter/Space belong to it
        }
        let events = ui.input(|i| i.events.clone());
        for e in events {
            match e {
                egui::Event::Text(t) => self.input.push_str(&t),
                egui::Event::Key {
                    key, pressed: true, ..
                } => match key {
                    egui::Key::Enter => self.evaluate(),
                    egui::Key::Backspace => {
                        self.input.pop();
                    }
                    egui::Key::Escape => self.input.clear(),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    fn keypad(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let key = egui::vec2(72.0, 44.0);
        ui.horizontal(|ui| {
            for (label, ins) in FN_KEYS {
                let mut m = Mode::Inactive; // momentary: the gesture picks the variant
                mode_button(ui, &t, label, &mut m, key).on_hover_text(format!(
                    "click {} · double {} · triple {}",
                    ins[0], ins[1], ins[2]
                ));
                let i = match m {
                    Mode::Primary => Some(0),
                    Mode::Alt1 => Some(1),
                    Mode::Alt2 => Some(2),
                    Mode::Inactive => None,
                };
                if let Some(i) = i {
                    self.input.push_str(ins[i]);
                }
            }
        });
        for row in KEYS {
            ui.horizontal(|ui| {
                for k in row {
                    if action_button(ui, &t, k, key).clicked() {
                        self.input.push_str(k);
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            if action_button(ui, &t, "AC", key).clicked() {
                self.input.clear();
                self.result = Ok(0.0);
            }
            if action_button(ui, &t, "Del", key).clicked() {
                self.input.pop();
            }
            if action_button(ui, &t, "Ans", key).clicked() {
                self.input.push_str("ans");
            }
            let wide = egui::vec2(key.x * 2.0 + ui.spacing().item_spacing.x, key.y);
            if action_button(ui, &t, "=", wide).clicked() {
                self.evaluate();
            }
        });
    }

    fn history_ui(&mut self, ui: &mut egui::Ui) {
        let before: Vec<Mode> = self.history.iter().map(|r| r.1).collect();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height())
            .show(ui, |ui| {
                mode_list(ui, &self.theme, "history", &mut self.history, super::LIST_ROW);
            });
        for (i, old) in before.iter().enumerate() {
            let m = self.history[i].1;
            if m == *old {
                continue;
            }
            match m {
                Mode::Primary => {
                    let v = self.history[i]
                        .0
                        .rsplit(" = ")
                        .next()
                        .unwrap_or("")
                        .to_owned();
                    self.input.push_str(&v);
                }
                Mode::Alt1 => self.input = self.exprs[i].clone(),
                _ => {}
            }
            // Primary/Alt1 are one-shot selections: only the latest row keeps them.
            for (j, r) in self.history.iter_mut().enumerate() {
                if j != i && matches!(r.1, Mode::Primary | Mode::Alt1) {
                    r.1 = Mode::Inactive;
                }
            }
        }
    }

    fn graph_ui(&mut self, ui: &mut egui::Ui, size: egui::Vec2) {
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::drag());
        let g = &mut self.graph;
        if resp.dragged() {
            let d = resp.drag_delta();
            g.cx -= d.x as f64 / g.ppu;
            g.cy += d.y as f64 / g.ppu;
        }
        if resp.hovered() {
            let z = ui.input(|i| i.smooth_scroll_delta.y);
            if z != 0.0 {
                g.ppu = (g.ppu * (1.0 + z as f64 * 0.002)).clamp(2.0, 2000.0);
            }
        }
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 4.0, palette::SURFACE);
        let to_px = |x: f64, y: f64| {
            egui::pos2(
                rect.center().x + ((x - g.cx) * g.ppu) as f32,
                rect.center().y - ((y - g.cy) * g.ppu) as f32,
            )
        };
        // grid at a "nice" step of ~50 px
        let raw = 50.0 / g.ppu;
        let mag = 10f64.powf(raw.log10().floor());
        let step = [1.0, 2.0, 5.0, 10.0]
            .iter()
            .map(|m| m * mag)
            .find(|s| *s >= raw)
            .unwrap();
        let (x0, x1) = (
            g.cx - rect.width() as f64 / 2.0 / g.ppu,
            g.cx + rect.width() as f64 / 2.0 / g.ppu,
        );
        let (y0, y1) = (
            g.cy - rect.height() as f64 / 2.0 / g.ppu,
            g.cy + rect.height() as f64 / 2.0 / g.ppu,
        );
        let grid = egui::Stroke::new(1.0, palette::STRIP);
        let axis = egui::Stroke::new(1.0, palette::TEXT_MUTED);
        let mut x = (x0 / step).floor() * step;
        while x <= x1 {
            p.line_segment([to_px(x, y0), to_px(x, y1)], grid);
            x += step;
        }
        let mut y = (y0 / step).floor() * step;
        while y <= y1 {
            p.line_segment([to_px(x0, y), to_px(x1, y)], grid);
            y += step;
        }
        p.line_segment([to_px(x0, 0.0), to_px(x1, 0.0)], axis);
        p.line_segment([to_px(0.0, y0), to_px(0.0, y1)], axis);
        let font = egui::FontId::monospace(self.theme.data_size * 0.8);
        p.text(
            rect.left_top() + egui::vec2(6.0, 4.0),
            egui::Align2::LEFT_TOP,
            format!("grid {}", fmt(step)),
            font,
            palette::TEXT_MUTED,
        );

        // functions: plotted history rows (purple) + live preview of the input
        let mut fns: Vec<(String, egui::Color32)> = self
            .history
            .iter()
            .zip(&self.exprs)
            .filter(|(r, _)| r.1 == Mode::Alt2)
            .enumerate()
            .map(|(k, (_, e))| {
                (
                    e.clone(),
                    // plotted rows are purple (Alt2): vary tint, never other state hues
                    rustcars::tint(Mode::Alt2.colors().label, [0.0, 0.45, -0.3][k % 3]),
                )
            })
            .collect();
        if eval::uses_x(&self.input) {
            fns.push((self.input.clone(), self.theme.text));
        }
        for (src, color) in fns {
            let mut env = Env {
                angle: self.env.angle,
                x: 0.0,
                ans: self.env.ans,
            };
            let mut pts: Vec<egui::Pos2> = vec![];
            let n = rect.width() as usize;
            for i in 0..=n {
                env.x = x0 + (x1 - x0) * i as f64 / n as f64;
                match eval::eval(&src, &env) {
                    Ok(v) if v.is_finite() && (y0 - 1e3..y1 + 1e3).contains(&v) => {
                        pts.push(to_px(env.x, v))
                    }
                    _ => {
                        if pts.len() > 1 {
                            p.add(egui::Shape::line(
                                std::mem::take(&mut pts),
                                egui::Stroke::new(2.0, color),
                            ));
                        }
                        pts.clear();
                    }
                }
            }
            if pts.len() > 1 {
                p.add(egui::Shape::line(pts, egui::Stroke::new(2.0, color)));
            }
        }
    }
}

impl Calc {
    pub fn angle_label(&self) -> &'static str {
        if self.env.angle == Angle::Deg { "DEG" } else { "RAD" }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        self.keyboard(ui);
        super::screen_header(ui, "calc_header", &t, |ui| {
                ui.horizontal(|ui| {
                    super::title_block(ui, &t, "Calc", palette::TAN, egui::vec2(110.0, t.bar_thin));
                    let label = if self.env.angle == Angle::Deg {
                        "Deg"
                    } else {
                        "Rad"
                    };
                    if lcars_button(
                        ui,
                        &t,
                        label,
                        palette::TEAL,
                        egui::vec2(90.0, t.bar_thin),
                        Cap::None,
                        true,
                    )
                    .on_hover_text("Toggle degrees / radians")
                    .clicked()
                    {
                        self.env.angle = if self.env.angle == Angle::Deg {
                            Angle::Rad
                        } else {
                            Angle::Deg
                        };
                    }
                });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.bg))
            .show(ui, |ui| {
                let full = ui.available_rect_before_wrap();
                let split = full.left() + 590.0_f32.min(full.width() * 0.55); // narrow windows: both panels stay usable
                let left = egui::Rect::from_min_max(full.min, egui::pos2(split, full.bottom()));
                let right =
                    egui::Rect::from_min_max(egui::pos2(split + t.main_gap, full.top()), full.max);
                ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                    let mut f = LcarsFrame::new(&t, Side::Left, 0.6);
                    f.min_height = left.height();
                    f.show(ui, &t, |ui| {
                        // Scrolls in a small window instead of being cut off.
                        egui::ScrollArea::both().id_salt("calc-left").auto_shrink([false, false]).show(ui, |ui| {
                        ui.label(egui::RichText::new("EXPRESSION").size(t.header_size));
                        let shown = if self.input.is_empty() {
                            "_".to_owned()
                        } else {
                            self.input.clone()
                        };
                        ui.label(
                            egui::RichText::new(shown)
                                .monospace()
                                .size(t.header_size)
                                .color(t.text),
                        );
                        match &self.result {
                            Ok(v) => readout(ui, &t, &format!("= {}", fmt(*v)), true),
                            Err(e) => readout(ui, &t, &format!("ERROR: {e}"), false),
                        }
                        ui.add_space(t.main_gap);
                        ui.label(
                            egui::RichText::new(
                                "FUNCTIONS: CLICK F · DOUBLE INVERSE · TRIPLE HYPERBOLIC",
                            )
                            .size(t.data_size),
                        );
                        self.keypad(ui);
                        });
                    });
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
                    // No right-hand elbow (as NanoVNA): the panel uses the whole rect.
                    ui.set_min_height(right.height());
                    (|ui: &mut egui::Ui| {
                        egui::ScrollArea::vertical().id_salt("calc-right").auto_shrink([false, false]).show(ui, |ui| {
                        ui.label(egui::RichText::new("GRAPH  Y = F(X)").size(t.header_size));
                        let w = ui.available_width();
                        self.graph_ui(ui, egui::vec2(w, 300.0));
                        ui.label(egui::RichText::new("HISTORY").size(t.header_size));
                        self.history_ui(ui);
                        });
                    })(ui);
                });
            });
    }
}

pub fn fmt(v: f64) -> String {
    if v != 0.0 && (v.abs() >= 1e10 || v.abs() < 1e-6) {
        format!("{v:.6e}")
    } else {
        let s = format!("{v:.10}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

