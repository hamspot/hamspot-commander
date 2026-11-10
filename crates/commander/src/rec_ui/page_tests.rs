//! The REC page driven the way eframe drives it: the real `App`, its whole
//! `ui()` (or the detached page window), real LCARS fonts and theme, and
//! the wheel events eframe sends (line units from a mouse, point units from
//! a touchpad, `ModifiersChanged` around them). Each test runs in a child
//! process with a temporary HOME, so nothing of the user's is read or written.

use super::*;
use eframe::App as _;
use std::path::{Path, PathBuf};

/// Run `name` again in a child process with a private HOME. Returns true in
/// the child (the caller then does the work), false in the parent.
fn in_child(name: &str) -> bool {
    if std::env::var_os("HS_REC_PAGE_CHILD").is_some() {
        return true;
    }
    let tmp = std::env::temp_dir().join(format!("hs-rec-page-{}-{}", std::process::id(), name.replace("::", "-")));
    let _ = std::fs::remove_dir_all(&tmp);
    for d in ["config", "data", "cache", "state"] {
        std::fs::create_dir_all(tmp.join(d)).unwrap();
    }
    let me = format!("rec_ui::page_tests::{name}");
    // The child's App starts the dummy rigctld: one at a time with other rig tests.
    let _lock = crate::cat::test_rig_lock();
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([me.as_str(), "--exact", "--nocapture", "--test-threads=1"])
        .env("HS_REC_PAGE_CHILD", "1")
        .env("HOME", &tmp)
        .env("XDG_CONFIG_HOME", tmp.join("config"))
        .env("XDG_DATA_HOME", tmp.join("data"))
        .env("XDG_CACHE_HOME", tmp.join("cache"))
        .env("XDG_STATE_HOME", tmp.join("state"))
        .output()
        .unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(out.status.success() && text.contains("1 passed"), "child failed:\n{text}");
    false
}

/// A station beside a REC database: a copy of `db` when given (never the
/// original), or a built one with `lanes` talkgroups and a burst of calls.
fn station(db: Option<&Path>, lanes: u32) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hs-rec-station-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let st = dir.join("SAFE-T.HSCommander");
    std::fs::write(&st, "").unwrap();
    let path = hsrec::hsdata::db_path(&st);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    if let Some(src) = db {
        std::fs::copy(src, &path).unwrap();
    } else {
        let d = Db::open(&path).unwrap();
        let now = chrono::Utc::now();
        for tg in 0..lanes {
            for k in 0..3 {
                let t0 = now - chrono::Duration::seconds(3000 - tg as i64 * 60 - k * 9);
                let t1 = t0 + chrono::Duration::seconds(4);
                let mut m = Meta {
                    source: meta::SOURCE_HACKRF_TG.into(),
                    source_kind: meta::KIND_RAW_IQ.into(),
                    talkgroup: Some(10000 + tg),
                    talkgroup_name: format!("Lane {tg:02}"),
                    raw_format: "cs16".into(),
                    sample_rate: 32_000,
                    start_utc: t0.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    end_utc: t1.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    duration_ms: 4000,
                    bytes: 128_000,
                    ..Default::default()
                };
                m.centre_hz = 857_200_000.0;
                m.channel_hz = 856_962_500.0;
                m.offset_hz = m.channel_hz - m.centre_hz;
                d.insert(&format!("hackrf_tg/x/{tg}_{k}.cs16"), &m).unwrap();
            }
        }
    }
    st
}

struct Harness {
    ctx: egui::Context,
    app: crate::App,
    frame: eframe::Frame,
    size: egui::Vec2,
    time: f64,
    tex: HashMap<egui::TextureId, egui::ColorImage>,
    detached: bool,
}

impl Harness {
    fn new(station: &Path, size: egui::Vec2, detached: bool) -> Self {
        let ctx = egui::Context::default();
        rustcars::install_lcars_font(&ctx);
        rustcars::apply_theme(&ctx, &rustcars::LcarsTheme::default());
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = crate::App::new(&cc);
        app.config_note.clear();
        app.config_path = station.display().to_string();
        app.page = crate::Page::Rec;
        Self { ctx, app, frame: eframe::Frame::_new_kittest(), size, time: 0.0, tex: HashMap::new(), detached }
    }

    /// One eframe frame with `events`.
    fn run(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let detached = self.detached;
        let (app, frame) = (&mut self.app, &mut self.frame);
        let mut out = self.ctx.run_ui(input, |ui| {
            if detached {
                // As the detached REC window: rustcars' real frame, the
                // viewport callback run like eframe's immediate viewport.
                let mut st = crate::group_box::DetachState { detached: true, ..Default::default() };
                let c = ui.ctx().clone();
                let id = crate::group_box::DetachState::id(("page", crate::page_name(crate::Page::Rec)));
                let mut snap = crate::empty_snap();
                rustcars::group::detached_window_in_opts(
                    &c, id, &crate::group_box::Metrics::default(), &crate::page_name(crate::Page::Rec), rustcars::palette::TAN, &mut st,
                    crate::detach_opts(crate::Page::Rec), |ui| app.page_body_of(crate::Page::Rec, true, ui, &mut snap),
                    |_, _, cb| cb(ui, true),
                );
            } else {
                app.ui(ui, frame);
            }
        });
        for (id, ds) in std::mem::take(&mut out.textures_delta.set) {
            for d in ds {
                let egui::ImageData::Color(img) = &d.image;
                match d.pos {
                    None => {
                        self.tex.insert(id, egui::ColorImage::clone(img));
                    }
                    Some([x0, y0]) => {
                        if let Some(t) = self.tex.get_mut(&id) {
                            for y in 0..img.size[1] {
                                for x in 0..img.size[0] {
                                    t.pixels[(y0 + y) * t.size[0] + x0 + x] = img.pixels[y * img.size[0] + x];
                                }
                            }
                        }
                    }
                }
            }
        }
        out.textures_delta.clear();
        out
    }

    fn settle(&mut self, n: usize) -> egui::FullOutput {
        let mut out = self.run(vec![]);
        for _ in 1..n {
            out = self.run(vec![]);
        }
        out
    }

