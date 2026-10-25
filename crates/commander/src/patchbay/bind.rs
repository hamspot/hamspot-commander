//! Patch bay amplifier boxes bound to configured amplifiers (#96) and the
//! TX-path warnings that come from their live state (#97).
//!
//! Binding is by the amplifier's fixed id, never by name or position, so a
//! rename, a reorder or a second amplifier with the same name can never
//! send a command to the wrong amplifier. A box whose id has no amplifier
//! is "missing" and resolves to nothing.
//!
//! Pure functions. Nothing here sends a command, keys a radio or switches
//! anything.

use super::model::{Device, End, Kind, PatchBay, Report};
use crate::amp::{AmplifierRow, AmplifierSpec, Power};

/// Extra box rows a bound amplifier uses for its live readout.
pub const STATUS_ROWS: usize = 2;

impl Device {
    pub fn is_bound_amp(&self) -> bool {
        self.kind == Kind::Amplifier && !self.amp_id.is_empty()
    }
}

#[derive(Debug, PartialEq)]
pub enum Binding<'a> {
    /// Not linked: the box is a drawing only.
    Unbound,
    /// Linked to an id no configured amplifier has (removed). Sends nothing.
    Missing,
    Bound(&'a AmplifierRow),
}

pub fn binding<'a>(dev: &Device, rows: &'a [AmplifierRow]) -> Binding<'a> {
    if dev.kind != Kind::Amplifier || dev.amp_id.is_empty() {
        return Binding::Unbound;
    }
    match resolve(&dev.amp_id, rows) {
        Some(row) => Binding::Bound(row),
        None => Binding::Missing,
    }
}

/// The live row for an amplifier id, looked up at send time. Exactly one
/// match or nothing: an empty or ambiguous id resolves to None.
pub fn resolve<'a>(id: &str, rows: &'a [AmplifierRow]) -> Option<&'a AmplifierRow> {
    if id.is_empty() {
        return None;
    }
    let mut hits = rows.iter().filter(|row| row.spec.id == id);
    let first = hits.next()?;
    hits.next().is_none().then_some(first)
}

/// After a load: `dup` are ids `amp::repair_ids` found duplicated. Boxes
/// bound to them are unbound with a note. Boxes from this branch's earlier
/// name format bind to the id only when exactly one amplifier has that
/// name; otherwise they are unbound with a note. Returns the notes.
pub fn migrate(bay: &mut PatchBay, specs: &[AmplifierSpec], dup: &[String]) -> Vec<String> {
    let mut notes = Vec::new();
    for dev in bay.devices.iter_mut().filter(|d| d.kind == Kind::Amplifier) {
        let legacy = std::mem::take(&mut dev.amp_legacy_name);
        if dev.amp_id.is_empty() && !legacy.is_empty() {
            let named: Vec<&AmplifierSpec> = specs.iter().filter(|s| s.name == legacy).collect();
            if named.len() == 1 {
                dev.amp_id = named[0].id.clone();
            } else {
                dev.note = format!(
                    "unbound: {} configured amplifiers are named \"{legacy}\"; pick one",
                    named.len()
                );
                notes.push(format!("{}: {}", dev.name, dev.note));
            }
        }
        if !dev.amp_id.is_empty() && dup.contains(&dev.amp_id) {
            dev.amp_id.clear();
            dev.note = "unbound: its amplifier id was duplicated in the station file; pick one".into();
            notes.push(format!("{}: {}", dev.name, dev.note));
        }
    }
    notes
}

