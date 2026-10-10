//! Simplified patch bay for the Audio frame.
//!
//! The dropdowns and this canvas both end in `set_endpoint`. A drop chooses
//! the computer device for one radio. Port names are only where the wire is drawn.

use crate::audio::{self, AudioLink, AudioNode};

/// One radio's saved computer devices.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioBinding {
    pub id: u64,
    pub input: String,
    pub output: String,
}

/// A place a wire can start or land.
#[derive(Clone, Debug, PartialEq)]
pub enum End {
    Capture { device: String },
    Playback { device: String },
    RadioTx { id: u64 },
    RadioRx { id: u64 },
}

/// What the pointer picked up.
#[derive(Clone, Debug, PartialEq)]
pub enum Grab {
    /// A microphone port. `radio` is set when the drag started on an existing wire.
    Capture {
        device: String,
        radio: Option<u64>,
    },
    /// A speaker port. `radio` is set when the drag started on an existing wire.
    Playback {
        device: String,
        radio: Option<u64>,
    },
    RadioTx {
        id: u64,
    },
    RadioRx {
        id: u64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct EndpointUpdate {
    pub id: u64,
    pub input: String,
    pub output: String,
}

#[derive(Clone, Debug)]
pub struct BayRadio {
    pub id: u64,
    pub title: String,
    pub rx_title: String,
    pub tx_title: String,
    pub input: String,
    pub output: String,
}

pub struct BayIn<'a> {
    pub captures: &'a [AudioNode],
    pub playbacks: &'a [AudioNode],
    pub radios: &'a [BayRadio],
    pub links: &'a [AudioLink],
    pub ports: &'a std::collections::BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IconKind {
    Mic,
    Speaker,
    Radio,
    Xdg,
}

#[derive(Clone, Debug)]
pub struct IconSpot {
    pub x: f32,
    pub y: f32,
    pub kind: IconKind,
    /// Freedesktop icon name when `kind` is `Xdg`.
    pub xdg: String,
    pub label: String,
    pub hit: Option<End>,
}

#[derive(Clone, Debug)]
pub struct PortSpot {
    pub end: End,
    pub port: String,
    pub at: [f32; 2],
    pub label: String,
    pub label_right: bool,
}

#[derive(Clone, Debug)]
pub struct WireSpot {
    pub from: [f32; 2],
    pub to: [f32; 2],
    pub from_grab: Grab,
    pub to_grab: Grab,
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub icons: Vec<IconSpot>,
    pub ports: Vec<PortSpot>,
    pub wires: Vec<WireSpot>,
}

#[derive(Clone, Debug)]
pub struct Drag {
    pub grab: Grab,
    pub anchor: [f32; 2],
}

const ICON: f32 = 48.0;
const DOT: f32 = 12.0;

/// Icon name for a PipeWire node that Commander did not create.
pub fn xdg_icon_name(class: &str, name: &str, description: &str) -> &'static str {
    let blob = format!("{name} {description}").to_ascii_lowercase();
    if blob.contains("headphone") || blob.contains("headset") {
        return "audio-headphones";
    }
    if class == "Audio/Source" || blob.contains("mic") || blob.contains("capture") {
        return "audio-input-microphone";
    }
    if class == "Audio/Sink" {
        return "audio-speakers";
    }
    "audio-card"
}

