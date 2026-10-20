use crate::features::shell::clipboard::Clip;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Saved,
    Info,
    Error,
}

pub struct Status {
    pub at: Instant,
    pub text: String,
    pub kind: StatusKind,
}

#[derive(Default)]
pub struct Shell {
    /// Whether the Pandora window has focus.
    pub focused: bool,
    pub activity: Option<Instant>,
    pub status: Option<Status>,
    /// The last secret copied out, waiting for its countdown.
    pub clip: Option<Clip>,
}
