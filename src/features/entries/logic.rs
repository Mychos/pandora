//! Creating, deleting and selecting the records of the box on screen. A record
//! belongs to the pane that owns it, so each kind is created there; what is blind
//! to the kind — deleting, selecting, the prompts — is here.

use crate::features::boxes::model::{Boxes, Tab};
use crate::features::entries::model::View;
use crate::features::shell::confirm::Confirm;
use crate::vault::model::{now_secs, Entry, Kind, Login, Note};

impl Boxes {
    /// A login, created in the box on screen and selected.
    pub fn add_login(&mut self) {
        if let Some(s) = &mut self.session {
            let it = Login::new(now_secs());
            let id = it.id.clone();
            s.vault.logins.push(it);
            s.selected = Some(id);
            s.mark_dirty();
        }
    }

    /// A note in the folder being browsed, created and selected. A file is not
    /// this: it is its own entry, and arrives through the picker.
    pub fn add_note(&mut self) {
        let folder = self.session.as_ref().and_then(|s| s.folder.clone());
        self.add_note_in(folder);
    }

    pub fn add_note_in(&mut self, folder: Option<String>) {
        if let Some(s) = &mut self.session {
            let mut it = Note::new(now_secs());
            it.folder = folder;
            let id = it.id.clone();
            s.vault.drive.push(Entry::Note(it));
            s.selected = Some(id.clone());
            s.editing_entry = Some(id);
            s.mark_dirty();
        }
    }

    /// Move a record to the trash, wherever it lives in the box. The one delete of
    /// the app: erasing for good is the trash's own button.
    pub fn trash_item(&mut self, id: &str) {
        let now = now_secs();
        if let Some(s) = &mut self.session {
            if s.vault.take(id, now).is_some() {
                s.selected = None;
                s.editing_entry = None;
                s.confirm = None;
                s.last_delete = Some(id.to_owned());
                s.mark_dirty();
            }
        }
    }

    /// Put the last delete back. True when there was one still in the trash: a
    /// second Ctrl+Z has nothing left to undo.
    pub fn undo_delete(&mut self) -> bool {
        let Some(id) = self.session.as_ref().and_then(|s| s.last_delete.clone()) else {
            return false;
        };
        self.restore_item(&id)
    }

    /// Put a trashed record back in its pane and show it. False when the id is
    /// not in the trash.
    pub fn restore_item(&mut self, id: &str) -> bool {
        let kind = self.session.as_mut().and_then(|s| s.vault.restore(id));
        let Some(kind) = kind else {
            return false;
        };
        if let Some(s) = self.session.as_mut() {
            s.last_delete = None;
            s.confirm = None;
            s.mark_dirty();
        }
        self.reveal(kind, id);
        true
    }

    /// Erase everything the trash holds. True when something was erased.
    pub fn empty_trash(&mut self) -> bool {
        let Some(s) = self.session.as_mut() else {
            return false;
        };
        if s.vault.empty_trash() == 0 {
            return false;
        }
        s.last_delete = None;
        s.confirm = None;
        s.mark_dirty();
        true
    }