pub fn endpoint_updates(
    radios: &[RadioBinding],
    grab: &Grab,
    target: Option<&End>,
) -> Vec<EndpointUpdate> {
    let find = |id: u64| radios.iter().find(|radio| radio.id == id);
    match (grab, target) {
        (
            Grab::Capture {
                device,
                radio: None,
            },
            Some(End::RadioTx { id }),
        ) => {
            let Some(radio) = find(*id) else {
                return Vec::new();
            };
            vec![one(*id, device.clone(), radio.output.clone())]
        }
        (Grab::RadioTx { id }, Some(End::Capture { device })) => {
            let Some(radio) = find(*id) else {
                return Vec::new();
            };
            vec![one(*id, device.clone(), radio.output.clone())]
        }
        (Grab::RadioTx { id }, None) => {
            let Some(radio) = find(*id) else {
                return Vec::new();
            };
            vec![one(*id, String::new(), radio.output.clone())]
        }
        (
            Grab::Capture {
                device,
                radio: Some(id),
            },
            Some(End::Capture { device: next }),
        ) => {
            let Some(radio) = find(*id) else {
                return Vec::new();
            };
            if radio.input != *device {
                return Vec::new();
            }
            vec![one(*id, next.clone(), radio.output.clone())]
        }
        (
            Grab::Capture {
                device,
                radio: Some(id),
            },
            None,
        ) => {
            let Some(radio) = find(*id) else {
                return Vec::new();
            };
            if radio.input != *device {
                return Vec::new();
            }
            vec![one(*id, String::new(), radio.output.clone())]
        }
        (
            Grab::Capture {
                device,
                radio: None,
            },
            None,
        ) => radios
            .iter()
            .filter(|radio| radio.input == *device)
            .map(|radio| one(radio.id, String::new(), radio.output.clone()))
            .collect(),
        (
            Grab::Playback {
                device,
                radio: None,
            },
            Some(End::RadioRx { id }),
        ) => {
            let Some(radio) = find(*id) else {
                return Vec::new();
            };
            vec![one(*id, radio.input.clone(), device.clone())]
        }
        (Grab::RadioRx { id }, Some(End::Playback { device })) => {
            let Some(radio) = find(*id) else {
                return Vec::new();
            };
            vec![one(*id, radio.input.clone(), device.clone())]
        }
        (Grab::RadioRx { id }, None) => {
            let Some(radio) = find(*id) else {
                return Vec::new();
            };
            vec![one(*id, radio.input.clone(), String::new())]
        }
        (
            Grab::Playback {
                device,
                radio: Some(id),
            },
            Some(End::Playback { device: next }),
        ) => {
            let Some(radio) = find(*id) else {
                return Vec::new();
            };
            if radio.output != *device {
                return Vec::new();
            }
            vec![one(*id, radio.input.clone(), next.clone())]
        }
        (
            Grab::Playback {
                device,
                radio: Some(id),
            },
            None,
        ) => {
            let Some(radio) = find(*id) else {
                return Vec::new();
            };
            if radio.output != *device {
                return Vec::new();
            }
            vec![one(*id, radio.input.clone(), String::new())]
        }
        (
            Grab::Playback {
                device,
                radio: None,
            },
            None,
        ) => radios
            .iter()
            .filter(|radio| radio.output == *device)
            .map(|radio| one(radio.id, radio.input.clone(), String::new()))
            .collect(),
        (Grab::RadioTx { id: from }, Some(End::RadioTx { id: to })) if from != to => {
            let (Some(src), Some(dst)) = (find(*from), find(*to)) else {
                return Vec::new();
            };
            if src.input.is_empty() {
                return Vec::new();
            }
            vec![
                one(*from, String::new(), src.output.clone()),
                one(*to, src.input.clone(), dst.output.clone()),
            ]
        }
        (Grab::RadioRx { id: from }, Some(End::RadioRx { id: to })) if from != to => {
            let (Some(src), Some(dst)) = (find(*from), find(*to)) else {
                return Vec::new();
            };
            if src.output.is_empty() {
                return Vec::new();
            }
            vec![
                one(*from, src.input.clone(), String::new()),
                one(*to, dst.input.clone(), src.output.clone()),
            ]
        }
        _ => Vec::new(),
    }
}

fn one(id: u64, input: String, output: String) -> EndpointUpdate {
    EndpointUpdate { id, input, output }
}

pub fn layout(width: f32, input: &BayIn<'_>) -> Scene {
    let width = width.max(680.0);
    let col = (width - 16.0) / 3.0;
    let mut icons = Vec::new();
    let mut ports = Vec::new();
    let mut y_left = 8.0;
    for device in input.captures {
        let names = shown_ports(input.ports, &device.name, fallback_capture(device));
        let row = row_height(names.len());
        place_system(
            &mut icons,
            &mut ports,
            device,
            End::Capture {
                device: device.name.clone(),
            },
            8.0,
            y_left,
            col,
            true,
            &names,
        );
        y_left += row;
    }
    let mut y_right = 8.0;
    for device in input.playbacks {
        let names = shown_ports(input.ports, &device.name, &["playback_FL", "playback_FR"]);
        let row = row_height(names.len());
        place_system(
            &mut icons,
            &mut ports,
            device,
            End::Playback {
                device: device.name.clone(),
            },
            8.0 + 2.0 * col,
            y_right,
            col,
            false,
            &names,
        );
        y_right += row;
    }
    let mut y_mid = 8.0;
    for radio in input.radios {
        let tx_ports = shown_ports(
            input.ports,
            &audio::tx_stream(radio.id),
            &["input_FL", "input_FR"],
        );
        let rx_ports = shown_ports(
            input.ports,
            &audio::rx_stream(radio.id),
            &["output_FL", "output_FR"],
        );
        let row = row_height(tx_ports.len().max(rx_ports.len())).max(78.0);
        let x0 = 8.0 + col;
        place_radio_side(
            &mut icons,
            &mut ports,
            x0 + 28.0,
            y_mid,
            IconKind::Mic,
            &radio.tx_title,
            End::RadioTx { id: radio.id },
            &tx_ports,
            true,
        );
        icons.push(IconSpot {
            x: x0 + col * 0.5 - ICON * 0.5,
            y: y_mid,
            kind: IconKind::Radio,
            xdg: String::new(),
            label: radio.title.clone(),
            hit: None,
        });
        place_radio_side(
            &mut icons,
            &mut ports,
            x0 + col - ICON - 28.0,
            y_mid,
            IconKind::Speaker,
            &radio.rx_title,
            End::RadioRx { id: radio.id },
            &rx_ports,
            false,
        );
        y_mid += row;
    }
    let height = y_left.max(y_right).max(y_mid).max(88.0);
    let wires = wires_of(input, &ports);
    Scene {
        width,
        height,
        icons,
        ports,
        wires,
    }
}

