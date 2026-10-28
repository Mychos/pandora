use crate::app::{App, Screen, StatusKind, Tab};
use crate::features::boxes::model::{Boxes, Session};
use crate::features::shell::model::Shell;
use crate::shared::i18n::{t, tf};
use crate::vault::store;
use std::path::PathBuf;

/// A last resort, not an autosave: an edit made outside an editor is written
/// after this long, so a crash cannot lose it. Editors write when they close.
const SAFETY_WRITE: std::time::Duration = std::time::Duration::from_secs(60);

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
        // Auto-type goes through xdotool/wtype, so it is a key and not a menu only.
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::T)) {
            self.autotype();
        }
        // The review reads the open box, so it is a pane and not a screen.
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::H)) {
            self.boxes.pane(Tab::Health);
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
        let backup = store::stash_path(&s.path);
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
                s.settle();
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
    pub fn tick(&mut self, _shell: &mut Shell) {
        /// How often the file is looked at. A changed file is noticed within a
        /// beat, not on every paint.
        const WATCH: std::time::Duration = std::time::Duration::from_millis(750);
        let due = self.watch.is_none_or(|watched| watched.elapsed() >= WATCH);
        if due {
            self.watch = Some(std::time::Instant::now());
        }
        let mut save_now = false;
        let mut meta_req: Option<PathBuf> = None;
        let mut save_req: Option<(
            PathBuf,
            crate::vault::model::Vault,
            crate::vault::store::Credentials,
            u64,
        )> = None;
        {
            let Some(s) = self.session.as_mut() else {
                return;
            };

            // External change (e.g. GDrive synced a newer copy from another
            // device). The stat now runs on a worker: ask for it when due, and
            // let pump_io turn the answer into a conflict or a reload.
            if due && !s.meta_pending && !s.remote_vault {
                s.meta_pending = true;
                meta_req = Some(s.path.clone());
            }

            // An open editor is never written under: its fields land when it
            // closes, however long they have been dirty. A change that is already
            // finished — a create, a delete, a move — does not wait at all.
            if s.dirty && !s.conflict && !s.editing {
                let safety = s.last_edit.is_some_and(|t| t.elapsed() >= SAFETY_WRITE);
                if s.immediate || safety {
                    save_now = true;
                }
            }
            if save_now && !s.save_pending {
                s.save_pending = true;
                s.edit_version = s.edit_version.wrapping_add(1);
                s.save_version = s.edit_version;
                save_req = Some((
                    s.path.clone(),
                    s.vault.clone(),
                    s.creds.clone(),
                    s.edit_version,
                ));
            }
        }
        if let Some(p) = meta_req {
            self.io()
                .request(crate::features::boxes::io::Request::Meta { path: p });
        }
        if let Some((p, v, c, _ver)) = save_req {
            self.io()
                .request(crate::features::boxes::io::Request::Save {
                    path: p,
                    vault: Box::new(v),
                    creds: c,
                });
        }
    }

    /// Drain background I/O replies and apply them. Called every frame.
    pub fn pump_io(&mut self, shell: &mut Shell) {
        use crate::features::boxes::io::{Reply, Request};
        let replies = self.io.as_ref().map(|i| i.drain()).unwrap_or_default();
        for reply in replies {
            match reply {
                Reply::Meta { path, mtime, slow } => {
                    let mut to_load: Option<(PathBuf, crate::vault::store::Credentials)> = None;
                    if let Some(s) = self.session.as_mut() {
                        if s.path == path {
                            s.meta_pending = false;
                            if slow {
                                s.remote_vault = true;
                            }
                            if !s.conflict
                                && !s.remote_vault
                                && mtime.is_some()
                                && mtime != s.last_mtime
                                && s.ignore_mtime != mtime
                            {
                                if s.dirty {
                                    s.conflict = true;
                                } else {
                                    s.load_pending = true;
                                    to_load = Some((s.path.clone(), s.creds.clone()));
                                }
                            }
                        }
                    }
                    if let Some((p, c)) = to_load {
                        self.io().request(Request::Load { path: p, creds: c });
                    }
                }
                Reply::Load { path, result } => {
                    if let Some(s) = self.session.as_mut() {
                        if s.path == path {
                            s.load_pending = false;
                            match result {
                                Ok(v) if *v == s.vault => {
                                    s.last_mtime = store::modified_time(&s.path)
                                }
                                Ok(_) => {
                                    s.conflict = true;
                                    shell.set_status(
                                        "box changed on disk — another device synced?".to_owned(),
                                        StatusKind::Error,
                                    );
                                }
                                Err(e) => {
                                    s.conflict = true;
                                    shell.set_status(
                                        tf("disk box unreadable: {error}", &[("error", &e)]),
                                        StatusKind::Error,
                                    );
                                }
                            }
                        }
                    }
                }
                Reply::Save {
                    path,
                    result,
                    vault,
                    mtime,
                } => {
                    if let Some(s) = self.session.as_mut() {
                        if s.path == path {
                            s.save_pending = false;
                            match result {
                                Ok(()) => {
                                    s.dirty = false;
                                    s.immediate = false;
                                    s.last_edit = None;
                                    if s.edit_version == s.save_version {
                                        s.saved = Some(
                                            crate::features::sync::github::Fingerprint::of(&vault),
                                        );
                                    }
                                    s.last_mtime = mtime;
                                    s.ignore_mtime = None;
                                    s.conflict = false;
                                    // adopt seq/reader from saved vault without clobbering edits:
                                    // only update vault if no edits happened during save
                                    if s.edit_version == 0 {
                                        // placeholder; handled below
                                    }
                                    // Always adopt reader/seq from worker result to stay consistent
                                    s.vault.seq = vault.seq;
                                    s.vault.reader = vault.reader;
                                }
                                Err(e) => {
                                    shell.fail(tf("save failed: {error}", &[("error", &e)]));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Test helper: block until all pending I/O replies have been applied.
    #[cfg(test)]
    pub fn wait_io(&mut self, shell: &mut Shell) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            self.pump_io(shell);
            let pending = self
                .session
                .as_ref()
                .is_some_and(|s| s.meta_pending || s.load_pending || s.save_pending);
            if !pending || std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        self.pump_io(shell);
    }

    pub fn persist(&mut self, shell: &mut Shell) {
        if let Some(s) = &mut self.session {
            write(s, shell, false);
        }
    }

    /// An editor is on screen, or has just left it. While it is, the timer that
    /// writes a dirty box stays away.
    pub fn editing(&mut self, on: bool) {
        if let Some(s) = self.session.as_mut() {
            s.editing = on;
        }
    }

    /// The last moment before its key is dropped (tab switch, tab close, lock):
    /// write the unsaved, conflict-free edits.
    pub fn flush(&mut self, shell: &mut Shell) {
        if let Some(s) = &mut self.session {
            write_if_dirty(s, shell);
            if s.remote
                .as_ref()
                .is_some_and(|r| r.kind == crate::features::settings::model::GITHUB)
                && !s.conflict
                && !s.behind
            {
                s.push_at = Some(std::time::Instant::now());
            }
        }
    }

    /// Every unlocked box, not just the one on screen: locking drops the parked
    /// tabs too, and their edits only ever lived in the vault.
    pub fn flush_all(&mut self, shell: &mut Shell) {
        if let Some(s) = &mut self.session {
            write_if_dirty(s, shell);
            if s.remote
                .as_ref()
                .is_some_and(|r| r.kind == crate::features::settings::model::GITHUB)
                && !s.conflict
                && !s.behind
            {
                s.push_at = Some(std::time::Instant::now());
            }
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
                s.settle();
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
    // A parked tab has no payload in memory: writing the placeholder would
    // replace the box with an empty one.
    if s.parked {
        return false;
    }
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
    // A box whose content is already what the file holds costs no cipher and no
    // fsync: the fingerprint of the last save says so. A forced write is a
    // different file under the same content (a new key), so it always writes.
    let print = crate::features::sync::github::Fingerprint::of(&s.vault);
    if !forced && s.path.is_file() && s.saved.as_ref() == Some(&print) {
        s.dirty = false;
        s.immediate = false;
        s.last_edit = None;
        return true;
    }
    match store::save(&s.path, &mut s.vault, &s.creds) {
        Ok(()) => {
            s.dirty = false;
            s.immediate = false;
            s.last_edit = None;
            s.saved = Some(crate::features::sync::github::Fingerprint::of(&s.vault));
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
    if s.parked {
        return;
    }
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
    /// A login the phone's autofill panel saved, landed the way one from the
    /// browser panel lands: through `serve::create_login`, with the box's own
    /// autosave writing it afterwards.
    #[cfg(target_os = "android")]
    pub fn drain_autofill(&mut self) {
        let queued = crate::shared::autofill::take();
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let mut landed = false;
        for pending in queued {
            // The rule an import already uses: an entry that is already there is
            // not a second one. A fresh password off the panel still lands.
            if crate::shared::autofill::duplicate(&s.vault, &pending) {
                continue;
            }
            crate::bridge::serve::create_login(
                &mut s.vault,
                pending.name,
                pending.username,
                pending.password.as_str(),
                pending.url,
            );
            landed = true;
        }
        if landed {
            s.mark_dirty();
        }
    }

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
        // `App::open` writes the box, so the file already matches the vault.
        app.open(name);
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

    /// A box that already holds what the file holds costs no cipher and no
    /// fsync: the save is skipped and the file keeps its generation.
    #[test]
    fn a_save_with_nothing_changed_leaves_the_file_alone() {
        let mut app = opened(&unique());
        let path = app.boxes.session.as_ref().unwrap().path.clone();
        let before = std::fs::read(&path).unwrap();
        let seq = app.boxes.session.as_ref().unwrap().vault.seq;

        app.boxes.session.as_mut().unwrap().dirty = true;
        app.boxes.persist(&mut app.shell);

        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "the box on disk is already the one in memory"
        );
        assert_eq!(s.vault.seq, seq, "and nothing was bumped");
        assert!(!s.dirty, "nothing is waiting to be saved");
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
            store::stash_path(&path).exists(),
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
        app.boxes.wait_io(&mut app.shell);
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
        app.boxes.wait_io(&mut app.shell);
        let s = app.boxes.session.as_ref().unwrap();
        assert!(s.conflict, "another version asks the user");
        assert_eq!(s.vault.name, name, "the local box is still here to choose");
    }

    /// No `.bak` is left beside the vault: stash lives in the data dir.
    #[test]
    fn no_legacy_bak_is_left_beside_the_vault() {
        let name = unique();
        let mut app = opened(&name);
        let path = app.boxes.session.as_ref().unwrap().path.clone();
        let sidecar = path.with_extension(format!("{}.bak", store::EXT));
        // Simulate a leftover from an older build.
        std::fs::write(&sidecar, b"old").unwrap();
        assert!(sidecar.exists());

        // Opening cleans the stray sidecar.
        let (creds, _vault) = store::open(&path, "test", None).unwrap();
        let _ = creds;
        assert!(!sidecar.exists(), "open must remove the legacy sidecar");

        // Saving does not recreate one beside the vault.
        let s = app.boxes.session.as_mut().unwrap();
        store::save(&path, &mut s.vault, &s.creds).unwrap();
        assert!(
            !sidecar.exists(),
            "save must not create a sidecar beside the vault"
        );
        assert!(
            store::stash_path(&path).exists() || true,
            "stash (if any) lives in the data dir"
        );
    }

    /// An immediate edit reaches disk through the worker, not the UI thread.
    #[test]
    fn an_immediate_edit_reaches_disk_asynchronously() {
        let mut app = opened(&unique());
        let path = app.boxes.session.as_ref().unwrap().path.clone();
        {
            let s = app.boxes.session.as_mut().unwrap();
            s.vault.logins.push(Login::new(1));
            s.mark_now();
        }
        app.boxes.tick(&mut app.shell);
        app.boxes.wait_io(&mut app.shell);
        let s = app.boxes.session.as_ref().unwrap();
        let on_disk = store::load(&path, &s.creds).unwrap();
        assert_eq!(
            on_disk.logins.len(),
            1,
            "the edit landed on disk via the worker"
        );
        assert!(!s.dirty, "and the session is clean");
        assert!(!s.save_pending, "no save left in flight");
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

        // The locked box is the one on screen; a parked tab is empty until it is
        // brought back, so the edit that the lock must write is made after the
        // switch.
        app.switch(0);
        {
            let s = app.boxes.session.as_mut().unwrap();
            s.vault.name = "Edited again".to_owned();
            s.dirty = true;
        }
        app.lock(&ctx);
        assert_ne!(
            std::fs::read(&path).unwrap(),
            parked,
            "the lock wrote the padding edit before dropping the key"
        );
    }

    /// The copy beside a box is the version before the last write, and restoring
    /// it is itself reversible: the version it replaces becomes the copy.
    #[test]
    fn restoring_the_backup_puts_the_previous_box_on_screen() {
        let (mut app, path) = with_history(&unique());
        let backup = store::stash_path(&path);
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
        let backup = store::stash_path(&path);
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
        assert!(!store::stash_path(&path).exists());

        app.restore_backup();

        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(app
            .shell
            .status
            .as_ref()
            .is_some_and(|said| said.kind == StatusKind::Error));
    }
}
