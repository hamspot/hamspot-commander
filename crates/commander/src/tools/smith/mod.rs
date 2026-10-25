//! Smith chart built from rustcars widgets and the egui painter.
//! Run: `cargo run --example smith` (desktop only).
//!
//! * Enter a load R + jX, Z0 and frequency: the load is plotted with its constant-SWR circle,
//!   and the panel shows Gamma (magnitude and angle), SWR, return loss, mismatch loss and the
//!   normalized impedance and admittance.
//! * LINE rotates the load toward the generator along the SWR circle (length in wavelengths,
//!   degrees, or metres with a velocity factor) and shows the input impedance.
//! * Hover the chart to read the impedance under the pointer. Clicking places markers by
//!   click mode (AGENTS.md P2): click = primary (green), double = alt 1 (orange),
//!   triple = alt 2 (purple), each with its pips. Repeating a gesture on the same spot is fine;
//!   CLEAR MARKERS removes them.
//! * MATCH draws a single shunt-stub match (short or open stub) on the chart.
//!
//! Maths in `math.rs`; SWR / return-loss figures reuse `ham_rf`'s `calc.rs`.
#[allow(dead_code)]
#[path = "../ham_rf/calc.rs"]
mod calc;
pub(crate) mod math;

use egui::{Color32, Pos2, Stroke, Ui, vec2};
use math::{Cx, LenUnit, Stub, cx};
use rustcars::{
    LcarsFrame, LcarsTheme, Mode, Side, action_button, apply_theme, handle_mode,
    install_lcars_font, mode_button, paint_pips, palette, readout, tint,
};

fn g(v: f64) -> String {
    if !v.is_finite() {
        return "INF".into();
    }
    if v == 0.0 {
        return "0".into();
    }
    let dec = (2 - v.abs().log10().floor() as i32).clamp(0, 6) as usize;
    format!("{v:.dec$}")
}

fn cxs(z: Cx) -> String {
    if !z.re.is_finite() || !z.im.is_finite() {
        return "OPEN (INF)".into();
    }
    let s = if z.im < 0.0 { '-' } else { '+' };
    format!("{} {s} j{}", g(z.re), g(z.im.abs()))
}

fn num(s: &str) -> Result<f64, String> {
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("'{}' is not a number", s.trim()))
}

/// Same picker idiom as ham_rf: one button, click/double/triple pick choices 1/2/3.
fn pick(ui: &mut Ui, t: &LcarsTheme, title: &str, labels: &[&str], idx: &mut usize) {
    const MODES: [Mode; 3] = [Mode::Primary, Mode::Alt1, Mode::Alt2];
    ui.label(
        egui::RichText::new(title)
            .size(t.data_size)
            .color(palette::TEXT_MUTED),
    );
    let w = labels.iter().map(|l| l.len()).max().unwrap_or(4) as f32 * 9.0 + 56.0;
    let mut m = MODES[*idx];
    let hint = ["CLICK", "DOUBLE", "TRIPLE"]
        .iter()
        .zip(labels)
        .map(|(g, l)| format!("{g} {}", l.to_uppercase()))
        .collect::<Vec<_>>()
        .join(" · ");
    mode_button(ui, t, labels[*idx], &mut m, vec2(w.max(110.0), 34.0)).on_hover_text(&hint);
    if let Some(n) = MODES.iter().position(|x| *x == m) {
        if n < labels.len() {
            *idx = n;
        }
    }
}

fn field(ui: &mut Ui, t: &LcarsTheme, label: &str, text: &mut String, w: f32) {
    ui.vertical(|ui| {
        ui.label(
            egui::RichText::new(label)
                .size(t.data_size)
                .color(palette::TEXT_MUTED),
        );
        ui.add(
            egui::TextEdit::singleline(text)
                .font(egui::TextStyle::Monospace)
                .desired_width(w),
        )
        .on_hover_text(label);
    });
}

