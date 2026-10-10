//! Read-only wire picture for the Automation frame.
//!
//! The Config tab is what adds and removes a radio. This canvas draws the
//! match: each hamlib radio and each rotator on the left, Gpredict's hardware
//! files on the right, and a wire where a `hamspot_` file belongs to that row.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Radio,
    Rotator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub title: String,
    pub stem: String,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HwFile {
    pub stem: String,
    pub owned: bool,
}

#[derive(Clone, Debug)]
pub struct Card {
    pub title: String,
    pub origin: [f32; 2],
    pub port: [f32; 2],
    pub kind: Kind,
    pub wired: bool,
}

#[derive(Clone, Debug)]
pub struct FilePort {
    pub stem: String,
    pub at: [f32; 2],
    pub owned: bool,
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub cards: Vec<Card>,
    pub panel: [f32; 4],
    pub files: Vec<FilePort>,
    pub wires: Vec<([f32; 2], [f32; 2])>,
}

const CARD_H: f32 = 56.0;
const GAP: f32 = 8.0;

pub fn layout(width: f32, devices: &[Device], files: &[HwFile]) -> Scene {
    let width = width.max(560.0);
    let left_w = (width * 0.42).clamp(180.0, 280.0);
    let mut cards = Vec::new();
    let mut y = 8.0;
    for device in devices {
        let origin = [8.0, y];
        let port = [8.0 + left_w - 16.0, y + CARD_H * 0.5];
        cards.push(Card {
            title: device.title.clone(),
            origin,
            port,
            kind: device.kind,
            wired: !device.stem.is_empty(),
        });
        y += CARD_H + GAP;
    }
    let panel_x = width - left_w - 8.0;
    let file_top = 36.0;
    let mut ports = Vec::new();
    let mut file_y = file_top;
    for file in files {
        ports.push(FilePort {
            stem: file.stem.clone(),
            at: [panel_x + 16.0, file_y],
            owned: file.owned,
        });
        file_y += 22.0;
    }
    let panel_h = (file_y + 8.0).max(88.0);
    let height = y.max(panel_h + 8.0).max(96.0);
    let mut wires = Vec::new();
    for (card, device) in cards.iter().zip(devices.iter()) {
        if device.stem.is_empty() {
            continue;
        }
        if let Some(port) = ports.iter().find(|port| port.stem == device.stem) {
            wires.push((card.port, port.at));
        }
    }
    Scene {
        width,
        height,
        cards,
        panel: [panel_x, 8.0, left_w, panel_h - 8.0],
        files: ports,
        wires,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owned_stem_draws_one_wire_and_a_manual_file_does_not() {
        let devices = vec![
            Device {
                title: "FT-450D".into(),
                stem: "hamspot_FT-450D".into(),
                kind: Kind::Radio,
            },
            Device {
                title: "Waiting".into(),
                stem: String::new(),
                kind: Kind::Radio,
            },
        ];
        let files = vec![
            HwFile {
                stem: "hamspot_FT-450D".into(),
                owned: true,
            },
            HwFile {
                stem: "MyRig".into(),
                owned: false,
            },
        ];
        let scene = layout(640.0, &devices, &files);
        assert_eq!(scene.wires.len(), 1);
        assert_eq!(scene.files.len(), 2);
        assert!(scene
            .files
            .iter()
            .any(|file| file.stem == "MyRig" && !file.owned));
    }
}
