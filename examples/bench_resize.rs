//! Headless resize benchmark: runs the real `App::run_ui` at alternating widths
//! with a vault of `N` entries and reports the per-frame CPU cost. It exists to
//! keep the resize path honest; run it with an isolated runtime dir, e.g.
//!
//! ```text
//! XDG_RUNTIME_DIR=/tmp/pandora-bench cargo run --release --example bench_resize -- 1000 120
//! XDG_RUNTIME_DIR=/tmp/pandora-bench cargo run --release --example bench_resize -- 1000 120 folders
//! ```
//!
//! The first argument is the entry count, the second the number of frames; a
//! trailing `folders` switches the passwords pane to its generated folders.

use pandora::app::{App, Screen, Tab};
use pandora::features::boxes::model::Session;
use pandora::features::entries::model::{Axis, View};
use pandora::vault::model::{now_secs, Folder, Login, Note, Vault};
use pandora::vault::store::Credentials;
use std::time::{Duration, Instant};

fn make_vault(n: usize, drive: bool) -> Vault {
    let now = now_secs();
    let mut vault = Vault::new("Bench");
    let mut folder = Folder::new("Folder", None, now);
    folder.id = "folder".to_owned();
    vault.folders.push(folder);

    for i in 0..n {
        let mut login = Login::new(now);
        login.name = format!("Service {i:05}");
        login.username = format!("user{i:05}@example.com");
        login.url = format!("https://service{}.example.com/login", i % 250);
        login.notes = format!("notes for entry {i}").into();
        vault.logins.push(login);
    }
    for i in 0..(if drive { n } else { n / 10 }).max(1) {
        let mut note = Note::new(now);
        note.name = format!("Note {i:04}");
        note.folder = Some("folder".to_owned());
        note.body = format!("body of note {i}").into();
        vault.drive.push(pandora::vault::model::Entry::Note(note));
    }
    vault
}

fn percentile(values: &[Duration], p: f64) -> Duration {
    let mut sorted = values.to_vec();
    sorted.sort();
    let at = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[at]
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let frames: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let folders = std::env::args().any(|a| a == "folders");
    let drive = std::env::args().any(|a| a == "drive");

    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = App::new(&cc);

    let creds = Credentials::rekey("bench-password", None).expect("derive a key");
    let session = Session::fresh(
        std::env::temp_dir().join("pandora-bench.vault"),
        creds,
        make_vault(n, drive),
    );
    app.boxes.session = Some(session);
    app.screen = Screen::Main;
    if let Some(s) = app.boxes.session.as_mut() {
        s.tab = if drive { Tab::Drive } else { Tab::Passwords };
        if drive {
            s.folder = Some("folder".to_owned());
        }
        s.view = if folders { View::Folders } else { View::List };
        s.axis = Axis::Target;
    }

    let input = |w: f32| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(w, 700.0),
        )),
        ..Default::default()
    };

    // Warm up the fonts, icon textures and egui's layout caches.
    for frame in 0..8 {
        let w = if frame % 2 == 0 { 1000.0 } else { 1400.0 };
        let mut out = ctx.run_ui(input(w), |ui| app.run_ui(ui));
        out.textures_delta.clear();
    }

    let mut times = Vec::with_capacity(frames);
    for frame in 0..frames {
        let w = if frame % 2 == 0 { 1000.0 } else { 1400.0 };
        let started = Instant::now();
        let mut out = ctx.run_ui(input(w), |ui| app.run_ui(ui));
        out.textures_delta.clear();
        std::hint::black_box(&out.shapes);
        times.push(started.elapsed());
    }

    let total: Duration = times.iter().sum();
    println!(
        "n={n} view={} frames={frames} mean={:.3}ms p50={:.3}ms p95={:.3}ms max={:.3}ms",
        if drive {
            "drive"
        } else if folders {
            "folders"
        } else {
            "list"
        },
        total.as_secs_f64() * 1000.0 / frames as f64,
        percentile(&times, 0.50).as_secs_f64() * 1000.0,
        percentile(&times, 0.95).as_secs_f64() * 1000.0,
        percentile(&times, 1.0).as_secs_f64() * 1000.0,
    );
}
