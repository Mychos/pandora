//! Settings screen: vault info, master password rotation, lock.

use crate::app::{App, Confirm, OpKind};
use crate::features::auth::biometric;
use crate::features::auth::ui as auth;
use crate::features::settings::model as prefs;
use crate::shared::i18n::{self, t, tf, Lang};
use crate::shared::theme;
use crate::shared::ui::balance;
use crate::shared::ui::size;
use crate::vault::model::Kind;
use crate::vault::store;
use egui::{Label, RichText, TextEdit};

const CARD: f32 = 20.0;

const ROW: f32 = 12.0;

const ACT: f32 = 16.0;

const HINT: f32 = 8.0;

pub fn run(ui: &mut egui::Ui, app: &mut App, compact: bool) {
    egui::CentralPanel::default()
        .frame(theme::pane(compact))
        .show(ui, |ui| {
            theme::scroll(
                ui,
                egui::ScrollArea::vertical().auto_shrink([false, false]),
                |ui| {
                    ui.add_space(auth::top(ui));
                    ui.vertical_centered(|ui| {
                        ui.set_width(ui.available_width().min(theme::COLUMN));

                        let facts = facts(app);
                        let keyfile = keyfile(app);
                        box_card(ui, app, &facts, keyfile, compact);
                        ui.add_space(CARD);
                        crate::features::sync::ui::folder(ui, app);
                        ui.add_space(CARD);
                        // A phone's card, drawn only when there is a reader to
                        // offer: on a desk it paints nothing and takes no gap.
                        if fingerprint(ui, app) {
                            ui.add_space(CARD);
                        }
                        master_card(ui, app, keyfile);
                        ui.add_space(CARD);
                        recovery_card(ui, app);
                        // A desk has a port for a key; a phone does not.
                        #[cfg(not(any(target_os = "android", target_os = "ios")))]
                        {
                            key_card(ui, app, keyfile);
                            ui.add_space(CARD);
                        }
                        lock_card(ui, app);
                        ui.add_space(CARD);
                        clipboard_card(ui, app);
                        ui.add_space(CARD);
                        attempts_card(ui, app);
                        ui.add_space(CARD);
                        trash_card(ui, app, facts.trashed, compact);
                        ui.add_space(CARD);
                        browser(ui, app);
                        ui.add_space(CARD);
                        agent_card(ui, app);
                        ui.add_space(CARD);
                        #[cfg(target_os = "android")]
                        {
                            autofill(ui, app);
                            ui.add_space(CARD);
                        }
                        footer(ui, app);
                    });
                },
            );
        });
}

/// What the box holds, read in one pass: the card edits the session while it holds it.
struct Facts {
    path: String,
    size: String,
    total: usize,
    pw: usize,
    notes: usize,
    files: usize,
    codes: usize,
    dirs: usize,
    trashed: usize,
}

fn facts(app: &App) -> Facts {
    let Some(s) = app.boxes.session.as_ref() else {
        return Facts {
            path: String::new(),
            size: String::new(),
            total: 0,
            pw: 0,
            notes: 0,
            files: 0,
            codes: 0,
            dirs: 0,
            trashed: 0,
        };
    };
    let len = std::fs::metadata(&s.path).map(|m| m.len()).unwrap_or(0);
    let count = |kind| s.vault.drive.iter().filter(|e| e.kind() == kind).count();
    Facts {
        path: s.path.display().to_string(),
        size: size::human(len as usize),
        total: s.vault.records().count(),
        pw: s.vault.logins.len(),
        notes: count(Kind::Note),
        files: count(Kind::File),
        codes: s.vault.otps.len(),
        dirs: s.vault.folders.len(),
        trashed: s.vault.trash.len(),
    }
}

/// `Some` when the KDF proved whether a keyfile is needed; `None` after a biometric unlock.
fn keyfile(app: &App) -> Option<bool> {
    app.boxes
        .session
        .as_ref()
        .and_then(|s| s.creds.needs_keyfile())
}

/// The box card: name, file, contents, and the CSV way in and out.
fn box_card(ui: &mut egui::Ui, app: &mut App, facts: &Facts, keyfile: Option<bool>, compact: bool) {
    card(ui, "", |ui| {
        if let Some(s) = app.boxes.session.as_mut() {
            if ui
                .add(
                    TextEdit::singleline(&mut s.vault.name)
                        .hint_text(t("box name"))
                        .desired_width(f32::INFINITY)
                        .margin(egui::Margin::symmetric(12, 9)),
                )
                .changed()
            {
                s.mark_dirty();
            }
            ui.add_space(ROW);
        }
        kv(ui, "file", &facts.path, compact);
        kv(ui, "size", &facts.size, compact);
        if let Some(required) = keyfile {
            kv(
                ui,
                "keyfile",
                t(if required { "required" } else { "none" }),
                compact,
            );
        }
        kv(
                            ui,
                            "entries",
                            &tf(
                                "{total}  ({pw} password · {notes} note · {files} file · {codes} code · {dirs} folder)",
                                &[
                                    ("total", &facts.total),
                                    ("pw", &facts.pw),
                                    ("notes", &facts.notes),
                                    ("files", &facts.files),
                                    ("codes", &facts.codes),
                                    ("dirs", &facts.dirs),
                                ],
                            ),
                            compact,
                        );
        ui.add_space(HINT);
        theme::hint(
            ui,
            "move this one file anywhere to sync — GDrive, Dropbox, USB",
        );
        ui.add_space(ACT);
        let mut export = false;
        let mut import = false;
        if compact {
            export =
                theme::secondary_button(ui, &app.icons, Some(theme::Icon::Download), "Export CSV")
                    .clicked();
            ui.add_space(HINT);
            import = theme::secondary_button(ui, &app.icons, Some(theme::Icon::Upload), "Import…")
                .clicked();
        } else {
            ui.columns(2, |cols| {
                export = theme::secondary_button(
                    &mut cols[0],
                    &app.icons,
                    Some(theme::Icon::Download),
                    "Export CSV",
                )
                .clicked();
                import = theme::secondary_button(
                    &mut cols[1],
                    &app.icons,
                    Some(theme::Icon::Upload),
                    "Import…",
                )
                .clicked();
            });
        }
        if export {
            if let Some(s) = app.boxes.session.as_mut() {
                s.confirm = Some(Confirm::Export { neutral: false });
            }
        }
        if import {
            crate::features::settings::import::pick(app);
        }
        ui.add_space(HINT);
        if theme::secondary_button(ui, &app.icons, Some(theme::Icon::Upload), "Open a share…")
            .clicked()
        {
            crate::features::share::ui::pick(app);
        }
        ui.add_space(HINT);
        theme::hint(
                            ui,
                            "unencrypted CSV, no attachments — delete the export when you are done. Import reads a CSV, a Bitwarden .json, a 1Password .1pux or a KeePass .xml; it never overwrites entries already in the box, and the file stays where it is",
                        );
        ui.add_space(ACT);
        // The copy `store::save` keeps beside the box, restored on purpose. Off
        // when there is none: a button that would do nothing is worse than none.
        let backup = app.boxes.session.as_ref().is_some_and(|s| {
            s.path
                .with_extension(format!("{}.bak", store::EXT))
                .is_file()
        });
        let mut restore = false;
        ui.add_enabled_ui(backup, |ui| {
            restore = theme::secondary_button(
                ui,
                &app.icons,
                Some(theme::Icon::RefreshCw),
                "Restore previous version",
            )
            .clicked();
        });
        if restore {
            if let Some(s) = app.boxes.session.as_mut() {
                s.confirm = Some(Confirm::RestoreVersion);
            }
        }
        ui.add_space(HINT);
        if app
            .icons
            .labeled(
                ui,
                theme::Icon::Trash,
                15.0,
                theme::palette().error,
                "Delete box file",
            )
            .clicked()
        {
            if let Some(s) = app.boxes.session.as_mut() {
                s.confirm = Some(Confirm::Box);
            }
        }
    });
}

