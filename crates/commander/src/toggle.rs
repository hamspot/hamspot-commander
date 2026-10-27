//! Toggle buttons (rustcars rule P1b): a button that switches a function on
//! or off has a thin `TOGGLE_BORDER` outline in both states and a ● (on) /
//! ○ (off) marker before its label. Momentary buttons stay plain.

use eframe::egui;

/// `"● NR"` when on, `"○ NR"` when off. Case is kept.
pub fn toggle_text(on: bool, label: &str) -> String {
    format!("{} {label}", if on { "●" } else { "○" })
}

/// The base hue a Commander toggle uses (the theme's selection fill).
pub fn toggle_base(v: &egui::Visuals) -> egui::Color32 {
    v.selection.bg_fill
}

/// A bordered toggle button for `ui.add` / `ui.add_enabled`. Colours come from
/// `rustcars::toggle_colors`, so the off border keeps at least 3:1 contrast.
/// A disabled toggle keeps its border and marker (egui fades it).
pub fn toggle_btn(v: &egui::Visuals, on: bool, label: &str) -> egui::Button<'static> {
    let (fill, border, ink) = rustcars::toggle_colors(toggle_base(v), on);
    egui::Button::new(egui::RichText::new(toggle_text(on, label)).color(ink))
        .fill(fill)
        .stroke(egui::Stroke::new(rustcars::TOGGLE_BORDER, border))
}

/// Adds a toggle and gives it a checkbox-like accessibility role (name and
/// selected state), so screen readers hear "NR, checked".
pub fn add_toggle(ui: &mut egui::Ui, enabled: bool, on: bool, label: &str) -> egui::Response {
    let btn = toggle_btn(ui.visuals(), on, label);
    let resp = ui.add_enabled(enabled, btn);
    let name = label.to_string();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, on, &name));
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, click: Option<egui::Pos2>, on: &mut bool, rect: &mut egui::Rect) {
        let mut events = vec![];
        if let Some(p) = click {
            events.push(egui::Event::PointerMoved(p));
            events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
            events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        }
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 200.0))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let r = add_toggle(ui, true, *on, "NR");
            if r.clicked() {
                *on = !*on;
            }
            *rect = r.rect;
        });
        out.textures_delta.clear();
    }

    /// Headless: every converted toggle, inside a Config-style group, for a
    /// few frames on a worker thread; fails after 10 s (lock re-entry).
    #[test]
    fn converted_toggles_render_headless_without_deadlock() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ctx = egui::Context::default();
            let labels = [
                "ATU", "NR", "Pre 1", "Pre 2", "Bk-in", "Fast", "Proc", "Lock", "Notch", "MOX", "Moni",
                "BSP", "Receiver", "Follow", "Follow rig frequency", "Limit test on",
            ];
            for f in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 400.0))),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    for (name, c) in crate::CONFIG_GROUPS {
                        crate::group_box::group_box(ui, name, c, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for (i, l) in labels.iter().enumerate() {
                                    let _ = add_toggle(ui, i % 3 != 0, (i + f) % 2 == 0, l);
                                }
                            });
                        });
                    }
                    let theme = rustcars::LcarsTheme::default();
                    let mut on = f % 2 == 0;
                    let _ = rustcars::toggle_button(ui, &theme, "Live", &mut on, rustcars::palette::BRICK, egui::vec2(60.0, 20.0));
                });
                out.textures_delta.clear();
            }
            let _ = tx.send(());
        });
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("headless toggles did not finish in 10 s (deadlock?)");
    }

    #[test]
    fn marker_text() {
        assert_eq!(toggle_text(true, "NR"), "● NR");
        assert_eq!(toggle_text(false, "Pre 1"), "○ Pre 1");
    }

    #[test]
    fn border_differs_from_fill_in_both_states() {
        let v = egui::Visuals::dark();
        for on in [false, true] {
            let (fill, border, _) = rustcars::toggle_colors(toggle_base(&v), on);
            assert_ne!(fill, border);
        }
    }

    #[test]
    fn click_flips_state() {
        let ctx = egui::Context::default();
        let mut on = false;
        let mut rect = egui::Rect::NOTHING;
        frame(&ctx, None, &mut on, &mut rect);
        frame(&ctx, Some(rect.center()), &mut on, &mut rect);
        assert!(on);
        frame(&ctx, None, &mut on, &mut rect);
        frame(&ctx, Some(rect.center()), &mut on, &mut rect);
        assert!(!on);
    }
}
