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

/// Merge point of E-Stop (#198) and the TX lockout (#219): a transmit action
/// is refused when the E-Stop is engaged or any lockout hold remains. E-Stop is
/// checked first (it is latched and also enforced at the device hubs); its text
/// names RELEASE, the lockout's text names the hold. Neither clears the other.
pub fn refuse_transmit(lock: &TxLockout, estop_engaged: bool, action: &str) -> Result<(), String> {
    if estop_engaged {
        return Err(format!("{action} refused. E-STOP is engaged; press RELEASE to return to normal operation."));
    }
    lock.refuse(action)
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
    fn transmit_is_refused_under_estop_or_lockout_with_its_own_text() {
        let mut l = TxLockout::default();
        assert!(refuse_transmit(&l, false, "PTT").is_ok());
        let e = refuse_transmit(&l, true, "PTT").unwrap_err();
        assert!(e.starts_with("PTT refused.") && e.contains("E-STOP is engaged") && !e.contains("TX LOCKOUT"));
        l.engage("vna", "VNA scan in progress on Duplexer");
        let e = refuse_transmit(&l, false, "Amplifier OPERATE").unwrap_err();
        assert!(e.contains("TX LOCKOUT: VNA scan in progress on Duplexer") && !e.contains("E-STOP"));
        assert!(refuse_transmit(&l, true, "MOX").unwrap_err().contains("E-STOP is engaged"));
        // Releasing the VNA hold does not clear E-Stop, and vice versa.
        l.release("vna");
        assert!(refuse_transmit(&l, true, "PTT").is_err());
        assert!(refuse_transmit(&l, false, "PTT").is_ok());
        // The hub rules still refuse on their own while E-Stop is engaged.
        assert!(!crate::estop::allows_rig(&rigctl::Command::SetPtt(true), true));
        assert!(!crate::estop::allows_amp(&crate::amp::Control::SetPower(crate::amp::Power::Operate), true));
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
