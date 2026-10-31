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

/// Gain slider font: smaller than the key labels so the field fits one key row.
pub const GAIN_FONT: f32 = 11.0;
/// Track width bounds; the track shrinks (never clips) on a narrow column.
pub const GAIN_TRACK_MAX: f32 = 112.0;
pub const GAIN_TRACK_MIN: f32 = 40.0;

/// Instrument-style gain field, exactly one key row (`KEY_H`) tall: a square
/// inset with a thin border, small "LNA" caption, 2 px track with the filled
/// part in the trace colour, a slim square handle, and the value ("24 dB").
/// Drag or click sets the value; arrow keys step it while focused (as
/// `egui::Slider`). The value is printed, so state is never colour alone.
pub fn gain_slider(ui: &mut egui::Ui, label: &str, value: &mut u32, max: u32, step: u32) -> egui::Response {
    let font = egui::FontId::proportional(GAIN_FONT);
    let text = ui.visuals().text_color();
    let (cap_g, val_w) = ui.fonts_mut(|f| {
        let cap = f.layout_no_wrap(label.to_owned(), font.clone(), GRID);
        let w = f.layout_no_wrap(format!("{max} dB"), font.clone(), text).size().x;
        (cap, w)
    });
    let pad = 6.0;
    let fixed = pad + cap_g.size().x + 6.0 + 6.0 + val_w + pad;
    let room = ui.max_rect().width().min(ui.clip_rect().right() - ui.max_rect().left());
    let track_w = (room - fixed).clamp(GAIN_TRACK_MIN, GAIN_TRACK_MAX);
    let (rect, mut resp) = ui.allocate_exact_size(vec2((fixed + track_w).ceil(), KEY_H), Sense::click_and_drag());
    let track = Rect::from_min_size(
        Pos2::new(rect.left() + pad + cap_g.size().x + 6.0, rect.center().y - 1.0),
        vec2(track_w, 2.0),
    );
    let step = step.max(1);
    let snap = |v: f32| -> u32 { ((v / step as f32).round() as u32 * step).min(max) };
    let old = *value;
    if resp.clicked() || resp.drag_started() {
        resp.request_focus();
    }
    if resp.is_pointer_button_down_on() || resp.dragged() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let t = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0);
            *value = snap(t * max as f32);
        }
    }
    if resp.has_focus() {
        let (up, down) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowDown),
            )
        });
        if up {
            *value = (*value + step).min(max);
        }
        if down {
            *value = value.saturating_sub(step);
        }
    }
    if *value != old {
        resp.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, 0.0, PANE_BG);
        let border = if resp.has_focus() || resp.hovered() { TRACE1 } else { GRID };
        p.rect_stroke(rect.shrink(0.5), 0.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);
        p.galley(Pos2::new(rect.left() + pad, rect.center().y - cap_g.size().y / 2.0), cap_g, GRID);
        p.rect_filled(track, 0.0, GRID_FAINT);
        let t = if max == 0 { 0.0 } else { *value as f32 / max as f32 };
        let x = track.left() + t * track.width();
        p.rect_filled(Rect::from_min_max(track.min, Pos2::new(x, track.max.y)), 0.0, TRACE1);
        let handle = Rect::from_center_size(Pos2::new(x, rect.center().y), vec2(4.0, 12.0));
        p.rect_filled(handle, 0.0, text);
        p.text(
            Pos2::new(rect.right() - pad, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            format!("{} dB", *value),
            font,
            text,
        );
    }
    let name = format!("{label} {} dB", *value);
    resp.widget_info(|| egui::WidgetInfo::slider(true, *value as f64, &name));
    resp
}

