use crate::app::{App, Op, OpKind, OpOutcome, Screen, Session, StatusKind};
use crate::features::auth::biometric;
use crate::features::auth::model::Auth;
use crate::features::boxes::model::Boxes;
use crate::features::settings::model as prefs;
use crate::features::shell::model::Shell;
use crate::shared::i18n::{t, tf};
use crate::vault::crypto::{self, MIN_PASSWORD_LEN};
use crate::vault::model::Vault;
use crate::vault::store::{self, Credentials};
use crate::vault::Failure;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use zeroize::{Zeroize, Zeroizing};

impl App {
    /// A file already open is brought forward: two sessions on one file would fight over the disk copy.
    pub(crate) fn enter(&mut self, creds: Credentials, vault: Vault, path: PathBuf) {
        if let Some(i) = self.boxes.position(&path) {
            self.switch(i);
            self.shell.fail("that box is already open");
            return;
        }
        // The box on screen is about to be parked behind the new one, where no
        // tick reaches it any more: its edits go to disk first, and only a clean
        // one gives up its decrypted payload.
        self.boxes.flush(&mut self.shell);
        if let Some(mut current) = self.boxes.session.take() {
            current.park();
            self.boxes
                .tabs
                .insert(self.boxes.active.min(self.boxes.tabs.len()), current);
        }
        self.auth.unlock_path = path.display().to_string();
        self.boxes.active = self.boxes.tabs.len();
        let last_mtime = store::modified_time(&path);
        // The repo file is named after the box's own file name, so a clone-landed box just works.
        let key = prefs::box_key(&path.display().to_string());
        let remote = self.prefs.remotes.get(&key).cloned();
        // The box as it is now is the commit baseline: the first push of a run
        // carries the same fixed subject as every other.
        let baseline = Some(crate::features::sync::github::Fingerprint::of(&vault));
        self.boxes.session = Some({
            let mut session = Session::fresh(path, creds, vault);
            session.last_mtime = last_mtime;
            session.remote = remote;
            session.baseline = baseline;
            session.view = self.prefs.view;
            session.axis = self.prefs.axis;
            session
        });
        self.shell.activity = Some(std::time::Instant::now());
        // A box just opened is looked at once, even when it was the last one on
        // screen before it was locked.
        self.sync.folder_seen = None;
        self.sync.folder_resume = false;
        self.screen = Screen::Main;
    }

    pub fn unlock(&mut self) {
        if self.auth.op.is_some() {
            return;
        }
        let path_str = self.auth.unlock_path.trim().to_owned();
        if path_str.is_empty() {
            self.shell.fail("enter the box path");
            return;
        }
        let key = prefs::box_key(&path_str);
        let wait = self.prefs.wait(&key, prefs::now());
        if wait > 0 {
            self.shell
                .fail(tf("too many attempts — wait {wait} s", &[("wait", &wait)]));
            return;
        }
        let path = PathBuf::from(&path_str);
        // Guard before the (seconds-long) derivation: never two sessions on one file.
        if let Some(i) = self.boxes.position(&path) {
            self.switch(i);
            self.shell.fail("that box is already open");
            return;
        }
        let mut pwd = self.auth.unlock_password.clone();
        let mut code = Zeroizing::new(std::mem::take(&mut self.auth.unlock_code));
        let keyfile_path = self.auth.unlock_keyfile.trim().to_owned();
        self.auth.task(OpKind::Unlock, move || {
            let res = (|| -> Result<OpOutcome, String> {
                // A printed code is the other door: it wraps the key the password
                // would have derived, and it is only asked for when one was typed.
                if !code.trim().is_empty() {
                    let (c, v) = store::open_with_code(&path, code.trim())?;
                    return Ok(OpOutcome::Entered(path.clone(), c, v));
                }
                let keyfile = open_keyfile(&keyfile_path)?;
                let kf = keyfile.as_ref().map(|k| k.as_slice());
                // A box enrolled with a key asks for it here; a box without one never
                // touches the key, even when it is plugged in.
                let key = crate::shared::yubikey::Usb;
                let factor =
                    crate::shared::yubikey::present().then_some(&key as &dyn store::Factor);
                let (c, v) = store::open_with(&path, &pwd, kf, factor)?;
                Ok(OpOutcome::Entered(path.clone(), c, v))
            })();
            pwd.zeroize();
            code.zeroize();
            res
        });
    }

    /// The agent follows the box: started when the switch is on, handed the keys the
    /// user allowed, emptied when the box locks or closes.
    fn pump_agent(&mut self) {
        let wanted = self
            .prefs
            .ssh_agent
            .then(|| crate::features::agent::socket_path(&self.prefs.ssh_socket));
        let vault = self.boxes.session.as_ref().map(|s| &s.vault);
        self.agent.poll(wanted.as_deref(), vault);
    }

