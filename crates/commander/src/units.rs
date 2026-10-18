//! Radios named on a P25 control channel.
//!
//! A quiet radio never appears. The description is the operator's text and
//! starts as the decimal id. A later sight does not replace it.

use serde::{Deserialize, Serialize};

use crate::follow::UnitSight;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub id: u32,
    /// Operator text. A new radio starts as the decimal id.
    #[serde(default)]
    pub description: String,
    /// Last talkgroup on a group voice grant for this radio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tg: Option<u32>,
    /// The other radio on a unit-to-unit grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<u32>,
    /// Set when a grant for this radio was marked emergency. It stays set.
    #[serde(default, skip_serializing_if = "is_false")]
    pub emergency: bool,
    /// Set when a grant for this radio was marked encrypted. It stays set.
    #[serde(default, skip_serializing_if = "is_false")]
    pub encrypted: bool,
    /// Site from a registration response, as printed in hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wacn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sys: Option<String>,
    /// `REG_ACCEPT`, `REG_FAIL`, `REG_DENY`, or `REG_REFUSED` when printed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reg: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Unit {
    #[cfg(test)]
    pub fn named(id: u32) -> Self {
        Self {
            description: id.to_string(),
            id,
            last_tg: None,
            peer: None,
            emergency: false,
            encrypted: false,
            site: None,
            wacn: None,
            sys: None,
            reg: None,
        }
    }
}

/// Insert `sight` or refresh the detected fields. The description is left
/// alone. `true` means the list gained a radio or a detected field changed.
pub fn note(units: &mut Vec<Unit>, sight: &UnitSight) -> bool {
    if sight.id == 0 {
        return false;
    }
    if let Some(row) = units.iter_mut().find(|row| row.id == sight.id) {
        let mut changed = false;
        changed |= assign_some(&mut row.last_tg, sight.last_tg);
        changed |= assign_some(&mut row.peer, sight.peer);
        changed |= assign_text(&mut row.wacn, sight.wacn.as_deref());
        changed |= assign_text(&mut row.sys, sight.sys.as_deref());
        changed |= assign_text(&mut row.reg, sight.reg.as_deref());
        if let Some(site) = sight.site {
            if row.site != Some(site) {
                row.site = Some(site);
                changed = true;
            }
        }
        if sight.emergency && !row.emergency {
            row.emergency = true;
            changed = true;
        }
        if sight.encrypted && !row.encrypted {
            row.encrypted = true;
            changed = true;
        }
        return changed;
    }
    units.push(Unit {
        id: sight.id,
        description: sight.id.to_string(),
        last_tg: sight.last_tg,
        peer: sight.peer,
        emergency: sight.emergency,
        encrypted: sight.encrypted,
        site: sight.site,
        wacn: sight.wacn.clone(),
        sys: sight.sys.clone(),
        reg: sight.reg.clone(),
    });
    true
}

fn assign_some<T: Copy + PartialEq>(slot: &mut Option<T>, next: Option<T>) -> bool {
    match next {
        Some(value) if *slot != Some(value) => {
            *slot = Some(value);
            true
        }
        _ => false,
    }
}

fn assign_text(slot: &mut Option<String>, next: Option<&str>) -> bool {
    match next {
        Some(value) if slot.as_deref() != Some(value) => {
            *slot = Some(value.to_string());
            true
        }
        _ => false,
    }
}

/// Detected facts for one row. The description is not repeated here.
pub fn facts(unit: &Unit) -> String {
    let mut parts = Vec::new();
    if let Some(tg) = unit.last_tg {
        parts.push(format!("TG {tg}"));
    }
    if let Some(peer) = unit.peer {
        parts.push(format!("peer {peer}"));
    }
    if unit.emergency {
        parts.push("emergency".to_string());
    }
    if unit.encrypted {
        parts.push("encrypted".to_string());
    }
    if let Some(site) = unit.site {
        parts.push(format!("site {site:03X}"));
    }
    if let Some(wacn) = &unit.wacn {
        parts.push(format!("WACN {wacn}"));
    }
    if let Some(sys) = &unit.sys {
        parts.push(format!("SYS {sys}"));
    }
    if let Some(reg) = &unit.reg {
        parts.push(reg.clone());
    }
    parts.join("  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sight(id: u32) -> UnitSight {
        UnitSight {
            id,
            last_tg: Some(100),
            peer: None,
            emergency: false,
            encrypted: false,
            site: None,
            wacn: None,
            sys: None,
            reg: None,
        }
    }

    #[test]
    fn a_new_radio_keeps_its_id_as_the_description() {
        let mut units = Vec::new();
        assert!(note(&mut units, &sight(42)));
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].description, "42");
        assert_eq!(units[0].last_tg, Some(100));
        units[0].description = "engine 42".into();
        assert!(!note(&mut units, &sight(42)));
        assert_eq!(units[0].description, "engine 42");
        assert!(!note(&mut units, &sight(0)));
        assert_eq!(units.len(), 1);
    }

    #[test]
    fn flags_stick_and_a_new_fact_is_a_change() {
        let mut units = Vec::new();
        let mut first = sight(7);
        first.encrypted = true;
        first.emergency = true;
        assert!(note(&mut units, &first));
        let mut quiet = sight(7);
        quiet.last_tg = None;
        assert!(!note(&mut units, &quiet));
        assert!(units[0].encrypted);
        assert!(units[0].emergency);
        assert_eq!(units[0].last_tg, Some(100));
        let mut moved = sight(7);
        moved.last_tg = Some(200);
        moved.peer = Some(9);
        moved.site = Some(0x0A);
        moved.wacn = Some("BEE00".into());
        moved.sys = Some("2A1".into());
        moved.reg = Some("REG_ACCEPT".into());
        assert!(note(&mut units, &moved));
        assert!(!note(&mut units, &moved));
        assert_eq!(units[0].last_tg, Some(200));
        assert_eq!(units[0].peer, Some(9));
        assert_eq!(units[0].site, Some(0x0A));
        assert_eq!(facts(&units[0]), "TG 200  peer 9  emergency  encrypted  site 00A  WACN BEE00  SYS 2A1  REG_ACCEPT");
    }
}
