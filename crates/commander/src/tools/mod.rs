//! Tools mounted from the rustcars examples (MIT). See `licenses/rustcars-MIT.txt`.
//! One screen at a time. The whole tab can tear off into its own window.

mod calculator;
mod ham_rf;
mod hexview;
mod rotator;
pub mod nanovna;
pub(crate) mod smith;
mod station_log;
pub mod timers;

use rustcars::{Cap, LcarsTheme, Side, palette};

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

pub(crate) const SCREENS: [(Screen, &str); 7] = [
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
    /// Mirror of the Tools page pill's detach state (show_page_windows).
    pub detached: bool,
    pub saved_pos: Option<egui::Pos2>,
    pub saved_size: Option<egui::Vec2>,
    theme: LcarsTheme,
    estop_chord: crate::estop::Chord,
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
        no_scroll_fade(ctx);
        Self {
            screen: Screen::Log,
            estop_chord: crate::estop::Chord::default(),
            detached: false,
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

    pub fn nanovna_mut(&mut self) -> &mut nanovna::NanoVna {
        &mut self.nanovna
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

    /// The detached Tools window (page pill): E-STOP / RELEASE at the right
    /// of the top row, as in the page-bar slot, then the same body as the
    /// page. The window has its own keyboard focus, so it polls the chord.
    pub fn window_body(&mut self, ui: &mut egui::Ui) {
        self.estop_chord.poll(ui.ctx());
        let h = self.theme.bar_thin;
        let row = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover()).0;
        let estop_w = crate::estop::button_width(ui, crate::estop::engaged()) + 8.0;
        let tabs = egui::Rect::from_min_max(row.min, egui::pos2(row.right() - estop_w, row.bottom()));
        self.bar_tabs(ui, tabs);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(egui::pos2(tabs.right(), row.top()), row.max))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                crate::estop::button(ui);
            },
        );
        ui.add_space(self.theme.frame_gap);
        self.draw_in(ui);
    }

    /// The Tools page body inside the shared page frame, REC style: the
    /// tool sub-tabs (more-tabs overflow), one status line for the open
    /// tool, then the tool screen in the remaining area. The tool screens
    /// add their panels inside `ui`, so they stay within the frame.
    pub fn draw_in(&mut self, ui: &mut egui::Ui) {
        // The tool sub-tabs are in the top elbow bar (`bar_tabs`), not here.
        // NanoVNA has its own status bar under the chart (device key,
        // STATUS), so it gets no top line (David, #222): the chart starts
        // right under the bar.
        if self.screen == Screen::NanoVna {
            crate::status_note::clear(ui.ctx());
            self.screen(ui);
            crate::status_note::clear(ui.ctx());
            return;
        }
        // Every other tool: the screen fills the frame, and one status strip
        // in NanoVNA's style sits at the bottom, always in the same place
        // (David, #221): hints, notes and warnings as plain text.
        // In a detached window the page sits in a scroll area, whose height
        // is unbounded: bound it by what is visible, or the strip and any
        // screen sized from the height (the rotator dial) land far below.
        let mut full = ui.available_rect_before_wrap();
        let vis = ui.clip_rect();
        if !full.bottom().is_finite() || full.bottom() > vis.bottom() {
            full.max.y = vis.bottom().max(full.top() + 120.0);
        }
        let strip = egui::Rect::from_min_max(egui::pos2(full.left(), full.bottom() - crate::status_note::STRIP_H), full.max);
        let body = egui::Rect::from_min_max(full.min, egui::pos2(full.right(), strip.top() - self.theme.frame_gap));
        crate::status_note::clear(ui.ctx());
        // A child that does not grow the frame: a screen taller than the
        // window is clipped above the strip, never pushes it down.
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("tool-screen"));
        child.set_clip_rect(body.intersect(ui.clip_rect()));
        self.screen(&mut child);
        let base = self.status_line();
        crate::status_note::paint_strip(ui, strip, &base);
        ui.allocate_rect(full, egui::Sense::hover());
    }

    /// Width the open tool's header keys want in the elbow bar.
    pub fn header_width(&self) -> f32 {
        let g = BAR_TAB_GAP;
        match self.screen {
            Screen::Log => 90.0 * 2.0 + g,
            Screen::HamRf => ham_rf::TOOL_COUNT as f32 * (96.0 + g) - g,
            Screen::Calculator => 90.0,
            Screen::Hex => 96.0,
            Screen::Rotator => 2.0 * rotator::BAR_LED_W + g,
            _ => 0.0,
        }
    }

    pub fn tab_labels() -> Vec<String> {
        SCREENS.iter().map(|(_, l)| (*l).to_owned()).collect()
    }

    /// The tool sub-tabs in a top-bar gap (page bar or detached window row).
    pub fn bar_tabs(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let labels = Self::tab_labels();
        let sel = SCREENS.iter().position(|(s, _)| *s == self.screen);
        if let Some(i) = bar_tab_row(ui, rect, "tool-tabs", &labels, sel) {
            self.screen = SCREENS[i].0;
        }
    }

    /// One status line per tool, under the sub-tabs. Plain text.
    pub fn status_line(&self) -> String {
        let mut line = match self.screen {
            Screen::NanoVna => self.nanovna.status_line(),
            Screen::HamRf => {
                let name = self.ham.tool_name().to_uppercase();
                if self.ham.status.is_empty() { format!("RF CALC {name}") } else { format!("RF CALC {name}   {}", self.ham.status) }
            }
            Screen::Calculator => {
                let angle = self.calc.angle_label();
                // The answer is on the calculator's own display; only an
                // error is repeated here, in words.
                match &self.calc.result {
                    Ok(_) => format!("CALC {angle}   HISTORY {}", self.calc.history.len()),
                    Err(e) => format!("CALC {angle}   ERROR {e}   HISTORY {}", self.calc.history.len()),
                }
            }
            // R, X, Z0 and F are in the Smith fields on screen.
            Screen::Smith => "SMITH CHART".to_owned(),
            Screen::Hex => {
                let mut l = if self.hex.data.is_empty() {
                    "HEX NO FILE: OPEN ONE WITH OPEN".to_owned()
                } else {
                    format!("HEX {}   {} BYTES{}", self.hex.name, self.hex.data.len(), if self.hex.truncated { " (TRUNCATED)" } else { "" })
                };
                if !self.hex.status.is_empty() {
                    l.push_str("   ");
                    l.push_str(&self.hex.status);
                }
                l
            }
            Screen::Log => self.log.status_line(),
            Screen::Rotator => self.rotator.status_line(),
        };
        if !self.open_note.is_empty() {
            line.push_str("   ");
            line.push_str(&self.open_note);
        }
        line
    }

    /// Station clocks and timers. The Timers page draws them inside the
    /// shared page frame (sub-tabs, status line, bar controls in the slot).
    pub fn timers_mut(&mut self) -> &mut timers::Timers {
        &mut self.timers
    }

    /// The tools are not on screen. The next visit to NanoVNA may connect.
    pub fn conceal_nanovna(&mut self) {
        if self.nano_open {
            self.nanovna.hidden();
            self.nano_open = false;
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

fn or_dash(s: &str) -> &str {
    if s.trim().is_empty() { "-" } else { s }
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
    // In the main window the tool's header keys go into the page's elbow
    // bar, between the tool tabs and E-STOP (David, #221), as square bar
    // keys; too many scroll sideways there. The name block is left out (the
    // open tab names the tool). A detached Tools window keeps the row.
    if let Some(rect) = bar_keys_rect(ui.ctx()) {
        let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new(("tool-bar-keys", id)));
        let flag = in_bar_id(layer);
        ui.ctx().data_mut(|d| d.insert_temp(flag, true));
        let mut row = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(rect).id_salt(("tool-bar-keys", id)));
        row.set_clip_rect(rect);
        egui::ScrollArea::horizontal()
            .id_salt(("tool-bar-keys-scroll", id))
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .auto_shrink([false, false])
            .show(&mut row, |ui| {
                ui.spacing_mut().item_spacing.x = BAR_TAB_GAP;
                add(ui);
            });
        return;
    }
    egui::Panel::top(id)
        .exact_size(tab_row_height(theme))
        .show_separator_line(false)
        .frame(tab_row(theme.bg))
        .show(ui, add);
}

