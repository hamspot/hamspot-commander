//! Logbook. The screen is the rustcars diary example (MIT), stored in SQLite
//! beside the open station as `Name.HSLog`.
//!
//! * The NEW ENTRY field is focused unless another field has focus.
//!   Enter saves the entry with the current time (stored UTC, shown local).
//! * `Log` stamps the live dial onto the next new entry and focuses the text.
//!   Saving still waits for the operator.
//! * Entry rows use click modes: click = select, double = edit, triple = quote.
//! * Delete asks for confirmation, and the last delete can be undone.
mod store;

use rustcars::{
    Cap, LcarsFrame, LcarsTheme, Mode, Side, action_button, apply_theme, install_lcars_font,
    lcars_button, mode_list, palette,
};
use std::collections::HashMap;
use store::{Entry, Stamp, Store};

pub struct Diary {
    pub theme: LcarsTheme,
    pub store: Store,
    pub input: String,
    pub search: String,
    pub date: String,
    pub modes: HashMap<i64, Mode>,
    pub editing: Option<i64>,
    pub confirm_delete: Option<i64>,
    pub undo: Option<Entry>,
    pub status: String,
    pub dial_hz: f64,
    pub dial_mode: String,
    pub dial_radio: String,
    stamp: Option<Stamp>,
    focus_input: bool,
}

impl Diary {
    pub fn new(ctx: &egui::Context, store: Store) -> Self {
        let theme = LcarsTheme::default();
        install_lcars_font(ctx);
        apply_theme(ctx, &theme);
        Self {
            theme,
            store,
            input: String::new(),
            search: String::new(),
            date: String::new(),
            modes: HashMap::new(),
            editing: None,
            confirm_delete: None,
            undo: None,
            status: "READY".into(),
            dial_hz: 0.0,
            dial_mode: String::new(),
            dial_radio: String::new(),
            stamp: None,
            focus_input: false,
        }
    }

    /// Stamp the dial the shell last reported, and focus the entry field.
    fn arm_from_dial(&mut self) {
        let mode = if self.dial_mode.is_empty() {
            String::new()
        } else {
            self.dial_mode.clone()
        };
        self.stamp = Some(Stamp {
            hz: self.dial_hz,
            mode,
            radio: self.dial_radio.clone(),
        });
        self.focus_input = true;
        let mhz = self.dial_hz / 1_000_000.0;
        self.status = format!(
            "LOG {mhz:.6} MHz {} {}",
            self.dial_mode, self.dial_radio
        );
    }

    fn selected(&self) -> Option<i64> {
        self.modes
            .iter()
            .find(|(_, m)| **m == Mode::Primary)
            .map(|(id, _)| *id)
    }

