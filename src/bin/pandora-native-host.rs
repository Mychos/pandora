//! The stdio end of the browser extension's native-messaging host. The browser
//! launches this binary and speaks the framed protocol on its stdin/stdout; every
//! secret operation lives in the library (`pandora::host`), so this file only
//! reports a fatal error and sets the exit code.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(e) = pandora::host::run() {
        eprintln!("pandora native host: {e}");
        std::process::exit(1);
    }
}
