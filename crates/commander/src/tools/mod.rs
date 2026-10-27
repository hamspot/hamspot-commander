//! Tools mounted from the rustcars examples (MIT). See `licenses/rustcars-MIT.txt`.
//! One screen at a time. The whole tab can tear off into its own window.

mod calculator;
mod ham_rf;
mod hexview;
mod rotator;
pub mod nanovna;
pub(crate) mod smith;
mod station_log;
mod timers;

use rustcars::{Cap, LcarsTheme, Side, lcars_button, palette, readout};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    NanoVna,
    HamRf,
    Calculator,
    Smith,
    Hex,
    Log,
    Rotator,
}

const SCREENS: [(Screen, &str); 7] = [
    (Screen::NanoVna, "NanoVNA"),
    (Screen::HamRf, "RF Calc"),
    (Screen::Calculator, "Calc"),
    (Screen::Smith, "Smith"),
    (Screen::Hex, "Hex"),
    (Screen::Log, "Logbook"),
    (Screen::Rotator, "Rotator"),
];

pub struct Tools {
    pub screen: Screen,
    pub detached: bool,
    pub dock: bool,
    pub saved_pos: Option<egui::Pos2>,
    pub saved_size: Option<egui::Vec2>,
    theme: LcarsTheme,
    calc: calculator::Calc,
    log: station_log::StationLog,
    ham: ham_rf::HamRf,
    timers: timers::Timers,
    hex: hexview::HexView,
    smith: smith::Smith,
    nanovna: nanovna::NanoVna,
    rotator: rotator::Rotator,
    /// True while the NanoVNA screen was showing on the previous frame.
    nano_open: bool,
    open_note: String,
}

impl Tools {
    pub fn new(ctx: &egui::Context) -> Self {
        let theme = LcarsTheme::default();
        rustcars::install_lcars_font(ctx);
        rustcars::apply_theme(ctx, &theme);
        Self {
            screen: Screen::Log,
            detached: false,
            dock: false,
            saved_pos: None,
            saved_size: None,
            theme,
            calc: calculator::Calc::new(ctx),
            log: station_log::StationLog::new(),
            ham: ham_rf::HamRf::new(ctx),
            timers: timers::Timers::new(ctx),
            hex: hexview::HexView::new(
                ctx,
                "Antonio-Variable.ttf (sample)".into(),
                rustcars::ANTONIO_TTF.to_vec(),
                false,
            ),
            smith: smith::Smith::new(ctx),
            nanovna: nanovna::NanoVna::new(ctx),
            rotator: rotator::Rotator::new(),
            nano_open: false,
            open_note: String::new(),
        }
    }

    /// The rotators Config Automation starts. The Rotator screen drives the selected one.
    pub fn set_rotators(&mut self, hub: std::sync::Arc<crate::rot::Hub>) {
        self.rotator.set_hub(hub);
    }

    /// Rotator tab settings as JSON, for eframe storage.
    pub fn rotator_settings(&self) -> String {
        self.rotator.settings_json()
    }

    pub fn load_rotator_settings(&mut self, text: &str) {
        self.rotator.load_settings_json(text);
    }

    pub fn set_plan(&mut self, plan: &crate::plan::BandPlan) {
        self.nanovna.set_plan(plan);
    }

    pub fn nanovna_face(&self) -> nanovna::Face {
        self.nanovna.station_face()
    }

    pub fn set_nanovna_face(&mut self, face: nanovna::Face) {
        self.nanovna.apply_face(face);
    }

    pub fn set_vna_archive(&mut self, archive: nanovna::VnaFile) {
        self.nanovna.set_archive(archive);
    }

    pub fn vna_archive(&self) -> nanovna::VnaFile {
        self.nanovna.archive()
    }

    pub fn nanovna_dirty(&mut self) -> bool {
        self.nanovna.take_dirty()
    }

    pub fn take_vna_dirty(&mut self) -> bool {
        self.nanovna.take_archive_dirty()
    }

    pub fn mark_vna_dirty(&mut self) {
        self.nanovna.mark_archive_dirty();
    }

    pub fn set_dial(&mut self, hz: f64, mode: &str, radio: &str) {
        self.log.set_dial(hz, mode, radio);
    }