    pub fn pump(&mut self) {
        self.pump_agent();
        let Some(op) = self.auth.op.take() else {
            return;
        };
        let res = match op.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => {
                self.auth.op = Some(op);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.shell.fail("operation interrupted");
                return;
            }
        };
        self.auth.unlock_password.zeroize();
        match res {
            Ok(OpOutcome::Kept) => {
                self.shell.set_status(
                    "this box now opens with your fingerprint",
                    StatusKind::Saved,
                );
            }
            Ok(OpOutcome::Entered(path, creds, vault)) => {
                if op.kind == OpKind::Unlock
                    && self.prefs.opened(&prefs::box_key(&self.auth.unlock_path))
                {
                    prefs::save(&self.prefs);
                }
                self.auth.recents = crate::features::auth::model::remember(&path);
                // A password that just worked is the moment to keep the key for
                // the device's reader, before it moves into the session.
                if op.kind != OpKind::Biometric {
                    self.keep_for_reader(&path, creds.key.clone());
                }
                let source = (op.kind == OpKind::Create)
                    .then(|| self.source_of(&path))
                    .flatten();
                self.enter(creds, vault, path.clone());
                if let Some(source) = source {
                    self.mirror(&source, &path);
                    self.send(false);
                }
                self.shell.set_status(
                    if op.kind == OpKind::Create {
                        "box created"
                    } else {
                        "unlocked"
                    },
                    StatusKind::Saved,
                );
            }
            Ok(OpOutcome::PasswordChanged(path, creds, erase_remote)) => {
                // The vault may have left the screen while the derivation ran: only the file it started on takes the new key.
                if !self.boxes.session.as_ref().is_some_and(|s| s.path == path) {
                    self.shell
                        .fail("that box left the screen — password unchanged");
                } else if self.boxes.session.as_ref().is_some_and(|s| s.conflict) {
                    // Rewriting the file with a new key while the disk copy is
                    // someone else's would leave that copy unopenable by either
                    // password. The banner comes first.
                    self.shell
                        .fail("settle the disk conflict before changing the password");
                } else {
                    // The old seal no longer opens this box: it is dropped, and the
                    // key the new password derived takes its place.
                    biometric::forget(&path);
                    self.keep_for_reader(&path, creds.key.clone());
                    self.boxes.session.as_mut().unwrap().creds = creds;
                    // Deliberate and verified against the file just now: the write
                    // goes through even if a stamp moved under it.
                    if self.boxes.overwrite(&mut self.shell) {
                        // The copy beside the box is sealed with the key this
                        // password just replaced, so it goes with it: what it held
                        // the old password would still open.
                        store::forget_stash(&path);
                        // And the repo holds a version of the same box, one that
                        // the old password still opens. Asked for on the form,
                        // because taking it out of the repo is the user's call —
                        // and answered once: the box it was asked about is gone.
                        self.auth.change.erase_remote = false;
                        if erase_remote {
                            self.erase_remote();
                        }
                        self.shell
                            .set_status("master password updated", StatusKind::Saved);
                    }
                }
            }
            Ok(OpOutcome::KeyChanged(path, creds, enrolled)) => {
                // Same guards as a password change: the derivation ran on a box that
                // may have left the screen, and rewriting the file under a conflict
                // would leave the other copy unopenable.
                if !self.boxes.session.as_ref().is_some_and(|s| s.path == path) {
                    self.shell
                        .fail("that box left the screen — the key is unchanged");
                } else if self.boxes.session.as_ref().is_some_and(|s| s.conflict) {
                    self.shell
                        .fail("settle the disk conflict before changing the key");
                } else {
                    // The derived key moved, so the seal made from the old one goes.
                    biometric::forget(&path);
                    self.keep_for_reader(&path, creds.key.clone());
                    self.boxes.session.as_mut().unwrap().creds = creds;
                    if self.boxes.overwrite(&mut self.shell) {
                        // What the copy beside the box holds is sealed with the key
                        // this change replaced.
                        store::forget_stash(&path);
                        self.shell.set_status(
                            if enrolled {
                                "the key is now part of this box — without it the box cannot be opened"
                            } else {
                                "this box no longer asks for a key"
                            },
                            StatusKind::Saved,
                        );
                    }
                }
            }
            Ok(OpOutcome::Picked(path)) => {
                self.auth.unlock_path = path.display().to_string();
                self.shell.set_status(
                    "box file imported — enter its master password",
                    StatusKind::Info,
                );
            }
            Ok(OpOutcome::Destination(uri)) => {
                self.auth.welcome.path = uri;
                self.shell
                    .set_status("destination chosen — create the box", StatusKind::Info);
            }
            // Backing out of a prompt is a choice, not a failure: it leaves the
            // seal exactly as it was.
            Err(e) if e == biometric::CANCELED => {}
            Err(e) => self.failed_unlock(op.kind, e),
        }
    }

    /// Keep this box's key for the device's own reader, when the user asked for it
    /// and the device has one. Off the frame, and not a file write: the keystore
    /// will not let that key wrap anything without a live finger, so this is a
    /// prompt and the screen keeps painting while it is up. Best effort — a
    /// keystore that refuses costs the feature, never the unlock that just
    /// happened. A box already kept is left alone: sealing it again would ask for
    /// a finger on every unlock.
    fn keep_for_reader(&mut self, path: &Path, key: crypto::Key) {
        if !self.prefs.biometric || !biometric::available() || biometric::sealed(path) {
            return;
        }
        if self.auth.op.is_some() {
            return; // one keystore operation at a time
        }
        let path = path.to_path_buf();
        self.auth
            .task(OpKind::Seal, move || match biometric::seal(&path, &key) {
                Ok(()) => Ok(OpOutcome::Kept),
                Err(e) => Err(e),
            });
    }

    /// Open a box with the device's reader instead of the master password. It runs
    /// on the same worker as the password path, so the frame keeps painting while
    /// the prompt is up; the key it frees opens the file exactly as the password
    /// would have, and a box whose password was rotated falls back to the form.
    pub fn unlock_biometric(&mut self) {
        if self.auth.op.is_some() {
            return;
        }
        let path_str = self.auth.unlock_path.trim().to_owned();
        if path_str.is_empty() {
            self.shell.fail("enter the box path");
            return;
        }
        let path = PathBuf::from(&path_str);
        if let Some(i) = self.boxes.position(&path) {
            self.switch(i);
            self.shell.fail("that box is already open");
            return;
        }
        self.auth.task(OpKind::Biometric, move || {
            let key = match biometric::unseal(&path) {
                Ok(key) => key,
                // A prompt that was backed out of leaves the seal alone. A key the
                // keystore no longer has — a re-enrolled finger took it — has
                // nothing left to keep: dropping it here is what lets the next
                // password unlock seal the box again.
                Err(e) => {
                    if e != biometric::CANCELED {
                        biometric::forget(&path);
                    }
                    return Err(e);
                }
            };
            let (creds, vault) = store::open_keyed(&path, key)?;
            Ok(OpOutcome::Entered(path.clone(), creds, vault))
        });
    }

    /// Choose where a new box will live, through the system's own picker (Android).
    #[cfg(target_os = "android")]
    pub fn create_box_destination(&mut self) {
        if self.auth.op.is_some() {
            return;
        }
        self.auth.task(
            OpKind::Pick,
            || match crate::shared::android::document_create(
                "application/octet-stream",
                "vault.pandora",
            ) {
                Ok(Some(uri)) => Ok(OpOutcome::Destination(uri)),
                Ok(None) => Err("no destination chosen".to_owned()),
                Err(e) => Err(e),
            },
        );
    }

    /// Bring a box file in from the phone's own storage. Android's picker belongs
    /// to the activity, not to a window, so it cannot run inside a frame.
    #[cfg(target_os = "android")]
    pub fn import_box(&mut self) {
        if self.auth.op.is_some() {
            return;
        }
        self.auth.task(OpKind::Pick, || {
            crate::features::auth::pick::import().map(OpOutcome::Picked)
        });
    }

    /// The user just asked for the reader: keep the open box's key now, so the
    /// next way in is a finger rather than the password they just typed.
    pub fn keep_key(&mut self) {
        let Some(session) = self.boxes.session.as_ref() else {
            return;
        };
        let path = session.path.clone();
        let key = session.creds.key.clone();
        self.keep_for_reader(&path, key);
    }

    /// The user turned it off: the device's reader forgets the open box.
    pub fn drop_key(&mut self) {
        if let Some(path) = self.boxes.session.as_ref().map(|s| s.path.clone()) {
            biometric::forget(&path);
        }
    }

    /// Only a wrong password is a guess and costs an attempt; an unreadable box
    /// (a wrong path, a missing keyfile) does not. A missing keyfile now answers
    /// with the same `Failure::Wrong` a bad password does — the box stopped saying
    /// which one it needs — so it costs an attempt too, and the form says so by
    /// offering the keyfile field on every unlock.
    fn failed_unlock(&mut self, kind: OpKind, err: String) {
        if kind != OpKind::Unlock || Failure::of(&err) != Some(Failure::Wrong) {
            self.shell.fail(err);
            return;
        }
        let key = prefs::box_key(&self.auth.unlock_path);
        let wait = self.prefs.failed(&key, prefs::now());
        prefs::save(&self.prefs);
        let max = self.prefs.max_attempts;
        let fails = self.prefs.tries.get(&key).map_or(0, |t| t.fails);
        let err = t(Failure::Wrong.text()).to_owned();
        self.shell.fail(match (max, fails) {
            (0, _) => err,
            (_, 0) => tf("too many attempts — wait {wait} s", &[("wait", &wait)]),
            (m, f) => tf(
                "{err} — {left} of {max} attempts left",
                &[("err", &err), ("left", &(m - f)), ("max", &m)],
            ),
        });
    }
}