    /// The wheel the way eframe sends it: modifiers first, one wheel event.
    fn wheel(&mut self, at: egui::Pos2, unit: egui::MouseWheelUnit, delta: egui::Vec2, mods: egui::Modifiers) -> egui::FullOutput {
        let ev = vec![
            egui::Event::PointerMoved(at),
            egui::Event::ModifiersChanged(mods),
            egui::Event::MouseWheel { unit, delta, modifiers: mods, phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel },
        ];
        let out = self.run(ev);
        self.run(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        out
    }

    fn view(&self) -> View {
        *self.app.rec.views.get(meta::SOURCE_HACKRF_TG).expect("a view for HackRF TG")
    }

    fn lane_off(&self) -> f32 {
        self.app.rec.lane_off.get(meta::SOURCE_HACKRF_TG).copied().unwrap_or(0.0)
    }

    /// Software render of `out` to a PNG (for showing what the page looks like).
    fn png(&self, out: &egui::FullOutput, path: &Path) {
        let ppp = out.pixels_per_point;
        let prims = self.ctx.tessellate(out.shapes.clone(), ppp);
        let (w, h) = ((self.size.x * ppp) as usize, (self.size.y * ppp) as usize);
        let mut buf = vec![[0f32; 4]; w * h];
        for p in prims {
            let egui::epaint::Primitive::Mesh(mesh) = p.primitive else { continue };
            let Some(tex) = self.tex.get(&mesh.texture_id) else { continue };
            let clip = p.clip_rect;
            let (cx0, cy0) = ((clip.min.x * ppp).max(0.0) as i64, (clip.min.y * ppp).max(0.0) as i64);
            let (cx1, cy1) = (((clip.max.x * ppp) as i64).min(w as i64), ((clip.max.y * ppp) as i64).min(h as i64));
            for tri in mesh.indices.chunks(3) {
                let v = [&mesh.vertices[tri[0] as usize], &mesh.vertices[tri[1] as usize], &mesh.vertices[tri[2] as usize]];
                let pt = |i: usize| (v[i].pos.x * ppp, v[i].pos.y * ppp);
                let (a, b, c) = (pt(0), pt(1), pt(2));
                let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
                if area.abs() < 1e-6 {
                    continue;
                }
                let x0 = (a.0.min(b.0).min(c.0).floor() as i64).max(cx0);
                let x1 = (a.0.max(b.0).max(c.0).ceil() as i64).min(cx1);
                let y0 = (a.1.min(b.1).min(c.1).floor() as i64).max(cy0);
                let y1 = (a.1.max(b.1).max(c.1).ceil() as i64).min(cy1);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                        let w0 = ((b.0 - px) * (c.1 - py) - (b.1 - py) * (c.0 - px)) / area;
                        let w1 = ((c.0 - px) * (a.1 - py) - (c.1 - py) * (a.0 - px)) / area;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                            continue;
                        }
                        let u = v[0].uv.x * w0 + v[1].uv.x * w1 + v[2].uv.x * w2;
                        let vv = v[0].uv.y * w0 + v[1].uv.y * w1 + v[2].uv.y * w2;
                        let tx = ((u * tex.size[0] as f32) as usize).min(tex.size[0] - 1);
                        let ty = ((vv * tex.size[1] as f32) as usize).min(tex.size[1] - 1);
                        let t = tex.pixels[ty * tex.size[0] + tx].to_array();
                        let mut src = [0f32; 4];
                        for k in 0..4 {
                            let col = v[0].color.to_array()[k] as f32 * w0 + v[1].color.to_array()[k] as f32 * w1 + v[2].color.to_array()[k] as f32 * w2;
                            src[k] = col / 255.0 * t[k] as f32 / 255.0;
                        }
                        let d = &mut buf[y as usize * w + x as usize];
                        for k in 0..4 {
                            d[k] = src[k] + d[k] * (1.0 - src[3]);
                        }
                    }
                }
            }
        }
        let mut img = image::RgbaImage::new(w as u32, h as u32);
        for (i, px) in img.pixels_mut().enumerate() {
            let d = buf[i];
            *px = image::Rgba([(d[0] * 255.0) as u8, (d[1] * 255.0) as u8, (d[2] * 255.0) as u8, 255]);
        }
        img.save(path).unwrap();
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.app.hub.shutdown();
        self.app.rotators.shutdown();
        self.app.amplifiers.shutdown();
    }
}

/// Text shapes on screen (inside their clip): text and rect.
fn texts(out: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => {
                let r = t.visual_bounding_rect().intersect(c.clip_rect);
                r.is_positive().then(|| (t.galley.text().to_string(), r))
            }
            _ => None,
        })
        .collect()
}

/// Filled rects of `color` on screen (inside their clip).
fn fills(out: &egui::FullOutput, color: egui::Color32) -> Vec<egui::Rect> {
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == color => {
                let v = r.rect.intersect(c.clip_rect);
                v.is_positive().then_some(v)
            }
            _ => None,
        })
        .collect()
}

/// The lanes' area on screen: between the "Lane" axis header and the
/// status strip, from the text layout.
fn lanes_area(out: &egui::FullOutput) -> egui::Rect {
    let t = texts(out);
    let head = t.iter().find(|(s, _)| s == "Lane").map(|(_, r)| *r).expect("the Lane header");
    // The status strip is the lowest "REC…" text (the page tab is at the top).
    let status = t.iter().filter(|(s, _)| s.starts_with("REC")).map(|(_, r)| r.top()).fold(f32::NEG_INFINITY, f32::max);
    let status = if status > head.bottom() { status } else { f32::INFINITY };
    egui::Rect::from_min_max(egui::pos2(head.left(), head.bottom() + 2.0), egui::pos2(f32::INFINITY, status - 2.0))
}

fn lane_label(out: &egui::FullOutput, name: &str) -> Option<egui::Rect> {
    texts(out).into_iter().find(|(s, _)| s.starts_with(name)).map(|(_, r)| r)
}

fn run_page_case(detached: bool) {
    let st = station(None, 30);
    let mut h = Harness::new(&st, egui::vec2(1200.0, 560.0), detached);
    let out = h.settle(8);
    let area = lanes_area(&out);
    let first = lane_label(&out, "Lane 00").expect("first lane shown");
    assert!(area.contains(first.center()));
    let last_hidden = lane_label(&out, "Lane 29").is_none_or(|r| !area.contains(r.center()));
    assert!(last_hidden, "30 lanes do not fit in 560 px");
    // A scrollbar is there to drag: a thin rect right of the lanes.
    let over = egui::pos2(area.left() + 600.0, area.top() + 60.0);
    // Plain wheel over the lanes zooms time (mouse: line units).
    let span0 = h.view().span;
    let off0 = h.lane_off();
    h.wheel(over, egui::MouseWheelUnit::Line, egui::vec2(0.0, 1.0), egui::Modifiers::NONE);
    let span1 = h.view().span;
    assert!(span1 < span0, "plain wheel zooms in: {span0} -> {span1}");
    assert_eq!(h.lane_off(), off0, "plain wheel does not scroll the lanes");
    // Touchpad (point units) zooms out the same way.
    h.wheel(over, egui::MouseWheelUnit::Point, egui::vec2(0.0, -80.0), egui::Modifiers::NONE);
    assert!(h.view().span > span1);
    // Ctrl+wheel zooms too.
    let s = h.view().span;
    h.wheel(over, egui::MouseWheelUnit::Line, egui::vec2(0.0, 1.0), egui::Modifiers::CTRL);
    assert!(h.view().span < s, "ctrl+wheel zooms");
    // Shift+wheel scrolls the lanes, the time view stays.
    let s = h.view().span;
    let mut out = h.settle(1);
    for _ in 0..40 {
        h.wheel(over, egui::MouseWheelUnit::Line, egui::vec2(0.0, -3.0), egui::Modifiers::SHIFT);
        out = h.settle(2);
        if lane_label(&out, "Lane 29").is_some_and(|r| lanes_area(&out).contains(r.center())) {
            break;
        }
    }
    assert_eq!(h.view().span, s, "shift+wheel does not zoom");
    assert!(h.lane_off() > 0.0);
    let area = lanes_area(&out);
    assert!(lane_label(&out, "Lane 29").is_some_and(|r| area.contains(r.center())), "shift+wheel reveals the last lane");
    assert!(lane_label(&out, "Lane 00").is_none_or(|r| !area.contains(r.center())), "the first lane scrolled away");
    // Some winit backends send shift+wheel sideways.
    h.wheel(over, egui::MouseWheelUnit::Line, egui::vec2(3.0, 0.0), egui::Modifiers::SHIFT);
    let up = h.lane_off();
    assert!(up < 1e9);
    // A plain wheel over the lane names scrolls back to the top.
    let names = egui::pos2(area.left() + 30.0, area.top() + 40.0);
    for _ in 0..40 {
        h.wheel(names, egui::MouseWheelUnit::Line, egui::vec2(0.0, 3.0), egui::Modifiers::NONE);
    }
    let out = h.settle(2);
    assert_eq!(h.lane_off(), 0.0);
    assert!(lane_label(&out, "Lane 00").is_some_and(|r| lanes_area(&out).contains(r.center())));
    // The time axis header stays put above the lanes.
    assert!(texts(&out).iter().any(|(s, r)| s == "Lane" && (r.top() - area.top()).abs() < 40.0));
}

