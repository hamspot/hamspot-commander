//! Hamlib `rigctld` client and the in-process demo rig.
//!
//! The live client speaks Hamlib 4's extended response protocol: each command
//! is prefixed with `+\` so the reply ends with an `RPRT` line. The demo rig
//! answers the same [`Command`] values without a socket.

mod caps;
mod demo;
mod live;

pub use caps::{parse_caps, Caps, Range};
pub use demo::DemoRig;
pub use live::LiveRig;

use std::collections::BTreeMap;

/// Screenshot freeze: 14.153.608 MHz, USB, S9+20.
pub const DEMO_FREQ_HZ: f64 = 14_153_608.0;
pub const DEMO_MODE: &str = "USB";
pub const DEMO_PASSBAND_HZ: i32 = 2400;
pub const DEMO_STRENGTH_DB: f64 = 20.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    SetFreq(f64),
    SetMode {
        mode: String,
        passband_hz: i32,
    },
    /// Mode of the VFO that is not on the main display.
    SetSubMode {
        mode: String,
        passband_hz: i32,
    },
    /// Frequency of the VFO that is not on the main display.
    SetSplitFreq(f64),
    SetVfo(String),
    /// Hamlib `vfo_op` token, for example `XCHG` or `CPY`.
    VfoOp(String),
    SetRit(i32),
    SetLevel {
        name: String,
        value: f64,
    },
    SetFunc {
        name: String,
        on: bool,
    },
    SetPtt(bool),
    /// Antenna number. The option byte is always 0; `get_ant` is not polled.
    SetAnt(i32),
    SetSplit {
        on: bool,
        tx_vfo: String,
    },
    SetPower(bool),
}

impl Command {
    /// The rigctld command body. The live client sends this after the `+\`
    /// extended-response prefix. Polls are not commands.
    pub fn cat_line(&self) -> String {
        match self {
            Command::SetFreq(hz) => format!("set_freq {}", hz.round() as i64),
            Command::SetMode { mode, passband_hz } => {
                format!("set_mode {mode} {passband_hz}")
            }
            Command::SetSubMode { mode, passband_hz } => {
                format!("set_split_mode {mode} {passband_hz}")
            }
            Command::SetSplitFreq(hz) => format!("set_split_freq {}", hz.round() as i64),
            Command::SetVfo(name) => format!("set_vfo {name}"),
            Command::VfoOp(op) => format!("vfo_op {op}"),
            Command::SetRit(hz) => format!("set_rit {hz}"),
            Command::SetLevel { name, value } => format!("set_level {name} {value}"),
            Command::SetFunc { name, on } => {
                format!("set_func {name} {}", if *on { 1 } else { 0 })
            }
            Command::SetPtt(on) => format!("set_ptt {}", if *on { 1 } else { 0 }),
            Command::SetAnt(n) => format!("set_ant {n} 0"),
            Command::SetSplit { on, tx_vfo } => {
                format!("set_split_vfo {} {tx_vfo}", if *on { 1 } else { 0 })
            }
            Command::SetPower(on) => format!("set_powerstat {}", if *on { 1 } else { 0 }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Demo,
    Live,
    Down,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub kind: LinkKind,
    pub model: String,
    pub endpoint: String,
    /// Last command failure. Empty when the last exchange worked.
    pub message: String,
    /// Last operator command handed to the rig, as a rigctld body.
    /// Background polls are not recorded.
    pub last_cat: String,
    pub freq_hz: f64,
    pub mode: String,
    pub passband_hz: i32,
    pub sub_mode: String,
    /// Frequency of the paired VFO (`sub_vfo`). 0 when the rig has not reported one.
    pub sub_freq_hz: f64,
    /// Hamlib token for the paired VFO, such as `VFOB` or `Sub`. Empty when
    /// the rig's VFO list has no pair for [`Snapshot::vfo`].
    pub sub_vfo: String,
    pub vfo: String,
    pub tx_vfo: String,
    pub rit_hz: i32,
    pub ptt: bool,
    pub power_on: bool,
    pub ant: i32,
    /// dB relative to S9. S9 is 0, S9+20 is 20, S0 is about -54.
    pub strength_db: f64,
    pub levels: BTreeMap<String, f64>,
    pub funcs: BTreeMap<String, bool>,
    pub caps: Caps,
    pub roofing: Option<String>,
}

impl Snapshot {
    pub fn online(&self) -> bool {
        self.kind != LinkKind::Down
    }
}

#[derive(Debug)]
pub enum RigError {
    /// The socket failed or the peer never answered. The session is dead.
    Io(String),
    /// Hamlib returned `RPRT` other than 0. The session is still up.
    Remote(String),
}

impl std::fmt::Display for RigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RigError::Io(msg) | RigError::Remote(msg) => f.write_str(msg),
        }
    }
}