/// What the trash holds: look at it, or erase it; nothing leaves until the second.
fn trash_card(ui: &mut egui::Ui, app: &mut App, trashed: usize, compact: bool) {
    card(ui, "Trash", |ui| {
        kv(
            ui,
            "deleted",
            &tf("{count} records", &[("count", &trashed)]),
            compact,
        );
        ui.add_space(HINT);
        theme::hint(
            ui,
            "a deleted record waits here until the trash is emptied — it is part of the box, and travels with it",
        );
        ui.add_space(ACT);
        let mut review = false;
        let mut empty = false;
        if compact {
            review = theme::secondary_button(
                ui,
                &app.icons,
                Some(theme::Icon::Trash),
                "Review the trash",
            )
            .clicked();
            if trashed > 0 {
                ui.add_space(HINT);
                empty = theme::secondary_button(
                    ui,
                    &app.icons,
                    Some(theme::Icon::Trash),
                    "Empty the trash",
                )
                .clicked();
            }
        } else {
            ui.columns(2, |cols| {
                review = theme::secondary_button(
                    &mut cols[0],
                    &app.icons,
                    Some(theme::Icon::Trash),
                    "Review the trash",
                )
                .clicked();
                empty = trashed > 0
                    && theme::secondary_button(
                        &mut cols[1],
                        &app.icons,
                        Some(theme::Icon::Trash),
                        "Empty the trash",
                    )
                    .clicked();
            });
        }
        if review {
            if let Some(s) = app.boxes.session.as_mut() {
                s.show_trash = true;
            }
        }
        if empty {
            if let Some(s) = app.boxes.session.as_mut() {
                s.confirm = Some(Confirm::EmptyTrash);
            }
        }
    });
}

/// The trash's window, outside the settings pane so a restore can jump to the record.
/// The permanent erase asks for the usual confirm.
pub fn trash_modal(ctx: &egui::Context, app: &mut App) {
    if !app.boxes.session.as_ref().is_some_and(|s| s.show_trash) {
        return;
    }
    let rows: Vec<(String, String, String, theme::Icon, egui::Color32)> = app
        .boxes
        .session
        .as_ref()
        .map(|s| {
            s.vault
                .trash
                .iter()
                .map(|it| {
                    let kind = it.record.kind();
                    (
                        it.record.id().to_owned(),
                        it.record.name().to_owned(),
                        ago(it.deleted),
                        theme::kind_icon(kind),
                        theme::tag_color(kind),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    let mut restore: Option<String> = None;
    let mut empty = false;
    app.boxes.editing(true);
    let open = theme::dialog(ctx, egui::Id::new("trash"), |ui| {
        theme::width(ui, 380.0);
        let closed = theme::head(ui, &app.icons, "Trash");
        if rows.is_empty() {
            theme::empty(ui, false, "the trash is empty", "deleted records wait here");
        } else {
            ui.add_space(6.0);
            theme::scroll(ui, egui::ScrollArea::vertical().max_height(320.0), |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for (id, name, when, icon, color) in &rows {
                    ui.horizontal(|ui| {
                        if let Some(img) = app.icons.image(*icon, 15.0, *color) {
                            ui.add(img);
                        }
                        ui.add(Label::new(RichText::new(name).color(theme::palette().text)).wrap());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(t("Restore")).clicked() {
                                restore = Some(id.clone());
                            }
                            ui.label(RichText::new(when).small().color(theme::palette().faint));
                        });
                    });
                }
            });
        }
        ui.add_space(ACT);
        if !rows.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                empty = theme::secondary_button(
                    ui,
                    &app.icons,
                    Some(theme::Icon::Trash),
                    "Empty the trash",
                )
                .clicked();
            });
        }
        closed
    });

    if let Some(id) = restore {
        app.boxes.restore_item(&id);
        if let Some(s) = app.boxes.session.as_mut() {
            s.show_trash = false;
        }
    } else if empty {
        // The confirm draws above this window, so it steps aside for it.
        if let Some(s) = app.boxes.session.as_mut() {
            s.show_trash = false;
            s.confirm = Some(Confirm::EmptyTrash);
        }
    } else if !open {
        if let Some(s) = app.boxes.session.as_mut() {
            s.show_trash = false;
        }
    } else {
        return;
    }
    app.boxes.editing(false);
    app.boxes.flush(&mut app.shell);
}

/// How long ago a record was deleted, in the coarsest unit.
fn ago(secs: u64) -> String {
    let now = crate::vault::model::now_secs();
    let d = now.saturating_sub(secs);
    if d < 60 {
        tf("{n} s ago", &[("n", &d)])
    } else if d < 3600 {
        tf("{n} min ago", &[("n", &(d / 60))])
    } else if d < 86_400 {
        tf("{n} h ago", &[("n", &(d / 3600))])
    } else {
        tf("{n} d ago", &[("n", &(d / 86_400))])
    }
}

/// The printed code that opens the box when the password is gone. It is a copy of the
/// key, wrapped under a code the user writes down — not a second password, and not a
/// reset: nothing here can recover a box whose code was never made.
fn recovery_card(ui: &mut egui::Ui, app: &mut App) {
    card(ui, "Recovery code", |ui| {
        theme::hint(
            ui,
            t("a printed code opens this box when the master password is gone — write it down and keep it away from the box"),
        );
        ui.add_space(HINT);
        let has = app
            .boxes
            .session
            .as_ref()
            .is_some_and(|s| s.creds.recovery.is_some());
        if let Some(code) = app.auth.code_shown.clone() {
            ui.add_space(HINT);
            balance::label(
                ui,
                RichText::new(code.as_str())
                    .monospace()
                    .color(theme::palette().text),
            );
            ui.add_space(HINT);
            theme::hint(ui, t("this is the only time it is shown — copy it now"));
            ui.add_space(ACT);
            ui.horizontal(|ui| {
                if theme::primary_button(ui, &app.icons, Some(theme::Icon::Copy), "Copy it", None)
                    .clicked()
                {
                    app.shell.copy_text(ui.ctx(), &code);
                }
                ui.add_space(HINT);
                if theme::secondary_button(ui, &app.icons, None, "I wrote it down").clicked() {
                    app.auth.code_shown = None;
                }
            });
            return;
        }
        ui.add_space(ACT);
        let label = if has {
            "Make a new code (the old one stops working)"
        } else {
            "Make a recovery code"
        };
        if theme::secondary_button(ui, &app.icons, Some(theme::Icon::Key), label).clicked() {
            let code = crate::vault::recovery::make_code();
            let made = match app.boxes.session.as_mut() {
                Some(s) => match crate::vault::store::box_id(&s.path) {
                    Ok(id) => match crate::vault::store::set_recovery(&mut s.creds, &code, &id) {
                        Ok(()) => {
                            s.mark_dirty();
                            true
                        }
                        Err(e) => {
                            app.shell.fail(e);
                            false
                        }
                    },
                    Err(e) => {
                        app.shell.fail(e);
                        false
                    }
                },
                None => false,
            };
            if made {
                app.auth.code_shown = Some(code);
                app.shell.set_status(
                    "a recovery code was made — it is written with the next save",
                    crate::app::StatusKind::Saved,
                );
            }
        }
        if has {
            ui.add_space(HINT);
            theme::hint(
                ui,
                t("changing the master password or the key drops this code: make a new one after"),
            );
        }
    });
}

