//! Keeping the window out of screen captures. A vault on screen that a screen
//! share, a recording or a screenshot picks up is a vault handed to whoever is
//! watching, so the operating system is asked to leave the window out.

use raw_window_handle::RawWindowHandle;

/// Whether this system can keep a window out of a capture. Windows and macOS can;
/// Linux cannot, because neither X11 nor Wayland offers an API for it.
pub fn supported() -> bool {
    cfg!(any(target_os = "windows", target_os = "macos"))
}

/// Let the window be captured (`allow`), or keep it out of every capture.
pub fn apply(handle: Option<&RawWindowHandle>, allow: bool) {
    #[cfg(target_os = "windows")]
    windows(handle, allow);
    #[cfg(target_os = "macos")]
    macos(handle, allow);
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = (handle, allow);
}

#[cfg(target_os = "windows")]
fn windows(handle: Option<&RawWindowHandle>, allow: bool) {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_MONITOR, WDA_NONE,
    };

    let Some(RawWindowHandle::Win32(win)) = handle else {
        return;
    };
    let hwnd = win.hwnd.get() as HWND;
    unsafe {
        if allow {
            SetWindowDisplayAffinity(hwnd, WDA_NONE);
        } else if SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) == 0 {
            // Before Windows 10 2004 there is only WDA_MONITOR: it blanks the
            // window in a capture instead of leaving it out of one.
            SetWindowDisplayAffinity(hwnd, WDA_MONITOR);
        }
    }
}

#[cfg(target_os = "macos")]
fn macos(handle: Option<&RawWindowHandle>, allow: bool) {
    use objc2_app_kit::{NSView, NSWindowSharingType};

    let Some(RawWindowHandle::AppKit(win)) = handle else {
        return;
    };
    // The handle is the view eframe draws into, and sharing is decided on the
    // window holding it. Both are read on this thread, which is the one painting.
    let view = unsafe { &*(win.ns_view.as_ptr() as *const NSView) };
    let Some(window) = view.window() else {
        return;
    };
    window.setSharingType(if allow {
        NSWindowSharingType::ReadOnly
    } else {
        NSWindowSharingType::None
    });
}

#[cfg(all(test, not(any(target_os = "windows", target_os = "macos"))))]
mod tests {
    use super::*;

    /// A system without the API says so instead of promising a window nobody can
    /// capture, and applying it is not an error either.
    #[test]
    fn a_system_without_the_api_says_so() {
        assert!(!supported());
        apply(None, false);
        apply(None, true);
    }
}