fn in_bar_id(layer: egui::LayerId) -> egui::Id {
    egui::Id::NULL.with(("tool-keys-in-bar", layer))
}

/// True while drawing a tool's header keys in the elbow bar.
pub(crate) fn in_bar(ui: &egui::Ui) -> bool {
    ui.data(|d| d.get_temp::<bool>(in_bar_id(ui.layer_id()))).unwrap_or(false)
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
    if in_bar(ui) {
        return; // the open tab already names the tool
    }
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

/// Paint one page inside an LCARS frame. Instrument paint stays in the content well.
pub fn frame(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) -> egui::Rect {
    frame_with_slot(ui, None, add).0
}

/// The opened gap in the top elbow bar, right before the end cap.
/// Returns (black cover, where controls go). The cover is `slot_w` wide and
/// ends at the cap's left edge; the controls sit inside it with
/// `frame_gap` of black on each side, at the bar's full height.
pub fn top_slot(outer: egui::Rect, cap_len: f32, thin: f32, slot_w: f32) -> (egui::Rect, egui::Rect) {
    let gap = LcarsTheme::default().frame_gap;
    let right = outer.right() - cap_len;
    let cover = egui::Rect::from_min_max(
        egui::pos2(right - gap - slot_w, outer.top()),
        egui::pos2(right, outer.top() + thin),
    );
    let inner = egui::Rect::from_min_max(
        egui::pos2(cover.left() + gap, cover.top()),
        egui::pos2(right - gap, cover.bottom()),
    );
    (cover, inner)
}

/// `frame`, optionally opening a `slot_w` gap in the top bar before the end
/// cap. The tan bar then runs from the elbow to the gap, and the gap is
/// returned for the caller to put controls in. `None` keeps the bar closed.
pub fn frame_with_slot(
    ui: &mut egui::Ui,
    slot_w: Option<f32>,
    add: impl FnOnce(&mut egui::Ui),
) -> (egui::Rect, Option<egui::Rect>) {
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
    let id = top_cap_id(ui.ctx());
    ui.ctx().data_mut(|d| d.insert_temp(id, cap));
    let slot = slot_w.map(|w| top_slot(outer, frame.cap_len, frame.thin, w));
    // Page keys asked for this frame (see `request_bar_keys`) get their part
    // of the slot now, before the page draws them.
    let keys_req = bar_keys_req_id(ui.ctx());
    let req = ui.ctx().data_mut(|d| d.remove_temp::<(f32, f32)>(keys_req));
    let keys = match (slot, req) {
        (Some((_, inner)), Some((w, reserve))) => Some(egui::Rect::from_min_max(
            egui::pos2(inner.right() - reserve - w, inner.top()),
            egui::pos2(inner.right() - reserve, inner.bottom()),
        )),
        _ => None,
    };
    let keys_id = bar_keys_rect_id(ui.ctx());
    ui.ctx().data_mut(|d| d.insert_temp(keys_id, keys));
    let cover_id = top_slot_cover_id(ui.ctx());
    let cover = slot.map(|(c, _)| c);
    ui.ctx().data_mut(|d| d.insert_temp(cover_id, cover));
    let rect = frame.show(ui, &theme, add).1;
    // Paint-over: the frame's chrome sits behind content, so this black
    // cover lands on top of the tan and opens the gap.
    let inner = slot.map(|(cover, inner)| {
        ui.painter().rect_filled(cover, 0.0, theme.bg);
        inner
    });
    (rect, inner)
}

/// egui 0.37 paints a dark fade at the edges of every scroll area that has
/// more content. On the page-tab strip it reads as a drop shadow, so turn
/// it off everywhere.
pub(crate) fn no_scroll_fade(ctx: &egui::Context) {
    ctx.global_style_mut(|s| s.spacing.scroll.fade.strength = 0.0);
}

fn bar_keys_req_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::NULL.with(("bar-keys-req", ctx.viewport_id()))
}

