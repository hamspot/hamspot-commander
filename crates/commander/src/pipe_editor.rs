//! Pipe editor window: lists pipe files per kind, shows their text, edits
//! user files, and wraps a refused custom command into a user pipe file.
//! Every write is an explicit, confirmed act; the station field only ever
//! receives a `modems/<name>` reference.
use crate::pipes;
use crate::rx_pipe::{self, ModemScript};

/// What a confirmed step will do.
#[derive(Clone)]
enum Confirm {
    Save { name: String, text: String, overwrite: bool },
    Delete { name: String },
    Rename { old: String, new: String, overwrite: bool },
}

/// A station field change the caller applies: `(radio id, tx, reference)`.
pub type Apply = (u64, bool, String);

#[derive(Default)]
pub struct PipeEditor {
    pub open: bool,
    /// Radio row the editor was opened from, and which field.
    target: Option<(u64, bool)>,
    /// Current value of that field (shown in the header).
    target_value: String,
    tx: bool,
    selected: Option<ModemScript>,
    text: String,
    original: String,
    name: String,
    confirm: Option<Confirm>,
    note: String,
    /// The target field uses the selected file.
    used: bool,
}

fn muted(ui: &egui::Ui, text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).small().color(ui.visuals().weak_text_color())
}

impl PipeEditor {
    /// Open for a radio's RX (`tx == false`) or TX field.
    pub fn open_for(&mut self, id: u64, tx: bool, value: &str) {
        *self = PipeEditor { open: true, target: Some((id, tx)), target_value: value.to_string(), tx, ..Default::default() };
        let refs = if tx { rx_pipe::tx_scripts() } else { rx_pipe::modem_scripts() };
        if let Some(s) = refs.into_iter().find(|s| s.command == value) {
            self.select(s);
        } else if !value.trim().is_empty() {
            self.start_wrap(value);
        }
    }

