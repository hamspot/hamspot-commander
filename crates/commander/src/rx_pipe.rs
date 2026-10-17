//! Receiver stdin/stdout pipe. Issue #39.
//!
//! `pw-cat` records the radio's RX sink at 48 kHz mono s16le and writes it to
//! the operator's command. The command's stdout is played to the speaker the
//! RX stream was using. The caller unlinks that direct wire only after this
//! process group is still alive, and puts the wire back when the group ends.

use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Sample rate, channel count, and format on both pipes.
pub const FORMAT: &str = "48 kHz mono s16le";

pub struct Running {
    leader: Child,
    filter: Child,
    play: Child,
    pub command: String,
    pub output: String,
    pub diverted: bool,
}

impl Running {
    pub fn leader_id(&self) -> u32 {
        self.leader.id()
    }
}

/// First word of a command line. Quotes around that word are stripped.
pub fn program_name(command: &str) -> Option<&str> {
    let token = command.trim().split_whitespace().next()?;
    let token = token.trim_matches('"').trim_matches('\'');
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

pub fn on_path(program: &str) -> bool {
    let path = Path::new(program);
    if program.contains('/') {
        return is_exec(path);
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| is_exec(&dir.join(program)))
}

fn is_exec(path: &Path) -> bool {
    match path.metadata() {
        Ok(meta) => meta.is_file() && meta.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

/// One `*.rxpipe` script the Audio frame can start.
#[derive(Clone, Debug, PartialEq)]
pub struct ModemScript {
    /// File name, including `.rxpipe`.
    pub name: String,
    /// Command passed to the pipe. This is the script path.
    pub command: String,
}

/// Label in the dropdown and on the patch-bay block.
pub fn modem_label(name: &str) -> &str {
    name.strip_suffix(".rxpipe").unwrap_or(name)
}

/// Folders that hold RX pipe scripts, first match wins for a repeated name.
/// The install copy is `~/.local/share/hamspot-commander/modems`. The source
/// tree is included so a checkout works before install.
pub fn modem_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data) = data_home() {
        dirs.push(data.join("hamspot-commander/modems"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join("modems"));
            // The test executable lives in `target/<profile>/deps`.
            dirs.push(dir.join("../modems"));
        }
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../modems"));
    dirs.push(PathBuf::from("modems"));
    dirs
}

fn data_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(path) if !path.is_empty() => Some(PathBuf::from(path)),
        _ => std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")),
    }
}

pub fn modem_scripts() -> Vec<ModemScript> {
    scripts_in(&modem_dirs())
}

pub fn scripts_in(dirs: &[PathBuf]) -> Vec<ModemScript> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut found = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            // A directory named `foo.rxpipe` is not a script.
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.ends_with(".rxpipe") || !seen.insert(name.to_string()) {
                continue;
            }
            found.push(ModemScript {
                name: name.to_string(),
                command: path.to_string_lossy().into_owned(),
            });
        }
        found.sort_by(|left, right| left.name.cmp(&right.name));
        out.extend(found);
    }
    out
}

pub fn script_matches(command: &str, script: &ModemScript) -> bool {
    if command == script.command || command == script.name {
        return true;
    }
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        == Some(script.name.as_str())
}

/// Text for the closed dropdown. A saved command that is not one of the
/// scripts stays visible until another script is chosen.
pub fn choice_text(command: &str, scripts: &[ModemScript]) -> String {
    if let Some(script) = scripts.iter().find(|script| script_matches(command, script)) {
        return modem_label(&script.name).to_string();
    }
    if command.is_empty() {
        return "none".to_string();
    }
    let mut count = 0;
    for (index, _) in command.char_indices() {
        count += 1;
        if count > 32 {
            return format!("{}…", &command[..index]);
        }
    }
    command.to_string()
}

