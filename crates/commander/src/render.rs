//! How Commander draws: GPU (OpenGL via EGL/glow) or software (Mesa llvmpipe).
//! Chosen before the window opens (David, 2026-10-30: video playback in other
//! apps failed while Commander ran on a 3-GPU Tower; an amdgpu sdma hang on
//! another GPU). Order: `HAMSPOT_SOFTWARE_RENDER` (1/0) > saved setting >
//! auto (software when the machine has more than one GPU render node).
//! The choice and the reason are printed at start and shown in Config.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const RENDER_VERSION: u32 = 1;
pub const ENV: &str = "HAMSPOT_SOFTWARE_RENDER";
const FILE: &str = "render.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mode {
    /// Software when more than one GPU render node exists, else GPU.
    #[default]
    Auto,
    Gpu,
    Software,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderSettings {
    pub version: u32,
    pub mode: Mode,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self { version: RENDER_VERSION, mode: Mode::Auto }
    }
}

/// What was decided at start, for the Config note.
#[derive(Clone, Debug, Default)]
pub struct Choice {
    pub software: bool,
    pub reason: String,
    /// Problems reading the setting file (never silent).
    pub warning: Option<String>,
}

pub fn path() -> Option<PathBuf> {
    eframe::storage_dir("hamspotcommander").map(|d| d.join(FILE))
}

pub fn load() -> (RenderSettings, Option<String>) {
    let Some(p) = path() else { return (RenderSettings::default(), None) };
    match std::fs::read_to_string(&p) {
        Ok(t) => match serde_json::from_str::<RenderSettings>(&t) {
            Ok(s) => (s, None),
            Err(e) => (RenderSettings::default(), Some(format!("render setting unreadable ({e}); using Auto"))),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (RenderSettings::default(), None),
        Err(e) => (RenderSettings::default(), Some(format!("render setting unreadable ({e}); using Auto"))),
    }
}

pub fn save(s: &RenderSettings) -> Result<(), String> {
    let p = path().ok_or("no data folder for the render setting")?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// GPU render nodes (/dev/dri/renderD*).
pub fn render_nodes() -> usize {
    std::fs::read_dir("/dev/dri")
        .map(|d| d.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("renderD")).count())
        .unwrap_or(0)
}

pub fn decide(env: Option<&str>, saved: Mode, nodes: usize) -> (bool, String) {
    match env.map(str::trim) {
        Some("1") | Some("true") | Some("yes") => return (true, format!("{ENV}=1")),
        Some("0") | Some("false") | Some("no") => return (false, format!("{ENV}=0")),
        Some(other) if !other.is_empty() => {
            let (sw, why) = decide(None, saved, nodes);
            return (sw, format!("{why} ({ENV}={other:?} ignored: use 1 or 0)"));
        }
        _ => {}
    }
    match saved {
        Mode::Software => (true, "Config setting: software".into()),
        Mode::Gpu => (false, "Config setting: GPU".into()),
        Mode::Auto if nodes > 1 => (true, format!("Auto: {nodes} GPUs, software avoids cross-GPU copies")),
        Mode::Auto => (false, format!("Auto: {nodes} GPU")),
    }
}

/// Call first in main, before any thread or GL init.
pub fn apply() -> Choice {
    let (saved, warning) = load();
    let env = std::env::var(ENV).ok();
    let (software, reason) = decide(env.as_deref(), saved.mode, render_nodes());
    if software {
        // SAFETY: main has not started any thread yet.
        unsafe {
            std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
            std::env::set_var("GALLIUM_DRIVER", "llvmpipe");
        }
    }
    eprintln!("commander: rendering {} ({reason})", if software { "software (llvmpipe)" } else { "GPU" });
    if let Some(w) = &warning {
        eprintln!("commander: {w}");
    }
    Choice { software, reason, warning }
}

/// The note for Config, comparing what was asked with the GL renderer string.
pub fn note(choice: &Choice, gl_renderer: Option<&str>) -> String {
    let got = gl_renderer.unwrap_or("unknown renderer");
    let mut s = format!("Rendering: {} ({}). OpenGL: {got}.", if choice.software { "software" } else { "GPU" }, choice.reason);
    let soft = gl_renderer.is_some_and(|r| r.to_ascii_lowercase().contains("llvmpipe") || r.to_ascii_lowercase().contains("softpipe"));
    if choice.software && gl_renderer.is_some() && !soft {
        s.push_str(" WARNING: software was asked for but the GPU driver loaded.");
    }
    if let Some(w) = &choice.warning {
        s.push_str(&format!(" {w}."));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_wins_then_setting_then_auto() {
        assert!(decide(Some("1"), Mode::Gpu, 1).0);
        assert!(!decide(Some("0"), Mode::Software, 3).0);
        assert!(decide(None, Mode::Software, 1).0);
        assert!(!decide(None, Mode::Gpu, 3).0);
        assert!(decide(None, Mode::Auto, 3).0, "multi-GPU auto is software");
        assert!(!decide(None, Mode::Auto, 1).0);
        let (sw, why) = decide(Some("maybe"), Mode::Auto, 1);
        assert!(!sw && why.contains("ignored"), "bad env is reported");
    }

    #[test]
    fn settings_round_trip_and_old_files_default() {
        let s = RenderSettings { version: 1, mode: Mode::Gpu };
        let t = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<RenderSettings>(&t).unwrap(), s);
        assert_eq!(serde_json::from_str::<RenderSettings>("{}").unwrap(), RenderSettings::default());
    }

    #[test]
    fn note_warns_when_software_did_not_take() {
        let c = Choice { software: true, reason: "x".into(), warning: None };
        assert!(note(&c, Some("ATI FirePro W5000 (radeonsi)")).contains("WARNING"));
        assert!(!note(&c, Some("llvmpipe (LLVM 21)")).contains("WARNING"));
    }
}