impl Auth {
    /// The first screen: the sources, which is where the recents, the local
    /// files and the GitHub repos all meet. It is the way in whether or not a
    /// box was opened before — the create form and the unlock form are reached
    /// from it, never the other way round.
    pub(crate) fn landing(&self) -> Screen {
        Screen::Sources
    }

    /// Key derivation runs on a worker thread.
    pub fn create_vault(&mut self, shell: &mut Shell) {
        if self.op.is_some() {
            return;
        }
        let mut form = mem::take(&mut self.welcome);
        let mut password = mem::take(&mut form.password);
        let confirm = mem::take(&mut form.confirm);
        let err = if form.name.trim().is_empty() {
            Some("give your box a name".into())
        } else if form.path.trim().is_empty() {
            Some("choose a file path for the box".into())
        } else if password.len() < MIN_PASSWORD_LEN {
            Some(tf(
                "master password must be at least {min} characters",
                &[("min", &MIN_PASSWORD_LEN)],
            ))
        } else if password != confirm {
            Some("passwords don't match".into())
        } else {
            None
        };
        if let Some(error) = err {
            shell.fail(error);
            password.zeroize();
            let mut confirm = confirm;
            confirm.zeroize();
            self.welcome = form;
            return;
        }
        let path = PathBuf::from(form.path.trim());
        let name = form.name.trim().to_owned();
        let keyfile_path = form.keyfile.trim().to_owned();
        let mut confirm = confirm;
        self.task(OpKind::Create, move || {
            let result = (|| -> Result<OpOutcome, String> {
                let keyfile = open_keyfile(&keyfile_path)?;
                let keyfile = keyfile.as_ref().map(|key| key.as_slice());
                let (creds, vault) = store::create(&path, &name, &password, keyfile)?;
                Ok(OpOutcome::Entered(path.clone(), creds, vault))
            })();
            password.zeroize();
            confirm.zeroize();
            result
        });
        self.welcome = form;
    }