fn bar_keys_rect_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::NULL.with(("bar-keys-rect", ctx.viewport_id()))
}

/// Ask the next [`frame_with_slot`] to set aside `w` of its slot for page
/// keys, ending `reserve` from the slot's right edge (the E-STOP).
pub fn request_bar_keys(ctx: &egui::Context, w: f32, reserve: f32) {
    let id = bar_keys_req_id(ctx);
    ctx.data_mut(|d| d.insert_temp(id, (w, reserve)));
}

/// The page-key part of the top-bar slot for the frame being drawn.
pub fn bar_keys_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    let id = bar_keys_rect_id(ctx);
    ctx.data(|d| d.get_temp::<Option<egui::Rect>>(id)).flatten()
}

/// The three-dot more key at `rect` with `hidden` items behind it.
pub fn more_key(ui: &mut egui::Ui, rect: egui::Rect, salt: impl std::hash::Hash + std::fmt::Debug, hidden: usize) -> egui::Response {
    let id = ui.id().with(salt);
    let resp = ui.interact(rect, id, egui::Sense::click());
    paint_more_key(ui, rect, &resp);
    let resp = resp.on_hover_text(format!("More keys ({hidden} hidden)"));
    let l = format!("More keys, {hidden} hidden");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &l));
    resp
}

fn top_slot_cover_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::NULL.with(("top-slot-cover", ctx.viewport_id()))
}

/// The top elbow's end cap (top right of the page frame), as laid out by
/// [`frame_with_slot`] this frame.
pub fn top_end_cap(ctx: &egui::Context) -> Option<egui::Rect> {
    let cap_id = top_cap_id(ctx);
    ctx.data(|d| d.get_temp::<egui::Rect>(cap_id))
}

/// Where the "more tabs" button goes on the top bar: the end cap's width,
/// always immediately left of the bar slot (E-STOP / RELEASE, plus REC's
/// controls on REC), so the slot keeps its full size and the end cap stays
/// the Config detach pill. A frame with no slot puts it left of the end cap.
pub fn more_button_rect(cap: egui::Rect, cover: Option<egui::Rect>, gap: f32) -> egui::Rect {
    let right = match cover {
        Some(c) => c.left(),
        None => cap.left() - gap,
    };
    egui::Rect::from_min_max(egui::pos2(right - cap.width(), cap.top()), egui::pos2(right, cap.bottom()))
}

