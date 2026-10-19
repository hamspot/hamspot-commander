//! Hamspot Commander, phase 1: the rig-control face.
//!
//! The window follows the RT-950 CPS shell: a page tab row at the top,
//! grouped frames in the body, and a status line. The controls are the rig
//! face (frequency, meters, VFO, band scales, levels).

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
mod plan;
mod talkgroups;
mod units;
mod rot;
mod rx_pipe;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use eframe::egui;
use rigctl::{Command, LinkKind, LiveRig, Rig, RigError, Snapshot};

use dial::{
    channel_dial, channel_window, digit_steps, format_hz, lock_toggle,
    merge_meter_order, meter, meter_columns, meter_faces, meter_hit, meter_origin, move_meter,
    order_with_clock, SYSTEM_CLOCK,
    gqrx_dbfs_to_s9, needle_settled, paint_analog_meter, parse_freq, scale, step_needle,
    strength_frac, strength_label, swatch, vfo_label, wheel_notches, zone_scale, MeterFace, Needle,
    PaintSpan,
    SubBand, HAM, METER_GAP, METER_H, METER_W,
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
    /// Apply was sent. The station file is written once the hub has that command.
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
    cc_voice_sent: HashMap<u64, bool>,
    /// Last control-channel mode sent, so a parked radio is not told again.
    cc_mode_sent: HashMap<u64, String>,
    followers: HashMap<u64, follow::Follower>,
    feeds: HashMap<u64, follow::DsdFeed>,
    /// Last decoder stderr lines per radio. The hub queue is drained for
    /// the follower, so this is what the Automation frame shows.
    cc_tail: HashMap<u64, Vec<String>>,
    /// Last Phase 1 sync identity per radio, for the scanner face.
    cc_site: HashMap<u64, follow::SiteBanner>,
    /// Trunking-system time as a skew from this machine's clock.
    system_clock: clock::SystemClock,
    config_path: String,
    /// Last folder chosen in the config browser. Absent means the working directory.
    config_dir: Option<PathBuf>,
    config_note: String,
    /// Canonical audio graph last written, so a quiet PipeWire graph is not saved again.
    audio_saved: String,
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
            rx_pipe_diverted: false,
            tx_pipe_status: String::new(),
            tx_pipe_diverted: false,
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
        file.zones[0].talkgroups.push(crate::talkgroups::Talkgroup {
            name: "ISP".into(),
            tg: 10006,
            encrypted: false,
        });
        assert_eq!(talkgroup_radio_ids(&rows, true), vec![3]);
        let tabs = super::page_tabs(&rows, &file, &[]);
        assert!(tabs.iter().any(|(page, label)| {
            *page == Page::Talkgroups(3) && label == "HackRF Talkgroups"
        }));
        assert!(tabs.iter().all(|(page, _)| !matches!(page, Page::RadioIds(_))));
        let units = vec![crate::units::Unit::named(42)];
        let empty = crate::talkgroups::empty();
        let radio_tabs = super::page_tabs(&rows, &empty, &units);
        assert!(radio_tabs.iter().any(|(page, label)| {
            *page == Page::RadioIds(3) && label == "HackRF Radio IDs"
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
        let colors = super::FOLDER_TAB_COLORS;
        let count = colors.len();
        assert!(count >= 2);
        for index in 0..count {
            assert_ne!(colors[index], colors[(index + 1) % count]);
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
    /// Talkgroup directory for one radio's control-channel follower.
    Talkgroups(u64),
    /// Radios named on that control channel.
    RadioIds(u64),
    Meters,
    Levels,
    Config,
}

const PAGES: &[(Page, &str)] = &[
    (Page::Tune, "Tune"),
    (Page::Plan, "Plan"),
    (Page::Channels, "Channels"),
    (Page::Meters, "Meters"),
    (Page::Levels, "Levels"),
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

/// Radios that get a Talkgroups tab. A live P25 receive pipe wins. With the
/// pipe off, a saved directory still keeps a tab on each GQRX radio so the
/// list does not disappear when the decoder is stopped.
fn talkgroup_radio_ids(rows: &[cat::RadioRow], directory: bool) -> Vec<u64> {
    let piped: Vec<u64> = rows
        .iter()
        .filter(|row| crate::rx_pipe::is_p25_rx(&row.spec.rx_pipe))
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
        .filter(|row| row.selected)
        .map(|row| row.id)
        .collect()
}

/// Dark folder-card stock. Neighbors in this list differ, and the first
/// color differs from the last, so a repeated cycle still changes at the join.
const FOLDER_TAB_COLORS: [[u8; 3]; 8] = [
    [124, 96, 58],
    [58, 78, 102],
    [86, 96, 58],
    [112, 70, 62],
    [52, 90, 84],
    [96, 70, 102],
    [110, 82, 52],
    [64, 70, 102],
];

fn folder_tab_fill(index: usize, selected: bool, hovered: bool) -> egui::Color32 {
    let [r, g, b] = FOLDER_TAB_COLORS[index % FOLDER_TAB_COLORS.len()];
    let lift: u8 = match (selected, hovered) {
        (true, true) => 48,
        (true, false) => 32,
        (false, true) => 14,
        (false, false) => 0,
    };
    egui::Color32::from_rgb(
        r.saturating_add(lift),
        g.saturating_add(lift),
        b.saturating_add(lift),
    )
}

/// A page or zone tab: flat bottom, rounded top, one folder color.
///
/// The open tab is the same stock, lifted. The label stays a warm paper color.
fn folder_tab(ui: &mut egui::Ui, selected: bool, index: usize, label: &str) -> egui::Response {
    let text = egui::Color32::from_rgb(248, 240, 226);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, text);
    let pad = egui::vec2(14.0, 6.0);
    let size = galley.size() + pad * 2.0;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = folder_tab_fill(index, selected, response.hovered());
        let edge = egui::Color32::from_rgb(
            fill.r().saturating_sub(36),
            fill.g().saturating_sub(36),
            fill.b().saturating_sub(36),
        );
        ui.painter().rect(
            rect,
            egui::CornerRadius {
                nw: 9,
                ne: 9,
                sw: 0,
                se: 0,
            },
            fill,
            egui::Stroke::new(1.0, edge),
            egui::StrokeKind::Inside,
        );
        let pos = egui::pos2(
            rect.center().x - galley.size().x * 0.5,
            rect.center().y - galley.size().y * 0.5,
        );
        ui.painter().galley(pos, galley, text);
    }
    response
}

const LCD_ORANGE: egui::Color32 = egui::Color32::from_rgb(232, 128, 42);
const LCD_YELLOW: egui::Color32 = egui::Color32::from_rgb(245, 214, 64);
const LCD_PAPER: egui::Color32 = egui::Color32::from_rgb(226, 222, 206);

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

/// One key beside the scanner face. `lit` is the Follow key while it is on.
/// Load and Save stay dark; they act on the click.
fn radio_key(ui: &mut egui::Ui, label: &str, lit: bool, enabled: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(72.0, 46.0), egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hover = enabled && response.hovered();
    let fill = if lit {
        egui::Color32::from_rgb(18, 58, 28)
    } else if hover {
        egui::Color32::from_rgb(48, 48, 52)
    } else {
        egui::Color32::from_rgb(28, 28, 32)
    };
    let edge = if lit {
        egui::Color32::from_rgb(46, 176, 78)
    } else if hover {
        egui::Color32::from_rgb(120, 120, 126)
    } else {
        egui::Color32::from_rgb(78, 78, 84)
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
    } else if lit {
        egui::Color32::from_rgb(150, 230, 140)
    } else {
        egui::Color32::from_rgb(232, 228, 216)
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
        "Talkgroups",
        Page::Talkgroups,
    );
    let radios = radio_page_tabs(
        rows,
        talkgroup_radio_ids(rows, !units.is_empty()),
        "Radio IDs",
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
            channel_path: String::new(),
            channel_dir,
            channel_note: String::new(),
            talkgroups: talkgroups::empty(),
            talkgroup_zone: 0,
            talkgroup_sel: 0,
            talkgroup_path: String::new(),
            talkgroup_dir: None,
            talkgroup_note: String::new(),
            units: Vec::new(),
            unit_sel: 0,
            unit_desc_dirty: false,
            cc_on: HashMap::new(),
            pending_control: None,
            cc_sent: HashMap::new(),
            cc_voice_sent: HashMap::new(),
            cc_mode_sent: HashMap::new(),
            followers: HashMap::new(),
            feeds: HashMap::new(),
            cc_tail: HashMap::new(),
            cc_site: HashMap::new(),
            system_clock: clock::SystemClock::default(),
            config_path: String::new(),
            config_dir,
            config_note: String::new(),
            audio_saved: crate::audio::canonical(&crate::audio::AudioGraph::default()),
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
        };
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

    fn snap_gen(&self) -> (Snapshot, u64) {
        let guard = self.shared.lock().expect("rig thread");
        (guard.snap.clone(), guard.live_gen)
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
        if row.spec.follow || !crate::rx_pipe::is_p25_rx(&row.spec.rx_pipe) {
            return false;
        }
        self.cc_on.get(&row.id).copied().unwrap_or(false)
    }

    fn dial_held(&self) -> bool {
        self.following() || self.following_control()
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
        // The open station is the copy of the face. A crash never reaches
        // here, so a change that has to survive that is written when it happens.
        if !self.config_path.is_empty() {
            let snap = self.shared.lock().expect("rig thread").snap.clone();
            self.unit_desc_dirty = false;
            self.save_config(&snap);
        }
        self.hub.shutdown();
        self.rotators.shutdown();
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
        let (mut snap, live_gen) = self.snap_gen();
        self.maybe_push_loaded(&snap, live_gen);
        self.autosave_audio(&snap);
        self.flush_rx_pipe(&snap);
        self.flush_tx_pipe(&snap);
        self.service_cc(&snap);
        self.sync_gpredict();
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
        let face_hz = if self.page == Page::Tune {
            let hz = self.shown_freq(&snap);
            self.follow_band(hz, &rx_pairs(&snap));
            hz
        } else {
            snap.freq_hz
        };
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
        // a second line and pushing the page body down.
        egui::Panel::top("pages").show_inside(ui, |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("page-tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let rows = self.hub.view().rows;
                        let tabs = page_tabs(&rows, &self.talkgroups, &self.units);
                        let still_here = match self.page {
                            Page::Talkgroups(id) => tabs
                                .iter()
                                .any(|(page, _)| *page == Page::Talkgroups(id)),
                            Page::RadioIds(id) => {
                                tabs.iter().any(|(page, _)| *page == Page::RadioIds(id))
                            }
                            _ => true,
                        };
                        if !still_here {
                            self.page = Page::Channels;
                        }
                        for (index, (page, label)) in tabs.into_iter().enumerate() {
                            if folder_tab(ui, self.page == page, index, &label).clicked() {
                                self.page = page;
                            }
                        }
                        if self.unit_desc_dirty && !matches!(self.page, Page::RadioIds(_)) {
                            self.save_unit_description(&snap);
                        }
                    });
                });
        });
        egui::Panel::bottom("status").show_inside(ui, |ui| {
            self.status_line(ui, &snap, face_hz);
        });
        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Tune owns the wheel. Plan and Channels own a bandspread click.
            // A scroll area would take those pointers and move the page.
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
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("body")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.page_body(ui, &mut snap));
            }
        });

        let cat_driver = self.cat_edit.as_ref().map(|edit| edit.driver.clone());
        match cat_driver {
            Some(cat::RadioDriver::Hamlib) => self.hamlib_window(&ctx),
            Some(cat::RadioDriver::Gqrx) => self.gqrx_window(&ctx),
            None => {}
        }
        if self.rot_edit.is_some() {
            self.rotator_window(&ctx);
        }
        self.show_meter_window(&ctx, &snap);
    }
}

