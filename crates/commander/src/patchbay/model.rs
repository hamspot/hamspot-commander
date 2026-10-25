//! RF patch bay model: devices, jacks, cables, and path tracing.
//! Pure data and functions. Nothing here talks to hardware (#92, #94).

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Hamlib amplifier models from `ampctl -l` (Hamlib 4.6.5). (model, label, beta)
pub const HAMLIB_AMPS: &[(i32, &str, bool)] = &[
    (1, "Hamlib Dummy", false),
    (2, "NET ampctl", false),
    (201, "Elecraft KPA1500", true),
    (301, "Gemini DX1200/HF-1K", false),
    (401, "Expert 1.3K/1.5K/2K-FA", true),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Radio,
    Amplifier,
    Switch,
    Tuner,
    Meter,
    Load,
    Antenna,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Radio => "Radio",
            Kind::Amplifier => "Amplifier",
            Kind::Switch => "Antenna switch",
            Kind::Tuner => "Tuner",
            Kind::Meter => "SWR/power meter",
            Kind::Load => "Dummy load",
            Kind::Antenna => "Antenna",
        }
    }
}

/// Direction in the transmit sense. RF received flows the other way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    In,
    Out,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Coax,
    Control,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub name: String,
    pub dir: Dir,
    pub signal: Signal,
}

fn port(name: &str, dir: Dir, signal: Signal) -> Port {
    Port { name: name.into(), dir, signal }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub id: u32,
    pub kind: Kind,
    pub name: String,
    /// Hamlib amplifier model number. 0 means a user entry.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub model: i32,
    /// Radio row name in the station this device stands for.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub radio: String,
    /// The radio can transmit in this station.
    #[serde(default)]
    pub tx: bool,
    /// Band coverage note for an antenna, tuner or amplifier.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bands: String,
    /// Switch positions (1..=8).
    #[serde(default = "default_ways")]
    pub ways: u8,
    /// Selected switch position, 1-based.
    #[serde(default = "default_sel")]
    pub sel: u8,
    /// Top-left corner on the panel, in panel points.
    pub pos: [f32; 2],
}

fn default_ways() -> u8 {
    4
}
fn default_sel() -> u8 {
    1
}
fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

impl Device {
    pub fn new(id: u32, kind: Kind, name: &str, pos: [f32; 2]) -> Self {
        Device {
            id,
            kind,
            name: name.into(),
            model: 0,
            radio: String::new(),
            tx: false,
            bands: String::new(),
            ways: default_ways(),
            sel: default_sel(),
            pos,
        }
    }

    pub fn ports(&self) -> Vec<Port> {
        use Dir::*;
        use Signal::*;
        match self.kind {
            Kind::Radio => vec![
                port("ANT1", Out, Coax),
                port("ANT2", Out, Coax),
                port("PTT OUT", Out, Control),
                port("ALC IN", In, Control),
            ],
            Kind::Amplifier => vec![
                port("INPUT", In, Coax),
                port("PTT IN", In, Control),
                port("OUTPUT", Out, Coax),
                port("ALC OUT", Out, Control),
            ],
            Kind::Switch => {
                let mut v = vec![port("COMMON", In, Coax)];
                for n in 1..=self.ways.clamp(1, 8) {
                    v.push(port(&n.to_string(), Out, Coax));
                }
                v
            }
            Kind::Tuner | Kind::Meter => vec![port("IN", In, Coax), port("OUT", Out, Coax)],
            Kind::Load => vec![port("IN", In, Coax)],
            Kind::Antenna => vec![port("FEED", In, Coax)],
        }
    }

    pub fn port(&self, name: &str) -> Option<Port> {
        self.ports().into_iter().find(|p| p.name == name)
    }

    /// Outputs RF entering `input` reaches inside this device.
    fn through(&self, input: &str) -> Vec<String> {
        match (self.kind, input) {
            (Kind::Amplifier, "INPUT") => vec!["OUTPUT".into()],
            (Kind::Tuner | Kind::Meter, "IN") => vec!["OUT".into()],
            (Kind::Switch, "COMMON") => vec![self.sel.clamp(1, self.ways.max(1)).to_string()],
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Jack {
    pub device: u32,
    pub port: String,
}

impl Jack {
    pub fn new(device: u32, port: &str) -> Self {
        Jack { device, port: port.into() }
    }
}

/// One cable. `from` is the transmit-side output, `to` the input it feeds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cable {
    pub from: Jack,
    pub to: Jack,
}

/// The whole panel as stored in the station file under `patchbay` (#95).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchBay {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<Device>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cables: Vec<Cable>,
}

impl PatchBay {
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty() && self.cables.is_empty()
    }