    fn ext(&self) -> &'static str {
        if self.tx { ".txpipe" } else { ".rxpipe" }
    }

    fn select(&mut self, s: ModemScript) {
        self.text = std::fs::read_to_string(&s.path).unwrap_or_else(|e| format!("# cannot read: {e}\n"));
        self.original = self.text.clone();
        self.name = s.name.clone();
        self.selected = Some(s);
        self.confirm = None;
        self.note.clear();
    }

    /// Draft a wrapper for a refused custom command. Nothing is written
    /// until the user confirms the shown text.
    fn start_wrap(&mut self, command: &str) {
        let name = pipes::slug_for(command, self.ext());
        let date = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        self.selected = None;
        self.text = pipes::wrapper(&name, command.trim(), &date);
        self.original.clear();
        self.name = name.clone();
        self.note = "Custom command not allowed. Review the wrapper below; Save creates it as a user pipe file.".into();
        self.ask_save(true);
    }

    fn ask_save(&mut self, new_file: bool) {
        let Some(user) = rx_pipe::user_modem_dir() else {
            self.note = "no user pipe folder (HOME unset)".into();
            return;
        };
        match pipes::check_new(&self.name, &user, &rx_pipe::installed_modem_dirs()) {
            Ok(path) => {
                let exists = path.exists();
                let overwrite = exists && !new_file;
                self.confirm = Some(Confirm::Save { name: self.name.clone(), text: self.text.clone(), overwrite: overwrite || exists });
                if exists && new_file {
                    self.note = format!("{} already exists. Confirming replaces it.", self.name);
                }
            }
            Err(e) => self.note = e,
        }
    }

    /// Draw the window. Returns a station field change to apply.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Apply> {
        if !self.open {
            return None;
        }
        let mut apply = None;
        let mut open = true;
        egui::Window::new("Pipe files").open(&mut open).default_width(640.0).show(ctx, |ui| {
            apply = self.body(ui);
        });
        self.open = open;
        apply
    }

    fn body(&mut self, ui: &mut egui::Ui) -> Option<Apply> {
        let mut apply = None;
        ui.horizontal(|ui| {
            if ui.selectable_label(!self.tx, "RX pipes").clicked() && self.tx {
                self.tx = false;
                self.selected = None;
                self.confirm = None;
            }
            if ui.selectable_label(self.tx, "TX pipes").clicked() && !self.tx {
                self.tx = true;
                self.selected = None;
                self.confirm = None;
            }
            if let Some((_, tx)) = self.target {
                ui.label(muted(ui, format!("radio {} pipe: {}", if tx { "TX" } else { "RX" }, if self.target_value.is_empty() { "none" } else { &self.target_value })));
            }
        });
        ui.label(muted(ui, "Installed files are read-only. User files live in ~/.local/share/hamspot-commander/modems.d."));
        let scripts = if self.tx { rx_pipe::tx_scripts() } else { rx_pipe::modem_scripts() };
        let mut pick = None;
        ui.columns(2, |cols| {
            egui::ScrollArea::vertical().id_salt("pipe-list").max_height(320.0).show(&mut cols[0], |ui| {
                for s in &scripts {
                    let on = self.selected.as_ref().is_some_and(|x| x.path == s.path);
                    let tag = if s.user { "user" } else { "installed" };
                    if ui.selectable_label(on, format!("{}  ({tag})", s.name)).clicked() {
                        pick = Some(s.clone());
                    }
                }
            });
            let ui = &mut cols[1];
            let editable = self.selected.as_ref().map_or(true, |s| s.user);
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.add_enabled(editable, egui::TextEdit::singleline(&mut self.name).desired_width(220.0));
            });
            egui::ScrollArea::vertical().id_salt("pipe-text").max_height(260.0).show(ui, |ui| {
                ui.add_enabled(
                    editable,
                    egui::TextEdit::multiline(&mut self.text).code_editor().desired_rows(16).desired_width(f32::INFINITY),
                );
            });
        });
        if let Some(s) = pick {
            self.select(s);
        }
        let user = rx_pipe::user_modem_dir();
        let installed = rx_pipe::installed_modem_dirs();
        ui.horizontal(|ui| {
            if ui.small_button("New pipe…").clicked() {
                self.selected = None;
                self.name = format!("my-pipe{}", self.ext());
                self.text = pipes::template(&self.name);
                self.original.clear();
                self.note = "New user pipe from the template. Edit the name and text, then Save.".into();
                self.confirm = None;
            }
            let sel = self.selected.clone();
            match &sel {
                Some(s) if !s.user => {
                    if ui.small_button("Duplicate to user pipe…").clicked() {
                        self.selected = None;
                        self.name = format!("my-{}", s.name.trim_start_matches("commander-"));
                        self.note = "Copy of an installed file. Choose a name, then Save.".into();
                    }
                }
                Some(s) => {
                    let renamed = self.name != s.name;
                    if ui.add_enabled(self.text != self.original || renamed, egui::Button::new("Save…").small()).clicked() {
                        if renamed {
                            self.confirm = Some(Confirm::Rename { old: s.name.clone(), new: self.name.clone(), overwrite: false });
                        } else {
                            self.ask_save(false);
                        }
                    }
                    if ui.small_button("Delete…").clicked() {
                        self.confirm = Some(Confirm::Delete { name: s.name.clone() });
                    }
                }
                None => {
                    if ui.small_button("Save…").clicked() {
                        self.ask_save(true);
                    }
                }
            }
            if let (Some((id, tx)), Some(s)) = (self.target, &sel) {
                if tx == self.tx && s.command != self.target_value && ui.small_button("Use for this radio").clicked() {
                    apply = Some((id, tx, s.command.clone()));
                    self.target_value = s.command.clone();
                }
            }
        });
        if !self.note.is_empty() {
            ui.label(egui::RichText::new(&self.note).small().color(crate::caution()));
        }
        if let Some(c) = self.confirm.clone() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                let (title, body) = match &c {
                    Confirm::Save { name, text, overwrite } => (
                        format!("{} {} in modems.d with mode 0750:", if *overwrite { "Replace" } else { "Create" }, name),
                        text.clone(),
                    ),
                    Confirm::Delete { name } => (format!("Delete user pipe file {name}? Radios using it stop their pipe."), String::new()),
                    Confirm::Rename { old, new, .. } => (format!("Rename {old} to {new}? Radios using the old name stop their pipe."), self.text.clone()),
                };
                ui.label(title);
                if !body.is_empty() {
                    egui::ScrollArea::vertical().id_salt("pipe-confirm").max_height(200.0).show(ui, |ui| {
                        ui.label(egui::RichText::new(body).monospace().small());
                    });
                }
                ui.horizontal(|ui| {
                    if ui.small_button("Confirm").clicked() {
                        apply = apply.take().or(self.run(c.clone(), user.as_deref(), &installed));
                        self.confirm = None;
                    }
                    if ui.small_button("Cancel").clicked() {
                        self.confirm = None;
                    }
                });
            });
        }
        apply
    }

    fn run(&mut self, c: Confirm, user: Option<&std::path::Path>, installed: &[std::path::PathBuf]) -> Option<Apply> {
        let Some(user) = user else {
            self.note = "no user pipe folder".into();
            return None;
        };
        let result = match &c {
            Confirm::Save { name, text, overwrite } => pipes::save(name, text, user, installed, *overwrite),
            Confirm::Rename { old, new, overwrite } => pipes::rename(old, new, user, installed, *overwrite).and_then(|p| {
                if self.text != self.original {
                    pipes::save(new, &self.text, user, installed, true)
                } else {
                    Ok(p)
                }
            }),
            Confirm::Delete { name } => pipes::delete(name, user).map(|_| user.join(name)),
        };
        match result {
            Err(e) => {
                self.note = e;
                None
            }
            Ok(_) if matches!(c, Confirm::Delete { .. }) => {
                self.selected = None;
                self.text.clear();
                self.original.clear();
                self.note = "Deleted.".into();
                None
            }
            Ok(path) => {
                let name = self.name.clone();
                self.original = self.text.clone();
                self.note = format!("Saved {}", path.display());
                self.selected = Some(ModemScript { command: hsrec::safepath::MODEMS.reference(&name), name, path, user: true });
                // A wrapper made for a refused field is pointed at right away:
                // the user just confirmed creating it for that field.
                match (self.target, self.selected.as_ref()) {
                    (Some((id, tx)), Some(s)) if tx == self.tx && self.target_value != s.command && rx_pipe::resolve_pipe(&self.target_value).is_err() && !self.target_value.trim().is_empty() => {
                        self.target_value = s.command.clone();
                        Some((id, tx, s.command.clone()))
                    }
                    _ => None,
                }
            }
        }
    }
}
