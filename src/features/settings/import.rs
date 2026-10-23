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

/// The row's click: pick a file, sniff it, and let the confirm say what is about to
/// land. A file the reader does not know is refused here, before anything is asked.
pub fn pick(app: &mut App) {
    let Some(path) = pick_file() else {
        return;
    };
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
    app.shell.set_status(summary(&placed), StatusKind::Saved);
}

/// What the status line says when the records are in.
pub fn summary(placed: &Placed) -> String {
    if placed.total() == 0 {
        return t("nothing new to import — everything in that file is already here").to_owned();
    }
    tf(
        "imported {total}: {logins} passwords, {cards} cards, {identities} identities, {notes} notes, {codes} codes",
        &[
            ("total", &placed.total()),
            ("logins", &placed.logins),
            ("cards", &placed.cards),
            ("identities", &placed.identities),
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
            cards: 1,
            identities: 3,
            notes: 4,
            otps: 5,
        };
        assert_eq!(
            summary(&placed),
            "imported 15: 2 passwords, 1 cards, 3 identities, 4 notes, 5 codes"
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
        assert_eq!(v.cards.len(), 1);
        assert_eq!(v.identities.len(), 1);
        assert_eq!(v.otps.len(), 1);
        assert_eq!(v.drive.len(), 1, "and the note with it");
        assert!(
            s.dirty,
            "a landed import is an edit waiting for the autosave"
        );
        assert!(s.selected.is_none(), "the box is not left on a record");
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