    pub fn follow_station(&mut self, ctx: &egui::Context, saved: bool, station: &str) {
        self.log.follow(ctx, saved, station);
    }

    /// `in_window` is the torn-off viewport. Closing that window docks the
    /// tools. The main window's tab row owns the tear-off square.
    ///
    /// Panels are added before any other widget. egui ignores a panel that
    /// follows ordinary content in the same ui.
    pub fn ui(&mut self, ui: &mut egui::Ui, in_window: bool) {
        if in_window && ui.input(|input| input.viewport().close_requested()) {
            self.dock = true;
            self.detached = false;
        }
        egui::Panel::top("tool_nav")
            .show_separator_line(false)
            .frame(tab_row(self.theme.bg))
            .show(ui, |ui| {
                self.nav(ui);
            });
        if in_window {
            self.remember_window(ui);
        }
        self.screen(ui);
    }

    /// Station clocks. The Timers page calls this on the root ui, the same
    /// way the tool screens add their own panels.
    pub fn draw_timers(&mut self, ui: &mut egui::Ui) {
        self.timers.draw(ui);
    }

    /// The tools are not on screen. The next visit to NanoVNA may connect.
    pub fn conceal_nanovna(&mut self) {
        if self.nano_open {
            self.nanovna.hidden();
            self.nano_open = false;
        }
    }

    fn remember_window(&mut self, ui: &egui::Ui) {
        let info = ui.input(|input| input.viewport().clone());
        if let Some(outer) = info.outer_rect {
            self.saved_pos = Some(outer.min);
        }
        if let Some(inner) = info.inner_rect {
            let size = inner.size();
            if size.x >= 64.0 && size.y >= 64.0 {
                self.saved_size = Some(size);
            }
        }
    }

    fn nav(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        egui::ScrollArea::horizontal()
            .id_salt("tool-nav")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
            for (index, (screen, label)) in SCREENS.iter().enumerate() {
                let color = if *screen == self.screen {
                    palette::TAN
                } else {
                    screen_color(index)
                };
                let cap = if index == 0 { Cap::Left } else { Cap::None };
                let width = (label.len() as f32) * 12.0 + 36.0;
                if lcars_button(
                    ui,
                    &theme,
                    label,
                    color,
                    egui::vec2(width, theme.bar_thin),
                    cap,
                    true,
                )
                .clicked()
                {
                    self.screen = *screen;
                }
            }
                });
            });
        if !self.open_note.is_empty() {
            readout(ui, &theme, &self.open_note, false);
        }
    }

    fn open_hex(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open file")
            .pick_file()
        else {
            return;
        };
        self.open_note = match self.hex.load_file(&path) {
            Ok(()) => String::new(),
            Err(err) => err,
        };
    }

    fn screen(&mut self, ui: &mut egui::Ui) {
        let nano = self.screen == Screen::NanoVna;
        if nano && !self.nano_open {
            self.nanovna.shown();
        } else if !nano && self.nano_open {
            self.nanovna.hidden();
        }
        self.nano_open = nano;
        match self.screen {
            Screen::Calculator => self.calc.draw(ui),
            Screen::Log => self.log.draw(ui),
            Screen::HamRf => self.ham.draw(ui),
            Screen::Hex => {
                self.hex.draw(ui);
                if self.hex.take_open() {
                    self.open_hex();
                }
            }
            Screen::Smith => self.smith.draw(ui),
            Screen::NanoVna => self.nanovna.draw(ui),
            Screen::Rotator => self.rotator.draw(ui),
        }
    }
}

/// One tab row. The bottom gap is `frame_gap`. A row with no gap shares its
/// edge with the next row, and the button paint overlaps that row.
pub(crate) fn tab_row(bg: egui::Color32) -> egui::Frame {
    let gap = LcarsTheme::default().frame_gap as i8;
    egui::Frame::new().fill(bg).inner_margin(egui::Margin {
        left: 0,
        right: 0,
        top: 0,
        bottom: gap,
    })
}

/// Outer height of a tab row: the buttons stay `bar_thin`, then the gap.
pub(crate) fn tab_row_height(theme: &LcarsTheme) -> f32 {
    theme.bar_thin + theme.frame_gap
}

