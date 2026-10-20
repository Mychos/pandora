//! Pandora — offline secrets vault, one portable encrypted file.
//!
//! A library so the same code serves the desktop binary (`main.rs`) and the
//! Android entry point ([`android_main`]); iOS follows the same shape.

pub mod app;
pub mod bridge;
pub mod features;
pub mod host;
pub mod shared;
pub mod vault;

/// What Android calls to start the app. The activity hands us its own handle, so
/// the app can reach the platform's file system, clipboard and keystore — and it
/// is what `eframe` needs to build its event loop.
#[cfg(target_os = "android")]
#[no_mangle]
pub fn android_main(activity: android_activity::AndroidApp) {
    // A phone has no console: everything the app and `eframe` say goes to
    // `adb logcat -s pandora`, a panic included. `with_max_level` is not
    // cosmetic — `init_once` only calls `log::set_max_level` when the config
    // carries one, and `log` starts at `Off`, so without it the crate logs
    // nothing at all and a panic leaves a process that vanished.
    //
    // `Info` and not `Debug`: a launch logs ~2500 debug lines, most of them
    // `naga` narrating its own shader lowering, and they bury the few that
    // matter. Everything above is still there — the fonts, the warnings, the
    // panics. Chasing a renderer or lifecycle problem means putting `Debug`
    // back for as long as it takes.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("pandora"),
    );
    log_panics();
    shared::android::init(&activity);
    if let Some(dir) = activity.internal_data_path() {
        vault::store::set_data_dir(dir);
    }
    let _hold = app::hold().unwrap_or_else(|e| {
        log::error!("pandora: another instance is already running ({e})");
        std::process::exit(1);
    });
    let options = eframe::NativeOptions {
        android_app: Some(activity),
        ..Default::default()
    };
    let _ = eframe::run_native(
        "Pandora",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    );
}

/// A panic on a phone is otherwise a process that vanished: the message and where
/// it happened are the whole diagnosis.
#[cfg(target_os = "android")]
fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        previous(info);
    }));
}