    fn save(&mut self) {
        let text = self.input.trim().to_owned();
        if text.is_empty() {
            return;
        }
        let r = match self.editing.take() {
            Some(id) => self
                .store
                .update(id, &text)
                .map(|_| "ENTRY UPDATED".to_owned()),
            None => {
                let stamp = self.stamp.take();
                self.store
                    .add(&text, &Store::now_utc(), stamp.as_ref())
                    .map(|e| format!("SAVED {}", e.local()))
            }
        };
        self.status = r.unwrap_or_else(|e| format!("ERROR: {e}"));
        self.input.clear();
        self.modes.clear();
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        super::screen_header(ui, "diary_header", &t, |ui| {
                ui.horizontal(|ui| {
                    super::title_block(ui, &t, "Logbook", palette::TAN, egui::vec2(110.0, t.bar_thin));
                    let seg = egui::vec2(90.0, t.bar_thin);
                    if lcars_button(ui, &t, "Today", palette::TEAL, seg, Cap::None, true).clicked()
                    {
                        self.date = chrono::Local::now().format("%Y-%m-%d").to_string();
                    }
                    if lcars_button(ui, &t, "All", palette::SLATE, seg, Cap::None, true).clicked() {
                        self.date.clear();
                        self.search.clear();
                    }
                });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.bg))
            .show(ui, |ui| {
                let full = ui.available_rect_before_wrap();
                let split = full.left() + full.width() * 0.45;
                let left = egui::Rect::from_min_max(full.min, egui::pos2(split, full.bottom()));
                let right =
                    egui::Rect::from_min_max(egui::pos2(split + t.main_gap, full.top()), full.max);
                ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                    let mut f = LcarsFrame::new(&t, Side::Left, 0.6);
                    f.min_height = left.height();
                    f.show(ui, &t, |ui| self.left_panel(ui));
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
                    let mut f = LcarsFrame::new(&t, Side::Right, 0.6);
                    f.top_color = palette::PLUM;
                    f.bottom_color = palette::OLIVE;
                    f.min_height = right.height();
                    f.show(ui, &t, |ui| self.entries_panel(ui));
                });
            });
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let title = if self.editing.is_some() {
            "EDIT ENTRY"
        } else {
            "NEW ENTRY"
        };
        ui.label(egui::RichText::new(title).size(t.header_size));
        let input_id = ui.make_persistent_id("diary_input");
        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.input)
                .id(input_id)
                .hint_text("Type and press Enter")
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.save();
        }
        if self.focus_input {
            resp.request_focus();
            self.focus_input = false;
        } else if ui.ctx().memory(|m| m.focused().is_none()) {
            resp.request_focus();
        }
        crate::status_note::note(ui, self.status.clone());
        let b = egui::vec2(160.0, 40.0);
        ui.horizontal(|ui| {
            if action_button(ui, &t, "Log", b).clicked() {
                self.arm_from_dial();
            }
            if self.editing.is_some() && action_button(ui, &t, "Cancel edit", b).clicked() {
                self.editing = None;
                self.input.clear();
                self.modes.clear();
            }
            if let Some(id) = self.selected() {
                if action_button(ui, &t, "Edit", b).clicked() {
                    self.modes.insert(id, Mode::Alt1);
                }
                if action_button(ui, &t, "Delete", b).clicked() {
                    self.confirm_delete = Some(id);
                }
            }
        });
        if let Some(id) = self.confirm_delete {
            ui.label(
                egui::RichText::new("DELETE THIS ENTRY?")
                    .size(t.header_size)
                    .color(Mode::Alt1.colors().label),
            );
            ui.horizontal(|ui| {
                if action_button(ui, &t, "Confirm delete", b).clicked() {
                    match self.store.delete(id) {
                        Ok(e) => {
                            self.undo = e;
                            self.status = "ENTRY DELETED".into();
                        }
                        Err(e) => self.status = format!("ERROR: {e}"),
                    }
                    self.modes.clear();
                    self.confirm_delete = None;
                }
                if action_button(ui, &t, "Cancel", b).clicked() {
                    self.confirm_delete = None;
                }
            });
        }
        if self.undo.is_some() && action_button(ui, &t, "Undo delete", b).clicked() {
            if let Some(e) = self.undo.take() {
                self.status = match self.store.restore(&e) {
                    Ok(()) => "DELETE UNDONE".into(),
                    Err(e) => format!("ERROR: {e}"),
                };
            }
        }
        ui.add_space(t.main_gap);
        ui.label(egui::RichText::new("FILTER").size(t.header_size));
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Text contains")
                .desired_width(f32::INFINITY),
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.date)
                .hint_text("Date YYYY-MM-DD (prefix)")
                .desired_width(f32::INFINITY),
        );
    }

    fn entries_panel(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let entries = self
            .store
            .list(&self.search, &self.date)
            .unwrap_or_default();
        ui.label(egui::RichText::new(format!("ENTRIES ({})", entries.len())).size(t.header_size));
        let mut rows: Vec<(String, Mode)> = entries
            .iter()
            .map(|e| {
                let mut text = e.text.replace('\n', " ");
                if text.chars().count() > 40 {
                    text = text.chars().take(39).collect::<String>() + "…";
                }
                (
                    format!("{}  {text}", entry_line(e)),
                    self.modes.get(&e.id).copied().unwrap_or_default(),
                )
            })
            .collect();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height())
            .show(ui, |ui| {
                mode_list(ui, &t, "entries", &mut rows, super::LIST_ROW);
            });
        for (e, (_, m)) in entries.iter().zip(rows) {
            let old = self.modes.get(&e.id).copied().unwrap_or_default();
            if m == old {
                continue;
            }
            self.modes.clear(); // one active row at a time
            if m != Mode::Inactive {
                self.modes.insert(e.id, m);
            }
            match m {
                Mode::Alt1 => {
                    self.editing = Some(e.id);
                    self.input = e.text.clone();
                }
                Mode::Alt2 => {
                    self.input = format!("Re {}: {}", e.local(), e.text);
                    self.editing = None;
                }
                _ => {
                    if self.editing == Some(e.id) {
                        self.editing = None;
                        self.input.clear();
                    }
                }
            }
        }
    }
}