/// Header above a tool screen or the Timers page. `exact_size` includes the
/// bottom gap, so the buttons are not clipped to make room for it.
pub(crate) fn screen_header(
    ui: &mut egui::Ui,
    id: &str,
    theme: &LcarsTheme,
    add: impl FnOnce(&mut egui::Ui),
) {
    egui::Panel::top(id)
        .exact_size(tab_row_height(theme))
        .show_separator_line(false)
        .frame(tab_row(theme.bg))
        .show(ui, add);
}

/// Square tab at the right end of the page row. Same height as those tabs.
/// The mark is an arrow leaving a box: the tools window will open.
pub fn tear_off_button(ui: &mut egui::Ui) -> egui::Response {
    let theme = LcarsTheme::default();
    let side = theme.bar_thin;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    let base = palette::TAN;
    let fill = if resp.is_pointer_button_down_on() {
        rustcars::tint(base, -0.25)
    } else if resp.hovered() {
        rustcars::tint(base, 0.3)
    } else {
        base
    };
    let painter = ui.painter();
    painter.rect_filled(rect, rustcars::cap_radius(Cap::None, rect.height()), fill);
    paint_window_mark(painter, rect, rustcars::ink_on(fill), theme.frame_gap * 0.5);
    let name = "Tear off";
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), name));
    resp.on_hover_text(name)
}

/// A box and a diagonal arrow pointing out of it.
fn paint_window_mark(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32, stroke_width: f32) {
    let stroke = egui::Stroke::new(stroke_width.max(1.0), color);
    let box_rect = rect.shrink(rect.width() * 0.30);
    painter.rect_stroke(
        box_rect,
        egui::CornerRadius::ZERO,
        stroke,
        egui::StrokeKind::Inside,
    );
    let outside = rect.right_top() + egui::vec2(-rect.width() * 0.16, rect.width() * 0.16);
    let from = box_rect.center();
    let to = outside;
    painter.line_segment([from, to], stroke);
    let delta = to - from;
    let len = delta.length();
    if len < 1.0 {
        return;
    }
    let dir = delta / len;
    let normal = egui::vec2(-dir.y, dir.x);
    let head = rect.width() * 0.16;
    painter.line_segment([to, to - dir * head + normal * (head * 0.55)], stroke);
    painter.line_segment([to, to - dir * head - normal * (head * 0.55)], stroke);
}

/// Name block at the left of a screen's header: the same tan cap as a tab,
/// but it is a label. It has no click sense and no hover or press color.
pub(crate) fn title_block(
    ui: &mut egui::Ui,
    theme: &LcarsTheme,
    label: &str,
    base: egui::Color32,
    size: egui::Vec2,
) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, rustcars::cap_radius(Cap::Left, rect.height()), base);
    let pad = (rect.height() / 2.0).min(theme.main_gap);
    painter.text(
        egui::pos2(rect.right() - pad, rect.bottom() - theme.frame_gap),
        egui::Align2::RIGHT_BOTTOM,
        label.to_uppercase(),
        egui::FontId::proportional(theme.header_size),
        rustcars::ink_on(base),
    );
}

/// A header key beside the tabs: tab size and tab label placement, a dark
/// fill, and a thin amber border. It is for an action, not a tab choice.
pub(crate) fn header_key(
    ui: &mut egui::Ui,
    theme: &LcarsTheme,
    label: &str,
    size: egui::Vec2,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let colors = rustcars::Mode::Inactive.colors();
    let mut fill = colors.fill;
    if resp.is_pointer_button_down_on() {
        fill = rustcars::tint(fill, -0.25);
    } else if resp.hovered() {
        fill = rustcars::tint(fill, 0.12);
    }
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, fill);
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, palette::TAN),
        egui::StrokeKind::Inside,
    );
    if resp.has_focus() {
        painter.rect_stroke(
            rect.shrink(3.0),
            0.0,
            egui::Stroke::new(1.0, theme.text_hi),
            egui::StrokeKind::Inside,
        );
    }
    let pad = (rect.height() / 2.0).min(theme.main_gap);
    painter.text(
        egui::pos2(rect.right() - pad, rect.bottom() - theme.frame_gap),
        egui::Align2::RIGHT_BOTTOM,
        label.to_uppercase(),
        egui::FontId::proportional(theme.header_size),
        palette::TAN,
    );
    let enabled = ui.is_enabled();
    let l = label.to_owned();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &l));
    resp
}