/// The SSH agent: the switch, where its socket is, and the keys the user allows. A key
/// is made here, never imported: a private key file from outside is its own parser and
/// its own policy, and this box can make its own.
fn agent_card(ui: &mut egui::Ui, app: &mut App) {
    card(ui, "SSH agent", |ui| {
        let mut on = app.prefs.ssh_agent;
        if theme::check(ui, &mut on, "Serve the box's SSH keys on a socket").changed() {
            app.prefs.ssh_agent = on;
            prefs::save(&app.prefs);
        }
        ui.add_space(HINT);
        let socket = crate::features::agent::socket_path(&app.prefs.ssh_socket);
        theme::hint(
            ui,
            &tf(
                "point SSH_AUTH_SOCK at {path}",
                &[("path", &socket.display())],
            ),
        );
        ui.add_space(HINT);
        theme::hint(
            ui,
            t("only the keys you allow are listed and sign; the socket is 0600 and answers only while a box is unlocked"),
        );
        ui.add_space(ACT);

        let mut copy: Option<String> = None;
        let mut drop: Option<String> = None;
        let mut add = false;
        let names = match app.boxes.session.as_ref() {
            Some(s) => s
                .vault
                .ssh_keys
                .iter()
                .map(|it| (it.id.clone(), it.name.clone(), it.allowed))
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        for (id, name, allowed) in names {
            ui.horizontal(|ui| {
                let mut ticked = allowed;
                if theme::check(ui, &mut ticked, "").changed() {
                    if let Some(s) = app.boxes.session.as_mut() {
                        if let Some(key) = s.vault.ssh_keys.iter_mut().find(|it| it.id == id) {
                            key.allowed = ticked;
                            key.touched(crate::vault::model::now_secs());
                        }
                        s.mark_dirty();
                    }
                }
                ui.label(RichText::new(&name).color(theme::palette().text));
                if app
                    .icons
                    .button(
                        ui,
                        theme::Icon::Copy,
                        13.0,
                        theme::palette().muted,
                        "copy the public key",
                    )
                    .clicked()
                {
                    copy = app
                        .boxes
                        .session
                        .as_ref()
                        .and_then(|s| s.vault.ssh_keys.iter().find(|it| it.id == id))
                        .map(crate::features::agent::public_line);
                }
                if app
                    .icons
                    .button(
                        ui,
                        theme::Icon::Trash,
                        13.0,
                        theme::palette().error,
                        "Delete",
                    )
                    .clicked()
                {
                    drop = Some(id.clone());
                }
            });
        }
        if app.boxes.session.is_some() {
            ui.add_space(HINT);
            if theme::secondary_button(ui, &app.icons, Some(theme::Icon::Plus), "Add a key")
                .clicked()
            {
                add = true;
            }
        } else {
            theme::hint(ui, t("open a box to manage its keys"));
        }

        if let Some(line) = copy {
            app.shell.copy_text(ui.ctx(), &line);
            app.shell.set_status(
                "public key copied — paste it into remote's authorized_keys",
                crate::app::StatusKind::Saved,
            );
        }
        if let Some(id) = drop {
            if let Some(s) = app.boxes.session.as_mut() {
                s.vault.ssh_keys.retain(|it| it.id != id);
                s.mark_dirty();
            }
        }
        if add {
            if let Some(s) = app.boxes.session.as_mut() {
                let n = s.vault.ssh_keys.len() + 1;
                let comment = s.vault.name.clone();
                let key = crate::vault::model::SshKey::make(
                    crate::vault::model::now_secs(),
                    &format!("Key {n}"),
                    &comment,
                );
                s.vault.ssh_keys.push(key);
                s.mark_dirty();
            }
        }
    });
}

/// The box's YubiKey: enrol it, or drop it. The key is a second factor, never the only
/// one — the master password is asked for here because the box's key is derived from
/// it — and losing the key loses the box.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn key_card(ui: &mut egui::Ui, app: &mut App, keyfile: Option<bool>) {
    let busy = app
        .auth
        .op
        .as_ref()
        .is_some_and(|o| o.kind == OpKind::EnrolKey);
    let present = crate::shared::yubikey::present();
    card(ui, "YubiKey", |ui| {
        ui.add_enabled_ui(!busy, |ui| {
            auth::secret_field(
                ui,
                egui::Id::new("key-password"),
                &app.icons,
                &mut app.auth.key.password,
                "master password (needed to re-derive the box)",
            );
            if keyfile != Some(false) {
                ui.add_space(ROW);
                auth::file_field(
                    ui,
                    &app.icons,
                    &mut app.auth.key.keyfile,
                    if keyfile == Some(true) {
                        "keyfile (required by this box)"
                    } else {
                        "keyfile (if this box uses one)"
                    },
                    false,
                );
            }
            ui.add_space(HINT);
            theme::hint(
                ui,
                if present {
                    t("a key is plugged in — press Enrol and touch it when it blinks")
                } else {
                    t("no key is plugged in")
                },
            );
            ui.add_space(HINT);
            // Said before the button, not after: the key is a factor, and the box is
            // lost without it.
            theme::hint(
                ui,
                t("this adds a second factor: the box then needs the password and the key, on every device. Lose the key and the box is lost — there is no reset"),
            );
            ui.add_space(ACT);
        });
        let enrol = theme::primary_button(
            ui,
            &app.icons,
            Some(theme::Icon::Fingerprint),
            "Enrol this key",
            busy.then_some("Working…"),
        );
        if enrol.clicked() {
            app.auth.enrol_key(&app.boxes, true);
        }
        ui.add_space(HINT);
        if theme::secondary_button(ui, &app.icons, None, "Stop using it").clicked() {
            app.auth.enrol_key(&app.boxes, false);
        }
    });
}

/// Change the box's master password.
fn master_card(ui: &mut egui::Ui, app: &mut App, keyfile: Option<bool>) {
    let busy = app
        .auth
        .op
        .as_ref()
        .is_some_and(|o| o.kind == OpKind::ChangePassword);
    // The repo holds a copy of this box, sealed with the password about to be
    // replaced. The form has to say so, and offer the one thing this app can do
    // about it, before the button is pressed.
    let mirrored = app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.remote.as_ref())
        .map(|r| r.repo.clone());
    card(ui, "Master password", |ui| {
        ui.add_enabled_ui(!busy, |ui| {
            auth::secret_field(
                ui,
                egui::Id::new("change-current"),
                &app.icons,
                &mut app.auth.change.current,
                "current password",
            );
            ui.add_space(ROW);
            auth::secret_pair(
                ui,
                &app.icons,
                (
                    egui::Id::new("change-new"),
                    &mut app.auth.change.new,
                    "new password (min 12)",
                ),
                (
                    egui::Id::new("change-confirm"),
                    &mut app.auth.change.confirm,
                    "confirm new",
                ),
            );
            ui.add_space(HINT);
            theme::strength_meter(ui, &app.auth.change.new);
            if keyfile != Some(false) {
                ui.add_space(ROW);
                auth::file_field(
                    ui,
                    &app.icons,
                    &mut app.auth.change.keyfile,
                    // A finger-unlocked session cannot say which box needs one,
                    // and the file no longer tells it: the field is offered, and
                    // the change only goes through if it was needed.
                    if keyfile == Some(true) {
                        "keyfile (required by this box)"
                    } else {
                        "keyfile (if this box uses one)"
                    },
                    false,
                );
            }
            if let Some(repo) = mirrored.as_deref() {
                ui.add_space(ROW);
                theme::hint(
                    ui,
                    &tf(
                        "the repo {repo} keeps every version of this box it was sent. Those copies still open with the password you are replacing — only rewriting that repo's history erases them",
                        &[("repo", &repo)],
                    ),
                );
                ui.add_space(HINT);
                theme::check(
                    ui,
                    &mut app.auth.change.erase_remote,
                    "Erase the repo's copy",
                )
                .on_hover_text(t(
                    "the file leaves the repo and the next send starts it again from nothing — the commits behind it stay until that history is rewritten",
                ));
            }
            ui.add_space(ACT);
        });
        if theme::primary_button(
            ui,
            &app.icons,
            None,
            "Update password",
            busy.then_some("Updating…"),
        )
        .clicked()
        {
            app.auth.change_password(&app.boxes);
        }
    });
}

