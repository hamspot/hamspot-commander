//! Virtual PipeWire cards for the radios, and the graph other tools leave behind.
//!
//! A card is two `pw-loopback` nodes. RX is a sink the radio plays into. TX is
//! a source the radio captures. Nothing here reconnects a wire after another
//! process moves it. `pw-dump -m` is the change signal. Its first line is the
//! whole graph. Later lines list only the objects that changed, and `info:
//! null` removes an id. Those lines are merged; a changed line is not a new graph.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Devices and wires stored in the station file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioGraph {
    #[serde(default)]
    pub devices: Vec<AudioNode>,
    #[serde(default)]
    pub links: Vec<AudioLink>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioNode {
    pub name: String,
    pub description: String,
    pub class: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioLink {
    /// Output port, `node.name:port.name`.
    pub output: String,
    /// Input port, `node.name:port.name`.
    pub input: String,
}

/// One `pw-dump` snapshot reduced to what the window shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioLook {
    pub graph: AudioGraph,
    pub playback: Vec<AudioNode>,
    pub capture: Vec<AudioNode>,
    /// Connectable port names keyed by `node.name`. Monitor taps are left out.
    pub connect_ports: BTreeMap<String, Vec<String>>,
}

/// `{Make} {Model} {id} RX` or the TX form. This is the name a person sees.
pub fn describe(make: &str, model: &str, id: u64, end: &str) -> String {
    format!("{make} {model} {id} {end}")
}

pub fn rx_name(id: u64) -> String {
    format!("hamspot_{id}_rx")
}

pub fn tx_name(id: u64) -> String {
    format!("hamspot_{id}_tx")
}

pub fn rx_stream(id: u64) -> String {
    format!("output.hamspot_{id}_rx")
}

pub fn tx_stream(id: u64) -> String {
    format!("input.hamspot_{id}_tx")
}

/// `pw-loopback` arguments for the receiver sink. The playback side does not
/// pick a target; Commander links that once, and a later outside change stays.
pub fn rx_loopback_args(id: u64, description: &str) -> Vec<String> {
    vec![
        "-n".into(),
        rx_name(id),
        "-i".into(),
        props(&[
            ("media.class", "Audio/Sink"),
            ("node.name", &rx_name(id)),
            ("node.description", description),
            ("device.description", description),
            ("node.autoconnect", "false"),
            ("node.dont-reconnect", "true"),
            ("node.passive", "true"),
        ]),
        "-o".into(),
        passive_props(),
    ]
}

/// `pw-loopback` arguments for the transmit source.
pub fn tx_loopback_args(id: u64, description: &str) -> Vec<String> {
    vec![
        "-n".into(),
        tx_name(id),
        "-o".into(),
        props(&[
            ("media.class", "Audio/Source"),
            ("node.name", &tx_name(id)),
            ("node.description", description),
            ("device.description", description),
            ("node.autoconnect", "false"),
            ("node.dont-reconnect", "true"),
            ("node.passive", "true"),
        ]),
        "-i".into(),
        passive_props(),
    ]
}

fn passive_props() -> String {
    props(&[
        ("node.autoconnect", "false"),
        ("node.dont-reconnect", "true"),
        ("node.passive", "true"),
    ])
}