#[test]
fn real_page_wheel_zooms_and_shift_wheel_scrolls_lanes() {
    if !in_child("real_page_wheel_zooms_and_shift_wheel_scrolls_lanes") {
        return;
    }
    run_page_case(false);
}

#[test]
fn real_detached_page_wheel_zooms_and_shift_wheel_scrolls_lanes() {
    if !in_child("real_detached_page_wheel_zooms_and_shift_wheel_scrolls_lanes") {
        return;
    }
    run_page_case(true);
}

#[test]
fn real_page_draws_a_live_call_within_a_second_and_keeps_it() {
    if !in_child("real_page_draws_a_live_call_within_a_second_and_keeps_it") {
        return;
    }
    let st = station(None, 5);
    let mut h = Harness::new(&st, egui::vec2(1200.0, 700.0), false);
    h.settle(8);
    assert!(fills(&h.settle(1), LIVE_BLOCK).is_empty());
    // The live recorder opens a call, as the segmenter does on a grant.
    let now = chrono::Utc::now();
    let m = Meta {
        source: meta::SOURCE_HACKRF_TG.into(),
        source_kind: meta::KIND_RAW_IQ.into(),
        talkgroup: Some(10741),
        talkgroup_name: "ISP IND DISP — District 52 - Indianapolis Dispatch".into(),
        raw_format: "cs16".into(),
        sample_rate: 32_000,
        start_utc: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        centre_hz: 857_200_000.0,
        channel_hz: 856_962_500.0,
        offset_hz: -237_500.0,
        ..Default::default()
    };
    let key = 0xC0FFEE;
    crate::rec_live::recorder().send(Cmd::Open { key, meta: m });
    let t0 = std::time::Instant::now();
    let mut seen = None;
    while t0.elapsed() < std::time::Duration::from_millis(1000) {
        let out = h.run(vec![]);
        let area = lanes_area(&out);
        if let Some(r) = fills(&out, LIVE_BLOCK).into_iter().find(|r| area.intersects(*r)) {
            seen = Some((t0.elapsed(), r, out));
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    let (after, block, out) = seen.expect("the live call is drawn within 1 s");
    assert!(block.width() >= MIN_BLOCK_W - 0.5, "visible, not a hairline: {block:?}");
    assert!(lane_label(&out, "ISP IND DISP").is_some(), "its lane is listed");
    eprintln!("live call drawn after {after:?} at {block:?}");
    // It ends: still there, now as a finished call.
    // It carries signal (an empty recording is not kept), then ends.
    crate::rec_live::push(key, 856_962_500, vec![100i16; 32_000]);
    std::thread::sleep(std::time::Duration::from_millis(300));
    crate::rec_live::recorder().send(Cmd::Close { key, keep: true });
    let t1 = std::time::Instant::now();
    let mut kept = false;
    while t1.elapsed() < std::time::Duration::from_secs(5) {
        let out = h.run(vec![]);
        let green = fills(&out, egui::Color32::from_rgb(46, 120, 70));
        let label = lane_label(&out, "ISP IND DISP");
        if fills(&out, LIVE_BLOCK).is_empty() && label.is_some_and(|l| green.iter().any(|g| (g.center().y - l.center().y).abs() < LANE_H)) {
            kept = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    assert!(kept, "the finished call stays in its lane");
}

/// A HackRF row for the tests: no hardware, the P25 pipe as its decoder.
fn hackrf_row(id: u64, center: f64) -> cat::RadioRow {
    let mut spec = cat::RadioSpec::dummy();
    spec.driver = cat::RadioDriver::Hackrf;
    spec.name = "HackRF".into();
    spec.rx_pipe = "modems/commander-demod-p25.rxpipe".into();
    cat::RadioRow {
        id,
        spec,
        port: None,
        status: String::new(),
        selected: false,
        make: String::new(),
        model_label: String::new(),
        audio_status: String::new(),
        rx_pipe_status: String::new(),
        rx_chain: String::new(),
        rx_pipe_diverted: false,
        tx_pipe_status: String::new(),
        tx_pipe_diverted: false,
        center_hz: center,
        voice_empty: false,
    }
}

fn install_modems() {
    use std::os::unix::fs::PermissionsExt;
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modems");
    let dst = crate::rx_pipe::installed_modem_dirs().into_iter().next().expect("data home");
    std::fs::create_dir_all(&dst).unwrap();
    for e in std::fs::read_dir(&src).unwrap().flatten() {
        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            let to = dst.join(e.file_name());
            std::fs::copy(e.path(), &to).unwrap();
            let mode = std::fs::metadata(e.path()).unwrap().permissions().mode() & 0o755;
            std::fs::set_permissions(&to, std::fs::Permissions::from_mode(mode)).unwrap();
        }
    }
    let mut d = dst.as_path();
    for _ in 0..2 {
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o755)).unwrap();
        d = d.parent().unwrap();
    }
}

fn p25_syncs(disc: &[i16]) -> usize {
    let argv = crate::rx_pipe::command_argv("modems/commander-demod-p25.rxpipe").expect("p25 pipe file");
    let mut child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .env("HAMSPOT_CC_EVENTS", "1")
        .env("HAMSPOT_RX_MUTE", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("p25 pipe");
    let mut stdin = child.stdin.take().unwrap();
    let bytes: Vec<u8> = disc.iter().flat_map(|s| s.to_le_bytes()).collect();
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        let _ = stdin.write_all(&bytes);
    });
    let out = child.wait_with_output().expect("p25 pipe output");
    writer.join().unwrap();
    String::from_utf8_lossy(&out.stderr).matches("+P25p1").count()
}