    pub fn device(&self, id: u32) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }

    pub fn next_id(&self) -> u32 {
        self.devices.iter().map(|d| d.id).max().unwrap_or(0) + 1
    }

    pub fn add(&mut self, kind: Kind, name: &str, pos: [f32; 2]) -> u32 {
        let id = self.next_id();
        self.devices.push(Device::new(id, kind, name, pos));
        id
    }

    pub fn remove_device(&mut self, id: u32) {
        self.devices.retain(|d| d.id != id);
        self.cables.retain(|c| c.from.device != id && c.to.device != id);
    }

    pub fn cable_at(&self, jack: &Jack) -> Option<usize> {
        self.cables.iter().position(|c| &c.from == jack || &c.to == jack)
    }

    /// Drop cables whose jacks no longer exist (switch made smaller, device gone).
    pub fn prune(&mut self) {
        let ok = |bay: &PatchBay, j: &Jack| bay.device(j.device).and_then(|d| d.port(&j.port)).is_some();
        let keep: Vec<bool> = self.cables.iter().map(|c| ok(self, &c.from) && ok(self, &c.to)).collect();
        let mut i = 0;
        self.cables.retain(|_| {
            i += 1;
            keep[i - 1]
        });
    }

    /// Check a cable between two jacks without adding it. The result is oriented
    /// output → input.
    pub fn check(&self, a: &Jack, b: &Jack, ignore: Option<usize>) -> Result<Cable, String> {
        let da = self.device(a.device).ok_or("no such device")?;
        let db = self.device(b.device).ok_or("no such device")?;
        let pa = da.port(&a.port).ok_or("no such jack")?;
        let pb = db.port(&b.port).ok_or("no such jack")?;
        if a == b {
            return Err("same jack".into());
        }
        if a.device == b.device {
            return Err(format!("{} cannot be patched to itself", da.name));
        }
        if pa.signal != pb.signal {
            return Err("coax and control jacks do not mix".into());
        }
        if pa.dir == pb.dir {
            let what = if pa.dir == Dir::Out { "output to output" } else { "input to input" };
            return Err(format!("{what}: {} {} and {} {}", da.name, pa.name, db.name, pb.name));
        }
        for jack in [a, b] {
            if let Some(i) = self.cable_at(jack) {
                if Some(i) != ignore {
                    let d = self.device(jack.device).map(|d| d.name.as_str()).unwrap_or("?");
                    return Err(format!("{d} {} already has a cable", jack.port));
                }
            }
        }
        Ok(if pa.dir == Dir::Out {
            Cable { from: a.clone(), to: b.clone() }
        } else {
            Cable { from: b.clone(), to: a.clone() }
        })
    }

    pub fn connect(&mut self, a: &Jack, b: &Jack) -> Result<(), String> {
        let cable = self.check(a, b, None)?;
        self.cables.push(cable);
        Ok(())
    }

    /// Radio rows from the station: add missing ones, update TX ability, and
    /// drop radio devices whose row is gone.
    pub fn sync_radios(&mut self, radios: &[(String, bool)]) {
        let gone: Vec<u32> = self
            .devices
            .iter()
            .filter(|d| d.kind == Kind::Radio && !radios.iter().any(|(n, _)| *n == d.radio))
            .map(|d| d.id)
            .collect();
        for id in gone {
            self.remove_device(id);
        }
        for (i, (name, tx)) in radios.iter().enumerate() {
            if let Some(d) = self.devices.iter_mut().find(|d| d.kind == Kind::Radio && d.radio == *name) {
                d.tx = *tx;
            } else {
                let id = self.add(Kind::Radio, name, [12.0, 12.0 + 120.0 * i as f32]);
                if let Some(d) = self.devices.iter_mut().find(|d| d.id == id) {
                    d.radio = name.clone();
                    d.tx = *tx;
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum End {
    /// Ends in an antenna or dummy load (device id).
    Load(u32),
    /// An output with no cable.
    Open(Jack),
    /// The path came back to a device it already passed.
    Loop(u32),
    /// An input that goes nowhere inside its device (e.g. a radio's ALC).
    Stub(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub radio: u32,
    pub tx: bool,
    /// Device ids from the radio to the end, radio first.
    pub chain: Vec<u32>,
    pub end: End,
    /// Cable indexes along the path.
    pub cables: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Tx,
    Rx,
    Control,
    Idle,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub paths: Vec<Path>,
    pub lines: Vec<String>,
    pub warnings: Vec<String>,
}

/// Follow every coax path from each radio output (#94).
pub fn trace(bay: &PatchBay) -> Report {
    let mut rep = Report::default();
    for radio in bay.devices.iter().filter(|d| d.kind == Kind::Radio) {
        for p in radio.ports().iter().filter(|p| p.dir == Dir::Out && p.signal == Signal::Coax) {
            let start = Jack::new(radio.id, &p.name);
            if bay.cable_at(&start).is_none() {
                continue;
            }
            let mut seen = BTreeSet::new();
            seen.insert(radio.id);
            walk(bay, radio, start, vec![radio.id], Vec::new(), &mut seen, &mut rep.paths);
        }
    }
    let name = |id: u32| bay.device(id).map(|d| d.name.clone()).unwrap_or_default();
    for path in &rep.paths {
        let chain: Vec<String> = path.chain.iter().map(|id| name(*id)).collect();
        let tail = match &path.end {
            End::Load(_) => String::new(),
            End::Open(j) => format!(" → open: {} {}", name(j.device), j.port),
            End::Loop(id) => format!(" → loop back to {}", name(*id)),
            End::Stub(_) => " → dead end".into(),
        };
        if path.tx {
            rep.lines.push(format!("TX path: {}{tail}", chain.join(" → ")));
        } else {
            let mut back = chain.clone();
            back.reverse();
            rep.lines.push(format!("RX path: {}{}", back.join(" → "), tail));
        }
        let amps: Vec<&u32> = path
            .chain
            .iter()
            .filter(|id| bay.device(**id).map(|d| d.kind) == Some(Kind::Amplifier))
            .collect();
        if !path.tx && !amps.is_empty() {
            rep.warnings.push(format!(
                "warning: amplifier {} in line with receive-only {} ({} has no TX in this station)",
                name(*amps[0]),
                name(path.radio),
                name(path.radio)
            ));
        }
        match &path.end {
            End::Open(j) => rep.warnings.push(format!("open circuit: {} {} unconnected", name(j.device), j.port)),
            End::Loop(id) => rep.warnings.push(format!("loop: path returns to {}", name(*id))),
            End::Stub(id) => rep.warnings.push(format!("dead end at {}", name(*id))),
            End::Load(id) if path.tx && bay.device(*id).map(|d| d.kind) == Some(Kind::Load) => {
                rep.lines.push(format!("note: {} TX goes to dummy load {}", name(path.radio), name(*id)))
            }
            _ => {}
        }
    }
    for amp in bay.devices.iter().filter(|d| d.kind == Kind::Amplifier) {
        let fed = bay.cable_at(&Jack::new(amp.id, "INPUT")).is_some();
        let out = bay.cable_at(&Jack::new(amp.id, "OUTPUT")).is_some();
        if !fed && out {
            rep.warnings.push(format!("amplifier {} INPUT is fed from nothing", amp.name));
        }
        if fed && !out && !rep.warnings.iter().any(|w| w.contains(&format!("{} OUTPUT", amp.name))) {
            rep.warnings.push(format!("open circuit: {} OUTPUT unconnected", amp.name));
        }
    }
    rep.warnings.dedup();
    rep
}

fn walk(
    bay: &PatchBay,
    radio: &Device,
    out: Jack,
    chain: Vec<u32>,
    cables: Vec<usize>,
    seen: &mut BTreeSet<u32>,
    paths: &mut Vec<Path>,
) {
    let done = |chain: Vec<u32>, cables: Vec<usize>, end: End, paths: &mut Vec<Path>| {
        paths.push(Path { radio: radio.id, tx: radio.tx, chain, end, cables })
    };
    let Some(ci) = bay.cables.iter().position(|c| c.from == out) else {
        return done(chain, cables, End::Open(out), paths);
    };
    let mut cables = cables;
    cables.push(ci);
    let to = bay.cables[ci].to.clone();
    let Some(dev) = bay.device(to.device) else { return };
    if seen.contains(&dev.id) {
        return done(chain, cables, End::Loop(dev.id), paths);
    }
    let mut chain = chain;
    chain.push(dev.id);
    if matches!(dev.kind, Kind::Antenna | Kind::Load) {
        return done(chain, cables, End::Load(dev.id), paths);
    }
    let outs = dev.through(&to.port);
    if outs.is_empty() {
        return done(chain, cables, End::Stub(dev.id), paths);
    }
    seen.insert(dev.id);
    for o in outs {
        walk(bay, radio, Jack::new(dev.id, &o), chain.clone(), cables.clone(), seen, paths);
    }
    seen.remove(&dev.id);
}

/// What each cable carries, for its tint and label.
pub fn roles(bay: &PatchBay, rep: &Report) -> Vec<Role> {
    let mut out: Vec<Role> = bay
        .cables
        .iter()
        .map(|c| match bay.device(c.from.device).and_then(|d| d.port(&c.from.port)) {
            Some(p) if p.signal == Signal::Control => Role::Control,
            _ => Role::Idle,
        })
        .collect();
    for path in &rep.paths {
        for ci in &path.cables {
            if let Some(r) = out.get_mut(*ci) {
                if *r == Role::Idle || (*r == Role::Rx && path.tx) {
                    *r = if path.tx { Role::Tx } else { Role::Rx };
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station() -> (PatchBay, u32, u32, u32, u32) {
        let mut bay = PatchBay::default();
        bay.sync_radios(&[("IC-9700".into(), true), ("HackRF".into(), false)]);
        let amp = bay.add(Kind::Amplifier, "KPA1500", [200.0, 0.0]);
        let atu = bay.add(Kind::Tuner, "ATU", [400.0, 0.0]);
        let ant = bay.add(Kind::Antenna, "Antenna A", [600.0, 0.0]);
        (bay, 1, amp, atu, ant)
    }

    #[test]
    fn traces_a_tx_chain_to_the_antenna() {
        let (mut bay, rig, amp, atu, ant) = station();
        bay.connect(&Jack::new(rig, "ANT1"), &Jack::new(amp, "INPUT")).unwrap();
        // Reversed order is oriented output → input.
        bay.connect(&Jack::new(atu, "IN"), &Jack::new(amp, "OUTPUT")).unwrap();
        bay.connect(&Jack::new(atu, "OUT"), &Jack::new(ant, "FEED")).unwrap();
        assert_eq!(bay.cables[1].from, Jack::new(amp, "OUTPUT"));
        let rep = trace(&bay);
        assert_eq!(rep.lines, vec!["TX path: IC-9700 → KPA1500 → ATU → Antenna A".to_string()]);
        assert!(rep.warnings.is_empty(), "{:?}", rep.warnings);
        assert_eq!(roles(&bay, &rep), vec![Role::Tx; 3]);
    }

    #[test]
    fn refuses_bad_cables() {
        let (mut bay, rig, amp, atu, ant) = station();
        let err = bay.connect(&Jack::new(amp, "OUTPUT"), &Jack::new(atu, "OUT")).unwrap_err();
        assert!(err.starts_with("output to output"), "{err}");
        let err = bay.connect(&Jack::new(ant, "FEED"), &Jack::new(atu, "IN")).unwrap_err();
        assert!(err.starts_with("input to input"), "{err}");
        assert!(bay.connect(&Jack::new(rig, "PTT OUT"), &Jack::new(amp, "INPUT")).is_err());
        assert!(bay.connect(&Jack::new(amp, "OUTPUT"), &Jack::new(amp, "INPUT")).is_err());
        bay.connect(&Jack::new(rig, "ANT1"), &Jack::new(ant, "FEED")).unwrap();
        let err = bay.connect(&Jack::new(rig, "ANT2"), &Jack::new(ant, "FEED")).unwrap_err();
        assert!(err.contains("already has a cable"), "{err}");
        // Moving an existing cable end may reuse its own jack.
        assert!(bay.check(&Jack::new(rig, "ANT2"), &Jack::new(ant, "FEED"), Some(0)).is_ok());
        bay.connect(&Jack::new(rig, "PTT OUT"), &Jack::new(amp, "PTT IN")).unwrap();
        assert_eq!(roles(&bay, &trace(&bay))[1], Role::Control);
    }

    #[test]
    fn warns_on_rx_only_radio_with_amp_and_open_output() {
        let (mut bay, _, amp, _, _) = station();
        let hackrf = 2;
        bay.connect(&Jack::new(hackrf, "ANT1"), &Jack::new(amp, "INPUT")).unwrap();
        let rep = trace(&bay);
        assert_eq!(rep.lines, vec!["RX path: KPA1500 → HackRF → open: KPA1500 OUTPUT".to_string()]);
        assert!(rep.warnings.iter().any(|w| w.contains("receive-only HackRF")));
        assert!(rep.warnings.iter().any(|w| w == "open circuit: KPA1500 OUTPUT unconnected"));
        assert_eq!(roles(&bay, &rep), vec![Role::Rx]);
    }

    #[test]
    fn amp_fed_from_nothing_and_switch_position() {
        let (mut bay, rig, amp, _, ant) = station();
        bay.connect(&Jack::new(amp, "OUTPUT"), &Jack::new(ant, "FEED")).unwrap();
        assert!(trace(&bay).warnings.iter().any(|w| w.contains("fed from nothing")));
        bay.cables.clear();
        let sw = bay.add(Kind::Switch, "SW", [0.0, 0.0]);
        let ant2 = bay.add(Kind::Antenna, "Antenna B", [0.0, 0.0]);
        bay.connect(&Jack::new(rig, "ANT1"), &Jack::new(sw, "COMMON")).unwrap();
        bay.connect(&Jack::new(sw, "1"), &Jack::new(ant, "FEED")).unwrap();
        bay.connect(&Jack::new(sw, "2"), &Jack::new(ant2, "FEED")).unwrap();
        assert_eq!(trace(&bay).lines[0], "TX path: IC-9700 → SW → Antenna A");
        bay.devices.iter_mut().find(|d| d.id == sw).unwrap().sel = 2;
        assert_eq!(trace(&bay).lines[0], "TX path: IC-9700 → SW → Antenna B");
        bay.devices.iter_mut().find(|d| d.id == sw).unwrap().ways = 1;
        bay.prune();
        assert_eq!(bay.cables.len(), 2);
    }

    #[test]
    fn detects_a_loop() {
        let (mut bay, rig, amp, atu, _) = station();
        let sw = bay.add(Kind::Switch, "SW", [0.0, 0.0]);
        bay.connect(&Jack::new(rig, "ANT1"), &Jack::new(sw, "COMMON")).unwrap();
        bay.connect(&Jack::new(sw, "1"), &Jack::new(amp, "INPUT")).unwrap();
        bay.connect(&Jack::new(amp, "OUTPUT"), &Jack::new(atu, "IN")).unwrap();
        bay.cables.push(Cable { from: Jack::new(atu, "OUT"), to: Jack::new(sw, "COMMON") });
        let rep = trace(&bay);
        assert!(rep.warnings.iter().any(|w| w == "loop: path returns to SW"), "{:?}", rep.warnings);
    }

    #[test]
    fn sync_radios_follows_the_station() {
        let (mut bay, rig, amp, _, _) = station();
        bay.connect(&Jack::new(rig, "ANT1"), &Jack::new(amp, "INPUT")).unwrap();
        bay.sync_radios(&[("HackRF".into(), true)]);
        assert!(bay.device(rig).is_none());
        assert!(bay.cables.is_empty());
        assert!(bay.device(2).unwrap().tx);
    }

    #[test]
    fn round_trips_and_old_json_loads() {
        let (mut bay, rig, _, _, ant) = station();
        bay.connect(&Jack::new(rig, "ANT1"), &Jack::new(ant, "FEED")).unwrap();
        let text = serde_json::to_string(&bay).unwrap();
        let back: PatchBay = serde_json::from_str(&text).unwrap();
        assert_eq!(back, bay);
        let empty: PatchBay = serde_json::from_str("{}").unwrap();
        assert!(empty.is_empty());
    }
}
