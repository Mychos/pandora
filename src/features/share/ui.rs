//! The share dialog: one record out, or one file in. Both ask for the passphrase in
//! the same field, and both say who sealed it before anything is written or read.

use crate::app::{App, StatusKind};
use crate::features::share::Form;
use crate::shared::i18n::{t, tf};
use crate::shared::theme;
use crate::shared::utils::import;
use crate::vault::share::{self, SealTo};

/// Pick a `.pandora-share` and ask how to open it. The desktop dialog runs off the
/// frame thread, so the window keeps painting while it is up.
pub fn pick(app: &mut App, ctx: &egui::Context) {
    #[cfg(target_os = "android")]
    match crate::shared::android::text_with("pick", "*/*") {
        Ok(Some(path)) => app.share.open(std::path::PathBuf::from(path)),
        Ok(None) => {}
        Err(e) => app.shell.fail(e),
    };
    #[cfg(not(target_os = "android"))]
    app.ask(
        ctx,
        || {
            rfd::FileDialog::new()
                .add_filter("Pandora share", &[share::EXT])
                .pick_file()
                .into_iter()
                .collect()
        },
        |app, mut paths| {
            if let Some(path) = paths.pop() {
                app.share.open(path);
            }
        },
    );
}

pub fn modal(ctx: &egui::Context, app: &mut App) {
    if app.share.form.is_none() {
        return;
    }
    let mut act = false;
    let form = app.share.form.clone().expect("checked above");
    let title = match &form {
        Form::Send { .. } => t("Share a record"),
        Form::Open { .. } => t("Open a share"),
    };
    app.boxes.editing(true);
    let open = theme::dialog(ctx, egui::Id::new("share"), |ui| {
        theme::width(ui, 360.0);
        let mut closed = false;
        ui.horizontal(|ui| {
            crate::shared::ui::balance::label(
                ui,
                egui::RichText::new(title)
                    .strong()
                    .color(theme::palette().text),
            );
            closed = theme::close(ui, &app.icons);
        });
        ui.add_space(10.0);
        match &form {
            Form::Send { id } => send(ui, app, id, &mut act),
            Form::Open { path } => open_file(ui, app, path, &mut act),
        }
        ui.add_space(12.0);
        if let Some(error) = &app.share.error {
            crate::shared::ui::balance::label(
                ui,
                egui::RichText::new(error).color(theme::palette().error),
            );
            ui.add_space(8.0);
        }
        let label = match form {
            Form::Send { .. } => t("Seal it"),
            Form::Open { .. } => t("Open it"),
        };
        if theme::primary_button(ui, &app.icons, None, label, None).clicked() {
            act = true;
        }
        closed
    });
    let close = !open;
    if act {
        match app.share.form.clone() {
            Some(Form::Send { id }) => seal(app, &id),
            Some(Form::Open { path }) => land(app, &path),
            None => {}
        }
    }
    if close || !open {
        app.share.clear();
        app.boxes.editing(false);
        app.boxes.flush(&mut app.shell);
    }
}

fn send(ui: &mut egui::Ui, app: &mut App, id: &str, _act: &mut bool) {
    let name = app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.vault.any(id))
        .map(|it| it.name().to_owned())
        .unwrap_or_default();
    crate::shared::ui::balance::label(
        ui,
        egui::RichText::new(tf("“{name}” leaves this box sealed", &[("name", &name)]))
            .color(theme::palette().muted),
    );
    ui.add_space(8.0);
    theme::hint(
        ui,
        t("a passphrase is the simpler way and the weaker one — the other party has to know it. A key is a line the two of you exchange once, and it never travels with the file"),
    );
    ui.add_space(8.0);
    ui.add(
        egui::TextEdit::singleline(&mut app.share.passphrase)
            .password(true)
            .hint_text(t("passphrase (or seal to a key below)"))
            .desired_width(f32::INFINITY),
    );
    ui.add_space(6.0);
    ui.add(
        egui::TextEdit::singleline(&mut app.share.to)
            .hint_text(t("their share key (or use a passphrase above)"))
            .desired_width(f32::INFINITY),
    );
}

