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
    Capture {
        device: String,
    },
    Playback {
        device: String,
    },
    RadioTx {
        id: u64,
    },
    RadioRx {
        id: u64,
    },
    /// Left jack of the receive-pipe block.
    RxPipeIn {
        id: u64,
    },
    /// Right jack of the receive-pipe block.
    RxPipeOut {
        id: u64,
    },
    /// Left jack of the transmit-pipe block, toward the microphone.
    TxPipeIn {
        id: u64,
    },
    /// Right jack of the transmit-pipe block, toward the radio.
    TxPipeOut {
        id: u64,
    },
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
    /// The receive pipe is in the speaker path. The block replaces that wire.
    pub rx_pipe_live: bool,
    /// Program name drawn in the pipe block.
    pub rx_pipe_label: String,
    /// The transmit pipe is in the microphone path. The block replaces that wire.
    pub tx_pipe_live: bool,
    /// Program name drawn in the transmit-pipe block.
    pub tx_pipe_label: String,
    /// Discriminator tap. The left jack is a sound-card capture and the
    /// right jack is the demodulator. There is no virtual mic or speaker.
    pub tap: bool,
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
    /// Receive-pipe block drawn between the radio and the speaker.
    Pipe,
    /// Transmit-pipe block drawn between the microphone and the radio.
    TxPipe,
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

/// A cable drawn on the radio strip. It is not a PipeWire link and not a drag handle.
#[derive(Clone, Debug)]
pub struct FixedWire {
    pub from: [f32; 2],
    pub to: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub icons: Vec<IconSpot>,
    pub ports: Vec<PortSpot>,
    pub wires: Vec<WireSpot>,
    pub fixed: Vec<FixedWire>,
}

#[derive(Clone, Debug)]
pub struct Drag {
    pub grab: Grab,
    pub anchor: [f32; 2],
}

const ICON: f32 = 48.0;
/// Painted jack radius. The drop area is the jack plus its label.
pub const JACK: f32 = 8.0;
const PORT_STEP: f32 = 22.0;
/// Clear space kept between a radio and its own mic and speaker.
/// Wider windows used to stretch that hole with the column.
const RADIO_JOIN: f32 = 48.0;
/// The receive-pipe icon matches the other device icons. The demodulator
/// name is painted under it, the same way the radio and speaker names are.
pub const PIPE_W: f32 = ICON;

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
    let mut fixed = Vec::new();
    for radio in input.radios {
        if radio.tap {
            let leaf = recorded_capture(radio, input.captures);
            let row = row_height(1).max(78.0);
            let x0 = 8.0 + col;
            let (_, radio_x, _) = radio_icon_xs(x0, col);
            icons.push(IconSpot {
                x: radio_x,
                y: y_mid,
                kind: IconKind::Radio,
                xdg: String::new(),
                label: radio.title.clone(),
                hit: None,
            });
            let mid_y = y_mid + ICON * 0.5;
            push_port(
                &mut ports,
                PortSpot {
                    end: End::RadioTx { id: radio.id },
                    port: leaf.to_string(),
                    at: [radio_x - 28.0, mid_y],
                    label: channel_label(leaf).to_string(),
                    label_right: false,
                },
            );
            push_port(
                &mut ports,
                PortSpot {
                    end: End::RadioRx { id: radio.id },
                    port: "output_FL".into(),
                    at: [radio_x + ICON + 28.0, mid_y],
                    label: "FL".into(),
                    label_right: true,
                },
            );
            if radio.rx_pipe_live && !radio.output.is_empty() {
                let speaker_ports =
                    shown_ports(input.ports, &radio.output, &["playback_FL", "playback_FR"]);
                place_rx_pipe(
                    &mut icons,
                    &mut ports,
                    radio,
                    &["output_FL"],
                    &speaker_ports,
                    y_mid,
                    radio_x + ICON + 28.0,
                    playback_port_x(8.0 + 2.0 * col, col),
                );
            }
            y_mid += row;
            continue;
        }
        let tx_ports = shown_ports(
            input.ports,
            &audio::tx_stream(radio.id),
            &["input_FL", "input_FR"],
        );
        let rx_ports = shown_ports(
            input.ports,
            &audio::rx_stream(radio.id),
            radio_rx_fallback(radio),
        );
        let row = row_height(tx_ports.len().max(rx_ports.len())).max(78.0);
        let x0 = 8.0 + col;
        let (tx_x, radio_x, rx_x) = radio_icon_xs(x0, col);
        place_radio_side(
            &mut icons,
            &mut ports,
            tx_x,
            y_mid,
            IconKind::Mic,
            &radio.tx_title,
            End::RadioTx { id: radio.id },
            &tx_ports,
            true,
        );
        icons.push(IconSpot {
            x: radio_x,
            y: y_mid,
            kind: IconKind::Radio,
            xdg: String::new(),
            label: radio.title.clone(),
            hit: None,
        });
        place_radio_side(
            &mut icons,
            &mut ports,
            rx_x,
            y_mid,
            IconKind::Speaker,
            &radio.rx_title,
            End::RadioRx { id: radio.id },
            &rx_ports,
            false,
        );
        let mid_y = y_mid + ICON * 0.5;
        fixed.push(FixedWire {
            from: [tx_x + ICON, mid_y],
            to: [radio_x, mid_y],
        });
        fixed.push(FixedWire {
            from: [radio_x + ICON, mid_y],
            to: [rx_x, mid_y],
        });
        if radio.rx_pipe_live && !radio.output.is_empty() {
            let speaker_ports =
                shown_ports(input.ports, &radio.output, &["playback_FL", "playback_FR"]);
            place_rx_pipe(
                &mut icons,
                &mut ports,
                radio,
                &rx_ports,
                &speaker_ports,
                y_mid,
                rx_port_x(x0, col),
                playback_port_x(8.0 + 2.0 * col, col),
            );
        }
        if radio.tx_pipe_live && !radio.input.is_empty() {
            let fallback = input
                .captures
                .iter()
                .find(|device| device.name == radio.input)
                .map(fallback_capture)
                .unwrap_or(&["capture_FL", "capture_FR"]);
            let mic_ports = shown_ports(input.ports, &radio.input, fallback);
            place_tx_pipe(
                &mut icons,
                &mut ports,
                radio,
                &mic_ports,
                &tx_ports,
                y_mid,
                capture_port_x(),
                radio_tx_port_x(col),
            );
        }
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
        fixed,
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
        icon_x + ICON + 28.0
    } else {
        icon_x - 28.0
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
        x - 28.0
    } else {
        x + ICON + 28.0
    };
    place_ports(ports, end, names, port_x, y + ICON * 0.5, !ports_on_left);
}

