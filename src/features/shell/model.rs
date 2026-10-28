use crate::features::shell::clipboard::Clip;
use std::time::{Duration, Instant};

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

/// How long a native move that never lost focus keeps the curtain away.
const MOVE_GRACE: Duration = Duration::from_secs(1);

/// A move that never reports focus back cannot suppress the curtain forever.
const MOVE_MAX: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Shell {
    /// Whether the Pandora window has focus.
    pub focused: bool,
    /// The window is being moved or resized by the compositor: it can report no
    /// focus while the native gesture owns the pointer, and that is not background.
    moving: bool,
    /// Focus was seen gone since the gesture began, so its return ends the move.
    moving_lost: bool,
    moving_since: Option<Instant>,
    pub activity: Option<Instant>,
    pub status: Option<Status>,
    /// The last secret copied out, waiting for its countdown.
    pub clip: Option<Clip>,
}

impl Shell {
    /// The compositor has taken the pointer for a native move or resize.
    pub fn moved(&mut self) {
        self.moving = true;
        self.moving_lost = false;
        self.moving_since = Some(Instant::now());
    }

    /// Whether the curtain must stay away for a native gesture.
    pub fn moving(&self) -> bool {
        self.moving
    }

    /// A move can lose focus for as long as the gesture lasts. Keep the curtain
    /// away until focus comes back, with a cap so a lost event cannot disable it.
    pub fn settle(&mut self, focused: bool, released: bool) {
        if !self.moving {
            return;
        }
        if !focused {
            self.moving_lost = true;
        }
        let age = self.moving_since.map_or(Duration::MAX, |at| at.elapsed());
        if released
            || (focused && self.moving_lost)
            || (!self.moving_lost && age >= MOVE_GRACE)
            || age >= MOVE_MAX
        {
            self.moving = false;
            self.moving_lost = false;
            self.moving_since = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_native_move_is_not_background() {
        let mut shell = Shell::default();
        shell.moved();
        shell.settle(true, false);
        assert!(
            shell.moving(),
            "the frame after StartDrag can still read as focused"
        );
        shell.settle(false, false);
        assert!(shell.moving(), "the gesture outlasts the lost focus");
        shell.settle(true, false);
        assert!(!shell.moving(), "focus back ends the gesture");

        shell.moved();
        shell.settle(true, true);
        assert!(
            !shell.moving(),
            "a release ends it even without a focus event"
        );
    }

    #[test]
    fn a_move_that_never_lost_focus_does_not_pin_the_curtain_away() {
        let mut shell = Shell::default();
        shell.moved();
        shell.moving_since = Some(Instant::now() - MOVE_GRACE);
        shell.settle(true, false);
        assert!(
            !shell.moving(),
            "the grace is over, so the window behaves normally"
        );
    }
}
