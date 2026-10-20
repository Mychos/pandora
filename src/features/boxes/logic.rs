use crate::app::{App, Screen, StatusKind, Tab};
use crate::features::boxes::model::{Boxes, Session};
use crate::features::shell::model::Shell;
use crate::shared::i18n::{t, tf};
use crate::vault::store;

const AUTOSAVE_DELAY: std::time::Duration = std::time::Duration::from_millis(900);

/// Which half of a credentials entry a copy takes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Secret {
    Username,
    Password,
}

impl Secret {
    fn label(self) -> &'static str {
        match self {
            Secret::Username => "username",
            Secret::Password => "password",
        }
    }
}

impl App {
    /// One entry's secret to the clipboard: the countdown is armed, and the
    /// status says which half went.
    pub fn copy_secret(&mut self, ctx: &egui::Context, id: &str, secret: Secret) {
        let text = {
            let Some(it) = self
                .boxes
                .session
                .as_ref()
                .and_then(|s| s.vault.logins.iter().find(|i| i.id == id))
            else {
                return;
            };
            match secret {
                Secret::Username => it.username.clone(),
                Secret::Password => it.password.to_string(),
            }
        };
        let label = t(secret.label());
        if text.is_empty() {
            self.shell.set_status(
                tf("no {label} to copy", &[("label", &label)]),
                StatusKind::Info,
            );
            return;
        }
        self.shell.copy_text(ctx, &text);
        self.shell.set_status(
            tf("{label} copied", &[("label", &label)]),
            StatusKind::Saved,
        );
    }

