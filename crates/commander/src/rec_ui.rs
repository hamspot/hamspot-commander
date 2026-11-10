//! REC (#119) on the face: right-click marks, REC keys, the REC page tab
//! icon, source tabs, the timeline, replay, archive and export.

//!
//! HackRF sources record raw channelised IQ (32 ksps cs16) and every
//! playback re-runs it through the current discriminator and decoder.
//! Hamlib and GQRX record the rig's own 48 kHz demod audio
//! (`source_kind = demod_audio`). Encrypted traffic is never recorded.
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
use hsrec::segment::{self, Action, CallKey, Press, RecMode, Segmenter};
use hsrec::timeline::{self, View};
use hsrec::writer::Cmd;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn rig_driver(driver: cat::RadioDriver) -> bool {
    matches!(driver, cat::RadioDriver::Hamlib | cat::RadioDriver::Gqrx)
}

pub const ROW_HOVER: &str = "Right-click: mark or unmark for recording (● REC).";
/// Shown only as the REC page watermark on the bottom elbow bar.
pub const PRIVACY: &str = "Recording radio traffic may be regulated where you are; that is your responsibility. \
Encrypted traffic is never recorded. Nothing is uploaded.";
/// Small gap kept below the lanes. The status strip is outside the timeline's
/// area (the page splits it off first). The disclaimer is a watermark on the
/// frame's bottom bar. The selected recording's details live in the Record /
/// Playback dialog, so nothing else is reserved.
const TIMELINE_RESERVE: f32 = 4.0;
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
    /// The recorder cannot record (database or folder problem).
    Error,
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
        IconState::Error => egui::Color32::from_rgb(230, 140, 0),
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
        IconState::Error => "REC error: NOT recording (see the REC page)",
    }
}

/// Honest REC state: writing only when the recorder has its database open
/// and is writing; any open/create error wins over the armed keys.
pub fn rec_state(armed: bool, ready: bool, writing: bool, error: Option<&str>) -> IconState {
    if error.is_some_and(|e| !e.is_empty()) {
        IconState::Error
    } else if !armed {
        IconState::Off
    } else if ready && writing {
        IconState::Writing
    } else {
        IconState::Armed
    }
}

/// One line for the REC page and bar. Text carries the state, not colour.
pub fn status_line(state: IconState, ready: bool, error: Option<&str>) -> String {
    match state {
        IconState::Error => format!(
            "\u{26a0} REC is NOT recording: {}. Check free space and that the folder beside the station file is writable, then press REC again. A damaged or newer database is left untouched; move it aside to start fresh.",
            error.unwrap_or("unknown error")
        ),
        IconState::Off => "REC off: nothing is being recorded.".into(),
        IconState::Armed if !ready => "REC armed: starting the recorder\u{2026}".into(),
        IconState::Armed => "REC armed: recorder ready, waiting for traffic.".into(),
        IconState::Writing => "REC: recording now.".into(),
    }
}

