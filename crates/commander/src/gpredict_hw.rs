//! Gpredict hardware files for the radios and rotators on the Config tab.
//!
//! Gpredict builds its device menu from `hwconf/*.rig` and `*.rot` when Radio
//! Control or Rotator Control opens. Files this program owns start with
//! `hamspot_`. A file with any other name is never created, edited, or deleted.

use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const PREFIX: &str = "hamspot_";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Radio,
    Rotator,
}

impl Kind {
    fn ext(self) -> &'static str {
        match self {
            Kind::Radio => "rig",
            Kind::Rotator => "rot",
        }
    }

    fn group(self) -> &'static str {
        match self {
            Kind::Radio => "Radio",
            Kind::Rotator => "Rotator",
        }
    }
}

/// One Config-tab row. `hamlib` is false for a GQRX radio. Gpredict speaks hamlib.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub name: String,
    pub port: Option<u16>,
    pub hamlib: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownFile {
    pub stem: String,
    pub kind: Kind,
    pub owned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedWire {
    pub id: u64,
    pub stem: String,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncOut {
    pub files: Vec<ShownFile>,
    pub wires: Vec<OwnedWire>,
}

/// `$XDG_CONFIG_HOME/Gpredict/hwconf`, or `~/.config/Gpredict/hwconf`.
pub fn hwconf_dir() -> Result<PathBuf, String> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("Gpredict").join("hwconf"));
        }
    }
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => Ok(PathBuf::from(home)
            .join(".config")
            .join("Gpredict")
            .join("hwconf")),
        _ => Err("HOME is not set, so Gpredict hardware files were not written".into()),
    }
}

