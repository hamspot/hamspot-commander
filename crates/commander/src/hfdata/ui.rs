//! The HF Data page: UTC slot clock, clock check, mode toggles, dial presets
//! and the decode list. Every failure is shown as words in the window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

use super::clock_check::{self, ClockState};
use super::decodes::DecodeList;
use super::presets::{self, PresetFile};
use super::registry::{Registry, Session, SourceStatus};
use super::slots;
use crate::{group_box, toggle, App, Snapshot};

const CLOCK_EVERY: Duration = Duration::from_secs(60);
const WARN_INK: egui::Color32 = egui::Color32::from_rgb(232, 128, 42);

pub struct HfState {
    registry: Registry,
    presets: Result<PresetFile, String>,
    mode: String,
    list: DecodeList,
    session: Option<Session>,
    source_note: String,
    clock: Arc<Mutex<ClockState>>,
    clock_busy: Arc<AtomicBool>,
    clock_asked: Option<Instant>,
}

impl Default for HfState {
    fn default() -> Self {
        let registry = Registry::builtin();
        let mode = "FT8".to_string();
        let mut s = HfState {
            registry,
            presets: presets::embedded(),
            mode,
            list: DecodeList::default(),
            session: None,
            source_note: String::new(),
            clock: Arc::new(Mutex::new(ClockState::Checking)),
            clock_busy: Arc::new(AtomicBool::new(false)),
            clock_asked: None,
        };
        s.select_mode("FT8");
        s
    }
}

impl HfState {
    fn select_mode(&mut self, id: &str) {
        self.mode = id.to_string();
        self.session = None;
        self.source_note = match self.registry.decoder(id) {
            // A decoder exists, but nothing starts it from this page yet.
            Ok(_) => format!("{id} decoder available; no audio source is connected."),
            Err(why) => why,
        };
    }

    fn poll_clock(&mut self) {
        let due = self.clock_asked.is_none_or(|t| t.elapsed() >= CLOCK_EVERY);
        if !due || self.clock_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        self.clock_asked = Some(Instant::now());
        let slot = Arc::clone(&self.clock);
        let busy = Arc::clone(&self.clock_busy);
        let spawned = std::thread::Builder::new().name("hf-clock-check".into()).spawn(move || {
            let state = clock_check::probe();
            if let Ok(mut s) = slot.lock() {
                *s = state;
            }
            busy.store(false, Ordering::SeqCst);
        });
        if let Err(e) = spawned {
            self.clock_busy.store(false, Ordering::SeqCst);
            if let Ok(mut s) = self.clock.lock() {
                *s = ClockState::Unknown(format!("could not start the clock check: {e}"));
            }
        }
    }
}

fn bordered(ui: &egui::Ui, text: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(text).stroke(egui::Stroke::new(1.0, ui.visuals().widgets.inactive.fg_stroke.color))
}

