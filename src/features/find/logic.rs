//! The palette's behaviour: what the query matches, how the keyboard moves, and
//! the one place a hit becomes a pane and a selection.

use crate::app::{App, Tab};
use crate::features::find::model::{Find, Row};
use crate::vault::model::Kind;
use crate::vault::search;

/// How many rows the palette shows. The box is in memory, so the rest are only
/// dropped, never lost.
pub const SHOWN: usize = 20;

/// The id the palette's field carries, so opening it can hand it the keyboard.
pub fn find_id() -> egui::Id {
    egui::Id::new("find-field")
}

impl App {
    /// Open the palette empty. The field takes the keyboard in its own frame —
    /// see `Find::needs_focus`.
    pub fn open_find(&mut self) {
        self.find = Find {
            open: true,
            query: String::new(),
            at: 0,
            needs_focus: true,
            ..Find::default()
        };
    }

    /// What the query matches right now, owned so the caller may change the box
    /// while it holds the rows. A frame that changes neither the query nor the
    /// box's generation reuses the last list instead of searching again.
    pub fn find_rows(&mut self) -> Vec<Row> {
        let Some(s) = self.boxes.session.as_ref() else {
            return Vec::new();
        };
        let key = (self.find.query.clone(), s.vault.seq);
        if self.find.searched.as_ref() != Some(&key) {
            let rows: Vec<Row> = search::find(&s.vault, &self.find.query, &mut self.find.notes)
                .into_iter()
                .take(SHOWN)
                .map(|hit| Row {
                    id: hit.id().to_owned(),
                    name: hit.name().to_owned(),
                    kind: hit.kind(),
                    detail: hit.detail.into_owned(),
                })
                .collect();
            self.find.rows = rows;
            self.find.searched = Some(key);
        }
        self.find.rows.clone()
    }

    /// Move the highlighted row, wrapping round the ends.
    pub fn find_move(&mut self, down: bool, len: usize) {
        if len == 0 {
            self.find.at = 0;
        } else if down {
            self.find.at = (self.find.at + 1) % len;
        } else {
            self.find.at = (self.find.at + len - 1) % len;
        }
    }

    /// Take the highlighted row and close: the choice is shown on its own pane.
    pub fn find_choose(&mut self, rows: &[Row]) {
        let Some(row) = rows.get(self.find.at) else {
            return;
        };
        let choice = match row.kind {
            Some(kind) => Choice::Record(kind, row.id.clone()),
            None => Choice::Folder(row.id.clone()),
        };
        self.find = Find::default();
        match choice {
            Choice::Record(kind, id) => self.boxes.reveal(kind, &id),
            Choice::Folder(id) => self.show_folder(id),
        }
    }

    /// Put the drive on a folder and select it. A folder is not a record, so it
    /// does not come through `reveal`.
    fn show_folder(&mut self, id: String) {
        let Some(s) = self.boxes.session.as_mut() else {
            return;
        };
        s.tab = Tab::Drive;
        s.folder = Some(id.clone());
        s.selected = Some(id);
        s.search.clear();
    }
}

/// The palette's choice, owned before the box is touched.
enum Choice {
    Record(Kind, String),
    Folder(String),
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::features::boxes::model::Tab;
    use crate::features::entries::model::{Axis, View};
    use crate::features::entries::tree;
    use crate::vault::model::{Entry, Folder, Kind, Login, Note, Otp};
    use zeroize::Zeroizing;

    fn app() -> App {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("Box");
        {
            let s = app.boxes.session.as_mut().unwrap();
            let mut login = Login::new(1);
            login.id = "login".to_owned();
            login.name = "GitHub".to_owned();
            login.username = "octocat".to_owned();
            login.url = "https://github.com".to_owned();
            s.vault.logins.push(login);
            let mut note = Note::new(1);
            note.id = "note".to_owned();
            note.name = "GitHub recovery".to_owned();
            s.vault.drive.push(Entry::Note(note));
            let mut otp = Otp::new(1);
            otp.id = "otp".to_owned();
            otp.name = "GitHub 2FA".to_owned();
            s.vault.otps.push(otp);
            let mut folder = Folder::new("GitHub docs", None, 1);
            folder.id = "folder".to_owned();
            s.vault.folders.push(folder);
        }
        app
    }

