//! Pandora — offline secrets vault, one portable encrypted file. A library so the
//! desktop binary and `android_main` run the same app.

pub mod app;
pub mod bridge;
pub mod features;
pub mod host;
pub mod shared;
pub mod vault;

/// Android's entry: the activity handle also gives access to files, clipboard and
/// keystore, and is what `eframe` needs for its event loop.
#[cfg(target_os = "android")]
#[no_mangle]
pub fn android_main(activity: android_activity::AndroidApp) {
    // A phone has no console: logs and panics go to `adb logcat -s pandora`. Without
    // `with_max_level`, `init_once` leaves `log` at `Off` and a panic leaves nothing.
    // `Info`, not `Debug`: ~2500 debug lines, most of them `naga`, bury the rest.
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

/// A panic on a phone is otherwise a vanished process; the message and location are
/// the whole diagnosis.
#[cfg(target_os = "android")]
fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        previous(info);
    }));
}