/// Three calls granted at once inside one HackRF capture. The speaker
/// follows A (priority); B and C are recorded at the same time in their own
/// talkgroup files, drawn live in their lanes, and B replays from REC.
#[test]
fn real_page_records_every_concurrent_call_while_one_is_heard() {
    if !in_child("real_page_records_every_concurrent_call_while_one_is_heard") {
        return;
    }
    // The child has a private HOME: install the repo's modems there, owner
    // writable only, as an installed copy has them.
    install_modems();
    let st = station(None, 0);
    let mut h = Harness::new(&st, egui::vec2(1200.0, 700.0), false);
    h.app.rec.face.talkgroups = hsrec::segment::RecMode::All;
    h.app.rec.face.source_tab = meta::SOURCE_HACKRF_TG.into();
    h.settle(8);
    let radio = 0x4ACC;
    let center = 857_200_000.0;
    let (a, b, c) = (856_962_500.0, 857_437_500.0, 857_712_500.0);
    let (tg_a, tg_b, tg_c) = (10_119u32, 10_741u32, 10_334u32);
    h.app.rec.test_rows.push(hackrf_row(radio, center));
    h.app.hackrf_call.insert(radio, Some((a, tg_a)));
    h.app.hackrf_open.insert(
        radio,
        vec![
            crate::HeardGrant { hz: a, tg: tg_a, seq: 1 },
            crate::HeardGrant { hz: b, tg: tg_b, seq: 2 },
            crate::HeardGrant { hz: c, tg: tg_c, seq: 3 },
        ],
    );
    h.settle(2);
    let mut taps = crate::rec_live::grant_taps(radio);
    taps.sort_unstable();
    assert_eq!(taps, vec![b as u64, c as u64], "B and C are channelised besides the speaker's A");
    let open: Vec<u64> = h.app.rec.grant_open.keys().map(|(_, hz)| *hz).collect();
    assert_eq!(open.len(), 2, "two talkgroup files open beside the speaker's call: {open:?}");

    // The capture: A and B on the air, C silent. Same routing as the USB thread.
    let cs8 = crate::hackrf::tests::two_calls_cs8(center, a, b, 1_627.0, 2.0);
    let mut bank = crate::hackrf::Bank::new(center, &[]);
    bank.set_voice(Some(a));
    let mut voice = Vec::new();
    let mut most_live = 0;
    for (k, block) in cs8.chunks(262_144).enumerate() {
        bank.set_extras(&crate::rec_live::grant_taps(radio));
        bank.set_tap(true);
        voice.clear();
        bank.push(block, &mut voice, &mut []);
        crate::rec_live::note_extras(radio, &bank.extras());
        crate::hackrf::route_taps(radio, a as u64, bank.take_carrier_taps());
        if k % 4 == 0 {
            let out = h.run(vec![]);
            let area = lanes_area(&out);
            most_live = most_live.max(fills(&out, LIVE_BLOCK).into_iter().filter(|r| area.intersects(*r)).count());
        }
    }
    let t0 = std::time::Instant::now();
    while most_live < 2 && t0.elapsed() < std::time::Duration::from_secs(2) {
        let out = h.run(vec![]);
        let area = lanes_area(&out);
        most_live = most_live.max(fills(&out, LIVE_BLOCK).into_iter().filter(|r| area.intersects(*r)).count());
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    assert!(most_live >= 2, "concurrent calls drawn live in their own lanes at once ({most_live})");

    // The calls end: their grants leave the list.
    h.app.hackrf_call.insert(radio, None);
    h.app.hackrf_open.insert(radio, Vec::new());
    h.settle(2);
    assert!(h.app.rec.grant_open.is_empty());
    assert!(crate::rec_live::grant_taps(radio).is_empty());
    let db = Db::open(&h.app.rec.db_path).unwrap();
    let t1 = std::time::Instant::now();
    let rec_b = loop {
        let r = db.by_talkgroup(tg_b).unwrap().into_iter().find(|r| !r.meta.end_utc.is_empty() && r.meta.bytes > 0);
        if let Some(r) = r {
            break r;
        }
        assert!(t1.elapsed() < std::time::Duration::from_secs(5), "B was filed");
        h.run(vec![]);
        std::thread::sleep(std::time::Duration::from_millis(30));
    };
    assert_eq!(rec_b.meta.source, meta::SOURCE_HACKRF_TG);
    assert_eq!(rec_b.meta.raw_format, "cs16");
    assert_eq!(rec_b.meta.channel_hz, b);
    assert!(rec_b.meta.bytes >= 200_000, "about 2 s of raw IQ: {} bytes", rec_b.meta.bytes);
    let path = h.app.rec.dir.join(&rec_b.raw_path);
    assert!(std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) >= 200_000, "raw IQ on disk");
    // C was granted and filed too (it was silent, so only noise).
    assert!(!db.by_talkgroup(tg_c).unwrap().is_empty(), "C has its own file");
    // Played from REC: the same decoder and re-demodulation REC playback uses.
    let decoder = h.app.rec_decoder(&rec_b.meta);
    assert!(decoder.as_ref().is_some_and(|d| d.argv[0].ends_with("commander-demod-p25.rxpipe")), "{decoder:?} {}", h.app.rec.note);
    let disc = hsrec::replay::decode_all(&path, &rec_b.meta).unwrap();
    assert!(disc.len() > 60_000, "replay re-demodulates B ({} samples)", disc.len());
    if std::process::Command::new("sh").args(["-c", "command -v dsd-fme"]).output().map(|o| o.status.success()).unwrap_or(false) {
        let syncs = p25_syncs(&disc);
        assert!(syncs >= 10, "B's replay decodes as P25: {syncs} syncs");
    } else {
        eprintln!("dsd-fme not on PATH: P25 decode of B skipped");
    }
}

