//! Station-wide TX lockout (#219). A set of named holds; transmit is refused
//! while any hold remains. The VNA device scan is the first holder. E-Stop
//! (#198) is meant to become a second holder at the same refusal sites
//! (`App::ptt`, `App::send` for `SetPtt(true)`, `App::send_amp` for OPERATE);
//! E-Stop itself stays separate: latched, manual release.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Hold {
    pub text: String,
    /// Only a "Release TX lockout" press clears it (error or unknown state).
    pub manual: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TxLockout {
    holds: BTreeMap<String, Hold>,
}

impl TxLockout {
    pub fn engage(&mut self, key: &str, text: &str) {
        self.holds.insert(key.into(), Hold { text: text.into(), manual: false });
    }

    /// The holder no longer knows its state: keep the hold until released by hand.
    pub fn make_manual(&mut self, key: &str, text: &str) {
        self.holds.insert(key.into(), Hold { text: text.into(), manual: true });
    }

    /// Automatic release by the holder. A manual hold is not released.
    pub fn release(&mut self, key: &str) -> bool {
        match self.holds.get(key) {
            Some(h) if !h.manual => {
                self.holds.remove(key);
                true
            }
            _ => false,
        }
    }

    /// The bordered "Release TX lockout" button.
    pub fn release_by_user(&mut self, key: &str) {
        self.holds.remove(key);
    }

    pub fn holds(&self, key: &str) -> bool {
        self.holds.contains_key(key)
    }

    pub fn is_manual(&self, key: &str) -> bool {
        self.holds.get(key).is_some_and(|h| h.manual)
    }

    pub fn engaged(&self) -> bool {
        !self.holds.is_empty()
    }

    /// One line per hold, e.g. "TX LOCKOUT: VNA scan in progress on Duplexer".
    pub fn banner(&self) -> Option<String> {
        if self.holds.is_empty() {
            return None;
        }
        Some(self.holds.values().map(|h| format!("TX LOCKOUT: {}", h.text)).collect::<Vec<_>>().join(" · "))
    }

    /// Ok when `action` may transmit; otherwise the plain refusal text.
    pub fn refuse(&self, action: &str) -> Result<(), String> {
        match self.banner() {
            None => Ok(()),
            Some(b) => Err(format!("{action} refused. {b}")),
        }
    }
}

/// Amplifier commands that put it in a transmit-ready state.
pub fn amp_cmd_transmits(cmd: &crate::amp::Control) -> bool {
    matches!(cmd, crate::amp::Control::SetPower(crate::amp::Power::Operate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_holds_release_only_when_none_remain() {
        let mut l = TxLockout::default();
        assert!(l.refuse("PTT").is_ok());
        l.engage("vna", "VNA scan in progress on Duplexer");
        l.engage("estop", "E-STOP engaged");
        assert!(l.refuse("PTT").unwrap_err().contains("VNA scan in progress on Duplexer"));
        assert!(l.release("vna"));
        assert!(l.engaged());
        assert!(l.refuse("MOX").unwrap_err().starts_with("MOX refused"));
        l.release("estop");
        assert!(!l.engaged());
        assert!(l.refuse("PTT").is_ok());
    }

    #[test]
    fn amp_operate_is_a_transmit_action_and_standby_is_not() {
        use crate::amp::{Control, Power};
        assert!(amp_cmd_transmits(&Control::SetPower(Power::Operate)));
        assert!(!amp_cmd_transmits(&Control::SetPower(Power::Standby)));
        assert!(!amp_cmd_transmits(&Control::SetFreq(7.0e6)));
    }

    #[test]
    fn a_manual_hold_survives_automatic_release() {
        let mut l = TxLockout::default();
        l.engage("vna", "x");
        l.make_manual("vna", "scan state unknown");
        assert!(!l.release("vna"));
        assert!(l.engaged());
        l.release_by_user("vna");
        assert!(!l.engaged());
    }
}
