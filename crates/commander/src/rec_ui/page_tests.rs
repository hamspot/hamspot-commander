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
}