    /// The query finds each kind and a folder, and the row says which one it is.
    #[test]
    fn the_query_reaches_every_kind_and_a_folder() {
        let mut app = app();
        app.open_find();
        app.find.query = "github".to_owned();
        let rows = app.find_rows();
        assert!(rows.iter().any(|r| r.kind == Some(Kind::Login)));
        assert!(rows.iter().any(|r| r.kind == Some(Kind::Note)));
        assert!(rows.iter().any(|r| r.kind == Some(Kind::Otp)));
        assert!(rows.iter().any(|r| r.kind.is_none()), "a folder is a hit");
    }

    /// Choosing a row shows the record on its own pane, not where the user was.
    #[test]
    fn a_choice_lands_on_the_pane_that_owns_the_record() {
        let mut app = app();
        app.open_find();
        app.find.query = "GitHub 2FA".to_owned();
        let rows = app.find_rows();
        assert_eq!(rows.len(), 1);
        app.find.at = 0;
        app.find_choose(&rows);

        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.tab, Tab::Codes, "a code belongs to the codes pane");
        assert_eq!(s.selected.as_deref(), Some("otp"));
        assert!(!app.find.open, "and the palette closed");
    }

    /// In the generated-folders view a row can sit under a folded node; the jump
    /// puts the pane inside it so the selection is actually painted.
    #[test]
    fn a_login_jump_unfolds_the_generated_folder_it_lives_in() {
        let mut app = app();
        app.open_find();
        {
            let s = app.boxes.session.as_mut().unwrap();
            s.view = View::Folders;
            s.axis = Axis::Target;
            s.at = Vec::new();
            s.collapsed
                .insert(tree::node_key(Axis::Target, &["github.com".into()]));
        }
        app.find.query = "octocat".to_owned();
        let rows = app.find_rows();
        app.find.at = 0;
        app.find_choose(&rows);

        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.tab, Tab::Passwords);
        assert_eq!(s.selected.as_deref(), Some("login"));
        assert_eq!(s.at, ["github.com"], "the jump entered the row's folder");
        assert!(
            !s.collapsed
                .contains(&tree::node_key(Axis::Target, &["github.com".into()])),
            "and unfolded it"
        );
    }

    /// A folder is not a record: its row lands the drive on it instead.
    #[test]
    fn a_folder_choice_opens_the_drive_on_it() {
        let mut app = app();
        app.boxes.session.as_mut().unwrap().tab = Tab::Passwords;
        app.open_find();
        app.find.query = "GitHub docs".to_owned();
        let rows = app.find_rows();
        app.find.at = 0;
        app.find_choose(&rows);

        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.tab, Tab::Drive);
        assert_eq!(s.folder.as_deref(), Some("folder"));
    }

    /// The palette reads the same rules as the finder: a password and a seed are
    /// searchable by nothing.
    #[test]
    fn a_password_and_a_seed_never_reach_the_rows() {
        let mut app = app();
        {
            let s = app.boxes.session.as_mut().unwrap();
            s.vault.logins[0].password = Zeroizing::new("unique-pass".to_owned());
            s.vault.otps[0].secret = Zeroizing::new("UNIQUESEED".to_owned());
        }
        app.open_find();
        app.find.query = "unique".to_owned();
        assert!(app.find_rows().is_empty(), "the login is not a hit");
        app.find.query = "UNIQUESEED".to_owned();
        assert!(app.find_rows().is_empty(), "the seed is not a hit");
    }

    /// The same query is not searched again, and a new one is.
    #[test]
    fn a_row_list_is_reused_until_the_query_changes() {
        let mut app = app();
        app.open_find();
        app.find.query = "github".to_owned();
        let first = app.find_rows();
        assert!(!first.is_empty());

        app.boxes.session.as_mut().unwrap().vault.logins[0].name = "Renamed".to_owned();
        assert_eq!(
            app.find_rows().len(),
            first.len(),
            "an unchanged query reuses the rows it already has"
        );

        app.find.query = "renamed".to_owned();
        assert_eq!(app.find_rows().len(), 1, "a new query searches again");
    }
}