    /// Everything the keyboard can reach without the pointer; called once per frame,
    /// after the screens, so every widget has had its own say first.
    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        self.copy(ctx);
        self.keys(ctx);
    }

    /// Ctrl+C copies the selected entry's password and Ctrl+B its username.
    /// `egui-winit` turns Ctrl+C into `Event::Copy` and drops the `Key::C` event
    /// entirely, so both forms have to be watched.
    fn copy(&mut self, ctx: &egui::Context) {
        let (mut copy_password, copy_username) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            (
                (cmd && i.key_pressed(egui::Key::C))
                    || i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
                cmd && i.key_pressed(egui::Key::B),
            )
        });
        // A field with a live selection handles Ctrl+C itself; an empty one (as
        // in the masked password field) leaves the key to the entry below.
        if copy_password
            && ctx
                .memory(|m| m.focused())
                .and_then(|id| egui::widgets::text_edit::TextEditState::load(ctx, id))
                .and_then(|state| state.cursor.char_range())
                .is_some_and(|range| !range.is_empty())
        {
            copy_password = false;
        }
        if !copy_password && !copy_username {
            return;
        }
        let secret = if copy_password {
            Secret::Password
        } else {
            Secret::Username
        };
        let Some(id) = self.boxes.selected() else {
            return;
        };
        self.copy_secret(ctx, &id, secret);
    }

    /// The keyboard's way round the app. Escape that reaches this point means
    /// "drop the selection" (a modal eats its own), and the bare keys stay out of
    /// the way of a field being typed in.
    fn keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};

        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
            && self.screen == Screen::Main
        {
            if let Some(s) = self.boxes.session.as_mut() {
                s.selected = None;
                s.editing_entry = None;
            }
        }
        // The phone's back button and back gesture arrive as this key (Android
        // consumes `KEYCODE_BACK` natively), so it is the one place that handles
        // both; on a desktop a mouse's back button lands here too.
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::BrowserBack)) {
            self.back_or_leave(ctx);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
            self.boxes.persist(&mut self.shell);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::L)) {
            self.lock(ctx);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::W))
            && self.boxes.session.is_some()
        {
            self.close(self.boxes.active);
        }
        // Shift first: `consume_key` ignores an extra shift, so the plain
        // shortcut would eat Ctrl+Shift+Tab as well.
        let back = ctx.input_mut(|i| i.consume_key(shift_command(), Key::Tab));
        let forward = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Tab));
        if back || forward {
            self.rotate(forward);
        }
        for (key, tab) in [
            (Key::Num1, Tab::Passwords),
            (Key::Num2, Tab::Drive),
            (Key::Num5, Tab::Codes),
            (Key::Num4, Tab::Settings),
        ] {
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, key)) {
                self.boxes.pane(tab);
            }
        }
        // The tool is no pane of the box: Ctrl+3 opens it with or without one.
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Num3)) {
            self.screen = Screen::Generate;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
            // The only search on the sources screen is the recents'.
            if self.screen == Screen::Sources {
                self.auth.side = Some(crate::app::Side::Recents);
            }
            ctx.memory_mut(|m| m.request_focus(search_id(self.screen)));
        }
        // Ctrl+K searches the whole box, wherever the pane is looking; there is
        // nothing to search without one open.
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K))
            && self.screen == Screen::Main
            && self.boxes.session.is_some()
        {
            self.open_find();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::N)) {
            self.boxes.new_entry();
        }
        if !typing(ctx) {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete)) {
                self.boxes.ask_delete();
            }
            // Ctrl+Z is a text field's own undo while one has the keyboard, so the
            // record delete only takes the key when nothing is being typed in.
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z))
                && self.boxes.undo_delete()
            {
                self.shell
                    .set_status("deleted entry restored", StatusKind::Saved);
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F2)) {
                self.boxes.edit_folder();
            }
        }
    }

    /// Erase this box's file from disk for good. The tab closes, the device's
    /// seal is forgotten and the path leaves the recents; every edit that only
    /// lived in this file goes with it.
    pub fn destroy(&mut self) {
        let Some(path) = self.boxes.session.as_ref().map(|s| s.path.clone()) else {
            return;
        };
        // The recents go first: `close` lands on whatever screen the list left
        // calls for, and a box that no longer exists must not be offered.
        self.auth.recents.retain(|r| r.path != path);
        self.close(self.boxes.active);
        crate::features::auth::biometric::forget(&path);
        // The stash is a version of this box: erasing the box erases it too.
        store::forget_stash(&path);
        match std::fs::remove_file(&path) {
            Ok(()) => self.shell.set_status("box file deleted", StatusKind::Saved),
            Err(e) => self.shell.fail(tf(
                "could not delete the box file: {error}",
                &[("error", &e)],
            )),
        }
    }

    /// Replace the box on screen with the copy `store::save` keeps beside it: the
    /// version before the last write. Deliberate and verified — `store::adopt`
    /// opens the copy with the key this box already holds before it touches the
    /// file, and stashes the current one in the same slot, so another restore
    /// takes the restore back.
    pub fn restore_backup(&mut self) {
        let Some(s) = self.boxes.session.as_mut() else {
            return;
        };
        if s.conflict {
            self.shell
                .fail("settle the disk conflict before restoring a version");
            return;
        }
        // The prompt was accepted: it is gone whether or not the copy opens.
        s.confirm = None;
        // What is on screen goes to the file first, so the copy the restore pushes
        // aside holds it and nothing unsaved is what gets lost.
        write_if_dirty(s, &mut self.shell);
        let backup = s.path.with_extension(format!("{}.bak", store::EXT));
        let Ok(bytes) = std::fs::read(&backup) else {
            self.shell
                .fail("there is no previous version beside this box");
            return;
        };
        // A restore is the one copy that is *meant* to go back, so it names no
        // generation: the stash is older than the box by definition.
        match store::adopt(&s.path, &bytes, &s.creds, None) {
            Ok(vault) => {
                s.vault = vault;
                s.sanitize();
                s.dirty = false;
                s.last_edit = None;
                s.conflict = false;
                s.ignore_mtime = None;
                s.last_mtime = store::modified_time(&s.path);
                s.baseline = Some(crate::features::sync::github::Fingerprint::of(&s.vault));
                s.push_at = None;
                self.shell
                    .set_status("previous version restored", StatusKind::Saved);
            }
            Err(e) => self.shell.fail(tf(
                "the copy beside the box does not open with this box's key: {error}",
                &[("error", &e)],
            )),
        }
    }
}