/// Dead air: a zone channel inside the capture is filed on HACKRF CHANNELS
/// only while the REC squelch hears a signal on it, and closes after the
/// hang. A silent channel gets no file at all.
#[test]
fn real_page_channel_files_follow_the_rf_squelch() {
    if !in_child("real_page_channel_files_follow_the_rf_squelch") {
        return;
    }
    let st = station(None, 0);
    let mut h = Harness::new(&st, egui::vec2(1200.0, 700.0), false);
    h.app.rec.face.talkgroups = hsrec::segment::RecMode::All;
    h.app.rec.face.source_tab = meta::SOURCE_HACKRF_CHANNELS.into();
    let radio = 0x5EC1;
    let center = 857_200_000.0;
    let (a, b, c) = (856_962_500.0, 857_437_500.0, 857_712_500.0);
    let ch = |name: &str, hz: f64| crate::channels::Channel {
        name: name.into(),
        hz,
        mode: "nfm".into(),
        control: false,
        encrypted: false,
        vocoder: String::new(),
        invert: false,
        record: false,
    };
    h.app.channels.zones.push(crate::channels::ChannelZone {
        name: "Squelch".into(),
        channels: vec![ch("Squelch A", a), ch("Squelch B", b), ch("Squelch C", c)],
    });
    h.app.channel_zone = h.app.channels.zones.len() - 1;
    h.app.rec.test_rows.push(hackrf_row(radio, center));
    h.settle(2);
    assert!(h.app.rec.carrier_open.get(&radio).is_none_or(|o| o.is_empty()), "nothing heard yet: no channel files");

    // The capture thread: A and B on the air, C silent, every channel mixed.
    let cs8 = crate::hackrf::tests::two_calls_cs8(center, a, b, 1_627.0, 1.5);
    let mut bank = crate::hackrf::Bank::new(center, &[]);
    bank.set_extras(&[a as u64, b as u64, c as u64]);
    bank.set_tap(true);
    let mut voice = Vec::new();
    for block in cs8.chunks(65_536) {
        voice.clear();
        bank.push(block, &mut voice, &mut []);
        crate::rec_live::note_gates(radio, &bank.gates());
        crate::hackrf::route_taps(radio, 0, bank.take_carrier_taps());
        h.run(vec![]);
    }
    let key = |hz: f64| crate::rec_live::carrier_key(radio, hz as u64);
    let open = h.app.rec.carrier_open.get(&radio).cloned().unwrap_or_default();
    assert!(
        open.contains(&key(a)) && open.contains(&key(b)),
        "A and B on the air are filed: open {open:?} squelch {:?}",
        bank.gates()
    );
    assert!(!open.contains(&key(c)), "silent C is not filed");

    // The calls end; after the hang the files close and are kept.
    let t0 = std::time::Instant::now();
    while h.app.rec.carrier_open.get(&radio).is_some_and(|o| !o.is_empty()) {
        assert!(t0.elapsed() < std::time::Duration::from_secs(4), "channel files close after the hang");
        h.run(vec![]);
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    assert!(t0.elapsed() >= crate::rec_ui::SIGNAL_HANG / 2, "the hang holds a file through a short pause");
    let db = Db::open(&h.app.rec.db_path).unwrap();
    let t1 = std::time::Instant::now();
    let rows = loop {
        let rows: Vec<_> = db.all().unwrap().into_iter().filter(|r| r.meta.source == meta::SOURCE_HACKRF_CHANNELS).collect();
        if rows.len() >= 2 && rows.iter().all(|r| !r.meta.end_utc.is_empty()) {
            break rows;
        }
        assert!(t1.elapsed() < std::time::Duration::from_secs(5), "A and B filed: {}", rows.len());
        h.run(vec![]);
        std::thread::sleep(std::time::Duration::from_millis(30));
    };
    let hz: Vec<f64> = rows.iter().map(|r| r.meta.channel_hz).collect();
    assert!(hz.contains(&a) && hz.contains(&b) && !hz.contains(&c), "{hz:?}");
    for r in &rows {
        assert!(r.meta.bytes > 100_000, "real IQ kept: {} bytes on {}", r.meta.bytes, r.meta.channel_hz);
    }
    // Drawn on the CHANNELS tab: blocks for A and B in their lanes.
    h.app.rec_reload(true);
    h.settle(2);
    let out = h.run(vec![]);
    let lanes = lanes_area(&out);
    let shown: Vec<String> = texts(&out).into_iter().filter(|(_, r)| r.max.y > lanes.min.y).map(|(t, _)| t).collect();
    let has = |n: &str| shown.iter().any(|t| t.contains(n));
    assert!(has("Squelch A") && has("Squelch B"), "lanes for the live channels: {shown:?}");
    assert!(!has("Squelch C"), "no lane for the silent channel: {shown:?}");
}

/// A P25 channel recording (filed as "fm" on HACKRF CHANNELS) clicked on
/// the timeline: the Record / Playback dialog opens, the Demodulator
/// defaults to the P25 pipe and playback runs through that decoder, not the
/// raw discriminator. FM audio can be picked instead. Closing the dialog
/// stops playback.
#[test]
fn real_page_channel_recording_plays_through_p25_and_shows_the_dialog() {
    if !in_child("real_page_channel_recording_plays_through_p25_and_shows_the_dialog") {
        return;
    }
    install_modems();
    let st = station(None, 0);
    let mut h = Harness::new(&st, egui::vec2(1200.0, 760.0), false);
    h.app.rec.face.talkgroups = hsrec::segment::RecMode::All;
    h.app.rec.face.source_tab = meta::SOURCE_HACKRF_CHANNELS.into();
    h.app.rec.play_argv = Some(vec!["sh".into(), "-c".into(), "cat > /dev/null".into()]);
    h.settle(4);
    let raw = "hackrf_channels/x/856_9625.cs16";
    let path = h.app.rec.dir.join(raw);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let iq = hsrec::dsp::synth_c4fm_cs16(1_627.0, 3.0, 9_000.0);
    std::fs::write(&path, iq.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let t0 = chrono::Utc::now() - chrono::Duration::seconds(8);
    let mut m = Meta {
        source: meta::SOURCE_HACKRF_CHANNELS.into(),
        source_kind: meta::KIND_RAW_IQ.into(),
        channel_name: "856.9625".into(),
        raw_format: "cs16".into(),
        sample_rate: 32_000,
        start_utc: t0.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        end_utc: (t0 + chrono::Duration::seconds(3)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        duration_ms: 3000,
        bytes: (iq.len() * 2) as i64,
        modulation: "fm".into(),
        centre_hz: 857_200_000.0,
        channel_hz: 856_962_500.0,
        offset_hz: -237_500.0,
        ..Default::default()
    };
    m.decoder = serde_json::json!({ "script": "modems/commander-demod-p25.rxpipe", "vocoder": "", "mode": "FM", "invert": false });
    let id = Db::open(&h.app.rec.db_path).unwrap().insert(raw, &m).unwrap();
    let t = std::time::Instant::now();
    let block = loop {
        let out = h.run(vec![]);
        let area = lanes_area(&out);
        if let Some(r) = fills(&out, egui::Color32::from_rgb(46, 120, 70)).into_iter().find(|r| area.intersects(*r)) {
            break r;
        }
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "the channel recording is drawn");
        std::thread::sleep(std::time::Duration::from_millis(30));
    };
    if let Some(dir) = std::env::var_os("HS_REC_PNG_DIR") {
        let out = h.run(vec![]);
        h.png(&out, &Path::new(&dir).join("rec-channel-before-click.png"));
    }
    let rec = h.app.rec.rows.iter().find(|r| r.id == id).cloned().expect("row loaded");
    // Default: the P25 pipe, through the decoder.
    assert_eq!(h.app.rec_demod(&rec), "modems/commander-demod-p25.rxpipe");
    let (dec, pm) = h.app.rec_play_plan(&rec, &h.app.rec_demod(&rec));
    assert!(dec.as_ref().is_some_and(|d| d.argv[0].ends_with("commander-demod-p25.rxpipe")), "{dec:?} {}", h.app.rec.note);
    assert!(pm.is_digital(), "played as digital, not the raw discriminator");
    let (dec, pm) = h.app.rec_play_plan(&rec, DEMOD_FM);
    assert!(dec.is_none() && !pm.is_digital(), "FM audio is the discriminator");
    assert!(h.app.rec_demod_options().iter().any(|(k, _)| k == DEMOD_FM));

    // A click on the block plays it and opens the dialog. The view follows
    // NOW, so the block is found again in the frame just before the click.
    h.app.rec.detail_open = false;
    let m0 = egui::Modifiers::NONE;
    let out = h.run(vec![]);
    let area = lanes_area(&out);
    let block = fills(&out, egui::Color32::from_rgb(46, 120, 70)).into_iter().find(|r| area.intersects(*r)).expect("block");
    // Right part: the left of a block can sit under the lane-name shelf.
    let at = egui::pos2(block.right() - block.width() * 0.2, block.center().y);
    h.run(vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: m0 },
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: m0 },
    ]);
    let out = h.settle(3);
    assert!(h.app.rec.player.as_ref().is_some_and(|(pid, _)| *pid == id), "clicked recording plays: selected {:?} open {} note {:?} block {block:?}", h.app.rec.selected, h.app.rec.detail_open, h.app.rec.note);
    assert!(h.app.rec.detail_open);
    let words: Vec<String> = texts(&out).into_iter().map(|(t, _)| t).collect();
    assert!(words.iter().any(|t| t.to_uppercase().contains("RECORD / PLAYBACK")), "dialog shown: {words:?}");
    assert!(words.iter().any(|t| t == "Demodulator"), "demodulator selector shown");
    assert!(words.iter().any(|t| t.starts_with("P25")), "P25 is the default: {words:?}");
    let t = std::time::Instant::now();
    while !h.app.rec.player.as_ref().is_some_and(|(_, p)| p.decoder_ran.load(Ordering::Relaxed)) {
        assert!(t.elapsed() < std::time::Duration::from_secs(3), "playback runs the P25 decoder");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // FM audio picked: restarts without a decoder.
    h.app.rec_pick_demod(id, DEMOD_FM.into());
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(h.app.rec.player.as_ref().is_some_and(|(_, p)| !p.decoder_ran.load(Ordering::Relaxed)), "FM audio has no decoder");
    // Closing the dialog stops playback.
    h.app.rec_close_detail();
    assert!(h.app.rec.player.is_none() && !h.app.rec.detail_open);
    // Playback started any other way (play-next, keys) also shows the dialog.
    h.app.rec.demod.remove(&id);
    h.app.rec.selected = None;
    h.app.rec_play(id, 0);
    h.settle(2);
    assert!(h.app.rec.detail_open && h.app.rec.selected == Some(id));
    h.app.rec_close_detail();
}

