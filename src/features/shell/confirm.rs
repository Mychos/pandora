//! One confirmation modal for every delete in the app.

use crate::app::App;
use crate::shared::i18n::{t, tf};
use crate::shared::theme;
use crate::shared::ui::balance;
use egui::RichText;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Confirm {
    Entry(String),
    Folder(String),
    /// Not a delete: the one prompt that says what leaves the box in plaintext.
    /// `neutral` is the name it goes out under — the box's own, or a blank one.
    Export {
        neutral: bool,
    },
    /// Not a delete: a document leaves the box as a plain file of its own, and
    /// this is where the user is told so — and asked which name it takes.
    File {
        id: String,
        neutral: bool,
    },
    /// The box file itself, erased from disk.
    Box,
    /// Not a delete: the disk copy replaces this box, and anything unsaved in it.
    Reload,
    /// Everything the trash holds, erased for good. The only permanent delete.
    EmptyTrash,
    /// Not a delete: the copy beside the box replaces it, and what is on screen
    /// goes into that same copy.
    RestoreVersion,
}

impl Confirm {
    /// Whether this prompt is about a file leaving the box in the clear, which is
    /// the one question the modal puts to the user besides "are you sure".
    pub fn neutral(&self) -> Option<bool> {
        match self {
            Confirm::Export { neutral } | Confirm::File { neutral, .. } => Some(*neutral),
            _ => None,
        }
    }

    fn with_neutral(self, blank: bool) -> Self {
        match self {
            Confirm::Export { .. } => Confirm::Export { neutral: blank },
            Confirm::File { id, .. } => Confirm::File { id, neutral: blank },
            other => other,
        }
    }
}

pub fn modal(ctx: &egui::Context, app: &mut App) {
    let Some(pending) = app.boxes.session.as_ref().and_then(|s| s.confirm.clone()) else {
        return;
    };

    // The prompt's own answers live in the request itself, so choosing a name and
    // pressing the button can never disagree about which file it means.
    let mut asked = pending.clone();
    let (title, _) = describe(app, &asked);
    let mut accept = false;
    let open = theme::dialog(ctx, egui::Id::new("confirm-delete"), |ui| {
        theme::width(ui, 320.0);
        let mut closed = false;
        ui.horizontal(|ui| {
            balance::label(
                ui,
                RichText::new(title).strong().color(theme::palette().text),
            );
            closed = theme::close(ui, &app.icons);
        });
        // What leaves the box is the one thing worth asking about twice: it goes
        // out in the clear, under a name that may say what it is.
        if let Some(neutral) = asked.neutral() {
            ui.add_space(10.0);
            let mut blank = neutral;
            if theme::check(ui, &mut blank, "Use a neutral file name").changed() {
                asked = asked.clone().with_neutral(blank);
            }
        }
        ui.add_space(6.0);
        balance::label(
            ui,
            RichText::new(describe(app, &asked).1).color(theme::palette().muted),
        );
        ui.add_space(16.0);
        let (icon, color, label) = match asked {
            Confirm::Export { .. } => (theme::Icon::Download, theme::ACCENT, "Export"),
            Confirm::File { .. } => (theme::Icon::Download, theme::ACCENT, "Save to disk"),
            Confirm::Reload => (theme::Icon::RefreshCw, theme::ACCENT, "Reload"),
            Confirm::RestoreVersion => (theme::Icon::RefreshCw, theme::ACCENT, "Restore"),
            Confirm::EmptyTrash => (
                theme::Icon::Trash,
                theme::palette().error,
                "Empty the trash",
            ),
            _ => (theme::Icon::Trash, theme::palette().error, "Delete"),
        };
        let (accepted, refused) = theme::actions(
            ui,
            ctx,
            |ui| app.icons.labeled(ui, icon, 15.0, color, label),
            |ui| ui.button(crate::shared::i18n::t("Cancel")),
        );
        accept = accepted;
        closed || refused
    });

    if accept {
        apply(app, &asked);
    } else if !open {
        if let Some(s) = app.boxes.session.as_mut() {
            s.confirm = None;
        }
    }
}

