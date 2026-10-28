//! Thin LCARS group box: a thin bar across the top that turns down the left
//! edge through a small rounded elbow, a short closing cap on the right, and
//! the group's name knocked out of the bar in `ink_on` colour.
//!
//! Purely visual. The chrome is painted *behind* the contents (paint-behind
//! with a reserved shape slot), so it is sized to the contents' real height.
//! Reusable on any dense page: `group_box(ui, "Radio", palette::SLATE, |ui| ...)`.
//!
//! Lock safety: no `ctx.data*` / `ctx.input` closures are used here.

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Shape, Ui, UiBuilder, pos2, vec2};
use rustcars::{Corner, Elbow, elbow_shapes, ink_on};

/// Sizes for the thin elbow. Much thinner than rustcars' `bar_thin` (40).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Height of the top bar (the title lives in it).
    pub bar: f32,
    /// Width of the left rail.
    pub rail: f32,
    /// Outer elbow radius.
    pub r_out: f32,
    /// Inner (concave) elbow radius.
    pub r_in: f32,
    /// Gap between chrome and contents.
    pub gap: f32,
    /// Space under the contents before the next widget.
    pub bottom: f32,
    /// Width of the closing cap at the right end of the bar.
    pub cap_w: f32,
    /// Gap between the bar and the cap.
    pub cap_gap: f32,
    /// Narrower than this, the cap is dropped.
    pub cap_min_width: f32,
    /// Title font size.
    pub title_size: f32,
    /// Space between the elbow's curve and the title.
    pub title_pad: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            bar: 16.0,
            rail: 8.0,
            r_out: 12.0,
            r_in: 6.0,
            gap: 6.0,
            bottom: 10.0,
            cap_w: 18.0,
            cap_gap: 4.0,
            cap_min_width: 160.0,
            title_size: 13.0,
            title_pad: 6.0,
        }
    }
}

/// Where every part goes, for a group at (`left`, `top`) of `width` whose
/// contents are `content_h` tall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// Whole group, chrome included.
    pub outer: Rect,
    /// Rect handed to rustcars' `elbow_shapes` (bar + rail + curve).
    pub elbow: Rect,
    /// Horizontal bar (up to the cap gap).
    pub bar: Rect,
    /// Left rail, full height.
    pub rail: Rect,
    /// Closing cap, if wide enough.
    pub cap: Option<Rect>,
    /// Space for the contents.
    pub inner: Rect,
    /// Where the title may be drawn (inside the bar, right of the curve).
    pub title: Rect,
    /// Gap at the right end of the bar for a control (the E-STOP), if asked.
    pub slot: Option<Rect>,
}

#[allow(dead_code)] // used by tests
pub fn layout(left: f32, top: f32, width: f32, content_h: f32, m: &Metrics) -> Layout {
    layout_with_slot(left, top, width, content_h, m, 0.0)
}

/// [`layout`] with a `slot_w` gap at the right end of the bar, before the cap.
/// The bar stops short of it. A group too narrow for title room keeps no slot.
pub fn layout_with_slot(
    left: f32,
    top: f32,
    width: f32,
    content_h: f32,
    m: &Metrics,
    slot_w: f32,
) -> Layout {
    let width = width.max(m.rail + m.gap + 1.0);
    let right = left + width;
    let inner = Rect::from_min_size(
        pos2(left + m.rail + m.gap, top + m.bar + m.gap),
        vec2((width - m.rail - m.gap).max(0.0), content_h.max(0.0)),
    );
    let bottom = inner.bottom() + m.bottom;
    let (bar_end, cap) = if width >= m.cap_min_width {
        let cap = Rect::from_min_max(pos2(right - m.cap_w, top), pos2(right, top + m.bar));
        (cap.left() - m.cap_gap, Some(cap))
    } else {
        (right, None)
    };
    let curve_w = (m.rail + m.r_in).max(2.0 * m.r_out);
    let (bar_end, slot) = if slot_w > 0.0 && bar_end - slot_w - m.cap_gap > left + curve_w + m.title_pad {
        let r = Rect::from_min_max(pos2(bar_end - slot_w, top), pos2(bar_end, top + m.bar));
        (r.left() - m.cap_gap, Some(r))
    } else {
        (bar_end, None)
    };
    let elbow = Rect::from_min_max(pos2(left, top), pos2(bar_end, bottom));
    let bar = Rect::from_min_max(pos2(left, top), pos2(bar_end, top + m.bar));
    let rail = Rect::from_min_max(pos2(left, top), pos2(left + m.rail, bottom));
    // rustcars sizes the swept corner block as max(rail + r_in, 2 * r_out).
    let title_left = (left + curve_w + m.title_pad).min(bar_end);
    let title = Rect::from_min_max(
        pos2(title_left, top),
        pos2((bar_end - m.title_pad).max(title_left), top + m.bar),
    );
    Layout {
        outer: Rect::from_min_max(pos2(left, top), pos2(right, bottom)),
        elbow,
        bar,
        rail,
        cap,
        inner,
        title,
        slot,
    }
}


