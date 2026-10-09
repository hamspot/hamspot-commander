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
    pub freq_hz: f64,
    pub mode: String,
    pub passband_hz: i32,
    pub sub_mode: String,
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
