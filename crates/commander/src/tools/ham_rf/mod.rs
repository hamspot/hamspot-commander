//! Ham radio RF calculators built only from rustcars widgets.
//! Run: `cargo run --example ham_rf` (desktop only).
//!
//! Ten tools, one shared FREQUENCY on the left (so you type 14.2 once). Click modes (AGENTS.md P2)
//! are used as *variants of one function*, never as unrelated actions:
//! * unit and direction pickers: one button, click = first choice, double = second, triple = third
//!   (frequency unit MHz/kHz/GHz, length unit ft/m/in, SWR input as SWR/return loss/reflection...).
//! * result and band rows (`mode_list`): antenna rows copy a length (click = selected unit,
//!   double = the other system, triple = whole line); band rows set the frequency
//!   (click = band centre, double = lower edge, triple = upper edge); cable rows (click = pick the cable,
//!   double = pin it as the comparison).
//!
//! The maths is in `calc.rs` and the band table in `bands.rs`, both unit tested.
//! Everything here is approximate bench math: trim real antennas with an analyzer, and
//! check the current FCC rules before you transmit.
pub(crate) mod bands;
mod calc;

use calc::{CABLES, DistUnit, FreqUnit, LNetwork, LcSolve, LenUnit, PowerUnit, Q, Reflection};
use egui::{Ui, vec2};
use rustcars::{
    Cap, LcarsFrame, LcarsTheme, Mode, Side, action_button, apply_theme, install_lcars_font,
    lcars_button, mode_button, mode_list, palette, readout, tint,
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tool {
    Wave,
    Antenna,
    Coax,
    Swr,
    Power,
    Ohm,
    Lc,
    Match,
    Path,
    Bands,
}

/// How many calculators the picker has.
pub const TOOL_COUNT: usize = 10;

const TOOLS: [(Tool, &str, &str); TOOL_COUNT] = [
    (Tool::Wave, "Wave", "FREQUENCY AND WAVELENGTH"),
    (Tool::Antenna, "Antenna", "ANTENNA ELEMENT LENGTHS"),
    (Tool::Coax, "Coax", "COAX LOSS (APPROXIMATE)"),
    (Tool::Swr, "SWR", "SWR, RETURN LOSS, MISMATCH"),
    (Tool::Power, "Power", "DB, POWER AND VOLTAGE"),
    (Tool::Ohm, "Ohm", "OHM'S LAW AND POWER"),
    (Tool::Lc, "LC", "LC RESONANCE"),
    (Tool::Match, "Match", "L-NETWORK MATCHING"),
    (Tool::Path, "Path", "FREE-SPACE PATH LOSS"),
    (Tool::Bands, "Bands", "US BAND PLAN (INFORMATIONAL)"),
];

/// The function key labels, in order.
pub fn tool_labels() -> Vec<String> {
    TOOLS.iter().map(|(_, l, _)| (*l).to_owned()).collect()
}

/// Format with about five significant figures.
pub fn g(v: f64) -> String {
    gs(v, 5)
}

/// Format with `sig` significant figures (at least that many, fewer for big numbers).
pub fn gs(v: f64, sig: i32) -> String {
    if v.is_nan() {
        return "n/a".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "INF".into() } else { "-INF".into() };
    }
    if v == 0.0 {
        return "0".into();
    }
    let dec = (sig - 1 - v.abs().log10().floor() as i32).clamp(0, 9) as usize;
    format!("{v:.dec$}")
}

/// Band edge without trailing zeros: 0.1357, 14, 14.35.
fn edge(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn num(s: &str) -> Result<f64, String> {
    let t = s.trim().replace(',', "");
    if t.is_empty() {
        return Err("enter a value".into());
    }
    t.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("'{}' is not a number", s.trim()))
}

fn pos(s: &str) -> Result<f64, String> {
    let v = num(s)?;
    if v > 0.0 {
        Ok(v)
    } else {
        Err("value must be greater than zero".into())
    }
}

/// One control, up to three variants (click / double / triple). A repeated gesture can't switch
/// it off: a picker always has a value. Gestures beyond the number of labels are ignored.
pub fn pick(ui: &mut Ui, t: &LcarsTheme, title: &str, labels: &[&str], idx: &mut usize) {
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
    mode_button(ui, t, labels[*idx], &mut m, vec2(w.max(110.0), 36.0)).on_hover_text(&hint);
    if let Some(n) = MODES.iter().position(|x| *x == m) {
        if n < labels.len() {
            *idx = n;
        }
    }
}

/// A row of single-choice buttons (selected one is green, any gesture selects).
fn radio_row(ui: &mut Ui, t: &LcarsTheme, labels: &[&str], idx: &mut usize) {
    ui.horizontal_wrapped(|ui| {
        for (i, l) in labels.iter().enumerate() {
            let mut m = if i == *idx {
                Mode::Primary
            } else {
                Mode::Inactive
            };
            mode_button(ui, t, l, &mut m, vec2(72.0, 34.0));
            if m != Mode::Inactive && i != *idx {
                *idx = i;
            }
        }
    });
}