fn entry_line(e: &Entry) -> String {
    let mut line = e.local();
    if let Some(hz) = e.hz {
        line.push_str(&format!("  {:.6} MHz", hz / 1_000_000.0));
    }
    if !e.mode.is_empty() {
        line.push(' ');
        line.push_str(&e.mode);
    }
    if !e.radio.is_empty() {
        line.push(' ');
        line.push_str(&e.radio);
    }
    line
}

/// The diary screen, opened only after the station file has been saved.
pub struct StationLog {
    open_path: Option<std::path::PathBuf>,
    diary: Option<Diary>,
    dial_hz: f64,
    dial_mode: String,
    dial_radio: String,
    note: String,
}

impl StationLog {
    pub fn new() -> Self {
        Self {
            open_path: None,
            diary: None,
            dial_hz: 0.0,
            dial_mode: String::new(),
            dial_radio: String::new(),
            note: String::new(),
        }
    }

    /// One status line for the Tools page: which log is open, the filter, the last note.
    pub fn status_line(&self) -> String {
        let mut line = match (&self.open_path, &self.diary) {
            (Some(p), Some(d)) => {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let mut l = format!("LOG {name}");
                if !d.date.is_empty() {
                    l.push_str(&format!("   DATE {}", d.date));
                }
                if !d.search.is_empty() {
                    l.push_str(&format!("   SEARCH \"{}\"", d.search));
                }
                if !d.status.is_empty() {
                    l.push_str(&format!("   {}", d.status));
                }
                l
            }
            _ => "NO LOG OPEN: SAVE THE STATION FILE TO START ITS LOGBOOK".to_owned(),
        };
        if !self.note.is_empty() {
            line.push_str("   ");
            line.push_str(&self.note);
        }
        line
    }

    pub fn set_dial(&mut self, hz: f64, mode: &str, radio: &str) {
        self.dial_hz = hz;
        self.dial_mode = mode.to_string();
        self.dial_radio = radio.to_string();
        if let Some(diary) = &mut self.diary {
            diary.dial_hz = hz;
            diary.dial_mode = mode.to_string();
            diary.dial_radio = radio.to_string();
        }
    }

    /// Open `Name.HSLog` beside a saved station. An unsaved station does not create a file.
    pub fn follow(&mut self, ctx: &egui::Context, saved: bool, station: &str) {
        if !saved || station.is_empty() {
            self.diary = None;
            self.open_path = None;
            self.note.clear();
            return;
        }
        let path = std::path::Path::new(station).with_extension("HSLog");
        if self.open_path.as_ref() == Some(&path) && self.diary.is_some() {
            return;
        }
        match Store::open(&path) {
            Ok(store) => {
                let mut diary = Diary::new(ctx, store);
                diary.dial_hz = self.dial_hz;
                diary.dial_mode = self.dial_mode.clone();
                diary.dial_radio = self.dial_radio.clone();
                self.diary = Some(diary);
                self.open_path = Some(path);
                self.note.clear();
            }
            Err(err) => {
                self.diary = None;
                self.open_path = None;
                self.note = format!("Cannot open the station log: {err}");
            }
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        if let Some(diary) = &mut self.diary {
            diary.draw(ui);
            return;
        }
        let theme = LcarsTheme::default();
        let message = if self.note.is_empty() {
            "Until the station has been saved, the page says so and does not write a log file."
        } else {
            self.note.as_str()
        };
        let _ = theme;
        crate::status_note::note(ui, message);
    }
}

