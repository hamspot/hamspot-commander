//! Patch bay amplifier boxes bound to configured amplifiers (#96) and the
//! TX-path warnings that come from their live state (#97).
//!
//! Pure functions. The patch bay only reads amplifier state; nothing here
//! sends a command, keys a radio or switches anything.

use super::model::{Device, End, Kind, PatchBay, Report};
use crate::amp::{AmplifierRow, AmplifierSpec, Power};

/// Extra box rows a bound amplifier uses for its live readout.
pub const STATUS_ROWS: usize = 2;

impl Device {
    pub fn is_bound_amp(&self) -> bool {
        self.kind == Kind::Amplifier && !self.amp.is_empty()
    }
}

#[derive(Debug, PartialEq)]
pub enum Binding<'a> {
    /// Not linked: the box is a drawing only.
    Unbound,
    /// Linked to a name no configured amplifier has (removed or renamed
    /// outside Commander).
    Missing(String),
    Bound(&'a AmplifierRow),
}

pub fn binding<'a>(dev: &Device, rows: &'a [AmplifierRow]) -> Binding<'a> {
    if dev.kind != Kind::Amplifier || dev.amp.is_empty() {
        return Binding::Unbound;
    }
    match rows.iter().find(|row| row.spec.name == dev.amp) {
        Some(row) => Binding::Bound(row),
        None => Binding::Missing(dev.amp.clone()),
    }
}

/// Follow a rename in the Automation list. Returns how many boxes moved.
pub fn rename_amp(bay: &mut PatchBay, old: &str, new: &str) -> usize {
    if old == new || old.is_empty() {
        return 0;
    }
    let mut n = 0;
    for dev in bay.devices.iter_mut().filter(|d| d.kind == Kind::Amplifier && d.amp == old) {
        dev.amp = new.to_string();
        n += 1;
    }
    n
}

/// A configured-amplifier row for a patch bay box ("Add to Automation
/// list"). The name is the box name, made unique against `taken`.
pub fn spec_for(dev: &Device, taken: &[String]) -> AmplifierSpec {
    let base = if dev.name.trim().is_empty() { "Amplifier".to_string() } else { dev.name.trim().to_string() };
    let mut name = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| *t == name) {
        name = format!("{base} {n}");
        n += 1;
    }
    AmplifierSpec {
        name,
        model: if dev.model > 0 { dev.model } else { 1 },
        device: String::new(),
        serial_speed: String::new(),
        conf: String::new(),
        follow_rig: false,
    }
}

/// Connection words for a bound box. Not colour alone: the text says it.
pub fn link_text(row: &AmplifierRow) -> String {
    if row.port.is_some() {
        "daemon up".into()
    } else if row.status == "device missing" {
        "device missing".into()
    } else {
        format!("daemon down ({})", row.status)
    }
}

