//! The settings row that reads another manager's file: pick it, say what it is, and
//! put its records in the open box. Nothing is replaced, and the file stays where
//! the user left it — this app never deletes somebody's export.

use crate::app::{App, Confirm, StatusKind};
use crate::shared::i18n::{t, tf};
use crate::shared::utils::import::{self, Placed};
use std::path::{Path, PathBuf};

/// Ask for a file; `None` when the picker was closed.
fn pick_file() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        match crate::shared::android::text_with("pick", "*/*") {
            Ok(path) => path.map(PathBuf::from),
            Err(_) => None,
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        rfd::FileDialog::new()
            .add_filter(
                "Bitwarden, 1Password, KeePass, CSV",
                &["json", "1pux", "zip", "xml", "csv", "txt"],
            )
            .pick_file()
    }
}

/// The row's click: hand the dialog to the shell so the window keeps painting, and
/// sniff what came back on a later frame.
pub fn pick(app: &mut App, ctx: &egui::Context) {
    app.ask(
        ctx,
        || pick_file().into_iter().collect(),
        |app, mut paths| {
            if let Some(path) = paths.pop() {
                picked(app, path);
            }
        },
    );
}

/// Sniff the file and let the confirm say what is about to land. A file the reader
/// does not know is refused here, before anything is asked.
fn picked(app: &mut App, path: PathBuf) {
    let too_big = std::fs::metadata(&path)
        .map(|meta| meta.len() > import::MAX_BYTES as u64)
        .unwrap_or(false);
    if too_big {
        app.shell.fail(t("this file is too large to import"));
        return;
    }
    let format = match std::fs::read(&path) {
        // Not one of the three: the CSV reader is the other door, and it says what
        // it could not find if it is not a CSV either.
        Ok(bytes) => import::detect(&bytes).ok(),
        Err(e) => {
            app.shell
                .fail(tf("import failed: {error}", &[("error", &e.to_string())]));
            return;
        }
    };
    if let Some(s) = app.boxes.session.as_mut() {
        s.confirm = Some(Confirm::Import { path, format });
    }
}

/// The confirm was accepted: read the file and land its records.
pub fn land(app: &mut App, path: &Path, format: Option<import::Format>) {
    let Some(format) = format else {
        // A CSV keeps its own reader, its own duplicate rules and its own report.
        app.import_copy(path);
        return;
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            let error = e.to_string();
            app.shell
                .fail(tf("import failed: {error}", &[("error", &error)]));
            return;
        }
    };
    let bundle = match import::read(&bytes) {
        Ok(bundle) => bundle,
        Err(e) => {
            app.shell
                .fail(tf("import failed: {error}", &[("error", &e)]));
            return;
        }
    };
    let _ = format;
    let mut placed = Placed::default();
    if let Some(s) = app.boxes.session.as_mut() {
        placed = import::land(&mut s.vault, bundle);
        s.selected = None;
        s.at.clear();
        if placed.total() > 0 {
            s.mark_now();
        }
    }
    if placed.total() > 0 {
        // The file is the box: an import that only lives in memory is one the user
        // finds gone the next time the box opens. Written now, or said out loud.
        app.boxes.persist(&mut app.shell);
        if app.boxes.session.as_ref().is_some_and(|s| s.dirty) {
            app.shell.set_status(
                tf(
                    "imported {total} into the box on screen, but its file is not written yet",
                    &[("total", &placed.total())],
                ),
                StatusKind::Error,
            );
            return;
        }
    }
    app.shell.set_status(summary(&placed), StatusKind::Saved);
}

/// What the status line says when the records are in.
pub fn summary(placed: &Placed) -> String {
    if placed.total() == 0 {
        return t("nothing new to import — everything in that file is already here").to_owned();
    }
    tf(
        "imported {total}: {logins} passwords, {notes} notes, {codes} codes",
        &[
            ("total", &placed.total()),
            ("logins", &placed.logins),
            ("notes", &placed.notes),
            ("codes", &placed.otps),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Screen;
    use crate::vault::model::Vault;

    fn app() -> App {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.screen = Screen::Main;
        app.open("import");
        app
    }

    #[test]
    fn a_reader_the_summary_names_every_shape_it_landed() {
        let placed = Placed {
            logins: 2,
            notes: 4,
            otps: 5,
        };
        assert_eq!(
            summary(&placed),
            "imported 11: 2 passwords, 4 notes, 5 codes"
        );
        assert_eq!(
            summary(&Placed::default()),
            "nothing new to import — everything in that file is already here"
        );
    }

    #[test]
    fn a_fixture_lands_through_the_screen_into_the_open_box() {
        let mut app = app();
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/import-bitwarden.json");
        land(&mut app, &path, Some(import::Format::Bitwarden));

        let s = app.boxes.session.as_ref().unwrap();
        let v: &Vault = &s.vault;
        assert_eq!(v.logins.len(), 1, "the login is in the box on screen");
        assert_eq!(v.otps.len(), 1);
        assert_eq!(v.drive.len(), 1, "and the note with it");
        assert!(
            !s.dirty,
            "a landed import is written before the status line says so"
        );
        assert!(s.selected.is_none(), "the box is not left on a record");
    }

    /// The file is the box: the moment the status line says "imported", the records
    /// are on disk, without waiting for a tick that may never come.
    #[test]
    fn an_import_reaches_the_file_before_it_is_reported() {
        use crate::vault::store;

        let mut app = app();
        let path = app.boxes.session.as_ref().unwrap().path.clone();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/import-bitwarden.json");

        land(&mut app, &fixture, Some(import::Format::Bitwarden));

        let creds = app.boxes.session.as_ref().unwrap().creds.clone();
        let on_disk = store::load(&path, &creds).unwrap();
        assert_eq!(on_disk.logins.len(), 1, "the login is in the file");
        assert_eq!(on_disk.otps.len(), 1);
        assert_eq!(on_disk.drive.len(), 1);
        assert!(
            !app.boxes.session.as_ref().unwrap().dirty,
            "and nothing is left waiting for an autosave"
        );
    }

    #[test]
    fn a_file_that_does_not_read_says_so_and_changes_nothing() {
        let mut app = app();
        let path = std::env::temp_dir().join("pandora-import-broken.json");
        std::fs::write(&path, b"{not json").unwrap();
        land(&mut app, &path, Some(import::Format::Bitwarden));
        let s = app.boxes.session.as_ref().unwrap();
        assert!(s.vault.logins.is_empty(), "nothing landed");
        assert!(!s.dirty, "and nothing was marked to save");
        let _ = std::fs::remove_file(&path);
    }
}