fn place_system(
    icons: &mut Vec<IconSpot>,
    ports: &mut Vec<PortSpot>,
    device: &AudioNode,
    end: End,
    x0: f32,
    y: f32,
    col: f32,
    ports_on_right: bool,
    names: &[&str],
) {
    let icon_x = if ports_on_right {
        x0
    } else {
        x0 + col - ICON - 8.0
    };
    let label = device_title(device);
    icons.push(IconSpot {
        x: icon_x,
        y,
        kind: IconKind::Xdg,
        xdg: xdg_icon_name(&device.class, &device.name, &device.description).to_string(),
        label,
        hit: Some(end.clone()),
    });
    let port_x = if ports_on_right {
        icon_x + ICON + 16.0
    } else {
        icon_x - 16.0
    };
    place_ports(ports, end, names, port_x, y + ICON * 0.5, ports_on_right);
}

fn place_radio_side(
    icons: &mut Vec<IconSpot>,
    ports: &mut Vec<PortSpot>,
    x: f32,
    y: f32,
    kind: IconKind,
    label: &str,
    end: End,
    names: &[&str],
    ports_on_left: bool,
) {
    icons.push(IconSpot {
        x,
        y,
        kind,
        xdg: String::new(),
        label: short(label),
        hit: Some(end.clone()),
    });
    let port_x = if ports_on_left {
        x - 16.0
    } else {
        x + ICON + 16.0
    };
    place_ports(ports, end, names, port_x, y + ICON * 0.5, !ports_on_left);
}

fn place_ports(
    ports: &mut Vec<PortSpot>,
    end: End,
    names: &[&str],
    x: f32,
    mid_y: f32,
    label_on_right: bool,
) {
    let n = names.len() as f32;
    for (i, name) in names.iter().enumerate() {
        let at_y = mid_y + (i as f32 - (n - 1.0) * 0.5) * 16.0;
        ports.push(PortSpot {
            end: end.clone(),
            port: (*name).to_string(),
            at: [x, at_y],
            label: (*name).to_string(),
            label_right: label_on_right,
        });
    }
}

fn wires_of(input: &BayIn<'_>, ports: &[PortSpot]) -> Vec<WireSpot> {
    let mut wires = Vec::new();
    for link in input.links {
        let out_node = audio::port_node(&link.output);
        let in_node = audio::port_node(&link.input);
        if let Some(id) = stream_id(out_node, "output.hamspot_", "_rx") {
            let Some(radio) = input.radios.iter().find(|radio| radio.id == id) else {
                continue;
            };
            if radio.output != in_node {
                continue;
            }
            let from = port_at(ports, &End::RadioRx { id }, port_leaf(&link.output));
            let to = port_at(
                ports,
                &End::Playback {
                    device: in_node.to_string(),
                },
                port_leaf(&link.input),
            );
            let (Some(from), Some(to)) = (from, to) else {
                continue;
            };
            wires.push(WireSpot {
                from,
                to,
                from_grab: Grab::RadioRx { id },
                to_grab: Grab::Playback {
                    device: in_node.to_string(),
                    radio: Some(id),
                },
            });
        } else if let Some(id) = stream_id(in_node, "input.hamspot_", "_tx") {
            let Some(radio) = input.radios.iter().find(|radio| radio.id == id) else {
                continue;
            };
            if radio.input != out_node {
                continue;
            }
            let from = port_at(
                ports,
                &End::Capture {
                    device: out_node.to_string(),
                },
                port_leaf(&link.output),
            );
            let to = port_at(ports, &End::RadioTx { id }, port_leaf(&link.input));
            let (Some(from), Some(to)) = (from, to) else {
                continue;
            };
            wires.push(WireSpot {
                from,
                to,
                from_grab: Grab::Capture {
                    device: out_node.to_string(),
                    radio: Some(id),
                },
                to_grab: Grab::RadioTx { id },
            });
        }
    }
    wires
}

