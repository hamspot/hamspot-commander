//! REC (#119) on the face: right-click marks, REC keys, the REC page tab
//! icon, source tabs, the timeline, replay, archive and export.

//!
//! HackRF sources record raw channelised IQ (32 ksps cs16) and every
//! playback re-runs it through the current discriminator and decoder.
//! Encrypted traffic is never recorded.
use super::{caution, folder_tab, radio_key, App, KeyLamp, Snapshot};
use crate::{cat, config};
use eframe::egui;
use hsrec::db::{Db, Recording};

/// Well-known `decoder_params` keys. Any `HAMSPOT_[A-Z0-9_]+` key is allowed.
pub mod rec_params {
    /// dsd-fme `-X` word (WACN, system id, NAC) for P25 Phase 2 voice.
    pub const P25_X: &str = "HAMSPOT_P25_X";
}
use hsrec::meta::{self, Meta};
use hsrec::segment::{self, Action, CallKey, Press, RecMode, Segmenter, VoiceGate};
use hsrec::timeline::{self, View};
use hsrec::writer::Cmd;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const ROW_HOVER: &str = "Right-click: mark or unmark for recording (● REC).";
/// Shown only as the REC page watermark on the bottom elbow bar.
pub const PRIVACY: &str = "Recording radio traffic may be regulated where you are; that is your responsibility. \
Encrypted traffic is never recorded. Nothing is uploaded.";
/// Height kept below the timeline for the one-line usage hint. The
/// disclaimer is a watermark on the frame's bottom bar. The selected recording's details live in the Record / Playback
/// dialog, so nothing else is reserved.
const TIMELINE_RESERVE: f32 = 24.0;
/// Lane rows stretch to fill spare height, up to this multiple of LANE_H.
const LANE_GROW_MAX: f32 = 2.0;
/// The timeline never shrinks below this many lane rows.
const TIMELINE_MIN_LANES: f32 = 4.0;
const LANE_H: f32 = 22.0;
const AXIS_H: f32 = 20.0;

/// Record icon colour on the REC tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconState {
    /// No REC key armed.
    Off,
    /// Armed, nothing being written.
    Armed,
    /// Raw IQ is being written right now.
    Writing,
}

pub fn icon_state(armed: bool, writing: bool) -> IconState {
    match (armed, writing) {
        (_, true) => IconState::Writing,
        (true, false) => IconState::Armed,
        (false, false) => IconState::Off,
    }
}

/// Same tones as the speaker glyph: black off, dark red armed, and a modest
/// red while writing. No glow; the glyph never grows.
pub fn icon_color(state: IconState) -> egui::Color32 {
    match state {
        IconState::Off => egui::Color32::from_rgb(0, 0, 0),
        IconState::Armed => egui::Color32::from_rgb(110, 14, 10),
        IconState::Writing => egui::Color32::from_rgb(196, 28, 20),
    }
}

/// Width of the speaker emoji's slot on a voice tab.
pub const TAB_GLYPH_SLOT: f32 = 22.0;
/// Record glyph: a thin ring with a small core, inside the speaker's footprint.
pub const TAB_GLYPH_RING: f32 = 5.0;
pub const TAB_GLYPH_CORE: f32 = 2.5;
pub const TAB_GLYPH_STROKE: f32 = 1.25;

/// Small, static record glyph. Off is a hollow ring; armed and writing fill
/// the core so the state also shows in shape, not only colour.
pub fn paint_tab_glyph(painter: &egui::Painter, center: egui::Pos2, state: IconState) {
    let color = icon_color(state);
    painter.circle_stroke(center, TAB_GLYPH_RING, egui::Stroke::new(TAB_GLYPH_STROKE, color));
    if state != IconState::Off {
        painter.circle_filled(center, TAB_GLYPH_CORE, color);
    }
}

pub fn icon_text(state: IconState) -> &'static str {
    match state {
        IconState::Off => "REC off",
        IconState::Armed => "REC armed (waiting for traffic)",
        IconState::Writing => "REC: recording now",
    }
}

pub fn key_lamp(mode: RecMode) -> KeyLamp {
    match mode {
        RecMode::Off => KeyLamp::Dark,
        RecMode::Marked => KeyLamp::Red,
        RecMode::All => KeyLamp::Amber,
    }
}

/// Left gutter reserved on every talkgroup and channel row, marked or not,
/// so the columns never shift. The row text starts at `rect.left() + ROW_GUTTER`.
pub const ROW_GUTTER: f32 = 16.0;
/// Row mark: a small muted red dot centred in the gutter.
pub const ROW_DOT_RADIUS: f32 = 3.5;
pub const ROW_DOT_X: f32 = 9.0;
pub const ROW_DOT_COLOR: egui::Color32 = egui::Color32::from_rgb(170, 40, 32);
pub const ROW_MARKED_HOVER: &str = "Marked for recording \u{2014} right-click to unmark.";

/// Hover text for a row, by its mark.
pub fn row_hover(marked: bool) -> &'static str {
    if marked { ROW_MARKED_HOVER } else { ROW_HOVER }
}

/// Small dot in the left gutter of a marked row. No text, never over the name.
pub fn paint_row_mark(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().circle_filled(
        egui::pos2(rect.left() + ROW_DOT_X, rect.center().y),
        ROW_DOT_RADIUS,
        ROW_DOT_COLOR,
    );
}

#[derive(Default)]
struct JobState {
    label: String,
    done: usize,
    total: usize,
    finished: bool,
    lines: Vec<String>,
}

struct Job {
    state: Arc<Mutex<JobState>>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct RecUi {
    pub face: config::RecFace,
    /// Station path the recorder is set up for. None forces a redo.
    pub configured: Option<String>,
    db: Option<Db>,
    dir: PathBuf,
    db_path: PathBuf,
    seg: HashMap<u64, Segmenter>,
    /// Per radio: talkgroup calls only record while the voice decoder speaks.
    gate: HashMap<u64, VoiceGate>,
    /// Audio outputs listed for the replay output menu.
    sinks: Vec<hsrec::replay::Sink>,
    sinks_listed: bool,
    /// Where the current replay plays, as text for the dialog.
    output_note: String,
    /// Radios whose voice decoder reported a non-clear algorithm this tick.
    pub enc: HashSet<u64>,
    marks_dirty: bool,
    face_dirty: bool,
    rows: Vec<Recording>,
    rows_gen: u64,
    rows_source: String,
    rows_at: Option<Instant>,
    views: HashMap<String, View>,
    /// Recordings whose last replay decoded no voice (tagged on the timeline).
    no_voice: HashSet<i64>,
    /// Highlighted lane per source tab.
    lane_sel: HashMap<String, String>,
    query: String,
    selected: Option<i64>,
    player: Option<(i64, hsrec::replay::Player)>,
    play_next: bool,
    notes_edit: String,
    note: String,
    log: Vec<String>,
    undo: Option<(Recording, PathBuf, Instant)>,
    job: Option<Job>,
    waveforms: HashMap<i64, Vec<u8>>,
    wf_rx: Option<std::sync::mpsc::Receiver<(i64, Vec<u8>)>>,
    wf_asked: HashSet<i64>,
    archive_days: f64,
    drives: Vec<hsrec::export::Drive>,
    drive_pick: usize,
    drives_at: Option<Instant>,
    usage: Option<hsrec::storage::Usage>,
    /// Data dialog (storage, archive, drive) open. Session only.
    data_open: bool,
    /// Record / Playback dialog shown for the selected recording.
    detail_open: bool,
    /// Clicks on the header-bar controls, applied by the timeline this frame.
    bar_actions: Vec<BarAction>,
    /// Whether the shown view follows now, for the header bar's LIVE marker.
    bar_live: bool,
}

fn rows_meta(rows: &[cat::RadioRow], id: u64) -> Option<&cat::RadioRow> {
    rows.iter().find(|r| r.id == id)
}

impl App {
    fn rec_armed(&self) -> bool {
        self.rec.face.armed()
    }

    pub(super) fn rec_icon(&self) -> IconState {
        let writing = self.rec.db.is_some()
            && crate::rec_live::recorder().status.writing.load(Ordering::Relaxed);
        icon_state(self.rec_armed(), writing)
    }

    /// Tooltip text for the REC page tab (warnings carry text, never colour alone).
    pub(super) fn rec_icon_hover(
        &self,
        ui: &egui::Ui,
        resp: egui::Response,
        state: IconState,
    ) -> egui::Response {
        if state == IconState::Writing {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        resp.on_hover_text(icon_text(state))
    }

    /// REC key above LOAD/SAVE. `talkgroups` picks the tab it controls.
    pub(super) fn rec_key(&mut self, ui: &mut egui::Ui, talkgroups: bool) {
        let mode = if talkgroups { self.rec.face.talkgroups } else { self.rec.face.channels };
        let resp = radio_key(ui, "REC", key_lamp(mode), true);
        let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command || i.modifiers.ctrl));
        let press = if resp.double_clicked() {
            Some(Press::DoubleClick)
        } else if resp.clicked() {
            Some(if ctrl { Press::CtrlClick } else if shift { Press::ShiftClick } else { Press::Click })
        } else {
            None
        };
        let what = if talkgroups { "talkgroups" } else { "channels" };
        resp.on_hover_text(format!(
            "REC {what}: {}.\nClick: red, record marked {what} only.\nShift+click or double-click: amber, record all {what} traffic.\nClick again: off. Right-click a row to mark it.\nEncrypted traffic is never recorded.",
            mode.label()
        ));
        if let Some(p) = press {
            let next = segment::press(mode, p);
            if next != mode {
                if talkgroups {
                    self.rec.face.talkgroups = next;
                } else {
                    self.rec.face.channels = next;
                }
                self.rec.face_dirty = true;
                self.rec_log(format!("REC {what}: {}", next.label()));
            }
        }
    }

    pub(super) fn rec_toggle_talkgroup(&mut self, idx: usize) {
        let zone = self.talkgroup_zone;
        let Some(row) = self.talkgroups.zones.get_mut(zone).and_then(|z| z.talkgroups.get_mut(idx)) else { return };
        if row.encrypted && !row.record {
            self.talkgroup_note = "Encrypted talkgroups are never recorded; mark refused.".into();
            return;
        }
        row.record = !row.record;
        self.rec.marks_dirty = true;
    }

    pub(super) fn rec_toggle_channel(&mut self, idx: usize) {
        let zone = self.channel_zone;
        let Some(ch) = self.channels.zones.get_mut(zone).and_then(|z| z.channels.get_mut(idx)) else { return };
        if ch.encrypted && !ch.record {
            self.rec.note = "Encrypted channels are never recorded; mark refused.".into();
            return;
        }
        ch.record = !ch.record;
        self.rec.marks_dirty = true;
    }

    fn rec_log(&mut self, line: String) {
        self.rec.log.push(line);
        let n = self.rec.log.len();
        if n > 200 {
            self.rec.log.drain(..n - 200);
        }
    }

    /// Once per frame: configure the recorder for the open station, run the
    /// segmenters from the follower / scanner state, and save marks.
    pub(super) fn rec_frame(&mut self, snap: &Snapshot) {
        let path = self.config_path.clone();
        if self.rec.configured.as_deref() != Some(path.as_str()) {
            self.rec_configure(&path);
        }
        if self.rec.marks_dirty || self.rec.face_dirty {
            self.rec.marks_dirty = false;
            self.rec.face_dirty = false;
            if !self.config_path.is_empty() {
                self.save_config(snap);
            }
        }
        let armed = self.rec_armed() && self.rec.db.is_some();
        crate::rec_live::set_tap(armed);
        let recorder = crate::rec_live::recorder();
        for line in recorder.status.take_log() {
            self.rec_log(line);
        }
        let hang = Duration::from_millis(u64::from(self.voice_hang_ms));
        let now = Instant::now();
        let rows = self.hub.view().rows;
        let hackrf: Vec<u64> = rows.iter().filter(|r| r.spec.driver == cat::RadioDriver::Hackrf).map(|r| r.id).collect();
        let gone: Vec<u64> = self.rec.seg.keys().copied().filter(|id| !hackrf.contains(id) || !armed).collect();
        for id in gone {
            self.rec.gate.remove(&id);
            if let Some(mut s) = self.rec.seg.remove(&id) {
                for a in s.stop() {
                    if let Action::Close { keep } = a {
                        recorder.send(Cmd::Close { key: id, keep });
                    }
                }
            }
        }
        if !armed {
            self.rec.enc.clear();
            return;
        }
        for id in hackrf {
            let Some(row) = rows_meta(&rows, id) else { continue };
            let (want, meta) = self.rec_want(row);
            // The follower can keep a talkgroup call picked with nothing on
            // the voice channel. Record only while voice frames decode.
            let want = if want.as_ref().is_some_and(|k| k.talkgroup.is_some()) {
                let last = self.voice_frames.get(&id).copied();
                self.rec.gate.entry(id).or_default().pass(want, last, hang, now)
            } else {
                self.rec.gate.remove(&id);
                want
            };
            let enc = self.rec.enc.remove(&id);
            let max = Duration::from_secs(u64::from(
                self.rec.face.max_segment_s.clamp(segment::MAX_SEGMENT_MIN_S, segment::MAX_SEGMENT_MAX_S),
            ));
            let actions = self.rec.seg.entry(id).or_default().step_max(want, enc, hang, Some(max), now);
            for a in actions {
                match a {
                    Action::Open(_) => {
                        if let Some(m) = meta.clone() {
                            recorder.send(Cmd::Open { key: id, meta: m });
                        }
                    }
                    Action::Close { keep } => {
                        if !keep {
                            self.rec_log("REC: encrypted traffic on this call; segment discarded.".into());
                        }
                        recorder.send(Cmd::Close { key: id, keep });
                    }
                }
            }
        }
    }

