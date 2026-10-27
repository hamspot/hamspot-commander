//! Receiver stdin/stdout pipe. Issue #39.
//!
//! `pw-cat` records the radio's RX sink at 48 kHz mono s16le and writes it to
//! the operator's command. The command's stdout is played to the speaker the
//! RX stream was using. The caller unlinks that direct wire only after this
//! process group is still alive, and puts the wire back when the group ends.
//!
//! The sink monitor is stereo (`monitor_FL` / `monitor_FR`). Asking `pw-cat`
//! for `MONO` does not attach to that monitor: the link lands on the machine's
//! mono capture, which here is the USB microphone. The recorder asks for one
//! FL channel, does not autoconnect, and is wired to `monitor_FL`.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::UdpSocket;
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Sample rate, channel count, and format on both pipes.
pub const FORMAT: &str = "48 kHz mono s16le";

pub struct Running {
    leader: Child,
    filter: Child,
    play: Child,
    pub command: String,
    /// Speaker the receive pipe replaced. Empty for a transmit pipe.
    pub output: String,
    /// Microphone the transmit pipe replaced. Empty for a receive pipe.
    pub source: String,
    pub diverted: bool,
    /// The raw RX stream has no speaker wire. A snapshot can miss the wire
    /// that was just created, so this is set from a live `pw-link` listing.
    pub raw_muted: bool,
    /// UDP port passed to a script. `None` for a shell command that does not
    /// take one. Released after the process group is gone.
    pub port: Option<u16>,
    /// Wires this pipe owns. The receive pipe records `monitor_FL`. The
    /// transmit pipe records the microphone and plays into `input.hamspot_*_tx`.
    pub links: Vec<(String, String)>,
    /// Set once every wire in `links` exists. A missing port is retried.
    pub tapped: bool,
    /// Decoder stderr is kept for the control-channel follower.
    pub events: bool,
    /// Decoder stderr is kept for the scanner glass. Never set with `events`.
    /// Those lines are not grants and do not tune.
    pub identity: bool,
    /// `HAMSPOT_INVERT` was set. The script appends the one polarity flag
    /// this decoder implements for that mode. A change restarts the process.
    pub invert: bool,
    /// `false` writes silence to the speaker. The decoder is not restarted.
    voice: Option<Arc<AtomicBool>>,
    /// stderr lines from the decoder, oldest first. Capped so a stalled
    /// window cannot fill the pipe and stop `dsd-fme`.
    log: Option<Arc<Mutex<Vec<String>>>>,
}

/// Recorder node name. It must not start with `hamspot_`: those names are
/// recreated as linger cards.
pub fn tap_node(id: u64) -> String {
    format!("commander-rx-{id}")
}

pub fn tap_ports(id: u64) -> (String, String) {
    (
        format!("{}:monitor_FL", crate::audio::rx_name(id)),
        format!("{}:input_FL", tap_node(id)),
    )
}

/// Capture port a discriminator tap records. A node whose name or
/// description says mono has one channel. Everything else uses the left
/// channel, the same rule as the patch bay.
pub fn capture_channel(name: &str, description: &str) -> &'static str {
    let blob = format!("{name} {description}").to_ascii_lowercase();
    if blob.contains("mono") {
        "capture_MONO"
    } else {
        "capture_FL"
    }
}

/// Arguments for a record `pw-cat` that does not autoconnect. Stdout is
/// 48 kHz mono s16le. `channel_map` is `FL` or `MONO`.
pub fn routed_record_args(node: &str, channel_map: &str) -> Vec<String> {
    vec![
        "-v".into(),
        "-r".into(),
        "--target".into(),
        "0".into(),
        "-P".into(),
        format!("node.name={node}"),
        "--rate".into(),
        "48000".into(),
        "--channels".into(),
        "1".into(),
        "--channel-map".into(),
        channel_map.to_string(),
        "--format".into(),
        "s16".into(),
        "-a".into(),
        "-".into(),
    ]
}

/// Arguments for the receive-pipe recorder.
pub fn record_args(id: u64) -> Vec<String> {
    routed_record_args(&tap_node(id), "FL")
}

/// Play 48 kHz mono s16le to a speaker node. Stdin is the stream.
pub fn speaker_play_args(speaker: &str) -> Vec<String> {
    vec![
        "-p".into(),
        "--target".into(),
        speaker.to_string(),
        "--rate".into(),
        "48000".into(),
        "--channels".into(),
        "1".into(),
        "--channel-map".into(),
        "MONO".into(),
        "--format".into(),
        "s16".into(),
        "-a".into(),
        "-".into(),
    ]
}

/// Playback that does not autoconnect. Stdin is 48 kHz mono s16le.
pub fn routed_play_args(node: &str) -> Vec<String> {
    vec![
        "-v".into(),
        "-p".into(),
        "--target".into(),
        "0".into(),
        "-P".into(),
        format!("node.name={node}"),
        "--rate".into(),
        "48000".into(),
        "--channels".into(),
        "1".into(),
        "--channel-map".into(),
        "FL".into(),
        "--format".into(),
        "s16".into(),
        "-a".into(),
        "-".into(),
    ]
}