impl Boxes {
    pub fn tick(&mut self, shell: &mut Shell) {
        /// How often the file is looked at. Autosave runs on its own 900 ms
        /// debounce; a changed file is noticed within a beat, not on every paint.
        const WATCH: std::time::Duration = std::time::Duration::from_millis(750);
        let due = self.watch.is_none_or(|watched| watched.elapsed() >= WATCH);
        if due {
            self.watch = Some(std::time::Instant::now());
        }
        let mut note: Option<(String, StatusKind)> = None;
        let mut save_now = false;
        {
            let Some(s) = self.session.as_mut() else {
                return;
            };

            // External change (e.g. GDrive synced a newer copy from another
            // device). A file that was merely touched is not a change; a file
            // that holds a different box is another version, and the user picks
            // which one wins — this never replaces the vault in silence.
            if let Some(m) = due.then(|| store::modified_time(&s.path)).flatten() {
                if !s.conflict && Some(m) != s.last_mtime && s.ignore_mtime != Some(m) {
                    if s.dirty {
                        s.conflict = true;
                    } else {
                        match store::load(&s.path, &s.creds) {
                            Ok(v) if v == s.vault => s.last_mtime = Some(m),
                            Ok(_) => {
                                s.conflict = true;
                                note = Some((
                                    "box changed on disk — another device synced?".to_owned(),
                                    StatusKind::Error,
                                ));
                            }
                            Err(e) => {
                                s.conflict = true;
                                note = Some((
                                    tf("disk box unreadable: {error}", &[("error", &e)]),
                                    StatusKind::Error,
                                ));
                            }
                        }
                    }
                }
            }

            if s.dirty && !s.conflict {
                if let Some(t) = s.last_edit {
                    if t.elapsed() >= AUTOSAVE_DELAY {
                        save_now = true;
                    }
                }
            }
        }
        if let Some((text, kind)) = note {
            shell.set_status(text, kind);
        }
        if save_now {
            self.persist(shell);
        }
    }

    pub fn persist(&mut self, shell: &mut Shell) {
        if let Some(s) = &mut self.session {
            write(s, shell, false);
        }
    }

    /// The last moment before its key is dropped (tab switch, tab close, lock):
    /// write the unsaved, conflict-free edits.
    pub fn flush(&mut self, shell: &mut Shell) {
        if let Some(s) = &mut self.session {
            write_if_dirty(s, shell);
        }
    }

    /// Every unlocked box, not just the one on screen: locking drops the parked
    /// tabs too, and their edits only ever lived in the vault.
    pub fn flush_all(&mut self, shell: &mut Shell) {
        if let Some(s) = &mut self.session {
            write_if_dirty(s, shell);
        }
        for s in &mut self.tabs {
            write_if_dirty(s, shell);
        }
    }

    pub fn reload(&mut self, shell: &mut Shell) {
        let Some(s) = &mut self.session else {
            return;
        };
        match store::load(&s.path, &s.creds) {
            Ok(v) => {
                s.vault = v;
                s.sanitize();
                s.dirty = false;
                s.last_edit = None;
                s.conflict = false;
                s.last_mtime = store::modified_time(&s.path);
                s.push_at = None;
                s.baseline = Some(crate::features::sync::github::Fingerprint::of(&s.vault));
                shell.set_status("box reloaded — local changes discarded", StatusKind::Info);
            }
            Err(e) => shell.fail(tf("reload failed: {error}", &[("error", &e)])),
        }
    }

    pub fn conflict_overwrite(&mut self, shell: &mut Shell) {
        if self.overwrite(shell) {
            shell.set_status("overwrote the on-disk version", StatusKind::Saved);
        }
    }

    /// Write the box over whatever the disk holds. Only for a write the user
    /// asked for on purpose and that was checked against the file just before —
    /// a password rotation, or the banner's own "overwrite with mine".
    pub fn overwrite(&mut self, shell: &mut Shell) -> bool {
        match &mut self.session {
            Some(s) => write(s, shell, true),
            None => false,
        }
    }
}

/// The one write of a session's vault, guarded. A pending conflict, or a file
/// that changed under us, is the user's to settle: taking the disk copy blind is
/// how the other device's edits disappear.
fn write(s: &mut Session, shell: &mut Shell, forced: bool) -> bool {
    if !forced {
        if s.conflict {
            return false;
        }
        if superseded(s) {
            s.conflict = true;
            shell.set_status(
                "box changed on disk — another device synced?",
                StatusKind::Error,
            );
            return false;
        }
    }
    match store::save(&s.path, &mut s.vault, &s.creds) {
        Ok(()) => {
            s.dirty = false;
            s.last_edit = None;
            s.last_mtime = store::modified_time(&s.path);
            s.ignore_mtime = None;
            s.conflict = false;
            true
        }
        Err(e) => {
            shell.fail(tf("save failed: {error}", &[("error", &e)]));
            false
        }
    }
}

fn write_if_dirty(s: &mut Session, shell: &mut Shell) {
    if s.dirty && !s.conflict {
        write(s, shell, false);
    }
}