fn capture_port_x() -> f32 {
    8.0 + ICON + 28.0
}

/// Mic, radio, and speaker x for one radio row. Side icons move inward until
/// `RADIO_JOIN` of cable remains. A column that is already tighter stays put.
fn radio_icon_xs(x0: f32, col: f32) -> (f32, f32, f32) {
    let tx = x0 + 28.0;
    let radio = x0 + col * 0.5 - ICON * 0.5;
    let rx = x0 + col - ICON - 28.0;
    let gap = radio - (tx + ICON);
    let pull = (gap - RADIO_JOIN).max(0.0);
    (tx + pull, radio, rx - pull)
}

fn radio_tx_port_x(col: f32) -> f32 {
    let x0 = 8.0 + col;
    let (tx, _, _) = radio_icon_xs(x0, col);
    tx - 28.0
}

fn rx_port_x(x0: f32, col: f32) -> f32 {
    let (_, _, rx) = radio_icon_xs(x0, col);
    rx + ICON + 28.0
}

fn playback_port_x(x0: f32, col: f32) -> f32 {
    let icon_x = x0 + col - ICON - 8.0;
    icon_x - 28.0
}

/// Block between the radio's receive ports and the speaker ports.
/// Each receive channel gets a left jack and a right jack.
fn place_rx_pipe(
    icons: &mut Vec<IconSpot>,
    ports: &mut Vec<PortSpot>,
    radio: &BayRadio,
    rx_ports: &[&str],
    speaker_ports: &[&str],
    y: f32,
    rx_x: f32,
    speaker_x: f32,
) {
    let pairs = pipe_pairs(rx_ports, speaker_ports);
    if pairs.is_empty() {
        return;
    }
    let ys: Vec<f32> = pairs
        .iter()
        .filter_map(|(rx_leaf, _)| port_at(ports, &End::RadioRx { id: radio.id }, rx_leaf))
        .map(|at| at[1])
        .collect();
    let center_y = if ys.is_empty() {
        y + ICON * 0.5
    } else {
        ys.iter().sum::<f32>() / ys.len() as f32
    };
    let pipe_x = (rx_x + speaker_x) * 0.5 - PIPE_W * 0.5;
    icons.push(IconSpot {
        x: pipe_x,
        y: center_y - ICON * 0.5,
        kind: IconKind::Pipe,
        xdg: String::new(),
        label: short(if radio.rx_pipe_label.is_empty() {
            "RX pipe"
        } else {
            &radio.rx_pipe_label
        }),
        hit: None,
    });
    let left = pipe_x - 10.0;
    let right = pipe_x + PIPE_W + 10.0;
    for (rx_leaf, speaker_leaf) in &pairs {
        let Some(at) = port_at(ports, &End::RadioRx { id: radio.id }, rx_leaf) else {
            continue;
        };
        push_port(
            ports,
            PortSpot {
                end: End::RxPipeIn { id: radio.id },
                port: format!("in:{rx_leaf}"),
                at: [left, at[1]],
                label: channel_label(rx_leaf).to_string(),
                label_right: false,
            },
        );
        push_port(
            ports,
            PortSpot {
                end: End::RxPipeOut { id: radio.id },
                port: format!("out:{speaker_leaf}"),
                at: [right, at[1]],
                label: channel_label(speaker_leaf).to_string(),
                label_right: true,
            },
        );
    }
}

