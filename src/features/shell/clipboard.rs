//! Clipboard hygiene: every secret this app copies is remembered so a countdown
//! can take it back. Only text this app put there is wiped, identified by reading
//! the clipboard back; where that read fails the text is left alone.

use crate::features::shell::model::{Shell, StatusKind};
use blake2::{Blake2b512, Digest};
use egui::Context;
use std::time::{Duration, Instant};

/// A copy this app made: when, and a fingerprint of what. The fingerprint
/// recognises the text without keeping a second plaintext of the secret around.
#[derive(Debug, Clone, Copy)]
pub struct Clip {
    pub at: Instant,
    pub hash: u64,
}

impl Clip {
    pub fn new(text: &str) -> Self {
        Self {
            at: Instant::now(),
            hash: fingerprint(text),
        }
    }

    pub fn holds(&self, text: &str) -> bool {
        fingerprint(text) == self.hash
    }
}

fn fingerprint(text: &str) -> u64 {
    let digest = Blake2b512::digest(text.as_bytes());
    u64::from_le_bytes(digest[..8].try_into().unwrap_or_default())
}

impl Shell {
    pub fn copy_text(&mut self, ctx: &Context, text: &str) {
        // egui's own clipboard is the desktop's — `egui-winit` compiles arboard
        // out on Android, so there the platform is asked directly.
        #[cfg(not(target_os = "android"))]
        ctx.copy_text(text.to_owned());
        #[cfg(target_os = "android")]
        if let Err(e) = crate::shared::android::send("copy", text) {
            self.fail(e);
        }
        self.arm_clip(ctx, Clip::new(text));
    }

    /// Arm the countdown for a copy the caller wrote out, because the item it
    /// copies from is borrowed at the time.
    pub fn arm_clip(&mut self, ctx: &Context, clip: Clip) {
        self.clip = Some(clip);
        ctx.request_repaint();
    }

    pub fn take_back_clip(&mut self) {
        let Some(clip) = self.clip.take() else {
            return;
        };
        // Only our own text goes: whatever the user copied afterwards stays.
        if read().as_deref().is_some_and(|text| clip.holds(text)) {
            clear();
        }
    }
}

/// What the system clipboard holds, where this platform lets the app read it
/// back. `None` is "cannot tell", which leaves the text alone.
#[cfg(not(target_os = "android"))]
fn read() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

#[cfg(not(target_os = "android"))]
fn clear() {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.clear();
    }
}

#[cfg(target_os = "android")]
fn read() -> Option<String> {
    crate::shared::android::text("paste").ok().flatten()
}

#[cfg(target_os = "android")]
fn clear() {
    let _ = crate::shared::android::send_none("clear");
}

impl Shell {
    /// Runs every frame, box open or not: locking must not leave a password on
    /// the clipboard for the rest of the session.
    pub fn sweep_clip(&mut self, ctx: &Context, clip_clear: u64) {
        let Some(clip) = self.clip else { return };
        if clip_clear == 0 {
            self.clip = None;
            return;
        }
        let left = Duration::from_secs(clip_clear).saturating_sub(clip.at.elapsed());
        if !left.is_zero() {
            // Nothing else asks for frames while the window sits idle.
            ctx.request_repaint_after(left);
            return;
        }
        self.take_back_clip();
        self.set_status("cleared the clipboard", StatusKind::Info);
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;

    #[test]
    fn a_copy_recognises_its_own_text() {
        let clip = Clip::new("hunter2");
        assert!(clip.holds("hunter2"));
        assert!(!clip.holds("hunter3"));
        assert!(!clip.holds(""));
        assert!(!Clip::new("").holds("x"));
        assert!(Clip::new("").holds(""));
    }

    /// `0` means never take it back, decided before the clipboard is touched.
    #[test]
    fn a_grace_of_zero_never_clears() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.prefs.clip_clear = 0;
        app.shell.clip = Some(Clip::new("hunter2"));
        app.shell.sweep_clip(&ctx, app.prefs.clip_clear);
        assert!(app.shell.clip.is_none(), "nothing is left to count down");
    }
}
