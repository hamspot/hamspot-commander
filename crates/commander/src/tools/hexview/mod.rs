//! Hex viewer built from rustcars widgets.
//! Run: `cargo run --example hexview [-- FILE]` (desktop only; no file = a built-in sample, the
//! Antonio font). Files over 16 MiB are cut at 16 MiB and the title says so.
//!
//! Rows are a virtualised `mode_list`. The click modes are three ways of *marking* a row
//! (AGENTS.md P2):
//! * click   = inspect: the INSPECTOR on the right reads the bytes at that row.
//! * double  = bookmark: the row stays orange and is listed under BOOKMARKS for a quick jump.
//! * triple  = copy: the row's hex bytes go to the clipboard and the row stays purple.
//!
//! Also: GO TO an offset (`496` or `0x1F0`), FIND hex bytes (`DE AD`) or `"text"`.
//! The logic is in `hex.rs` and is unit tested.
mod hex;

use hex::ROW;
use rustcars::{
    LcarsFrame, LcarsTheme, Mode, Side, action_button, apply_theme, install_lcars_font,
    mode_list, palette, readout,
};
use std::collections::BTreeMap;

const LIMIT: usize = 16 * 1024 * 1024;
const ROW_H: f32 = super::LIST_ROW;
const GAP: f32 = 2.0;

pub struct HexView {
    pub theme: LcarsTheme,
    pub name: String,
    pub data: Vec<u8>,
    pub truncated: bool,
    /// Row index -> mode, for rows that are not inactive.
    pub modes: BTreeMap<usize, Mode>,
    pub selected: Option<usize>,
    pub goto: String,
    pub find: String,
    pub scroll_to: Option<usize>,
    pub status: String,
    open_request: bool,
}

impl HexView {
    pub fn new(ctx: &egui::Context, name: String, data: Vec<u8>, truncated: bool) -> Self {
        let theme = LcarsTheme::default();
        install_lcars_font(ctx);
        apply_theme(ctx, &theme);
        Self {
            theme,
            name,
            data,
            truncated,
            modes: BTreeMap::new(),
            selected: None,
            goto: String::new(),
            find: String::new(),
            scroll_to: None,
            status: "READY".into(),
            open_request: false,
        }
    }

    pub fn take_open(&mut self) -> bool {
        let open = self.open_request;
        self.open_request = false;
        open
    }

    /// Replace the buffer with a file. Files over 16 MiB are cut, and the title says so.
    pub fn load_file(&mut self, path: &std::path::Path) -> Result<(), String> {
        let (data, truncated) = hex::load(path, LIMIT).map_err(|err| err.to_string())?;
        self.name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        if truncated {
            self.name.push_str(" (cut at 16 MiB)");
        }
        self.data = data;
        self.truncated = truncated;
        self.modes.clear();
        self.selected = None;
        self.scroll_to = Some(0);
        self.status = format!("OPENED {}", self.name);
        Ok(())
    }

    fn row_bytes(&self, row: usize) -> &[u8] {
        let s = (row * ROW).min(self.data.len());
        &self.data[s..(s + ROW).min(self.data.len())]
    }

    fn bookmarks(&self) -> Vec<usize> {
        self.modes
            .iter()
            .filter(|(_, m)| **m == Mode::Alt1)
            .map(|(r, _)| *r)
            .collect()
    }

    /// Select the byte at `off`: marks its row (click mode) and scrolls to it.
    pub fn select(&mut self, off: usize) {
        let row = off / ROW;
        self.modes.retain(|_, m| *m != Mode::Primary);
        self.modes.insert(row, Mode::Primary);
        self.selected = Some(off);
        self.scroll_to = Some(row);
    }

    fn do_goto(&mut self) {
        self.status = match hex::parse_offset(&self.goto) {
            Ok(o) if o < self.data.len() => {
                self.select(o);
                format!("AT 0x{o:06X}")
            }
            Ok(o) => format!("ERROR: 0x{o:X} is past the end (0x{:X})", self.data.len()),
            Err(e) => format!("ERROR: {e}"),
        };
    }

