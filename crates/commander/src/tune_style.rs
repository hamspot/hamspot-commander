//! NanoVNA look for the Tune page below the tuning widget: captions with a
//! rule, solid key caps (radius 4), ●/○ toggles, and a dark chart pane with a
//! header line and one status strip. Reuses the NanoVNA helpers so the two
//! pages stay one style. Layout and styling only; callers keep behaviour.

use crate::tools::nanovna::{key_cap, key_caption, GRID, GRID_FAINT, KEY_H, PANE_BG, TRACE1};
use egui::{vec2, Color32, Pos2, Rect, Sense, Stroke};
use rustcars::{palette, LcarsTheme};

/// Header line above and status strip below the spectrum pane.
pub const STRIP_H: f32 = 18.0;
/// The pane never grows past this, so a tall detached window stays sane.
pub const SPECTRUM_MAX_H: f32 = 420.0;

fn theme() -> LcarsTheme {
    LcarsTheme::default()
}

fn key_font(t: &LcarsTheme) -> egui::FontId {
    egui::FontId::proportional((t.header_size * 0.85).min(17.0))
}

/// Width that fits a label with padding plus room for the ●/○ marker.
pub fn key_width(ui: &mut egui::Ui, label: &str) -> f32 {
    let t = theme();
    let w = ui.fonts_mut(|f| f.layout_no_wrap(format!("● {}", label.to_uppercase()), key_font(&t), Color32::WHITE).size().x);
    (w + 22.0).max(56.0)
}

/// Section caption with a thin rule, as NanoVNA's "SWEEP · CAL".
pub fn caption(ui: &mut egui::Ui, text: &str) {
    let w = ui.available_width().max(60.0);
    key_caption(ui, &theme(), text, w);
}

/// Momentary solid key cap (NanoVNA key_cap, idle slate).
pub fn action(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    let w = key_width(ui, label);
    key_cap(ui, &theme(), label, false, w, enabled)
}

/// Latching key: hue fill, thin border and the ●/○ marker (never colour alone).
/// Painted here (not in a child Ui) so it wraps with its row.
pub fn toggle(ui: &mut egui::Ui, label: &str, enabled: bool, on: bool, hue: Color32) -> egui::Response {
    let w = key_width(ui, label);
    let t = theme();
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, KEY_H), sense);
    let (fill, border, ink) = rustcars::toggle_colors(hue, on);
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    let fill = if enabled && resp.hovered() { rustcars::tint(fill, 0.08) } else { fill };
    let p = ui.painter();
    p.rect_filled(rect, 4.0, fade(fill));
    p.rect_stroke(rect, 4.0, Stroke::new(rustcars::TOGGLE_BORDER, fade(border)), egui::StrokeKind::Inside);
    rustcars::paint_toggle_marker_label(p, rect, label, on, key_font(&t), fade(ink));
    if resp.has_focus() {
        p.rect_stroke(rect.expand(2.0), 5.0, Stroke::new(1.0, t.text), egui::StrokeKind::Outside);
    }
    let name = rustcars::toggle_label(label, on);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, on, &name));
    resp
}

/// Inset field frame (dark glass, thin grid border, radius 4) around a control.
pub fn inset<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(PANE_BG)
        .stroke(Stroke::new(1.0, GRID))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_min_height(KEY_H - 8.0);
            add(ui)
        })
        .inner
}

/// Height for the spectrum pane: fill what is left, bounded, never NaN.
pub fn spectrum_height(ui: &egui::Ui, reserve: f32) -> f32 {
    let room = ui.available_height().min(ui.clip_rect().bottom() - ui.cursor().top());
    let h = room - reserve - 2.0 * STRIP_H;
    if h.is_finite() { h.clamp(crate::dial::SPECTRUM_H, SPECTRUM_MAX_H) } else { crate::dial::SPECTRUM_H }
}

