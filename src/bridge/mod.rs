//! Talking to browsers: the messages the extension and the native host exchange,
//! and — as the bridge takes shape — the local socket the app listens on so the
//! host can relay to a vault that is already open and unlocked.
//!
//! Platform-agnostic core, like `vault/`: no egui, no knowledge of features.

pub mod protocol;
pub mod serve;
pub mod socket;
