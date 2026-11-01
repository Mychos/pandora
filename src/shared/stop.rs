//! The two signals that end a process where it stands: a logout, a `systemctl
//! stop`, a `kill`. The window never sees them and `on_exit` never runs, so the
//! handler only raises a flag and the frame loop gets to write what is still in
//! memory before it closes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static ASKED: AtomicBool = AtomicBool::new(false);

/// Nothing but a store: a handler may not allocate, lock, or touch the vault.
extern "C" fn ask(_signal: libc::c_int) {
    ASKED.store(true, Ordering::SeqCst);
}

/// Take over the two signals that used to end the run mid-edit, and keep a thread
/// that wakes the frame which will act on them: a window nobody touches draws no
/// frame, and a flag nobody reads writes nothing.
pub fn listen(ctx: egui::Context) {
    // SAFETY: `ask` writes one atomic, which is async-signal-safe, and the mask is
    // the empty set the handler is documented to accept.
    unsafe {
        let mut handler: libc::sigaction = std::mem::zeroed();
        handler.sa_sigaction = ask as *const () as usize;
        handler.sa_flags = 0;
        for signal in [libc::SIGTERM, libc::SIGINT] {
            libc::sigaction(signal, &handler, std::ptr::null_mut());
        }
    }
    // Polling, not a pipe: a handler may only store, and the frame it wakes does the
    // writing. A tenth of a second is nothing next to the wait it replaces.
    std::thread::Builder::new()
        .name("stop".to_owned())
        .spawn(move || loop {
            if asked() {
                ctx.request_repaint();
            }
            std::thread::sleep(Duration::from_millis(100));
        })
        .ok();
}

/// Whether one of them has arrived.
pub fn asked() -> bool {
    ASKED.load(Ordering::SeqCst)
}