fn kv(ui: &mut Ui, t: &LcarsTheme, id: &str, rows: &[(&str, String)]) {
    egui::Grid::new(id)
        .spacing(vec2(t.main_gap, 2.0))
        .show(ui, |ui| {
            for (k, v) in rows {
                ui.label(egui::RichText::new(*k).color(palette::TEXT_MUTED));
                readout(ui, t, v, false);
                ui.end_row();
            }
        });
}

// chart colours (all >= 3:1 against palette::BG for the drawn data, grid is decorative)
const GRID_Z: Color32 = Color32::from_rgb(0x4A, 0x5A, 0x6E);
const GRID_Y: Color32 = Color32::from_rgb(0x6E, 0x4E, 0x48);
const LOAD: Color32 = palette::TAN;
const SWR_C: Color32 = Color32::from_rgb(0x6F, 0xA8, 0xA0);
const ZIN: Color32 = palette::TEXT;
const MATCH_C: Color32 = Color32::from_rgb(0xC9, 0xA2, 0x6B);

pub struct Smith {
    pub theme: LcarsTheme,
    pub r: String,
    pub x: String,
    pub z0: String,
    pub f: String,
    pub len: String,
    pub len_unit: usize,
    pub vf: String,
    pub grid: usize,
    pub matching: usize,
    /// Markers in the Gamma plane, by mode: [primary, alt1, alt2].
    pub markers: [Option<Cx>; 3],
    pub last_click: Option<Cx>,
    pub hover: Option<Cx>,
}

struct Solved {
    z0: f64,
    zl: Cx,
    gl: Cx,
    line: f64,
    gin: Cx,
}

impl Smith {
    pub fn new(ctx: &egui::Context) -> Self {
        let theme = LcarsTheme::default();
        install_lcars_font(ctx);
        apply_theme(ctx, &theme);
        let s = |v: &str| v.to_owned();
        Self {
            theme,
            r: s("100"),
            x: s("50"),
            z0: s("50"),
            f: s("146"),
            len: s("0.1"),
            len_unit: 0,
            vf: s("0.66"),
            grid: 0,
            matching: 0,
            markers: [None; 3],
            last_click: None,
            hover: None,
        }
    }