/// How long the box stays unlocked with nothing happening.
fn lock_card(ui: &mut egui::Ui, app: &mut App) {
    card(ui, "Lock on inactivity", |ui| {
        let current = app.prefs.lock_after;
        let label = match current {
            0 => t("Never").to_owned(),
            seconds if seconds % 60 == 0 => tf("{count} minutes", &[("count", &(seconds / 60))]),
            seconds => tf("{count} seconds", &[("count", &seconds)]),
        };
        ui.scope(|ui| {
            // A select as wide as its own text is a stray control on a card three
            // times as wide: it takes the row, like the sliders do. `combo_width`
            // is a floor rather than a ceiling, so the label is truncated too — a
            // longer language shortens the text, never widens the button past the
            // card. The menu is as wide as the button, so the two read as one.
            ui.spacing_mut().combo_width = ui.available_width();
            theme::combo(
                ui,
                egui::ComboBox::from_id_salt("lock-after")
                    .truncate()
                    .selected_text(label),
                |ui| {
                    for (seconds, label) in [
                        (0, t("Never")),
                        (5 * 60, t("5 minutes")),
                        (10 * 60, t("10 minutes")),
                        (15 * 60, t("15 minutes")),
                        (prefs::MAX_LOCK_AFTER, t("30 minutes")),
                    ] {
                        if ui.selectable_label(current == seconds, label).clicked() {
                            app.prefs.lock_after = seconds;
                            prefs::save(&app.prefs);
                        }
                    }
                },
            );
        });
        ui.add_space(HINT);
        theme::hint(
                            ui,
                            "Uses system-wide inactivity when available; Pandora and approved-browser activity count too. Detected session locks and sleep always lock. Never disables only the idle timer.",
                        );
    });
}

/// How long a copied secret stays on the system clipboard.
fn clipboard_card(ui: &mut egui::Ui, app: &mut App) {
    card(ui, "Clipboard", |ui| {
        if theme::slider(
            ui,
            &format!("{} s", prefs::MAX_CLIP_CLEAR),
            egui::Slider::new(&mut app.prefs.clip_clear, 0..=prefs::MAX_CLIP_CLEAR).suffix(" s"),
        )
        .changed()
        {
            prefs::save(&app.prefs);
        }
        ui.add_space(HINT);
        theme::hint(
                            ui,
                            "0 s never takes a copy back. Otherwise a copied password is wiped from the system clipboard once the countdown runs out, and as soon as a box locks — whatever you copied afterwards is left alone",
                        );
    });
}

/// How many wrong passwords a box takes before it starts refusing, and for how long.
fn attempts_card(ui: &mut egui::Ui, app: &mut App) {
    card(ui, "Unlock attempts", |ui| {
        let mut save = false;
        if theme::slider(
            ui,
            &format!("{} {}", prefs::MAX_ATTEMPTS, t("attempts")),
            egui::Slider::new(&mut app.prefs.max_attempts, 0..=prefs::MAX_ATTEMPTS)
                .suffix(format!(" {}", t("attempts"))),
        )
        .changed()
        {
            // A limit with no wait would refuse a box for no time at all.
            if app.prefs.max_attempts > 0 {
                app.prefs.retry_delay = app.prefs.retry_delay.max(prefs::MIN_RETRY);
            }
            save = true;
        }
        ui.add_space(HINT);
        let floor = if app.prefs.max_attempts > 0 {
            prefs::MIN_RETRY
        } else {
            0
        };
        if theme::slider(
            ui,
            &format!("{} s", prefs::MAX_RETRY_DELAY),
            egui::Slider::new(&mut app.prefs.retry_delay, floor..=prefs::MAX_RETRY_DELAY)
                .suffix(" s"),
        )
        .changed()
        {
            save = true;
        }
        if save {
            prefs::save(&app.prefs);
        }
        ui.add_space(HINT);
        theme::hint(
                            ui,
                            "0 attempts never refuses a box. Once the limit is reached the box waits, then the count starts over; an unlock that works forgets the failures. The limit is for every box, the count is kept per box file, on this machine.",
                        );
    });
}

/// Lock now, and what the app is.
fn footer(ui: &mut egui::Ui, app: &mut App) {
    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
        if app
            .icons
            .button(
                ui,
                theme::Icon::LockKeyhole,
                15.0,
                theme::palette().error,
                "Lock now",
            )
            .clicked()
        {
            app.lock(ui.ctx());
        }
        ui.add_space(ROW);
        theme::hint(
            ui,
            "the key lives in memory while unlocked — disable hibernation if disk dumps worry you",
        );
        ui.add_space(4.0);
        theme::hint(
            ui,
            &tf(
                "Pandora v{version} — one encrypted file, no server",
                &[("version", &env!("CARGO_PKG_VERSION"))],
            ),
        );
        crate::features::update::ui::row(ui, app);
        ui.add_space(4.0);
        crate::features::health::ui::row(ui, app);
    });
}

/// The bar's way into the app's settings: scheme and language, which outlive every box.
pub fn button(ui: &mut egui::Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    let width = crate::shared::ui::responsive::target(&ctx);
    let glyph = crate::shared::ui::responsive::grow(&ctx, 15.0);
    let resp = theme::slot(
        ui,
        width,
        egui::Id::new("bar-prefs"),
        app.prefs_open,
        ui.available_height(),
        theme::Edge::Floor,
        |ui| {
            ui.add_space(((width - glyph) / 2.0).floor().max(0.0));
            if let Some(img) = app.icons.image(
                theme::Icon::Sliders,
                glyph,
                if app.prefs_open {
                    theme::ACCENT
                } else {
                    theme::palette().muted
                },
            ) {
                ui.add(img);
            }
        },
    );
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t("General settings"))
    });
    if resp.on_hover_text(t("General settings")).clicked() {
        app.prefs_open = true;
    }
}

/// App settings in a dialog: scheme and language, read before any box opens.
pub fn general(ctx: &egui::Context, app: &mut App) {
    if !app.prefs_open {
        return;
    }
    let open = theme::dialog(ctx, egui::Id::new("general-settings"), |ui| {
        theme::width(ui, 300.0);
        let closed = theme::head(ui, &app.icons, t("General settings"));
        appearance(ui, app);
        ui.add_space(ACT);
        languages(ui, app);
        closed
    });
    if !open {
        app.prefs_open = false;
    }
}