/// Block between the microphone ports and the radio's transmit ports.
fn place_tx_pipe(
    icons: &mut Vec<IconSpot>,
    ports: &mut Vec<PortSpot>,
    radio: &BayRadio,
    mic_ports: &[&str],
    tx_ports: &[&str],
    y: f32,
    mic_x: f32,
    tx_x: f32,
) {
    let pairs = pipe_pairs(mic_ports, tx_ports);
    if pairs.is_empty() {
        return;
    }
    let ys: Vec<f32> = pairs
        .iter()
        .filter_map(|(_, tx_leaf)| port_at(ports, &End::RadioTx { id: radio.id }, tx_leaf))
        .map(|at| at[1])
        .collect();
    let center_y = if ys.is_empty() {
        y + ICON * 0.5
    } else {
        ys.iter().sum::<f32>() / ys.len() as f32
    };
    let pipe_x = (mic_x + tx_x) * 0.5 - PIPE_W * 0.5;
    icons.push(IconSpot {
        x: pipe_x,
        y: center_y - ICON * 0.5,
        kind: IconKind::TxPipe,
        xdg: String::new(),
        label: short(if radio.tx_pipe_label.is_empty() {
            "TX pipe"
        } else {
            &radio.tx_pipe_label
        }),
        hit: None,
    });
    let left = pipe_x - 10.0;
    let right = pipe_x + PIPE_W + 10.0;
    for (mic_leaf, tx_leaf) in &pairs {
        let Some(at) = port_at(ports, &End::RadioTx { id: radio.id }, tx_leaf) else {
            continue;
        };
        push_port(
            ports,
            PortSpot {
                end: End::TxPipeIn { id: radio.id },
                port: format!("in:{mic_leaf}"),
                at: [left, at[1]],
                label: channel_label(mic_leaf).to_string(),
                label_right: false,
            },
        );
        push_port(
            ports,
            PortSpot {
                end: End::TxPipeOut { id: radio.id },
                port: format!("out:{tx_leaf}"),
                at: [right, at[1]],
                label: channel_label(tx_leaf).to_string(),
                label_right: true,
            },
        );
    }
}