impl App {
    pub(crate) fn page_hfdata(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.hf.poll_clock();
        if let Some(s) = self.hf.session.as_mut() {
            s.pump(&mut self.hf.list, now_ms);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(250));
        let colour = rustcars::palette::TAN;

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            group_box::group_box(ui, "UTC Slot Clock", colour, |ui| {
                ui.label(
                    egui::RichText::new(format!("{} UTC", slots::format_utc(now_ms)))
                        .size(28.0)
                        .monospace()
                        .strong(),
                );
                let slot_len = self.hf.registry.get(&self.hf.mode).and_then(|e| e.info.slot);
                match slot_len {
                    Some(len) => {
                        let slot = slots::slot_at(now_ms, len);
                        let left = slot.remaining_ms(now_ms) as f32 / 1000.0;
                        let text = format!(
                            "{} slot {} · {} · started {} · {:.1} s left",
                            self.hf.mode,
                            len.label(),
                            if slot.even() { "even" } else { "odd" },
                            slots::format_utc(slot.start_ms),
                            left
                        );
                        ui.add(egui::ProgressBar::new(slot.fraction(now_ms)).text(text));
                    }
                    None => {
                        ui.label(format!("{} is not slotted.", self.hf.mode));
                    }
                }
                let state = self
                    .hf
                    .clock
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_else(|_| ClockState::Unknown("clock check state unavailable".into()));
                let v = clock_check::verdict(&state);
                let mut t = egui::RichText::new(v.text).monospace();
                if v.warn {
                    t = t.strong().color(WARN_INK);
                }
                ui.label(t);
            });

            ui.add_space(6.0);
            group_box::group_box(ui, "Mode", colour, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let modes: Vec<(String, String)> = self
                        .hf
                        .registry
                        .entries()
                        .iter()
                        .map(|e| (e.info.id.to_string(), e.info.name.to_string()))
                        .collect();
                    for (id, name) in modes {
                        let on = self.hf.mode == id;
                        if toggle::add_toggle(ui, true, on, &name).clicked() && !on {
                            self.hf.select_mode(&id);
                        }
                    }
                });
                ui.label("Receive only.");
            });

            ui.add_space(6.0);
            group_box::group_box(ui, "Dial Presets", colour, |ui| {
                let can_tune = self.can_tune_freq(snap, true);
                if !can_tune {
                    ui.label(
                        egui::RichText::new("Radio offline or not tunable: presets cannot tune it.")
                            .color(WARN_INK),
                    );
                }
                let list = match &self.hf.presets {
                    Ok(file) => presets::for_mode(file, &self.hf.mode).into_iter().cloned().collect::<Vec<_>>(),
                    Err(e) => {
                        ui.label(egui::RichText::new(format!("Presets not loaded: {e}")).color(WARN_INK));
                        Vec::new()
                    }
                };
                if list.is_empty() && self.hf.presets.is_ok() {
                    ui.label(format!("No presets for {}.", self.hf.mode));
                }
                let mut pick = None;
                egui::Grid::new("hf_presets").num_columns(2).striped(true).show(ui, |ui| {
                    for p in &list {
                        let btn = bordered(ui, format!("{} MHz", p.mhz_label()));
                        if ui.add_enabled(can_tune, btn).clicked() {
                            pick = Some(p.clone());
                        }
                        let mut note = presets::fit_text(&presets::plan_fit(&self.plan, p.dial_hz));
                        note.push_str(&format!(" · {}", p.sideband));
                        if !p.checked {
                            note.push_str(" · UNVERIFIED frequency");
                        }
                        ui.label(note);
                        ui.end_row();
                    }
                });
                if let Some(p) = pick {
                    self.tune(snap, p.dial_hz);
                    self.send_plan_mode(snap, &p.sideband);
                }
            });

            ui.add_space(6.0);
            group_box::group_box(ui, "Decodes", colour, |ui| {
                let source = match &self.hf.session {
                    Some(s) => match &s.status {
                        SourceStatus::Running => format!("Decoder: {} running", s.mode()),
                        SourceStatus::Ended(why) => format!("Decoder stopped: {why}"),
                    },
                    None => self.hf.source_note.clone(),
                };
                let warn = !matches!(self.hf.session.as_ref().map(|s| &s.status), Some(SourceStatus::Running));
                let mut t = egui::RichText::new(source);
                if warn {
                    t = t.color(WARN_INK);
                }
                ui.label(t);
                if let Some(e) = self.hf.session.as_ref().and_then(|s| s.last_error.clone()) {
                    ui.label(egui::RichText::new(format!("Decoder error: {e}")).color(WARN_INK));
                }
                ui.horizontal(|ui| {
                    if ui.add(bordered(ui, "Clear list")).clicked() {
                        self.hf.list.clear();
                    }
                    ui.label(format!("{} rows", self.hf.list.len()));
                    if self.hf.list.future_refused > 0 {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} rows ignored: stamped ahead of this clock",
                                self.hf.list.future_refused
                            ))
                            .color(WARN_INK),
                        );
                    }
                });
                let rows: Vec<_> = self.hf.list.visible(now_ms).cloned().collect();
                if rows.is_empty() {
                    ui.label("No decodes yet.");
                    return;
                }
                egui::Grid::new("hf_decodes").num_columns(6).striped(true).show(ui, |ui| {
                    for h in ["UTC", "dB", "DT", "Hz", "Mode", "Message"] {
                        ui.label(egui::RichText::new(h).strong());
                    }
                    ui.end_row();
                    for r in rows {
                        let mono = |s: String| egui::RichText::new(s).monospace();
                        ui.label(mono(slots::format_utc(r.utc_ms)));
                        ui.label(mono(r.snr_db.map(|v| format!("{v:+.0}")).unwrap_or_else(|| "—".into())));
                        ui.label(mono(r.dt_s.map(|v| format!("{v:+.1}")).unwrap_or_else(|| "—".into())));
                        ui.label(mono(format!("{:.0}", r.offset_hz)));
                        ui.label(mono(r.mode));
                        ui.label(mono(r.text));
                        ui.end_row();
                    }
                });
            });
        });
    }
}