    fn do_find(&mut self) {
        self.status = match hex::parse_pattern(&self.find) {
            Ok(p) => {
                let from = self.selected.map_or(0, |s| s + 1);
                match hex::find(&self.data, &p, from) {
                    Some(o) => {
                        self.select(o);
                        format!("FOUND AT 0x{o:06X}")
                    }
                    None => "NOT FOUND".into(),
                }
            }
            Err(e) => format!("ERROR: {e}"),
        };
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        super::screen_header(ui, "hex_header", &t, |ui| {
                ui.horizontal(|ui| {
                    super::title_block(ui, &t, "Hex", palette::TAN, egui::vec2(110.0, t.bar_thin));
                    if super::header_key(ui, &t, "Open", egui::vec2(96.0, t.bar_thin)).clicked() {
                        self.open_request = true;
                    }
                });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.bg))
            .show(ui, |ui| {
                let full = ui.available_rect_before_wrap();
                let split = full.left() + 830.0;
                let left = egui::Rect::from_min_max(full.min, egui::pos2(split, full.bottom()));
                let right =
                    egui::Rect::from_min_max(egui::pos2(split + t.main_gap, full.top()), full.max);
                ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                    let mut f = LcarsFrame::new(&t, Side::Left, 0.6);
                    f.min_height = left.height();
                    f.show(ui, &t, |ui| self.rows_panel(ui));
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
                    // No right-hand elbow (as NanoVNA): the panel uses the whole rect.
                    ui.set_min_height(right.height());
                    (|ui: &mut egui::Ui| self.side_panel(ui))(ui);
                });
            });
    }

    fn rows_panel(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let rows = hex::row_count(self.data.len());
        let cut = if self.truncated {
            " (CUT AT 16 MIB)"
        } else {
            ""
        };
        ui.label(
            egui::RichText::new(format!(
                "{}: {} BYTES{cut}",
                self.name.to_uppercase(),
                self.data.len()
            ))
            .size(t.header_size),
        );
        crate::status_note::note(ui, "ROWS: CLICK INSPECTS · DOUBLE BOOKMARKS · TRIPLE COPIES");
        // show_rows measures a row as height + item spacing: keep it equal to mode_list's gap
        ui.spacing_mut().item_spacing.y = GAP;
        let mut area = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height() - 4.0);
        if let Some(r) = self.scroll_to.take() {
            area = area.vertical_scroll_offset((r.saturating_sub(2)) as f32 * (ROW_H + GAP));
        }
        let out = area.show_rows(ui, ROW_H, rows, |ui, range| {
            let mut list: Vec<(String, Mode)> = range
                .clone()
                .map(|r| {
                    (
                        hex::format_row(r * ROW, self.row_bytes(r)),
                        self.modes.get(&r).copied().unwrap_or_default(),
                    )
                })
                .collect();
            mode_list(ui, &t, "hex_rows", &mut list, ROW_H);
            (range, list)
        });
        let (range, list) = out.inner;
        for (k, (_, m)) in list.iter().enumerate() {
            let row = range.start + k;
            let old = self.modes.get(&row).copied().unwrap_or_default();
            if *m == old {
                continue;
            }
            match m {
                Mode::Inactive => {
                    self.modes.remove(&row);
                    if self.selected.map(|s| s / ROW) == Some(row) {
                        self.selected = None;
                    }
                }
                _ => {
                    // inspect and copy keep one row each; bookmarks accumulate
                    if matches!(m, Mode::Primary | Mode::Alt2) {
                        self.modes.retain(|_, x| x != m);
                    }
                    self.modes.insert(row, *m);
                }
            }
            match m {
                Mode::Primary => self.selected = Some(row * ROW),
                Mode::Alt1 => self.status = format!("BOOKMARKED 0x{:06X}", row * ROW),
                Mode::Alt2 => {
                    let text = hex::row_hex(self.row_bytes(row));
                    ui.ctx().copy_text(text.clone());
                    self.status = format!("COPIED: {text}");
                }
                Mode::Inactive => {}
            }
        }
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        ui.label(egui::RichText::new("GO TO").size(t.header_size));
        let r = ui.add(
            egui::TextEdit::singleline(&mut self.goto)
                .hint_text("496 or 0x1F0")
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.do_goto();
        }
        ui.label(egui::RichText::new("FIND").size(t.header_size));
        let r = ui.add(
            egui::TextEdit::singleline(&mut self.find)
                .hint_text("DE AD BE EF or \"RIFF\"")
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            let b = egui::vec2(110.0, 36.0);
            if action_button(ui, &t, "Go to", b).clicked() {
                self.do_goto();
            }
            if action_button(ui, &t, "Find next", b).clicked() || enter {
                self.do_find();
            }
        });
        crate::status_note::note(ui, self.status.clone());
        ui.add_space(t.main_gap / 2.0);
        ui.label(egui::RichText::new("INSPECTOR").size(t.header_size));
        match self.selected {
            Some(off) => {
                readout(ui, &t, &format!("OFFSET 0x{off:06X} ({off})"), true);
                egui::Grid::new("inspect")
                    .spacing(egui::vec2(t.main_gap, t.frame_gap))
                    .show(ui, |ui| {
                        for (k, v) in hex::inspect(&self.data, off) {
                            ui.label(egui::RichText::new(k).color(palette::TEXT_MUTED));
                            readout(ui, &t, &v, false);
                            ui.end_row();
                        }
                    });
            }
            None => {}
        }
        ui.add_space(t.main_gap / 2.0);
        ui.label(egui::RichText::new("BOOKMARKS").size(t.header_size));
        let marks = self.bookmarks();

        ui.horizontal_wrapped(|ui| {
            for row in marks.into_iter().take(12) {
                let label = format!("{:06X}", row * ROW);
                if action_button(ui, &t, &label, egui::vec2(96.0, 32.0)).clicked() {
                    self.scroll_to = Some(row);
                }
            }
        });
    }
}