    pub fn change_password(&mut self, boxes: &Boxes) {
        if self.op.is_some() {
            return;
        }
        let Some(session) = boxes.session.as_ref() else {
            return;
        };
        let path = session.path.clone();
        let mut form = mem::take(&mut self.change);
        let mut current = mem::take(&mut form.current);
        let mut new = mem::take(&mut form.new);
        let mut confirm = mem::take(&mut form.confirm);
        let keyfile_path = form.keyfile.trim().to_owned();
        let erase_remote = form.erase_remote;
        self.task(OpKind::ChangePassword, move || {
            let result = (|| -> Result<OpOutcome, String> {
                if current.is_empty() {
                    return Err("enter your current master password".into());
                }
                if new.len() < MIN_PASSWORD_LEN {
                    return Err(tf(
                        "new password must be at least {min} characters",
                        &[("min", &MIN_PASSWORD_LEN)],
                    ));
                }
                if new != confirm {
                    return Err("new passwords don't match".into());
                }
                let keyfile = open_keyfile(&keyfile_path)?;
                let keyfile = keyfile.as_ref().map(|key| key.as_slice());
                // Opening the box with the current password is the check, and it
                // is the whole check: `open` tries the password with the keyfile
                // and then alone, so this works whether or not this session ever
                // knew which one the box uses — a finger-unlocked one does not.
                let (opened, _) = store::open(&path, &current, keyfile).map_err(|_| {
                    "current master password is incorrect, this box needs its keyfile, or its file changed"
                        .to_owned()
                })?;
                // The second factor survives because it was needed to get here;
                // it is never added by a keyfile this box did not ask for.
                let keeps = opened.needs_keyfile().unwrap_or(false);
                let creds = Credentials::rekey(&new, keyfile.filter(|_| keeps))?;
                Ok(OpOutcome::PasswordChanged(path, creds, erase_remote))
            })();
            current.zeroize();
            new.zeroize();
            confirm.zeroize();
            result
        });
        self.change = form;
    }