/// Below this page width the scope stacks under the keys instead of beside them.
pub const TWO_COL_MIN_W: f32 = 720.0;
/// Smallest scope pane (inside the header and status strips).
pub const SCOPE_MIN_W: f32 = 320.0;
pub const SCOPE_MIN_H: f32 = 160.0;
const GAP: f32 = 8.0;

/// The page below the tuning widget, NanoVNA style: keys on the left half,
/// scope on the right half at full height (David, 2026-10-30). Narrow pages
/// stack the scope under a scrolling key column; the scope keeps its minimum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Split {
    pub keys: Rect,
    /// Whole scope block: header strip, pane, status strip.
    pub scope: Rect,
    pub two_col: bool,
}

impl Split {
    pub fn pane_h(&self) -> f32 {
        (self.scope.height() - 2.0 * STRIP_H).max(0.0)
    }
}

pub fn split(area: Rect) -> Split {
    let w = area.width().max(0.0);
    let h = area.height().max(0.0);
    if w >= TWO_COL_MIN_W {
        let keys_w = ((w - GAP) * 0.5).floor();
        let keys = Rect::from_min_size(area.min, vec2(keys_w, h));
        let scope = Rect::from_min_max(Pos2::new(keys.right() + GAP, area.top()), area.max);
        Split { keys, scope, two_col: true }
    } else {
        // Scope first: it keeps its minimum, up to half, and the keys scroll.
        let want = (h * 0.5).max(SCOPE_MIN_H + 2.0 * STRIP_H).min(SPECTRUM_MAX_H + 2.0 * STRIP_H);
        let scope_h = want.min(h);
        let keys = Rect::from_min_size(area.min, vec2(w, (h - scope_h - GAP).max(0.0)));
        let scope = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - scope_h), area.max);
        Split { keys, scope, two_col: false }
    }
}

/// Bandspread share at the bottom of the Tune page when BSP is on.
pub const BSP_MIN_H: f32 = 80.0;
pub const BSP_MAX_H: f32 = 200.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TunePanes {
    /// Keys (and scope) area.
    pub top: Rect,
    /// Bandspread pane, when shown.
    pub bsp: Option<Rect>,
}

/// Split the area under the tuning widget into the keys/scope area and the
/// bandspread pane; they never overlap.
pub fn tune_panes(area: Rect, bsp: bool) -> TunePanes {
    if !bsp {
        return TunePanes { top: area, bsp: None };
    }
    let h = (area.height() * 0.35).clamp(BSP_MIN_H, BSP_MAX_H).min(area.height());
    let bsp_r = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - h), area.max);
    let top = Rect::from_min_max(area.min, Pos2::new(area.right(), (bsp_r.top() - GAP).max(area.top())));
    TunePanes { top, bsp: Some(bsp_r) }
}