/// What an empty REC page says, so a blank window still gives hints.
pub fn empty_hint(tg: RecMode, n_tg: usize, ch: RecMode, n_ch: usize, ready: bool, storage: &str) -> String {
    let part = |what: &str, mode: RecMode, n: usize| match mode {
        RecMode::Off => format!("{what}: OFF"),
        RecMode::Marked => format!("{what}: ON for {n} marked"),
        RecMode::All => format!("{what}: ON for all traffic"),
    };
    let on = tg != RecMode::Off || ch != RecMode::Off;
    let mut t = format!(
        "No recordings yet. Recording is {} ({}, {}). Recorder {}. Storage: {storage}.",
        if on { "ON" } else { "OFF" },
        part("talkgroups", tg, n_tg),
        part("channels", ch, n_ch),
        if ready { "running" } else { "not running" },
    );
    let marked_none = |m: RecMode, n: usize| m == RecMode::Off || (m == RecMode::Marked && n == 0);
    if !on {
        t.push_str(" Arm REC on the Talkgroups or Channels tab (red: marked, amber: all).");
    } else if marked_none(tg, n_tg) && marked_none(ch, n_ch) {
        t.push_str(" Nothing is marked: right-click a row to mark it.");
    }
    t
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

/// How long before a call starts a sighting still counts for it: the grant
/// that names the talking radio arrives just before the follower picks it.
const UNIT_LOOKBACK_MS: i64 = 5_000;
/// Sightings older than this are dropped.
const UNIT_KEEP_MS: i64 = 60_000;

fn utc_minus(at: &str, ms: i64) -> String {
    chrono::DateTime::parse_from_rfc3339(at)
        .map(|t| (t.with_timezone(&chrono::Utc) - chrono::Duration::milliseconds(ms)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

pub fn utc_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Radio units the control channel reported per talkgroup, for one HackRF,
/// and the call it is on. Feeds `Meta::units` (issue #162).
#[derive(Default, Debug)]
pub struct CallUnits {
    seen: Vec<(u32, meta::UnitSeen)>,
    call: Option<(u32, String)>,
    /// Every sighting (tg, unit, utc), kept `UNIT_KEEP_MS`: who talked when.
    log: Vec<(u32, u32, String)>,
}

impl CallUnits {
    /// Unit `unit` talking on talkgroup `tg` at `at` (UTC ISO).
    pub fn sight(&mut self, tg: u32, unit: u32, at: &str) {
        if unit == 0 {
            return;
        }
        let keep = utc_minus(at, UNIT_KEEP_MS);
        self.seen.retain(|(_, u)| u.last_utc >= keep);
        self.log.retain(|(_, _, t)| *t >= keep);
        self.log.push((tg, unit, at.to_string()));
        match self.seen.iter_mut().find(|(t, u)| *t == tg && u.id == unit) {
            Some((_, u)) => {
                if u.last_utc.as_str() < at {
                    u.last_utc = at.to_string();
                }
            }
            None => self.seen.push((tg, meta::UnitSeen { id: unit, first_utc: at.into(), last_utc: at.into() })),
        }
    }
    /// The follower moved to talkgroup `tg` (or to silence) at `at`.
    pub fn begin(&mut self, tg: Option<u32>, at: &str) {
        self.call = tg.map(|t| (t, at.to_string()));
    }
    /// Sightings (unit, utc) on talkgroup `tg` from `since` (UTC ISO) on.
    pub fn sightings(&self, tg: u32, since: &str) -> Vec<(u32, String)> {
        self.log.iter().filter(|(t, _, at)| *t == tg && at.as_str() >= since).map(|(_, u, at)| (*u, at.clone())).collect()
    }
    /// The current call's talkgroup and start.
    pub fn call(&self) -> Option<(u32, String)> {
        self.call.clone()
    }
    /// Units heard on `tg` from `since` on, first-heard first.
    pub fn units_on(&self, tg: u32, since: &str) -> Vec<meta::UnitSeen> {
        let mut v: Vec<meta::UnitSeen> = Vec::new();
        for (u, at) in self.sightings(tg, since) {
            meta::note_unit(&mut v, u, &at);
        }
        v
    }
    /// Units heard on the current call, first-heard first.
    pub fn units(&self) -> Vec<meta::UnitSeen> {
        let Some((tg, start)) = &self.call else { return Vec::new() };
        let from = utc_minus(start, UNIT_LOOKBACK_MS);
        let mut v: Vec<meta::UnitSeen> =
            self.seen.iter().filter(|(t, u)| t == tg && u.last_utc >= from).map(|(_, u)| u.clone()).collect();
        v.sort_by(|a, b| a.first_utc.cmp(&b.first_utc));
        v
    }
}

/// "51007" or "51007 (+2: 42, 9)" for a call's radio IDs.
pub fn units_text(m: &Meta) -> String {
    units_named(m, &HashMap::new())
}

/// A radio id with its description when it has one: "Engine 7 (100)".
pub fn unit_named(id: u32, names: &HashMap<u32, String>) -> String {
    match names.get(&id).map(|d| d.trim()).filter(|d| !d.is_empty() && *d != id.to_string()) {
        Some(d) => format!("{d} ({id})"),
        None => id.to_string(),
    }
}

/// The radios on a call, each with its description when it has one.
pub fn units_named(m: &Meta, names: &HashMap<u32, String>) -> String {
    let ids = m.unit_ids();
    match ids.split_first() {
        None => String::new(),
        Some((first, [])) => unit_named(*first, names),
        Some((first, rest)) => format!(
            "{} (+{}: {})",
            unit_named(*first, names),
            rest.len(),
            rest.iter().map(|u| unit_named(*u, names)).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Lane name for radio `id` on the Radio IDs tab: "Alias (id)" when the
/// station has a description for it, else the number.
pub fn radio_lane(id: u32, names: &HashMap<u32, String>) -> String {
    match names.get(&id).map(|d| d.trim()).filter(|d| !d.is_empty() && *d != id.to_string()) {
        Some(d) => format!("{d} ({id})"),
        None => format!("Radio {id}"),
    }
}

/// What a saved description shows as (the lane name).
pub fn rec_radio_label(id: u32, names: &HashMap<u32, String>) -> String {
    radio_lane(id, names)
}

/// Lane for talkgroup calls with no known radio. They are listed, not left out.
pub const UNKNOWN_RADIO: &str = "Unknown radio";

/// The Radio IDs tab: every talkgroup recording cut into one block per
/// transmission, laned by radio. Ids are negative so they never reach the
/// database; the map gives each one's recording and its span in that file.
pub fn radio_rows(all: &[Recording], names: &HashMap<u32, String>) -> (Vec<Recording>, HashMap<i64, (i64, u64, u64)>) {
    let mut rows = Vec::new();
    let mut map = HashMap::new();
    for r in all.iter().filter(|r| r.meta.talkgroup.is_some() && r.meta.source != meta::SOURCE_RADIO_IDS) {
        let live = r.meta.end_utc.is_empty();
        let Ok(start) = chrono::DateTime::parse_from_rfc3339(&r.meta.start_utc) else { continue };
        let start = start.with_timezone(&chrono::Utc);
        let mut segs = r.meta.talker_segments();
        if live {
            // Still recording: the last radio heard so far owns the call to NOW.
            let ids = r.meta.unit_ids();
            segs = match ids.last() {
                Some(id) => vec![meta::Talk { id: *id, start_ms: 0, end_ms: 0 }],
                None => vec![meta::Talk { id: 0, start_ms: 0, end_ms: 0 }],
            };
        } else if segs.is_empty() {
            segs = vec![meta::Talk { id: 0, start_ms: 0, end_ms: r.meta.duration_ms.max(1) }];
        }
        for (k, t) in segs.iter().enumerate() {
            let vid = -(r.id * 1000 + k as i64 + 1);
            let mut v = r.clone();
            v.id = vid;
            v.meta.lane_override = Some(if t.id == 0 { UNKNOWN_RADIO.to_string() } else { radio_lane(t.id, names) });
            v.meta.radio_unit_id = (t.id != 0).then_some(t.id);
            v.meta.units.retain(|u| u.id == t.id);
            v.meta.talkers.clear();
            let s = start + chrono::Duration::milliseconds(t.start_ms);
            v.meta.start_utc = s.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            if live {
                v.meta.end_utc.clear();
            } else {
                v.meta.duration_ms = t.end_ms - t.start_ms;
                v.meta.end_utc = (start + chrono::Duration::milliseconds(t.end_ms)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            }
            map.insert(vid, (r.id, t.start_ms.max(0) as u64, if live { u64::MAX } else { t.end_ms.max(0) as u64 }));
            rows.push(v);
        }
    }
    (rows, map)
}

/// Block colour for talkgroup `tg` on the Radio IDs tab: a fixed pick from
/// a dark palette, so one talkgroup keeps its colour across radios.
pub fn tg_colour(tg: u32) -> egui::Color32 {
    const P: [(u8, u8, u8); 8] = [(46, 120, 70), (40, 90, 140), (130, 80, 30), (110, 50, 120), (30, 110, 110), (120, 110, 30), (140, 60, 60), (70, 70, 140)];
    let (r, g, b) = P[(tg.wrapping_mul(2_654_435_761) >> 29) as usize % P.len()];
    egui::Color32::from_rgb(r, g, b)
}

/// A call recorded while another is on the speaker.
pub struct GrantRec {
    tg: u32,
    seq: u64,
    opened: Instant,
    /// Last time the grant was renewed or its carrier was on the air.
    live: Instant,
}

/// A recorded-only call ends after this long with no carrier and no renewed
/// grant (at least the voice hang).
pub const GRANT_QUIET: Duration = Duration::from_millis(2_000);
/// A grant with no mixer of its own counts as live while its REC squelch
/// heard a signal this recently.
pub const GRANT_SIGNAL_FRESH: Duration = Duration::from_millis(500);

/// How long the status strip leads with the last result note.
pub const NOTE_SHOWN: Duration = Duration::from_secs(60);

/// A channel file stays open this long after its last signal block.
pub const SIGNAL_HANG: Duration = Duration::from_millis(1_500);

/// Playback demodulator key for analog FM audio (the re-demodulated
/// discriminator, no decoder).
pub const DEMOD_FM: &str = "fm";

#[derive(Default)]
pub struct RecUi {
    /// Per HackRF radio: units heard on its calls (#162).
    pub call_units: HashMap<u64, CallUnits>,
    /// Per HackRF radio: the unit list last sent to the open segment.
    units_sent: HashMap<u64, Vec<meta::UnitSeen>>,
    /// Last radio sightings sent per recorder key.
    talk_sent: HashMap<u64, Vec<(u32, String)>>,
    /// (unit, rows_gen, recordings) for the Radio IDs tab.
    unit_count: Option<(u32, u64, usize)>,
    pub face: config::RecFace,
    /// Station path the recorder is set up for. None forces a redo.
    pub configured: Option<String>,
    db: Option<Db>,
    dir: PathBuf,
    db_path: PathBuf,
    seg: HashMap<u64, Segmenter>,
    /// Carrier files open for each HackRF radio. Every channel in the capture
    /// records at once. The key is `rec_live::carrier_key`.
    carrier_open: HashMap<u64, HashSet<u64>>,
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
    /// Result of the last Data dialog action (rescan, archive, export, move);
    /// the Data dialog shows only this.
    data_note: String,
    /// The last result note and when it first showed; the status strip
    /// leads with it for `NOTE_SHOWN`.
    note_seen: Option<(String, Instant)>,
    /// Result of the last playback (decoder, no voice, failure); shown in
    /// the playback dialog's Playback section.
    play_note: String,
    /// Radio IDs dialog: the description being typed for one radio.
    desc_edit: Option<(u32, String)>,
    /// A description left (Enter or focus out), saved on the next frame.
    pub(crate) desc_commit: Option<(u32, String)>,
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
    /// The axis LIVE key was clicked: follow NOW from the next frame.
    go_live: bool,
    /// Lane list scroll offset (points from the top), per source tab.
    lane_off: HashMap<String, f32>,
    /// Calls recorded but not on the speaker, by (radio, hz).
    grant_open: HashMap<(u64, u64), GrantRec>,
    /// The last (tg, grant sequence) filed per (radio, hz): a grant that is
    /// still listed after its call ended is not opened again.
    grant_done: HashMap<(u64, u64), (u32, u64)>,
    /// Radio IDs tab: each shown block's (recording, start ms, end ms) in
    /// the talkgroup recording it was cut from.
    radio_map: HashMap<i64, (i64, u64, u64)>,
    /// Where the playing file started (ms into the file) and where it stops
    /// (a radio's transmission on the Radio IDs tab).
    play_base: u64,
    play_until: Option<u64>,
    /// Demodulator picked in the Record / Playback dialog, per recording.
    /// Session only; absent means the recording's own default.
    demod: HashMap<i64, String>,
    /// Tests play into this command instead of the speaker.
    #[cfg(test)]
    pub(crate) play_argv: Option<Vec<String>>,
    /// Radios the tests add to the hub's list (a HackRF without hardware).
    #[cfg(test)]
    pub(crate) test_rows: Vec<cat::RadioRow>,
    /// Why the database could not be opened or created (shown, never silent).
    db_error: Option<String>,
}

fn rows_meta(rows: &[cat::RadioRow], id: u64) -> Option<&cat::RadioRow> {
    rows.iter().find(|r| r.id == id)
}

impl App {
    /// How many recordings have radio `unit` on the call (Radio IDs tab).
    pub(crate) fn rec_unit_count(&mut self, unit: u32) -> Option<usize> {
        let gen = self.rec.rows_gen;
        if let Some((u, g, n)) = self.rec.unit_count {
            if u == unit && g == gen {
                return Some(n);
            }
        }
        let n = self.rec.db.as_ref()?.by_unit(unit).ok()?.len();
        self.rec.unit_count = Some((unit, gen, n));
        Some(n)
    }

    fn rec_armed(&self) -> bool {
        self.rec.face.armed()
    }

    pub(super) fn rec_icon(&self) -> IconState {
        let (ready, err) = self.rec_ready();
        let writing = crate::rec_live::recorder().status.writing.load(Ordering::Relaxed);
        rec_state(self.rec_armed(), ready, writing, err.as_deref())
    }

    /// Recorder can write (UI and writer both have the database), and any error.
    fn rec_ready(&self) -> (bool, Option<String>) {
        let status = &crate::rec_live::recorder().status;
        let ready = self.rec.db.is_some() && status.ready.load(Ordering::Relaxed);
        let err = self.rec.db_error.clone().or_else(|| {
            let e = status.error.lock().map(|e| e.clone()).unwrap_or_default();
            (!e.is_empty() && self.rec.db.is_some()).then_some(e)
        });
        (ready, err)
    }

    fn rec_marked_counts(&self) -> (usize, usize) {
        let tg = self.talkgroups.zones.iter().flat_map(|z| z.talkgroups.iter()).filter(|r| r.record).count();
        let ch = self.channels.zones.iter().flat_map(|z| z.channels.iter()).filter(|c| c.record).count();
        (tg, ch)
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
                // A new press retries a database that failed to open.
                self.rec.db_error = None;
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
        if let Some((id, text)) = self.rec.desc_commit.take() {
            self.rec_save_description(snap, id, &text);
        }
        let path = self.config_path.clone();
        if self.rec.configured.as_deref() != Some(path.as_str()) {
            self.rec_configure(&path, false);
        }
        // REC was armed after the station loaded without a database: create
        // it quietly now instead of pretending to record.
        if self.rec_armed() && self.rec.db.is_none() && self.rec.db_error.is_none() && !path.is_empty() {
            self.rec_configure(&path, true);
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
        #[allow(unused_mut)]
        let mut rows = self.hub.view().rows;
        #[cfg(test)]
        rows.extend(self.rec.test_rows.iter().cloned());
        let live: Vec<u64> = rows
            .iter()
            .filter(|r| matches!(r.spec.driver, cat::RadioDriver::Hackrf | cat::RadioDriver::Hamlib | cat::RadioDriver::Gqrx))
            .map(|r| r.id)
            .collect();
        let mut gone: Vec<u64> = self.rec.seg.keys().copied().filter(|id| !live.contains(id) || !armed).collect();
        for (radio, _) in self.rec.grant_open.keys() {
            if (!live.contains(radio) || !armed) && !gone.contains(radio) {
                gone.push(*radio);
            }
        }
        for id in gone {
            crate::rec_live::set_rec_hz(id, 0);
            self.close_carriers(id, &recorder);
            self.close_grants(id, &recorder);
            if let Some(mut s) = self.rec.seg.remove(&id) {
                for a in s.stop() {
                    if let Action::Close { keep } = a {
                        recorder.send(Cmd::Close { key: id, keep });
                    }
                }
            }
        }
        if !armed {
            for row in &rows {
                if rig_driver(row.spec.driver) {
                    crate::rec_live::set_rec_hz(row.id, 0);
                }
                self.close_carriers(row.id, &recorder);
                self.close_grants(row.id, &recorder);
            }
            self.rec.enc.clear();
            return;
        }
        for id in live {
            let Some(row) = rows_meta(&rows, id) else { continue };
            let (want, meta) = self.rec_want(row);
            if rig_driver(row.spec.driver) {
                // The tee files samples only while this is the call REC wants.
                crate::rec_live::set_rec_hz(id, want.as_ref().map(|k| k.hz).unwrap_or(0));
            }
            // The file stays open for the whole time the follower holds this
            // call. Voice-frame lines are not what starts or ends it.
            let mut enc = self.rec.enc.remove(&id);
            // A rig has no IQ tap. Air encryption still deletes the open file
            // once, then the call stays unwanted so it is not opened again.
            if rig_driver(row.spec.driver) && self.rig_air_encrypted(row) && self.rec.seg.get(&id).is_some_and(|s| s.recording().is_some()) {
                enc = true;
            }
            let max = Duration::from_secs(u64::from(
                self.rec.face.max_segment_s.clamp(segment::MAX_SEGMENT_MIN_S, segment::MAX_SEGMENT_MAX_S),
            ));
            let actions = self.rec.seg.entry(id).or_default().step_max(want, enc, hang, Some(max), now);
            for a in actions {
                match a {
                    Action::Open(_) => {
                        if let Some(mut m) = meta.clone() {
                            self.rec.units_sent.insert(id, m.units.clone());
                            m.afc_hz = crate::rec_live::afc_hz(id);
                            recorder.send(Cmd::Open { key: id, meta: m });
                        }
                    }
                    Action::Close { keep } => {
                        if !keep {
                            self.rec_log("REC: encrypted traffic on this call; segment discarded.".into());
                        }
                        recorder.send(Cmd::Close { key: id, keep });
                        self.rec.units_sent.remove(&id);
                    }
                }
            }
            // Who talked when on the open call, for the Radio IDs tab.
            if self.rec.seg.get(&id).is_some_and(|s| s.recording().is_some()) {
                if let Some((tg, start)) = self.rec.call_units.get(&id).and_then(|c| c.call()) {
                    let since = utc_minus(&start, UNIT_LOOKBACK_MS);
                    let s = self.rec.call_units.get(&id).map(|c| c.sightings(tg, &since)).unwrap_or_default();
                    if !s.is_empty() && self.rec.talk_sent.get(&id) != Some(&s) {
                        self.rec.talk_sent.insert(id, s.clone());
                        recorder.send(Cmd::Talkers { key: id, sightings: s });
                    }
                }
            } else {
                self.rec.talk_sent.remove(&id);
            }
            // Talkers who join mid-call go to the open segment as they appear.
            if self.rec.seg.get(&id).is_some_and(|s| s.recording().is_some()) {
                if let Some(m) = &meta {
                    if !m.units.is_empty() && self.rec.units_sent.get(&id) != Some(&m.units) {
                        self.rec.units_sent.insert(id, m.units.clone());
                        recorder.send(Cmd::Units { key: id, units: m.units.clone() });
                    }
                }
            }
            if row.spec.driver == cat::RadioDriver::Hackrf {
                self.sync_carrier_files(row, &recorder);
                self.sync_grant_files(row, &recorder, now);
            }
        }
    }

    /// Every channel inside this HackRF capture gets its own IQ file while
    /// it carries an RF signal (the REC squelch, `hsrec::dsp::Squelch`, run
    /// by the capture thread) and for `SIGNAL_HANG` after, so a pause between
    /// words stays in one file and noise is never filed. A control channel on
    /// the air is a signal the whole time and stays one long recording; a
    /// silent alternate is not recorded. The speaker decoder is a different path.
    fn sync_carrier_files(&mut self, row: &cat::RadioRow, recorder: &hsrec::writer::Recorder) {
        let center = row.center_hz;
        let Some(zone) = self.channels.zones.get(self.channel_zone) else {
            self.close_carriers(row.id, recorder);
            return;
        };
        if !center.is_finite() || center <= 0.0 {
            self.close_carriers(row.id, recorder);
            return;
        }
        let mut next = HashSet::new();
        for channel in &zone.channels {
            if !channel.hz.is_finite() || channel.hz <= 0.0 {
                continue;
            }
            if channel.encrypted || !hsrec::dsp::in_capture(channel.hz - center) {
                continue;
            }
            let hz = channel.hz.round() as u64;
            let key = crate::rec_live::carrier_key(row.id, hz);
            // A call the control channel granted here (followed or not) is
            // kept whatever the squelch says: it came from control sync.
            let granted = self.hackrf_call.get(&row.id).copied().flatten().is_some_and(|(f, _)| f.round() as u64 == hz)
                || self.hackrf_open.get(&row.id).is_some_and(|g| g.iter().any(|g| g.hz.round() as u64 == hz));
            let signal = granted || crate::rec_live::signal_at(row.id, hz).is_some_and(|t| t.elapsed() < SIGNAL_HANG);
            if !signal {
                continue;
            }
            next.insert(key);
            if self.rec.carrier_open.get(&row.id).is_some_and(|open| open.contains(&key)) {
                continue;
            }
            let mut meta = Meta {
                source: meta::SOURCE_HACKRF_CHANNELS.into(),
                source_kind: meta::KIND_RAW_IQ.into(),
                radio_id: row.id,
                radio_name: row.spec.name.clone(),
                raw_format: "cs16".into(),
                sample_rate: hsrec::dsp::REC_RATE,
                centre_hz: center,
                channel_hz: channel.hz,
                offset_hz: channel.hz - center,
                bandwidth_hz: hsrec::dsp::REC_RATE as f64,
                channel_name: channel.name.clone(),
                chain: hsrec::dsp::CHAIN_ID.into(),
                sw_version: format!("commander {} / {}", env!("CARGO_PKG_VERSION"), meta::sw_version()),
                modulation: if channel.vocoder.is_empty() { "fm".into() } else { channel.vocoder.clone() },
                ..Default::default()
            };
            meta.decoder = serde_json::json!({
                "script": hsrec::safepath::portable_decoder(&row.spec.rx_pipe),
                "vocoder": channel.vocoder,
                "invert": channel.invert,
                "mode": channel.mode
            });
            hsrec::safepath::portable_meta(&mut meta);
            meta.decoder_params = self.rec_live_params(row, &meta);
            meta.afc_hz = crate::rec_live::afc_hz(row.id);
            recorder.send(Cmd::Open { key, meta });
        }
        if let Some(prev) = self.rec.carrier_open.get(&row.id) {
            for key in prev.difference(&next) {
                recorder.send(Cmd::Close { key: *key, keep: true });
            }
        }
        self.rec.carrier_open.insert(row.id, next);
    }

    /// Radio descriptions by id, as the Radio IDs page and the lanes use them.
    pub(crate) fn unit_names(&self) -> HashMap<u32, String> {
        self.units.iter().map(|u| (u.id, u.description.clone())).collect()
    }

    /// Save a radio's description from the Radio IDs dialog into the
    /// station file (its `units`), the same save the Radio IDs page does.
    /// The lanes, title and tooltips pick it up on the reload. A failed
    /// save is shown in the status line and logged; it is never silent.
    pub(crate) fn rec_save_description(&mut self, snap: &Snapshot, id: u32, text: &str) {
        if !crate::units::set_description(&mut self.units, id, text) {
            return;
        }
        let shown = rec_radio_label(id, &self.unit_names());
        if self.config_path.is_empty() {
            self.rec.note = format!("Radio {id} description NOT saved: no station file is open.");
            eprintln!("commander: {}", self.rec.note);
            self.rec_log(self.rec.note.clone());
            self.rec_reload(true);
            return;
        }
        self.unit_desc_dirty = true;
        self.config_note.clear();
        self.save_config(snap);
        if self.config_note.is_empty() {
            self.rec.note = format!("Saved: {shown}.");
        } else {
            self.unit_desc_dirty = true;
            self.rec.note = format!("Radio {id} description NOT saved: {}", self.config_note);
            eprintln!("commander: {}", self.rec.note);
            self.rec_log(self.rec.note.clone());
        }
        self.rec_reload(true);
    }

    fn close_carriers(&mut self, radio: u64, recorder: &hsrec::writer::Recorder) {
        if let Some(open) = self.rec.carrier_open.remove(&radio) {
            for key in open {
                recorder.send(Cmd::Close { key, keep: true });
            }
        }
    }

    /// Drop the open index and point REC at wherever the station data lives now.
    pub(crate) fn rec_retarget(&mut self, path: &str) {
        self.rec.db = None;
        self.rec_configure(path, true);
    }

    fn rec_configure(&mut self, path: &str, force: bool) {
        self.rec.configured = Some(path.to_string());
        self.rec.db = None;
        self.rec.db_error = None;
        self.rec.rows.clear();
        self.rec.waveforms.clear();
        self.rec.wf_asked.clear();
        self.rec.selected = None;
        self.rec.player = None;
        if path.is_empty() {
            return;
        }
        let station = Path::new(path);
        self.rec.dir = hsrec::hsdata::recordings_dir(station);
        self.rec.db_path = hsrec::hsdata::db_path(station);
        // The UI only opens the database (and creates it) once REC was
        // used: armed, or the file already exists.
        // `force`: REC armed or the REC page opened; a missing (or empty)
        // database is created quietly.
        if !self.rec.db_path.exists() && !self.rec_armed() && !force {
            return;
        }
        match Db::open(&self.rec.db_path) {
            Ok(db) => self.rec.db = Some(db),
            Err(e) => {
                self.rec.db_error = Some(e.to_string());
                return;
            }
        }
        let recorder = crate::rec_live::recorder();
        recorder.send(Cmd::Configure { rec_dir: self.rec.dir.clone(), db_path: self.rec.db_path.clone() });
        recorder.send(Cmd::Cap { gb: self.rec.face.cap_gb, overwrite_oldest: self.rec.face.overwrite_oldest });
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

    /// The call this radio should be recording now, and its metadata.
    fn rec_want(&self, row: &cat::RadioRow) -> (Option<CallKey>, Option<Meta>) {
        match row.spec.driver {
            cat::RadioDriver::Hackrf => self.rec_want_hackrf(row),
            cat::RadioDriver::Hamlib | cat::RadioDriver::Gqrx => self.rec_want_rig(row),
            cat::RadioDriver::Tap => (None, None),
        }
    }

    /// True when the follower's current grant is not clear. A filled key
    /// does not make that recordable.
    fn rig_air_encrypted(&self, row: &cat::RadioRow) -> bool {
        let Some(follower) = self.followers.get(&row.id) else { return false };
        let Some(tg) = follower.active_tg() else { return false };
        follower.heard().is_some_and(|h| h.tg == tg && h.alg != crate::follow::CLEAR_ALG)
    }

    fn rig_script<'a>(row: &'a cat::RadioRow) -> &'a str {
        if row.rx_chain.is_empty() { &row.spec.rx_pipe } else { &row.rx_chain }
    }

    fn rig_meta(&self, row: &cat::RadioRow, hz: f64) -> Meta {
        Meta {
            source: meta::SOURCE_RIG_AUDIO.into(),
            source_kind: meta::KIND_DEMOD_AUDIO.into(),
            radio_id: row.id,
            radio_name: row.spec.name.clone(),
            raw_format: "s16le".into(),
            sample_rate: meta::RIG_RATE,
            centre_hz: hz,
            channel_hz: hz,
            offset_hz: 0.0,
            bandwidth_hz: 0.0,
            chain: meta::RIG_CHAIN.into(),
            sw_version: format!("commander {} / {}", env!("CARGO_PKG_VERSION"), meta::sw_version()),
            ..Default::default()
        }
    }

    /// Hamlib / GQRX: the rig's demod audio for a held talkgroup, or for the
    /// channel the scanner is holding. A scan held on a control memory is not
    /// a channel recording, but a voice grant is a call on any frequency, one
    /// marked control included: the mark only says where to hunt for control.
    fn rec_want_rig(&self, row: &cat::RadioRow) -> (Option<CallKey>, Option<Meta>) {
        let face = &self.rec.face;
        let voice = self.followers.get(&row.id).and_then(|follower| match (follower.voice_hz(), follower.active_tg()) {
            (Some(hz), Some(tg)) if hz.is_finite() && hz > 0.0 => Some((hz, tg)),
            _ => None,
        });
        if let Some((hz, tg)) = voice {
            if face.talkgroups.armed() {
                let found = self.talkgroups.zones.iter().flat_map(|z| z.talkgroups.iter()).find(|t| t.tg == tg);
                let marked = found.is_some_and(|t| t.record);
                let enc = found.is_some_and(|t| t.encrypted) || self.rig_air_encrypted(row);
                if segment::wanted(face.talkgroups, marked, enc) {
                    let key = CallKey { talkgroup: Some(tg), channel: String::new(), hz: hz.round() as u64, centre: hz.round() as u64 };
                    let mut m = self.rig_meta(row, hz);
                    m.talkgroup = Some(tg);
                    m.talkgroup_name = found.map(|t| t.name.clone()).unwrap_or_default();
                    m.priority = found.is_some_and(|t| t.priority);
                    m.emergency = self.trunk_emergency.iter().any(|(_, _, t)| *t == tg);
                    m.channel_hz = hz;
                    if let Some(c) = self.rec.call_units.get(&row.id) {
                        m.set_units(c.units());
                    }
                    let script = Self::rig_script(row);
                    let label = crate::rx_pipe::pipeline_label(script);
                    m.modulation = if label.is_empty() || label.contains('.') { "fm".into() } else { label.into() };
                    m.decoder = serde_json::json!({ "script": hsrec::safepath::portable_decoder(script).unwrap_or_default() });
                    hsrec::safepath::portable_meta(&mut m);
                    m.decoder_params = self.rec_live_params(row, &m);
                    if let Some(b) = self.cc_site.get(&row.id) {
                        m.system = format!("WACN {} SYS {}", b.wacn, b.sys);
                        m.site = format!("RFSS {} Site {}", b.rfss, b.site);
                    }
                    return (Some(key), Some(m));
                }
            }
            // A voice grant is not also filed as a channel recording.
            return (None, None);
        }
        if row.selected && face.channels.armed() {
            if let Some((z, i)) = self.memory_scan.hold_memory() {
                if let Some(ch) = self.channels.zones.get(z).and_then(|zone| zone.channels.get(i)) {
                    if !ch.control && ch.hz.is_finite() && ch.hz > 0.0 && segment::wanted(face.channels, ch.record, ch.encrypted) {
                        let hz = ch.hz;
                        let key = CallKey { talkgroup: None, channel: ch.name.clone(), hz: hz.round() as u64, centre: hz.round() as u64 };
                        let mut m = self.rig_meta(row, hz);
                        m.channel_name = ch.name.clone();
                        m.channel_hz = hz;
                        m.modulation = if ch.vocoder.is_empty() { "fm".into() } else { ch.vocoder.clone() };
                        let script = Self::rig_script(row);
                        m.decoder = serde_json::json!({
                            "script": hsrec::safepath::portable_decoder(script).unwrap_or_default(),
                            "vocoder": ch.vocoder,
                            "invert": ch.invert,
                            "mode": ch.mode
                        });
                        hsrec::safepath::portable_meta(&mut m);
                        m.decoder_params = self.rec_live_params(row, &m);
                        return (Some(key), Some(m));
                    }
                }
            }
        }
        (None, None)
    }

    /// Metadata of a HackRF talkgroup call on `hz`.
    fn hackrf_tg_meta(&self, row: &cat::RadioRow, hz: f64, tg: u32) -> Meta {
        let found = self.talkgroups.zones.iter().flat_map(|z| z.talkgroups.iter()).find(|t| t.tg == tg);
        let mut m = Meta {
            source: meta::SOURCE_HACKRF_TG.into(),
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
        m
    }

    /// Calls granted inside this HackRF capture that are not on the
    /// speaker. Each is recorded in its own file at the same time, so a call
    /// missed because another had priority can be played from REC later.
    /// (hz, tg, grant sequence).
    fn hackrf_other_grants(&self, row: &cat::RadioRow) -> Vec<(u64, u32, u64)> {
        let face = &self.rec.face;
        if !face.talkgroups.armed() || !row.center_hz.is_finite() {
            return Vec::new();
        }
        let heard = self.hackrf_call.get(&row.id).copied().flatten().map(|(hz, _)| hz.round() as u64);
        let mut out: Vec<(u64, u32, u64)> = Vec::new();
        for g in self.hackrf_open.get(&row.id).map(Vec::as_slice).unwrap_or(&[]) {
            if !g.hz.is_finite() || g.hz <= 0.0 || !hsrec::dsp::in_capture(g.hz - row.center_hz) {
                continue;
            }
            let hz = g.hz.round() as u64;
            if heard == Some(hz) || out.iter().any(|(h, _, _)| *h == hz) {
                continue;
            }
            // A grant on a frequency marked control is still a call: the site
            // moves its control channel and hands the old one out for voice.
            let found = self.talkgroups.zones.iter().flat_map(|z| z.talkgroups.iter()).find(|t| t.tg == g.tg);
            if segment::wanted(face.talkgroups, found.is_some_and(|t| t.record), found.is_some_and(|t| t.encrypted)) {
                out.push((hz, g.tg, g.seq));
            }
        }
        out
    }

    /// Open, keep and close the files of the calls not on the speaker.
    fn sync_grant_files(&mut self, row: &cat::RadioRow, recorder: &hsrec::writer::Recorder, now: Instant) {
        let radio = row.id;
        let want = self.hackrf_other_grants(row);
        let quiet = Duration::from_millis(u64::from(self.voice_hang_ms)).max(GRANT_QUIET);
        let open: Vec<u64> = self.rec.grant_open.iter().filter(|((r, _), _)| *r == radio).map(|((_, hz), _)| *hz).collect();
        for hz in open {
            let Some(g) = self.rec.grant_open.get_mut(&(radio, hz)) else { continue };
            let same = want.iter().find(|(h, tg, _)| *h == hz && *tg == g.tg);
            if let Some((_, _, seq)) = same {
                if *seq != g.seq {
                    g.seq = *seq;
                    g.live = now;
                }
            }
            match crate::rec_live::extra_state(radio, hz) {
                Some(false) => g.live = now,
                // No mixer of its own: a frequency marked control is already
                // mixed by its control decoder. Its REC squelch says it is live.
                None if crate::rec_live::signal_at(radio, hz).is_some_and(|at| now.saturating_duration_since(at) < GRANT_SIGNAL_FRESH) => g.live = now,
                _ => {}
            }
            let ended = same.is_none() || (now.duration_since(g.live) > quiet && now.duration_since(g.opened) > GRANT_QUIET);
            if ended {
                let g = self.rec.grant_open.remove(&(radio, hz)).expect("open grant");
                self.rec.grant_done.insert((radio, hz), (g.tg, g.seq));
                self.rec.talk_sent.remove(&crate::rec_live::grant_key(radio, hz));
                recorder.send(Cmd::Close { key: crate::rec_live::grant_key(radio, hz), keep: true });
            }
        }
        // Radios heard on each recorded-only call, and who talked when.
        let open: Vec<(u64, u32, Instant)> = self.rec.grant_open.iter().filter(|((r, _), _)| *r == radio).map(|((_, hz), g)| (*hz, g.tg, g.opened)).collect();
        for (hz, tg, opened) in open {
            let key = crate::rec_live::grant_key(radio, hz);
            let ago = now.saturating_duration_since(opened).as_millis() as i64 + UNIT_LOOKBACK_MS;
            let since = utc_minus(&utc_now(), ago);
            let Some(cu) = self.rec.call_units.get(&radio) else { continue };
            let s = cu.sightings(tg, &since);
            let units = cu.units_on(tg, &since);
            if !s.is_empty() && self.rec.talk_sent.get(&key) != Some(&s) {
                self.rec.talk_sent.insert(key, s.clone());
                recorder.send(Cmd::Units { key, units });
                recorder.send(Cmd::Talkers { key, sightings: s });
            }
        }
        for (hz, tg, seq) in want {
            if self.rec.grant_open.contains_key(&(radio, hz)) || self.rec.grant_done.get(&(radio, hz)) == Some(&(tg, seq)) {
                continue;
            }
            let mut m = self.hackrf_tg_meta(row, hz as f64, tg);
            if let Some(cu) = self.rec.call_units.get(&radio) {
                m.set_units(cu.units_on(tg, &utc_minus(&utc_now(), UNIT_LOOKBACK_MS)));
            }
            m.afc_hz = crate::rec_live::afc_hz(radio);
            recorder.send(Cmd::Open { key: crate::rec_live::grant_key(radio, hz), meta: m });
            self.rec.grant_open.insert((radio, hz), GrantRec { tg, seq, opened: now, live: now });
        }
        let taps: Vec<u64> = self.rec.grant_open.keys().filter(|(r, _)| *r == radio).map(|(_, hz)| *hz).collect();
        crate::rec_live::set_grant_taps(radio, taps);
    }

    fn close_grants(&mut self, radio: u64, recorder: &hsrec::writer::Recorder) {
        let keys: Vec<(u64, u64)> = self.rec.grant_open.keys().filter(|(r, _)| *r == radio).copied().collect();
        for k in keys {
            if let Some(g) = self.rec.grant_open.remove(&k) {
                self.rec.grant_done.insert(k, (g.tg, g.seq));
            }
            recorder.send(Cmd::Close { key: crate::rec_live::grant_key(k.0, k.1), keep: true });
        }
        crate::rec_live::set_grant_taps(radio, Vec::new());
    }

    /// The call this HackRF radio should be recording now, and its metadata.
    fn rec_want_hackrf(&self, row: &cat::RadioRow) -> (Option<CallKey>, Option<Meta>) {
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
                    // The grant names the voice channel. A Control mark there is
                    // only where to hunt for control; it does not refuse the call.
                    let key = CallKey { talkgroup: Some(tg), channel: String::new(), hz: hz.round() as u64, centre: row.center_hz.round() as u64 };
                    let mut m = self.hackrf_tg_meta(row, hz, tg);
                    if let Some(c) = self.rec.call_units.get(&row.id) {
                        m.set_units(c.units());
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
        let stale = self.rec.rows_at.is_none_or(|t| t.elapsed() > Duration::from_millis(900));
        if !force && gen == self.rec.rows_gen && source == self.rec.rows_source && !stale {
            return;
        }
        let all = db.all().unwrap_or_default();
        self.rec.usage = Some(hsrec::storage::Usage::from_rows(&all, self.rec.face.cap_gb));
        if source == meta::SOURCE_RADIO_IDS {
            let (rows, map) = radio_rows(&all, &self.unit_names());
            self.rec.rows = rows;
            self.rec.radio_map = map;
        } else {
            self.rec.rows = all.into_iter().filter(|r| r.meta.source == source).collect();
        }
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
        if self.rec.db.is_none() && self.rec.db_error.is_none() {
            let path = self.config_path.clone();
            self.rec_configure(&path, true);
        }
        if self.rec.db.is_none() {
            let err = self.rec.db_error.clone().unwrap_or_default();
            ui.label(
                egui::RichText::new(status_line(IconState::Error, false, Some(&err)))
                    .strong()
                    .color(egui::Color32::from_rgb(240, 60, 50)),
            );
            if ui.button("Try again").clicked() {
                let path = self.config_path.clone();
                self.rec_configure(&path, true);
            }
            return;
        }
        self.rec_poll_jobs();
        self.rec_reload(false);
        // The page's status sits in the one status strip at the bottom of the
        // frame, as on the tool pages, not as a row over the lanes (David).
        let full = page_area(ui);
        let (body, strip) = status_split(full);
        crate::status_note::clear(ui.ctx());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("rec-body"));
        child.set_clip_rect(body.intersect(ui.clip_rect()));
        self.page_rec_body(&mut child);
        let mut base = self.rec_status_text();
        // The last action's result (playback, save, data) leads the strip for a while.
        if self.rec.note_seen.as_ref().is_none_or(|(n, _)| *n != self.rec.note) {
            self.rec.note_seen = Some((self.rec.note.clone(), Instant::now()));
        }
        if let Some((n, at)) = &self.rec.note_seen {
            if !n.is_empty() && at.elapsed() < NOTE_SHOWN {
                base = format!("{n}   {base}");
                ui.ctx().request_repaint_after(NOTE_SHOWN);
            }
        }
        crate::status_note::paint_strip(ui, strip, &base);
        ui.allocate_rect(full, egui::Sense::hover());
        if self.rec.player.is_some() || self.rec.job.is_some() || self.rec_icon() == IconState::Writing {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
    }

    /// REC page above its status strip: source tabs, jobs, timeline.
    fn page_rec_body(&mut self, ui: &mut egui::Ui) {
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
        if self.rec.rows.is_empty() && self.rec.query.is_empty() {
            let (n_tg, n_ch) = self.rec_marked_counts();
            let storage = match self.rec.dir.file_name() {
                Some(n) => format!("{} beside the station file", n.to_string_lossy()),
                None => "beside the station file".into(),
            };
            let (ready, _) = self.rec_ready();
            crate::status_note::note(ui, empty_hint(self.rec.face.talkgroups, n_tg, self.rec.face.channels, n_ch, ready, &storage));
        }
        if crate::rec_live::recorder().status.cap_hit.load(Ordering::Relaxed) {
            crate::status_note::note(ui, "\u{26a0} DISK CAP REACHED: recording stopped. Nothing was deleted. Archive, export, or raise the cap.");
        }
        self.rec_job_row(ui);
        self.rec_data_window(ui.ctx());
        self.rec_detail_window(ui.ctx());
        self.rec_timeline(ui);
    }

    /// The REC state for the page's status strip: recorder, what records,
    /// disk use. Plain text; the words carry any warning.
    fn rec_status_text(&self) -> String {
        let (ready, err) = self.rec_ready();
        let mut line = status_line(self.rec_icon(), ready, err.as_deref());
        line.push_str(&rec_status_tail(self.rec.face.talkgroups.label(), self.rec.face.channels.label(), self.rec.usage.as_ref().map(|u| u.text())));
        line
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
                // Only storage, library, move/copy and archive results here;
                // playback results belong to the playback dialog.
                if !self.rec.data_note.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new(&self.rec.data_note).color(caution()));
                }
            });
        if !open {
            self.rec.data_open = false;
        }
    }

    fn rec_data_body(&mut self, ui: &mut egui::Ui) {
        use rustcars::palette::{SLATE, TAN, TEAL};
        let mut cap_changed = false;
        if self.rec.archive_days <= 0.0 {
            self.rec.archive_days = 7.0;
        }
        // No mount event is wired into this dialog. Refresh the removable-drive
        // list on a short interval while it is open, same five seconds as before.
        if self.rec.drives_at.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) {
            self.rec.drives = hsrec::export::removable_drives();
            self.rec.drives_at = Some(Instant::now());
        }
        let has_drive = !self.rec.drives.is_empty();
        crate::group_box::group_box(ui, "Storage", TAN, |ui| {
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
            ui.horizontal_wrapped(|ui| {
                ui.label("Older than");
                ui.add(egui::DragValue::new(&mut self.rec.archive_days).range(0.0..=3650.0).suffix(" d"));
            });
            if !has_drive {
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
            }
        });
        crate::group_box::group_box(ui, "Library", SLATE, |ui| {
            let busy = self.rec.job.is_some();
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!busy, egui::Button::new("Rescan")).on_hover_text("Re-add recordings from sidecars on disk; flag missing files.").clicked() {
                    if let Some(db) = &self.rec.db {
                        match db.rescan(&self.rec.dir) {
                            Ok((a, m)) => { self.rec.note = format!("Rescan: {a} added, {m} missing."); self.rec.data_note = self.rec.note.clone(); }
                            Err(e) => { self.rec.note = format!("Rescan failed: {e}"); self.rec.data_note = self.rec.note.clone(); }
                        }
                        self.rec_reload(true);
                    }
                }
                if has_drive && ui.add_enabled(!busy, egui::Button::new("Re-attach")).on_hover_text("Copy recordings that were moved to this drive back, verified by sha256.").clicked() {
                    self.rec_start_reattach();
                }
                if has_drive && ui.add_enabled(!busy, egui::Button::new("Import")).on_hover_text("Import another Commander's export (HamSpot-REC/index.json) from this drive.").clicked() {
                    self.rec_start_import();
                }
            });
        });
        if has_drive {
            crate::group_box::group_box(ui, "Move & copy", TEAL, |ui| {
                let busy = self.rec.job.is_some();
                let sel = self.rec.selected;
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(!busy && sel.is_some(), egui::Button::new("Copy selected")).clicked() {
                        self.rec_start_export(sel, false);
                    }
                    if ui.add_enabled(!busy, egui::Button::new(format!("Copy older than {:.0} d", self.rec.archive_days))).clicked() {
                        self.rec_start_export(None, false);
                    }
                });
            });
        }
        // Archive and Move remove the local file after a verified copy. Clear cache drops thumbnails.
        // Copy stays in Move & copy. The days field those buttons read is in Storage.
        crate::group_box::group_box(ui, "Danger", crate::estop::DANGER, |ui| {
            let busy = self.rec.job.is_some();
            let sel = self.rec.selected;
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!busy, egui::Button::new("Archive older than")).on_hover_text("zstd compress, verify by sha256, then replace. Archived recordings still play.").clicked() {
                    let rows = self.rec.db.as_ref().and_then(|d| d.all().ok()).unwrap_or_default();
                    let todo: Vec<Recording> = hsrec::archive::older_than(&rows, self.rec.archive_days, chrono::Utc::now()).into_iter().cloned().collect();
                    self.rec_start_archive(todo);
                }
                if has_drive && ui.add_enabled(!busy && sel.is_some(), egui::Button::new("Move selected")).on_hover_text("The local copy is deleted only after the copy on the drive verifies.").clicked() {
                    self.rec_start_export(sel, true);
                }
                if has_drive && ui.add_enabled(!busy, egui::Button::new(format!("Move older than {:.0} d", self.rec.archive_days))).clicked() {
                    self.rec_start_export(None, true);
                }
                if ui.add_enabled(!busy, egui::Button::new("Clear cache")).on_hover_text("Waveform thumbnails are rebuilt from raw IQ. Nothing is lost.").clicked() {
                    if let Some(db) = &self.rec.db {
                        let _ = hsrec::cache::clear(db, &self.rec.dir);
                    }
                    self.rec.waveforms.clear();
                    self.rec.wf_asked.clear();
                }
            });
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
                self.rec.data_note = self.rec.note.clone();
                return;
            }
            Err(e) => {
                self.rec.note = format!("REC lock: {e}");
                self.rec.data_note = self.rec.note.clone();
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
            self.rec.data_note = self.rec.note.clone();
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
            self.rec.data_note = self.rec.note.clone();
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
                    self.rec.data_note = self.rec.note.clone();
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
        // A radio's transmission stops where the next radio starts.
        if let (Some((_, p)), Some(until)) = (&self.rec.player, self.rec.play_until) {
            if p.pos_ms.load(Ordering::Relaxed) >= until {
                p.stop();
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
                    self.rec.play_note = self.rec.note.clone();
                    self.rec_log(self.rec.note.clone());
                } else if let (Some(msg), Some(rec)) = (outcome.no_voice_text(), rec.as_ref()) {
                    self.rec.no_voice.insert(id);
                    let band = if hsrec::dsp::in_capture(rec.meta.offset_hz) {
                        String::new()
                    } else {
                        format!(" The channel was {:.3} MHz from the capture centre, outside the HackRF's usable 6 MHz, so the call is not in this recording.", rec.meta.offset_hz / 1e6)
                    };
                    self.rec.note = format!("{msg}{band}");
                    self.rec.play_note = self.rec.note.clone();
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

    /// The decoder REC replays a recording through. The recording names it
    /// only as `modems/<file>`; Commander resolves it in its own modems
    /// folders and never runs a stored path.
    pub(super) fn rec_decoder(&mut self, rec_meta: &Meta) -> Option<hsrec::replay::DecoderCmd> {
        let fallback = self.rec_param_fallback(rec_meta);
        let mut meta = rec_meta.clone();
        hsrec::safepath::portable_meta(&mut meta);
        let stored = meta.decoder.get("script").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if stored.is_empty() {
            return None;
        }
        let script = match hsrec::safepath::resolve_decoder(&stored, &crate::rx_pipe::modem_dirs()) {
            Ok(p) => p,
            Err(e) => {
                self.rec.note = e;
                self.rec.play_note = self.rec.note.clone();
                return None;
            }
        };
        let argv = vec![script.display().to_string()];
        let env = hsrec::replay::decoder_env(rec_meta, &fallback);
        Some(hsrec::replay::DecoderCmd { argv, env, hold: None })
    }

    fn rec_play(&mut self, id: i64, from_ms: u64) {
        self.rec.player = None;
        self.rec.play_note.clear();
        let Some(rec) = self.rec.rows.iter().find(|r| r.id == id).cloned() else { return };
        // A radio's transmission plays its span of the talkgroup recording.
        let (base, until) = match self.rec.radio_map.get(&id) {
            Some((_, b, u)) => (*b, (*u != u64::MAX).then_some(*u)),
            None => (0, None),
        };
        let from_ms = base + from_ms;
        let path = if rec.offline() {
            match hsrec::export::locate(&rec, &self.rec.drives) {
                Some(p) => p,
                None => {
                    self.rec.note = format!("This recording is on drive {}; attach it to play.", rec.location.trim_start_matches("offline:"));
                    self.rec.play_note = self.rec.note.clone();
                    return;
                }
            }
        } else {
            self.rec.dir.join(&rec.raw_path)
        };
        let demod = self.rec_demod(&rec);
        let (decoder, play_meta) = self.rec_play_plan(&rec, &demod);
        if decoder.is_none() && play_meta.is_digital() && play_meta.source_kind != meta::KIND_DEMOD_AUDIO {
            // Not the raw discriminator instead: say why nothing plays.
            self.rec.selected = Some(id);
            self.rec.detail_open = true;
            self.rec_log(format!("Playback: {}", self.rec.note));
            return;
        }
        let (rate, channels) = decoder
            .as_ref()
            .map(|d| hsrec::replay::script_pcm(&d.argv[0]))
            .unwrap_or((hsrec::replay::OUT_RATE, 1));
        if !self.rec.face.output.is_empty() || !self.rec.sinks_listed {
            self.rec.sinks = hsrec::replay::list_sinks();
            self.rec.sinks_listed = true;
        }
        #[allow(unused_mut)]
        let (mut argv, missing) = hsrec::replay::pick_output(&self.rec.face.output, &self.rec.sinks, rate, channels);
        #[cfg(test)]
        if let Some(a) = &self.rec.play_argv {
            argv = a.clone();
        }
        self.rec.output_note = missing.unwrap_or_default();
        match hsrec::replay::Player::start(&path, &play_meta, from_ms, decoder, argv) {
            Ok(p) => {
                self.rec.player = Some((id, p));
                self.rec.play_base = base;
                self.rec.play_until = until;
                // Every playback shows its controls.
                if self.rec.selected != Some(id) {
                    self.rec.notes_edit = rec.notes.clone();
                }
                self.rec.selected = Some(id);
                self.rec.detail_open = true;
            }
            Err(e) => { self.rec.note = format!("Playback: {e}"); self.rec.play_note = self.rec.note.clone(); }
        }
    }

    /// Demodulators the Record / Playback dialog offers: every modem pipe
    /// Commander has, then analog FM audio. (key, label)
    pub(super) fn rec_demod_options(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for m in crate::rx_pipe::modem_scripts() {
            if out.iter().any(|(k, _)| *k == m.command) {
                continue;
            }
            let kind = m.name.trim_end_matches(".rxpipe").trim_start_matches("commander-demod-").to_uppercase();
            out.push((m.command.clone(), format!("{kind} ({})", m.name)));
        }
        out.push((DEMOD_FM.into(), "FM audio (analog)".into()));
        out
    }

    /// The demodulator a recording plays through: the one picked for it, else
    /// its own decoder pipe (the pipe the radio was decoding with, so a P25
    /// channel recording is decoded, never played as a raw discriminator),
    /// else analog FM audio.
    pub(super) fn rec_demod(&self, rec: &Recording) -> String {
        if let Some(d) = self.rec.demod.get(&rec.id) {
            return d.clone();
        }
        if rec.meta.source_kind == meta::KIND_DEMOD_AUDIO {
            return DEMOD_FM.into();
        }
        let mut m = rec.meta.clone();
        hsrec::safepath::portable_meta(&mut m);
        match m.decoder.get("script").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            Some(script) => script.to_string(),
            None => DEMOD_FM.into(),
        }
    }

    /// Decoder and the metadata playback runs with for `demod`.
    pub(super) fn rec_play_plan(&mut self, rec: &Recording, demod: &str) -> (Option<hsrec::replay::DecoderCmd>, Meta) {
        let mut m = rec.meta.clone();
        if demod == DEMOD_FM || rec.meta.source_kind == meta::KIND_DEMOD_AUDIO {
            if rec.meta.source_kind != meta::KIND_DEMOD_AUDIO {
                m.modulation = "fm".into();
            }
            return (None, m);
        }
        m.decoder["script"] = serde_json::Value::String(demod.to_string());
        if !m.is_digital() {
            // A channel recording is filed as "fm"; the pipe decides.
            let kind = demod.trim_start_matches("modems/").trim_end_matches(".rxpipe").trim_start_matches("commander-demod-");
            m.modulation = if kind.is_empty() || kind == "fm" { "digital".into() } else { kind.to_string() };
        }
        let decoder = self.rec_decoder(&m);
        if decoder.is_none() {
            // Never fall back silently to the raw discriminator for a digital pick.
            if self.rec.note.is_empty() {
                self.rec.note = format!("Demodulator {demod} not found.");
                self.rec.play_note = self.rec.note.clone();
            }
        }
        (decoder, m)
    }

    /// Close the Record / Playback dialog. Closing it stops playback.
    pub(super) fn rec_close_detail(&mut self) {
        self.rec.detail_open = false;
        self.rec.play_note.clear();
        self.rec.player = None;
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
                    let resp = rustcars::bar_toggle_button(ui, &theme, seg.label, &mut local, base, size); // square in the bar (P1c)
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
        if self.rec_icon() == IconState::Error {
            let (_, err) = self.rec_ready();
            ui.label(egui::RichText::new("\u{26a0} REC NOT RECORDING").strong().color(egui::Color32::from_rgb(240, 60, 50)))
                .on_hover_text(status_line(IconState::Error, false, err.as_deref()));
        }
    }

    fn rec_timeline(&mut self, ui: &mut egui::Ui) {
        let now = chrono::Utc::now().timestamp_millis();
        let source = self.rec_source();
        let query = self.rec.query.clone();
        let names = self.unit_names();
        let q = query.trim().to_lowercase();
        let named = |r: &Recording| !q.is_empty() && r.meta.unit_ids().iter().any(|u| names.get(u).is_some_and(|d| d.to_lowercase().contains(&q)));
        let recs: Vec<Recording> = self.rec.rows.iter().filter(|r| timeline::matches(r, &query) || named(r)).cloned().collect();
        let lanes = timeline::lanes(&recs);
        // First look at a source frames its most recent recordings.
        let mut view = match self.rec.views.get(&source) {
            Some(v) => *v,
            None if !recs.is_empty() => timeline::fit(&recs, now),
            None => View::live(now, 3_600_000),
        };
        if std::mem::take(&mut self.rec.go_live) && !view.live {
            view.toggle_live(now);
        }
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
        let viewport_h = timeline_viewport_h(visible_height(ui));
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
        // Parked in the past: a key on the axis goes back to following NOW.
        if back_to_live_key(ui, axis, view.live) {
            self.rec.go_live = true;
            ui.ctx().request_repaint();
        }
        // Wheel (as before tonight): a plain wheel over the timeline zooms
        // time. Shift+wheel, or a plain wheel over the lane names, scrolls
        // the lanes up and down. The scrollbar right of the lanes drags.
        let body_view = egui::Rect::from_min_size(egui::pos2(axis.left(), axis.bottom()), egui::vec2(plot_w, viewport_h));
        let wheel_pos = ui.input(|i| i.pointer.hover_pos());
        // A REC window (Record / Playback, Data) above the timeline owns the
        // pointer there: no zoom, and its own scroll area keeps the wheel.
        let own_layer = ui.layer_id();
        let layer_ctx = ui.ctx().clone();
        let ours = move |p: egui::Pos2| pointer_is_ours(layer_ctx.layer_id_at(p), own_layer);
        let solid_shelf = shelf_w * timeline::SHELF_OPAQUE_FRAC;
        let over = match wheel_pos.filter(|p| ours(*p)) {
            Some(p) if axis.contains(p) => WheelOver::Axis,
            Some(p) if body_view.contains(p) && p.x - body_view.left() < solid_shelf => WheelOver::Names,
            Some(p) if body_view.contains(p) => WheelOver::Lanes,
            Some(p) if p.y >= body_view.top() && p.y <= body_view.bottom() && p.x > body_view.right() && p.x <= body_view.right() + SCROLL_GUTTER + 2.0 => WheelOver::Names,
            _ => WheelOver::Elsewhere,
        };
        let wheel = take_timeline_wheel(ui, over);
        let zoom_scroll = wheel.zoom;
        let lane_off_key = source.clone();
        let mut lane_off = self.rec.lane_off.get(&lane_off_key).copied().unwrap_or(0.0) - wheel.lanes;
        let max_off = (body_h - viewport_h).max(0.0);
        lane_off = if lane_off.is_finite() { lane_off.clamp(0.0, max_off) } else { 0.0 };
        let mut drag_dx = if axis_resp.dragged() { axis_resp.drag_delta().x } else { 0.0 };
        let mut hit: Option<(i64, u64)> = None;
        let mut want_wf = Vec::new();
        let mut lane_click: Option<String> = None;
        let sel_lane = self.rec.lane_sel.get(&source).cloned();
        let by_id: HashMap<i64, &Recording> = recs.iter().map(|r| (r.id, r)).collect();
        let selected_id = self.rec.selected;
        let waveforms = &self.rec.waveforms;
        let no_voice = &self.rec.no_voice;
        let base = self.rec.play_base as i64;
        let player = self.rec.player.as_ref().map(|(id, pl)| (*id, pl.pos_ms.load(Ordering::Relaxed) as i64 - base));
        let shown = lane_scroll(("rec-timeline", source.clone()), viewport_h)
            .vertical_scroll_offset(lane_off)
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
                // NOW tint under the blocks: a call at NOW stays visible.
                let now_x = grid.left() + view.x(now, width);
                let now_font = egui::FontId::monospace(10.0);
                let now_w = ui.fonts_mut(|f| f.layout_no_wrap("NOW".into(), now_font.clone(), egui::Color32::WHITE).size().x);
                if let Some((x0, x1)) = now_fade_span(now_x - grid.left(), plot_w, right_shelf_w(now_w)) {
                    let r = egui::Rect::from_min_max(egui::pos2(grid.left() + x0, grid.top()), egui::pos2(grid.left() + x1, grid.bottom()));
                    paint_shelf_right(&ui.painter_at(rect), r, NOW_SHELF);
                }
                if now_x >= grid.left() && now_x <= grid.right() {
                    gp.line_segment([egui::pos2(now_x, grid.top()), egui::pos2(now_x, grid.bottom())], egui::Stroke::new(1.0, egui::Color32::from_rgb(240, 60, 50)));
                }
                for b in &blocks {
                    let Some(r) = by_id.get(&b.id) else { continue };
                    let y = rect.top() + lane_top[b.lane] + b.sub as f32 * row_h;
                    let mut br = egui::Rect::from_min_max(egui::pos2(grid.left() + b.x0.max(0.0), y + 3.0), egui::pos2(grid.left() + b.x1.min(width), y + row_h - 3.0));
                    if b.x1 < 0.0 || b.x0 > width {
                        continue;
                    }
                    // A short call (or one just started) is never a hairline.
                    if br.width() < MIN_BLOCK_W {
                        br.min.x = (br.max.x - MIN_BLOCK_W).max(grid.left());
                        br.max.x = br.min.x + MIN_BLOCK_W;
                    }
                    let fill = if r.offline() || r.missing {
                        egui::Color32::from_gray(70)
                    } else if b.live {
                        LIVE_BLOCK
                    } else if b.id < 0 {
                        tg_colour(r.meta.talkgroup.unwrap_or(0))
                    } else if r.meta.source_kind == meta::KIND_DEMOD_AUDIO {
                        egui::Color32::from_rgb(40, 90, 120)
                    } else {
                        egui::Color32::from_rgb(46, 120, 70)
                    };
                    gp.rect_filled(br, egui::CornerRadius::ZERO, fill);
                    if let Some(wf) = waveforms.get(&b.id) {
                        let n = wf.len().max(1) as f32;
                        for (k, v) in wf.iter().enumerate() {
                            let x = br.left() + br.width() * k as f32 / n;
                            let a = br.height() * 0.45 * (*v as f32 / 255.0);
                            gp.line_segment([egui::pos2(x, br.center().y - a), egui::pos2(x, br.center().y + a)], egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(220, 240, 220, 120)));
                        }
                    } else if br.width() > 20.0 && b.id > 0 {
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
                    if b.id < 0 && br.width() > 40.0 {
                        tag = format!("{} {tag}", r.meta.talkgroup_name.split(" — ").next().unwrap_or(""));
                    }
                    if !tag.trim().is_empty() && br.width() > 30.0 {
                        gp.text(br.left_center() + egui::vec2(3.0, 0.0), egui::Align2::LEFT_CENTER, tag.trim(), egui::FontId::monospace(10.0), egui::Color32::WHITE);
                    }
                    if r.meta.emergency {
                        gp.rect_stroke(br, egui::CornerRadius::ZERO, egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 60, 40)), egui::StrokeKind::Outside);
                    }
                    if b.live {
                        gp.rect_stroke(br, egui::CornerRadius::ZERO, egui::Stroke::new(1.0, egui::Color32::WHITE), egui::StrokeKind::Inside);
                    }
                    if selected_id == Some(b.id) {
                        gp.rect_stroke(br, egui::CornerRadius::ZERO, egui::Stroke::new(2.0, egui::Color32::WHITE), egui::StrokeKind::Outside);
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
                        let unit = match units_named(&r.meta, &names) { t if t.is_empty() => t, t => format!("\nRadio ID {t}") };
                        let mut tip = format!("{}\n{} ({:.1} s){unit}", r.meta.lane(), start.format("%Y-%m-%d %H:%M:%S"), r.meta.duration_ms as f64 / 1000.0);
                        if b.id < 0 {
                            let tg = r.meta.talkgroup.map(|t| t.to_string()).unwrap_or_default();
                            tip = format!("{}\nTalkgroup: {} ({tg})\n{} ({:.1} s)", r.meta.lane(), r.meta.talkgroup_name, start.format("%Y-%m-%d %H:%M:%S"), r.meta.duration_ms as f64 / 1000.0);
                        }
                        block_tip = Some(tip);
                    }
                    if let Some((pid, pos)) = player {
                        if pid == b.id {
                            let x = grid.left() + view.x(t0 + pos, width);
                            gp.line_segment([egui::pos2(x, br.top() - 2.0), egui::pos2(x, br.bottom() + 2.0)], egui::Stroke::new(2.0, egui::Color32::YELLOW));
                        }
                    }
                }
                // The NOW label sits at the bottom of the visible lanes.
                if now_x >= grid.left() && now_x <= grid.right() {
                    let bottom = grid.bottom().min(ui.clip_rect().bottom());
                    gp.text(egui::pos2(now_x - 2.0, bottom - 2.0), egui::Align2::RIGHT_BOTTOM, "NOW", now_font, egui::Color32::from_rgb(255, 90, 75));
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
        // The scrollbar may have moved it; keep what it shows.
        let off = shown.state.offset().y;
        self.rec.lane_off.insert(lane_off_key, if off.is_finite() { off.clamp(0.0, max_off) } else { 0.0 });
        if let Some(name) = lane_click {
            if self.rec.lane_sel.get(&source) == Some(&name) {
                self.rec.lane_sel.remove(&source);
            } else {
                self.rec.lane_sel.insert(source.clone(), name);
            }
        }
        if drag_dx != 0.0 {
            // Dragging back parks the view; dragging forward onto NOW follows again.
            view.pan(drag_dx, width, now);
        }
        if zoom_scroll.abs() > 0.5 {
            let anchor = wheel_pos.map(|p| p.x - axis.left()).unwrap_or(width).clamp(0.0, width);
            view.zoom_at(wheel_zoom_factor(zoom_scroll), anchor, width, now);
        }
        view.clamp(now);
        // The live edge is the wall clock. The next frame is what moves it.
        // Waiting on decoder lines or a slow poll makes the grid jump.
        if view.live {
            ui.ctx().request_repaint();
        }
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
        let names = self.unit_names();
        if let Some(u) = m.radio_unit_id { put("Radio unit ID", unit_named(u, &names)); }
        let hhmmss = |utc: &str| {
            chrono::DateTime::parse_from_rfc3339(utc)
                .map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
                .unwrap_or_default()
        };
        put(
            "Radio IDs on call",
            m.units
                .iter()
                .map(|u| format!("{} (heard {}–{})", unit_named(u.id, &names), hhmmss(&u.first_utc), hhmmss(&u.last_utc)))
                .collect::<Vec<_>>()
                .join(", "),
        );
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
    /// Demodulator for this recording. A change while it plays restarts it
    /// from the same point.
    fn rec_demod_row(&mut self, ui: &mut egui::Ui, rec: &Recording) {
        let current = self.rec_demod(rec);
        let options = self.rec_demod_options();
        let label = options.iter().find(|(k, _)| *k == current).map(|(_, l)| l.clone()).unwrap_or_else(|| format!("{current} (not found)"));
        let mut pick = None;
        ui.horizontal_wrapped(|ui| {
            ui.label("Demodulator");
            egui::ComboBox::from_id_salt("rec_demod")
                .selected_text(label)
                .width(260.0)
                .show_ui(ui, |ui| {
                    for (k, l) in &options {
                        if ui.selectable_label(*k == current, l).clicked() {
                            pick = Some(k.clone());
                        }
                    }
                })
                .response
                .on_hover_text("How playback turns this recording's raw IQ into sound. Defaults to the decoder the radio used.");
        });
        if let Some(k) = pick.filter(|k| *k != current) {
            self.rec_pick_demod(rec.id, k);
        }
    }

    pub(super) fn rec_pick_demod(&mut self, id: i64, demod: String) {
        self.rec.demod.insert(id, demod);
        if let Some((pid, p)) = &self.rec.player {
            if *pid == id {
                let at = p.pos_ms.load(Ordering::Relaxed);
                self.rec_play(id, at);
            }
        }
    }

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

    /// Status text for this dialog. The no-voice replay sentence stays off it:
    /// the timeline already tags that recording, and the sentence is logged.
    fn detail_note(note: &str) -> bool {
        !note.is_empty() && !note.starts_with("No voice decoded")
    }

    /// Record / Playback dialog: follows the selection, one at a time.
    /// The frame is an elbow. Its end pill closes. Call, Playback and Notes
    /// stay put; everything under them scrolls.
    fn rec_detail_window(&mut self, ctx: &egui::Context) {
        // Playback from anywhere shows its controls.
        if let Some((pid, _)) = &self.rec.player {
            let pid = *pid;
            self.rec.detail_open = true;
            if self.rec.selected != Some(pid) && self.rec.rows.iter().any(|r| r.id == pid) {
                self.rec.selected = Some(pid);
            }
        }
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
        let win_id = egui::Id::unique("rec-playback-window");
        let pos_id = win_id.with("pos");
        let stored = ctx.data_mut(|d| d.get_persisted::<egui::Pos2>(pos_id));
        let mut area = egui::Area::new(win_id).order(egui::Order::Middle).constrain(true);
        if let Some(p) = stored {
            area = area.fixed_pos(p);
        } else {
            let size = egui::vec2(520.0, 640.0);
            area = area.default_pos(ctx.content_rect().center() - size * 0.5);
        }
        let mut drag = egui::Vec2::ZERO;
        let mut close = false;
        let shown = area.show(ctx, |ui| {
            let screen = ui.ctx().content_rect().size();
            egui::Resize::default()
                .id(win_id.with("resize"))
                .default_size(egui::vec2(520.0, 640.0))
                .min_size(egui::vec2(400.0, 360.0))
                .max_size(screen * 0.92)
                .with_stroke(false)
                .show(ui, |ui| {
                    let (_, closed, delta) = crate::group_box::elbow_dialog(ui, "Record / Playback", rustcars::palette::TAN, |ui| {
                        self.rec_detail(ui, rec);
                    });
                    close = closed;
                    drag = delta;
                });
        });
        let mut pos = shown.response.rect.min;
        if drag != egui::Vec2::ZERO {
            pos += drag;
            ctx.request_repaint();
        }
        ctx.data_mut(|d| d.insert_persisted(pos_id, pos));
        if close {
            self.rec_close_detail();
        }
    }

    /// Radio IDs dialog: Notes is the radio's description (station `units`),
    /// shared with the Radio IDs page, the lanes and every tooltip.
    fn rec_radio_notes(&mut self, ui: &mut egui::Ui, rec: &Recording) {
        use rustcars::palette::PLUM;
        let radio = rec.meta.radio_unit_id.filter(|u| *u != 0);
        crate::group_box::group_box(ui, "Notes", PLUM, |ui| {
            let Some(radio) = radio else {
                ui.label("No radio was heard on this call, so there is no radio to describe.");
                return;
            };
            if self.rec.desc_edit.as_ref().is_none_or(|(r, _)| *r != radio) {
                let now = self.units.iter().find(|u| u.id == radio).map(|u| u.description.clone()).unwrap_or_default();
                let now = if now.trim() == radio.to_string() { String::new() } else { now };
                self.rec.desc_edit = Some((radio, now));
            }
            let Some((_, text)) = self.rec.desc_edit.as_mut() else { return };
            let r = ui
                .horizontal(|ui| {
                    ui.label("Description");
                    ui.add(
                        egui::TextEdit::singleline(text)
                            .id_salt(("rec-radio-desc", radio))
                            .hint_text(format!("Radio {radio}"))
                            .desired_width(f32::INFINITY),
                    )
                })
                .inner;
            let enter = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if r.lost_focus() || enter {
                let text = text.clone();
                self.rec.desc_commit = Some((radio, text));
            }
            ui.label(
                egui::RichText::new(format!("Names radio {radio} everywhere: its lane, tooltips, search and the Radio IDs page. Empty shows it as Radio {radio}."))
                    .small(),
            );
        });
    }

    fn rec_detail(&mut self, ui: &mut egui::Ui, rec: Recording) {
        let id = rec.id;
        let local = |utc: &str| {
            chrono::DateTime::parse_from_rfc3339(utc)
                .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|_| if utc.is_empty() { "recording…".into() } else { utc.to_string() })
        };
        use rustcars::palette::{PLUM, SLATE, TAN, TEAL, TEXT};
        let mut star = rec.star;
        let mut protect = rec.protected;
        let mut delete = false;
        // Timeline and find stay on the REC page. This dialog groups the fields that are already here.
        crate::group_box::group_box(ui, "Call", TAN, |ui| {
            ui.heading(rec.meta.lane());
            ui.label(format!(
                "{} → {}   {:.1} s",
                local(&rec.meta.start_utc),
                local(&rec.meta.end_utc),
                rec.meta.duration_ms as f64 / 1000.0
            ));
            ui.horizontal_wrapped(|ui| {
                let red = egui::Color32::from_rgb(240, 60, 50);
                if !hsrec::dsp::in_capture(rec.meta.offset_hz) && rec.meta.source_kind != meta::KIND_DEMOD_AUDIO {
                    ui.label(egui::RichText::new("OUT OF BAND").small().strong().color(red));
                }
                if rec.meta.emergency { ui.label(egui::RichText::new("EMERGENCY").small().strong().color(red)); }
                if rec.meta.priority { ui.label(egui::RichText::new("PRIORITY").small()); }
                if rec.archived { ui.label(egui::RichText::new("archived (zst)").small()); }
                if rec.offline() { ui.label(egui::RichText::new("offline").small()); }
                if rec.missing { ui.label(egui::RichText::new("MISSING").small().color(red)); }
            });
            // "No voice decoded…" is a timeline tag and a log line. It does not
            // belong in the Call section; playback results show under Playback.
            if Self::detail_note(&self.rec.note) && self.rec.note != self.rec.play_note && self.rec.note != self.rec.data_note {
                ui.label(egui::RichText::new(&self.rec.note).color(caution()));
            }
        });
        crate::group_box::group_box(ui, "Playback", TEAL, |ui| {
            ui.horizontal_wrapped(|ui| {
                let playing = self.rec.player.as_ref().is_some_and(|(pid, _)| *pid == id);
                if playing {
                    if ui.button("■ Stop").clicked() {
                        self.rec.player = None;
                    }
                } else if ui.button("▶ Play").on_hover_text("Re-demodulates the raw IQ now, through the current decoder.").clicked() {
                    self.rec_play(id, 0);
                }
                // A Radio IDs block is a span of a talkgroup recording: flags,
                // notes, export and delete stay with that recording's own tab.
                if id > 0 {
                    ui.checkbox(&mut star, "★ Star");
                    ui.checkbox(&mut protect, "🔒 Protect").on_hover_text("Never removed by overwrite-oldest.");
                } else {
                    ui.label(format!("{} — this radio's part of the call. Notes below name the radio; star, export and delete are on the talkgroup's tab.", rec.meta.talkgroup_name));
                }
            });
            // The last playback's result (for example "No voice decoded…").
            if !self.rec.play_note.is_empty() {
                ui.label(egui::RichText::new(&self.rec.play_note).color(caution()));
            }
            self.rec_demod_row(ui, &rec);
            self.rec_output_row(ui);
        });
        if id < 0 {
            self.rec_radio_notes(ui, &rec);
            return;
        }
        crate::group_box::group_box(ui, "Notes", PLUM, |ui| {
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
        });
        // Call, Playback and Notes stay above. Metadata, Export and Danger scroll.
        let rest_h = ui.available_height();
        let rest_h = if rest_h.is_finite() { rest_h.max(0.0) } else { 240.0 };
        egui::ScrollArea::vertical()
            .id_salt(("rec-detail-rest", id))
            .auto_shrink([false, false])
            .max_height(rest_h)
            .show(ui, |ui| {
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
        for (title, colour, keys) in [
            ("Source", TAN, &["Source kind", "Source tab", "Radio", "Frequency", "Capture centre", "Bandwidth", "System", "Site", "Talkgroup", "Channel", "Radio unit ID", "Radio IDs on call", "Sample rate / format", "Gains", "Dropped samples"][..]),
            ("Processing", SLATE, &["Modulation", "Chain", "Decoder", "Software"][..]),
            ("File", TEAL, &["Start", "End", "Duration", "File", "Size", "Archive", "Location", "SHA-256", "Part"][..]),
        ] {
            let part: Vec<_> = rows.iter().filter(|(k, _)| keys.contains(k)).cloned().collect();
            if part.is_empty() {
                continue;
            }
            crate::group_box::group_box(ui, title, colour, |ui| {
                egui::Grid::new(("rec-meta", id, title)).num_columns(2).striped(true).spacing([12.0, 2.0]).show(ui, |ui| {
                    for (k, v) in &part {
                        ui.label(egui::RichText::new(*k).weak());
                        ui.add(egui::Label::new(v).wrap());
                        ui.end_row();
                    }
                });
            });
        }
        // Export and Danger sit below the File section (David).
        crate::group_box::group_box(ui, "Export", TEXT, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Export WAV…").on_hover_text("Demodulates now and writes a WAV. The WAV is not kept by REC.").clicked() {
                    self.rec_export_wav(&rec);
                }
                if ui.button("Export raw…").on_hover_text("Raw IQ (or archive) plus the JSON sidecar.").clicked() {
                    self.rec_export_raw(&rec);
                }
                // Unarchive restores a file. It is not a discard, so it stays out of Danger.
                if rec.archived && ui.add_enabled(self.rec.job.is_none(), egui::Button::new("Unarchive")).clicked() {
                    let dir = self.rec.dir.clone();
                    let one = rec.clone();
                    self.rec_job("Unarchiving", move |db, st, _| {
                        let r = hsrec::archive::unarchive_one(&db, &dir, &one);
                        st.lock().expect("job").lines.push(r.err().unwrap_or_else(|| "Unarchived.".into()));
                    });
                }
            });
        });
        crate::group_box::group_box(ui, "Danger", crate::estop::DANGER, |ui| {
            ui.horizontal_wrapped(|ui| {
                if !rec.archived && ui.add_enabled(self.rec.job.is_none() && !rec.meta.end_utc.is_empty(), egui::Button::new("Archive")).clicked() {
                    self.rec_start_archive(vec![rec.clone()]);
                }
                if ui.button("Delete").on_hover_text("Removes the raw file, its row, and its cache. Undo for 60 s.").clicked() {
                    delete = true;
                }
            });
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
mod page_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn themed_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        rustcars::install_lcars_font(&ctx);
        rustcars::apply_theme(&ctx, &rustcars::LcarsTheme::default());
        ctx
    }

    /// One frame of an axis row like the timeline's, with the LIVE key.
    fn axis_frame(ctx: &egui::Context, events: Vec<egui::Event>, live: bool) -> (bool, Option<egui::Rect>) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 300.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut clicked = false;
        let mut key = None;
        let mut out = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let (axis, _) = ui.allocate_exact_size(egui::vec2(800.0, AXIS_H), egui::Sense::click_and_drag());
                clicked = back_to_live_key(ui, axis, live);
                key = (!live).then(|| back_to_live_rect(axis));
            });
        });
        out.textures_delta.clear();
        (clicked, key)
    }

    #[test]
    fn rec_status_sits_in_the_bottom_status_strip() {
        let ctx = themed_ctx();
        let status = format!("{}{}", status_line(IconState::Writing, true, None), rec_status_tail("all traffic", "off", Some("0.90 / 20.0 GB (4%)".into())));
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 700.0));
        let mut strip_r = egui::Rect::NOTHING;
        let mut frame = egui::Rect::NOTHING;
        let mut line = String::new();
        let mut out = egui::FullOutput::default();
        for _ in 0..3 {
            out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    // As page_rec: split, body first (it may add notes), strip last.
                    let full = page_area(ui);
                    let (body, strip) = status_split(full);
                    crate::status_note::clear(ui.ctx());
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("rec-body"));
                    child.label("Search");
                    crate::status_note::note(&child, "DISK CAP note");
                    line = crate::status_note::paint_strip(ui, strip, &status);
                    ui.allocate_rect(full, egui::Sense::hover());
                    strip_r = strip;
                    frame = full;
                });
            });
            out.textures_delta.clear();
        }
        assert!(line.starts_with("REC: recording now.   TG: all traffic   Channels: off   Disk 0.90 / 20.0 GB (4%)"), "{line}");
        assert!(line.ends_with("DISK CAP note"));
        assert!((strip_r.bottom() - frame.bottom()).abs() < 0.5 && (strip_r.height() - crate::status_note::STRIP_H).abs() < 0.5);
        // The status text is painted inside the strip at the frame's bottom,
        // and the body's first row ("Search") is at the top of the frame.
        let texts: Vec<(String, egui::Rect)> = out.shapes.iter().filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.visual_bounding_rect().intersect(c.clip_rect))),
            _ => None,
        }).collect();
        let st = texts.iter().find(|(t, _)| t.starts_with("REC: recording now")).expect("status text painted");
        assert!(strip_r.expand(1.0).contains_rect(st.1), "status {:?} not in strip {strip_r:?}", st.1);
        let search = texts.iter().find(|(t, _)| t == "Search").expect("body painted");
        assert!(search.1.top() < frame.top() + 30.0, "the body starts at the top");
    }

    #[test]
    fn a_parked_timeline_shows_a_live_key_that_goes_back_to_now() {
        let ctx = themed_ctx();
        let (_, key) = axis_frame(&ctx, vec![], false);
        let key = key.expect("parked view shows the key");
        // Square, on the axis row's right end, and its label fits in it.
        assert_eq!(key.height(), AXIS_H);
        let label_w = ctx.fonts_mut(|f| f.layout_no_wrap(BACK_TO_LIVE.into(), egui::FontId::monospace(10.0), egui::Color32::WHITE).size().x);
        assert!(label_w + 8.0 <= key.width(), "label {label_w} in key {}", key.width());
        let at = key.center();
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let (c1, _) = axis_frame(&ctx, vec![egui::Event::PointerMoved(at)], false);
        let (c2, _) = axis_frame(&ctx, vec![press(true)], false);
        let (c3, _) = axis_frame(&ctx, vec![press(false)], false);
        assert!(!c1 && !c2 && c3, "click on the key reports once");
        // Following NOW: no key, and a click there is the axis's, not the key's.
        let (c4, k4) = axis_frame(&ctx, vec![egui::Event::PointerMoved(at), press(true)], true);
        let (c5, _) = axis_frame(&ctx, vec![press(false)], true);
        assert!(k4.is_none() && !c4 && !c5);
    }

    #[test]
    fn call_units_grant_before_call_and_mid_call_talker() {
        let mut c = CallUnits::default();
        // The grant naming the talker arrives just before the follower picks the call.
        c.sight(100, 51007, "2026-01-01T10:00:00.000Z");
        c.sight(200, 9, "2026-01-01T10:00:00.500Z");
        c.begin(Some(100), "2026-01-01T10:00:01.000Z");
        assert_eq!(c.units().iter().map(|u| u.id).collect::<Vec<_>>(), vec![51007]);
        c.sight(100, 42, "2026-01-01T10:00:04.000Z");
        c.sight(100, 51007, "2026-01-01T10:00:06.000Z");
        let u = c.units();
        assert_eq!(u.iter().map(|u| u.id).collect::<Vec<_>>(), vec![51007, 42]);
        assert_eq!(u[0].last_utc, "2026-01-01T10:00:06.000Z");
        let mut m = Meta::default();
        m.set_units(u);
        assert_eq!(m.radio_unit_id, Some(51007));
        assert_eq!(units_text(&m), "51007 (+1: 42)");
        // A stale sighting from long before the call does not count.
        c.begin(Some(200), "2026-01-01T10:00:30.000Z");
        assert!(c.units().is_empty());
        c.begin(None, "2026-01-01T10:00:31.000Z");
        assert!(c.units().is_empty());
    }

    #[test]
    fn units_text_single_and_none() {
        let mut m = Meta::default();
        assert_eq!(units_text(&m), "");
        m.radio_unit_id = Some(7);
        assert_eq!(units_text(&m), "7");
    }

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
    }

    #[test]
    fn rec_state_is_honest() {
        // Armed keys alone never claim recording.
        assert_eq!(rec_state(false, false, false, None), IconState::Off);
        assert_eq!(rec_state(true, false, false, None), IconState::Armed);
        assert_eq!(rec_state(true, false, true, None), IconState::Armed);
        assert_eq!(rec_state(true, true, true, None), IconState::Writing);
        assert_eq!(rec_state(true, true, true, Some("disk full")), IconState::Error);
        assert_eq!(rec_state(false, false, false, Some("")), IconState::Off);
        let e = status_line(IconState::Error, false, Some("file is not a database"));
        assert!(e.contains("NOT recording") && e.contains("file is not a database"), "{e}");
        assert!(status_line(IconState::Armed, false, None).contains("starting"));
        assert!(status_line(IconState::Off, false, None).contains("off"));
    }

    #[test]
    fn empty_page_gives_hints() {
        let t = empty_hint(RecMode::Marked, 3, RecMode::Off, 0, true, "S.recordings beside the station file");
        assert!(t.contains("Recording is ON") && t.contains("3 marked") && t.contains("running") && t.contains("S.recordings"), "{t}");
        assert!(!t.contains('/'), "no absolute path: {t}");
        let off = empty_hint(RecMode::Off, 0, RecMode::Off, 0, false, "x");
        assert!(off.contains("Recording is OFF") && off.contains("not running") && off.contains("Arm REC"), "{off}");
        let none = empty_hint(RecMode::Marked, 0, RecMode::Off, 0, true, "x");
        assert!(none.contains("Nothing is marked"), "{none}");
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
/// Label of the axis key shown while the timeline is not following NOW.
pub const BACK_TO_LIVE: &str = "LIVE \u{25b6}";

/// Where the back-to-live key sits: the right end of the axis row.
pub fn back_to_live_rect(axis: egui::Rect) -> egui::Rect {
    let w = 64.0f32.min(axis.width());
    egui::Rect::from_min_max(egui::pos2(axis.right() - w, axis.top()), axis.right_bottom())
}

/// The rest of the REC status line after the recorder state.
pub fn rec_status_tail(tg: &str, channels: &str, disk: Option<String>) -> String {
    let mut s = format!("   TG: {tg}   Channels: {channels}");
    if let Some(d) = disk {
        s.push_str("   Disk ");
        s.push_str(&d);
    }
    s
}

/// The page's area that is on screen (a detached page sits in an unbounded
/// scroll area: bound it by what is visible).
pub fn page_area(ui: &egui::Ui) -> egui::Rect {
    let mut full = ui.available_rect_before_wrap();
    let vis = ui.clip_rect();
    if !full.bottom().is_finite() || full.bottom() > vis.bottom() {
        full.max.y = vis.bottom().max(full.top() + 120.0);
    }
    full
}

/// Body above, status strip (the tool pages' height) at the bottom.
pub fn status_split(full: egui::Rect) -> (egui::Rect, egui::Rect) {
    let h = crate::status_note::STRIP_H.min(full.height());
    let strip = egui::Rect::from_min_max(egui::pos2(full.left(), full.bottom() - h), full.max);
    let gap = rustcars::LcarsTheme::default().frame_gap;
    let body = egui::Rect::from_min_max(full.min, egui::pos2(full.right(), (strip.top() - gap).max(full.top())));
    (body, strip)
}

/// Where the pointer was for this frame's wheel input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelOver {
    /// The time axis row.
    Axis,
    /// The lanes (grid), right of the solid name column.
    Lanes,
    /// The solid lane-name column, or the scrollbar gutter.
    Names,
    Elsewhere,
}