fn describe(app: &App, c: &Confirm) -> (String, String) {
    let s = app.boxes.session.as_ref();
    match c {
        Confirm::Entry(id) => {
            let name = s
                .and_then(|s| s.vault.any(id))
                .map(|it| it.name().to_owned())
                .unwrap_or_else(|| "this".to_owned());
            (
                t("Delete?").to_owned(),
                tf(
                    "“{name}” moves to the trash. Empty the trash to erase it for good.",
                    &[("name", &name)],
                ),
            )
        }
        Confirm::Folder(id) => {
            let name = s
                .and_then(|s| s.vault.folders.iter().find(|f| &f.id == id))
                .map(|f| f.name.clone())
                .unwrap_or_else(|| "this folder".to_owned());
            (
                t("Delete folder?").to_owned(),
                tf(
                    "“{name}” will be deleted. Its contents move up one level.",
                    &[("name", &name)],
                ),
            )
        }
        Confirm::Export { neutral } => {
            let name = s
                .map(|s| s.vault.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| t("this box").to_owned());
            let file = crate::shared::utils::csv::export_name(&name, *neutral);
            (
                t("Export in plaintext?").to_owned(),
                tf(
                    "Every password in “{name}” is written to “{file}” in your Downloads folder, unencrypted. Delete it once you are done.",
                    &[("name", &name), ("file", &file)],
                ),
            )
        }
        Confirm::File { id, neutral } => {
            let file = s
                .and_then(|s| s.vault.drive.iter().find(|i| i.id() == id))
                .and_then(|i| match i {
                    crate::vault::model::Entry::File(f) => {
                        Some(crate::features::drive::logic::export_name(f, *neutral))
                    }
                    _ => None,
                });
            let name = s
                .and_then(|s| s.vault.any(id))
                .map(|it| it.name().to_owned())
                .unwrap_or_else(|| t("this file").to_owned());
            let file = file.unwrap_or_else(|| t("this document").to_owned());
            (
                t("Save it to disk?").to_owned(),
                tf(
                    "“{name}” leaves the box as “{file}” in your Downloads folder, and nothing encrypts it there.",
                    &[("name", &name), ("file", &file)],
                ),
            )
        }
        Confirm::Box => {
            let file = s
                .map(|s| s.path.display().to_string())
                .unwrap_or_else(|| "this box file".to_owned());
            (
                t("Delete box file?").to_owned(),
                tf(
                    "“{file}” is erased from disk for good, with everything in it. A copy on another device or in a repo is not touched.",
                    &[("file", &file)],
                ),
            )
        }
        Confirm::Reload => (
            t("Reload from disk?").to_owned(),
            t("Everything this box has that is not saved is dropped, and the file on disk is opened instead.")
                .to_owned(),
        ),
        Confirm::EmptyTrash => {
            let count = s.map_or(0, |s| s.vault.trash.len());
            (
                t("Empty the trash?").to_owned(),
                tf(
                    "{count} deleted records are erased for good. A file's bytes leave the box with it.",
                    &[("count", &count)],
                ),
            )
        }
        Confirm::RestoreVersion => {
            let file = s
                .map(|s| s.path.display().to_string())
                .unwrap_or_else(|| "this box file".to_owned());
            (
                t("Restore the previous version?").to_owned(),
                tf(
                    "The copy beside “{file}” replaces this box. What is on screen now is kept in that same copy, so another restore can take it back.",
                    &[("file", &file)],
                ),
            )
        }
    }
}

