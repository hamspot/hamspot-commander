//! Station clocks and timers for the shack, built from rustcars widgets.
//! Run: `cargo run --example timers` (desktop only).
//!
//! * CLOCKS: big UTC clock, local time, day of year and the time to the top of the hour.
//!   COPY TIME is one button with three variants (click = log style UTC, double = local with
//!   offset, triple = ISO 8601), and it copies to the clipboard.
//! * WORLD CLOCKS (`mode_list`): click pins a zone with a 24-hour face, double a 12-hour face,
//!   triple an ISO 8601 face. Fixed offsets, no daylight saving.
//! * STOPWATCH with laps, and COUNTDOWN. ADD TIME is one button: click +1 min, double +5, triple +10.
//!   ID 10:00 starts a 10 minute countdown, the interval for station identification
//!   (FCC 97.119: at least every 10 minutes during a contact and at the end).
//!
//! The logic lives in `clock.rs` (no system clock access, unit tested).
mod clock;

use chrono::{DateTime, FixedOffset, Local, Offset, Utc};
use clock::{ClockFmt, Countdown, Stamp, Stopwatch, ZONES};
use rustcars::{
    LcarsFrame, LcarsTheme, Mode, Side, action_button, apply_theme, install_lcars_font,
    mode_button, mode_list, palette, readout,
};

pub struct Timers {
    pub theme: LcarsTheme,
    pub sw: Stopwatch,
    pub cd: Countdown,
    pub zone_modes: Vec<(String, Mode)>,
    pub status: String,
    /// Fixed clock for tests and screenshots.
    pub fixed_now: Option<(DateTime<Utc>, FixedOffset)>,
    /// Fixed UI time (seconds) for tests and screenshots.
    pub fixed_time: Option<f64>,
}

impl Timers {
    pub fn new(ctx: &egui::Context) -> Self {
        let theme = LcarsTheme::default();
        install_lcars_font(ctx);
        apply_theme(ctx, &theme);
        Self {
            theme,
            sw: Stopwatch::default(),
            cd: Countdown::default(),
            zone_modes: vec![(String::new(), Mode::Inactive); ZONES.len()],
            status: "READY".into(),
            fixed_now: None,
            fixed_time: None,
        }
    }

    fn clock(&self) -> (DateTime<Utc>, FixedOffset) {
        self.fixed_now.unwrap_or_else(|| {
            let l = Local::now();
            (l.with_timezone(&Utc), l.offset().fix())
        })
    }

