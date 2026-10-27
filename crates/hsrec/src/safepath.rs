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
    stored.trim().strip_prefix(MODEMS_PREFIX).and_then(script_name).ok_or_else(missing)?;
    MODEMS.resolve(stored, dirs).map_err(|e| if e.starts_with("pipe file not found") { missing() } else { e })
}

/// One folder of named, runnable asset files Commander owns. Station files
/// and recordings refer to an asset only as `<dir>/<name><ext>`; the file is
/// looked up in Commander's own folders for that kind and nowhere else.
/// A new kind (e.g. `flowgraphs`) is one more entry in [`KINDS`].
#[derive(Debug, PartialEq, Eq)]
pub struct AssetKind {
    /// Folder name and reference prefix, e.g. `modems`.
    pub dir: &'static str,
    /// Allowed file name endings.
    pub exts: &'static [&'static str],
    /// Name of the user-writable folder next to the installed one.
    pub user_dir: &'static str,
}

/// RX and TX pipe files.
pub const MODEMS: AssetKind = AssetKind { dir: "modems", exts: &[".rxpipe", ".txpipe"], user_dir: "modems.d" };

/// Every asset kind. Add new folders here.
pub const KINDS: &[&AssetKind] = &[&MODEMS];

/// Text shown when a stored value is not a pipe file reference.
pub const CUSTOM_REFUSED: &str = "custom command not allowed; create a pipe file in modems/ — see docs (Pipe files)";

