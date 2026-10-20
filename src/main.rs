//! Pandora — offline secrets vault, one portable encrypted file.
//!
//! The desktop entry point. Everything it runs lives in the library, so Android
//! (see `android_main`) starts the very same app.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_os = "android"))]
use pandora::{app, shared};

/// Android never runs this binary — it loads the cdylib and `lib.rs` owns
/// `android_main`. The empty `main` is only so a build of the whole package
/// for the phone (which is how `scripts/android-apk.sh` asks for the `.so`)
/// still has a binary target to compile.
#[cfg(target_os = "android")]
fn main() {}

#[cfg(not(target_os = "android"))]
fn main() -> eframe::Result {
    // Native Wayland gives the compositor the resize gesture, which is the
    // fluid path on a modern desktop. winit 0.30 still only emits file drop
    // events from X11 (`DroppedFile` is an X11/macOS/Windows event), so a
    // Wayland session loses drag-and-drop. That is the trade: Wayland unless
    // `PANDORA_X11=1` asks for the X11 backend to keep dropping files on the
    // window.
    #[cfg(target_os = "linux")]
    {
        let force_x11 = std::env::var_os("PANDORA_X11").is_some_and(|v| v != "0");
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        if force_x11 {
            if std::env::var_os("DISPLAY")
                .map(|d| !d.is_empty())
                .unwrap_or(false)
            {
                std::env::remove_var("WAYLAND_DISPLAY");
                std::env::remove_var("WAYLAND_SOCKET");
            }
        } else if wayland {
            // Keep winit on Wayland even though XWayland also exports DISPLAY.
            std::env::remove_var("DISPLAY");
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Pandora")
            // On Wayland a surface has no icon: GNOME matches the window to the
            // `.desktop` file with this id (`assets/app/linux/pandora.desktop`).
            .with_app_id("pandora")
            .with_icon(shared::theme::window_icon())
            .with_decorations(false)
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([380.0, 520.0]),
        // One frame of swapchain room normally; `App::ui` raises it to
        // `HIGH_THROUGHPUT` while a resize is in flight, so the edge keeps up
        // without leaving the whole app a frame behind its pointer.
        wgpu_options: eframe::WgpuConfiguration::default()
            .with_surface_config(eframe::SurfaceConfig::LOW_LATENCY),
        ..Default::default()
    };
    let _hold = app::hold().unwrap_or_else(|e| {
        eprintln!("pandora: another instance is already running ({e})");
        std::process::exit(1);
    });
    eframe::run_native(
        "Pandora",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