/// Make the owned `hamspot_` files match `radios` and `rotators`.
pub fn sync(dir: &Path, radios: &[Row], rotators: &[Row]) -> Result<SyncOut, String> {
    fs::create_dir_all(dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    let dir = dir
        .canonicalize()
        .map_err(|err| format!("cannot resolve {}: {err}", dir.display()))?;
    let radios = assign(radios, Kind::Radio);
    let rotators = assign(rotators, Kind::Rotator);
    apply(&dir, Kind::Radio, &radios)?;
    apply(&dir, Kind::Rotator, &rotators)?;
    let mut wires = Vec::new();
    for row in radios.iter().chain(&rotators) {
        wires.push(OwnedWire {
            id: row.id,
            stem: row.stem.clone(),
            kind: row.kind,
        });
    }
    Ok(SyncOut {
        files: list(&dir)?,
        wires,
    })
}

/// Hardware files currently in `dir`. A missing directory is an empty list.
pub fn list(dir: &Path) -> Result<Vec<ShownFile>, String> {
    let mut files = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(files),
        Err(err) => return Err(format!("cannot read {}: {err}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|err| format!("cannot read {}: {err}", dir.display()))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let kind = if name.ends_with(".rig") {
            Kind::Radio
        } else if name.ends_with(".rot") {
            Kind::Rotator
        } else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let stem = name[..name.len() - 4].to_string();
        if stem.is_empty() || stem.contains('/') || stem.contains("..") {
            continue;
        }
        let stem_owned = stem.starts_with(PREFIX);
        files.push(ShownFile {
            stem,
            kind,
            owned: stem_owned,
        });
    }
    files.sort_by(|a, b| {
        a.stem
            .cmp(&b.stem)
            .then(kind_ord(a.kind).cmp(&kind_ord(b.kind)))
    });
    Ok(files)
}

fn kind_ord(kind: Kind) -> u8 {
    match kind {
        Kind::Radio => 0,
        Kind::Rotator => 1,
    }
}

struct Assigned {
    id: u64,
    stem: String,
    port: u16,
    kind: Kind,
}

fn assign(rows: &[Row], kind: Kind) -> Vec<Assigned> {
    let mut used = BTreeSet::new();
    let mut rows: Vec<&Row> = rows
        .iter()
        .filter(|row| row.hamlib && row.port.is_some())
        .collect();
    rows.sort_by_key(|row| row.id);
    let mut out = Vec::new();
    for row in rows {
        let Some(stem) = stem_for(&row.name, row.id, &used) else {
            continue;
        };
        used.insert(stem.clone());
        out.push(Assigned {
            id: row.id,
            stem,
            port: row.port.expect("filtered"),
            kind,
        });
    }
    out
}

fn stem_for(name: &str, id: u64, used: &BTreeSet<String>) -> Option<String> {
    let clean = sanitize(name)?;
    let base = format!("{PREFIX}{clean}");
    if !used.contains(&base) {
        return Some(base);
    }
    let alt = format!("{base}_{id}");
    if used.contains(&alt) {
        return None;
    }
    Some(alt)
}

/// Drop separators and a leading dot. A stem that still contains `..` is refused.
fn sanitize(name: &str) -> Option<String> {
    let mut clean: String = name
        .chars()
        .filter(|ch| !matches!(ch, '/' | '\\' | '\0' | '\n' | '\r'))
        .collect();
    while clean.starts_with('.') {
        clean.remove(0);
    }
    loop {
        let next = clean.replace(".rig", "").replace(".rot", "");
        if next == clean {
            break;
        }
        clean = next;
    }
    let clean = clean.trim().to_string();
    if clean.is_empty() || clean.contains("..") || clean.contains('/') || clean.contains('\\') {
        return None;
    }
    Some(clean)
}

fn apply(dir: &Path, kind: Kind, owned: &[Assigned]) -> Result<(), String> {
    let existing = owned_files(dir, kind)?;
    let desired: BTreeSet<&str> = owned.iter().map(|row| row.stem.as_str()).collect();
    let mut claimed = BTreeSet::new();
    let mut pending = Vec::new();
    for row in owned {
        if existing.iter().any(|(stem, _)| stem == &row.stem) {
            let path = file_path(dir, &row.stem, kind)?;
            write_port(&path, kind, row.port)?;
            claimed.insert(row.stem.clone());
        } else {
            pending.push(row);
        }
    }
    let mut leftovers = Vec::new();
    for (stem, path) in &existing {
        if claimed.contains(stem) {
            continue;
        }
        if let Some(port) = read_port(path, kind) {
            leftovers.push((stem.clone(), path.clone(), port));
        }
    }
    for row in pending {
        if let Some(index) = leftovers.iter().position(|(_, _, port)| *port == row.port) {
            let (_, old_path, _) = leftovers.remove(index);
            let new_path = file_path(dir, &row.stem, kind)?;
            // `rename` replaces a symlink with a normal directory entry. The
            // following write refuses to follow a symlink out of this directory.
            fs::rename(&old_path, &new_path).map_err(|err| {
                format!(
                    "cannot rename {} to {}: {err}",
                    old_path.display(),
                    new_path.display()
                )
            })?;
            write_port(&new_path, kind, row.port)?;
        } else {
            let path = file_path(dir, &row.stem, kind)?;
            write_new(&path, kind, row.port)?;
        }
    }
    for (stem, path) in owned_files(dir, kind)? {
        if !desired.contains(stem.as_str()) {
            fs::remove_file(&path)
                .map_err(|err| format!("cannot remove {}: {err}", path.display()))?;
        }
    }
    Ok(())
}

fn owned_files(dir: &Path, kind: Kind) -> Result<Vec<(String, PathBuf)>, String> {
    let mut found = Vec::new();
    for file in list(dir)? {
        if file.kind == kind && file.owned {
            found.push((file.stem.clone(), file_path(dir, &file.stem, kind)?));
        }
    }
    Ok(found)
}

fn file_path(dir: &Path, stem: &str, kind: Kind) -> Result<PathBuf, String> {
    if !stem.starts_with(PREFIX)
        || stem.contains('/')
        || stem.contains('\\')
        || stem.contains('\0')
        || stem.contains("..")
    {
        return Err(format!("refusing hardware name {stem}"));
    }
    let name = format!("{stem}.{}", kind.ext());
    let path = dir.join(&name);
    if path.parent() != Some(dir) {
        return Err(format!("refusing hardware path {}", path.display()));
    }
    Ok(path)
}

fn write_new(path: &Path, kind: Kind, port: u16) -> Result<(), String> {
    refuse_symlink(path)?;
    fs::write(path, template(kind, port))
        .map_err(|err| format!("cannot write {}: {err}", path.display()))
}

fn write_port(path: &Path, kind: Kind, port: u16) -> Result<(), String> {
    refuse_symlink(path)?;
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => return write_new(path, kind, port),
        Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
    };
    let text = match String::from_utf8(bytes) {
        Ok(text) if !blank(&text) => text,
        _ => return write_new(path, kind, port),
    };
    let next = match rewrite_host_port(&text, kind.group(), port) {
        Rewrite::Same => text.clone(),
        Rewrite::Changed(next) => next,
    };
    // Type 2 is one VFO. Gpredict draws the uplink anyway, and sends it only
    // as `F` on that same VFO. Duplex sends `S VFOA 1 VFOB` and `I VFOA`,
    // which is the command that moves VFO B. A file that already has VFO_UP
    // or VFO_DOWN is left as the operator saved it.
    let next = if kind == Kind::Radio {
        promote_duplex(&next).unwrap_or(next)
    } else {
        next
    };
    if next == text {
        return Ok(());
    }
    fs::write(path, next).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

/// `VFO_UP=2` is VFO B and `VFO_DOWN=1` is VFO A. Gpredict writes those only
/// when Type is duplex (3). Half-duplex with no VFO keys is the file this
/// program used to create, and it never reaches the sub VFO.
fn promote_duplex(text: &str) -> Option<String> {
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    let mut in_group = false;
    let mut saw = false;
    let mut type_at = None;
    let mut type_is_half = false;
    let mut has_vfo = false;
    for (index, line) in lines.iter().enumerate() {
        let body = line.trim_end_matches(['\n', '\r']);
        if let Some(name) = group_name(body) {
            if saw {
                break;
            }
            in_group = name == "Radio";
            if in_group {
                saw = true;
            }
            continue;
        }
        if !in_group {
            continue;
        }
        if let Some((key, value)) = key_of(body) {
            if key == "Type" && type_at.is_none() {
                type_at = Some(index);
                type_is_half = value == "2";
            }
            if key == "VFO_UP" || key == "VFO_DOWN" {
                has_vfo = true;
            }
        }
    }
    if !type_is_half || has_vfo {
        return None;
    }
    let index = type_at?;
    lines[index] = set_value(&lines[index], "3");
    if !lines[index].ends_with('\n') {
        lines[index].push('\n');
    }
    lines.insert(index + 1, "VFO_UP=2\nVFO_DOWN=1\n".to_string());
    Some(lines.concat())
}

/// `std::fs::write` follows a symlink. Remove one so the new bytes stay in `hwconf`.
fn refuse_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::remove_file(path)
            .map_err(|err| format!("cannot replace symlink {}: {err}", path.display())),
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => Err(format!("{} is not a hardware file", path.display())),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("cannot inspect {}: {err}", path.display())),
    }
}