/// One list row, on every tool screen. Monospace at `data_size`.
pub(crate) const LIST_ROW: f32 = 28.0;

fn screen_color(index: usize) -> egui::Color32 {
    const COLORS: [egui::Color32; 6] = [
        palette::SLATE,
        palette::OLIVE,
        palette::BRICK,
        palette::TEAL,
        palette::PLUM,
        palette::TAN,
    ];
    COLORS[index % COLORS.len()]
}

/// The tools window is its own viewport, same idea as the meter window.
pub fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("hamspot-tools")
}

/// Paint one page inside an LCARS frame. Instrument paint stays in the content well.
pub fn frame(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) -> egui::Rect {
    let theme = LcarsTheme::default();
    let mut frame = rustcars::LcarsFrame::new(&theme, Side::Left, 1.0);
    frame.min_height = ui.available_height();
    // The top bar's end cap: the tab strips below may turn it into a
    // "more tabs" key. Same geometry as rustcars' LcarsFrame for Side::Left.
    let outer = ui.available_rect_before_wrap();
    let cap = egui::Rect::from_min_size(
        egui::pos2(outer.right() - frame.cap_len, outer.top()),
        egui::vec2(frame.cap_len, frame.thin),
    );
    ui.ctx().data_mut(|d| d.insert_temp(top_cap_id(ui.ctx()), cap));
    frame.show(ui, &theme, add).1
}

fn top_cap_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::NULL.with(("elbow-top-cap", ctx.viewport_id()))
}

/// Which tabs fit in `avail` px. `widths` are the tabs' real sizes, `gap` the
/// spacing between them. Order is kept. The selected tab always stays in the
/// row: if it would not fit, earlier tabs leave the row to make room.
/// Returns (shown, hidden) as indices. When everything fits, hidden is empty.
pub fn partition_tabs(
    widths: &[f32],
    gap: f32,
    avail: f32,
    selected: Option<usize>,
) -> (Vec<usize>, Vec<usize>) {
    let span = |idx: &[usize]| -> f32 {
        idx.iter().map(|&i| widths[i]).sum::<f32>() + gap * idx.len().saturating_sub(1) as f32
    };
    let all: Vec<usize> = (0..widths.len()).collect();
    if span(&all) <= avail {
        return (all, Vec::new());
    }
    let mut shown: Vec<usize> = Vec::new();
    for i in 0..widths.len() {
        let mut next = shown.clone();
        next.push(i);
        if span(&next) <= avail {
            shown = next;
        } else {
            break;
        }
    }
    if let Some(sel) = selected.filter(|&s| s < widths.len()) {
        if !shown.contains(&sel) {
            loop {
                let mut next = shown.clone();
                next.push(sel);
                if span(&next) <= avail || shown.is_empty() {
                    shown = next;
                    break;
                }
                shown.pop();
            }
        }
    }
    shown.sort_unstable();
    let hidden = all.into_iter().filter(|i| !shown.contains(i)).collect();
    (shown, hidden)
}

/// One row of tabs under the frame's top elbow. Tabs that do not fit leave the
/// row, and the elbow's end cap becomes a "more tabs" key that lists them.
/// When all fit, the cap stays plain chrome. `width(i)` is tab i's real
/// width; `draw(ui, i, lead)` paints tab i and returns its response.
/// Returns the index picked from the row or from the list.
pub fn overflow_tab_row(
    ui: &mut egui::Ui,
    id_salt: &str,
    labels: &[String],
    selected: Option<usize>,
    width: impl Fn(&egui::Ui, usize) -> f32,
    mut draw: impl FnMut(&mut egui::Ui, usize, bool) -> egui::Response,
) -> Option<usize> {
    let widths: Vec<f32> = (0..labels.len()).map(|i| width(ui, i)).collect();
    let gap = ui.spacing().item_spacing.x;
    let (shown, hidden) = partition_tabs(&widths, gap, ui.available_width(), selected);
    let mut picked = None;
    ui.horizontal(|ui| {
        for (n, &i) in shown.iter().enumerate() {
            if draw(ui, i, n == 0).clicked() {
                picked = Some(i);
            }
        }
    });
    if hidden.is_empty() {
        return picked;
    }
    let Some(cap) = ui.ctx().data(|d| d.get_temp::<egui::Rect>(top_cap_id(ui.ctx()))) else {
        return picked;
    };
    let id = ui.scope_id().with(("more-tabs", id_salt));
    let resp = ui.interact(cap, id, egui::Sense::click());
    paint_more_key(ui, cap, &resp);
    let resp = resp.on_hover_text(format!("More tabs ({} hidden)", hidden.len()));
    let l = format!("More tabs, {} hidden", hidden.len());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &l));
    egui::Popup::menu(&resp).show(|ui| {
        for &i in &hidden {
            if ui.selectable_label(selected == Some(i), labels[i].to_uppercase()).clicked() {
                picked = Some(i);
                ui.close();
            }
        }
    });
    picked
}

