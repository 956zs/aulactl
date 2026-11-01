//! Protocol, hidraw transport and settings model for the AULA WIN 60 HE keyboard (`docs/protocol.md`).
//!
//! Pure encode/decode functions live in the feature modules (`lighting`, `keymap`, `macros`, `advanced`,
//! `trigger`, `socd`, `travel`, `info`); `Keyboard` does the I/O. Writes are refused unless the board
//! identifies as a WIN 60 HE.

pub mod advanced;
pub mod align;
pub mod error;
pub mod feed;
pub mod frame;
pub mod hidraw;
pub mod info;
pub mod keyboard;
pub mod keycodes;
pub mod keymap;
pub mod layout;
pub mod lighting;
pub mod macros;
pub mod osr;
pub mod overlay;
pub mod perkey;
pub mod replay;
pub mod report;
#[cfg(any(test, feature = "sim"))]
pub mod sim;
pub mod session;
pub mod snapshot;
pub mod socd;
pub mod store;
pub mod travel;
pub mod trigger;

#[cfg(test)]
mod tests;

pub use error::{Error, Result};
pub use keyboard::{Calibration, Keyboard, TravelStream};
pub use snapshot::Snapshot;
pub use travel::Confirm;