/// VFD-style VFO digits (David, 2026-10-31): lit segment core, two
/// translucent glow strokes behind it, and a very faint ghost for unlit ones.
pub const VFD_LIT: Color32 = Color32::from_rgb(0x5C, 0xD6, 0xFF);
pub const VFD_GLOW_OUTER: Color32 = Color32::from_rgba_premultiplied(0x0C, 0x24, 0x30, 0x30);
pub const VFD_GLOW_INNER: Color32 = Color32::from_rgba_premultiplied(0x1C, 0x50, 0x68, 0x68);
pub const VFD_GHOST: Color32 = Color32::from_rgba_premultiplied(0x08, 0x12, 0x18, 0x40);
/// Underline under the selected (step) digit, so it is not colour alone.
pub const VFD_CURSOR: Color32 = palette::TEXT;

/// Segment strokes (a..g) for one digit cell inside `rect`, inset so the
/// widest glow stroke stays inside the rect. Returns (segments, stroke width).
pub fn vfd_segments(rect: Rect) -> ([[Pos2; 2]; 7], f32) {
    let h = rect.height() * 0.74;
    let w = (rect.width() * 0.62).min(h * 0.56);
    let th = (h * 0.10).max(1.5);
    let x0 = rect.center().x - w / 2.0;
    let top = rect.center().y - h / 2.0;
    let mid = top + h / 2.0;
    let bot = top + h;
    let p = |x: f32, y: f32| Pos2::new(x, y);
    let mut segs = [
        [p(x0, top), p(x0 + w, top)],
        [p(x0 + w, top), p(x0 + w, mid)],
        [p(x0 + w, mid), p(x0 + w, bot)],
        [p(x0, bot), p(x0 + w, bot)],
        [p(x0, mid), p(x0, bot)],
        [p(x0, top), p(x0, mid)],
        [p(x0, mid), p(x0 + w, mid)],
    ];
    for s in &mut segs {
        let d = (s[1] - s[0]).normalized() * th * 0.7;
        *s = [s[0] + d, s[1] - d];
    }
    (segs, th)
}