/// Keys alone in `area` (no HackRF scope), scrolling and clipped.
pub fn keys_only(ui: &mut egui::Ui, area: Rect, keys: impl FnOnce(&mut egui::Ui)) {
    ui.scope_builder(egui::UiBuilder::new().max_rect(area).id_salt("tune-keys"), |ui| {
        ui.set_clip_rect(area.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical()
            .id_salt("tune-keys-scroll")
            .max_height(area.height())
            .min_scrolled_height(area.height())
            .auto_shrink([false, true])
            .show(ui, keys);
    });
    ui.advance_cursor_after_rect(area);
}

/// Lay out keys (scrolling if they do not fit) and the scope in `area`.
pub fn keys_and_scope(
    ui: &mut egui::Ui,
    area: Rect,
    keys: impl FnOnce(&mut egui::Ui),
    scope: impl FnOnce(&mut egui::Ui, f32),
) -> Split {
    let sp = split(area);
    ui.scope_builder(egui::UiBuilder::new().max_rect(sp.keys).id_salt("tune-keys"), |ui| {
        ui.set_clip_rect(sp.keys.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical()
            .id_salt("tune-keys-scroll")
            .max_height(sp.keys.height())
            .min_scrolled_height(sp.keys.height())
            .auto_shrink([false, true])
            .show(ui, keys);
    });
    ui.scope_builder(egui::UiBuilder::new().max_rect(sp.scope).id_salt("tune-scope"), |ui| {
        ui.set_clip_rect(sp.scope.intersect(ui.clip_rect()));
        scope(ui, sp.pane_h());
    });
    ui.advance_cursor_after_rect(area);
    sp
}

/// Fixed-height status line (never appears or disappears, so nothing
/// below it jumps). `warn` text is drawn in the caution colour and prefixed
/// "!" so it does not rely on colour alone.
pub fn status_line(ui: &mut egui::Ui, text: &str, warn: bool) -> Rect {
    let shown = if warn && !text.is_empty() { format!("! {text}") } else { text.to_string() };
    let ink = if warn { Color32::from_rgb(0xFF, 0xB0, 0x60) } else { palette::TEXT };
    let r = strip(ui, &shown, "", ink);
    r
}

fn strip(ui: &mut egui::Ui, left: &str, right: &str, ink: Color32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), STRIP_H), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, PANE_BG);
    let font = egui::FontId::monospace(12.0);
    p.text(Pos2::new(rect.left() + 6.0, rect.center().y), egui::Align2::LEFT_CENTER, left, font.clone(), ink);
    p.text(Pos2::new(rect.right() - 6.0, rect.center().y), egui::Align2::RIGHT_CENTER, right, font, palette::TEXT_MUTED);
    rect
}

pub struct Chart<'a> {
    pub header: &'a str,
    pub header_right: &'a str,
    pub center_hz: f64,
    pub rate_hz: f64,
    pub usable_hz: f64,
    pub db: &'a [f32],
    pub needle_hz: f64,
    pub height: f32,
    pub status: &'a str,
}