impl AssetKind {
    /// `name` is a bare slug file name of this kind: `[a-z0-9._-]`, not
    /// starting with `.`, ending in one of the extensions.
    pub fn valid_name(&self, name: &str) -> bool {
        let ok_chars = name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'));
        ok_chars && !name.starts_with('.') && self.exts.iter().any(|e| name.len() > e.len() && name.ends_with(e))
    }
    pub fn has_ext(&self, name: &str, ext: &str) -> bool {
        self.exts.contains(&ext) && name.ends_with(ext) && self.valid_name(name)
    }
    /// Bare name from a strict `<dir>/<name>` reference.
    pub fn name_of<'a>(&self, stored: &'a str) -> Option<&'a str> {
        let rest = stored.strip_prefix(self.dir)?.strip_prefix('/')?;
        self.valid_name(rest).then_some(rest)
    }
    pub fn reference(&self, name: &str) -> String {
        format!("{}/{name}", self.dir)
    }
    /// In-memory migration of an old value: an absolute path whose file name
    /// is a valid name of this kind and exists in `dirs` becomes
    /// `<dir>/<name>`. A strict reference is returned unchanged. Anything else
    /// (a shell command, args, an unknown path) gives `None`.
    pub fn normalise(&self, value: &str, dirs: &[PathBuf]) -> Option<String> {
        let v = value.trim();
        if let Some(n) = self.name_of(v) {
            return Some(self.reference(n));
        }
        let p = Path::new(v);
        if !p.is_absolute() || v.contains(char::is_whitespace) || p.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
            return None;
        }
        let name = p.file_name()?.to_str()?;
        if !self.valid_name(name) {
            return None;
        }
        dirs.iter().any(|d| d.join(name).is_file()).then(|| self.reference(name))
    }
    /// Resolve a strict reference to a real file in one of `dirs` (first
    /// match wins). The entry must be a regular file (not a symlink),
    /// executable by the owner, and neither it nor its folder may be group-
    /// or world-writable.
    pub fn resolve(&self, stored: &str, dirs: &[PathBuf]) -> Result<PathBuf, String> {
        let stored = stored.trim();
        let Some(name) = self.name_of(stored) else {
            return Err(if stored.is_empty() { "no pipe file chosen".into() } else { CUSTOM_REFUSED.to_string() });
        };
        for dir in dirs {
            let Ok(root) = dir.canonicalize() else { continue };
            let path = root.join(name);
            let Ok(md) = std::fs::symlink_metadata(&path) else { continue };
            if !md.file_type().is_file() {
                return Err(format!("{stored} is not a regular file (symlinks are refused)"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let dir_mode = std::fs::metadata(&root).map(|m| m.permissions().mode()).unwrap_or(0o777);
                let mode = md.permissions().mode();
                if mode & 0o022 != 0 || dir_mode & 0o022 != 0 {
                    return Err(format!("{stored} refused: file or folder is group/world-writable (chmod go-w {})", path.display()));
                }
                if mode & 0o100 == 0 {
                    return Err(format!("{stored} refused: not executable (chmod u+x {})", path.display()));
                }
            }
            return Ok(path);
        }
        Err(format!("pipe file not found: {stored}"))
    }
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

    fn chmod(p: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn kind_names_and_references() {
        for ok in ["a.rxpipe", "commander-demod-p25.rxpipe", "x_1.2.txpipe"] {
            assert!(MODEMS.valid_name(ok), "{ok}");
        }
        for bad in ["", ".rxpipe", ".a.rxpipe", "A.rxpipe", "a b.rxpipe", "a;rm.rxpipe", "a.sh", "a.rxpipe.bak", "sub/a.rxpipe", "../a.rxpipe", "a$(x).rxpipe"] {
            assert!(!MODEMS.valid_name(bad), "{bad}");
        }
        assert_eq!(MODEMS.name_of("modems/a.txpipe"), Some("a.txpipe"));
        for bad in ["a.txpipe", "/modems/a.txpipe", "modems/sub/a.txpipe", "modems/../a.txpipe", "modems/a.txpipe --x", "modemsx/a.txpipe"] {
            assert_eq!(MODEMS.name_of(bad), None, "{bad}");
        }
        assert!(KINDS.iter().any(|k| k.dir == "modems"));
    }

    #[test]
    fn kind_normalise_and_resolve() {
        let base = tmp("kind");
        let inst = base.join("modems");
        let user = base.join("modems.d");
        std::fs::create_dir_all(&inst).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        chmod(&inst, 0o755);
        chmod(&user, 0o750);
        for (d, n) in [(&inst, "i.rxpipe"), (&user, "u.txpipe"), (&user, "ww.rxpipe"), (&user, "noexec.rxpipe"), (&base, "out.rxpipe")] {
            std::fs::write(d.join(n), "#!/bin/sh\n").unwrap();
            chmod(&d.join(n), 0o750);
        }
        chmod(&user.join("ww.rxpipe"), 0o757);
        chmod(&user.join("noexec.rxpipe"), 0o640);
        std::os::unix::fs::symlink(base.join("out.rxpipe"), user.join("ln.rxpipe")).unwrap();
        let dirs = vec![inst.clone(), user.clone()];
        assert_eq!(MODEMS.normalise("/somewhere/else/i.rxpipe", &dirs).as_deref(), Some("modems/i.rxpipe"));
        assert_eq!(MODEMS.normalise("modems/u.txpipe", &dirs).as_deref(), Some("modems/u.txpipe"));
        for bad in ["/usr/bin/dsdccx", "dsdccx -i - -o -", "/x/missing.rxpipe", "/x/../i.rxpipe", "sh -c 'i.rxpipe'"] {
            assert_eq!(MODEMS.normalise(bad, &dirs), None, "{bad}");
        }
        assert!(MODEMS.resolve("modems/i.rxpipe", &dirs).is_ok());
        assert!(MODEMS.resolve("modems/u.txpipe", &dirs).is_ok());
        assert!(MODEMS.resolve("modems/ww.rxpipe", &dirs).unwrap_err().contains("writable"));
        assert!(MODEMS.resolve("modems/noexec.rxpipe", &dirs).unwrap_err().contains("executable"));
        assert!(MODEMS.resolve("modems/ln.rxpipe", &dirs).unwrap_err().contains("symlink"));
        assert_eq!(MODEMS.resolve("dsdccx -i - -o -", &dirs).unwrap_err(), CUSTOM_REFUSED);
        for bad in ["/abs/i.rxpipe", "modems/../out.rxpipe", "modems/x/i.rxpipe", "modems/i.rxpipe 1", "modems/i.sh", "modems/zz.rxpipe"] {
            assert!(MODEMS.resolve(bad, &dirs).is_err(), "{bad}");
        }
        chmod(&user, 0o770);
        assert!(MODEMS.resolve("modems/u.txpipe", &dirs).unwrap_err().contains("writable"));
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
        chmod(&modems, 0o755);
        chmod(&modems.join("a.rxpipe"), 0o750);
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