/// Which way the app is painted: tiles, not a select, so the choice is made by eye.
/// The change lands on the next frame.
fn appearance(ui: &mut egui::Ui, app: &mut App) {
    theme::field_label(ui, "Appearance");
    ui.add_space(8.0);
    let mut picked = None;
    ui.horizontal(|ui| {
        // The pair takes the row whole: half the width each, less the gap the
        // layout leaves between the two.
        let gap = ui.spacing().item_spacing.x;
        let width = (ui.available_width() - gap) / 2.0;
        for scheme in theme::Scheme::ALL {
            let selected = app.prefs.theme == scheme;
            if scheme_tile(ui, scheme, selected, width).clicked() {
                picked = Some(scheme);
            }
        }
    });
    let Some(scheme) = picked.filter(|scheme| *scheme != app.prefs.theme) else {
        return;
    };
    app.prefs.theme = scheme;
    theme::set(scheme);
    theme::apply(ui.ctx());
    prefs::save(&app.prefs);
}

/// One scheme painted in its own colours; the one in use wears the green edge and a check.
fn scheme_tile(
    ui: &mut egui::Ui,
    scheme: theme::Scheme,
    selected: bool,
    width: f32,
) -> egui::Response {
    let palette = theme::palette_of(scheme);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 84.0), egui::Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let edge = if selected {
        theme::ACCENT
    } else if resp.hovered() {
        theme::palette().muted
    } else {
        theme::palette().hair
    };
    {
        let painter = ui.painter();
        painter.rect_filled(rect, theme::radius(10), palette.paper);
        painter.rect_stroke(
            rect,
            theme::radius(10),
            theme::stroke(edge, if selected { 2.0 } else { 1.0 }),
            egui::StrokeKind::Inside,
        );
        // What every page of the app is: a card on the paper, two lines of text
        // and a button in the brand green.
        let card = egui::Rect::from_min_size(
            rect.min + egui::vec2(10.0, 10.0),
            egui::vec2(rect.width() - 20.0, 46.0),
        );
        painter.rect_filled(card, theme::radius(6), palette.panel);
        painter.rect_stroke(
            card,
            theme::radius(6),
            theme::stroke(palette.hair, 1.0),
            egui::StrokeKind::Inside,
        );
        let bar = |at: egui::Vec2, w: f32, h: f32, color: egui::Color32| {
            painter.rect_filled(
                egui::Rect::from_min_size(card.min + at, egui::vec2(w, h)),
                theme::radius(2),
                color,
            );
        };
        bar(
            egui::vec2(12.0, 11.0),
            card.width() * 0.42,
            4.0,
            palette.text,
        );
        bar(
            egui::vec2(12.0, 21.0),
            card.width() * 0.62,
            3.0,
            palette.muted,
        );
        bar(egui::vec2(12.0, 31.0), 22.0, 9.0, theme::ACCENT);
        painter.text(
            egui::pos2(rect.center().x, rect.max.y - 13.0),
            egui::Align2::CENTER_CENTER,
            scheme.name(),
            egui::FontId::proportional(12.5),
            if selected {
                palette.text
            } else {
                palette.muted
            },
        );
        if selected {
            let check = egui::pos2(rect.max.x - 15.0, rect.max.y - 13.0);
            painter.circle_filled(check, 7.0, theme::ACCENT);
            let tick = theme::stroke(palette.paper, 1.6);
            painter.line_segment(
                [check + egui::vec2(-3.0, 0.2), check + egui::vec2(-0.8, 2.5)],
                tick,
            );
            painter.line_segment(
                [check + egui::vec2(-0.8, 2.5), check + egui::vec2(3.2, -2.3)],
                tick,
            );
        }
    }
    theme::focus(ui, &resp);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, scheme.name())
    });
    resp
}

/// The languages, one row each.
fn languages(ui: &mut egui::Ui, app: &mut App) {
    theme::field_label(ui, "Language");
    ui.add_space(8.0);
    let mut picked = None;
    for lang in Lang::ALL {
        let selected = app.prefs.lang == lang;
        // The chosen one wears the same green a selected row does everywhere
        // else; the rest keep an edge, so no language floats bare in the card.
        let edge = if selected {
            theme::ACCENT
        } else {
            theme::palette().hair
        };
        let mut row = egui::Button::new(RichText::new(lang.name()).color(theme::palette().text))
            .stroke(theme::stroke(edge, 1.0));
        if selected {
            row = row.fill(theme::palette().accent_dim);
        }
        if ui
            .add_sized(egui::vec2(ui.available_width(), 36.0), row)
            .clicked()
        {
            picked = Some(lang);
        }
    }
    let Some(lang) = picked else {
        return;
    };
    app.prefs.lang = lang;
    i18n::set(lang);
    prefs::save(&app.prefs);
}

/// What the fingerprint card has to say about the box on screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reader {
    /// No reader this app may use: nothing enrolled, or no screen lock.
    None,
    /// The reader is there and this box already opens with it.
    Set,
    /// The user asked for it, but this box was never kept — the password has to
    /// work once before it can be.
    Pending,
    /// A reader is there and the feature is off.
    Off,
}

impl Reader {
    /// The line under the switch: a switch alone cannot tell a box that opens
    /// with a fingerprint from one that only looks set up.
    fn status(self) -> &'static str {
        match self {
            Reader::None => t(
                "no fingerprint this app may use — enroll one, with a screen lock, in Android settings",
            ),
            Reader::Set => t("this box opens with your fingerprint"),
            Reader::Pending => t("not set up yet — do it here, or open the box with its password once"),
            Reader::Off => t("off — the master password is asked for every time"),
        }
    }
}

/// The card's state from the three facts it depends on; pure so the branching is testable.
fn reader(available: bool, sealed: bool, wanted: bool) -> Reader {
    if !available {
        return Reader::None;
    }
    if sealed {
        return Reader::Set;
    }
    if wanted {
        Reader::Pending
    } else {
        Reader::Off
    }
}

/// The fingerprint card. A desktop never paints it (the master password is the only
/// door); Android always does, and a reader it may not use explains itself.
/// Whether a browser may open a box of its own. Separate from the bridge, which needs
/// no switch: with the app open the extension uses the box this window has and never
/// asks for a password. This is the other thing — a master password typed into a
/// browser — so it starts off.
fn browser(ui: &mut egui::Ui, app: &mut App) {
    card(ui, "Browsers", |ui| {
        let mut on = app.prefs.unlock_from_browser;
        if theme::check(ui, &mut on, "Let a browser open a box on its own").changed() {
            app.prefs.unlock_from_browser = on;
            prefs::save(&app.prefs);
        }
        ui.add_space(HINT);
        balance::label(
            ui,
            RichText::new(t(open_by_browser(on)))
                .small()
                .color(theme::palette().muted),
        );
        ui.add_space(HINT);
        theme::hint(ui, &let_in(app.prefs.browsers.len()));
        ui.add_space(HINT);
        theme::hint(ui, t(passkey_browser()));
    });
}

/// Where passkeys are used, in one line: the box holds them, and only the browser
/// with a provider API can ask for one.
fn passkey_browser() -> &'static str {
    "passkeys live in this box and only the extension on Chrome 129+ can use them — Firefox has no passkey provider API"
}

/// What a tick alone cannot say: off and on paint the same box, and the difference
/// — whose keyboard the master password is typed on — is the whole switch.
fn open_by_browser(on: bool) -> &'static str {
    if on {
        t("on — a browser can also open a box of its own, which means the master password is typed into the browser")
    } else {
        t("off — a browser reads the box you have open here and nothing else, and never opens one of its own")
    }
}

