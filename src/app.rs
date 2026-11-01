//! App state and the eframe loop; each feature keeps its own state in `features/`.

use crate::features::auth::model::{Auth, WelcomeForm};
use crate::features::boxes::model::Boxes;
use crate::features::entries::model::{Axis, View};
use crate::features::settings::model::Prefs;
use crate::features::shell::model::Shell;
use crate::features::sync::model::Sync;
use crate::shared::idle::Monitor;
use crate::shared::theme;
use crate::vault::store;
use std::path::PathBuf;
use zeroize::Zeroize;

pub use crate::features::auth::model::{Login, Op, OpKind, OpOutcome, Side, SignIn};
pub use crate::features::boxes::model::{Session, Tab};
pub use crate::features::generator::model::Form as GenerateForm;
pub use crate::features::shell::confirm::Confirm;
pub use crate::features::shell::model::StatusKind;
pub use crate::features::sync::model::{Job, Reply, RepoJob, RepoReply};

/// One Pandora per data dir: token, prefs and vault are single-writer resources.
pub fn hold_at(dir: &std::path::Path) -> std::io::Result<std::fs::File> {
    let lock = std::fs::File::create(dir.join("pandora.lock"))?;
    lock.try_lock()?;
    Ok(lock)
}

/// How long a starting build waits for the lock, and how often it asks.
const HOLD_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);
const HOLD_STEP: std::time::Duration = std::time::Duration::from_millis(25);

pub fn hold() -> std::io::Result<std::fs::File> {
    let _ = std::fs::create_dir_all(store::data_dir());
    hold_within(&store::data_dir(), HOLD_WAIT)
}