impl App {
    fn status_line(&self, ui: &mut egui::Ui, snap: &Snapshot, face_hz: f64) {
        let link = match snap.kind {
            LinkKind::Demo => "Demo".to_string(),
            LinkKind::Live => format!("{}  {}", snap.model, snap.endpoint),
            LinkKind::Down => format!("Disconnected  {}", snap.message),
        };
        ui.horizontal(|ui| {
            ui.label(link);
            if !snap.message.is_empty() && snap.kind != LinkKind::Down {
                ui.separator();
                ui.label(
                    egui::RichText::new(&snap.message).color(egui::Color32::from_rgb(220, 160, 80)),
                );
            }
            if let Some(label) = self
                .plan
                .reference_label(self.zone_sel, self.band_sel, face_hz)
            {
                ui.separator();
                ui.label(label);
            }
            if !self.plan_note.is_empty() {
                ui.separator();
                ui.label(
                    egui::RichText::new(&self.plan_note)
                        .color(egui::Color32::from_rgb(220, 160, 80)),
                );
            }
            if !self.channel_note.is_empty() {
                ui.separator();
                ui.label(
                    egui::RichText::new(&self.channel_note)
                        .color(egui::Color32::from_rgb(220, 160, 80)),
                );
            }
            if !snap.last_cat.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&snap.last_cat).monospace());
                });
            }
        });
    }

    fn page_body(&mut self, ui: &mut egui::Ui, snap: &mut Snapshot) {
        match self.page {
            Page::Tune => self.page_tune(ui, snap),
            Page::Plan => self.page_plan(ui, snap),
            Page::Channels => self.page_channels(ui, snap),
            Page::Talkgroups(id) => self.page_talkgroups(ui, id, snap),
            Page::RadioIds(id) => self.page_radio_ids(ui, id, snap),
            Page::Meters => self.page_meters(ui, snap),
            Page::Levels => self.page_levels(ui, snap),
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
                let bsp = ui.selectable_label(self.show_spread, "BSP");
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
                if radio_key(ui, "LOAD", false, true).clicked() {
                    self.browse_plan(false);
                }
                if radio_key(ui, "SAVE", false, true).clicked() {
                    self.browse_plan(true);
                }
                if radio_key(ui, "ADD", false, true).clicked() {
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
        ui.horizontal_wrapped(|ui| {
            for (idx, name) in names.iter().enumerate() {
                let response = folder_tab(ui, self.zone_sel == idx, idx, name);
                if response.clicked() || response.secondary_clicked() {
                    self.zone_sel = idx;
                    self.band_sel = 0;
                }
                response.context_menu(|ui| self.zone_menu(ui, idx));
            }
        });
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
                if radio_key(ui, "LOAD", false, true).clicked() {
                    self.browse_channels(false);
                }
                if radio_key(ui, "SAVE", false, true).clicked() {
                    self.browse_channels(true);
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
                            let (clicked, response) = self.zone_row(ui, idx);
                            if jump && idx == self.channel_zone {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            if clicked {
                                self.channel_zone = idx;
                                self.channel_sel = 0;
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
                            if self.channel_row(ui, idx) {
                                self.recall_channel(snap, idx);
                            }
                        }
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

    fn load_channels(&mut self) {
        match channels::load_file(Path::new(&self.channel_path)) {
            Ok(file) => {
                self.channels = file;
                self.channel_zone = 0;
                self.channel_sel = 0;
                self.channel_note.clear();
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

    fn page_config(&mut self, ui: &mut egui::Ui, snap: &mut Snapshot) {
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
                egui::RichText::new(&self.config_note).color(egui::Color32::from_rgb(220, 160, 80)),
            );
        }
        ui.add_space(8.0);
        ui.label(format!("Band plan: {}", file_label(&self.plan_path)));
        ui.label(format!("Channels: {} zones", self.channels.zones.len()));
        ui.label(format!("Talkgroups: {} zones", self.talkgroups.zones.len()));
        ui.add_space(8.0);
        self.cat_frame(ui);
        ui.add_space(8.0);
        self.audio_frame(ui);
        ui.add_space(8.0);
        self.automation_frame(ui, snap);
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
            .filter(|row| crate::rx_pipe::is_p25_rx(&row.spec.rx_pipe))
            .collect();
        if p25.is_empty() {
            ui.label("No P25 receive pipe is on.");
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
            .filter(|row| crate::rx_pipe::is_p25_rx(&row.spec.rx_pipe))
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
        let mut select = None;
        let mut follow = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
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
                        .color(egui::Color32::from_rgb(220, 160, 80)),
                );
            }
            ui.label("Leave Gpredict device 2 at None. Downlink is VFO A and uplink is VFO B. Close Radio Control and open it again, then Engage. A wire does not move a radio that is already engaged.");
            let selected = radios.rows.iter().find(|row| row.selected);
            match selected {
                Some(row) => {
                    let mut on = row.spec.follow;
                    if ui.checkbox(&mut on, "Follow").changed() {
                        follow = Some((row.id, on));
                    }
                }
                None => {
                    ui.label("No radio is selected.");
                }
            }
            ui.add_space(8.0);
            ui.label("Rotators");
            ui.add_space(4.0);
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
        if let Some(id) = select {
            self.rotators.select(id);
        }
        if let Some((id, on)) = follow {
            self.hub.set_follow(id, on);
        }
    }

    fn cat_frame(&mut self, ui: &mut egui::Ui) {
        let view = self.hub.view();
        let mut select = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label("CAT Control");
            ui.add_space(4.0);
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
                        let listen = match row.port {
                            Some(port) => format!("127.0.0.1:{port}"),
                            None => "127.0.0.1".to_string(),
                        };
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

    fn audio_frame(&mut self, ui: &mut egui::Ui) {
        let view = self.hub.view();
        let mut endpoint = None;
        let mut pipe_apply = None;
        let mut tx_apply = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Audio");
                if ui
                    .add_enabled(helvum_on_path(), egui::Button::new("Helvum").small())
                    .clicked()
                {
                    launch_helvum();
                }
            });
            ui.add_space(4.0);
            if self.config_path.is_empty() {
                ui.label("Open a station file to keep PipeWire changes.");
            }
            if !view.audio_note.is_empty() {
                ui.label(
                    egui::RichText::new(&view.audio_note)
                        .color(egui::Color32::from_rgb(220, 160, 80)),
                );
            }
            for row in &view.rows {
                let rx = crate::audio::describe(&row.make, &row.model_label, row.id, "RX");
                let tx = crate::audio::describe(&row.make, &row.model_label, row.id, "TX");
                ui.label(format!("{}  {} {}", row.id, row.make, row.model_label));
                ui.horizontal(|ui| {
                    ui.label(rx);
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
                    ui.label(tx);
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
                            .color(egui::Color32::from_rgb(220, 160, 80)),
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
                });
                if !row.rx_pipe_status.is_empty() {
                    ui.label(
                        egui::RichText::new(&row.rx_pipe_status)
                            .color(egui::Color32::from_rgb(220, 160, 80)),
                    );
                }
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
                });
                if !row.tx_pipe_status.is_empty() {
                    ui.label(
                        egui::RichText::new(&row.tx_pipe_status)
                            .color(egui::Color32::from_rgb(220, 160, 80)),
                    );
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
                rx_pipe_label: pipe_block_label(&row.spec.rx_pipe, "RX pipe"),
                tx_pipe_live: row.tx_pipe_diverted,
                tx_pipe_label: pipe_block_label(&row.spec.tx_pipe, "TX pipe"),
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
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(220, 160, 80)),
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
                    egui::RichText::new(&edit.error).color(egui::Color32::from_rgb(220, 160, 80)),
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
                            .color(egui::Color32::from_rgb(220, 160, 80)),
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
                            .color(egui::Color32::from_rgb(220, 160, 80)),
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
            audio: edit
                .id
                .and_then(|id| {
                    self.hub
                        .view()
                        .rows
                        .iter()
                        .find(|row| row.id == id)
                        .map(|row| row.spec.audio.clone())
                })
                .unwrap_or_default(),
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
                cat::RadioDriver::Hamlib => cat::GqrxConfig::new(),
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
        if live_gen <= pending.after_gen {
            return;
        }
        let view = self.hub.view();
        let Some(row) = view.rows.iter().find(|row| row.selected) else {
            return;
        };
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
        match config::load_file(Path::new(&self.config_path)) {
            Ok(file) => self.apply_config(snap, file),
            Err(err) => self.config_note = err,
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
                self.config_note.clear();
            }
            Err(err) => self.config_note = err,
        }
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
                .map(|row| row.spec)
                .collect(),
            audio: self.hub.view().audio.clone(),
            rotators: self
                .rotators
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
            channel_zone: self.channel_zone,
            channel_sel: self.channel_sel,
            talkgroup_zone: self.talkgroup_zone,
            talkgroup_sel: self.talkgroup_sel,
            unit_sel: self.unit_sel,
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
            meters: self.meters_face(),
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
            selected,
            page,
            channel_zone,
            channel_sel,
            talkgroup_zone,
            talkgroup_sel,
            unit_sel,
            control_follow,
            meters,
            ..
        } = file;
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
        self.pending_tune = Some(PendingTune {
            tune: tune.clone(),
            levels: levels.clone(),
            after_gen,
        });
        self.audio_saved = crate::audio::canonical(&audio);
        self.audio_hold = true;
        // The generation is the list on screen now. Replace bumps it, and
        // the flags are applied to that newer list.
        let list_gen = self.hub.view().list_gen;
        let follow_flags = config::control_follow_flags(radios.len(), &control_follow);
        self.hub.replace(radios, selected);
        self.pending_control = Some((list_gen, follow_flags));
        self.cc_on.clear();
        self.hub.apply_audio(audio);
        self.rotators.replace(rotators);
        self.page = page_from_name(&page);
        self.channel_zone = channel_zone;
        self.channel_sel = channel_sel;
        self.talkgroup_zone = talkgroup_zone;
        self.talkgroup_sel = talkgroup_sel;
        self.unit_sel = unit_sel;
        self.zone_list_follow = true;
        self.clamp_channel_selection();
        self.apply_meters(&meters);
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

    fn zone_row(&self, ui: &mut egui::Ui, idx: usize) -> (bool, egui::Response) {
        let name = self
            .channels
            .zones
            .get(idx)
            .map(|zone| zone.name.clone())
            .unwrap_or_default();
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        let painter = ui.painter();
        if self.channel_zone == idx {
            painter.rect_filled(rect, egui::CornerRadius::same(4), HAM.gamma_multiply(0.28));
        } else if idx % 2 == 1 {
            painter.rect_filled(rect, egui::CornerRadius::ZERO, ui.visuals().faint_bg_color);
        }
        painter.text(
            rect.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &name,
            egui::FontId::proportional(14.0),
            ui.visuals().text_color(),
        );
        (response.clicked(), response.on_hover_text(name))
    }

    fn channel_row(&self, ui: &mut egui::Ui, idx: usize) -> bool {
        let Some(channel) = self
            .channels
            .zones
            .get(self.channel_zone)
            .and_then(|zone| zone.channels.get(idx))
        else {
            return false;
        };
        let freq = format!("{:10.3}", channel.hz / 1_000_000.0);
        let slot = format!("{:>3}", idx + 1);
        let name = channel.name.clone();
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        let painter = ui.painter();
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
            rect.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::monospace(14.0),
            ui.visuals().text_color(),
        );
        response.clicked()
    }

    fn channel_editor(&mut self, ui: &mut egui::Ui, snap: &Snapshot, width: f32) {
        let frame = egui::Frame::group(ui.style());
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
        let mut picked = None;
        let mut add = false;
        let mut delete = false;
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
            ui.checkbox(&mut control, "Control");
            ui.checkbox(&mut encrypted, "Encrypted");
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
            .is_some_and(|channel| channel.control != control || channel.encrypted != encrypted);
        if flags_changed {
            if let Some(channel) = self
                .channels
                .zones
                .get_mut(zone_idx)
                .and_then(|zone| zone.channels.get_mut(sel))
            {
                channel.control = control;
                channel.encrypted = encrypted;
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
            name: String::new(),
            hz,
            mode,
            control: false,
            encrypted: false,
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
        if control {
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
            const CAP: usize = 80;
            if tail.len() > CAP {
                let extra = tail.len() - CAP;
                tail.drain(0..extra);
            }
        }
        if let Some(banner) = banner {
            self.cc_site.insert(id, banner);
        }
        let now = clock::unix_now_ms();
        for plain in plains {
            self.system_clock.note_line(&plain, now);
        }
    }

    /// Rolling stderr from the P25 decoder. Empty means the pipe has not
    /// written a line yet, not that the log was discarded.
    fn decoder_log(&self, ui: &mut egui::Ui, id: u64, radio: &str) {
        ui.label(format!("{radio} decoder"));
        let lines = self.cc_tail.get(&id).cloned().unwrap_or_default();
        egui::ScrollArea::vertical()
            .id_salt(("cc-log", id))
            .max_height(140.0)
            .auto_shrink([false, true])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if lines.is_empty() {
                    ui.label(
                        "No decoder lines yet. This is dsd-fme stderr from the P25 receive pipe.",
                    );
                } else {
                    for line in &lines {
                        ui.label(egui::RichText::new(line).monospace().size(11.0));
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

    /// Park a P25 radio on a control memory and hop to the newest clear grant.
    /// Gpredict follow keeps the radio. The decoder process stays up across hops.
    fn service_cc(&mut self, snap: &Snapshot) {
        self.apply_saved_control_follow();
        let rows = self.hub.view().rows;
        let p25: Vec<(u64, bool, bool)> = rows
            .iter()
            .filter(|row| crate::rx_pipe::is_p25_rx(&row.spec.rx_pipe))
            .map(|row| (row.id, row.selected, row.spec.follow))
            .collect();
        let ids: Vec<u64> = p25.iter().map(|row| row.0).collect();
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

        for (id, selected, gpredict) in p25 {
            let on = self.cc_on.get(&id).copied().unwrap_or(false);
            let events = on && !gpredict;
            if self.cc_sent.get(&id).copied() != Some(events) {
                self.hub.set_cc(id, events);
                self.cc_sent.insert(id, events);
            }
            let lines = self.hub.take_cc_events(id);
            self.note_decoder(id, &lines);
            // The advertisement is useful with Follow off. The feed has to
            // see every line or a header and its frequency can land on
            // different wakes.
            let mut parsed = Vec::new();
            {
                let feed = self.feeds.entry(id).or_default();
                for line in &lines {
                    parsed.extend(feed.push_line(line));
                }
            }
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
            for event in &parsed {
                match event {
                    follow::CcEvent::Grant { tg, alg, .. } => {
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
            if !self.followers.contains_key(&id) {
                self.followers
                    .insert(id, follow::Follower::new(control_hz.clone()));
            }
            if let Some(follower) = self.followers.get_mut(&id) {
                if !follower.controls_match(&control_hz) {
                    follower.set_controls(control_hz.clone());
                }
            }
            if let Some(follower) = self.followers.get_mut(&id) {
                for event in parsed {
                    follower.push(event);
                }
            }
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
    /// Closing the window writes it too. A crash does not reach that close.
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

        let (status, tg, live, encrypted, hz) = if let Some(follower) = self.followers.get(&id) {
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
            } else {
                format!("{:.0} dB", snap.strength_db)
            }
        } else {
            "—".to_string()
        };
        let freq = hz.map(|hz| format!("{:.6} MHz", hz / 1_000_000.0));
        let (headline, headline_live) = if tg.is_some() && !alias.is_empty() {
            (alias, live)
        } else if status.starts_with("scanning") {
            ("SCAN".to_string(), false)
        } else {
            (status.clone(), false)
        };

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
        let tgid = match tg {
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
        let facts = format!("Sys {sys}   RFSS {rfss}   Site {site_id}");
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
        let net = format!("WACN {wacn}    NAC {nac}");
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

    fn page_talkgroups(&mut self, ui: &mut egui::Ui, id: u64, snap: &Snapshot) {
        let rows = self.hub.view().rows;
        let Some(row) = rows.iter().find(|row| row.id == id) else {
            ui.label("This radio is gone.");
            return;
        };
        let p25 = crate::rx_pipe::is_p25_rx(&row.spec.rx_pipe);
        let selected = row.selected;
        let radio_name = row.spec.name.clone();
        let gpredict = row.spec.follow;
        let mut follow = p25 && self.cc_on.get(&id).copied().unwrap_or(false);
        if !self.talkgroup_note.is_empty() {
            ui.label(
                egui::RichText::new(&self.talkgroup_note)
                    .color(egui::Color32::from_rgb(220, 160, 80)),
            );
        }
        let active = self.followers.get(&id).and_then(|follower| follower.active_tg());
        ui.horizontal_top(|ui| {
            // The face is 204 px. Three 46 px keys with 6 px between them
            // sit on the vertical middle of that bezel, 4 px from its edge.
            ui.spacing_mut().item_spacing.x = 4.0;
            let face_h = 204.0_f32;
            let key_h = 46.0;
            let key_gap = 6.0;
            let stack = key_h * 3.0 + key_gap * 2.0;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.set_width(72.0);
                ui.add_space(((face_h - stack) * 0.5).max(0.0));
                if radio_key(ui, "LOAD", false, true).clicked() {
                    self.browse_talkgroups(false);
                }
                ui.add_space(key_gap);
                if radio_key(ui, "SAVE", false, true).clicked() {
                    self.browse_talkgroups(true);
                }
                ui.add_space(key_gap);
                let toggle = radio_key(ui, "FOLLOW", follow, p25);
                if toggle.clicked() && p25 {
                    follow = !follow;
                }
                if !p25 {
                    toggle.on_hover_text("The P25 receive pipe is off.");
                } else if gpredict {
                    toggle.on_hover_text("Gpredict is moving this radio.");
                }
            });
            self.scanner_face(ui, id, selected, &radio_name, follow, snap);
        });
        if self.cc_on.get(&id).copied().unwrap_or(false) != follow {
            self.cc_on.insert(id, follow);
            if !self.config_path.is_empty() {
                self.save_config(snap);
            }
        }
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
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 26.0),
                                egui::Sense::click(),
                            );
                            if decoding {
                                // Same dark red as the talkgroup row for this grant.
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(4),
                                    egui::Color32::from_rgb(146, 24, 18),
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
                            ui.painter().text(
                                rect.left_center() + egui::vec2(8.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                &name,
                                egui::FontId::proportional(14.0),
                                ui.visuals().text_color(),
                            );
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
                                    if self.talkgroup_row(ui, idx, active) {
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

    fn talkgroup_row(&self, ui: &mut egui::Ui, idx: usize, active: Option<u32>) -> bool {
        let Some(row) = self
            .talkgroups
            .zones
            .get(self.talkgroup_zone)
            .and_then(|zone| zone.talkgroups.get(idx))
        else {
            return false;
        };
        let slot = format!("{:>4}", idx + 1);
        let mut text = format!("{slot}   {:>8}   {}", row.tg, row.name);
        if row.encrypted {
            text.push_str("  enc");
        }
        let followed = active == Some(row.tg);
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        if followed {
            // Same dark red as the meter needle. The row is decoding this grant.
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(4),
                egui::Color32::from_rgb(146, 24, 18),
            );
        } else if self.talkgroup_sel == idx {
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
            text,
            egui::FontId::monospace(14.0),
            ui.visuals().text_color(),
        );
        response.clicked()
    }

    fn talkgroup_editor(&mut self, ui: &mut egui::Ui, width: f32) {
        let frame = egui::Frame::group(ui.style());
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
            ui.checkbox(&mut encrypted, "Encrypted");
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
            name: String::new(),
            tg: next,
            encrypted: false,
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
    egui::CentralPanel::default().show_inside(ui, |ui| {
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
                let id = ui.id().with(salt);
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
        if ui
            .add_enabled(enabled, egui::Button::new(label).selected(on))
            .clicked()
        {
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
        if ui
            .add_enabled(enabled, egui::Button::new(label).selected(on))
            .clicked()
        {
            self.send(Command::SetLevel {
                name: "PREAMP".into(),
                value: db,
            });
        }
    }

    fn ptt(&mut self, ui: &mut egui::Ui, snap: &Snapshot, label: &str) {
        let enabled = snap.online() && snap.caps.can_set_ptt;
        if ui
            .add_enabled(enabled, egui::Button::new(label).selected(snap.ptt))
            .clicked()
        {
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
        let label = if snap.power_on { "RX" } else { "Off" };
        let enabled = snap.online() && snap.caps.can_set_power;
        ui.add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt("receiver")
                .selected_text(format!("Receiver: {label}"))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(snap.power_on, "RX").clicked() {
                        self.send(Command::SetPower(true));
                    }
                    if ui.selectable_label(!snap.power_on, "Off").clicked() {
                        self.send(Command::SetPower(false));
                    }
                });
        });
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
    loop {
        // The condvar is waited on `shared`. The hub's kick holds that same
        // mutex before it notifies, and this loop reads `face.connect` while
        // holding it, so a handed port is not missed.
        let (cmds, connect, classic) = {
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
        let mut sent = None;
        let mut remote = None;
        for cmd in &cmds {
            sent = Some(cmd.cat_line());
            match session.apply(cmd) {
                Ok(()) => {}
                Err(RigError::Remote(msg)) => remote = Some(msg),
                Err(RigError::Io(msg)) => {
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