/// [`more_button_rect`] for this frame's page bar.
pub fn more_button_rect_now(ctx: &egui::Context) -> Option<egui::Rect> {
    let cover_id = top_slot_cover_id(ctx);
    let cover = ctx.data(|d| d.get_temp::<Option<egui::Rect>>(cover_id)).flatten();
    top_end_cap(ctx).map(|cap| more_button_rect(cap, cover, LcarsTheme::default().frame_gap))
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
/// Bar width kept for the left elbow, its tan run and the end cap when
/// sub-tabs share the top bar.
pub const BAR_TABS_RESERVE: f32 = 220.0;

/// Gap between sub-tabs in the top bar.
pub const BAR_TAB_GAP: f32 = 4.0;

/// Width of the more-tabs key in the top bar.
pub fn more_key_w() -> f32 {
    LcarsTheme::default().bar_thin * 1.2
}

/// Width all `labels` take as sub-tabs in the top bar, with gaps.
pub fn bar_tabs_width(ui: &egui::Ui, labels: &[String]) -> f32 {
    let w: f32 = labels.iter().map(|l| crate::folder_tab_width(ui, l, false)).sum();
    w + BAR_TAB_GAP * labels.len().saturating_sub(1) as f32
}

/// Sub-tabs inside the top elbow bar (`rect` is the black gap opened in the
/// bar, directly left of the E-STOP slot). Tabs run left to right. When they
/// do not all fit, the more-tabs key takes the right end of `rect` (so it
/// sits directly left of E-STOP) and the hidden tabs are in its menu; the
/// open tab always stays visible.
pub fn bar_tab_row(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: &str,
    labels: &[String],
    selected: Option<usize>,
) -> Option<usize> {
    let widths: Vec<f32> = labels.iter().map(|l| crate::folder_tab_width(ui, l, false)).collect();
    let all = bar_tabs_width(ui, labels);
    let overflow = all > rect.width() + 0.5;
    let room = if overflow { (rect.width() - more_key_w() - BAR_TAB_GAP).max(0.0) } else { rect.width() };
    let (shown, hidden) = partition_tabs(&widths, BAR_TAB_GAP, room, selected);
    let mut picked = None;
    let row = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + room, rect.bottom()));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(row)
            .id_salt(("bar-tabs", id_salt))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = BAR_TAB_GAP;
            for (n, &i) in shown.iter().enumerate() {
                // In the bar every tab is a plain segment: no left end cap.
                let _ = n;
                let resp = crate::folder_tab(ui, selected == Some(i), i, &labels[i], false);
                // The open tab is the light tint, as on the page tab strip
                // (rustcars IMG2 tabs); screen readers get it as selected.
                let (sel, name) = (selected == Some(i), labels[i].to_uppercase());
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, sel, &name));
                if resp.clicked() {
                    picked = Some(i);
                }
            }
        },
    );
    if !hidden.is_empty() {
        let key = egui::Rect::from_min_max(egui::pos2(rect.right() - more_key_w(), rect.top()), rect.max);
        let id = ui.id().with(("bar-more-tabs", id_salt));
        let resp = ui.interact(key, id, egui::Sense::click());
        paint_more_key(ui, key, &resp);
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
    }
    if picked.is_some() {
        ui.ctx().request_repaint();
    }
    picked
}

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
    // A button of its own on the top bar, left of the end cap (which is the
    // Config detach pill) and of the E-STOP slot.
    let Some(cap) = more_button_rect_now(ui.ctx()) else {
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

/// The "more tabs" button: a pill (round both ends) in the bar colour with a
/// highlight outline, like the detach pill, and three painted dots. No text;
/// the tooltip and accessibility label say "More tabs".
/// An elbow-bar button, by the rustcars rules (AGENTS.md §0 P1/P1b,
/// docs/manual/04-controls.md §4.4) and the REC bar (the reference):
/// a momentary key is a solid square frame segment (`lcars_button`,
/// `Cap::None`, no border, ink by `ink_on`) in its own hue; a key that
/// switches a state is a rustcars `toggle_button` (thin border, ●/○
/// marker). Each key brings its own palette hue.
pub fn bar_button(ui: &mut egui::Ui, label: &str, base: egui::Color32, size: egui::Vec2, on: Option<bool>, enabled: bool) -> egui::Response {
    let t = LcarsTheme::default();
    match on {
        None => rustcars::lcars_button(ui, &t, label, base, size, Cap::None, enabled),
        Some(state) => {
            ui.add_enabled_ui(enabled, |ui| {
                let mut local = state;
                // Square in the bar (rustcars P1c).
                rustcars::bar_toggle_button(ui, &t, label, &mut local, base, size)
            })
            .inner
        }
    }
}

fn paint_more_key(ui: &egui::Ui, rect: egui::Rect, resp: &egui::Response) {
    let theme = LcarsTheme::default();
    let painter = ui.painter();
    let color = rustcars::palette::TAN;
    let (fill, outline) = rustcars::group::pill_colors(color, false);
    let gap = theme.frame_gap;
    // Black gap on both sides so it reads as its own button on the bar.
    painter.rect_filled(rect.expand2(egui::vec2(gap, 0.0)), 0.0, theme.bg);
    // A plain segment in the bar (Cap::None), like the keys beside it.
    let r = egui::CornerRadius::ZERO;
    painter.rect_filled(rect, r, fill);
    let w = if resp.hovered() { rustcars::group::PILL_BORDER + 0.5 } else { rustcars::group::PILL_BORDER };
    painter.rect_stroke(rect, r, egui::Stroke::new(w, outline), egui::StrokeKind::Inside);
    if resp.has_focus() {
        painter.rect_stroke(rect.shrink(3.0), r, egui::Stroke::new(1.0, rustcars::ink_on(fill)), egui::StrokeKind::Inside);
    }
    let ink = rustcars::ink_on(fill);
    let d = (rect.height() * 0.1).clamp(2.0, 4.0);
    for k in [-1.0, 0.0, 1.0] {
        painter.circle_filled(egui::pos2(rect.center().x + k * d * 3.0, rect.center().y), d, ink);
    }
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

#[cfg(test)]
mod top_slot_tests {
    #[test]
    fn slot_sits_before_the_cap_with_gaps() {
        let outer = egui::Rect::from_min_size(egui::pos2(8.0, 50.0), egui::vec2(1264.0, 700.0));
        let (cover, inner) = super::top_slot(outer, 48.0, 40.0, 100.0);
        assert_eq!(cover.right(), outer.right() - 48.0);
        assert_eq!(inner.right(), cover.right() - 4.0);
        assert_eq!(inner.left(), cover.left() + 4.0);
        assert_eq!(inner.width(), 100.0 - 4.0);
        assert_eq!((inner.top(), inner.height()), (50.0, 40.0));
    }
}

#[cfg(test)]
mod top_cap_deadlock_tests {
    /// Regression: computing the cap id inside `data_mut` re-locked the egui
    /// context and hung the app on its first frame (b928260).
    #[test]
    fn frame_records_cap_without_deadlock() {
        let ctx = egui::Context::default();
        let id = super::top_cap_id(&ctx);
        ctx.data_mut(|d| d.insert_temp(id, egui::Rect::ZERO));
        let got = ctx.data(|d| d.get_temp::<egui::Rect>(id));
        assert_eq!(got, Some(egui::Rect::ZERO));
    }

    /// Headless smoke test: runs `frame` + `overflow_tab_row` (forced into
    /// overflow) through `Context::run_ui` for a few frames on a worker thread
    /// and fails if it doesn't finish in 10 s. Catches egui lock re-entry
    /// deadlocks like the one that hung the app before its first frame.
    /// Does not cover the full app `update` or the REC page.
    #[test]
    fn frame_and_overflow_row_run_headless_without_deadlock() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ctx = egui::Context::default();
            let labels: Vec<String> = (0..40).map(|i| format!("TAB NUMBER {i}")).collect();
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    {
                        super::frame(ui, |ui| {
                            super::overflow_tab_row(
                                ui,
                                "smoke",
                                &labels,
                                Some(0),
                                |_, _| 120.0,
                                |ui, i, sel| ui.selectable_label(sel, &labels[i]),
                            );
                        });
                    }
                });
                out.textures_delta.clear();
            }
            let _ = tx.send(());
        });
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("headless frame/overflow_tab_row did not finish in 10 s (deadlock?)");
    }

    /// Same watchdog for the opened top-bar gap with buttons in it, plus
    /// the overflow key on the end cap.
    #[test]
    fn frame_with_slot_runs_headless_without_deadlock() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ctx = egui::Context::default();
            let labels: Vec<String> = (0..40).map(|i| format!("TAB {i}")).collect();
            let mut got = None;
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 400.0))),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    let (_, slot) = super::frame_with_slot(ui, Some(300.0), |ui| {
                        super::overflow_tab_row(ui, "smoke2", &labels, Some(0), |_, _| 120.0, |ui, i, sel| {
                            ui.selectable_label(sel, &labels[i])
                        });
                    });
                    if let Some(r) = slot {
                        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                            ui.horizontal(|ui| {
                                let _ = ui.button("A");
                                let _ = ui.button("B");
                            });
                        });
                    }
                    got = slot;
                });
                out.textures_delta.clear();
            }
            let _ = tx.send(got);
        });
        let slot = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("headless frame_with_slot did not finish in 10 s (deadlock?)");
        assert!(slot.is_some());
    }
}