    fn rec_configure(&mut self, path: &str) {
        self.rec.configured = Some(path.to_string());
        self.rec.db = None;
        self.rec.rows.clear();
        self.rec.waveforms.clear();
        self.rec.wf_asked.clear();
        self.rec.selected = None;
        self.rec.player = None;
        if path.is_empty() {
            return;
        }
        let station = Path::new(path);
        self.rec.dir = hsrec::storage::recordings_dir_beside(station);
        self.rec.db_path = hsrec::db::db_path_beside(station);
        // The UI only opens the database (and creates it) once REC was
        // used: armed, or the file already exists.
        if !self.rec.db_path.exists() && !self.rec_armed() {
            return;
        }
        let recorder = crate::rec_live::recorder();
        recorder.send(Cmd::Configure { rec_dir: self.rec.dir.clone(), db_path: self.rec.db_path.clone() });
        recorder.send(Cmd::Cap { gb: self.rec.face.cap_gb, overwrite_oldest: self.rec.face.overwrite_oldest });
        match Db::open(&self.rec.db_path) {
            Ok(db) => self.rec.db = Some(db),
            Err(e) => self.rec.note = format!("REC database: {e}"),
        }
        if self.rec_armed() {
            let line = format!(
                "REC resumed from the station file: talkgroups {}, channels {}.",
                self.rec.face.talkgroups.label(),
                self.rec.face.channels.label()
            );
            self.rec_log(line);
        }
    }

    /// What the live decoder for `row` gets from its environment, as saved
    /// `decoder_params` (see [`rec_params`]).
    fn rec_live_params(&self, row: &cat::RadioRow, m: &Meta) -> std::collections::BTreeMap<String, String> {
        let mut p = std::collections::BTreeMap::new();
        if m.modulation == "p25" {
            if let Some(word) = self.hackrf_p2_sent.get(&row.id) {
                p.insert(rec_params::P25_X.to_string(), word.clone());
            }
            if let Some(b) = self.cc_site.get(&row.id) {
                for (k, v) in [("HAMSPOT_P25_WACN", &b.wacn), ("HAMSPOT_P25_SYS", &b.sys), ("HAMSPOT_P25_NAC", &b.nac), ("HAMSPOT_P25_RFSS", &b.rfss), ("HAMSPOT_P25_SITE", &b.site)] {
                    if !v.is_empty() { p.insert(k.to_string(), v.clone()); }
                }
            }
        }
        if !m.modulation.is_empty() {
            p.insert("HAMSPOT_MODULATION".into(), m.modulation.clone());
        }
        if let Some(lna) = row.spec.lna_db { p.insert("HAMSPOT_HACKRF_LNA_DB".into(), lna.to_string()); }
        if let Some(vga) = row.spec.vga_db { p.insert("HAMSPOT_HACKRF_VGA_DB".into(), vga.to_string()); }
        p.retain(|k, _| hsrec::replay::param_key_ok(k));
        p
    }

    /// The call this HackRF radio should be recording now, and its metadata.
    fn rec_want(&self, row: &cat::RadioRow) -> (Option<CallKey>, Option<Meta>) {
        let face = &self.rec.face;
        let base = |source: &str| Meta {
            source: source.into(),
            source_kind: meta::KIND_RAW_IQ.into(),
            radio_id: row.id,
            radio_name: row.spec.name.clone(),
            raw_format: "cs16".into(),
            sample_rate: hsrec::dsp::REC_RATE,
            centre_hz: row.center_hz,
            bandwidth_hz: hsrec::dsp::REC_RATE as f64,
            chain: hsrec::dsp::CHAIN_ID.into(),
            sw_version: format!("commander {} / {}", env!("CARGO_PKG_VERSION"), meta::sw_version()),
            ..Default::default()
        };
        if let Some((hz, tg)) = self.hackrf_call.get(&row.id).copied().flatten() {
            if face.talkgroups.armed() {
                let found = self.talkgroups.zones.iter().flat_map(|z| z.talkgroups.iter()).find(|t| t.tg == tg);
                let marked = found.is_some_and(|t| t.record);
                let enc = found.is_some_and(|t| t.encrypted);
                if segment::wanted(face.talkgroups, marked, enc) {
                    // A grant outside the capture is not in these samples. The follower
                    // retunes for it; record once the new centre is live (the key
                    // carries the centre, so that starts a fresh segment).
                    if !hsrec::dsp::in_capture(hz - row.center_hz) {
                        return (None, None);
                    }
                    // A known control channel is not a talkgroup call.
                    if self.channels.zones.iter().flat_map(|z| z.channels.iter()).any(|c| c.control && (c.hz - hz).abs() < 1.0) {
                        return (None, None);
                    }
                    let key = CallKey { talkgroup: Some(tg), channel: String::new(), hz: hz.round() as u64, centre: row.center_hz.round() as u64 };
                    let mut m = base(meta::SOURCE_HACKRF_TG);
                    m.talkgroup = Some(tg);
                    m.talkgroup_name = found.map(|t| t.name.clone()).unwrap_or_default();
                    m.priority = found.is_some_and(|t| t.priority);
                    m.emergency = self.trunk_emergency.iter().any(|(_, _, t)| *t == tg);
                    m.channel_hz = hz;
                    m.offset_hz = hz - row.center_hz;
                    m.modulation = "p25".into();
                    m.decoder = serde_json::json!({ "script": hsrec::safepath::portable_decoder(&row.spec.rx_pipe), "vocoder": "p25" });
                    hsrec::safepath::portable_meta(&mut m);
                    m.decoder_params = self.rec_live_params(row, &m);
                    if let Some(b) = self.cc_site.get(&row.id) {
                        m.system = format!("WACN {} SYS {}", b.wacn, b.sys);
                        m.site = format!("RFSS {} Site {}", b.rfss, b.site);
                    }
                    return (Some(key), Some(m));
                }
            }
            return (None, None);
        }
        if row.selected && face.channels.armed() {
            if let Some((z, i)) = self.memory_scan.hold_memory() {
                if let Some(ch) = self.channels.zones.get(z).and_then(|zone| zone.channels.get(i)) {
                    if !ch.control && segment::wanted(face.channels, ch.record, ch.encrypted) && hsrec::dsp::in_capture(ch.hz - row.center_hz) {
                        let key = CallKey { talkgroup: None, channel: ch.name.clone(), hz: ch.hz.round() as u64, centre: row.center_hz.round() as u64 };
                        let mut m = base(meta::SOURCE_HACKRF_CHANNELS);
                        m.channel_name = ch.name.clone();
                        m.channel_hz = ch.hz;
                        m.offset_hz = ch.hz - row.center_hz;
                        m.modulation = if ch.vocoder.is_empty() { "fm".into() } else { ch.vocoder.clone() };
                        m.decoder = serde_json::json!({
                            "script": hsrec::safepath::portable_decoder(&row.spec.rx_pipe), "vocoder": ch.vocoder, "invert": ch.invert, "mode": ch.mode
                        });
                        m.decoder_params = self.rec_live_params(row, &m);
                        return (Some(key), Some(m));
                    }
                }
            }
        }
        (None, None)
    }

