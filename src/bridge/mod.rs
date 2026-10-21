//! Browser messaging: the framed messages and the local socket the app listens on
//! so the host can relay to an already unlocked box. No egui, no features.

pub mod protocol;
pub mod serve;
pub mod socket;