/// A configured-amplifier row for a patch bay box ("Add to Automation
/// list") with the given fresh id. The name is the box name, made unique
/// against `taken` for readability only; binding uses the id.
pub fn spec_for(dev: &Device, taken: &[String], id: String) -> AmplifierSpec {
    let base = if dev.name.trim().is_empty() { "Amplifier".to_string() } else { dev.name.trim().to_string() };
    let mut name = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| *t == name) {
        name = format!("{base} {n}");
        n += 1;
    }
    AmplifierSpec {
        id,
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
/// answers the live power state of a configured amplifier by id, None
/// when it is unknown, missing or the daemon is down (no warning then).
pub fn amp_warnings(bay: &PatchBay, rep: &Report, power: impl Fn(&str) -> Option<Power>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for path in rep.paths.iter().filter(|p| p.tx) {
        for dev in path.chain.iter().filter_map(|id| bay.device(*id)).filter(|d| d.is_bound_amp()) {
            let Some(state) = power(&dev.amp_id) else { continue };
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

    fn spec(id: &str, name: &str) -> AmplifierSpec {
        AmplifierSpec { id: id.into(), name: name.into(), ..AmplifierSpec::dummy() }
    }

    fn row(slot: u64, id: &str, name: &str, port: Option<u16>, power: Option<Power>) -> AmplifierRow {
        AmplifierRow {
            id: slot,
            spec: spec(id, name),
            port,
            status: if port.is_some() { "up".into() } else { "device missing".into() },
            selected: false,
            state: AmpState { power, ..Default::default() },
            make: String::new(),
            model_label: String::new(),
            settable: Vec::new(),
        }
    }

    /// IC-9700 → KPA → Antenna A, box bound to `amp_id`.
    fn bay(amp_id: &str, end_load: bool) -> (PatchBay, u32) {
        let mut bay = PatchBay::default();
        bay.sync_radios(&[("IC-9700".into(), true)]);
        let amp = bay.add(Kind::Amplifier, "KPA", [200.0, 0.0]);
        bay.devices.iter_mut().find(|d| d.id == amp).unwrap().amp_id = amp_id.into();
        bay.connect(&Jack::new(1, "ANT1"), &Jack::new(amp, "INPUT")).unwrap();
        if end_load {
            let ant = bay.add(Kind::Antenna, "Antenna A", [400.0, 0.0]);
            bay.connect(&Jack::new(amp, "OUTPUT"), &Jack::new(ant, "FEED")).unwrap();
        }
        (bay, amp)
    }

    fn dev(b: &PatchBay, id: u32) -> &Device {
        b.device(id).unwrap()
    }

    #[test]
    fn binding_bound_unbound_missing() {
        let rows = vec![row(1, "aa", "KPA1500", Some(4534), Some(Power::Standby))];
        let (b, amp) = bay("", true);
        assert_eq!(binding(dev(&b, amp), &rows), Binding::Unbound);
        let (b, amp) = bay("aa", true);
        assert!(matches!(binding(dev(&b, amp), &rows), Binding::Bound(r) if r.spec.id == "aa"));
        // A radio box is never bound, even with a stray id.
        let radio = Device { amp_id: "aa".into(), ..dev(&b, 1).clone() };
        assert_eq!(binding(&radio, &rows), Binding::Unbound);
        assert_eq!(link_text(&rows[0]), "daemon up");
        assert_eq!(link_text(&row(2, "bb", "X", None, None)), "device missing");
    }

    #[test]
    fn rename_keeps_the_binding() {
        let (b, amp) = bay("aa", true);
        let before = vec![row(1, "aa", "Old", Some(1), None)];
        let after = vec![row(1, "aa", "New name", Some(1), None)];
        assert!(matches!(binding(dev(&b, amp), &before), Binding::Bound(r) if r.spec.name == "Old"));
        assert!(matches!(binding(dev(&b, amp), &after), Binding::Bound(r) if r.spec.name == "New name"));
    }

    #[test]
    fn delete_leaves_the_box_missing_and_resolves_to_nothing() {
        let (b, amp) = bay("aa", true);
        // Another amplifier still exists, even one with the same name: no fallback.
        let rows = vec![row(9, "zz", "KPA", Some(1), Some(Power::Operate))];
        assert_eq!(binding(dev(&b, amp), &rows), Binding::Missing);
        assert!(resolve("aa", &rows).is_none());
        assert!(amp_warnings(&b, &trace(&b), |id| resolve(id, &rows).and_then(|r| r.state.power)).is_empty());
    }

    #[test]
    fn name_collision_does_not_misroute() {
        let (b, amp) = bay("bb", true);
        // Same name, the wrong one first in the list.
        let rows = vec![row(1, "aa", "KPA", Some(1), None), row(2, "bb", "KPA", Some(2), None)];
        assert!(matches!(binding(dev(&b, amp), &rows), Binding::Bound(r) if r.id == 2));
        assert_eq!(resolve("bb", &rows).map(|r| r.id), Some(2));
        // An ambiguous id resolves to nothing rather than the first match.
        let twins = vec![row(1, "cc", "A", None, None), row(2, "cc", "B", None, None)];
        assert!(resolve("cc", &twins).is_none());
        assert!(resolve("", &rows).is_none());
    }

    #[test]
    fn duplicate_ids_are_repaired_and_their_bindings_dropped() {
        let mut specs = vec![spec("dd", "A"), spec("dd", "B"), spec("ee", "C")];
        let dup = crate::amp::repair_ids(&mut specs);
        assert_eq!(dup, ["dd"]);
        assert_ne!(specs[1].id, "dd");
        let (mut b, amp) = bay("dd", true);
        let other = b.add(Kind::Amplifier, "C box", [0.0, 200.0]);
        b.devices.iter_mut().find(|d| d.id == other).unwrap().amp_id = "ee".into();
        let notes = migrate(&mut b, &specs, &dup);
        assert_eq!(notes.len(), 1);
        assert!(dev(&b, amp).amp_id.is_empty());
        assert!(dev(&b, amp).note.contains("duplicated"));
        assert_eq!(dev(&b, other).amp_id, "ee");
    }

    #[test]
    fn old_name_bindings_migrate_only_when_unambiguous() {
        let specs = vec![spec("aa", "KPA1500"), spec("bb", "Twin"), spec("cc", "Twin")];
        let old = |name: &str| {
            let text = format!(r#"{{"id":3,"kind":"amplifier","name":"Box","amp":"{name}","pos":[0,0]}}"#);
            let d: Device = serde_json::from_str(&text).unwrap();
            PatchBay { devices: vec![d], ..Default::default() }
        };
        let mut b = old("KPA1500");
        assert!(migrate(&mut b, &specs, &[]).is_empty());
        assert_eq!(b.devices[0].amp_id, "aa");
        let mut b = old("Twin");
        assert_eq!(migrate(&mut b, &specs, &[]).len(), 1);
        assert!(b.devices[0].amp_id.is_empty());
        assert!(b.devices[0].note.contains("2 configured"));
        let mut b = old("Gone");
        assert_eq!(migrate(&mut b, &specs, &[]).len(), 1);
        assert!(b.devices[0].amp_id.is_empty());
        // The old name field is never written back.
        let text = serde_json::to_string(&b.devices[0]).unwrap();
        assert!(!text.contains("\"amp\""));
    }

    #[test]
    fn add_to_automation_list_makes_a_unique_spec() {
        let mut d = Device::new(3, Kind::Amplifier, "Elecraft KPA1500", [0.0, 0.0]);
        d.model = 201;
        let id = crate::amp::new_id();
        let s = spec_for(&d, &["Elecraft KPA1500".into()], id.clone());
        assert_eq!(s.id, id);
        assert_eq!(s.name, "Elecraft KPA1500 2");
        assert_eq!(s.model, 201);
        assert!(!s.follow_rig);
        assert!(crate::amp::validate_spec(&s).is_ok());
        d.model = 0;
        assert_eq!(spec_for(&d, &[], crate::amp::new_id()).model, 1);
    }

    #[test]
    fn warning_rules() {
        let p = |s: Option<Power>| move |_: &str| s;
        let (b, _) = bay("aa", true);
        let rep = trace(&b);
        let w = amp_warnings(&b, &rep, p(Some(Power::Standby)));
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("STBY"));
        assert_eq!(amp_warnings(&b, &rep, p(Some(Power::Off))).len(), 1);
        assert!(amp_warnings(&b, &rep, p(Some(Power::Operate))).is_empty());
        assert!(amp_warnings(&b, &rep, p(None)).is_empty());
        let (b, _) = bay("aa", false);
        let w = amp_warnings(&b, &trace(&b), p(Some(Power::Operate)));
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("OPERATE"));
        let (b, _) = bay("", false);
        assert!(amp_warnings(&b, &trace(&b), p(Some(Power::Operate))).is_empty());
    }

    #[test]
    fn receive_only_paths_do_not_warn_about_power() {
        let mut b = PatchBay::default();
        b.sync_radios(&[("HackRF".into(), false)]);
        let amp = b.add(Kind::Amplifier, "KPA", [200.0, 0.0]);
        b.devices.iter_mut().find(|d| d.id == amp).unwrap().amp_id = "aa".into();
        b.connect(&Jack::new(1, "ANT1"), &Jack::new(amp, "INPUT")).unwrap();
        assert!(amp_warnings(&b, &trace(&b), |_| Some(Power::Standby)).is_empty());
    }

    #[test]
    fn binding_round_trips_and_is_optional() {
        let d = Device::new(3, Kind::Amplifier, "A", [0.0, 0.0]);
        let text = serde_json::to_string(&d).unwrap();
        assert!(!text.contains("amp_id"));
        let old: Device = serde_json::from_str(r#"{"id":3,"kind":"amplifier","name":"A","pos":[0,0]}"#).unwrap();
        assert!(old.amp_id.is_empty());
        let bound = Device { amp_id: "aa".into(), ..d };
        let back: Device = serde_json::from_str(&serde_json::to_string(&bound).unwrap()).unwrap();
        assert_eq!(back.amp_id, "aa");
        let s = spec("aa", "KPA");
        let back: AmplifierSpec = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