/// The end cap as a key: a thin dark border and a small down-pointing
/// triangle, both muted, so its new job shows by shape and not by colour.
fn paint_more_key(ui: &egui::Ui, cap: egui::Rect, resp: &egui::Response) {
    let theme = LcarsTheme::default();
    let painter = ui.painter();
    let ink = theme.bg.gamma_multiply(if resp.hovered() { 0.85 } else { 0.6 });
    let r = rustcars::cap_radius(Cap::Right, cap.height());
    painter.rect_stroke(cap.shrink(2.0), r, egui::Stroke::new(1.0, ink), egui::StrokeKind::Inside);
    if resp.has_focus() {
        painter.rect_stroke(cap.shrink(4.0), r, egui::Stroke::new(1.0, theme.text_hi), egui::StrokeKind::Inside);
    }
    let c = egui::pos2(cap.center().x - 2.0, cap.center().y + 1.0);
    let s = (cap.height() * 0.18).clamp(3.0, 6.0);
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(c.x - s, c.y - s * 0.6),
            egui::pos2(c.x + s, c.y - s * 0.6),
            egui::pos2(c.x, c.y + s * 0.6),
        ],
        ink,
        egui::Stroke::NONE,
    ));
}

/// The straight run of the bottom elbow's horizontal bar: right of the
/// elbow's curve, left of the end cap. Mirrors rustcars' LcarsFrame geometry
/// for `Side::Left` at scale 1.
pub fn bottom_bar_run(frame_rect: egui::Rect) -> egui::Rect {
    let t = LcarsTheme::default();
    let cap_len = 48.0;
    egui::Rect::from_min_max(
        egui::pos2(frame_rect.left() + t.bar_thick + t.bar_thin, frame_rect.bottom() - t.bar_thin),
        egui::pos2(frame_rect.right() - cap_len - 2.0 * t.frame_gap, frame_rect.bottom()),
    )
}

/// Split `text` at the word boundary nearest its middle. Never cuts a word.
pub fn split_two_lines(text: &str) -> (String, String) {
    let mid = text.len() / 2;
    let best = text
        .char_indices()
        .filter(|(_, c)| *c == ' ')
        .map(|(i, _)| i)
        .min_by_key(|i| i.abs_diff(mid));
    match best {
        Some(i) => (text[..i].trim_end().to_string(), text[i + 1..].trim_start().to_string()),
        None => (text.to_string(), String::new()),
    }
}

/// Faint watermark ink on a frame bar: the theme background at low alpha,
/// so it reads as a darker shade of the bar colour.
pub const WATERMARK_ALPHA: f32 = 0.22;
pub fn watermark_color() -> egui::Color32 {
    LcarsTheme::default().bg.gamma_multiply(WATERMARK_ALPHA)
}