fn read_port(path: &Path, kind: Kind) -> Option<u16> {
    let text = fs::read_to_string(path).ok()?;
    host_and_port(&text, kind.group()).1
}

fn template(kind: Kind, port: u16) -> String {
    match kind {
        Kind::Radio => format!(
            "[Radio]\nHost=127.0.0.1\nPort={port}\nLO=0\nLO_UP=0\nDefaultUplinkFreqHz=145890000\nDefaultDownlinkFreqHz=145890000\nType=3\nVFO_UP=2\nVFO_DOWN=1\nPTT=0\nSIGNAL_AOS=false\nSIGNAL_LOS=false\n"
        ),
        Kind::Rotator => format!(
            "[Rotator]\nHost=127.0.0.1\nPort={port}\nMinAz=0\nMaxAz=360\nMinEl=0\nMaxEl=90\nAzStopPos=0\n"
        ),
    }
}

enum Rewrite {
    Same,
    Changed(String),
}

fn rewrite_host_port(text: &str, group: &str, port: u16) -> Rewrite {
    let (host, found) = host_and_port(text, group);
    if host.as_deref() == Some("127.0.0.1") && found == Some(port) {
        return Rewrite::Same;
    }
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    if lines.is_empty() {
        return Rewrite::Changed(format!("[{group}]\nHost=127.0.0.1\nPort={port}\n"));
    }
    let mut in_group = false;
    let mut saw_group = false;
    let mut host_at = None;
    let mut port_at = None;
    let mut group_end = None;
    for (index, line) in lines.iter().enumerate() {
        let body = line.trim_end_matches(['\n', '\r']);
        if let Some(name) = group_name(body) {
            if in_group && group_end.is_none() {
                group_end = Some(index);
            }
            in_group = !saw_group && name == group;
            if in_group {
                saw_group = true;
            }
            continue;
        }
        if !in_group {
            continue;
        }
        if let Some((key, _)) = key_of(body) {
            if key == "Host" && host_at.is_none() {
                host_at = Some(index);
            } else if key == "Port" && port_at.is_none() {
                port_at = Some(index);
            }
        }
    }
    if in_group {
        group_end = Some(lines.len());
    }
    if !saw_group {
        let mut next = text.to_string();
        if !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(&format!("[{group}]\nHost=127.0.0.1\nPort={port}\n"));
        return Rewrite::Changed(next);
    }
    if let Some(index) = host_at {
        lines[index] = set_value(&lines[index], "127.0.0.1");
    }
    if let Some(index) = port_at {
        lines[index] = set_value(&lines[index], &port.to_string());
    }
    let mut extra = String::new();
    if host_at.is_none() {
        extra.push_str("Host=127.0.0.1\n");
    }
    if port_at.is_none() {
        extra.push_str(&format!("Port={port}\n"));
    }
    if !extra.is_empty() {
        let insert_at = group_end.unwrap_or(lines.len());
        if insert_at > 0 {
            let previous = &mut lines[insert_at - 1];
            if !previous.ends_with('\n') {
                previous.push('\n');
            }
        }
        lines.insert(insert_at, extra);
    }
    Rewrite::Changed(lines.concat())
}

