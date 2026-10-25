//! RF patch bay on the Config tab (#98). Visual only: nothing here drives hardware.

pub mod geom;
pub mod model;
mod view;

pub use model::PatchBay;
pub use view::{show, UiState};