/// What one wheel event does on the timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelUse {
    /// Zoom time about the pointer.
    Zoom,
    /// Scroll the lane list up and down.
    Lanes,
}

/// Plain wheel over the axis or lanes zooms time (as it always did);
/// Shift+wheel anywhere on the timeline, or a plain wheel over the names,
/// scrolls the lanes. Ctrl/Cmd+wheel zooms.
pub fn wheel_use(over: WheelOver, mods: egui::Modifiers) -> Option<WheelUse> {
    match over {
        WheelOver::Elsewhere => None,
        _ if mods.shift => Some(WheelUse::Lanes),
        _ if mods.command || mods.ctrl => Some(WheelUse::Zoom),
        WheelOver::Names => Some(WheelUse::Lanes),
        WheelOver::Axis | WheelOver::Lanes => Some(WheelUse::Zoom),
    }
}

/// This frame's timeline wheel: zoom delta (points, + = zoom in) and the
/// lane scroll delta (points, + = toward the top).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimelineWheel {
    pub zoom: f32,
    pub lanes: f32,
}

/// Take this frame's wheel from the raw events, once per event (the
/// smoothed delta repeats a notch over several frames, which zoomed tenfold
/// per click). Shift may arrive as a sideways wheel, so x and y both count.
/// The wheel is then taken from egui so no scroll area also moves.
pub fn take_timeline_wheel(ui: &mut egui::Ui, over: WheelOver) -> TimelineWheel {
    if over == WheelOver::Elsewhere {
        return TimelineWheel::default();
    }
    ui.input_mut(|i| {
        let page = i.content_rect().height();
        let mut w = TimelineWheel::default();
        let mut any = false;
        for e in &i.raw.events {
            let egui::Event::MouseWheel { unit, delta, modifiers, .. } = e else { continue };
            let Some(usage) = wheel_use(over, *modifiers) else { continue };
            any = true;
            let d = delta.y + delta.x;
            let d = match unit {
                egui::MouseWheelUnit::Point => d,
                egui::MouseWheelUnit::Line => d * NOTCH_PTS,
                egui::MouseWheelUnit::Page => d * page,
            };
            match usage {
                WheelUse::Zoom => w.zoom += d,
                WheelUse::Lanes => w.lanes += d,
            }
        }
        if any {
            i.smooth_scroll_delta = egui::Vec2::ZERO;
        }
        w
    })
}

