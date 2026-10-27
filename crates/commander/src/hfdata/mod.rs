//! HF data decode window (#217): one window shared by every HF data mode.
//!
//! - [`slots`]: UTC slot boundaries (FT8 15 s, FT4 7.5 s, WSPR 2 min).
//! - [`clock_check`]: chrony / timedatectl offset check, always shown as text.
//! - [`presets`]: data-driven dial presets checked against the band plan.
//! - [`decodes`]: the decode list (ordering, dedupe, cap, no future rows).
//! - [`registry`]: the plug-in point each mode decoder (#206-#216) joins.
//!
//! Receive only. No demodulator lives here; decoders are external rxpipe
//! programs whose output lines a [`registry::DecodeMode`] turns into rows.

// Parts of the plug-in API are used only by tests until the first decoder lands.
#![allow(dead_code)]

pub mod clock_check;
pub mod decodes;
pub mod presets;
pub mod registry;
pub mod slots;
mod ui;

pub use ui::HfState;