fn field(
    ui: &mut Ui,
    t: &LcarsTheme,
    label: &str,
    text: &mut String,
    width: f32,
) -> egui::Response {
    ui.label(
        egui::RichText::new(label)
            .size(t.data_size)
            .color(palette::TEXT_MUTED),
    );
    ui.add(
        egui::TextEdit::singleline(text)
            .font(egui::TextStyle::Monospace)
            .desired_width(width),
    )
    .on_hover_text(label)
}

fn kv(ui: &mut Ui, t: &LcarsTheme, id: &str, rows: &[(&str, String)]) {
    egui::Grid::new(id)
        .spacing(vec2(t.main_gap, t.frame_gap))
        .show(ui, |ui| {
            for (k, v) in rows {
                ui.label(
                    egui::RichText::new(*k)
                        .size(t.data_size)
                        .color(palette::TEXT_MUTED),
                );
                readout(ui, t, v, false);
                ui.end_row();
            }
        });
}

/// Input errors go to the Tools status line (plain text, "ERROR:").
fn error(ui: &mut Ui, _t: &LcarsTheme, msg: &str) {
    crate::status_note::note(ui, format!("ERROR: {msg}"));
}

/// Explanations and approximation notes go to the Tools status line.
fn note(ui: &mut Ui, _t: &LcarsTheme, msg: &str) {
    crate::status_note::note(ui, msg);
}

fn heading(ui: &mut Ui, t: &LcarsTheme, s: &str) {
    ui.label(egui::RichText::new(s).size(t.header_size));
}

pub struct HamRf {
    pub theme: LcarsTheme,
    pub tool: Tool,
    // shared
    pub freq: String,
    pub funit: usize,
    pub lunit: usize,
    pub status: String,
    // wave
    pub wave_dir: usize,
    pub wave_in: String,
    // antenna
    pub ant_modes: Vec<Mode>,
    // coax
    pub coax_len: String,
    pub coax_unit: usize,
    pub coax_power: String,
    pub coax_swr: String,
    pub coax_modes: Vec<Mode>,
    // swr
    pub swr_from: usize,
    pub swr_in: String,
    // power
    pub pow_unit: usize,
    pub pow_in: String,
    pub pow_r: String,
    pub ratio_db: String,
    pub ratio_x: String,
    // ohm
    pub ohm: [String; 4],
    pub ohm_order: Vec<usize>,
    // lc
    pub lc_solve: usize,
    pub lc: [String; 3],
    // match
    pub match_form: usize,
    pub rs: String,
    pub rl: String,
    // path
    pub dist_unit: usize,
    pub dist: String,
    pub tx_w: String,
    pub gain_tx: String,
    pub gain_rx: String,
    pub line_loss: String,
    // bands
    pub band_modes: Vec<Mode>,
}

impl HamRf {
    pub fn new(ctx: &egui::Context) -> Self {
        let theme = LcarsTheme::default();
        install_lcars_font(ctx);
        apply_theme(ctx, &theme);
        let s = |x: &str| x.to_owned();
        let mut coax_modes = vec![Mode::Inactive; CABLES.len()];
        coax_modes[3] = Mode::Primary; // RG-213
        coax_modes[1] = Mode::Alt1; // RG-58 pinned for comparison
        Self {
            theme,
            tool: Tool::Wave,
            freq: s("14.200"),
            funit: 0,
            lunit: 0,
            status: s("READY"),
            wave_dir: 0,
            wave_in: s("21.1"),
            ant_modes: vec![Mode::Inactive; 11],
            coax_len: s("100"),
            coax_unit: 0,
            coax_power: s("100"),
            coax_swr: s("1.5"),
            coax_modes,
            swr_from: 0,
            swr_in: s("1.5"),
            pow_unit: 0,
            pow_in: s("100"),
            pow_r: s("50"),
            ratio_db: s("3"),
            ratio_x: s("100"),
            ohm: [s("12"), s(""), s("6"), s("")],
            ohm_order: vec![0, 2],
            lc_solve: 0,
            lc: [s("7.15"), s("10"), s("100")],
            match_form: 0,
            rs: s("50"),
            rl: s("200"),
            dist_unit: 0,
            dist: s("10"),
            tx_w: s("5"),
            gain_tx: s("0"),
            gain_rx: s("0"),
            line_loss: s("1"),
            band_modes: vec![Mode::Inactive; bands::BANDS.len()],
        }
    }

    fn funit(&self) -> FreqUnit {
        FreqUnit::ALL[self.funit]
    }
    fn lunit(&self) -> LenUnit {
        LenUnit::ALL[self.lunit]
    }
    /// The shared frequency in MHz.
    pub fn f_mhz(&self) -> Result<f64, String> {
        pos(&self.freq).map(|v| self.funit().to_mhz(v))
    }
    fn set_f_mhz(&mut self, mhz: f64) {
        self.freq = g(self.funit().mhz_to(mhz));
    }
    fn len(&self, ft: f64) -> String {
        format!("{} {}", g(self.lunit().ft_to(ft)), self.lunit().label())
    }

    fn copy(&mut self, ui: &Ui, text: String) {
        ui.ctx().copy_text(text.clone());
        self.status = format!("COPIED: {text}");
    }

