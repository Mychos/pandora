//! The live scanner: what the pane keeps while a camera is open.

use std::time::{Duration, Instant};

/// How often a frame is taken from the camera. The camera runs faster than this:
/// a frame is a few megabytes and every one crosses the JNI boundary as a copy,
/// so the preview keeps a pace that is smooth enough to aim and no smoother.
const FRAME: Duration = Duration::from_millis(100);

/// The reader looks at one frame in this many. A QR coming into view is caught
/// within a couple of frames, and the phone's CPU is left to the preview.
const READ_EVERY: u32 = 3;

/// A read on its own thread: the frame went out with it, and the answer comes
/// back through it. Reading a 1080p frame takes long enough that doing it between
/// two drawn frames is what makes a camera stutter.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub struct Reading(pub(crate) std::sync::mpsc::Receiver<Option<(Option<String>, String)>>);

/// A scan in progress. `Option<Scan>` on the session is the scanner being up.
pub struct Scan {
    /// What the camera answers with: every frame's shape, in pixels.
    pub width: u32,
    pub height: u32,
    /// How far the frame has to turn to stand up, in degrees.
    pub turn: u32,
    /// When the pane may take the next frame.
    next: Instant,
    /// Frames taken so far, which is what picks the ones the reader sees.
    taken: u32,
    /// The read in flight, if any: one at a time, so a slow frame is skipped
    /// rather than queued.
    pub reading: Option<Reading>,
}

impl Scan {
    pub fn new(width: u32, height: u32, turn: u32) -> Self {
        Self {
            width,
            height,
            turn,
            next: Instant::now(),
            taken: 0,
            reading: None,
        }
    }

    /// Take the frame this moment is for: `None` while it is too soon for
    /// another, else whether the reader should look at it.
    pub fn take(&mut self, now: Instant) -> Option<bool> {
        if now < self.next {
            return None;
        }
        self.next = now + FRAME;
        self.taken = self.taken.wrapping_add(1);
        Some(self.taken.is_multiple_of(READ_EVERY))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame is taken at the pane's own pace, not on every repaint, and the
    /// reader sees every third one.
    #[test]
    fn frames_are_taken_at_a_pace_and_read_in_turns() {
        let mut scan = Scan::new(640, 480, 90);
        let start = Instant::now();

        // The pane asks once per frame; the reader sees every third.
        let mut reads = Vec::new();
        for turn in 0..6 {
            if let Some(read) = scan.take(start + FRAME * turn) {
                reads.push(read);
            }
        }
        assert_eq!(
            reads,
            [false, false, true, false, false, true],
            "one frame in three is read"
        );
        assert_eq!(
            scan.take(start + 5 * FRAME + FRAME / 2),
            None,
            "and a repaint in between takes nothing"
        );
    }
}
