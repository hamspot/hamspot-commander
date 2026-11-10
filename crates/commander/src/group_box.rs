//! Thin LCARS group box. The elbow, its layout and the detach pill live in
//! rustcars (`rustcars::group`); Commander re-exports them here.
//!
//! Lock safety: no `ctx.data*` / `ctx.input` closures are used here.

#[allow(unused_imports)]
pub use rustcars::group::{
    DetachState, GroupBox, Layout, Metrics, chrome_shapes, detached_stub_text, detached_window, detached_window_with, DetachOpts, detach_failed_text,
    detach_pill, group_box, group_box_slot, group_box_with, layout, layout_with_slot,
};

/// End pill of a dialog elbow. Same shape as the detach pill. A click closes.
pub fn close_pill(ui: &mut egui::Ui, rect: egui::Rect, id: egui::Id, color: egui::Color32) -> egui::Response {
    use egui::{CornerRadius, Sense, Stroke, StrokeKind};
    use rustcars::group::{pill_colors, PILL_BORDER};
    let resp = ui.interact(rect, id, Sense::click());
    let r = (rect.height() / 2.0).clamp(0.0, 255.0) as u8;
    let cr = CornerRadius { nw: 0, sw: 0, ne: r, se: r };
    let (fill, outline) = pill_colors(color, false);
    let width = if resp.hovered() { PILL_BORDER + 0.5 } else { PILL_BORDER };
    let p = ui.painter();
    p.rect_filled(rect, cr, fill);
    p.rect_stroke(rect, cr, Stroke::new(width, outline), StrokeKind::Inside);
    if resp.has_focus() {
        p.rect_stroke(rect.expand(1.0), cr, Stroke::new(1.0, rustcars::ink_on(fill)), StrokeKind::Outside);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Close"));
    resp.on_hover_text("Close")
}

/// Elbow that fills the available rect. The end pill closes. Dragging the
/// bar returns a delta the caller applies to the window position.
pub fn elbow_dialog<R>(
    ui: &mut egui::Ui,
    title: &str,
    color: egui::Color32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> (R, bool, egui::Vec2) {
    use egui::{Align2, FontId, Sense, Shape, UiBuilder, pos2};
    use rustcars::group::{chrome_shapes, detach_pill_width, layout_with_slot};
    use rustcars::ink_on;
    let mut m = Metrics::default();
    m.cap_w = detach_pill_width(ui, &m);
    m.cap_min_width = 0.0;
    let bg = ui.visuals().panel_fill;
    let slot = ui.painter().add(Shape::Noop);
    let avail = ui.available_rect_before_wrap();
    let (left, top, width) = (avail.left(), avail.top(), ui.available_width());
    let probe = layout_with_slot(left, top, width, 0.0, &m, 0.0);
    let bottom_inset = (probe.outer.bottom() - probe.inner.bottom()).max(0.0);
    let fill_h = if avail.bottom().is_finite() {
        (avail.bottom() - bottom_inset - probe.inner.top()).max(0.0)
    } else {
        0.0
    };
    let max = egui::Rect::from_min_max(probe.inner.min, pos2(probe.inner.right(), probe.inner.top() + fill_h));
    let mut child = ui.new_child(UiBuilder::new().max_rect(max).layout(*ui.layout()));
    child.set_clip_rect(max.intersect(ui.clip_rect()));
    let value = add(&mut child);
    let l = layout_with_slot(left, top, width, fill_h, &m, 0.0);
    let _ = ui.allocate_rect(l.outer, Sense::hover());
    let mut chrome = l;
    chrome.cap = None;
    // The elbow is only the bars. Fill the plate or the page shows through.
    let mut plate = vec![Shape::rect_filled(l.outer, 0.0, bg)];
    plate.extend(chrome_shapes(&chrome, &m, color, bg));
    ui.painter().set(slot, Shape::Vec(plate));
    if l.title.width() > 0.0 {
        ui.painter().with_clip_rect(l.title).text(
            pos2(l.title.left(), l.title.center().y),
            Align2::LEFT_CENTER,
            title.to_uppercase(),
            FontId::proportional(m.title_size),
            ink_on(color),
        );
    }
    let closed = l.cap.is_some_and(|cap| {
        close_pill(ui, cap, ui.scope_id().with(("elbow-close", title)), color).clicked()
    });
    let drag = if let Some(bar) = Some(l.bar).filter(|b| b.width() > 0.0) {
        let resp = ui.interact(bar, ui.scope_id().with(("elbow-drag", title)), Sense::drag());
        resp.on_hover_cursor(egui::CursorIcon::Grab)
            .on_hover_text("Drag to move")
            .drag_delta()
    } else {
        egui::Vec2::ZERO
    };
    (value, closed, drag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2, vec2};
    use rustcars::ink_on;

    #[test]
    fn layout_rects_line_up() {
        let m = Metrics::default();
        let l = layout(10.0, 20.0, 400.0, 100.0, &m);
        assert_eq!(l.outer.min, pos2(10.0, 20.0));
        assert_eq!(l.outer.right(), 410.0);
        assert_eq!(l.inner.min, pos2(10.0 + m.rail + m.gap, 20.0 + m.bar + m.gap));
        assert_eq!(l.inner.height(), 100.0);
        assert_eq!(l.inner.right(), 410.0);
        assert_eq!(l.outer.bottom(), l.inner.bottom() + m.bottom);
        assert_eq!(l.bar.height(), m.bar);
        assert_eq!(l.rail.width(), m.rail);
        assert_eq!(l.rail.bottom(), l.outer.bottom());
        let cap = l.cap.expect("wide group has a cap");
        assert_eq!(cap.right(), 410.0);
        assert_eq!(l.bar.right(), cap.left() - m.cap_gap);
        assert!(l.title.left() > l.rail.right() + m.r_in);
        assert!(l.title.right() <= l.bar.right());
        assert!(l.bar.top() == l.title.top() && l.bar.bottom() == l.title.bottom());
    }

    #[test]
    fn narrow_group_drops_cap_and_stays_sane() {
        let m = Metrics::default();
        let l = layout(0.0, 0.0, 100.0, 0.0, &m);
        assert!(l.cap.is_none());
        assert_eq!(l.bar.right(), 100.0);
        let tiny = layout(0.0, 0.0, 2.0, -5.0, &m);
        assert!(tiny.inner.width() >= 0.0 && tiny.inner.height() == 0.0);
        assert!(tiny.title.width() >= 0.0);
    }

    #[test]
    fn chrome_has_elbow_and_cap() {
        let m = Metrics::default();
        let l = layout(0.0, 0.0, 400.0, 50.0, &m);
        let s = chrome_shapes(&l, &m, rustcars::palette::TAN, rustcars::palette::BG);
        // block + bar run + rail run + carve + cap
        assert_eq!(s.len(), 5);
    }

    #[test]
    fn group_colours_have_readable_titles() {
        for (_, c) in crate::CONFIG_GROUPS {
            assert!(rustcars::contrast(c, ink_on(c)) >= 4.5, "{c:?}");
        }
    }

    /// Headless smoke test: Config-style groups (nested too) for a few frames
    /// on a worker thread; fails after 10 s (egui lock re-entry deadlock).
    #[test]
    fn group_boxes_run_headless_without_deadlock() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ctx = egui::Context::default();
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0))),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (name, c) in crate::CONFIG_GROUPS {
                            group_box(ui, name, c, |ui| {
                                ui.label("setting");
                                ui.horizontal(|ui| {
                                    let _ = ui.button("Add");
                                });
                                group_box(ui, "nested", c, |ui| ui.label("x"));
                            });
                        }
                    });
                });
                out.textures_delta.clear();
            }
            let _ = tx.send(());
        });
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("headless group_box did not finish in 10 s (deadlock?)");
    }

    #[test]
    fn slot_sits_at_right_end_before_cap() {
        let m = Metrics::default();
        assert_eq!((m.bar, m.r_out), (16.0, 12.0));
        let l = layout_with_slot(0.0, 0.0, 800.0, 40.0, &m, 120.0);
        let slot = l.slot.expect("wide group has a slot");
        let cap = l.cap.expect("cap");
        assert_eq!(slot.right(), cap.left() - m.cap_gap);
        assert_eq!(slot.width(), 120.0);
        assert_eq!(slot.height(), m.bar);
        assert_eq!(l.bar.right(), slot.left() - m.cap_gap);
        assert!(l.title.right() <= slot.left());
        // No slot asked: same as before.
        assert_eq!(layout_with_slot(0.0, 0.0, 800.0, 40.0, &m, 0.0), layout(0.0, 0.0, 800.0, 40.0, &m));
        // Too narrow for title room: no slot, bar unchanged.
        let n = layout_with_slot(0.0, 0.0, 150.0, 0.0, &m, 120.0);
        assert!(n.slot.is_none());
        assert_eq!(n.bar.right(), 150.0);
    }

    #[test]
    fn group_bars_never_draw_the_estop() {
        // 16 px group bars are too short for the 40 px E-STOP (#200).
        let src = include_str!("group_box.rs");
        let call = ["crate::estop::", "button("].concat();
        assert!(!src.contains(&call), "group_box must not draw the E-STOP");
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 400.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                group_box(ui, "Rotator", rustcars::palette::OLIVE, |ui| ui.label("x"));
            });
            out.textures_delta.clear();
            for c in &out.shapes {
                if let egui::Shape::Text(t) = &c.shape {
                    assert!(!t.galley.text().contains("E-STOP"));
                }
            }
        }
    }

    #[test]
    fn close_pill_click_closes() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(pos2(20.0, 20.0), vec2(46.0, 16.0));
        let center = rect.center();
        let button = |pressed| egui::Event::PointerButton {
            pos: center,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let frames = [
            vec![egui::Event::PointerMoved(center)],
            vec![button(true)],
            vec![button(false)],
            vec![],
        ];
        let mut clicked = false;
        for events in frames {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 80.0))),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                if close_pill(ui, rect, egui::Id::unique("close-test"), rustcars::palette::TAN).clicked() {
                    clicked = true;
                }
            });
            out.textures_delta.clear();
        }
        assert!(clicked, "a click on the end pill closes");
    }

    #[test]
    fn elbow_dialog_paints_the_title_and_fills() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(520.0, 360.0))),
            ..Default::default()
        };
        let mut saw = false;
        let mut out = ctx.run_ui(input, |ui| {
            let bounds = Rect::from_min_size(pos2(0.0, 0.0), vec2(480.0, 320.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bounds));
            let (h, closed, drag) = elbow_dialog(&mut child, "Record / Playback", rustcars::palette::TAN, |ui| {
                ui.label("body");
                ui.min_rect().height()
            });
            assert!(!closed);
            assert_eq!(drag, egui::Vec2::ZERO);
            assert!(h >= 0.0);
        });
        for c in &out.shapes {
            if let egui::Shape::Text(t) = &c.shape {
                if t.galley.text().contains("RECORD / PLAYBACK") {
                    saw = true;
                }
            }
        }
        let mut filled = false;
        fn walk(shape: &egui::Shape, filled: &mut bool) {
            match shape {
                egui::Shape::Vec(v) => {
                    for s in v {
                        walk(s, filled);
                    }
                }
                egui::Shape::Rect(r) if r.fill.a() == 255 && r.rect.width() > 200.0 && r.rect.height() > 100.0 => {
                    *filled = true;
                }
                _ => {}
            }
        }
        for c in &out.shapes {
            walk(&c.shape, &mut filled);
        }
        out.textures_delta.clear();
        assert!(saw, "elbow title is painted");
        assert!(filled, "dialog plate is opaque");
    }

    #[test]
    fn groups_stack_without_overlap() {
        // Several groups in a row, with and without a bar slot, must lay out
        // one after another (the Config page overlap, #200).
        let ctx = egui::Context::default();
        let mut rects = Vec::new();
        for _ in 0..2 {
            rects.clear();
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 2000.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                for i in 0..6 {
                    let before = ui.cursor().top();
                    let m = Metrics::default();
                    let slot = if i % 2 == 0 { 120.0 } else { 0.0 };
                    group_box_slot(ui, &m, "G", rustcars::palette::OLIVE, slot, |ui| {
                        for _ in 0..4 { ui.label("row"); }
                    }, |ui| { ui.label("s"); });
                    let after = ui.min_rect().bottom();
                    rects.push((before, after));
                }
            });
            out.textures_delta.clear();
        }
        for w in rects.windows(2) {
            assert!(w[1].0 >= w[0].1 - 0.5, "group overlaps previous: {:?}", rects);
            assert!(w[0].1 - w[0].0 > 40.0, "group collapsed: {:?}", rects);
        }
    }
}