fn port_at(ports: &[PortSpot], end: &End, port: &str) -> Option<[f32; 2]> {
    ports
        .iter()
        .find(|spot| spot.end == *end && spot.port == port)
        .map(|spot| spot.at)
        .or_else(|| {
            ports
                .iter()
                .find(|spot| spot.end == *end)
                .map(|spot| spot.at)
        })
}

pub fn grab_at(scene: &Scene, x: f32, y: f32) -> Option<Drag> {
    if let Some(port) = nearest_port(scene, x, y, DOT) {
        let grab = scene
            .wires
            .iter()
            .find_map(|wire| {
                if near(wire.to, port.at) {
                    Some(wire.to_grab.clone())
                } else if near(wire.from, port.at) {
                    Some(wire.from_grab.clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| grab_from_end(&port.end));
        return Some(Drag {
            grab,
            anchor: port.at,
        });
    }
    if let Some(icon) = scene
        .icons
        .iter()
        .find(|icon| icon.hit.is_some() && in_icon(icon, x, y))
    {
        let end = icon.hit.clone().expect("hit");
        return Some(Drag {
            grab: grab_from_end(&end),
            anchor: [icon.x + ICON * 0.5, icon.y + ICON * 0.5],
        });
    }
    let mut best: Option<(f32, &WireSpot, bool)> = None;
    for wire in &scene.wires {
        let (dist, near_end) = wire_distance(wire, x, y);
        if dist > 8.0 {
            continue;
        }
        if best.as_ref().is_none_or(|best| dist < best.0) {
            best = Some((dist, wire, near_end));
        }
    }
    best.map(|(_, wire, near_to)| {
        if near_to {
            Drag {
                grab: wire.to_grab.clone(),
                anchor: wire.from,
            }
        } else {
            Drag {
                grab: wire.from_grab.clone(),
                anchor: wire.to,
            }
        }
    })
}

pub fn target_at(scene: &Scene, x: f32, y: f32) -> Option<End> {
    if let Some(port) = nearest_port(scene, x, y, DOT) {
        return Some(port.end.clone());
    }
    scene
        .icons
        .iter()
        .find(|icon| icon.hit.is_some() && in_icon(icon, x, y))
        .and_then(|icon| icon.hit.clone())
}

fn grab_from_end(end: &End) -> Grab {
    match end {
        End::Capture { device } => Grab::Capture {
            device: device.clone(),
            radio: None,
        },
        End::Playback { device } => Grab::Playback {
            device: device.clone(),
            radio: None,
        },
        End::RadioTx { id } => Grab::RadioTx { id: *id },
        End::RadioRx { id } => Grab::RadioRx { id: *id },
    }
}

fn near(a: [f32; 2], b: [f32; 2]) -> bool {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    dx * dx + dy * dy < 1.0
}

fn nearest_port(scene: &Scene, x: f32, y: f32, radius: f32) -> Option<&PortSpot> {
    scene.ports.iter().find(|port| {
        let dx = port.at[0] - x;
        let dy = port.at[1] - y;
        dx * dx + dy * dy <= radius * radius
    })
}

fn in_icon(icon: &IconSpot, x: f32, y: f32) -> bool {
    x >= icon.x && x <= icon.x + ICON && y >= icon.y && y <= icon.y + ICON
}

fn wire_distance(wire: &WireSpot, x: f32, y: f32) -> (f32, bool) {
    let curve = curve_points(wire.from, wire.to);
    let mut best = f32::MAX;
    let mut at = 0.0;
    let mut step = 0.0;
    while step <= 1.0 {
        let point = cubic(curve, step);
        let dx = point[0] - x;
        let dy = point[1] - y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < best {
            best = dist;
            at = step;
        }
        step += 0.05;
    }
    (best, at > 0.5)
}

pub fn curve_points(from: [f32; 2], to: [f32; 2]) -> [[f32; 2]; 4] {
    let pull = (to[0] - from[0]).abs().max(48.0) * 0.45;
    let dir = if to[0] >= from[0] { 1.0 } else { -1.0 };
    [
        from,
        [from[0] + pull * dir, from[1]],
        [to[0] - pull * dir, to[1]],
        to,
    ]
}

fn cubic(points: [[f32; 2]; 4], t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    let mut out = [0.0, 0.0];
    for axis in 0..2 {
        out[axis] = u * u * u * points[0][axis]
            + 3.0 * u * u * t * points[1][axis]
            + 3.0 * u * t * t * points[2][axis]
            + t * t * t * points[3][axis];
    }
    out
}

fn row_height(nports: usize) -> f32 {
    (nports as f32 * 16.0 + 28.0).max(72.0)
}

fn shown_ports<'a>(
    ports: &'a std::collections::BTreeMap<String, Vec<String>>,
    node: &str,
    fallback: &'a [&'a str],
) -> Vec<&'a str> {
    if let Some(names) = ports.get(node) {
        if !names.is_empty() {
            return names.iter().map(String::as_str).collect();
        }
    }
    fallback.to_vec()
}