pub trait Rig {
    fn apply(&mut self, cmd: &Command) -> Result<(), RigError>;
    fn poll(&mut self) -> Result<Snapshot, RigError>;
}

/// The other half of a hamlib VFO pair. `VFOA` pairs with `VFOB`, and `Main`
/// pairs with `Sub`, including the A/B/C forms. The name is returned in the
/// spelling from `vfos` when that list has it. An empty list still returns the
/// canonical token.
pub fn other_vfo_name(current: &str, vfos: &[String]) -> Option<String> {
    const PAIRS: &[(&str, &str)] = &[
        ("VFOA", "VFOB"),
        ("Main", "Sub"),
        ("MainA", "SubA"),
        ("MainB", "SubB"),
        ("MainC", "SubC"),
    ];
    let current = current.trim();
    if current.is_empty() {
        return None;
    }
    let other = PAIRS.iter().find_map(|(left, right)| {
        if current.eq_ignore_ascii_case(left) {
            Some(*right)
        } else if current.eq_ignore_ascii_case(right) {
            Some(*left)
        } else {
            None
        }
    })?;
    if vfos.is_empty() {
        return Some(other.to_string());
    }
    vfos.iter()
        .find(|name| name.eq_ignore_ascii_case(other))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vfo_pairs_follow_the_rig_list() {
        let yaesu = vec!["VFOA".into(), "VFOB".into(), "VFOC".into()];
        assert_eq!(other_vfo_name("VFOA", &yaesu).as_deref(), Some("VFOB"));
        assert_eq!(other_vfo_name("VFOB", &yaesu).as_deref(), Some("VFOA"));
        assert_eq!(other_vfo_name("VFOC", &yaesu), None);

        let main_sub = vec!["Main".into(), "Sub".into()];
        assert_eq!(other_vfo_name("Main", &main_sub).as_deref(), Some("Sub"));
        assert_eq!(other_vfo_name("Sub", &main_sub).as_deref(), Some("Main"));
        assert_eq!(other_vfo_name("VFOA", &main_sub), None);

        let split = vec!["MainA".into(), "MainB".into(), "SubA".into(), "SubB".into()];
        assert_eq!(other_vfo_name("MainA", &split).as_deref(), Some("SubA"));
        assert_eq!(other_vfo_name("SubB", &split).as_deref(), Some("MainB"));
    }

    #[test]
    fn cat_line_is_the_rigctld_body() {
        assert_eq!(
            Command::SetFreq(14_153_608.4).cat_line(),
            "set_freq 14153608"
        );
        assert_eq!(
            Command::SetMode {
                mode: "USB".into(),
                passband_hz: 2400,
            }
            .cat_line(),
            "set_mode USB 2400"
        );
        assert_eq!(Command::SetPtt(true).cat_line(), "set_ptt 1");
        assert_eq!(Command::SetPtt(false).cat_line(), "set_ptt 0");
        assert_eq!(
            Command::SetSplit {
                on: true,
                tx_vfo: "VFOB".into(),
            }
            .cat_line(),
            "set_split_vfo 1 VFOB"
        );
    }
}