pub fn tx_record_node(id: u64) -> String {
    format!("commander-tx-in-{id}")
}

pub fn tx_play_node(id: u64) -> String {
    format!("commander-tx-out-{id}")
}

fn recorder_port(node: &str, channel_map: &str) -> String {
    let leaf = if channel_map == "MONO" {
        "input_MONO"
    } else {
        "input_FL"
    };
    format!("{node}:{leaf}")
}

fn player_port(node: &str) -> String {
    format!("{node}:output_FL")
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

/// File name without `.rxpipe`, for the patch-bay block.
pub fn modem_label(name: &str) -> &str {
    name.strip_suffix(".rxpipe")
        .or_else(|| name.strip_suffix(".txpipe"))
        .unwrap_or(name)
}

/// Configuration dropdown text. `commander-demod-p25.rxpipe` is shown as `p25`.
/// `commander-mod-am.txpipe` is shown as `am`.
pub fn pipeline_label(name: &str) -> &str {
    let bare = modem_label(name);
    let short = bare
        .strip_prefix("commander-demod-")
        .or_else(|| bare.strip_prefix("commander-mod-"))
        .unwrap_or(bare);
    if short.is_empty() {
        bare
    } else {
        short
    }
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

/// `*.txpipe` scripts in the same folders as the receive scripts.
pub fn tx_scripts() -> Vec<ModemScript> {
    scripts_suffixed(&modem_dirs(), ".txpipe")
}

pub fn scripts_in(dirs: &[PathBuf]) -> Vec<ModemScript> {
    scripts_suffixed(dirs, ".rxpipe")
}

pub fn scripts_suffixed(dirs: &[PathBuf], suffix: &str) -> Vec<ModemScript> {
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
            if !name.ends_with(suffix) || !seen.insert(name.to_string()) {
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
    if let Some(script) = scripts
        .iter()
        .find(|script| script_matches(command, script))
    {
        return pipeline_label(&script.name).to_string();
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

/// First UDP port handed to an RX pipe script. The P25 script's own default
/// is the same number, for a manual run. Each live pipe takes the next free one.
const VOICE_START: u16 = 23469;
const VOICE_TRIES: u16 = 64;

/// Ports a live pipe has reserved. The probe socket is dropped so the script
/// can bind, and the set keeps the next pipe from taking it during that gap.
static VOICE_PORTS: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

/// A script path gets `port` as its first argument. A shell command is started
/// unchanged, because it has no port argument to fill in.
pub fn command_argv(command: &str, port: u16) -> Vec<String> {
    let trimmed = command.trim();
    if script_takes_port(trimmed) {
        vec![trimmed.to_string(), port.to_string()]
    } else {
        vec!["sh".into(), "-c".into(), trimmed.to_string()]
    }
}

fn script_takes_port(command: &str) -> bool {
    let Some(program) = program_name(command) else {
        return false;
    };
    command == program && Path::new(program).is_absolute()
}

/// Next localhost UDP port at or above [`VOICE_START`] that this process can
/// bind. The reservation stays until [`release_voice_port`].
pub fn claim_voice_port() -> Result<u16, String> {
    let mut claimed = VOICE_PORTS.lock().expect("voice ports");
    let last = VOICE_START.saturating_add(VOICE_TRIES - 1);
    for port in VOICE_START..=last {
        if claimed.contains(&port) {
            continue;
        }
        // `0.0.0.0` fails when the port is already bound on localhost or on
        // the wildcard, which is where socat's `udp-listen` sits.
        if UdpSocket::bind(("0.0.0.0", port)).is_ok() {
            claimed.insert(port);
            return Ok(port);
        }
    }
    Err(format!(
        "no free UDP port from {VOICE_START} through {last}"
    ))
}

pub fn release_voice_port(port: u16) {
    VOICE_PORTS.lock().expect("voice ports").remove(&port);
}

/// A voice port reservation that lasts as long as this value. REC replay
/// keeps one for the whole playback, so a live decoder that starts mid-replay
/// takes the next port instead of the one the replay's socat listens on.
#[derive(Debug)]
pub struct VoicePort(u16);

impl VoicePort {
    pub fn claim() -> Result<Self, String> {
        claim_voice_port().map(VoicePort)
    }
    pub fn port(&self) -> u16 {
        self.0
    }
}

impl Drop for VoicePort {
    fn drop(&mut self) {
        release_voice_port(self.0);
    }
}

/// True when `command` is the P25 trunk receive script.
///
/// That script is `-ft`: Phase 1 control and Phase 2 voice in one process.
/// The trace script is the same pipe with a crash log and stays Phase 1.
/// Follow, the speaker gate, and decoder stderr stay on for both. A DMR
/// script, and the conventional Phase 2 vocoder, are not this.
pub fn is_p25_rx(command: &str) -> bool {
    matches!(
        Path::new(command)
            .file_name()
            .and_then(|name| name.to_str()),
        Some("commander-demod-p25.rxpipe" | "commander-demod-p25-trace.rxpipe")
    )
}

/// True when `command` is the DMR Tier III receive script.
pub fn is_dmr_rx(command: &str) -> bool {
    matches!(
        Path::new(command)
            .file_name()
            .and_then(|name| name.to_str()),
        Some("commander-demod-dmr.rxpipe")
    )
}

/// True when `command` is an NXDN Type-C / Type-D trunk script.
///
/// The conventional NXDN48 and NXDN96 vocoders are not this. They demodulate
/// and do not follow.
pub fn is_nxdn_trunk(command: &str) -> bool {
    matches!(
        Path::new(command)
            .file_name()
            .and_then(|name| name.to_str()),
        Some("commander-demod-nxdn48-trunk.rxpipe" | "commander-demod-nxdn96-trunk.rxpipe")
    )
}

/// P25, DMR, or NXDN trunk. Each keeps decoder stderr, the speaker gate,
/// and FOLLOW. A conventional vocoder is none of these.
pub fn is_trunk_rx(command: &str) -> bool {
    is_p25_rx(command) || is_dmr_rx(command) || is_nxdn_trunk(command)
}

/// Channel-vocoder scripts that can keep stderr for the scanner glass.
/// Trunk follow uses [`is_trunk_rx`] instead, and the two are not combined.
pub fn keeps_identity(command: &str) -> bool {
    matches!(
        Path::new(command)
            .file_name()
            .and_then(|name| name.to_str()),
        Some(
            "commander-demod-p25.rxpipe"
                | "commander-demod-p25-trace.rxpipe"
                | "commander-demod-dmr.rxpipe"
                | "commander-demod-dstar.rxpipe"
                | "commander-demod-ysf.rxpipe"
                | "commander-demod-nxdn48.rxpipe"
                | "commander-demod-nxdn96.rxpipe"
                | "commander-demod-dpmr.rxpipe"
                | "commander-demod-m17.rxpipe"
                | "commander-demod-p25p2.rxpipe"
                | "commander-demod-provoice.rxpipe"
                | "commander-demod-edacs.rxpipe"
                | "commander-demod-edacs-esk.rxpipe"
                | "commander-demod-edacs-344.rxpipe"
                | "commander-demod-edacs-434.rxpipe"
                | "commander-demod-edacs-ea.rxpipe"
                | "commander-demod-edacs-ea-esk.rxpipe"
                | "commander-demod-x2tdma.rxpipe"
                | "commander-demod-freedv1600.rxpipe"
                | "commander-demod-freedv700c.rxpipe"
                | "commander-demod-freedv700d.rxpipe"
                | "commander-demod-freedv700e.rxpipe"
                | "commander-demod-freedv800xa.rxpipe"
                | "commander-demod-freedv2400a.rxpipe"
                | "commander-demod-freedv2400b.rxpipe"
                | "commander-demod-drm.rxpipe"
        )
    )
}

/// The selected memory's invert flag applies to this running script.
/// P25, Phase 2, ProVoice, EDACS, X2-TDMA, D-STAR, Fusion, and M17 have
/// no polarity switch. X2-TDMA stays at the decoder default: `-xx` means
/// non-inverted, which is the opposite of this flag.
pub fn invert_applies(command: &str, kind: &str) -> bool {
    let name = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str());
    matches!(
        (name, kind),
        (Some("commander-demod-dmr.rxpipe"), "dmr")
            | (
                Some(
                    "commander-demod-nxdn48.rxpipe" | "commander-demod-nxdn48-trunk.rxpipe"
                ),
                "nxdn48"
            )
            | (
                Some(
                    "commander-demod-nxdn96.rxpipe" | "commander-demod-nxdn96-trunk.rxpipe"
                ),
                "nxdn96"
            )
            | (Some("commander-demod-dpmr.rxpipe"), "dpmr")
    )
}

/// Absolute path of the demod script for a channel vocoder.
///
/// An empty or unknown kind is analog and returns an empty command.
/// `Err` means the kind is real and that script is not installed. The caller
/// leaves the analog path up and shows the error.
pub fn vocoder_command(kind: &str) -> Result<String, String> {
    let file = match kind {
        "p25" => "commander-demod-p25.rxpipe",
        "p25p2" => "commander-demod-p25p2.rxpipe",
        "dmr" => "commander-demod-dmr.rxpipe",
        "dstar" => "commander-demod-dstar.rxpipe",
        "ysf" => "commander-demod-ysf.rxpipe",
        "nxdn48" => "commander-demod-nxdn48.rxpipe",
        "nxdn96" => "commander-demod-nxdn96.rxpipe",
        "dpmr" => "commander-demod-dpmr.rxpipe",
        "m17" => "commander-demod-m17.rxpipe",
        "provoice" => "commander-demod-provoice.rxpipe",
        "edacs" => "commander-demod-edacs.rxpipe",
        "edacs-esk" => "commander-demod-edacs-esk.rxpipe",
        "edacs-344" => "commander-demod-edacs-344.rxpipe",
        "edacs-434" => "commander-demod-edacs-434.rxpipe",
        "edacs-ea" => "commander-demod-edacs-ea.rxpipe",
        "edacs-ea-esk" => "commander-demod-edacs-ea-esk.rxpipe",
        "x2tdma" => "commander-demod-x2tdma.rxpipe",
        "freedv1600" => "commander-demod-freedv1600.rxpipe",
        "freedv700c" => "commander-demod-freedv700c.rxpipe",
        "freedv700d" => "commander-demod-freedv700d.rxpipe",
        "freedv700e" => "commander-demod-freedv700e.rxpipe",
        "freedv800xa" => "commander-demod-freedv800xa.rxpipe",
        "freedv2400a" => "commander-demod-freedv2400a.rxpipe",
        "freedv2400b" => "commander-demod-freedv2400b.rxpipe",
        "drm" => "commander-demod-drm.rxpipe",
        _ => return Ok(String::new()),
    };
    modem_scripts()
        .into_iter()
        .find(|script| script.name == file)
        .map(|script| script.command)
        .ok_or_else(|| format!("vocoder: {file} is not installed"))
}

/// Record the radio monitor, or `source` when this is an audio tap, run
/// `command`, and play to `speaker`. The three processes share the record
/// process group.
///
/// `source` is `(node, leaf)` with leaf `capture_FL` or `capture_MONO`.
/// `None` records `hamspot_{id}_rx:monitor_FL`.
///
/// `events` asks a trunk script to keep decoder stderr, and gates the speaker
/// until [`set_voice`] is set. `identity` keeps that stderr for the scanner
/// glass without the gate and without the hunt counter. The two are never
/// both set. `invert` is `HAMSPOT_INVERT=1`. The decoder process is the same
/// shape either way.
pub fn spawn(
    id: u64,
    speaker: &str,
    command: &str,
    events: bool,
    identity: bool,
    invert: bool,
    on_line: Option<Arc<dyn Fn() + Send + Sync>>,
    source: Option<(&str, &str)>,
    key_csv: Option<&str>,
) -> Result<Running, String> {
    let port = if script_takes_port(command.trim()) {
        Some(claim_voice_port()?)
    } else {
        None
    };
    match spawn_group(
        id, speaker, command, port, events, identity, invert, on_line, source, key_csv,
    ) {
        Ok(running) => Ok(running),
        Err(err) => {
            if let Some(port) = port {
                release_voice_port(port);
            }
            Err(err)
        }
    }
}

fn spawn_group(
    id: u64,
    speaker: &str,
    command: &str,
    port: Option<u16>,
    events: bool,
    identity: bool,
    invert: bool,
    on_line: Option<Arc<dyn Fn() + Send + Sync>>,
    source: Option<(&str, &str)>,
    key_csv: Option<&str>,
) -> Result<Running, String> {
    let args = record_args(id);
    let (tap_output, tap_input, source_text) = match source {
        Some((node, leaf)) => (
            format!("{node}:{leaf}"),
            format!("{}:input_FL", tap_node(id)),
            format!("{node}:{leaf}"),
        ),
        None => {
            let (output, input) = tap_ports(id);
            (output, input, String::new())
        }
    };
    let mut record = Command::new("pw-cat");
    record
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut record = record
        .spawn()
        .map_err(|err| format!("pw-cat record: {err}"))?;
    // The first "stream state" line is the signal that the input port exists.
    // Linking before that line races the port list and returns ENOENT.
    if let Err(err) = wait_stream_state(&mut record) {
        stop_group(&mut record);
        return Err(err);
    }
    let links = vec![(tap_output, tap_input)];
    let tapped = match link_all(&links) {
        Ok(done) => done,
        Err(err) => {
            stop_group(&mut record);
            return Err(err);
        }
    };
    let rec_out = record
        .stdout
        .take()
        .ok_or_else(|| "pw-cat record has no stdout".to_string())?;
    let group = i32::try_from(record.id()).map_err(|_| {
        stop_group(&mut record);
        "rx pipe process id does not fit".to_string()
    })?;

    let argv = command_argv(command, port.unwrap_or(VOICE_START));
    let keep_stderr = events || identity;
    let mut filter = Command::new(&argv[0]);
    filter
        .args(&argv[1..])
        .stdin(Stdio::from(rec_out))
        .stdout(Stdio::piped())
        .stderr(if keep_stderr {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .process_group(group);
    if events {
        // The script keeps dsd-fme stderr and starts the hunt counter only
        // when this is set. Identity uses a different variable so a vocoder
        // cannot feed the follower.
        filter.env("HAMSPOT_CC_EVENTS", "1");
    } else if identity {
        filter.env("HAMSPOT_IDENTITY", "1");
    }
    if invert {
        filter.env("HAMSPOT_INVERT", "1");
    }
    if let Some(path) = key_csv.map(str::trim).filter(|path| !path.is_empty()) {
        // dsd-fme -K. The P25 scripts read this. Other scripts ignore it.
        // A hamlib trunk pipe is also the control decoder, so a later key
        // edit does not restart it. The next start of this pipe picks it up.
        filter.env("HAMSPOT_P25_K", path);
    }
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
    let (play_in, voice, log) = if events {
        let (play_side, gate_side) = match UnixStream::pair() {
            Ok(pair) => pair,
            Err(err) => {
                stop_group(&mut record);
                stop_group(&mut filter);
                return Err(format!("rx pipe gate: {err}"));
            }
        };
        let voice = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&voice);
        thread::spawn(move || gate_voice(filt_out, gate_side, flag));
        let log = Arc::new(Mutex::new(Vec::new()));
        if let Some(stderr) = filter.stderr.take() {
            let sink = Arc::clone(&log);
            thread::spawn(move || collect_events(stderr, sink, on_line));
        }
        (Stdio::from(OwnedFd::from(play_side)), Some(voice), Some(log))
    } else if identity {
        // Same stderr capture as a trunk pipe. The speaker is not gated,
        // and the lines go to the identity log rather than the follower.
        let log = Arc::new(Mutex::new(Vec::new()));
        if let Some(stderr) = filter.stderr.take() {
            let sink = Arc::clone(&log);
            thread::spawn(move || collect_events(stderr, sink, on_line));
        }
        (Stdio::from(filt_out), None, Some(log))
    } else {
        (Stdio::from(filt_out), None, None)
    };

    let mut play = Command::new("pw-cat");
    play.args(speaker_play_args(speaker))
    .stdin(play_in)
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
        source: source_text,
        diverted: false,
        raw_muted: false,
        port,
        links,
        tapped,
        events,
        identity: identity && !events,
        invert,
        voice,
        log,
    })
}

/// Write decoder audio through, or the same number of zero samples.
///
/// The player stays running during a control-channel watch. Stopping it
/// would look like a dead pipe and restart `dsd-fme`, which drops the
/// identifier table the next grant needs.
fn gate_voice(mut audio: impl Read, mut play: UnixStream, voice: Arc<AtomicBool>) {
    let mut buf = [0u8; 4096];
    let zeros = [0u8; 4096];
    loop {
        let read = match audio.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let chunk = if voice.load(Ordering::Relaxed) {
            &buf[..read]
        } else {
            &zeros[..read]
        };
        if play.write_all(chunk).is_err() {
            return;
        }
    }
}

/// Keep at most this many unread decoder lines. Past that, the oldest go.
/// The cap is the bound on stderr memory, not a timer.
const EVENT_CAP: usize = 400;

fn collect_events(
    stderr: impl Read,
    sink: Arc<Mutex<Vec<String>>>,
    on_line: Option<Arc<dyn Fn() + Send + Sync>>,
) {
    let reader = BufReader::new(stderr);
    for line in reader.lines() {
        let Ok(line) = line else {
            return;
        };
        if crate::keys::is_key_dump(&line) {
            continue;
        }
        let mut queue = sink.lock().expect("cc events");
        if queue.len() >= EVENT_CAP {
            let drop_n = queue.len() - EVENT_CAP + 1;
            queue.drain(0..drop_n);
        }
        queue.push(line);
        drop(queue);
        if let Some(poke) = &on_line {
            poke();
        }
    }
}

/// Lines the decoder wrote since the last drain.
pub fn drain_events(running: &Running) -> Vec<String> {
    let Some(log) = &running.log else {
        return Vec::new();
    };
    std::mem::take(&mut *log.lock().expect("cc events"))
}

/// `true` plays the decoder. `false` plays silence. The process stays up.
pub fn set_voice(running: &Running, on: bool) {
    if let Some(flag) = &running.voice {
        flag.store(on, Ordering::Relaxed);
    }
}

/// Record `capture_port`, run `command`, and play into each transmit input.
/// `channel_map` matches `capture_port` (`FL` or `MONO`). The script still
/// sees 48 kHz mono s16le. Nothing here is autoconnected: a mono playback
/// targeted at the transmit node can miss `input_FL` and land on a speaker.
pub fn spawn_tx(
    id: u64,
    source: &str,
    capture_port: &str,
    channel_map: &str,
    play_ports: &[String],
    command: &str,
) -> Result<Running, String> {
    if play_ports.is_empty() {
        return Err("tx pipe has no transmit input".into());
    }
    let port = if script_takes_port(command.trim()) {
        Some(claim_voice_port()?)
    } else {
        None
    };
    match spawn_tx_group(
        id,
        source,
        capture_port,
        channel_map,
        play_ports,
        command,
        port,
    ) {
        Ok(running) => Ok(running),
        Err(err) => {
            if let Some(port) = port {
                release_voice_port(port);
            }
            Err(err)
        }
    }
}

fn spawn_tx_group(
    id: u64,
    source: &str,
    capture_port: &str,
    channel_map: &str,
    play_ports: &[String],
    command: &str,
    port: Option<u16>,
) -> Result<Running, String> {
    let record_node = tx_record_node(id);
    let play_node = tx_play_node(id);
    let mut record = Command::new("pw-cat");
    record
        .args(routed_record_args(&record_node, channel_map))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut record = record
        .spawn()
        .map_err(|err| format!("pw-cat record: {err}"))?;
    if let Err(err) = wait_stream_state(&mut record) {
        stop_group(&mut record);
        return Err(err);
    }
    let rec_out = record
        .stdout
        .take()
        .ok_or_else(|| "pw-cat record has no stdout".to_string())?;
    let group = i32::try_from(record.id()).map_err(|_| {
        stop_group(&mut record);
        "tx pipe process id does not fit".to_string()
    })?;
    let argv = command_argv(command, port.unwrap_or(VOICE_START));
    let mut filter = Command::new(&argv[0]);
    filter
        .args(&argv[1..])
        .stdin(Stdio::from(rec_out))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(group);
    let mut filter = match filter.spawn() {
        Ok(child) => child,
        Err(err) => {
            stop_group(&mut record);
            return Err(format!("tx pipe: {err}"));
        }
    };
    let Some(filt_out) = filter.stdout.take() else {
        stop_group(&mut record);
        stop_group(&mut filter);
        return Err("tx pipe has no stdout".into());
    };
    let mut play = Command::new("pw-cat");
    play.args(routed_play_args(&play_node))
        .stdin(Stdio::from(filt_out))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(group);
    let mut play = match play.spawn() {
        Ok(child) => child,
        Err(err) => {
            stop_group(&mut record);
            stop_group(&mut filter);
            return Err(format!("pw-cat play: {err}"));
        }
    };
    if let Err(err) = wait_stream_state(&mut play) {
        stop_group(&mut record);
        stop_group(&mut filter);
        stop_group(&mut play);
        return Err(err);
    }
    let mut links = vec![(
        capture_port.to_string(),
        recorder_port(&record_node, channel_map),
    )];
    let played = player_port(&play_node);
    for input in play_ports {
        links.push((played.clone(), input.clone()));
    }
    let tapped = match link_all(&links) {
        Ok(done) => done,
        Err(err) => {
            stop_group(&mut record);
            stop_group(&mut filter);
            stop_group(&mut play);
            return Err(err);
        }
    };
    Ok(Running {
        leader: record,
        filter,
        play,
        command: command.to_string(),
        output: String::new(),
        source: source.to_string(),
        diverted: false,
        raw_muted: false,
        port,
        links,
        tapped,
        events: false,
        identity: false,
        invert: false,
        voice: None,
        log: None,
    })
}

/// `Ok(true)` when every wire is up. `Ok(false)` when a port is not in the
/// graph yet. A different `pw-link` failure is returned.
fn link_all(links: &[(String, String)]) -> Result<bool, String> {
    for (output, input) in links {
        match crate::audio::pw_link(output, input) {
            Ok(()) => {}
            Err(err) if crate::audio::missing_port(&err) => return Ok(false),
            Err(err) => return Err(err),
        }
    }
    Ok(true)
}

/// Retry wires that were missing when the process group started.
pub fn ensure_tap(running: &mut Running) -> Result<(), String> {
    if running.tapped {
        return Ok(());
    }
    running.tapped = link_all(&running.links)?;
    Ok(())
}

/// Read verbose `pw-cat` until it reports a stream state, then keep draining
/// stderr so a full pipe cannot stall the recorder.
fn wait_stream_state(child: &mut Child) -> Result<(), String> {
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "pw-cat record has no stderr".to_string())?;
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|err| format!("pw-cat record: {err}"))?;
        if read == 0 {
            let status = child.try_wait().ok().flatten();
            return Err(format!(
                "pw-cat record closed its log before it connected ({status:?})"
            ));
        }
        if line.contains("stream state") {
            thread::spawn(move || {
                let mut rest = Vec::new();
                let _ = reader.read_to_end(&mut rest);
            });
            return Ok(());
        }
    }
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
    fn record_args_take_the_left_monitor_and_stay_mono() {
        let args = record_args(3);
        let has = |left: &str, right: &str| {
            args.windows(2)
                .any(|pair| pair[0] == left && pair[1] == right)
        };
        assert!(has("--target", "0"));
        assert!(has("--channel-map", "FL"));
        assert!(has("--channels", "1"));
        assert!(!args.iter().any(|arg| arg == "MONO"));
        assert!(args.iter().any(|arg| arg == "node.name=commander-rx-3"));
        assert!(!tap_node(3).starts_with("hamspot_"));
        assert!(!tx_record_node(3).starts_with("hamspot_"));
        assert!(!tx_play_node(3).starts_with("hamspot_"));
        let speaker = speaker_play_args("bluez_output.example");
        assert!(speaker.windows(2).any(|pair| {
            pair[0] == "--target" && pair[1] == "bluez_output.example"
        }));
        assert!(speaker
            .windows(2)
            .any(|pair| pair[0] == "--channel-map" && pair[1] == "MONO"));
        assert!(!speaker.iter().any(|arg| arg.starts_with("node.name=")));
        assert_eq!(pipeline_label("commander-mod-am.txpipe"), "am");
        assert_eq!(
            tap_ports(3),
            (
                "hamspot_3_rx:monitor_FL".to_string(),
                "commander-rx-3:input_FL".to_string(),
            )
        );
    }

    #[test]
    fn program_name_is_the_first_word() {
        assert_eq!(program_name("  dsdccx -i - -o - "), Some("dsdccx"));
        assert_eq!(program_name("\"/usr/bin/cat\" -"), Some("/usr/bin/cat"));
        assert_eq!(program_name("   "), None);
    }

    #[test]
    fn a_mono_capture_is_the_one_channel_and_a_line_uses_the_left() {
        assert_eq!(capture_channel("alsa_input.usb.mono-fallback", ""), "capture_MONO");
        assert_eq!(capture_channel("line", "USB Mono"), "capture_MONO");
        assert_eq!(capture_channel("line-in", "Analog Stereo"), "capture_FL");
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
        assert!(script_matches("commander-demod-p25.rxpipe", &scripts[0]));
        assert_eq!(choice_text("commander-demod-p25.rxpipe", &scripts), "p25");
        assert_eq!(pipeline_label("commander-demod-dmr.rxpipe"), "dmr");
        assert!(is_p25_rx(
            "/opt/modems/commander-demod-p25-trace.rxpipe"
        ));
        assert!(is_p25_rx("commander-demod-p25.rxpipe"));
        assert!(!is_p25_rx("commander-demod-dmr.rxpipe"));
        assert!(is_dmr_rx("commander-demod-dmr.rxpipe"));
        assert!(is_dmr_rx("/opt/modems/commander-demod-dmr.rxpipe"));
        assert!(!is_dmr_rx("commander-demod-p25.rxpipe"));
        assert!(!is_dmr_rx("commander-demod-p25-trace.rxpipe"));
        assert!(is_trunk_rx("commander-demod-p25.rxpipe"));
        assert!(is_trunk_rx("commander-demod-dmr.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-fm.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-dstar.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-ysf.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-nxdn48.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-nxdn96.rxpipe"));
        assert!(is_nxdn_trunk("commander-demod-nxdn48-trunk.rxpipe"));
        assert!(is_nxdn_trunk(
            "/opt/modems/commander-demod-nxdn96-trunk.rxpipe"
        ));
        assert!(!is_nxdn_trunk("commander-demod-nxdn96.rxpipe"));
        assert!(is_trunk_rx("commander-demod-nxdn48-trunk.rxpipe"));
        assert!(is_trunk_rx("commander-demod-nxdn96-trunk.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-dpmr.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-m17.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-p25p2.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-provoice.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-edacs.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-edacs-esk.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-edacs-344.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-edacs-434.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-edacs-ea.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-edacs-ea-esk.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-x2tdma.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-freedv700d.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-freedv2400a.rxpipe"));
        assert!(!is_trunk_rx("commander-demod-drm.rxpipe"));
        assert!(keeps_identity("commander-demod-dstar.rxpipe"));
        assert!(keeps_identity("commander-demod-dmr.rxpipe"));
        assert!(keeps_identity("commander-demod-p25p2.rxpipe"));
        assert!(keeps_identity("commander-demod-provoice.rxpipe"));
        assert!(keeps_identity("commander-demod-edacs-ea-esk.rxpipe"));
        assert!(keeps_identity("commander-demod-x2tdma.rxpipe"));
        assert!(keeps_identity("commander-demod-freedv1600.rxpipe"));
        assert!(keeps_identity("commander-demod-freedv2400b.rxpipe"));
        assert!(keeps_identity("commander-demod-drm.rxpipe"));
        assert!(!keeps_identity("commander-demod-nxdn48-trunk.rxpipe"));
        assert!(invert_applies("commander-demod-dmr.rxpipe", "dmr"));
        assert!(invert_applies(
            "commander-demod-nxdn48-trunk.rxpipe",
            "nxdn48"
        ));
        assert!(invert_applies("commander-demod-nxdn96.rxpipe", "nxdn96"));
        assert!(invert_applies("commander-demod-dpmr.rxpipe", "dpmr"));
        assert!(!invert_applies("commander-demod-p25.rxpipe", "p25"));
        assert!(!invert_applies("commander-demod-p25p2.rxpipe", "p25p2"));
        assert!(!invert_applies("commander-demod-provoice.rxpipe", "provoice"));
        assert!(!invert_applies("commander-demod-edacs.rxpipe", "edacs"));
        assert!(!invert_applies("commander-demod-x2tdma.rxpipe", "x2tdma"));
        assert!(!invert_applies("commander-demod-freedv700e.rxpipe", "freedv700e"));
        assert!(!invert_applies("commander-demod-drm.rxpipe", "drm"));
        assert!(!invert_applies("commander-demod-dstar.rxpipe", "dstar"));
        assert!(!invert_applies("commander-demod-ysf.rxpipe", "ysf"));
        assert!(!invert_applies("commander-demod-m17.rxpipe", "m17"));
        assert!(!invert_applies("commander-demod-dmr.rxpipe", "nxdn48"));
        assert!(!invert_applies(
            "commander-demod-nxdn48-trunk.rxpipe",
            "nxdn96"
        ));
        assert_eq!(vocoder_command("").unwrap(), "");
        assert_eq!(vocoder_command("nxdn").unwrap(), "");
        assert_eq!(
            pipeline_label("commander-demod-p25-trace.rxpipe"),
            "p25-trace"
        );
        assert_eq!(pipeline_label("custom.rxpipe"), "custom");
        assert_eq!(choice_text("", &scripts), "none");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_channel_vocoder_resolves_to_the_demod_script() {
        let p25 = vocoder_command("p25").expect("p25 script");
        assert!(p25.ends_with("commander-demod-p25.rxpipe"), "{p25}");
        assert!(!p25.contains("trace"), "{p25}");
        let dmr = vocoder_command("dmr").expect("dmr script");
        assert!(dmr.ends_with("commander-demod-dmr.rxpipe"), "{dmr}");
        for (kind, file) in [
            ("dstar", "commander-demod-dstar.rxpipe"),
            ("ysf", "commander-demod-ysf.rxpipe"),
            ("nxdn48", "commander-demod-nxdn48.rxpipe"),
            ("nxdn96", "commander-demod-nxdn96.rxpipe"),
            ("dpmr", "commander-demod-dpmr.rxpipe"),
            ("m17", "commander-demod-m17.rxpipe"),
            ("p25p2", "commander-demod-p25p2.rxpipe"),
            ("provoice", "commander-demod-provoice.rxpipe"),
            ("edacs", "commander-demod-edacs.rxpipe"),
            ("edacs-esk", "commander-demod-edacs-esk.rxpipe"),
            ("edacs-344", "commander-demod-edacs-344.rxpipe"),
            ("edacs-434", "commander-demod-edacs-434.rxpipe"),
            ("edacs-ea", "commander-demod-edacs-ea.rxpipe"),
            ("edacs-ea-esk", "commander-demod-edacs-ea-esk.rxpipe"),
            ("x2tdma", "commander-demod-x2tdma.rxpipe"),
            ("freedv1600", "commander-demod-freedv1600.rxpipe"),
            ("freedv700c", "commander-demod-freedv700c.rxpipe"),
            ("freedv700d", "commander-demod-freedv700d.rxpipe"),
            ("freedv700e", "commander-demod-freedv700e.rxpipe"),
            ("freedv800xa", "commander-demod-freedv800xa.rxpipe"),
            ("freedv2400a", "commander-demod-freedv2400a.rxpipe"),
            ("freedv2400b", "commander-demod-freedv2400b.rxpipe"),
            ("drm", "commander-demod-drm.rxpipe"),
        ] {
            let path = vocoder_command(kind).unwrap_or_else(|err| panic!("{kind}: {err}"));
            assert!(path.ends_with(file), "{path}");
            assert!(!is_trunk_rx(&path), "{path}");
        }
    }

    #[test]
    fn a_script_path_receives_its_own_udp_port() {
        assert_eq!(
            command_argv("/opt/modems/commander-demod-p25.rxpipe", 23470),
            vec![
                "/opt/modems/commander-demod-p25.rxpipe".to_string(),
                "23470".to_string()
            ]
        );
        assert_eq!(
            command_argv("dsd-fme -f1 -i -", 23470),
            vec![
                "sh".to_string(),
                "-c".to_string(),
                "dsd-fme -f1 -i -".to_string()
            ]
        );
        let first = claim_voice_port().expect("first voice port");
        let second = claim_voice_port().expect("second voice port");
        assert_ne!(first, second);
        let occupied = first.saturating_add(1);
        let held = UdpSocket::bind(("0.0.0.0", occupied)).ok();
        if held.is_some() {
            let third = claim_voice_port().expect("third voice port");
            assert_ne!(third, first);
            assert_ne!(third, second);
            assert_ne!(third, occupied);
            release_voice_port(third);
        }
        release_voice_port(first);
        release_voice_port(second);
    }

    #[test]
    fn held_voice_port_is_not_reused_until_dropped() {
        let held = VoicePort::claim().expect("replay port");
        let port = held.port();
        for _ in 0..3 {
            let other = claim_voice_port().expect("live port");
            assert_ne!(other, port);
            release_voice_port(other);
        }
        assert!(VOICE_PORTS.lock().unwrap().contains(&port));
        drop(held);
        assert!(!VOICE_PORTS.lock().unwrap().contains(&port));
    }
}