/// Where the browsers the user answered for live, and what the list being empty
/// means: with none allowed there is no chip on the bar to point at.
fn let_in(allowed: usize) -> String {
    if allowed == 0 {
        t("no browser is allowed yet — one that asks gets a prompt, and whatever you allow is listed on the window bar")
            .to_owned()
    } else {
        t("the browsers you allow are listed on the window bar, with a way to take each back")
            .to_owned()
    }
}

/// The phone's own autofill. On, another app's login form can fill from the box
/// on screen; off, the box stays in this window and the panel offers nothing but
/// the way to open the app.
#[cfg(target_os = "android")]
fn autofill(ui: &mut egui::Ui, app: &mut App) {
    card(ui, t("Autofill"), |ui| {
        let mut on = app.prefs.autofill;
        if theme::check(ui, &mut on, t("Fill logins from this box in other apps")).changed() {
            app.prefs.autofill = on;
            prefs::save(&app.prefs);
            if !on {
                // Off now, not at the next frame: the copy is what the switch is about.
                crate::shared::autofill::clear();
            }
        }
        ui.add_space(HINT);
        balance::label(
            ui,
            RichText::new(t(autofill_switch(on)))
                .small()
                .color(theme::palette().muted),
        );
    });
}

/// What a tick alone cannot say: off and on paint the same box, and the difference
/// — whether another app can read this one — is the whole switch.
#[cfg(target_os = "android")]
fn autofill_switch(on: bool) -> &'static str {
    if on {
        t("on — another app's login form fills from this box while it is open here, and the copy goes as soon as it locks")
    } else {
        t("off — the box stays in this window, and no other app can fill from it")
    }
}

/// The fingerprint card: the switch, and what it cannot say by itself. `true` when
/// it painted, so the pile that holds it owns the room under it — on a desk there
/// is no reader and no card, and a gap with nothing over it reads as a mistake.
fn fingerprint(ui: &mut egui::Ui, app: &mut App) -> bool {
    let state = reader(
        biometric::available(),
        app.boxes
            .session
            .as_ref()
            .is_some_and(|s| biometric::sealed(&s.path)),
        app.prefs.biometric,
    );
    if state == Reader::None && !cfg!(target_os = "android") {
        return false;
    }
    card(ui, "Fingerprint unlock", |ui| {
        // A reader the app cannot use is a switch that would do nothing.
        ui.add_enabled_ui(state != Reader::None, |ui| {
            let mut on = app.prefs.biometric;
            if theme::check(ui, &mut on, "Open this box with my fingerprint").changed() {
                app.prefs.biometric = on;
                prefs::save(&app.prefs);
                if on {
                    app.keep_key();
                } else {
                    app.drop_key();
                }
            }
        });
        ui.add_space(HINT);
        balance::label(
            ui,
            RichText::new(t(state.status()))
                .small()
                .color(theme::palette().muted),
        );
        if state == Reader::Pending {
            ui.add_space(ROW);
            let now = theme::secondary_button(ui, &app.icons, None, "Set it up now").clicked();
            if now {
                app.keep_key();
            }
        }
        ui.add_space(HINT);
        theme::hint(
            ui,
            "the key this box opens with is kept in the device's keystore, behind your fingerprint — the password itself never is. Every box you open with a password is kept the same way. Enrolling a new fingerprint drops the key, and the password is asked for once more",
        );
    });
    true
}

/// A settings card: the app's card, its body reading from the card's left edge
/// like the heading over it. Centred, the prose of a hint swayed line by line with
/// the width of the card; only the buttons centre themselves, and they do it in
/// their own layout.
fn card(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    theme::card(ui, title, |ui| {
        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| add(ui));
    });
}