fn host_and_port(text: &str, group: &str) -> (Option<String>, Option<u16>) {
    let mut in_group = false;
    let mut saw_group = false;
    let mut host = None;
    let mut port = None;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        if let Some(name) = group_name(body) {
            in_group = !saw_group && name == group;
            if in_group {
                saw_group = true;
            }
            continue;
        }
        if !in_group {
            continue;
        }
        if let Some((key, value)) = key_of(body) {
            if key == "Host" && host.is_none() {
                host = Some(value.to_string());
            } else if key == "Port" && port.is_none() {
                port = value.parse().ok();
            }
        }
    }
    (host, port)
}

fn group_name(body: &str) -> Option<&str> {
    let body = body.trim();
    if body.starts_with('[') && body.ends_with(']') && body.len() > 2 {
        let name = body[1..body.len() - 1].trim();
        if !name.is_empty() && !name.contains('[') && !name.contains(']') {
            return Some(name);
        }
    }
    None
}

fn key_of(body: &str) -> Option<(&str, &str)> {
    let trimmed = body.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
        return None;
    }
    if group_name(body).is_some() {
        return None;
    }
    let eq = body.find('=')?;
    let key = body[..eq].trim();
    if key.is_empty() {
        return None;
    }
    Some((key, body[eq + 1..].trim()))
}

fn set_value(line: &str, new_val: &str) -> String {
    let ending = if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let body = line.trim_end_matches(['\n', '\r']);
    let Some(eq) = body.find('=') else {
        return line.to_string();
    };
    let prefix = &body[..=eq];
    let rest = &body[eq + 1..];
    let gap_len = rest.len() - rest.trim_start().len();
    format!("{prefix}{}{new_val}{ending}", &rest[..gap_len])
}