    fn solve(&self) -> Result<Solved, String> {
        let (r, x, z0, f, len, vf) = (
            num(&self.r)?,
            num(&self.x)?,
            num(&self.z0)?,
            num(&self.f)?,
            num(&self.len)?,
            num(&self.vf)?,
        );
        if r < 0.0 {
            return Err("R must be 0 or more (passive load)".into());
        }
        if z0 <= 0.0 || f <= 0.0 || !(0.0..=1.0).contains(&vf) || vf == 0.0 {
            return Err("Z0 and frequency must be > 0, VF in 0..1".into());
        }
        let unit = [LenUnit::Wavelengths, LenUnit::Degrees, LenUnit::Meters][self.len_unit];
        let line = math::to_wavelengths(len, unit, f, vf);
        let zl = cx(r, x);
        let gl = math::gamma(zl.scale(1.0 / z0));
        Ok(Solved {
            z0,
            zl,
            gl,
            line,
            gin: math::toward_generator(gl, line),
        })
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let t = self.theme;
        super::screen_header(ui, "smith_header", &t, |ui| {
                ui.horizontal(|ui| {
                    super::title_block(ui, &t, "Smith", palette::TAN, vec2(120.0, t.bar_thin));
                    ui.add_space(t.main_gap);
                    super::screen_title(ui, &t, "SMITH CHART");
                });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.bg))
            .show(ui, |ui| {
                let full = ui.available_rect_before_wrap();
                let split = full.left() + 470.0;
                let left = egui::Rect::from_min_max(full.min, egui::pos2(split, full.bottom()));
                let right =
                    egui::Rect::from_min_max(egui::pos2(split + t.main_gap, full.top()), full.max);
                ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                    let mut f = LcarsFrame::new(&t, Side::Left, 0.6);
                    f.min_height = left.height();
                    f.show(ui, &t, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .max_height(ui.available_height())
                            .show(ui, |ui| self.panel(ui));
                    });
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
                    let mut f = LcarsFrame::new(&t, Side::Right, 0.6);
                    f.top_color = palette::PLUM;
                    f.bottom_color = palette::SLATE;
                    f.min_height = right.height();
                    f.show(ui, &t, |ui| self.chart(ui));
                });
            });
    }

    fn panel(&mut self, ui: &mut Ui) {
        let t = self.theme;
        ui.label(egui::RichText::new("LOAD").size(t.header_size));
        ui.horizontal(|ui| {
            field(ui, &t, "R (ohm)", &mut self.r, 80.0);
            field(ui, &t, "X (ohm)", &mut self.x, 80.0);
            field(ui, &t, "Z0 (ohm)", &mut self.z0, 70.0);
            field(ui, &t, "MHz", &mut self.f, 80.0);
        });
        ui.label(egui::RichText::new("LINE TOWARD GENERATOR").size(t.header_size));
        ui.horizontal(|ui| {
            field(ui, &t, "Length", &mut self.len, 80.0);
            field(ui, &t, "VF", &mut self.vf, 70.0);
            ui.vertical(|ui| {
                pick(
                    ui,
                    &t,
                    "UNIT",
                    &["wavelengths", "degrees", "metres"],
                    &mut self.len_unit,
                )
            });
        });
        let s = match self.solve() {
            Ok(s) => s,
            Err(e) => {
                ui.label(
                    egui::RichText::new(format!("ERROR: {e}")).color(Mode::Alt1.colors().label),
                );
                return;
            }
        };
        let zn = s.zl.scale(1.0 / s.z0);
        let swr_txt = |gm: f64| match calc::Reflection::from_gamma(gm.min(1.0)) {
            Ok(r) => (
                format!("{} : 1", g(r.swr)),
                format!("{} dB", g(r.return_loss_db)),
                format!("{} dB", g(r.mismatch_loss_db)),
            ),
            Err(_) => ("n/a".into(), "n/a".into(), "n/a".into()),
        };
        let (swr, rl, ml) = swr_txt(s.gl.abs());
        let zin = math::z_of(s.gin);
        kv(
            ui,
            &t,
            "load_kv",
            &[
                (
                    "GAMMA",
                    format!("{} at {} deg", g(s.gl.abs()), g(s.gl.arg().to_degrees())),
                ),
                ("SWR", swr),
                ("RETURN LOSS", rl),
                ("MISMATCH LOSS", ml),
                ("z = Z / Z0", cxs(zn)),
                ("y = 1 / z", cxs(zn.inv())),
                ("LINE", format!("{} wavelengths", g(s.line))),
                ("ZIN (OHM)", cxs(zin.scale(s.z0))),
                ("zin", cxs(zin)),
            ],
        );
        ui.add_space(t.main_gap / 2.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                pick(
                    ui,
                    &t,
                    "GRID",
                    &["Impedance", "Z + admittance"],
                    &mut self.grid,
                )
            });
            ui.vertical(|ui| {
                pick(
                    ui,
                    &t,
                    "MATCH",
                    &["Off", "Short stub", "Open stub"],
                    &mut self.matching,
                )
            });
        });
        if self.matching > 0 {
            let kind = if self.matching == 1 {
                Stub::Short
            } else {
                Stub::Open
            };
            match math::stub_match(zn, kind) {
                Some(m) => kv(
                    ui,
                    &t,
                    "match_kv",
                    &[
                        ("STUB AT", format!("{} wavelengths from load", g(m.d))),
                        ("y THERE", cxs(m.y_at_d)),
                        (
                            "STUB LENGTH",
                            format!(
                                "{} wavelengths ({})",
                                g(m.stub_len),
                                if kind == Stub::Short {
                                    "shorted"
                                } else {
                                    "open"
                                }
                            ),
                        ),
                    ],
                ),
                None => {
                    ui.label(
                        egui::RichText::new("NO STUB NEEDED (MATCHED) OR LOAD IS PURE REACTANCE")
                            .color(palette::TEXT_MUTED),
                    );
                }
            }
        }
        ui.add_space(t.main_gap / 2.0);
        ui.label(egui::RichText::new("MARKERS").size(t.header_size));
        ui.label(
            egui::RichText::new("CLICK THE CHART · DOUBLE ALT 1 · TRIPLE ALT 2")
                .color(palette::TEXT_MUTED),
        );
        for (i, m) in [Mode::Primary, Mode::Alt1, Mode::Alt2].iter().enumerate() {
            if let Some(p) = self.markers[i] {
                let z = math::z_of(p);
                for line in [
                    format!("{:<7} z {}", m.short_name().to_uppercase(), cxs(z)),
                    format!("{:<7} Z {} ohm", "", cxs(z.scale(s.z0))),
                ] {
                    ui.label(
                        egui::RichText::new(line)
                            .monospace()
                            .color(m.colors().label),
                    );
                }
            }
        }
        if self.markers.iter().any(|m| m.is_some())
            && action_button(ui, &t, "Clear markers", vec2(180.0, 34.0)).clicked()
        {
            self.markers = [None; 3];
        }
    }

    fn chart(&mut self, ui: &mut Ui) {
        let t = self.theme;
        let avail = ui.available_size();
        let readout_h = 28.0;
        let side = avail.x.min(avail.y - readout_h).max(120.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(avail.x, side), egui::Sense::click());
        let center = rect.center();
        let rad = side / 2.0 - 30.0; // room for rim labels
        let to_px = |p: Cx| {
            egui::pos2(
                center.x + (p.re as f32) * rad,
                center.y - (p.im as f32) * rad,
            )
        };
        let to_g = |q: Pos2| {
            cx(
                ((q.x - center.x) / rad) as f64,
                ((center.y - q.y) / rad) as f64,
            )
        };
        let painter = ui.painter_at(rect);
        let poly = |pts: &[Cx], stroke: Stroke| {
            if pts.len() > 1 {
                painter.add(egui::Shape::line(
                    pts.iter().map(|p| to_px(*p)).collect(),
                    stroke,
                ));
            }
        };
        let circle = |c: Cx, r: f64, stroke: Stroke| {
            for run in math::clipped_circle(c, r, 720) {
                poly(&run, stroke);
            }
        };
        let font = egui::FontId::monospace(t.data_size * 0.8);
        painter.circle_filled(center, rad, palette::SURFACE);
        // admittance grid: the impedance grid mirrored through the centre
        if self.grid == 1 {
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

        if let Ok(s) = self.solve() {
            let m = s.gl.abs();
            if m <= 1.0 {
                painter.circle_stroke(center, m as f32 * rad, Stroke::new(1.5, SWR_C));
            }
            // line rotation arc, load -> input
            let steps = 120;
            let arc: Vec<Cx> = (0..=steps)
                .map(|i| math::toward_generator(s.gl, s.line * i as f64 / steps as f64))
                .collect();
            poly(&arc, Stroke::new(3.0, ZIN));
            if self.matching > 0 {
                let kind = if self.matching == 1 {
                    Stub::Short
                } else {
                    Stub::Open
                };
                if let Some(mt) = math::stub_match(s.zl.scale(1.0 / s.z0), kind) {
                    // drawn in the admittance plane (y = -Gamma): line to the g = 1 circle,
                    // then the stub moves along it to the centre
                    let path: Vec<Cx> = (0..=steps)
                        .map(|i| {
                            math::toward_generator(s.gl, mt.d * i as f64 / steps as f64).scale(-1.0)
                        })
                        .collect();
                    poly(&path, Stroke::new(2.0, MATCH_C));
                    let pd = *path.last().unwrap();
                    let yl = to_px(s.gl.scale(-1.0));
                    painter.circle_stroke(yl, 5.0, Stroke::new(2.0, MATCH_C));
                    painter.text(
                        yl + vec2(-8.0, 8.0),
                        egui::Align2::RIGHT_TOP,
                        "yL",
                        font.clone(),
                        MATCH_C,
                    );
                    painter.line_segment([to_px(pd), center], Stroke::new(2.0, MATCH_C));
                    let (c1, r1) = math::r_circle(1.0);
                    circle(c1.scale(-1.0), r1, Stroke::new(1.0, tint(MATCH_C, -0.3)));
                    painter.circle_filled(to_px(pd), 5.0, MATCH_C);
                    painter.text(
                        to_px(pd) + vec2(8.0, -8.0),
                        egui::Align2::LEFT_BOTTOM,
                        "STUB",
                        font.clone(),
                        MATCH_C,
                    );
                }
            }
            if m <= 1.0 {
                painter.circle_filled(to_px(s.gl), 7.0, LOAD);
                painter.text(
                    to_px(s.gl) + vec2(9.0, 9.0),
                    egui::Align2::LEFT_TOP,
                    "LOAD",
                    font.clone(),
                    LOAD,
                );
                painter.circle_stroke(to_px(s.gin), 7.0, Stroke::new(2.0, ZIN));
                painter.text(
                    to_px(s.gin) + vec2(9.0, -9.0),
                    egui::Align2::LEFT_BOTTOM,
                    "IN",
                    font.clone(),
                    ZIN,
                );
            }
        }

        // markers: a click is remembered; the click-count gesture decides which marker it sets
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                self.last_click = Some(to_g(p));
            }
        }
        let mut gesture = Mode::Inactive;
        handle_mode(ui, &resp, &mut gesture);
        let slot = match gesture {
            Mode::Primary => Some(0),
            Mode::Alt1 => Some(1),
            Mode::Alt2 => Some(2),
            Mode::Inactive => None,
        };
        if let (Some(i), Some(p)) = (slot, self.last_click) {
            if p.abs() <= 1.0 {
                self.markers[i] = Some(p);
            }
        }
        for (i, mode) in [Mode::Primary, Mode::Alt1, Mode::Alt2].iter().enumerate() {
            if let Some(p) = self.markers[i] {
                let c = mode.colors();
                let q = to_px(p);
                painter.circle(q, 9.0, c.fill, Stroke::new(1.5, c.label));
                paint_pips(ui, q, 14.0, mode.pips(), c.label);
            }
        }
        // hover read-out
        let hover = resp.hover_pos().map(to_g).filter(|p| p.abs() <= 1.0);
        if hover.is_some() {
            self.hover = hover;
        }
        let z0 = num(&self.z0).unwrap_or(50.0);
        let text = match self.hover.filter(|_| hover.is_some()).or(hover) {
            Some(p) => {
                let z = math::z_of(p);
                format!(
                    "POINTER  z = {}   Z = {} ohm   |GAMMA| {}",
                    cxs(z),
                    cxs(z.scale(z0)),
                    g(p.abs())
                )
            }
            None => "HOVER THE CHART TO READ IMPEDANCE · CLICK TO PLACE MARKERS".into(),
        };
        readout(ui, &t, &text, hover.is_some());
    }
}


#[cfg(test)]
mod logic {
    use super::*;

    #[test]
    fn app_solves_defaults() {
        let ctx = egui::Context::default();
        let mut a = Smith::new(&ctx);
        let s = a.solve().unwrap();
        assert!((s.gl.abs() - 0.4472).abs() < 1e-3);
        a.r = "-5".into();
        assert!(a.solve().is_err());
        a.r = "abc".into();
        assert!(a.solve().is_err());
        assert_eq!(cxs(cx(1.0, -0.5)), "1.00 - j0.500");
    }
}
