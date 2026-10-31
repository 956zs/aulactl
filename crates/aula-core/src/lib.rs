//! Protocol, hidraw transport and settings model for the AULA WIN 60 HE keyboard (`docs/protocol.md`).
//!
//! Pure encode/decode functions live in the feature modules (`lighting`, `keymap`, `macros`, `advanced`,
//! `trigger`, `socd`, `travel`, `info`).

pub mod advanced;
pub mod error;
pub mod frame;
pub mod hidraw;
pub mod info;
pub mod keycodes;
pub mod keymap;
pub mod layout;
pub mod lighting;
pub mod macros;
pub mod perkey;
pub mod socd;
pub mod travel;
pub mod trigger;

pub use error::{Error, Result};
pub use travel::Confirm;