    /// The open calculator's tab label, for the Tools status line.
    pub fn tool_name(&self) -> &'static str {
        TOOLS.iter().find(|x| x.0 == self.tool).map(|x| x.1).unwrap_or("")
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let t = self.theme;
        super::screen_header(ui, "ham_header", &t, |ui| {
                ui.horizontal(|ui| {
                    super::title_block(ui, &t, "RF Calc", palette::TAN, vec2(110.0, t.bar_thin));
                    // Function keys: folder tabs as the NanoVNA band row, each
                    // its own hue, the open one lighter with a light bottom
                    // mark; what does not fit goes behind the more key.
                    let labels = tool_labels();
                    let sel = TOOLS.iter().position(|(tool, _, _)| *tool == self.tool);
                    let rect = super::bar_keys_rect(ui.ctx())
                        .filter(|r| r.width() > 0.0)
                        .unwrap_or_else(|| egui::Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), t.bar_thin)));
                    if let Some(i) = super::bar_tab_row_from(ui, rect, "rf-tools", &labels, sel, 3) {
                        self.tool = TOOLS[i].0;
                    }
                });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.bg))
            .show(ui, |ui| {
                let full = ui.available_rect_before_wrap();
                let split = full.left() + 360.0_f32.min(full.width() * 0.55); // narrow windows: both panels stay usable
                let left = egui::Rect::from_min_max(full.min, egui::pos2(split, full.bottom()));
                let right =
                    egui::Rect::from_min_max(egui::pos2(split + t.main_gap, full.top()), full.max);
                ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                    let mut f = LcarsFrame::new(&t, Side::Left, 0.6);
                    f.top_color = palette::TAN;
                    f.bottom_color = palette::TEAL;
                    f.min_height = left.height();
                    f.show(ui, &t, |ui| {
                        egui::ScrollArea::both().id_salt("rf-left").auto_shrink([false, false]).show(ui, |ui| self.shared_panel(ui));
                    });
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
                    // No right-hand elbow (as NanoVNA): the panel uses the whole rect.
                    ui.set_min_height(right.height());
                    (|ui: &mut egui::Ui| {
                        egui::ScrollArea::both().id_salt("rf-right").auto_shrink([false, false]).show(ui, |ui| {
                        let title = TOOLS.iter().find(|x| x.0 == self.tool).unwrap().2;
                        heading(ui, &t, title);
                        if self.status != "READY" && !self.status.is_empty() {
                            crate::status_note::note(ui, self.status.clone());
                        }
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .max_height(ui.available_height())
                            .show(ui, |ui| match self.tool {
                                Tool::Wave => self.tool_wave(ui),
                                Tool::Antenna => self.tool_antenna(ui),
                                Tool::Coax => self.tool_coax(ui),
                                Tool::Swr => self.tool_swr(ui),
                                Tool::Power => self.tool_power(ui),
                                Tool::Ohm => self.tool_ohm(ui),
                                Tool::Lc => self.tool_lc(ui),
                                Tool::Match => self.tool_match(ui),
                                Tool::Path => self.tool_path(ui),
                                Tool::Bands => self.tool_bands(ui),
                            });
                        });
                    })(ui);
                });
            });
    }

    fn shared_panel(&mut self, ui: &mut Ui) {
        let t = self.theme;
        heading(ui, &t, "FREQUENCY");
        field(ui, &t, "Operating frequency", &mut self.freq, 150.0);
        pick(
            ui,
            &t,
            "FREQUENCY UNIT",
            &["MHz", "kHz", "GHz"],
            &mut self.funit,
        );
        ui.add_space(t.main_gap / 2.0);
        pick(ui, &t, "LENGTH UNIT", &["ft", "m", "in"], &mut self.lunit);
        ui.add_space(t.main_gap / 2.0);
        match self.f_mhz() {
            Ok(f) => {
                let wl = calc::wavelength_m(f);
                kv(
                    ui,
                    &t,
                    "shared_kv",
                    &[
                        ("WAVELENGTH", self.len(wl * calc::FT_PER_M)),
                        ("IN MHZ", format!("{} MHz", g(f))),
                    ],
                );
                match bands::find_band(f) {
                    Some(b) => readout(ui, &t, &format!("BAND {}", b.name), true),
                    None => readout(ui, &t, "OUTSIDE US AMATEUR BANDS", false),
                }
            }
            Err(e) => error(ui, &t, &e),
        }
    }

    // ------------------------------------------------------------ tools
    fn tool_wave(&mut self, ui: &mut Ui) {
        let t = self.theme;
        pick(
            ui,
            &t,
            "DIRECTION",
            &["Frequency to wavelength", "Wavelength to frequency"],
            &mut self.wave_dir,
        );
        ui.add_space(t.main_gap / 2.0);
        let lu = self.lunit();
        let wl_m = if self.wave_dir == 0 {
            self.f_mhz().map(calc::wavelength_m)
        } else {
            let label = format!("Wavelength ({})", lu.label());
            field(ui, &t, &label, &mut self.wave_in, 150.0);
            pos(&self.wave_in).map(|v| match lu {
                LenUnit::M => v,
                LenUnit::Ft => v * 0.3048,
                LenUnit::In => v * 0.0254,
            })
        };
        let wl = match wl_m {
            Ok(w) => w,
            Err(e) => return error(ui, &t, &e),
        };
        ui.add_space(t.main_gap / 2.0);
        if self.wave_dir == 1 {
            let f = calc::freq_mhz(wl);
            let u = self.funit();
            readout(
                ui,
                &t,
                &format!("FREQUENCY {} {}", g(u.mhz_to(f)), u.label()),
                true,
            );
            if action_button(ui, &t, "Use as frequency", vec2(200.0, 36.0)).clicked() {
                self.set_f_mhz(f);
                self.status = format!("FREQUENCY SET TO {} MHZ", g(f));
            }
            ui.add_space(t.main_gap / 2.0);
        }
        let rows: Vec<(&str, String)> = [
            ("FULL WAVE", 1.0),
            ("HALF WAVE", 0.5),
            ("QUARTER WAVE", 0.25),
        ]
        .iter()
        .map(|(n, k)| {
            let m = wl * k;
            (
                *n,
                format!(
                    "{} m   {} ft   {} in   {} cm",
                    g(m),
                    g(m * calc::FT_PER_M),
                    g(m * calc::FT_PER_M * 12.0),
                    g(m * 100.0)
                ),
            )
        })
        .collect();
        kv(ui, &t, "wave_kv", &rows);
        note(
            ui,
            &t,
            "Free-space wavelength. Wire antennas are shorter: see ANTENNA.",
        );
    }

    fn tool_antenna(&mut self, ui: &mut Ui) {
        let t = self.theme;
        let f = match self.f_mhz() {
            Ok(f) => f,
            Err(e) => return error(ui, &t, &e),
        };
        let els = calc::antenna_elements(f);
        let lu = self.lunit();
        let rows_txt: Vec<String> = els
            .iter()
            .map(|e| {
                format!(
                    "{:<27}{:>10} ft {:>9} m   {}",
                    e.name,
                    g(e.ft),
                    g(e.ft * 0.3048),
                    e.note
                )
            })
            .collect();
        let mut rows: Vec<(String, Mode)> = rows_txt
            .iter()
            .cloned()
            .zip(self.ant_modes.iter().copied())
            .collect();
        let before = self.ant_modes.clone();
        mode_list(ui, &t, "antennas", &mut rows, super::LIST_ROW);
        for (i, (_, m)) in rows.iter().enumerate() {
            if *m == before[i] {
                continue;
            }
            self.ant_modes.iter_mut().for_each(|x| *x = Mode::Inactive);
            self.ant_modes[i] = *m;
            let ft = els[i].ft;
            match m {
                Mode::Primary => {
                    let text = format!("{} {}", g(lu.ft_to(ft)), lu.label());
                    self.copy(ui, text);
                }
                Mode::Alt1 => {
                    let text = if lu == LenUnit::M {
                        format!("{} ft", g(ft))
                    } else {
                        format!("{} m", g(ft * 0.3048))
                    };
                    self.copy(ui, text);
                }
                Mode::Alt2 => {
                    let text = rows_txt[i].split_whitespace().collect::<Vec<_>>().join(" ");
                    self.copy(ui, text);
                }
                Mode::Inactive => {}
            }
        }
        note(
            ui,
            &t,
            "CLICK COPIES IN THE SELECTED LENGTH UNIT · DOUBLE COPIES THE OTHER SYSTEM · TRIPLE COPIES THE WHOLE LINE",
        );
        note(
            ui,
            &t,
            "Starting points only. Cut long, then trim for lowest SWR. Height, ground and nearby objects change the answer.",
        );
    }

    fn tool_coax(&mut self, ui: &mut Ui) {
        let t = self.theme;
        let f = match self.f_mhz() {
            Ok(f) => f,
            Err(e) => return error(ui, &t, &e),
        };
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                field(ui, &t, "Cable length", &mut self.coax_len, 110.0);
            });
            ui.vertical(|ui| {
                pick(ui, &t, "LENGTH UNIT", &["ft", "m"], &mut self.coax_unit);
            });
            ui.vertical(|ui| {
                field(ui, &t, "TX power (W)", &mut self.coax_power, 90.0);
                field(ui, &t, "Load SWR", &mut self.coax_swr, 90.0);
            });
        });
        let (len, pw, swr) = match (
            pos(&self.coax_len),
            pos(&self.coax_power),
            num(&self.coax_swr),
        ) {
            (Ok(l), Ok(p), Ok(s)) if s >= 1.0 => (l, p, s),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return error(ui, &t, &e),
            _ => return error(ui, &t, "SWR must be 1 or more"),
        };
        let len_ft = if self.coax_unit == 0 {
            len
        } else {
            len * calc::FT_PER_M
        };
        let results: Vec<(f64, bool, f64)> = CABLES
            .iter()
            .map(|c| {
                let (per100, ex) = calc::loss_per_100ft(c, f);
                (per100, ex, per100 * len_ft / 100.0)
            })
            .collect();
        let rows_txt: Vec<String> = CABLES
            .iter()
            .zip(&results)
            .map(|(c, (per100, ex, total))| {
                format!(
                    "{:<17}{:>6} dB/100ft{:>7} dB matched{}",
                    c.name,
                    gs(*per100, 3),
                    gs(*total, 3),
                    if *ex { "  (extrapolated)" } else { "" }
                )
            })
            .collect();
        let before = self.coax_modes.clone();
        let mut rows: Vec<(String, Mode)> = rows_txt
            .iter()
            .cloned()
            .zip(self.coax_modes.iter().copied())
            .collect();
        mode_list(ui, &t, "cables", &mut rows, super::LIST_ROW);
        for (i, (_, m)) in rows.iter().enumerate() {
            if *m == before[i] {
                continue;
            }
            // one chosen cable (Primary) and one pinned comparison (Alt1) at a time
            for (j, x) in self.coax_modes.iter_mut().enumerate() {
                if j != i && *x == *m && *m != Mode::Inactive {
                    *x = Mode::Inactive;
                }
            }
            self.coax_modes[i] = *m;
        }
        note(
            ui,
            &t,
            "CLICK PICKS THE CABLE · DOUBLE PINS IT FOR COMPARISON",
        );
        ui.add_space(t.main_gap / 2.0);
        let report = |ui: &mut Ui, tag: &str, i: usize| {
            let (per100, ex, total) = results[i];
            let with_swr = calc::loss_with_swr_db(total, swr);
            let out = pw * 10f64.powf(-with_swr / 10.0);
            readout(
                ui,
                &t,
                &format!(
                    "{tag} {} (VF {}): {} dB matched, {} dB with SWR {}, {} W of {} W arrives ({}%)",
                    CABLES[i].name,
                    CABLES[i].vf,
                    gs(total, 3),
                    gs(with_swr, 3),
                    g(swr),
                    gs(out, 3),
                    g(pw),
                    gs(out / pw * 100.0, 3)
                ),
                tag == "PICKED",
            );
            if ex {
                note(
                    ui,
                    &t,
                    &format!(
                        "{per100:.2} dB/100 ft is extrapolated beyond the table (1 to 900 MHz)."
                    ),
                );
            }
        };
        let find = |m: Mode| self.coax_modes.iter().position(|x| *x == m);
        match find(Mode::Primary) {
            Some(i) => report(ui, "PICKED", i),
            None => note(
                ui,
                &t,
                "Click a cable to see the loss for your power and SWR.",
            ),
        }
        if let Some(i) = find(Mode::Alt1) {
            report(ui, "PINNED", i);
        }
        note(
            ui,
            &t,
            "APPROXIMATE: typical datasheet values, rounded, log-interpolated. Real cable varies with age, water, connectors and maker.",
        );
    }

    fn tool_swr(&mut self, ui: &mut Ui) {
        let t = self.theme;
        pick(
            ui,
            &t,
            "ENTER",
            &["SWR", "Return loss (dB)", "Reflection coefficient"],
            &mut self.swr_from,
        );
        ui.add_space(t.main_gap / 2.0);
        let label = ["SWR (1 or more)", "Return loss (dB)", "|Gamma| (0 to 1)"][self.swr_from];
        field(ui, &t, label, &mut self.swr_in, 150.0);
        let r = num(&self.swr_in).and_then(|v| {
            match self.swr_from {
                0 => Reflection::from_swr(v),
                1 => Reflection::from_return_loss(v),
                _ => Reflection::from_gamma(v),
            }
            .map_err(|e| e.to_owned())
        });
        match r {
            Ok(r) => {
                ui.add_space(t.main_gap / 2.0);
                kv(
                    ui,
                    &t,
                    "swr_kv",
                    &[
                        ("SWR", format!("{} : 1", gs(r.swr, 4))),
                        ("RETURN LOSS", format!("{} dB", gs(r.return_loss_db, 4))),
                        ("REFLECTION |GAMMA|", gs(r.gamma, 4)),
                        ("MISMATCH LOSS", format!("{} dB", gs(r.mismatch_loss_db, 3))),
                        ("POWER REFLECTED", format!("{}%", gs(r.reflected_pct, 3))),
                        (
                            "POWER DELIVERED",
                            format!("{}%", gs(100.0 - r.reflected_pct, 4)),
                        ),
                    ],
                );
            }
            Err(e) => error(ui, &t, &e),
        }
        note(
            ui,
            &t,
            "Mismatch loss is at the load. Add cable loss separately: see COAX.",
        );
    }

    fn tool_power(&mut self, ui: &mut Ui) {
        let t = self.theme;
        let labels: Vec<&str> = PowerUnit::ALL.iter().map(|u| u.label()).collect();
        ui.label(
            egui::RichText::new("INPUT UNIT")
                .size(t.data_size)
                .color(palette::TEXT_MUTED),
        );
        radio_row(ui, &t, &labels, &mut self.pow_unit);
        ui.horizontal(|ui| {
            field(ui, &t, "Value", &mut self.pow_in, 130.0);
            field(ui, &t, "Load (ohm, for volts)", &mut self.pow_r, 90.0);
        });
        match (num(&self.pow_in), pos(&self.pow_r)) {
            (Ok(v), Ok(r)) => {
                let u = PowerUnit::ALL[self.pow_unit];
                let w = u.to_watts(v, r);
                if w.is_finite() && w > 0.0 {
                    let mut rows: Vec<(&str, String)> = PowerUnit::ALL
                        .iter()
                        .map(|x| {
                            let name = match x {
                                PowerUnit::W => "WATTS",
                                PowerUnit::MW => "MILLIWATTS",
                                PowerUnit::Dbm => "dBm",
                                PowerUnit::Dbw => "dBW",
                                PowerUnit::Vrms => "VOLTS RMS",
                            };
                            (name, format!("{} {}", g(x.watts_to(w, r)), x.label()))
                        })
                        .collect();
                    rows.push((
                        "VOLTS PEAK-PEAK",
                        format!("{} V", g(calc::vpp(PowerUnit::Vrms.watts_to(w, r)))),
                    ));
                    let dbm = PowerUnit::Dbm.watts_to(w, r);
                    let vhf = self.f_mhz().map(|f| f >= 30.0).unwrap_or(false);
                    rows.push((
                        if vhf { "S-METER (VHF)" } else { "S-METER (HF)" },
                        calc::s_meter(dbm, vhf),
                    ));
                    ui.add_space(t.main_gap / 2.0);
                    kv(ui, &t, "pow_kv", &rows);
                } else {
                    error(ui, &t, "power must be greater than zero");
                }
            }
            (Err(e), _) | (_, Err(e)) => error(ui, &t, &e),
        }
        ui.add_space(t.main_gap);
        heading(ui, &t, "RATIOS");
        ui.horizontal(|ui| {
            field(ui, &t, "Gain (dB)", &mut self.ratio_db, 100.0);
            field(ui, &t, "Power ratio (x)", &mut self.ratio_x, 100.0);
        });
        let mut rows = vec![];
        if let Ok(db) = num(&self.ratio_db) {
            rows.push((
                "dB TO RATIO",
                format!(
                    "{}x power, {}x voltage",
                    g(calc::db_to_power_ratio(db)),
                    g(calc::db_to_voltage_ratio(db))
                ),
            ));
        }
        if let Ok(x) = pos(&self.ratio_x) {
            rows.push((
                "RATIO TO dB",
                format!("{} dB", g(calc::power_ratio_to_db(x))),
            ));
        }
        kv(ui, &t, "ratio_kv", &rows);
        note(
            ui,
            &t,
            "S-meter: S9 = -73 dBm (HF) or -93 dBm (VHF/UHF), 6 dB per S-unit. Real meters vary.",
        );
    }

    fn tool_ohm(&mut self, ui: &mut Ui) {
        let t = self.theme;
        note(
            ui,
            &t,
            "ENTER ANY TWO. THE TWO MOST RECENTLY EDITED FIELDS ARE THE INPUTS.",
        );
        for q in Q::ALL {
            let i = q.index();
            let r = field(ui, &t, q.label(), &mut self.ohm[i], 160.0);
            if r.changed() {
                self.ohm_order.retain(|x| *x != i);
                self.ohm_order.push(i);
            }
        }
        let known: Vec<(Q, f64)> = self
            .ohm_order
            .iter()
            .rev()
            .filter_map(|i| pos(&self.ohm[*i]).ok().map(|v| (Q::ALL[*i], v)))
            .take(2)
            .collect();
        ui.add_space(t.main_gap / 2.0);
        if known.len() < 2 {
            return note(ui, &t, "Waiting for two valid positive values.");
        }
        match calc::ohms_law(known[0], known[1]) {
            Ok(r) => {
                let rows: Vec<(&str, String)> = Q::ALL
                    .iter()
                    .map(|q| {
                        let tag = if known.iter().any(|k| k.0 == *q) {
                            "INPUT"
                        } else {
                            "RESULT"
                        };
                        let unit = ["V", "A", "ohm", "W"][q.index()];
                        (tag, format!("{} {unit}", g(r[q.index()])))
                    })
                    .collect();
                kv(ui, &t, "ohm_kv", &rows);
            }
            Err(e) => error(ui, &t, e),
        }
    }

    fn tool_lc(&mut self, ui: &mut Ui) {
        let t = self.theme;
        pick(
            ui,
            &t,
            "SOLVE FOR",
            &["Frequency", "Inductance", "Capacitance"],
            &mut self.lc_solve,
        );
        ui.add_space(t.main_gap / 2.0);
        let solve = [LcSolve::Freq, LcSolve::L, LcSolve::C][self.lc_solve];
        let names = ["Frequency (MHz)", "Inductance (uH)", "Capacitance (pF)"];
        for (i, name) in names.iter().enumerate() {
            if i != self.lc_solve {
                field(ui, &t, name, &mut self.lc[i], 150.0);
            }
        }
        let vals: Vec<f64> = self.lc.iter().map(|s| num(s).unwrap_or(f64::NAN)).collect();
        ui.add_space(t.main_gap / 2.0);
        match calc::lc_resonance(solve, vals[0], vals[1], vals[2]) {
            Ok(r) => {
                let (name, unit) = [
                    ("FREQUENCY", "MHz"),
                    ("INDUCTANCE", "uH"),
                    ("CAPACITANCE", "pF"),
                ][self.lc_solve];
                readout(ui, &t, &format!("{name} {} {unit}", g(r)), true);
                let (f, l, c) = match solve {
                    LcSolve::Freq => (r, vals[1], vals[2]),
                    LcSolve::L => (vals[0], r, vals[2]),
                    LcSolve::C => (vals[0], vals[1], r),
                };
                kv(
                    ui,
                    &t,
                    "lc_kv",
                    &[
                        ("RESONANCE", format!("{} MHz", g(f))),
                        (
                            "REACTANCE AT F0",
                            format!("{} ohm", g(calc::lc_reactance_ohm(l, c))),
                        ),
                        ("WAVELENGTH", format!("{} m", g(calc::wavelength_m(f)))),
                    ],
                );
                if solve == LcSolve::Freq
                    && action_button(ui, &t, "Use as frequency", vec2(200.0, 36.0)).clicked()
                {
                    self.set_f_mhz(r);
                    self.status = format!("FREQUENCY SET TO {} MHZ", g(r));
                }
            }
            Err(_) => error(ui, &t, "enter two positive numbers"),
        }
        note(
            ui,
            &t,
            "Ideal parts. Real inductors have loss and self-resonance; capacitors have tolerance.",
        );
    }

    fn tool_match(&mut self, ui: &mut Ui) {
        let t = self.theme;
        pick(
            ui,
            &t,
            "FORM",
            &[
                "Low-pass (series L, shunt C)",
                "High-pass (series C, shunt L)",
            ],
            &mut self.match_form,
        );
        ui.add_space(t.main_gap / 2.0);
        ui.horizontal(|ui| {
            field(ui, &t, "Source (ohm)", &mut self.rs, 110.0);
            field(ui, &t, "Load (ohm)", &mut self.rl, 110.0);
        });
        let f = match self.f_mhz() {
            Ok(f) => f,
            Err(e) => return error(ui, &t, &e),
        };
        let net: Result<LNetwork, String> = pos(&self.rs).and_then(|s| {
            pos(&self.rl).and_then(|l| calc::l_network(s, l, f).map_err(|e| e.to_owned()))
        });
        ui.add_space(t.main_gap / 2.0);
        match net {
            Ok(n) => {
                let side = if n.shunt_at_source { "SOURCE" } else { "LOAD" };
                let (series, shunt) = if self.match_form == 0 {
                    (
                        format!("L {} nH", g(n.lp_series_l_nh)),
                        format!("C {} pF", g(n.lp_shunt_c_pf)),
                    )
                } else {
                    (
                        format!("C {} pF", g(n.hp_series_c_pf)),
                        format!("L {} nH", g(n.hp_shunt_l_nh)),
                    )
                };
                kv(
                    ui,
                    &t,
                    "match_kv",
                    &[
                        ("FREQUENCY", format!("{} MHz", g(f))),
                        ("Q", g(n.q)),
                        ("SERIES REACTANCE", format!("{} ohm", g(n.x_series))),
                        ("SHUNT REACTANCE", format!("{} ohm", g(n.x_shunt))),
                        ("SERIES PART", series),
                        ("SHUNT PART", shunt),
                        ("SHUNT GOES ACROSS", format!("THE {side} (HIGHER R)")),
                    ],
                );
            }
            Err(e) => error(ui, &t, &e),
        }
        note(
            ui,
            &t,
            "Resistive loads only. Uses the shared frequency. Real parts add loss; a tuner or a transformer may fit better.",
        );
    }

    fn tool_path(&mut self, ui: &mut Ui) {
        let t = self.theme;
        pick(
            ui,
            &t,
            "DISTANCE UNIT",
            &["km", "mi", "nmi"],
            &mut self.dist_unit,
        );
        ui.add_space(t.main_gap / 2.0);
        ui.horizontal(|ui| {
            field(ui, &t, "Distance", &mut self.dist, 100.0);
            field(ui, &t, "TX power (W)", &mut self.tx_w, 100.0);
        });
        ui.horizontal(|ui| {
            field(ui, &t, "TX gain (dBi)", &mut self.gain_tx, 90.0);
            field(ui, &t, "RX gain (dBi)", &mut self.gain_rx, 90.0);
            field(ui, &t, "Line loss (dB)", &mut self.line_loss, 90.0);
        });
        let f = match self.f_mhz() {
            Ok(f) => f,
            Err(e) => return error(ui, &t, &e),
        };
        let r = (|| {
            Ok::<_, String>((
                pos(&self.dist)?,
                pos(&self.tx_w)?,
                num(&self.gain_tx)?,
                num(&self.gain_rx)?,
                num(&self.line_loss)?,
            ))
        })();
        ui.add_space(t.main_gap / 2.0);
        match r {
            Ok((d, w, gt, gr, ll)) => {
                let km = DistUnit::ALL[self.dist_unit].to_km(d);
                let path = calc::fspl_db(km, f);
                let rx = calc::link_rx_dbm(w, gt, gr, ll, path);
                let vhf = f >= 30.0;
                kv(
                    ui,
                    &t,
                    "path_kv",
                    &[
                        (
                            "PATH LOSS",
                            format!(
                                "{} dB over {} {} ({} km)",
                                g(path),
                                g(d),
                                DistUnit::ALL[self.dist_unit].label(),
                                g(km)
                            ),
                        ),
                        ("RECEIVED", format!("{} dBm", g(rx))),
                        (
                            "RECEIVED",
                            format!(
                                "{} uV across 50 ohm",
                                g(PowerUnit::Vrms
                                    .watts_to(PowerUnit::Dbm.to_watts(rx, 50.0), 50.0)
                                    * 1e6)
                            ),
                        ),
                        ("S-METER", calc::s_meter(rx, vhf)),
                    ],
                );
            }
            Err(e) => error(ui, &t, &e),
        }
        note(
            ui,
            &t,
            "Free space only: no terrain, ground reflection, ducting or atmosphere. Real paths are usually worse.",
        );
    }

    fn tool_bands(&mut self, ui: &mut Ui) {
        let t = self.theme;
        note(
            ui,
            &t,
            "INFORMATIONAL ONLY. US (FCC Part 97) band edges. Privileges depend on your licence class and change over time: check the current rules before transmitting. Technicians: all modes above 50 MHz, CW on 80/40/15 m, CW and SSB on 28.0-28.5 MHz.",
        );
        match self.f_mhz() {
            Ok(f) => {
                if let Some(b) = bands::find_band(f) {
                    readout(
                        ui,
                        &t,
                        &format!(
                            "{} MHz IS IN THE {} BAND ({} to {} MHz)",
                            g(f),
                            b.name,
                            g(b.lo_mhz),
                            g(b.hi_mhz)
                        ),
                        true,
                    );
                    note(ui, &t, b.note);
                    if let Some(s) = bands::segment_at(b, f) {
                        readout(ui, &t, &format!("SEGMENT: {s}"), false);
                    }
                } else {
                    readout(
                        ui,
                        &t,
                        &format!("{} MHz IS NOT IN A US AMATEUR BAND", g(f)),
                        false,
                    );
                    let (lo, hi) = bands::neighbours(f);
                    if let Some(b) = lo {
                        note(
                            ui,
                            &t,
                            &format!("Band below: {} (up to {} MHz)", b.name, g(b.hi_mhz)),
                        );
                    }
                    if let Some(b) = hi {
                        note(
                            ui,
                            &t,
                            &format!("Band above: {} (from {} MHz)", b.name, g(b.lo_mhz)),
                        );
                    }
                }
            }
            Err(e) => error(ui, &t, &e),
        }
        ui.add_space(t.main_gap / 2.0);
        let rows_txt: Vec<String> = bands::BANDS
            .iter()
            .map(|b| {
                format!(
                    "{:<8}{:>8} to {:<8} MHz  {}",
                    b.name,
                    edge(b.lo_mhz),
                    edge(b.hi_mhz),
                    b.note
                )
            })
            .collect();
        let before = self.band_modes.clone();
        let mut rows: Vec<(String, Mode)> = rows_txt
            .iter()
            .cloned()
            .zip(self.band_modes.iter().copied())
            .collect();
        mode_list(ui, &t, "bands", &mut rows, super::LIST_ROW);
        for (i, (_, m)) in rows.iter().enumerate() {
            if *m == before[i] {
                continue;
            }
            self.band_modes.iter_mut().for_each(|x| *x = Mode::Inactive);
            self.band_modes[i] = *m;
            let b = &bands::BANDS[i];
            let (mhz, what) = match m {
                Mode::Primary => (Some(b.center_mhz()), "CENTRE"),
                Mode::Alt1 => (Some(b.lo_mhz), "LOWER EDGE"),
                Mode::Alt2 => (Some(b.hi_mhz), "UPPER EDGE"),
                Mode::Inactive => (None, ""),
            };
            if let Some(mhz) = mhz {
                self.set_f_mhz(mhz);
                self.status = format!("FREQUENCY SET TO {} {what}", b.name);
            }
        }
        note(
            ui,
            &t,
            "CLICK SETS THE BAND CENTRE · DOUBLE THE LOWER EDGE · TRIPLE THE UPPER EDGE",
        );
    }
}


#[cfg(test)]
mod logic {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(g(14.2), "14.200");
        assert_eq!(g(1234.5678), "1234.6");
        assert_eq!(g(0.001234), "0.0012340");
        assert_eq!(g(f64::INFINITY), "INF");
        assert_eq!(gs(2.3527, 3), "2.35");
        assert_eq!(gs(0.64783, 3), "0.648");
        assert_eq!(edge(0.1357), "0.1357");
        assert_eq!(edge(14.0), "14");
        assert_eq!(edge(14.35), "14.35");
        assert_eq!(num(" 1,234.5 ").unwrap(), 1234.5);
        assert!(num("abc").is_err());
        assert!(num("").is_err());
        assert!(pos("0").is_err());
        assert!(pos("-3").is_err());
        assert!(num("NaN").is_err());
    }
}