#[cfg(test)]
mod page_frame_tests {
    use super::*;

    /// Real LCARS fonts and theme: the Timers bar (COPY TIME, ID 10:00, the
    /// CLOCKS / WORLD / STOPWATCH / COUNTDOWN tabs, each one open in turn)
    /// and the Tools tabs paint no dark outline: momentary keys and tabs
    /// have no stroke; the mode button's border is its state colour.
    #[test]
    fn bar_buttons_have_no_black_borders() {
        let ctx = egui::Context::default();
        let mut tools = Tools::new(&ctx);
        let dark = |c: egui::Color32| c.a() > 0 && (c.r() as u16 + c.g() as u16 + c.b() as u16) < 3 * 48;
        let tl = timers::tab_labels();
        for sel in 0..tl.len() {
            let mut out = egui::FullOutput::default();
            for _ in 0..2 {
                out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    ui.horizontal(|ui| tools.timers_mut().bar(ui));
                    let r = egui::Rect::from_min_size(egui::pos2(300.0, 0.0), egui::vec2(700.0, 40.0));
                    bar_tab_row(ui, r, "timers", &tl, Some(sel));
                    let r = egui::Rect::from_min_size(egui::pos2(0.0, 100.0), egui::vec2(900.0, 40.0));
                    bar_tab_row(ui, r, "tools", &Tools::tab_labels(), Some(sel));
                });
                out.textures_delta.clear();
            }
            for c in &out.shapes {
                if let egui::Shape::Rect(r) = &c.shape {
                    assert!(!(r.stroke.width > 0.0 && dark(r.stroke.color)), "tab {sel}: dark border {r:?}");
                    // rustcars P1c: every bar button is square.
                    assert_eq!(r.corner_radius, egui::CornerRadius::ZERO, "tab {sel}: rounded bar button {r:?}");
                }
            }
        }
    }

    /// In the bar the tool tabs are plain segments: no end cap on the
    /// first (NANOVNA) or any tab, at wide and narrow widths.
    #[test]
    fn bar_tabs_have_no_end_caps() {
        let ctx = egui::Context::default();
        let labels = Tools::tab_labels();
        for w in [700.0, 300.0] {
            let mut out = egui::FullOutput::default();
            for _ in 0..2 {
                out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let r = egui::Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(w, 40.0));
                    bar_tab_row(ui, r, "t", &labels, Some(0));
                });
                out.textures_delta.clear();
            }
            for c in &out.shapes {
                if let egui::Shape::Rect(r) = &c.shape {
                    assert_eq!(r.corner_radius, egui::CornerRadius::ZERO, "no rounded tab or more key in the bar at {w}: {r:?}");
                }
            }
        }
    }

    /// Bar buttons follow rustcars: momentary = square solid segment with
    /// no outline, toggle = rustcars toggle (thin border, ●/○ marker).
    #[test]
    fn bar_buttons_follow_rustcars() {
        let ctx = egui::Context::default();
        for on in [None, Some(false), Some(true)] {
            let mut out = egui::FullOutput::default();
            for _ in 0..2 {
                out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    bar_button(ui, "Load", rustcars::palette::SLATE, egui::vec2(80.0, 40.0), on, true);
                });
                out.textures_delta.clear();
            }
            let rects: Vec<_> = out.shapes.iter().filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) => Some(r.clone()),
                _ => None,
            }).collect();
            let body = rects.iter().find(|r| r.fill != egui::Color32::TRANSPARENT).expect("filled");
            match on {
                None => {
                    assert_eq!(body.corner_radius, egui::CornerRadius::ZERO, "momentary bar key is square");
                    assert!(rects.iter().all(|r| r.stroke.width == 0.0), "momentary: no outline");
                }
                Some(_) => {
                    assert!(rects.iter().any(|r| (r.stroke.width - rustcars::TOGGLE_BORDER).abs() < 0.01), "toggle: thin border");
                }
            }
        }
    }

    fn texts(out: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                // What is painted: the text clipped to its clip rect.
                egui::Shape::Text(t) => {
                    let r = t.visual_bounding_rect().intersect(c.clip_rect);
                    r.is_positive().then(|| (t.galley.text().to_owned(), r))
                }
                _ => None,
            })
            .collect()
    }

    /// One main-window frame as main.rs draws Tools: the sub-tabs in the
    /// top-bar gap directly left of the E-STOP, the screen in the frame.
    /// Returns (output, frame rect, tab gap rect, E-STOP rect).
    fn tools_frame(ctx: &egui::Context, tools: &mut Tools, w: f32) -> (egui::FullOutput, egui::Rect, egui::Rect, egui::Rect) {
        let (mut frame, mut tabs_r, mut stop_r) = (egui::Rect::NOTHING, egui::Rect::NOTHING, egui::Rect::NOTHING);
        let mut out = egui::FullOutput::default();
        for _ in 0..3 {
            out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, 800.0))),
                    ..Default::default()
                },
                |ui| {
                    let estop_w = crate::estop::button_width(ui, false) + 16.0;
                    let half = ((ui.available_width() - BAR_TABS_RESERVE - estop_w) * 0.5).max(0.0);
                    let kw = tools.header_width().min(half);
                    request_bar_keys(ui.ctx(), kw, estop_w + BAR_TAB_GAP);
                    let keys_w = if kw > 0.0 { kw + BAR_TAB_GAP } else { 0.0 };
                    let room = (ui.available_width() - BAR_TABS_RESERVE - estop_w - keys_w).max(more_key_w() + 80.0);
                    let tabs_w = bar_tabs_width(ui, &Tools::tab_labels()).min(room) + 2.0 * BAR_TAB_GAP;
                    let (f, slot) = frame_with_slot(ui, Some(tabs_w + keys_w + estop_w), |ui| tools.draw_in(ui));
                    let r = slot.expect("slot");
                    tabs_r = egui::Rect::from_min_max(r.min, egui::pos2(r.right() - estop_w - BAR_TAB_GAP - keys_w, r.bottom()));
                    stop_r = egui::Rect::from_min_max(egui::pos2(r.right() - estop_w, r.top()), r.max);
                    tools.bar_tabs(ui, tabs_r);
                    frame = f;
                },
            );
            out.textures_delta.clear();
        }
        (out, frame, tabs_r, stop_r)
    }

    /// Real LCARS fonts and theme. The tool sub-tabs are inside the top bar,
    /// left of the E-STOP slot, never overlapping it; nothing else of the
    /// page sits in the bar; a narrow window puts the rest into more-tabs
    /// at the right end of the gap. The status line is one row.
    #[test]
    fn tool_tabs_sit_in_the_bar_left_of_the_estop() {
        let ctx = egui::Context::default();
        let mut tools = Tools::new(&ctx);
        let th = LcarsTheme::default();
        for w in [1400.0, 760.0] {
            for (screen, label) in SCREENS {
                tools.screen = screen;
                let (out, frame, tabs_r, stop_r) = tools_frame(&ctx, &mut tools, w);
                let t = texts(&out);
                assert!(tabs_r.top() == frame.top() && tabs_r.bottom() <= frame.top() + th.bar_thin + 0.5, "gap is in the top bar");
                assert!(tabs_r.right() <= stop_r.left(), "tabs end left of the E-STOP");
                let in_bar: Vec<_> = t.iter().filter(|(_, r)| r.top() < frame.top() + th.bar_thin).collect();
                for (s, r) in &in_bar {
                    // Tool keys (and Rotator LEDs) sit in their own rect
                    // between the tabs and the E-STOP.
                    let keys = bar_keys_rect(&ctx).unwrap_or(egui::Rect::NOTHING);
                    assert!(keys.right() <= stop_r.left() + 0.5, "keys left of the E-STOP");
                    assert!(
                        tabs_r.expand(1.0).contains_rect(*r) || keys.expand(1.0).contains_rect(*r),
                        "{label} at {w}: {s:?} {r:?} in the bar outside the tab gap {tabs_r:?} and keys {keys:?}"
                    );
                }
                assert!(in_bar.iter().any(|(s, _)| s.eq_ignore_ascii_case(label)), "{label} at {w}: open tab shown in the bar");
                if w >= 1400.0 && tools.header_width() == 0.0 {
                    for (_, l) in SCREENS {
                        assert!(in_bar.iter().any(|(s, _)| s.eq_ignore_ascii_case(l)), "tab {l} in the bar at {w}");
                    }
                } else if w < 1400.0 {
                    assert!(SCREENS.iter().any(|(_, l)| !in_bar.iter().any(|(s, _)| s.eq_ignore_ascii_case(l))), "narrow: some tabs hidden");
                }
                // Below the bar: the page content, inside the frame.
                let inner = egui::Rect::from_min_max(frame.min + egui::vec2(th.bar_thick, th.bar_thin), frame.max);
                let body: Vec<_> = t.iter().filter(|(_, r)| r.top() >= frame.top() + th.bar_thin).collect();
                for (s, r) in &body {
                    assert!(inner.expand(1.0).contains_rect(*r), "{label} at {w}: {s:?} {r:?} outside frame {inner:?}");
                }
                // The status strip: the last body row, one row, at the
                // frame's bottom (NanoVNA style).
                let bottom = body.iter().map(|(_, r)| r.bottom()).fold(f32::NEG_INFINITY, f32::max);
                let last: Vec<_> = body.iter().filter(|(_, r)| (r.bottom() - bottom).abs() < 1.0).collect();
                let base = tools.status_line();
                let head: String = base.chars().take(8).collect();
                if screen == Screen::NanoVna {
                    let n = t.iter().filter(|(s, _)| s.contains("NOT CONNECTED")).count();
                    assert!(n <= 1, "NanoVNA: connection state shown once, got {n}");
                    assert!(!t.iter().any(|(s, _)| s.starts_with(&head) && s.len() >= base.len().min(20)), "NanoVNA: no extra status line");
                    continue;
                }
                assert!(last.iter().any(|(s, _)| s.starts_with(&head)), "{label}: status strip last, got {last:?}");
                // Just above the bottom elbow (bar plus its gap).
                assert!(frame.bottom() - bottom < LcarsTheme::default().bar_thin + 24.0 + crate::status_note::STRIP_H, "{label} at {w}: strip at the frame bottom ({bottom}) frame {frame:?} last {last:?}");
                assert!(last.iter().all(|(_, r)| r.height() < crate::status_note::STRIP_H), "{label}: one row");
            }
        }
        tools.conceal_nanovna();
    }

    /// A detached Tools window puts the page in a vertical scroll area
    /// (unbounded height). Every screen, the rotator dial included, still
    /// draws inside the visible window, and the status strip is visible.
    /// Real fonts/theme, window sizes 720x480 and 1200x800.
    #[test]
    fn detached_window_screens_draw_in_view() {
        let ctx = egui::Context::default();
        let mut tools = Tools::new(&ctx);
        for size in [egui::vec2(720.0, 480.0), egui::vec2(1200.0, 800.0), egui::vec2(500.0, 700.0)] {
            for (screen, label) in SCREENS {
                tools.screen = screen;
                let mut out = egui::FullOutput::default();
                for _ in 0..3 {
                    out = ctx.run_ui(
                        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() },
                        |ui| {
                            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| tools.window_body(ui));
                        },
                    );
                    out.textures_delta.clear();
                }
                let view = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                let painted: Vec<egui::Rect> = out.shapes.iter().filter_map(|c| {
                    let r = c.shape.visual_bounding_rect().intersect(c.clip_rect);
                    r.is_positive().then_some(r)
                }).collect();
                assert!(!painted.is_empty(), "{label} at {size:?}: nothing drawn");
                if screen == Screen::Rotator {
                    // The dial: large shapes in the visible window.
                    let big = painted.iter().filter(|r| view.contains_rect(**r) && r.width() > size.x.min(size.y) * 0.3).count();
                    assert!(big > 0, "{label} at {size:?}: no dial in view");
                }
                if screen != Screen::NanoVna {
                    let strip = out.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r)
                        if r.fill == rustcars::palette::STRIP && view.contains_rect(r.rect) && (r.rect.height() - crate::status_note::STRIP_H).abs() < 0.5));
                    assert!(strip, "{label} at {size:?}: status strip not in view");
                }
            }
        }
        tools.conceal_nanovna();
    }

    /// Rotator LEDs in the elbow bar: at the bar height, inside the bar gap
    /// left of E-STOP, both when wide, AZ only or none (then on the page)
    /// when the bar is short. Real fonts/theme, through the main layout.
    #[test]
    fn rotator_leds_sit_in_the_bar() {
        assert_eq!(rotator::leds_fit(2.0 * rotator::BAR_LED_W + BAR_TAB_GAP), 2);
        assert_eq!(rotator::leds_fit(rotator::BAR_LED_W + 10.0), 1);
        assert_eq!(rotator::leds_fit(60.0), 0);
        let ctx = egui::Context::default();
        let mut tools = Tools::new(&ctx);
        tools.screen = Screen::Rotator;
        let th = LcarsTheme::default();
        let (out, frame, _, stop_r) = tools_frame(&ctx, &mut tools, 1600.0);
        let glass = egui::Color32::from_rgb(8, 8, 6);
        let insets: Vec<egui::Rect> = out.shapes.iter().filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == glass && r.rect.top() < frame.top() + th.bar_thin => Some(r.rect),
            _ => None,
        }).collect();
        assert_eq!(insets.len(), 2, "AZ and EL in the bar at 1600: {insets:?}");
        for r in &insets {
            assert!(r.bottom() <= frame.top() + th.bar_thin + 0.5 && r.right() <= stop_r.left(), "{r:?}");
            assert!(r.height() >= th.bar_thin - 8.0, "bar height");
        }
        tools.conceal_nanovna();
    }

    /// Rotator: the dial beside the controls when wide, on top with the
    /// controls below when narrow; the dial is never squeezed away.
    #[test]
    fn rotator_dial_beside_when_wide_and_on_top_when_narrow() {
        let wide = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0));
        let (d, p) = rotator::Rotator::layout_for(wide);
        assert!(p.left() > d.right() && d.width() >= 320.0 && d.height() == 700.0);
        for w in [600.0, 420.0] {
            let narrow = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, 700.0));
            let (d, p) = rotator::Rotator::layout_for(narrow);
            assert!(p.top() >= d.bottom() && d.width() == w && d.height() >= 160.0, "{w}: {d:?} {p:?}");
            assert!(p.height() > 100.0, "controls get room below");
        }
    }

    /// Every tool screen (real fonts/theme): no right-hand elbow (no PLUM
    /// chrome on the frame's right edge below the bar), and the content
    /// reaches the right edge; the status strip is NanoVNA's (STRIP fill at
    /// the bottom) on every tool but NanoVNA, which has its own.
    #[test]
    fn no_tool_screen_has_a_right_elbow() {
        let ctx = egui::Context::default();
        let mut tools = Tools::new(&ctx);
        let th = LcarsTheme::default();
        for (screen, label) in SCREENS {
            tools.screen = screen;
            let (out, frame, _, _) = tools_frame(&ctx, &mut tools, 1400.0);
            let edge = frame.right() - 2.0;
            let below = frame.top() + th.bar_thin + 1.0;
            let elbow = out.shapes.iter().any(|c| match &c.shape {
                egui::Shape::Rect(r) => r.fill == rustcars::palette::PLUM && r.rect.right() >= edge && r.rect.top() > below,
                egui::Shape::Path(p) => p.fill == rustcars::palette::PLUM && p.points.iter().any(|q| q.x >= edge && q.y > below),
                _ => false,
            });
            assert!(!elbow, "{label}: right-hand elbow drawn");
            if screen != Screen::NanoVna {
                let strip = out.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r)
                    if r.fill == rustcars::palette::STRIP && r.rect.bottom() >= frame.bottom() - th.bar_thin - 24.0
                        && (r.rect.height() - crate::status_note::STRIP_H).abs() < 0.5));
                let strips: Vec<_> = out.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Rect(r) if r.fill == rustcars::palette::STRIP => Some(r.rect), _ => None }).collect();
                assert!(strip, "{label}: status strip at the bottom; frame {frame:?} strips {strips:?}");
            }
        }
        tools.conceal_nanovna();
    }

    /// NanoVNA: the display has no right-hand elbow and runs to the bottom
    /// of the frame; the status text does not move it.
    #[test]
    fn nanovna_display_has_no_right_elbow_and_fills_the_height() {
        let ctx = egui::Context::default();
        let mut tools = Tools::new(&ctx);
        tools.screen = Screen::NanoVna;
        let (out, frame, _, _) = tools_frame(&ctx, &mut tools, 1400.0);
        let grid = tools.nanovna.last_grid();
        assert!(grid.is_positive(), "chart drawn");
        let plum = rustcars::palette::PLUM;
        let _ = plum;
        // An elbow hugs the frame's right edge below the top bar.
        let edge = frame.right() - 2.0;
        let below = frame.top() + LcarsTheme::default().bar_thin + 1.0;
        let right_elbow: Vec<_> = out.shapes.iter().filter(|c| match &c.shape {
            egui::Shape::Rect(r) => r.fill == plum && r.rect.right() >= edge && r.rect.top() > below,
            egui::Shape::Path(p) => p.fill == plum && p.points.iter().any(|q| q.x >= edge && q.y > below),
            _ => false,
        }).collect();
        let right_elbow = !right_elbow.is_empty();
        assert!(!right_elbow, "no right-hand elbow around the display");
        assert!(grid.right() > frame.right() - 40.0, "display runs to the right edge: {grid:?} in {frame:?}");
        // Fixed parts under the grid: band strip, status strip, NanoVNA status bar.
        assert!(frame.bottom() - grid.bottom() < 120.0, "display fills the height: {grid:?} in {frame:?}");
        // Stable: redrawing does not move the chart.
        let _ = tools_frame(&ctx, &mut tools, 1400.0);
        assert_eq!(tools.nanovna.last_grid(), grid, "chart is stable frame to frame");
        tools.conceal_nanovna();
    }

    #[test]
    fn status_lines_name_each_tool() {
        let ctx = egui::Context::default();
        let mut tools = Tools::new(&ctx);
        for (screen, want) in [
            (Screen::NanoVna, "NANOVNA NOT CONNECTED"),
            (Screen::HamRf, "RF CALC"),
            (Screen::Calculator, "CALC RAD"),
            (Screen::Smith, "SMITH CHART"),
            (Screen::Hex, "HEX"),
            (Screen::Log, "NO LOG OPEN"),
            (Screen::Rotator, "NO ROTATOR"),
        ] {
            tools.screen = screen;
            let line = tools.status_line();
            assert!(line.starts_with(want), "{line:?} should start with {want:?}");
        }
    }
}