fn open_file(ui: &mut egui::Ui, app: &mut App, path: &std::path::Path, _act: &mut bool) {
    let file = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let info = std::fs::read(path)
        .ok()
        .and_then(|bytes| crate::vault::share::info(&bytes));
    crate::shared::ui::balance::label(
        ui,
        egui::RichText::new(match &info {
            Some(info) if !info.from.is_empty() => tf(
                "“{file}”, sealed by {from}",
                &[("file", &file), ("from", &info.from)],
            ),
            _ => tf("“{file}”", &[("file", &file)]),
        })
        .color(theme::palette().muted),
    );
    ui.add_space(8.0);
    if info
        .as_ref()
        .is_some_and(|it| it.mode == share::Mode::Passphrase)
    {
        ui.add(
            egui::TextEdit::singleline(&mut app.share.passphrase)
                .password(true)
                .hint_text(t("passphrase"))
                .desired_width(f32::INFINITY),
        );
    } else if app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.vault.share.as_ref())
        .is_none()
    {
        theme::hint(
            ui,
            t("this share is sealed to a key, and this box has no key yet — the one it is for made it"),
        );
    }
}

/// Seal one record and write the file where the user's other exports land.
fn seal(app: &mut App, id: &str) {
    let Some(s) = app.boxes.session.as_ref() else {
        return;
    };
    let name = s
        .vault
        .any(id)
        .map(|it| it.name().to_owned())
        .unwrap_or_default();
    let bundle = match crate::features::share::bundle(&s.vault, id) {
        Ok(bundle) => bundle,
        Err(e) => {
            app.share.error = Some(e);
            return;
        }
    };
    let from = s.vault.name.clone();
    let mode = if app.share.passphrase.is_empty() {
        match share::public_array(&app.share.to) {
            Ok(key) => SealTo::Key(key),
            Err(e) => {
                app.share.error = Some(e);
                return;
            }
        }
    } else {
        SealTo::Passphrase(&app.share.passphrase)
    };
    let sealed = match share::seal(&bundle, &from, &mode, crate::vault::model::now_secs()) {
        Ok(bytes) => bytes,
        Err(e) => {
            app.share.error = Some(e);
            return;
        }
    };
    let file = format!("{}.{}", crate::vault::store::sanitize(&name), share::EXT);
    let path = match crate::shared::utils::csv::download_dir() {
        Ok(dir) => dir.join(file),
        Err(e) => {
            app.share.error = Some(e);
            return;
        }
    };
    match crate::vault::store::write_private(&path, &sealed) {
        Ok(()) => {
            app.share.clear();
            app.shell.set_status(
                tf(
                    "share written to {path} — only its key or passphrase opens it",
                    &[("path", &path.display())],
                ),
                StatusKind::Saved,
            );
        }
        Err(e) => app.share.error = Some(e),
    }
}

/// Open one file into the box on screen, through the same landing an import uses.
fn land(app: &mut App, path: &std::path::Path) {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            app.share.error = Some(e.to_string());
            return;
        }
    };
    let identity = app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.vault.share.as_ref())
        .cloned();
    let passphrase = (!app.share.passphrase.is_empty()).then(|| app.share.passphrase.clone());
    let bundle = match share::open(&bytes, identity.as_ref(), passphrase.as_deref()) {
        Ok(bundle) => bundle,
        Err(e) => {
            app.share.error = Some(e);
            return;
        }
    };
    let mut placed = import::Placed::default();
    if let Some(s) = app.boxes.session.as_mut() {
        placed = import::land(&mut s.vault, bundle);
        if placed.total() > 0 {
            s.selected = None;
            s.mark_now();
        }
    }
    app.share.clear();
    app.shell.set_status(
        crate::features::settings::import::summary(&placed),
        StatusKind::Saved,
    );
}