/// Did something else write this box's file since the session last saw it? The
/// one change the user already answered for does not count.
fn superseded(s: &Session) -> bool {
    store::modified_time(&s.path)
        .is_some_and(|m| Some(m) != s.last_mtime && s.ignore_mtime != Some(m))
}

impl Boxes {
    pub fn pane(&mut self, tab: Tab) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        // Every frame asks for the pane already on screen, so this has to come
        // after the guard: a scan belongs to the pane that opened it, and closing
        // it here means leaving Codes, not sitting on it.
        if s.tab == tab {
            return;
        }
        #[cfg(target_os = "android")]
        if s.scan.take().is_some() {
            let _ = crate::shared::android::send_none("stopCamera");
        }
        s.tab = tab;
        s.selected = None;
        s.search.clear();
        s.confirm = None;
        s.show_trash = false;
        s.last_delete = None;
        s.editing_folder = None;
        s.new_folder = None;
        s.editing_entry = None;
    }

    pub fn conflict_keep_mine(&mut self) {
        let Some(s) = &mut self.session else {
            return;
        };
        s.ignore_mtime = store::modified_time(&s.path);
        s.conflict = false;
    }
}

/// Ctrl on Linux and Windows, Cmd on a Mac — plus Shift.
fn shift_command() -> egui::Modifiers {
    egui::Modifiers {
        command: true,
        shift: true,
        ..Default::default()
    }
}

/// The search field Ctrl+F hands the keyboard to; only one pane is drawn, so the
/// panes share one id.
pub fn search_id(screen: Screen) -> egui::Id {
    match screen {
        Screen::Main => egui::Id::new("pane-search"),
        _ => egui::Id::new("recents-search"),
    }
}

