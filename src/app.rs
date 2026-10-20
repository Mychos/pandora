//! Application state + eframe loop: navigation plus the cross-cutting bits, with
//! each feature's own state grouped in `features/`.

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

/// Hold the data directory against another instance. One Pandora per data dir:
/// the refresh token, the prefs and the vault are all single-writer resources.
pub fn hold_at(dir: &std::path::Path) -> std::io::Result<std::fs::File> {
    let lock = std::fs::File::create(dir.join("pandora.lock"))?;
    lock.try_lock()?;
    Ok(lock)
}

pub fn hold() -> std::io::Result<std::fs::File> {
    let _ = std::fs::create_dir_all(store::data_dir());
    hold_at(&store::data_dir())
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
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Welcome,
    Unlock,
    /// The boxes the app knows about: local files first, then each repo. Also
    /// where a repo is added.
    Sources,
    Main,
    /// The password tool. It reads nothing from a box, so it opens with none.
    Generate,
    /// The camera reading a QR, full screen: a picture to aim, not a dialog. A
    /// phone's screen, so on a desktop the variant is not even here.
    #[cfg(target_os = "android")]
    Scan,
}

/// Masked text fields of an entry. Each one reveals on its own.
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

/// A coarse location: the screen, the box on it, the pane and folder inside it,
/// and the sources view. A change to any field is one step back.
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
    /// Settings from the app data dir; `prefs.lock_after` is the system-idle timeout.
    pub prefs: Prefs,
    pub generate: GenerateForm,
    /// The open document preview: the audio player and the rendered PDF page.
    /// Both stop when the entry they belong to is no longer selected.
    pub preview: crate::features::drive::preview::Preview,
    /// The Ctrl+K palette: what was typed and which hit the keyboard is on.
    pub find: crate::features::find::model::Find,
    /// Where a file dropped on the window lands this frame, if anywhere. Set by
    /// the drive's grid, tree and breadcrumb while they are drawn, consumed by
    /// `handle_dropped` at the end of the frame. `None` = nothing under the
    /// pointer, `Some(None)` = the drive root.
    pub drop_at: Option<Option<String>>,
    /// The rect of the folder under the pointer while files are being dragged,
    /// with its own corner radius, so the overlay can outline exactly where they
    /// would land.
    pub drop_highlight: Option<(egui::Rect, u8)>,
    /// The language picker is open: the bar's globe, as a modal.
    pub lang_open: bool,
    pub idle: Monitor,
    /// The local socket the native host relays through, so the browser never has to
    /// ask for the master password while this window is open.
    pub bridge: crate::features::bridge::Bridge,
    /// The swapchain latency currently applied. `LOW_LATENCY` at rest, raised
    /// while a resize is in flight so the edge does not trail the pointer.
    /// Desktop only: the phone draws through glow, where the swap interval is
    /// the compositor's.
    #[cfg(not(target_os = "android"))]
    surface: eframe::SurfaceConfig,
    /// Until this instant the window is considered resizing.
    #[cfg(not(target_os = "android"))]
    resize_until: Option<std::time::Instant>,
    /// Last viewport size, so a resize can be detected without a winit event.
    #[cfg(not(target_os = "android"))]
    last_size: egui::Vec2,
    nav: Nav,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // The prefs come first: the language and the scheme are both read out of them
        // and both are applied to the context, so loading them after the theme would
        // paint the first frame in a scheme the user did not choose.
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
        // A remembered session comes back before the first frame, so the one the
        // user signed in for is already there when the window opens. What the
        // store kept is the refresh token when there is one, and the access
        // token that goes with it is minted on the first frame.
        if prefs.remember {
            // A session that was asked for and is not there is worth a line: the
            // three ways the store can answer "nothing" — never kept, the key
            // rotated, the keystore not up yet — are otherwise one and the same
            // silence on screen. `Pandora.recall` names its own in `logcat`.
            log::info!("pandora: the session is remembered; asking the store for it");
            match crate::features::sync::remember::load() {
                Ok(Some(kept)) => {
                    log::info!("pandora: the store handed back a remembered session");
                    sync.restore(kept);
                }
                Ok(None) => log::warn!("pandora: the store holds no remembered session"),
                Err(e) => shell.fail(e),
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
        Self {
            screen,
            icons,
            shell,
            auth,
            boxes: Boxes::default(),
            sync,
            prefs,
            generate: GenerateForm::default(),
            preview: crate::features::drive::preview::Preview::default(),
            find: crate::features::find::model::Find::default(),
            drop_at: None,
            drop_highlight: None,
            lang_open: false,
            idle: Monitor::start(),
            bridge: crate::features::bridge::Bridge::start(cc.egui_ctx.clone()),
            #[cfg(not(target_os = "android"))]
            surface: eframe::SurfaceConfig::LOW_LATENCY,
            #[cfg(not(target_os = "android"))]
            resize_until: None,
            #[cfg(not(target_os = "android"))]
            last_size: egui::Vec2::ZERO,
            nav: Nav::default(),
        }
    }

    /// Lock every unlocked vault and land on the Unlock screen; unsaved,
    /// conflict-free edits are written first.
    pub fn lock(&mut self, ctx: &egui::Context) {
        self.boxes.flush_all(&mut self.shell);
        // The key is about to go and no other moment is coming, so a push the box
        // has not sent yet goes now. The worker keeps its own copy.
        if self
            .boxes
            .session
            .as_ref()
            .is_some_and(|s| s.push_at.is_some())
        {
            self.send(false);
        }
        self.shell.take_back_clip();
        self.shell.activity = None;
        self.boxes.session = None; // drops keys + vaults: Zeroizing fields wipe secrets
        self.boxes.tabs.clear();
        self.boxes.active = 0;
        self.preview = Default::default();
        self.auth.unlock_password.zeroize();
        self.generate.out.zeroize();
        self.auth.recent_search.clear();
        self.auth.op = None;
        self.auth.login = None;
        self.lang_open = false;
        // The repo token stays, and so does a push in flight: the token reads
        // nothing without the master password. Sign out clears it.
        // Every vault is gone, so no place behind this one can be returned to.
        self.nav = Nav::default();
        self.screen = self.auth.landing();
        // Dropping the sessions wipes the buffers the app owns, but egui keeps its
        // own copy of every field — the master password included, in the undo
        // snapshot behind it. That copy is only reachable by clearing the lot.
        ctx.data_mut(|d| d.clear());
    }

    fn watch_lock(&mut self, ctx: &egui::Context) {
        self.shell.focused = ctx.input(|i| i.focused);
        if ctx.input(|i| !i.events.is_empty()) {
            self.shell.activity = Some(std::time::Instant::now());
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
        self.shell
            .set_status(format!("box locked: {reason}"), StatusKind::Info);
    }

    /// Where the user is, coarsely: the screen, the box on it, the pane and the
    /// folder inside it, and the sources view. Any change is one step back.
    fn place(&self) -> Place {
        let s = self.boxes.session.as_ref();
        Place {
            screen: self.screen,
            // None and `Recents` show the same view: one spelling, so picking
            // the default view is not a step.
            side: self.auth.side.or(Some(Side::Recents)),
            vault: s.map(|s| s.path.clone()),
            tab: s.map_or(Tab::Passwords, |s| s.tab),
            folder: s.and_then(|s| s.folder.clone()),
            view: s.map_or(View::List, |s| s.view),
            axis: s.map_or(Axis::Target, |s| s.axis),
            at: s.map_or_else(Vec::new, |s| s.at.clone()),
        }
    }

    /// Push the place just left when the current one changes. A place whose box
    /// is no longer open is dropped: there would be nowhere to return. Called
    /// once a frame, after the screens, so it catches every way of navigating.
    pub(crate) fn record_place(&mut self) {
        let now = self.place();
        let Some(old) = self.nav.at.clone() else {
            self.nav.at = Some(now);
            return;
        };
        if old == now {
            return;
        }
        // A scan lasts as long as its screen: leaving it — by the bar's back, by
        // the platform's — is what puts the camera away.
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
        self.lang_open
            || self.find.open
            || overlay
            || self.sync.adding
            || self.sync.new_in.is_some()
            || !self.nav.past.is_empty()
    }

    /// Close the topmost layer: the language picker, the QR chooser, the entry's
    /// generator, the viewer, the delete prompt, the new-entry chooser, the folder
    /// editor or the open entry; then the sources' new-box form and repo picker.
    /// Overlays are not places.
    fn close_overlay(&mut self) -> bool {
        if std::mem::take(&mut self.find.open) {
            self.find.query.clear();
            return true;
        }
        if std::mem::take(&mut self.lang_open) {
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

    /// Return to a place: switch box if needed, then set screen, view, pane and
    /// folder. Overlays stay closed.
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
        // Before anything draws: a character typed on a phone's keyboard arrives
        // here, and it has to land on the widget that has focus.
        #[cfg(target_os = "android")]
        crate::shared::android::feed_keyboard(&ctx);
        self.pump();
        self.pump_bridge(&ctx);
        self.pump_sync(&ctx);
        // A preview belongs to the entry it was opened for: closing the modal,
        // changing pane or locking the box puts the player and the PDF away.
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

        // While files are being dragged over the window, keep the frame loop
        // alive so the overlay and the hovered folder stay painted. The drag is
        // announced as a hover until it is released as a drop.
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
            self.boxes.tick(&mut self.shell);
        }
        // The bar runs everywhere: a phone has no window to drag or close, but it
        // keeps Back and the tool. Only a desktop has edges to resize, and they
        // ride the same layer as the bar so a modal cannot cover them.
        crate::features::shell::windowbar::show(ui, self);
        self.shell.draw(ui);
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
        // The picker rides every screen: its globe is on the bar.
        crate::features::settings::ui::modal(&ctx, self);
        // The trash rides the settings card that opens it, and draws outside the
        // pane so a restore can take the user to the record it put back.
        crate::features::settings::ui::trash_modal(&ctx, self);
        // The Ctrl+K palette is an overlay like any other: it searches the box on
        // screen and takes the user to what it finds.
        crate::features::find::ui::modal(&ctx, self);
        // Last, so it sits over whatever screen is behind it: a browser asking to be
        // let in can arrive on any of them.
        crate::features::bridge::ui::modal(&ctx, self);

        // While files are being dragged over the window, paint a drop zone over
        // everything so it is obvious where they land, and outline the folder the
        // pointer is hovering. The whole viewport is covered, bar included, so the
        // window's outline stays unbroken.
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
        // A resize is a short burst of frames: give the swapchain two frames of
        // room for its duration, then go back to one so the window's edge does
        // not trail the pointer in the steady state. It is a wgpu knob, so the
        // phone — drawing through glow — has nothing to do here.
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
                _frame.set_wgpu_surface_config(wanted);
                self.surface = wanted;
            }
        }
        self.run_ui(ui);
    }

    /// Quitting is the one exit the clip countdown never gets: without this a
    /// password copied a minute ago outlives the app on the system clipboard.
    #[cfg(not(target_os = "android"))]
    fn on_exit(&mut self) {
        self.shell.take_back_clip();
    }

    /// The same, for the phone: glow's exit hook hands over the GL context, and
    /// nothing here wants it.
    #[cfg(target_os = "android")]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shell.take_back_clip();
    }
}

/// Test fixtures: a bare app, and a throwaway unlocked vault.
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
            prefs: Prefs::default(),
            generate: GenerateForm::default(),
            preview: crate::features::drive::preview::Preview::default(),
            find: crate::features::find::model::Find::default(),
            drop_at: None,
            drop_highlight: None,
            lang_open: false,
            idle: Monitor::default(),
            bridge: crate::features::bridge::Bridge::default(),
            #[cfg(not(target_os = "android"))]
            surface: eframe::SurfaceConfig::LOW_LATENCY,
            #[cfg(not(target_os = "android"))]
            resize_until: None,
            #[cfg(not(target_os = "android"))]
            last_size: egui::Vec2::ZERO,
            nav: Nav::default(),
        }
    }

    pub fn open(&mut self, name: &str) {
        let path = std::path::PathBuf::from(format!("/tmp/{name}.vault"));
        let creds = store::Credentials::rekey("test", None).unwrap();
        self.enter(creds, crate::vault::model::Vault::new(name), path);
    }
}