    /// Show a record: the pane that owns it, the folder it lives in, and the row
    /// itself selected. The one way back to a record, shared by a restore and by
    /// the search palette. A folder is not a record and does not come through
    /// here.
    pub fn reveal(&mut self, kind: Kind, id: &str) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        // The entry's own folder before the session is touched, so the pane paints
        // it where it is instead of at the root.
        let folder = s
            .vault
            .any(id)
            .and_then(|it| it.folder().map(str::to_owned));
        s.tab = Tab::of(kind);
        s.selected = Some(id.to_owned());
        s.search.clear();
        s.editing_entry = None;
        s.editing_folder = None;
        s.confirm = None;
        if kind != Kind::Login {
            s.folder = folder;
        }
        // In the generated-folders view a row can sit under a folded node, and a
        // selection nobody can see reads as a lost one: put the pane inside the
        // node the row hangs from.
        if kind == Kind::Login && s.view == View::Folders {
            let axis = s.axis;
            let node = s
                .vault
                .logins
                .iter()
                .find(|it| it.id == id)
                .and_then(|login| {
                    crate::features::entries::tree::folders(std::slice::from_ref(&login), axis)
                        .into_iter()
                        .next()
                });
            if let Some(node) = node {
                s.at = node.path.clone();
                s.collapsed.remove(&node.key);
            }
        }
    }

    /// Ctrl+N: what the pane's own `+` button does.
    pub(crate) fn new_entry(&mut self) {
        if let Some(s) = self.session.as_mut() {
            s.show_new = true;
        }
    }

    /// The prompt the row's own "Delete" opens; context menus route their Delete here too.
    pub(crate) fn ask_delete(&mut self) {
        let Some(id) = self.selected() else {
            return;
        };
        let pending = if self.is_folder(&id) {
            Confirm::Folder(id)
        } else {
            Confirm::Entry(id)
        };
        if let Some(s) = self.session.as_mut() {
            s.confirm = Some(pending);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::features::boxes::model::Tab;
    use crate::vault::model::Entry;

    #[test]
    fn notes_land_in_the_browsing_folder_unless_told_otherwise() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("t");

        app.boxes.add_note();
        let last = |app: &App| match app.boxes.session.as_ref().unwrap().vault.drive.last() {
            Some(Entry::Note(it)) => it.folder.clone(),
            other => panic!("a new note is a note: {other:?}"),
        };
        assert_eq!(last(&app), None, "browsing the root");

        app.boxes.add_note_in(Some("deep".to_owned()));
        assert_eq!(last(&app).as_deref(), Some("deep"));

        app.boxes.add_login();
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(
            s.vault.logins.last().unwrap().name,
            "New password",
            "a login is created by its own pane"
        );
        assert!(
            s.vault
                .any(s.vault.logins.last().unwrap().id.as_str())
                .is_some(),
            "and it is a record of the box"
        );
    }

    /// A delete is a move: the record leaves the pane, waits in the trash, and
    /// one Ctrl+Z puts it back. Only the last delete is undoable; anything older
    /// waits in the trash.
    #[test]
    fn deleting_moves_a_record_to_the_trash_and_undo_puts_it_back() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("t");
        app.boxes.add_login();
        let id = app.boxes.selected().unwrap();

        app.boxes.trash_item(&id);
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.trash.len(), 1, "the login waits in the trash");
        assert!(s.vault.logins.is_empty(), "and left the pane");
        assert!(s.selected.is_none());
        assert_eq!(
            s.last_delete.as_deref(),
            Some(id.as_str()),
            "the undo remembers it"
        );

        assert!(app.boxes.undo_delete(), "one delete can be taken back");
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.logins.len(), 1, "and the login came home");
        assert!(s.vault.trash.is_empty());
        assert!(s.last_delete.is_none());
        assert!(!app.boxes.undo_delete(), "the second time there is nothing");
    }

    /// A restore shows the record on the pane that owns it, not wherever the user
    /// happened to be.
    #[test]
    fn a_deleted_code_is_restored_onto_its_own_pane() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("t");
        app.boxes.add_otp();
        let id = app.boxes.selected().unwrap();
        app.boxes.trash_item(&id);

        assert!(app.boxes.restore_item(&id));
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.tab, Tab::Codes, "a code belongs to the codes pane");
        assert_eq!(s.selected.as_deref(), Some(id.as_str()));
        assert!(!app.boxes.restore_item(&id), "once is enough");
    }

    #[test]
    fn emptying_the_trash_is_the_only_erase() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("t");
        app.boxes.add_login();
        let id = app.boxes.selected().unwrap();
        app.boxes.trash_item(&id);

        assert!(app.boxes.empty_trash(), "there was something to erase");
        let s = app.boxes.session.as_ref().unwrap();
        assert!(s.vault.trash.is_empty());
        assert!(!app.boxes.restore_item(&id), "and nothing to put back");
        assert!(!app.boxes.empty_trash(), "a second empty does nothing");
    }
}