    /// Decoder params for a recording made before `decoder_params` existed:
    /// the P25 site word this session learned for the same radio, if any.
    /// The station file stores no WACN/system/NAC, so otherwise nothing.
    fn rec_param_fallback(&self, m: &Meta) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if m.modulation == "p25" && !m.decoder_params.contains_key(rec_params::P25_X) {
            if let Some(word) = self.hackrf_p2_sent.get(&m.radio_id) {
                out.push((rec_params::P25_X.to_string(), word.clone()));
            }
        }
        out
    }

    fn rec_source(&self) -> String {
        let s = &self.rec.face.source_tab;
        if s.is_empty() { meta::SOURCE_HACKRF_TG.to_string() } else { s.clone() }
    }

    fn rec_reload(&mut self, force: bool) {
        let Some(db) = self.rec.db.as_ref() else { return };
        let gen = crate::rec_live::recorder().status.generation.load(Ordering::Relaxed);
        let source = self.rec_source();
        let stale = self.rec.rows_at.is_none_or(|t| t.elapsed() > Duration::from_secs(2));
        if !force && gen == self.rec.rows_gen && source == self.rec.rows_source && !stale {
            return;
        }
        let all = db.all().unwrap_or_default();
        self.rec.usage = Some(hsrec::storage::Usage::from_rows(&all, self.rec.face.cap_gb));
        self.rec.rows = all.into_iter().filter(|r| r.meta.source == source).collect();
        self.rec.rows_gen = gen;
        self.rec.rows_source = source;
        self.rec.rows_at = Some(Instant::now());
    }

    /// REC page: source tabs, status, timeline, selected recording.
    pub(super) fn page_rec(&mut self, ui: &mut egui::Ui, _snap: &Snapshot) {
        if self.config_path.is_empty() {
            ui.label("Open or save a station first. Recordings live beside the station file.");
            return;
        }
        if self.rec.db.is_none() {
            ui.label("No recordings yet. Arm REC on the Talkgroups or Channels tab (red: marked, amber: all).");
            if ui.button("Create the recordings database now").clicked() {
                self.rec.configured = None;
                if let Ok(db) = Db::open(&self.rec.db_path) {
                    drop(db);
                }
            }
            return;
        }
        self.rec_poll_jobs();
        self.rec_reload(false);
        let sources = self.rec.db.as_ref().and_then(|db| db.sources().ok()).unwrap_or_default();
        let tabs = meta::source_tabs(&sources);
        let current = self.rec_source();
        let labels: Vec<String> = tabs.iter().map(|s| meta::source_label(s)).collect();
        let sel = tabs.iter().position(|s| *s == current);
        let picked = crate::tools::overflow_tab_row(
            ui,
            "rec-sources",
            &labels,
            sel,
            |ui, i| super::folder_tab_width(ui, &labels[i], false),
            |ui, i, lead| folder_tab(ui, Some(i) == sel, i, &labels[i], lead),
        );
        if let Some(s) = picked.map(|i| &tabs[i]).filter(|s| **s != current) {
            self.rec.face.source_tab = s.clone();
            self.rec.face_dirty = true;
            self.rec.selected = None;
        }
        ui.add_space(4.0);
        self.rec_status_row(ui);
        self.rec_job_row(ui);
        self.rec_data_window(ui.ctx());
        self.rec_detail_window(ui.ctx());
        self.rec_timeline(ui);
        ui.label(egui::RichText::new("Click a block to select it and play from that point. Drag pans, wheel over lanes scrolls, wheel over the timeline zooms.").weak().small());
        if self.rec.player.is_some() || self.rec.job.is_some() || self.rec_icon() == IconState::Writing {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
    }

    fn rec_status_row(&mut self, ui: &mut egui::Ui) {
        let status = &crate::rec_live::recorder().status;
        let state = self.rec_icon();
        ui.horizontal_wrapped(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            paint_tab_glyph(ui.painter(), rect.center(), state);
            ui.label(icon_text(state));
            ui.label(format!(
                "   TG: {}   Channels: {}",
                self.rec.face.talkgroups.label(),
                self.rec.face.channels.label()
            ));
            if let Some(u) = &self.rec.usage {
                let text = u.text();
                let color = match u.level() {
                    hsrec::storage::Level::Ok => ui.visuals().text_color(),
                    hsrec::storage::Level::Warn80 => caution(),
                    _ => egui::Color32::from_rgb(240, 60, 50),
                };
                ui.label(egui::RichText::new(format!("   Disk {text}")).color(color));
            }
        });
        if status.cap_hit.load(Ordering::Relaxed) {
            ui.label(
                egui::RichText::new("⚠ DISK CAP REACHED: recording stopped. Nothing was deleted. Archive, export, or raise the cap.")
                    .strong()
                    .color(egui::Color32::from_rgb(240, 60, 50)),
            );
        }
    }

    fn rec_job_row(&mut self, ui: &mut egui::Ui) {
        if let Some(job) = &self.rec.job {
            let st = job.state.lock().expect("rec job");
            ui.horizontal(|ui| {
                let frac = if st.total == 0 { 0.0 } else { st.done as f32 / st.total as f32 };
                ui.add(egui::ProgressBar::new(frac).desired_width(240.0).text(format!("{} {}/{}", st.label, st.done, st.total)));
                if ui.button("Cancel").clicked() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            });
        }
    }

    /// The Data dialog: storage cap, archive/rescan/cache, and drive export.
    /// One window shared by both source tabs (the state was never per tab).
    fn rec_data_window(&mut self, ctx: &egui::Context) {
        if !self.rec.data_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Data")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                self.rec_data_body(ui);
                if !self.rec.note.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new(&self.rec.note).color(caution()));
                }
            });
        if !open {
            self.rec.data_open = false;
        }
    }

    fn rec_data_body(&mut self, ui: &mut egui::Ui) {
        let mut cap_changed = false;
        ui.label(egui::RichText::new("Storage").strong());
        if let Some(u) = &self.rec.usage {
            let color = match u.level() {
                hsrec::storage::Level::Ok => ui.visuals().text_color(),
                hsrec::storage::Level::Warn80 => caution(),
                _ => egui::Color32::from_rgb(240, 60, 50),
            };
            // text() carries the warning words, so colour is never the only cue.
            ui.label(egui::RichText::new(format!("Disk {}", u.text())).color(color));
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Cap GB");
            if ui.add(egui::DragValue::new(&mut self.rec.face.cap_gb).range(0.1..=100_000.0).speed(0.5)).changed() {
                cap_changed = true;
            }
            if ui
                .checkbox(&mut self.rec.face.overwrite_oldest, "Overwrite oldest unprotected at cap")
                .on_hover_text("Off (default): stop recording at the cap and warn. Protected recordings are never overwritten.")
                .changed()
            {
                cap_changed = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Max segment");
            let r = ui
                .add(
                    egui::DragValue::new(&mut self.rec.face.max_segment_s)
                        .range(segment::MAX_SEGMENT_MIN_S..=segment::MAX_SEGMENT_MAX_S)
                        .speed(1.0)
                        .suffix(" s"),
                )
                .on_hover_text("A call still going at this length is split into a new recording, so nothing is lost. Default 120 s.");
            if r.changed() {
                self.rec.face_dirty = true;
            }
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Archive").strong());
        ui.horizontal_wrapped(|ui| {
            let busy = self.rec.job.is_some();
            if self.rec.archive_days <= 0.0 {
                self.rec.archive_days = 7.0;
            }
            if ui.add_enabled(!busy, egui::Button::new("Archive older than")).on_hover_text("zstd compress, verify by sha256, then replace. Archived recordings still play.").clicked() {
                let rows = self.rec.db.as_ref().and_then(|d| d.all().ok()).unwrap_or_default();
                let todo: Vec<Recording> = hsrec::archive::older_than(&rows, self.rec.archive_days, chrono::Utc::now()).into_iter().cloned().collect();
                self.rec_start_archive(todo);
            }
            ui.add(egui::DragValue::new(&mut self.rec.archive_days).range(0.0..=3650.0).suffix(" d"));
            if ui.add_enabled(!busy, egui::Button::new("Rescan")).on_hover_text("Re-add recordings from sidecars on disk; flag missing files.").clicked() {
                if let Some(db) = &self.rec.db {
                    match db.rescan(&self.rec.dir) {
                        Ok((a, m)) => self.rec.note = format!("Rescan: {a} added, {m} missing."),
                        Err(e) => self.rec.note = format!("Rescan failed: {e}"),
                    }
                    self.rec_reload(true);
                }
            }
            if ui.add_enabled(!busy, egui::Button::new("Clear cache")).on_hover_text("Waveform thumbnails are rebuilt from raw IQ. Nothing is lost.").clicked() {
                if let Some(db) = &self.rec.db {
                    let _ = hsrec::cache::clear(db, &self.rec.dir);
                }
                self.rec.waveforms.clear();
                self.rec.wf_asked.clear();
            }
        });
        // Export / move to a removable drive.
        if self.rec.drives_at.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) {
            self.rec.drives = hsrec::export::removable_drives();
            self.rec.drives_at = Some(Instant::now());
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Drive").strong());
        ui.horizontal_wrapped(|ui| {
            if self.rec.drives.is_empty() {
                ui.label(egui::RichText::new("no removable drive under /media or /run/media").weak());
            } else {
                if self.rec.drive_pick >= self.rec.drives.len() {
                    self.rec.drive_pick = 0;
                }
                let d = &self.rec.drives[self.rec.drive_pick];
                let text = format!("{} ({:.1} GB free)", d.label, d.free as f64 / 1e9);
                egui::ComboBox::from_id_salt("rec-drive").selected_text(text).show_ui(ui, |ui| {
                    for (i, d) in self.rec.drives.iter().enumerate() {
                        ui.selectable_value(&mut self.rec.drive_pick, i, format!("{} ({:.1} GB free)", d.label, d.free as f64 / 1e9));
                    }
                });
                let busy = self.rec.job.is_some();
                let sel = self.rec.selected;
                if ui.add_enabled(!busy && sel.is_some(), egui::Button::new("Copy selected")).clicked() {
                    self.rec_start_export(sel, false);
                }
                if ui.add_enabled(!busy && sel.is_some(), egui::Button::new("Move selected")).on_hover_text("The local copy is deleted only after the copy on the drive verifies.").clicked() {
                    self.rec_start_export(sel, true);
                }
                if ui.add_enabled(!busy, egui::Button::new(format!("Copy older than {:.0} d", self.rec.archive_days))).clicked() {
                    self.rec_start_export(None, false);
                }
                if ui.add_enabled(!busy, egui::Button::new(format!("Move older than {:.0} d", self.rec.archive_days))).clicked() {
                    self.rec_start_export(None, true);
                }
                if ui.add_enabled(!busy, egui::Button::new("Re-attach")).on_hover_text("Copy recordings that were moved to this drive back, verified by sha256.").clicked() {
                    self.rec_start_reattach();
                }
                if ui.add_enabled(!busy, egui::Button::new("Import")).on_hover_text("Import another Commander's export (HamSpot-REC/index.json) from this drive.").clicked() {
                    self.rec_start_import();
                }
            }
        });
        self.rec_job_row(ui);
        if cap_changed {
            self.rec.face_dirty = true;
            crate::rec_live::recorder().send(Cmd::Cap { gb: self.rec.face.cap_gb, overwrite_oldest: self.rec.face.overwrite_oldest });
            self.rec_reload(true);
        }
    }

    fn rec_job(&mut self, label: &str, work: impl FnOnce(Db, &Mutex<JobState>, &AtomicBool) + Send + 'static) {
        let db_path = self.rec.db_path.clone();
        let lock = match hsrec::lock::try_lock(&db_path) {
            Ok(Some(l)) => l,
            Ok(None) => {
                self.rec.note = "Another archive or export run (commander-rec) holds the lock; try later.".into();
                return;
            }
            Err(e) => {
                self.rec.note = format!("REC lock: {e}");
                return;
            }
        };
        let state = Arc::new(Mutex::new(JobState { label: label.to_string(), ..Default::default() }));
        let cancel = Arc::new(AtomicBool::new(false));
        let (s, c) = (state.clone(), cancel.clone());
        std::thread::spawn(move || {
            let _lock = lock;
            unsafe {
                let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
                libc::setpriority(libc::PRIO_PROCESS, tid, 19);
                libc::syscall(libc::SYS_ioprio_set, 1, tid as libc::c_int, 3 << 13);
            }
            match Db::open(&db_path) {
                Ok(db) => work(db, &s, &c),
                Err(e) => s.lock().expect("rec job").lines.push(format!("database: {e}")),
            }
            s.lock().expect("rec job").finished = true;
        });
        self.rec.job = Some(Job { state, cancel });
    }

    fn rec_start_archive(&mut self, todo: Vec<Recording>) {
        if todo.is_empty() {
            self.rec.note = "Nothing to archive.".into();
            return;
        }
        let dir = self.rec.dir.clone();
        self.rec_job("Archiving", move |db, st, cancel| {
            st.lock().expect("job").total = todo.len();
            let (mut ok, mut saved) = (0, 0i64);
            for (i, r) in todo.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                match hsrec::archive::archive_one(&db, &dir, r, hsrec::archive::DEFAULT_LEVEL, cancel) {
                    hsrec::archive::Outcome::Archived { saved: s } => {
                        ok += 1;
                        saved += s;
                    }
                    hsrec::archive::Outcome::Failed(e) => st.lock().expect("job").lines.push(e),
                    _ => {}
                }
                st.lock().expect("job").done = i + 1;
            }
            st.lock().expect("job").lines.push(format!("Archived {ok}, saved {:.1} MB.", saved as f64 / 1e6));
        });
    }

    fn rec_drive(&self) -> Option<hsrec::export::Drive> {
        self.rec.drives.get(self.rec.drive_pick).cloned()
    }

    fn rec_start_export(&mut self, one: Option<i64>, move_files: bool) {
        let Some(drive) = self.rec_drive() else { return };
        let all = self.rec.db.as_ref().and_then(|d| d.all().ok()).unwrap_or_default();
        let todo: Vec<Recording> = match one {
            Some(id) => all.into_iter().filter(|r| r.id == id).collect(),
            None => hsrec::archive::older_than(&all, self.rec.archive_days, chrono::Utc::now()).into_iter().cloned().collect(),
        };
        if todo.is_empty() {
            self.rec.note = "Nothing to export.".into();
            return;
        }
        if self.rec.player.as_ref().is_some_and(|(id, _)| todo.iter().any(|r| r.id == *id)) {
            self.rec.player = None;
        }
        let dir = self.rec.dir.clone();
        self.rec_job(if move_files { "Moving" } else { "Copying" }, move |db, st, cancel| {
            let rep = hsrec::export::export(&db, &dir, &todo, &drive.mount, &drive.label, move_files, cancel, |n, of| {
                let mut s = st.lock().expect("job");
                s.done = n;
                s.total = of;
            });
            let mut s = st.lock().expect("job");
            s.lines.extend(rep.failed.iter().cloned());
            s.lines.push(format!(
                "{} copied, {} moved, {} failed{}. Synced: you may remove the drive.",
                rep.copied,
                rep.moved,
                rep.failed.len(),
                if rep.cancelled { " (cancelled)" } else { "" }
            ));
        });
    }

    fn rec_start_reattach(&mut self) {
        let Some(drive) = self.rec_drive() else { return };
        let dir = self.rec.dir.clone();
        self.rec_job("Re-attaching", move |db, st, _| {
            let rep = hsrec::export::reattach(&db, &dir, &drive);
            let mut s = st.lock().expect("job");
            s.lines.extend(rep.failed.iter().cloned());
            s.lines.push(format!("{} recordings back on this disk.", rep.copied));
        });
    }

    fn rec_start_import(&mut self) {
        let Some(drive) = self.rec_drive() else { return };
        let dir = self.rec.dir.clone();
        self.rec_job("Importing", move |db, st, _| {
            let rep = hsrec::export::import(&db, &dir, &drive.mount.join(hsrec::export::EXPORT_DIR));
            let mut s = st.lock().expect("job");
            s.lines.extend(rep.failed.iter().cloned());
            s.lines.push(format!("{} recordings imported.", rep.copied));
        });
    }

    fn rec_poll_jobs(&mut self) {
        let finished = self.rec.job.as_ref().is_some_and(|j| j.state.lock().expect("job").finished);
        if finished {
            if let Some(job) = self.rec.job.take() {
                let lines = std::mem::take(&mut job.state.lock().expect("job").lines);
                if let Some(last) = lines.last() {
                    self.rec.note = last.clone();
                }
                for l in lines {
                    self.rec_log(l);
                }
                self.rec_reload(true);
            }
        }
        if let Some(rx) = &self.rec.wf_rx {
            while let Ok((id, wf)) = rx.try_recv() {
                self.rec.waveforms.insert(id, wf);
            }
        }
        // Undo window for delete: 60 s, then the trash is emptied.
        if self.rec.undo.as_ref().is_some_and(|(_, _, t)| t.elapsed() > Duration::from_secs(60)) {
            if let Some((_, trash, _)) = self.rec.undo.take() {
                let _ = std::fs::remove_dir_all(trash);
            }
        }
        if let Some((id, p)) = &self.rec.player {
            if p.finished() {
                let id = *id;
                let err = p.error.lock().expect("replay").clone();
                let outcome = p.outcome();
                self.rec.player = None;
                let rec = self.rec.rows.iter().find(|r| r.id == id).cloned();
                if !err.is_empty() {
                    self.rec.note = format!("Playback failed: {err}");
                    self.rec_log(self.rec.note.clone());
                } else if let (Some(msg), Some(rec)) = (outcome.no_voice_text(), rec.as_ref()) {
                    self.rec.no_voice.insert(id);
                    let band = if hsrec::dsp::in_capture(rec.meta.offset_hz) {
                        String::new()
                    } else {
                        format!(" The channel was {:.3} MHz from the capture centre, outside the HackRF's usable 6 MHz, so the call is not in this recording.", rec.meta.offset_hz / 1e6)
                    };
                    self.rec.note = format!("{msg}{band}");
                    self.rec_log(self.rec.note.clone());
                } else if self.rec.play_next {
                    if let Some(next) = timeline::next_after(&self.rec.rows, id) {
                        self.rec.selected = Some(next);
                        self.rec_play(next, 0);
                    }
                }
            }
        }
    }

    /// Ask a background thread for waveform thumbnails of `ids`.
    fn rec_waveforms(&mut self, ids: Vec<i64>) {
        let ids: Vec<i64> = ids.into_iter().filter(|id| self.rec.wf_asked.insert(*id)).take(32).collect();
        if ids.is_empty() {
            return;
        }
        let rows: Vec<Recording> = self.rec.rows.iter().filter(|r| ids.contains(&r.id) && !r.offline() && !r.missing && !r.meta.end_utc.is_empty()).cloned().collect();
        let (tx, rx) = std::sync::mpsc::channel();
        let (dir, db_path) = (self.rec.dir.clone(), self.rec.db_path.clone());
        std::thread::spawn(move || {
            unsafe {
                let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
                libc::setpriority(libc::PRIO_PROCESS, tid, 19);
            }
            let Ok(db) = Db::open(&db_path) else { return };
            let version = hsrec::cache::decoder_version();
            for r in rows {
                if let Ok(wf) = hsrec::cache::waveform(&db, &dir, &r, &version) {
                    if tx.send((r.id, wf)).is_err() {
                        return;
                    }
                }
            }
            hsrec::cache::prune(&dir, hsrec::cache::DEFAULT_CACHE_BYTES, &version);
        });
        self.rec.wf_rx = Some(rx);
    }

    fn rec_play(&mut self, id: i64, from_ms: u64) {
        self.rec.player = None;
        let Some(rec) = self.rec.rows.iter().find(|r| r.id == id).cloned() else { return };
        let path = if rec.offline() {
            match hsrec::export::locate(&rec, &self.rec.drives) {
                Some(p) => p,
                None => {
                    self.rec.note = format!("This recording is on drive {}; attach it to play.", rec.location.trim_start_matches("offline:"));
                    return;
                }
            }
        } else {
            self.rec.dir.join(&rec.raw_path)
        };
        let fallback = self.rec_param_fallback(&rec.meta);
        // The recording names its decoder only as `modems/<file>`; Commander
        // resolves it in its own modems folders and never runs a stored path.
        let mut meta = rec.meta.clone();
        hsrec::safepath::portable_meta(&mut meta);
        let stored = meta.decoder.get("script").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let script = if stored.is_empty() {
            None
        } else {
            match hsrec::safepath::resolve_decoder(&stored, &crate::rx_pipe::modem_dirs()) {
                Ok(p) => Some(p),
                Err(e) => {
                    self.rec.note = e;
                    None
                }
            }
        };
        let decoder = script.and_then(|script| {
            // The script's socat listens on this port for the whole replay, so
            // the reservation rides with the player and is released when its
            // thread ends (finish, stop, or a new selection).
            let port = crate::rx_pipe::VoicePort::claim().ok()?;
            let argv = vec![script.display().to_string(), port.port().to_string()];
            let env = hsrec::replay::decoder_env(&rec.meta, &fallback);
            Some(hsrec::replay::DecoderCmd { argv, env, hold: Some(std::sync::Arc::new(hsrec::replay::Hold(Box::new(port)))) })
        });
        if !self.rec.face.output.is_empty() || !self.rec.sinks_listed {
            self.rec.sinks = hsrec::replay::list_sinks();
            self.rec.sinks_listed = true;
        }
        let (argv, missing) = hsrec::replay::pick_output(&self.rec.face.output, &self.rec.sinks);
        self.rec.output_note = missing.unwrap_or_default();
        match hsrec::replay::Player::start(&path, &rec.meta, from_ms, decoder, argv) {
            Ok(p) => self.rec.player = Some((id, p)),
            Err(e) => self.rec.note = format!("Playback: {e}"),
        }
    }

    /// The REC view controls, drawn as a run of LCARS segments at the right
    /// end of the page-tab bar. Clicks are queued and applied by the timeline.
    pub(crate) fn rec_bar(&mut self, ui: &mut egui::Ui, compact: bool) {
        let theme = rustcars::LcarsTheme::default();
        let segs = bar_segments(compact);
        for seg in segs.iter() {
            // Every control is a plain rectangle, first and last included.
            let cap = rustcars::Cap::None;
            let on = match seg.action {
                BarAction::Live => Some(self.rec.bar_live),
                BarAction::Data => Some(self.rec.data_open),
                BarAction::PlayNext => Some(self.rec.play_next),
                _ => None,
            };
            let base = bar_colour(seg.action);
            let size = egui::vec2(seg.width, theme.bar_thin);
            let resp = match on {
                // Toggles: the rustcars toggle button (thin border, painted
                // ●/○). The timeline applies the action, so the local flip
                // is discarded here.
                Some(state) => {
                    let mut local = state;
                    let resp = rustcars::toggle_button(ui, &theme, seg.label, &mut local, base, size);
                    let name = bar_label(seg, on);
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, state, &name));
                    resp
                }
                None => rustcars::lcars_button(ui, &theme, &bar_label(seg, on), base, size, cap, true),
            };
            let resp = resp.on_hover_text(bar_hint(seg.action, on));
            if resp.clicked() {
                self.rec.bar_actions.push(seg.action);
                // The timeline applies the action; it may already be drawn.
                ui.ctx().request_repaint();
            }
        }
    }

    fn rec_timeline(&mut self, ui: &mut egui::Ui) {
        let now = chrono::Utc::now().timestamp_millis();
        let source = self.rec_source();
        let query = self.rec.query.clone();
        let recs: Vec<Recording> = self.rec.rows.iter().filter(|r| timeline::matches(r, &query)).cloned().collect();
        let lanes = timeline::lanes(&recs);
        // First look at a source frames its most recent recordings.
        let mut view = match self.rec.views.get(&source) {
            Some(v) => *v,
            None if !recs.is_empty() => timeline::fit(&recs, now),
            None => View::live(now, 3_600_000),
        };
        view.follow(now);
        let width_all = ui.available_width();
        for action in std::mem::take(&mut self.rec.bar_actions) {
            let w = plot_width(width_all);
            match action {
                BarAction::DayBack => view.day(view.start + view.span / 2, -1),
                BarAction::Today => view.day(now, 0),
                BarAction::DayForward => view.day(view.start + view.span / 2, 1),
                BarAction::ZoomOut => view.zoom_at(BUTTON_ZOOM, w / 2.0, w, now),
                BarAction::ZoomIn => view.zoom_at(1.0 / BUTTON_ZOOM, w / 2.0, w, now),
                BarAction::Fit => view = timeline::fit(&recs, now),
                BarAction::Live => view.toggle_live(now),
                BarAction::Data => self.rec.data_open = !self.rec.data_open,
                BarAction::PlayNext => self.rec.play_next = !self.rec.play_next,
            }
            // Never show the future: Today / Day ▶ / Fit stop at NOW.
            view.clamp(now);
        }
        view.clamp(now);
        self.rec.bar_live = view.live;
        // The timeline spans the full width; the lane shelf is drawn over its left side.
        // The plot stops short of a gutter on the right for the lane scrollbar.
        let plot_w = plot_width(width_all);
        let width = plot_w;
        let shelf_w = timeline::shelf_width(plot_w);
        ui.horizontal(|ui| {
            ui.label("Search");
            ui.add(egui::TextEdit::singleline(&mut self.rec.query).desired_width(search_width(width_all)).hint_text("TG, channel, radio…"));
            let shown = timeline::layout(&recs, &lanes, &view, width, now).len();
            ui.label(format!("{} recordings ({shown} in view), {} lanes", recs.len(), lanes.len()));
            if let Some((rec, _, _)) = &self.rec.undo {
                let label = format!("Undo delete of {}", rec.meta.lane());
                if ui.small_button(label).clicked() {
                    self.rec_undo();
                }
            }
        });
        let blocks = timeline::layout(&recs, &lanes, &view, width, now);
        let subs: Vec<usize> = (0..lanes.len())
            .map(|l| blocks.iter().filter(|b| b.lane == l).map(|b| b.sub + 1).max().unwrap_or(1))
            .collect();
        let viewport_h = timeline_viewport_h(ui.available_height());
        let row_h = lane_row_h(subs.iter().sum(), viewport_h);
        let lane_h: Vec<f32> = subs.iter().map(|n| *n as f32 * row_h).collect();
        let lane_top: Vec<f32> = lane_h.iter().scan(0.0, |y, h| { let top = *y; *y += *h; Some(top) }).collect();
        let body_h: f32 = lane_h.iter().sum::<f32>().max(LANE_H * 2.0);
        // Fill the viewport even with few lanes, so the grid takes all free height.
        let body_h = body_h.max(viewport_h);
        let weak = ui.visuals().weak_text_color();
        let bg = egui::Color32::from_rgb(14, 14, 18);
        let step = timeline::tick_step(view.span, width);
        let ticks: Vec<i64> = {
            let mut v = Vec::new();
            let mut t = view.start - view.start.rem_euclid(step);
            while t <= view.end() {
                if t >= view.start { v.push(t); }
                t += step;
            }
            v
        };
        // Axis row: label column header, then tick labels over the grid only.
        let (axis, axis_resp) = ui.allocate_exact_size(egui::vec2(plot_w, AXIS_H), egui::Sense::click_and_drag());
        {
            let painter = ui.painter_at(axis);
            painter.rect_filled(axis, egui::CornerRadius::ZERO, bg);
            let grid = axis;
            let gp = ui.painter_at(grid);
            let fmt = if step >= 86_400_000 { "%a %m-%d" } else if step >= 60_000 { "%H:%M" } else { "%H:%M:%S" };
            for t in &ticks {
                let x = grid.left() + view.x(*t, width);
                let local = chrono::DateTime::from_timestamp_millis(*t).unwrap_or_default().with_timezone(&chrono::Local);
                gp.text(egui::pos2(x + 2.0, grid.top() + 2.0), egui::Align2::LEFT_TOP, local.format(fmt).to_string(), egui::FontId::monospace(11.0), weak);
            }
            paint_shelf(&painter, egui::Rect::from_min_size(axis.min, egui::vec2(shelf_w, AXIS_H)), bg);
            let now_w = ui.fonts_mut(|f| f.layout_no_wrap("NOW".into(), egui::FontId::monospace(10.0), weak).size().x);
            if let Some((x0, x1)) = now_fade_span(view.x(now, width), plot_w, right_shelf_w(now_w)) {
                let r = egui::Rect::from_min_max(egui::pos2(axis.left() + x0, axis.top()), egui::pos2(axis.left() + x1, axis.bottom()));
                paint_shelf_right(&painter, r, NOW_SHELF);
            }
            painter.text(axis.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, "Lane", egui::FontId::monospace(11.0), weak);
        }
        // Wheel: over the opaque shelf it scrolls the lanes (the scroll area
        // does that); over the grid, the fade, or the axis it zooms time, so
        // take it away from the scroll area there.
        let body_view = egui::Rect::from_min_size(egui::pos2(axis.left(), axis.bottom()), egui::vec2(plot_w, viewport_h));
        let wheel_pos = ui.input(|i| i.pointer.hover_pos());
        // A REC window (Record / Playback, Data) above the timeline owns the
        // pointer there: no zoom, and its own scroll area keeps the wheel.
        let own_layer = ui.layer_id();
        let layer_ctx = ui.ctx().clone();
        let ours = move |p: egui::Pos2| pointer_is_ours(layer_ctx.layer_id_at(p), own_layer);
        let zoom_here = wheel_pos.is_some_and(|p| {
            ours(p) && (axis.contains(p) || body_view.contains(p)) && !timeline::on_shelf(p.x - axis.left(), shelf_w)
        });
        let zoom_scroll = if zoom_here {
            ui.input_mut(|i| {
                // Raw delta, once per event: the smoothed delta repeats a
                // notch over several frames, which zoomed tenfold per click.
                let page = i.content_rect().height();
                let d: f32 = i
                    .raw
                    .events
                    .iter()
                    .map(|e| match e {
                        egui::Event::MouseWheel { unit, delta, .. } => match unit {
                            egui::MouseWheelUnit::Point => delta.y,
                            egui::MouseWheelUnit::Line => delta.y * NOTCH_PTS,
                            egui::MouseWheelUnit::Page => delta.y * page,
                        },
                        _ => 0.0,
                    })
                    .sum();
                i.smooth_scroll_delta.y = 0.0;
                d
            })
        } else {
            0.0
        };
        let mut drag_dx = if axis_resp.dragged() { axis_resp.drag_delta().x } else { 0.0 };
        let mut hit: Option<(i64, u64)> = None;
        let mut want_wf = Vec::new();
        let mut lane_click: Option<String> = None;
        let sel_lane = self.rec.lane_sel.get(&source).cloned();
        let by_id: HashMap<i64, &Recording> = recs.iter().map(|r| (r.id, r)).collect();
        let selected_id = self.rec.selected;
        let waveforms = &self.rec.waveforms;
        let no_voice = &self.rec.no_voice;
        let player = self.rec.player.as_ref().map(|(id, pl)| (*id, pl.pos_ms.load(Ordering::Relaxed) as i64));
        egui::ScrollArea::vertical()
            .id_salt(("rec-timeline", source.clone()))
            .min_scrolled_height(viewport_h)
            .max_height(viewport_h)
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .show(ui, |ui| {
                ui.style_mut().spacing.scroll = egui::style::ScrollStyle::thin();
                let (rect, _) = ui.allocate_exact_size(egui::vec2(plot_w, body_h), egui::Sense::hover());
                let label_rect = egui::Rect::from_min_size(rect.min, egui::vec2(shelf_w, body_h));
                let grid = rect;
                ui.painter_at(rect).rect_filled(rect, egui::CornerRadius::ZERO, bg);
                let gp = ui.painter_at(grid);
                for t in &ticks {
                    let x = grid.left() + view.x(*t, width);
                    gp.line_segment([egui::pos2(x, grid.top()), egui::pos2(x, grid.bottom())], egui::Stroke::new(1.0, egui::Color32::from_gray(40)));
                }
                for (i, name) in lanes.iter().enumerate() {
                    let y = rect.top() + lane_top[i];
                    let full = egui::Rect::from_min_size(egui::pos2(rect.left(), y), egui::vec2(plot_w, lane_h[i]));
                    if sel_lane.as_deref() == Some(name.as_str()) {
                        gp.rect_filled(full, egui::CornerRadius::ZERO, egui::Color32::from_rgb(40, 40, 70));
                    } else if i % 2 == 1 {
                        gp.rect_filled(full, egui::CornerRadius::ZERO, egui::Color32::from_rgb(20, 20, 26));
                    }
                }
                // Lane names run on past the shelf into the lane, fading out
                // only by halfway across. They are drawn after the shelf (full
                // contrast) further down; painted only, so input past the
                // solid shelf passes through to the grid.
                let label_font = egui::FontId::monospace(12.0);
                let now_w_pre = ui.fonts_mut(|f| f.layout_no_wrap("NOW".into(), egui::FontId::monospace(10.0), egui::Color32::WHITE).size().x);
                let label_cap = lane_label_cap(width, width);
                let _ = now_w_pre;
                let solid_end = shelf_w * timeline::SHELF_OPAQUE_FRAC;
                let lane_ink = |ui: &egui::Ui, selected: bool, cell: egui::Rect| if selected || ui.rect_contains_pointer(cell) { egui::Color32::WHITE } else { ui.visuals().text_color() };
                if lanes.is_empty() {
                    gp.text(grid.center(), egui::Align2::CENTER_CENTER, "No recordings in this view.", egui::FontId::proportional(14.0), weak);
                }
                // Grid interaction: drag pans, scroll zooms, click plays a block.
                let gresp = ui.interact(grid, ui.scope_id().with("rec-grid"), egui::Sense::click_and_drag());
                if gresp.dragged() { drag_dx += gresp.drag_delta().x; }
                let pointer = gresp.interact_pointer_pos().filter(|p| ours(*p));
                let hover = gresp.hover_pos().filter(|p| ours(*p));
                let mut block_tip: Option<String> = None;
                for b in &blocks {
                    let Some(r) = by_id.get(&b.id) else { continue };
                    let y = rect.top() + lane_top[b.lane] + b.sub as f32 * row_h;
                    let br = egui::Rect::from_min_max(egui::pos2(grid.left() + b.x0.max(0.0), y + 3.0), egui::pos2(grid.left() + b.x1.min(width), y + row_h - 3.0));
                    if br.width() <= 0.0 {
                        continue;
                    }
                    let fill = if r.offline() || r.missing {
                        egui::Color32::from_gray(70)
                    } else if b.live {
                        egui::Color32::from_rgb(170, 24, 18)
                    } else if r.meta.source_kind == meta::KIND_DEMOD_AUDIO {
                        egui::Color32::from_rgb(40, 90, 120)
                    } else {
                        egui::Color32::from_rgb(46, 120, 70)
                    };
                    gp.rect_filled(br, egui::CornerRadius::same(2), fill);
                    if let Some(wf) = waveforms.get(&b.id) {
                        let n = wf.len().max(1) as f32;
                        for (k, v) in wf.iter().enumerate() {
                            let x = br.left() + br.width() * k as f32 / n;
                            let a = br.height() * 0.45 * (*v as f32 / 255.0);
                            gp.line_segment([egui::pos2(x, br.center().y - a), egui::pos2(x, br.center().y + a)], egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(220, 240, 220, 120)));
                        }
                    } else if br.width() > 20.0 {
                        want_wf.push(b.id);
                    }
                    let mut tag = String::new();
                    if r.meta.emergency { tag.push_str("EMERG "); }
                    if r.meta.priority { tag.push_str("PRI "); }
                    if r.archived { tag.push_str("zst "); }
                    if r.protected { tag.push_str("🔒"); }
                    if r.star { tag.push('★'); }
                    if r.offline() { tag = format!("{tag}offline"); }
                    if r.missing { tag = format!("{tag}MISSING"); }
                    if !hsrec::dsp::in_capture(r.meta.offset_hz) && r.meta.source_kind != meta::KIND_DEMOD_AUDIO { tag = format!("{tag} OUT OF BAND"); }
                    if no_voice.contains(&b.id) { tag = format!("{tag} no voice"); }
                    if !tag.is_empty() && br.width() > 30.0 {
                        gp.text(br.left_center() + egui::vec2(3.0, 0.0), egui::Align2::LEFT_CENTER, tag.trim(), egui::FontId::monospace(10.0), egui::Color32::WHITE);
                    }
                    if r.meta.emergency {
                        gp.rect_stroke(br, egui::CornerRadius::same(2), egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 60, 40)), egui::StrokeKind::Outside);
                    }
                    if selected_id == Some(b.id) {
                        gp.rect_stroke(br, egui::CornerRadius::same(2), egui::Stroke::new(2.0, egui::Color32::WHITE), egui::StrokeKind::Outside);
                    }
                    let t0 = timeline::parse_ms(&r.meta.start_utc).unwrap_or(0);
                    if let Some(p) = pointer {
                        if gresp.clicked() && br.expand(2.0).contains(p) {
                            let at = (view.t(p.x - grid.left(), width) - t0).max(0) as u64;
                            hit = Some((b.id, at));
                        }
                    }
                    if hover.is_some_and(|p| br.expand(2.0).contains(p)) {
                        let start = chrono::DateTime::from_timestamp_millis(t0).unwrap_or_default().with_timezone(&chrono::Local);
                        let unit = r.meta.radio_unit_id.map(|u| format!("\nRadio ID {u}")).unwrap_or_default();
                        let tip = format!("{}\n{} ({:.1} s){unit}", r.meta.lane(), start.format("%Y-%m-%d %H:%M:%S"), r.meta.duration_ms as f64 / 1000.0);
                        block_tip = Some(tip);
                    }
                    if let Some((pid, pos)) = player {
                        if pid == b.id {
                            let x = grid.left() + view.x(t0 + pos, width);
                            gp.line_segment([egui::pos2(x, br.top() - 2.0), egui::pos2(x, br.bottom() + 2.0)], egui::Stroke::new(2.0, egui::Color32::YELLOW));
                        }
                    }
                }
                let now_x = grid.left() + view.x(now, width);
                if now_x >= grid.left() && now_x <= grid.right() {
                    gp.line_segment([egui::pos2(now_x, grid.top()), egui::pos2(now_x, grid.bottom())], egui::Stroke::new(1.0, egui::Color32::from_rgb(240, 60, 50)));
                }
                // Dark red shelf on the right edge, mirroring the lane shelf,
                // just wide enough for the NOW label. Purely painted: input
                // still goes to the grid underneath.
                let now_font = egui::FontId::monospace(10.0);
                let now_w = ui.fonts_mut(|f| f.layout_no_wrap("NOW".into(), now_font.clone(), egui::Color32::WHITE).size().x);
                if now_x >= grid.left() && now_x <= grid.right() {
                    gp.text(egui::pos2(now_x - 2.0, grid.bottom() - 2.0), egui::Align2::RIGHT_BOTTOM, "NOW", now_font, egui::Color32::from_rgb(255, 90, 75));
                }
                // Tint goes over the NOW line and label so they sit under it.
                if let Some((x0, x1)) = now_fade_span(now_x - grid.left(), plot_w, right_shelf_w(now_w)) {
                    let r = egui::Rect::from_min_max(egui::pos2(grid.left() + x0, grid.top()), egui::pos2(grid.left() + x1, grid.bottom()));
                    paint_shelf_right(&ui.painter_at(rect), r, NOW_SHELF);
                }
                if let Some(tip) = block_tip {
                    gresp.on_hover_text(tip);
                }
                // Shelf over the grid: opaque on the left, fading out. Labels are
                // clickable only on the opaque part; the fade passes through.
                paint_shelf(&ui.painter_at(rect), label_rect, bg);
                for (i, name) in lanes.iter().enumerate() {
                    let y = rect.top() + lane_top[i];
                    let selected = sel_lane.as_deref() == Some(name.as_str());
                    let cell = egui::Rect::from_min_size(egui::pos2(label_rect.left(), y), egui::vec2(solid_end, row_h));
                    let resp = ui.interact(cell, ui.scope_id().with(("rec-lane", i)), egui::Sense::click());
                    let ink = lane_ink(ui, selected, cell);
                    let job = lane_label_job(ui, name, &label_font, ink, LABEL_PAD, label_cap);
                    let g = ui.fonts_mut(|f| f.layout_job(job));
                    ui.painter_at(grid).galley(egui::pos2(cell.left() + LABEL_PAD, cell.center().y - g.size().y / 2.0), g, ink);
                    if resp.clicked() {
                        lane_click = Some(name.clone());
                    }
                    resp.on_hover_text(format!("{name}\nClick to highlight this lane."));
                }
            });
        if let Some(name) = lane_click {
            if self.rec.lane_sel.get(&source) == Some(&name) {
                self.rec.lane_sel.remove(&source);
            } else {
                self.rec.lane_sel.insert(source.clone(), name);
            }
        }
        if drag_dx != 0.0 {
            view.pan_px(drag_dx, width);
        }
        if zoom_scroll.abs() > 0.5 {
            let anchor = wheel_pos.map(|p| p.x - axis.left()).unwrap_or(width).clamp(0.0, width);
            view.zoom_at(wheel_zoom_factor(zoom_scroll), anchor, width, now);
        }
        view.clamp(now);
        // Store only once there is something to frame, so the first rows fit.
        if !recs.is_empty() || self.rec.views.contains_key(&source) {
            self.rec.views.insert(source, view);
        }
        if let Some((id, at)) = hit {
            if self.rec.selected != Some(id) {
                self.rec.notes_edit = self.rec.rows.iter().find(|r| r.id == id).map(|r| r.notes.clone()).unwrap_or_default();
            }
            self.rec.selected = Some(id);
            self.rec.detail_open = true;
            // Click plays from the clicked point (seek).
            self.rec_play(id, at);
        }
        self.rec_waveforms(want_wf);
    }

    /// Label/value rows for the selected recording; missing fields omitted.
    fn rec_meta_rows(&self, rec: &Recording) -> Vec<(&'static str, String)> {
        let m = &rec.meta;
        let local = |utc: &str| {
            chrono::DateTime::parse_from_rfc3339(utc)
                .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S %Z").to_string())
                .unwrap_or_else(|_| if utc.is_empty() { "recording…".into() } else { utc.to_string() })
        };
        let mut v: Vec<(&'static str, String)> = Vec::new();
        let mut put = |k: &'static str, s: String| {
            if !s.trim().is_empty() {
                v.push((k, s));
            }
        };
        put("Source kind", if m.source_kind == meta::KIND_DEMOD_AUDIO { "demod audio (rig's own audio, not raw)".into() } else { m.source_kind.clone() });
        put("Source tab", meta::source_label(&m.source));
        put("Radio", if m.radio_name.is_empty() { String::new() } else { format!("{} (id {})", m.radio_name, m.radio_id) });
        if m.channel_hz > 0.0 { put("Frequency", format!("{:.4} MHz", m.channel_hz / 1e6)); }
        if m.centre_hz > 0.0 { put("Capture centre", format!("{:.4} MHz (offset {:+.1} kHz)", m.centre_hz / 1e6, m.offset_hz / 1e3)); }
        if m.bandwidth_hz > 0.0 { put("Bandwidth", format!("{:.1} kHz", m.bandwidth_hz / 1e3)); }
        put("Modulation", m.modulation.clone());
        put("System", m.system.clone());
        put("Site", m.site.clone());
        if let Some(tg) = m.talkgroup { put("Talkgroup", format!("{} ({tg})", m.talkgroup_name)); }
        put("Channel", m.channel_name.clone());
        if let Some(u) = m.radio_unit_id { put("Radio unit ID", u.to_string()); }
        put("Start", local(&m.start_utc));
        put("End", local(&m.end_utc));
        put("Duration", format!("{:.1} s", m.duration_ms as f64 / 1000.0));
        let kind = if m.raw_format == "cs16" { "raw IQ" } else { "audio" };
        put("Sample rate / format", format!("{kind} {} ksps {}", m.sample_rate / 1000, m.raw_format));
        put("File", self.rec_raw_path(rec).map(|p| p.display().to_string()).unwrap_or_else(|| rec.raw_path.clone()));
        if m.bytes > 0 { put("Size", format!("{:.1} MB", m.bytes as f64 / 1e6)); }
        if rec.archived { put("Archive", format!("{} level {}, {:.1} MB", rec.codec, rec.level, rec.compressed_bytes as f64 / 1e6)); }
        put("Location", rec.location.clone());
        put("SHA-256", rec.sha256.clone());
        put("Chain", m.chain.clone());
        if !m.gains.is_null() { put("Gains", m.gains.to_string()); }
        if !m.decoder.is_null() { put("Decoder", m.decoder.to_string()); }
        put("Software", m.sw_version.clone());
        if m.part > 0 { put("Part", m.part.to_string()); }
        if m.dropped > 0 { put("Dropped samples", format!("⚠ {} (disk slow)", m.dropped)); }
        v
    }

    /// Replay output: which sink pw-cat plays to, shown as text.
    fn rec_output_row(&mut self, ui: &mut egui::Ui) {
        if !self.rec.sinks_listed {
            self.rec.sinks = hsrec::replay::list_sinks();
            self.rec.sinks_listed = true;
        }
        let label = |name: &str, sinks: &[hsrec::replay::Sink]| -> String {
            if name.is_empty() {
                return "System default".into();
            }
            match sinks.iter().find(|s| s.name == name) {
                Some(s) if !s.description.is_empty() => format!("{} ({})", s.description, s.name),
                Some(s) => s.name.clone(),
                None => format!("{name} (not found)"),
            }
        };
        let mut pick = None;
        let mut refresh = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Output");
            let current = label(&self.rec.face.output, &self.rec.sinks);
            egui::ComboBox::from_id_salt("rec_output")
                .selected_text(current)
                .width(260.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(self.rec.face.output.is_empty(), "System default").clicked() {
                        pick = Some(String::new());
                    }
                    for s in &self.rec.sinks {
                        let text = if s.description.is_empty() { s.name.clone() } else { format!("{} ({})", s.description, s.name) };
                        if ui.selectable_label(self.rec.face.output == s.name, text).clicked() {
                            pick = Some(s.name.clone());
                        }
                    }
                });
            if ui.button("Refresh").on_hover_text("List the audio outputs again.").clicked() {
                refresh = true;
            }
        });
        if refresh {
            self.rec.sinks = hsrec::replay::list_sinks();
        }
        if let Some(name) = pick {
            if name != self.rec.face.output {
                self.rec.face.output = name;
                self.rec.face_dirty = true;
            }
        }
        let missing = !self.rec.face.output.is_empty() && !self.rec.sinks.iter().any(|s| s.name == self.rec.face.output);
        if missing {
            ui.label(egui::RichText::new(format!("⚠ Output {} not found; playing to the system default.", self.rec.face.output)).color(caution()));
        } else if !self.rec.output_note.is_empty() {
            ui.label(egui::RichText::new(&self.rec.output_note).color(caution()));
        }
    }

    /// Record / Playback dialog: follows the selection, one at a time.
    fn rec_detail_window(&mut self, ctx: &egui::Context) {
        if !self.rec.detail_open {
            return;
        }
        let Some(id) = self.rec.selected else {
            self.rec.detail_open = false;
            return;
        };
        let Some(rec) = self.rec.rows.iter().find(|r| r.id == id).cloned() else {
            self.rec.selected = None;
            self.rec.detail_open = false;
            return;
        };
        let mut open = true;
        // Fixed title = fixed id: egui keeps its position for the session while the
        // contents follow the selection.
        egui::Window::new("Record / Playback")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.set_max_height(ctx.content_rect().height() * 0.8);
                self.rec_detail(ui, rec);
            });
        if !open {
            self.rec.detail_open = false;
        }
    }

    fn rec_detail(&mut self, ui: &mut egui::Ui, rec: Recording) {
        let id = rec.id;
        let local = |utc: &str| {
            chrono::DateTime::parse_from_rfc3339(utc)
                .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|_| if utc.is_empty() { "recording…".into() } else { utc.to_string() })
        };
        // Header
        ui.heading(rec.meta.lane());
        ui.label(format!(
            "{} → {}   {:.1} s",
            local(&rec.meta.start_utc),
            local(&rec.meta.end_utc),
            rec.meta.duration_ms as f64 / 1000.0
        ));
        ui.separator();
        // Playback
        ui.strong("Playback");
        let mut star = rec.star;
        let mut protect = rec.protected;
        ui.horizontal_wrapped(|ui| {
            let playing = self.rec.player.as_ref().is_some_and(|(pid, _)| *pid == id);
            if playing {
                if ui.button("■ Stop").clicked() {
                    self.rec.player = None;
                }
            } else if ui.button("▶ Play").on_hover_text("Re-demodulates the raw IQ now, through the current decoder.").clicked() {
                self.rec_play(id, 0);
            }
            ui.checkbox(&mut star, "★ Star");
            ui.checkbox(&mut protect, "🔒 Protect").on_hover_text("Never removed by overwrite-oldest.");
        });
        self.rec_output_row(ui);
        ui.horizontal_wrapped(|ui| {
            let red = egui::Color32::from_rgb(240, 60, 50);
            if !hsrec::dsp::in_capture(rec.meta.offset_hz) && rec.meta.source_kind != meta::KIND_DEMOD_AUDIO {
                ui.label(egui::RichText::new("OUT OF BAND").small().strong().color(red));
            }
            if self.rec.no_voice.contains(&id) {
                ui.label(egui::RichText::new("no voice").small().strong().color(caution()));
            }
            if rec.meta.emergency { ui.label(egui::RichText::new("EMERGENCY").small().strong().color(red)); }
            if rec.meta.priority { ui.label(egui::RichText::new("PRIORITY").small()); }
            if rec.archived { ui.label(egui::RichText::new("archived (zst)").small()); }
            if rec.offline() { ui.label(egui::RichText::new("offline").small()); }
            if rec.missing { ui.label(egui::RichText::new("MISSING").small().color(red)); }
        });
        // One status line: the latest transport / action message.
        if !self.rec.note.is_empty() {
            ui.label(egui::RichText::new(&self.rec.note).color(caution()));
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Notes");
            let r = ui.add(egui::TextEdit::singleline(&mut self.rec.notes_edit).desired_width(f32::INFINITY));
            if r.lost_focus() && self.rec.notes_edit != rec.notes {
                if let Some(db) = &self.rec.db {
                    let _ = db.set_notes(id, &self.rec.notes_edit);
                }
                self.rec_reload(true);
            }
        });
        let mut delete = false;
        ui.horizontal_wrapped(|ui| {
            if ui.button("Export WAV…").on_hover_text("Demodulates now and writes a WAV. The WAV is not kept by REC.").clicked() {
                self.rec_export_wav(&rec);
            }
            if ui.button("Export raw…").on_hover_text("Raw IQ (or archive) plus the JSON sidecar.").clicked() {
                self.rec_export_raw(&rec);
            }
            if rec.archived {
                if ui.add_enabled(self.rec.job.is_none(), egui::Button::new("Unarchive")).clicked() {
                    let dir = self.rec.dir.clone();
                    let one = rec.clone();
                    self.rec_job("Unarchiving", move |db, st, _| {
                        let r = hsrec::archive::unarchive_one(&db, &dir, &one);
                        st.lock().expect("job").lines.push(r.err().unwrap_or_else(|| "Unarchived.".into()));
                    });
                    return;
                }
            } else if ui.add_enabled(self.rec.job.is_none() && !rec.meta.end_utc.is_empty(), egui::Button::new("Archive")).clicked() {
                self.rec_start_archive(vec![rec.clone()]);
            }
            if ui.button("Delete").on_hover_text("Removes the raw file, its row, and its cache. Undo for 60 s.").clicked() {
                delete = true;
            }
        });
        ui.separator();
        // Metadata scrolls on its own so the block above never leaves view.
        egui::ScrollArea::vertical().id_salt(("rec-meta-scroll", id)).auto_shrink([false, true]).show(ui, |ui| {
        let rows = self.rec_meta_rows(&rec);
        ui.horizontal(|ui| {
            ui.strong("Metadata");
            if ui.small_button("📋 Copy").on_hover_text("Copy all metadata and decoder parameters").clicked() {
                let mut text: Vec<String> = vec![rec.meta.lane()];
                text.extend(rows.iter().map(|(k, v)| format!("{k}: {v}")));
                text.extend(rec.meta.decoder_params.iter().map(|(k, v)| format!("{k}={v}")));
                ui.ctx().copy_text(text.join("\n"));
            }
        });
        egui::Grid::new(("rec-meta", id)).num_columns(2).striped(true).spacing([12.0, 2.0]).show(ui, |ui| {
            for (k, v) in &rows {
                ui.label(egui::RichText::new(*k).weak());
                ui.add(egui::Label::new(v).wrap());
                ui.end_row();
            }
        });
        if !rec.meta.decoder_params.is_empty() {
            egui::CollapsingHeader::new(format!("Decoder parameters ({})", rec.meta.decoder_params.len()))
                .id_salt("rec-decoder-params")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Replay passes these to the decoder script as environment variables.").small().weak());
                    egui::Grid::new(("rec-params", id)).num_columns(2).striped(true).show(ui, |ui| {
                        for (k, v) in &rec.meta.decoder_params {
                            ui.monospace(k);
                            ui.monospace(v);
                            ui.end_row();
                        }
                    });
                });
        }
        });
        let mut changed = false;
        if let Some(db) = &self.rec.db {
            if star != rec.star {
                let _ = db.set_flag(id, "star", star);
                changed = true;
            }
            if protect != rec.protected {
                let _ = db.set_flag(id, "protected", protect);
                changed = true;
            }
        }
        if changed {
            self.rec_reload(true);
        }
        if delete {
            self.rec_delete(&rec);
        }
    }

    fn rec_delete(&mut self, rec: &Recording) {
        if rec.meta.end_utc.is_empty() {
            self.rec.note = "Still recording; delete it after it ends.".into();
            return;
        }
        if let Some((_, trash, _)) = self.rec.undo.take() {
            let _ = std::fs::remove_dir_all(trash);
        }
        if self.rec.player.as_ref().is_some_and(|(id, _)| *id == rec.id) {
            self.rec.player = None;
        }
        let trash = self.rec.dir.join(".trash").join(rec.id.to_string());
        let _ = std::fs::create_dir_all(&trash);
        let raw = self.rec.dir.join(&rec.raw_path);
        let side = hsrec::storage::sidecar_for(&raw);
        let _ = std::fs::rename(&raw, trash.join("raw"));
        let _ = std::fs::rename(&side, trash.join("side"));
        if let Some(db) = &self.rec.db {
            let _ = db.delete(rec.id);
        }
        for kind in ["waveform"] {
            let _ = std::fs::remove_file(hsrec::cache::entry_path(&self.rec.dir, rec.id, kind, &hsrec::cache::decoder_version()));
        }
        self.rec.waveforms.remove(&rec.id);
        self.rec.undo = Some((rec.clone(), trash, Instant::now()));
        self.rec.selected = None;
        self.rec.note = format!("Deleted {}. Undo is in the toolbar for 60 s.", rec.meta.lane());
        self.rec_reload(true);
    }

    fn rec_undo(&mut self) {
        let Some((rec, trash, _)) = self.rec.undo.take() else { return };
        let raw = self.rec.dir.join(&rec.raw_path);
        let _ = std::fs::rename(trash.join("raw"), &raw);
        let _ = std::fs::rename(trash.join("side"), hsrec::storage::sidecar_for(&raw));
        let _ = std::fs::remove_dir_all(&trash);
        if let Some(db) = &self.rec.db {
            let _ = db.restore(&rec);
        }
        self.rec.note = "Delete undone.".into();
        self.rec_reload(true);
    }

    fn rec_raw_path(&self, rec: &Recording) -> Option<PathBuf> {
        if rec.offline() {
            hsrec::export::locate(rec, &self.rec.drives)
        } else {
            Some(self.rec.dir.join(&rec.raw_path))
        }
    }

    fn rec_export_wav(&mut self, rec: &Recording) {
        let Some(src) = self.rec_raw_path(rec) else { return };
        let name = format!("{}.wav", Path::new(&rec.raw_path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "rec".into()).trim_end_matches(".cs16"));
        let Some(dst) = rfd::FileDialog::new().set_file_name(&name).add_filter("WAV", &["wav"]).save_file() else { return };
        let meta = rec.meta.clone();
        // Analog: discriminator audio. Digital: the discriminator stream the
        // decoder reads (speech decode for WAV needs the station decoder).
        std::thread::spawn(move || {
            if let Ok(audio) = hsrec::replay::decode_all(&src, &meta) {
                let _ = hsrec::replay::write_wav(&dst, &audio, hsrec::replay::OUT_RATE);
            }
        });
        self.rec.note = "Writing WAV in the background.".into();
    }

    fn rec_export_raw(&mut self, rec: &Recording) {
        let Some(src) = self.rec_raw_path(rec) else { return };
        let Some(folder) = rfd::FileDialog::new().pick_folder() else { return };
        let side = hsrec::storage::sidecar_for(&src);
        let mut ok = src.file_name().map(|n| std::fs::copy(&src, folder.join(n)).is_ok()).unwrap_or(false);
        if side.exists() {
            ok &= side.file_name().map(|n| std::fs::copy(&side, folder.join(n)).is_ok()).unwrap_or(false);
        }
        self.rec.note = if ok { format!("Raw IQ and sidecar copied to {}.", folder.display()) } else { "Raw export failed.".into() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_order_and_caps() {
        use BarAction::*;
        for compact in [false, true] {
            let a: Vec<BarAction> = bar_segments(compact).iter().map(|s| s.action).collect();
            assert_eq!(a, [DayBack, Today, DayForward, ZoomOut, ZoomIn, Fit, Live, Data, PlayNext]);
        }
        assert_eq!(bar_segments(false)[6].label, "LIVE");
        assert_eq!(bar_segments(false)[8].label, PLAY_NEXT_LABEL);
    }

    #[test]
    fn bar_widths_and_compact_switch() {
        assert!(bar_width(true) < bar_width(false));
        let full: f32 = bar_segments(false).iter().map(|s| s.width).sum::<f32>() + 8.0 * BAR_GAP;
        assert_eq!(bar_width(false), full);
        assert!(bar_segments(true).iter().all(|s| s.width >= 36.0));
        assert!(!bar_compact(2000.0));
        assert!(bar_compact(800.0));
        let edge = bar_width(false) + BAR_GAP + BAR_TABS_MIN;
        assert!(!bar_compact(edge));
        assert!(bar_compact(edge - 1.0));
    }

    #[test]
    fn bar_toggles_show_state_in_text() {
        let live = bar_segments(true)[6];
        assert_eq!(bar_label(&live, Some(true)), "● L");
        assert_eq!(bar_label(&live, Some(false)), "○ L");
        assert_eq!(bar_label(&bar_segments(false)[1], None), "TODAY");
    }

    #[test]
    fn bar_goes_in_the_elbow_then_compact_then_tab_row() {
        let need = |c| ELBOW_CURVE_W + END_CAP_W + bar_slot_width(c) + BAR_TAN_MIN;
        assert_eq!(bar_place(1264.0, 1280.0), BarPlace::InFrame { compact: false });
        assert_eq!(bar_place(need(false), 0.0), BarPlace::InFrame { compact: false });
        assert_eq!(bar_place(need(false) - 1.0, 0.0), BarPlace::InFrame { compact: true });
        assert_eq!(bar_place(need(true), 0.0), BarPlace::InFrame { compact: true });
        assert!(matches!(bar_place(need(true) - 1.0, 2000.0), BarPlace::TabRow { compact: false }));
        assert!(matches!(bar_place(300.0, 300.0), BarPlace::TabRow { compact: true }));
        assert_eq!(bar_slot_width(false), bar_width(false) + 8.0);
        assert!(bar_hint(BarAction::Data, Some(false)).ends_with("(OFF)"));
        assert!(bar_shown(true) && !bar_shown(false));
    }

    #[test]
    fn toggle_text_marks_state_without_colour() {
        assert_eq!(toggle_text(true, LIVE_LABEL), "● Live");
        assert_eq!(toggle_text(false, LIVE_LABEL), "○ Live");
        assert_eq!(toggle_text(true, PLAY_NEXT_LABEL), "● ▶▶");
    }

    #[test]
    fn toolbar_order_live_data_play_next() {
        assert_eq!(TOOLBAR_ORDER, ["Live", "Data", "▶▶"]);
        assert!(play_next_hint(true).contains("ON"));
        assert!(play_next_hint(false).contains("OFF"));
    }

    #[test]
    fn icon_three_states() {
        assert_eq!(icon_state(false, false), IconState::Off);
        assert_eq!(icon_state(true, false), IconState::Armed);
        assert_eq!(icon_state(true, true), IconState::Writing);
        assert_eq!(icon_color(IconState::Off), egui::Color32::BLACK);
        assert_ne!(icon_color(IconState::Armed), icon_color(IconState::Writing));
        // The row dot fits inside the gutter, left of the row text.
        assert!(ROW_DOT_X + ROW_DOT_RADIUS < ROW_GUTTER);
        assert!(ROW_DOT_X - ROW_DOT_RADIUS > 3.0);
        assert!(ROW_DOT_COLOR.r() < 200);
        assert_eq!(row_hover(true), ROW_MARKED_HOVER);
        assert_eq!(row_hover(false), ROW_HOVER);
        // Writing is a modest brightening of armed, not a glare.
        let (a, w) = (icon_color(IconState::Armed), icon_color(IconState::Writing));
        assert!(w.r() > a.r() && w.r() < 220);
        // The glyph stays inside the speaker emoji's slot.
        assert!(2.0 * (TAB_GLYPH_RING + TAB_GLYPH_STROKE) <= TAB_GLYPH_SLOT * 0.6);
        assert!(icon_text(IconState::Writing).contains("recording"));
    }

    #[test]
    fn wheel_zoom_is_fine_clamped_and_anchored() {
        let notch = wheel_zoom_factor(-NOTCH_PTS);
        assert!((notch - 1.18).abs() < 1e-9);
        assert!((wheel_zoom_factor(NOTCH_PTS) * notch - 1.0).abs() < 1e-9);
        assert!((wheel_zoom_factor(-4.0) - 1.0).abs() < 0.02);
        assert_eq!(wheel_zoom_factor(-10_000.0), MAX_FRAME_ZOOM);
        assert_eq!(wheel_zoom_factor(10_000.0), 1.0 / MAX_FRAME_ZOOM);
        assert_eq!(wheel_zoom_factor(f32::NAN), 1.0);
        let mut v = View { start: 1_000_000, span: 60_000, live: false };
        let (w, x) = (1000.0, 250.0);
        let before = v.t(x, w);
        v.zoom(wheel_zoom_factor(NOTCH_PTS), x, w);
        assert!((v.t(x, w) - before).abs() <= 1);
        assert!(v.span < 60_000);
        for _ in 0..200 { v.zoom(wheel_zoom_factor(NOTCH_PTS), x, w); }
        assert_eq!(v.span, timeline::MIN_SPAN_MS);
    }

    #[test]
    fn lane_rows_grow_to_fill_but_cap() {
        assert_eq!(lane_row_h(0, 500.0), LANE_H);
        assert_eq!(lane_row_h(4, 500.0), LANE_H * LANE_GROW_MAX);
        assert_eq!(lane_row_h(100, 500.0), LANE_H);
        let h = lane_row_h(15, 500.0);
        assert!(h > LANE_H && h < LANE_H * LANE_GROW_MAX && (h * 15.0 - 500.0).abs() < 0.01);
        assert_eq!(lane_row_h(3, f32::INFINITY), LANE_H);
    }

    #[test]
    fn timeline_fills_free_height_with_a_floor() {
        assert_eq!(timeline_viewport_h(800.0), 800.0 - AXIS_H - TIMELINE_RESERVE);
    }

    #[test]
    fn lane_labels_cap_halfway_and_fade_softly() {
        assert_eq!(lane_label_cap(800.0, 760.0), 400.0);
        assert_eq!(lane_label_cap(100.0, 30.0), 30.0);
        assert_eq!(lane_label_cap(f32::NAN, 30.0), 0.0);
        assert_eq!(lane_label_alpha(10.0, 400.0), 1.0);
        assert_eq!(lane_label_alpha(400.0 - LABEL_FADE, 400.0), 1.0);
        assert!((lane_label_alpha(400.0 - LABEL_FADE / 2.0, 400.0) - 0.5).abs() < 1e-6);
        assert_eq!(lane_label_alpha(400.0, 400.0), 0.0);
        assert_eq!(lane_label_alpha(500.0, 400.0), 0.0);
    }

    #[test]
    fn now_tint_is_a_steep_translucent_highlight() {
        assert_eq!(now_tint_alpha(0.0), 0.0);
        assert!((now_tint_alpha(1.0) - NOW_TINT_PEAK).abs() < 1e-6);
        assert!(NOW_TINT_PEAK > 0.35 && NOW_TINT_PEAK < 0.5);
        assert!(now_tint_alpha(0.5) < NOW_TINT_PEAK * 0.2);
        let mut prev = -1.0;
        for k in 0..=20 {
            let a = now_tint_alpha(k as f32 / 20.0);
            assert!(a > prev || (k == 0 && a == 0.0));
            prev = a;
        }
        assert_eq!(now_tint_alpha(f32::NAN), 0.0);
        assert_eq!(now_tint_alpha(2.0), NOW_TINT_PEAK);
    }

    #[test]
    fn now_fade_ends_at_the_line_and_gutter_is_excluded() {
        assert_eq!(now_fade_span(300.0, 500.0, 40.0), Some((260.0, 300.0)));
        assert_eq!(now_fade_span(500.0, 500.0, 40.0), Some((460.0, 500.0)));
        assert_eq!(now_fade_span(20.0, 500.0, 40.0), Some((0.0, 20.0)));
        assert_eq!(now_fade_span(501.0, 500.0, 40.0), None);
        assert_eq!(now_fade_span(-1.0, 500.0, 40.0), None);
        assert_eq!(now_fade_span(f32::NAN, 500.0, 40.0), None);
        assert_eq!(plot_width(800.0), 800.0 - SCROLL_GUTTER);
        assert_eq!(plot_width(5.0), 1.0);
    }

    #[test]
    fn right_shelf_is_one_and_a_half_labels_clamped() {
        assert_eq!(right_shelf_w(18.0), 40.0);
        assert_eq!(right_shelf_w(40.0), 60.0);
        assert_eq!(right_shelf_w(200.0), 90.0);
        assert_eq!(right_shelf_w(f32::NAN), 40.0);
        let r = right_shelf_rect(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(500.0, 300.0)), 40.0);
        assert_eq!((r.left(), r.right(), r.height()), (440.0, 500.0, 300.0));
        assert!(timeline_viewport_h(1200.0) > timeline_viewport_h(800.0));
        assert_eq!(timeline_viewport_h(100.0), LANE_H * TIMELINE_MIN_LANES);
        assert_eq!(timeline_viewport_h(f32::INFINITY), LANE_H * 12.0);
    }

    #[test]
    fn pointer_owner_is_top_layer_only() {
        let bg = egui::LayerId::background();
        let win = egui::LayerId::new(egui::Order::Middle, egui::Id::NULL);
        assert!(pointer_is_ours(Some(bg), bg));
        assert!(!pointer_is_ours(Some(win), bg));
        assert!(!pointer_is_ours(None, bg));
    }

    #[test]
    fn search_width_fits_narrow_panels() {
        assert_eq!(search_width(1000.0), 120.0);
        assert_eq!(search_width(400.0), 80.0);
        assert_eq!(search_width(3000.0), 140.0);
    }

    #[test]
    fn key_lamps() {
        assert!(matches!(key_lamp(RecMode::Off), KeyLamp::Dark));
        assert!(matches!(key_lamp(RecMode::Marked), KeyLamp::Red));
        assert!(matches!(key_lamp(RecMode::All), KeyLamp::Amber));
    }

    #[test]
    fn page_name_round_trip() {
        assert_eq!(super::super::page_name(super::super::Page::Rec), "REC");
        assert!(super::super::page_from_name("REC") == super::super::Page::Rec);
    }

    #[test]
    fn marks_round_trip_and_old_files_load_unmarked() {
        let tg: crate::talkgroups::Talkgroup = serde_json::from_str(r#"{"name":"Fire","tg":101}"#).unwrap();
        assert!(!tg.record);
        let mut tg = tg;
        tg.record = true;
        let text = serde_json::to_string(&tg).unwrap();
        assert!(text.contains("\"record\":true"));
        tg.record = false;
        assert!(!serde_json::to_string(&tg).unwrap().contains("record"));
        let ch: crate::channels::Channel = serde_json::from_str(r#"{"name":"Ops","hz":155000000.0}"#).unwrap();
        assert!(!ch.record);
    }

    #[test]
    fn rec_face_persists_modes() {
        let mut f = config::RecFace::default();
        assert!(f.is_default());
        f.talkgroups = RecMode::Marked;
        f.channels = RecMode::All;
        let text = serde_json::to_string(&f).unwrap();
        let back: config::RecFace = serde_json::from_str(&text).unwrap();
        assert_eq!(back, f);
        assert!(back.armed());
        let old: config::RecFace = serde_json::from_str("{}").unwrap();
        assert_eq!(old.cap_gb, 20.0);
    }
}

/// Lane shelf: theme-dark fill, opaque on the left and fading to clear on the
/// right (see `timeline::shelf_alpha`).
/// Search box width in the timeline toolbar: narrow enough that the
/// toolbar fits a ~1000 px panel on one line.
/// Text for a toolbar toggle: the state is shown by a marker, never by colour alone.
/// REC toolbar labels, left to right: Live, Data, then play-next.
#[cfg_attr(not(test), allow(dead_code))]
pub const LIVE_LABEL: &str = "Live";
#[cfg_attr(not(test), allow(dead_code))]
pub const DATA_LABEL: &str = "Data";
/// Play next. "▶" is already used elsewhere on this page, so it renders in
/// egui's default fonts; "⏭" (U+23ED) is not reliably covered.
pub const PLAY_NEXT_LABEL: &str = "▶▶";
#[cfg_attr(not(test), allow(dead_code))]
pub const TOOLBAR_ORDER: [&str; 3] = [LIVE_LABEL, DATA_LABEL, PLAY_NEXT_LABEL];

pub fn play_next_hint(on: bool) -> String {
    format!("Play next: {}. When a recording finishes playing, play the following one.", if on { "ON" } else { "OFF" })
}

/// One control in the REC run of the header bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarAction {
    DayBack,
    Today,
    DayForward,
    ZoomOut,
    ZoomIn,
    Fit,
    Live,
    Data,
    PlayNext,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarSeg {
    pub action: BarAction,
    pub label: &'static str,
    pub width: f32,
}

/// Gap between header-bar segments (rustcars `frame_gap`).
pub const BAR_GAP: f32 = 4.0;
/// Below this much room for the tabs, the REC controls go compact.
pub const BAR_TABS_MIN: f32 = 420.0;

/// Left to right: day back, today, day forward, zoom out, zoom in, fit,
/// then Live, Data and play-next. Compact drops the words.
pub fn bar_segments(compact: bool) -> Vec<BarSeg> {
    use BarAction::*;
    let full: [(BarAction, &str, &str, f32, f32); 9] = [
        (DayBack, "◀ DAY", "◀", 76.0, 40.0),
        (Today, "TODAY", "T", 72.0, 36.0),
        (DayForward, "DAY ▶", "▶", 76.0, 40.0),
        (ZoomOut, "−", "−", 40.0, 36.0),
        (ZoomIn, "+", "+", 40.0, 36.0),
        (Fit, "FIT", "FIT", 56.0, 44.0),
        (Live, "LIVE", "L", 84.0, 48.0),
        (Data, "DATA", "D", 84.0, 48.0),
        (PlayNext, PLAY_NEXT_LABEL, PLAY_NEXT_LABEL, 68.0, 60.0),
    ];
    full.iter()
        .map(|(a, l, c, w, cw)| if compact { BarSeg { action: *a, label: c, width: *cw } } else { BarSeg { action: *a, label: l, width: *w } })
        .collect()
}

/// Total width of the REC run, gaps included.
pub fn bar_width(compact: bool) -> f32 {
    let segs = bar_segments(compact);
    segs.iter().map(|s| s.width).sum::<f32>() + BAR_GAP * (segs.len() as f32 - 1.0)
}

/// Compact when the full run would leave the tabs less than `BAR_TABS_MIN`.
pub fn bar_compact(row_w: f32) -> bool {
    row_w - bar_width(false) - BAR_GAP < BAR_TABS_MIN
}

/// Minimum tan left showing in the top elbow bar beside the controls.
pub const BAR_TAN_MIN: f32 = 80.0;
/// Horizontal room the top elbow's curve takes before its straight run
/// (rustcars: max(thick + r_in, 2 * r_out) at scale 1).
pub const ELBOW_CURVE_W: f32 = 180.0;
/// rustcars LcarsFrame end cap length.
pub const END_CAP_W: f32 = 48.0;
/// The central panel's left + right margin around the frame.
pub const FRAME_MARGIN_X: f32 = 16.0;

/// Where the REC controls go this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarPlace {
    /// Inside the top elbow bar, in a gap opened before its end cap.
    InFrame { compact: bool },
    /// Fallback: the right end of the page-tab row.
    TabRow { compact: bool },
}

/// Slot width the controls need in the elbow bar: the run plus a
/// `BAR_GAP` of black on each side.
pub fn bar_slot_width(compact: bool) -> f32 {
    bar_width(compact) + 2.0 * BAR_GAP
}

/// Full run in the elbow bar if it leaves `BAR_TAN_MIN` of tan, else the
/// compact run, else back to the tab row. `frame_w` is the frame's width,
/// `row_w` the page-tab row's.
pub fn bar_place(frame_w: f32, row_w: f32) -> BarPlace {
    let run = frame_w - ELBOW_CURVE_W - END_CAP_W;
    for compact in [false, true] {
        if run - bar_slot_width(compact) >= BAR_TAN_MIN {
            return BarPlace::InFrame { compact };
        }
    }
    BarPlace::TabRow { compact: bar_compact(row_w) }
}

/// Only the REC page carries the run.
pub fn bar_shown(rec_page: bool) -> bool {
    rec_page
}

pub fn bar_label(seg: &BarSeg, on: Option<bool>) -> String {
    match on {
        Some(on) => toggle_text(on, seg.label),
        None => seg.label.to_string(),
    }
}

fn bar_colour(action: BarAction) -> egui::Color32 {
    use BarAction::*;
    match action {
        DayBack | Today | DayForward => rustcars::palette::SLATE,
        ZoomOut | ZoomIn | Fit => rustcars::palette::TEAL,
        Live => rustcars::palette::BRICK,
        Data => rustcars::palette::OLIVE,
        PlayNext => rustcars::palette::PLUM,
    }
}

pub fn bar_hint(action: BarAction, on: Option<bool>) -> String {
    use BarAction::*;
    let state = match on {
        Some(true) => " (ON)",
        Some(false) => " (OFF)",
        None => "",
    };
    let text = match action {
        DayBack => "Previous day".to_string(),
        Today => "Today".to_string(),
        DayForward => "Next day".to_string(),
        ZoomOut => "Zoom out".to_string(),
        ZoomIn => "Zoom in".to_string(),
        Fit => "Fit to the most recent recordings".to_string(),
        Live => "Live: follow the present (the red NOW line)".to_string(),
        Data => "Data: storage cap, archive, rescan, cache, and thumb-drive copy/move".to_string(),
        PlayNext => return play_next_hint(on == Some(true)),
    };
    format!("{text}{state}")
}

pub fn toggle_text(on: bool, label: &str) -> String {
    format!("{} {label}", if on { "●" } else { "○" })
}

/// The timeline handles pointer input only when its own layer is the top
/// layer under the pointer (not a floating REC window above it).
pub fn pointer_is_ours(top: Option<egui::LayerId>, own: egui::LayerId) -> bool {
    top == Some(own)
}

pub fn search_width(avail: f32) -> f32 {
    (avail * 0.12).clamp(80.0, 140.0)
}

/// Lane viewport height: all the free panel height above the reserved
/// details block, with a floor so it never collapses.
pub fn timeline_viewport_h(avail: f32) -> f32 {
    let min = LANE_H * TIMELINE_MIN_LANES;
    if !avail.is_finite() {
        return LANE_H * 12.0;
    }
    (avail - AXIS_H - TIMELINE_RESERVE).max(min)
}

/// One toolbar −/+ step.
pub const BUTTON_ZOOM: f64 = 1.5;
/// Points of wheel delta in one mouse notch (egui's line scroll speed).
const NOTCH_PTS: f32 = 40.0;
/// Span change per notch (18 %).
const NOTCH_ZOOM: f64 = 1.18;
/// Largest span change from one frame's wheel input.
const MAX_FRAME_ZOOM: f64 = 1.5;

/// Span multiplier for a wheel delta (points, + = away from you = zoom in).
/// Exponential, so touchpad deltas zoom proportionally finer; clamped so a
/// burst of notches never jumps more than MAX_FRAME_ZOOM in one frame.
pub fn wheel_zoom_factor(delta_y: f32) -> f64 {
    if !delta_y.is_finite() {
        return 1.0;
    }
    let k = NOTCH_ZOOM.ln() / NOTCH_PTS as f64;
    (-(delta_y as f64) * k).exp().clamp(1.0 / MAX_FRAME_ZOOM, MAX_FRAME_ZOOM)
}

/// Lane row height: the base height, grown to fill the viewport when there
/// are few rows (capped at LANE_GROW_MAX x), so the area looks filled.
pub fn lane_row_h(rows: usize, viewport_h: f32) -> f32 {
    if rows == 0 || !viewport_h.is_finite() {
        return LANE_H;
    }
    (viewport_h / rows as f32).clamp(LANE_H, LANE_H * LANE_GROW_MAX)
}

/// Left padding of lane names inside the shelf.
const LABEL_PAD: f32 = 6.0;
/// Width over which a long lane name fades out before its cap.
const LABEL_FADE: f32 = 48.0;

/// Where lane names stop, measured from the timeline's left edge: halfway
/// across the lane, and never into the right-hand NOW shelf.
pub fn lane_label_cap(lane_w: f32, right_shelf_left: f32) -> f32 {
    if !lane_w.is_finite() || lane_w <= 0.0 {
        return 0.0;
    }
    (lane_w * 0.5).min(right_shelf_left.max(0.0))
}

/// Alpha (0..1) of lane-name text at `x` (from the timeline's left edge):
/// full until `LABEL_FADE` before `cap`, then a linear fade to 0 at `cap`.
pub fn lane_label_alpha(x: f32, cap: f32) -> f32 {
    if !(x.is_finite() && cap.is_finite()) {
        return 0.0;
    }
    ((cap - x) / LABEL_FADE).clamp(0.0, 1.0)
}

/// One lane name as a per-character coloured job: full ink, fading only in
/// the last `LABEL_FADE` before `cap`. Characters past `cap` are dropped.
fn lane_label_job(ui: &egui::Ui, name: &str, font: &egui::FontId, ink: egui::Color32, pad: f32, cap: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let mut buf = [0u8; 4];
    let mut x = pad;
    for c in name.chars() {
        let w = ui.fonts_mut(|f| f.glyph_width(font, c));
        let mid = x + w / 2.0;
        if mid >= cap {
            break;
        }
        let a = lane_label_alpha(mid, cap);
        job.append(c.encode_utf8(&mut buf), 0.0, egui::TextFormat::simple(font.clone(), ink.gamma_multiply(a)));
        x += w;
    }
    job
}

/// Dark red of the right-hand NOW shelf (kin to the armed REC icon's red).
const NOW_SHELF: egui::Color32 = egui::Color32::from_rgb(120, 16, 12);

/// Free strip at the right of the lane area for the lane scrollbar.
pub const SCROLL_GUTTER: f32 = 14.0;

/// Plot (axis + lanes) width: the available width minus the scrollbar gutter.
pub fn plot_width(avail: f32) -> f32 {
    if !avail.is_finite() {
        return 1.0;
    }
    (avail - SCROLL_GUTTER).max(1.0)
}

/// Span (x0, x1) of the NOW tint relative to the plot's left edge: it ends
/// exactly at the NOW line and reaches `w` to its left. None when NOW is
/// outside the visible plot.
pub fn now_fade_span(now_x: f32, plot_w: f32, w: f32) -> Option<(f32, f32)> {
    if !(now_x.is_finite() && plot_w.is_finite()) || now_x < 0.0 || now_x > plot_w {
        return None;
    }
    Some(((now_x - w).max(0.0), now_x))
}

/// Right shelf width: 1.5x the NOW label, clamped to 40..90 px.
pub fn right_shelf_w(label_w: f32) -> f32 {
    if !label_w.is_finite() {
        return 40.0;
    }
    (label_w * 1.5).clamp(40.0, 90.0)
}

/// Full-height strip at the right edge of `area`, `right_shelf_w` wide.
#[cfg(test)]
fn right_shelf_rect(area: egui::Rect, label_w: f32) -> egui::Rect {
    let w = right_shelf_w(label_w).min(area.width());
    egui::Rect::from_min_max(egui::pos2(area.right() - w, area.top()), area.max)
}

/// Mirror of `paint_shelf`: opaque at the right edge, fading out leftward.
fn paint_shelf_right(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    // Translucent red highlight: eased (cubic) from clear on the left to
    // NOW_TINT_PEAK at the right edge, so what is underneath shows through.
    const STEPS: usize = 16;
    let mut mesh = egui::Mesh::default();
    for k in 0..=STEPS {
        let t = k as f32 / STEPS as f32;
        let x = rect.left() + rect.width() * t;
        let a = (now_tint_alpha(t) * 255.0).round() as u8;
        let c = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a);
        mesh.colored_vertex(egui::pos2(x, rect.top()), c);
        mesh.colored_vertex(egui::pos2(x, rect.bottom()), c);
        if k > 0 {
            let b = (2 * k) as u32;
            mesh.add_triangle(b - 2, b - 1, b);
            mesh.add_triangle(b - 1, b + 1, b);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Peak alpha (0..1) of the NOW tint at the right edge (~45 %).
pub const NOW_TINT_PEAK: f32 = 115.0 / 255.0;

/// NOW tint alpha at `t` across the strip (0 = left end, 1 = right edge):
/// cubic ease-in, so most of the width is nearly clear.
pub fn now_tint_alpha(t: f32) -> f32 {
    if !t.is_finite() {
        return 0.0;
    }
    let t = t.clamp(0.0, 1.0);
    NOW_TINT_PEAK * t * t * t
}

fn paint_shelf(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let solid_w = rect.width() * timeline::SHELF_OPAQUE_FRAC;
    let solid = egui::Rect::from_min_size(rect.min, egui::vec2(solid_w, rect.height()));
    painter.rect_filled(solid, egui::CornerRadius::ZERO, color);
    let mut mesh = egui::Mesh::default();
    let clear = egui::Color32::TRANSPARENT;
    let (x0, x1, y0, y1) = (solid.right(), rect.right(), rect.top(), rect.bottom());
    mesh.colored_vertex(egui::pos2(x0, y0), color);
    mesh.colored_vertex(egui::pos2(x1, y0), clear);
    mesh.colored_vertex(egui::pos2(x1, y1), clear);
    mesh.colored_vertex(egui::pos2(x0, y1), color);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}