/// Paint one VFO character as VFD seven-segment (digits) or an LED dot ('.').
/// Uses the Rotator LED segment table. Cheap: 3 strokes per lit segment.
pub fn vfd_char(p: &egui::Painter, rect: Rect, ch: char) {
    if ch == '.' {
        let r = (rect.height() * 0.06).max(1.5);
        let c = Pos2::new(rect.center().x, rect.center().y + rect.height() * 0.37 - r);
        p.circle_filled(c, r * 2.0, VFD_GLOW_OUTER);
        p.circle_filled(c, r * 1.5, VFD_GLOW_INNER);
        p.circle_filled(c, r, VFD_LIT);
        return;
    }
    let (segs, th) = vfd_segments(rect);
    let bits = crate::tools::rotator::led::segments(ch);
    for (n, s) in segs.iter().enumerate() {
        if bits & (1 << n) == 0 {
            p.line_segment(*s, Stroke::new(th, VFD_GHOST));
        }
    }
    for (n, s) in segs.iter().enumerate() {
        if bits & (1 << n) != 0 {
            p.line_segment(*s, Stroke::new(th * 2.2, VFD_GLOW_OUTER));
            p.line_segment(*s, Stroke::new(th * 1.5, VFD_GLOW_INNER));
            p.line_segment(*s, Stroke::new(th, VFD_LIT));
        }
    }
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

/// One key of the VFO grid: momentary (`on` None) or latching.
#[derive(Clone, Copy, Debug)]
pub struct GridKey {
    pub label: &'static str,
    pub enabled: bool,
    pub on: Option<bool>,
}

impl GridKey {
    pub const fn action(label: &'static str, enabled: bool) -> Self {
        Self { label, enabled, on: None }
    }
    pub const fn toggle(label: &'static str, enabled: bool, on: bool) -> Self {
        Self { label, enabled, on: Some(on) }
    }
}

/// Compact grid key height and font (smaller than the page keys).
pub const GRID_KEY_H: f32 = 23.0;
/// Gap between grid keys (rows and columns).
pub const GRID_GAP: f32 = 3.0;
/// Horizontal padding around the widest label.
pub const GRID_PAD: f32 = 14.0;
pub const GRID_FONT: f32 = 13.0;

/// One size for every key: the widest label plus the ●/○ marker and padding.
pub fn grid_key_size(ui: &mut egui::Ui, keys: &[GridKey]) -> egui::Vec2 {
    let font = egui::FontId::proportional(GRID_FONT);
    let w = keys
        .iter()
        .map(|k| ui.fonts_mut(|f| f.layout_no_wrap(format!("● {}", k.label.to_uppercase()), font.clone(), Color32::WHITE).size().x))
        .fold(0.0, f32::max);
    vec2((w + GRID_PAD).ceil(), GRID_KEY_H)
}

/// Square (radius 0) key in its own hue. Latching keys carry a border and
/// the ●/○ marker; momentary keys are solid. Disabled keys are faded.
pub fn grid_key(ui: &mut egui::Ui, k: &GridKey, size: egui::Vec2) -> egui::Response {
    let sense = if k.enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let hue = crate::bar_key_hue(k.label, crate::KeyLamp::Dark);
    let fade = |c: Color32| if k.enabled { c } else { c.gamma_multiply(0.45) };
    let font = egui::FontId::proportional(GRID_FONT);
    let p = ui.painter();
    match k.on {
        Some(on) => {
            let (fill, border, ink) = rustcars::toggle_colors(hue, on);
            let fill = if k.enabled && resp.hovered() { rustcars::tint(fill, 0.08) } else { fill };
            p.rect_filled(rect, 0.0, fade(fill));
            p.rect_stroke(rect, 0.0, Stroke::new(rustcars::TOGGLE_BORDER, fade(border)), egui::StrokeKind::Inside);
            rustcars::paint_toggle_marker_label(p, rect, k.label, on, font, fade(ink));
        }
        None => {
            let fill = if k.enabled && resp.hovered() { rustcars::tint(hue, 0.25) } else { hue };
            p.rect_filled(rect, 0.0, fade(fill));
            let ink = rustcars::ink_on(hue);
            p.text(rect.center(), egui::Align2::CENTER_CENTER, k.label.to_uppercase(), font, fade(ink));
        }
    }
    if resp.has_focus() {
        p.rect_stroke(rect.expand(2.0), 0.0, Stroke::new(1.0, palette::TEXT), egui::StrokeKind::Outside);
    }
    let name = match k.on {
        Some(on) => rustcars::toggle_label(k.label, on),
        None => k.label.to_uppercase(),
    };
    let (enabled, on) = (k.enabled, k.on);
    resp.widget_info(|| match on {
        Some(v) => egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, v, &name),
        None => egui::WidgetInfo::labeled(egui::Role::Button, enabled, &name),
    });
    resp
}

/// Equal-size keys in `cols` columns (fewer if the row has no room).
/// Returns the index of the key clicked this frame.
pub fn key_grid(ui: &mut egui::Ui, keys: &[GridKey], cols: usize) -> Option<usize> {
    let size = grid_key_size(ui, keys);
    let gap = GRID_GAP;
    let fit = (((ui.available_width() + gap) / (size.x + gap)).floor() as usize).max(1);
    let cols = cols.min(fit).max(1);
    let mut hit = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(gap, gap);
        for (r, row) in keys.chunks(cols).enumerate() {
            ui.horizontal(|ui| {
                for (c, k) in row.iter().enumerate() {
                    if grid_key(ui, k, size).clicked() {
                        hit = Some(r * cols + c);
                    }
                }
            });
        }
    });
    hit
}