/// Hold the lock, outwaiting the build this one replaced: an update starts the new
/// process before the old one has finished exiting, and a lock taken instantly
/// would turn that into a window that closes itself. A second launch by hand waits
/// the same and then says the same thing it always did.
fn hold_within(dir: &std::path::Path, wait: std::time::Duration) -> std::io::Result<std::fs::File> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match hold_at(dir) {
            Ok(lock) => return Ok(lock),
            Err(why) if std::time::Instant::now() >= deadline => return Err(why),
            Err(_) => std::thread::sleep(HOLD_STEP),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_hold_at_the_same_dir_fails() {
        let dir = std::env::temp_dir().join("pandora-lock-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = hold_at(&dir).unwrap();
        let second = hold_at(&dir);
        assert!(second.is_err(), "one lock per data dir");
        drop(first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The update path starts the new build before the old one is gone, so the new
    /// one has to outwait the lock rather than exit with no window.
    #[test]
    fn a_hold_outwaits_the_build_that_is_still_exiting() {
        let dir = std::env::temp_dir().join("pandora-lock-wait-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let held = hold_at(&dir).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(120));
            drop(held);
        });
        let waited = hold_within(&dir, std::time::Duration::from_millis(2000));
        release.join().unwrap();
        assert!(
            waited.is_ok(),
            "the replacement has to take the lock, not give up on it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// And it gives up on a lock that never comes, with the error the app reports.
    #[test]
    fn a_hold_that_is_never_released_still_fails() {
        let dir = std::env::temp_dir().join("pandora-lock-giveup-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _held = hold_at(&dir).unwrap();
        let refused = hold_within(&dir, std::time::Duration::from_millis(80));
        assert!(refused.is_err(), "a running instance still wins");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A new release is announced on whatever screen the app is on, not only once a
    /// box is open: the notice is not about the box.
    #[test]
    fn a_new_release_is_announced_with_no_box_open() {
        use crate::shared::ui::shot;
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.update.info = Some(crate::features::update::model::Info {
            latest: "9.9.9".to_owned(),
            url: "https://example.invalid/releases".to_owned(),
            installable: false,
        });
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(shot::input(egui::vec2(420.0, 780.0), Vec::new()), |ui| {
                app.run_ui(ui)
            });
            out.textures_delta.clear();
            shapes = out.shapes;
        }
        let drawn = shot::words(&shapes);
        assert!(
            drawn.iter().any(|word| word.contains("9.9.9")),
            "the version is named with no box open: {drawn:?}"
        );
        assert!(
            drawn
                .iter()
                .any(|word| word.starts_with("Open release page")),
            "a build that cannot replace itself is offered the page: {drawn:?}"
        );
        assert!(!app.update.busy(), "no check was asked of the network");
    }

    /// The app's settings dialog over whatever screen is behind it; the whole loop
    /// runs so the button, its state and the dialog are exercised together.
    #[test]
    fn the_apps_own_settings_open_over_a_screen_with_no_box() {
        use crate::shared::ui::shot;
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        let size = egui::vec2(420.0, 780.0);
        // The bar rides an `Area`: two idle frames before a rect is worth clicking.
        for _ in 0..2 {
            let mut out = ctx.run_ui(shot::input(size, Vec::new()), |ui| app.run_ui(ui));
            out.textures_delta.clear();
        }
        let at = ctx
            .read_response(egui::Id::new("bar-prefs"))
            .expect("the bar carries the app's own settings")
            .rect
            .center();
        for events in [
            vec![egui::Event::PointerMoved(at), shot::press(at, true)],
            vec![shot::press(at, false)],
        ] {
            let mut out = ctx.run_ui(shot::input(size, events), |ui| app.run_ui(ui));
            out.textures_delta.clear();
        }
        assert!(app.prefs_open, "the sliders open the app's settings");
        assert_eq!(app.screen, Screen::Sources, "and leave the screen behind");

        let mut shapes = Vec::new();
        for _ in 0..2 {
            // The dialog is an `Area` too, with its own sizing pass.
            let mut out = ctx.run_ui(shot::input(size, Vec::new()), |ui| app.run_ui(ui));
            out.textures_delta.clear();
            shapes = out.shapes;
        }
        let drawn = shot::words(&shapes);
        for want in [
            "GENERAL SETTINGS",
            "APPEARANCE",
            "LANGUAGE",
            "Dark",
            "Light",
            "English",
            "Español",
        ] {
            assert!(
                drawn.iter().any(|word| word == want),
                "the app's settings are painted over the screen: missing {want}"
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Welcome,
    Unlock,
    /// The boxes the app knows about: local files first, then each repo.
    Sources,
    Main,
    /// The password tool. It reads nothing from a box, so it opens with none.
    Generate,
    /// The camera reading a QR, full screen; the variant exists only on a phone.
    #[cfg(target_os = "android")]
    Scan,
}

/// Masked text fields of an entry; each reveals on its own.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Password,
    Notes,
    Secret,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Drag {
    Entry(String),
    Folder(String),
}

/// A coarse place: screen, box, pane, folder and sources view; any change is one step back.
#[derive(Clone, PartialEq)]
struct Place {
    screen: Screen,
    side: Option<Side>,
    vault: Option<PathBuf>,
    tab: Tab,
    folder: Option<String>,
    view: View,
    axis: Axis,
    at: Vec<String>,
}

/// The place on screen plus the ones left behind, oldest first.
#[derive(Default)]
struct Nav {
    at: Option<Place>,
    past: Vec<Place>,
}

pub struct App {
    pub screen: Screen,
    pub icons: theme::Icons,
    pub shell: Shell,
    pub auth: Auth,
    pub boxes: Boxes,
    pub sync: Sync,
    /// The one thing this app asks the network without an account: is there a
    /// newer Pandora than this one?
    pub update: crate::features::update::Update,
    /// Settings from the app data dir; `prefs.lock_after` is the system-idle timeout.
    pub prefs: Prefs,
    pub generate: GenerateForm,
    /// The open document preview: the audio player and the rendered PDF page.
    /// Both stop when the entry they belong to is no longer selected.
    pub preview: crate::features::drive::preview::Preview,
    /// The SSH agent: the box's allowed keys on a socket, empty while locked.
    pub agent: crate::features::agent::Agent,
    /// The share dialog: one record sealed out, or one file opened in.
    pub share: crate::features::share::Share,
    /// The review's one optional lookup: whether a password is in a public breach.
    /// Off until the user turns it on, and it never leaves this process while off.
    pub health: crate::features::health::Check,
    /// The Ctrl+K palette: what was typed and which hit the keyboard is on.
    pub find: crate::features::find::model::Find,
    /// Where a dropped file lands this frame: set by the drive while drawing, consumed
    /// by `handle_dropped`. `None` = nothing under the pointer, `Some(None)` = root.
    pub drop_at: Option<Option<String>>,
    /// The rect of the folder under the pointer while files are being dragged,
    /// with its own corner radius, so the overlay can outline exactly where they
    /// would land.
    pub drop_highlight: Option<(egui::Rect, u8)>,
    /// The app's own settings are open: the bar's sliders, as a dialog. It is not
    /// a page of the box, and it rides every screen.
    pub prefs_open: bool,
    pub idle: Monitor,
    /// The local socket the host relays through, so a browser never asks for the password.
    pub bridge: crate::features::bridge::Bridge,
    /// The one system file dialog that may be out: it runs off the frame thread,
    /// so the window keeps painting while the picker waits for the user.
    pub ask: Option<crate::features::shell::pick::Ask>,
    /// `LOW_LATENCY` at rest, raised while a resize is in flight; wgpu only, since with
    /// Glow the swap interval is the compositor's.
    #[cfg(not(target_os = "android"))]
    surface: eframe::SurfaceConfig,
    /// Until this instant the window is considered resizing.
    #[cfg(not(target_os = "android"))]
    resize_until: Option<std::time::Instant>,
    /// Last viewport size, so a resize can be detected without a winit event.
    #[cfg(not(target_os = "android"))]
    last_size: egui::Vec2,
    /// Whether this process draws through wgpu; the swapchain knob is a wgpu API.
    #[cfg(not(target_os = "android"))]
    wgpu: bool,
    /// What the system needs to keep this window out of a screen capture.
    #[cfg(not(target_os = "android"))]
    handle: Option<raw_window_handle::RawWindowHandle>,
    nav: Nav,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // A logout or a `kill` ends the run without the window ever closing, so the
        // signals are taken over before anything else: the frame that reads the flag
        // writes the boxes and only then lets the process go.
        #[cfg(unix)]
        crate::shared::stop::listen(cc.egui_ctx.clone());
        // A picker copy that never reached a save is plaintext; drop it now.
        store::clear_staging();
        // Drop orphaned atomic-write temps left by a crashed save.
        store::clear_orphan_tmps(&store::data_dir());
        // Prefs first: language and scheme are applied to the context, or the first
        // frame paints a scheme the user did not choose.
        let prefs = crate::features::settings::model::load();
        crate::shared::i18n::set(prefs.lang);
        theme::set(prefs.theme);
        theme::apply(&cc.egui_ctx);
        let icons = theme::Icons::load(&cc.egui_ctx);
        let path = store::default_path();
        let path_str = path.display().to_string();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Box")
            .to_owned();
        let recents = crate::features::auth::model::load();
        let mut shell = Shell::default();
        let mut sync = Sync::default();
        // A remembered session comes back before the first frame. A store
        // answering "nothing" is worth a line on screen.
        if prefs.remember {
            // Never kept, key rotated or keystore not up yet would otherwise be the same silence.
            log::info!("pandora: the session is remembered; asking the store for it");
            if let Some(why) = sync.recall() {
                shell.fail(why);
            }
        }
        let auth = Auth {
            unlock_path: path_str.clone(),
            recents,
            welcome: WelcomeForm {
                name,
                path: path_str,
                ..Default::default()
            },
            ..Auth::default()
        };
        let screen = auth.landing();
        #[cfg(not(target_os = "android"))]
        let handle = {
            use raw_window_handle::HasWindowHandle as _;
            cc.window_handle().ok().map(|handle| handle.as_raw())
        };
        let mut app = Self {
            screen,
            icons,
            shell,
            auth,
            boxes: Boxes::default(),
            sync,
            update: crate::features::update::Update::default(),
            prefs,
            generate: GenerateForm::default(),
            preview: crate::features::drive::preview::Preview::default(),
            find: crate::features::find::model::Find::default(),
            agent: crate::features::agent::Agent::default(),
            share: crate::features::share::Share::default(),
            health: crate::features::health::Check::default(),
            drop_at: None,
            drop_highlight: None,
            prefs_open: false,
            idle: Monitor::start(),
            bridge: crate::features::bridge::Bridge::start(cc.egui_ctx.clone()),
            ask: None,
            #[cfg(not(target_os = "android"))]
            surface: eframe::SurfaceConfig::LOW_LATENCY,
            #[cfg(not(target_os = "android"))]
            resize_until: None,
            #[cfg(not(target_os = "android"))]
            last_size: egui::Vec2::ZERO,
            #[cfg(not(target_os = "android"))]
            wgpu: cc.wgpu_render_state.is_some(),
            #[cfg(not(target_os = "android"))]
            handle,
            nav: Nav::default(),
        };
        // Before the first frame: a window painted once and hidden after would
        // already be in whatever capture caught that frame.
        #[cfg(not(target_os = "android"))]
        crate::shared::capture::apply(app.handle.as_ref(), app.prefs.allow_capture);
        // One look at startup, and only when the switch allows it: a vault that
        // phones home on its own is the thing this app promises not to be.
        if app.prefs.update_check {
            app.update.check(&cc.egui_ctx);
        }
        app
    }

    /// Apply the capture switch to the window, for the change made in Settings.
    #[cfg(not(target_os = "android"))]
    pub fn capture(&self) {
        crate::shared::capture::apply(self.handle.as_ref(), self.prefs.allow_capture);
    }

    /// Lock every vault and land on Unlock; unsaved edits are written first.
    pub fn lock(&mut self, ctx: &egui::Context) {
        self.boxes.flush_all(&mut self.shell);
        // A push the box has not sent goes now; the worker keeps its own copy.
        if self
            .boxes
            .session
            .as_ref()
            .is_some_and(|s| s.push_at.is_some())
        {
            self.send(false);
        }
        self.shell.take_back_clip();
        // A recovery code on screen goes with the box: it is a copy of the key.
        self.auth.code_shown = None;
        self.shell.activity = None;
        self.boxes.session = None; // drops keys + vaults: Zeroizing fields wipe secrets
                                   // The phone's autofill copy goes with the keys, not with the next frame:
                                   // an idle lock, a closed box and a lock from Settings all land here.
        crate::shared::autofill::clear();
        self.boxes.tabs.clear();
        self.boxes.active = 0;
        self.preview = Default::default();
        self.find = Default::default();
        self.auth.unlock_password.zeroize();
        self.generate.out.zeroize();
        self.auth.recent_search.clear();
        self.auth.op = None;
        self.auth.login = None;
        self.prefs_open = false;
        // The repo token and a push in flight stay: neither reads anything without the
        // password. Sign out clears them.
        // Every vault is gone, so no place behind this one can be returned to.
        self.nav = Nav::default();
        self.screen = self.auth.landing();
        // egui keeps its own copy of every field, the master password included in the
        // undo snapshot; clearing the lot is the only way to reach it.
        ctx.data_mut(|d| d.clear());
    }

    fn watch_lock(&mut self, ctx: &egui::Context) {
        self.shell.focused = ctx.input(|i| i.focused);
        if ctx.input(|i| !i.events.is_empty()) {
            self.shell.activity = Some(std::time::Instant::now());
        }
        // A phone in a pocket paints no frame, so the first one after the pause is
        // the last chance to put what is only in memory on disk. The box is not
        // locked here: it is only written.
        #[cfg(target_os = "android")]
        if crate::shared::android::take_paused() {
            self.boxes.flush_all(&mut self.shell);
        }
        let state = self.idle.poll();
        if self.boxes.session.is_none() {
            self.idle.set_active(false);
            return;
        }
        self.idle.set_active(true);
        #[cfg(target_os = "android")]
        if crate::shared::android::take_resumed() {
            self.shell.activity = Some(std::time::Instant::now());
            self.sync.folder_resume = true;
        }
        #[cfg(not(target_os = "android"))]
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        let activity_idle = self
            .shell
            .activity
            .map_or(std::time::Duration::ZERO, |at| at.elapsed());
        #[cfg(target_os = "android")]
        let activity_idle = if crate::shared::android::foreground() {
            activity_idle
        } else {
            std::time::Duration::ZERO
        };
        let timeout = std::time::Duration::from_secs(self.prefs.lock_after);
        #[cfg(target_os = "android")]
        if crate::shared::android::foreground() && timeout > std::time::Duration::ZERO {
            let left = timeout.saturating_sub(activity_idle);
            if !left.is_zero() {
                ctx.request_repaint_after(left);
            }
        }
        if !crate::shared::idle::should_lock(state, timeout, activity_idle) {
            return;
        }
        let reason = if state.locked {
            "system locked"
        } else if state.suspended {
            "system resumed"
        } else {
            "inactivity"
        };
        self.lock(ctx);
        self.shell.set_status(
            crate::shared::i18n::tf(
                "box locked: {reason}",
                &[("reason", &crate::shared::i18n::t(reason))],
            ),
            StatusKind::Info,
        );
    }

    /// The place, coarsely; any change is one step back.
    fn place(&self) -> Place {
        let s = self.boxes.session.as_ref();
        Place {
            screen: self.screen,
            // None and `Recents` show the same view: one spelling, so it is not a step.
            side: self.auth.side.or(Some(Side::Recents)),
            vault: s.map(|s| s.path.clone()),
            tab: s.map_or(Tab::Passwords, |s| s.tab),
            folder: s.and_then(|s| s.folder.clone()),
            view: s.map_or(View::List, |s| s.view),
            axis: s.map_or(Axis::Target, |s| s.axis),
            at: s.map_or_else(Vec::new, |s| s.at.clone()),
        }
    }

    /// Push the place left when the current one changes; a place whose box is closed is
    /// dropped. Once a frame, after the screens, so it catches every navigation.
    pub(crate) fn record_place(&mut self) {
        let now = self.place();
        let Some(old) = self.nav.at.clone() else {
            self.nav.at = Some(now);
            return;
        };
        if old == now {
            return;
        }
        // Leaving the scanner — bar or platform — puts the camera away.
        #[cfg(target_os = "android")]
        if old.screen == Screen::Scan && now.screen != Screen::Scan {
            self.boxes.stop_scan();
        }
        self.nav.at = Some(now);
        if old
            .vault
            .as_ref()
            .is_none_or(|p| self.boxes.position(p).is_some())
        {
            self.nav.past.push(old);
        }
    }

    /// Is there anything to undo? The overlay on top, or a place behind.
    pub fn can_back(&self) -> bool {
        let overlay = self.boxes.session.as_ref().is_some_and(|s| {
            s.confirm.is_some()
                || s.show_new
                || s.show_qr
                || s.show_trash
                || s.editing_folder.is_some()
                || s.new_folder.is_some()
                || s.editing_entry.is_some()
                || s.selected.is_some()
        });
        self.prefs_open
            || self.find.open
            || overlay
            || self.sync.adding
            || self.sync.new_in.is_some()
            || !self.nav.past.is_empty()
    }

    /// Close the topmost layer, then step back through places; overlays are not places.
    fn close_overlay(&mut self) -> bool {
        if std::mem::take(&mut self.find.open) {
            self.find.query.clear();
            return true;
        }
        if std::mem::take(&mut self.prefs_open) {
            return true;
        }
        if self.generate.entry.take().is_some() {
            return true;
        }
        if let Some(s) = self.boxes.session.as_mut() {
            if std::mem::take(&mut s.show_qr) {
                return true;
            }
            if std::mem::take(&mut s.show_trash) {
                return true;
            }
            if s.confirm.take().is_some() {
                return true;
            }
            if std::mem::take(&mut s.show_new) {
                return true;
            }
            if s.new_folder.take().is_some() {
                return true;
            }
            if let Some(id) = s.editing_folder.take() {
                // The folder's own row was selected when the editor opened: one
                // press closes both, not two.
                if s.selected.as_deref() == Some(id.as_str()) {
                    s.selected = None;
                }
                return true;
            }
            if let Some(id) = s.editing_entry.take() {
                // The entry's own card was selected when the editor opened: one
                // press closes both, not two.
                if s.selected.as_deref() == Some(id.as_str()) {
                    s.selected = None;
                }
                self.preview.sync(None);
                return true;
            }
            if s.selected.take().is_some() {
                self.preview.sync(None);
                return true;
            }
        }
        if self.sync.new_in.take().is_some() {
            return true;
        }
        if std::mem::take(&mut self.sync.adding) {
            self.sync.repo_search.clear();
            return true;
        }
        false
    }

    /// One step back: close the overlay on top, else return to the place left.
    pub fn back(&mut self) {
        if self.close_overlay() {
            return;
        }
        let Some(place) = self.nav.past.pop() else {
            return;
        };
        self.apply(place);
        self.nav.at = Some(self.place());
    }

    /// The phone's own back: a field with focus eats the first press to put the
    /// keyboard away, then one step back, then leaving the app when there is
    /// nowhere behind. Android's back button and back gesture arrive as
    /// `Key::BrowserBack` (`KEYCODE_BACK` is consumed by the native activity
    /// before the framework can, so nothing else ever sees it).
    pub fn back_or_leave(&mut self, ctx: &egui::Context) {
        if let Some(id) = ctx.memory(|m| m.focused()) {
            if egui::widgets::text_edit::TextEditState::load(ctx, id).is_some() {
                ctx.memory_mut(|m| m.surrender_focus(id));
                return;
            }
        }
        if self.can_back() {
            self.back();
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Return to a place: switch box if needed, then screen, view, pane and folder;
    /// overlays stay closed.
    fn apply(&mut self, place: Place) {
        if let Some(path) = place.vault.as_ref() {
            if let Some(at) = self.boxes.position(path) {
                let same = self.boxes.session.as_ref().is_some_and(|s| &s.path == path);
                if !same {
                    self.switch(at);
                }
            }
        }
        let on_vault = self.boxes.session.as_ref().map(|s| &s.path) == place.vault.as_ref();
        self.screen = place.screen;
        self.auth.side = place.side;
        self.generate.entry = None;
        if on_vault {
            if let Some(s) = self.boxes.session.as_mut() {
                s.tab = place.tab;
                s.folder = place.folder;
                s.view = place.view;
                s.axis = place.axis;
                s.at = place.at.clone();
                s.selected = None;
                s.search.clear();
                s.confirm = None;
                s.editing_folder = None;
                s.editing_entry = None;
                s.sanitize();
            }
            self.preview.sync(None);
        }
    }

    /// Forget the places whose box has just been closed.
    pub(crate) fn forget_closed(&mut self) {
        let mut open: Vec<PathBuf> = Vec::with_capacity(self.boxes.tabs.len() + 1);
        if let Some(s) = self.boxes.session.as_ref() {
            open.push(s.path.clone());
        }
        open.extend(self.boxes.tabs.iter().map(|s| s.path.clone()));
        self.nav
            .past
            .retain(|p| p.vault.as_ref().is_none_or(|v| open.contains(v)));
    }

    pub fn run_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // A signal asked to stop: everything in memory goes to disk, and only then
        // does the window close. The exit hook writes again, which is free when clean.
        #[cfg(unix)]
        if crate::shared::stop::asked() {
            self.boxes.flush_all(&mut self.shell);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // A native move can take the focus away for as long as the gesture lasts;
        // that is not the user walking away from the window.
        self.shell.settle(
            ctx.input(|i| i.focused),
            ctx.input(|i| i.pointer.any_released()),
        );
        // Before anything draws: a phone's typed character must land on the focused widget.
        #[cfg(target_os = "android")]
        crate::shared::android::feed_keyboard(&ctx);
        self.pump();
        self.pump_bridge(&ctx);
        self.pump_sync(&ctx);
        self.pump_update(&ctx);
        self.pump_pick(&ctx);
        self.health.pump(&ctx);
        // A preview belongs to its entry: closing the modal, changing pane or locking
        // puts the player and the PDF away.
        let watching = self.screen == Screen::Main
            && self
                .boxes
                .session
                .as_ref()
                .is_some_and(|s| s.tab == Tab::Drive);
        let entry = if watching {
            self.boxes
                .session
                .as_ref()
                .and_then(|s| s.editing_entry.clone())
        } else {
            None
        };
        self.preview.sync(entry.as_deref());
        self.drop_at = None;
        self.drop_highlight = None;

        // While files are dragged, keep frames coming so the overlay and the hovered
        // folder stay painted; the drag arrives as a hover until it is dropped.
        let dragging = self.screen == Screen::Main
            && self.boxes.session.is_some()
            && ui
                .ctx()
                .input(|i| !i.raw.hovered_files.is_empty() || !i.raw.dropped_files.is_empty());
        if dragging
            && self
                .boxes
                .session
                .as_ref()
                .is_some_and(|s| s.tab != Tab::Drive)
        {
            self.shell.set_status(
                crate::shared::i18n::t("open Files to drop files"),
                StatusKind::Info,
            );
        }
        if dragging {
            ui.ctx().request_repaint();
        }
        self.shell.expire_status();
        self.shell.sweep_clip(&ctx, self.prefs.clip_clear);
        self.watch_lock(&ctx);
        if self.boxes.session.is_some() {
            self.boxes.pump_io(&mut self.shell);
            self.boxes.tick(&mut self.shell);
        }
        // What a phone's autofill may offer, kept in step after the tick that can
        // save and the watch that can lock: one call, so no transition forgets it.
        #[cfg(target_os = "android")]
        {
            // Off publishes nothing: the service then finds no box and offers only
            // the way to open the app.
            let published = if self.prefs.autofill {
                self.boxes
                    .session
                    .as_ref()
                    .map(|s| (s.path.as_path(), &s.vault))
            } else {
                None
            };
            crate::shared::autofill::follow(published);
            // Still drained when it is off: a login queued a moment before the
            // switch was turned off is the user's, not the setting's.
            self.boxes.drain_autofill();
        }
        // A window nobody is looking at shows only the logo: the app keeps ticking
        // above, but no screen, status or modal paints through it.
        #[cfg(not(target_os = "android"))]
        if self.prefs.hide_unfocused && !self.shell.moving() && !ctx.input(|i| i.focused) {
            crate::features::shell::cover::show(&ctx, self);
            return;
        }
        // The bar runs everywhere; only a desktop has edges, on the bar's layer so a
        // modal cannot cover them.
        crate::features::shell::windowbar::show(ui, self);
        self.shell.draw(ui);
        // An available release rides every screen: it is not about the open box.
        crate::features::update::ui::strip(ui, self);
        match self.screen {
            Screen::Welcome => crate::features::auth::ui::welcome(ui, self),
            Screen::Unlock => crate::features::auth::ui::unlock(ui, self),
            Screen::Sources => crate::features::sync::ui::run(ui, self),
            Screen::Generate => crate::features::generator::ui::main(ui, self),
            #[cfg(target_os = "android")]
            Screen::Scan => crate::features::codes::ui::scan(ui, self),
            Screen::Main => {
                if self.boxes.session.is_none() {
                    self.screen = Screen::Unlock;
                    return;
                }
                crate::features::shell::screen::main(ui, self);
            }
        }
        // The app's settings ride every screen: their button is on the bar.
        crate::features::settings::ui::general(&ctx, self);
        // The trash draws outside the settings pane, so a restore can jump to the record.
        crate::features::settings::ui::trash_modal(&ctx, self);
        // The Ctrl+K palette is an overlay: it searches the box on screen.
        crate::features::find::ui::modal(&ctx, self);
        // Last, over whatever screen is behind: a browser asking in can arrive on any.
        crate::features::bridge::modal(&ctx, self);

        // While files are dragged, cover the viewport (bar included) and outline the
        // hovered folder.
        if dragging {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop-overlay"),
            ));
            let rect = ctx.viewport_rect();
            painter.rect_filled(rect, egui::CornerRadius::ZERO, theme::palette().scrim);
            let message = crate::shared::i18n::t("Drop the file");
            let galley = painter.layout_no_wrap(
                message.to_owned(),
                egui::FontId::proportional(22.0),
                theme::ACCENT,
            );
            let icon_size = 56.0;
            let gap = 16.0;
            let total = icon_size + gap + galley.size().y;
            let top = rect.center().y - total * 0.5;
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(rect.center().x, top + icon_size * 0.5),
                egui::vec2(icon_size, icon_size),
            );
            if let Some(id) = self.icons.id(theme::Icon::Upload) {
                painter.image(
                    id,
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    theme::ACCENT,
                );
            }
            let pos = egui::pos2(
                rect.center().x - galley.size().x * 0.5,
                top + icon_size + gap,
            );
            painter.galley(pos, galley, theme::ACCENT);
            if let Some((highlight, radius)) = self.drop_highlight {
                painter.rect_stroke(
                    highlight,
                    theme::radius(radius),
                    theme::stroke(theme::ACCENT, 4.0),
                    egui::StrokeKind::Inside,
                );
            }
        }

        self.shortcuts(&ctx);
        self.handle_dropped(&ctx);
        self.record_place();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // A resize raises the swapchain to two frames of room, then back to one; wgpu only.
        #[cfg(not(target_os = "android"))]
        {
            const SETTLE: std::time::Duration = std::time::Duration::from_millis(250);
            let size = ui.ctx().content_rect().size();
            if size != self.last_size {
                self.last_size = size;
                self.resize_until = Some(std::time::Instant::now() + SETTLE);
            }
            let resizing = self
                .resize_until
                .is_some_and(|at| at > std::time::Instant::now());
            let wanted = if resizing {
                eframe::SurfaceConfig::HIGH_THROUGHPUT
            } else {
                eframe::SurfaceConfig::LOW_LATENCY
            };
            if wanted != self.surface {
                if self.wgpu {
                    _frame.set_wgpu_surface_config(wanted);
                }
                self.surface = wanted;
            }
        }
        self.run_ui(ui);
    }

    /// Quitting is the two last moments at once: the box on screen and every parked tab
    /// are written, so nothing that only lived in memory dies with the window, and the
    /// clipboard is taken back, which the countdown never gets to see.
    #[cfg(all(not(target_os = "android"), feature = "glow"))]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.boxes.flush_all(&mut self.shell);
        self.shell.take_back_clip();
    }

    #[cfg(all(not(target_os = "android"), not(feature = "glow")))]
    fn on_exit(&mut self) {
        self.boxes.flush_all(&mut self.shell);
        self.shell.take_back_clip();
    }

    /// The same for the phone: glow's exit hook hands over the GL context, unused.
    #[cfg(target_os = "android")]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.boxes.flush_all(&mut self.shell);
        self.shell.take_back_clip();
    }
}

/// Test fixtures: a bare app and a throwaway unlocked vault.
#[cfg(test)]
impl App {
    pub fn blank(ctx: &egui::Context) -> Self {
        Self {
            screen: Screen::Sources,
            icons: theme::Icons::load(ctx),
            shell: Shell::default(),
            auth: Auth::default(),
            boxes: Boxes::default(),
            sync: Sync::default(),
            update: crate::features::update::Update::default(),
            prefs: Prefs::default(),
            generate: GenerateForm::default(),
            preview: crate::features::drive::preview::Preview::default(),
            find: crate::features::find::model::Find::default(),
            agent: crate::features::agent::Agent::default(),
            share: crate::features::share::Share::default(),
            health: crate::features::health::Check::default(),
            drop_at: None,
            drop_highlight: None,
            prefs_open: false,
            idle: Monitor::default(),
            bridge: crate::features::bridge::Bridge::default(),
            #[cfg(not(target_os = "android"))]
            ask: None,
            #[cfg(not(target_os = "android"))]
            surface: eframe::SurfaceConfig::LOW_LATENCY,
            #[cfg(not(target_os = "android"))]
            resize_until: None,
            #[cfg(not(target_os = "android"))]
            last_size: egui::Vec2::ZERO,
            #[cfg(not(target_os = "android"))]
            wgpu: true,
            #[cfg(not(target_os = "android"))]
            handle: None,
            nav: Nav::default(),
        }
    }

    pub fn open(&mut self, name: &str) {
        // A path per test and per box name: two parallel tests never share a file.
        let test = std::thread::current()
            .name()
            .unwrap_or("test")
            .replace("::", "-");
        let path = std::env::temp_dir().join(format!(
            "pandora-{}-{test}-{name}.vault",
            std::process::id()
        ));
        // A box already in the strip is switched to, not saved: its parked session holds
        // the credentials.
        if let Some(i) = self.boxes.position(&path) {
            self.switch(i);
            return;
        }
        // A fresh run starts from a fresh file, not from the previous run's leftovers.
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(store::stash_path(&path));
        let creds = store::Credentials::rekey("test", None).unwrap();
        let mut vault = crate::vault::model::Vault::new(name);
        store::save(&path, &mut vault, &creds).unwrap();
        self.enter(creds, vault, path);
    }
}