fn tg_rec(db: &Db, dir: &Path, name: &str, tg: u32, tg_name: &str, ago_s: i64, dur_ms: i64) -> Meta {
    let raw = format!("hackrf_tg/x/{name}.cs16");
    let path = dir.join(&raw);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let iq = hsrec::dsp::synth_c4fm_cs16(0.0, dur_ms as f64 / 1000.0, 9_000.0);
    std::fs::write(&path, iq.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    let t0 = chrono::Utc::now() - chrono::Duration::seconds(ago_s);
    let mut m = Meta {
        source: meta::SOURCE_HACKRF_TG.into(),
        source_kind: meta::KIND_RAW_IQ.into(),
        talkgroup: Some(tg),
        talkgroup_name: tg_name.into(),
        raw_format: "cs16".into(),
        sample_rate: 32_000,
        start_utc: t0.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        end_utc: (t0 + chrono::Duration::milliseconds(dur_ms)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        duration_ms: dur_ms,
        bytes: (iq.len() * 2) as i64,
        modulation: "p25".into(),
        centre_hz: 857_200_000.0,
        channel_hz: 856_962_500.0,
        offset_hz: -237_500.0,
        ..Default::default()
    };
    m.decoder = serde_json::json!({ "script": "modems/commander-demod-p25.rxpipe", "vocoder": "p25" });
    let _ = db;
    m
}

/// The Radio IDs tab: talkgroup recordings cut into one block per
/// transmission and laned by radio (alias when known), calls with no
/// radio in "Unknown radio", search by radio, alias or talkgroup, and a
/// click plays only that radio's span of the talkgroup recording.
#[test]
fn real_page_radio_ids_tab_lanes_searches_and_plays_one_radio() {
    if !in_child("real_page_radio_ids_tab_lanes_searches_and_plays_one_radio") {
        return;
    }
    install_modems();
    let st = station(None, 0);
    let mut h = Harness::new(&st, egui::vec2(1200.0, 760.0), false);
    h.app.rec.face.source_tab = meta::SOURCE_RADIO_IDS.into();
    h.app.rec.play_argv = Some(vec!["sh".into(), "-c".into(), "cat > /dev/null".into()]);
    let mut engine = crate::units::Unit::named(100);
    engine.description = "Engine 7".into();
    h.app.units = vec![engine, crate::units::Unit::named(200)];
    h.settle(4);
    let db = Db::open(&h.app.rec.db_path).unwrap();
    let dir = h.app.rec.dir.clone();
    let put = |name: &str, m: &Meta| db.insert(&format!("hackrf_tg/x/{name}.cs16"), m).unwrap();
    // A: three transmissions, 100 → 200 → 100 (new recordings carry talkers).
    let mut a = tg_rec(&db, &dir, "a", 10119, "49-IUH-DISP — IUH Public Safety", 14, 10_000);
    a.talkers = vec![
        meta::Talk { id: 100, start_ms: 0, end_ms: 4_000 },
        meta::Talk { id: 200, start_ms: 4_000, end_ms: 7_000 },
        meta::Talk { id: 100, start_ms: 7_000, end_ms: 10_000 },
    ];
    a.radio_unit_id = Some(100);
    let id_a = put("a", &a);
    // B: an older recording with only its first radio.
    let mut b = tg_rec(&db, &dir, "b", 10334, "30-HCLE — Law: Dispatch (Countywide)", 12, 3_000);
    b.radio_unit_id = Some(300);
    put("b", &b);
    // C: no radio known.
    let c = tg_rec(&db, &dir, "c", 10200, "Fire Main", 9, 2_000);
    put("c", &c);
    let lanes = |out: &egui::FullOutput| -> Vec<String> {
        let area = lanes_area(out);
        texts(out).into_iter().filter(|(_, r)| area.contains(r.center())).map(|(t, _)| t).collect()
    };
    let t = std::time::Instant::now();
    let out = loop {
        let out = h.run(vec![]);
        if h.app.rec.rows.len() == 5 {
            break h.settle(2);
        }
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "rows: {}", h.app.rec.rows.len());
        std::thread::sleep(std::time::Duration::from_millis(30));
        drop(out);
    };
    let names = lanes(&out);
    for want in ["Engine 7 (100)", "Radio 200", "Radio 300", "Unknown radio"] {
        assert!(names.iter().any(|n| n.starts_with(want)), "lane {want} in {names:?}");
    }
    assert!(texts(&out).iter().any(|(t, _)| t.contains("5 recordings")), "five transmissions");
    // Search: a radio, an alias, a talkgroup name.
    for (q, yes, no) in [("200", "Radio 200", "Engine 7"), ("engine", "Engine 7", "Radio 200"), ("HCLE", "Radio 300", "Engine 7")] {
        h.app.rec.query = q.into();
        let out = h.settle(2);
        let names = lanes(&out);
        assert!(names.iter().any(|n| n.starts_with(yes)), "{q}: {yes} in {names:?}");
        assert!(!names.iter().any(|n| n.starts_with(no)), "{q}: no {no} in {names:?}");
    }
    h.app.rec.query.clear();
    h.settle(2);
    // Click radio 200's block: plays 4.0–7.0 s of recording A.
    let out = h.run(vec![]);
    let area = lanes_area(&out);
    let lane = lane_label(&out, "Radio 200").expect("radio 200 lane");
    let block = fills(&out, tg_colour(10119))
        .into_iter()
        .filter(|r| area.intersects(*r))
        .find(|r| (r.center().y - lane.center().y).abs() < LANE_H)
        .expect("radio 200's block, in its talkgroup's colour");
    let at = egui::pos2(block.right() - block.width() * 0.2, block.center().y);
    let m0 = egui::Modifiers::NONE;
    h.run(vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: m0 },
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: m0 },
    ]);
    let out = h.settle(3);
    let (pid, _) = h.app.rec.player.as_ref().map(|(i, p)| (*i, p.pos_ms.load(Ordering::Relaxed))).expect("plays");
    assert_eq!(h.app.rec.radio_map.get(&pid).map(|(r, s, e)| (*r, *s, *e)), Some((id_a, 4_000, 7_000)), "radio 200's span of A");
    assert_eq!(h.app.rec.play_base, 4_000);
    assert_eq!(h.app.rec.play_until, Some(7_000));
    let words: Vec<String> = texts(&out).into_iter().map(|(t, _)| t).collect();
    assert!(words.iter().any(|t| t.to_uppercase().contains("RECORD / PLAYBACK")), "dialog shown");
    assert!(words.iter().any(|t| t == "Demodulator"));
    assert!(!words.iter().any(|t| t == "Delete"), "a radio block never deletes its talkgroup recording");
    // It stops at the end of the transmission.
    let t = std::time::Instant::now();
    while h.app.rec.player.is_some() {
        h.run(vec![]);
        assert!(t.elapsed() < std::time::Duration::from_secs(8), "stops at 7.0 s");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if let Some(dir) = std::env::var_os("HS_REC_PNG_DIR") {
        let out = h.settle(2);
        h.png(&out, &Path::new(&dir).join("rec-radio-ids-synthetic.png"));
    }
}