/// Paint `text` as a faint watermark centred in the bottom elbow's bar.
/// One line if it fits, else two lines split at a word, shrinking the font
/// as needed. Hover shows the full text; it senses hover only, never clicks.
pub fn bottom_bar_watermark(ui: &mut egui::Ui, frame_rect: egui::Rect, text: &str) {
    let run = bottom_bar_run(frame_rect).shrink2(egui::vec2(8.0, 2.0));
    if run.width() < 60.0 || run.height() < 8.0 {
        return;
    }
    let color = watermark_color();
    let painter = ui.painter().with_clip_rect(run);
    let fits = |size: f32, s: &str| {
        ui.fonts_mut(|f| f.layout_no_wrap(s.to_string(), egui::FontId::proportional(size), color))
    };
    let mut painted = false;
    for size in [12.0_f32, 11.0, 10.0] {
        let g = fits(size, text);
        if g.size().x <= run.width() {
            painter.galley(run.center() - g.size() / 2.0, g, color);
            painted = true;
            break;
        }
    }
    if !painted {
        let (a, b) = split_two_lines(text);
        for size in [11.0_f32, 10.0, 9.0, 8.0] {
            let ga = fits(size, &a);
            let gb = fits(size, &b);
            let w = ga.size().x.max(gb.size().x);
            let h = ga.size().y + gb.size().y;
            if (w <= run.width() && h <= run.height()) || size <= 8.0 {
                let top = run.center().y - h / 2.0;
                let ha = ga.size();
                painter.galley(egui::pos2(run.center().x - ha.x / 2.0, top), ga, color);
                painter.galley(egui::pos2(run.center().x - gb.size().x / 2.0, top + ha.y), gb, color);
                break;
            }
        }
    }
    let id = ui.scope_id().with("bottom-bar-watermark");
    let resp = ui.interact(run, id, egui::Sense::hover());
    resp.on_hover_text(text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tabs_fit_hides_nothing() {
        let (shown, hidden) = partition_tabs(&[50.0, 50.0, 50.0], 4.0, 158.0, Some(2));
        assert_eq!(shown, vec![0, 1, 2]);
        assert!(hidden.is_empty());
    }

    #[test]
    fn overflow_hides_the_tail() {
        let (shown, hidden) = partition_tabs(&[50.0, 50.0, 50.0, 50.0], 4.0, 110.0, Some(0));
        assert_eq!(shown, vec![0, 1]);
        assert_eq!(hidden, vec![2, 3]);
    }

    #[test]
    fn selected_tab_stays_in_the_row() {
        let (shown, hidden) = partition_tabs(&[50.0, 50.0, 50.0, 50.0], 4.0, 110.0, Some(3));
        assert_eq!(shown, vec![0, 3]);
        assert_eq!(hidden, vec![1, 2]);
        // A wide selected tab pushes out more than one.
        let (shown, _) = partition_tabs(&[50.0, 50.0, 50.0, 100.0], 4.0, 160.0, Some(3));
        assert_eq!(shown, vec![0, 3]);
        // Even when it alone is too wide, it is still shown.
        let (shown, hidden) = partition_tabs(&[50.0, 200.0], 4.0, 100.0, Some(1));
        assert_eq!(shown, vec![1]);
        assert_eq!(hidden, vec![0]);
    }

    #[test]
    fn no_selection_and_empty_rows() {
        let (shown, hidden) = partition_tabs(&[80.0, 80.0], 4.0, 100.0, None);
        assert_eq!((shown, hidden), (vec![0], vec![1]));
        let (shown, hidden) = partition_tabs(&[], 4.0, 100.0, None);
        assert!(shown.is_empty() && hidden.is_empty());
    }

    #[test]
    fn watermark_splits_at_a_word_near_the_middle() {
        let text = crate::rec_ui::PRIVACY;
        let (a, b) = split_two_lines(text);
        assert_eq!(format!("{a} {b}"), text);
        assert!(a.len().abs_diff(b.len()) < 20);
        assert_eq!(split_two_lines("one"), ("one".to_string(), String::new()));
    }

    #[test]
    fn watermark_is_faint_and_sits_in_the_bottom_bar() {
        let c = watermark_color();
        assert!(c.a() > 20 && c.a() < 80, "alpha {}", c.a());
        let f = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 800.0));
        let r = bottom_bar_run(f);
        assert_eq!(r.bottom(), 800.0);
        assert_eq!(r.height(), 40.0);
        assert!(r.left() >= 120.0 && r.right() <= 1280.0 - 48.0);
    }

    #[test]
    fn the_station_log_sits_in_the_tool_list() {
        let labels: Vec<_> = SCREENS.iter().map(|(_, label)| *label).collect();
        assert_eq!(
            labels,
            vec!["NanoVNA", "RF Calc", "Calc", "Smith", "Hex", "Logbook", "Rotator"]
        );
    }
}