/// Warnings from bound amplifiers on traced TX paths (#97). `power`
/// answers the live power state of a configured amplifier by name, None
/// when it is unknown or the daemon is down (no warning then).
pub fn amp_warnings(bay: &PatchBay, rep: &Report, power: impl Fn(&str) -> Option<Power>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for path in rep.paths.iter().filter(|p| p.tx) {
        for dev in path.chain.iter().filter_map(|id| bay.device(*id)).filter(|d| d.is_bound_amp()) {
            let Some(state) = power(&dev.amp) else { continue };
            let radio = bay.device(path.radio).map(|d| d.name.as_str()).unwrap_or("radio");
            let w = match state {
                Power::Standby | Power::Off | Power::On => Some(format!(
                    "amplifier {} is {} while the {radio} TX path is patched through it",
                    dev.name,
                    state.label()
                )),
                Power::Operate if !matches!(path.end, End::Load(_)) => Some(format!(
                    "amplifier {} is in OPERATE but its TX path from {radio} does not end in an antenna or dummy load",
                    dev.name
                )),
                _ => None,
            };
            if let Some(w) = w {
                if !out.contains(&w) {
                    out.push(w);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amp::AmpState;
    use crate::patchbay::model::{trace, Jack};

    fn row(name: &str, port: Option<u16>, power: Option<Power>) -> AmplifierRow {
        AmplifierRow {
            id: 7,
            spec: AmplifierSpec { name: name.into(), ..AmplifierSpec::dummy() },
            port,
            status: if port.is_some() { "up".into() } else { "device missing".into() },
            selected: false,
            state: AmpState { power, ..Default::default() },
            make: String::new(),
            model_label: String::new(),
            settable: Vec::new(),
        }
    }

    /// IC-9700 → KPA → Antenna A.
    fn bay(bound: &str, end_load: bool) -> (PatchBay, u32) {
        let mut bay = PatchBay::default();
        bay.sync_radios(&[("IC-9700".into(), true)]);
        let amp = bay.add(Kind::Amplifier, "KPA", [200.0, 0.0]);
        bay.devices.iter_mut().find(|d| d.id == amp).unwrap().amp = bound.into();
        bay.connect(&Jack::new(1, "ANT1"), &Jack::new(amp, "INPUT")).unwrap();
        if end_load {
            let ant = bay.add(Kind::Antenna, "Antenna A", [400.0, 0.0]);
            bay.connect(&Jack::new(amp, "OUTPUT"), &Jack::new(ant, "FEED")).unwrap();
        }
        (bay, amp)
    }

    #[test]
    fn binding_bound_unbound_missing() {
        let rows = vec![row("KPA1500", Some(4534), Some(Power::Standby))];
        let (mut b, amp) = bay("", true);
        assert_eq!(binding(b.device(amp).unwrap(), &rows), Binding::Unbound);
        b.devices.iter_mut().find(|d| d.id == amp).unwrap().amp = "KPA1500".into();
        assert!(matches!(binding(b.device(amp).unwrap(), &rows), Binding::Bound(r) if r.spec.name == "KPA1500"));
        assert_eq!(binding(b.device(amp).unwrap(), &[]), Binding::Missing("KPA1500".into()));
        // A radio box is never bound, even with a stray name.
        let radio = b.device(1).unwrap().clone();
        assert_eq!(binding(&Device { amp: "KPA1500".into(), ..radio }, &rows), Binding::Unbound);
        assert_eq!(link_text(&rows[0]), "daemon up");
        assert_eq!(link_text(&row("X", None, None)), "device missing");
    }

    #[test]
    fn rename_follows_the_automation_list() {
        let (mut b, amp) = bay("Old", true);
        assert_eq!(rename_amp(&mut b, "Old", "New"), 1);
        assert_eq!(b.device(amp).unwrap().amp, "New");
        assert_eq!(rename_amp(&mut b, "Old", "Other"), 0);
        assert_eq!(rename_amp(&mut b, "", "Other"), 0);
    }

    #[test]
    fn add_to_automation_list_makes_a_unique_spec() {
        let mut dev = Device::new(3, Kind::Amplifier, "Elecraft KPA1500", [0.0, 0.0]);
        dev.model = 201;
        let spec = spec_for(&dev, &["Elecraft KPA1500".into()]);
        assert_eq!(spec.name, "Elecraft KPA1500 2");
        assert_eq!(spec.model, 201);
        assert!(!spec.follow_rig);
        assert!(crate::amp::validate_spec(&spec).is_ok());
        dev.model = 0;
        assert_eq!(spec_for(&dev, &[]).model, 1);
    }

    #[test]
    fn warning_rules() {
        let p = |s: Option<Power>| move |_: &str| s;
        // Standby on a TX path to an antenna.
        let (b, _) = bay("KPA", true);
        let rep = trace(&b);
        let w = amp_warnings(&b, &rep, p(Some(Power::Standby)));
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("STBY"));
        assert_eq!(amp_warnings(&b, &rep, p(Some(Power::Off))).len(), 1);
        // Operate into an antenna is fine.
        assert!(amp_warnings(&b, &rep, p(Some(Power::Operate))).is_empty());
        // Unknown / daemon down: no guess.
        assert!(amp_warnings(&b, &rep, p(None)).is_empty());
        // Operate with an open output.
        let (b, _) = bay("KPA", false);
        let w = amp_warnings(&b, &trace(&b), p(Some(Power::Operate)));
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("OPERATE"));
        // Unbound boxes never warn.
        let (b, _) = bay("", false);
        assert!(amp_warnings(&b, &trace(&b), p(Some(Power::Operate))).is_empty());
    }

    #[test]
    fn receive_only_paths_do_not_warn_about_power() {
        let mut b = PatchBay::default();
        b.sync_radios(&[("HackRF".into(), false)]);
        let amp = b.add(Kind::Amplifier, "KPA", [200.0, 0.0]);
        b.devices.iter_mut().find(|d| d.id == amp).unwrap().amp = "KPA".into();
        b.connect(&Jack::new(1, "ANT1"), &Jack::new(amp, "INPUT")).unwrap();
        assert!(amp_warnings(&b, &trace(&b), |_| Some(Power::Standby)).is_empty());
    }

    #[test]
    fn bound_field_is_optional_in_the_file() {
        let dev = Device::new(3, Kind::Amplifier, "A", [0.0, 0.0]);
        let text = serde_json::to_string(&dev).unwrap();
        assert!(!text.contains("\"amp\""));
        let old: Device = serde_json::from_str(r#"{"id":3,"kind":"amplifier","name":"A","pos":[0,0]}"#).unwrap();
        assert!(old.amp.is_empty());
        let bound = Device { amp: "KPA".into(), ..dev };
        let back: Device = serde_json::from_str(&serde_json::to_string(&bound).unwrap()).unwrap();
        assert_eq!(back.amp, "KPA");
    }
}