/// Radio IDs dialog: Notes is the radio's description. Typing one saves it
/// to the station file's `units`, and the lane, heading and TG tab use it
/// at once; clearing it goes back to "Radio <id>". A failed save is shown.
#[test]
fn real_page_radio_ids_notes_edit_the_radio_description() {
    if !in_child("real_page_radio_ids_notes_edit_the_radio_description") {
        return;
    }
    install_modems();
    let st = station(None, 0);
    let mut h = Harness::new(&st, egui::vec2(1200.0, 900.0), false);
    h.app.rec.face.source_tab = meta::SOURCE_RADIO_IDS.into();
    h.app.rec.play_argv = Some(vec!["sh".into(), "-c".into(), "cat > /dev/null".into()]);
    h.app.units = vec![crate::units::Unit::named(30003)];
    h.settle(4);
    let db = Db::open(&h.app.rec.db_path).unwrap();
    let dir = h.app.rec.dir.clone();
    let mut a = tg_rec(&db, &dir, "a", 10741, "ISP IND DISP", 12, 4_000);
    a.radio_unit_id = Some(30003);
    let id_a = db.insert("hackrf_tg/x/a.cs16", &a).unwrap();
    let t = std::time::Instant::now();
    while h.app.rec.rows.len() != 1 {
        h.run(vec![]);
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "rows {}", h.app.rec.rows.len());
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let vid = h.app.rec.rows[0].id;
    assert!(vid < 0);
    h.app.rec_play(vid, 0);
    let out = h.settle(4);
    assert!(lane_label(&out, "Radio 30003").is_some(), "lane before");
    let words: Vec<(String, egui::Rect)> = texts(&out);
    let at = |w: &str| words.iter().find(|(t, _)| t.to_uppercase() == w).map(|(_, r)| r.top());
    let (play, notes) = (at("PLAYBACK").expect("PLAYBACK"), at("NOTES").expect("NOTES section"));
    assert!(notes > play, "Notes below Playback");
    assert!(words.iter().any(|(t, _)| t.contains("Notes below name the radio")), "hint");
    // Type into the Description field (its hint is the fallback name), then Enter.
    let field = words.iter().find(|(t, _)| t == "Description").map(|(_, r)| *r).expect("Description");
    let click = egui::pos2(field.right() + 60.0, field.center().y);
    let m0 = egui::Modifiers::NONE;
    h.run(vec![
        egui::Event::PointerMoved(click),
        egui::Event::PointerButton { pos: click, button: egui::PointerButton::Primary, pressed: true, modifiers: m0 },
        egui::Event::PointerButton { pos: click, button: egui::PointerButton::Primary, pressed: false, modifiers: m0 },
    ]);
    h.run(vec![egui::Event::Text("Engine 7".into())]);
    h.run(vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: m0 }]);
    let out = h.settle(4);
    let saved = crate::config::load_file(&st).expect("station saved");
    assert_eq!(saved.units.iter().find(|u| u.id == 30003).map(|u| u.description.as_str()), Some("Engine 7"), "in the station file");
    assert!(lane_label(&out, "Engine 7 (30003)").is_some(), "lane renamed at once: {:?}", texts(&out).iter().map(|t| &t.0).collect::<Vec<_>>());
    assert!(texts(&out).iter().any(|(t, _)| t == "Engine 7 (30003)" ), "dialog heading");
    assert!(texts(&out).iter().any(|(t, _)| t.contains("Saved: Engine 7 (30003)")), "status line");
    // The talkgroup's own recording names the radio too.
    let real = db.get(id_a).unwrap().unwrap();
    let rows = h.app.rec_meta_rows(&real);
    assert!(rows.iter().any(|(k, v)| *k == "Radio unit ID" && v == "Engine 7 (30003)"), "{rows:?}");
    if let Some(png) = std::env::var_os("HS_REC_PNG_DIR") {
        h.png(&out, &Path::new(&png).join("rec-radio-notes-synthetic.png"));
    }
    // Cleared: back to "Radio 30003".
    h.app.rec.desc_commit = Some((30003, String::new()));
    h.app.rec.desc_edit = None;
    let out = h.settle(4);
    assert!(lane_label(&out, "Radio 30003").is_some(), "fallback name");
    let saved = crate::config::load_file(&st).unwrap();
    assert_eq!(saved.units.iter().find(|u| u.id == 30003).map(|u| u.description.as_str()), Some(""));
    // A save that fails is said in the status line, not dropped.
    let bad = st.parent().unwrap().join("not-a-file");
    std::fs::create_dir_all(&bad).unwrap();
    h.app.config_path = bad.display().to_string();
    h.app.rec.configured = Some(h.app.config_path.clone());
    h.app.rec.desc_commit = Some((30003, "Car 9".into()));
    let out = h.settle(3);
    assert!(texts(&out).iter().any(|(t, _)| t.contains("description NOT saved")), "error shown: {}", h.app.rec.note);
}

