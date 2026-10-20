//! The app's end of the bridge: the socket it listens on so the native host can hand
//! it a request instead of opening the vault file itself.
//!
//! The point of it is the master password: when this is listening there is no reason
//! for the browser to ask for one, and none for it to hold. The box is already open
//! here; the extension asks *this* process, the way KeePassXC asks its own app.

pub mod logic;
pub mod model;
pub mod ui;

pub use model::{Bridge, Effect};