/// Is a text field taking the keyboard right now? A bare key must never do
/// something else at the same time as typing.
fn typing(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.focused())
        .and_then(|id| egui::widgets::text_edit::TextEditState::load(ctx, id))
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::shell::confirm::Confirm;
    use crate::vault::model::{Login, Vault};

    fn unique() -> String {
        format!("pandora-safety-{}", uuid::Uuid::new_v4())
    }

    /// A box on screen whose file is on disk and matches it: the state every
    /// write starts from.
    fn opened(name: &str) -> App {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open(name);
        let m = {
            let s = app.boxes.session.as_mut().unwrap();
            store::save(&s.path, &mut s.vault, &s.creds).unwrap();
            store::modified_time(&s.path)
        };
        app.boxes.session.as_mut().unwrap().last_mtime = m;
        app
    }

    /// A box with one login on screen and the empty box it replaced in the
    /// `.bak`: exactly what a restore has to find.
    fn with_history(name: &str) -> (App, std::path::PathBuf) {
        let mut app = opened(name);
        let path = app.boxes.session.as_ref().unwrap().path.clone();
        {
            let s = app.boxes.session.as_mut().unwrap();
            s.vault.logins.push(Login::new(1));
        }
        let s = app.boxes.session.as_mut().unwrap();
        store::save(&path, &mut s.vault, &s.creds).unwrap();
        app.boxes.session.as_mut().unwrap().last_mtime = store::modified_time(&path);
        (app, path)
    }

    /// Another device's box over the same file, a moment later, so the stamp the
    /// session holds is not the one on disk any more.
    fn another_device(app: &mut App) {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut other = Vault::new("Other");
        other.logins.push(Login::new(1));
        let s = app.boxes.session.as_ref().unwrap();
        store::save(&s.path, &mut other, &s.creds).unwrap();
    }

    #[test]
    fn a_save_never_takes_a_file_another_writer_moved() {
        let mut app = opened(&unique());
        another_device(&mut app);
        let path = app.boxes.session.as_ref().unwrap().path.clone();

        app.boxes.session.as_mut().unwrap().dirty = true;
        app.boxes.persist(&mut app.shell);

        let s = app.boxes.session.as_ref().unwrap();
        assert!(s.dirty, "the edit is still unsaved");
        assert!(s.conflict, "and the box asks instead of overwriting");
        assert!(
            path.with_extension(format!("{}.bak", store::EXT)).exists(),
            "the version about to be lost is beside the file"
        );
        assert_eq!(
            store::load(&path, &s.creds).unwrap().name,
            "Other",
            "the other device's box is intact"
        );
    }

    #[test]
    fn a_touch_is_not_a_change_but_a_different_box_is() {
        let name = unique();
        let mut app = opened(&name);
        let path = app.boxes.session.as_ref().unwrap().path.clone();

        // The same box, written again: a new stamp, nothing to ask about.
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let s = app.boxes.session.as_mut().unwrap();
            store::save(&path, &mut s.vault, &s.creds).unwrap();
        }
        app.boxes.tick(&mut app.shell);
        {
            let s = app.boxes.session.as_ref().unwrap();
            assert!(!s.conflict, "the same box is not a conflict");
            assert_eq!(s.last_mtime, store::modified_time(&path));
        }

        another_device(&mut app);
        // `tick` is paced: force the next stat so the test does not sleep out
        // the watch window.
        app.boxes.watch = None;
        app.boxes.tick(&mut app.shell);
        let s = app.boxes.session.as_ref().unwrap();
        assert!(s.conflict, "another version asks the user");
        assert_eq!(s.vault.name, name, "the local box is still here to choose");
    }

    /// A box that leaves the screen only lives in its vault: whatever it holds
    /// goes to disk as it parks, and again when the lock drops its key.
    #[test]
    fn a_box_that_leaves_the_screen_writes_its_edits_first() {
        let ctx = egui::Context::default();
        let mut app = opened(&unique());
        let path = app.boxes.session.as_ref().unwrap().path.clone();
        let before = std::fs::read(&path).unwrap();

        {
            let s = app.boxes.session.as_mut().unwrap();
            s.vault.name = "Edited".to_owned();
            s.dirty = true;
        }
        app.open(&unique()); // the edit is written before the box is parked
        let parked = std::fs::read(&path).unwrap();
        assert_ne!(parked, before, "the parked box's edit reached the disk");

        {
            let s = app.boxes.tabs.last_mut().unwrap();
            s.vault.name = "Edited again".to_owned();
            s.dirty = true;
        }
        app.lock(&ctx);
        assert_ne!(
            std::fs::read(&path).unwrap(),
            parked,
            "the lock wrote the parked edit before dropping the key"
        );
    }

    /// The copy beside a box is the version before the last write, and restoring
    /// it is itself reversible: the version it replaces becomes the copy.
    #[test]
    fn restoring_the_backup_puts_the_previous_box_on_screen() {
        let (mut app, path) = with_history(&unique());
        let backup = path.with_extension(format!("{}.bak", store::EXT));
        assert!(backup.exists(), "a save kept the version it replaced");
        assert_eq!(
            app.boxes.session.as_ref().unwrap().vault.logins.len(),
            1,
            "the box on screen is the newer one"
        );

        app.restore_backup();

        let s = app.boxes.session.as_ref().unwrap();
        assert!(
            s.vault.logins.is_empty(),
            "the previous version is on screen"
        );
        assert!(!s.dirty, "and it is what the file holds");
        assert!(store::load(&path, &s.creds).unwrap().logins.is_empty());
        assert_eq!(
            store::load(&backup, &s.creds).unwrap().logins.len(),
            1,
            "what was on screen went into the copy, so it can be taken back"
        );
    }

    /// A copy that is not this box — another vault, another password, a corrupt
    /// file — never replaces the one that works.
    #[test]
    fn a_backup_that_does_not_open_is_refused_and_touches_nothing() {
        let (mut app, path) = with_history(&unique());
        let before = std::fs::read(&path).unwrap();
        let backup = path.with_extension(format!("{}.bak", store::EXT));
        std::fs::write(&backup, b"not a box").unwrap();
        app.boxes.session.as_mut().unwrap().confirm = Some(Confirm::RestoreVersion);

        app.restore_backup();

        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "the box is untouched"
        );
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.logins.len(), 1, "and so is the screen");
        assert!(s.confirm.is_none(), "the prompt went with the answer");
        assert!(
            app.shell
                .status
                .as_ref()
                .is_some_and(|said| said.kind == StatusKind::Error),
            "and the refusal is what the status says"
        );
    }

    /// With no copy beside it there is nothing to restore, and the box is left
    /// alone instead of asking about a version that does not exist.
    #[test]
    fn a_restore_without_a_copy_touches_nothing() {
        let mut app = opened(&unique());
        let path = app.boxes.session.as_ref().unwrap().path.clone();
        let before = std::fs::read(&path).unwrap();
        assert!(!path.with_extension(format!("{}.bak", store::EXT)).exists());

        app.restore_backup();

        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(app
            .shell
            .status
            .as_ref()
            .is_some_and(|said| said.kind == StatusKind::Error));
    }
}