/// Chrome shapes for `l` (no text). `bg` must be the colour behind the group:
/// it carves the concave curve.
pub fn chrome_shapes(l: &Layout, m: &Metrics, color: Color32, bg: Color32) -> Vec<Shape> {
    let e = Elbow {
        corner: Corner::TL,
        thick: m.rail,
        thin: m.bar,
        r_out: m.r_out,
        r_in: m.r_in,
        color,
        bg,
    };
    let mut v = elbow_shapes(l.elbow, &e);
    if let Some(cap) = l.cap {
        let r = (m.bar / 2.0).clamp(0.0, 255.0) as u8;
        let cr = CornerRadius { nw: 0, sw: 0, ne: r, se: r };
        v.push(Shape::rect_filled(cap, cr, color));
    }
    v
}

/// Wrap `add` in a thin titled elbow. Contents get the remaining width.
pub fn group_box<R>(ui: &mut Ui, title: &str, color: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    group_box_with(ui, &Metrics::default(), title, color, add)
}

pub fn group_box_with<R>(
    ui: &mut Ui,
    m: &Metrics,
    title: &str,
    color: Color32,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    group_box_slot(ui, m, title, color, 0.0, add, |_| {})
}

/// The general form: `slot_w` > 0 opens a gap at the right end of the bar
/// and `slot` draws into it (right-aligned, vertically centred).
pub fn group_box_slot<R>(
    ui: &mut Ui,
    m: &Metrics,
    title: &str,
    color: Color32,
    slot_w: f32,
    add: impl FnOnce(&mut Ui) -> R,
    slot_ui: impl FnOnce(&mut Ui),
) -> R {
    let bg = ui.visuals().panel_fill;
    let slot = ui.painter().add(Shape::Noop);
    let avail = ui.available_rect_before_wrap();
    let (left, top, width) = (avail.left(), avail.top(), ui.available_width());
    let probe = layout_with_slot(left, top, width, 0.0, m, slot_w);
    let max = Rect::from_min_max(probe.inner.min, pos2(probe.inner.right(), f32::INFINITY));
    let mut child = ui.new_child(UiBuilder::new().max_rect(max).layout(*ui.layout()));
    let r = add(&mut child);
    let used = child.min_rect();
    let l = layout_with_slot(left, top, width, (used.bottom() - probe.inner.top()).max(0.0), m, slot_w);
    let resp = ui.allocate_rect(l.outer, Sense::hover());
    let label = title.to_owned();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &label));
    ui.painter().set(slot, Shape::Vec(chrome_shapes(&l, m, color, bg)));
    if l.title.width() > 0.0 {
        ui.painter().with_clip_rect(l.title).text(
            pos2(l.title.left(), l.title.center().y),
            Align2::LEFT_CENTER,
            title.to_uppercase(),
            FontId::proportional(m.title_size),
            ink_on(color),
        );
    }
    if let Some(rect) = l.slot {
        // A child, not `scope_builder`: a scope allocates its rect in the
        // parent and moves the cursor back up to the bar, so the next group
        // would be drawn on top of this one (the #200 Config overlap).
        let mut c = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        slot_ui(&mut c);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

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
