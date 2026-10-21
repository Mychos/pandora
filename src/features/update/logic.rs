//! The workers behind the notice: one check, one download, and what a frame does
//! with what they send back.

use super::model::{Event, State, Update};
use crate::app::App;
use crate::shared::i18n::{t, tf};
use egui::Context;
use std::sync::mpsc::TryRecvError;

// The worker half: compiled out of a test build, which reaches `pump` directly.
#[cfg(not(test))]
use super::model::channel;
#[cfg(not(test))]
use super::{github, install};
#[cfg(not(test))]
use std::sync::mpsc::Sender;

impl Update {
    /// Look for a release newer than this build. One worker at a time: a check that
    /// is already out is the answer the next one would give.
    pub fn check(&mut self, ctx: &Context) {
        if self.busy() {
            return;
        }
        // Compiled out of a test build, like `sync::logic::browse`: the suite drives
        // this state machine through `pump`, and a worker here would reach GitHub
        // from a test that had no intention of asking.
        #[cfg(not(test))]
        {
            self.state = State::Checking;
            let tx = self.open();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let event = match github::check() {
                    Ok(found) => Event::Checked(found),
                    Err(e) => Event::Failed(e),
                };
                let _ = tx.send(event);
                ctx.request_repaint();
            });
        }
        #[cfg(test)]
        let _ = ctx;
    }

    /// Download, verify and swap in the release the notice is about. `App` has
    /// already written the open box and found nothing unsaved, so nothing here can
    /// lose work — and nothing here runs without that click.
    pub fn install(&mut self, ctx: &Context) {
        if self.busy() || self.info.is_none() {
            return;
        }
        #[cfg(not(test))]
        {
            self.state = State::Downloading(Default::default());
            let tx = self.open();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let event = match fetch(&tx, &ctx) {
                    Ok(restart) => Event::Installed { restart },
                    Err(e) => Event::Failed(e),
                };
                let _ = tx.send(event);
                ctx.request_repaint();
            });
        }
        #[cfg(test)]
        let _ = ctx;
    }

    /// Ask again when the window comes back: a release published while the app sat
    /// in the background is exactly the one the user has not been told about.
    pub fn watch_focus(&mut self, focused: bool, ctx: &Context, enabled: bool) {
        if enabled && self.regain(focused) {
            self.check(ctx);
        }
    }

    /// Whether this frame is the window coming back. Kept apart from the check so
    /// the rule can be tested without a worker or a network.
    pub(super) fn regain(&mut self, focused: bool) -> bool {
        let regained = self.focused == Some(false) && focused;
        self.focused = Some(focused);
        regained
    }

    /// Land what the worker sent. A failure that happened while installing is
    /// handed back to be said out loud; one that happened while checking is not,
    /// because an offline machine is not an error the user can act on.
    pub fn pump(&mut self, ctx: &Context) -> Option<String> {
        let events: Vec<Event> = self.rx.as_ref()?.try_iter().collect();
        let finished = matches!(
            self.rx.as_ref().map(|rx| rx.try_recv()),
            Some(Err(TryRecvError::Disconnected))
        );

        let mut report = None;
        for event in events {
            match event {
                Event::Checked(found) => {
                    self.info = found;
                    self.state = State::Idle;
                    self.checked = true;
                }
                Event::Progress(progress) => self.state = State::Downloading(progress),
                Event::Installed { restart } => {
                    self.info = None;
                    self.state = State::Idle;
                    if restart {
                        // The new build is already running from this build's own
                        // file: this process has to let go of the window.
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Event::Failed(why) => {
                    let installing = matches!(self.state, State::Downloading(_));
                    self.state = State::Failed(why.clone());
                    if installing {
                        report = Some(why);
                    }
                }
            }
        }
        if finished {
            self.rx = None;
        }
        report
    }
}

/// The download half of an install, on a worker thread with nothing borrowed from
/// the app.
#[cfg(not(test))]
fn fetch(tx: &Sender<Event>, ctx: &Context) -> Result<bool, String> {
    let releases = github::releases()?;
    let release = github::pick(&releases).ok_or("no release is published")?;
    let asset = github::asset(channel(), release)
        .ok_or("the newest release has no file this build can install")?;
    let sums = github::sums(release)?;
    let dir = install::staging_dir()?;
    let staged = github::download(&asset.browser_download_url, &dir, |progress| {
        // A frame that is behind will draw a fresher percentage than this one, so
        // a dropped progress message costs nothing and an unbounded queue would.
        let _ = tx.send(Event::Progress(progress));
        ctx.request_repaint();
    })?;
    if let Err(e) = github::verify(&staged, &asset.name, &sums) {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }
    install::install(&staged)
}

impl App {
    /// The updater's frame: land what the worker sent, and ask again on the way in.
    pub fn pump_update(&mut self, ctx: &Context) {
        if let Some(why) = self.update.pump(ctx) {
            self.shell.fail(tf(
                "the update could not be installed: {error}",
                &[("error", &why)],
            ));
        }
        let focused = self.shell.focused;
        self.update
            .watch_focus(focused, ctx, self.prefs.update_check);
    }

    /// Install the release the notice is about — the click behind "Update".
    /// Whatever is only in memory goes to disk first: the restart this ends with
    /// must not be a rollback.
    pub fn install_update(&mut self, ctx: &Context) {
        self.boxes.flush_all(&mut self.shell);
        let unsaved = self
            .boxes
            .session
            .as_ref()
            .is_some_and(|s| s.conflict || s.dirty);
        if unsaved {
            self.shell
                .fail(t("save or settle the open box before updating"));
            return;
        }
        self.update.install(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::update::model::{Info, Progress};

    fn checked(update: &mut Update, info: Option<Info>) -> Option<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        update.rx = Some(rx);
        tx.send(Event::Checked(info)).unwrap();
        drop(tx);
        update.pump(&Context::default())
    }

    #[test]
    fn a_check_that_finds_a_release_keeps_it() {
        let mut update = Update::default();
        let info = Info {
            latest: "0.2.0".to_owned(),
            url: "https://example.invalid".to_owned(),
            installable: true,
        };
        assert_eq!(checked(&mut update, Some(info.clone())), None);
        assert_eq!(update.info, Some(info));
        assert_eq!(update.state, State::Idle);
        assert!(update.checked, "the row needs an answer, not a wait");
        assert!(!update.busy(), "a finished worker releases the updater");
    }

    #[test]
    fn progress_reaches_the_frame_and_does_not_end_the_worker() {
        let mut update = Update::default();
        let (tx, rx) = std::sync::mpsc::channel();
        update.rx = Some(rx);
        tx.send(Event::Progress(Progress {
            written: 5,
            total: Some(10),
        }))
        .unwrap();
        assert_eq!(update.pump(&Context::default()), None);
        assert_eq!(
            update.state,
            State::Downloading(Progress {
                written: 5,
                total: Some(10)
            })
        );
        assert!(update.busy(), "the download is still running");
    }

    #[test]
    fn a_failed_install_is_reported_and_a_failed_check_is_not() {
        let mut update = Update::default();
        let (tx, rx) = std::sync::mpsc::channel();
        update.rx = Some(rx);
        tx.send(Event::Failed("no network".to_owned())).unwrap();
        assert_eq!(
            update.pump(&Context::default()),
            None,
            "a check nobody answered is not the user's to solve"
        );
        assert!(matches!(update.state, State::Failed(_)));

        let mut update = Update {
            state: State::Downloading(Default::default()),
            ..Default::default()
        };
        let (tx, rx) = std::sync::mpsc::channel();
        update.rx = Some(rx);
        tx.send(Event::Failed("hash mismatch".to_owned())).unwrap();
        assert_eq!(
            update.pump(&Context::default()),
            Some("hash mismatch".to_owned()),
            "an install that failed happened because the user asked for it"
        );
    }

    #[test]
    fn only_the_window_coming_back_is_a_moment_to_look() {
        let mut update = Update {
            focused: Some(false),
            ..Default::default()
        };
        assert!(!update.regain(false));
        assert!(update.regain(true), "coming back is a moment to look");
        assert!(!update.regain(true), "still focused is not coming back");
        assert!(!update.regain(false));
    }

    #[test]
    fn the_first_frame_is_not_a_window_coming_back() {
        let mut update = Update::default();
        assert!(
            !update.regain(true),
            "the startup look is `App::new`'s, not the first frame's"
        );
    }

    #[test]
    fn the_switch_off_means_no_check_at_all() {
        let ctx = Context::default();
        let mut update = Update {
            focused: Some(false),
            ..Default::default()
        };
        update.watch_focus(true, &ctx, false);
        assert!(!update.busy(), "the switch is off, so nothing went out");
        assert_eq!(update.state, State::Idle);
    }
}