    /// Enrol the YubiKey into the box on screen, or drop it from it. The password is
    /// asked again because the box's key is derived from it; the touch is the key's
    /// only proof that it is here. The worker does all of it: the derivation and the
    /// USB exchange both take long enough to freeze a frame.
    pub fn enrol_key(&mut self, boxes: &Boxes, enrol: bool) {
        if self.op.is_some() {
            return;
        }
        let Some(path) = boxes.session.as_ref().map(|s| s.path.clone()) else {
            return;
        };
        let mut password = mem::take(&mut self.key.password);
        let keyfile_path = self.key.keyfile.trim().to_owned();
        self.task(OpKind::EnrolKey, move || {
            let result = (|| -> Result<OpOutcome, String> {
                if password.is_empty() {
                    return Err(t("enter the master password").to_owned());
                }
                let keyfile = open_keyfile(&keyfile_path)?;
                let keyfile = keyfile.as_ref().map(|key| key.as_slice());

                // One touch, one answer: the open below and the new material use the
                // same response, so a key waiting for a finger is asked once.
                let answer = crate::shared::yubikey::respond(&store::challenge(&path)?)?;
                // Every combination the box could have been written with, tried with
                // that answer. Succeeding is the proof the password is right.
                let (opened, _) = store::open_with(&path, &password, keyfile, Some(&Answered(&answer)))
                    .map_err(|_| {
                        "the master password is incorrect, this box needs its keyfile or its key, or its file changed"
                            .to_owned()
                    })?;
                let keeps = opened.needs_keyfile().unwrap_or(false);
                let creds = Credentials::rekey_with(
                    &password,
                    keyfile.filter(|_| keeps),
                    enrol.then_some(answer.as_slice()),
                )?;
                Ok(OpOutcome::KeyChanged(path, creds, enrol))
            })();
            password.zeroize();
            result
        });
    }

    fn task(
        &mut self,
        kind: OpKind,
        job: impl FnOnce() -> Result<OpOutcome, String> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(job());
        });
        self.op = Some(Op { kind, rx });
    }
}

/// The answer the key already gave, so a derivation does not touch it a second time.
struct Answered<'a>(&'a [u8]);

impl store::Factor for Answered<'_> {
    fn respond(&self, _challenge: &[u8]) -> Result<Vec<u8>, String> {
        Ok(self.0.to_vec())
    }
}

/// Keyfile bytes for a non-empty path — read off the UI thread, so a slow mount cannot freeze it.
fn open_keyfile(path: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    if path.is_empty() {
        return Ok(None);
    }
    store::read_keyfile(Path::new(path)).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A password change takes the copy beside the box with it.
    ///
    /// The write the change makes stashes the version it displaces — which is the
    /// one sealed with the key being replaced — so what is left on the machine
    /// would keep the old master password working for as long as the file is
    /// there. This is the handler that owns the change, driven the way the frame
    /// loop drives it.
    #[test]
    fn a_rotated_password_leaves_no_old_ciphertext() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("rotate");

        let path = app.boxes.session.as_ref().unwrap().path.clone();
        let backup = store::stash_path(&path);
        let old_key = {
            let s = app.boxes.session.as_mut().unwrap();
            // Two writes: the second one leaves the first in the stash.
            store::save(&s.path, &mut s.vault, &s.creds).unwrap();
            s.vault.logins.push(crate::vault::model::Login::new(1));
            store::save(&s.path, &mut s.vault, &s.creds).unwrap();
            s.creds.key.clone()
        };
        assert!(backup.is_file(), "the write before the change kept one");
        assert!(
            store::verify(&backup, &old_key),
            "and the old password opens it — that is the leak this closes"
        );

        // The rotation lands, exactly as the worker would hand it over.
        let rotated = Credentials::rekey("a-new-master-password", None).unwrap();
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(OpOutcome::PasswordChanged(path.clone(), rotated, false)))
            .unwrap();
        app.auth.op = Some(crate::app::Op {
            kind: OpKind::ChangePassword,
            rx,
        });
        app.pump();

        assert!(
            !backup.exists(),
            "the copy sealed with the old key is gone from the machine"
        );
        assert!(
            store::open(&path, "a-new-master-password", None).is_ok(),
            "the box opens with the password it now has"
        );
        assert!(
            store::open(&path, "test", None).is_err(),
            "and not with the one that was replaced"
        );

        let _ = std::fs::remove_file(&path);
    }
}