fn apply(app: &mut App, c: &Confirm) {
    match c {
        Confirm::Entry(id) => app.boxes.trash_item(id),
        Confirm::Folder(id) => app.boxes.delete_folder(id),
        Confirm::Export { neutral } => {
            if let Some(s) = app.boxes.session.as_mut() {
                s.confirm = None;
            }
            app.export_csv(*neutral);
        }
        Confirm::File { id, neutral } => {
            if let Some(s) = app.boxes.session.as_mut() {
                s.confirm = None;
            }
            app.boxes.export_file(id, *neutral, &mut app.shell);
        }
        Confirm::Box => app.destroy(),
        Confirm::Reload => {
            if let Some(s) = app.boxes.session.as_mut() {
                s.confirm = None;
            }
            app.boxes.reload(&mut app.shell);
        }
        Confirm::EmptyTrash => {
            app.boxes.empty_trash();
        }
        Confirm::RestoreVersion => {
            app.restore_backup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::auth::model::Recent;

    fn open(ctx: &egui::Context) -> App {
        let mut app = App::blank(ctx);
        app.open("Test");
        app
    }

    #[test]
    fn accepting_an_entry_delete_runs_it_and_drops_the_prompt() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = open(&ctx);
        app.boxes.add_login();
        let id = app
            .boxes
            .session
            .as_ref()
            .unwrap()
            .selected
            .clone()
            .unwrap();
        app.boxes.session.as_mut().unwrap().confirm = Some(Confirm::Entry(id.clone()));

        apply(&mut app, &Confirm::Entry(id));
        let s = app.boxes.session.as_ref().unwrap();
        assert!(s.vault.records().next().is_none(), "the entry left the box");
        assert_eq!(s.vault.trash.len(), 1, "and waits in the trash");
        assert!(s.confirm.is_none(), "the prompt is dropped");
    }

    /// The one prompt that is not a delete, and the one place the app says out
    /// loud that a CSV leaves in plaintext — and under which name.
    #[test]
    fn the_export_prompt_says_plaintext_and_names_the_box() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let app = open(&ctx);

        let (title, body) = describe(&app, &Confirm::Export { neutral: false });
        assert!(title.contains("plaintext"), "{title}");
        assert!(
            body.contains("Test") && body.contains("Test.csv") && body.contains("Downloads"),
            "it says what leaves and as what: {body}"
        );

        // The other answer: a file whose name says nothing about the box.
        let (_, blank) = describe(&app, &Confirm::Export { neutral: true });
        assert!(
            blank.contains("“pandora-export.csv”") && !blank.contains("“Test.csv”"),
            "the blank name is the one it lands under: {blank}"
        );
    }

    /// A document leaving the box is never silent either: the prompt names it,
    /// names the file it becomes, and says nothing encrypts it there.
    #[test]
    fn a_document_leaving_the_box_asks_first() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = open(&ctx);
        let s = app.boxes.session.as_mut().unwrap();
        let file = crate::vault::model::File::new("Divorcio.pdf".to_owned(), vec![1, 2, 3], 1);
        let id = file.id.clone();
        s.vault.drive.push(crate::vault::model::Entry::File(file));

        let (title, body) = describe(
            &app,
            &Confirm::File {
                id: id.clone(),
                neutral: false,
            },
        );
        assert!(title.contains("disk"), "{title}");
        assert!(
            body.contains("Divorcio.pdf") && body.contains("Downloads"),
            "{body}"
        );

        // What the file is called is the fragment after "as": the dialog may
        // name the document, but the name it lands under is the one that matters.
        let landed = |body: &str| -> String {
            body.split(" as “")
                .nth(1)
                .and_then(|rest| rest.split('”').next())
                .unwrap_or_default()
                .to_owned()
        };
        assert_eq!(landed(&body), "Divorcio.pdf");

        let (_, blank) = describe(&app, &Confirm::File { id, neutral: true });
        let dest = landed(&blank);
        assert!(
            dest.starts_with("pandora-") && dest.ends_with(".pdf"),
            "the blank name keeps the extension and nothing else: {dest}"
        );
        assert!(!dest.contains("Divorcio"), "{dest}");
    }

    #[test]
    fn deleting_the_box_file_erases_it_and_closes_the_tab() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = open(&ctx);
        let path = app.boxes.session.as_ref().unwrap().path.clone();
        std::fs::write(&path, b"box").unwrap();
        app.auth.recents = vec![Recent { path: path.clone() }];

        let (title, body) = describe(&app, &Confirm::Box);
        assert_eq!(title, "Delete box file?");
        assert!(body.contains("Test.vault"), "{body}");

        apply(&mut app, &Confirm::Box);
        assert!(!path.exists(), "the box file is gone");
        assert!(app.boxes.session.is_none(), "the tab is closed");
        assert!(app.auth.recents.is_empty(), "the recent is dropped");
    }
}