/// The lanes' vertical scroll area: `viewport_h` tall, a thin scrollbar in
/// the gutter whenever the lanes do not fit, wheel and scrollbar both scroll.
pub fn lane_scroll(salt: impl std::hash::Hash + std::fmt::Debug, viewport_h: f32) -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .min_scrolled_height(viewport_h)
        .max_height(viewport_h)
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .scroll_source(egui::scroll_area::ScrollSource { scroll_bar: true, drag: egui::scroll_area::DragScroll::Never, mouse_wheel: false })
}

/// Narrowest block drawn: a short call, or one that just started, is never
/// a hairline under the NOW line.
pub const MIN_BLOCK_W: f32 = 6.0;
/// A recording in progress: bright, with a white edge, over the NOW tint.
pub const LIVE_BLOCK: egui::Color32 = egui::Color32::from_rgb(255, 64, 40);

/// Height left in `ui` that is on screen: a page inside an unbounded
/// (detached) scroll area must not size its lane viewport past the window,
/// or the last lanes sit below it with nothing to scroll.
pub fn visible_height(ui: &egui::Ui) -> f32 {
    let avail = ui.available_height();
    let shown = ui.clip_rect().bottom() - ui.cursor().top();
    if avail.is_finite() { avail.min(shown) } else { shown }
}

/// The square LIVE key on the axis, shown only while the view is parked in
/// the past. True when it was clicked.
pub fn back_to_live_key(ui: &mut egui::Ui, axis: egui::Rect, live: bool) -> bool {
    if live {
        return false;
    }
    let r = back_to_live_rect(axis);
    let key = egui::Button::new(egui::RichText::new(BACK_TO_LIVE).monospace().small().strong().color(egui::Color32::WHITE))
        .fill(NOW_SHELF)
        .corner_radius(egui::CornerRadius::ZERO)
        .min_size(r.size());
    ui.put(r, key).on_hover_text("The lanes are not following the present. Click to follow NOW again.").clicked()
}

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