/// Bottom status strip and the area above it (status strip 4 px clear of it).
pub fn status_strip_split(area: Rect) -> (Rect, Rect) {
    let h = STRIP_H.min(area.height());
    let strip = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - h), area.max);
    let above = Rect::from_min_max(area.min, Pos2::new(area.right(), (strip.top() - 4.0).max(area.top())));
    (above, strip)
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
    let full = if warn && !text.is_empty() { format!("! {text}") } else { text.to_string() };
    // Too long for the strip: end with "…" (full text on hover), never cut mid-glyph.
    let room = ui.available_width() - 12.0;
    let font = egui::FontId::monospace(12.0);
    let fits = |t: &str, ui: &mut egui::Ui| ui.fonts_mut(|f| f.layout_no_wrap(t.to_owned(), font.clone(), Color32::WHITE).size().x) <= room;
    let mut shown = full.clone();
    if !fits(&shown, ui) {
        let chars: Vec<char> = full.chars().collect();
        let mut n = chars.len();
        while n > 0 {
            n -= 1;
            shown = chars[..n].iter().collect::<String>() + "…";
            if fits(&shown, ui) {
                break;
            }
        }
    }
    let ink = if warn { Color32::from_rgb(0xFF, 0xB0, 0x60) } else { palette::TEXT };
    let r = strip(ui, &shown, "", ink);
    if shown != full {
        ui.interact(r, ui.id().with("status-hover"), Sense::hover()).on_hover_text(full);
    }
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
    /// Vertical axis (dB); `Axis::manual` is the original fixed 70 dB view.
    pub axis: crate::scope_axis::Axis,
    /// AUTO FLOOR: label every grid line of the clean axis.
    pub auto: bool,
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
    let ax = c.axis;
    let floor = if ax.bottom.is_finite() { ax.bottom } else { -70.0 };
    let span = if (ax.top - ax.bottom).is_finite() && ax.top > ax.bottom { ax.top - ax.bottom } else { 70.0 };
    let step = if ax.step.is_finite() && ax.step > 0.0 { ax.step } else { 10.0 };
    let n = ((span / step).round() as usize).clamp(1, 20);
    for i in 0..=10 {
        let x = band.left() + band.width() * i as f32 / 10.0;
        p.line_segment([Pos2::new(x, band.top()), Pos2::new(x, band.bottom())], Stroke::new(1.0, if i % 5 == 0 { GRID } else { GRID_FAINT }));
        if i % 5 == 0 {
            let hz = lo + rate * i as f64 / 10.0;
            let align = match i { 0 => egui::Align2::LEFT_TOP, 10 => egui::Align2::RIGHT_TOP, _ => egui::Align2::CENTER_TOP };
            p.text(Pos2::new(x, band.bottom() + 2.0), align, format!("{:.3}", hz / 1e6), font.clone(), palette::TEXT_MUTED);
        }
    }
    for i in 0..=n {
        let y = band.bottom() - band.height() * i as f32 / n as f32;
        p.line_segment([Pos2::new(band.left(), y), Pos2::new(band.right(), y)], Stroke::new(1.0, if i == 0 || i == n { GRID } else { GRID_FAINT }));
        let label = if c.auto { n <= 8 || i % 2 == 0 || i == n } else { i % 2 == 1 || i == n };
        if label {
            p.text(Pos2::new(band.left() - 4.0, y), egui::Align2::RIGHT_CENTER, format!("{:.0}", floor + span * i as f32 / n as f32), font.clone(), palette::TEXT_MUTED);
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
                let c = Chart { header: "SPEC AVG", header_right: "LNA 16 dB", center_hz: 146e6, rate_hz: 2e6, usable_hz: 1.75e6, db: &db, needle_hz: 146.1e6, height, status: "ok", axis: crate::scope_axis::Axis::manual(&db), auto: false };
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
                    let c = Chart { header: "SPEC AVG", header_right: "LNA 40 dB  VGA 32 dB", center_hz: 146e6, rate_hz: 8e6, usable_hz: 6e6, db: &db, needle_hz: 146.1e6, height: ph, status: "ok", axis: crate::scope_axis::Axis::manual(&db), auto: false };
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
                        let c = Chart { header: "SPEC", header_right: "", center_hz: 1e8, rate_hz: 8e6, usable_hz: 6e6, db: &[], needle_hz: 1e8, height: h, status: "NO CAPTURE", axis: crate::scope_axis::Axis::manual(&[]), auto: false };
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

    /// The status strip is the bottom-most thing on the page, below the
    /// band plan and the keys/scope, and its text is not clipped.
    #[test]
    fn status_strip_is_at_the_page_bottom_under_the_band_plan() {
        for (w, h) in [(1024.0, 550.0), (1280.0, 720.0), (1400.0, 900.0)] {
            let page = Rect::from_min_max(Pos2::new(8.0, CHROME - 40.0), Pos2::new(w - 8.0, h - 8.0));
            let (above, strip_r) = status_strip_split(page);
            assert!((strip_r.bottom() - page.bottom()).abs() < 0.01 && strip_r.height() == STRIP_H);
            assert!((strip_r.width() - page.width()).abs() < 0.01, "full page width");
            let p = tune_panes(above, true);
            assert!(p.bsp.unwrap().bottom() <= strip_r.top() - 3.9, "band plan runs into the status strip at {w}x{h}");
            assert!(p.top.bottom() <= strip_r.top());
            let ctx = ctx();
            let msg = "FOLLOWING CONTROL  ·  Voice is refused on the CW segment 7.000–7.025 MHz";
            let mut got = Rect::NOTHING;
            let out = run(&ctx, w, h, &mut |ui| {
                ui.scope_builder(egui::UiBuilder::new().max_rect(strip_r), |ui| {
                    ui.set_clip_rect(strip_r);
                    got = status_line(ui, msg, true);
                });
            });
            assert_eq!(got, strip_r);
            let text = out.shapes.iter().find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text().starts_with('!') => Some((t.galley.text().to_owned(), t.visual_bounding_rect())),
                _ => None,
            }).expect("status text drawn");
            assert!(strip_r.contains_rect(text.1), "status text clipped at {w}x{h}");
            assert_eq!(text.0, format!("! {msg}"), "fits whole at {w}");
        }
    }

    #[test]
    fn long_status_ends_with_an_ellipsis_not_a_cut() {
        let ctx = ctx();
        let r = Rect::from_min_size(Pos2::new(0.0, 0.0), vec2(200.0, STRIP_H));
        let out = run(&ctx, 400.0, 100.0, &mut |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                status_line(ui, "A very long band plan note that cannot fit in two hundred pixels", true);
            });
        });
        let (t, b) = out.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text().starts_with('!') => Some((t.galley.text().to_owned(), t.visual_bounding_rect())),
            _ => None,
        }).unwrap();
        assert!(t.ends_with('…') && r.contains_rect(b), "{t}");
    }

    /// VFO key grid: every key the same size, columns equal and aligned, in
    /// both the 2-column (sub VFO shown) and 4-column layouts, unclipped.
    #[test]
    fn vfo_grid_keys_are_one_size_and_columns_align() {
        let keys = [
            GridKey::action("A <> B", true), GridKey::toggle("R-6kHz", true, false),
            GridKey::action("A > B", false), GridKey::toggle("R-3kHz", true, true),
            GridKey::action("A > Mem", true), GridKey::toggle("R-15kHz", true, false),
            GridKey::action("Mem > A", false), GridKey::action("R-Auto", true),
            GridKey::toggle("TX main", true, true), GridKey::toggle("TX sub", true, false),
            GridKey::toggle("VFO A", true, true), GridKey::toggle("VFO B", true, false),
        ];
        for (cols, width) in [(2usize, 400.0), (4, 900.0), (4, 230.0)] {
            let ctx = ctx();
            let out = run(&ctx, width, 400.0, &mut |ui| {
                key_grid(ui, &keys, cols);
            });
            let rects: Vec<Rect> = out.shapes.iter().filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.corner_radius == egui::CornerRadius::ZERO && r.fill.a() > 0 && r.rect.height() == GRID_KEY_H => Some(r.rect),
                _ => None,
            }).collect();
            assert_eq!(rects.len(), keys.len(), "one cap per key ({cols} cols at {width})");
            let s = rects[0].size();
            assert!(rects.iter().all(|r| (r.size() - s).length() < 0.01), "keys differ in size");
            assert!(s.y < KEY_H, "grid keys are more compact than page keys");
            let mut xs: Vec<i32> = rects.iter().map(|r| r.left().round() as i32).collect();
            xs.sort();
            xs.dedup();
            let n = xs.len();
            assert!(n == cols || width < 400.0, "{n} columns, wanted {cols}");
            if n > 1 {
                let step = xs[1] - xs[0];
                assert!(xs.windows(2).all(|w| w[1] - w[0] == step), "columns not evenly spaced: {xs:?}");
            }
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, 400.0));
            assert!(rects.iter().all(|r| screen.contains_rect(*r)), "a key is clipped at {width}");
            // Labels fit inside their keys.
            for c in &out.shapes {
                if let egui::Shape::Text(t) = &c.shape {
                    let b = t.visual_bounding_rect();
                    assert!(rects.iter().any(|r| r.expand(0.5).contains_rect(b)), "label {} spills", t.galley.text());
                }
            }
        }
    }

    /// David: LNA/VGA fields looked cartoonish. Each gain field is exactly the
    /// AVG/PEAK key height, square, value text inside, nothing clipped, at the
    /// left-column widths of 1024x550 / 1400x900 and a narrow column.
    #[test]
    fn gain_fields_match_the_key_row_and_never_clip() {
        // Left column widths: half the page at 1024 and 1400, and narrow.
        for w in [470.0, 650.0, 260.0] {
            let ctx = ctx();
            let mut keys = Vec::new();
            let mut fields = Vec::new();
            let out = run(&ctx, w, 300.0, &mut |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    keys.push(toggle(ui, "Avg", true, true, TRACE1).rect);
                    keys.push(toggle(ui, "Peak", true, false, TRACE1).rect);
                    let (mut l, mut v) = (24u32, 30u32);
                    ui.add_space(8.0);
                    fields.push(gain_slider(ui, "LNA", &mut l, 40, 8).rect);
                    ui.add_space(8.0);
                    fields.push(gain_slider(ui, "VGA", &mut v, 62, 2).rect);
                });
            });
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(w, 300.0));
            for f in &fields {
                assert_eq!(f.height(), keys[0].height(), "field height at {w}");
                assert!(screen.contains_rect(*f), "field clipped at {w}: {f:?}");
                assert!(f.width() >= GAIN_TRACK_MIN, "too small");
            }
            let mut texts = 0;
            for c in &out.shapes {
                match &c.shape {
                    egui::Shape::Text(t) => {
                        let b = t.visual_bounding_rect();
                        let s = t.galley.text();
                        if ["LNA", "VGA", "24 dB", "30 dB"].contains(&s) {
                            texts += 1;
                            assert!(fields.iter().any(|f| f.contains_rect(b)), "{s} spills its field at {w}");
                        }
                    }
                    egui::Shape::Rect(r) if fields.iter().any(|f| f.contains_rect(r.rect)) => {
                        assert_eq!(r.corner_radius, egui::CornerRadius::ZERO, "rounded part");
                        assert!(r.stroke.width == 0.0 || r.stroke.color != Color32::BLACK, "black stroke");
                    }
                    egui::Shape::Circle(c) if fields.iter().any(|f| f.contains(c.center)) => panic!("round blob in a gain field"),
                    _ => {}
                }
            }
            assert_eq!(texts, 4, "captions and values drawn at {w}");
        }
    }

    /// AUTO FLOOR draws the clean axis: every label is a multiple of the step,
    /// the bottom label sits under the noise and the top over the carrier;
    /// with auto off the labels are the original top-70 dB set.
    #[test]
    fn auto_floor_axis_labels_are_clean_and_auto_off_is_unchanged() {
        let db: Vec<f32> = (0..512).map(|i| if i == 250 { -30.0 } else { -80.0 + (i % 5) as f32 * 0.4 }).collect();
        for auto in [true, false] {
            let axis = if auto { crate::scope_axis::AutoFloor::default().update(&db).unwrap() } else { crate::scope_axis::Axis::manual(&db) };
            let ctx = ctx();
            let out = run(&ctx, 900.0, 500.0, &mut |ui| {
                let c = Chart { header: "SPEC AVG AUTO", header_right: "", center_hz: 146e6, rate_hz: 2e6, usable_hz: 1.75e6, db: &db, needle_hz: 146.1e6, height: 300.0, status: "ok", axis, auto };
                spectrum_chart(ui, c);
            });
            let labels: Vec<f32> = out.shapes.iter().filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => t.galley.text().parse::<f32>().ok().filter(|v| *v <= 50.0 && *v >= -200.0 && !t.galley.text().contains('.')),
                _ => None,
            }).collect();
            if auto {
                assert_eq!(axis, crate::scope_axis::Axis { bottom: -90.0, top: -20.0, step: 10.0 });
                assert!(labels.contains(&-90.0) && labels.contains(&-20.0), "{labels:?}");
                assert!(labels.iter().all(|v| v % 10.0 == 0.0), "{labels:?}");
            } else {
                assert_eq!(labels, vec![-90.0, -70.0, -50.0, -30.0], "original labels");
            }
        }
    }

    /// VFD VFO digits: each of 0-9 lights exactly its segment set (lit core
    /// strokes in VFD_LIT, the rest faint ghosts), the dot is an LED dot,
    /// everything (glow included) stays inside the digit block, no black strokes.
    #[test]
    fn vfd_digits_light_the_right_segments_inside_the_block() {
        let want: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];
        for scale in [0.9f32, 0.63] {
            for (d, bits) in want.iter().enumerate() {
                let ch = char::from_digit(d as u32, 10).unwrap();
                let ctx = ctx();
                // Block the size of a 48 px monospace digit button at this scale.
                let block = Rect::from_min_size(Pos2::new(20.0, 20.0), vec2(36.0 * scale, 62.0 * scale));
                let out = run(&ctx, 200.0, 200.0, &mut |ui| {
                    vfd_char(&ui.painter_at(block), block, ch);
                });
                let (segs, th) = vfd_segments(block);
                let mut lit = 0u8;
                for c in &out.shapes {
                    match &c.shape {
                        egui::Shape::LineSegment { points, stroke } => {
                            assert_ne!(stroke.color, Color32::BLACK, "black stroke");
                            let half = stroke.width / 2.0;
                            for q in points {
                                assert!(block.shrink(half - 0.01).contains(*q), "glow leaves the block: {q:?} {d}");
                            }
                            if stroke.color == VFD_LIT {
                                let n = segs.iter().position(|s| s == points).expect("lit stroke is a segment");
                                lit |= 1 << n;
                                assert_eq!(stroke.width, th);
                            }
                        }
                        other => panic!("unexpected shape {other:?}"),
                    }
                }
                assert_eq!(lit, *bits, "digit {d}");
            }
        }
        let ctx = ctx();
        let block = Rect::from_min_size(Pos2::new(20.0, 20.0), vec2(20.0, 56.0));
        let out = run(&ctx, 200.0, 200.0, &mut |ui| vfd_char(&ui.painter_at(block), block, '.'));
        let dots: Vec<_> = out.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Circle(c) => Some(*c), _ => None }).collect();
        assert_eq!(dots.len(), 3);
        assert!(dots.iter().all(|c| block.contains_rect(Rect::from_center_size(c.center, vec2(c.radius * 2.0, c.radius * 2.0)))));
        assert!(dots.iter().any(|c| c.fill == VFD_LIT));
    }
}