fn blank(text: &str) -> bool {
    text.lines().all(|line| {
        let trimmed = line.trim();
        trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("hamspot-hw-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn radio(id: u64, name: &str, port: Option<u16>, hamlib: bool) -> Row {
        Row {
            id,
            name: name.into(),
            port,
            hamlib,
        }
    }

    fn rot(id: u64, name: &str, port: Option<u16>) -> Row {
        Row {
            id,
            name: name.into(),
            port,
            hamlib: true,
        }
    }

    #[test]
    fn sync_writes_one_radio_and_one_rotator() {
        let dir = scratch();
        sync(
            &dir,
            &[radio(1, "FT-450D", Some(4532), true)],
            &[rot(2, "Tower", Some(4533))],
        )
        .unwrap();
        let rig = fs::read_to_string(dir.join("hamspot_FT-450D.rig")).unwrap();
        assert!(rig.contains("Host=127.0.0.1\n"));
        assert!(rig.contains("Port=4532\n"));
        assert!(rig.contains("Type=3\n"));
        assert!(rig.contains("VFO_UP=2\n"));
        assert!(rig.contains("VFO_DOWN=1\n"));
        assert!(rig.contains("PTT=0\n"));
        let rot = fs::read_to_string(dir.join("hamspot_Tower.rot")).unwrap();
        assert!(rot.contains("Host=127.0.0.1\n"));
        assert!(rot.contains("Port=4533\n"));
        assert!(rot.contains("MinAz=0\n"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_sync_keeps_type_and_a_new_port_changes_only_port() {
        let dir = scratch();
        sync(&dir, &[radio(1, "FT-450D", Some(4532), true)], &[]).unwrap();
        let path = dir.join("hamspot_FT-450D.rig");
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("Type=3", "Type=0");
        fs::write(&path, &edited).unwrap();
        sync(&dir, &[radio(1, "FT-450D", Some(4532), true)], &[]).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        sync(&dir, &[radio(1, "FT-450D", Some(4534), true)], &[]).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("Type=0\n"));
        assert!(after.contains("Port=4534\n"));
        assert!(!after.contains("Port=4532"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_manual_file_is_left_byte_for_byte() {
        let dir = scratch();
        let manual = dir.join("MyRig.rig");
        let bytes = b"[Radio]\nHost=10.0.0.1\nPort=1\nType=9\n";
        fs::write(&manual, bytes).unwrap();
        sync(&dir, &[radio(1, "Old", Some(4532), true)], &[]).unwrap();
        assert_eq!(fs::read(&manual).unwrap(), bytes);
        sync(&dir, &[radio(1, "New", Some(4532), true)], &[]).unwrap();
        assert_eq!(fs::read(&manual).unwrap(), bytes);
        sync(&dir, &[], &[]).unwrap();
        assert_eq!(fs::read(&manual).unwrap(), bytes);
        assert!(!dir.join("hamspot_Old.rig").exists());
        assert!(!dir.join("hamspot_New.rig").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_moves_the_owned_file_and_remove_deletes_it() {
        let dir = scratch();
        sync(&dir, &[radio(1, "Old", Some(4532), true)], &[]).unwrap();
        let old = dir.join("hamspot_Old.rig");
        let edited = fs::read_to_string(&old)
            .unwrap()
            .replace("VFO_UP=2", "VFO_UP=4");
        fs::write(&old, &edited).unwrap();
        sync(&dir, &[radio(1, "New", Some(4532), true)], &[]).unwrap();
        assert!(!old.exists());
        let moved = fs::read_to_string(dir.join("hamspot_New.rig")).unwrap();
        assert!(moved.contains("VFO_UP=4\n"));
        assert!(moved.contains("Port=4532\n"));
        sync(&dir, &[], &[]).unwrap();
        assert!(!dir.join("hamspot_New.rig").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn half_duplex_without_vfo_keys_becomes_duplex() {
        let dir = scratch();
        let path = dir.join("hamspot_Dummy.rig");
        fs::write(
            &path,
            "[Radio]\nHost=127.0.0.1\nPort=4534\nLO=0\nLO_UP=0\nType=2\nPTT=0\nSIGNAL_AOS=false\nSIGNAL_LOS=false\n",
        )
        .unwrap();
        sync(&dir, &[radio(1, "Dummy", Some(4534), true)], &[]).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("Type=3\n"));
        assert!(text.contains("VFO_UP=2\n"));
        assert!(text.contains("VFO_DOWN=1\n"));
        assert!(text.contains("PTT=0\n"));
        let kept = "[Radio]\nHost=127.0.0.1\nPort=4534\nType=2\nVFO_UP=1\nVFO_DOWN=2\n";
        fs::write(&path, kept).unwrap();
        sync(&dir, &[radio(1, "Dummy", Some(4534), true)], &[]).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), kept);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_orphan_is_deleted_and_gqrx_or_a_missing_port_writes_nothing() {
        let dir = scratch();
        fs::write(dir.join("hamspot_Gone.rig"), template(Kind::Radio, 9)).unwrap();
        sync(
            &dir,
            &[
                radio(1, "Remote", None, true),
                radio(2, "GQRX", Some(7356), false),
            ],
            &[rot(3, "Tower", None)],
        )
        .unwrap();
        let left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(left.is_empty(), "{left:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sanitized_name_cannot_escape_hwconf() {
        let root = scratch();
        let dir = root.join("hwconf");
        fs::create_dir_all(&dir).unwrap();
        sync(
            &dir,
            &[
                radio(1, "/tmp/evil", Some(4532), true),
                radio(2, "../../etc/passwd", Some(4533), true),
                radio(3, "..", Some(4534), true),
            ],
            &[],
        )
        .unwrap();
        let mut names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "hamspot_etcpasswd.rig".to_string(),
                "hamspot_tmpevil.rig".to_string()
            ]
        );
        assert!(!root.join("hamspot_tmpevil.rig").exists());
        assert!(!root.join("etc").join("passwd").exists());
        let canon = dir.canonicalize().unwrap();
        for name in &names {
            let path = dir.join(name).canonicalize().unwrap();
            assert_eq!(path.parent().unwrap(), canon.as_path());
        }
        let _ = fs::remove_dir_all(&root);
    }
}
