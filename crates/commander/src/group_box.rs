//! Thin LCARS group box. The elbow, its layout and the detach pill live in
//! rustcars (`rustcars::group`); Commander re-exports them here.
//!
//! Lock safety: no `ctx.data*` / `ctx.input` closures are used here.

#[allow(unused_imports)]
pub use rustcars::group::{
    DetachState, GroupBox, Layout, Metrics, chrome_shapes, detached_stub_text, detached_window,
    detach_pill, group_box, group_box_slot, group_box_with, layout, layout_with_slot,
};

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