/// Record `hamspot_{id}_rx`, run `command`, play to `speaker`.
/// The three processes share the record process group.
pub fn spawn(id: u64, speaker: &str, command: &str) -> Result<Running, String> {
    let rx = crate::audio::rx_name(id);
    let mut record = Command::new("pw-cat");
    record
        .args([
            "-r",
            "--target",
            &rx,
            "--rate",
            "48000",
            "--channels",
            "1",
            "--channel-map",
            "MONO",
            "--format",
            "s16",
            "-a",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    let mut record = record
        .spawn()
        .map_err(|err| format!("pw-cat record: {err}"))?;
    let rec_out = record
        .stdout
        .take()
        .ok_or_else(|| "pw-cat record has no stdout".to_string())?;
    let group = i32::try_from(record.id()).map_err(|_| {
        stop_group(&mut record);
        "rx pipe process id does not fit".to_string()
    })?;

    let mut filter = Command::new("sh");
    filter
        .arg("-c")
        .arg(command)
        .stdin(Stdio::from(rec_out))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(group);
    let mut filter = match filter.spawn() {
        Ok(child) => child,
        Err(err) => {
            stop_group(&mut record);
            return Err(format!("rx pipe: {err}"));
        }
    };
    let Some(filt_out) = filter.stdout.take() else {
        stop_group(&mut record);
        stop_group(&mut filter);
        return Err("rx pipe has no stdout".into());
    };

    let mut play = Command::new("pw-cat");
    play.args([
        "-p",
        "--target",
        speaker,
        "--rate",
        "48000",
        "--channels",
        "1",
        "--channel-map",
        "MONO",
        "--format",
        "s16",
        "-a",
        "-",
    ])
    .stdin(Stdio::from(filt_out))
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .process_group(group);
    let play = match play.spawn() {
        Ok(child) => child,
        Err(err) => {
            stop_group(&mut record);
            stop_group(&mut filter);
            return Err(format!("pw-cat play: {err}"));
        }
    };
    Ok(Running {
        leader: record,
        filter,
        play,
        command: command.to_string(),
        output: speaker.to_string(),
        diverted: false,
    })
}

/// `None` while every process is still running. The first exited child wins
/// the note. `try_wait` keeps that status, so a later call still sees it.
pub fn poll(running: &mut Running) -> Option<String> {
    let leader = exited(&mut running.leader).map(|status| format!("record {status}"));
    let filter = exited(&mut running.filter).map(|status| format!("command {status}"));
    let play = exited(&mut running.play).map(|status| format!("play {status}"));
    leader.or(filter).or(play)
}

pub fn shutdown(mut running: Running) {
    // The recorder, the script, and the player share the recorder's group.
    // Reaping the recorder in `poll` must not skip the signal: `try_wait`
    // has already cached that exit, and the script's children are still
    // in the group.
    let group = running.leader.id() as i32;
    signal_group(group, libc::SIGTERM);
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let done = exited(&mut running.leader).is_some()
            && exited(&mut running.filter).is_some()
            && exited(&mut running.play).is_some();
        if done {
            return;
        }
        if Instant::now() >= deadline {
            signal_group(group, libc::SIGKILL);
            let _ = running.leader.wait();
            let _ = running.filter.wait();
            let _ = running.play.wait();
            return;
        }
        thread_sleep();
    }
}

fn signal_group(group: i32, signal: i32) {
    if group <= 1 {
        return;
    }
    unsafe {
        libc::kill(-group, signal);
    }
}

/// `stop_child` in cat.rs uses the same bound. A decoder that ignores
/// SIGTERM would otherwise sit in `wait` and stall the supervisor. `try_wait`
/// is the completion signal. The short sleep is only the gap between those
/// checks; `wait` has no deadline.
fn stop_group(child: &mut Child) {
    let pid = child.id() as i32;
    if pid <= 1 {
        return;
    }
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    let start = Instant::now();
    loop {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        if start.elapsed() > Duration::from_secs(1) {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.wait();
            return;
        }
        thread_sleep();
    }
}

fn thread_sleep() {
    std::thread::sleep(Duration::from_millis(20));
}

fn exited(child: &mut Child) -> Option<String> {
    match child.try_wait() {
        Ok(Some(status)) => Some(status.to_string()),
        Ok(None) => None,
        Err(err) if err.kind() == ErrorKind::WouldBlock => None,
        Err(err) => Some(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_name_is_the_first_word() {
        assert_eq!(program_name("  dsdccx -i - -o - "), Some("dsdccx"));
        assert_eq!(program_name("\"/usr/bin/cat\" -"), Some("/usr/bin/cat"));
        assert_eq!(program_name("   "), None);
    }

    #[test]
    fn on_path_rejects_a_missing_name() {
        assert!(!on_path("hamspot-rx-pipe-missing-bin"));
        assert!(on_path("sh"));
    }

    #[test]
    fn scripts_in_lists_rxpipe_files_and_keeps_the_first_name() {
        let root = std::env::temp_dir().join(format!(
            "hamspot-modems-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let first = root.join("install");
        let second = root.join("source");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("commander-demod-p25.rxpipe"), b"#!/bin/sh\n").unwrap();
        std::fs::write(first.join("notes.txt"), b"no").unwrap();
        std::fs::create_dir(first.join("not-a.rxpipe")).unwrap();
        std::fs::write(second.join("commander-demod-p25.rxpipe"), b"other\n").unwrap();
        std::fs::write(second.join("commander-demod-dmr.rxpipe"), b"dmr\n").unwrap();
        let scripts = scripts_in(&[first.clone(), second]);
        assert_eq!(scripts[0].name, "commander-demod-p25.rxpipe");
        assert!(scripts[0].command.contains("install"));
        assert_eq!(scripts[1].name, "commander-demod-dmr.rxpipe");
        assert!(scripts[1].command.contains("source"));
        assert!(script_matches(
            "commander-demod-p25.rxpipe",
            &scripts[0]
        ));
        assert_eq!(
            choice_text("commander-demod-p25.rxpipe", &scripts),
            "commander-demod-p25"
        );
        assert_eq!(choice_text("", &scripts), "none");
        let _ = std::fs::remove_dir_all(root);
    }
}