/// NanoVNA chart: header line, dark pane with labelled graticule, status strip.
/// A click or drag in the pane returns a frequency.
pub fn spectrum_chart(ui: &mut egui::Ui, c: Chart) -> (Option<f64>, Rect) {
    let item = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    strip(ui, c.header, c.header_right, palette::TEXT);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), c.height), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, PANE_BG);
    let font = egui::FontId::monospace(10.0);
    let band = Rect::from_min_max(Pos2::new(rect.left() + 40.0, rect.top() + 6.0), Pos2::new(rect.right() - 8.0, rect.bottom() - 16.0));
    let rate = if c.rate_hz.is_finite() && c.rate_hz > 0.0 { c.rate_hz } else { 1.0 };
    let center = if c.center_hz.is_finite() { c.center_hz } else { 0.0 };
    let (lo, hi) = (center - rate / 2.0, center + rate / 2.0);
    let top = c.db.iter().copied().filter(|v| v.is_finite()).fold(f32::MIN, f32::max);
    let (floor, span) = (if top > f32::MIN / 2.0 { top - 70.0 } else { -70.0 }, 70.0f32);
    for i in 0..=10 {
        let x = band.left() + band.width() * i as f32 / 10.0;
        p.line_segment([Pos2::new(x, band.top()), Pos2::new(x, band.bottom())], Stroke::new(1.0, if i % 5 == 0 { GRID } else { GRID_FAINT }));
        if i % 5 == 0 {
            let hz = lo + rate * i as f64 / 10.0;
            let align = match i { 0 => egui::Align2::LEFT_TOP, 10 => egui::Align2::RIGHT_TOP, _ => egui::Align2::CENTER_TOP };
            p.text(Pos2::new(x, band.bottom() + 2.0), align, format!("{:.3}", hz / 1e6), font.clone(), palette::TEXT_MUTED);
        }
    }
    for i in 0..=7 {
        let y = band.bottom() - band.height() * i as f32 / 7.0;
        p.line_segment([Pos2::new(band.left(), y), Pos2::new(band.right(), y)], Stroke::new(1.0, if i == 0 || i == 7 { GRID } else { GRID_FAINT }));
        if i % 2 == 1 || i == 7 {
            p.text(Pos2::new(band.left() - 4.0, y), egui::Align2::RIGHT_CENTER, format!("{:.0}", floor + span * i as f32 / 7.0), font.clone(), palette::TEXT_MUTED);
        }
    }
    if c.usable_hz.is_finite() && c.usable_hz > 0.0 && c.usable_hz < rate {
        let inset = ((rate - c.usable_hz) / 2.0 / rate).clamp(0.0, 0.5) as f32 * band.width();
        let shade = Color32::from_rgba_unmultiplied(40, 46, 56, 120);
        p.rect_filled(Rect::from_min_max(band.left_top(), Pos2::new(band.left() + inset, band.bottom())), 0.0, shade);
        p.rect_filled(Rect::from_min_max(Pos2::new(band.right() - inset, band.top()), band.right_bottom()), 0.0, shade);
    }
    if c.db.len() >= 2 {
        let pts: Vec<Pos2> = c.db.iter().enumerate().map(|(i, v)| {
            let s = if v.is_finite() { *v } else { floor };
            let t = ((s - floor) / span).clamp(0.0, 1.0);
            Pos2::new(band.left() + (i as f32 + 0.5) / c.db.len() as f32 * band.width(), band.bottom() - t * band.height())
        }).collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.5, TRACE1)));
    }
    if c.needle_hz.is_finite() && c.needle_hz >= lo && c.needle_hz <= hi {
        let x = band.left() + ((c.needle_hz - lo) / rate) as f32 * band.width();
        p.line_segment([Pos2::new(x, band.top()), Pos2::new(x, band.bottom())], Stroke::new(1.5, palette::TEXT));
        p.text(Pos2::new(x + 3.0, band.top()), egui::Align2::LEFT_TOP, "▼ VFO", font.clone(), palette::TEXT);
    }
    p.rect_stroke(band, 0.0, Stroke::new(1.0, GRID), egui::StrokeKind::Inside);
    let resp = if let Some(pos) = response.hover_pos().filter(|p| band.contains(*p)) {
        let f = lo + ((pos.x - band.left()) / band.width()).clamp(0.0, 1.0) as f64 * rate;
        response.on_hover_text(format!("{:.6} MHz", f / 1e6))
    } else {
        response
    };
    let hz = if resp.clicked() || resp.dragged() {
        resp.interact_pointer_pos().filter(|p| band.contains(*p)).map(|pos| lo + ((pos.x - band.left()) / band.width()).clamp(0.0, 1.0) as f64 * rate)
    } else {
        None
    };
    strip(ui, c.status, "dB / MHz", palette::TEXT);
    ui.spacing_mut().item_spacing.y = item;
    (hz, rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key on the Tune page below the tuning widget, in page order.
    const KEYS: [&str; 33] = [
        "ALT", "BSP", "LOAD", "ATU", "Band -", "Band +", "Dimmer", "NR", "Pre 1", "Pre 2", "Ant 1", "Ant 2",
        "Bk-in", "Fast", "NR sub", "CW Spot", "Keyer", "Nar", "Proc", "Ant Rx", "Chan +", "Chan -", "Lock",
        "Notch", "QMR", "BC", "TX blocked: E-STOP", "PWR", "QMS", "Contour", "Moni", "Scan", "Scan Dn",
    ];

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        rustcars::install_lcars_font(&ctx);
        rustcars::apply_theme(&ctx, &LcarsTheme::default());
        ctx
    }

    fn run(ctx: &egui::Context, w: f32, h: f32, f: &mut dyn FnMut(&mut egui::Ui)) -> egui::FullOutput {
        let mut out = egui::FullOutput::default();
        for _ in 0..3 {
            out = ctx.run_ui(
                egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w, h))), ..Default::default() },
                |ui| f(ui),
            );
            out.textures_delta.clear();
        }
        out
    }

    fn dark(c: Color32) -> bool {
        c.a() > 0 && (c.r() as u32 + c.g() as u32 + c.b() as u32) < 60
    }

    #[test]
    fn keys_fit_unclipped_and_no_black_strokes() {
        for w in [1400.0, 900.0] {
            let ctx = ctx();
            let mut rects = Vec::new();
            let out = run(&ctx, w, 900.0, &mut |ui| {
                rects.clear();
                caption(ui, "FUNCTIONS");
                ui.horizontal_wrapped(|ui| {
                    for (i, k) in KEYS.iter().enumerate() {
                        let r = match i % 3 {
                            0 => action(ui, k, i % 2 == 0),
                            1 => toggle(ui, k, true, true, palette::TEAL),
                            _ => toggle(ui, k, i % 2 == 0, false, palette::PLUM),
                        };
                        rects.push((*k, r.rect));
                    }
                });
            });
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(w, 900.0));
            assert_eq!(rects.len(), KEYS.len());
            for (k, r) in &rects {
                assert!(screen.contains_rect(*r), "{k} clipped at {w}: {r:?}");
            }
            for c in &out.shapes {
                match &c.shape {
                    egui::Shape::Rect(r) => assert!(!dark(r.stroke.color) || r.stroke.width == 0.0, "dark stroke {:?}", r.stroke),
                    egui::Shape::LineSegment { stroke, .. } => assert!(!dark(stroke.color)),
                    egui::Shape::Text(t) => {
                        let s = t.galley.text();
                        if let Some((_, r)) = rects.iter().find(|(k, _)| s == k.to_uppercase()) {
                            assert!(r.expand(1.0).contains_rect(t.visual_bounding_rect()), "label {s} spills its key at {w}");
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn toggles_carry_a_marker_not_colour_alone() {
        let ctx = ctx();
        let out = run(&ctx, 600.0, 200.0, &mut |ui| {
            toggle(ui, "NR", true, true, palette::TEAL);
            toggle(ui, "Lock", true, false, palette::TEAL);
        });
        let (mut filled, mut ring) = (0, 0);
        for c in &out.shapes {
            if let egui::Shape::Circle(k) = &c.shape {
                if k.fill.a() > 0 { filled += 1 } else if k.stroke.width > 0.0 { ring += 1 }
            }
        }
        assert!(filled >= 1 && ring >= 1, "● on / ○ off markers missing: {filled} {ring}");
    }

    #[test]
    fn spectrum_stays_in_view_and_bounded() {
        let db: Vec<f32> = (0..256).map(|i| -80.0 + (i % 17) as f32).collect();
        for (w, h) in [(1400.0, 900.0), (900.0, 700.0), (500.0, 360.0)] {
            let ctx = ctx();
            let mut got = Rect::NOTHING;
            run(&ctx, w, h, &mut |ui| {
                ui.add_space(120.0);
                let height = spectrum_height(ui, 0.0);
                let c = Chart { header: "SPEC AVG", header_right: "LNA 16 dB", center_hz: 146e6, rate_hz: 2e6, usable_hz: 1.75e6, db: &db, needle_hz: 146.1e6, height, status: "ok" };
                got = spectrum_chart(ui, c).1;
            });
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(w, h));
            assert!(got.is_finite() && got.height() >= crate::dial::SPECTRUM_H && got.height() <= SPECTRUM_MAX_H, "{got:?}");
            assert!(screen.contains_rect(got) || h < 120.0 + 2.0 * STRIP_H + crate::dial::SPECTRUM_H, "spectrum out of view at {w}x{h}: {got:?}");
        }
        // Unbounded parent (a scroll area): height falls back, never NaN.
        let ctx = ctx();
        let mut hh = 0.0;
        run(&ctx, 800.0, 600.0, &mut |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| hh = spectrum_height(ui, 0.0));
        });
        assert!(hh.is_finite() && hh >= crate::dial::SPECTRUM_H && hh <= SPECTRUM_MAX_H);
    }

    /// Window sizes from David's report and the usual ones. `chrome` is the
    /// elbow frame plus the tuning widget above the split (measured on the
    /// 1024x550 screenshot: the keys started about 260 px down).
    const SIZES: [(f32, f32); 6] = [(1024.0, 550.0), (1280.0, 720.0), (1400.0, 900.0), (800.0, 500.0), (700.0, 600.0), (640.0, 480.0)];
    /// Frame plus tuning widget with the sub VFO stacked under the main one
    /// and the status line (estimated; the real widget needs a running App).
    const CHROME: f32 = 300.0;

    fn tune_page(w: f32, h: f32) -> (Split, Vec<Rect>, Rect, Rect) {
        let ctx = ctx();
        let db: Vec<f32> = (0..1024).map(|i| -80.0 + (i % 13) as f32).collect();
        let (mut sp, mut keys, mut chart) = (None, Vec::new(), Rect::NOTHING);
        let page = Rect::from_min_max(Pos2::new(8.0, CHROME), Pos2::new(w - 8.0, h - 8.0));
        run(&ctx, w, h, &mut |ui| {
            keys.clear();
            sp = Some(keys_and_scope(
                ui,
                page,
                |ui| {
                    for cap in ["VFO · BAND", "MODE · RECEIVER", "FUNCTIONS", "SPECTRUM · GAIN"] {
                        caption(ui, cap);
                        ui.horizontal_wrapped(|ui| {
                            for k in KEYS {
                                let r = toggle(ui, k, true, false, palette::TEAL);
                                if ui.clip_rect().contains_rect(r.rect) {
                                    keys.push(r.rect);
                                }
                            }
                        });
                    }
                },
                |ui, ph| {
                    let c = Chart { header: "SPEC AVG", header_right: "LNA 40 dB  VGA 32 dB", center_hz: 146e6, rate_hz: 8e6, usable_hz: 6e6, db: &db, needle_hz: 146.1e6, height: ph, status: "ok" };
                    chart = spectrum_chart(ui, c).1;
                },
            ));
        });
        (sp.unwrap(), keys, chart, page)
    }

    #[test]
    fn scope_right_keys_left_and_usable_at_every_size() {
        for (w, h) in SIZES {
            let (sp, keys, chart, page) = tune_page(w, h);
            assert!(page.expand(0.5).contains_rect(chart), "scope pane out of the page at {w}x{h}: {chart:?}");
            // Below the window's own minimum (960x640) the page can be shorter
            // than the scope minimum; then the scope takes all of it.
            let need = SCOPE_MIN_H + 2.0 * STRIP_H;
            if page.height() >= need {
                assert!(chart.height() >= SCOPE_MIN_H - 0.5, "scope only {} tall at {w}x{h}", chart.height());
            } else {
                assert!(chart.height() >= page.height() - 2.0 * STRIP_H - 0.5, "scope not given the whole page at {w}x{h}");
            }
            if page.height() >= SCOPE_MIN_H + 2.0 * STRIP_H + 8.0 + 60.0 || sp.two_col {
                assert!(!keys.is_empty(), "no key visible at {w}x{h}");
            }
            for k in &keys {
                assert!(sp.keys.expand(0.5).contains_rect(*k), "key {k:?} outside its column {:?} at {w}x{h}", sp.keys);
            }
            if w - 16.0 >= TWO_COL_MIN_W {
                assert!(sp.two_col);
                assert!(chart.left() >= page.center().x - 1.0, "scope not on the right half at {w}x{h}");
                assert!(chart.width() >= SCOPE_MIN_W, "scope only {} wide at {w}x{h}", chart.width());
                assert!(keys.iter().all(|k| k.right() <= page.center().x + 1.0), "keys spill into the scope half at {w}x{h}");
                assert!((chart.height() - (page.height() - 2.0 * STRIP_H)).abs() < 1.0, "scope not full height at {w}x{h}");
            } else {
                assert!(!sp.two_col, "narrow page stacks at {w}x{h}");
                assert!(chart.top() >= sp.keys.bottom(), "stacked scope under the keys at {w}x{h}");
            }
        }
    }

    /// David's screenshot: 1024x550 docked showed the keys and no scope.
    #[test]
    fn regression_1024x550_shows_the_scope() {
        let (sp, _, chart, _) = tune_page(1024.0, 550.0);
        assert!(sp.two_col);
        assert!(chart.height() >= SCOPE_MIN_H && chart.bottom() <= 550.0, "{chart:?}");
    }

    /// 1024x550 with BSP on: keys and the bandspread pane never overlap and
    /// every shape is clipped to its own pane (David's overlap screenshot).
    #[test]
    fn bsp_pane_and_keys_do_not_overlap_at_1024x550() {
        for hackrf in [false, true] {
            let ctx = ctx();
            let area = Rect::from_min_max(Pos2::new(8.0, CHROME), Pos2::new(1016.0, 542.0));
            let p = tune_panes(area, true);
            let bsp = p.bsp.unwrap();
            assert!(p.top.bottom() <= bsp.top() - GAP + 0.5, "{p:?}");
            assert!(bsp.height() >= BSP_MIN_H && area.contains_rect(bsp));
            let mut key_clip = Rect::NOTHING;
            let out = run(&ctx, 1024.0, 550.0, &mut |ui| {
                let rows = |ui: &mut egui::Ui| {
                    for cap in ["VFO · BAND", "MODE · RECEIVER", "FUNCTIONS"] {
                        caption(ui, cap);
                        ui.horizontal_wrapped(|ui| {
                            for k in KEYS {
                                toggle(ui, k, true, false, palette::TEAL);
                            }
                        });
                    }
                };
                if hackrf {
                    keys_and_scope(ui, p.top, rows, |ui, h| {
                        let c = Chart { header: "SPEC", header_right: "", center_hz: 1e8, rate_hz: 8e6, usable_hz: 6e6, db: &[], needle_hz: 1e8, height: h, status: "NO CAPTURE" };
                        spectrum_chart(ui, c);
                    });
                } else {
                    keys_only(ui, p.top, rows);
                }
                ui.scope_builder(egui::UiBuilder::new().max_rect(bsp).id_salt("bsp"), |ui| {
                    ui.set_clip_rect(bsp.intersect(ui.clip_rect()));
                    for _ in 0..12 {
                        ui.label("700  lte  n700  lteu");
                    }
                });
                key_clip = p.top;
            });
            for c in &out.shapes {
                let r = c.shape.visual_bounding_rect();
                if !r.is_finite() || r.area() <= 0.0 {
                    continue;
                }
                // A shape is drawn in exactly one pane: its clip never
                // reaches into the other.
                let in_bsp = c.clip_rect.intersects(bsp.shrink(1.0));
                let in_top = c.clip_rect.intersects(key_clip.shrink(1.0));
                assert!(!(in_bsp && in_top), "shape clip {:?} spans keys and BSP (hackrf {hackrf})", c.clip_rect);
            }
        }
    }

    #[test]
    fn status_line_has_a_fixed_height_and_marks_warnings() {
        let ctx = ctx();
        let (mut a, mut b) = (Rect::NOTHING, Rect::NOTHING);
        let out = run(&ctx, 600.0, 200.0, &mut |ui| {
            a = status_line(ui, "", false);
            b = status_line(ui, "Voice is refused on a CW segment", true);
        });
        assert_eq!(a.height(), b.height());
        let texts: Vec<String> = out.shapes.iter().filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        }).collect();
        assert!(texts.iter().any(|t| t.starts_with("! Voice")), "{texts:?}");
    }
}