fn props(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| {
            let value = value.replace('"', "'");
            if value.chars().any(char::is_whitespace) {
                format!("{key}=\"{value}\"")
            } else {
                format!("{key}={value}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn canonical(graph: &AudioGraph) -> String {
    let mut graph = graph.clone();
    graph.devices.sort_by(|a, b| a.name.cmp(&b.name));
    graph.devices.dedup_by(|a, b| a.name == b.name);
    graph
        .links
        .sort_by(|a, b| (&a.output, &a.input).cmp(&(&b.output, &b.input)));
    graph.links.dedup();
    serde_json::to_string(&graph).unwrap_or_default()
}

/// A node Commander watches. Stream nodes carry the wires of a `hamspot_` card.
pub fn watched_node(name: &str) -> bool {
    name.starts_with("hamspot_")
        || name.starts_with("input.hamspot_")
        || name.starts_with("output.hamspot_")
}

pub fn port_node(port: &str) -> &str {
    port.split_once(':').map(|(node, _)| node).unwrap_or(port)
}

/// Running `pw-dump -m` document. The first line fills it. Later lines update it.
#[derive(Clone, Debug, Default)]
pub struct DumpMerge {
    objects: BTreeMap<u64, Value>,
}

impl DumpMerge {
    /// Fold one monitor line into the graph Commander shows.
    pub fn apply(&mut self, text: &str) -> Option<AudioLook> {
        let items: Vec<Value> = serde_json::from_str(text).ok()?;
        for item in items {
            let Some(id) = item.get("id").and_then(Value::as_u64) else {
                continue;
            };
            let removed = item.get("info").is_some_and(Value::is_null);
            if removed {
                self.objects.remove(&id);
            } else {
                self.objects.insert(id, item);
            }
        }
        let all: Vec<Value> = self.objects.values().cloned().collect();
        let text = serde_json::to_string(&all).ok()?;
        graph_from_dump(&text)
    }
}

/// Identity of the hamspot graph and the hardware menus. Meter updates that
/// do not change a node name or a wire do not publish a new snapshot.
pub fn look_key(look: &AudioLook) -> String {
    let mut key = canonical(&look.graph);
    key.push('\n');
    for node in look.playback.iter().chain(look.capture.iter()) {
        key.push_str(&node.name);
        key.push('\n');
        key.push_str(&node.description);
        key.push('\n');
        key.push_str(&node.class);
        key.push('\n');
    }
    for (name, ports) in &look.connect_ports {
        key.push_str(name);
        key.push('\n');
        for port in ports {
            key.push_str(port);
            key.push('\n');
        }
    }
    key
}

pub fn graph_from_dump(text: &str) -> Option<AudioLook> {
    let items: Vec<Value> = serde_json::from_str(text).ok()?;
    let mut names = BTreeMap::<u64, (String, String, String)>::new();
    for item in &items {
        if item.get("type").and_then(Value::as_str) != Some("PipeWire:Interface:Node") {
            continue;
        }
        let Some(id) = item.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let Some(props) = item.get("info").and_then(|info| info.get("props")) else {
            continue;
        };
        let Some(name) = props.get("node.name").and_then(Value::as_str) else {
            continue;
        };
        let description = props
            .get("node.description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let class = props
            .get("media.class")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        names.insert(id, (name.to_string(), description, class));
    }
    let mut ports = BTreeMap::<u64, String>::new();
    let mut connect_ports: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for item in &items {
        if item.get("type").and_then(Value::as_str) != Some("PipeWire:Interface:Port") {
            continue;
        }
        let Some(id) = item.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let Some(props) = item.get("info").and_then(|info| info.get("props")) else {
            continue;
        };
        let Some(port_name) = props.get("port.name").and_then(Value::as_str) else {
            continue;
        };
        let Some(node_id) = props.get("node.id").and_then(Value::as_u64) else {
            continue;
        };
        let Some((node_name, _, _)) = names.get(&node_id) else {
            continue;
        };
        ports.insert(id, format!("{node_name}:{port_name}"));
        if connectable_port(port_name) {
            connect_ports
                .entry(node_name.clone())
                .or_default()
                .insert(port_name.to_string());
        }
    }
    let mut devices = Vec::new();
    let mut playback = Vec::new();
    let mut capture = Vec::new();
    for (name, description, class) in names.values() {
        if name.starts_with("hamspot_") {
            devices.push(AudioNode {
                name: name.clone(),
                description: description.clone(),
                class: class.clone(),
            });
        } else if class == "Audio/Sink" && menu_device(name) {
            playback.push(AudioNode {
                name: name.clone(),
                description: description.clone(),
                class: class.clone(),
            });
        } else if class == "Audio/Source" && menu_device(name) {
            capture.push(AudioNode {
                name: name.clone(),
                description: description.clone(),
                class: class.clone(),
            });
        }
    }
    let mut links = Vec::new();
    for item in &items {
        if item.get("type").and_then(Value::as_str) != Some("PipeWire:Interface:Link") {
            continue;
        }
        let Some(info) = item.get("info") else {
            continue;
        };
        let Some(output_id) = info.get("output-port-id").and_then(Value::as_u64) else {
            continue;
        };
        let Some(input_id) = info.get("input-port-id").and_then(Value::as_u64) else {
            continue;
        };
        let Some(output) = ports.get(&output_id) else {
            continue;
        };
        let Some(input) = ports.get(&input_id) else {
            continue;
        };
        if watched_node(port_node(output)) || watched_node(port_node(input)) {
            links.push(AudioLink {
                output: output.clone(),
                input: input.clone(),
            });
        }
    }
    let graph = AudioGraph { devices, links };
    let graph = serde_json::from_str(&canonical(&graph)).unwrap_or(graph);
    playback.sort_by(|a, b| a.name.cmp(&b.name));
    capture.sort_by(|a, b| a.name.cmp(&b.name));
    let connect_ports = connect_ports
        .into_iter()
        .map(|(name, ports)| (name, ports.into_iter().collect()))
        .collect();
    Some(AudioLook {
        graph,
        playback,
        capture,
        connect_ports,
    })
}

fn connectable_port(name: &str) -> bool {
    name.starts_with("playback_")
        || name.starts_with("capture_")
        || name.starts_with("output_")
        || name.starts_with("input_")
}

fn menu_device(name: &str) -> bool {
    !name.starts_with("hamspot_") && !name.starts_with("input.") && !name.starts_with("output.")
}

/// The computer node on the other side of a radio stream, skipping hamspot nodes.
pub fn peer_node(graph: &AudioGraph, stream: &str, stream_is_output: bool) -> Option<String> {
    for link in &graph.links {
        let (ours, peer) = if stream_is_output {
            (&link.output, &link.input)
        } else {
            (&link.input, &link.output)
        };
        if port_node(ours) != stream {
            continue;
        }
        let peer = port_node(peer);
        if watched_node(peer) {
            continue;
        }
        return Some(peer.to_string());
    }
    None
}

pub fn has_device(graph: &AudioGraph, name: &str) -> bool {
    graph.devices.iter().any(|device| device.name == name)
}

/// The loopback stream has at least one port. A station file can restore
/// `hamspot_{id}_rx` as a lingering null-sink with the same name and no stream.
pub fn stream_ports_ready(output: bool, node: &str) -> bool {
    let Ok(ports) = pw_ports(output) else {
        return false;
    };
    !ports_of(&ports, node).is_empty()
}

/// `card_present` is the sink or source name. `stream_ready` is the paired
/// `output.hamspot_*` or `input.hamspot_*` node. A linger card occupies the
/// name the loopback has to bind.
pub fn linger_occupies(card_present: bool, stream_ready: bool) -> bool {
    card_present && !stream_ready
}

/// Remove one hamspot node so `pw-loopback` can take its name. A name that is
/// already gone is success. Anything other than a `hamspot_` node is refused.
pub fn destroy_named_node(name: &str) -> Result<(), String> {
    if !safe_hamspot_name(name) {
        return Err(format!("not removing {name}"));
    }
    let output = command_output(
        "pw-dump",
        &["-N".into(), "-R".into(), "-i".into(), "0".into()],
    )?;
    if !output.status.success() {
        return Err(format!(
            "pw-dump: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> =
        serde_json::from_str(&text).map_err(|err| format!("pw-dump: {err}"))?;
    for item in items {
        if item.get("type").and_then(serde_json::Value::as_str) != Some("PipeWire:Interface:Node")
        {
            continue;
        }
        let node_name = item
            .get("info")
            .and_then(|info| info.get("props"))
            .and_then(|props| props.get("node.name"))
            .and_then(serde_json::Value::as_str);
        if node_name != Some(name) {
            continue;
        }
        let Some(id) = item.get("id").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        let result = command_output("pw-cli", &["destroy".into(), id.to_string()])?;
        if !result.status.success() {
            return Err(format!(
                "pw-cli destroy: {}",
                String::from_utf8_lossy(&result.stderr).trim()
            ));
        }
    }
    Ok(())
}

/// A name Commander is willing to recreate. Anything else is only recorded.
pub fn safe_hamspot_name(name: &str) -> bool {
    name.starts_with("hamspot_")
        && name.len() > "hamspot_".len()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub fn linger_args(name: &str, description: &str, class: &str) -> Option<Vec<String>> {
    if !safe_hamspot_name(name) {
        return None;
    }
    let media = if class.contains("Source") {
        "Audio/Source/Virtual"
    } else {
        "Audio/Sink"
    };
    let description = description.replace('"', "'");
    let spec = format!(
        "{{ factory.name=support.null-audio-sink node.name={name} node.description=\"{description}\" device.description=\"{description}\" media.class={media} object.linger=true audio.position=[FL FR] }}"
    );
    Some(vec!["create-node".into(), "adapter".into(), spec])
}

pub fn pair_ports(from: &[String], to: &[String]) -> Vec<(String, String)> {
    if from.is_empty() || to.is_empty() {
        return Vec::new();
    }
    if from.len() == 1 {
        return to
            .iter()
            .map(|port| (from[0].clone(), port.clone()))
            .collect();
    }
    if to.len() == 1 {
        return from
            .iter()
            .map(|port| (port.clone(), to[0].clone()))
            .collect();
    }
    from.iter()
        .zip(to.iter())
        .map(|(left, right)| (left.clone(), right.clone()))
        .collect()
}

pub fn ports_of(ports: &[String], node: &str) -> Vec<String> {
    let prefix = format!("{node}:");
    ports
        .iter()
        .filter(|port| port.starts_with(&prefix))
        .cloned()
        .collect()
}

/// Capture port a transmit pipe records, and the `pw-cat` channel map that
/// matches it. A mono capture is one channel. A stereo source uses the left
/// channel. Both stay 48 kHz mono s16le on the script's stdin.
pub fn capture_record_port(ports: &[String], source: &str) -> Option<(String, &'static str)> {
    let list = ports_of(ports, source);
    if let Some(port) = list.iter().find(|port| port.ends_with(":capture_MONO")) {
        return Some((port.clone(), "MONO"));
    }
    if let Some(port) = list.iter().find(|port| port.ends_with(":capture_FL")) {
        return Some((port.clone(), "FL"));
    }
    None
}

/// Input ports of a transmit stream. `input_FL` / `input_FR` / `input_MONO`
/// are the wires the microphone uses. Anything else on that node is left out
/// when one of those is present.
pub fn stream_input_ports(ports: &[String], stream: &str) -> Vec<String> {
    let list = ports_of(ports, stream);
    let preferred: Vec<String> = list
        .iter()
        .filter(|port| {
            let leaf = port.rsplit(':').next().unwrap_or("");
            leaf == "input_FL" || leaf == "input_FR" || leaf == "input_MONO"
        })
        .cloned()
        .collect();
    if preferred.is_empty() {
        list
    } else {
        preferred
    }
}

/// Saved wires split into the ones both ends can accept, and a flag when one
/// of this radio's own ports is not in the list yet. An outside name that is
/// absent (GQRX is not running) is neither ready nor this radio's wait.
/// `pw-link` on that name returns "No such file or directory".
pub struct SavedPass<'a> {
    pub ready: Vec<&'a AudioLink>,
    pub waiting_on_own: bool,
}

pub fn classify_saved<'a>(
    links: &'a [AudioLink],
    outputs: &[String],
    inputs: &[String],
    own_nodes: &[String],
) -> SavedPass<'a> {
    let listed = |port: &str| {
        outputs.iter().any(|item| item == port) || inputs.iter().any(|item| item == port)
    };
    let mut ready = Vec::new();
    let mut waiting_on_own = false;
    for link in links {
        let output_listed = listed(&link.output);
        let input_listed = listed(&link.input);
        if output_listed && input_listed {
            ready.push(link);
            continue;
        }
        let missing_own = |port: &str, is_listed: bool| {
            !is_listed && own_nodes.iter().any(|node| port_node(port) == node)
        };
        if missing_own(&link.output, output_listed) || missing_own(&link.input, input_listed) {
            waiting_on_own = true;
        }
    }
    SavedPass {
        ready,
        waiting_on_own,
    }
}

/// `pw-link` could not see one of the names. The port list was already stale,
/// or the other program has not created it. The caller keeps trying.
pub fn missing_port(err: &str) -> bool {
    err.to_ascii_lowercase().contains("no such file")
}

fn command_output(program: &str, args: &[String]) -> Result<std::process::Output, String> {
    Command::new(program)
        .args(args)
        .output()
        .map_err(|err| format!("{program}: {err}"))
}

pub fn pw_ports(output: bool) -> Result<Vec<String>, String> {
    let flag = if output { "-o" } else { "-i" };
    let output = command_output("pw-link", &[flag.into()])?;
    if !output.status.success() {
        return Err(format!(
            "pw-link {flag}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn pw_link(output_port: &str, input_port: &str) -> Result<(), String> {
    let output = command_output("pw-link", &[output_port.into(), input_port.into()])?;
    if output.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&output.stderr);
    // The wire is already there. That is the state we wanted.
    if err.to_ascii_lowercase().contains("exist") {
        return Ok(());
    }
    Err(format!("pw-link: {}", err.trim()))
}

/// Output-port links for `node` in `pw-link -l` text.
///
/// The listing names a port, then `|->` for a link that leaves it and
/// `|<-` for a link that arrives. Both directions describe one wire, so
/// each pair is returned once.
pub fn output_links_from_list(text: &str, node: &str) -> Vec<(String, String)> {
    let mut found = BTreeSet::new();
    let mut current = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(peer) = trimmed.strip_prefix("|->") {
            let peer = peer.trim();
            if port_node(&current) == node && !peer.is_empty() {
                found.insert((current.clone(), peer.to_string()));
            }
            continue;
        }
        if let Some(peer) = trimmed.strip_prefix("|<-") {
            let peer = peer.trim();
            if port_node(peer) == node && !current.is_empty() {
                found.insert((peer.to_string(), current.clone()));
            }
            continue;
        }
        current = trimmed.to_string();
    }
    found.into_iter().collect()
}

/// Remove every wire leaving `node`. The count is how many were listed
/// before the removal. Zero means the node is already quiet.
pub fn unlink_node_outputs(node: &str) -> Result<usize, String> {
    let output = command_output("pw-link", &["-l".into()])?;
    if !output.status.success() {
        return Err(format!(
            "pw-link -l: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let links = output_links_from_list(&text, node);
    let count = links.len();
    for (from, to) in links {
        pw_unlink(&from, &to)?;
    }
    Ok(count)
}

pub fn pw_unlink(output_port: &str, input_port: &str) -> Result<(), String> {
    let output = command_output(
        "pw-link",
        &["-d".into(), output_port.into(), input_port.into()],
    )?;
    if output.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&output.stderr);
    if err.to_ascii_lowercase().contains("no such")
        || err.to_ascii_lowercase().contains("not exist")
    {
        return Ok(());
    }
    Err(format!("pw-link -d: {}", err.trim()))
}

pub fn default_sink() -> Result<String, String> {
    pactl_line(&["get-default-sink".into()])
}

pub fn default_source() -> Result<String, String> {
    pactl_line(&["get-default-source".into()])
}

/// WirePlumber files that pin a sink, a source, a profile, a route, a stream
/// target, or a Bluetooth profile. Stock PipeWire has none of these.
const WIREPLUMBER_STATE_FILES: &[&str] = &[
    "default-nodes",
    "default-profile",
    "default-routes",
    "stream-properties",
    "bluetooth-autoswitch",
];

/// What the Config button did to the user PipeWire session.
pub struct UserPipewireReset {
    /// The user services were stopped and started again.
    pub restarted: bool,
    /// Empty when the desktop default device is ready.
    pub error: Option<String>,
}

/// A name `pactl get-default-sink` can be wired to. `auto_null` is the
/// fallback PipeWire inserts when the kernel has no card.
pub fn sink_is_desktop_default(name: &str, kernel_has_cards: bool) -> bool {
    let name = name.trim();
    if name.is_empty() || name.starts_with('@') {
        return false;
    }
    if name == "auto_null" {
        return !kernel_has_cards;
    }
    true
}

/// Drop the saved WirePlumber defaults in `dir`. Other files there stay.
pub fn clear_wireplumber_files(dir: &Path) -> Result<(), String> {
    for name in WIREPLUMBER_STATE_FILES {
        let path = dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(format!("remove {}: {err}", path.display())),
        }
    }
    Ok(())
}

fn wireplumber_state_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir).join("wireplumber"));
        }
    }
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty());
    let Some(home) = home else {
        return Err("HOME is not set, so WirePlumber state cannot be reset".into());
    };
    Ok(PathBuf::from(home).join(".local/state/wireplumber"))
}

fn systemctl(args: &[&str]) -> Result<(), String> {
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let output = command_output("systemctl", &owned)?;
    if output.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if err.is_empty() {
        Err(format!("systemctl {} failed", args.join(" ")))
    } else {
        Err(format!("systemctl: {err}"))
    }
}

fn systemctl_ok(args: &[&str]) -> bool {
    systemctl(args).is_ok()
}

fn kernel_has_soundcards() -> bool {
    let text = std::fs::read_to_string("/proc/asound/cards").unwrap_or_default();
    text.lines().any(|line| line.trim_start().starts_with(|ch: char| ch.is_ascii_digit()))
}

fn sink_ready() -> Result<bool, String> {
    let name = default_sink().unwrap_or_default();
    Ok(sink_is_desktop_default(&name, kernel_has_soundcards()))
}

/// `pactl subscribe` is the signal that a card or the default sink changed.
/// The call returns when the sink is a real device, or when subscribe ends.
/// A kernel card that PipeWire never opens leaves this read sitting there:
/// WirePlumber has no "enumeration finished" event for a card that does not appear.
fn wait_for_desktop_default() -> Result<(), String> {
    if sink_ready()? {
        return Ok(());
    }
    let mut child = Command::new("pactl")
        .arg("subscribe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("pactl subscribe: {err}"))?;
    let ready = match child.stdout.take() {
        Some(stdout) => read_subscribe_until_ready(stdout),
        None => Err("pactl subscribe: no output".into()),
    };
    let _ = child.kill();
    let _ = child.wait();
    ready
}

fn read_subscribe_until_ready(stdout: impl std::io::Read) -> Result<(), String> {
    if sink_ready()? {
        return Ok(());
    }
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        line.map_err(|err| format!("pactl subscribe: {err}"))?;
        if sink_ready()? {
            return Ok(());
        }
    }
    if sink_ready()? {
        Ok(())
    } else {
        Err("PipeWire did not publish a default device".into())
    }
}

/// Stop the user PipeWire session, delete WirePlumber's saved defaults, and
/// start the session again. The sockets are stopped too, so a client reconnect
/// cannot bring the old daemon back before those files are gone.
pub fn reset_user_pipewire() -> UserPipewireReset {
    let dir = match wireplumber_state_dir() {
        Ok(dir) => dir,
        Err(err) => {
            return UserPipewireReset {
                restarted: false,
                error: Some(err),
            };
        }
    };
    if let Err(err) = systemctl(&[
        "--user",
        "stop",
        "wireplumber.service",
        "filter-chain.service",
        "pipewire-pulse.service",
        "pipewire.service",
        "pipewire-pulse.socket",
        "pipewire.socket",
    ]) {
        return UserPipewireReset {
            restarted: false,
            error: Some(err),
        };
    }
    let cleared = clear_wireplumber_files(&dir);
    if let Err(err) = systemctl(&[
        "--user",
        "start",
        "pipewire.socket",
        "pipewire-pulse.socket",
        "pipewire.service",
        "wireplumber.service",
        "pipewire-pulse.service",
    ]) {
        return UserPipewireReset {
            restarted: false,
            error: Some(err),
        };
    }
    // filter-chain stops with pipewire (BindsTo). Start it again only when
    // the user had it enabled. A disabled unit stays down.
    if systemctl_ok(&["--user", "is-enabled", "filter-chain.service"]) {
        if let Err(err) = systemctl(&["--user", "start", "filter-chain.service"]) {
            return UserPipewireReset {
                restarted: true,
                error: Some(err),
            };
        }
    }
    let ready = wait_for_desktop_default();
    let error = match (cleared, ready) {
        (Err(err), _) => Some(err),
        (Ok(()), Err(err)) => Some(err),
        (Ok(()), Ok(())) => None,
    };
    UserPipewireReset {
        restarted: true,
        error,
    }
}

fn pactl_line(args: &[String]) -> Result<String, String> {
    let output = command_output("pactl", args)?;
    if !output.status.success() {
        return Err(format!(
            "pactl: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() {
        return Err("pactl returned an empty name".into());
    }
    Ok(name)
}

/// One shot of the running graph. The monitor is the ongoing signal; this is
/// only for the moment a station file is applied, before the next monitor line.
pub fn dump_once() -> Option<AudioGraph> {
    let output = command_output(
        "pw-dump",
        &["-N".into(), "-R".into(), "-i".into(), "0".into()],
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    graph_from_dump(&text).map(|look| look.graph)
}

pub fn create_linger(name: &str, description: &str, class: &str) -> Result<(), String> {
    let Some(args) = linger_args(name, description, class) else {
        return Err(format!("not recreating {name}"));
    };
    let output = command_output("pw-cli", &args)?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "pw-cli: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_use_make_model_and_connection_id() {
        assert_eq!(describe("Hamlib", "Dummy", 1, "RX"), "Hamlib Dummy 1 RX");
        assert_eq!(describe("Icom", "IC-7300", 1, "TX"), "Icom IC-7300 1 TX");
        assert_eq!(describe("GQRX", "SDR", 4, "RX"), "GQRX SDR 4 RX");
    }

    #[test]
    fn loopback_args_keep_the_description_and_do_not_target_a_device() {
        let args = rx_loopback_args(1, "Icom IC-7300 1 RX");
        let joined = args.join(" ");
        assert!(joined.contains("node.description=\"Icom IC-7300 1 RX\""));
        assert!(joined.contains("node.name=hamspot_1_rx"));
        assert!(joined.contains("node.dont-reconnect=true"));
        assert!(!joined.contains("node.target"));
        let tx = tx_loopback_args(4, "GQRX SDR 4 TX").join(" ");
        assert!(tx.contains("media.class=Audio/Source"));
        assert!(tx.contains("node.name=hamspot_4_tx"));
        assert!(tx.contains("GQRX SDR 4 TX"));
    }

    #[test]
    fn dump_keeps_hamspot_devices_and_their_wires() {
        let text = r#"[
          {"id": 1, "type": "PipeWire:Interface:Node", "info": {"props": {
            "node.name": "hamspot_1_rx", "node.description": "Hamlib Dummy 1 RX", "media.class": "Audio/Sink"}}},
          {"id": 2, "type": "PipeWire:Interface:Node", "info": {"props": {
            "node.name": "output.hamspot_1_rx", "node.description": "out", "media.class": "Stream/Output/Audio"}}},
          {"id": 3, "type": "PipeWire:Interface:Node", "info": {"props": {
            "node.name": "alsa_output.speakers", "node.description": "Speakers", "media.class": "Audio/Sink"}}},
          {"id": 4, "type": "PipeWire:Interface:Node", "info": {"props": {
            "node.name": "Radio-Playback", "node.description": "Radio", "media.class": "Audio/Sink"}}},
          {"id": 11, "type": "PipeWire:Interface:Port", "info": {"props": {"node.id": 2, "port.name": "output_FL"}}},
          {"id": 12, "type": "PipeWire:Interface:Port", "info": {"props": {"node.id": 3, "port.name": "playback_FL"}}},
          {"id": 20, "type": "PipeWire:Interface:Link", "info": {"output-port-id": 11, "input-port-id": 12}}
        ]"#;
        let look = graph_from_dump(text).expect("dump");
        assert_eq!(look.graph.devices.len(), 1);
        assert_eq!(look.graph.devices[0].name, "hamspot_1_rx");
        assert_eq!(look.graph.devices[0].description, "Hamlib Dummy 1 RX");
        assert_eq!(
            look.graph.links,
            vec![AudioLink {
                output: "output.hamspot_1_rx:output_FL".into(),
                input: "alsa_output.speakers:playback_FL".into(),
            }]
        );
        assert!(look
            .playback
            .iter()
            .any(|node| node.name == "alsa_output.speakers"));
        assert!(look
            .playback
            .iter()
            .any(|node| node.name == "Radio-Playback"));
        assert_eq!(
            look.connect_ports
                .get("alsa_output.speakers")
                .map(Vec::as_slice),
            Some(["playback_FL".to_string()].as_slice())
        );
        assert_eq!(
            look.connect_ports
                .get("output.hamspot_1_rx")
                .map(Vec::as_slice),
            Some(["output_FL".to_string()].as_slice())
        );
        assert_eq!(
            peer_node(&look.graph, "output.hamspot_1_rx", true).as_deref(),
            Some("alsa_output.speakers")
        );
        let again = canonical(&look.graph);
        assert_eq!(
            again,
            canonical(&serde_json::from_str::<AudioGraph>(&again).unwrap())
        );
    }

    #[test]
    fn one_capture_port_fans_out_to_a_stereo_sink() {
        let pairs = pair_ports(
            &["mic:capture_MONO".into()],
            &[
                "input.hamspot_1_tx:input_FL".into(),
                "input.hamspot_1_tx:input_FR".into(),
            ],
        );
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0, "mic:capture_MONO");
        assert_eq!(pairs[1].1, "input.hamspot_1_tx:input_FR");
    }

    #[test]
    fn a_linger_card_occupies_the_name_until_the_stream_exists() {
        assert!(linger_occupies(true, false));
        assert!(!linger_occupies(true, true));
        assert!(!linger_occupies(false, false));
    }

    #[test]
    fn destroy_named_node_refuses_a_speaker() {
        let err = destroy_named_node("bluez_output.E8:85:EF:F7:B7:10").unwrap_err();
        assert!(err.contains("not removing"));
    }

    #[test]
    fn linger_args_refuse_a_name_that_is_not_hamspot() {
        assert!(linger_args("Radio-Playback", "Radio", "Audio/Sink").is_none());
        let args = linger_args("hamspot_custom", "Extra", "Audio/Source").unwrap();
        assert_eq!(args[0], "create-node");
        assert!(args[2].contains("node.name=hamspot_custom"));
        assert!(args[2].contains("Audio/Source/Virtual"));
        assert!(args[2].contains("object.linger=true"));
    }

    #[test]
    fn a_monitor_change_does_not_drop_the_radio_card() {
        let full = r#"[
          {"id": 1, "type": "PipeWire:Interface:Node", "info": {"props": {
            "node.name": "hamspot_1_rx", "node.description": "Hamlib Dummy 1 RX", "media.class": "Audio/Sink"}}},
          {"id": 2, "type": "PipeWire:Interface:Node", "info": {"props": {
            "node.name": "output.hamspot_1_rx", "node.description": "out", "media.class": "Stream/Output/Audio"}}},
          {"id": 3, "type": "PipeWire:Interface:Node", "info": {"props": {
            "node.name": "alsa_output.speakers", "node.description": "Speakers", "media.class": "Audio/Sink"}}}
        ]"#;
        let mut merge = DumpMerge::default();
        let first = merge.apply(full).expect("full");
        assert!(has_device(&first.graph, "hamspot_1_rx"));
        let changed = r#"[
          {"id": 2, "type": "PipeWire:Interface:Node", "info": {"props": {
            "node.name": "output.hamspot_1_rx", "node.description": "out", "media.class": "Stream/Output/Audio"}}}
        ]"#;
        let second = merge.apply(changed).expect("change");
        assert!(has_device(&second.graph, "hamspot_1_rx"));
        assert!(second
            .playback
            .iter()
            .any(|node| node.name == "alsa_output.speakers"));
        let removed = r#"[{"id": 1, "info": null}]"#;
        let third = merge.apply(removed).expect("remove");
        assert!(!has_device(&third.graph, "hamspot_1_rx"));
    }

    #[test]
    fn a_mono_capture_is_the_transmit_record_port() {
        let ports = vec![
            "mic:capture_FL".into(),
            "mic:capture_MONO".into(),
            "other:capture_FL".into(),
        ];
        assert_eq!(
            capture_record_port(&ports, "mic"),
            Some(("mic:capture_MONO".into(), "MONO"))
        );
        let stereo = vec!["mic:capture_FR".into(), "mic:capture_FL".into()];
        assert_eq!(
            capture_record_port(&stereo, "mic"),
            Some(("mic:capture_FL".into(), "FL"))
        );
        assert!(capture_record_port(&stereo, "gone").is_none());
        assert_eq!(
            stream_input_ports(
                &[
                    "input.hamspot_2_tx:monitor_FL".into(),
                    "input.hamspot_2_tx:input_FR".into(),
                    "input.hamspot_2_tx:input_FL".into(),
                ],
                "input.hamspot_2_tx"
            ),
            vec![
                "input.hamspot_2_tx:input_FR".to_string(),
                "input.hamspot_2_tx:input_FL".to_string(),
            ]
        );
    }

    #[test]
    fn a_missing_outside_port_is_not_a_radio_wait() {
        let links = vec![
            AudioLink {
                output: "GQRX:output_FL".into(),
                input: "hamspot_3_rx:playback_FL".into(),
            },
            AudioLink {
                output: "mic:capture_MONO".into(),
                input: "input.hamspot_3_tx:input_FL".into(),
            },
        ];
        let outputs = vec![
            "mic:capture_MONO".into(),
            "output.hamspot_3_rx:output_FL".into(),
        ];
        let inputs = vec![
            "hamspot_3_rx:playback_FL".into(),
            "input.hamspot_3_tx:input_FL".into(),
        ];
        let own = vec![
            "hamspot_3_rx".into(),
            "hamspot_3_tx".into(),
            "output.hamspot_3_rx".into(),
            "input.hamspot_3_tx".into(),
        ];
        let pass = classify_saved(&links, &outputs, &inputs, &own);
        assert!(!pass.waiting_on_own);
        assert_eq!(pass.ready.len(), 1);
        assert_eq!(pass.ready[0].output, "mic:capture_MONO");
        assert!(missing_port(
            "pw-link: failed to link ports: No such file or directory"
        ));
    }

    #[test]
    fn a_pipewire_listing_names_each_output_link_once() {
        let text = "\
output.hamspot_3_rx:output_FL
  |-> alsa_output.speakers:playback_FL
output.hamspot_3_rx:output_FR
  |-> alsa_output.speakers:playback_FR
alsa_output.speakers:playback_FL
  |<- output.hamspot_3_rx:output_FL
  |<- pw-cat:output_FL
hamspot_3_rx:monitor_FL
  |-> commander-rx-3:input_FL
";
        let links = output_links_from_list(text, "output.hamspot_3_rx");
        assert_eq!(
            links,
            vec![
                (
                    "output.hamspot_3_rx:output_FL".to_string(),
                    "alsa_output.speakers:playback_FL".to_string()
                ),
                (
                    "output.hamspot_3_rx:output_FR".to_string(),
                    "alsa_output.speakers:playback_FR".to_string()
                ),
            ]
        );
        assert_eq!(
            output_links_from_list(text, "pw-cat"),
            vec![(
                "pw-cat:output_FL".to_string(),
                "alsa_output.speakers:playback_FL".to_string()
            )]
        );
        assert!(output_links_from_list(text, "commander-rx-3").is_empty());
    }

    #[test]
    fn a_missing_radio_port_keeps_the_wire_waiting() {
        let links = vec![AudioLink {
            output: "GQRX:output_FL".into(),
            input: "hamspot_3_rx:playback_FL".into(),
        }];
        let outputs = vec!["GQRX:output_FL".into()];
        let inputs = vec!["input.hamspot_3_tx:input_FL".into()];
        let own = vec!["hamspot_3_rx".into(), "input.hamspot_3_tx".into()];
        let pass = classify_saved(&links, &outputs, &inputs, &own);
        assert!(pass.waiting_on_own);
        assert!(pass.ready.is_empty());
    }

    #[test]
    fn saved_wireplumber_defaults_are_removed() {
        let dir = std::env::temp_dir().join(format!("hs-wp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "default-nodes",
            "default-profile",
            "default-routes",
            "stream-properties",
            "bluetooth-autoswitch",
            "keep-me",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        clear_wireplumber_files(&dir).unwrap();
        for name in [
            "default-nodes",
            "default-profile",
            "default-routes",
            "stream-properties",
            "bluetooth-autoswitch",
        ] {
            assert!(!dir.join(name).exists(), "{name}");
        }
        assert!(dir.join("keep-me").exists());
        clear_wireplumber_files(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_fallback_sink_is_not_a_desktop_default_when_a_card_exists() {
        assert!(!sink_is_desktop_default("", true));
        assert!(!sink_is_desktop_default("@DEFAULT_SINK@", true));
        assert!(!sink_is_desktop_default("auto_null", true));
        assert!(sink_is_desktop_default("auto_null", false));
        assert!(sink_is_desktop_default(
            "alsa_output.pci-0000_00_1b.0.analog-stereo",
            true
        ));
    }
}