    fn copy(&mut self, ui: &egui::Ui, kind: Stamp) {
        let (utc, local) = self.clock();
        let text = clock::stamp(utc, local, kind);
        ui.ctx().copy_text(text.clone());
        self.status = format!("COPIED: {text}");
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let now = self.fixed_time.unwrap_or_else(|| ui.input(|i| i.time));
        if self.fixed_time.is_none() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        super::screen_header(ui, "timers_header", &t, |ui| {
                ui.horizontal(|ui| {
                    super::title_block(ui, &t, "Timers", palette::TAN, egui::vec2(120.0, t.bar_thin));
                });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.bg))
            .show(ui, |ui| {
                let full = ui.available_rect_before_wrap();
                let split = full.left() + full.width() * 0.52;
                let left = egui::Rect::from_min_max(full.min, egui::pos2(split, full.bottom()));
                let right =
                    egui::Rect::from_min_max(egui::pos2(split + t.main_gap, full.top()), full.max);
                ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                    let mut f = LcarsFrame::new(&t, Side::Left, 0.6);
                    f.min_height = left.height();
                    f.show(ui, &t, |ui| self.clocks_panel(ui));
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
                    let mut f = LcarsFrame::new(&t, Side::Right, 0.6);
                    f.top_color = palette::PLUM;
                    f.bottom_color = palette::TEAL;
                    f.min_height = right.height();
                    f.show(ui, &t, |ui| self.watch_panel(ui, now));
                });
            });
    }

    fn clocks_panel(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let (utc, local) = self.clock();
        ui.label(egui::RichText::new("UTC").size(t.header_size));
        ui.label(
            egui::RichText::new(clock::face(utc, clock::offset(0), ClockFmt::H24) + " Z")
                .size(t.title_size * 1.6)
                .color(t.text_hi),
        );
        ui.label(
            egui::RichText::new(utc.format("%A %Y-%m-%d").to_string().to_uppercase())
                .size(t.header_size)
                .color(t.text),
        );
        let local_min = local.local_minus_utc() / 60;
        egui::Grid::new("clock_kv")
            .spacing(egui::vec2(t.main_gap, t.frame_gap))
            .show(ui, |ui| {
                for (k, v) in [
                    (
                        "LOCAL".to_owned(),
                        format!(
                            "{} ({})",
                            clock::face(utc, local, ClockFmt::H24),
                            clock::offset_label(local_min)
                        ),
                    ),
                    (
                        "DAY OF YEAR".to_owned(),
                        format!("{:03}", clock::day_of_year(utc)),
                    ),
                    (
                        "TOP OF HOUR IN".to_owned(),
                        clock::fmt_hms(clock::secs_to_next(utc, 3600) as f64),
                    ),
                ] {
                    ui.label(egui::RichText::new(k).color(palette::TEXT_MUTED));
                    readout(ui, &t, &v, false);
                    ui.end_row();
                }
            });
        ui.add_space(t.main_gap / 2.0);
        ui.label(
            egui::RichText::new("COPY TIME: CLICK UTC LOG · DOUBLE LOCAL · TRIPLE ISO 8601")
                .size(t.data_size)
                .color(palette::TEXT_MUTED),
        );
        let mut m = Mode::Inactive; // momentary: the gesture picks the format
        mode_button(ui, &t, "Copy time", &mut m, egui::vec2(180.0, 40.0)).on_hover_text(
            "click: 2026-10-23 1948Z · double: local with offset · triple: ISO 8601",
        );
        match m {
            Mode::Primary => self.copy(ui, Stamp::Utc),
            Mode::Alt1 => self.copy(ui, Stamp::Local),
            Mode::Alt2 => self.copy(ui, Stamp::Iso),
            Mode::Inactive => {}
        }
        ui.add_space(t.main_gap);
        ui.label(egui::RichText::new("WORLD CLOCKS").size(t.header_size));
        // one pinned zone at a time; its face format follows the click mode
        let pinned = self.zone_modes.iter().position(|z| z.1 != Mode::Inactive);
        match pinned {
            Some(i) => {
                let fmt = match self.zone_modes[i].1 {
                    Mode::Alt1 => ClockFmt::H12,
                    Mode::Alt2 => ClockFmt::Iso,
                    _ => ClockFmt::H24,
                };
                readout(
                    ui,
                    &t,
                    &format!(
                        "PINNED {}: {}",
                        clock::offset_label(ZONES[i].1),
                        clock::face(utc, clock::offset(ZONES[i].1), fmt)
                    ),
                    true,
                );
            }
            None => {
                ui.label(
                    egui::RichText::new(
                        "CLICK A ZONE TO PIN IT · DOUBLE 12 HOUR · TRIPLE ISO 8601",
                    )
                    .size(t.data_size)
                    .color(palette::TEXT_MUTED),
                );
            }
        }
        for (i, (label, _)) in self.zone_modes.iter_mut().enumerate() {
            let (name, min) = ZONES[i];
            *label = format!(
                "{:<8}{}   {}",
                clock::offset_label(min),
                clock::face(utc, clock::offset(min), ClockFmt::H24),
                name
            );
        }
        let before: Vec<Mode> = self.zone_modes.iter().map(|z| z.1).collect();
        let foot = t.data_size + t.frame_gap;
        let h = (ui.available_height() - foot).max(super::LIST_ROW);
        egui::ScrollArea::vertical()
            .id_salt("zone_scroll")
            .max_height(h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                mode_list(ui, &t, "zones", &mut self.zone_modes, super::LIST_ROW)
            });
        if let Some(i) = (0..before.len()).find(|i| self.zone_modes[*i].1 != before[*i]) {
            let m = self.zone_modes[i].1;
            for (j, z) in self.zone_modes.iter_mut().enumerate() {
                if j != i {
                    z.1 = Mode::Inactive;
                }
            }
            if m == Mode::Inactive {
                self.status = "UNPINNED".into();
            }
        }
        ui.label(
            egui::RichText::new("FIXED OFFSETS: NO DAYLIGHT SAVING TIME")
                .size(t.data_size)
                .color(palette::TEXT_MUTED),
        );
    }

    fn watch_panel(&mut self, ui: &mut egui::Ui, now: f64) {
        let t = self.theme;
        let b = egui::vec2(120.0, 40.0);
        ui.label(egui::RichText::new("STOPWATCH").size(t.header_size));
        ui.label(
            egui::RichText::new(clock::fmt_stopwatch(self.sw.elapsed(now)))
                .size(t.title_size * 1.6)
                .color(if self.sw.running() { t.text_hi } else { t.text }),
        );
        ui.horizontal(|ui| {
            let label = if self.sw.running() { "Stop" } else { "Start" };
            if action_button(ui, &t, label, b).clicked() {
                self.sw.toggle(now);
            }
            if action_button(ui, &t, "Lap", b).clicked() && self.sw.lap(now).is_none() {
                self.status = "START THE STOPWATCH TO RECORD LAPS".into();
            }
            if action_button(ui, &t, "Reset", b).clicked() {
                self.sw.reset();
            }
        });
        let splits = self.sw.lap_splits();
        egui::ScrollArea::vertical()
            .id_salt("laps")
            .max_height(110.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (i, (s, total)) in splits.iter().zip(&self.sw.laps).enumerate().rev() {
                    readout(
                        ui,
                        &t,
                        &format!(
                            "LAP {:<3}{}   TOTAL {}",
                            i + 1,
                            clock::fmt_stopwatch(*s),
                            clock::fmt_stopwatch(*total)
                        ),
                        false,
                    );
                }
                if splits.is_empty() {
                    ui.label(egui::RichText::new("NO LAPS").color(palette::TEXT_MUTED));
                }
            });

        ui.add_space(t.main_gap);
        ui.label(egui::RichText::new("COUNTDOWN").size(t.header_size));
        let rem = self.cd.remaining(now);
        let done = self.cd.finished(now);
        let (text, color) = if done {
            (
                "00:00 TIME!".to_owned(),
                Mode::Alt1.colors().label, // orange, plus the words: never colour alone
            )
        } else {
            (
                clock::fmt_hms(rem),
                if self.cd.running() { t.text_hi } else { t.text },
            )
        };
        ui.label(
            egui::RichText::new(text)
                .size(t.title_size * 1.6)
                .color(color),
        );
        ui.horizontal(|ui| {
            let label = if self.cd.running() { "Pause" } else { "Start" };
            if action_button(ui, &t, label, b).clicked() {
                self.cd.toggle(now);
            }
            if action_button(ui, &t, "Clear", b).clicked() {
                self.cd.set(0.0);
            }
            if action_button(ui, &t, "ID 10:00", b).clicked() {
                self.cd.set(600.0);
                self.cd.start(now);
                self.status = "ID TIMER STARTED: IDENTIFY BEFORE IT REACHES ZERO".into();
            }
        });
        ui.label(
            egui::RichText::new("ADD TIME: CLICK +1 MIN · DOUBLE +5 MIN · TRIPLE +10 MIN")
                .size(t.data_size)
                .color(palette::TEXT_MUTED),
        );
        let mut m = Mode::Inactive; // momentary
        mode_button(ui, &t, "Add time", &mut m, egui::vec2(180.0, 40.0))
            .on_hover_text("click +1 min · double +5 min · triple +10 min");
        let add = match m {
            Mode::Primary => 60.0,
            Mode::Alt1 => 300.0,
            Mode::Alt2 => 600.0,
            Mode::Inactive => 0.0,
        };
        if add > 0.0 {
            self.cd.add(now, add);
            self.status = format!("ADDED {} MIN", add / 60.0);
        }
        if self.status != "READY" && !self.status.is_empty() {
            readout(ui, &t, &self.status, self.status.starts_with("COPIED"));
        }
    }
}


#[cfg(test)]
mod logic {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn app_state_logic() {
        let ctx = egui::Context::default();
        let mut a = Timers::new(&ctx);
        a.fixed_now = Some((
            Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
            clock::offset(0),
        ));
        let (u, l) = a.clock();
        assert_eq!(clock::stamp(u, l, Stamp::Iso), "2026-01-02T03:04:05Z");
        assert_eq!(a.zone_modes.len(), ZONES.len());
    }
}
