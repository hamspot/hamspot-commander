//! Virtual PipeWire cards for the radios, and the graph other tools leave behind.
//!
//! A card is two `pw-loopback` nodes. RX is a sink the radio plays into. TX is
//! a source the radio captures. Nothing here reconnects a wire after another
//! process moves it. `pw-dump -m` is the change signal. Its first line is the
//! whole graph. Later lines list only the objects that changed, and `info:
//! null` removes an id. Those lines are merged; a changed line is not a new graph.

use std::collections::BTreeMap;
use std::process::Command;

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
    Some(AudioLook {
        graph,
        playback,
        capture,
    })
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
}
