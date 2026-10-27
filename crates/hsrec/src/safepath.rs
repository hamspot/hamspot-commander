//! Paths that come from stored or imported data (DB rows, sidecars, a thumb
//! drive's `index.json`) are never trusted as filesystem locations. Recordings
//! store only portable relative names; Commander resolves them against its own
//! known folders at use time and rejects anything that would land outside.
use std::path::{Component, Path, PathBuf};

/// Prefix of a portable decoder name, e.g. `modems/commander-demod-p25.rxpipe`.
pub const MODEMS_PREFIX: &str = "modems/";
pub const RX_SUFFIX: &str = ".rxpipe";

/// A stored relative path is safe to join under a root: not empty, not
/// absolute, only normal components (no `..`, `.`, root or drive prefix), no
/// backslash or NUL.
pub fn safe_rel(rel: &str) -> bool {
    if rel.is_empty() || rel.contains('\\') || rel.contains('\0') {
        return false;
    }
    let p = Path::new(rel);
    !p.is_absolute() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// `root.join(rel)` when `rel` is [`safe_rel`] and no existing component
/// under `root` is a symlink (so a planted link cannot redirect a read,
/// write or delete outside `root`).
pub fn join_inside(root: &Path, rel: &str) -> Option<PathBuf> {
    if !safe_rel(rel) {
        return None;
    }
    let mut cur = root.to_path_buf();
    for c in Path::new(rel).components() {
        cur.push(c);
        if let Ok(md) = std::fs::symlink_metadata(&cur) {
            if md.file_type().is_symlink() {
                return None;
            }
        }
    }
    Some(cur)
}

/// Single file name of an RX script: one normal component ending `.rxpipe`.
fn script_name(name: &str) -> Option<&str> {
    (name.len() > RX_SUFFIX.len() && name.ends_with(RX_SUFFIX) && safe_rel(name) && !name.contains('/')).then_some(name)
}

/// Portable form of a decoder script for recording metadata. Accepts the
/// live command (usually an absolute path into a modems folder), an old
/// stored absolute path, or an already portable `modems/<file>`. Returns
/// `modems/<file>`, or `None` for anything that is not a plain `.rxpipe`
/// script (a custom shell command is never stored as something to run).
pub fn portable_decoder(script: &str) -> Option<String> {
    let s = script.trim();
    if let Some(rest) = s.strip_prefix(MODEMS_PREFIX) {
        return script_name(rest).map(|n| format!("{MODEMS_PREFIX}{n}"));
    }
    let p = Path::new(s);
    if s.contains(char::is_whitespace) || p.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return None;
    }
    let name = p.file_name()?.to_str()?;
    script_name(name).map(|n| format!("{MODEMS_PREFIX}{n}"))
}

/// Resolve a stored decoder name to the real script inside one of `dirs`
/// (Commander's own modems folders). Only the strict `modems/<file>` form is
/// accepted; the result must canonicalise to a regular file inside the dir.
pub fn resolve_decoder(stored: &str, dirs: &[PathBuf]) -> Result<PathBuf, String> {
    let shown = if stored.trim().is_empty() { "(none)".to_string() } else { stored.trim().to_string() };
    let missing = || format!("decoder script not found: {shown}");
    let name = stored.trim().strip_prefix(MODEMS_PREFIX).and_then(script_name).ok_or_else(missing)?;
    for dir in dirs {
        let Ok(root) = dir.canonicalize() else { continue };
        let Ok(real) = root.join(name).canonicalize() else { continue };
        if real.parent() == Some(root.as_path()) && real.is_file() {
            return Ok(real);
        }
    }
    Err(missing())
}

/// Rewrite `meta.decoder.script` to its portable form (migrates old absolute
/// paths, drops anything not a plain script). Returns true if it changed.
pub fn portable_meta(meta: &mut crate::meta::Meta) -> bool {
    let Some(obj) = meta.decoder.as_object_mut() else { return false };
    let Some(old) = obj.get("script").and_then(|v| v.as_str()).map(str::to_string) else { return false };
    match portable_decoder(&old) {
        Some(p) if p == old => false,
        Some(p) => {
            obj.insert("script".into(), serde_json::Value::String(p));
            true
        }
        None => {
            obj.remove("script");
            if !old.trim().is_empty() {
                obj.insert("custom_command".into(), serde_json::Value::Bool(true));
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hsrec-safepath-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn normalises_to_modems_name() {
        assert_eq!(portable_decoder("/home/user/.local/share/hamspot-commander/modems/commander-demod-p25.rxpipe").as_deref(), Some("modems/commander-demod-p25.rxpipe"));
        assert_eq!(portable_decoder("modems/commander-demod-p25.rxpipe").as_deref(), Some("modems/commander-demod-p25.rxpipe"));
        assert_eq!(portable_decoder("sh -c 'evil'"), None);
        assert_eq!(portable_decoder("/usr/bin/evil"), None);
        assert_eq!(portable_decoder("modems/../x.rxpipe"), None);
        assert_eq!(portable_decoder("/a/../b/x.rxpipe"), None);
        assert_eq!(portable_decoder(""), None);
    }

    #[test]
    fn resolves_inside_modems_only() {
        let base = tmp("resolve");
        let modems = base.join("modems");
        std::fs::create_dir_all(&modems).unwrap();
        std::fs::write(modems.join("a.rxpipe"), "#!/bin/sh\n").unwrap();
        std::fs::write(base.join("outside.rxpipe"), "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(base.join("outside.rxpipe"), modems.join("link.rxpipe")).unwrap();
        let dirs = vec![base.join("nope"), modems.clone()];
        assert_eq!(resolve_decoder("modems/a.rxpipe", &dirs).unwrap(), modems.join("a.rxpipe").canonicalize().unwrap());
        for bad in ["/etc/passwd", "modems/../outside.rxpipe", "../outside.rxpipe", "modems/link.rxpipe", "modems/zzz.rxpipe", "a.rxpipe", "modems/sub/a.rxpipe", ""] {
            assert!(resolve_decoder(bad, &dirs).is_err(), "{bad}");
        }
        assert_eq!(resolve_decoder("modems/zzz.rxpipe", &dirs).unwrap_err(), "decoder script not found: modems/zzz.rxpipe");
    }

    #[test]
    fn old_absolute_metadata_migrates_on_read() {
        let mut m = crate::meta::Meta::default();
        m.decoder = serde_json::json!({"script": "/home/user/.local/share/hamspot-commander/modems/commander-demod-p25.rxpipe", "vocoder": "p25"});
        assert!(portable_meta(&mut m));
        assert_eq!(m.decoder["script"], "modems/commander-demod-p25.rxpipe");
        assert!(!portable_meta(&mut m));
        m.decoder = serde_json::json!({"script": "dsd-fme -i - | aplay"});
        assert!(portable_meta(&mut m));
        assert!(m.decoder.get("script").is_none() && m.decoder["custom_command"] == true);
    }

    #[test]
    fn rel_paths_and_symlink_escape() {
        assert!(safe_rel("2026/10/x.cs16"));
        for bad in ["", "/etc/x", "../x", "a/../../x", "./x", "a\\..\\x"] {
            assert!(!safe_rel(bad), "{bad}");
        }
        let base = tmp("join");
        let root = base.join("root");
        std::fs::create_dir_all(&root).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&base, root.join("esc")).unwrap();
        assert!(join_inside(&root, "esc/x.cs16").is_none());
        assert_eq!(join_inside(&root, "d/x.cs16"), Some(root.join("d/x.cs16")));
    }
}