/// The Record / Playback dialog keeps Call, Playback and Notes on top, then
/// the metadata (Source, Processing, File), then Export and Danger below File.
#[test]
fn real_dialog_puts_export_and_danger_below_the_file_section() {
    if !in_child("real_dialog_puts_export_and_danger_below_the_file_section") {
        return;
    }
    install_modems();
    let st = station(None, 0);
    let mut h = Harness::new(&st, egui::vec2(1400.0, 1500.0), false);
    h.app.rec.play_argv = Some(vec!["sh".into(), "-c".into(), "cat > /dev/null".into()]);
    h.settle(4);
    let db = Db::open(&h.app.rec.db_path).unwrap();
    let dir = h.app.rec.dir.clone();
    let mut m = tg_rec(&db, &dir, "d", 10741, "ISP IND DISP — District 52 - Indianapolis Dispatch", 10, 4_000);
    m.radio_unit_id = Some(9522067);
    let id = db.insert("hackrf_tg/x/d.cs16", &m).unwrap();
    let t = std::time::Instant::now();
    while !h.app.rec.rows.iter().any(|r| r.id == id) {
        h.run(vec![]);
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    h.app.rec_play(id, 0);
    let out = h.settle(4);
    // Drag the dialog's resize corner down so every section is in view.
    assert!(texts(&out).iter().any(|(t, _)| t.to_uppercase() == "RECORD / PLAYBACK"), "dialog shown");
    let corner = egui::pos2(700.0 + 260.0 - 2.0, 750.0 + 320.0 - 2.0);
    let to = egui::pos2(1250.0, 1440.0);
    let m0 = egui::Modifiers::NONE;
    h.run(vec![egui::Event::PointerMoved(corner)]);
    h.run(vec![egui::Event::PointerButton { pos: corner, button: egui::PointerButton::Primary, pressed: true, modifiers: m0 }]);
    for k in 1..=10 {
        let p = corner + (to - corner) * (k as f32 / 10.0);
        h.run(vec![egui::Event::PointerMoved(p)]);
    }
    h.run(vec![egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: m0 }]);
    let out = h.settle(4);
    let y = |name: &str| -> f32 {
        texts(&out).into_iter().find(|(t, _)| t == name).map(|(_, r)| r.center().y).unwrap_or_else(|| panic!("{name} not drawn"))
    };
    let order = ["CALL", "PLAYBACK", "NOTES", "SOURCE", "FILE", "EXPORT", "DANGER"];
    for w in order.windows(2) {
        assert!(y(w[0]) < y(w[1]), "{} above {}", w[0], w[1]);
    }
    if let Some(dir) = std::env::var_os("HS_REC_PNG_DIR") {
        h.png(&out, &Path::new(&dir).join("rec-playback-dialog.png"));
    }
    h.app.rec_close_detail();
}

/// Screenshots of the REC page from a copy of a real database, before and
/// after a Shift+wheel scroll. Runs only when HS_REC_DB names the database
/// and HS_REC_PNG_DIR the output folder.
#[test]
fn real_page_screenshots_from_a_database_copy() {
    let (Some(db), Some(dir)) = (std::env::var_os("HS_REC_DB"), std::env::var_os("HS_REC_PNG_DIR")) else { return };
    if std::env::var_os("HS_REC_PAGE_CHILD").is_none() {
        // Pass the two variables through to the child.
        in_child("real_page_screenshots_from_a_database_copy");
        return;
    }
    let copy = std::env::temp_dir().join(format!("hs-rec-copy-{}.sqlite", std::process::id()));
    std::fs::copy(PathBuf::from(&db), &copy).unwrap();
    let st = station(Some(&copy), 0);
    let mut h = Harness::new(&st, egui::vec2(1280.0, 480.0), false);
    let out = h.settle(10);
    let dir = PathBuf::from(dir);
    h.png(&out, &dir.join("rec-page-before.png"));
    let area = lanes_area(&out);
    let over = egui::pos2(area.left() + 600.0, area.top() + 60.0);
    for _ in 0..4 {
        h.wheel(over, egui::MouseWheelUnit::Line, egui::vec2(0.0, -3.0), egui::Modifiers::SHIFT);
    }
    let out = h.settle(3);
    h.png(&out, &dir.join("rec-page-after-shift-wheel.png"));
    eprintln!("lanes offset {} view {:?}", h.lane_off(), h.view());
    // The Radio IDs tab, with the radio names from a station file copy
    // (HS_REC_STATION, read only) when given.
    if let Some(stfile) = std::env::var_os("HS_REC_STATION") {
        let text = std::fs::read_to_string(PathBuf::from(stfile)).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        if let Ok(units) = serde_json::from_value::<Vec<crate::units::Unit>>(v["units"].clone()) {
            h.app.units = units;
        }
    }
    h.app.rec.face.source_tab = meta::SOURCE_RADIO_IDS.into();
    h.app.rec.rows_at = None;
    let out = h.settle(10);
    h.png(&out, &dir.join("rec-radio-ids.png"));
    eprintln!("radio ids rows {}", h.app.rec.rows.len());
    // One radio's playback dialog with its Notes (description) section,
    // opened without playing; nothing is typed, so nothing is saved.
    let pick = h.app.rec.rows.iter().filter(|r| r.meta.radio_unit_id.is_some_and(|u| u != 0)).max_by_key(|r| r.meta.start_utc.clone()).map(|r| r.id);
    if let Some(id) = pick {
        h.size = egui::vec2(1280.0, 900.0);
        h.app.rec.selected = Some(id);
        h.app.rec.detail_open = true;
        let out = h.settle(6);
        h.png(&out, &dir.join("rec-radio-notes.png"));
    }
}
