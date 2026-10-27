//! Hamspot Commander, phase 1: the rig-control face.
//!
//! The window follows the RT-950 CPS shell: a page tab row at the top
//! and grouped frames in the body. The frames reach the window edge.
//! The controls are the rig face (frequency, meters, VFO, band scales, levels).

mod audio;
mod auto_bay;
mod bay;
mod cat;
mod channels;
mod clock;
mod config;
mod dial;
mod exit_watch;
mod follow;
mod gpredict_hw;
mod hackrf;
mod keys;
mod plan;
mod patchbay;
mod pulse;
mod talkgroups;
mod units;
mod amp;
mod rot;
mod rec_live;
mod rec_ui;
mod pipe_editor;
mod pipes;
mod rx_pipe;
mod scan;
mod tools;
mod group_box;
mod estop;
mod toggle;

/// Config page groups, in page order, with their elbow colours.
pub(crate) const CONFIG_GROUPS: [(&str, egui::Color32); 9] = {
    use rustcars::palette::*;
    [
        ("Station", TAN),
        ("Display", TEXT_MUTED),
        ("Radio", SLATE),
        ("Rotator", OLIVE),
        ("Amplifier", BRICK),
        ("Trunking", PLUM),
        ("Audio patch", TEAL),
        ("Automation patch", PLUM),
        ("RF patch", TAN),
    ]
};

fn cfg_colour(name: &str) -> egui::Color32 {
    CONFIG_GROUPS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
        .unwrap_or(rustcars::palette::TAN)
}

/// Thin titled elbow around one Config group.
fn cfg_group<R>(ui: &mut egui::Ui, name: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if cfg_group_has_estop(name) {
        group_box::group_box_estop(ui, name, cfg_colour(name), add)
    } else {
        group_box::group_box(ui, name, cfg_colour(name), add)
    }
}

/// Config groups whose devices can transmit, key an amplifier or move
/// something carry the E-STOP in their bar (issue #198).
pub(crate) const ESTOP_GROUPS: [&str; 5] = ["Radio", "Rotator", "Amplifier", "Automation patch", "RF patch"];

fn cfg_group_has_estop(name: &str) -> bool {
    ESTOP_GROUPS.contains(&name)
}

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use eframe::egui;
use rigctl::{Command, LinkKind, LiveRig, Rig, RigError, Snapshot};

use dial::{
    channel_dial, channel_window, digit_steps, format_hz, lock_toggle,
    merge_meter_order, meter, meter_columns, meter_faces, meter_hit, meter_origin, move_meter,
    order_with_clock, SYSTEM_CLOCK,
    gqrx_dbfs_to_s9, needle_settled, paint_analog_meter, parse_freq, scale, spectrum_plot,
    step_needle, strength_frac, strength_label, swatch, vfo_label, wheel_notches, zone_scale,
    MeterFace, Needle, PaintSpan, SubBand, HAM, METER_GAP, METER_H, METER_W, SPECTRUM_H,
};
use plan::{BandPlan, Use};

/// The hamspot.com favicon, scaled with nearest-neighbor so the window list
/// and the dock stay sharp. The site file is 16×17, so the result is padded
/// out to a square.
fn hamspot_icon() -> egui::IconData {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/hamspot-favicon.png"))
        .expect("embedded hamspot favicon");
    square_icon(&scale_nearest(&icon, 8))
}

fn scale_nearest(icon: &egui::IconData, factor: u32) -> egui::IconData {
    let width = icon.width * factor;
    let height = icon.height * factor;
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let src = (((y / factor) * icon.width + (x / factor)) * 4) as usize;
            let dst = ((y * width + x) * 4) as usize;
            rgba[dst..dst + 4].copy_from_slice(&icon.rgba[src..src + 4]);
        }
    }
    egui::IconData {
        rgba,
        width,
        height,
    }
}

fn square_icon(icon: &egui::IconData) -> egui::IconData {
    let side = icon.width.max(icon.height);
    if icon.width == side && icon.height == side {
        return icon.clone();
    }
    let mut rgba = vec![0u8; (side * side * 4) as usize];
    let x0 = (side - icon.width) / 2;
    let y0 = (side - icon.height) / 2;
    for y in 0..icon.height {
        for x in 0..icon.width {
            let src = ((y * icon.width + x) * 4) as usize;
            let dst = (((y + y0) * side + (x + x0)) * 4) as usize;
            rgba[dst..dst + 4].copy_from_slice(&icon.rgba[src..src + 4]);
        }
    }
    egui::IconData {
        rgba,
        width: side,
        height: side,
    }
}

fn main() -> eframe::Result {
    if exit_watch::run_cleanup_arg() {
        return Ok(());
    }
    // Dropped when the window returns, and closed by the kernel if the
    // process crashes. The child on the other end of the pipe does the rest.
    let _cleanup = exit_watch::install();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([960.0, 640.0])
            .with_title("Hamspot Commander")
            // Wayland uses this id to find hamspotcommander.desktop, which is
            // what puts the icon in the window list and on the dock.
            .with_app_id("hamspotcommander")
            .with_icon(hamspot_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Hamspot Commander",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

struct Favorite {
    hz: f64,
    mode: String,
    passband_hz: i32,
}

struct PendingTune {
    tune: config::Tune,
    levels: BTreeMap<String, f64>,
    after_gen: u64,
    after_list: u64,
}

/// One clear talkgroup grant the HackRF voice channel can still play.
struct HeardGrant {
    hz: f64,
    tg: u32,
    seq: u64,
}

/// LNA and VGA the Tune sliders last sent. The station file and the hub
/// catch up on a later frame. The sliders keep showing this pair until then.
struct HackrfGainHold {
    id: u64,
    lna: u32,
    vga: u32,
}

#[derive(Clone)]
struct CatDraft {
    id: Option<u64>,
    name: String,
    model: i32,
    device: String,
    serial_speed: String,
    civ: String,
    ptt_type: String,
    ptt_file: String,
    conf: String,
    /// Kept from the row being edited. New radios are not parked.
    park: bool,
    port_label: String,
    error: String,
    driver: cat::RadioDriver,
}

#[derive(Clone)]
struct AmpDraft {
    id: Option<u64>,
    name: String,
    model: i32,
    device: String,
    serial_speed: String,
    conf: String,
    port_label: String,
    error: String,
}

#[derive(Clone)]
struct RotDraft {
    id: Option<u64>,
    name: String,
    model: i32,
    device: String,
    serial_speed: String,
    conf: String,
    port_label: String,
    error: String,
}

struct Shared {
    snap: Snapshot,
    cmds: Vec<Command>,
    /// Bumps once each time the drive loop has published a snapshot for a
    /// `rigctld` it just opened. A loaded config waits for a newer value so
    /// it is not applied to the radio that was already on the screen.
    live_gen: u64,
    /// Bumps once each time a live session publishes a snapshot. Hamlib does
    /// not push a settled S-meter, so the channel scan treats a new value as
    /// the next sample. This is not [`Shared::live_gen`].
    poll_gen: u64,
}

struct App {
    shared: Arc<Mutex<Shared>>,
    wake: Arc<Condvar>,
    face: Arc<Mutex<cat::FaceGate>>,
    hub: Arc<cat::Hub>,
    host: String,
    port: String,
    /// Tune settings from an opened config. Applied after the selected radio
    /// this open started is the one the drive loop is polling.
    pending_tune: Option<PendingTune>,
    models: Vec<cat::HamlibModel>,
    cat_edit: Option<CatDraft>,
    rotators: Arc<rot::Hub>,
    rot_models: Vec<rot::RotModel>,
    rot_edit: Option<RotDraft>,
    amplifiers: Arc<amp::Hub>,
    amp_models: Vec<amp::AmpModel>,
    amp_edit: Option<AmpDraft>,
    amp_follow: std::collections::HashMap<u64, amp::FollowRig>,
    amp_freq_text: String,
    amp_level_vals: std::collections::HashMap<String, u8>,
    /// Apply was sent. The station file is written once the hub has that command.
    pipe_editor: pipe_editor::PipeEditor,
    rx_pipe_pending: Option<(u64, String)>,
    tx_pipe_pending: Option<(u64, String)>,
    favorite: Option<Favorite>,
    show_spread: bool,
    band_mem: HashMap<String, f64>,
    step_hz: f64,
    freq_text: String,
    sub_freq_text: String,
    local_mem: Option<f64>,
    hold_freq: Option<f64>,
    hold_levels: HashMap<String, f64>,
    hold_pass: Option<i32>,
    fine_lock: Option<(f64, f64)>,
    page: Page,
    /// Page to show when Tools is torn off. Clicking Tools then focuses that
    /// window. This is never `Page::Tools`.
    page_return: Page,
    /// Saved page-tab order. Empty means the built-in order.
    tab_order: Vec<String>,
    /// Pointer held on a page tab. It becomes a move only after the pointer
    /// is pulled off the row. Releasing before that selects the page.
    tab_press: Option<TabPress>,
    plan: BandPlan,
    zone_sel: usize,
    band_sel: usize,
    plan_path: String,
    /// Last folder chosen in the band-plan browser. Absent means the working directory.
    plan_dir: Option<PathBuf>,
    plan_note: String,
    /// True when `plan_note` is a transmit refusal. Load and save errors stay
    /// until the next load or save.
    plan_note_ptt: bool,
    /// Text in the open zone-tab rename field. The plan keeps the previous
    /// name while this is empty or only spaces.
    zone_rename: Option<(usize, String)>,
    /// Mode last requested from a bandspread click. A drag across one segment
    /// does not send the same mode again until the rig reports it or the
    /// operator picks another mode.
    mode_hold: Option<String>,
    /// Last split frequency and mode requested by a right-click on the bandspread.
    sub_freq_hold: Option<f64>,
    /// Heights of the Tune VFO row from the previous frame. A horizontal row
    /// pins a shorter neighbor to the top, so each piece is padded to the
    /// tallest of the current VFO, the VFO button frame, and the paired VFO.
    main_cluster_height: f32,
    vfo_frame_height: f32,
    sub_vfo_height: f32,
    sub_mode_hold: Option<String>,
    channels: channels::ChannelFile,
    channel_zone: usize,
    /// Scroll the zone list to the selected row once after a station load.
    /// A later manual scroll is left alone.
    zone_list_follow: bool,
    channel_sel: usize,
    /// Scroll the channel list to the row SCAN just landed on.
    channel_list_follow: bool,
    /// Hops the zone on screen. Not written to the station file.
    memory_scan: scan::Scanner,
    /// Voice marks on the page tabs. Not written except the two settings below.
    voice_pulse: pulse::Pulse,
    /// Brightness pulse. The speaker stays when this is off.
    pulse_on: bool,
    /// Seconds for one brightness cycle.
    pulse_secs: f64,
    /// Trunk voice hang in milliseconds, from the station file.
    voice_hang_ms: u32,
    /// Emergency and priority traffic may take a receiver off a held call.
    override_hold: bool,
    /// Emergency grants heard on control receivers this update.
    trunk_emergency: Vec<(u64, f64, u32)>,
    /// RF patch bay on Config (#98). Visual only.
    patchbay: patchbay::PatchBay,
    patchbay_ui: patchbay::UiState,
    /// Read once from the desktop. The sine stays off. The speaker stays.
    reduced_motion: bool,
    /// Zones double-clicked into the scan list. Stored on the station file.
    /// While this is not empty, SCAN hops these zones in list order.
    scan_zones: BTreeSet<usize>,
    /// Talkgroup zones whose every talkgroup is monitored on a HackRF radio.
    monitor_zones: BTreeSet<usize>,
    /// Talkgroup numbers monitored even when their zone is not.
    monitor_talkgroups: BTreeSet<u32>,
    /// The scan running under a purple list was started as the amber all-zone
    /// scan. Clearing the last marked zone returns to that amber scan.
    scan_amber_origin: bool,
    /// Set while SCAN's own recall is sending, so that tune does not treat
    /// the hop as the operator taking the dial.
    scan_sending: bool,
    channel_path: String,
    /// Last folder chosen in the channel browser. Absent means the working directory.
    channel_dir: Option<PathBuf>,
    channel_note: String,
    talkgroups: talkgroups::TalkgroupFile,
    talkgroup_zone: usize,
    talkgroup_sel: usize,
    talkgroup_path: String,
    talkgroup_dir: Option<PathBuf>,
    talkgroup_note: String,
    /// Keys beside the station file. Not written into the station JSON.
    keys: keys::KeyFile,
    key_sel: usize,
    key_note: String,
    key_id_edit: String,
    key_alg_edit: String,
    key_name_edit: String,
    key_text_edit: String,
    /// Last folder chosen in the key import dialog. Absent means the working directory.
    key_dir: Option<PathBuf>,
    /// Row the editor is showing. A different selection reloads the fields.
    key_shown: Option<usize>,
    /// CSV path last handed to the radios, and the radio-list generation
    /// that received it. A new radio gets the same path.
    key_runtime: String,
    key_runtime_gen: u64,
    /// Radios named on a control channel. The description is the operator's text.
    units: Vec<units::Unit>,
    unit_sel: usize,
    /// A description edit is held until the field is left, then written.
    /// A keystroke does not rewrite the station. Closing the window does.
    unit_desc_dirty: bool,
    /// Control follower enabled per radio. Off leaves the P25 pipe as voice only.
    cc_on: HashMap<u64, bool>,
    /// Talkgroup Follow flags from a station file, waiting until the hub's
    /// radio-list generation moves past the first number.
    pending_control: Option<(u64, Vec<bool>)>,
    /// Last events flag sent to the hub, so a frame does not restart the pipe.
    cc_sent: HashMap<u64, bool>,
    /// Last channel-vocoder command sent per radio. The station file keeps
    /// the kind on the memory. This map only stops a repeat send.
    vocoder_sent: HashMap<u64, (String, String)>,
    cc_voice_sent: HashMap<u64, bool>,
    /// Last control-channel mode sent, so a parked radio is not told again.
    cc_mode_sent: HashMap<u64, String>,
    followers: HashMap<u64, follow::Follower>,
    /// Clear grants still inside the HackRF window, per radio.
    hackrf_open: HashMap<u64, Vec<HeardGrant>>,
    /// The grant on the voice discriminator. `None` is silence.
    hackrf_call: HashMap<u64, Option<(f64, u32)>>,
    /// Hold and hang for the call each HackRF is decoding.
    hackrf_hold: HashMap<u64, follow::VoiceHold>,
    /// A HackRF that left its control cluster for one voice call.
    /// Cleared when that call ends. Not written to the station file.
    hackrf_away: HashMap<u64, crate::hackrf::VoiceAway>,
    /// Clear selected grants heard this frame from radios still on a
    /// control channel. The voice receivers read it after every radio has
    /// been parsed. Not written to the station file.
    trunk_pool: Vec<(u64, f64, u32)>,
    /// Trunking job per radio index. An index that is absent is Both.
    trunk_receivers: Vec<config::TrunkReceiver>,
    hackrf_seq: u64,
    hackrf_cc_feeds: HashMap<u64, Vec<follow::DsdFeed>>,
    hackrf_voice_feed: HashMap<u64, follow::DsdFeed>,
    hackrf_watch_sent: HashMap<u64, (i64, Vec<i64>)>,
    hackrf_voice_sent: HashMap<u64, u64>,
    /// Site word already handed to a HackRF voice decoder. A repeat sync
    /// line must not spawn another process.
    hackrf_p2_sent: HashMap<u64, String>,
    /// Radios whose parked face frequency was already sent as the HackRF dial.
    /// Sending again would clear a capture failure that has already stuck.
    hackrf_dial_seeded: BTreeSet<u64>,
    /// Boards from the last `hackrf_info` while the HackRF dialog is open.
    hackrf_seen: Vec<crate::hackrf::Dongle>,
    hackrf_probe_note: String,
    /// Peak holds the spectrum trace. Clear is the running average.
    spectrum_peak: bool,
    /// LNA position while that slider is dragging. The dongle is unchanged.
    gain_drag_lna: Option<u32>,
    /// VGA position while that slider is dragging. The dongle is unchanged.
    gain_drag_vga: Option<u32>,
    /// Gain sent from the Tune sliders and not yet reflected on the radio row.
    hackrf_gain_hold: Option<HackrfGainHold>,
    spectrum_seq: u64,
    spectrum_center: f64,
    spectrum_avg: Vec<f32>,
    spectrum_peak_hold: Vec<f32>,
    /// Scanner poll for a HackRF capture. The frame sequence restarts when
    /// the LO moves, and a repeated count means the scanner has no new sample.
    hackrf_scan_poll: u64,
    hackrf_scan_seq: u64,
    hackrf_scan_center: f64,
    feeds: HashMap<u64, CcFeed>,
    /// Last decoder stderr lines per radio. The hub queue is drained for
    /// the follower, so this is what the Automation frame shows.
    cc_tail: HashMap<u64, Vec<String>>,
    /// Talkgroup each receiver's readout is holding.
    held_call: HashMap<u64, Option<u32>>,
    /// Show registration, authentication, and other housekeeping lines in
    /// the decoder log. Off at launch; the radio list keeps their facts.
    show_housekeeping: bool,
    /// Talkgroups with a recent grant or voice, for the TG list tint.
    tg_recent: pulse::Recent,
    /// Last direct call logged per radio, so grant updates do not repeat it.
    direct_seen: HashMap<u64, (u32, u32, std::time::Instant)>,
    /// Last decoded voice frame per radio (LDU1/LDU2, 4V/2V). A grant alone
    /// is not voice.
    voice_frames: HashMap<u64, std::time::Instant>,
    /// Last Phase 1 sync identity per radio, for the scanner face.
    cc_site: HashMap<u64, follow::SiteBanner>,
    /// Callsign, talkgroup, and access code from decoder text. Not a tune.
    cc_identity: HashMap<u64, follow::AirIdentity>,
    /// Which trunking lines may reach `cc_identity`.
    identity_gate: HashMap<u64, follow::IdentityGate>,
    /// Last invert flag sent per radio. The channel file owns the value.
    invert_sent: HashMap<u64, bool>,
    /// Trunking-system time as a skew from this machine's clock.
    system_clock: clock::SystemClock,
    config_path: String,
    /// Last folder chosen in the config browser. Absent means the working directory.
    config_dir: Option<PathBuf>,
    config_note: String,
    /// Station last loaded or written. Exit compares the face to this.
    station_saved: Option<config::ConfigFile>,
    /// The window close was held so the operator can save or drop the station.
    exit_prompt: bool,
    /// The operator chose to close without writing. The next close must not ask.
    exit_discard: bool,
    /// Canonical audio graph last written, so a quiet PipeWire graph is not saved again.
    audio_saved: String,
    /// A PipeWire reset is already talking to systemd. The button stays dark.
    pipewire_reset: Arc<AtomicBool>,
    /// A loaded file is not written back until the cards it asked for have appeared.
    audio_hold: bool,
    speaker_tex: Option<egui::TextureHandle>,
    mic_tex: Option<egui::TextureHandle>,
    radio_tex: Option<egui::TextureHandle>,
    scope_tex: Option<egui::TextureHandle>,
    /// `None` means the theme had no icon, so the frame does not ask again.
    xdg_tex: HashMap<String, Option<egui::TextureHandle>>,
    bay_drag: Option<bay::Drag>,
    /// Last Config-tab set written into Gpredict's hwconf. Unchanged rows are not written again.
    gpredict_sig: String,
    gpredict_note: String,
    gpredict_files: Vec<gpredict_hw::ShownFile>,
    gpredict_wires: Vec<gpredict_hw::OwnedWire>,
    /// Meter order, lock, needles, and whether the page is in its own window.
    meters: Arc<Mutex<MeterState>>,
    /// Latest radio reading for the detached window. The root writes it.
    meter_feed: Arc<Mutex<MeterFeed>>,
    /// Window icon, shared so a new viewport does not rebuild the PNG.
    icon: Arc<egui::IconData>,
    /// Position and size handed to the meter window when it opens.
    /// Left alone while the window is open, so a drag is not pulled back.
    meter_place: Option<egui::Pos2>,
    meter_size: Option<egui::Vec2>,
    meter_placed: bool,
    /// Calculator, station log, and the other rustcars screens.
    tools: Arc<Mutex<tools::Tools>>,
    /// Position and size handed to the tools window when it opens.
    tool_place: Option<egui::Pos2>,
    tool_size: Option<egui::Vec2>,
    tool_placed: bool,
    /// REC recorder state (#119).
    rec: rec_ui::RecUi,
}

/// Meter grid state. The main window and the detached window share one copy.
struct MeterState {
    locked: bool,
    order: Vec<String>,
    drag: Option<MeterDrag>,
    needles: HashMap<String, Needle>,
    /// The meters are a separate native window.
    detached: bool,
    /// The detached window asked to come back into the main window.
    dock: bool,
    /// Last known outer position and inner size. Absent on Wayland.
    saved_pos: Option<egui::Pos2>,
    saved_size: Option<egui::Vec2>,
    /// Monitor size in points, when the window system reports it.
    saved_monitor: Option<egui::Vec2>,
    /// OS pixels per point, when the window system reports it.
    saved_ppp: Option<f32>,
}

struct MeterFeed {
    snap: Snapshot,
    radio: Option<String>,
    clock: clock::SystemClock,
}

fn meter_viewport() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("hamspot-meters")
}

fn parse_point(text: &str) -> Option<egui::Pos2> {
    let (x, y) = text.split_once(',')?;
    let x: f32 = x.trim().parse().ok()?;
    let y: f32 = y.trim().parse().ok()?;
    if x.is_finite() && y.is_finite() {
        Some(egui::pos2(x, y))
    } else {
        None
    }
}

fn parse_size(text: &str) -> Option<egui::Vec2> {
    let pos = parse_point(text)?;
    if pos.x >= 64.0 && pos.y >= 64.0 {
        Some(egui::vec2(pos.x, pos.y))
    } else {
        None
    }
}

#[derive(Clone)]
struct MeterDrag {
    id: String,
    grab: egui::Vec2,
}

/// True when `name` is a file on `PATH` with an execute bit. A missing
/// `PATH` or a non-executable file is false, so the Audio frame can keep
/// the Helvum button visible and disabled.
fn executable_in_path(path_env: Option<&std::ffi::OsStr>, name: &str) -> bool {
    let Some(path_env) = path_env else {
        return false;
    };
    std::env::split_paths(path_env).any(|dir| is_executable(&dir.join(name)))
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

fn helvum_on_path() -> bool {
    executable_in_path(std::env::var_os("PATH").as_deref(), "helvum")
}

fn gpredict_on_path() -> bool {
    executable_in_path(std::env::var_os("PATH").as_deref(), "gpredict")
}

/// Identity of the rows whose hardware files should exist. A quiet frame does not write again.
fn gpredict_signature(radios: &cat::HubView, rots: &rot::RotView) -> String {
    let mut sig = String::new();
    for row in &radios.rows {
        let port = row.port.map(|port| port.to_string()).unwrap_or_default();
        sig.push_str(&format!(
            "r\t{}\t{}\t{}\t{port}\n",
            row.id,
            cat::driver_name(row.spec.driver),
            row.spec.name
        ));
    }
    for row in &rots.rows {
        let port = row.port.map(|port| port.to_string()).unwrap_or_default();
        sig.push_str(&format!("t\t{}\t{}\t{port}\n", row.id, row.spec.name));
    }
    sig
}

/// Follow is on: Tune must not write a frequency over the one Gpredict set.
fn frequency_writes_allowed(follow: bool) -> bool {
    !follow
}

/// Starts Helvum in its own process group. Hub shutdown only signals the
/// radio and `pw-dump` process groups, so this editor stays up. The waiter
/// reaps Helvum when it exits.
fn paint_curve(
    painter: &egui::Painter,
    origin: egui::Pos2,
    from: [f32; 2],
    to: [f32; 2],
    color: egui::Color32,
) {
    let points = bay::curve_points(from, to);
    let shape = egui::epaint::CubicBezierShape::from_points_stroke(
        [
            origin + egui::vec2(points[0][0], points[0][1]),
            origin + egui::vec2(points[1][0], points[1][1]),
            origin + egui::vec2(points[2][0], points[2][1]),
            origin + egui::vec2(points[3][0], points[3][1]),
        ],
        false,
        egui::Color32::TRANSPARENT,
        egui::Stroke::new(2.0, color),
    );
    painter.add(shape);
}

fn png_texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Option<egui::TextureHandle> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR))
}

/// Rasterize one XDG icon through the active GTK theme.
///
/// The microphone icon in the current theme is SVG only. GTK already knows
/// the theme and how to color a symbolic icon. The process exit is the result.
fn xdg_icon_png(name: &str) -> Option<Vec<u8>> {
    let script = r#"
import sys
import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk
Gtk.init([])
theme = Gtk.IconTheme.get_default()
info = None
for candidate in sys.argv[1:]:
    info = theme.lookup_icon(candidate, 64, 0)
    if info is not None:
        break
if info is None:
    sys.exit(1)
pixbuf = info.load_icon()
ok, buf = pixbuf.save_to_bufferv("png", [], [])
if not ok:
    sys.exit(1)
sys.stdout.buffer.write(buf)
"#;
    let symbolic = format!("{name}-symbolic");
    let output = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(name)
        .arg(&symbolic)
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    Some(output.stdout)
}

fn launch_helvum() {
    launch_detached("helvum");
}

/// Gpredict has no radio argument. The operator points it at the listen
/// line in the Automation frame. Its process group is not one the hub stops.
fn launch_gpredict() {
    let Some(pid) = spawn_grouped("gpredict") else {
        return;
    };
    exit_watch::note_gpredict(pid);
}

fn spawn_grouped(program: &str) -> Option<u32> {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(program);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0);
    let mut child = cmd.spawn().ok()?;
    let pid = child.id();
    thread::spawn(move || {
        let _ = child.wait();
    });
    Some(pid)
}

fn launch_detached(program: &str) {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(program);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0);
    if let Ok(mut child) = cmd.spawn() {
        thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

fn audio_device_label(devices: &[crate::audio::AudioNode], name: &str) -> String {
    devices
        .iter()
        .find(|device| device.name == name)
        .map(|device| {
            if device.description.is_empty() {
                device.name.clone()
            } else {
                device.description.clone()
            }
        })
        .unwrap_or_else(|| {
            if name.is_empty() {
                "default".into()
            } else {
                name.to_string()
            }
        })
}

fn rx_pairs(snap: &Snapshot) -> Vec<(f64, f64)> {
    snap.caps
        .rx_ranges
        .iter()
        .map(|range| (range.min, range.max))
        .collect()
}

fn rx_fallback(snap: &Snapshot) -> (f64, f64) {
    if snap.caps.rx_ranges.is_empty() {
        return (1_800_000.0, 29_700_000.0);
    }
    let min = snap
        .caps
        .rx_ranges
        .iter()
        .map(|range| range.min)
        .fold(f64::MAX, f64::min);
    let max = snap
        .caps
        .rx_ranges
        .iter()
        .map(|range| range.max)
        .fold(f64::MIN, f64::max);
    (min, max)
}

fn mode_choices(snap: &Snapshot) -> Vec<String> {
    if snap.caps.modes.is_empty() {
        ["USB", "LSB", "CW", "AM", "FM"]
            .into_iter()
            .map(str::to_string)
            .collect()
    } else {
        snap.caps.modes.clone()
    }
}

#[cfg(test)]
mod helvum_tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn helvum_is_enabled_only_when_the_file_on_path_is_executable() {
        let dir = std::env::temp_dir().join(format!("hs-helvum-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("helvum");
        fs::write(&bin, b"").unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&bin, perms.clone()).unwrap();
        assert!(!super::executable_in_path(Some(dir.as_os_str()), "helvum"));
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();
        assert!(super::executable_in_path(Some(dir.as_os_str()), "helvum"));
        assert!(!super::executable_in_path(None, "helvum"));
        let gpredict = dir.join("gpredict");
        fs::write(&gpredict, b"").unwrap();
        let mut perms = fs::metadata(&gpredict).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&gpredict, perms.clone()).unwrap();
        assert!(!super::executable_in_path(
            Some(dir.as_os_str()),
            "gpredict"
        ));
        perms.set_mode(0o755);
        fs::set_permissions(&gpredict, perms).unwrap();
        assert!(super::executable_in_path(Some(dir.as_os_str()), "gpredict"));
        assert!(!super::frequency_writes_allowed(true));
        assert!(super::frequency_writes_allowed(false));
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod talkgroup_tab_tests {
    use super::{directory_has_talkgroups, talkgroup_radio_ids, Page};

    fn row(
        id: u64,
        driver: crate::cat::RadioDriver,
        name: &str,
        rx_pipe: &str,
        selected: bool,
    ) -> crate::cat::RadioRow {
        let mut spec = crate::cat::RadioSpec::dummy();
        spec.driver = driver;
        spec.name = name.into();
        spec.model = 0;
        spec.park = false;
        spec.rx_pipe = rx_pipe.into();
        crate::cat::RadioRow {
            id,
            spec,
            port: None,
            status: String::new(),
            selected,
            make: String::new(),
            model_label: String::new(),
            audio_status: String::new(),
            rx_pipe_status: String::new(),
            rx_chain: String::new(),
            rx_pipe_diverted: false,
            tx_pipe_status: String::new(),
            tx_pipe_diverted: false,
            center_hz: 0.0,
        }
    }

    #[test]
    fn a_saved_directory_keeps_the_tab_when_the_p25_pipe_is_off() {
        let rows = [
            row(1, crate::cat::RadioDriver::Hamlib, "Dummy", "", true),
            row(3, crate::cat::RadioDriver::Gqrx, "HackRF", "", false),
        ];
        let mut file = crate::talkgroups::empty();
        assert!(!directory_has_talkgroups(&file));
        assert!(talkgroup_radio_ids(&rows, false).is_empty());
        file.zones[0].talkgroups.push(crate::talkgroups::Talkgroup { record: false,
            name: "ISP".into(),
            tg: 10006,
            encrypted: false,
            priority: false,
        });
        assert_eq!(talkgroup_radio_ids(&rows, true), vec![3]);
        let tabs = super::page_tabs(&rows, &file, &[]);
        assert!(tabs.iter().any(|(page, label)| {
            *page == Page::Talkgroups(3) && label == "HackRF TG"
        }));
        assert!(tabs.iter().all(|(page, _)| !matches!(page, Page::RadioIds(_))));
        let units = vec![crate::units::Unit::named(42)];
        let empty = crate::talkgroups::empty();
        let radio_tabs = super::page_tabs(&rows, &empty, &units);
        assert!(radio_tabs.iter().any(|(page, label)| {
            *page == Page::RadioIds(3) && label == "HackRF ID"
        }));
        assert!(radio_tabs
            .iter()
            .all(|(page, _)| !matches!(page, Page::Talkgroups(_))));
        let both = super::page_tabs(&rows, &file, &units);
        let talk_at = both
            .iter()
            .position(|(page, _)| *page == Page::Talkgroups(3))
            .expect("talkgroups");
        let radio_at = both
            .iter()
            .position(|(page, _)| *page == Page::RadioIds(3))
            .expect("radio ids");
        assert!(talk_at < radio_at);
        let meters_at = both
            .iter()
            .position(|(page, _)| *page == Page::Meters)
            .expect("meters");
        let timers_at = both
            .iter()
            .position(|(page, _)| *page == Page::Timers)
            .expect("timers");
        assert_eq!(meters_at + 1, timers_at);
        let keys_at = both
            .iter()
            .position(|(page, _)| *page == Page::Keys)
            .expect("keys");
        let tools_at = both
            .iter()
            .position(|(page, _)| *page == Page::Tools)
            .expect("tools");
        let config_at = both
            .iter()
            .position(|(page, _)| *page == Page::Config)
            .expect("config");
        assert_eq!(keys_at + 1, tools_at);
        assert_eq!(tools_at + 1, config_at);
        let piped = [
            row(1, crate::cat::RadioDriver::Hamlib, "Dummy", "", false),
            row(
                3,
                crate::cat::RadioDriver::Gqrx,
                "HackRF",
                "/opt/modems/commander-demod-p25.rxpipe",
                true,
            ),
        ];
        assert_eq!(talkgroup_radio_ids(&piped, true), vec![3]);
        let tap = [row(
            9,
            crate::cat::RadioDriver::Tap,
            "Desk",
            "commander-demod-p25.rxpipe",
            true,
        )];
        assert!(talkgroup_radio_ids(&tap, true).is_empty());
        assert!(talkgroup_radio_ids(&tap, false).is_empty());
    }

    #[test]
    fn tab_order_follows_the_station_and_inserts_a_new_tab() {
        let rows = [
            row(1, crate::cat::RadioDriver::Hamlib, "Dummy", "", true),
            row(3, crate::cat::RadioDriver::Gqrx, "HackRF", "", false),
        ];
        let mut file = crate::talkgroups::empty();
        file.zones[0].talkgroups.push(crate::talkgroups::Talkgroup { record: false,
            name: "ISP".into(),
            tg: 10006,
            encrypted: false,
            priority: false,
        });
        let units = vec![crate::units::Unit::named(42)];
        let default_tabs = super::page_tabs(&rows, &file, &units);
        let default_keys: Vec<_> = default_tabs
            .iter()
            .map(|(page, _)| super::page_name(*page))
            .collect();
        let same: Vec<_> = super::apply_tab_order(default_tabs.clone(), &[])
            .iter()
            .map(|(page, _)| super::page_name(*page))
            .collect();
        assert_eq!(same, default_keys);
        let saved = vec!["Config".into(), "Tune".into(), "Channels".into()];
        let ordered = super::apply_tab_order(default_tabs.clone(), &saved);
        let keys: Vec<_> = ordered
            .iter()
            .map(|(page, _)| super::page_name(*page))
            .collect::<Vec<_>>();
        assert_eq!(keys[0], "Config");
        assert_eq!(keys[1], "Tune");
        let channels_at = keys.iter().position(|key| key == "Channels").unwrap();
        let tg_at = keys.iter().position(|key| key == "Talkgroups:3").unwrap();
        assert!(channels_at < tg_at);
        assert!(keys.contains(&"Plan".to_string()));
        let mut moved = default_keys.clone();
        super::move_tab(&mut moved, "Tune", "Channels");
        assert_eq!(moved[0], "Plan");
        assert_eq!(moved[1], "Channels");
        assert_eq!(moved[2], "Tune");
        let mut left = default_keys;
        super::move_tab(&mut left, "Config", "Tune");
        assert_eq!(left[0], "Config");
        assert_eq!(left[1], "Tune");
    }
}

#[cfg(test)]
mod icon_tests {
    #[test]
    fn favicon_becomes_a_square_window_icon() {
        let icon = super::hamspot_icon();
        assert_eq!(icon.width, icon.height);
        assert!(icon.width >= 128);
        assert_eq!(
            icon.rgba.len(),
            icon.width as usize * icon.height as usize * 4
        );
    }
}

#[cfg(test)]
mod folder_tab_tests {
    #[test]
    fn neighboring_folder_colors_differ() {
        let count = 6;
        for index in 0..count {
            assert_ne!(
                super::tab_color(index),
                super::tab_color((index + 1) % count)
            );
        }
    }
}

#[cfg(test)]
mod column_tests {
    #[test]
    fn wide_row_gives_the_extra_to_the_flex_column() {
        let widths = super::fit_columns(1000.0, 8.0, &[280.0, 220.0, 320.0], 1);
        let sum = widths[0] + widths[1] + widths[2] + 16.0;
        assert!((sum - 1000.0).abs() < 0.1);
        assert!((widths[0] - 280.0).abs() < 0.1);
        assert!((widths[2] - 320.0).abs() < 0.1);
        assert!(widths[1] > 220.0);
    }

    #[test]
    fn narrow_row_stays_inside_the_window() {
        let widths = super::fit_columns(500.0, 8.0, &[280.0, 220.0, 320.0], 1);
        let sum = widths[0] + widths[1] + widths[2] + 16.0;
        assert!((sum - 500.0).abs() < 0.1);
        assert!(widths.iter().all(|width| *width > 0.0));
    }
}

fn file_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string())
}

fn remembered_dir(dir: &Option<PathBuf>) -> PathBuf {
    dir.clone()
        .filter(|dir| dir.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn mhz_drag(ui: &mut egui::Ui, label: &str, mhz: &mut f64) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::DragValue::new(mhz)
                .speed(0.001)
                .range(0.0..=1500.0)
                .clamp_existing_to_range(false)
                .max_decimals(6)
                .suffix(" MHz"),
        )
    })
    .inner
    .changed()
}

fn paint_spans(zone: &plan::Zone) -> Vec<PaintSpan> {
    let mut spans: Vec<(u8, PaintSpan)> = zone
        .bands
        .iter()
        .flat_map(|band| band.segments.iter())
        .map(|seg| {
            let rank = seg.service.as_deref().map(plan::service_rank).unwrap_or(0);
            let [r, g, b] = plan::segment_rgb(seg);
            (
                rank,
                PaintSpan {
                    lo: seg.lo_hz,
                    hi: seg.hi_hz,
                    color: egui::Color32::from_rgb(r, g, b),
                },
            )
        })
        .collect();
    // Later paint covers an overlap. Ham sits above broadcast.
    spans.sort_by_key(|(rank, span)| (*rank, span.lo as i64));
    spans.into_iter().map(|(_, span)| span).collect()
}

fn legend_items(zone: &plan::Zone, lo: f64, hi: f64) -> Vec<(egui::Color32, String, bool)> {
    let tagged = zone.bands.iter().any(|band| {
        band.segments
            .iter()
            .any(|seg| seg.service.as_ref().is_some_and(|name| !name.is_empty()))
    });
    if !tagged {
        return [Use::Cw, Use::Digital, Use::Voice, Use::Mixed]
            .into_iter()
            .map(|kind| {
                let drawn = use_in_window(zone, kind, lo, hi);
                let [r, g, b] = kind.rgb();
                (
                    egui::Color32::from_rgb(r, g, b),
                    kind.label().to_string(),
                    drawn,
                )
            })
            .collect();
    }
    plan::SERVICES
        .iter()
        .filter(|name| {
            zone.bands.iter().any(|band| {
                band.segments
                    .iter()
                    .any(|seg| seg.service.as_deref() == Some(**name))
            })
        })
        .map(|name| {
            let drawn = zone.bands.iter().any(|band| {
                band.segments.iter().any(|seg| {
                    seg.service.as_deref() == Some(*name) && seg.hi_hz >= lo && seg.lo_hz <= hi
                })
            });
            let [r, g, b] = plan::service_rgb(name);
            (
                egui::Color32::from_rgb(r, g, b),
                plan::service_label(name),
                drawn,
            )
        })
        .collect()
}

fn use_in_window(zone: &plan::Zone, kind: Use, lo: f64, hi: f64) -> bool {
    zone.bands.iter().any(|band| {
        band.segments
            .iter()
            .any(|seg| seg.use_kind == kind && seg.hi_hz >= lo && seg.lo_hz <= hi)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Tune,
    Plan,
    Channels,
    /// Station key list. The bytes are not in the station JSON.
    Keys,
    /// rustcars screens: NanoVNA, RF Calc, Calc, Smith, Hex, and Logbook.
    Tools,
    /// Station clocks. Drawn like Tools, on the page row beside Meters.
    Timers,
    /// Talkgroup directory for one radio's control-channel follower.
    Talkgroups(u64),
    /// Radios named on that control channel.
    RadioIds(u64),
    Meters,
    Levels,
    Config,
    /// Recordings (#119): source tabs and the timeline.
    Rec,
}

const PAGES: &[(Page, &str)] = &[
    (Page::Tune, "Tune"),
    (Page::Plan, "Plan"),
    (Page::Channels, "Channels"),
    (Page::Rec, "REC"),
    (Page::Meters, "Meters"),
    (Page::Timers, "Timers"),
    (Page::Levels, "Levels"),
    (Page::Keys, "Keys"),
    (Page::Tools, "Tools"),
    (Page::Config, "Config"),
];

fn page_name(page: Page) -> String {
    match page {
        Page::Talkgroups(id) => return format!("Talkgroups:{id}"),
        Page::RadioIds(id) => return format!("RadioIds:{id}"),
        _ => {}
    }
    PAGES
        .iter()
        .find(|(id, _)| *id == page)
        .map(|(_, name)| (*name).to_string())
        .unwrap_or_else(|| "Tune".to_string())
}

fn page_from_name(name: &str) -> Page {
    if let Some(id) = name.strip_prefix("Talkgroups:") {
        if let Ok(id) = id.parse() {
            return Page::Talkgroups(id);
        }
    }
    if let Some(id) = name.strip_prefix("RadioIds:") {
        if let Ok(id) = id.parse() {
            return Page::RadioIds(id);
        }
    }
    PAGES
        .iter()
        .find(|(_, label)| *label == name)
        .map(|(page, _)| *page)
        .unwrap_or(Page::Tune)
}

fn directory_has_talkgroups(file: &talkgroups::TalkgroupFile) -> bool {
    file.zones.iter().any(|zone| !zone.talkgroups.is_empty())
}

/// Which decoder stderr this radio is feeding. A pipe change starts a new
/// feed so a P25 grant cannot finish on a DMR or NXDN line.
enum CcFeed {
    P25(follow::DsdFeed),
    Dmr(follow::DmrFeed),
    Nxdn(follow::NxdnFeed),
}

impl CcFeed {
    fn for_pipe(command: &str) -> Self {
        if crate::rx_pipe::is_dmr_rx(command) {
            CcFeed::Dmr(follow::DmrFeed::default())
        } else if crate::rx_pipe::is_nxdn_trunk(command) {
            CcFeed::Nxdn(follow::NxdnFeed::default())
        } else {
            CcFeed::P25(follow::DsdFeed::default())
        }
    }

    fn matches(&self, command: &str) -> bool {
        match self {
            CcFeed::Dmr(_) => crate::rx_pipe::is_dmr_rx(command),
            CcFeed::Nxdn(_) => crate::rx_pipe::is_nxdn_trunk(command),
            CcFeed::P25(_) => crate::rx_pipe::is_p25_rx(command),
        }
    }

    fn push_line(&mut self, line: &str) -> Vec<follow::CcEvent> {
        match self {
            CcFeed::P25(feed) => feed.push_line(line),
            CcFeed::Dmr(feed) => feed.push_line(line),
            CcFeed::Nxdn(feed) => feed.push_line(line),
        }
    }
}

/// Radios that get a Talkgroups tab. A live trunking receive pipe wins. With
/// the pipe off, a saved directory still keeps a tab on each GQRX radio so the
/// list does not disappear when the decoder is stopped.
/// A trunking receive pipe Commander may follow. An audio tap plays that
/// demodulator from a discriminator and is not tuned.
fn hears_trunk_pipe(row: &cat::RadioRow) -> bool {
    row.spec.driver != cat::RadioDriver::Tap && crate::rx_pipe::is_trunk_rx(&row.spec.rx_pipe)
}

fn talkgroup_radio_ids(rows: &[cat::RadioRow], directory: bool) -> Vec<u64> {
    let piped: Vec<u64> = rows
        .iter()
        .filter(|row| row.spec.driver == cat::RadioDriver::Hackrf || hears_trunk_pipe(row))
        .map(|row| row.id)
        .collect();
    if !piped.is_empty() {
        return piped;
    }
    if !directory {
        return Vec::new();
    }
    let gqrx: Vec<u64> = rows
        .iter()
        .filter(|row| row.spec.driver == cat::RadioDriver::Gqrx)
        .map(|row| row.id)
        .collect();
    if !gqrx.is_empty() {
        return gqrx;
    }
    rows.iter()
        .filter(|row| row.selected && row.spec.driver != cat::RadioDriver::Tap)
        .map(|row| row.id)
        .collect()
}

fn radio_listen(row: &cat::RadioRow) -> String {
    if row.spec.driver == cat::RadioDriver::Hackrf {
        let serial = row.spec.device.trim();
        if serial.is_empty() {
            "direct".to_string()
        } else {
            serial.to_string()
        }
    } else if row.spec.driver == cat::RadioDriver::Tap {
        "audio".to_string()
    } else {
        match row.port {
            Some(port) => format!("127.0.0.1:{port}"),
            None => "127.0.0.1".to_string(),
        }
    }
}

fn talkgroup_zone_of(file: &talkgroups::TalkgroupFile, tg: u32) -> Option<usize> {
    file.zones
        .iter()
        .position(|zone| zone.talkgroups.iter().any(|row| row.tg == tg))
}

fn tab_color(index: usize) -> egui::Color32 {
    const COLORS: [egui::Color32; 6] = [
        rustcars::palette::TAN,
        rustcars::palette::SLATE,
        rustcars::palette::OLIVE,
        rustcars::palette::BRICK,
        rustcars::palette::TEAL,
        rustcars::palette::PLUM,
    ];
    COLORS[index % COLORS.len()]
}

/// Caution text. This is the theme alert, which is also the Meters bar.
/// It is not a separate caution role. See the rustcars issue for that gap.
/// Steady amber edge on a talkgroup or zone row with recent traffic. It is
/// the non-motion cue and fades with the traffic level.
/// Alpha of the talkgroup and channel details panels: mostly opaque, so list
/// text never bleeds through, but not a solid slab.
const DETAILS_PANEL_ALPHA: f32 = 0.94;

/// Group frame for the details panels, filled with the theme panel colour.
fn details_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::group(ui.style()).fill(ui.visuals().panel_fill.gamma_multiply(DETAILS_PANEL_ALPHA))
}

fn recent_bar(ui: &egui::Ui, rect: egui::Rect, level: f32) {
    if level <= 0.0 {
        return;
    }
    let bar = egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height()));
    let alpha = 0.35 + 0.65 * level.clamp(0.0, 1.0);
    ui.painter()
        .rect_filled(bar, egui::CornerRadius::ZERO, caution().gamma_multiply(alpha));
}

/// Whether a radio row can transmit in the patch bay. HackRF, GQRX and line
/// taps are receive only in Commander today.
fn patch_radio_tx(spec: &cat::RadioSpec) -> bool {
    matches!(spec.driver, cat::RadioDriver::Hamlib)
}

pub(crate) fn caution() -> egui::Color32 {
    rustcars::LcarsTheme::default().alert
}

/// A page or zone tab drawn as an LCARS segment. The open tab is lifted.
fn folder_tab(
    ui: &mut egui::Ui,
    selected: bool,
    index: usize,
    label: &str,
    lead: bool,
) -> egui::Response {
    folder_tab_colored(ui, selected, index, label, lead, None, None)
}

/// The width a [`folder_tab_colored`] tab takes in the row.
pub(crate) fn folder_tab_width(ui: &egui::Ui, label: &str, speaker: bool) -> f32 {
    let theme = rustcars::LcarsTheme::default();
    let width = if speaker {
        voice_tab_width(ui.painter(), &theme, label)
    } else {
        label
            .chars()
            .map(|ch| if (ch as u32) >= 0x1F000 { 22.0 } else { 11.0 })
            .sum::<f32>()
            + 28.0
    };
    width.max(72.0)
}

/// Same tab, with a fill that replaces the zone color. NanoVNA uses this
/// while a zone sweep is running.
pub(crate) fn folder_tab_colored(
    ui: &mut egui::Ui,
    selected: bool,
    index: usize,
    label: &str,
    lead: bool,
    fill: Option<egui::Color32>,
    speaker: Option<f32>,
) -> egui::Response {
    let theme = rustcars::LcarsTheme::default();
    let fill = fill.unwrap_or_else(|| {
        if selected {
            rustcars::tint(tab_color(index), 0.35)
        } else {
            tab_color(index)
        }
    });
    let width = folder_tab_width(ui, label, speaker.is_some());
    let cap = if lead {
        rustcars::Cap::Left
    } else {
        rustcars::Cap::None
    };
    // The speaker is painted on the left of the name, with the button's
    // own right inset removed. An empty label keeps that inset from drawing
    // a second copy of the name.
    let button_label = if speaker.is_some() { "" } else { label };
    let resp = rustcars::lcars_button(
        ui,
        &theme,
        button_label,
        fill,
        egui::vec2(width, theme.bar_thin),
        cap,
        true,
    );
    if let Some(alpha) = speaker {
        paint_voice_caption(ui.painter(), resp.rect, &theme, label, fill, alpha);
    }
    resp
}

/// The REC page tab. Same width, inset and name placement as a speaker tab,
/// with the small record glyph in the speaker's slot.
pub(crate) fn folder_tab_rec(
    ui: &mut egui::Ui,
    selected: bool,
    index: usize,
    label: &str,
    lead: bool,
    fill: Option<egui::Color32>,
    state: rec_ui::IconState,
) -> egui::Response {
    let theme = rustcars::LcarsTheme::default();
    let fill = fill.unwrap_or_else(|| {
        if selected {
            rustcars::tint(tab_color(index), 0.35)
        } else {
            tab_color(index)
        }
    });
    let width = voice_tab_width(ui.painter(), &theme, label).max(72.0);
    let cap = if lead { rustcars::Cap::Left } else { rustcars::Cap::None };
    let resp = rustcars::lcars_button(
        ui,
        &theme,
        "",
        fill,
        egui::vec2(width, theme.bar_thin),
        cap,
        true,
    );
    paint_rec_caption(ui.painter(), resp.rect, &theme, label, fill, state);
    resp
}

/// Name on the right like [`paint_voice_caption`], and a small record glyph
/// centred in the 22px slot the speaker emoji uses.
fn paint_rec_caption(
    painter: &egui::Painter,
    rect: egui::Rect,
    theme: &rustcars::LcarsTheme,
    label: &str,
    fill: egui::Color32,
    state: rec_ui::IconState,
) {
    let font = egui::FontId::proportional(theme.header_size);
    let upper = label.to_uppercase();
    let ink = rustcars::ink_on(fill);
    let galley = painter.layout_no_wrap(upper.clone(), font.clone(), ink);
    let pad = 4.0;
    painter.text(
        egui::pos2(rect.right() - pad, rect.bottom() - theme.frame_gap),
        egui::Align2::RIGHT_BOTTOM,
        upper,
        font,
        ink,
    );
    let slot_right = rect.right() - pad - galley.rect.width() - 2.0;
    let center = egui::pos2(slot_right - rec_ui::TAB_GLYPH_SLOT / 2.0, rect.center().y);
    rec_ui::paint_tab_glyph(painter, center, state);
}

/// Right inset used when a tab has no speaker. Matches `lcars_button`.
fn tab_text_pad(theme: &rustcars::LcarsTheme) -> f32 {
    (theme.bar_thin / 2.0).min(theme.main_gap)
}

fn voice_tab_width(painter: &egui::Painter, theme: &rustcars::LcarsTheme, label: &str) -> f32 {
    let font = egui::FontId::proportional(theme.header_size);
    let text = painter.layout_no_wrap(label.to_uppercase(), font, egui::Color32::WHITE);
    // Emoji, a 2px gap, the name, and a 4px right inset. The usual 16px inset is left off.
    text.rect.width() + 22.0 + 2.0 + 4.0
}

fn paint_voice_caption(
    painter: &egui::Painter,
    rect: egui::Rect,
    theme: &rustcars::LcarsTheme,
    label: &str,
    fill: egui::Color32,
    alpha: f32,
) {
    let font = egui::FontId::proportional(theme.header_size);
    let upper = label.to_uppercase();
    let galley = painter.layout_no_wrap(upper.clone(), font.clone(), rustcars::ink_on(fill));
    let pad = 4.0;
    let ink = rustcars::ink_on(fill);
    painter.text(
        egui::pos2(rect.right() - pad, rect.bottom() - theme.frame_gap),
        egui::Align2::RIGHT_BOTTOM,
        upper,
        font.clone(),
        ink,
    );
    painter.text(
        egui::pos2(
            rect.right() - pad - galley.rect.width() - 2.0,
            rect.center().y,
        ),
        egui::Align2::RIGHT_CENTER,
        pulse::SPEAKER,
        font,
        ink.gamma_multiply(alpha.clamp(0.0, 1.0)),
    );
}

/// The tab being moved, drawn above the row. It uses the same fill, cap, and
/// label as [`folder_tab`]. It does not take clicks.
fn paint_floating_tab(
    ui: &egui::Ui,
    rect: egui::Rect,
    selected: bool,
    index: usize,
    label: &str,
    speaker: Option<f32>,
    rec: Option<rec_ui::IconState>,
) {
    let theme = rustcars::LcarsTheme::default();
    let fill = if selected {
        rustcars::tint(tab_color(index), 0.35)
    } else {
        tab_color(index)
    };
    let cap = if index == 0 {
        rustcars::Cap::Left
    } else {
        rustcars::Cap::None
    };
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::unique("tab-tear"),
    ));
    painter.rect_filled(rect, rustcars::cap_radius(cap, rect.height()), fill);
    if let Some(alpha) = speaker {
        paint_voice_caption(&painter, rect, &theme, label, fill, alpha);
        return;
    }
    if let Some(state) = rec {
        paint_rec_caption(&painter, rect, &theme, label, fill, state);
        return;
    }
    let pad = tab_text_pad(&theme);
    painter.text(
        egui::pos2(rect.right() - pad, rect.bottom() - theme.frame_gap),
        egui::Align2::RIGHT_BOTTOM,
        label.to_uppercase(),
        egui::FontId::proportional(theme.header_size),
        rustcars::ink_on(fill),
    );
}

const LCD_ORANGE: egui::Color32 = egui::Color32::from_rgb(232, 128, 42);
const LCD_YELLOW: egui::Color32 = egui::Color32::from_rgb(245, 214, 64);
const LCD_PAPER: egui::Color32 = egui::Color32::from_rgb(226, 222, 206);
/// Column starts on a key row: id, algorithm, status, name.
const KEY_LIST_X: [f32; 4] = [8.0, 72.0, 168.0, 256.0];
const KEY_LIST_HEAD: [&str; 4] = ["Key ID", "Algorithm", "Status", "Name"];

/// The scanner bezel and glass. The closure draws on the glass and fills the width.
///
/// `Frame` copies the parent layout. Plan and Channels call this from a horizontal
/// key row, and a horizontal glass laid the readout in one line that ran off the
/// window and stretched the lists under it. The inner column is top-down and locked
/// to the width still free, so the bezel cannot grow the page.
fn lcd_panel(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    let bezel = egui::Frame::new()
        .fill(egui::Color32::from_rgb(42, 42, 46))
        .inner_margin(7)
        .corner_radius(8);
    let screen = egui::Frame::new()
        .fill(egui::Color32::from_rgb(8, 8, 6))
        .inner_margin(10)
        .corner_radius(3);
    bezel.show(ui, |ui| {
        screen.show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                body(ui);
            });
        });
    });
}

/// Column widths that add up to `total`, including `gap` between them.
/// Leftover space goes to `flex`. When the preferred widths do not fit, each
/// column shrinks by the same ratio so the row stays inside the window.
fn fit_columns(total: f32, gap: f32, preferred: &[f32], flex: usize) -> Vec<f32> {
    let n = preferred.len();
    if n == 0 {
        return Vec::new();
    }
    let gaps = gap * (n - 1) as f32;
    let usable = (total - gaps).max(0.0);
    let sum: f32 = preferred.iter().copied().sum();
    if sum <= usable && sum > 0.0 {
        let mut out = preferred.to_vec();
        let grow = if flex < n { flex } else { 0 };
        out[grow] += usable - sum;
        return out;
    }
    if sum <= 0.0 {
        return vec![usable / n as f32; n];
    }
    let scale = usable / sum;
    preferred.iter().map(|width| width * scale).collect()
}

/// Zone on the left, mode on the right, both clipped to the glass.
fn lcd_title_row(ui: &mut egui::Ui, title: &str, mode: &str) {
    let full = ui.available_width();
    ui.horizontal(|ui| {
        ui.set_max_width(full);
        let reserve = 80.0;
        let gap = ui.spacing().item_spacing.x;
        ui.scope(|ui| {
            ui.set_max_width((full - reserve - gap).max(0.0));
            ui.add(
                egui::Label::new(
                    egui::RichText::new(title)
                        .size(16.0)
                        .monospace()
                        .color(LCD_ORANGE),
                )
                .truncate(),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(mode)
                        .size(16.0)
                        .monospace()
                        .color(LCD_ORANGE),
                )
                .truncate(),
            );
        });
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyLamp {
    Dark,
    /// REC armed for marked talkgroups / channels.
    Red,
    Green,
    Amber,
    Purple,
}

/// One key beside the scanner face. Green is Follow or a one-zone scan.
/// Amber is the all-zone scan. Purple is a scan limited to marked zones.
/// Load and Save stay dark; they act on the click.
fn radio_key(ui: &mut egui::Ui, label: &str, lamp: KeyLamp, enabled: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(72.0, 46.0), egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hover = enabled && response.hovered();
    let fill = match lamp {
        KeyLamp::Green => egui::Color32::from_rgb(18, 58, 28),
        KeyLamp::Amber => egui::Color32::from_rgb(72, 48, 10),
        KeyLamp::Purple => egui::Color32::from_rgb(42, 16, 64),
        KeyLamp::Red => egui::Color32::from_rgb(70, 12, 10),
        KeyLamp::Dark if hover => egui::Color32::from_rgb(48, 48, 52),
        KeyLamp::Dark => egui::Color32::from_rgb(28, 28, 32),
    };
    let edge = match lamp {
        KeyLamp::Green => egui::Color32::from_rgb(46, 176, 78),
        KeyLamp::Amber => egui::Color32::from_rgb(214, 148, 32),
        KeyLamp::Purple => egui::Color32::from_rgb(168, 96, 214),
        KeyLamp::Red => egui::Color32::from_rgb(214, 44, 32),
        KeyLamp::Dark if hover => egui::Color32::from_rgb(120, 120, 126),
        KeyLamp::Dark => egui::Color32::from_rgb(78, 78, 84),
    };
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(3),
        fill,
        egui::Stroke::new(1.0, edge),
        egui::StrokeKind::Inside,
    );
    let text = if !enabled {
        egui::Color32::from_rgb(110, 108, 100)
    } else {
        match lamp {
            KeyLamp::Green => egui::Color32::from_rgb(150, 230, 140),
            KeyLamp::Amber => egui::Color32::from_rgb(255, 196, 96),
            KeyLamp::Purple => egui::Color32::from_rgb(220, 170, 255),
            KeyLamp::Red => egui::Color32::from_rgb(255, 150, 140),
            KeyLamp::Dark => egui::Color32::from_rgb(232, 228, 216),
        }
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::monospace(13.0),
        text,
    );
    response
}

/// Column titles above the key list. They stay put while the rows scroll.
fn key_list_header(ui: &mut egui::Ui) {
    let (rect, _response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
    let color = ui.visuals().weak_text_color();
    let font = egui::FontId::monospace(12.0);
    let painter = ui.painter();
    for (x, label) in KEY_LIST_X.iter().zip(KEY_LIST_HEAD) {
        painter.text(
            rect.left_center() + egui::vec2(*x, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            font.clone(),
            color,
        );
    }
    let rule = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - 1.0),
        rect.right_bottom(),
    );
    painter.rect_filled(rule, egui::CornerRadius::ZERO, color);
}

fn page_tabs(
    rows: &[cat::RadioRow],
    talkgroups: &talkgroups::TalkgroupFile,
    units: &[units::Unit],
) -> Vec<(Page, String)> {
    let mut tabs: Vec<(Page, String)> = PAGES
        .iter()
        .map(|(page, name)| (*page, (*name).to_string()))
        .collect();
    let talk = radio_page_tabs(
        rows,
        talkgroup_radio_ids(rows, directory_has_talkgroups(talkgroups)),
        "TG",
        Page::Talkgroups,
    );
    let radios = radio_page_tabs(
        rows,
        talkgroup_radio_ids(rows, !units.is_empty()),
        "ID",
        Page::RadioIds,
    );
    let mut at = tabs
        .iter()
        .position(|(page, _)| *page == Page::Channels)
        .map(|idx| idx + 1)
        .unwrap_or(tabs.len());
    for item in talk.into_iter().chain(radios) {
        tabs.insert(at, item);
        at += 1;
    }
    tabs
}

/// Place tabs named in `saved` in that order. A tab the station does not
/// name keeps its place after the last already-placed tab that precedes
/// it in the built-in order.
fn apply_tab_order(
    default_tabs: Vec<(Page, String)>,
    saved: &[String],
) -> Vec<(Page, String)> {
    if saved.is_empty() {
        return default_tabs;
    }
    let mut remaining = default_tabs.clone();
    let mut out = Vec::new();
    for key in saved {
        if let Some(index) = remaining
            .iter()
            .position(|(page, _)| page_name(*page) == *key)
        {
            out.push(remaining.remove(index));
        }
    }
    for tab in remaining {
        let def_pos = default_tabs
            .iter()
            .position(|(page, _)| *page == tab.0)
            .unwrap_or(default_tabs.len());
        let mut insert_at = 0;
        for (index, placed) in out.iter().enumerate() {
            let placed_pos = default_tabs
                .iter()
                .position(|(page, _)| *page == placed.0)
                .unwrap_or(0);
            if placed_pos < def_pos {
                insert_at = index + 1;
            }
        }
        out.insert(insert_at, tab);
    }
    out
}

/// A page-tab press. `torn` is set only after the pointer travels at least
/// one tab-height off the row. Until then the press is a click.
struct TabPress {
    page: Page,
    name: String,
    label: String,
    index: usize,
    selected: bool,
    origin: egui::Pos2,
    home: egui::Rect,
    torn: bool,
}

/// Move `from` to the slot occupied by `to`. Dragging right lands after
/// that tab. Dragging left lands on it, and the target shifts right.
fn move_tab(tabs: &mut Vec<String>, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(from_at) = tabs.iter().position(|name| name == from) else {
        return;
    };
    let Some(to_at) = tabs.iter().position(|name| name == to) else {
        return;
    };
    if from_at == to_at {
        return;
    }
    let item = tabs.remove(from_at);
    // A rightward remove shifts the target down by one, so the original
    // target index is the slot just after it. A leftward remove leaves
    // that index on the target.
    tabs.insert(to_at, item);
}

fn radio_page_tabs(
    rows: &[cat::RadioRow],
    shown: Vec<u64>,
    title: &str,
    page_of: fn(u64) -> Page,
) -> Vec<(Page, String)> {
    let mut tabs = Vec::new();
    for row in rows {
        if shown.contains(&row.id) {
            tabs.push((page_of(row.id), format!("{} {title}", row.spec.name)));
        }
    }
    let mut seen = std::collections::BTreeMap::<String, usize>::new();
    for (_, label) in &tabs {
        *seen.entry(label.clone()).or_insert(0) += 1;
    }
    for (page, label) in &mut tabs {
        if seen.get(label.as_str()).copied().unwrap_or(0) > 1 {
            let id = match *page {
                Page::Talkgroups(id) | Page::RadioIds(id) => id,
                _ => continue,
            };
            label.push_str(&format!(" {id}"));
        }
    }
    tabs
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        // The face talks to the hamlib dummy this window starts. Demo does not.
        let mut snap = empty_snap();
        snap.message = "starting dummy".into();
        let shared = Arc::new(Mutex::new(Shared {
            snap,
            cmds: Vec::new(),
            live_gen: 0,
            poll_gen: 0,
        }));
        let wake = Arc::new(Condvar::new());
        let face = Arc::new(Mutex::new(cat::FaceGate::new()));
        let ctx = cc.egui_ctx.clone();
        let kick_shared = Arc::clone(&shared);
        let kick_wake = Arc::clone(&wake);
        let kick: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _guard = kick_shared.lock().expect("ui");
            kick_wake.notify_one();
        });
        let hub = cat::Hub::spawn(Arc::clone(&face), kick, ctx.clone());
        let rotators = rot::Hub::spawn(ctx.clone());
        let amplifiers = amp::Hub::spawn(ctx.clone());
        {
            // E-STOP fan-out: wake every device hub the moment any copy of
            // the button is pressed. Each hub also checks the flag on its own.
            let (h, r, a) = (Arc::clone(&hub), Arc::clone(&rotators), Arc::clone(&amplifiers));
            estop::on_engage(Box::new(move || {
                h.wake();
                r.estop();
                a.estop();
            }));
            // Start engaged if Commander was closed with the E-STOP engaged.
            estop::load_persisted(config::station_dir().join("estop-state"));
        }
        let shared_thread = Arc::clone(&shared);
        let wake_thread = Arc::clone(&wake);
        let face_thread = Arc::clone(&face);
        let hub_thread = Arc::clone(&hub);
        thread::spawn(move || drive(shared_thread, wake_thread, face_thread, hub_thread, ctx));
        let plan_path = plan::default_path();
        let plan = plan::load_file(&plan_path).unwrap_or_else(|_| plan::embedded());
        let plan_dir = cc
            .storage
            .and_then(|storage| storage.get_string("plan_dir"))
            .map(PathBuf::from)
            .filter(|dir| dir.is_dir())
            .or_else(plan::bsp_dir);
        let channel_dir = cc
            .storage
            .and_then(|storage| storage.get_string("channel_dir"))
            .map(PathBuf::from)
            .filter(|dir| dir.is_dir());
        let key_dir = cc
            .storage
            .and_then(|storage| storage.get_string("key_dir"))
            .map(PathBuf::from)
            .filter(|dir| dir.is_dir());
        let config_dir = cc
            .storage
            .and_then(|storage| storage.get_string("config_dir"))
            .map(PathBuf::from)
            .filter(|dir| dir.is_dir());
        let band_sel = plan
            .zones
            .first()
            .and_then(|zone| zone.bands.iter().position(|band| band.chip == "20m"))
            .unwrap_or(0);
        let mut app = Self {
            shared,
            wake,
            face,
            hub,
            host: "127.0.0.1".into(),
            port: "4532".into(),
            pending_tune: None,
            models: Vec::new(),
            cat_edit: None,
            rotators,
            rot_models: Vec::new(),
            rot_edit: None,
            amplifiers,
            amp_models: Vec::new(),
            amp_edit: None,
            amp_follow: Default::default(),
            amp_freq_text: String::new(),
            amp_level_vals: Default::default(),
            pipe_editor: Default::default(),
            rx_pipe_pending: None,
            tx_pipe_pending: None,
            favorite: None,
            show_spread: true,
            band_mem: HashMap::new(),
            step_hz: 100.0,
            freq_text: format_hz(rigctl::DEMO_FREQ_HZ),
            sub_freq_text: format_hz(7_074_000.0),
            local_mem: None,
            hold_freq: None,
            hold_levels: HashMap::new(),
            hold_pass: None,
            fine_lock: None,
            page: Page::Tune,
            page_return: Page::Tune,
            tab_order: Vec::new(),
            tab_press: None,
            plan,
            zone_sel: 0,
            band_sel,
            plan_path: plan_path.display().to_string(),
            plan_dir,
            plan_note: String::new(),
            plan_note_ptt: false,
            zone_rename: None,
            mode_hold: None,
            sub_freq_hold: None,
            sub_mode_hold: None,
            main_cluster_height: 0.0,
            vfo_frame_height: 0.0,
            sub_vfo_height: 0.0,
            channels: channels::empty(),
            channel_zone: 0,
            zone_list_follow: true,
            channel_sel: 0,
            channel_list_follow: false,
            memory_scan: scan::Scanner::new(),
            voice_pulse: pulse::Pulse::new(),
            pulse_on: true,
            pulse_secs: pulse::PERIOD_DEFAULT,
            voice_hang_ms: follow::VOICE_HANG_DEFAULT.as_millis() as u32,
            override_hold: true,
            trunk_emergency: Vec::new(),
            patchbay: Default::default(),
            rec: Default::default(),
            patchbay_ui: Default::default(),
            reduced_motion: pulse::system_reduced_motion(),
            scan_zones: BTreeSet::new(),
            monitor_zones: BTreeSet::new(),
            monitor_talkgroups: BTreeSet::new(),
            scan_amber_origin: false,
            scan_sending: false,
            channel_path: String::new(),
            channel_dir,
            channel_note: String::new(),
            talkgroups: talkgroups::empty(),
            talkgroup_zone: 0,
            talkgroup_sel: 0,
            talkgroup_path: String::new(),
            talkgroup_dir: None,
            talkgroup_note: String::new(),
            keys: keys::empty(),
            key_sel: 0,
            key_note: String::new(),
            key_id_edit: String::new(),
            key_alg_edit: keys::ALG_DES.to_string(),
            key_name_edit: String::new(),
            key_text_edit: String::new(),
            key_dir,
            key_shown: None,
            key_runtime: String::new(),
            key_runtime_gen: 0,
            units: Vec::new(),
            unit_sel: 0,
            unit_desc_dirty: false,
            cc_on: HashMap::new(),
            pending_control: None,
            cc_sent: HashMap::new(),
            vocoder_sent: HashMap::new(),
            cc_voice_sent: HashMap::new(),
            cc_mode_sent: HashMap::new(),
            followers: HashMap::new(),
            hackrf_open: HashMap::new(),
            hackrf_call: HashMap::new(),
            hackrf_hold: HashMap::new(),
            hackrf_away: HashMap::new(),
            trunk_pool: Vec::new(),
            trunk_receivers: Vec::new(),
            hackrf_seq: 0,
            hackrf_cc_feeds: HashMap::new(),
            hackrf_voice_feed: HashMap::new(),
            hackrf_watch_sent: HashMap::new(),
            hackrf_voice_sent: HashMap::new(),
            hackrf_p2_sent: HashMap::new(),
            hackrf_dial_seeded: BTreeSet::new(),
            hackrf_seen: Vec::new(),
            hackrf_probe_note: String::new(),
            spectrum_peak: false,
            gain_drag_lna: None,
            gain_drag_vga: None,
            hackrf_gain_hold: None,
            spectrum_seq: 0,
            spectrum_center: 0.0,
            spectrum_avg: Vec::new(),
            spectrum_peak_hold: Vec::new(),
            hackrf_scan_poll: 0,
            hackrf_scan_seq: 0,
            hackrf_scan_center: 0.0,
            feeds: HashMap::new(),
            cc_tail: HashMap::new(),
            held_call: HashMap::new(),
            show_housekeeping: false,
            tg_recent: pulse::Recent::default(),
            direct_seen: HashMap::new(),
            voice_frames: HashMap::new(),
            cc_site: HashMap::new(),
            cc_identity: HashMap::new(),
            identity_gate: HashMap::new(),
            invert_sent: HashMap::new(),
            system_clock: clock::SystemClock::default(),
            config_path: String::new(),
            config_dir,
            config_note: String::new(),
            station_saved: None,
            exit_prompt: false,
            exit_discard: false,
            audio_saved: crate::audio::canonical(&crate::audio::AudioGraph::default()),
            pipewire_reset: Arc::new(AtomicBool::new(false)),
            audio_hold: false,
            speaker_tex: None,
            mic_tex: None,
            radio_tex: None,
            scope_tex: None,
            xdg_tex: HashMap::new(),
            bay_drag: None,
            gpredict_sig: String::new(),
            gpredict_note: String::new(),
            gpredict_files: Vec::new(),
            gpredict_wires: Vec::new(),
            meters: Arc::new(Mutex::new(MeterState {
                locked: cc
                    .storage
                    .and_then(|storage| storage.get_string("meter_lock"))
                    .map(|value| value != "0")
                    .unwrap_or(true),
                order: cc
                    .storage
                    .and_then(|storage| storage.get_string("meter_order"))
                    .map(|value| {
                        value
                            .split(',')
                            .filter(|id| !id.is_empty())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                drag: None,
                needles: HashMap::new(),
                detached: cc
                    .storage
                    .and_then(|storage| storage.get_string("meter_detach"))
                    .is_some_and(|value| value == "1"),
                dock: false,
                saved_pos: cc
                    .storage
                    .and_then(|storage| storage.get_string("meter_win_pos"))
                    .as_deref()
                    .and_then(parse_point),
                saved_size: cc
                    .storage
                    .and_then(|storage| storage.get_string("meter_win_size"))
                    .as_deref()
                    .and_then(parse_size),
                saved_monitor: None,
                saved_ppp: None,
            })),
            meter_feed: Arc::new(Mutex::new(MeterFeed {
                snap: empty_snap(),
                radio: None,
                clock: clock::SystemClock::default(),
            })),
            icon: Arc::new(hamspot_icon()),
            meter_place: None,
            meter_size: None,
            meter_placed: false,
            tools: Arc::new(Mutex::new(tools::Tools::new(&cc.egui_ctx))),
            tool_place: None,
            tool_size: None,
            tool_placed: false,
        };
        {
            let mut tools = app.tools.lock().expect("tools");
            tools.set_rotators(Arc::clone(&app.rotators));
            if let Some(text) = cc.storage.and_then(|storage| storage.get_string("rotator_settings")) {
                tools.load_rotator_settings(&text);
            }
            tools.detached = cc
                .storage
                .and_then(|storage| storage.get_string("tool_detach"))
                .is_some_and(|value| value == "1");
            tools.saved_pos = cc
                .storage
                .and_then(|storage| storage.get_string("tool_win_pos"))
                .as_deref()
                .and_then(parse_point);
            tools.saved_size = cc
                .storage
                .and_then(|storage| storage.get_string("tool_win_size"))
                .as_deref()
                .and_then(parse_size);
        }
        app.open_startup_station();
        app
    }

    /// A clicked station file wins. With no argument, open Default.HSCommander
    /// when it already exists. A missing default stays the save target and is
    /// written the next time the station changes.
    fn open_startup_station(&mut self) {
        let path = config::station_from_args(std::env::args().skip(1));
        let clicked = path != config::default_station_path();
        if clicked && !path.is_file() {
            self.config_note = format!("{}: no such station file", path.display());
            return;
        }
        self.config_path = path.display().to_string();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                self.config_dir = Some(parent.to_path_buf());
            }
        }
        if path.is_file() {
            self.load_config(&empty_snap());
        }
    }

    fn snap_gen(&self) -> (Snapshot, u64, u64) {
        let guard = self.shared.lock().expect("rig thread");
        (guard.snap.clone(), guard.live_gen, guard.poll_gen)
    }

    /// The dial reads the HackRF center. The rigctl snapshot has no port
    /// for this radio, and an empty receive range would refuse the wheel.
    fn overlay_hackrf(&self, snap: &mut Snapshot) {
        let view = self.hub.view();
        let Some(row) = view
            .rows
            .iter()
            .find(|row| row.selected && row.spec.driver == cat::RadioDriver::Hackrf)
        else {
            return;
        };
        // Caps stay up before the first capture. An empty receive range is
        // what paints "No channel this radio can tune." and leaves the face
        // on the rigctl session this driver already closed.
        let center = row.center_hz;
        snap.kind = LinkKind::Live;
        snap.model = "HackRF".into();
        snap.endpoint = "direct".into();
        if center > 0.0 {
            snap.freq_hz = center;
        }
        snap.mode = "FM".into();
        snap.passband_hz = 12_500;
        snap.message.clear();
        snap.caps.model = "HackRF".into();
        snap.caps.can_set_freq = true;
        snap.caps.can_set_mode = false;
        snap.caps.modes = vec!["FM".into()];
        snap.caps.rx_ranges = vec![rigctl::Range {
            min: 1_000_000.0,
            max: 6_000_000_000.0,
            step: 1.0,
        }];
    }

    fn send(&self, cmd: Command) {
        let mut guard = self.shared.lock().expect("rig thread");
        guard.cmds.push(cmd);
        self.wake.notify_one();
    }

    fn shown_freq(&mut self, snap: &Snapshot) -> f64 {
        if let Some(held) = self.hold_freq {
            if (held - snap.freq_hz).abs() < 0.5 {
                self.hold_freq = None;
                snap.freq_hz
            } else {
                held
            }
        } else {
            snap.freq_hz
        }
    }

    /// Point the Tune chips and the bandspread at the band that contains `hz`.
    /// The current band wins when it still contains `hz`, so a shared
    /// frequency stays on the line the operator clicked. A gap leaves the
    /// selection alone.
    fn follow_band(&mut self, hz: f64, rx: &[(f64, f64)]) {
        if let Some(band) = self
            .plan
            .zones
            .get(self.zone_sel)
            .and_then(|zone| zone.bands.get(self.band_sel))
        {
            let inside = plan::pieces_inside(band.lo_hz, band.hi_hz, rx)
                .iter()
                .any(|(lo, hi)| hz >= *lo && hz <= *hi);
            if inside {
                return;
            }
        }
        let bands = plan::tunable_bands(&self.plan, rx);
        let Some(idx) = plan::band_at(&bands, hz) else {
            return;
        };
        self.zone_sel = bands[idx].zone;
        self.band_sel = bands[idx].band;
    }

    fn following(&self) -> bool {
        if self.face.lock().expect("face").demo {
            return false;
        }
        self.hub
            .view()
            .rows
            .iter()
            .any(|row| row.selected && row.spec.follow)
    }

    /// The selected radio's control follower is the one writing the dial.
    fn following_control(&self) -> bool {
        if self.face.lock().expect("face").demo {
            return false;
        }
        let rows = self.hub.view().rows;
        let Some(row) = rows.iter().find(|row| row.selected) else {
            return false;
        };
        if row.spec.follow {
            return false;
        }
        if row.spec.driver == cat::RadioDriver::Hackrf {
            return self.cc_on.get(&row.id).copied().unwrap_or(false);
        }
        if !hears_trunk_pipe(row) {
            return false;
        }
        self.cc_on.get(&row.id).copied().unwrap_or(false)
    }

    fn dial_held(&self) -> bool {
        if self.following() {
            return true;
        }
        // Trunk Follow parks and grants. SCAN is the writer while it is
        // hopping control channels. Follow resumes when the scan stops.
        self.following_control() && !self.memory_scan.is_on()
    }

    fn can_tune_freq(&self, snap: &Snapshot, main: bool) -> bool {
        if !frequency_writes_allowed(self.dial_held()) {
            return false;
        }
        snap.online()
            && if main {
                snap.caps.can_set_freq
            } else {
                snap.caps.can_set_split
            }
    }

    fn shown_sub_freq(&self, snap: &Snapshot) -> f64 {
        self.sub_freq_hold.unwrap_or(snap.sub_freq_hz)
    }

    fn send_plan_mode(&mut self, snap: &Snapshot, wanted: &str) {
        if !snap.online() || !snap.caps.can_set_mode {
            return;
        }
        let Some(mode) = plan::choose_mode(wanted, &snap.caps.modes) else {
            return;
        };
        if snap.mode.eq_ignore_ascii_case(&mode) {
            return;
        }
        if self
            .mode_hold
            .as_deref()
            .is_some_and(|held| held.eq_ignore_ascii_case(&mode))
        {
            return;
        }
        self.mode_hold = Some(mode.clone());
        self.send(Command::SetMode {
            mode,
            passband_hz: snap.passband_hz,
        });
    }

    /// Right-click path. Same frequency and plan mode as the main VFO, sent
    /// to the other VFO. The main frequency and mode are left alone.
    fn tune_sub(&mut self, snap: &Snapshot, hz: f64, wanted: Option<&str>) {
        if !frequency_writes_allowed(self.dial_held()) {
            return;
        }
        let Some(hz) = snap.caps.allow_freq(hz) else {
            return;
        };
        if !snap.online() || !snap.caps.can_set_split {
            return;
        }
        self.sub_freq_text = format_hz(hz);
        let held = self
            .sub_freq_hold
            .is_some_and(|held| (held - hz).abs() < 0.5);
        let reported = snap.sub_freq_hz > 0.0 && (snap.sub_freq_hz - hz).abs() < 0.5;
        if !held && !reported {
            self.sub_freq_hold = Some(hz);
            self.send(Command::SetSplitFreq(hz));
        }
        let Some(wanted) = wanted else {
            return;
        };
        let Some(mode) = plan::choose_mode(wanted, &snap.caps.modes) else {
            return;
        };
        if snap.sub_mode.eq_ignore_ascii_case(&mode) {
            return;
        }
        if self
            .sub_mode_hold
            .as_deref()
            .is_some_and(|held| held.eq_ignore_ascii_case(&mode))
        {
            return;
        }
        self.sub_mode_hold = Some(mode.clone());
        self.send(Command::SetSubMode {
            mode,
            passband_hz: snap.passband_hz,
        });
    }

    fn tune(&mut self, snap: &Snapshot, hz: f64) {
        let Some(hz) = snap.caps.allow_freq(hz) else {
            let current = self.shown_freq(snap);
            self.freq_text = format_hz(current);
            return;
        };
        self.hold_and_send(hz);
    }

    /// Move one VFO by `delta` hertz. One digit click or one wheel notch
    /// passes a single place value. The step stops at the receive edge.
    fn step_digit(&mut self, snap: &Snapshot, current: f64, delta: f64, main: bool) {
        let target = current + delta;
        if main {
            self.nudge(snap, current, target);
        } else {
            self.nudge_sub(snap, current, target);
        }
    }

    /// Wheel and digit steps. Stops on the edge of the current receive
    /// segment instead of jumping a gap or going negative.
    fn nudge(&mut self, snap: &Snapshot, current: f64, target: f64) {
        let Some(hz) = snap.caps.step_freq(current, target) else {
            return;
        };
        if (hz - current).abs() < 0.5 {
            return;
        }
        self.hold_and_send(hz);
    }

    fn nudge_sub(&mut self, snap: &Snapshot, current: f64, target: f64) {
        let Some(hz) = snap.caps.step_freq(current, target) else {
            return;
        };
        if (hz - current).abs() < 0.5 {
            return;
        }
        self.tune_sub(snap, hz, None);
    }

    fn hold_and_send(&mut self, hz: f64) {
        if !self.scan_sending {
            self.stop_scan();
        }
        if let Some(row) = self
            .hub
            .view()
            .rows
            .iter()
            .find(|row| row.selected && row.spec.driver == cat::RadioDriver::Hackrf)
            .map(|row| row.id)
        {
            if self.cc_on.get(&row).copied().unwrap_or(false) {
                return;
            }
            self.hold_freq = Some(hz);
            self.freq_text = format_hz(hz);
            self.hub.hackrf_dial(row, hz);
            return;
        }
        if !frequency_writes_allowed(self.dial_held()) {
            return;
        }
        self.hold_freq = Some(hz);
        self.freq_text = format_hz(hz);
        self.send(Command::SetFreq(hz));
    }

    /// ±1.2 kHz around `freq`, cut off at the receive segment so the fine
    /// scale cannot offer a frequency the radio will not tune.
    fn fine_window(&self, snap: &Snapshot, freq: f64) -> (f64, f64) {
        if let Some(locked) = self.fine_lock {
            return locked;
        }
        let mut lo = freq - 1_200.0;
        let mut hi = freq + 1_200.0;
        if let Some(seg) = snap.caps.rx_segment(freq) {
            lo = lo.max(seg.min);
            hi = hi.min(seg.max);
        }
        if lo > hi {
            (freq, freq)
        } else {
            (lo, hi)
        }
    }

    fn shown_level(&mut self, name: &str, snap: &Snapshot) -> f64 {
        let live = snap.levels.get(name).copied().unwrap_or(0.0);
        if let Some(held) = self.hold_levels.get(name).copied() {
            if (held - live).abs() < 1e-3 {
                self.hold_levels.remove(name);
                live
            } else {
                held
            }
        } else {
            live
        }
    }
}

fn pipe_block_label(command: &str, fallback: &str) -> String {
    crate::rx_pipe::modem_label(
        std::path::Path::new(crate::rx_pipe::program_name(command).unwrap_or(fallback))
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(fallback),
    )
    .to_string()
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // The close prompt writes the station when the operator says to.
        // A crash never reaches either path. Edits that must survive a crash
        // are still written when they happen.
        // Calibration and scans are never left only in memory. Issue 65.
        let vna_dirty = self.tools.lock().map(|mut t| t.take_vna_dirty()).unwrap_or(false);
        if vna_dirty && self.station_saved.is_some() {
            self.save_vna();
        }
        self.hub.shutdown();
        self.rotators.shutdown();
        self.amplifiers.shutdown();
        // A crash never reaches here. The cleanup child covers that, and this
        // call is only for a window close where that child failed to start.
        if !exit_watch::child_is_watching() {
            exit_watch::cleanup_self();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(dir) = &self.plan_dir {
            storage.set_string("plan_dir", dir.display().to_string());
        }
        if let Some(dir) = &self.channel_dir {
            storage.set_string("channel_dir", dir.display().to_string());
        }
        if let Some(dir) = &self.key_dir {
            storage.set_string("key_dir", dir.display().to_string());
        }
        if let Some(dir) = &self.config_dir {
            storage.set_string("config_dir", dir.display().to_string());
        }
        let meters = self.meters.lock().expect("meters");
        storage.set_string("meter_order", meters.order.join(","));
        storage.set_string(
            "meter_lock",
            if meters.locked { "1" } else { "0" }.to_string(),
        );
        storage.set_string(
            "meter_detach",
            if meters.detached { "1" } else { "0" }.to_string(),
        );
        if let Some(pos) = meters.saved_pos {
            storage.set_string("meter_win_pos", format!("{},{}", pos.x, pos.y));
        }
        if let Some(size) = meters.saved_size {
            storage.set_string("meter_win_size", format!("{},{}", size.x, size.y));
        }
        let tools = self.tools.lock().expect("tools");
        storage.set_string(
            "tool_detach",
            if tools.detached { "1" } else { "0" }.to_string(),
        );
        if let Some(pos) = tools.saved_pos {
            storage.set_string("tool_win_pos", format!("{},{}", pos.x, pos.y));
        }
        if let Some(size) = tools.saved_size {
            storage.set_string("tool_win_size", format!("{},{}", size.x, size.y));
        }
        storage.set_string("rotator_settings", tools.rotator_settings());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        {
            let mut meters = self.meters.lock().expect("meters");
            if meters.dock {
                meters.detached = false;
                meters.dock = false;
                meters.drag = None;
            }
        }
        let (mut snap, live_gen, poll_gen) = self.snap_gen();
        self.overlay_hackrf(&mut snap);
        self.clear_stale_channel_note(&snap);
        self.maybe_push_loaded(&snap, live_gen);
        self.seed_hackrf_dial(&snap);
        self.autosave_audio(&snap);
        self.flush_rx_pipe(&snap);
        self.flush_tx_pipe(&snap);
        self.service_cc(&snap);
        self.release_memory_scan(&snap);
        self.drain_identity();
        self.apply_channel_vocoder();
        self.sync_gpredict();
        self.follow_amplifiers(&snap, ui.ctx());
        let follow = self.following();
        self.face.lock().expect("face").follow = follow;
        if follow {
            self.hold_freq = None;
            self.sub_freq_hold = None;
        }
        if let Some(row) = self.hub.view().rows.iter().find(|row| row.selected) {
            self.host = "127.0.0.1".into();
            if let Some(port) = row.port {
                self.port = port.to_string();
            }
        }
        // Tune follows the frequency the readout is showing, including a
        // set_freq that the radio has not confirmed yet. The Plan editor
        // keeps its own zone tab.
        if self.page == Page::Tune {
            let hz = self.shown_freq(&snap);
            self.follow_band(hz, &rx_pairs(&snap));
        }
        if snap.online() {
            if let Some(band) = self
                .plan
                .zones
                .get(self.zone_sel)
                .and_then(|zone| zone.bands.get(self.band_sel))
            {
                if snap.freq_hz >= band.lo_hz && snap.freq_hz <= band.hi_hz {
                    self.band_mem.insert(band.name.clone(), snap.freq_hz);
                }
            }
        }
        if self
            .mode_hold
            .as_deref()
            .is_some_and(|held| snap.mode.eq_ignore_ascii_case(held))
        {
            self.mode_hold = None;
        }
        if self
            .sub_freq_hold
            .is_some_and(|held| (snap.sub_freq_hz - held).abs() < 0.5)
        {
            self.sub_freq_hold = None;
        }
        if self
            .sub_mode_hold
            .as_deref()
            .is_some_and(|held| snap.sub_mode.eq_ignore_ascii_case(held))
        {
            self.sub_mode_hold = None;
        }
        if self.plan_note_ptt
            && self
                .plan
                .transmit_block_on(self.zone_sel, self.band_sel, snap.freq_hz, &snap.mode)
                .is_none()
        {
            self.plan_note.clear();
            self.plan_note_ptt = false;
        }
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.pointer.any_released()) {
            self.fine_lock = None;
        }

        // One row, at the top of the window. A later tab scrolls sideways instead of wrapping onto
        // a second line and pushing the page body down. On Tools, the right end of this same
        // row is a square tab that tears the tools off. Closing that window docks them.
        let tools_open = self.tools.lock().expect("tools").detached;
        if tools_open && self.page == Page::Tools {
            self.page = self.page_return;
        }
        let tab_side = rustcars::LcarsTheme::default().bar_thin;
        let mut rec_place: Option<rec_ui::BarPlace> = None;
        egui::Panel::top("pages")
            .show_separator_line(false)
            .frame(tools::tab_row(rustcars::LcarsTheme::default().bg))
            .show(ui, |ui| {
            let gap = 4.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let show_tear = self.page == Page::Tools && !tools_open;
                // On REC the right end of the bar carries the view controls.
                let show_rec_bar = rec_ui::bar_shown(self.page == Page::Rec);
                let row_w = ui.available_width();
                let place = rec_ui::bar_place(row_w - rec_ui::FRAME_MARGIN_X, row_w);
                rec_place = show_rec_bar.then_some(place);
                let (show_rec_bar, rec_compact) = match place {
                    rec_ui::BarPlace::TabRow { compact } => (show_rec_bar, compact),
                    rec_ui::BarPlace::InFrame { .. } => (false, false),
                };
                let tabs_w = if show_tear {
                    (ui.available_width() - tab_side - gap).max(0.0)
                } else if show_rec_bar {
                    (ui.available_width() - rec_ui::bar_width(rec_compact) - gap).max(0.0)
                } else {
                    ui.available_width()
                };
                egui::ScrollArea::horizontal()
                    .id_salt("page-tabs")
                    .max_width(tabs_w)
                    .max_height(tab_side)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            let rows = self.hub.view().rows;
                            let default_tabs = page_tabs(&rows, &self.talkgroups, &self.units);
                            let tabs = apply_tab_order(default_tabs.clone(), &self.tab_order);
                            let still_here = match self.page {
                                Page::Talkgroups(id) => {
                                    tabs.iter().any(|(page, _)| *page == Page::Talkgroups(id))
                                }
                                Page::RadioIds(id) => {
                                    tabs.iter().any(|(page, _)| *page == Page::RadioIds(id))
                                }
                                _ => true,
                            };
                            if !still_here {
                                self.page = Page::Channels;
                            }
                            let shown_keys: Vec<String> =
                                tabs.iter().map(|(page, _)| page_name(*page)).collect();
                            let default_keys: Vec<String> = default_tabs
                                .iter()
                                .map(|(page, _)| page_name(*page))
                                .collect();
                            let mut hits: Vec<(String, egui::Rect)> = Vec::new();
                            // Keyboard activation has no pointer press. A pointer
                            // press selects on release unless it has been torn off.
                            let mut clicked_page: Option<Page> = None;
                            let mut started = false;
                            let torn = self.tab_press.as_ref().is_some_and(|press| press.torn);
                            let now = std::time::Instant::now();
                            self.note_voice(now);
                            for (index, (page, label)) in tabs.into_iter().enumerate() {
                                let name = page_name(page);
                                let voice = self.tab_voice(page, now);
                                let speaker = self.speaker_alpha(page, now);
                                let lift = voice
                                    .as_ref()
                                    .map(|tab| {
                                        self.voice_pulse.lift(
                                            tab,
                                            now,
                                            self.pulse_secs,
                                            self.pulse_on,
                                            self.reduced_motion,
                                        )
                                    })
                                    .unwrap_or(0.0);
                                let fill = (lift > 0.0).then(|| {
                                    let base = if self.page == page {
                                        rustcars::tint(tab_color(index), 0.35)
                                    } else {
                                        tab_color(index)
                                    };
                                    pulse::brighten(base, lift)
                                });
                                let rec_tab = page == Page::Rec;
                                let resp = if rec_tab {
                                    let state = self.rec_icon();
                                    let resp = folder_tab_rec(
                                        ui,
                                        self.page == page,
                                        index,
                                        &label,
                                        index == 0,
                                        fill,
                                        state,
                                    );
                                    self.rec_icon_hover(ui, resp, state)
                                } else {
                                    folder_tab_colored(
                                        ui,
                                        self.page == page,
                                        index,
                                        &label,
                                        index == 0,
                                        fill,
                                        speaker,
                                    )
                                };
                                hits.push((name, resp.rect));
                                if resp.clicked() {
                                    clicked_page = Some(page);
                                }
                                if resp.is_pointer_button_down_on() && self.tab_press.is_none() {
                                    if let Some(origin) = resp.interact_pointer_pos() {
                                        started = true;
                                        self.tab_press = Some(TabPress {
                                            page,
                                            name: page_name(page),
                                            label: label.clone(),
                                            index,
                                            selected: self.page == page,
                                            origin,
                                            home: resp.rect,
                                            torn: false,
                                        });
                                    }
                                }
                            }
                            let pointer = ui.input(|input| input.pointer.interact_pos());
                            let down = ui.input(|input| input.pointer.primary_down());
                            let released = ui.input(|input| input.pointer.any_released());
                            if down {
                                if let (Some(pos), Some(press)) = (pointer, self.tab_press.as_mut())
                                {
                                    let height = press.home.height().max(1.0);
                                    // Off the row by a full tab. A sideways nudge stays a click.
                                    if (pos.y - press.origin.y).abs() >= height {
                                        press.torn = true;
                                    }
                                }
                            }
                            if let Some(press) = self.tab_press.as_ref().filter(|press| press.torn)
                            {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                if let Some(pos) = pointer {
                                    paint_floating_tab(
                                        ui,
                                        press.home.translate(pos - press.origin),
                                        press.selected,
                                        press.index,
                                        &press.label,
                                        self.speaker_alpha(press.page, now),
                                        (press.page == Page::Rec).then(|| self.rec_icon()),
                                    );
                                }
                            }
                            // A press that began this frame stays until the button
                            // comes up. Clearing it here would drop the gesture
                            // before the next frame can measure the pull.
                            let mut open_page: Option<Page> = None;
                            if released || (!down && !started) {
                                if let Some(press) = self.tab_press.take() {
                                    if press.torn {
                                        if let Some(pos) = pointer {
                                            if let Some((target, _)) = hits.iter().find(|(_, rect)| {
                                                pos.x >= rect.left() && pos.x <= rect.right()
                                            }) {
                                                if target != &press.name {
                                                    let mut keys = shown_keys.clone();
                                                    move_tab(&mut keys, &press.name, target);
                                                    self.tab_order = if keys == default_keys {
                                                        Vec::new()
                                                    } else {
                                                        keys
                                                    };
                                                    if self.station_saved.is_some()
                                                        && !self.config_path.is_empty()
                                                    {
                                                        self.save_config(&snap);
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        open_page = Some(press.page);
                                    }
                                } else if !torn {
                                    open_page = clicked_page;
                                }
                            }
                            if let Some(page) = open_page {
                                if tools_open && page == Page::Tools {
                                    let id = tools::viewport_id();
                                    ui.ctx().send_viewport_cmd_to(
                                        id,
                                        egui::ViewportCommand::Minimized(false),
                                    );
                                    ui.ctx().send_viewport_cmd_to(
                                        id,
                                        egui::ViewportCommand::Focus,
                                    );
                                    ui.ctx().send_viewport_cmd_to(
                                        id,
                                        egui::ViewportCommand::RequestUserAttention(
                                            egui::UserAttentionType::Informational,
                                        ),
                                    );
                                } else {
                                    if page != Page::Tools {
                                        self.page_return = page;
                                    }
                                    self.page = page;
                                }
                            }
                            if self.unit_desc_dirty && !matches!(self.page, Page::RadioIds(_)) {
                                self.save_unit_description(&snap);
                            }
                        });
                    });
                if show_rec_bar {
                    self.rec_bar(ui, rec_compact);
                }
                if show_tear && tools::tear_off_button(ui).clicked() {
                    self.tools.lock().expect("tools").detached = true;
                    self.page = self.page_return;
                }
            });
        });
        // The brightness sine is wall-clock. Nothing else signals the next
        // frame. An idle station has no tab inside the hang, so it does not
        // ask to be drawn again.
        if self.voice_pulse.busy() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        // Every page frame runs to the window edge. A status strip would
        // leave text under the elbows.
        self.refresh_tools(ui.ctx(), &snap);
        if self.page == Page::Tools {
            self.page_tools(ui);
        } else if self.page == Page::Timers {
            self.page_timers(ui);
        } else {
            // The stock central panel keeps an 8px margin on every side.
            // The bottom one would lift the elbows off the window.
            egui::CentralPanel::default()
                .frame(egui::Frame::central_panel(ui.style()).inner_margin(egui::Margin {
                    left: 8,
                    right: 8,
                    top: 8,
                    bottom: 0,
                }))
                .show(ui, |ui| {
                // Tune owns the wheel. Plan and Channels own a bandspread click.
                // A scroll area would take those pointers and move the page.
                let in_frame = match rec_place {
                    Some(rec_ui::BarPlace::InFrame { compact }) if self.page == Page::Rec => Some(compact),
                    _ => None,
                };
                // Every main page carries the E-STOP at the right end of its
                // tan top bar (issue #198; Tune is the reference placement).
                // REC's in-frame view controls share the same gap, to its left.
                let estop_w = estop::button_width(estop::engaged()) + 16.0;
                let slot_w = Some(in_frame.map(rec_ui::bar_slot_width).unwrap_or(0.0) + estop_w);
                let (frame_rect, slot) = tools::frame_with_slot(ui, slot_w, |ui| {
                    if self.page == Page::Tune {
                        self.page_tune(ui, &mut snap);
                    } else if self.page == Page::Plan {
                        self.page_plan(ui, &mut snap);
                    } else if self.page == Page::Channels {
                        self.page_channels(ui, &mut snap);
                    } else if let Page::Talkgroups(id) = self.page {
                        self.page_talkgroups(ui, id, &snap);
                    } else if let Page::RadioIds(id) = self.page {
                        self.page_radio_ids(ui, id, &snap);
                    } else if self.page == Page::Rec {
                        self.page_rec(ui, &snap);
                    } else {
                        egui::ScrollArea::vertical()
                            .id_salt("body")
                            .auto_shrink([false, false])
                            .show(ui, |ui| self.page_body(ui, &mut snap));
                    }
                });
                // REC's view controls and the E-STOP sit in the gap opened in the top bar.
                if let Some(r) = slot {
                    let (rec_r, stop_r) = (
                        egui::Rect::from_min_max(r.min, egui::pos2(r.right() - estop_w, r.bottom())),
                        egui::Rect::from_min_max(egui::pos2(r.right() - estop_w, r.top()), r.max),
                    );
                    if let Some(compact) = in_frame {
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rec_r).id_salt("rec-bar-in-frame"), |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = rec_ui::BAR_GAP;
                                self.rec_bar(ui, compact);
                            });
                        });
                    }
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(stop_r)
                            .id_salt("estop-page-bar")
                            .layout(egui::Layout::right_to_left(egui::Align::Center)),
                        |ui| {
                            estop::button(ui);
                        },
                    );
                }
                // REC's legal note rides the bottom elbow's bar, REC page only.
                if self.page == Page::Rec {
                    tools::bottom_bar_watermark(ui, frame_rect, rec_ui::PRIVACY);
                }
            });
        }

        if let Some((id, tx, reference)) = self.pipe_editor.show(&ctx) {
            self.apply_pipe(id, tx, reference);
        }
        let cat_driver = self.cat_edit.as_ref().map(|edit| edit.driver.clone());
        match cat_driver {
            Some(cat::RadioDriver::Hamlib) => self.hamlib_window(&ctx),
            Some(cat::RadioDriver::Gqrx) => self.gqrx_window(&ctx),
            Some(cat::RadioDriver::Hackrf) => self.hackrf_window(&ctx),
            Some(cat::RadioDriver::Tap) => self.tap_window(&ctx),
            None => {}
        }
        if self.rot_edit.is_some() {
            self.rotator_window(&ctx);
        }
        if self.amp_edit.is_some() {
            self.amplifier_window(&ctx);
        }
        self.show_meter_window(&ctx, &snap);
        self.show_tools_window(&ctx);
        // Clicks in this frame already ran. A channel or zone pick stops
        // the scan before this hop, and service_cc has already stored any
        // neighbor announcement from the control channel we are held on.
        self.advance_memory_scan(&ctx, &snap, poll_gen);
        self.consider_close(&ctx, &snap);
    }
}

impl App {
    fn page_body(&mut self, ui: &mut egui::Ui, snap: &mut Snapshot) {
        match self.page {
            Page::Tune => self.page_tune(ui, snap),
            Page::Plan => self.page_plan(ui, snap),
            Page::Channels => self.page_channels(ui, snap),
            Page::Keys => self.page_keys(ui),
            Page::Tools => {}
            Page::Timers => {}
            Page::Talkgroups(id) => self.page_talkgroups(ui, id, snap),
            Page::RadioIds(id) => self.page_radio_ids(ui, id, snap),
            Page::Meters => self.page_meters(ui, snap),
            Page::Levels => self.page_levels(ui, snap),
            Page::Rec => self.page_rec(ui, snap),
            Page::Config => self.page_config(ui, snap),
        }
    }

    fn page_tune(&mut self, ui: &mut egui::Ui, snap: &mut Snapshot) {
        let freq = self.shown_freq(snap);
        let sub_freq = self.shown_sub_freq(snap);
        lcd_panel(ui, |ui| {
            if self.following() {
                ui.label(
                    egui::RichText::new("Following Gpredict")
                        .monospace()
                        .color(LCD_ORANGE),
                );
            } else if self.following_control() {
                ui.label(
                    egui::RichText::new("Following control")
                        .monospace()
                        .color(LCD_ORANGE),
                );
            }
            ui.horizontal(|ui| {
                let known_main = self.main_cluster_height;
                let known_frame = self.vfo_frame_height;
                let known_sub = self.sub_vfo_height;
                let show_sub = !snap.sub_vfo.is_empty();
                let row_h = known_main
                    .max(known_frame)
                    .max(if show_sub { known_sub } else { 0.0 });
                let main_h = place_centered(ui, row_h, known_main, |ui| {
                    self.vfo_cluster(ui, snap, true, freq);
                });
                ui.add_space(16.0);
                let frame_h = place_centered(ui, row_h, known_frame, |ui| {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        self.vfo_column(ui, snap);
                    });
                });
                let sub_h = if show_sub {
                    ui.add_space(16.0);
                    place_centered(ui, row_h, known_sub, |ui| {
                        self.vfo_cluster(ui, snap, false, sub_freq);
                    })
                } else {
                    known_sub
                };
                let moved = (main_h - known_main).abs() > 0.5
                    || (frame_h - known_frame).abs() > 0.5
                    || (show_sub && (sub_h - known_sub).abs() > 0.5);
                self.main_cluster_height = main_h;
                self.vfo_frame_height = frame_h;
                if show_sub {
                    self.sub_vfo_height = sub_h;
                }
                if moved {
                    ui.ctx()
                        .request_discard("center the VFO frame with the readouts");
                }
            });
            ui.add_space(6.0);
            let (lo, hi) = self.fine_window(snap, freq);
            if let Some(hz) = scale(ui, "Fine", freq, lo, hi, HAM, 36.0, &[]) {
                if self.fine_lock.is_none() {
                    self.fine_lock = Some((lo, hi));
                }
                if self.can_tune_freq(snap, true) {
                    self.tune(snap, hz);
                }
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
                let alt = ui.button("ALT");
                if alt.clicked() && snap.caps.vfo_ops.contains("XCHG") {
                    self.send(Command::VfoOp("XCHG".into()));
                }
                let bsp = crate::toggle::add_toggle(ui, true, self.show_spread, "BSP");
                if bsp.clicked() {
                    self.show_spread = !self.show_spread;
                }
                if ui.button("LOAD").clicked() {
                    self.browse_plan(false);
                }
                let mut chip_pick = None;
                // Memory is keyed by band name, the same key `band_step` uses.
                let rx = rx_pairs(snap);
                let listed: Vec<(usize, String, String, f64, Vec<(f64, f64)>)> = self
                    .plan
                    .zones
                    .get(self.zone_sel)
                    .map(|zone| {
                        zone.bands
                            .iter()
                            .enumerate()
                            .filter_map(|(idx, band)| {
                                let pieces = plan::pieces_inside(band.lo_hz, band.hi_hz, &rx);
                                if pieces.is_empty() {
                                    return None;
                                }
                                Some((
                                    idx,
                                    band.chip.clone(),
                                    band.name.clone(),
                                    band.home_hz,
                                    pieces,
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let chips: Vec<(usize, String, String, f64)> = listed
                    .into_iter()
                    .map(|(idx, chip, name, home, pieces)| {
                        let remembered = self.band_mem.get(&name).copied().unwrap_or(home);
                        let hz = if pieces
                            .iter()
                            .any(|(start, end)| remembered >= *start && remembered <= *end)
                        {
                            remembered
                        } else {
                            pieces[0].0
                        };
                        (idx, chip, name, hz)
                    })
                    .collect();
                for (idx, chip, name, hz) in &chips {
                    let on = self.band_sel == *idx;
                    if ui.selectable_label(on, chip).clicked() && !self.following() {
                        self.band_sel = *idx;
                        if snap.caps.can_set_freq {
                            let tuned = self.band_mem.get(name).copied().unwrap_or(*hz);
                            chip_pick = Some(tuned);
                        }
                    }
                }
                if let Some(hz) = chip_pick {
                    self.tune(snap, hz);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.receiver_box(ui, snap);
                    ui.add_space(8.0);
                    self.mode_box(ui, snap, "Sub", false);
                    ui.add_space(8.0);
                    self.mode_box(ui, snap, "Mode", true);
                });
            });
        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.take_available_width();
            self.buttons(ui, snap);
        });
        if self.selected_hackrf() {
            ui.add_space(8.0);
            self.hackrf_spectrum(ui, snap, freq);
        }
        if self.show_spread {
            ui.add_space(8.0);
            // One line per zone. Subbands share that line, with a small gap
            // between them. The frequency wheel stays above this scroll.
            let frame = egui::Frame::group(ui.style());
            let chrome =
                frame.inner_margin.sum().y + frame.stroke.width * 2.0 + frame.outer_margin.sum().y;
            let height = (ui.available_height() - chrome).max(80.0);
            let mut scale_pick: Option<(usize, usize, f64, bool)> = None;
            frame.show(ui, |ui| {
                ui.take_available_width();
                egui::ScrollArea::vertical()
                    .id_salt("tune-bsp")
                    .auto_shrink([false, true])
                    .max_height(height)
                    .show(ui, |ui| {
                        let rx = rx_pairs(snap);
                        for (zone_idx, zone) in self.plan.zones.iter().enumerate().rev() {
                            let mut subs = Vec::new();
                            let mut ids = Vec::new();
                            for (band_idx, band) in zone.bands.iter().enumerate() {
                                let spans: Vec<PaintSpan> = band
                                    .segments
                                    .iter()
                                    .map(|seg| {
                                        let [r, g, b] = plan::segment_rgb(seg);
                                        PaintSpan {
                                            lo: seg.lo_hz,
                                            hi: seg.hi_hz,
                                            color: egui::Color32::from_rgb(r, g, b),
                                        }
                                    })
                                    .collect();
                                for (lo, hi) in plan::pieces_inside(band.lo_hz, band.hi_hz, &rx) {
                                    ids.push(band_idx);
                                    subs.push(SubBand {
                                        lo,
                                        hi,
                                        chip: band.chip.clone(),
                                        spans: spans.clone(),
                                    });
                                }
                            }
                            if subs.is_empty() {
                                continue;
                            }
                            let picked = (zone_idx == self.zone_sel)
                                .then(|| {
                                    ids.iter()
                                        .enumerate()
                                        .find(|(i, id)| {
                                            **id == self.band_sel
                                                && freq >= subs[*i].lo
                                                && freq <= subs[*i].hi
                                        })
                                        .or_else(|| {
                                            ids.iter()
                                                .enumerate()
                                                .find(|(_, id)| **id == self.band_sel)
                                        })
                                        .map(|(i, _)| i)
                                })
                                .flatten();
                            if let Some((slot, hz, sub)) =
                                zone_scale(ui, &zone.name, freq, &subs, picked, 28.0)
                            {
                                // Left click needs set_freq. Right click needs split.
                                // The other button is ignored when its command is closed.
                                let allowed = if sub {
                                    self.can_tune_freq(snap, false)
                                } else {
                                    self.can_tune_freq(snap, true)
                                };
                                if allowed {
                                    scale_pick = Some((zone_idx, ids[slot], hz, sub));
                                }
                            }
                        }
                    });
            });
            if let Some((zone_idx, band_idx, hz, sub)) = scale_pick {
                let mode = self
                    .plan
                    .zones
                    .get(zone_idx)
                    .and_then(|zone| zone.bands.get(band_idx))
                    .map(|band| plan::mode_at(band, hz));
                self.zone_sel = zone_idx;
                self.band_sel = band_idx;
                if sub {
                    self.tune_sub(snap, hz, mode.as_deref());
                } else if snap.caps.allow_freq(hz).is_some() {
                    self.tune(snap, hz);
                    if let Some(mode) = mode {
                        self.send_plan_mode(snap, &mode);
                    }
                }
            }
        }
    }

    fn page_plan(&mut self, ui: &mut egui::Ui, snap: &mut Snapshot) {
        if self.zone_sel >= self.plan.zones.len() {
            self.zone_sel = 0;
            self.band_sel = 0;
        }
        if !self.plan.zones.is_empty() {
            let band_count = self.plan.zones[self.zone_sel].bands.len();
            if self.band_sel >= band_count {
                self.band_sel = 0;
            }
        }
        let selected = self.plan.zones.get(self.zone_sel).and_then(|zone| {
            zone.bands
                .get(self.band_sel)
                .map(|band| (band.home_hz, band.name.clone(), band.mode.clone()))
        });
        let zone_name = self
            .plan
            .zones
            .get(self.zone_sel)
            .map(|zone| zone.name.clone())
            .unwrap_or_default();
        let homes: Vec<f64> = self
            .plan
            .zones
            .get(self.zone_sel)
            .map(|zone| zone.bands.iter().map(|band| band.home_hz).collect())
            .unwrap_or_default();
        let (win_lo, win_hi) = channel_window(&homes, rx_fallback(snap));
        let spans = self
            .plan
            .zones
            .get(self.zone_sel)
            .map(paint_spans)
            .unwrap_or_default();
        let mut clicked = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.set_width(72.0);
                if radio_key(ui, "LOAD", KeyLamp::Dark, true).clicked() {
                    self.browse_plan(false);
                }
                if radio_key(ui, "SAVE", KeyLamp::Dark, true).clicked() {
                    self.browse_plan(true);
                }
                if radio_key(ui, "ADD", KeyLamp::Dark, true).clicked() {
                    let color = self
                        .plan
                        .zones
                        .get(self.zone_sel)
                        .map(|zone| zone.color)
                        .unwrap_or([92, 184, 120]);
                    let home = self.shown_freq(snap);
                    self.zone_sel = plan::push_zone(&mut self.plan, color, home, &snap.mode);
                    self.band_sel = 0;
                    self.zone_rename = None;
                }
            });
            lcd_panel(ui, |ui| {
                if self.plan.zones.is_empty() {
                    ui.label(
                        egui::RichText::new("No zones in this plan.")
                            .monospace()
                            .color(LCD_PAPER),
                    );
                    return;
                }
                let (name, mode, mhz) = match &selected {
                    Some((hz, name, mode)) => {
                        let name = if name.trim().is_empty() {
                            "—".to_string()
                        } else {
                            name.clone()
                        };
                        (name, mode.clone(), format!("{:.3}", hz / 1_000_000.0))
                    }
                    None => ("—".to_string(), String::new(), "—".to_string()),
                };
                let mode = if mode.trim().is_empty() {
                    "—".to_string()
                } else {
                    mode
                };
                let zone_title = if zone_name.trim().is_empty() {
                    "—".to_string()
                } else {
                    zone_name
                };
                lcd_title_row(ui, &zone_title, &mode);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(name)
                            .size(28.0)
                            .strong()
                            .monospace()
                            .color(LCD_YELLOW),
                    )
                    .truncate(),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("{mhz} MHz"))
                            .size(20.0)
                            .monospace()
                            .color(LCD_PAPER),
                    )
                    .truncate(),
                );
                ui.add_space(4.0);
                let needle = selected.as_ref().map(|(hz, _, _)| *hz);
                clicked = channel_dial(ui, needle, win_lo, win_hi, &spans);
                ui.add_space(4.0);
                let legend_w = ui.available_width();
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(legend_w);
                    ui.set_min_height(18.0);
                    if let Some(zone) = self.plan.zones.get(self.zone_sel) {
                        for (color, label, drawn) in legend_items(zone, win_lo, win_hi) {
                            ui.scope(|ui| {
                                if !drawn {
                                    ui.set_opacity(0.35);
                                }
                                swatch(ui, color, &label);
                            });
                            ui.add_space(10.0);
                        }
                    }
                });
            });
        });
        if let Some(hz) = clicked {
            self.dial_home(snap, hz);
        }
        if self.plan.zones.is_empty() {
            return;
        }
        ui.add_space(6.0);
        self.zone_tabs(ui);
        ui.add_space(8.0);
        let gap = ui.spacing().item_spacing.x;
        let total_w = ui.available_width();
        let widths = fit_columns(total_w, gap, &[280.0, 320.0], 0);
        let left_w = widths[0];
        let right_w = widths[1];
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(left_w);
                egui::ScrollArea::vertical()
                    .id_salt("plan-list")
                    .auto_shrink([false, false])
                    .max_height(height.max(80.0))
                    .show(ui, |ui| {
                        let count = self
                            .plan
                            .zones
                            .get(self.zone_sel)
                            .map(|zone| zone.bands.len())
                            .unwrap_or(0);
                        for idx in 0..count {
                            if self.band_row(ui, idx) {
                                self.recall_band(snap, idx);
                            }
                        }
                    });
            });
            ui.vertical(|ui| {
                ui.set_width(right_w);
                egui::ScrollArea::vertical()
                    .id_salt("plan-editor")
                    .auto_shrink([false, true])
                    .max_height(height.max(80.0))
                    .show(ui, |ui| self.band_editor(ui, snap, right_w));
            });
        });
    }

    fn zone_tabs(&mut self, ui: &mut egui::Ui) {
        let names: Vec<String> = self
            .plan
            .zones
            .iter()
            .map(|zone| zone.name.clone())
            .collect();
        // Zones that do not fit go into the list on the elbow's end cap.
        let sel = self.zone_sel;
        let picked = tools::overflow_tab_row(
            ui,
            "plan-zones",
            &names,
            Some(sel),
            |ui, i| folder_tab_width(ui, &names[i], false),
            |ui, idx, lead| {
                let response = folder_tab(ui, sel == idx, idx, &names[idx], lead);
                if response.secondary_clicked() {
                    self.zone_sel = idx;
                    self.band_sel = 0;
                }
                response.context_menu(|ui| self.zone_menu(ui, idx));
                response
            },
        );
        if let Some(idx) = picked {
            self.zone_sel = idx;
            self.band_sel = 0;
        }
    }

    fn zone_menu(&mut self, ui: &mut egui::Ui, idx: usize) {
        let current = self
            .plan
            .zones
            .get(idx)
            .map(|zone| zone.name.clone())
            .unwrap_or_default();
        let mut text = self
            .zone_rename
            .as_ref()
            .filter(|(stored, _)| *stored == idx)
            .map(|(_, text)| text.clone())
            .unwrap_or(current);
        let edited = ui
            .add(
                egui::TextEdit::singleline(&mut text)
                    .id_salt(("zone-rename", idx))
                    .desired_width(160.0),
            )
            .changed();
        if edited {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                if let Some(zone) = self.plan.zones.get_mut(idx) {
                    plan::rename_zone(zone, trimmed);
                }
            }
            self.zone_rename = Some((idx, text));
        }
        if ui.button("Delete").clicked() {
            self.zone_sel = plan::delete_zone(&mut self.plan, idx, self.zone_sel);
            self.band_sel = 0;
            self.zone_rename = None;
            ui.close();
        }
    }

    fn plan_save_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                self.browse_plan(true);
            }
            ui.label(file_label(&self.plan_path))
                .on_hover_text(&self.plan_path);
        });
    }

    fn page_channels(&mut self, ui: &mut egui::Ui, snap: &mut Snapshot) {
        if !self.channels.zones.is_empty() {
            if self.channel_zone >= self.channels.zones.len() {
                self.channel_zone = 0;
                self.channel_sel = 0;
            }
            let count = self.channels.zones[self.channel_zone].channels.len();
            if count == 0 {
                self.channel_sel = 0;
            } else if self.channel_sel >= count {
                self.channel_sel = count - 1;
            }
        }
        let selected = self.channels.zones.get(self.channel_zone).and_then(|zone| {
            zone.channels
                .get(self.channel_sel)
                .map(|channel| (channel.hz, channel.name.clone(), channel.mode.clone()))
        });
        let zone_name = self
            .channels
            .zones
            .get(self.channel_zone)
            .map(|zone| zone.name.clone())
            .unwrap_or_default();
        let freqs: Vec<f64> = self
            .channels
            .zones
            .get(self.channel_zone)
            .map(|zone| zone.channels.iter().map(|channel| channel.hz).collect())
            .unwrap_or_default();
        let (win_lo, win_hi) = channel_window(&freqs, rx_fallback(snap));
        let marks = plan::marks_for_window(&self.plan, win_lo, win_hi);
        let spans: Vec<PaintSpan> = marks
            .iter()
            .map(|mark| {
                let [r, g, b] = mark.rgb;
                PaintSpan {
                    lo: mark.lo_hz,
                    hi: mark.hi_hz,
                    color: egui::Color32::from_rgb(r, g, b),
                }
            })
            .collect();
        let mut clicked = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.set_width(72.0);
                self.rec_key(ui, false);
                if radio_key(ui, "LOAD", KeyLamp::Dark, true).clicked() {
                    self.browse_channels(false);
                }
                if radio_key(ui, "SAVE", KeyLamp::Dark, true).clicked() {
                    self.browse_channels(true);
                }
                let scan_on = self.memory_scan.is_on();
                let all_zones = self.memory_scan.all_zones();
                let listed_on = !self.scan_zones.is_empty();
                let listed = if listed_on {
                    self.listed_targets(snap)
                } else {
                    Vec::new()
                };
                let lamp = if scan_on && listed_on {
                    KeyLamp::Purple
                } else if all_zones {
                    KeyLamp::Amber
                } else if scan_on {
                    KeyLamp::Green
                } else {
                    KeyLamp::Dark
                };
                let link_block = self.scan_link_block(snap);
                let zone_block = self.scan_block_reason(snap);
                let any_zone = link_block.is_none() && !self.every_target(snap).is_empty();
                let scan_ready = if listed_on {
                    !listed.is_empty()
                } else {
                    any_zone
                };
                let scan_key = radio_key(ui, "SCAN", lamp, link_block.is_none() && scan_ready);
                let scan_double = scan_key.double_clicked();
                let scan_clicked = scan_key.clicked();
                if listed_on && !listed.is_empty() {
                    scan_key.on_hover_text("Selected zones.");
                } else if listed_on {
                    scan_key.on_hover_text("No channel this radio can tune.");
                } else if all_zones {
                    scan_key.on_hover_text("Every zone.");
                } else if zone_block.is_some() && any_zone {
                    scan_key.on_hover_text(
                        "This zone has no channel this radio can tune. Double-click scans every zone.",
                    );
                } else if let Some(reason) = link_block.or(zone_block) {
                    scan_key.on_hover_text(reason);
                }
                if scan_double {
                    // The first click of the double already stopped a running
                    // scan. Marked zones win over the amber all-zone scan.
                    if let Some(reason) = link_block {
                        self.channel_note = reason.to_string();
                    } else if listed_on {
                        self.start_listed_scan(snap);
                    } else if !any_zone {
                        self.channel_note = "No channel this radio can tune.".to_string();
                    } else {
                        self.start_all_scan(snap);
                    }
                } else if scan_clicked {
                    if scan_on {
                        self.stop_scan();
                    } else if listed_on {
                        self.start_listed_scan(snap);
                    } else if let Some(reason) = link_block.or(zone_block) {
                        self.channel_note = reason.to_string();
                    } else {
                        self.start_memory_scan(snap);
                    }
                }
                let dwell = radio_key(
                    ui,
                    "DWELL",
                    if self.memory_scan.dwell_on() {
                        KeyLamp::Green
                    } else {
                        KeyLamp::Dark
                    },
                    true,
                );
                if dwell.clicked() {
                    self.memory_scan.toggle_dwell();
                }
            });
            lcd_panel(ui, |ui| {
                if self.channels.zones.is_empty() {
                    ui.label(
                        egui::RichText::new("No zones in this channel file.")
                            .monospace()
                            .color(LCD_PAPER),
                    );
                    return;
                }
                let mode = selected
                    .as_ref()
                    .map(|(_, _, mode)| mode.clone())
                    .filter(|mode| !mode.trim().is_empty())
                    .unwrap_or_else(|| "—".into());
                let name = selected
                    .as_ref()
                    .map(|(_, name, _)| name.clone())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| "—".into());
                let mhz = match selected.as_ref() {
                    Some((hz, _, _)) => format!("{:.3}", hz / 1_000_000.0),
                    None => "—".to_string(),
                };
                let plan_title = selected
                    .as_ref()
                    .and_then(|(hz, _, _)| plan::label_for_window(&self.plan, win_lo, win_hi, *hz));
                let zone_title = if zone_name.trim().is_empty() {
                    "—".to_string()
                } else {
                    zone_name
                };
                lcd_title_row(ui, &zone_title, &mode);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(name)
                            .size(28.0)
                            .strong()
                            .monospace()
                            .color(LCD_YELLOW),
                    )
                    .truncate(),
                );
                let freq_w = ui.available_width();
                ui.horizontal(|ui| {
                    ui.set_max_width(freq_w);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("{mhz} MHz"))
                                .size(20.0)
                                .monospace()
                                .color(LCD_PAPER),
                        )
                        .truncate(),
                    );
                    if let Some(title) = plan_title {
                        ui.add_space(16.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(title)
                                    .size(16.0)
                                    .monospace()
                                    .color(LCD_PAPER),
                            )
                            .truncate(),
                        );
                    }
                });
                ui.add_space(4.0);
                let needle = selected.as_ref().map(|(hz, _, _)| *hz);
                clicked = channel_dial(ui, needle, win_lo, win_hi, &spans);
                ui.add_space(4.0);
                let legend_w = ui.available_width();
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(legend_w);
                    ui.set_min_height(18.0);
                    let mut seen = Vec::new();
                    for mark in &marks {
                        if seen.contains(&mark.legend) {
                            continue;
                        }
                        seen.push(mark.legend.clone());
                        let [r, g, b] = mark.rgb;
                        swatch(ui, egui::Color32::from_rgb(r, g, b), &mark.legend);
                        ui.add_space(10.0);
                    }
                });
            });
        });
        if let Some(hz) = clicked {
            self.set_channel_hz(snap, hz);
        }
        if self.channels.zones.is_empty() {
            return;
        }
        ui.add_space(8.0);
        let gap = ui.spacing().item_spacing.x;
        let total_w = ui.available_width();
        let widths = fit_columns(total_w, gap, &[280.0, 220.0, 320.0], 1);
        let zone_w = widths[0];
        let channel_w = widths[1];
        let editor_w = widths[2];
        let height = ui.available_height().max(80.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(zone_w);
                egui::ScrollArea::vertical()
                    .id_salt("zone-list")
                    .auto_shrink([false, false])
                    .max_height(height)
                    .show(ui, |ui| {
                        let jump = self.zone_list_follow;
                        let count = self.channels.zones.len();
                        for idx in 0..count {
                            let (clicked, doubled, response) = self.zone_row(ui, idx);
                            if jump && idx == self.channel_zone {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            if doubled {
                                // The first click already selected this zone.
                                // This click only marks or unmarks the scan list.
                                self.toggle_scan_zone(snap, idx);
                            } else if clicked && idx != self.channel_zone {
                                self.channel_zone = idx;
                                let in_list = self.scan_zones.is_empty() || self.scan_zones.contains(&idx);
                                if self.memory_scan.is_on() && in_list {
                                    self.retarget_scan(snap);
                                } else if !self.memory_scan.is_on() {
                                    self.channel_sel = 0;
                                }
                            }
                        }
                        self.zone_list_follow = false;
                    });
            });
            ui.vertical(|ui| {
                ui.set_width(channel_w);
                egui::ScrollArea::vertical()
                    .id_salt("channel-list")
                    .auto_shrink([false, false])
                    .max_height(height)
                    .show(ui, |ui| {
                        let count = self
                            .channels
                            .zones
                            .get(self.channel_zone)
                            .map(|zone| zone.channels.len())
                            .unwrap_or(0);
                        for idx in 0..count {
                            let (clicked, marked) = self.channel_row(ui, idx);
                            if marked {
                                self.rec_toggle_channel(idx);
                            }
                            if clicked {
                                self.stop_scan();
                                self.recall_channel(snap, idx);
                            }
                        }
                        self.channel_list_follow = false;
                    });
            });
            ui.vertical(|ui| {
                ui.set_width(editor_w);
                egui::ScrollArea::vertical()
                    .id_salt("channel-editor")
                    .auto_shrink([false, true])
                    .max_height(height)
                    .show(ui, |ui| self.channel_editor(ui, snap, editor_w));
            });
        });
    }

    fn browse_plan(&mut self, save: bool) {
        let dir = remembered_dir(&self.plan_dir);
        let dialog = rfd::FileDialog::new()
            .set_title(if save {
                "Save band plan"
            } else {
                "Load band plan"
            })
            .add_filter("Band plan", &["json"])
            .set_directory(&dir);
        let picked = if save {
            let name = Path::new(&self.plan_path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "band-plan.json".into());
            dialog.set_file_name(name).save_file()
        } else {
            dialog.pick_file()
        };
        let Some(path) = picked else {
            return;
        };
        if let Some(parent) = path.parent() {
            if parent.is_dir() {
                self.plan_dir = Some(parent.to_path_buf());
            }
        }
        self.plan_path = path.display().to_string();
        if save {
            self.save_plan();
        } else {
            self.load_plan();
        }
    }

    fn load_plan(&mut self) {
        match plan::load_file(Path::new(&self.plan_path)) {
            Ok(plan) => {
                self.plan = plan;
                self.zone_sel = 0;
                self.band_sel = self
                    .plan
                    .zones
                    .first()
                    .and_then(|zone| zone.bands.iter().position(|band| band.chip == "20m"))
                    .unwrap_or(0);
                self.plan_note.clear();
                self.plan_note_ptt = false;
            }
            Err(err) => {
                self.plan_note = err;
                self.plan_note_ptt = false;
            }
        }
    }

    fn save_plan(&mut self) {
        match plan::save_file(Path::new(&self.plan_path), &self.plan) {
            Ok(()) => {
                if !self.plan_note_ptt {
                    self.plan_note.clear();
                }
            }
            Err(err) => {
                self.plan_note = err;
                self.plan_note_ptt = false;
            }
        }
    }

    fn browse_channels(&mut self, save: bool) {
        let dir = remembered_dir(&self.channel_dir);
        let dialog = rfd::FileDialog::new()
            .set_title(if save {
                "Save channels"
            } else {
                "Load channels"
            })
            .add_filter("Channels", &["json"])
            .set_directory(&dir);
        let picked = if save {
            let name = if self.channel_path.is_empty() {
                "channels.json".to_string()
            } else {
                file_label(&self.channel_path)
            };
            dialog.set_file_name(name).save_file()
        } else {
            dialog.pick_file()
        };
        let Some(path) = picked else {
            return;
        };
        if let Some(parent) = path.parent() {
            if parent.is_dir() {
                self.channel_dir = Some(parent.to_path_buf());
            }
        }
        self.channel_path = path.display().to_string();
        if save {
            self.save_channels();
        } else {
            self.load_channels();
        }
    }

    /// Read a dsd-fme hex CSV from disk. The first line is the header.
    /// Cancel leaves the stored keys alone.
    fn browse_keys(&mut self) {
        let dir = remembered_dir(&self.key_dir);
        let picked = rfd::FileDialog::new()
            .set_title("Import keys")
            .add_filter("dsd-fme hex CSV", &["csv", "txt"])
            .add_filter("All files", &["*"])
            .set_directory(&dir)
            .pick_file();
        let Some(path) = picked else {
            return;
        };
        if let Some(parent) = path.parent() {
            if parent.is_dir() {
                self.key_dir = Some(parent.to_path_buf());
            }
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                self.key_note = format!("Could not read {}: {err}", path.display());
                return;
            }
        };
        match keys::import_csv(&mut self.keys, &text) {
            Ok(count) => {
                self.key_shown = None;
                self.save_keys();
                if self.key_note.is_empty() {
                    self.key_note = format!("Imported {count}.");
                }
            }
            Err(err) => self.key_note = err,
        }
    }

    /// Write the key list as the hex CSV Import reads. Cancel writes nothing.
    fn export_keys(&mut self) {
        let dir = remembered_dir(&self.key_dir);
        let picked = rfd::FileDialog::new()
            .set_title("Export keys")
            .add_filter("dsd-fme hex CSV", &["csv", "txt"])
            .add_filter("All files", &["*"])
            .set_directory(&dir)
            .set_file_name("keys.csv")
            .save_file();
        let Some(path) = picked else {
            return;
        };
        if let Some(parent) = path.parent() {
            if parent.is_dir() {
                self.key_dir = Some(parent.to_path_buf());
            }
        }
        match keys::export_file(&path, &self.keys) {
            Ok(count) => self.key_note = format!("Exported {count}."),
            Err(err) => self.key_note = err,
        }
    }

    fn load_channels(&mut self) {
        match channels::load_file(Path::new(&self.channel_path)) {
            Ok(file) => {
                self.channels = file;
                self.channel_zone = 0;
                self.channel_sel = 0;
                self.channel_note.clear();
                self.clear_scan_zones();
            }
            Err(err) => {
                self.channel_note = err;
            }
        }
    }

    fn save_channels(&mut self) {
        match channels::save_file(Path::new(&self.channel_path), &self.channels) {
            Ok(()) => self.channel_note.clear(),
            Err(err) => self.channel_note = err,
        }
    }

    /// Grants, direct calls, and radio sightings from one batch of decoder
    /// events. `true` means the talkgroup or radio file gained something.
    fn note_trunk_events(&mut self, id: u64, parsed: &[follow::CcEvent]) -> bool {
        let now = std::time::Instant::now();
        let mut added = false;
        let mut direct: Vec<(f64, u32)> = Vec::new();
        for event in parsed {
            match event {
                follow::CcEvent::Direct { src, tgt, hz, encrypted } => {
                    if let Some(hz) = hz {
                        direct.push((*hz, *tgt));
                    }
                    self.note_direct(id, *src, *tgt, *hz, *encrypted, now);
                }
                follow::CcEvent::Grant { hz, tg, alg } => {
                    // A direct call's grant carries the called radio as its
                    // "talkgroup". It is not a talkgroup.
                    if direct.iter().any(|(dhz, dtg)| dtg == tg && (dhz - hz).abs() < 1.0) {
                        continue;
                    }
                    // A grant alone is the weaker "granted" mark.
                    self.tg_recent.grant(*tg, now);
                    let encrypted = *alg != follow::CLEAR_ALG;
                    if talkgroups::note_seen(&mut self.talkgroups, *tg, encrypted) {
                        added = true;
                    }
                }
                follow::CcEvent::Unit(sight) => {
                    if units::note(&mut self.units, sight) {
                        added = true;
                    }
                }
                _ => {}
            }
        }
        added
    }

    fn unit_label(&self, id: u32) -> String {
        match self.units.iter().find(|unit| unit.id == id) {
            Some(unit) if !unit.description.is_empty() && unit.description != id.to_string() => {
                format!("{id} ({})", unit.description)
            }
            _ => id.to_string(),
        }
    }

    /// Log a direct call once per pair, again after ten quiet seconds.
    fn note_direct(
        &mut self,
        id: u64,
        src: u32,
        tgt: u32,
        hz: Option<f64>,
        encrypted: bool,
        now: std::time::Instant,
    ) {
        if let Some((s, t, at)) = self.direct_seen.get(&id) {
            if *s == src && *t == tgt && now.saturating_duration_since(*at).as_secs() < 10 {
                self.direct_seen.insert(id, (src, tgt, now));
                return;
            }
        }
        self.direct_seen.insert(id, (src, tgt, now));
        let line =
            follow::direct_summary(&self.unit_label(src), &self.unit_label(tgt), hz, encrypted);
        self.cc_tail.entry(id).or_default().push(line);
    }

    fn note_voice_frames(&mut self, id: u64, events: &[follow::CcEvent]) {
        if events.iter().any(|event| matches!(event, follow::CcEvent::Voice)) {
            self.voice_frames.insert(id, std::time::Instant::now());
        }
    }

    /// Voice frames were decoded on this radio within the pulse hang.
    fn hearing_voice(&self, id: u64, now: std::time::Instant) -> bool {
        self.voice_frames
            .get(&id)
            .is_some_and(|at| now.saturating_duration_since(*at) <= pulse::HANG)
    }

    fn talkgroup_encrypted(&self, tg: u32) -> bool {
        self.talkgroups
            .zones
            .iter()
            .flat_map(|zone| zone.talkgroups.iter())
            .any(|row| row.tg == tg && row.encrypted)
    }

    /// A followed call. The tab pulses and the row is fully lit only while
    /// voice frames are decoded; a grant alone gets the weaker mark.
    fn mark_call(&mut self, id: u64, tg: u32, now: std::time::Instant) {
        let voice = self.hearing_voice(id, now);
        if voice {
            self.voice_pulse.hear_talkgroup(id, tg, now);
        }
        if self.is_direct_target(id, tg, now) {
            return;
        }
        if voice {
            self.tg_recent.note(tg, now);
        } else {
            self.tg_recent.grant(tg, now);
        }
    }

    /// A followed direct call reports the called radio as its talkgroup.
    fn is_direct_target(&self, radio: u64, tg: u32, now: std::time::Instant) -> bool {
        self.direct_seen.get(&radio).is_some_and(|(_, t, at)| {
            *t == tg && now.saturating_duration_since(*at).as_secs() < 30
        })
    }

    /// Conventional hold, or a clear voice grant. A control channel is not a mark.
    fn note_voice(&mut self, now: std::time::Instant) {
        self.voice_pulse.prune(now);
        self.tg_recent.prune(now);
        if let Some((zone, index)) = self.memory_scan.hold_memory() {
            let control = self
                .channels
                .zones
                .get(zone)
                .and_then(|zone| zone.channels.get(index))
                .is_some_and(|channel| channel.control);
            if !control {
                self.voice_pulse.hear_channel(zone, index, now);
            }
        }
        let grants: Vec<(u64, u32)> = self
            .followers
            .iter()
            .filter_map(|(id, follower)| follower.active_tg().map(|tg| (*id, tg)))
            .collect();
        for (id, tg) in grants {
            self.mark_call(id, tg, now);
        }
        let calls: Vec<(u64, u32)> = self
            .hackrf_call
            .iter()
            .filter_map(|(id, call)| call.map(|(_, tg)| (*id, tg)))
            .collect();
        for (id, tg) in calls {
            self.mark_call(id, tg, now);
        }
    }

    fn tab_voice(&self, page: Page, now: std::time::Instant) -> Option<pulse::VoiceTab> {
        match page {
            Page::Channels => self.voice_pulse.showing_channel(now),
            Page::Talkgroups(id) => {
                let tab = pulse::VoiceTab::Talkgroup { radio: id };
                self.voice_pulse.marker(&tab, now).then_some(tab)
            }
            _ => None,
        }
    }

    /// Channels and each TG tab always carry a speaker. Quiet is dim.
    fn speaker_alpha(&self, page: Page, now: std::time::Instant) -> Option<f32> {
        let presence = match page {
            Page::Channels => self.voice_pulse.channel_presence(now),
            Page::Talkgroups(id) => {
                self.voice_pulse
                    .presence(&pulse::VoiceTab::Talkgroup { radio: id }, now)
            }
            _ => return None,
        };
        Some(pulse::speaker_alpha(presence))
    }

    fn store_pulse(&mut self, snap: &Snapshot) {
        if self.station_saved.is_some() && !self.config_path.is_empty() {
            self.save_config(snap);
        }
    }

    fn page_config(&mut self, ui: &mut egui::Ui, snap: &mut Snapshot) {
        cfg_group(ui, "Station", |ui| {
            ui.horizontal(|ui| {
                if ui.button("Load").clicked() {
                    self.browse_config(false, snap);
                }
                if ui.button("Save").clicked() {
                    self.browse_config(true, snap);
                }
                let label = if self.config_path.is_empty() {
                    "No file".to_string()
                } else {
                    file_label(&self.config_path)
                };
                ui.label(label).on_hover_text(&self.config_path);
            });
            if !self.config_note.is_empty() {
                ui.label(
                    egui::RichText::new(&self.config_note).color(caution()),
                );
            }
            ui.add_space(8.0);
            ui.label(format!("Band plan: {}", file_label(&self.plan_path)));
            ui.label(format!("Channels: {} zones", self.channels.zones.len()));
            ui.label(format!("Talkgroups: {} zones", self.talkgroups.zones.len()));
        });
        cfg_group(ui, "Display", |ui| {
            let mut pulse_on = self.pulse_on;
            if ui.checkbox(&mut pulse_on, "Pulse busy tabs").changed() {
                self.pulse_on = pulse_on;
                self.store_pulse(snap);
            }
            ui.horizontal(|ui| {
                ui.label("Pulse speed");
                for (label, secs) in [("Slow", 3.0), ("Normal", 2.0), ("Fast", 1.0)] {
                    let on = (self.pulse_secs - secs).abs() < 0.05;
                    if ui.selectable_label(on, label).clicked() && !on {
                        self.pulse_secs = secs;
                        self.store_pulse(snap);
                    }
                }
            });
        });
        self.cat_frame(ui);
        self.rotator_amp_frame(ui);
        cfg_group(ui, "Trunking", |ui| {
            ui.horizontal(|ui| {
                ui.label("Trunk voice hang")
                    .on_hover_text("How long a followed call is held after the talker stops, so butt-keys and dead carriers do not drop the audio.");
                const HANGS: [u32; 3] = [500, 1500, 3000];
                for ms in HANGS {
                    let on = self.voice_hang_ms == ms;
                    if ui.selectable_label(on, format!("{ms} ms")).clicked() && !on {
                        self.voice_hang_ms = ms;
                        self.store_pulse(snap);
                    }
                }
                if !HANGS.contains(&self.voice_hang_ms) {
                    // A station file may hold another value. Show it and keep it.
                    let _ = ui.selectable_label(true, format!("{} ms", self.voice_hang_ms));
                }
                let mut allow = self.override_hold;
                if ui
                    .checkbox(&mut allow, "Emergency/priority override")
                    .on_hover_text("An emergency grant or a talkgroup marked Priority may take the receiver off the call it is holding.")
                    .changed()
                {
                    self.override_hold = allow;
                    self.store_pulse(snap);
                }
            });
        });
        self.audio_frame(ui);
        self.automation_frame(ui, snap);
        let radios: Vec<(String, bool)> = self
            .hub
            .view()
            .rows
            .iter()
            .map(|row| (row.spec.name.clone(), patch_radio_tx(&row.spec)))
            .collect();
        if self.amp_models.is_empty() {
            self.amp_models = amp::model_list();
        }
        let amps = self.amplifiers.view();
        let link = patchbay::AmpLink { rows: &amps.rows, models: &self.amp_models };
        cfg_group(ui, "RF patch", |ui| {
            patchbay::show(ui, &mut self.patchbay, &mut self.patchbay_ui, &radios, &link);
        });
        for action in self.patchbay_ui.take_actions() {
            match action {
                patchbay::Action::AddAmplifier(spec) => {
                    if amp::validate_spec(&spec).is_ok() {
                        self.amplifiers.add(spec);
                    }
                }
                // An explicit Standby/Operate click on a bound box. The id
                // is resolved now; a missing or ambiguous id sends nothing.
                patchbay::Action::Control(amp_id, cmd) => {
                    self.send_amp(&amp_id, cmd);
                }
            }
        }
    }

    fn sync_gpredict(&mut self) {
        let radios = self.hub.view();
        let rots = self.rotators.view();
        let sig = gpredict_signature(&radios, &rots);
        if sig == self.gpredict_sig {
            return;
        }
        let dir = match gpredict_hw::hwconf_dir() {
            Ok(dir) => dir,
            Err(err) => {
                self.gpredict_note = err;
                self.gpredict_sig = sig;
                return;
            }
        };
        let radio_rows: Vec<gpredict_hw::Row> = radios
            .rows
            .iter()
            .map(|row| gpredict_hw::Row {
                id: row.id,
                name: row.spec.name.clone(),
                port: row.port,
                hamlib: row.spec.driver == cat::RadioDriver::Hamlib,
            })
            .collect();
        let rot_rows: Vec<gpredict_hw::Row> = rots
            .rows
            .iter()
            .map(|row| gpredict_hw::Row {
                id: row.id,
                name: row.spec.name.clone(),
                port: row.port,
                hamlib: true,
            })
            .collect();
        match gpredict_hw::sync(&dir, &radio_rows, &rot_rows) {
            Ok(out) => {
                self.gpredict_note.clear();
                self.gpredict_files = out.files;
                self.gpredict_wires = out.wires;
                self.gpredict_sig = sig;
            }
            Err(err) => {
                self.gpredict_note = err;
                self.gpredict_sig = sig;
            }
        }
    }

    fn paint_gpredict(&mut self, ui: &mut egui::Ui, radios: &cat::HubView, rots: &rot::RotView) {
        if let Ok(dir) = gpredict_hw::hwconf_dir() {
            if let Ok(files) = gpredict_hw::list(&dir) {
                self.gpredict_files = files;
            }
        }
        self.ensure_bay_art(ui.ctx());
        let mut devices = Vec::new();
        for row in radios
            .rows
            .iter()
            .filter(|row| row.spec.driver == cat::RadioDriver::Hamlib)
        {
            let stem = self
                .gpredict_wires
                .iter()
                .find(|wire| wire.kind == gpredict_hw::Kind::Radio && wire.id == row.id)
                .map(|wire| wire.stem.clone())
                .unwrap_or_default();
            devices.push(auto_bay::Device {
                title: row.spec.name.clone(),
                stem,
                kind: auto_bay::Kind::Radio,
            });
        }
        for row in &rots.rows {
            let stem = self
                .gpredict_wires
                .iter()
                .find(|wire| wire.kind == gpredict_hw::Kind::Rotator && wire.id == row.id)
                .map(|wire| wire.stem.clone())
                .unwrap_or_default();
            devices.push(auto_bay::Device {
                title: row.spec.name.clone(),
                stem,
                kind: auto_bay::Kind::Rotator,
            });
        }
        let files: Vec<auto_bay::HwFile> = self
            .gpredict_files
            .iter()
            .map(|file| auto_bay::HwFile {
                stem: file.stem.clone(),
                label: String::new(),
                owned: file.owned,
            })
            .collect();
        self.paint_auto_scene(ui, "Gpredict", &devices, &files);
    }

    fn paint_auto_scene(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        devices: &[auto_bay::Device],
        files: &[auto_bay::HwFile],
    ) {
        let scene = auto_bay::layout(ui.available_width(), devices, files);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(scene.width, scene.height), egui::Sense::hover());
        let origin = rect.min;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(22, 24, 28));
        let panel = egui::Rect::from_min_size(
            origin + egui::vec2(scene.panel[0], scene.panel[1]),
            egui::vec2(scene.panel[2], scene.panel[3]),
        );
        painter.rect_filled(panel, 4.0, egui::Color32::from_rgb(32, 36, 42));
        painter.text(
            panel.left_top() + egui::vec2(12.0, 6.0),
            egui::Align2::LEFT_TOP,
            title,
            egui::FontId::proportional(13.0),
            egui::Color32::from_gray(220),
        );
        let wire_color = egui::Color32::from_rgb(70, 140, 220);
        for (from, to) in &scene.wires {
            paint_curve(&painter, origin, *from, *to, wire_color);
        }
        for card in &scene.cards {
            let card_w = (card.port[0] - card.origin[0] + 12.0).max(48.0);
            let card_rect = egui::Rect::from_min_size(
                origin + egui::vec2(card.origin[0], card.origin[1]),
                egui::vec2(card_w, 56.0),
            );
            painter.rect_filled(card_rect, 4.0, egui::Color32::from_rgb(32, 36, 42));
            if card.kind == auto_bay::Kind::Radio {
                if let Some(tex) = &self.radio_tex {
                    let icon = egui::Rect::from_min_size(
                        card_rect.min + egui::vec2(6.0, 8.0),
                        egui::vec2(36.0, 36.0),
                    );
                    painter.image(
                        tex.id(),
                        icon,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
            }
            painter.text(
                card_rect.left_center() + egui::vec2(48.0, 0.0),
                egui::Align2::LEFT_CENTER,
                &card.title,
                egui::FontId::proportional(13.0),
                egui::Color32::from_gray(220),
            );
            let color = if card.wired {
                wire_color
            } else {
                egui::Color32::from_gray(90)
            };
            painter.circle_filled(origin + egui::vec2(card.port[0], card.port[1]), 4.0, color);
        }
        for file in &scene.files {
            let at = origin + egui::vec2(file.at[0], file.at[1]);
            let color = if file.owned {
                wire_color
            } else {
                egui::Color32::from_gray(140)
            };
            painter.circle_filled(at, 4.0, color);
            painter.text(
                at + egui::vec2(10.0, 0.0),
                egui::Align2::LEFT_CENTER,
                &file.label,
                egui::FontId::monospace(10.0),
                color,
            );
        }
    }

    /// P25 radios on the left, the control channel they are hunting on the
    /// right. A blue wire means that channel is moving the dial.
    fn paint_control(&mut self, ui: &mut egui::Ui, radios: &cat::HubView, snap: &Snapshot) {
        let zone_name = self
            .channels
            .zones
            .get(self.channel_zone)
            .map(|zone| zone.name.as_str())
            .unwrap_or("");
        ui.label(format!("Control · {zone_name}"));
        let p25: Vec<_> = radios
            .rows
            .iter()
            .filter(|row| hears_trunk_pipe(row))
            .collect();
        if p25.is_empty() {
            ui.label("No trunking receive pipe is on.");
            return;
        }
        let mut devices = Vec::new();
        let mut files = Vec::new();
        for row in p25 {
            let driving = self.cc_on.get(&row.id).copied().unwrap_or(false) && !row.spec.follow;
            let stem = format!("cc-{}", row.id);
            devices.push(auto_bay::Device {
                title: row.spec.name.clone(),
                stem: if driving { stem.clone() } else { String::new() },
                kind: auto_bay::Kind::Radio,
            });
            files.push(auto_bay::HwFile {
                stem,
                label: self.control_port_label(row.id),
                owned: driving,
            });
        }
        self.paint_auto_scene(ui, "Control", &devices, &files);
        for row in radios
            .rows
            .iter()
            .filter(|row| hears_trunk_pipe(row))
        {
            let mut follow = self.cc_on.get(&row.id).copied().unwrap_or(false);
            let toggle = ui.add_enabled(
                !row.spec.follow,
                egui::Checkbox::new(&mut follow, format!("Follow control ({})", row.spec.name)),
            );
            if row.spec.follow {
                toggle.on_hover_text("Gpredict is moving this radio.");
            }
            if self.cc_on.get(&row.id).copied().unwrap_or(false) != follow {
                self.cc_on.insert(row.id, follow);
                if !self.config_path.is_empty() {
                    self.save_config(snap);
                }
            }
            if let Some(status) = self.followers.get(&row.id).map(|follower| follower.status()) {
                ui.label(format!("{} · {status}", row.spec.name));
            }
            self.decoder_log(ui, row.id, &row.spec.name);
        }
    }

    fn automation_frame(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        let radios = self.hub.view();
        let rots = self.rotators.view();
        let mut follow = None;
        cfg_group(ui, "Automation patch", |ui| {
            ui.horizontal(|ui| {
                ui.label("Automation");
                if ui
                    .add_enabled(gpredict_on_path(), egui::Button::new("Gpredict").small())
                    .clicked()
                {
                    launch_gpredict();
                }
            });
            ui.add_space(4.0);
            self.paint_gpredict(ui, &radios, &rots);
            ui.add_space(8.0);
            self.paint_control(ui, &radios, snap);
            if !self.gpredict_note.is_empty() {
                ui.label(
                    egui::RichText::new(&self.gpredict_note)
                        .color(caution()),
                );
            }
            ui.label("Leave Gpredict device 2 at None. Downlink is VFO A and uplink is VFO B. Close Radio Control and open it again, then Engage. A wire does not move a radio that is already engaged.");
            let selected = radios.rows.iter().find(|row| row.selected);
            match selected {
                Some(row) => {
                    let on = row.spec.follow;
                    if crate::toggle::add_toggle(ui, true, on, "Follow").clicked() {
                        follow = Some((row.id, !on));
                    }
                }
                None => {
                    ui.label("No radio is selected.");
                }
            }
        });
        if let Some((id, on)) = follow {
            self.hub.set_follow(id, on);
        }
    }

    /// Rotator and Amplifier groups, shown under Radio.
    fn rotator_amp_frame(&mut self, ui: &mut egui::Ui) {
        let rots = self.rotators.view();
        let mut select = None;
        cfg_group(ui, "Rotator", |ui| {
            egui::Grid::new("rotators")
                .striped(true)
                .min_col_width(64.0)
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.label("Model");
                    ui.label("Device");
                    ui.label("Listen");
                    ui.label("Az");
                    ui.label("El");
                    ui.label("Status");
                    ui.end_row();
                    for row in &rots.rows {
                        let model = if row.make.is_empty() && row.model_label.is_empty() {
                            row.spec.model.to_string()
                        } else {
                            format!("{}  {} {}", row.spec.model, row.make, row.model_label)
                        };
                        let device = if row.spec.device.trim().is_empty() {
                            "—".to_string()
                        } else {
                            row.spec.device.clone()
                        };
                        let listen = match row.port {
                            Some(port) => format!("127.0.0.1:{port}"),
                            None => "127.0.0.1".to_string(),
                        };
                        let az = row
                            .azimuth
                            .map(|deg| format!("{deg:.1}"))
                            .unwrap_or_else(|| "—".into());
                        let el = row
                            .elevation
                            .map(|deg| format!("{deg:.1}"))
                            .unwrap_or_else(|| "—".into());
                        if cat_cell(ui, &row.spec.name, row.selected) {
                            select = Some(row.id);
                        }
                        if cat_cell(ui, &model, row.selected) {
                            select = Some(row.id);
                        }
                        if cat_cell(ui, &device, row.selected) {
                            select = Some(row.id);
                        }
                        if cat_cell(ui, &listen, row.selected) {
                            select = Some(row.id);
                        }
                        if cat_cell(ui, &az, row.selected) {
                            select = Some(row.id);
                        }
                        if cat_cell(ui, &el, row.selected) {
                            select = Some(row.id);
                        }
                        if cat_cell(ui, &row.status, row.selected) {
                            select = Some(row.id);
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Add").clicked() {
                    self.open_rotator(None);
                }
                let selected = rots.rows.iter().find(|row| row.selected).map(|row| row.id);
                if ui
                    .add_enabled(selected.is_some(), egui::Button::new("Edit"))
                    .clicked()
                {
                    if let Some(id) = selected {
                        if let Some(row) = rots.rows.iter().find(|row| row.id == id) {
                            self.open_rotator(Some(row));
                        }
                    }
                }
                if ui
                    .add_enabled(selected.is_some(), egui::Button::new("Remove"))
                    .clicked()
                {
                    if let Some(id) = selected {
                        self.rotators.remove(id);
                    }
                }
            });
            ui.label("A Yaesu G-5500 uses GS-232A (601) or GS-232B (603).");
        });
        cfg_group(ui, "Amplifier", |ui| {
            self.amplifier_rows(ui);
        });
        if let Some(id) = select {
            self.rotators.select(id);
        }
    }

    fn cat_frame(&mut self, ui: &mut egui::Ui) {
        let view = self.hub.view();
        let mut select = None;
        cfg_group(ui, "Radio", |ui| {
            egui::Grid::new("cat-radios")
                .striped(true)
                .min_col_width(72.0)
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.label("Driver");
                    ui.label("Device");
                    ui.label("Listen");
                    ui.label("Status");
                    ui.end_row();
                    for row in &view.rows {
                        let listen = radio_listen(row);
                        let device = if row.spec.device.trim().is_empty() {
                            "—".to_string()
                        } else {
                            row.spec.device.clone()
                        };
                        if cat_cell(ui, &row.spec.name, row.selected) {
                            select = Some(row.id);
                        }
                        if cat_cell(ui, cat::driver_name(row.spec.driver), row.selected) {
                            select = Some(row.id);
                        }
                        if cat_cell(ui, &device, row.selected) {
                            select = Some(row.id);
                        }
                        if cat_cell(ui, &listen, row.selected) {
                            select = Some(row.id);
                        }
                        if cat_cell(ui, &row.status, row.selected) {
                            select = Some(row.id);
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Add hamlib").clicked() {
                    self.open_hamlib(None);
                }
                if ui.button("Add GQRX").clicked() {
                    self.open_gqrx(None);
                }
                if ui.button("Add HackRF").clicked() {
                    self.open_hackrf(None);
                }
                if ui.button("Add audio tap").clicked() {
                    self.open_tap(None);
                }
                let selected = view.rows.iter().find(|row| row.selected).map(|row| row.id);
                if ui
                    .add_enabled(selected.is_some(), egui::Button::new("Edit"))
                    .clicked()
                {
                    if let Some(id) = selected {
                        if let Some(row) = view.rows.iter().find(|row| row.id == id) {
                            match row.spec.driver {
                                cat::RadioDriver::Hamlib => self.open_hamlib(Some(row)),
                                cat::RadioDriver::Gqrx => self.open_gqrx(Some(row)),
                                cat::RadioDriver::Hackrf => self.open_hackrf(Some(row)),
                                cat::RadioDriver::Tap => self.open_tap(Some(row)),
                            }
                        }
                    }
                }
                if ui
                    .add_enabled(selected.is_some(), egui::Button::new("Remove"))
                    .clicked()
                {
                    if let Some(id) = selected {
                        self.hub.remove(id);
                    }
                }
            });
            ui.add_space(6.0);
            ui.label("USB ports");
            if view.serial.is_empty() {
                ui.label("none");
            } else {
                for path in &view.serial {
                    ui.label(path);
                }
            }
        });
        if let Some(id) = select {
            self.hub.select(id);
        }
    }

    /// Clear WirePlumber's saved defaults, restart the user PipeWire session,
    /// then drop this station's speaker and microphone pins. The hub wires
    /// the cards to whatever the desktop default is after that restart.
    fn start_pipewire_reset(&self) {
        if self
            .pipewire_reset
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let flag = Arc::clone(&self.pipewire_reset);
        let hub = Arc::clone(&self.hub);
        thread::spawn(move || {
            let reset = crate::audio::reset_user_pipewire();
            hub.reset_pipewire_defaults(reset);
            flag.store(false, Ordering::Release);
        });
    }

    fn audio_frame(&mut self, ui: &mut egui::Ui) {
        let view = self.hub.view();
        let mut endpoint = None;
        let mut pipe_apply = None;
        let mut tx_apply = None;
        let mut pipe_editor_open: Option<(u64, bool, String)> = None;
        cfg_group(ui, "Audio patch", |ui| {
            ui.horizontal(|ui| {
                ui.label("Audio");
                if ui
                    .add_enabled(helvum_on_path(), egui::Button::new("Helvum").small())
                    .clicked()
                {
                    launch_helvum();
                }
                let resetting = self.pipewire_reset.load(Ordering::Acquire);
                let label = if resetting {
                    "Resetting PipeWire…"
                } else {
                    "Reset PipeWire"
                };
                if ui
                    .add_enabled(!resetting, egui::Button::new(label).small())
                    .on_hover_text(
                        "Forget WirePlumber's saved devices and rewire this station to the desktop default sink and source. Live audio drops until PipeWire is back.",
                    )
                    .clicked()
                {
                    self.start_pipewire_reset();
                }
            });
            ui.add_space(4.0);
            if self.config_path.is_empty() {
                ui.label("Open a station file to keep PipeWire changes.");
            }
            if !view.audio_note.is_empty() {
                ui.label(
                    egui::RichText::new(&view.audio_note)
                        .color(caution()),
                );
            }
            for row in &view.rows {
                let tap = row.spec.driver == cat::RadioDriver::Tap;
                let rx = crate::audio::describe(&row.make, &row.model_label, row.id, "RX");
                let tx = crate::audio::describe(&row.make, &row.model_label, row.id, "TX");
                ui.label(format!("{}  {} {}", row.id, row.make, row.model_label));
                ui.horizontal(|ui| {
                    ui.label(if tap { "Speaker".to_string() } else { rx });
                    let current = audio_device_label(&view.playback, &row.spec.audio.output);
                    egui::ComboBox::from_id_salt(("audio-out", row.id))
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for dev in &view.playback {
                                let text = audio_device_label(&view.playback, &dev.name);
                                if ui
                                    .selectable_label(row.spec.audio.output == dev.name, text)
                                    .clicked()
                                {
                                    endpoint = Some((
                                        row.id,
                                        row.spec.audio.input.clone(),
                                        dev.name.clone(),
                                    ));
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(if tap { "Audio in".to_string() } else { tx });
                    let current = audio_device_label(&view.capture, &row.spec.audio.input);
                    egui::ComboBox::from_id_salt(("audio-in", row.id))
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for dev in &view.capture {
                                let text = audio_device_label(&view.capture, &dev.name);
                                if ui
                                    .selectable_label(row.spec.audio.input == dev.name, text)
                                    .clicked()
                                {
                                    endpoint = Some((
                                        row.id,
                                        dev.name.clone(),
                                        row.spec.audio.output.clone(),
                                    ));
                                }
                            }
                        });
                });
                if !row.audio_status.is_empty() {
                    ui.label(
                        egui::RichText::new(&row.audio_status)
                            .color(caution()),
                    );
                }
                ui.horizontal(|ui| {
                    ui.label("RX pipe");
                    let scripts = crate::rx_pipe::modem_scripts();
                    let current = row.spec.rx_pipe.as_str();
                    egui::ComboBox::from_id_salt(("rx-pipe", row.id))
                        .selected_text(crate::rx_pipe::choice_text(current, &scripts))
                        .width(280.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(current.is_empty(), "none").clicked()
                                && !current.is_empty()
                            {
                                pipe_apply = Some((row.id, String::new()));
                            }
                            for script in &scripts {
                                let on = crate::rx_pipe::script_matches(current, script);
                                if ui
                                    .selectable_label(
                                        on,
                                        crate::rx_pipe::pipeline_label(&script.name),
                                    )
                                    .clicked()
                                    && !on
                                {
                                    pipe_apply = Some((row.id, script.command.clone()));
                                }
                            }
                        });
                    if ui.small_button("Pipes…").clicked() {
                        pipe_editor_open = Some((row.id, false, row.spec.rx_pipe.clone()));
                    }
                    if !current.is_empty() && crate::rx_pipe::resolve_pipe(current).is_err() {
                        ui.label(egui::RichText::new("not runnable").small().color(caution()));
                    }
                });
                if !row.rx_pipe_status.is_empty() {
                    ui.label(
                        egui::RichText::new(&row.rx_pipe_status)
                            .color(caution()),
                    );
                }
                if !tap {
                ui.horizontal(|ui| {
                    ui.label("TX pipe");
                    let scripts = crate::rx_pipe::tx_scripts();
                    let current = row.spec.tx_pipe.as_str();
                    egui::ComboBox::from_id_salt(("tx-pipe", row.id))
                        .selected_text(crate::rx_pipe::choice_text(current, &scripts))
                        .width(280.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(current.is_empty(), "none").clicked()
                                && !current.is_empty()
                            {
                                tx_apply = Some((row.id, String::new()));
                            }
                            for script in &scripts {
                                let on = crate::rx_pipe::script_matches(current, script);
                                if ui
                                    .selectable_label(
                                        on,
                                        crate::rx_pipe::pipeline_label(&script.name),
                                    )
                                    .clicked()
                                    && !on
                                {
                                    tx_apply = Some((row.id, script.command.clone()));
                                }
                            }
                        });
                    if ui.small_button("Pipes…").clicked() {
                        pipe_editor_open = Some((row.id, true, row.spec.tx_pipe.clone()));
                    }
                    if !current.is_empty() && crate::rx_pipe::resolve_pipe(current).is_err() {
                        ui.label(egui::RichText::new("not runnable").small().color(caution()));
                    }
                });
                if !row.tx_pipe_status.is_empty() {
                    ui.label(
                        egui::RichText::new(&row.tx_pipe_status)
                            .color(caution()),
                    );
                }
                }
                ui.add_space(4.0);
            }
            self.paint_bay(ui, &view);
            let owned: Vec<String> = view
                .rows
                .iter()
                .flat_map(|row| [crate::audio::rx_name(row.id), crate::audio::tx_name(row.id)])
                .collect();
            let extras: Vec<_> = view
                .audio
                .devices
                .iter()
                .filter(|device| !owned.iter().any(|name| name == &device.name))
                .collect();
            if !extras.is_empty() {
                ui.label("Other hamspot devices");
                for device in extras {
                    ui.label(format!(
                        "{}  {}  {}",
                        device.name, device.class, device.description
                    ));
                }
            }
        });
        if let Some((id, input, output)) = endpoint {
            self.hub.set_endpoint(id, input, output);
        }
        if let Some((id, tx, value)) = pipe_editor_open {
            self.pipe_editor.open_for(id, tx, &value);
        }
        if let Some((id, command)) = pipe_apply {
            self.hub.set_rx_pipe(id, command.clone());
            if !self.config_path.is_empty() {
                self.rx_pipe_pending = Some((id, command));
            }
        }
        if let Some((id, command)) = tx_apply {
            self.hub.set_tx_pipe(id, command.clone());
            if !self.config_path.is_empty() {
                self.tx_pipe_pending = Some((id, command));
            }
        }
    }

    /// Point a radio's RX or TX pipe at a `modems/<name>` reference and save
    /// the station once the hub has it.
    fn apply_pipe(&mut self, id: u64, tx: bool, reference: String) {
        if tx {
            self.hub.set_tx_pipe(id, reference.clone());
            if !self.config_path.is_empty() {
                self.tx_pipe_pending = Some((id, reference));
            }
        } else {
            self.hub.set_rx_pipe(id, reference.clone());
            if !self.config_path.is_empty() {
                self.rx_pipe_pending = Some((id, reference));
            }
        }
    }

    /// The hub applies the command on its own thread. Write the station file
    /// only after that row shows the same text, so the save cannot race ahead.
    fn flush_rx_pipe(&mut self, snap: &Snapshot) {
        let Some((id, command)) = self.rx_pipe_pending.clone() else {
            return;
        };
        let view = self.hub.view();
        let Some(row) = view.rows.iter().find(|row| row.id == id) else {
            self.rx_pipe_pending = None;
            return;
        };
        if row.spec.rx_pipe != command {
            return;
        }
        self.rx_pipe_pending = None;
        if !self.config_path.is_empty() {
            self.save_config(snap);
        }
    }

    fn flush_tx_pipe(&mut self, snap: &Snapshot) {
        let Some((id, command)) = self.tx_pipe_pending.clone() else {
            return;
        };
        let view = self.hub.view();
        let Some(row) = view.rows.iter().find(|row| row.id == id) else {
            self.tx_pipe_pending = None;
            return;
        };
        if row.spec.tx_pipe != command {
            return;
        }
        self.tx_pipe_pending = None;
        if !self.config_path.is_empty() {
            self.save_config(snap);
        }
    }

    fn paint_bay(&mut self, ui: &mut egui::Ui, view: &cat::HubView) {
        self.ensure_bay_art(ui.ctx());
        let radios: Vec<bay::BayRadio> = view
            .rows
            .iter()
            .map(|row| bay::BayRadio {
                id: row.id,
                title: format!("{} {} {}", row.id, row.make, row.model_label),
                rx_title: crate::audio::describe(&row.make, &row.model_label, row.id, "RX"),
                tx_title: crate::audio::describe(&row.make, &row.model_label, row.id, "TX"),
                input: row.spec.audio.input.clone(),
                output: row.spec.audio.output.clone(),
                rx_pipe_live: row.rx_pipe_diverted,
                rx_pipe_label: pipe_block_label(&row.rx_chain, "RX pipe"),
                tx_pipe_live: row.tx_pipe_diverted,
                tx_pipe_label: pipe_block_label(&row.spec.tx_pipe, "TX pipe"),
                tap: row.spec.driver == cat::RadioDriver::Tap,
            })
            .collect();
        let bindings: Vec<bay::RadioBinding> = radios
            .iter()
            .map(|radio| bay::RadioBinding {
                id: radio.id,
                input: radio.input.clone(),
                output: radio.output.clone(),
            })
            .collect();
        ui.add_space(8.0);
        ui.label("Drag a wire from a microphone to the radio, or from the radio to a speaker.");
        if view
            .rows
            .iter()
            .any(|row| row.spec.driver == cat::RadioDriver::Tap)
        {
            ui.label(
                "An audio tap takes a discriminator or line input on the left and plays the demodulator on the right. Commander does not tune that radio.",
            );
        }
        let input = bay::BayIn {
            captures: &view.capture,
            playbacks: &view.playback,
            radios: &radios,
            links: &view.audio.links,
            ports: &view.connect_ports,
        };
        let width = ui.available_width().max(680.0);
        let scene = bay::layout(width, &input);
        for icon in &scene.icons {
            if icon.kind == bay::IconKind::Xdg {
                self.ensure_xdg(ui.ctx(), &icon.xdg);
            }
        }
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(scene.width, scene.height),
            egui::Sense::click_and_drag(),
        );
        let origin = rect.min;
        let pointer = response.interact_pointer_pos().map(|pos| pos - origin);
        if response.drag_started() {
            if let Some(pos) = pointer {
                self.bay_drag = bay::grab_at(&scene, pos.x, pos.y);
            }
        }
        let mut updates = Vec::new();
        if response.drag_stopped() {
            if let Some(drag) = self.bay_drag.take() {
                if let Some(pos) = pointer {
                    let inside = pos.x >= 0.0
                        && pos.y >= 0.0
                        && pos.x <= scene.width
                        && pos.y <= scene.height;
                    if inside
                        && !bay::over_rx_pipe(&scene, pos.x, pos.y)
                        && !bay::over_tx_pipe(&scene, pos.x, pos.y)
                    {
                        let target = bay::target_at(&scene, pos.x, pos.y);
                        updates = bay::endpoint_updates(&bindings, &drag.grab, target.as_ref());
                    }
                }
            }
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(22, 24, 28));
        let wire_color = egui::Color32::from_rgb(70, 140, 220);
        for wire in &scene.wires {
            paint_curve(&painter, origin, wire.from, wire.to, wire_color);
        }
        for wire in &scene.fixed {
            paint_curve(&painter, origin, wire.from, wire.to, wire_color);
        }
        if let Some(drag) = &self.bay_drag {
            if let Some(pos) = pointer {
                paint_curve(
                    &painter,
                    origin,
                    drag.anchor,
                    [pos.x, pos.y],
                    egui::Color32::from_rgb(160, 200, 255),
                );
            }
        }
        for icon in &scene.icons {
            let icon_rect = egui::Rect::from_min_size(
                origin + egui::vec2(icon.x, icon.y),
                egui::vec2(
                    if icon.kind == bay::IconKind::Pipe || icon.kind == bay::IconKind::TxPipe {
                        bay::PIPE_W
                    } else {
                        48.0
                    },
                    48.0,
                ),
            );
            let tex = match icon.kind {
                bay::IconKind::Mic => self.mic_tex.as_ref(),
                bay::IconKind::Speaker => self.speaker_tex.as_ref(),
                bay::IconKind::Radio => self.radio_tex.as_ref(),
                bay::IconKind::Xdg => self.xdg_tex.get(&icon.xdg).and_then(Option::as_ref),
                bay::IconKind::Pipe | bay::IconKind::TxPipe => self.scope_tex.as_ref(),
            };
            if let Some(tex) = tex {
                painter.image(
                    tex.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else if icon.kind == bay::IconKind::Pipe || icon.kind == bay::IconKind::TxPipe {
                painter.rect_filled(
                    icon_rect,
                    egui::CornerRadius::same(6),
                    egui::Color32::from_rgb(28, 32, 38),
                );
                painter.rect_stroke(
                    icon_rect,
                    egui::CornerRadius::same(6),
                    egui::Stroke::new(1.5, caution()),
                    egui::StrokeKind::Inside,
                );
            }
            painter.text(
                icon_rect.center_bottom() + egui::vec2(0.0, 2.0),
                egui::Align2::CENTER_TOP,
                &icon.label,
                egui::FontId::proportional(10.0),
                egui::Color32::from_gray(210),
            );
        }
        for port in &scene.ports {
            let at = origin + egui::vec2(port.at[0], port.at[1]);
            painter.circle_filled(at, bay::JACK + 3.0, egui::Color32::from_rgb(18, 36, 58));
            painter.circle_filled(at, bay::JACK, wire_color);
            let anchor = if port.label_right {
                (at + egui::vec2(8.0, 0.0), egui::Align2::LEFT_CENTER)
            } else {
                (at + egui::vec2(-8.0, 0.0), egui::Align2::RIGHT_CENTER)
            };
            painter.text(
                anchor.0,
                anchor.1,
                &port.label,
                egui::FontId::monospace(9.0),
                egui::Color32::from_gray(170),
            );
        }
        for update in updates {
            self.hub
                .set_endpoint(update.id, update.input, update.output);
        }
    }

    fn ensure_bay_art(&mut self, ctx: &egui::Context) {
        if self.speaker_tex.is_none() {
            self.speaker_tex = png_texture(
                ctx,
                "hamspot-speaker",
                include_bytes!("../assets/icon-speaker.png"),
            );
        }
        if self.mic_tex.is_none() {
            self.mic_tex =
                png_texture(ctx, "hamspot-mic", include_bytes!("../assets/icon-mic.png"));
        }
        if self.radio_tex.is_none() {
            self.radio_tex = png_texture(
                ctx,
                "hamspot-radio",
                include_bytes!("../assets/icon-radio.png"),
            );
        }
        if self.scope_tex.is_none() {
            self.scope_tex = png_texture(
                ctx,
                "hamspot-scope",
                include_bytes!("../assets/icon-scope.png"),
            );
        }
    }

    fn ensure_xdg(&mut self, ctx: &egui::Context, name: &str) {
        if self.xdg_tex.contains_key(name) {
            return;
        }
        let tex = xdg_icon_png(name).and_then(|bytes| png_texture(ctx, name, &bytes));
        self.xdg_tex.insert(name.to_string(), tex);
    }

    fn open_hamlib(&mut self, row: Option<&cat::RadioRow>) {
        self.models = cat::model_list().unwrap_or_default();
        let draft = if let Some(row) = row {
            CatDraft {
                id: Some(row.id),
                name: row.spec.name.clone(),
                model: row.spec.model,
                device: row.spec.device.clone(),
                serial_speed: row.spec.serial_speed.clone(),
                civ: row.spec.civ.clone(),
                ptt_type: row.spec.ptt_type.clone(),
                ptt_file: row.spec.ptt_file.clone(),
                conf: row.spec.conf.clone(),
                park: row.spec.park,
                port_label: row
                    .port
                    .map(|port| port.to_string())
                    .unwrap_or_else(|| "assigning".into()),
                error: String::new(),
                driver: cat::RadioDriver::Hamlib,
            }
        } else {
            CatDraft {
                id: None,
                name: "Radio".into(),
                model: 1,
                device: String::new(),
                serial_speed: String::new(),
                civ: String::new(),
                ptt_type: String::new(),
                ptt_file: String::new(),
                conf: String::new(),
                park: false,
                port_label: "assigning".into(),
                error: String::new(),
                driver: cat::RadioDriver::Hamlib,
            }
        };
        if self.models.is_empty() {
            let mut draft = draft;
            draft.error =
                "rigctld -l did not return a model list. The model number is still saved.".into();
            self.cat_edit = Some(draft);
        } else {
            self.cat_edit = Some(draft);
        }
    }

    fn open_gqrx(&mut self, row: Option<&cat::RadioRow>) {
        let draft = if let Some(row) = row {
            CatDraft {
                id: Some(row.id),
                name: row.spec.name.clone(),
                model: 0,
                device: String::new(),
                serial_speed: String::new(),
                civ: String::new(),
                ptt_type: String::new(),
                ptt_file: String::new(),
                conf: String::new(),
                park: false,
                port_label: row
                    .port
                    .map(|port| port.to_string())
                    .unwrap_or_else(|| "assigning".into()),
                error: String::new(),
                driver: cat::RadioDriver::Gqrx,
            }
        } else {
            CatDraft {
                id: None,
                name: "GQRX".into(),
                model: 0,
                device: String::new(),
                serial_speed: String::new(),
                civ: String::new(),
                ptt_type: String::new(),
                ptt_file: String::new(),
                conf: String::new(),
                park: false,
                port_label: "assigning".into(),
                error: String::new(),
                driver: cat::RadioDriver::Gqrx,
            }
        };
        self.cat_edit = Some(draft);
    }

    fn open_hackrf(&mut self, row: Option<&cat::RadioRow>) {
        let draft = if let Some(row) = row {
            CatDraft {
                id: Some(row.id),
                name: row.spec.name.clone(),
                model: 0,
                device: row.spec.device.clone(),
                serial_speed: String::new(),
                civ: String::new(),
                ptt_type: String::new(),
                ptt_file: String::new(),
                conf: String::new(),
                park: false,
                port_label: "direct".into(),
                error: String::new(),
                driver: cat::RadioDriver::Hackrf,
            }
        } else {
            CatDraft {
                id: None,
                name: "HackRF".into(),
                model: 0,
                device: String::new(),
                serial_speed: String::new(),
                civ: String::new(),
                ptt_type: String::new(),
                ptt_file: String::new(),
                conf: String::new(),
                park: false,
                port_label: "direct".into(),
                error: String::new(),
                driver: cat::RadioDriver::Hackrf,
            }
        };
        self.probe_hackrf_dongles();
        self.cat_edit = Some(draft);
    }

    fn open_tap(&mut self, row: Option<&cat::RadioRow>) {
        let draft = if let Some(row) = row {
            CatDraft {
                id: Some(row.id),
                name: row.spec.name.clone(),
                model: 0,
                device: String::new(),
                serial_speed: String::new(),
                civ: String::new(),
                ptt_type: String::new(),
                ptt_file: String::new(),
                conf: String::new(),
                park: false,
                port_label: "audio".into(),
                error: String::new(),
                driver: cat::RadioDriver::Tap,
            }
        } else {
            CatDraft {
                id: None,
                name: "Audio tap".into(),
                model: 0,
                device: String::new(),
                serial_speed: String::new(),
                civ: String::new(),
                ptt_type: String::new(),
                ptt_file: String::new(),
                conf: String::new(),
                park: false,
                port_label: "audio".into(),
                error: String::new(),
                driver: cat::RadioDriver::Tap,
            }
        };
        self.cat_edit = Some(draft);
    }

    fn tap_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut commit = false;
        egui::Window::new("Audio tap").open(&mut open).show(ctx, |ui| {
            let Some(edit) = &mut self.cat_edit else {
                return;
            };
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut edit.name);
            });
            ui.label(
                "This block replaces the radio. Wire a discriminator or line input to it, then choose a demodulator. The external radio stays on the frequency you set there.",
            );
            if !edit.error.is_empty() {
                ui.label(
                    egui::RichText::new(&edit.error).color(caution()),
                );
            }
            if ui.button("Save").clicked() {
                commit = true;
            }
        });
        if !open {
            self.cat_edit = None;
            return;
        }
        if commit {
            self.commit_hamlib();
        }
    }

    fn probe_hackrf_dongles(&mut self) {
        match crate::hackrf::list_dongles() {
            Ok(list) => {
                self.hackrf_seen = list;
                self.hackrf_probe_note.clear();
            }
            Err(err) => {
                self.hackrf_seen.clear();
                self.hackrf_probe_note = err;
            }
        }
    }

    fn hackrf_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut commit = false;
        let mut refresh = false;
        let seen = self.hackrf_seen.clone();
        let probe_note = self.hackrf_probe_note.clone();
        egui::Window::new("HackRF").open(&mut open).show(ctx, |ui| {
            let Some(edit) = &mut self.cat_edit else {
                return;
            };
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut edit.name);
            });
            ui.label("Connected");
            if ui
                .selectable_label(edit.device.trim().is_empty(), "only HackRF")
                .clicked()
            {
                edit.device.clear();
            }
            if seen.is_empty() {
                let note = if probe_note.is_empty() {
                    "No HackRF found.".to_string()
                } else {
                    probe_note.clone()
                };
                ui.label(egui::RichText::new(note).color(caution()));
            }
            for dongle in &seen {
                let label = crate::hackrf::dongle_label(dongle);
                if ui
                    .selectable_label(edit.device == dongle.serial, label)
                    .clicked()
                {
                    edit.device.clone_from(&dongle.serial);
                }
            }
            if ui.button("Refresh").clicked() {
                refresh = true;
            }
            ui.horizontal(|ui| {
                ui.label("Serial");
                ui.text_edit_singleline(&mut edit.device);
            });
            ui.label("An empty serial uses the only HackRF. Save stores the serial you pick.");
            ui.label("Listen  direct");
            if !edit.error.is_empty() {
                ui.label(
                    egui::RichText::new(&edit.error).color(caution()),
                );
            }
            if ui.button("Save").clicked() {
                commit = true;
            }
        });
        if !open {
            self.cat_edit = None;
            return;
        }
        if refresh {
            self.probe_hackrf_dongles();
        }
        if commit {
            self.commit_hamlib();
        }
    }

    fn gqrx_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut commit = false;
        egui::Window::new("GQRX").open(&mut open).show(ctx, |ui| {
            let Some(edit) = &mut self.cat_edit else {
                return;
            };
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut edit.name);
            });
            ui.label("Commander starts this GQRX and stops it on exit.");
            if edit.id.is_none() {
                ui.label("A new GQRX has no input device, so it opens the I/O chooser.");
            }
            ui.label(format!(
                "Config  {}",
                cat::gqrx_conf_for_name(&edit.name).display()
            ));
            ui.label(format!("Listen  127.0.0.1:{}", edit.port_label));
            if !edit.error.is_empty() {
                ui.label(
                    egui::RichText::new(&edit.error).color(caution()),
                );
            }
            if ui.button("Save").clicked() {
                commit = true;
            }
        });
        if !open {
            self.cat_edit = None;
            return;
        }
        if commit {
            self.commit_hamlib();
        }
    }

    fn hamlib_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut commit = false;
        let serial = self.hub.view().serial;
        egui::Window::new("Hamlib radio")
            .open(&mut open)
            .show(ctx, |ui| {
                let models = self.models.clone();
                let Some(edit) = &mut self.cat_edit else {
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut edit.name);
                });
                let current = models
                    .iter()
                    .find(|row| row.id == edit.model)
                    .map(cat::model_button_label)
                    .unwrap_or_else(|| edit.model.to_string());
                egui::ComboBox::from_id_salt("hamlib-model")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        // The header uses the same selectable as the rows so
                        // its left inset matches and the columns stay even.
                        let _ = ui.selectable_label(
                            false,
                            egui::RichText::new(cat::model_menu_header())
                                .monospace()
                                .strong(),
                        );
                        for row in &models {
                            let text = egui::RichText::new(cat::format_model_row(row)).monospace();
                            if ui.selectable_label(edit.model == row.id, text).clicked() {
                                edit.model = row.id;
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Model");
                    let mut text = edit.model.to_string();
                    if ui.text_edit_singleline(&mut text).changed() {
                        if let Ok(model) = text.trim().parse::<i32>() {
                            edit.model = model;
                        }
                    }
                });
                let device_label = if edit.device.trim().is_empty() {
                    "none".to_string()
                } else {
                    edit.device.clone()
                };
                egui::ComboBox::from_id_salt("hamlib-device")
                    .selected_text(device_label)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(edit.device.trim().is_empty(), "none")
                            .clicked()
                        {
                            edit.device.clear();
                        }
                        for path in &serial {
                            if ui.selectable_label(edit.device == *path, path).clicked() {
                                edit.device.clone_from(path);
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Device");
                    ui.text_edit_singleline(&mut edit.device);
                });
                ui.horizontal(|ui| {
                    ui.label("Serial speed");
                    ui.text_edit_singleline(&mut edit.serial_speed);
                });
                ui.horizontal(|ui| {
                    ui.label("CI-V");
                    ui.text_edit_singleline(&mut edit.civ);
                });
                ui.horizontal(|ui| {
                    ui.label("PTT type");
                    ui.text_edit_singleline(&mut edit.ptt_type);
                });
                ui.horizontal(|ui| {
                    ui.label("PTT device");
                    ui.text_edit_singleline(&mut edit.ptt_file);
                });
                ui.horizontal(|ui| {
                    ui.label("Extra -C");
                    ui.text_edit_singleline(&mut edit.conf);
                });
                ui.label(format!("Listen  127.0.0.1:{}", edit.port_label));
                if !edit.error.is_empty() {
                    ui.label(
                        egui::RichText::new(&edit.error)
                            .color(caution()),
                    );
                }
                if ui.button("Save").clicked() {
                    commit = true;
                }
            });
        if !open {
            self.cat_edit = None;
            return;
        }
        if commit {
            self.commit_hamlib();
        }
    }

    fn open_rotator(&mut self, row: Option<&rot::RotatorRow>) {
        self.rot_models = rot::model_list().unwrap_or_default();
        let draft = if let Some(row) = row {
            RotDraft {
                id: Some(row.id),
                name: row.spec.name.clone(),
                model: row.spec.model,
                device: row.spec.device.clone(),
                serial_speed: row.spec.serial_speed.clone(),
                conf: row.spec.conf.clone(),
                port_label: row
                    .port
                    .map(|port| port.to_string())
                    .unwrap_or_else(|| "assigning".into()),
                error: String::new(),
            }
        } else {
            RotDraft {
                id: None,
                name: "Rotator".into(),
                model: 1,
                device: String::new(),
                serial_speed: String::new(),
                conf: String::new(),
                port_label: "assigning".into(),
                error: String::new(),
            }
        };
        if self.rot_models.is_empty() {
            let mut draft = draft;
            draft.error =
                "rotctld -l did not return a model list. The model number is still saved.".into();
            self.rot_edit = Some(draft);
        } else {
            self.rot_edit = Some(draft);
        }
    }

    fn rotator_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut commit = false;
        let serial = self.hub.view().serial;
        egui::Window::new("Rotator")
            .open(&mut open)
            .show(ctx, |ui| {
                let models = self.rot_models.clone();
                let Some(edit) = &mut self.rot_edit else {
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut edit.name);
                });
                let current = models
                    .iter()
                    .find(|row| row.id == edit.model)
                    .map(rot::model_button_label)
                    .unwrap_or_else(|| edit.model.to_string());
                egui::ComboBox::from_id_salt("rotator-model")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        let _ = ui.selectable_label(
                            false,
                            egui::RichText::new(rot::model_menu_header())
                                .monospace()
                                .strong(),
                        );
                        for row in &models {
                            let text = egui::RichText::new(rot::format_model_row(row)).monospace();
                            if ui.selectable_label(edit.model == row.id, text).clicked() {
                                edit.model = row.id;
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Model");
                    let mut text = edit.model.to_string();
                    if ui.text_edit_singleline(&mut text).changed() {
                        if let Ok(model) = text.trim().parse::<i32>() {
                            edit.model = model;
                        }
                    }
                });
                let device_label = if edit.device.trim().is_empty() {
                    "none".to_string()
                } else {
                    edit.device.clone()
                };
                egui::ComboBox::from_id_salt("rotator-device")
                    .selected_text(device_label)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(edit.device.trim().is_empty(), "none")
                            .clicked()
                        {
                            edit.device.clear();
                        }
                        for path in &serial {
                            if ui.selectable_label(edit.device == *path, path).clicked() {
                                edit.device.clone_from(path);
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Device");
                    ui.text_edit_singleline(&mut edit.device);
                });
                ui.horizontal(|ui| {
                    ui.label("Serial speed");
                    ui.text_edit_singleline(&mut edit.serial_speed);
                });
                ui.horizontal(|ui| {
                    ui.label("Extra -C");
                    ui.text_edit_singleline(&mut edit.conf);
                });
                ui.label(format!("Listen  127.0.0.1:{}", edit.port_label));
                ui.label("In Gpredict, set the rotator interface to that host and port.");
                if !edit.error.is_empty() {
                    ui.label(
                        egui::RichText::new(&edit.error)
                            .color(caution()),
                    );
                }
                if ui.button("Save").clicked() {
                    commit = true;
                }
            });
        if !open {
            self.rot_edit = None;
            return;
        }
        if commit {
            self.commit_rotator();
        }
    }

    fn commit_rotator(&mut self) {
        let Some(edit) = self.rot_edit.clone() else {
            return;
        };
        let spec = rot::RotatorSpec {
            name: edit.name.trim().to_string(),
            model: edit.model,
            device: edit.device.trim().to_string(),
            serial_speed: edit.serial_speed.trim().to_string(),
            conf: edit.conf.trim().to_string(),
        };
        if let Err(err) = rot::validate_spec(&spec) {
            if let Some(edit) = &mut self.rot_edit {
                edit.error = err;
            }
            return;
        }
        match edit.id {
            Some(id) => self.rotators.update(id, spec),
            None => self.rotators.add(spec),
        }
        self.rot_edit = None;
    }

    /// Amplifiers grid and the selected amplifier's keys, under Rotators.
    /// Power keys send only on a click. A load or a restart sends none.
    fn amplifier_rows(&mut self, ui: &mut egui::Ui) {
        let amps = self.amplifiers.view();
        let mut select = None;
        ui.label("Amplifiers");
        ui.add_space(4.0);
        egui::Grid::new("amplifiers")
            .striped(true)
            .min_col_width(64.0)
            .show(ui, |ui| {
                for head in ["Name", "Model", "Device", "Listen", "Freq", "Power", "SWR", "Fwd", "Status"] {
                    ui.label(head);
                }
                ui.end_row();
                for row in &amps.rows {
                    let model = if row.make.is_empty() && row.model_label.is_empty() {
                        row.spec.model.to_string()
                    } else {
                        format!("{}  {} {}", row.spec.model, row.make, row.model_label)
                    };
                    let device = if row.spec.device.trim().is_empty() {
                        "—".to_string()
                    } else {
                        row.spec.device.clone()
                    };
                    let listen = match row.port {
                        Some(port) => format!("127.0.0.1:{port}"),
                        None => "127.0.0.1".to_string(),
                    };
                    let freq = row
                        .state
                        .freq_hz
                        .map(|hz| format!("{:.3} MHz", hz / 1e6))
                        .unwrap_or_else(|| "—".into());
                    let power = row
                        .state
                        .power
                        .map(|p| p.label().to_string())
                        .unwrap_or_else(|| "—".into());
                    let level = |name: &str| {
                        row.state
                            .level(name)
                            .map(amp::format_level)
                            .unwrap_or_else(|| "—".into())
                    };
                    let cells = [
                        row.spec.name.clone(),
                        model,
                        device,
                        listen,
                        freq,
                        power,
                        level("SWR"),
                        level("PWRFORWARD"),
                        row.status.clone(),
                    ];
                    for cell in &cells {
                        if cat_cell(ui, cell, row.selected) {
                            select = Some(row.id);
                        }
                    }
                    ui.end_row();
                }
            });
        ui.add_space(6.0);
        let selected = amps.rows.iter().find(|row| row.selected);
        ui.horizontal(|ui| {
            if ui.button("Add").clicked() {
                self.open_amplifier(None);
            }
            if ui.add_enabled(selected.is_some(), egui::Button::new("Edit")).clicked() {
                if let Some(row) = selected {
                    self.open_amplifier(Some(row));
                }
            }
            if ui.add_enabled(selected.is_some(), egui::Button::new("Remove")).clicked() {
                if let Some(row) = selected {
                    self.amplifiers.remove(row.id);
                }
            }
        });
        let live = selected.filter(|row| row.port.is_some());
        ui.horizontal(|ui| {
            let keys = [
                ("Standby", amp::Control::SetPower(amp::Power::Standby)),
                ("Operate", amp::Control::SetPower(amp::Power::Operate)),
                ("Off", amp::Control::SetPower(amp::Power::Off)),
                ("Reset", amp::Control::Reset),
            ];
            for (label, cmd) in keys {
                let text = if label == "Operate" {
                    egui::RichText::new(label).color(caution())
                } else {
                    egui::RichText::new(label)
                };
                if ui.add_enabled(live.is_some(), egui::Button::new(text)).clicked() {
                    if let Some(row) = live {
                        self.send_amp(&row.spec.id, cmd);
                    }
                }
            }
        });
        if let Some(row) = live {
            self.amp_set_controls(ui, row);
        }
        if let Some(row) = selected {
            if !row.state.levels.is_empty() {
                let text = row
                    .state
                    .levels
                    .iter()
                    .map(|(name, value)| format!("{name} {}", amp::format_level(value)))
                    .collect::<Vec<_>>()
                    .join("  ·  ");
                ui.label(egui::RichText::new(text).monospace());
            }
        }
        ui.label("Commander never sends Operate on its own. Test with the Dummy (model 1).");
        if let Some(id) = select {
            self.amplifiers.select(id);
        }
    }

    /// Send one command to the amplifier with this fixed id, looked up now.
    /// A missing or ambiguous id sends nothing (#96).
    fn send_amp(&self, amp_id: &str, cmd: amp::Control) -> bool {
        let rows = self.amplifiers.view().rows;
        match patchbay::bind::resolve(amp_id, &rows) {
            Some(row) => {
                self.amplifiers.control(row.id, cmd);
                true
            }
            None => false,
        }
    }

    /// Mirror the active rig's frequency to amplifiers with "follow rig
    /// frequency" ticked (#103). Opt-in per amplifier, debounced, and only
    /// `set_freq`: never power or levels.
    fn follow_amplifiers(&mut self, snap: &Snapshot, ctx: &egui::Context) {
        let rig_hz = snap.online().then_some(snap.freq_hz);
        let now = std::time::Instant::now();
        let rows = self.amplifiers.view().rows;
        self.amp_follow.retain(|id, _| rows.iter().any(|r| r.id == *id));
        let mut pending = false;
        for row in &rows {
            let f = self.amp_follow.entry(row.id).or_default();
            let on = row.spec.follow_rig && row.port.is_some();
            if let Some(hz) = f.step(on, rig_hz, now) {
                self.send_amp(&row.spec.id, amp::Control::SetFreq(hz));
            }
            pending |= on;
        }
        if pending {
            ctx.request_repaint_after(amp::FOLLOW_SETTLE);
        }
    }

    /// Frequency entry, follow-rig opt-in and settable levels for the
    /// selected live amplifier (#103). Each sends only on its own click.
    fn amp_set_controls(&mut self, ui: &mut egui::Ui, row: &amp::AmplifierRow) {
        ui.horizontal(|ui| {
            ui.label("Freq MHz");
            ui.add(egui::TextEdit::singleline(&mut self.amp_freq_text).hint_text("14.200").desired_width(90.0));
            let hz = amp::parse_mhz(&self.amp_freq_text);
            if ui.add_enabled(hz.is_some(), egui::Button::new("Set")).clicked() {
                if let Some(hz) = hz {
                    self.send_amp(&row.spec.id, amp::Control::SetFreq(hz));
                }
            }
            let follow = !row.spec.follow_rig;
            if crate::toggle::add_toggle(ui, true, row.spec.follow_rig, "Follow rig frequency")
                .on_hover_text("Sends the active rig's frequency to this amplifier (set_freq only) after it settles. Saved per amplifier.")
                .clicked()
            {
                self.amplifiers.update(row.id, amp::AmplifierSpec { follow_rig: follow, ..row.spec.clone() });
            }
        });
        if row.settable.is_empty() {
            ui.label("No settable levels reported (set_level ?). The readouts are meters.");
            return;
        }
        for name in &row.settable {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(name).monospace());
                let raw = row.state.level(name).and_then(|v| v.trim().parse::<f64>().ok());
                let current = amp::step_from_level(raw.unwrap_or(0.0));
                let value = self.amp_level_vals.entry(format!("{}/{name}", row.spec.id)).or_insert(current);
                ui.add(egui::Slider::new(value, 1..=10).text("1–10")).on_hover_text(format!(
                    "Sends {:.1} to Hamlib (step ÷ 10). Amplifier reads {}.",
                    amp::level_from_step(*value),
                    raw.map(|v| format!("{v:.3} (step {current})")).unwrap_or_else(|| "—".into())
                ));
                let send = amp::level_from_step(*value);
                if ui.button("Set").clicked() {
                    self.send_amp(&row.spec.id, amp::Control::SetLevel(name.clone(), format!("{send:.1}")));
                }
            });
        }
    }

    fn open_amplifier(&mut self, row: Option<&amp::AmplifierRow>) {
        self.amp_models = amp::model_list();
        self.amp_edit = Some(match row {
            Some(row) => AmpDraft {
                id: Some(row.id),
                name: row.spec.name.clone(),
                model: row.spec.model,
                device: row.spec.device.clone(),
                serial_speed: row.spec.serial_speed.clone(),
                conf: row.spec.conf.clone(),
                port_label: row
                    .port
                    .map(|port| port.to_string())
                    .unwrap_or_else(|| "assigning".into()),
                error: String::new(),
            },
            None => AmpDraft {
                id: None,
                name: "Amplifier".into(),
                model: 1,
                device: String::new(),
                serial_speed: String::new(),
                conf: String::new(),
                port_label: "assigning".into(),
                error: String::new(),
            },
        });
    }

    fn amplifier_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut commit = false;
        let serial = self.hub.view().serial;
        egui::Window::new("Amplifier").open(&mut open).show(ctx, |ui| {
            let models = self.amp_models.clone();
            let Some(edit) = &mut self.amp_edit else {
                return;
            };
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut edit.name);
            });
            let current = models
                .iter()
                .find(|row| row.id == edit.model)
                .map(amp::model_button_label)
                .unwrap_or_else(|| edit.model.to_string());
            egui::ComboBox::from_id_salt("amplifier-model")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    let _ = ui.selectable_label(
                        false,
                        egui::RichText::new(amp::model_menu_header()).monospace().strong(),
                    );
                    for row in &models {
                        let text = egui::RichText::new(amp::format_model_row(row)).monospace();
                        if ui.selectable_label(edit.model == row.id, text).clicked() {
                            edit.model = row.id;
                        }
                    }
                });
            ui.horizontal(|ui| {
                ui.label("Model");
                let mut text = edit.model.to_string();
                if ui.text_edit_singleline(&mut text).changed() {
                    if let Ok(model) = text.trim().parse::<i32>() {
                        edit.model = model;
                    }
                }
            });
            let device_label = if edit.device.trim().is_empty() {
                "none".to_string()
            } else {
                edit.device.clone()
            };
            egui::ComboBox::from_id_salt("amplifier-device")
                .selected_text(device_label)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(edit.device.trim().is_empty(), "none").clicked() {
                        edit.device.clear();
                    }
                    for path in &serial {
                        if ui.selectable_label(edit.device == *path, path).clicked() {
                            edit.device.clone_from(path);
                        }
                    }
                });
            ui.horizontal(|ui| {
                ui.label("Device");
                ui.text_edit_singleline(&mut edit.device);
            });
            ui.horizontal(|ui| {
                ui.label("Serial speed");
                ui.text_edit_singleline(&mut edit.serial_speed);
            });
            ui.horizontal(|ui| {
                ui.label("Extra -C");
                ui.text_edit_singleline(&mut edit.conf);
            });
            ui.label(format!("Listen  127.0.0.1:{}", edit.port_label));
            if !edit.error.is_empty() {
                ui.label(egui::RichText::new(&edit.error).color(caution()));
            }
            if ui.button("Save").clicked() {
                commit = true;
            }
        });
        if !open {
            self.amp_edit = None;
            return;
        }
        if commit {
            self.commit_amplifier();
        }
    }

    fn commit_amplifier(&mut self) {
        let Some(edit) = self.amp_edit.clone() else {
            return;
        };
        let old = edit
            .id
            .and_then(|id| self.amplifiers.view().rows.into_iter().find(|r| r.id == id))
            .map(|r| r.spec);
        let spec = amp::AmplifierSpec {
            // Kept across edits; a new row gets a fresh random id.
            id: old.as_ref().map(|s| s.id.clone()).filter(|id| !id.is_empty()).unwrap_or_else(amp::new_id),
            name: edit.name.trim().to_string(),
            model: edit.model,
            device: edit.device.trim().to_string(),
            serial_speed: edit.serial_speed.trim().to_string(),
            conf: edit.conf.trim().to_string(),
            follow_rig: old.as_ref().is_some_and(|s| s.follow_rig),
        };
        if let Err(err) = amp::validate_spec(&spec) {
            if let Some(edit) = &mut self.amp_edit {
                edit.error = err;
            }
            return;
        }
        match edit.id {
            Some(id) => self.amplifiers.update(id, spec),
            None => self.amplifiers.add(spec),
        }
        self.amp_edit = None;
    }

    fn commit_hamlib(&mut self) {
        let Some(edit) = self.cat_edit.clone() else {
            return;
        };
        let spec = cat::RadioSpec {
            driver: edit.driver.clone(),
            name: edit.name.trim().to_string(),
            model: edit.model,
            device: edit.device.trim().to_string(),
            serial_speed: edit.serial_speed.trim().to_string(),
            civ: edit.civ.trim().to_string(),
            ptt_type: edit.ptt_type.trim().to_string(),
            ptt_file: edit.ptt_file.trim().to_string(),
            conf: edit.conf.trim().to_string(),
            park: edit.park,
            follow: edit
                .id
                .and_then(|id| {
                    self.hub
                        .view()
                        .rows
                        .iter()
                        .find(|row| row.id == id)
                        .map(|row| row.spec.follow)
                })
                .unwrap_or(false),
            audio: {
                let mut audio = edit
                    .id
                    .and_then(|id| {
                        self.hub
                            .view()
                            .rows
                            .iter()
                            .find(|row| row.id == id)
                            .map(|row| row.spec.audio.clone())
                    })
                    .unwrap_or_default();
                if edit.driver == cat::RadioDriver::Tap {
                    audio.rx = false;
                    audio.tx = false;
                }
                audio
            },
            gqrx: match edit.driver {
                cat::RadioDriver::Gqrx if edit.id.is_none() => cat::gqrx_chooser_config(),
                cat::RadioDriver::Gqrx => edit
                    .id
                    .and_then(|id| {
                        self.hub
                            .view()
                            .rows
                            .iter()
                            .find(|row| row.id == id)
                            .map(|row| row.spec.gqrx.clone())
                    })
                    .unwrap_or_else(cat::gqrx_chooser_config),
                cat::RadioDriver::Hamlib | cat::RadioDriver::Hackrf | cat::RadioDriver::Tap => {
                    cat::GqrxConfig::new()
                }
            },
            rx_pipe: edit
                .id
                .and_then(|id| {
                    self.hub
                        .view()
                        .rows
                        .iter()
                        .find(|row| row.id == id)
                        .map(|row| row.spec.rx_pipe.clone())
                })
                .unwrap_or_default(),
            tx_pipe: edit
                .id
                .and_then(|id| {
                    self.hub
                        .view()
                        .rows
                        .iter()
                        .find(|row| row.id == id)
                        .map(|row| row.spec.tx_pipe.clone())
                })
                .unwrap_or_default(),
            sample_rate: edit
                .id
                .and_then(|id| {
                    self.hub
                        .view()
                        .rows
                        .iter()
                        .find(|row| row.id == id)
                        .map(|row| row.spec.sample_rate)
                })
                .unwrap_or(crate::hackrf::SAMPLE_RATE),
            lna_db: edit.id.and_then(|id| {
                self.hackrf_gain_hold
                    .as_ref()
                    .filter(|hold| hold.id == id)
                    .map(|hold| hold.lna)
                    .or_else(|| {
                        self.hub
                            .view()
                            .rows
                            .iter()
                            .find(|row| row.id == id)
                            .and_then(|row| row.spec.lna_db)
                    })
            }),
            vga_db: edit.id.and_then(|id| {
                self.hackrf_gain_hold
                    .as_ref()
                    .filter(|hold| hold.id == id)
                    .map(|hold| hold.vga)
                    .or_else(|| {
                        self.hub
                            .view()
                            .rows
                            .iter()
                            .find(|row| row.id == id)
                            .and_then(|row| row.spec.vga_db)
                    })
            }),
        };
        if let Err(err) = cat::validate_spec(&spec) {
            if let Some(edit) = &mut self.cat_edit {
                edit.error = err;
            }
            return;
        }
        match edit.id {
            Some(id) => self.hub.update(id, spec),
            None => self.hub.add(spec),
        }
        self.cat_edit = None;
    }

    /// Send a loaded Tune face only after the drive loop has connected to the
    /// radio this open started. The previous session can still be answering
    /// on the same port number until that happens.
    fn maybe_push_loaded(&mut self, snap: &Snapshot, live_gen: u64) {
        let Some(pending) = &self.pending_tune else {
            return;
        };
        let view = self.hub.view();
        let Some(row) = view.rows.iter().find(|row| row.selected) else {
            return;
        };
        if row.spec.driver == cat::RadioDriver::Hackrf {
            if view.list_gen <= pending.after_list {
                return;
            }
            let hz = pending.tune.freq_hz;
            let id = row.id;
            self.pending_tune = None;
            if hz.is_finite() && hz > 0.0 {
                self.hold_freq = Some(hz);
                self.freq_text = format_hz(hz);
                self.hub.hackrf_dial(id, hz);
            }
            return;
        }
        if live_gen <= pending.after_gen {
            return;
        }
        let Some(port) = row.port else {
            return;
        };
        let marker = format!(":{port}");
        if snap.kind != LinkKind::Live || !snap.endpoint.ends_with(&marker) {
            return;
        }
        let tune = pending.tune.clone();
        let levels = pending.levels.clone();
        self.pending_tune = None;
        self.push_radio(snap, &tune, &levels);
    }

    /// The face still shows the frequency of the rig this HackRF replaced.
    /// One dial send starts capture there. Follow keeps the watch center.
    fn seed_hackrf_dial(&mut self, snap: &Snapshot) {
        if snap.model != "HackRF" || !snap.online() {
            return;
        }
        let view = self.hub.view();
        let Some(row) = view.rows.iter().find(|row| {
            row.selected && row.spec.driver == cat::RadioDriver::Hackrf
        }) else {
            return;
        };
        if row.center_hz > 0.0 || row.spec.follow || self.following_control() {
            return;
        }
        if !self.hackrf_dial_seeded.insert(row.id) {
            return;
        }
        let hz = snap.freq_hz;
        if !hz.is_finite() || hz < 1_000_000.0 {
            self.hackrf_dial_seeded.remove(&row.id);
            return;
        }
        self.hold_freq = Some(hz);
        self.freq_text = format_hz(hz);
        self.hub.hackrf_dial(row.id, hz);
    }

    fn browse_config(&mut self, save: bool, snap: &Snapshot) {
        let dir = remembered_dir(&self.config_dir);
        let dialog = rfd::FileDialog::new()
            .set_title(if save { "Save config" } else { "Load config" })
            .add_filter("Hamspot Commander", &["HSCommander", "hscommander", "json"])
            .set_directory(&dir);
        let picked = if save {
            let name = if self.config_path.is_empty() {
                config::DEFAULT_STATION_NAME.to_string()
            } else {
                file_label(&self.config_path)
            };
            dialog.set_file_name(name).save_file()
        } else {
            dialog.pick_file()
        };
        let Some(path) = picked else {
            return;
        };
        let path = if save {
            config::with_station_extension(&path)
        } else {
            path
        };
        if let Some(parent) = path.parent() {
            if parent.is_dir() {
                self.config_dir = Some(parent.to_path_buf());
            }
        }
        self.config_path = path.display().to_string();
        if save {
            self.save_config(snap);
        } else {
            self.load_config(snap);
        }
    }

    fn load_config(&mut self, snap: &Snapshot) {
        match config::load_file_in(Path::new(&self.config_path), &crate::rx_pipe::modem_dirs()) {
            Ok((mut file, pipe_warnings)) => {
                if !pipe_warnings.is_empty() {
                    self.config_note = pipe_warnings.join("\n");
                }
                file.scan_zones.sort_unstable();
                let station_path = Path::new(&self.config_path);
                match crate::tools::nanovna::load_beside(station_path, &mut file.nanovna) {
                    Ok(loaded) => {
                        if let Some(note) = &loaded.note {
                            self.config_note = note.clone();
                        }
                        if loaded.stripped_station {
                            if let Err(err) = config::save_file(station_path, &file) {
                                self.config_note = err;
                            }
                        }
                        self.station_saved = Some(file.clone());
                        self.apply_config(snap, file);
                        self.tools
                            .lock()
                            .expect("tools")
                            .set_vna_archive(loaded.archive);
                    }
                    Err(err) => {
                        self.config_note = err;
                        self.station_saved = Some(file.clone());
                        self.apply_config(snap, file);
                        self.tools.lock().expect("tools").mark_vna_dirty();
                    }
                }
            }
            Err(err) => self.config_note = err,
        }
        self.load_keys();
    }

    fn load_keys(&mut self) {
        self.key_shown = None;
        if self.config_path.is_empty() {
            self.keys = keys::empty();
            self.sync_key_csv();
            return;
        }
        match keys::load_file(&keys::path_beside(Path::new(&self.config_path))) {
            Ok(file) => {
                self.keys = file;
                self.key_note.clear();
            }
            Err(err) => {
                self.keys = keys::empty();
                self.key_note = err;
            }
        }
        if self.key_sel >= self.keys.keys.len() {
            self.key_sel = 0;
        }
        self.sync_key_csv();
    }

    fn save_keys(&mut self) {
        if self.config_path.is_empty() {
            self.key_note = "The station has not been saved.".into();
            self.sync_key_csv();
            return;
        }
        let path = keys::path_beside(Path::new(&self.config_path));
        match keys::save_file(&path, &self.keys) {
            Ok(()) => self.key_note.clear(),
            Err(err) => self.key_note = err,
        }
        self.sync_key_csv();
    }

    /// Hand the private CSV to every radio. A repeat of the same path on the
    /// same radio list does not send again.
    fn sync_key_csv(&mut self) {
        let path = match keys::publish_runtime(&self.keys) {
            Ok(path) => path,
            Err(err) => {
                self.key_note = err;
                return;
            }
        };
        let gen = self.hub.view().list_gen;
        if self.key_runtime == path && self.key_runtime_gen == gen {
            return;
        }
        self.key_runtime = path.clone();
        self.key_runtime_gen = gen;
        self.hub.set_key_csv(&path);
    }

    /// A key id printed by the decoder becomes a placeholder. A row that is
    /// already there is left alone.
    fn note_asked_keys(&mut self, lines: &[String]) {
        let mut added = false;
        for line in lines {
            let plain = follow::plain_line(line);
            if let Some((alg, kid)) = follow::key_on_line(plain.trim()) {
                if keys::note_missing(&mut self.keys, alg, kid) {
                    added = true;
                }
            }
        }
        if added {
            self.key_shown = None;
            self.save_keys();
        }
    }

    fn save_config(&mut self, snap: &Snapshot) {
        let file = match self.capture_config(snap) {
            Ok(file) => file,
            Err(err) => {
                self.config_note = err;
                return;
            }
        };
        match config::save_file(Path::new(&self.config_path), &file) {
            Ok(()) => {
                self.audio_saved = crate::audio::canonical(&file.audio);
                self.audio_hold = false;
                self.unit_desc_dirty = false;
                self.config_note.clear();
                self.station_saved = Some(file);
            }
            Err(err) => self.config_note = err,
        }
        self.save_vna();
    }

    /// Calibration and zone scans. The station file keeps the port and the
    /// sweep settings. This file is `Name.HSVNA` beside that station.
    fn save_vna(&mut self) {
        if self.config_path.is_empty() {
            return;
        }
        let archive = self.tools.lock().expect("tools").vna_archive();
        let path = crate::tools::nanovna::path_beside(Path::new(&self.config_path));
        if let Err(err) = crate::tools::nanovna::save_file(&path, &archive) {
            self.config_note = err;
            self.tools.lock().expect("tools").mark_vna_dirty();
        }
    }

    /// True when the face differs from the station last loaded or written.
    /// No saved copy means the open face has not been written yet.
    fn station_changed(&mut self, snap: &Snapshot) -> Result<bool, String> {
        let now = self.capture_config(snap)?;
        Ok(self.station_saved.as_ref() != Some(&now))
    }

    fn consider_close(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        if ctx.input(|input| input.viewport().close_requested()) {
            if self.exit_discard {
                return;
            }
            if self.exit_prompt {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else {
                match self.station_changed(snap) {
                    Ok(false) => {}
                    Ok(true) => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                        self.exit_prompt = true;
                    }
                    Err(err) => {
                        self.config_note = err;
                        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                        self.exit_prompt = true;
                    }
                }
            }
        }
        if self.exit_prompt {
            self.exit_dialog(ctx, snap);
        }
    }

    fn exit_dialog(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        let name = if self.config_path.is_empty() {
            "this station".to_string()
        } else {
            file_label(&self.config_path)
        };
        let note = self.config_note.clone();
        let mut choice = None;
        let response = egui::Modal::new(egui::Id::unique("station-exit")).show(ctx, |ui| {
            ui.set_min_width(320.0);
            ui.label(format!("Save changes to {name}?"));
            if !note.is_empty() {
                ui.label(
                    egui::RichText::new(&note).color(caution()),
                );
            }
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(true);
                }
                if ui.button("Don't save").clicked() {
                    choice = Some(false);
                }
                if ui.button("Cancel").clicked() {
                    choice = None;
                    self.exit_prompt = false;
                }
            });
        });
        if self.exit_prompt && response.should_close() {
            self.exit_prompt = false;
            return;
        }
        match choice {
            Some(true) => self.save_and_close(ctx, snap),
            Some(false) => self.discard_and_close(ctx),
            None => {}
        }
    }

    fn save_and_close(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        if self.config_path.is_empty() {
            self.browse_config(true, snap);
        } else {
            self.save_config(snap);
        }
        let saved = self.station_changed(snap).ok() == Some(false);
        if saved {
            self.exit_prompt = false;
            self.exit_discard = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn discard_and_close(&mut self, ctx: &egui::Context) {
        self.exit_prompt = false;
        self.exit_discard = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// PipeWire edits land in the open station file. No open file means there
    /// is nowhere to write, and a load waits until the saved cards are back.
    fn autosave_audio(&mut self, snap: &Snapshot) {
        if self.config_path.is_empty() {
            return;
        }
        let view = self.hub.view();
        if !view.audio_ready {
            return;
        }
        let canon = crate::audio::canonical(&view.audio);
        if canon == self.audio_saved {
            self.audio_hold = false;
            return;
        }
        self.audio_hold = false;
        self.save_config(snap);
    }

    /// The open plan, channel file, Tune face, and settable levels.
    /// PTT is not included. This does not write `bsp/` and does not send CAT.
    fn capture_config(&mut self, snap: &Snapshot) -> Result<config::ConfigFile, String> {
        let bsp_name = config::bsp_name_from_path(&self.plan_path)?;
        let names: Vec<String> = snap.caps.set_levels.keys().cloned().collect();
        let mut levels = BTreeMap::new();
        for name in names {
            if snap.caps.level_range(&name).is_none() {
                continue;
            }
            levels.insert(name.clone(), self.shown_level(&name, snap));
        }
        let split = !snap.tx_vfo.is_empty() && !snap.tx_vfo.eq_ignore_ascii_case(&snap.vfo);
        let radio_count = self.hub.view().rows.len();
        self.trunk_receivers.retain(|entry| entry.radio < radio_count);
        self.trunk_receivers.sort_by_key(|entry| entry.radio);
        self.trunk_receivers.dedup_by_key(|entry| entry.radio);
        Ok(config::ConfigFile {
            version: 1,
            bsp_name,
            bsp: self.plan.clone(),
            channels: self.channels.clone(),
            talkgroups: self.talkgroups.clone(),
            units: self.units.clone(),
            tune: config::Tune {
                show_spread: self.show_spread,
                step_hz: self.step_hz,
                zone_sel: self.zone_sel,
                band_sel: self.band_sel,
                band_mem: self
                    .band_mem
                    .iter()
                    .map(|(name, hz)| (name.clone(), *hz))
                    .collect(),
                favorite: self.favorite.as_ref().map(|fav| config::FavoriteSlot {
                    hz: fav.hz,
                    mode: fav.mode.clone(),
                    passband_hz: fav.passband_hz,
                }),
                host: self.host.clone(),
                port: self.port.clone(),
                freq_hz: self.shown_freq(snap),
                mode: self.mode_hold.clone().unwrap_or_else(|| snap.mode.clone()),
                passband_hz: self.hold_pass.unwrap_or(snap.passband_hz),
                sub_freq_hz: self.shown_sub_freq(snap),
                sub_mode: self
                    .sub_mode_hold
                    .clone()
                    .unwrap_or_else(|| snap.sub_mode.clone()),
                rit_hz: snap.rit_hz,
                split,
                mem_hz: self.local_mem,
            },
            levels,
            radios: self
                .hub
                .view()
                .rows
                .into_iter()
                .map(|row| {
                    let mut spec = row.spec;
                    if let Some(hold) = &self.hackrf_gain_hold {
                        if hold.id == row.id {
                            spec.lna_db = Some(hold.lna);
                            spec.vga_db = Some(hold.vga);
                        }
                    }
                    spec
                })
                .collect(),
            audio: self.hub.view().audio.clone(),
            rotators: self
                .rotators
                .view()
                .rows
                .into_iter()
                .map(|row| row.spec)
                .collect(),
            amplifiers: self
                .amplifiers
                .view()
                .rows
                .into_iter()
                .map(|row| row.spec)
                .collect(),
            selected: self
                .hub
                .view()
                .rows
                .iter()
                .position(|row| row.selected)
                .unwrap_or(0),
            page: page_name(self.page),
            tabs: self.tab_order.clone(),
            channel_zone: self.channel_zone,
            channel_sel: self.channel_sel,
            talkgroup_zone: self.talkgroup_zone,
            talkgroup_sel: self.talkgroup_sel,
            unit_sel: self.unit_sel,
            trunk_receivers: self.trunk_receivers.clone(),
            control_follow: match &self.pending_control {
                Some((_, flags)) => flags
                    .iter()
                    .enumerate()
                    .filter(|(_, on)| **on)
                    .map(|(index, _)| index)
                    .collect(),
                None => self
                    .hub
                    .view()
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| self.cc_on.get(&row.id).copied().unwrap_or(false))
                    .map(|(index, _)| index)
                    .collect(),
            },
            scan_zones: self.scan_zones.iter().copied().collect(),
            monitor_zones: self.monitor_zones.iter().copied().collect(),
            monitor_talkgroups: self.monitor_talkgroups.iter().copied().collect(),
            meters: self.meters_face(),
            nanovna: self.tools.lock().expect("tools").nanovna_face(),
            pulse: self.pulse_on,
            pulse_secs: self.pulse_secs,
            voice_hang_ms: self.voice_hang_ms,
            override_hold_emergency_priority: self.override_hold,
            patchbay: self.patchbay.clone(),
            rec: self.rec.face.clone(),
        })
    }

    fn meters_face(&self) -> config::MetersFace {
        let meters = self.meters.lock().expect("meters");
        let (x, y) = match meters.saved_pos {
            Some(pos) => (Some(pos.x), Some(pos.y)),
            None => (None, None),
        };
        let (width, height) = match meters.saved_size {
            Some(size) => (Some(size.x), Some(size.y)),
            None => (None, None),
        };
        let (monitor_width, monitor_height) = match meters.saved_monitor {
            Some(size) => (Some(size.x), Some(size.y)),
            None => (None, None),
        };
        config::MetersFace {
            locked: meters.locked,
            detached: meters.detached,
            order: meters.order.clone(),
            window: config::WindowPlace {
                x,
                y,
                width,
                height,
                monitor_width,
                monitor_height,
                pixels_per_point: meters.saved_ppp,
            },
        }
    }

    fn apply_config(&mut self, _snap: &Snapshot, file: config::ConfigFile) {
        self.gain_drag_lna = None;
        self.gain_drag_vga = None;
        self.hackrf_gain_hold = None;
        let config::ConfigFile {
            bsp_name,
            bsp,
            channels,
            talkgroups,
            units,
            tune,
            levels,
            radios,
            audio,
            rotators,
            mut amplifiers,
            selected,
            page,
            tabs,
            channel_zone,
            channel_sel,
            talkgroup_zone,
            talkgroup_sel,
            unit_sel,
            control_follow,
            trunk_receivers,
            scan_zones,
            monitor_zones,
            monitor_talkgroups,
            meters,
            nanovna,
            pulse,
            pulse_secs,
            voice_hang_ms,
            override_hold_emergency_priority,
            mut patchbay,
            rec,
            ..
        } = file;
        self.rec.face = rec;
        self.rec.configured = None;
        // Every amplifier gets a unique fixed id; bindings to a duplicated
        // id or an ambiguous old name are dropped with a note (#96).
        let dup = amp::repair_ids(&mut amplifiers);
        patchbay::bind::migrate(&mut patchbay, &amplifiers, &dup);
        self.patchbay = patchbay;
        self.patchbay_ui = Default::default();
        self.pulse_on = pulse;
        self.pulse_secs = pulse_secs;
        self.voice_hang_ms = voice_hang_ms;
        self.override_hold = override_hold_emergency_priority;
        self.plan = bsp;
        self.channels = channels;
        self.talkgroups = talkgroups;
        self.units = units;
        self.unit_sel = 0;
        self.unit_desc_dirty = false;
        self.talkgroup_zone = 0;
        self.talkgroup_sel = 0;
        self.talkgroup_note.clear();
        // The channel file on disk is a different document. Leave its path
        // empty so a later Channels Save does not overwrite that file.
        self.channel_path.clear();
        self.channel_zone = 0;
        self.channel_sel = 0;
        self.channel_note.clear();
        self.zone_rename = None;
        // The manager stops the radios it started and brings this list up.
        // An empty list is the hamlib dummy. The face fields are sent once
        // that new session is the one on screen.
        let after_gen = self.shared.lock().expect("rig thread").live_gen;
        let after_list = self.hub.view().list_gen;
        self.pending_tune = Some(PendingTune {
            tune: tune.clone(),
            levels: levels.clone(),
            after_gen,
            after_list,
        });
        self.audio_saved = crate::audio::canonical(&audio);
        self.audio_hold = true;
        // The generation is the list on screen now. Replace bumps it, and
        // the flags are applied to that newer list.
        let list_gen = self.hub.view().list_gen;
        let follow_flags = config::control_follow_flags(radios.len(), &control_follow);
        self.hub.replace(radios, selected);
        self.pending_control = Some((list_gen, follow_flags));
        self.trunk_receivers = trunk_receivers;
        self.cc_on.clear();
        self.hub.apply_audio(audio);
        self.rotators.replace(rotators);
        // The daemons start and are polled. No power command is sent.
        self.amplifiers.replace(amplifiers);
        self.page = page_from_name(&page);
        if self.page != Page::Tools {
            self.page_return = self.page;
        }
        self.tab_order = tabs;
        self.channel_zone = channel_zone;
        self.channel_sel = channel_sel;
        self.clear_scan_zones();
        self.scan_zones = scan_zones.into_iter().collect();
        self.monitor_zones = monitor_zones.into_iter().collect();
        self.monitor_talkgroups = monitor_talkgroups.into_iter().collect();
        self.hackrf_open.clear();
        self.hackrf_call.clear();
        self.hackrf_hold.clear();
        self.hackrf_away.clear();
        self.trunk_pool.clear();
        self.hackrf_cc_feeds.clear();
        self.hackrf_voice_feed.clear();
        self.hackrf_watch_sent.clear();
        self.hackrf_voice_sent.clear();
        self.hackrf_p2_sent.clear();
        self.hackrf_dial_seeded.clear();
        self.talkgroup_zone = talkgroup_zone;
        self.talkgroup_sel = talkgroup_sel;
        self.unit_sel = unit_sel;
        self.zone_list_follow = true;
        self.clamp_channel_selection();
        self.apply_meters(&meters);
        self.tools
            .lock()
            .expect("tools")
            .set_nanovna_face(nanovna);
        self.local_mem = tune.mem_hz;
        self.show_spread = tune.show_spread;
        self.step_hz = tune.step_hz;
        self.zone_sel = tune.zone_sel;
        self.band_sel = tune.band_sel;
        self.clamp_plan_selection();
        self.band_mem = tune.band_mem.into_iter().collect();
        self.favorite = tune.favorite.map(|fav| Favorite {
            hz: fav.hz,
            mode: fav.mode,
            passband_hz: fav.passband_hz,
        });
        self.host = tune.host;
        self.port = tune.port;
        match config::install_new_bsp(plan::bsp_dir().as_deref(), &bsp_name, &self.plan) {
            Ok(config::BspInstall::Written(path)) => {
                self.plan_path = path.display().to_string();
                self.config_note = format!("Wrote {bsp_name} into the band-plan folder.");
            }
            Ok(config::BspInstall::Kept(path)) => {
                self.plan_path = path.display().to_string();
                self.config_note.clear();
            }
            Ok(config::BspInstall::NoFolder) => {
                self.plan_path = bsp_name.clone();
                self.config_note =
                    format!("Band-plan folder was not found, so {bsp_name} was not written.");
            }
            Err(err) => self.config_note = err,
        }
    }

    fn clamp_channel_selection(&mut self) {
        if self.channels.zones.is_empty() {
            self.channel_zone = 0;
            self.channel_sel = 0;
            return;
        }
        if self.channel_zone >= self.channels.zones.len() {
            self.channel_zone = 0;
        }
        let count = self.channels.zones[self.channel_zone].channels.len();
        if count == 0 || self.channel_sel >= count {
            self.channel_sel = 0;
        }
    }

    fn apply_meters(&mut self, face: &config::MetersFace) {
        let pos = match (face.window.x, face.window.y) {
            (Some(x), Some(y)) => Some(egui::pos2(x, y)),
            _ => None,
        };
        let size = match (face.window.width, face.window.height) {
            (Some(width), Some(height)) => Some(egui::vec2(width, height)),
            _ => None,
        };
        let monitor = match (face.window.monitor_width, face.window.monitor_height) {
            (Some(width), Some(height)) => Some(egui::vec2(width, height)),
            _ => None,
        };
        {
            let mut meters = self.meters.lock().expect("meters");
            meters.locked = face.locked;
            meters.detached = face.detached;
            meters.order = face.order.clone();
            meters.drag = None;
            meters.dock = false;
            meters.saved_pos = pos;
            meters.saved_size = size;
            meters.saved_monitor = monitor;
            meters.saved_ppp = face.window.pixels_per_point;
        }
        self.meter_place = pos;
        self.meter_size = size;
        self.meter_placed = false;
    }

    fn clamp_plan_selection(&mut self) {
        if self.plan.zones.is_empty() {
            self.zone_sel = 0;
            self.band_sel = 0;
            return;
        }
        if self.zone_sel >= self.plan.zones.len() {
            self.zone_sel = 0;
        }
        let count = self.plan.zones[self.zone_sel].bands.len();
        if count == 0 || self.band_sel >= count {
            self.band_sel = 0;
        }
    }

    /// Send the radio fields from a loaded config. A refused frequency does
    /// not stop the mode, split, RIT, or level commands. PTT is not keyed.
    fn push_radio(&mut self, snap: &Snapshot, tune: &config::Tune, levels: &BTreeMap<String, f64>) {
        if !snap.online() {
            return;
        }
        self.tune(snap, tune.freq_hz);
        if snap.caps.can_set_mode && !tune.mode.is_empty() {
            let allowed = snap.caps.modes.is_empty()
                || snap
                    .caps
                    .modes
                    .iter()
                    .any(|mode| mode.eq_ignore_ascii_case(&tune.mode));
            if allowed {
                self.mode_hold = Some(tune.mode.clone());
                self.hold_pass = Some(tune.passband_hz);
                self.send(Command::SetMode {
                    mode: tune.mode.clone(),
                    passband_hz: tune.passband_hz,
                });
            }
        }
        if tune.sub_freq_hz > 0.0 {
            let wanted = (!tune.sub_mode.is_empty()).then_some(tune.sub_mode.as_str());
            self.tune_sub(snap, tune.sub_freq_hz, wanted);
        } else if snap.caps.can_set_mode && !tune.sub_mode.is_empty() {
            let allowed = snap.caps.modes.is_empty()
                || snap
                    .caps
                    .modes
                    .iter()
                    .any(|mode| mode.eq_ignore_ascii_case(&tune.sub_mode));
            if allowed {
                self.sub_mode_hold = Some(tune.sub_mode.clone());
                self.send(Command::SetSubMode {
                    mode: tune.sub_mode.clone(),
                    passband_hz: tune.passband_hz,
                });
            }
        }
        if snap.caps.can_set_rit {
            self.send(Command::SetRit(tune.rit_hz));
        }
        if snap.caps.can_set_split {
            let tx_vfo = if tune.split {
                if snap.sub_vfo.is_empty() {
                    "VFOB".to_string()
                } else {
                    snap.sub_vfo.clone()
                }
            } else if snap.vfo.is_empty() {
                "VFOA".to_string()
            } else {
                snap.vfo.clone()
            };
            self.send(Command::SetSplit {
                on: tune.split,
                tx_vfo,
            });
        }
        for (name, value) in levels {
            if snap.caps.level_range(name).is_none() {
                continue;
            }
            self.hold_levels.insert(name.clone(), *value);
            self.send(Command::SetLevel {
                name: name.clone(),
                value: *value,
            });
        }
    }

    fn zone_row(&self, ui: &mut egui::Ui, idx: usize) -> (bool, bool, egui::Response) {
        let name = self
            .channels
            .zones
            .get(idx)
            .map(|zone| zone.name.clone())
            .unwrap_or_default();
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        let painter = ui.painter();
        let listed = self.scan_zones.contains(&idx);
        let text = if listed {
            egui::Color32::from_rgb(236, 226, 246)
        } else {
            ui.visuals().text_color()
        };
        if listed {
            let edge = if self.channel_zone == idx {
                egui::Stroke::new(1.5, egui::Color32::from_rgb(186, 120, 230))
            } else {
                egui::Stroke::NONE
            };
            painter.rect(
                rect,
                egui::CornerRadius::same(4),
                egui::Color32::from_rgb(42, 16, 64),
                edge,
                egui::StrokeKind::Inside,
            );
        } else if self.channel_zone == idx {
            painter.rect_filled(rect, egui::CornerRadius::same(4), HAM.gamma_multiply(0.28));
        } else if idx % 2 == 1 {
            painter.rect_filled(rect, egui::CornerRadius::ZERO, ui.visuals().faint_bg_color);
        }
        painter.text(
            rect.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &name,
            egui::FontId::proportional(14.0),
            text,
        );
        let clicked = response.clicked();
        let doubled = response.double_clicked();
        (clicked, doubled, response)
    }

    fn channel_row(&self, ui: &mut egui::Ui, idx: usize) -> (bool, bool) {
        let Some(channel) = self
            .channels
            .zones
            .get(self.channel_zone)
            .and_then(|zone| zone.channels.get(idx))
        else {
            return (false, false);
        };
        let freq = format!("{:10.3}", channel.hz / 1_000_000.0);
        let slot = format!("{:>3}", idx + 1);
        let name = channel.name.clone();
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        // Clipped to the row so long names never run under the details panel.
        let painter = ui.painter_at(rect);
        if self.channel_sel == idx {
            painter.rect_filled(rect, egui::CornerRadius::same(4), HAM.gamma_multiply(0.28));
        } else if idx % 2 == 1 {
            painter.rect_filled(rect, egui::CornerRadius::ZERO, ui.visuals().faint_bg_color);
        }
        let mut text = format!("{slot}   {freq}   {name}");
        if channel.control {
            text.push_str("  ctrl");
        }
        if channel.encrypted {
            text.push_str("  enc");
        }
        painter.text(
            rect.left_center() + egui::vec2(rec_ui::ROW_GUTTER, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::monospace(14.0),
            ui.visuals().text_color(),
        );
        if channel.record {
            rec_ui::paint_row_mark(ui, rect);
        }
        let response = response.on_hover_text(rec_ui::row_hover(channel.record));
        if self.channel_list_follow && idx == self.channel_sel {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        let marked = response.secondary_clicked();
        (response.clicked(), marked)
    }

    fn channel_editor(&mut self, ui: &mut egui::Ui, snap: &Snapshot, width: f32) {
        let frame = details_frame(ui);
        let chrome =
            frame.inner_margin.sum().x + frame.stroke.width * 2.0 + frame.outer_margin.sum().x;
        let inner = (width - chrome).max(0.0);
        let zone_idx = self.channel_zone;
        let sel = self.channel_sel;
        let Some(channel) = self
            .channels
            .zones
            .get(zone_idx)
            .and_then(|zone| zone.channels.get(sel))
        else {
            let mut add = false;
            frame.show(ui, |ui| {
                ui.set_min_width(inner);
                ui.label("No channel.");
                if ui.button("Add").clicked() {
                    add = true;
                }
            });
            if add {
                let hz = self.shown_freq(snap);
                let mode = snap.mode.clone();
                self.add_channel(hz, mode);
            }
            return;
        };
        let mut name = channel.name.clone();
        let mut mhz = channel.hz / 1_000_000.0;
        let current = channel.mode.clone();
        let modes = mode_choices(snap);
        let mut name_changed = false;
        let mut freq_changed = false;
        let mut control = channel.control;
        let mut encrypted = channel.encrypted;
        let vocoder_now = channels::vocoder_kind(&channel.vocoder);
        let mut picked = None;
        let mut vocoder_pick = None;
        let mut add = false;
        let mut delete = false;
        let show_invert = channels::vocoder_can_invert(vocoder_now);
        let mut invert = channel.invert;
        frame.show(ui, |ui| {
            ui.set_min_width(inner);
            ui.label(egui::RichText::new(format!("Channel {}", sel + 1)).strong());
            name_changed = ui
                .horizontal(|ui| {
                    ui.label("Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut name)
                            .id_salt(("ch-name", zone_idx, sel))
                            .desired_width(180.0),
                    )
                })
                .inner
                .changed();
            freq_changed = mhz_drag(ui, "Frequency", &mut mhz);
            ui.horizontal(|ui| {
                ui.label("Mode");
                egui::ComboBox::from_id_salt(("ch-mode", zone_idx, sel))
                    .width(180.0)
                    .selected_text(if current.is_empty() {
                        "—".to_string()
                    } else {
                        current.clone()
                    })
                    .show_ui(ui, |ui| {
                        for mode in &modes {
                            if ui.selectable_label(current == *mode, mode).clicked() {
                                picked = Some(mode.clone());
                            }
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Vocoder");
                egui::ComboBox::from_id_salt(("ch-vocoder", zone_idx, sel))
                    .width(180.0)
                    .selected_text(channels::vocoder_label(vocoder_now))
                    .show_ui(ui, |ui| {
                        for (value, label) in channels::VOCODER_CHOICES {
                            let on = vocoder_now == *value;
                            if ui.selectable_label(on, *label).clicked() && !on {
                                vocoder_pick = Some((*value).to_string());
                            }
                        }
                    });
            });
            if show_invert {
                ui.checkbox(&mut invert, "Invert");
            }
            ui.checkbox(&mut control, "Control");
            ui.checkbox(&mut encrypted, "Encrypted")
                .on_hover_text("No encryption key is stored, so this is not scanned or played.");
            ui.horizontal(|ui| {
                if ui.button("Add").clicked() {
                    add = true;
                }
                if ui.button("Delete").clicked() {
                    delete = true;
                }
            });
        });
        if name_changed {
            if let Some(channel) = self
                .channels
                .zones
                .get_mut(zone_idx)
                .and_then(|zone| zone.channels.get_mut(sel))
            {
                channel.name = name;
            }
        }
        if freq_changed {
            let hz = mhz * 1_000_000.0;
            if hz.is_finite() && hz > 0.0 {
                self.set_channel_hz(snap, hz);
            }
        }
        if let Some(mode) = picked {
            if let Some(channel) = self
                .channels
                .zones
                .get_mut(zone_idx)
                .and_then(|zone| zone.channels.get_mut(sel))
            {
                channel.mode = mode.clone();
            }
            if snap.online() && snap.caps.can_set_mode {
                self.send(Command::SetMode {
                    mode,
                    passband_hz: snap.passband_hz,
                });
            }
        }
        let flags_changed = self
            .channels
            .zones
            .get(zone_idx)
            .and_then(|zone| zone.channels.get(sel))
            .is_some_and(|channel| {
                channel.control != control
                    || channel.encrypted != encrypted
                    || (show_invert && channel.invert != invert)
            });
        if flags_changed {
            if let Some(channel) = self
                .channels
                .zones
                .get_mut(zone_idx)
                .and_then(|zone| zone.channels.get_mut(sel))
            {
                channel.control = control;
                channel.encrypted = encrypted;
                if show_invert {
                    channel.invert = invert;
                }
            }
        }
        if let Some(kind) = vocoder_pick {
            if let Some(channel) = self
                .channels
                .zones
                .get_mut(zone_idx)
                .and_then(|zone| zone.channels.get_mut(sel))
            {
                channel.vocoder = kind.clone();
                if !channels::vocoder_can_invert(&kind) {
                    channel.invert = false;
                }
            }
        }
        if delete {
            self.delete_channel();
        } else if add {
            let hz = self.shown_freq(snap);
            let mode = snap.mode.clone();
            self.add_channel(hz, mode);
        }
    }

    fn add_channel(&mut self, hz: f64, mode: String) {
        if !hz.is_finite() || hz <= 0.0 {
            self.channel_note = "Channel frequency must be greater than zero.".into();
            return;
        }
        let Some(zone) = self.channels.zones.get_mut(self.channel_zone) else {
            return;
        };
        zone.channels.push(channels::Channel {
            record: false,
            name: String::new(),
            hz,
            mode,
            control: false,
            encrypted: false,
            vocoder: String::new(),
            invert: false,
        });
        self.channel_sel = zone.channels.len() - 1;
        self.channel_note.clear();
    }

    fn delete_channel(&mut self) {
        let Some(zone) = self.channels.zones.get_mut(self.channel_zone) else {
            return;
        };
        if self.channel_sel < zone.channels.len() {
            zone.channels.remove(self.channel_sel);
            if self.channel_sel >= zone.channels.len() && self.channel_sel > 0 {
                self.channel_sel -= 1;
            }
        }
    }

    /// Stores `hz` on the selected memory. A frequency the rig cannot receive
    /// stays in the file and is not sent.
    fn set_channel_hz(&mut self, snap: &Snapshot, hz: f64) {
        if !hz.is_finite() || hz <= 0.0 {
            return;
        }
        let Some(channel) = self
            .channels
            .zones
            .get_mut(self.channel_zone)
            .and_then(|zone| zone.channels.get_mut(self.channel_sel))
        else {
            return;
        };
        channel.hz = hz;
        if snap.caps.allow_freq(hz).is_some() {
            self.tune(snap, hz);
        }
    }

    fn selected_trunk(&self) -> bool {
        self.hub.view().rows.iter().any(|row| {
            if !row.selected {
                return false;
            }
            if row.spec.driver == cat::RadioDriver::Hackrf {
                return self.cc_on.get(&row.id).copied().unwrap_or(false);
            }
            hears_trunk_pipe(row)
        })
    }

    /// Frequency the scanner is measuring.
    fn scan_memory_hz(&self) -> Option<f64> {
        let zone = self.memory_scan.zone()?;
        let index = self.memory_scan.index()?;
        let hz = self.channels.zones.get(zone)?.channels.get(index)?.hz;
        if hz.is_finite() && hz > 0.0 {
            Some(hz)
        } else {
            None
        }
    }

    /// One scanner sample from the capture. A rigctl poll never arrives for
    /// this radio, and the capture sequence goes back to 1 when the LO moves.
    fn hackrf_scan_sample(&mut self, snap: &Snapshot) -> (u64, f64, f64) {
        let spec = self.hub.spectrum();
        let fresh = !spec.db.is_empty()
            && spec.seq != 0
            && (spec.seq != self.hackrf_scan_seq
                || (spec.center_hz - self.hackrf_scan_center).abs() > 1.0);
        if fresh {
            self.hackrf_scan_poll = self.hackrf_scan_poll.wrapping_add(1);
            if self.hackrf_scan_poll == 0 {
                self.hackrf_scan_poll = 1;
            }
            self.hackrf_scan_seq = spec.seq;
            self.hackrf_scan_center = spec.center_hz;
        }
        let center = if spec.center_hz > 0.0 {
            spec.center_hz
        } else {
            snap.freq_hz
        };
        let target = self.scan_memory_hz().unwrap_or(center);
        let strength = crate::hackrf::channel_db(&spec.db, spec.center_hz, target);
        let freq = crate::hackrf::scan_freq(center, target, strength);
        (self.hackrf_scan_poll, freq, strength)
    }

    fn zone_candidates(&self, snap: &Snapshot, zone_index: usize) -> Vec<scan::Candidate> {
        let Some(zone) = self.channels.zones.get(zone_index) else {
            return Vec::new();
        };
        zone.channels
            .iter()
            .enumerate()
            .map(|(index, channel)| scan::Candidate {
                index,
                hz: channel.hz,
                control: channel.control,
                tunable: channel.hz.is_finite()
                    && channel.hz > 0.0
                    && !channel.encrypted
                    && snap.caps.allow_freq(channel.hz).is_some(),
            })
            .collect()
    }

    fn scan_targets(&self, snap: &Snapshot) -> Vec<scan::Memory> {
        scan::targets(
            self.channel_zone,
            &self.zone_candidates(snap, self.channel_zone),
            self.selected_trunk(),
        )
    }

    fn every_target(&self, snap: &Snapshot) -> Vec<scan::Memory> {
        let trunk = self.selected_trunk();
        self.channels
            .zones
            .iter()
            .enumerate()
            .flat_map(|(zone, _)| scan::targets(zone, &self.zone_candidates(snap, zone), trunk))
            .collect()
    }

    /// Tunable memories in the marked zones, in zone-list order.
    fn listed_targets(&self, snap: &Snapshot) -> Vec<scan::Memory> {
        let trunk = self.selected_trunk();
        self.scan_zones
            .iter()
            .flat_map(|zone| scan::targets(*zone, &self.zone_candidates(snap, *zone), trunk))
            .collect()
    }

    fn stop_scan(&mut self) {
        self.memory_scan.stop();
        self.scan_amber_origin = false;
    }

    /// The channel document changed. Marked zones from the previous file
    /// are dropped. A purple scan that was not amber underneath stops.
    /// An amber scan keeps walking every zone of the file now on screen.
    fn clear_scan_zones(&mut self) {
        let had = !self.scan_zones.is_empty();
        self.scan_zones.clear();
        if !self.memory_scan.is_on() {
            self.scan_amber_origin = false;
            return;
        }
        if had && !self.scan_amber_origin {
            self.stop_scan();
        }
    }

    fn scan_link_block(&self, snap: &Snapshot) -> Option<&'static str> {
        if snap.kind != LinkKind::Live {
            return Some("The radio is not on.");
        }
        if self.following() {
            return Some("Gpredict is moving this radio.");
        }
        None
    }

    fn scan_block_reason(&self, snap: &Snapshot) -> Option<&'static str> {
        if let Some(reason) = self.scan_link_block(snap) {
            return Some(reason);
        }
        if self.scan_targets(snap).is_empty() {
            return Some("This zone has no channel this radio can tune.");
        }
        None
    }

    fn start_memory_scan(&mut self, snap: &Snapshot) {
        self.scan_amber_origin = false;
        let memories = self.scan_targets(snap);
        let Some(first) = memories
            .iter()
            .find(|memory| memory.index == self.channel_sel)
            .copied()
            .or_else(|| memories.first().copied())
        else {
            self.memory_scan.stop();
            return;
        };
        self.memory_scan.start(
            self.channel_zone,
            first.index,
            first.hz,
            self.selected_trunk(),
            false,
        );
        self.channel_sel = first.index;
        self.channel_list_follow = true;
    }

    /// Amber scan. The list is every tunable memory, zone by zone.
    fn start_all_scan(&mut self, snap: &Snapshot) {
        let memories = self.every_target(snap);
        let Some(first) = memories
            .iter()
            .copied()
            .find(|memory| memory.zone == self.channel_zone)
            .or_else(|| memories.first().copied())
        else {
            self.channel_note = "No channel this radio can tune.".into();
            return;
        };
        self.scan_amber_origin = true;
        self.channel_zone = first.zone;
        self.memory_scan
            .start(first.zone, first.index, first.hz, self.selected_trunk(), true);
        self.channel_sel = first.index;
        self.channel_list_follow = true;
        self.zone_list_follow = true;
    }

    /// Purple scan. Marked zones only. Does not record an amber origin;
    /// a double-click of SCAN while marks exist stops first, and that stop
    /// already cleared the origin.
    fn start_listed_scan(&mut self, snap: &Snapshot) {
        let memories = self.listed_targets(snap);
        let Some(first) = memories
            .iter()
            .copied()
            .find(|memory| memory.zone == self.channel_zone)
            .or_else(|| memories.first().copied())
        else {
            self.channel_note = "No channel this radio can tune.".into();
            return;
        };
        self.scan_amber_origin = false;
        self.channel_zone = first.zone;
        self.memory_scan
            .start(first.zone, first.index, first.hz, self.selected_trunk(), true);
        self.channel_sel = first.index;
        self.channel_list_follow = true;
        self.zone_list_follow = true;
    }

    fn toggle_scan_zone(&mut self, snap: &Snapshot, idx: usize) {
        if !self.scan_zones.remove(&idx) {
            self.scan_zones.insert(idx);
        }
        if self.memory_scan.is_on() {
            self.apply_scan_list(snap);
        }
    }

    /// The marked zones changed while SCAN is on. The floor stays. Dropping
    /// the last mark returns to amber when this scan began as amber.
    fn apply_scan_list(&mut self, snap: &Snapshot) {
        if self.scan_zones.is_empty() {
            if self.scan_amber_origin {
                self.memory_scan.follow_list();
                self.keep_or_jump(&self.every_target(snap));
                return;
            }
            self.start_memory_scan(snap);
            if !self.memory_scan.is_on() {
                self.channel_note = "This zone has no channel this radio can tune.".into();
            }
            return;
        }
        let memories = self.listed_targets(snap);
        if memories.is_empty() {
            self.stop_scan();
            self.channel_note = "No channel this radio can tune.".into();
            return;
        }
        self.memory_scan.follow_list();
        self.keep_or_jump(&memories);
    }

    /// Stay on the current memory when it is still in `memories`. Otherwise
    /// jump to the first memory of that zone, or the first memory in the list.
    fn keep_or_jump(&mut self, memories: &[scan::Memory]) {
        let zone = self.memory_scan.zone();
        let index = self.memory_scan.index();
        let still = memories
            .iter()
            .any(|memory| Some(memory.zone) == zone && Some(memory.index) == index);
        if still {
            return;
        }
        let Some(next) = zone
            .and_then(|zone| memories.iter().copied().find(|memory| memory.zone == zone))
            .or_else(|| memories.first().copied())
        else {
            self.stop_scan();
            self.channel_note = "No channel this radio can tune.".into();
            return;
        };
        self.memory_scan.jump(next.zone, next.index, next.hz);
        self.channel_zone = next.zone;
        self.channel_sel = next.index;
        self.channel_list_follow = true;
        self.zone_list_follow = true;
    }

    /// SCAN stays on. A one-zone scan measures the new zone from scratch.
    /// An all-zone scan jumps into it and keeps walking the other zones.
    fn retarget_scan(&mut self, snap: &Snapshot) {
        if !self.scan_zones.is_empty() {
            let memories = self.listed_targets(snap);
            let Some(first) = memories
                .iter()
                .copied()
                .find(|memory| memory.zone == self.channel_zone)
            else {
                self.stop_scan();
                self.channel_sel = 0;
                self.channel_note = "This zone has no channel this radio can tune.".into();
                return;
            };
            self.memory_scan.follow_list();
            self.memory_scan.jump(first.zone, first.index, first.hz);
            self.channel_sel = first.index;
            self.channel_list_follow = true;
            return;
        }
        if self.memory_scan.all_zones() {
            let memories = self.every_target(snap);
            let Some(first) = memories
                .iter()
                .copied()
                .find(|memory| memory.zone == self.channel_zone)
            else {
                self.stop_scan();
                self.channel_sel = 0;
                self.channel_note = "This zone has no channel this radio can tune.".into();
                return;
            };
            self.memory_scan.jump(first.zone, first.index, first.hz);
            self.channel_sel = first.index;
            self.channel_list_follow = true;
            return;
        }
        let memories = self.scan_targets(snap);
        let Some(first) = memories.first().copied() else {
            self.stop_scan();
            self.channel_sel = 0;
            self.channel_note = "This zone has no channel this radio can tune.".into();
            return;
        };
        self.scan_amber_origin = false;
        self.memory_scan.start(
            self.channel_zone,
            first.index,
            first.hz,
            self.selected_trunk(),
            false,
        );
        self.channel_sel = first.index;
        self.channel_list_follow = true;
    }

    /// The short status sentence stays after the scan that wrote it has
    /// stopped. Drop it once this radio can tune a channel in the list.
    fn clear_stale_channel_note(&mut self, snap: &Snapshot) {
        if self.channel_note != "No channel this radio can tune." {
            return;
        }
        let tunable = if self.scan_zones.is_empty() {
            !self.every_target(snap).is_empty()
        } else {
            !self.listed_targets(snap).is_empty()
        };
        if tunable {
            self.channel_note.clear();
        }
    }

    /// Drop SCAN when the radio, Gpredict, or the zone can no longer be
    /// hopped. Trunk Follow does not drop it; the scan owns those hops.
    fn release_memory_scan(&mut self, snap: &Snapshot) {
        if !self.memory_scan.is_on() {
            return;
        }
        if snap.ptt {
            self.stop_scan();
            return;
        }
        if let Some(reason) = self.scan_link_block(snap) {
            self.stop_scan();
            self.channel_note = reason.to_string();
            return;
        }
        if !self.scan_zones.is_empty() {
            if self.listed_targets(snap).is_empty() {
                self.stop_scan();
                self.channel_note = "No channel this radio can tune.".into();
            }
            return;
        }
        if self.memory_scan.all_zones() {
            return;
        }
        if self.scan_targets(snap).is_empty() {
            self.stop_scan();
            self.channel_note = "This zone has no channel this radio can tune.".into();
        }
    }

    fn advance_memory_scan(&mut self, ctx: &egui::Context, snap: &Snapshot, poll_gen: u64) {
        if let Some(deadline) = self.memory_scan.dwell_deadline() {
            let now = std::time::Instant::now();
            if deadline > now {
                ctx.request_repaint_after(deadline.saturating_duration_since(now));
            } else {
                ctx.request_repaint();
            }
        }
        if !self.memory_scan.is_on() {
            return;
        }
        let memories = if !self.scan_zones.is_empty() {
            self.listed_targets(snap)
        } else if self.memory_scan.all_zones() {
            self.every_target(snap)
        } else {
            self.scan_targets(snap)
        };
        // A zone outside the list must not stop the scan. The click handler
        // jumps only when that zone is one the scan is allowed to hop.
        let step_zone = if memories.iter().any(|memory| memory.zone == self.channel_zone) {
            self.channel_zone
        } else {
            self.memory_scan.zone().unwrap_or(self.channel_zone)
        };
        let (poll_gen, freq_hz, strength) = if snap.model == "HackRF" {
            self.hackrf_scan_sample(snap)
        } else if snap.model == "GQRX" {
            (poll_gen, snap.freq_hz, gqrx_dbfs_to_s9(snap.strength_db))
        } else {
            (poll_gen, snap.freq_hz, snap.strength_db)
        };
        let action = self.memory_scan.step(
            &scan::Sample {
                now: std::time::Instant::now(),
                poll_gen,
                freq_hz,
                strength_db: strength,
                trunk: self.selected_trunk(),
            },
            step_zone,
            &memories,
        );
        match action {
            scan::Action::Idle => {}
            scan::Action::Stop => {
                // A trunk or zone change stops the scan while the marked
                // channels are still tunable. That stop is not an empty list.
                let note = if self.scan_zones.is_empty() {
                    self.scan_block_reason(snap)
                } else if self.listed_targets(snap).is_empty() {
                    Some("No channel this radio can tune.")
                } else {
                    None
                };
                self.stop_scan();
                if let Some(note) = note {
                    self.channel_note = note.to_string();
                } else if self.channel_note == "No channel this radio can tune." {
                    self.channel_note.clear();
                }
            }
            scan::Action::Tune { index, resend } => {
                if let Some(zone) = self.memory_scan.zone() {
                    if zone != self.channel_zone {
                        self.channel_zone = zone;
                        self.zone_list_follow = true;
                        self.channel_list_follow = true;
                    }
                }
                if index != self.channel_sel {
                    self.channel_list_follow = true;
                }
                self.channel_sel = index;
                if resend {
                    self.scan_sending = true;
                    self.recall_channel(snap, index);
                    self.scan_sending = false;
                }
            }
        }
    }

    /// A clear unmarked grant is heard. An encrypted grant, or a talkgroup or
    /// channel marked encrypted, is heard only when a filled key is loaded.
    /// A placeholder does not count.
    fn hear_grant(&self, alg: u16, tg: u32, hz: f64) -> bool {
        follow::hear_clear(
            alg,
            talkgroups::is_encrypted(&self.talkgroups, tg),
            channels::hz_encrypted(&self.channels, hz),
        ) || keys::has_filled(&self.keys)
    }

    fn recall_channel(&mut self, snap: &Snapshot, idx: usize) {
        self.channel_sel = idx;
        let Some(channel) = self
            .channels
            .zones
            .get(self.channel_zone)
            .and_then(|zone| zone.channels.get(idx))
        else {
            return;
        };
        let hz = channel.hz;
        let mode = channel.mode.clone();
        let control = channel.control;
        // No key is stored. Selecting the row still opens the editor.
        if channel.encrypted {
            return;
        }
        // A scan hop is not the operator picking a control. hold() would
        // reset every trunk follower, including a radio this scan is not tuning.
        if control && !self.scan_sending {
            self.hold_control(hz);
        }
        if hz.is_finite() && hz > 0.0 && snap.caps.allow_freq(hz).is_some() {
            self.tune(snap, hz);
            if snap.online() && snap.caps.can_set_mode && !mode.is_empty() {
                self.send(Command::SetMode {
                    mode,
                    passband_hz: snap.passband_hz,
                });
            }
        }
    }

    fn band_row(&self, ui: &mut egui::Ui, idx: usize) -> bool {
        let Some(band) = self
            .plan
            .zones
            .get(self.zone_sel)
            .and_then(|zone| zone.bands.get(idx))
        else {
            return false;
        };
        let freq = format!("{:10.3}", band.home_hz / 1_000_000.0);
        let slot = format!("{:>3}", idx + 1);
        let name = band.name.clone();
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        let painter = ui.painter();
        if self.band_sel == idx {
            painter.rect_filled(rect, egui::CornerRadius::same(4), HAM.gamma_multiply(0.28));
        } else if idx % 2 == 1 {
            painter.rect_filled(rect, egui::CornerRadius::ZERO, ui.visuals().faint_bg_color);
        }
        let text = format!("{slot}   {freq}   {name}");
        painter.text(
            rect.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::monospace(14.0),
            ui.visuals().text_color(),
        );
        response.clicked()
    }

    fn band_editor(&mut self, ui: &mut egui::Ui, snap: &Snapshot, width: f32) {
        let frame = egui::Frame::group(ui.style());
        let chrome =
            frame.inner_margin.sum().x + frame.stroke.width * 2.0 + frame.outer_margin.sum().x;
        let inner = (width - chrome).max(0.0);
        let sel = self.band_sel;
        let Some(band) = self
            .plan
            .zones
            .get(self.zone_sel)
            .and_then(|zone| zone.bands.get(sel))
        else {
            frame.show(ui, |ui| {
                ui.set_min_width(inner);
                self.plan_save_row(ui);
                ui.label("No band.");
            });
            return;
        };
        let title = band.name.clone();
        let mut name = band.name.clone();
        let mut home_mhz = band.home_hz / 1_000_000.0;
        let mut lo_mhz = band.lo_hz / 1_000_000.0;
        let mut hi_mhz = band.hi_hz / 1_000_000.0;
        let current = band.mode.clone();
        let modes = mode_choices(snap);
        let mut name_changed = false;
        let mut home_changed = false;
        let mut lo_changed = false;
        let mut hi_changed = false;
        let mut picked = None;
        frame.show(ui, |ui| {
            ui.set_min_width(inner);
            self.plan_save_row(ui);
            ui.label(egui::RichText::new(title).strong());
            name_changed = ui
                .horizontal(|ui| {
                    ui.label("Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut name)
                            .id_salt(("band-name", self.zone_sel, sel))
                            .desired_width(180.0),
                    )
                })
                .inner
                .changed();
            home_changed = mhz_drag(ui, "Home", &mut home_mhz);
            lo_changed = mhz_drag(ui, "Low", &mut lo_mhz);
            hi_changed = mhz_drag(ui, "High", &mut hi_mhz);
            ui.horizontal(|ui| {
                ui.label("Mode");
                egui::ComboBox::from_id_salt(("band-mode", self.zone_sel, sel))
                    .width(180.0)
                    .selected_text(if current.is_empty() {
                        "—".to_string()
                    } else {
                        current.clone()
                    })
                    .show_ui(ui, |ui| {
                        for mode in &modes {
                            if ui.selectable_label(current == *mode, mode).clicked() {
                                picked = Some(mode.clone());
                            }
                        }
                    });
            });
        });
        if name_changed {
            if let Some(band) = self
                .plan
                .zones
                .get_mut(self.zone_sel)
                .and_then(|zone| zone.bands.get_mut(sel))
            {
                band.name = name;
            }
        }
        if lo_changed {
            if let Some(band) = self
                .plan
                .zones
                .get_mut(self.zone_sel)
                .and_then(|zone| zone.bands.get_mut(sel))
            {
                band.lo_hz = lo_mhz * 1_000_000.0;
            }
        }
        if hi_changed {
            if let Some(band) = self
                .plan
                .zones
                .get_mut(self.zone_sel)
                .and_then(|zone| zone.bands.get_mut(sel))
            {
                band.hi_hz = hi_mhz * 1_000_000.0;
            }
        }
        if home_changed {
            self.write_band_home(snap, sel, home_mhz * 1_000_000.0);
        }
        if let Some(mode) = picked {
            if let Some(band) = self
                .plan
                .zones
                .get_mut(self.zone_sel)
                .and_then(|zone| zone.bands.get_mut(sel))
            {
                band.mode = mode.clone();
            }
            if snap.online() && snap.caps.can_set_mode {
                self.send(Command::SetMode {
                    mode,
                    passband_hz: snap.passband_hz,
                });
            }
        }
    }

    /// A click inside a band moves that band's home. Clicks in a gap do nothing.
    fn dial_home(&mut self, snap: &Snapshot, hz: f64) {
        if !hz.is_finite() || hz < 0.0 {
            return;
        }
        let Some(idx) = self.plan.band_index_at(self.zone_sel, hz) else {
            return;
        };
        self.band_sel = idx;
        self.write_band_home(snap, idx, hz);
    }

    fn write_band_home(&mut self, snap: &Snapshot, idx: usize, hz: f64) {
        if !hz.is_finite() || hz < 0.0 {
            return;
        }
        let Some(band) = self
            .plan
            .zones
            .get_mut(self.zone_sel)
            .and_then(|zone| zone.bands.get_mut(idx))
        else {
            return;
        };
        if band.tx_hz.is_none() || band.tx_hz == Some(band.home_hz) {
            band.tx_hz = Some(hz);
        }
        band.home_hz = hz;
        if snap.caps.allow_freq(hz).is_some() {
            self.tune(snap, hz);
        }
    }

    fn recall_band(&mut self, snap: &Snapshot, idx: usize) {
        self.band_sel = idx;
        let Some(band) = self
            .plan
            .zones
            .get(self.zone_sel)
            .and_then(|zone| zone.bands.get(idx))
        else {
            return;
        };
        let hz = band.home_hz;
        let mode = band.mode.clone();
        if snap.caps.allow_freq(hz).is_some() {
            self.tune(snap, hz);
            if snap.online() && snap.caps.can_set_mode && !mode.is_empty() {
                self.send(Command::SetMode {
                    mode,
                    passband_hz: snap.passband_hz,
                });
            }
        }
    }

    fn page_meters(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        let radio = self
            .hub
            .view()
            .rows
            .iter()
            .find(|row| row.selected)
            .map(|row| row.spec.name.clone());
        let mut meters = self.meters.lock().expect("meters");
        if meters.detached {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label("Meters are in their own window.");
                if ui.button("Dock").clicked() {
                    meters.detached = false;
                    meters.drag = None;
                }
            });
            return;
        }
        paint_meter_page(
            ui,
            snap,
            radio.as_deref(),
            &mut meters,
            &self.system_clock,
            false,
        );
    }

    fn refresh_tools(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        let radio = self
            .hub
            .view()
            .rows
            .iter()
            .find(|row| row.selected)
            .map(|row| row.spec.name.clone())
            .unwrap_or_default();
        let plan = self.plan.clone();
        let (dock, nanovna_dirty, vna_dirty) = {
            let mut tools = self.tools.lock().expect("tools");
            let dock = tools.dock;
            if dock {
                tools.detached = false;
                tools.dock = false;
            }
            // Detached tools still draw in their window. Any other page,
            // including Timers, leaves NanoVNA so the next visit can connect.
            if self.page != Page::Tools && !tools.detached {
                tools.conceal_nanovna();
            }
            tools.set_dial(snap.freq_hz, &snap.mode, &radio);
            let saved = self.station_saved.is_some() && !self.config_path.is_empty();
            tools.follow_station(ctx, saved, &self.config_path);
            // The plan copy is display-only. It is not part of the NanoVNA face.
            tools.set_plan(&plan);
            let nanovna_dirty = tools.nanovna_dirty();
            let vna_dirty = tools.take_vna_dirty();
            (dock, nanovna_dirty, vna_dirty)
        };
        if dock {
            self.tool_placed = false;
        }
        let station_open = self.station_saved.is_some() && !self.config_path.is_empty();
        // A finished sweep updates the VNA file only. The station file is
        // rewritten when the port or the sweep settings change.
        if vna_dirty && station_open {
            self.save_vna();
        }
        if nanovna_dirty && station_open {
            self.save_config(snap);
        }
    }

    fn page_tools(&mut self, ui: &mut egui::Ui) {
        if self.tools.lock().expect("tools").detached {
            return;
        }
        // The example screens create their own top and central panels, so this
        // call stays on the root ui.
        self.tools.lock().expect("tools").ui(ui, false);
    }

    fn page_timers(&mut self, ui: &mut egui::Ui) {
        self.tools.lock().expect("tools").draw_timers(ui);
    }

    /// Open the tools as a native window when they are torn off.
    /// The builder's position is fixed for the life of that window.
    fn show_tools_window(&mut self, ctx: &egui::Context) {
        let viewport = tools::viewport_id();
        let closing = ctx.input_for(viewport, |input| input.viewport().close_requested());
        let mut tools = self.tools.lock().expect("tools");
        if closing {
            tools.detached = false;
            tools.dock = false;
            drop(tools);
            self.tool_placed = false;
            return;
        }
        let detached = tools.detached;
        drop(tools);
        if !detached {
            self.tool_placed = false;
            return;
        }
        if !self.tool_placed {
            let tools = self.tools.lock().expect("tools");
            self.tool_place = tools.saved_pos;
            self.tool_size = tools.saved_size;
            self.tool_placed = true;
        }
        let mut builder = egui::ViewportBuilder::default()
            .with_title("Tools")
            .with_app_id("hamspotcommander")
            .with_inner_size(self.tool_size.unwrap_or(egui::vec2(1180.0, 760.0)))
            .with_min_inner_size([720.0, 480.0])
            .with_icon(Arc::clone(&self.icon));
        if let Some(pos) = self.tool_place {
            builder = builder.with_position(pos);
        }
        let tools = Arc::clone(&self.tools);
        let viewport = tools::viewport_id();
        ctx.show_viewport_deferred(viewport, builder, move |ui, _class| {
            tools.lock().expect("tools").ui(ui, true);
        });
        ctx.request_repaint_of(viewport);
    }

    /// Open the meters as a native window when they are detached.
    /// The builder's position is fixed for the life of that window.
    fn show_meter_window(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        let detached = self.meters.lock().expect("meters").detached;
        if !detached {
            self.meter_placed = false;
            return;
        }
        {
            let mut feed = self.meter_feed.lock().expect("meters");
            feed.snap = snap.clone();
            feed.clock = self.system_clock.clone();
            feed.radio = self
                .hub
                .view()
                .rows
                .iter()
                .find(|row| row.selected)
                .map(|row| row.spec.name.clone());
        }
        if !self.meter_placed {
            let meters = self.meters.lock().expect("meters");
            self.meter_place = meters.saved_pos;
            self.meter_size = meters.saved_size;
            self.meter_placed = true;
        }
        let mut builder = egui::ViewportBuilder::default()
            .with_title("Meters")
            .with_app_id("hamspotcommander")
            .with_inner_size(self.meter_size.unwrap_or(egui::vec2(1000.0, 560.0)))
            .with_min_inner_size([480.0, 280.0])
            .with_icon(Arc::clone(&self.icon));
        if let Some(pos) = self.meter_place {
            builder = builder.with_position(pos);
        }
        let state = Arc::clone(&self.meters);
        let feed = Arc::clone(&self.meter_feed);
        let viewport = meter_viewport();
        ctx.show_viewport_deferred(viewport, builder, move |ui, _class| {
            meter_window_ui(ui, &state, &feed);
        });
        // The radio poll wakes the main window. The meter window needs its own.
        ctx.request_repaint_of(viewport);
    }

    /// Point every follower at a control the operator just picked. The radio
    /// was being pulled back to the first memory in the zone.
    fn hold_control(&mut self, hz: f64) {
        let now = std::time::Instant::now();
        for follower in self.followers.values_mut() {
            follower.hold(hz, now);
        }
    }

    fn control_port_label(&self, id: u64) -> String {
        let controls = channels::zone_controls(&self.channels, self.channel_zone);
        let hz = self
            .followers
            .get(&id)
            .and_then(|follower| follower.shown_control_hz())
            .or_else(|| controls.first().map(|(hz, _, _)| *hz));
        let Some(hz) = hz else {
            return "no control channel".into();
        };
        let name = controls
            .iter()
            .find(|(item, _, _)| (*item - hz).abs() < 1.0)
            .map(|(_, _, name)| name.as_str())
            .unwrap_or("");
        let mhz = format!("{:.4}", hz / 1_000_000.0);
        let text = if name.is_empty() {
            mhz
        } else {
            format!("{mhz}  {name}")
        };
        if text.chars().count() > 32 {
            let mut short: String = text.chars().take(31).collect();
            short.push('…');
            short
        } else {
            text
        }
    }

    fn note_decoder(&mut self, id: u64, lines: &[String]) {
        if lines.is_empty() {
            return;
        }
        let mut plains = Vec::new();
        let mut banner = None;
        {
            let tail = self.cc_tail.entry(id).or_default();
            for line in lines {
                if keys::is_key_dump(line) {
                    continue;
                }
                let plain = follow::plain_line(line);
                let plain = plain.trim();
                if plain.is_empty() {
                    continue;
                }
                if let Some(next) = follow::site_banner(plain) {
                    banner = Some(next);
                }
                plains.push(plain.to_string());
                tail.push(plain.to_string());
            }
            // Housekeeping lines are hidden at display time, so keep enough
            // behind them for the visible tail.
            const CAP: usize = 240;
            if tail.len() > CAP {
                let extra = tail.len() - CAP;
                tail.drain(0..extra);
            }
        }
        if let Some(banner) = banner {
            if let Some(word) = follow::p2_parameter_word(&banner) {
                let hackrf = self.hub.view().rows.iter().any(|row| {
                    row.id == id && row.spec.driver == cat::RadioDriver::Hackrf
                });
                if hackrf && self.hackrf_p2_sent.get(&id) != Some(&word) {
                    self.hub.hackrf_p2(id, &word);
                    self.hackrf_p2_sent.insert(id, word);
                }
            }
            self.cc_site.insert(id, banner);
        }
        self.note_trunk_identity(id, lines);
        let now = clock::unix_now_ms();
        for plain in plains {
            self.system_clock.note_line(&plain, now);
        }
    }

    /// Rolling stderr from the P25 decoder. Empty means the pipe has not
    /// written a line yet, not that the log was discarded.
    fn decoder_log(&mut self, ui: &mut egui::Ui, id: u64, radio: &str) {
        let all = self.cc_tail.get(&id).cloned().unwrap_or_default();
        let (lines, hidden) = follow::visible_lines(&all, self.show_housekeeping, 80);
        ui.horizontal(|ui| {
            ui.label(format!("{radio} decoder"));
            let label = if hidden > 0 {
                format!("Show housekeeping ({hidden} hidden)")
            } else {
                "Show housekeeping".to_string()
            };
            ui.checkbox(&mut self.show_housekeeping, label).on_hover_text(
                "Registrations, deregistrations, authentication, affiliation queries, status and acknowledgements. Their facts still go to the radio list.",
            );
        });
        egui::ScrollArea::vertical()
            .id_salt(("cc-log", id))
            .max_height(140.0)
            .auto_shrink([false, true])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if lines.is_empty() {
                    ui.label(
                        "No decoder lines yet. This is dsd-fme stderr from the trunking receive pipe.",
                    );
                } else {
                    for line in &lines {
                        let text = egui::RichText::new(*line).monospace().size(11.0);
                        if line.starts_with("DIRECT ") {
                            ui.label(text.color(caution()).strong());
                        } else {
                            ui.label(text);
                        }
                    }
                }
            });
    }

    /// Copy a loaded talkgroup Follow list onto the radios the station installed.
    ///
    /// The hub publishes that list on its own thread. Until its generation
    /// moves, the boxes stay off so they are not applied to the radios being
    /// replaced.
    fn apply_saved_control_follow(&mut self) {
        let Some((after_gen, flags)) = &self.pending_control else {
            return;
        };
        let view = self.hub.view();
        if view.list_gen == *after_gen {
            return;
        }
        let flags = flags.clone();
        self.pending_control = None;
        self.cc_on.clear();
        for (row, on) in view.rows.iter().zip(flags.iter()) {
            if *on {
                self.cc_on.insert(row.id, true);
            }
        }
    }

    /// Insert the selected memory's vocoder into the selected radio.
    ///
    /// The demod starts when the memory is selected. Squelch is not polled.
    /// Its stderr is the scanner identity, not a grant. While that radio
    /// is following a trunked control channel, its own receive pipe stays.
    fn apply_channel_vocoder(&mut self) {
        let rows = self.hub.view().rows;
        let selected = rows.iter().find(|row| row.selected).map(|row| row.id);
        let selected_channel = self
            .channels
            .zones
            .get(self.channel_zone)
            .and_then(|zone| zone.channels.get(self.channel_sel));
        let encrypted = selected_channel.is_some_and(|channel| channel.encrypted);
        let kind = if encrypted {
            String::new()
        } else {
            selected_channel
                .map(|channel| channels::vocoder_kind(&channel.vocoder).to_string())
                .unwrap_or_default()
        };
        let channel_invert = !encrypted && selected_channel.is_some_and(|channel| channel.invert);
        self.vocoder_sent
            .retain(|id, _| rows.iter().any(|row| row.id == *id));
        self.invert_sent
            .retain(|id, _| rows.iter().any(|row| row.id == *id));
        for row in &rows {
            let following = self.cc_on.get(&row.id).copied().unwrap_or(false)
                && !row.spec.follow
                && crate::rx_pipe::is_trunk_rx(&row.spec.rx_pipe);
            let use_kind = if Some(row.id) == selected && !following {
                kind.as_str()
            } else {
                ""
            };
            let (command, note) = match crate::rx_pipe::vocoder_command(use_kind) {
                Ok(command) => (command, String::new()),
                Err(err) => (String::new(), err),
            };
            let running = if command.is_empty() {
                row.spec.rx_pipe.as_str()
            } else {
                command.as_str()
            };
            let want_invert = Some(row.id) == selected
                && channel_invert
                && crate::rx_pipe::invert_applies(running, &kind);
            if self
                .vocoder_sent
                .get(&row.id)
                .is_some_and(|(sent, sent_note)| sent == &command && sent_note == &note)
            {
                // The demod is already the one this memory asked for.
            } else {
                self.cc_identity.remove(&row.id);
                self.hub
                    .set_vocoder(row.id, command.clone(), note.clone());
                self.vocoder_sent.insert(row.id, (command, note));
            }
            if self.invert_sent.get(&row.id) != Some(&want_invert) {
                self.cc_identity.remove(&row.id);
                self.hub.set_invert(row.id, want_invert);
                self.invert_sent.insert(row.id, want_invert);
            }
        }
    }

    /// Scanner-glass lines from a channel vocoder. They are not fed to a follower.
    fn drain_identity(&mut self) {
        let ids: Vec<u64> = self.hub.view().rows.iter().map(|row| row.id).collect();
        self.cc_identity.retain(|id, _| ids.contains(id));
        for id in ids {
            let lines = self.hub.take_identity(id);
            self.note_identity(id, &lines);
        }
    }

    /// Write follower overrides to the decoder log, and clear the readout
    /// when the held call has been let go or replaced.
    fn after_follow(&mut self, id: u64) {
        let Some(follower) = self.followers.get_mut(&id) else {
            return;
        };
        let notes = follower.take_overrides();
        let held = follower.holding().then(|| follower.active_tg()).flatten();
        let was = self.held_call.get(&id).copied().flatten();
        if was.is_some() && was != held {
            if let Some(ident) = self.cc_identity.get_mut(&id) {
                follow::clear_call(ident);
            }
        }
        self.held_call.insert(id, held);
        if !notes.is_empty() {
            self.cc_tail.entry(id).or_default().extend(notes);
        }
    }

    /// Trunking decoder lines. Only call evidence reaches the glass; a
    /// terminator clears the last call so an idle control channel is idle.
    fn note_trunk_identity(&mut self, id: u64, lines: &[String]) {
        if lines.is_empty() {
            return;
        }
        let gate = self.identity_gate.entry(id).or_default();
        let mut admitted = Vec::new();
        let mut ended = false;
        for line in lines {
            let plain = follow::plain_line(line);
            if follow::is_call_end(&plain) {
                ended = true;
                admitted.clear();
                continue;
            }
            if gate.admit(line) {
                admitted.push(line.clone());
            }
        }
        // A held call keeps its readout. A terminator inside the hang does
        // not blank it, and another talkgroup's lines do not replace it. The
        // decoder log and the radio list still get every line.
        let held = self
            .followers
            .get(&id)
            .filter(|follower| follower.holding())
            .and_then(|follower| follower.active_tg())
            .or_else(|| {
                self.hackrf_call
                    .get(&id)
                    .copied()
                    .flatten()
                    .filter(|_| self.hackrf_hold.get(&id).is_some_and(|hold| hold.active()))
                    .map(|(_, tg)| tg)
            });
        if ended && held.is_none() {
            if let Some(ident) = self.cc_identity.get_mut(&id) {
                follow::clear_call(ident);
            }
        }
        let before = self.cc_identity.get(&id).cloned();
        self.note_identity(id, &admitted);
        if let Some(tg) = held {
            if let Some(ident) = self.cc_identity.get_mut(&id) {
                if !follow::readout_keeps(ident, tg) {
                    *ident = before.unwrap_or_default();
                }
            }
        }
    }

    fn note_identity(&mut self, id: u64, lines: &[String]) {
        if lines.is_empty() {
            return;
        }
        let ident = self.cc_identity.entry(id).or_default();
        for line in lines {
            follow::note_identity(ident, line);
        }
    }

    /// Park a trunking radio on a control memory and hop to the newest clear grant.
    /// Gpredict follow keeps the radio. The decoder process stays up across hops.
    fn service_cc(&mut self, snap: &Snapshot) {
        self.sync_key_csv();
        self.apply_saved_control_follow();
        self.trunk_pool.clear();
        self.trunk_emergency.clear();
        let priority = talkgroups::priority_tgs(&self.talkgroups);
        let rows = self.hub.view().rows;
        let trunk: Vec<(u64, bool, bool, String)> = rows
            .iter()
            .filter(|row| {
                row.spec.driver != cat::RadioDriver::Hackrf && hears_trunk_pipe(row)
            })
            .map(|row| {
                (
                    row.id,
                    row.selected,
                    row.spec.follow,
                    row.spec.rx_pipe.clone(),
                )
            })
            .collect();
        let mut ids: Vec<u64> = trunk.iter().map(|row| row.0).collect();
        ids.extend(
            rows.iter()
                .filter(|row| row.spec.driver == cat::RadioDriver::Hackrf)
                .map(|row| row.id),
        );
        self.cc_on.retain(|id, _| ids.contains(id));
        self.cc_sent.retain(|id, _| ids.contains(id));
        self.cc_voice_sent.retain(|id, _| ids.contains(id));
        self.cc_mode_sent.retain(|id, _| ids.contains(id));
        self.followers.retain(|id, _| ids.contains(id));
        self.feeds.retain(|id, _| ids.contains(id));
        self.cc_tail.retain(|id, _| ids.contains(id));
        self.cc_site.retain(|id, _| ids.contains(id));

        // The selected channel zone. Other sites stay out of the hunt.
        let controls = channels::zone_controls(&self.channels, self.channel_zone);
        let control_hz: Vec<f64> = controls.iter().map(|(hz, _, _)| *hz).collect();

        for (id, selected, gpredict, pipe) in trunk {
            let on = self.cc_on.get(&id).copied().unwrap_or(false);
            let events = on && !gpredict;
            if self.cc_sent.get(&id).copied() != Some(events) {
                self.hub.set_cc(id, events);
                self.cc_sent.insert(id, events);
            }
            let lines = self.hub.take_cc_events(id);
            self.note_asked_keys(&lines);
            self.note_decoder(id, &lines);
            // The advertisement is useful with Follow off. The feed has to
            // see every line or a header and its frequency can land on
            // different wakes.
            let mut parsed = Vec::new();
            let mut feed_reset = false;
            {
                let feed = self.feeds.entry(id).or_insert_with(|| CcFeed::for_pipe(&pipe));
                if !feed.matches(&pipe) {
                    *feed = CcFeed::for_pipe(&pipe);
                    feed_reset = true;
                }
                for line in &lines {
                    parsed.extend(feed.push_line(line));
                }
            }
            if feed_reset {
                self.cc_identity.remove(&id);
            }
            self.note_voice_frames(id, &parsed);
            let mut added = false;
            if let Some(zone) = self.channels.zones.get_mut(self.channel_zone) {
                for event in &parsed {
                    if let follow::CcEvent::Adjacent { hz, rfss, site } = event {
                        if channels::add_control(zone, *hz, *rfss, *site) {
                            added = true;
                        }
                    }
                }
            }
            if self.note_trunk_events(id, &parsed) {
                added = true;
            }
            // A crash never reaches the window close. A new memory is written
            // when it is inserted, or the next launch loses it.
            if added && !self.config_path.is_empty() {
                self.save_config(snap);
            }
            if !events {
                if self.cc_voice_sent.get(&id).copied() != Some(true) {
                    self.hub.set_cc_voice(id, true);
                    self.cc_voice_sent.insert(id, true);
                }
                continue;
            }
            if selected && self.memory_scan.is_on() {
                // SCAN is walking control channels and holding an active one
                // so the adjacent-site lines stored above can arrive. A voice
                // grant must not take the dial during that listen.
                if self.cc_voice_sent.get(&id).copied() != Some(true) {
                    self.hub.set_cc_voice(id, true);
                    self.cc_voice_sent.insert(id, true);
                }
                continue;
            }
            if !self.followers.contains_key(&id) {
                self.followers
                    .insert(id, follow::Follower::new(control_hz.clone()));
            }
            if let Some(follower) = self.followers.get_mut(&id) {
                let hang = std::time::Duration::from_millis(u64::from(self.voice_hang_ms));
                if follower.hang() != hang {
                    follower.set_hang(hang);
                }
                if !follower.override_matches(self.override_hold, &priority) {
                    follower.set_override(self.override_hold, priority.clone());
                }
                if !follower.controls_match(&control_hz) {
                    follower.set_controls(control_hz.clone());
                }
            }
            let role = rows
                .iter()
                .position(|row| row.id == id)
                .map(|index| self.trunk_role_at(index))
                .unwrap_or(config::TrunkRole::Both);
            // A receiver already on a voice channel is hearing that channel,
            // not a new control-channel grant. A control receiver never leaves.
            let on_control = role == config::TrunkRole::Control
                || self
                    .followers
                    .get(&id)
                    .is_none_or(|follower| follower.role() != follow::Role::Voice);
            if on_control {
                let zones: Vec<usize> = self.monitor_zones.iter().copied().collect();
                let marked: Vec<u32> = self.monitor_talkgroups.iter().copied().collect();
                for event in &parsed {
                    if let follow::CcEvent::Emergency { hz, tg } = event {
                        if hz.is_finite() && *hz >= 1_000_000.0 {
                            self.trunk_emergency.push((id, *hz, *tg));
                        }
                    }
                    if let follow::CcEvent::Grant { hz, tg, alg } = event {
                        if !self.hear_grant(*alg, *tg, *hz) || !hz.is_finite() || *hz < 1_000_000.0
                        {
                            continue;
                        }
                        let zone = talkgroup_zone_of(&self.talkgroups, *tg);
                        if crate::hackrf::tg_selected(zone, *tg, &zones, &marked) {
                            self.trunk_pool.push((id, *hz, *tg));
                        }
                    }
                }
            }
            // A dedicated control receiver keeps hunting. Voice grants, an
            // encryption notice, and a terminator must not take its dial.
            let keyed = keys::has_filled(&self.keys);
            let parsed: Vec<follow::CcEvent> = parsed
                .into_iter()
                .filter_map(|event| {
                    if role == config::TrunkRole::Control
                        && matches!(
                            event,
                            follow::CcEvent::Grant { .. }
                                | follow::CcEvent::Alg(_)
                                | follow::CcEvent::End
                        )
                    {
                        return None;
                    }
                    match event {
                        follow::CcEvent::Grant { hz, tg, alg } => {
                            if !self.hear_grant(alg, tg, hz) {
                                return None;
                            }
                            // The hamlib follower hops a clear grant. The
                            // decoder still has the real algorithm and the
                            // key file. A non-clear Alg line must not send
                            // the radio back before that voice is played.
                            if alg != follow::CLEAR_ALG && keyed {
                                return Some(follow::CcEvent::Grant {
                                    hz,
                                    tg,
                                    alg: follow::CLEAR_ALG,
                                });
                            }
                            Some(follow::CcEvent::Grant { hz, tg, alg })
                        }
                        follow::CcEvent::Alg(alg) if alg != follow::CLEAR_ALG && keyed => None,
                        other => Some(other),
                    }
                })
                .collect();
            if let Some(follower) = self.followers.get_mut(&id) {
                let now = std::time::Instant::now();
                for event in parsed {
                    follower.push_at(event, now);
                }
                // The hang runs out on time even when the decoder is silent.
                follower.tick(now);
            }
            self.after_follow(id);
            // The S-meter on the snapshot is the selected radio only.
            // A scan of any other radio would rank channels it is not hearing.
            if selected {
                let on_frequency = self
                    .followers
                    .get(&id)
                    .and_then(|follower| follower.target_hz())
                    .is_some_and(|hz| (snap.freq_hz - hz).abs() < 500.0);
                if let Some(follower) = self.followers.get_mut(&id) {
                    follower.observe(std::time::Instant::now(), snap.strength_db, on_frequency);
                }
            }
            let Some(target) = self.followers.get(&id).and_then(|follower| follower.target_hz())
            else {
                continue;
            };
            let sent = self.followers.get(&id).and_then(|follower| follower.sent());
            let already = sent.is_some_and(|hz| (hz - target).abs() < 1.0);
            if !already {
                let close = selected && (snap.freq_hz - target).abs() < 500.0;
                let landed = if close {
                    true
                } else if selected {
                    let allowed = snap.online() && snap.caps.allow_freq(target).is_some();
                    if allowed {
                        self.send(Command::SetFreq(target));
                    }
                    allowed
                } else {
                    self.hub.tune(id, target);
                    true
                };
                if landed {
                    if let Some(follower) = self.followers.get_mut(&id) {
                        follower.note_sent(target);
                    }
                }
            }
            let watching = self
                .followers
                .get(&id)
                .is_some_and(|follower| follower.voice_hz().is_none());
            if selected && watching {
                if let Some((_, mode, _)) = controls
                    .iter()
                    .find(|(hz, _, _)| (*hz - target).abs() < 1.0)
                    .filter(|(_, mode, _)| !mode.is_empty())
                {
                    self.send_control_mode(snap, id, mode);
                }
            }
            let play = self
                .followers
                .get(&id)
                .is_some_and(|follower| follower.play_voice());
            if self.cc_voice_sent.get(&id).copied() != Some(play) {
                self.hub.set_cc_voice(id, play);
                self.cc_voice_sent.insert(id, play);
            }
        }
        self.service_hackrf(snap);
        self.rec_frame(snap);
        self.steer_voice_radios();
    }

    /// HackRF Follow. A grant inside the window is a discriminator offset.
    /// A clear selected grant outside the window retunes this radio onto
    /// that voice channel until the voice decoder finishes the call, then
    /// the control cluster returns. A control-only receiver never leaves.
    /// A control channel's terminator does not end the call.
    fn service_hackrf(&mut self, snap: &Snapshot) {
        let rows = self.hub.view().rows;
        let direct: Vec<(usize, u64, bool)> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.spec.driver == cat::RadioDriver::Hackrf)
            .map(|(index, row)| (index, row.id, row.spec.follow))
            .collect();
        let ids: Vec<u64> = direct.iter().map(|row| row.1).collect();
        self.hackrf_open.retain(|id, _| ids.contains(id));
        self.hackrf_call.retain(|id, _| ids.contains(id));
        self.hackrf_hold.retain(|id, _| ids.contains(id));
        self.hackrf_away.retain(|id, _| ids.contains(id));
        self.hackrf_cc_feeds.retain(|id, _| ids.contains(id));
        self.hackrf_voice_feed.retain(|id, _| ids.contains(id));
        self.hackrf_watch_sent.retain(|id, _| ids.contains(id));
        self.hackrf_voice_sent.retain(|id, _| ids.contains(id));
        self.hackrf_p2_sent.retain(|id, _| ids.contains(id));
        if direct.is_empty() {
            return;
        }
        let controls = channels::zone_controls(&self.channels, self.channel_zone);
        let control_hz: Vec<f64> = controls.iter().map(|(hz, _, _)| *hz).collect();
        let zones: Vec<usize> = self.monitor_zones.iter().copied().collect();
        let marked: Vec<u32> = self.monitor_talkgroups.iter().copied().collect();
        let (cluster, cluster_controls) =
            crate::hackrf::place(&control_hz).unwrap_or((0.0, Vec::new()));
        let away_now: Vec<u64> = self.hackrf_away.keys().copied().collect();
        struct Tick {
            id: u64,
            scanning: bool,
            role: config::TrunkRole,
            voice_events: Vec<follow::CcEvent>,
        }
        let mut ticks: Vec<Tick> = Vec::new();

        for (index, id, gpredict) in direct {
            let on = self.cc_on.get(&id).copied().unwrap_or(false);
            let events = on && !gpredict;
            if self.cc_sent.get(&id).copied() != Some(events) {
                self.hub.set_cc(id, events);
                self.cc_sent.insert(id, events);
            }
            let groups = self.hub.take_hackrf_controls(id);
            let voice_lines = self.hub.take_hackrf_voice(id);
            let mut logged = Vec::new();
            for lines in &groups {
                logged.extend(lines.iter().cloned());
            }
            logged.extend(voice_lines.iter().cloned());
            self.note_asked_keys(&logged);
            self.note_decoder(id, &logged);
            let mut parsed = Vec::new();
            {
                let feeds = self.hackrf_cc_feeds.entry(id).or_default();
                if feeds.len() != groups.len() {
                    *feeds = vec![follow::DsdFeed::default(); groups.len()];
                }
                for (feed, lines) in feeds.iter_mut().zip(groups) {
                    for line in lines {
                        parsed.extend(feed.push_line(&line));
                    }
                }
            }
            let mut voice_events = Vec::new();
            {
                let feed = self.hackrf_voice_feed.entry(id).or_default();
                for line in &voice_lines {
                    voice_events.extend(feed.push_line(line));
                }
            }
            self.note_voice_frames(id, &voice_events);
            let mut added = false;
            if let Some(zone) = self.channels.zones.get_mut(self.channel_zone) {
                for event in &parsed {
                    if let follow::CcEvent::Adjacent { hz, rfss, site } = event {
                        if channels::add_control(zone, *hz, *rfss, *site) {
                            added = true;
                        }
                    }
                }
            }
            if self.note_trunk_events(id, &parsed) {
                added = true;
            }
            if added && !self.config_path.is_empty() {
                self.save_config(snap);
            }
            if !events {
                self.hackrf_open.remove(&id);
                self.hackrf_call.insert(id, None);
                self.hackrf_away.remove(&id);
                self.hackrf_watch_sent.remove(&id);
                self.hackrf_voice_sent.remove(&id);
                continue;
            }
            let role = self.trunk_role_at(index);
            let scanning = rows.iter().any(|row| row.id == id && row.selected)
                && self.memory_scan.is_on();
            // Grants from a radio that has left the cluster are voice-channel
            // link control. A scan parked outside the cluster is the same.
            let parked_outside = scanning
                && self
                    .scan_memory_hz()
                    .is_some_and(|hz| !crate::hackrf::inside(cluster, hz));
            if !away_now.contains(&id) && !parked_outside {
                for event in &parsed {
                    if let follow::CcEvent::Emergency { hz, tg } = event {
                        if hz.is_finite() && *hz >= 1_000_000.0 {
                            self.trunk_emergency.push((id, *hz, *tg));
                        }
                    }
                    if let follow::CcEvent::Grant { hz, tg, alg } = event {
                        if !self.hear_grant(*alg, *tg, *hz) || !hz.is_finite() || *hz < 1_000_000.0
                        {
                            continue;
                        }
                        let zone = talkgroup_zone_of(&self.talkgroups, *tg);
                        if crate::hackrf::tg_selected(zone, *tg, &zones, &marked) {
                            self.trunk_pool.push((id, *hz, *tg));
                        }
                    }
                }
            }
            ticks.push(Tick {
                id,
                scanning,
                role,
                voice_events,
            });
        }

        for tick in ticks {
            self.hold_freq = None;
            let may_play = tick.role != config::TrunkRole::Control;
            let may_leave = may_play && !tick.scanning;
            let fresh: Vec<(f64, u32)> = if may_play {
                self.trunk_pool
                    .iter()
                    .filter(|(from, _, _)| !away_now.contains(&tick.id) || *from != tick.id)
                    .map(|(_, hz, tg)| (*hz, *tg))
                    .collect()
            } else {
                Vec::new()
            };
            // SCAN reads each control from the capture already running.
            // A memory outside the cluster is a capture of its own, and a
            // voice trip must not fight that hop.
            let scan = if tick.scanning {
                Some(match self.scan_memory_hz() {
                    Some(hz) => {
                        let (cluster, controls) = if cluster >= 1_000_000.0 {
                            (cluster, cluster_controls.clone())
                        } else {
                            (hz, vec![hz])
                        };
                        crate::hackrf::scan_capture(cluster, &controls, hz)
                    }
                    None => (cluster, cluster_controls.clone()),
                })
            } else {
                None
            };
            let keyed = keys::has_filled(&self.keys);
            let previous = self.hackrf_call.get(&tick.id).copied().flatten();
            let open = self
                .hackrf_open
                .remove(&tick.id)
                .unwrap_or_default()
                .into_iter()
                .filter(|row| {
                    let zone = talkgroup_zone_of(&self.talkgroups, row.tg);
                    self.hear_grant(follow::CLEAR_ALG, row.tg, row.hz)
                        && crate::hackrf::tg_selected(zone, row.tg, &zones, &marked)
                })
                .map(|row| (row.hz, row.tg, row.seq))
                .collect();
            let saw = |kind: fn(&follow::CcEvent) -> bool| tick.voice_events.iter().any(kind);
            // A filled key is supposed to play this call. The air algorithm
            // stays non-clear after the decoder has the key.
            let saw_end = tick.voice_events.iter().any(|event| match event {
                follow::CcEvent::End => true,
                follow::CcEvent::Alg(alg) => *alg != follow::CLEAR_ALG && !keyed,
                _ => false,
            });
            // The shared hold decides whether the call being decoded stays.
            // Control decoders keep running inside the capture and their
            // lines are logged above; only the choice of call is held.
            let now = std::time::Instant::now();
            let hang = std::time::Duration::from_millis(u64::from(self.voice_hang_ms));
            let priority = talkgroups::priority_tgs(&self.talkgroups);
            let encrypted_end = tick.voice_events.iter().any(|event| {
                matches!(event, follow::CcEvent::Alg(alg) if *alg != follow::CLEAR_ALG && !keyed)
            });
            // REC never keeps encrypted traffic, even with a filled key.
            if tick.voice_events.iter().any(|event| {
                matches!(event, follow::CcEvent::Alg(alg) if *alg != follow::CLEAR_ALG)
            }) {
                self.rec.enc.insert(tick.id);
            }
            let mut candidates: Vec<(f64, u32, bool)> =
                fresh.iter().map(|(hz, tg)| (*hz, *tg, false)).collect();
            for (from, hz, tg) in &self.trunk_emergency {
                if !away_now.contains(&tick.id) || *from != tick.id {
                    candidates.push((*hz, *tg, true));
                }
            }
            let step = {
                let hold = self.hackrf_hold.entry(tick.id).or_default();
                if hold.hang() != hang {
                    hold.set_hang(hang);
                }
                if !hold.override_matches(self.override_hold, &priority) {
                    hold.set_override(self.override_hold, priority);
                }
                if may_play && !tick.scanning {
                    let reachable = |hz: f64| may_leave || crate::hackrf::inside(cluster, hz);
                    follow::wide_hold_step(
                        hold,
                        previous,
                        &tick.voice_events,
                        encrypted_end,
                        &candidates,
                        &reachable,
                        now,
                    )
                } else {
                    hold.clear();
                    follow::WideHold::Free
                }
            };
            let mut open = open;
            let (fresh, sync, lost, end, search) = crate::hackrf::hold_inputs(
                &step,
                fresh,
                (
                    saw(|event| matches!(event, follow::CcEvent::Sync)),
                    saw(|event| matches!(event, follow::CcEvent::SyncLost)),
                    saw_end,
                    saw(|event| matches!(event, follow::CcEvent::Search)),
                ),
                &mut open,
                self.hackrf_seq,
                previous,
            );
            let picked = crate::hackrf::follow_capture(
                cluster,
                &cluster_controls,
                scan,
                may_play,
                may_leave,
                open,
                self.hackrf_seq,
                previous,
                &fresh,
                self.hackrf_away.get(&tick.id).copied(),
                sync,
                lost,
                end,
                search,
            );
            if picked.call != previous {
                if let Some(ident) = self.cc_identity.get_mut(&tick.id) {
                    follow::clear_call(ident);
                }
                if let Some(hold) = self.hackrf_hold.get_mut(&tick.id) {
                    if let Some(call) = picked.call {
                        hold.begin_call(call, now);
                    } else {
                        hold.clear();
                    }
                }
            }
            if let Some(hold) = self.hackrf_hold.get_mut(&tick.id) {
                let notes = hold.take_overrides();
                if !notes.is_empty() {
                    self.cc_tail.entry(tick.id).or_default().extend(notes);
                }
            }
            self.hackrf_seq = picked.seq;
            self.hackrf_open.insert(
                tick.id,
                picked
                    .open
                    .into_iter()
                    .map(|(hz, tg, seq)| HeardGrant { hz, tg, seq })
                    .collect(),
            );
            self.hackrf_call.insert(tick.id, picked.call);
            if let Some(away) = picked.away {
                self.hackrf_away.insert(tick.id, away);
            } else {
                self.hackrf_away.remove(&tick.id);
            }
            let sig = (
                picked.center.round() as i64,
                picked
                    .controls
                    .iter()
                    .map(|hz| hz.round() as i64)
                    .collect::<Vec<_>>(),
            );
            if self.hackrf_watch_sent.get(&tick.id) != Some(&sig) {
                self.hub.hackrf_watch(tick.id, picked.center, picked.controls);
                self.hackrf_watch_sent.insert(tick.id, sig);
            }
            if self.hackrf_voice_sent.get(&tick.id).copied() != Some(picked.voice_hz) {
                self.hub.hackrf_voice(tick.id, picked.voice_hz);
                self.hackrf_voice_sent.insert(tick.id, picked.voice_hz);
            }
        }
    }

    /// Role stored for this radio index. A radio with no entry is both the
    /// control receiver and the voice receiver.
    fn trunk_role_at(&self, index: usize) -> config::TrunkRole {
        self.trunk_receivers
            .iter()
            .find(|entry| entry.radio == index)
            .map(|entry| entry.role)
            .unwrap_or(config::TrunkRole::Both)
    }

    /// Follow off, or Control / Voice / Both. Both is the default and is
    /// not written. Control and Voice are written on the station file.
    fn set_trunk_role(
        &mut self,
        index: usize,
        role: Option<config::TrunkRole>,
        snap: &Snapshot,
    ) {
        let rows = self.hub.view().rows;
        let Some(row) = rows.get(index) else {
            return;
        };
        let id = row.id;
        self.trunk_receivers.retain(|entry| entry.radio != index);
        match role {
            None => {
                self.cc_on.insert(id, false);
                self.hackrf_away.remove(&id);
            }
            Some(config::TrunkRole::Both) => {
                self.cc_on.insert(id, true);
            }
            Some(role) => {
                self.cc_on.insert(id, true);
                if role == config::TrunkRole::Control {
                    self.hackrf_away.remove(&id);
                }
                self.trunk_receivers
                    .push(config::TrunkReceiver { radio: index, role });
                self.trunk_receivers.sort_by_key(|entry| entry.radio);
            }
        }
        if !self.config_path.is_empty() {
            self.save_config(snap);
        }
    }

    /// A hamlib radio marked Voice takes one grant from a receiver that is
    /// still on the control channel. Its own decoder ends the hop. A radio
    /// that has not learned a channel map stays on the control channel.
    fn steer_voice_radios(&mut self) {
        let rows = self.hub.view().rows;
        let grants = self.trunk_pool.clone();
        let emergencies = self.trunk_emergency.clone();
        for (index, row) in rows.iter().enumerate() {
            if row.spec.driver == cat::RadioDriver::Hackrf || !hears_trunk_pipe(row) {
                continue;
            }
            if row.spec.follow || !self.cc_on.get(&row.id).copied().unwrap_or(false) {
                continue;
            }
            if self.trunk_role_at(index) != config::TrunkRole::Voice {
                continue;
            }
            if row.selected && self.memory_scan.is_on() {
                continue;
            }
            let Some((_, hz, tg)) = grants.iter().rev().find(|(from, _, _)| *from != row.id) else {
                continue;
            };
            let Some(follower) = self.followers.get_mut(&row.id) else {
                continue;
            };
            if follower.role() == follow::Role::Voice {
                // A held call stays. Only an emergency or a priority talkgroup
                // from another receiver may take it.
                if let Some((_, ehz, etg)) =
                    emergencies.iter().rev().find(|(from, _, _)| *from != row.id)
                {
                    follower.push(follow::CcEvent::Emergency { hz: *ehz, tg: *etg });
                } else {
                    follower.push(follow::CcEvent::Grant {
                        hz: *hz,
                        tg: *tg,
                        alg: follow::CLEAR_ALG,
                    });
                }
                self.after_follow(row.id);
                continue;
            }
            if follower.voice_hz().is_some() {
                continue;
            }
            follower.push(follow::CcEvent::Grant {
                hz: *hz,
                tg: *tg,
                alg: follow::CLEAR_ALG,
            });
        }
    }

    /// One mode command for the control memory. A later frame does not repeat it.
    fn send_control_mode(&mut self, snap: &Snapshot, id: u64, mode: &str) {
        if !snap.online() || !snap.caps.can_set_mode {
            return;
        }
        if snap.mode.eq_ignore_ascii_case(mode) {
            self.cc_mode_sent.insert(id, mode.to_string());
            return;
        }
        if self.cc_mode_sent.get(&id).is_some_and(|sent| sent == mode) {
            return;
        }
        self.cc_mode_sent.insert(id, mode.to_string());
        self.send(Command::SetMode {
            mode: mode.to_string(),
            passband_hz: snap.passband_hz,
        });
    }

    /// Write a radio-id description when the field is left.
    /// The exit prompt writes it too when the operator saves. A crash does not.
    fn save_unit_description(&mut self, snap: &Snapshot) {
        if !self.unit_desc_dirty {
            return;
        }
        self.unit_desc_dirty = false;
        if self.config_path.is_empty() {
            return;
        }
        self.save_config(snap);
    }

    fn page_radio_ids(&mut self, ui: &mut egui::Ui, id: u64, snap: &Snapshot) {
        let rows = self.hub.view().rows;
        let Some(row) = rows.iter().find(|row| row.id == id) else {
            ui.label("This radio is gone.");
            return;
        };
        ui.label(format!(
            "{} radio IDs are stored in the station file.",
            row.spec.name
        ));
        if self.units.is_empty() {
            ui.label("No radio IDs yet. A unit named on this control channel is added here.");
            return;
        }
        if self.unit_sel >= self.units.len() {
            self.unit_sel = self.units.len() - 1;
        }
        ui.add_space(6.0);
        let gap = ui.spacing().item_spacing.x;
        let editor_w = 320.0;
        let list_w = (ui.available_width() - gap - editor_w).max(220.0);
        let height = ui.available_height().max(80.0);
        let mut picked = None;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(list_w);
                egui::ScrollArea::vertical()
                    .id_salt(("radio-ids", id))
                    .auto_shrink([false, false])
                    .max_height(height)
                    .show(ui, |ui| {
                        let count = self.units.len();
                        for idx in 0..count {
                            if self.unit_row(ui, idx) {
                                picked = Some(idx);
                            }
                        }
                    });
            });
            ui.vertical(|ui| {
                ui.set_width(editor_w);
                self.unit_editor(ui, snap);
            });
        });
        if let Some(idx) = picked {
            if self.unit_sel != idx {
                self.save_unit_description(snap);
                self.unit_sel = idx;
            }
        }
    }

    /// Selecting a row does not tune. The facts are what the control channel printed.
    fn unit_row(&self, ui: &mut egui::Ui, idx: usize) -> bool {
        let Some(row) = self.units.get(idx) else {
            return false;
        };
        let facts = units::facts(row);
        let text = if facts.is_empty() {
            format!("{:>8}   {}", row.id, row.description)
        } else {
            format!("{:>8}   {}   {facts}", row.id, row.description)
        };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        if self.unit_sel == idx {
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(4),
                HAM.gamma_multiply(0.28),
            );
        } else if idx % 2 == 1 {
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::ZERO,
                ui.visuals().faint_bg_color,
            );
        }
        ui.painter().text(
            rect.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &text,
            egui::FontId::monospace(14.0),
            ui.visuals().text_color(),
        );
        response.on_hover_text(&text).clicked()
    }

    fn unit_editor(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        let sel = self.unit_sel;
        let Some(row) = self.units.get(sel) else {
            ui.label("No radio ID.");
            return;
        };
        let unit_id = row.id;
        let mut description = row.description.clone();
        let fact_line = units::facts(row);
        let response = ui
            .horizontal(|ui| {
                ui.label("Description");
                ui.add(
                    egui::TextEdit::singleline(&mut description)
                        .id_salt(("unit-desc", unit_id))
                        .desired_width(180.0),
                )
            })
            .inner;
        if response.changed() {
            if let Some(row) = self.units.get_mut(sel) {
                row.description = description;
                self.unit_desc_dirty = true;
            }
        }
        let enter = response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if response.lost_focus() || enter {
            self.save_unit_description(snap);
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new(format!("Radio {unit_id}")).strong());
        if fact_line.is_empty() {
            ui.label("No extra detail yet.");
            return;
        }
        for part in fact_line.split("  ") {
            ui.label(part);
        }
    }

    /// Wide scanner face for one P25 radio. The decoder log stays on Config.
    fn scanner_face(
        &self,
        ui: &mut egui::Ui,
        id: u64,
        selected: bool,
        radio_name: &str,
        follow: bool,
        direct: bool,
        snap: &Snapshot,
    ) {
        fn clip(ui: &egui::Ui, text: &str, font: egui::FontId, max_w: f32) -> String {
            if text.is_empty() || max_w <= 8.0 {
                return String::new();
            }
            let fits = |sample: &str| {
                ui.painter()
                    .layout_no_wrap(sample.to_string(), font.clone(), egui::Color32::WHITE)
                    .size()
                    .x
                    <= max_w
            };
            if fits(text) {
                return text.to_string();
            }
            let chars: Vec<char> = text.chars().collect();
            let mut kept = chars.len();
            while kept > 0 {
                kept -= 1;
                let mut short: String = chars.iter().take(kept).collect();
                short.push('…');
                if fits(&short) {
                    return short;
                }
            }
            "…".into()
        }
        fn alias_parts(name: &str) -> (String, String) {
            for sep in [" — ", " – ", " - "] {
                if let Some((left, right)) = name.split_once(sep) {
                    return (left.trim().to_string(), right.trim().to_string());
                }
            }
            (name.trim().to_string(), String::new())
        }

        let (status, tg, live, encrypted, hz) = if direct {
            let call = self.hackrf_call.get(&id).copied().flatten();
            let tg = call.map(|(_, tg)| tg);
            let live = tg.is_some();
            let control_hz = if selected && self.memory_scan.is_on() {
                self.scan_memory_hz()
            } else {
                None
            }
            .or_else(|| {
                channels::zone_controls(&self.channels, self.channel_zone)
                    .first()
                    .map(|(hz, _, _)| *hz)
            });
            let hz = call.map(|(hz, _)| hz).or(control_hz);
            let status = if selected && self.memory_scan.is_on() {
                "scanning".to_string()
            } else if !follow {
                "follow is off".to_string()
            } else if let Some(tg) = tg {
                if self.hackrf_hold.get(&id).is_some_and(|hold| hold.in_hang()) {
                    format!("voice talkgroup {tg} · HANG")
                } else {
                    format!("voice talkgroup {tg}")
                }
            } else if self.cc_site.contains_key(&id) {
                "locked".to_string()
            } else {
                "watching".to_string()
            };
            let encrypted = tg.is_some_and(|tg| self.talkgroup_encrypted(tg));
            (status, tg, live, encrypted, hz)
        } else if let Some(follower) = self.followers.get(&id) {
            let active = follower.active_tg();
            let heard = follower.heard().cloned();
            let status = follower.status().to_string();
            let live = active.is_some();
            let tg = active.or_else(|| heard.as_ref().map(|heard| heard.tg));
            let encrypted = heard
                .as_ref()
                .is_some_and(|heard| heard.alg != follow::CLEAR_ALG);
            let hz = if live {
                follower.voice_hz()
            } else {
                None
            }
            .or_else(|| follower.shown_control_hz());
            (status, tg, live, encrypted, hz)
        } else if follow {
            ("watching".to_string(), None, false, false, None)
        } else {
            ("follow is off".to_string(), None, false, false, None)
        };
        // A grant is not audio. Say so rather than look like live voice.
        let status = follow::call_status(
            status,
            live,
            encrypted,
            self.hearing_voice(id, std::time::Instant::now()),
        );
        let system = self
            .channels
            .zones
            .get(self.channel_zone)
            .map(|zone| zone.name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| radio_name.to_string());
        let department = tg.and_then(|tg| {
            let selected_zone = self.talkgroups.zones.get(self.talkgroup_zone);
            if selected_zone.is_some_and(|zone| zone.talkgroups.iter().any(|row| row.tg == tg)) {
                return selected_zone.map(|zone| zone.name.clone());
            }
            self.talkgroups
                .zones
                .iter()
                .find(|zone| zone.talkgroups.iter().any(|row| row.tg == tg))
                .map(|zone| zone.name.clone())
        });
        let (alias, detail) = match tg {
            Some(tg) => {
                let name = talkgroups::name_of(&self.talkgroups, tg)
                    .unwrap_or_else(|| "not in this directory".to_string());
                alias_parts(&name)
            }
            None => (String::new(), String::new()),
        };
        let site = self.cc_site.get(&id).cloned().unwrap_or_default();
        let mode = {
            let controls = channels::zone_controls(&self.channels, self.channel_zone);
            let stored = hz
                .and_then(|hz| {
                    controls
                        .iter()
                        .find(|(item, _, _)| (*item - hz).abs() < 1.0)
                        .map(|(_, mode, _)| mode.clone())
                })
                .or_else(|| controls.first().map(|(_, mode, _)| mode.clone()))
                .unwrap_or_default();
            if stored.trim().is_empty() {
                "P25".to_string()
            } else {
                stored
            }
        };
        let clock = self
            .system_clock
            .face_at(clock::unix_now_ms())
            .map(|read| {
                let star = if read.zone_half { "*" } else { "" };
                format!(
                    "{}  {:02}:{:02}{}{star}",
                    read.date, read.hour, read.minute, read.zone
                )
            });
        let rssi = if selected && snap.online() {
            if snap.model == "GQRX" {
                strength_label(gqrx_dbfs_to_s9(snap.strength_db))
            } else if snap.model == "HackRF" {
                let spec = self.hub.spectrum();
                let at = hz.unwrap_or(spec.center_hz);
                let db = crate::hackrf::channel_db(&spec.db, spec.center_hz, at);
                if db.is_finite() {
                    format!("{db:.0} dB")
                } else {
                    "—".to_string()
                }
            } else if snap.strength_db.is_finite() {
                format!("{:.0} dB", snap.strength_db)
            } else {
                "—".to_string()
            }
        } else {
            "—".to_string()
        };
        let freq = hz.map(|hz| format!("{:.6} MHz", hz / 1_000_000.0));
        let (mut headline, mut headline_live) = if tg.is_some() && !alias.is_empty() {
            (alias, live)
        } else if status.starts_with("scanning") {
            ("SCAN".to_string(), false)
        } else {
            (status.clone(), false)
        };
        let ident = self.cc_identity.get(&id).cloned().unwrap_or_default();
        let glass = follow::glass_rows(if live { tg } else { None }, &ident);
        let heard_identity = glass.headline.is_some() || glass.tgid.is_some();
        if let Some(text) = glass.headline.clone() {
            headline = text;
            headline_live = true;
        }
        let mut detail = detail;
        let mut encrypted = encrypted;
        let shown_tg = if live { tg } else { glass.tgid.or(tg) };
        if !live {
            if let Some(text) = glass.detail.clone() {
                detail = text;
            }
            if heard_identity {
                encrypted = glass.encrypted;
            }
        }

        let width = ui.available_width();
        let (outer, _) = ui.allocate_exact_size(egui::vec2(width, 204.0), egui::Sense::hover());
        if !ui.is_rect_visible(outer) {
            return;
        }
        let orange = egui::Color32::from_rgb(232, 128, 42);
        let yellow = egui::Color32::from_rgb(245, 214, 64);
        let held = egui::Color32::from_rgb(168, 146, 48);
        let paper = egui::Color32::from_rgb(226, 222, 206);
        let dim = egui::Color32::from_rgb(148, 146, 132);
        let painter = ui.painter();
        painter.rect_filled(
            outer,
            egui::CornerRadius::same(8),
            egui::Color32::from_rgb(42, 42, 46),
        );
        let screen = outer.shrink(7.0);
        painter.rect_filled(
            screen,
            egui::CornerRadius::same(3),
            egui::Color32::from_rgb(8, 8, 6),
        );
        let left = screen.left() + 12.0;
        let right = screen.right() - 12.0;
        let inner = (right - left).max(8.0);
        let mut y = screen.top() + 8.0;
        let status_font = egui::FontId::monospace(13.0);
        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            &clip(ui, &status, status_font.clone(), inner * 0.55),
            status_font,
            dim,
        );
        if let Some(clock) = clock {
            let font = egui::FontId::monospace(13.0);
            painter.text(
                egui::pos2(right, y),
                egui::Align2::RIGHT_TOP,
                &clip(ui, &clock, font.clone(), inner * 0.42),
                font,
                dim,
            );
        }
        y += 18.0;
        let system_font = egui::FontId::monospace(22.0);
        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            &clip(ui, &system, system_font.clone(), inner),
            system_font,
            orange,
        );
        y += 28.0;
        let dept_font = egui::FontId::monospace(16.0);
        let dept = department.unwrap_or_else(|| "—".to_string());
        // Mode sits on this row, as on the BCD325P2. A line under the
        // WACN row falls off the bottom of the glass.
        let mode_w = (inner * 0.28).max(48.0);
        painter.text(
            egui::pos2(right, y),
            egui::Align2::RIGHT_TOP,
            &clip(ui, &mode, dept_font.clone(), mode_w),
            dept_font.clone(),
            orange,
        );
        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            &clip(ui, &dept, dept_font.clone(), (inner - mode_w - 12.0).max(8.0)),
            dept_font,
            orange,
        );
        y += 22.0;
        let head_font = egui::FontId::monospace(26.0);
        let head_color = if headline_live { yellow } else { held };
        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            &clip(ui, &headline, head_font.clone(), inner),
            head_font,
            head_color,
        );
        y += 32.0;
        let id_font = egui::FontId::monospace(15.0);
        let tgid = match shown_tg {
            Some(tg) => format!("TGID {tg}"),
            None => "TGID —".to_string(),
        };
        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            &clip(ui, &tgid, id_font.clone(), inner * 0.7),
            id_font.clone(),
            paper,
        );
        if encrypted {
            painter.text(
                egui::pos2(right, y),
                egui::Align2::RIGHT_TOP,
                "ENC",
                id_font,
                egui::Color32::from_rgb(220, 72, 48),
            );
        }
        y += 20.0;
        if !detail.is_empty() {
            let font = egui::FontId::monospace(15.0);
            painter.text(
                egui::pos2(left, y),
                egui::Align2::LEFT_TOP,
                &clip(ui, &detail, font.clone(), inner),
                font,
                paper,
            );
        }
        y += 20.0;
        let fact_font = egui::FontId::monospace(14.0);
        let sys = if site.sys.is_empty() {
            "—".to_string()
        } else {
            site.sys.clone()
        };
        let site_id = if site.site.is_empty() {
            "—".to_string()
        } else {
            site.site.clone()
        };
        let rfss = if site.rfss.is_empty() {
            "—".to_string()
        } else {
            site.rfss.clone()
        };
        let facts = glass
            .facts
            .clone()
            .unwrap_or_else(|| format!("Sys {sys}   RFSS {rfss}   Site {site_id}"));
        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            &clip(ui, &facts, fact_font.clone(), inner * 0.58),
            fact_font.clone(),
            paper,
        );
        if let Some(freq) = freq {
            painter.text(
                egui::pos2(right, y),
                egui::Align2::RIGHT_TOP,
                &clip(ui, &freq, fact_font.clone(), inner * 0.4),
                fact_font.clone(),
                paper,
            );
        }
        y += 20.0;
        let wacn = if site.wacn.is_empty() {
            "—".to_string()
        } else {
            site.wacn.clone()
        };
        let nac = if site.nac.is_empty() {
            "—".to_string()
        } else {
            site.nac.clone()
        };
        let net = if !site.wacn.is_empty() || !site.nac.is_empty() {
            format!("WACN {wacn}    NAC {nac}")
        } else if let Some(code) = &glass.code_line {
            code.clone()
        } else {
            format!("WACN {wacn}    NAC {nac}")
        };
        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            &clip(ui, &net, fact_font.clone(), inner * 0.58),
            fact_font.clone(),
            paper,
        );
        let signal = format!("RSSI {rssi}");
        painter.text(
            egui::pos2(right, y),
            egui::Align2::RIGHT_TOP,
            &clip(ui, &signal, fact_font.clone(), inner * 0.4),
            fact_font,
            paper,
        );
    }

    /// Every station radio, so a second receiver can stay on the control
    /// channel while another follows a voice call outside the HackRF window.
    fn trunk_receiver_strip(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        let rows = self.hub.view().rows;
        if rows.is_empty() {
            return;
        }
        ui.add_space(8.0);
        ui.label("Receivers");
        ui.label(
            egui::RichText::new(
                "One dongle cannot watch the control channel and a far voice channel at the same time. Add a second receiver to keep a dedicated control channel.",
            )
            .weak()
            .small(),
        );
        let choices = [
            ("Off", None),
            ("Control", Some(config::TrunkRole::Control)),
            ("Voice", Some(config::TrunkRole::Voice)),
            ("Both", Some(config::TrunkRole::Both)),
        ];
        let mut pick = None;
        for (index, row) in rows.iter().enumerate() {
            let trunk = row.spec.driver == cat::RadioDriver::Hackrf || hears_trunk_pipe(row);
            let cc = self.cc_on.get(&row.id).copied().unwrap_or(false);
            let current = if cc {
                Some(self.trunk_role_at(index))
            } else {
                None
            };
            ui.horizontal(|ui| {
                for (label, role) in choices {
                    let response = ui
                        .add_enabled(trunk, egui::Button::new(label).selected(current == role))
                        .on_hover_text(if !trunk {
                            "This radio has no trunking receive pipe."
                        } else if role == Some(config::TrunkRole::Control) {
                            "Stays on the control channel and does not play voice. Pair it with a Voice receiver. One dongle cannot do both jobs."
                        } else if role == Some(config::TrunkRole::Voice) {
                            "Plays voice. A clear call outside the 6 MHz window leaves the control channel until that call ends. A hamlib radio still has to learn the channel map on a control channel before it can hop."
                        } else if role == Some(config::TrunkRole::Both) {
                            "Decodes the control channel and plays voice. A call outside the 6 MHz window drops the control channel until that call ends."
                        } else {
                            "Follow is off. This radio does not decode the control channel."
                        });
                    if response.clicked() && trunk && current != role {
                        pick = Some((index, role));
                    }
                }
                ui.label(&row.spec.name);
                if let Some(away) = self.hackrf_away.get(&row.id) {
                    ui.label(
                        egui::RichText::new(format!("on voice {:.4} MHz", away.hz / 1_000_000.0))
                            .color(caution()),
                    );
                }
            });
        }
        if let Some((index, role)) = pick {
            self.set_trunk_role(index, role, snap);
        }
    }

    fn page_keys(&mut self, ui: &mut egui::Ui) {
        if self.config_path.is_empty() {
            ui.label("The station has not been saved.");
            return;
        }
        ui.label(keys::path_beside(Path::new(&self.config_path)).display().to_string());
        if !self.key_note.is_empty() {
            ui.label(
                egui::RichText::new(&self.key_note).color(caution()),
            );
        }
        ui.horizontal(|ui| {
            if ui.button("Add").clicked() {
                self.key_sel = keys::add_placeholder(&mut self.keys);
                self.key_shown = None;
                self.save_keys();
            }
            let can_remove = self.key_sel < self.keys.keys.len();
            if ui.add_enabled(can_remove, egui::Button::new("Remove")).clicked() {
                self.keys.keys.remove(self.key_sel);
                if self.key_sel >= self.keys.keys.len() && self.key_sel > 0 {
                    self.key_sel -= 1;
                }
                self.key_shown = None;
                self.save_keys();
            }
            if ui
                .button("Import")
                .on_hover_text("A dsd-fme hex CSV. The first line is the header.")
                .clicked()
            {
                self.browse_keys();
            }
            if ui
                .button("Export")
                .on_hover_text("A dsd-fme hex CSV. The first line is the header.")
                .clicked()
            {
                self.export_keys();
            }
        });
        if self.keys.keys.is_empty() {
            ui.label("No keys. A key id the decoder prints is added here with blank text.");
            if ui
                .button("Generate")
                .on_hover_text("Random DES key for local debugging. Change the algorithm and generate again for RC4 or AES-256.")
                .clicked()
            {
                self.generate_selected_key();
            }
            return;
        }
        if self.key_sel >= self.keys.keys.len() {
            self.key_sel = 0;
            self.key_shown = None;
        }
        if self.key_shown != Some(self.key_sel) {
            if let Some(row) = self.keys.keys.get(self.key_sel) {
                self.key_id_edit = row.key_id.clone();
                self.key_alg_edit = row.alg.clone();
                self.key_name_edit = row.name.clone();
                self.key_text_edit = row.text.clone();
            }
            self.key_shown = Some(self.key_sel);
        }
        let mut order: Vec<usize> = (0..self.keys.keys.len()).collect();
        order.sort_by_key(|index| {
            let row = &self.keys.keys[*index];
            (row.is_filled(), row.key_id.clone())
        });
        ui.add_space(8.0);
        let gap = ui.spacing().item_spacing.x;
        let widths = fit_columns(ui.available_width(), gap, &[460.0, 320.0], 0);
        let list_w = widths[0];
        let editor_w = widths[1];
        let height = ui.available_height().max(80.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(list_w);
                key_list_header(ui);
                egui::ScrollArea::vertical()
                    .id_salt("key-list")
                    .auto_shrink([false, false])
                    .max_height((height - 22.0).max(40.0))
                    .show(ui, |ui| {
                        for (stripe, index) in order.iter().copied().enumerate() {
                            if self.key_row(ui, index, stripe) {
                                self.key_sel = index;
                            }
                        }
                    });
            });
            ui.vertical(|ui| {
                ui.set_width(editor_w);
                egui::ScrollArea::vertical()
                    .id_salt("key-editor")
                    .auto_shrink([false, true])
                    .max_height(height)
                    .show(ui, |ui| {
                        ui.label("Key ID");
                        ui.text_edit_singleline(&mut self.key_id_edit);
                        ui.label("Algorithm");
                        let current = keys::alg_label(&self.key_alg_edit);
                        egui::ComboBox::from_id_salt("key-alg")
                            .selected_text(current)
                            .show_ui(ui, |ui| {
                                for (alg, label) in keys::ALG_CHOICES {
                                    ui.selectable_value(
                                        &mut self.key_alg_edit,
                                        (*alg).to_string(),
                                        *label,
                                    );
                                }
                                if keys::expected_len(&self.key_alg_edit).is_none() {
                                    let alg = self.key_alg_edit.clone();
                                    ui.selectable_value(&mut self.key_alg_edit, alg.clone(), alg);
                                }
                            });
                        ui.label("Name");
                        ui.text_edit_singleline(&mut self.key_name_edit);
                        ui.label("Key text");
                        ui.text_edit_multiline(&mut self.key_text_edit);
                        ui.horizontal(|ui| {
                            if ui.button("Save").clicked() {
                                match keys::apply_edit(
                                    &mut self.keys,
                                    self.key_sel,
                                    &self.key_id_edit,
                                    &self.key_alg_edit,
                                    &self.key_name_edit,
                                    &self.key_text_edit,
                                ) {
                                    Ok(()) => {
                                        self.key_shown = None;
                                        self.save_keys();
                                    }
                                    Err(err) => self.key_note = err,
                                }
                            }
                            if ui
                                .button("Generate")
                                .on_hover_text("Replaces the key text with random hex for the algorithm above and saves it. For a radio you program yourself.")
                                .clicked()
                            {
                                self.generate_selected_key();
                            }
                        });
                    });
            });
        });
    }

    /// One key row. Same stripe and selection fill as a channel row.
    /// Missing stays amber in the status column. The key text stays off the list.
    fn key_row(&self, ui: &mut egui::Ui, index: usize, stripe: usize) -> bool {
        let Some(row) = self.keys.keys.get(index) else {
            return false;
        };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        let painter = ui.painter();
        if self.key_sel == index {
            painter.rect_filled(rect, egui::CornerRadius::same(4), HAM.gamma_multiply(0.28));
        } else if stripe % 2 == 1 {
            painter.rect_filled(rect, egui::CornerRadius::ZERO, ui.visuals().faint_bg_color);
        }
        let font = egui::FontId::monospace(14.0);
        let normal = ui.visuals().text_color();
        let amber = caution();
        let filled = row.is_filled();
        let alg = keys::alg_label(&row.alg);
        let status = if filled { "key" } else { "missing" };
        let cells = [row.key_id.as_str(), alg.as_str(), status, row.name.as_str()];
        for (slot, text) in cells.iter().enumerate() {
            if text.is_empty() {
                continue;
            }
            let color = if slot == 2 && !filled { amber } else { normal };
            painter.text(
                rect.left_center() + egui::vec2(KEY_LIST_X[slot], 0.0),
                egui::Align2::LEFT_CENTER,
                text,
                font.clone(),
                color,
            );
        }
        response.clicked()
    }

    /// Fill the selected row with a random key and write the key file.
    /// An empty list gets one DES row. A DMR placeholder is refused.
    fn generate_selected_key(&mut self) {
        if self.config_path.is_empty() {
            self.key_note = "The station has not been saved.".into();
            return;
        }
        if self.keys.keys.is_empty() {
            self.key_sel = keys::add_placeholder(&mut self.keys);
            let row = &self.keys.keys[self.key_sel];
            self.key_id_edit = row.key_id.clone();
            self.key_alg_edit = row.alg.clone();
            self.key_name_edit.clear();
            self.key_text_edit.clear();
            self.key_shown = Some(self.key_sel);
        }
        let text = match keys::generate_text(&self.key_alg_edit) {
            Ok(text) => text,
            Err(err) => {
                self.key_note = err;
                return;
            }
        };
        self.key_text_edit = text;
        match keys::apply_edit(
            &mut self.keys,
            self.key_sel,
            &self.key_id_edit,
            &self.key_alg_edit,
            &self.key_name_edit,
            &self.key_text_edit,
        ) {
            Ok(()) => {
                self.key_shown = None;
                self.save_keys();
            }
            Err(err) => self.key_note = err,
        }
    }

    fn page_talkgroups(&mut self, ui: &mut egui::Ui, id: u64, snap: &Snapshot) {
        let rows = self.hub.view().rows;
        let Some(row) = rows.iter().find(|row| row.id == id) else {
            ui.label("This radio is gone.");
            return;
        };
        let direct = row.spec.driver == cat::RadioDriver::Hackrf;
        let trunk = direct || hears_trunk_pipe(row);
        let selected = row.selected;
        let radio_name = row.spec.name.clone();
        let gpredict = row.spec.follow;
        let mut follow = trunk && self.cc_on.get(&id).copied().unwrap_or(false);
        if !self.talkgroup_note.is_empty() {
            ui.label(
                egui::RichText::new(&self.talkgroup_note)
                    .color(caution()),
            );
        }
        let active = if direct {
            self.hackrf_call.get(&id).and_then(|call| call.map(|(_, tg)| tg))
        } else {
            self.followers.get(&id).and_then(|follower| follower.active_tg())
        };
        ui.horizontal_top(|ui| {
            // The face is 204 px. Three 46 px keys with 6 px between them
            // sit on the vertical middle of that bezel, 4 px from its edge.
            ui.spacing_mut().item_spacing.x = 4.0;
            let face_h = 204.0_f32;
            let key_h = 46.0;
            let key_gap = 6.0;
            let stack = key_h * 4.0 + key_gap * 3.0;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.set_width(72.0);
                ui.add_space(((face_h - stack) * 0.5).max(0.0));
                self.rec_key(ui, true);
                ui.add_space(key_gap);
                if radio_key(ui, "LOAD", KeyLamp::Dark, true).clicked() {
                    self.browse_talkgroups(false);
                }
                ui.add_space(key_gap);
                if radio_key(ui, "SAVE", KeyLamp::Dark, true).clicked() {
                    self.browse_talkgroups(true);
                }
                ui.add_space(key_gap);
                let toggle = radio_key(
                    ui,
                    "FOLLOW",
                    if follow { KeyLamp::Green } else { KeyLamp::Dark },
                    trunk,
                );
                if toggle.clicked() && trunk {
                    follow = !follow;
                }
                if !trunk {
                    toggle.on_hover_text("The trunking receive pipe is off.");
                } else if gpredict {
                    toggle.on_hover_text("Gpredict is moving this radio.");
                }
            });
            self.scanner_face(ui, id, selected, &radio_name, follow, direct, snap);
        });
        if self.cc_on.get(&id).copied().unwrap_or(false) != follow {
            self.cc_on.insert(id, follow);
            if !follow {
                if let Some(index) = rows.iter().position(|row| row.id == id) {
                    self.trunk_receivers.retain(|entry| entry.radio != index);
                }
            }
            if !self.config_path.is_empty() {
                self.save_config(snap);
            }
        }
        self.trunk_receiver_strip(ui, snap);
        if self.talkgroups.zones.is_empty() {
            ui.label("No zones in this talkgroup file.");
            return;
        }
        if self.talkgroup_zone >= self.talkgroups.zones.len() {
            self.talkgroup_zone = 0;
            self.talkgroup_sel = 0;
        }
        let count = self.talkgroups.zones[self.talkgroup_zone].talkgroups.len();
        if count == 0 {
            self.talkgroup_sel = 0;
        } else if self.talkgroup_sel >= count {
            self.talkgroup_sel = count - 1;
        }
        ui.add_space(6.0);
        let gap = ui.spacing().item_spacing.x;
        let zone_w = 280.0;
        let editor_w = 320.0;
        let total_w = ui.available_width();
        let list_w = (total_w - gap * 2.0 - zone_w - editor_w).max(220.0);
        let editor_w = (total_w - gap * 2.0 - zone_w - list_w).max(0.0);
        let height = ui.available_height().max(80.0);
        let recent_now = std::time::Instant::now();
        if self.tg_recent.any(recent_now) {
            // The tint decays; repaint slowly so it fades without a busy loop.
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
        }
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(zone_w);
                egui::ScrollArea::vertical()
                    .id_salt("tg-zone-list")
                    .auto_shrink([false, false])
                    .max_height(height)
                    .show(ui, |ui| {
                        let count = self.talkgroups.zones.len();
                        for idx in 0..count {
                            let name = self.talkgroups.zones[idx].name.clone();
                            let selected = self.talkgroup_zone == idx;
                            let decoding = active.is_some_and(|tg| {
                                self.talkgroups.zones[idx]
                                    .talkgroups
                                    .iter()
                                    .any(|row| row.tg == tg)
                            });
                            let listed = self.monitor_zones.contains(&idx);
                            let recent = self.talkgroups.zones[idx]
                                .talkgroups
                                .iter()
                                .map(|row| self.tg_recent.level(row.tg, recent_now))
                                .fold(0.0_f32, f32::max);
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 26.0),
                                egui::Sense::click(),
                            );
                            let text = if listed && !decoding {
                                egui::Color32::from_rgb(236, 226, 246)
                            } else {
                                ui.visuals().text_color()
                            };
                            if decoding {
                                // Same dark red as the talkgroup row for this grant.
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(4),
                                    egui::Color32::from_rgb(146, 24, 18),
                                );
                            } else if listed {
                                let edge = if selected {
                                    egui::Stroke::new(1.5, egui::Color32::from_rgb(186, 120, 230))
                                } else {
                                    egui::Stroke::NONE
                                };
                                ui.painter().rect(
                                    rect,
                                    egui::CornerRadius::same(4),
                                    egui::Color32::from_rgb(42, 16, 64),
                                    edge,
                                    egui::StrokeKind::Inside,
                                );
                            } else if selected {
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(4),
                                    HAM.gamma_multiply(0.28),
                                );
                            } else if idx % 2 == 1 {
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::ZERO,
                                    ui.visuals().faint_bg_color,
                                );
                            }
                            if !decoding {
                                recent_bar(ui, rect, recent);
                            }
                            ui.painter().text(
                                rect.left_center() + egui::vec2(8.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                &name,
                                egui::FontId::proportional(14.0),
                                text,
                            );
                            if response.double_clicked() {
                                self.toggle_monitor_zone(idx);
                            }
                            if response.clicked() {
                                self.talkgroup_zone = idx;
                                self.talkgroup_sel = 0;
                            }
                            response.on_hover_text(&name);
                        }
                    });
            });
            ui.vertical(|ui| {
                ui.set_width(list_w + gap + editor_w);
                let list_h = ui.available_height().max(80.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(list_w);
                        egui::ScrollArea::vertical()
                            .id_salt("tg-list")
                            .max_height(list_h)
                            .show(ui, |ui| {
                                let count = self
                                    .talkgroups
                                    .zones
                                    .get(self.talkgroup_zone)
                                    .map(|zone| zone.talkgroups.len())
                                    .unwrap_or(0);
                                for idx in 0..count {
                                    let (clicked, doubled, marked) = self.talkgroup_row(ui, idx, active);
                                    if marked {
                                        self.rec_toggle_talkgroup(idx);
                                    }
                                    if doubled {
                                        if let Some(tg) = self
                                            .talkgroups
                                            .zones
                                            .get(self.talkgroup_zone)
                                            .and_then(|zone| zone.talkgroups.get(idx))
                                            .map(|row| row.tg)
                                        {
                                            self.toggle_monitor_talkgroup(tg);
                                        }
                                    }
                                    if clicked {
                                        self.talkgroup_sel = idx;
                                    }
                                }
                            });
                    });
                    ui.vertical(|ui| {
                        ui.set_width(editor_w);
                        egui::ScrollArea::vertical()
                            .id_salt("tg-editor")
                            .auto_shrink([false, true])
                            .max_height(list_h)
                            .show(ui, |ui| self.talkgroup_editor(ui, editor_w));
                    });
                });
            });
        });
    }

    fn toggle_monitor_zone(&mut self, idx: usize) {
        if !self.monitor_zones.remove(&idx) {
            self.monitor_zones.insert(idx);
        }
    }

    fn toggle_monitor_talkgroup(&mut self, tg: u32) {
        if tg == 0 {
            return;
        }
        if !self.monitor_talkgroups.remove(&tg) {
            self.monitor_talkgroups.insert(tg);
        }
    }

    fn talkgroup_row(&self, ui: &mut egui::Ui, idx: usize, active: Option<u32>) -> (bool, bool, bool) {
        let Some(row) = self
            .talkgroups
            .zones
            .get(self.talkgroup_zone)
            .and_then(|zone| zone.talkgroups.get(idx))
        else {
            return (false, false, false);
        };
        let slot = format!("{:>4}", idx + 1);
        let mut label = format!("{slot}   {:>8}   {}", row.tg, row.name);
        if row.encrypted {
            label.push_str("  enc");
        }
        let followed = active == Some(row.tg);
        let listed = self.monitor_talkgroups.contains(&row.tg);
        let open = self.talkgroup_sel == idx;
        let recent = self.tg_recent.level(row.tg, std::time::Instant::now());
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        let color = if listed && !followed {
            egui::Color32::from_rgb(236, 226, 246)
        } else {
            ui.visuals().text_color()
        };
        if followed {
            // Same dark red as the meter needle. The row is decoding this grant.
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(4),
                egui::Color32::from_rgb(146, 24, 18),
            );
        } else if listed {
            let edge = if open {
                egui::Stroke::new(1.5, egui::Color32::from_rgb(186, 120, 230))
            } else {
                egui::Stroke::NONE
            };
            ui.painter().rect(
                rect,
                egui::CornerRadius::same(4),
                egui::Color32::from_rgb(42, 16, 64),
                edge,
                egui::StrokeKind::Inside,
            );
        } else if open {
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(4),
                HAM.gamma_multiply(0.28),
            );
        } else if recent > 0.0 {
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(4),
                pulse::recent_fill(ui.visuals().panel_fill, recent),
            );
        } else if idx % 2 == 1 {
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::ZERO,
                ui.visuals().faint_bg_color,
            );
        }
        if !followed {
            recent_bar(ui, rect, recent);
        }
        ui.painter_at(rect).text(
            rect.left_center() + egui::vec2(rec_ui::ROW_GUTTER, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::monospace(14.0),
            color,
        );
        if row.record {
            rec_ui::paint_row_mark(ui, rect);
        }
        let marked = response.secondary_clicked();
        let response = response.on_hover_text(rec_ui::row_hover(row.record));
        (response.clicked(), response.double_clicked(), marked)
    }

    fn talkgroup_editor(&mut self, ui: &mut egui::Ui, width: f32) {
        let frame = details_frame(ui);
        let chrome =
            frame.inner_margin.sum().x + frame.stroke.width * 2.0 + frame.outer_margin.sum().x;
        let inner = (width - chrome).max(0.0);
        let zone_idx = self.talkgroup_zone;
        let sel = self.talkgroup_sel;
        let Some(row) = self
            .talkgroups
            .zones
            .get(zone_idx)
            .and_then(|zone| zone.talkgroups.get(sel))
        else {
            let mut add = false;
            frame.show(ui, |ui| {
                ui.set_min_width(inner);
                ui.label("No talkgroup.");
                if ui.button("Add").clicked() {
                    add = true;
                }
            });
            if add {
                self.add_talkgroup();
            }
            return;
        };
        let mut name = row.name.clone();
        let mut tg = row.tg;
        let mut encrypted = row.encrypted;
        let mut priority = row.priority;
        let mut name_changed = false;
        let mut add = false;
        let mut delete = false;
        frame.show(ui, |ui| {
            ui.set_min_width(inner);
            ui.label(egui::RichText::new(format!("Talkgroup {}", sel + 1)).strong());
            name_changed = ui
                .horizontal(|ui| {
                    ui.label("Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut name)
                            .id_salt(("tg-name", zone_idx, sel))
                            .desired_width(180.0),
                    )
                })
                .inner
                .changed();
            ui.horizontal(|ui| {
                ui.label("Number");
                ui.add(egui::DragValue::new(&mut tg).range(0..=16_777_215u32));
            });
            ui.checkbox(&mut encrypted, "Encrypted")
                .on_hover_text("No encryption key is stored, so this is not scanned or played.");
            ui.checkbox(&mut priority, "Priority")
                .on_hover_text("May take a receiver off a held call when emergency/priority override is on.");
            ui.horizontal(|ui| {
                if ui.button("Add").clicked() {
                    add = true;
                }
                if ui.button("Delete").clicked() {
                    delete = true;
                }
            });
        });
        if let Some(row) = self
            .talkgroups
            .zones
            .get_mut(zone_idx)
            .and_then(|zone| zone.talkgroups.get_mut(sel))
        {
            if name_changed {
                row.name = name;
            }
            row.tg = tg;
            row.encrypted = encrypted;
            row.priority = priority;
        }
        if delete {
            self.delete_talkgroup();
        } else if add {
            self.add_talkgroup();
        }
    }

    fn add_talkgroup(&mut self) {
        let Some(zone) = self.talkgroups.zones.get_mut(self.talkgroup_zone) else {
            return;
        };
        let next = zone.talkgroups.iter().map(|row| row.tg).max().unwrap_or(0).saturating_add(1);
        zone.talkgroups.push(talkgroups::Talkgroup {
            record: false,
            name: String::new(),
            tg: next,
            encrypted: false,
            priority: false,
        });
        self.talkgroup_sel = zone.talkgroups.len() - 1;
        self.talkgroup_note.clear();
    }

    fn delete_talkgroup(&mut self) {
        let Some(zone) = self.talkgroups.zones.get_mut(self.talkgroup_zone) else {
            return;
        };
        if self.talkgroup_sel < zone.talkgroups.len() {
            zone.talkgroups.remove(self.talkgroup_sel);
            if self.talkgroup_sel >= zone.talkgroups.len() && self.talkgroup_sel > 0 {
                self.talkgroup_sel -= 1;
            }
        }
    }

    fn browse_talkgroups(&mut self, save: bool) {
        let dir = remembered_dir(&self.talkgroup_dir);
        let dialog = rfd::FileDialog::new()
            .set_title(if save {
                "Save talkgroups"
            } else {
                "Load talkgroups"
            })
            .add_filter("Talkgroups", &["json"])
            .set_directory(&dir);
        let picked = if save {
            let name = if self.talkgroup_path.is_empty() {
                "talkgroups.json".to_string()
            } else {
                file_label(&self.talkgroup_path)
            };
            dialog.set_file_name(name).save_file()
        } else {
            dialog.pick_file()
        };
        let Some(path) = picked else {
            return;
        };
        if let Some(parent) = path.parent() {
            if parent.is_dir() {
                self.talkgroup_dir = Some(parent.to_path_buf());
            }
        }
        self.talkgroup_path = path.display().to_string();
        if save {
            match talkgroups::save_file(Path::new(&self.talkgroup_path), &self.talkgroups) {
                Ok(()) => self.talkgroup_note.clear(),
                Err(err) => self.talkgroup_note = err,
            }
        } else {
            match talkgroups::load_file(Path::new(&self.talkgroup_path)) {
                Ok(file) => {
                    self.talkgroups = file;
                    self.talkgroup_zone = 0;
                    self.talkgroup_sel = 0;
                    self.talkgroup_note.clear();
                }
                Err(err) => self.talkgroup_note = err,
            }
        }
    }
}

fn paint_meter_page(
    ui: &mut egui::Ui,
    snap: &Snapshot,
    radio: Option<&str>,
    meters: &mut MeterState,
    clock: &clock::SystemClock,
    own_window: bool,
) {
    let offers: Vec<(&str, f64, f64)> = snap
        .caps
        .get_levels
        .iter()
        .map(|(name, range)| (name.as_str(), range.min, range.max))
        .collect();
    let values: Vec<(&str, f64)> = snap
        .levels
        .iter()
        .map(|(name, value)| (name.as_str(), *value))
        .collect();
    let faces = meter_faces(
        &offers,
        &values,
        snap.strength_db,
        snap.ptt,
        snap.model == "GQRX",
    );
    let now_ms = clock::unix_now_ms();
    let readout = clock.face_at(now_ms);
    let mut order = order_with_clock(&meters.order, &faces, readout.is_some());
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Meters").strong());
            if let Some(name) = radio {
                ui.label(egui::RichText::new(name).weak());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let toggle = lock_toggle(ui, meters.locked);
                if toggle.clicked() {
                    meters.locked = !meters.locked;
                    meters.drag = None;
                }
                if own_window {
                    if ui.button("Dock").clicked() {
                        meters.dock = true;
                        meters.detached = false;
                        meters.drag = None;
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                } else if ui.button("Detach").clicked() {
                    meters.detached = true;
                    meters.drag = None;
                }
            });
        });
        status_column(ui, snap);
        if readout.is_some() {
            // The shown minute changes on the local clock. Nothing else
            // signals that moment.
            ui.ctx()
                .request_repaint_after(clock.next_paint_delay(now_ms));
        }
        if faces.is_empty() && readout.is_none() {
            ui.label("This radio lists no meter levels.");
        } else {
            order = paint_meter_grid(ui, &faces, order, meters, readout.as_ref());
            meters.order = merge_meter_order(&meters.order, &order);
        }
    });
}

fn meter_window_ui(
    ui: &mut egui::Ui,
    state: &Arc<Mutex<MeterState>>,
    feed: &Arc<Mutex<MeterFeed>>,
) {
    let (snap, radio, clock) = {
        let feed = feed.lock().expect("meters");
        (feed.snap.clone(), feed.radio.clone(), feed.clock.clone())
    };
    let mut meters = state.lock().expect("meters");
    if ui.input(|input| input.viewport().close_requested()) {
        meters.dock = true;
        meters.detached = false;
        meters.drag = None;
    }
    let info = ui.input(|input| input.viewport().clone());
    if let Some(outer) = info.outer_rect {
        meters.saved_pos = Some(outer.min);
    }
    if let Some(inner) = info.inner_rect {
        let size = inner.size();
        if size.x >= 64.0 && size.y >= 64.0 {
            meters.saved_size = Some(size);
        }
    }
    if let Some(monitor) = info.monitor_size {
        if monitor.x > 1.0 && monitor.y > 1.0 {
            meters.saved_monitor = Some(monitor);
        }
    }
    if let Some(ppp) = info.native_pixels_per_point {
        if ppp.is_finite() && ppp > 0.0 {
            meters.saved_ppp = Some(ppp);
        }
    }
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("meter-window")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                paint_meter_page(ui, &snap, radio.as_deref(), &mut meters, &clock, true);
            });
    });
}

/// The grid inside the Meters frame. A locked grid ignores the pointer.
/// An unlocked drag follows the pointer and lands on the cell under it.
fn paint_meter_grid(
    ui: &mut egui::Ui,
    faces: &[MeterFace],
    mut order: Vec<String>,
    meters: &mut MeterState,
    clock: Option<&clock::Readout>,
) -> Vec<String> {
    let width = ui.available_width();
    let cols = meter_columns(width);
    let count = order.len();
    let rows = count.div_ceil(cols);
    let height = rows as f32 * METER_H + rows.saturating_sub(1) as f32 * METER_GAP;
    // A locked grid does not take the drag, so the page can still scroll.
    let sense = if meters.locked {
        egui::Sense::hover()
    } else {
        egui::Sense::click_and_drag()
    };
    let dt = ui.input(|input| input.stable_dt).min(0.1);
    let moving = settle_needles(meters, faces, dt);
    let (area, response) = ui.allocate_exact_size(egui::vec2(width, height), sense);
    if meters.locked {
        meters.drag = None;
    } else if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            let rel = pos - area.min;
            if let Some(index) = meter_hit(rel.x, rel.y, cols, count) {
                let (x, y) = meter_origin(area.min.x, area.min.y, index, cols);
                meters.drag = Some(MeterDrag {
                    id: order[index].clone(),
                    grab: pos - egui::pos2(x, y),
                });
            }
        }
    }
    if !meters.locked && response.drag_stopped() {
        if let Some(drag) = meters.drag.take() {
            if let Some(pos) = ui.input(|input| input.pointer.latest_pos()) {
                let rel = pos - area.min;
                if let Some(to) = meter_hit(rel.x, rel.y, cols, count) {
                    if let Some(from) = order.iter().position(|id| id == &drag.id) {
                        order = move_meter(&order, from, to);
                    }
                }
            }
        }
    }
    let mut shown = order.clone();
    let floating = meters
        .drag
        .clone()
        .filter(|_| !meters.locked && response.dragged());
    if let Some(drag) = &floating {
        if let Some(pos) = response.interact_pointer_pos() {
            let rel = pos - area.min;
            if let Some(to) = meter_hit(rel.x, rel.y, cols, count) {
                if let Some(from) = shown.iter().position(|id| id == &drag.id) {
                    shown = move_meter(&shown, from, to);
                }
            }
        }
    }
    let painter = ui.painter();
    for (index, id) in shown.iter().enumerate() {
        if floating.as_ref().is_some_and(|drag| &drag.id == id) {
            continue;
        }
        let (x, y) = meter_origin(area.min.x, area.min.y, index, cols);
        let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(METER_W, METER_H));
        paint_meter_cell(&painter, rect, id, faces, meters, clock);
    }
    if let Some(drag) = &floating {
        if let Some(pos) = response.interact_pointer_pos() {
            let rect = egui::Rect::from_min_size(pos - drag.grab, egui::vec2(METER_W, METER_H));
            paint_meter_cell(&painter, rect, &drag.id, faces, meters, clock);
        }
    }
    if !meters.locked {
        let over_face = response.hover_pos().is_some_and(|pos| {
            let rel = pos - area.min;
            meter_hit(rel.x, rel.y, cols, count).is_some()
        });
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if over_face {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    // The movement is still traveling. The next frame is the completion
    // of that step, so the needle is not left waiting on the radio poll.
    if moving {
        ui.ctx().request_repaint();
    }
    order
}

fn paint_meter_cell(
    painter: &egui::Painter,
    rect: egui::Rect,
    id: &str,
    faces: &[MeterFace],
    meters: &MeterState,
    clock: Option<&clock::Readout>,
) {
    if id == SYSTEM_CLOCK {
        if let Some(readout) = clock {
            clock::paint(painter, rect, readout);
        }
        return;
    }
    let Some(face) = faces.iter().find(|face| face.id == id) else {
        return;
    };
    let frac = meters
        .needles
        .get(id)
        .map(|needle| needle.frac)
        .unwrap_or(face.frac);
    paint_analog_meter(painter.clone(), rect, frac, &face.scale, face.lamp);
}

fn settle_needles(meters: &mut MeterState, faces: &[MeterFace], dt: f32) -> bool {
    let mut moving = false;
    for face in faces {
        let needle = meters
            .needles
            .entry(face.id.clone())
            .or_insert_with(|| Needle::at(face.frac));
        *needle = step_needle(*needle, face.frac, dt);
        if !needle_settled(*needle, face.frac) {
            moving = true;
        }
    }
    meters
        .needles
        .retain(|id, _| faces.iter().any(|face| face.id == *id));
    moving
}

fn status_column(ui: &mut egui::Ui, snap: &Snapshot) {
    if let Some(roof) = &snap.roofing {
        ui.label(format!("Roofing: {roof}"));
    }
    let swr = snap.levels.get("SWR").copied().unwrap_or(1.0);
    let signs = [
        ("Busy", false),
        ("Mic Eq", false),
        ("PLL unlock", false),
        ("High SWR", swr >= 3.0),
    ];
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let width = ((ui.available_width() - gap * (signs.len() - 1) as f32) / signs.len() as f32)
            .max(64.0);
        for (text, hot) in signs {
            on_air_sign(ui, text, hot, width);
        }
    });
    ui.add_space(6.0);
}

impl App {
    fn page_levels(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            self.sliders(ui, snap);
        });
    }

    fn vfo_column(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        ui.vertical(|ui| {
            let can = snap.online();
            two(ui, |ui| {
                op_button(
                    ui,
                    can && snap.caps.vfo_ops.contains("XCHG"),
                    "A <> B",
                    || {
                        self.send(Command::VfoOp("XCHG".into()));
                    },
                );
                rit_button(ui, snap, "R-6kHz", -6_000, &|cmd| self.send(cmd));
            });
            two(ui, |ui| {
                op_button(
                    ui,
                    can && snap.caps.vfo_ops.contains("CPY"),
                    "A > B",
                    || {
                        self.send(Command::VfoOp("CPY".into()));
                    },
                );
                rit_button(ui, snap, "R-3kHz", -3_000, &|cmd| self.send(cmd));
            });
            two(ui, |ui| {
                if ui.add_enabled(can, egui::Button::new("A > Mem")).clicked() {
                    self.local_mem = Some(snap.freq_hz);
                }
                rit_button(ui, snap, "R-15kHz", -15_000, &|cmd| self.send(cmd));
            });
            two(ui, |ui| {
                if ui
                    .add_enabled(
                        can && self.local_mem.is_some(),
                        egui::Button::new("Mem > A"),
                    )
                    .clicked()
                {
                    if let Some(hz) = self.local_mem {
                        self.tune(snap, hz);
                    }
                }
                rit_button(ui, snap, "R-Auto", 0, &|cmd| self.send(cmd));
            });
            two(ui, |ui| {
                let main = !snap.tx_vfo.contains('B');
                if ui
                    .add_enabled(
                        can && snap.caps.can_set_split,
                        egui::Button::new("TX main").selected(main),
                    )
                    .clicked()
                {
                    self.send(Command::SetSplit {
                        on: false,
                        tx_vfo: snap.vfo.clone(),
                    });
                }
                if ui
                    .add_enabled(
                        can && snap.caps.can_set_split,
                        egui::Button::new("TX sub").selected(!main),
                    )
                    .clicked()
                {
                    self.send(Command::SetSplit {
                        on: true,
                        tx_vfo: "VFOB".into(),
                    });
                }
            });
            two(ui, |ui| {
                for name in ["VFOA", "VFOB"] {
                    let label = if name == "VFOA" { "VFO A" } else { "VFO B" };
                    let on = snap.vfo.eq_ignore_ascii_case(name);
                    if ui
                        .add_enabled(can, egui::Button::new(label).selected(on))
                        .clicked()
                    {
                        self.send(Command::SetVfo(name.into()));
                    }
                }
            });
        });
    }

    /// One VFO: the digit row, the typed frequency, and the hamlib name with
    /// its mode. `main` is the current VFO (`get_vfo`). The other cluster is
    /// `snap.sub_vfo`. The S-meter is the receiver, so it stays on the current
    /// VFO. Hamlib reports one strength.
    fn vfo_cluster(&mut self, ui: &mut egui::Ui, snap: &Snapshot, main: bool, freq: f64) {
        let name = if main {
            snap.vfo.as_str()
        } else {
            snap.sub_vfo.as_str()
        };
        let mode = if main {
            snap.mode.as_str()
        } else {
            snap.sub_mode.as_str()
        };
        let cluster = ui.horizontal(|ui| {
            // The sub VFO stays at 70% of the main. Both are then 90% so the
            // VFO button frame fits between them on the default window.
            let scale = if main { 0.9 } else { 0.7 * 0.9 };
            scale_vfo(ui, scale);
            let hovered_digit = self.frequency(ui, snap, freq, main, scale);
            ui.add_space(18.0 * scale);
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(vfo_label(name))
                        .strong()
                        .color(LCD_PAPER),
                );
                ui.label(
                    egui::RichText::new(mode)
                        .size(22.0 * scale)
                        .strong()
                        .color(LCD_ORANGE),
                );
                if main && snap.rit_hz != 0 {
                    ui.label(format!("RIT {:+} Hz", snap.rit_hz));
                }
                if main {
                    let db = if snap.model == "GQRX" {
                        gqrx_dbfs_to_s9(snap.strength_db)
                    } else {
                        snap.strength_db
                    };
                    let frac = strength_frac(db);
                    meter(ui, &strength_label(db), frac, "");
                }
            });
            hovered_digit
        });
        let can_tune = self.can_tune_freq(snap, main);
        // The cluster, not just the digit buttons. A digit under the pointer
        // wins. Anywhere else on this VFO, the wheel turns the selected digit.
        let over_cluster = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|pos| cluster.response.rect.contains(pos));
        if over_cluster && can_tune {
            let notches = ui.input(|input| {
                wheel_notches(input.events.iter().filter_map(|event| match event {
                    egui::Event::MouseWheel { delta, phase, .. } => match phase {
                        egui::TouchPhase::End | egui::TouchPhase::Cancel => None,
                        _ => Some(delta.y),
                    },
                    _ => None,
                }))
            });
            if notches != 0 {
                let step = cluster.inner.unwrap_or(self.step_hz);
                self.step_digit(snap, freq, f64::from(notches) * step, main);
            }
        }
    }

    /// The digit under the pointer, if the pointer is on one.
    fn frequency(
        &mut self,
        ui: &mut egui::Ui,
        snap: &Snapshot,
        freq: f64,
        main: bool,
        scale: f32,
    ) -> Option<f64> {
        let can_tune = self.can_tune_freq(snap, main);
        ui.vertical(|ui| {
            let digits = ui.horizontal(|ui| {
                let pointer = ui.input(|i| i.pointer.hover_pos());
                let mut hovered_step = None;
                for (ch, step) in digit_steps(freq) {
                    let Some(step) = step else {
                        ui.label(
                            egui::RichText::new(ch.to_string())
                                .size(48.0 * scale)
                                .strong()
                                .monospace(),
                        );
                        continue;
                    };
                    let selected = (step - self.step_hz).abs() < 0.1;
                    let text = egui::RichText::new(ch.to_string())
                        .size(48.0 * scale)
                        .strong()
                        .monospace();
                    let response =
                        ui.add_enabled(can_tune, egui::Button::new(text).selected(selected));
                    // The pointer position, not widget hover: a later hover
                    // sense would sit on top of these buttons and steal it.
                    if pointer.is_some_and(|pos| response.rect.contains(pos)) {
                        hovered_step = Some(step);
                    }
                    if response.clicked() {
                        // The clicked digit becomes the step, and this click
                        // turns that digit by one.
                        self.step_hz = step;
                        self.step_digit(snap, freq, step, main);
                    }
                }
                hovered_step
            });
            ui.horizontal(|ui| {
                let salt = if main { "main-freq" } else { "sub-freq" };
                let id = ui.scope_id().with(salt);
                if !ui.memory(|mem| mem.has_focus(id)) {
                    let shown = format_hz(freq);
                    if main {
                        self.freq_text = shown;
                    } else {
                        self.sub_freq_text = shown;
                    }
                }
                let width = 160.0 * scale;
                let resp = if main {
                    let edit = egui::TextEdit::singleline(&mut self.freq_text)
                        .id_salt(salt)
                        .desired_width(width);
                    ui.add_enabled(can_tune, edit)
                } else {
                    let edit = egui::TextEdit::singleline(&mut self.sub_freq_text)
                        .id_salt(salt)
                        .desired_width(width);
                    ui.add_enabled(can_tune, edit)
                };
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let raw = if main {
                        self.freq_text.clone()
                    } else {
                        self.sub_freq_text.clone()
                    };
                    if let Some(hz) = parse_freq(&raw) {
                        if main {
                            self.tune(snap, hz);
                        } else {
                            self.tune_sub(snap, hz, None);
                        }
                    }
                }
                ui.label(egui::RichText::new(format!("step {}", step_label(self.step_hz))).weak());
            });
            digits.inner
        })
        .inner
    }

    fn selected_hackrf(&self) -> bool {
        self.hub.view().rows.iter().any(|row| {
            row.selected && row.spec.driver == cat::RadioDriver::Hackrf
        })
    }

    /// Fold one published frame into the average and the decaying peak.
    /// A new center, or a capture that restarted, drops the old trace.
    fn fold_spectrum(&mut self, spec: &crate::hackrf::SpectrumSnap) {
        if spec.db.is_empty() || spec.seq == 0 {
            return;
        }
        let restarted = spec.seq < self.spectrum_seq;
        let moved = (spec.center_hz - self.spectrum_center).abs() > 1.0
            || self.spectrum_avg.len() != spec.db.len();
        if spec.seq == self.spectrum_seq && !moved {
            return;
        }
        self.spectrum_seq = spec.seq;
        self.spectrum_center = spec.center_hz;
        if restarted || moved {
            self.spectrum_avg.clone_from(&spec.db);
            self.spectrum_peak_hold.clone_from(&spec.db);
            return;
        }
        for (avg, sample) in self.spectrum_avg.iter_mut().zip(&spec.db) {
            *avg = *avg * 0.62 + sample * 0.38;
        }
        for (peak, sample) in self.spectrum_peak_hold.iter_mut().zip(&spec.db) {
            *peak = (*peak - 0.35).max(*sample);
        }
    }

    /// Spectrum under the tuner buttons, the height of the bandspread block.
    /// Avg and Peak use the same buttons as the row above the glass.
    fn hackrf_spectrum(&mut self, ui: &mut egui::Ui, snap: &Snapshot, freq: f64) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.take_available_width();
            let radio = self.hub.view().rows.into_iter().find(|row| {
                row.selected && row.spec.driver == cat::RadioDriver::Hackrf
            });
            if let Some(row) = radio.as_ref() {
                let matched = self.hackrf_gain_hold.as_ref().is_some_and(|hold| {
                    hold.id == row.id
                        && row.spec.lna_db == Some(hold.lna)
                        && row.spec.vga_db == Some(hold.vga)
                });
                if matched {
                    self.hackrf_gain_hold = None;
                }
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(egui::Button::new("Avg").selected(!self.spectrum_peak))
                    .clicked()
                {
                    self.spectrum_peak = false;
                }
                if ui
                    .add(egui::Button::new("Peak").selected(self.spectrum_peak))
                    .clicked()
                {
                    self.spectrum_peak = true;
                }
                if let Some(row) = radio {
                    ui.add_space(8.0);
                    let id = row.id;
                    let spec = row.spec;
                    self.hackrf_gain_slider(ui, snap, id, &spec, true);
                    ui.add_space(8.0);
                    self.hackrf_gain_slider(ui, snap, id, &spec, false);
                }
            });
            let frame = self.hub.spectrum();
            self.fold_spectrum(&frame);
            let center = if frame.center_hz > 0.0 {
                frame.center_hz
            } else {
                freq
            };
            let trace: &[f32] = if frame.db.is_empty() {
                &[]
            } else if self.spectrum_peak {
                &self.spectrum_peak_hold
            } else {
                &self.spectrum_avg
            };
            if let Some(hz) = spectrum_plot(
                ui,
                "Spec",
                center,
                crate::hackrf::SAMPLE_RATE as f64,
                crate::hackrf::FILTER_HZ,
                trace,
                freq,
                SPECTRUM_H,
            ) {
                if self.can_tune_freq(snap, true) {
                    self.tune(snap, hz);
                }
            }
        });
    }

    /// Gain the sliders show before a drag. A hold wins until the radio row
    /// has stored the same pair.
    fn applied_hackrf_gain(&self, id: u64, spec: &cat::RadioSpec) -> (u32, u32) {
        if let Some(hold) = &self.hackrf_gain_hold {
            if hold.id == id {
                return (hold.lna, hold.vga);
            }
        }
        let gain = cat::resolved_rx_gain(spec);
        (
            gain.lna_db.unwrap_or(crate::hackrf::LNA_OMITTED),
            gain.vga_db.unwrap_or(crate::hackrf::VGA_OMITTED),
        )
    }

    fn commit_hackrf_gain(&mut self, snap: &Snapshot, id: u64, lna: u32, vga: u32) {
        let lna = crate::hackrf::snap_lna(lna);
        let vga = crate::hackrf::snap_vga(vga);
        self.gain_drag_lna = None;
        self.gain_drag_vga = None;
        self.hackrf_gain_hold = Some(HackrfGainHold { id, lna, vga });
        self.hub.hackrf_gain(id, lna, vga);
        if !self.config_path.is_empty() {
            self.save_config(snap);
        }
    }

    /// One horizontal HackRF gain slider. `lna` selects LNA; VGA is the other.
    /// The wheel takes one legal step while the pointer is on the slider.
    /// The capture restarts when the drag ends or the wheel steps.
    fn hackrf_gain_slider(
        &mut self,
        ui: &mut egui::Ui,
        snap: &Snapshot,
        id: u64,
        spec: &cat::RadioSpec,
        lna: bool,
    ) {
        let (base_lna, base_vga) = self.applied_hackrf_gain(id, spec);
        let (step, max, base) = if lna {
            (
                crate::hackrf::LNA_STEP,
                crate::hackrf::LNA_MAX,
                base_lna,
            )
        } else {
            (
                crate::hackrf::VGA_STEP,
                crate::hackrf::VGA_MAX,
                base_vga,
            )
        };
        let mut value = if lna {
            self.gain_drag_lna.unwrap_or(base)
        } else {
            self.gain_drag_vga.unwrap_or(base)
        };
        ui.spacing_mut().slider_width = 112.0;
        let response = ui.add(
            egui::Slider::new(&mut value, 0u32..=max)
                .integer()
                .step_by(f64::from(step))
                .suffix(" dB")
                .text(if lna { "LNA" } else { "VGA" })
                .trailing_fill(true),
        );
        let notch = if lna { "8 dB" } else { "2 dB" };
        let stage = if lna { "LNA" } else { "VGA" };
        let response = response.on_hover_text(format!(
            "HackRF {stage}. The wheel changes it by {notch} while the pointer is on this slider."
        ));
        let notches = if response.hovered() {
            ui.input(|input| {
                wheel_notches(input.events.iter().filter_map(|event| match event {
                    egui::Event::MouseWheel { delta, phase, .. } => match phase {
                        egui::TouchPhase::End | egui::TouchPhase::Cancel => None,
                        _ => Some(delta.y),
                    },
                    _ => None,
                }))
            })
        } else {
            0
        };
        if notches != 0 {
            value = crate::hackrf::step_stage(value, notches, step, max);
        }
        let stopped = response.drag_stopped();
        let dragging = response.dragged() && !stopped && notches == 0;
        if lna {
            self.gain_drag_lna = dragging.then_some(value);
        } else {
            self.gain_drag_vga = dragging.then_some(value);
        }
        let typed = response.changed() && !response.dragged();
        if (notches != 0 || stopped || typed) && value != base {
            let lna_value = if lna {
                value
            } else {
                self.gain_drag_lna.unwrap_or(base_lna)
            };
            let vga_value = if lna {
                self.gain_drag_vga.unwrap_or(base_vga)
            } else {
                value
            };
            self.commit_hackrf_gain(snap, id, lna_value, vga_value);
        }
    }

    fn buttons(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        ui.horizontal_wrapped(|ui| {
            self.func_btn(ui, snap, "ATU", "TUNER");
            self.band_step(ui, snap, "Band -", -1);
            self.band_step(ui, snap, "Band +", 1);
            self.dead(ui, "Dimmer");
            self.func_btn(ui, snap, "NR", "NR");
            self.preamp(ui, snap, "Pre 1", 10.0);
            self.preamp(ui, snap, "Pre 2", 20.0);
            self.ant(ui, snap, "Ant 1", 1);
            self.ant(ui, snap, "Ant 2", 2);
            self.func_btn(ui, snap, "Bk-in", "SBKIN");
            self.func_btn(ui, snap, "Fast", "FAGC");
            self.dead(ui, "NR sub");
            self.dead(ui, "CW Spot");
            self.dead(ui, "Keyer");
            self.dead(ui, "Nar");
            self.func_btn(ui, snap, "Proc", "COMP");
            self.dead(ui, "Ant Rx");
            self.dead(ui, "Chan +");
            self.dead(ui, "Chan -");
            self.func_btn(ui, snap, "Lock", "LOCK");
            self.func_btn(ui, snap, "Notch", "ANF");
            self.dead(ui, "QMR");
            self.dead(ui, "BC");
            self.ptt(ui, snap, "MOX");
            self.dead(ui, "PWR");
            self.dead(ui, "QMS");
            self.dead(ui, "Contour");
            self.func_btn(ui, snap, "Moni", "MON");
            self.dead(ui, "Scan");
            self.dead(ui, "Scan Dn");
        });
    }

    fn func_btn(&self, ui: &mut egui::Ui, snap: &Snapshot, label: &str, name: &str) {
        let on = snap.funcs.get(name).copied().unwrap_or(false);
        let enabled = snap.online() && snap.caps.has_func(name);
        if crate::toggle::add_toggle(ui, enabled, on, label).clicked() {
            self.send(Command::SetFunc {
                name: name.into(),
                on: !on,
            });
        }
    }

    fn dead(&self, ui: &mut egui::Ui, label: &str) {
        ui.add_enabled(false, egui::Button::new(label));
    }

    fn ant(&self, ui: &mut egui::Ui, snap: &Snapshot, label: &str, n: i32) {
        let enabled = snap.online() && snap.caps.can_set_ant;
        if ui
            .add_enabled(enabled, egui::Button::new(label).selected(snap.ant == n))
            .clicked()
        {
            self.send(Command::SetAnt(n));
        }
    }

    fn preamp(&self, ui: &mut egui::Ui, snap: &Snapshot, label: &str, db: f64) {
        let enabled = snap.online() && snap.caps.level_range("PREAMP").is_some();
        let on = snap
            .levels
            .get("PREAMP")
            .is_some_and(|v| (*v - db).abs() < 0.5);
        if crate::toggle::add_toggle(ui, enabled, on, label).clicked() {
            self.send(Command::SetLevel {
                name: "PREAMP".into(),
                value: db,
            });
        }
    }

    fn ptt(&mut self, ui: &mut egui::Ui, snap: &Snapshot, label: &str) {
        let enabled = snap.online() && snap.caps.can_set_ptt && !estop::engaged();
        let label = if estop::engaged() { "TX blocked: E-STOP" } else { label };
        if crate::toggle::add_toggle(ui, enabled, snap.ptt, label).clicked() {
            let next = !snap.ptt;
            if next {
                if let Some(why) = self.plan.transmit_block_on(
                    self.zone_sel,
                    self.band_sel,
                    snap.freq_hz,
                    &snap.mode,
                ) {
                    self.plan_note = why;
                    self.plan_note_ptt = true;
                    return;
                }
            }
            self.plan_note.clear();
            self.plan_note_ptt = false;
            self.send(Command::SetPtt(next));
        }
    }

    fn band_step(&mut self, ui: &mut egui::Ui, snap: &Snapshot, label: &str, dir: isize) {
        let enabled = self.can_tune_freq(snap, true);
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            let freq = self.shown_freq(snap);
            let bands = plan::tunable_bands(&self.plan, &rx_pairs(snap));
            let Some(idx) = plan::adjacent_band(&bands, freq, dir as i32) else {
                return;
            };
            let band = &bands[idx];
            let remembered = self.band_mem.get(&band.name).copied();
            let hz = plan::band_tune_hz(band, remembered);
            // Hamlib has no band-up command. set_freq is the CAT command.
            // A frequency the radio will refuse does not move the chips.
            if snap.caps.allow_freq(hz).is_none() {
                return;
            }
            self.zone_sel = band.zone;
            self.band_sel = band.band;
            self.tune(snap, hz);
        }
    }
}

impl App {
    fn mode_box(&mut self, ui: &mut egui::Ui, snap: &Snapshot, label: &str, main: bool) {
        let current = if main {
            snap.mode.as_str()
        } else {
            snap.sub_mode.as_str()
        };
        let enabled = snap.online()
            && if main {
                snap.caps.can_set_mode
            } else {
                snap.caps.can_set_split && !snap.caps.modes.is_empty()
            };
        ui.add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt(label)
                .selected_text(format!("{label}: {current}"))
                .show_ui(ui, |ui| {
                    for mode in &snap.caps.modes {
                        if ui.selectable_label(mode == current, mode).clicked() {
                            let cmd = if main {
                                self.mode_hold = Some(mode.clone());
                                Command::SetMode {
                                    mode: mode.clone(),
                                    passband_hz: snap.passband_hz,
                                }
                            } else {
                                self.sub_mode_hold = Some(mode.clone());
                                Command::SetSubMode {
                                    mode: mode.clone(),
                                    passband_hz: snap.passband_hz,
                                }
                            };
                            self.send(cmd);
                        }
                    }
                });
        });
    }

    fn receiver_box(&self, ui: &mut egui::Ui, snap: &Snapshot) {
        let enabled = snap.online() && snap.caps.can_set_power;
        let state = if snap.power_on { "Receiver ON" } else { "Receiver OFF" };
        if crate::toggle::add_toggle(ui, enabled, snap.power_on, "Receiver")
            .on_hover_text(state)
            .clicked()
        {
            self.send(Command::SetPower(!snap.power_on));
        }
    }

    fn sliders(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        ui.label(egui::RichText::new("Levels").strong());
        self.level_slider(ui, snap, "AF", "AF gain (main)");
        self.level_slider(ui, snap, "RF", "RF gain (main)");
        self.agc(ui, snap);
        self.level_slider(ui, snap, "RFPOWER", "RF power");
        ui.add_space(6.0);
        fixed_slider(ui, "AF gain (sub)", 0.10);
        fixed_slider(ui, "RF gain (sub)", 1.0);
        ui.add_enabled(false, egui::Button::new("AGC (sub): Auto Fast"));
        self.level_slider(ui, snap, "NR", "Noise reduction");
        ui.add_space(6.0);
        self.level_slider(ui, snap, "IF", "IF shift");
        self.width_slider(ui, snap);
        ui.add_enabled(false, egui::Button::new("Contour: 20 Hz"));
        ui.add_space(6.0);
        self.level_slider(ui, snap, "MICGAIN", "Mic gain");
        self.level_slider(ui, snap, "VOXDELAY", "VOX delay");
        self.level_slider(ui, snap, "VOXGAIN", "VOX gain");
    }

    fn level_slider(&mut self, ui: &mut egui::Ui, snap: &Snapshot, name: &str, label: &str) {
        let Some(range) = snap.caps.level_range(name) else {
            let mut value = 0.0;
            ui.add_enabled(
                false,
                egui::Slider::new(&mut value, 0.0..=1.0).text(format!("{label}: —")),
            );
            return;
        };
        let mut value = self.shown_level(name, snap);
        let text = format!("{label}: {}", readout(name, value, range));
        let resp = ui.add(egui::Slider::new(&mut value, range.min..=range.max).text(text));
        if resp.changed() {
            self.hold_levels.insert(name.to_string(), value);
            self.send(Command::SetLevel {
                name: name.into(),
                value,
            });
        }
    }

    fn width_slider(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        if !snap.caps.can_set_mode {
            let mut value = 0.0;
            ui.add_enabled(
                false,
                egui::Slider::new(&mut value, 0.0..=1.0).text("Filter width: —"),
            );
            return;
        }
        let live = snap.passband_hz;
        let mut value = self.hold_pass.unwrap_or(live);
        if self.hold_pass.is_some_and(|held| (held - live).abs() <= 1) {
            self.hold_pass = None;
            value = live;
        }
        let text = format!("Filter width: {value} Hz");
        let resp = ui.add(egui::Slider::new(&mut value, 50..=16_000).text(text));
        if resp.changed() {
            self.hold_pass = Some(value);
            self.send(Command::SetMode {
                mode: snap.mode.clone(),
                passband_hz: value,
            });
        }
    }

    fn agc(&self, ui: &mut egui::Ui, snap: &Snapshot) {
        let enabled =
            snap.online() && snap.caps.level_range("AGC").is_some() && !snap.caps.agc.is_empty();
        let current = snap.levels.get("AGC").copied().unwrap_or(0.0).round() as i32;
        let name = snap
            .caps
            .agc
            .iter()
            .find(|(id, _)| *id == current)
            .map(|(_, name)| name.as_str())
            .unwrap_or("—");
        ui.add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt("agc")
                .selected_text(format!("AGC (main): {name}"))
                .show_ui(ui, |ui| {
                    for (id, label) in &snap.caps.agc {
                        if ui.selectable_label(*id == current, label).clicked() {
                            self.send(Command::SetLevel {
                                name: "AGC".into(),
                                value: f64::from(*id),
                            });
                        }
                    }
                });
        });
    }

}

fn drive(
    shared: Arc<Mutex<Shared>>,
    wake: Arc<Condvar>,
    face: Arc<Mutex<cat::FaceGate>>,
    hub: Arc<cat::Hub>,
    ctx: egui::Context,
) {
    let mut session = Session::Down {
        message: "starting dummy".into(),
    };
    let mut last = shared.lock().expect("ui").snap.clone();
    let mut estop_retry = estop::Retry::default();
    loop {
        // The condvar is waited on `shared`. The hub's kick holds that same
        // mutex before it notifies, and this loop reads `face.connect` while
        // holding it, so a handed port is not missed.
        let (mut cmds, connect, classic) = {
            let mut guard = shared.lock().expect("ui");
            loop {
                let demo = face.lock().expect("face").demo;
                let connect_ready = !demo && face.lock().expect("face").connect.is_some();
                let pending = !guard.cmds.is_empty() || connect_ready;
                if pending {
                    break;
                }
                if session.is_down() {
                    guard = wake.wait(guard).expect("ui");
                } else {
                    // The radio's own dial can move. Hamlib's default protocol
                    // does not push, so the listen socket is polled on this
                    // timeout and woken at once when the window queues a command.
                    let (next, _) = wake
                        .wait_timeout(guard, Duration::from_millis(200))
                        .expect("ui");
                    guard = next;
                    break;
                }
            }
            let cmds = std::mem::take(&mut guard.cmds);
            drop(guard);
            let demo = face.lock().expect("face").demo;
            let (connect, classic) = if demo {
                (None, false)
            } else {
                let mut gate = face.lock().expect("face");
                (gate.connect.take(), gate.connect_classic)
            };
            (cmds, connect, classic)
        };

        let mut armed = false;
        if connect.is_none() {
            let release = {
                let gate = face.lock().expect("face");
                gate.face_down && gate.face_port.is_none()
            };
            if release && matches!(session, Session::Live(_)) {
                session = Session::Down {
                    message: String::new(),
                };
            }
        }
        let demo = face.lock().expect("face").demo;
        if !demo {
            if let Some(port) = connect {
                let opened = if classic {
                    LiveRig::connect_classic("127.0.0.1", port)
                } else {
                    LiveRig::connect("127.0.0.1", port)
                };
                match opened {
                    Ok(rig) => {
                        // Drop the previous session before publishing the new
                        // port, so the hub does not open a second client on
                        // the radio the drive loop still holds.
                        session = Session::Live(rig);
                        let mut gate = face.lock().expect("face");
                        gate.face_port = Some(port);
                        gate.face_down = false;
                        gate.connect_failed = false;
                        armed = true;
                    }
                    Err(err) => {
                        let mut gate = face.lock().expect("face");
                        gate.face_port = None;
                        gate.face_down = true;
                        gate.connect_failed = true;
                        drop(gate);
                        hub.wake();
                        session = Session::Down {
                            message: err.to_string(),
                        };
                    }
                }
            }
        }

        let follow = face.lock().expect("face").follow;
        if let Session::Live(rig) = &mut session {
            rig.set_follow(follow);
        }
        // E-STOP choke point for the face radio: keying is refused, and the
        // radio is unkeyed until it confirms (and again if a poll shows it
        // keyed, for example by Gpredict on the same rigctld).
        let estop_on = estop::engaged();
        let estop_gen = estop::generation();
        cmds.retain(|cmd| estop::allows_rig(cmd, estop_on));
        let now = std::time::Instant::now();
        let estop_send = estop_on
            && (estop_retry.due(estop_on, estop_gen, now) || (last.ptt && estop_retry.confirmed == estop_gen));
        if estop_send {
            cmds.push(Command::SetPtt(false));
        }
        let mut estop_err: Option<String> = None;
        let mut sent = None;
        let mut remote = None;
        for cmd in &cmds {
            sent = Some(cmd.cat_line());
            match session.apply(cmd) {
                Ok(()) => {}
                Err(RigError::Remote(msg)) => {
                    if matches!(cmd, Command::SetPtt(false)) && estop_send {
                        estop_err = Some(msg.clone());
                    }
                    remote = Some(msg)
                }
                Err(RigError::Io(msg)) => {
                    estop_err = Some(msg.clone());
                    let was_live = matches!(session, Session::Live(_));
                    session = Session::Down { message: msg };
                    if was_live {
                        note_face_down(&face, &hub);
                    }
                    armed = false;
                    break;
                }
            }
        }

        if estop_send {
            let name = "selected radio";
            if session.is_down() {
                estop_err.get_or_insert_with(|| "radio offline".into());
            }
            match estop_err {
                None => {
                    estop_retry.tried(estop_gen, true, now);
                    estop::report(estop_gen, estop::Kind::Transmitter, name, estop::Outcome::Stopped);
                }
                Some(err) if err.contains("RPRT -11") => {
                    estop_retry.tried(estop_gen, true, now);
                    estop::report(estop_gen, estop::Kind::Transmitter, name, estop::Outcome::NoStopSupport);
                }
                Some(err) => {
                    estop_retry.tried(estop_gen, false, now);
                    estop::report(estop_gen, estop::Kind::Transmitter, name, estop::Outcome::Failed(err));
                }
            }
        }
        let mut snap = match session.poll() {
            Ok(snap) => snap,
            Err(RigError::Io(msg)) => {
                let was_live = matches!(session, Session::Live(_));
                session = Session::Down {
                    message: msg.clone(),
                };
                if was_live {
                    note_face_down(&face, &hub);
                }
                armed = false;
                offline(&last, &msg)
            }
            Err(RigError::Remote(msg)) => {
                remote = Some(msg);
                last.clone()
            }
        };
        if let Some(msg) = remote {
            snap.message = msg;
        }
        if snap.kind == LinkKind::Down {
            let mut parked = last.clone();
            parked.kind = LinkKind::Down;
            parked.message = snap.message.clone();
            parked.caps = rigctl::Caps::empty();
            snap = parked;
        }
        if snap.kind != LinkKind::Down {
            if let Session::Live(rig) = &session {
                if rig.follow_current_only() && snap.sub_vfo.is_empty() && !last.sub_vfo.is_empty()
                {
                    snap.sub_vfo.clone_from(&last.sub_vfo);
                    snap.sub_freq_hz = last.sub_freq_hz;
                    if snap.sub_mode.is_empty() {
                        snap.sub_mode.clone_from(&last.sub_mode);
                    }
                }
            }
        }
        // Polls replace the snapshot. Keep the last operator command, and
        // record a command from this pass after that replacement.
        snap.last_cat = sent.unwrap_or_else(|| last.last_cat.clone());
        if snap.kind != LinkKind::Down {
            last = snap.clone();
        } else {
            last.last_cat.clone_from(&snap.last_cat);
        }
        {
            let mut guard = shared.lock().expect("ui");
            if armed && snap.kind == LinkKind::Live {
                guard.live_gen = guard.live_gen.wrapping_add(1);
            }
            if matches!(session, Session::Live(_)) {
                guard.poll_gen = guard.poll_gen.wrapping_add(1);
            }
            guard.snap = snap;
        }
        ctx.request_repaint();
    }
}

fn note_face_down(face: &Mutex<cat::FaceGate>, hub: &cat::Hub) {
    let mut gate = face.lock().expect("face");
    gate.face_down = true;
    gate.face_port = None;
    gate.connect_failed = true;
    drop(gate);
    hub.wake();
}

fn cat_cell(ui: &mut egui::Ui, text: &str, selected: bool) -> bool {
    let mut text = egui::RichText::new(text);
    if selected {
        text = text.color(HAM);
    }
    ui.add(egui::Label::new(text).sense(egui::Sense::click()))
        .clicked()
}

enum Session {
    Live(LiveRig),
    Down { message: String },
}

impl Session {
    fn is_down(&self) -> bool {
        matches!(self, Session::Down { .. })
    }

    fn apply(&mut self, cmd: &Command) -> Result<(), RigError> {
        match self {
            Session::Live(rig) => rig.apply(cmd),
            Session::Down { message } => Err(RigError::Io(message.clone())),
        }
    }

    fn poll(&mut self) -> Result<Snapshot, RigError> {
        match self {
            Session::Live(rig) => rig.poll(),
            Session::Down { message } => Ok(offline(&empty_snap(), message)),
        }
    }
}

/// Scale a VFO cluster's type, button padding, and gaps. Child rows copy this
/// style when they are created, so it has to be set on the cluster before
/// those rows exist.
fn scale_vfo(ui: &mut egui::Ui, scale: f32) {
    let style = ui.style_mut();
    style.spacing.item_spacing *= scale;
    style.spacing.button_padding *= scale;
    style.spacing.interact_size *= scale;
    style.spacing.icon_width *= scale;
    style.spacing.icon_width_inner *= scale;
    for font in style.text_styles.values_mut() {
        font.size *= scale;
    }
}

/// egui keeps a shorter widget on the top edge of a horizontal row. Pad it so
/// its middle lines up with `row_h`. `known_h` is last frame's measurement;
/// the returned height is this frame's.
fn place_centered(
    ui: &mut egui::Ui,
    row_h: f32,
    known_h: f32,
    add: impl FnOnce(&mut egui::Ui),
) -> f32 {
    let pad = ((row_h - known_h) * 0.5).max(0.0);
    let mut measured = 0.0;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if pad > 0.0 {
            ui.add_space(pad);
        }
        let piece = ui.scope(add);
        measured = piece.response.rect.height();
    });
    measured
}

fn offline(last: &Snapshot, message: &str) -> Snapshot {
    let mut snap = last.clone();
    snap.kind = LinkKind::Down;
    snap.message = message.to_string();
    snap.caps = rigctl::Caps::empty();
    snap
}

fn empty_snap() -> Snapshot {
    offline(
        &Snapshot {
            kind: LinkKind::Down,
            model: String::new(),
            endpoint: String::new(),
            message: String::new(),
            last_cat: String::new(),
            freq_hz: 0.0,
            mode: String::new(),
            passband_hz: 0,
            sub_mode: String::new(),
            sub_freq_hz: 0.0,
            sub_vfo: String::new(),
            vfo: String::new(),
            tx_vfo: String::new(),
            rit_hz: 0,
            ptt: false,
            power_on: false,
            ant: 1,
            strength_db: -54.0,
            levels: Default::default(),
            funcs: Default::default(),
            caps: rigctl::Caps::empty(),
            roofing: None,
        },
        "",
    )
}

/// A lit legend in the shape of an on-air sign: black window, red border,
/// red letters. Lit letters are bright. A dark sign is the same lamp, off.
fn on_air_sign(ui: &mut egui::Ui, text: &str, hot: bool, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 40.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(3),
        egui::Color32::from_rgb(168, 172, 176),
    );
    let window = rect.shrink(3.0);
    painter.rect_filled(window, egui::CornerRadius::same(2), egui::Color32::BLACK);
    let red = if hot {
        egui::Color32::from_rgb(255, 28, 28)
    } else {
        egui::Color32::from_rgb(88, 16, 16)
    };
    painter.rect_stroke(
        window.shrink(2.0),
        egui::CornerRadius::same(1),
        egui::Stroke::new(2.0, red),
        egui::StrokeKind::Inside,
    );
    painter.text(
        window.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(15.0),
        red,
    );
}

fn two(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(body);
}

fn op_button(ui: &mut egui::Ui, enabled: bool, label: &str, on_click: impl FnOnce()) {
    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
        on_click();
    }
}

fn rit_button(ui: &mut egui::Ui, snap: &Snapshot, label: &str, hz: i32, send: &impl Fn(Command)) {
    // R-Auto clears RIT. Leaving it selected whenever RIT is zero would light
    // the button for the whole session, so only the offset buttons select.
    let selected = hz != 0 && snap.rit_hz == hz;
    if ui
        .add_enabled(
            snap.online() && snap.caps.can_set_rit,
            egui::Button::new(label).selected(selected),
        )
        .clicked()
    {
        send(Command::SetRit(hz));
    }
}

fn step_label(step: f64) -> String {
    if step >= 1_000_000.0 {
        format!("{} MHz", step / 1_000_000.0)
    } else if step >= 1_000.0 {
        format!("{} kHz", step / 1_000.0)
    } else {
        format!("{step:.0} Hz")
    }
}

fn readout(name: &str, value: f64, range: rigctl::Range) -> String {
    match name {
        "IF" => format!("{value:.0} Hz"),
        "VOXDELAY" if range.max > 20.0 => format!("{value:.0} ms"),
        "SWR" => format!("{value:.1}"),
        "AGC" | "PREAMP" => format!("{value:.0}"),
        _ if range.max <= 1.0 && range.min >= 0.0 => format!("{}", (value * 100.0).round() as i32),
        _ => format!("{value:.0}"),
    }
}

/// Sub-receiver and contour have no hamlib level. They stay on the face at the
/// reference numbers and never send a command.
fn fixed_slider(ui: &mut egui::Ui, label: &str, value: f64) {
    let shown = (value * 100.0).round() as i32;
    let mut value = value;
    ui.add_enabled(
        false,
        egui::Slider::new(&mut value, 0.0..=1.0).text(format!("{label}: {shown}")),
    );
}

#[cfg(test)]
mod config_order_tests {
    #[test]
    fn config_groups_run_in_page_order() {
        let names: Vec<&str> = crate::CONFIG_GROUPS.iter().map(|(n, _)| *n).collect();
        let want = [
            "Station", "Display", "Radio", "Rotator", "Amplifier",
            "Trunking", "Audio patch", "Automation patch", "RF patch",
        ];
        assert_eq!(names, want);
        // page_config must call the groups in the same order.
        let src = include_str!("main.rs");
        let body = &src[src.find("fn page_config(").unwrap()..];
        let body = &body[..body.find("\n    fn ").unwrap()];
        let pos = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("{needle}"));
        let seq = [
            pos("cfg_group(ui, \"Station\""), pos("cfg_group(ui, \"Display\""),
            pos("self.cat_frame(ui)"), pos("self.rotator_amp_frame(ui)"),
            pos("cfg_group(ui, \"Trunking\""), pos("self.audio_frame(ui)"),
            pos("self.automation_frame(ui, snap)"), pos("cfg_group(ui, \"RF patch\""),
        ];
        assert!(seq.windows(2).all(|w| w[0] < w[1]), "{seq:?}");
    }
}
