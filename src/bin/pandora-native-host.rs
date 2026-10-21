//! The stdio end of the native-messaging host: the framed protocol on stdin/stdout,
//! while every secret operation lives in `pandora::host`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(e) = pandora::host::run() {
        eprintln!("pandora native host: {e}");
        std::process::exit(1);
    }
}
