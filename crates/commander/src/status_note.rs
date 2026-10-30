//! Page status notes. Hints, notes, warnings and technical readouts that a
//! page screen used to print as loose labels go to its single status line
//! instead (#221, David: "that stuff belongs in the status area"). A screen
//! calls [`note`] while it draws; the page frame reserves one fixed line
//! above it, then paints what was noted this pass. No layout shift.
//!
//! Lock safety: ids are computed before `data_mut`; no ctx calls inside.

fn id(ctx: &egui::Context) -> egui::Id {
    egui::Id::NULL.with(("status-notes", ctx.viewport_id()))
}

/// Add one note for this pass. Duplicates and empties are dropped.
pub fn note(ui: &egui::Ui, text: impl Into<String>) {
    let text = text.into();
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let text = text.to_owned();
    let id = id(ui.ctx());
    ui.ctx().data_mut(|d| {
        let v = d.get_temp_mut_or_default::<Vec<String>>(id);
        if !v.contains(&text) {
            v.push(text);
        }
    });
}

/// Start a pass: forget earlier notes.
pub fn clear(ctx: &egui::Context) {
    let id = id(ctx);
    ctx.data_mut(|d| d.remove::<Vec<String>>(id));
}

/// The notes of this pass, in order.
pub fn take(ctx: &egui::Context) -> Vec<String> {
    let id = id(ctx);
    ctx.data_mut(|d| d.remove_temp::<Vec<String>>(id)).unwrap_or_default()
}

/// Reserve the fixed status line now (one row, full width).
pub fn reserve(ui: &mut egui::Ui) -> egui::Rect {
    let h = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover());
    rect
}

/// Paint `base` plus this pass's notes into the reserved line: one row,
/// truncated with "…", the full text on hover. Plain text.
pub fn paint(ui: &mut egui::Ui, rect: egui::Rect, base: &str) -> String {
    let mut line = base.to_owned();
    for n in take(ui.ctx()) {
        if !line.is_empty() {
            line.push_str("   ");
        }
        line.push_str(&n);
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).id_salt("status-line"), |ui| {
        ui.add(egui::Label::new(line.clone()).truncate());
    });
    line
}

#[cfg(test)]
mod tests {
    #[test]
    fn notes_collect_once_and_clear() {
        let ctx = egui::Context::default();
        let mut got = vec![];
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            super::clear(ui.ctx());
            super::note(ui, "A");
            super::note(ui, "A");
            super::note(ui, "  ");
            super::note(ui, "B");
            got = super::take(ui.ctx());
            assert!(super::take(ui.ctx()).is_empty());
        });
        out.textures_delta.clear();
        assert_eq!(got, vec!["A".to_owned(), "B".to_owned()]);
    }
}

/// Height of the status strip under a tool screen (as NanoVNA's).
pub const STRIP_H: f32 = 24.0;

/// NanoVNA-style status strip at `rect` (fixed place, one row): a dark
/// strip with `base` and this frame's notes as plain text, truncated.
/// Returns the full line (also the hover text).
pub fn paint_strip(ui: &mut egui::Ui, rect: egui::Rect, base: &str) -> String {
    let mut line = base.to_owned();
    for n in take(ui.ctx()) {
        if !line.is_empty() {
            line.push_str("   ");
        }
        line.push_str(&n);
    }
    ui.painter().rect_filled(rect, 0.0, rustcars::palette::STRIP);
    let inner = rect.shrink2(egui::vec2(8.0, 0.0));
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(inner).id_salt("status-strip").layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let text = egui::RichText::new(line.clone()).size(14.0).color(rustcars::LcarsTheme::default().text);
            ui.add(egui::Label::new(text).truncate()).on_hover_text(line.clone());
        },
    );
    line
}
