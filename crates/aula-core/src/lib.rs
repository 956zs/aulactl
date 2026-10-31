//! Protocol, hidraw transport and settings model for the AULA WIN 60 HE keyboard (`docs/protocol.md`).

pub mod error;
pub mod frame;
pub mod hidraw;
pub mod keycodes;
pub mod layout;
pub mod perkey;

pub use error::{Error, Result};