fn push_port(ports: &mut Vec<PortSpot>, spot: PortSpot) {
    if ports
        .iter()
        .any(|have| have.end == spot.end && have.port == spot.port)
    {
        return;
    }
    ports.push(spot);
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
        let at_y = mid_y + (i as f32 - (n - 1.0) * 0.5) * PORT_STEP;
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
            // The pipe block owns this path. The direct wire is not drawn.
            if radio.rx_pipe_live {
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
            // The transmit pipe owns this path. The direct wire is not drawn.
            if radio.tx_pipe_live {
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
    // A tap has no hamspot card, so the saved capture is the wire.
    for radio in input.radios {
        if !radio.tap || radio.input.is_empty() {
            continue;
        }
        let leaf = recorded_capture(radio, input.captures);
        let from = port_at(
            ports,
            &End::Capture {
                device: radio.input.clone(),
            },
            leaf,
        );
        let to = port_at(ports, &End::RadioTx { id: radio.id }, leaf);
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        wires.push(WireSpot {
            from,
            to,
            from_grab: Grab::Capture {
                device: radio.input.clone(),
                radio: Some(radio.id),
            },
            to_grab: Grab::RadioTx { id: radio.id },
        });
    }
    // The stored speaker is drawn before the pipe process diverts. A drop
    // onto a speaker has nowhere to show until that process is up, because
    // a configured pipe does not link the raw stream.
    for radio in input.radios {
        if radio.rx_pipe_live || radio.output.is_empty() {
            continue;
        }
        if wires.iter().any(|wire| {
            matches!(wire.from_grab, Grab::RadioRx { id } if id == radio.id)
        }) {
            continue;
        }
        let rx_ports = shown_ports(
            input.ports,
            &audio::rx_stream(radio.id),
            radio_rx_fallback(radio),
        );
        let speaker_ports =
            shown_ports(input.ports, &radio.output, &["playback_FL", "playback_FR"]);
        for (rx_leaf, speaker_leaf) in pipe_pairs(&rx_ports, &speaker_ports) {
            let from = port_at(ports, &End::RadioRx { id: radio.id }, &rx_leaf);
            let to = port_at(
                ports,
                &End::Playback {
                    device: radio.output.clone(),
                },
                &speaker_leaf,
            );
            let (Some(from), Some(to)) = (from, to) else {
                continue;
            };
            wires.push(WireSpot {
                from,
                to,
                from_grab: Grab::RadioRx { id: radio.id },
                to_grab: Grab::Playback {
                    device: radio.output.clone(),
                    radio: Some(radio.id),
                },
            });
        }
    }
    for radio in input.radios {
        if !radio.rx_pipe_live || radio.output.is_empty() {
            continue;
        }
        let rx_ports = shown_ports(
            input.ports,
            &audio::rx_stream(radio.id),
            radio_rx_fallback(radio),
        );
        let speaker_ports =
            shown_ports(input.ports, &radio.output, &["playback_FL", "playback_FR"]);
        for (rx_leaf, speaker_leaf) in pipe_pairs(&rx_ports, &speaker_ports) {
            let from = port_at(ports, &End::RadioRx { id: radio.id }, &rx_leaf);
            let into = port_at(
                ports,
                &End::RxPipeIn { id: radio.id },
                &format!("in:{rx_leaf}"),
            );
            let out_of = port_at(
                ports,
                &End::RxPipeOut { id: radio.id },
                &format!("out:{speaker_leaf}"),
            );
            let to = port_at(
                ports,
                &End::Playback {
                    device: radio.output.clone(),
                },
                &speaker_leaf,
            );
            let (Some(from), Some(into), Some(out_of), Some(to)) = (from, into, out_of, to) else {
                continue;
            };
            wires.push(WireSpot {
                from,
                to: into,
                from_grab: Grab::RadioRx { id: radio.id },
                to_grab: Grab::RadioRx { id: radio.id },
            });
            wires.push(WireSpot {
                from: out_of,
                to,
                from_grab: Grab::Playback {
                    device: radio.output.clone(),
                    radio: Some(radio.id),
                },
                to_grab: Grab::Playback {
                    device: radio.output.clone(),
                    radio: Some(radio.id),
                },
            });
        }
    }
    for radio in input.radios {
        if !radio.tx_pipe_live || radio.input.is_empty() {
            continue;
        }
        let fallback = input
            .captures
            .iter()
            .find(|device| device.name == radio.input)
            .map(fallback_capture)
            .unwrap_or(&["capture_FL", "capture_FR"]);
        let mic_ports = shown_ports(input.ports, &radio.input, fallback);
        let tx_ports = shown_ports(
            input.ports,
            &audio::tx_stream(radio.id),
            &["input_FL", "input_FR"],
        );
        for (mic_leaf, tx_leaf) in pipe_pairs(&mic_ports, &tx_ports) {
            let from = port_at(
                ports,
                &End::Capture {
                    device: radio.input.clone(),
                },
                &mic_leaf,
            );
            let into = port_at(
                ports,
                &End::TxPipeIn { id: radio.id },
                &format!("in:{mic_leaf}"),
            );
            let out_of = port_at(
                ports,
                &End::TxPipeOut { id: radio.id },
                &format!("out:{tx_leaf}"),
            );
            let to = port_at(ports, &End::RadioTx { id: radio.id }, &tx_leaf);
            let (Some(from), Some(into), Some(out_of), Some(to)) = (from, into, out_of, to) else {
                continue;
            };
            wires.push(WireSpot {
                from,
                to: into,
                from_grab: Grab::Capture {
                    device: radio.input.clone(),
                    radio: Some(radio.id),
                },
                to_grab: Grab::Capture {
                    device: radio.input.clone(),
                    radio: Some(radio.id),
                },
            });
            wires.push(WireSpot {
                from: out_of,
                to,
                from_grab: Grab::RadioTx { id: radio.id },
                to_grab: Grab::RadioTx { id: radio.id },
            });
        }
    }
    wires
}

/// Left and right channels, in order. A mono side fans out to every channel
/// on the other side, the same way `audio::pair_ports` does.
fn pipe_pairs(rx: &[&str], speaker: &[&str]) -> Vec<(String, String)> {
    if rx.is_empty() || speaker.is_empty() {
        return Vec::new();
    }
    if rx.len() == 1 {
        return speaker
            .iter()
            .map(|port| (rx[0].to_string(), (*port).to_string()))
            .collect();
    }
    if speaker.len() == 1 {
        return rx
            .iter()
            .map(|port| ((*port).to_string(), speaker[0].to_string()))
            .collect();
    }
    rx.iter()
        .zip(speaker.iter())
        .map(|(left, right)| ((*left).to_string(), (*right).to_string()))
        .collect()
}

fn channel_label(port: &str) -> &str {
    if port.ends_with("_FL") || port == "FL" {
        "FL"
    } else if port.ends_with("_FR") || port == "FR" {
        "FR"
    } else if port.ends_with("MONO") {
        "MONO"
    } else {
        port
    }
}

pub fn over_rx_pipe(scene: &Scene, x: f32, y: f32) -> bool {
    if scene.icons.iter().any(|icon| {
        icon.kind == IconKind::Pipe
            && x >= icon.x
            && x <= icon.x + PIPE_W
            && y >= icon.y
            && y <= icon.y + ICON
    }) {
        return true;
    }
    scene
        .ports
        .iter()
        .any(|port| pipe_end(&port.end) && port_covers(port, x, y))
}

fn pipe_end(end: &End) -> bool {
    matches!(
        end,
        End::RxPipeIn { .. } | End::RxPipeOut { .. } | End::TxPipeIn { .. } | End::TxPipeOut { .. }
    )
}

pub fn over_tx_pipe(scene: &Scene, x: f32, y: f32) -> bool {
    if scene.icons.iter().any(|icon| {
        icon.kind == IconKind::TxPipe
            && x >= icon.x
            && x <= icon.x + PIPE_W
            && y >= icon.y
            && y <= icon.y + ICON
    }) {
        return true;
    }
    scene
        .ports
        .iter()
        .any(|port| {
            matches!(port.end, End::TxPipeIn { .. } | End::TxPipeOut { .. })
                && port_covers(port, x, y)
        })
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
    if let Some(port) = nearest_port(scene, x, y) {
        // The pipe jacks stay put. The wires on either side are the handles.
        if pipe_end(&port.end) {
            return None;
        }
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
    if let Some(port) = nearest_port(scene, x, y) {
        if !pipe_end(&port.end) {
            return Some(port.end.clone());
        }
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
        // Pipe jacks are not handles. `grab_at` returns before this arm.
        End::RxPipeIn { id } | End::RxPipeOut { id } => Grab::RadioRx { id: *id },
        End::TxPipeIn { id } | End::TxPipeOut { id } => Grab::RadioTx { id: *id },
    }
}

fn near(a: [f32; 2], b: [f32; 2]) -> bool {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    dx * dx + dy * dy < 1.0
}

fn nearest_port(scene: &Scene, x: f32, y: f32) -> Option<&PortSpot> {
    scene
        .ports
        .iter()
        .filter(|port| port_covers(port, x, y))
        .min_by(|left, right| {
            let left = dist2(left.at, x, y);
            let right = dist2(right.at, x, y);
            left.total_cmp(&right)
        })
}

/// The jack, and the channel name painted beside it. The name is where a
/// wire is dropped; the dot alone is too small to hit.
fn port_covers(port: &PortSpot, x: f32, y: f32) -> bool {
    let dx = port.at[0] - x;
    let dy = port.at[1] - y;
    let reach = JACK * 2.4;
    if dx * dx + dy * dy <= reach * reach {
        return true;
    }
    let text = port.label.chars().count() as f32 * 6.4 + 14.0;
    let (left, right) = if port.label_right {
        (port.at[0] - JACK, port.at[0] + text)
    } else {
        (port.at[0] - text, port.at[0] + JACK)
    };
    x >= left && x <= right && dy.abs() <= PORT_STEP * 0.45
}

fn dist2(at: [f32; 2], x: f32, y: f32) -> f32 {
    let dx = at[0] - x;
    let dy = at[1] - y;
    dx * dx + dy * dy
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
    (nports as f32 * PORT_STEP + 36.0).max(78.0)
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

fn recorded_capture(radio: &BayRadio, captures: &[AudioNode]) -> &'static str {
    match captures.iter().find(|device| device.name == radio.input) {
        Some(device) => crate::rx_pipe::capture_channel(&device.name, &device.description),
        None => "capture_FL",
    }
}

fn radio_rx_fallback(radio: &BayRadio) -> &'static [&'static str] {
    if radio.tap {
        &["output_FL"]
    } else {
        &["output_FL", "output_FR"]
    }
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
    use crate::audio::{AudioLink, AudioNode};

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

    fn sample_radio(live: bool) -> BayRadio {
        BayRadio {
            id: 1,
            title: "1 Hamlib Dummy".into(),
            rx_title: "Hamlib Dummy 1 RX".into(),
            tx_title: "Hamlib Dummy 1 TX".into(),
            input: String::new(),
            output: "spk".into(),
            rx_pipe_live: live,
            rx_pipe_label: "dsd-fme".into(),
            tx_pipe_live: false,
            tx_pipe_label: String::new(),
            tap: false,
        }
    }

    fn sample_scene(live: bool, links: &[AudioLink]) -> Scene {
        let playback = [AudioNode {
            name: "spk".into(),
            description: "RCA".into(),
            class: "Audio/Sink".into(),
        }];
        let radios = [sample_radio(live)];
        let ports = std::collections::BTreeMap::new();
        layout(
            900.0,
            &BayIn {
                captures: &[],
                playbacks: &playback,
                radios: &radios,
                links,
                ports: &ports,
            },
        )
    }

    #[test]
    fn a_live_rx_pipe_is_a_block_between_the_radio_and_the_speaker() {
        let scene = sample_scene(true, &[]);
        let pipe = scene
            .icons
            .iter()
            .find(|icon| icon.kind == IconKind::Pipe)
            .expect("pipe block");
        assert_eq!(pipe.label, "dsd-fme");
        let inputs: Vec<_> = scene
            .ports
            .iter()
            .filter(|port| matches!(port.end, End::RxPipeIn { id: 1 }))
            .collect();
        let outputs: Vec<_> = scene
            .ports
            .iter()
            .filter(|port| matches!(port.end, End::RxPipeOut { id: 1 }))
            .collect();
        assert_eq!(
            inputs
                .iter()
                .map(|port| port.label.as_str())
                .collect::<Vec<_>>(),
            vec!["FL", "FR"]
        );
        assert_eq!(
            outputs
                .iter()
                .map(|port| port.label.as_str())
                .collect::<Vec<_>>(),
            vec!["FL", "FR"]
        );
        assert!(inputs.iter().all(|port| port.at[0] < pipe.x));
        assert!(outputs.iter().all(|port| port.at[0] > pipe.x + PIPE_W));
        assert_eq!(scene.wires.len(), 4);
        assert!(scene.wires.iter().all(|wire| wire.from[0] < wire.to[0]));
        let rx = scene
            .ports
            .iter()
            .find(|port| matches!(port.end, End::RadioRx { id: 1 }) && port.port == "output_FL")
            .expect("rx fl");
        let speaker = scene
            .ports
            .iter()
            .find(|port| {
                matches!(port.end, End::Playback { ref device } if device == "spk")
                    && port.port == "playback_FL"
            })
            .expect("speaker fl");
        assert!(rx.at[0] < pipe.x && pipe.x + PIPE_W < speaker.at[0]);
        assert!(over_rx_pipe(
            &scene,
            pipe.x + PIPE_W * 0.5,
            pipe.y + ICON * 0.5
        ));
        assert!(grab_at(&scene, inputs[0].at[0], inputs[0].at[1]).is_none());
    }

    #[test]
    fn a_chosen_speaker_is_drawn_before_the_rx_pipe_is_up() {
        let scene = sample_scene(false, &[]);
        assert!(scene.icons.iter().all(|icon| icon.kind != IconKind::Pipe));
        assert!(scene.wires.iter().any(|wire| {
            matches!(wire.from_grab, Grab::RadioRx { id: 1 })
                && matches!(
                    &wire.to_grab,
                    Grab::Playback { device, radio: Some(1) } if device == "spk"
                )
        }));
    }

    fn tap_scene(live: bool, mono: bool) -> Scene {
        let mut radio = sample_radio(live);
        radio.tap = true;
        radio.input = "line".into();
        radio.output = "spk".into();
        radio.title = "1 Audio Desk".into();
        let capture = [AudioNode {
            name: "line".into(),
            description: if mono { "USB Mono".into() } else { "Line".into() },
            class: "Audio/Source".into(),
        }];
        let playback = [AudioNode {
            name: "spk".into(),
            description: "RCA".into(),
            class: "Audio/Sink".into(),
        }];
        let radios = [radio];
        layout(
            900.0,
            &BayIn {
                captures: &capture,
                playbacks: &playback,
                radios: &radios,
                links: &[],
                ports: &std::collections::BTreeMap::new(),
            },
        )
    }

    #[test]
    fn an_audio_tap_draws_the_capture_into_the_demodulator() {
        let quiet = tap_scene(false, false);
        assert!(quiet.fixed.is_empty());
        assert!(quiet
            .icons
            .iter()
            .all(|icon| icon.kind != IconKind::Mic && icon.kind != IconKind::Speaker));
        assert!(quiet.icons.iter().any(|icon| icon.kind == IconKind::Radio));
        assert!(quiet.icons.iter().all(|icon| icon.kind != IconKind::Pipe));
        assert!(quiet
            .ports
            .iter()
            .all(|port| !port.port.contains("hamspot") && !port.label.contains("hamspot")));
        let leaf = "capture_FL";
        let from = port_at(
            &quiet.ports,
            &End::Capture {
                device: "line".into(),
            },
            leaf,
        )
        .expect("capture");
        let into = port_at(&quiet.ports, &End::RadioTx { id: 1 }, leaf).expect("tap in");
        assert!(quiet.wires.iter().any(|wire| wire.from == from && wire.to == into));
        assert!(quiet.wires.iter().any(|wire| {
            matches!(wire.from_grab, Grab::RadioRx { id: 1 })
                && matches!(wire.to_grab, Grab::Playback { radio: Some(1), .. })
        }));

        let live = tap_scene(true, false);
        assert!(live.icons.iter().any(|icon| icon.kind == IconKind::Pipe));
        assert!(live.wires.iter().any(|wire| {
            matches!(
                &wire.from_grab,
                Grab::Capture {
                    device,
                    radio: Some(1)
                } if device == "line"
            ) && matches!(wire.to_grab, Grab::RadioTx { id: 1 })
        }));
        assert!(!live.wires.iter().any(|wire| {
            matches!(wire.from_grab, Grab::RadioRx { id: 1 })
                && matches!(wire.to_grab, Grab::Playback { .. })
        }));

        let mono = tap_scene(false, true);
        assert!(port_at(&mono.ports, &End::RadioTx { id: 1 }, "capture_MONO").is_some());
        assert!(mono.wires.iter().any(|wire| {
            matches!(wire.to_grab, Grab::RadioTx { id: 1 })
        }));
    }

    #[test]
    fn a_live_tx_pipe_is_a_block_between_the_microphone_and_the_radio() {
        let mut radio = sample_radio(false);
        radio.input = "mic".into();
        radio.tx_pipe_live = true;
        radio.tx_pipe_label = "am".into();
        let capture = [AudioNode {
            name: "mic".into(),
            description: "Desk".into(),
            class: "Audio/Source".into(),
        }];
        let radios = [radio];
        let links = [
            AudioLink {
                output: "mic:capture_FL".into(),
                input: "input.hamspot_1_tx:input_FL".into(),
            },
            AudioLink {
                output: "mic:capture_FR".into(),
                input: "input.hamspot_1_tx:input_FR".into(),
            },
        ];
        let scene = layout(
            900.0,
            &BayIn {
                captures: &capture,
                playbacks: &[],
                radios: &radios,
                links: &links,
                ports: &std::collections::BTreeMap::new(),
            },
        );
        let pipe = scene
            .icons
            .iter()
            .find(|icon| icon.kind == IconKind::TxPipe)
            .expect("tx pipe block");
        assert_eq!(pipe.label, "am");
        let inputs: Vec<_> = scene
            .ports
            .iter()
            .filter(|port| matches!(port.end, End::TxPipeIn { id: 1 }))
            .collect();
        let outputs: Vec<_> = scene
            .ports
            .iter()
            .filter(|port| matches!(port.end, End::TxPipeOut { id: 1 }))
            .collect();
        assert_eq!(
            inputs
                .iter()
                .map(|port| port.label.as_str())
                .collect::<Vec<_>>(),
            vec!["FL", "FR"]
        );
        assert!(inputs.iter().all(|port| port.at[0] < pipe.x));
        assert!(outputs.iter().all(|port| port.at[0] > pipe.x + PIPE_W));
        assert!(scene.wires.iter().all(|wire| wire.from[0] < wire.to[0]));
        assert!(over_tx_pipe(
            &scene,
            pipe.x + PIPE_W * 0.5,
            pipe.y + ICON * 0.5
        ));
        assert!(grab_at(&scene, inputs[0].at[0], inputs[0].at[1]).is_none());
        let direct = scene.wires.iter().any(|wire| {
            wire.from_grab
                == Grab::Capture {
                    device: "mic".into(),
                    radio: Some(1),
                }
                && wire.to_grab == Grab::RadioTx { id: 1 }
        });
        assert!(!direct);
    }

    #[test]
    fn a_drop_on_the_speaker_label_hits_that_speaker() {
        let scene = sample_scene(false, &[]);
        let port = scene
            .ports
            .iter()
            .find(|port| {
                matches!(port.end, End::Playback { ref device } if device == "spk")
                    && port.port == "playback_FL"
            })
            .expect("speaker fl");
        let on_label = port.at[0] - 36.0;
        let hit = target_at(&scene, on_label, port.at[1]).expect("label is a target");
        assert_eq!(
            hit,
            End::Playback {
                device: "spk".into()
            }
        );
        assert!(grab_at(&scene, on_label, port.at[1]).is_some());
    }

    #[test]
    fn the_radio_icons_close_up_and_the_join_is_not_a_handle() {
        let radios = [sample_radio(false)];
        let ports = std::collections::BTreeMap::new();
        let scene = layout(
            1400.0,
            &BayIn {
                captures: &[],
                playbacks: &[],
                radios: &radios,
                links: &[],
                ports: &ports,
            },
        );
        let mic = scene
            .icons
            .iter()
            .find(|icon| icon.kind == IconKind::Mic)
            .expect("tx icon");
        let radio = scene
            .icons
            .iter()
            .find(|icon| icon.kind == IconKind::Radio)
            .expect("radio icon");
        let speaker = scene
            .icons
            .iter()
            .find(|icon| icon.kind == IconKind::Speaker)
            .expect("rx icon");
        let left = radio.x - (mic.x + ICON);
        let right = speaker.x - (radio.x + ICON);
        assert!((left - RADIO_JOIN).abs() < 0.01, "left gap {left}");
        assert!((right - RADIO_JOIN).abs() < 0.01, "right gap {right}");
        assert!(scene.wires.is_empty());
        assert_eq!(scene.fixed.len(), 2);
        assert!((scene.fixed[0].from[0] - (mic.x + ICON)).abs() < 0.01);
        assert!((scene.fixed[0].to[0] - radio.x).abs() < 0.01);
        assert!((scene.fixed[1].from[0] - (radio.x + ICON)).abs() < 0.01);
        assert!((scene.fixed[1].to[0] - speaker.x).abs() < 0.01);
        assert!((scene.fixed[0].from[1] - scene.fixed[0].to[1]).abs() < 0.01);
        let mid_x = (scene.fixed[0].from[0] + scene.fixed[0].to[0]) * 0.5;
        assert!(grab_at(&scene, mid_x, scene.fixed[0].from[1]).is_none());
        let narrow = layout(
            680.0,
            &BayIn {
                captures: &[],
                playbacks: &[],
                radios: &radios,
                links: &[],
                ports: &ports,
            },
        );
        let mic = narrow
            .icons
            .iter()
            .find(|icon| icon.kind == IconKind::Mic)
            .expect("tx icon");
        let radio = narrow
            .icons
            .iter()
            .find(|icon| icon.kind == IconKind::Radio)
            .expect("radio icon");
        let speaker = narrow
            .icons
            .iter()
            .find(|icon| icon.kind == IconKind::Speaker)
            .expect("rx icon");
        assert!(mic.x + ICON <= radio.x + 0.01);
        assert!(radio.x + ICON <= speaker.x + 0.01);
        assert_eq!(narrow.fixed.len(), 2);
    }

    #[test]
    fn a_quiet_radio_keeps_the_direct_speaker_wire() {
        let links = [
            AudioLink {
                output: "output.hamspot_1_rx:output_FL".into(),
                input: "spk:playback_FL".into(),
            },
            AudioLink {
                output: "output.hamspot_1_rx:output_FR".into(),
                input: "spk:playback_FR".into(),
            },
        ];
        let scene = sample_scene(false, &links);
        assert!(scene.icons.iter().all(|icon| icon.kind != IconKind::Pipe));
        assert_eq!(scene.wires.len(), 2);
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