/// One key/value row of a card. A narrow window stacks the value under its key:
/// side by side, the pair would meet in the middle of the row.
fn kv(ui: &mut egui::Ui, k: &str, v: &str, compact: bool) {
    if compact {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            theme::field_label(ui, k);
            ui.add(Label::new(RichText::new(v).small().color(theme::palette().text)).truncate());
        });
        ui.add_space(HINT);
        return;
    }
    ui.horizontal(|ui| {
        theme::field_label(ui, k);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(Label::new(RichText::new(v).small().color(theme::palette().text)).truncate());
        });
    });
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::ui::shot;

    /// The window sizes the page is measured at: the desktop default, a wide
    /// window, and a phone held upright.
    const SIZES: [(f32, f32); 3] = [(1000.0, 700.0), (1400.0, 900.0), (393.0, 851.0)];

    /// Tall enough that the whole page fits in the viewport. A scroll area only
    /// paints what it reaches, and a control nobody painted has no rect to read:
    /// the sliders and the select live far below a 700 pt window.
    const TALL: f32 = 2600.0;

    /// The settings page as `run` paints it, at one window size. A box is open,
    /// so the page is the one the app shows and not a page of cards without the
    /// first of them. `setup` runs before the page is drawn, which is how a slider
    /// is put at the top of its range.
    fn page_set(w: f32, h: f32, setup: impl FnOnce(&mut App)) -> Vec<egui::epaint::ClippedShape> {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("settings-measure");
        setup(&mut app);
        // The breakpoint is the window's own width, which only the frame's
        // input carries: at this point the context still holds the empty one.
        let compact = w < crate::shared::ui::responsive::BREAKPOINT;
        shot::frame(&ctx, egui::vec2(w, h), Vec::new(), |ui| {
            run(ui, &mut app, compact)
        })
    }

    fn page(w: f32, h: f32) -> Vec<egui::epaint::ClippedShape> {
        page_set(w, h, |_| {})
    }

    /// The app's own settings, as `general` paints them: a bare app with no box
    /// open — which is the point of them — and the dialog switched on.
    fn app_settings(size: (f32, f32)) -> Vec<egui::epaint::ClippedShape> {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.prefs_open = true;
        // A dialog is an `Area`: egui lays it out on a first, invisible sizing
        // pass and fades it in over the next few, so the frame read is one whose
        // colours are the palette's own.
        let mut shapes = Vec::new();
        for _ in 0..12 {
            let mut out = ctx.run_ui(shot::input(egui::vec2(size.0, size.1), Vec::new()), |ui| {
                let ctx = ui.ctx().clone();
                general(&ctx, &mut app)
            });
            out.textures_delta.clear();
            shapes = out.shapes;
        }
        shapes
    }

    /// Every card on the page, in the order they are painted.
    fn cards(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::Rect> {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == theme::palette().panel => Some(r.rect),
                _ => None,
            })
            .collect()
    }

    /// The card `title` heads: the panel under the heading. It is painted
    /// before the text it holds — a frame reserves its shape and fills it in at
    /// the end — so the card is found by the edge it shares with its heading, not
    /// by the order the two are painted in.
    fn card(shapes: &[egui::epaint::ClippedShape], title: &str) -> egui::Rect {
        let head = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == title.to_uppercase() => {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{title:?} heads a card on the page"));
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.fill == theme::palette().panel
                        && (r.rect.left() - head.left()).abs() < 1.5
                        && r.rect.top() >= head.bottom() - 1.0 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .min_by(|a, b| a.top().total_cmp(&b.top()))
            .unwrap_or_else(|| panic!("{title:?}'s heading sits over a card"))
    }

    /// The rail of a slider: egui fills it with the inactive widget fill, which
    /// this theme sets to the palette's `elev`, and it is only as tall as
    /// `slider_rail_height` — a strip, unlike the button of a select.
    fn rail(shapes: &[egui::epaint::ClippedShape], card: egui::Rect) -> Option<egui::Rect> {
        shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.fill == theme::palette().elev
                    && r.rect.height() < 12.0
                    && card.contains_rect(r.rect) =>
            {
                Some(r.rect)
            }
            _ => None,
        })
    }

    /// A whole control row, painted as a filled box: the button of a select, or
    /// the box a slider's value is typed into. Both are a [`theme::target`] wide
    /// and one row tall, which is what tells them from a rail and its handle.
    fn row_box(shapes: &[egui::epaint::ClippedShape], card: egui::Rect) -> Option<egui::Rect> {
        shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.fill == theme::palette().elev
                    && r.rect.height() >= 28.0
                    && r.rect.width() >= 48.0
                    && card.contains_rect(r.rect) =>
            {
                Some(r.rect)
            }
            _ => None,
        })
    }

    /// The room `theme::card`'s frame keeps on either side of what it holds.
    const CARD_SIDE: f32 = 20.0;

    /// What a card's frame leaves of its width for the controls inside it.
    fn interior(card: egui::Rect) -> f32 {
        card.width() - 2.0 * CARD_SIDE
    }

    /// The page's first card: the box card is the one the page opens with, and
    /// the one a window as short as 700 pt still paints whole.
    fn first_card(shapes: &[egui::epaint::ClippedShape]) -> egui::Rect {
        *cards(shapes)
            .first()
            .expect("the page paints its first card")
    }

    /// The page is a single column, one card's measure wide, hung on the pane's
    /// centre; a window narrower than that card gives it the width it has and no
    /// more.
    #[test]
    fn the_settings_page_is_measured_by_its_window() {
        for (w, h) in SIZES {
            let shapes = page(w, TALL);
            let column = first_card(&shapes);
            // The page scrolls: it hangs on the centre of the pane less the gutter
            // the scroll bar keeps for itself.
            let centre = (w - theme::BAR) / 2.0;
            println!(
                "{w}x{h}: column {:.1}..{:.1} — {} pt wide, centre {:.1} against the pane's {centre:.1}",
                column.left(),
                column.right(),
                column.width(),
                column.center().x,
            );
            assert!(
                column.left() >= 0.0 && column.right() <= w,
                "the page stays in the window: {column:?} in {w}"
            );
            assert!(
                (column.center().x - centre).abs() < 3.0,
                "the column hangs on the pane's centre: {column:?} in {w}"
            );
            assert!(
                column.width() <= theme::COLUMN,
                "the column never outgrows a card: {column:?}"
            );
            if w >= crate::shared::ui::responsive::BREAKPOINT {
                assert!(
                    (column.width() - theme::COLUMN).abs() < 1.0,
                    "a window wider than a card gives it the card's measure: {:.1} of {}",
                    column.width(),
                    theme::COLUMN
                );
            } else {
                assert!(
                    column.width() < theme::COLUMN,
                    "a narrow window gives the column its own width: {column:?}"
                );
            }
            // One column: every card is the page's own width, nothing beside
            // anything.
            for card in cards(&shapes) {
                assert!(
                    (card.width() - column.width()).abs() < 1.0,
                    "one column: every card is the page's width, got {card:?} in {column:?}"
                );
            }
        }
    }

    /// A slider and its value box are one control, and the control takes the row
    /// it is on. The rail is measured as a row and not on its own: what the rail
    /// does not take is the value's own box beside it — 418 of a 420 pt card is
    /// the slider, 342 of it is the rail. A slider on a card four times as wide as
    /// egui's own 100 pt rail is the thing this fixed.
    /// The key card says what it is for and what losing the key costs, before the
    /// button is pressed.
    #[test]
    fn the_key_card_says_the_box_is_lost_without_its_key() {
        let shapes = page(900.0, 2400.0);
        let drawn = shot::words(&shapes);
        for line in [
            "YUBIKEY",
            "Enrol this key",
            "Stop using it",
            "this adds a second factor: the box then needs the password and the key, on every device. Lose the key and the box is lost — there is no reset",
        ] {
            assert!(
                drawn.iter().any(|word| word == line),
                "{line:?} is missing: {drawn:?}"
            );
        }
        assert!(
            drawn
                .iter()
                .any(|word| word.starts_with("a key is plugged in")
                    || word == "no key is plugged in"),
            "the card says whether one is there: {drawn:?}"
        );
    }

    #[test]
    fn the_clipboard_slider_fills_its_row() {
        for (w, h) in SIZES {
            let shapes = page(w, TALL);
            let clipboard = card(&shapes, "Clipboard");
            let slider = rail(&shapes, clipboard).expect("the clipboard card holds a slider");
            let value = row_box(&shapes, clipboard).expect("the slider carries its value box");
            let (row, room) = (value.right() - slider.left(), interior(clipboard));
            println!(
                "{w}x{h}: slider row {row:.1} pt ({:.0}%) of {room:.1}, of which the rail is \
                 {:.1} pt ({:.0}%)",
                row / room * 100.0,
                slider.width(),
                slider.width() / room * 100.0,
            );
            assert!(
                clipboard.contains_rect(slider) && clipboard.contains_rect(value),
                "the slider stays inside its card: {slider:?} and {value:?} in {clipboard:?}"
            );
            assert!(
                row / room > 0.9,
                "the slider fills its row: {row:.1} pt of {room:.1}"
            );
            // And the row ends where the card does: the few points left over are
            // the width the value's own box rounds off the room reserved for it, not
            // a gap the slider forgot to take. A rail at egui's own 100 pt would
            // leave 320 of the 420 to the card's empty middle.
            assert!(
                room - row < 5.0,
                "the slider's row ends {:.1} pt short of the card, of {room:.1}",
                room - row
            );
            // The rail is all the row leaves it, and no more: what it does not take
            // is the value's box and the gap before it — a fifth of the row on a
            // card, a quarter on a phone, where the same box is a bigger share.
            // Nine tenths of the card would want that box 30 pt wide, and "600 s"
            // alone is wider than that: a rail the card's own width means the number
            // leaves the row, which is a slider of our own and not this one.
            assert!(
                slider.width() >= value.width(),
                "the rail is the wide half of the control: {:.1} against the value's {:.1}",
                slider.width(),
                value.width()
            );

            // At the top of its range the value's box is at its widest, and the
            // rail gave up exactly that: the row is the card's width, and no card
            // had to grow to hold it.
            let shapes = page_set(w, TALL, |app| app.prefs.clip_clear = prefs::MAX_CLIP_CLEAR);
            let clipboard = card(&shapes, "Clipboard");
            let slider = rail(&shapes, clipboard).expect("the clipboard card holds a slider");
            let value = row_box(&shapes, clipboard).expect("the slider carries its value box");
            let (row, room) = (value.right() - slider.left(), interior(clipboard));
            println!("{w}x{h} at the top: slider row {row:.1} pt of {room:.1}");
            assert!(
                (row / room - 1.0).abs() < 0.01,
                "the row is the card's at the top of the range: {row:.1} pt of {room:.1}"
            );
            for grown in cards(&shapes) {
                assert!(
                    grown.width() <= first_card(&shapes).width() + 1.0,
                    "no card grew to hold a value box: {grown:?}"
                );
            }
        }
    }

    /// The select takes the row like a slider does, and it shows its label whole:
    /// `combo_width` is a floor rather than a ceiling, so the label truncates —
    /// a longer language shortens the text, never widens the button past the card.
    #[test]
    fn the_lock_select_fills_its_row_without_cutting_its_label() {
        for (w, h) in SIZES {
            let shapes = page(w, TALL);
            let lock = card(&shapes, "Lock on inactivity");
            let select = row_box(&shapes, lock).expect("the lock card holds a select");
            let room = interior(lock);
            let inset = select.left() - (lock.left() + CARD_SIDE);
            println!(
                "{w}x{h}: select {:.1} pt ({:.0}%) of {room:.1}, {inset:.1} pt in from the card",
                select.width(),
                select.width() / room * 100.0
            );
            assert!(
                lock.contains_rect(select),
                "the select stays inside its card: {select:?} in {lock:?}"
            );
            // The button is the whole control — there is no value box beside it —
            // so it starts at the card's own inner edge and ends where the card
            // does, less the couple of points the frame's hairline rounds off. A
            // select as wide as its own text was a quarter of that, floating on the
            // card's middle.
            assert!(
                inset.abs() < 2.5,
                "the select starts at the card's inner edge, not in the middle: {inset:.1} pt in"
            );
            assert!(
                room - select.width() < 5.0,
                "the select ends {:.1} pt short of the card, of {room:.1}",
                room - select.width()
            );
            for s in &shapes {
                if let egui::Shape::Text(t) = &s.shape {
                    assert!(
                        !t.galley.elided || !lock.contains(t.pos),
                        "the select's label is cut: {:?}",
                        t.galley.text()
                    );
                }
            }
            // And the label it shows is on the page, whole: the box opens on
            // fifteen minutes unless the user said otherwise.
            let drawn = shot::words(&shapes);
            assert!(
                drawn.iter().any(|word| word == "15 minutes"),
                "the chosen lock is named: {drawn:?}"
            );
        }
    }

    /// The page stops growing at its measure: from a window that holds a card to
    /// one much wider, every card keeps its width, its height and its place, and
    /// the page paints the same words in the same order. What the window's own
    /// width buys is margin, not a wider card.
    ///
    /// The widths are the windows' own (1000 and 1400). The frames are tall so the
    /// cards below the fold are painted too — a width does not depend on a height.
    #[test]
    fn a_wider_window_grows_the_page_no_further() {
        let mut pages = Vec::new();
        for w in [1000.0, 1400.0] {
            let shapes = page(w, TALL);
            let centre = (w - theme::BAR) / 2.0;
            let boxes: Vec<(f32, f32, f32)> = cards(&shapes)
                .iter()
                .map(|card| (card.width(), card.height(), card.center().x - centre))
                .collect();
            let words = shot::words(&shapes);
            println!(
                "{w}pt wide: {} card(s), {} word(s)",
                boxes.len(),
                words.len()
            );
            // Nothing is painted past the pane, whatever the window's width.
            for s in &shapes {
                let r = s.shape.visual_bounding_rect();
                if !r.min.x.is_finite() || !r.max.x.is_finite() {
                    continue;
                }
                assert!(
                    r.min.x >= -1.0 && r.max.x <= w + 1.0,
                    "paints past the pane: {r:?} in {w}"
                );
            }
            pages.push((w, boxes, words));
        }
        let (_, first, words) = &pages[0];
        for (w, boxes, more) in &pages[1..] {
            assert_eq!(first.len(), boxes.len(), "the same cards at {w} as at 1000");
            for (a, b) in first.iter().zip(boxes) {
                assert!(
                    (a.0 - b.0).abs() < 0.5 && (a.1 - b.1).abs() < 0.5,
                    "a card grew or moved with the window: {a:?} against {b:?} at {w}"
                );
                // Same width, so the same wrapping: a different height would mean a
                // hint or a row came out differently at the wider window.
                assert!(
                    (a.2 - b.2).abs() < 0.5,
                    "a card holds the same thing, so the same height: {a:?} against {b:?} at {w}"
                );
            }
            assert_eq!(
                words, more,
                "the page shows the same words, in the same order, at {w} as at 1000"
            );
        }
    }

    /// The scheme and the language are read out of `prefs` before a box opens and
    /// outlive every one of them: the box's page must not offer them, and the
    /// app's own dialog must — with nothing unlocked.
    #[test]
    fn the_scheme_and_the_language_are_the_apps_not_the_boxs() {
        let drawn = shot::words(&page(1000.0, TALL));
        for gone in [
            "APPEARANCE",
            "LANGUAGE",
            "Dark",
            "Light",
            "English",
            "Español",
        ] {
            assert!(
                !drawn.iter().any(|word| word == gone),
                "{gone} is not the box's to set: {drawn:?}"
            );
        }

        let shapes = app_settings((420.0, 620.0));
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
                "missing {want}: {drawn:?}"
            );
        }
        // Each tile is painted in the scheme it offers, not the one in use: the
        // preview is the choice, so a wrong palette is the one bug that would
        // still read as a working switch.
        for scheme in theme::Scheme::ALL {
            let paper = theme::palette_of(scheme).paper;
            assert!(
                shapes
                    .iter()
                    .any(|s| matches!(&s.shape, egui::Shape::Rect(r)
                    if r.fill == paper && r.rect.height() > 60.0)),
                "no tile wears {scheme:?}'s own paper"
            );
        }
        // A dialog does not scroll: it has to fit the window it is given.
        for shape in &shapes {
            let rect = shape.shape.visual_bounding_rect();
            if !rect.min.is_finite() || !rect.max.is_finite() {
                continue;
            }
            assert!(
                rect.min.x >= -1.0
                    && rect.max.x <= 421.0
                    && rect.min.y >= -1.0
                    && rect.max.y <= 621.0,
                "the app's settings stay in their window: {rect:?}"
            );
        }
    }

    #[test]
    fn the_card_separates_a_set_up_box_from_one_that_merely_looks_it() {
        assert_eq!(
            reader(false, false, true),
            Reader::None,
            "no reader, no offer"
        );
        assert_eq!(
            reader(true, true, false),
            Reader::Set,
            "a seal is the truth, whatever the pref says now"
        );
        assert_eq!(
            reader(true, false, true),
            Reader::Pending,
            "asked for, and the password has not worked yet"
        );
        assert_eq!(reader(true, false, false), Reader::Off);

        let said = [
            Reader::None.status(),
            Reader::Set.status(),
            Reader::Pending.status(),
            Reader::Off.status(),
        ];
        assert!(
            said.iter().all(|s| !s.is_empty()),
            "every state says something"
        );
        assert!(
            said.iter().collect::<std::collections::HashSet<_>>().len() == 4,
            "and no two say the same: {said:?}"
        );
    }

    /// The browser card owes the same, for the other reason: a tick cannot say whose
    /// keyboard the master password lands on, and with nothing allowed it must not
    /// point at a chip the bar is not painting.
    #[test]
    fn the_browser_card_reads_its_own_state() {
        assert_ne!(open_by_browser(false), open_by_browser(true));
        assert!(
            open_by_browser(true).contains("master password"),
            "the switch exists only to put the password in the browser"
        );
        assert!(
            open_by_browser(false).contains("never opens one of its own"),
            "off is the promise that a browser stays a reader"
        );
        assert!(
            let_in(0).contains("no browser is allowed"),
            "with none there is no chip to point at: {}",
            let_in(0)
        );
        assert_eq!(
            let_in(1),
            let_in(2),
            "how many are allowed is the chip's job, not this line's"
        );
    }
}
