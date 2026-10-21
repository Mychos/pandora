//! The desktop entry point; everything it runs lives in the library, so Android
//! starts the same app through `android_main`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_os = "android"))]
use pandora::{app, shared};

/// The phone loads the cdylib and never runs this binary; this empty `main` only
/// gives the whole-package build a binary target to compile.
#[cfg(target_os = "android")]
fn main() {}

#[cfg(not(target_os = "android"))]
fn main() -> eframe::Result {
    // Wayland unless `PANDORA_X11=1`: winit 0.30 only emits `DroppedFile` from X11,
    // so a Wayland session would lose drag-and-drop.
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
            // XWayland also exports DISPLAY; drop it to stay on Wayland.
            std::env::remove_var("DISPLAY");
        }
    }
    #[cfg(feature = "glow")]
    let renderer = if std::env::var("PANDORA_RENDERER").is_ok_and(|value| value == "wgpu") {
        eframe::Renderer::Wgpu
    } else {
        eframe::Renderer::Glow
    };
    #[cfg(not(feature = "glow"))]
    let renderer = eframe::Renderer::Wgpu;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Pandora")
            // A Wayland surface has no icon; GNOME matches this id to the .desktop file.
            .with_app_id("pandora")
            .with_icon(shared::theme::window_icon())
            .with_decorations(false)
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([380.0, 520.0]),
        // One frame normally; `App::ui` raises it while a resize is in flight.
        wgpu_options: eframe::WgpuConfiguration::default()
            .with_surface_config(eframe::SurfaceConfig::LOW_LATENCY),
        renderer,
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