#[cfg(test)]
mod logic {
    use super::*;

    #[test]
    fn goto_find_and_marks() {
        let ctx = egui::Context::default();
        let mut a = HexView::new(
            &ctx,
            "t".into(),
            (0..=255u8).cycle().take(1000).collect(),
            false,
        );
        a.goto = "0x1F0".into();
        a.do_goto();
        assert_eq!(a.selected, Some(496));
        assert_eq!(a.modes.get(&31), Some(&Mode::Primary));
        assert_eq!(a.scroll_to, Some(31));
        a.goto = "5000".into();
        a.do_goto();
        assert!(a.status.starts_with("ERROR"));
        assert_eq!(a.selected, Some(496), "a failed go to keeps the selection");
        // one Primary row at a time
        a.select(16);
        assert_eq!(a.modes.values().filter(|m| **m == Mode::Primary).count(), 1);
        // find starts after the selection, then wraps
        a.find = "0A 0B".into();
        a.do_find();
        assert_eq!(a.selected, Some(266), "first match after the selection");
        a.do_find();
        assert_eq!(a.selected, Some(522), "next match, 256 bytes later");
        a.do_find();
        assert_eq!(a.selected, Some(778));
        a.do_find();
        assert_eq!(a.selected, Some(10), "wraps to the start");
        a.find = "\"zzz\"".into();
        a.do_find();
        assert_eq!(a.status, "NOT FOUND");
        a.modes.insert(2, Mode::Alt1);
        a.modes.insert(9, Mode::Alt1);
        assert_eq!(a.bookmarks(), vec![2, 9]);
        assert_eq!(
            a.row_bytes(62).len(),
            8,
            "short last row (1000 = 62 rows + 8)"
        );
    }
}