fn fallback_capture(device: &AudioNode) -> &'static [&'static str] {
    let blob = format!("{} {}", device.name, device.description).to_ascii_lowercase();
    if blob.contains("mono") {
        &["capture_MONO"]
    } else {
        &["capture_FL", "capture_FR"]
    }
}

fn device_title(device: &AudioNode) -> String {
    let text = if device.description.is_empty() {
        device.name.as_str()
    } else {
        device.description.as_str()
    };
    short(text)
}

fn short(text: &str) -> String {
    let mut count = 0;
    for (index, _) in text.char_indices() {
        count += 1;
        if count > 22 {
            return format!("{}…", &text[..index]);
        }
    }
    text.to_string()
}

fn stream_id(name: &str, prefix: &str, suffix: &str) -> Option<u64> {
    let rest = name.strip_prefix(prefix)?;
    let id = rest.strip_suffix(suffix)?;
    id.parse().ok()
}

fn port_leaf(port: &str) -> &str {
    port.split_once(':').map(|(_, leaf)| leaf).unwrap_or(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound() -> Vec<RadioBinding> {
        vec![RadioBinding {
            id: 1,
            input: "mic".into(),
            output: "spk".into(),
        }]
    }

    #[test]
    fn a_microphone_drop_sets_only_the_input() {
        let updates = endpoint_updates(
            &bound(),
            &Grab::Capture {
                device: "desk".into(),
                radio: None,
            },
            Some(&End::RadioTx { id: 1 }),
        );
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].input, "desk");
        assert_eq!(updates[0].output, "spk");
    }

    #[test]
    fn a_speaker_drop_sets_only_the_output() {
        let updates = endpoint_updates(
            &bound(),
            &Grab::RadioRx { id: 1 },
            Some(&End::Playback {
                device: "phones".into(),
            }),
        );
        assert_eq!(updates[0].input, "mic");
        assert_eq!(updates[0].output, "phones");
    }

    #[test]
    fn a_reversed_drop_is_refused() {
        let updates = endpoint_updates(
            &bound(),
            &Grab::Capture {
                device: "desk".into(),
                radio: None,
            },
            Some(&End::Playback {
                device: "phones".into(),
            }),
        );
        assert!(updates.is_empty());
    }

    #[test]
    fn an_empty_drop_clears_the_side_that_was_held() {
        let updates = endpoint_updates(&bound(), &Grab::RadioRx { id: 1 }, None);
        assert_eq!(updates[0].input, "mic");
        assert_eq!(updates[0].output, "");
        let updates = endpoint_updates(
            &bound(),
            &Grab::Capture {
                device: "mic".into(),
                radio: Some(1),
            },
            None,
        );
        assert_eq!(updates[0].input, "");
        assert_eq!(updates[0].output, "spk");
    }

    #[test]
    fn moving_the_speaker_end_of_a_wire_keeps_the_microphone() {
        let updates = endpoint_updates(
            &bound(),
            &Grab::Playback {
                device: "spk".into(),
                radio: Some(1),
            },
            Some(&End::Playback {
                device: "phones".into(),
            }),
        );
        assert_eq!(updates[0].input, "mic");
        assert_eq!(updates[0].output, "phones");
    }

    #[test]
    fn icon_names_follow_the_device_class() {
        assert_eq!(
            xdg_icon_name("Audio/Source", "alsa_input.usb", "USB Audio Device Mono"),
            "audio-input-microphone"
        );
        assert_eq!(
            xdg_icon_name(
                "Audio/Sink",
                "alsa_output.usb",
                "USB Audio Device Analog Stereo"
            ),
            "audio-speakers"
        );
        assert_eq!(
            xdg_icon_name("Audio/Sink", "bluez", "Headset"),
            "audio-headphones"
        );
        assert_eq!(
            xdg_icon_name("Stream/Input/Audio", "GNOME Settings", ""),
            "audio-card"
        );
    }
}
