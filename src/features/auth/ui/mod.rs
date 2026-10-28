//! Welcome (create vault) and Unlock screens.

use crate::app::{App, OpKind, Screen, Side};
use crate::features::settings::model as prefs;
use crate::features::shell::pick;
use crate::shared::i18n::{t, tf};
use crate::shared::theme;
use crate::shared::ui::balance;
use crate::shared::ui::responsive;
#[cfg(not(target_os = "android"))]
use crate::vault::store;
use egui::{Context, Key, RichText, TextEdit, Ui};
use std::path::{Path, PathBuf};

/// The room a field keeps over and under its own text: a phone's line has to
/// stand as tall as the button it shares the row with.
fn room(ui: &Ui) -> i8 {
    if responsive::compact(ui.ctx()) {
        12
    } else {
        9
    }
}

pub(crate) fn wordmark(ui: &mut egui::Ui, icons: &theme::Icons) {
    ui.add(icons.logo(96.0, theme::ACCENT));
    ui.add_space(4.0);
    balance::label(
        ui,
        RichText::new(t("Save your secrets inside Pandora’s box"))
            .small()
            .color(theme::palette().muted),
    );
    ui.add_space(6.0);
}

const TOP: f32 = 32.0;

pub(crate) fn top(ui: &egui::Ui) -> f32 {
    (ui.available_height() * 0.10).clamp(16.0, TOP)
}

pub(crate) fn column(ui: &mut egui::Ui) {
    const GUTTER: f32 = 12.0;
    ui.set_max_width((ui.available_width() - GUTTER * 2.0).min(440.0));
}

/// Path field with a file-picker button; `save_as` writes a new name instead of reading one.
/// The picker runs off the frame thread and leaves its answer in the context's temp
/// data under `id`, which this field takes on the next frame.
pub fn file_field(
    ui: &mut egui::Ui,
    ask: &mut Option<pick::Ask>,
    id: egui::Id,
    icons: &theme::Icons,
    path: &mut String,
    hint: &str,
    save_as: bool,
) {
    let margin = egui::Margin::symmetric(12, room(ui));
    if let Some(picked) = ui.ctx().data_mut(|d| d.remove_temp::<String>(id)) {
        *path = picked;
    }
    // Android has no dialog behind this field: a box file arrives through the
    // activity's own picker (see `auth::pick`), and a keyfile is typed. The path
    // is the app's own, so there is nothing to browse.
    #[cfg(target_os = "android")]
    {
        let _ = (ask, icons, save_as);
        ui.add(
            TextEdit::singleline(path)
                .hint_text(t(hint))
                .desired_width(f32::INFINITY)
                .margin(margin),
        );
    }
    #[cfg(not(target_os = "android"))]
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let side = (ui.text_style_height(&egui::TextStyle::Body) + margin.sum().y)
            .max(responsive::target(ui.ctx()));
        ui.add(
            TextEdit::singleline(path)
                .hint_text(t(hint))
                .desired_width((ui.available_width() - side - gap).max(120.0))
                .margin(margin),
        );
        if icons
            .framed(
                ui,
                theme::Icon::Folder,
                responsive::grow(ui.ctx(), 16.0),
                theme::palette().muted,
                "Choose a file…",
                side,
            )
            .clicked()
        {
            let current = save_as.then(|| PathBuf::from(path.as_str()));
            let answer = ui.ctx().clone();
            pick::launch(
                ask,
                ui.ctx(),
                move || {
                    let picked = match current {
                        Some(current) => {
                            let mut dialog = rfd::FileDialog::new();
                            if let Some(dir) = current.parent().filter(|d| d.is_dir()) {
                                dialog = dialog.set_directory(dir);
                            }
                            dialog.set_file_name(default_name(&current)).save_file()
                        }
                        None => rfd::FileDialog::new().pick_file(),
                    };
                    picked.into_iter().collect()
                },
                move |_app, mut paths| {
                    if let Some(path) = paths.pop() {
                        answer.data_mut(|d| d.insert_temp(id, path.display().to_string()));
                    }
                },
            );
        }
    });
}

#[cfg(not(target_os = "android"))]
fn default_name(path: &Path) -> String {
    match path.file_name().and_then(|s| s.to_str()) {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => format!("vault.{}", store::EXT),
    }
}

/// A password field with its reveal toggle: the eye keeps the square the file
/// picker wears, so the row ends on the card's edge either way. The reveal lives
/// in the context's temp data under the field's own id, so it survives the frame
/// and forgets itself whenever the app clears its data.
pub fn secret_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    icons: &theme::Icons,
    text: &mut String,
    hint: &str,
) -> egui::Response {
    let margin = egui::Margin::symmetric(12, room(ui));
    let mut show = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(id))
        .unwrap_or_default();
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        // egui rounds a button's padding to whole pixels, so the eye's square is
        // only its own side once that rounding is applied. The field gives up
        // exactly that much: a hair less and the row runs past its column.
        let eye = responsive::grow(ui.ctx(), 16.0);
        let floor = ui.text_style_height(&egui::TextStyle::Body) + margin.sum().y;
        let side = (eye + 2.0 * ((floor - eye) / 2.0).round()).max(responsive::target(ui.ctx()));
        let resp = ui.add(
            TextEdit::singleline(text)
                .id(id)
                .password(!show)
                .hint_text(t(hint))
                .desired_width((ui.available_width() - side - gap).max(0.0))
                .margin(margin),
        );
        if icons
            .framed(
                ui,
                if show {
                    theme::Icon::EyeOff
                } else {
                    theme::Icon::Eye
                },
                eye,
                theme::palette().muted,
                if show { "hide" } else { "show" },
                side,
            )
            .clicked()
        {
            show = !show;
            ui.ctx().data_mut(|d| d.insert_temp(id, show));
        }
        resp
    })
    .inner
}

/// A secret field and its confirmation, one above the other. They never share a
/// row, however wide the form is: the hint ("master password (min 12)") and the
/// reveal eye need the field's whole width to stay readable, and a confirmation
/// drawn beside its password is a row the eye skips. The response is the
/// confirmation's, so the form still submits on Enter from the second field.
pub fn secret_pair(
    ui: &mut egui::Ui,
    icons: &theme::Icons,
    first: (egui::Id, &mut String, &str),
    second: (egui::Id, &mut String, &str),
) -> egui::Response {
    let (first_id, first_text, first_hint) = first;
    let (second_id, second_text, second_hint) = second;
    secret_field(ui, first_id, icons, first_text, first_hint);
    secret_field(ui, second_id, icons, second_text, second_hint)
}

pub fn welcome(ui: &mut egui::Ui, app: &mut App) {
    let busy = app
        .auth
        .op
        .as_ref()
        .is_some_and(|o| o.kind == OpKind::Create);
    let m = egui::Margin::symmetric(12, 9);
    // A form page scrolls like settings and the generator, but with the bar only
    // when it is needed: a card that fits does not wear one. The pane is what
    // gives the page's bar a gutter to sit in when it does come up.
    let compact = responsive::compact(ui.ctx());
    egui::CentralPanel::default()
        .frame(theme::pane(compact))
        .show(ui, |ui| {
            theme::scroll_when_needed(
                ui,
                egui::ScrollArea::vertical()
                    // Each page keeps its own place: the two share the panel, so a
                    // page scrolled to its button must not open the other one there.
                    .id_salt("welcome")
                    .auto_shrink([false, false]),
                |ui| {
                    ui.add_space(top(ui));
                    ui.vertical_centered(|ui| {
                        // The pane already holds the page off the window's edge, so the
                        // form does not ask `column` for a second gutter: 440 is the
                        // width this card has always had.
                        ui.set_max_width(ui.available_width().min(440.0));
                        wordmark(ui, &app.icons);
                        theme::card(ui, "", |ui| {
                            let mut submit = false;
                            ui.add_enabled_ui(!busy, |ui| {
                                ui.add(
                                    TextEdit::singleline(&mut app.auth.welcome.name)
                                        .hint_text(t("box name"))
                                        .desired_width(f32::INFINITY)
                                        .margin(m),
                                );
                                ui.add_space(12.0);
                                file_field(
                                    ui,
                                    &mut app.ask,
                                    egui::Id::new("welcome-path"),
                                    &app.icons,
                                    &mut app.auth.welcome.path,
                                    "box file",
                                    true,
                                );
                                #[cfg(target_os = "android")]
                                {
                                    if theme::secondary_button(
                                        ui,
                                        &app.icons,
                                        Some(theme::Icon::Folder),
                                        "Choose destination…",
                                    )
                                    .clicked()
                                    {
                                        app.create_box_destination();
                                    }
                                    ui.add_space(6.0);
                                }
                                ui.add_space(12.0);
                                let confirm = secret_pair(
                                    ui,
                                    &app.icons,
                                    (
                                        egui::Id::new("welcome-password"),
                                        &mut app.auth.welcome.password,
                                        "master password (min 12)",
                                    ),
                                    (
                                        egui::Id::new("welcome-confirm"),
                                        &mut app.auth.welcome.confirm,
                                        "confirm",
                                    ),
                                );
                                submit = ui.ctx().input(|i| i.key_pressed(Key::Enter))
                                    && confirm.lost_focus();
                                ui.add_space(8.0);
                                theme::strength_meter(ui, &app.auth.welcome.password);
                                ui.add_space(12.0);
                                file_field(
                                    ui,
                                    &mut app.ask,
                                    egui::Id::new("welcome-keyfile"),
                                    &app.icons,
                                    &mut app.auth.welcome.keyfile,
                                    "keyfile (optional)",
                                    false,
                                );
                                ui.add_space(16.0);
                            });
                            if theme::primary_button(
                                ui,
                                &app.icons,
                                None,
                                "Create box",
                                busy.then_some("Creating box…"),
                            )
                            .clicked()
                                || submit
                            {
                                app.auth.create_vault(&mut app.shell);
                            }
                        });
                        ui.add_space(18.0);
                        if ui.link(t("I have a box file — open it")).clicked() {
                            app.screen = Screen::Unlock;
                        }
                        ui.add_space(6.0);
                        if ui.link(t("Keep the box in a GitHub repo")).clicked() {
                            app.auth.side = Some(Side::Github);
                            app.screen = Screen::Sources;
                            if !app.sync.signed() {
                                app.github_login();
                            }
                        }
                    });
                },
            );
        });
}

pub fn unlock(ui: &mut egui::Ui, app: &mut App) {
    let ctx: Context = ui.ctx().clone();
    // Any of the three — the derivation, the reader's prompt, the picker — owns
    // the form while it runs, so none of them can start on top of another.
    let busy = app.auth.op.is_some();
    let wait = app
        .prefs
        .wait(&prefs::box_key(app.auth.unlock_path.trim()), prefs::now());
    let pw_id = password_id();
    // Without a repaint request the countdown sits stale until the next mouse move.
    if wait > 0 {
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
    egui::CentralPanel::default()
        .frame(theme::pane(responsive::compact(ui.ctx())))
        .show(ui, |ui| {
            theme::scroll_when_needed(
                ui,
                egui::ScrollArea::vertical()
                    // Its own place, like the create page's: see `welcome`.
                    .id_salt("unlock")
                    .auto_shrink([false, false]),
                |ui| {
                    ui.add_space(top(ui));
                    ui.vertical_centered(|ui| {
                        // The pane already holds the page off the window's edge, so the
                        // form does not ask `column` for a second gutter: 440 is the
                        // width this card has always had.
                        ui.set_max_width(ui.available_width().min(440.0));
                        wordmark(ui, &app.icons);
                        theme::card(ui, "", |ui| {
                            let mut submit = false;
                            ui.add_enabled_ui(!busy, |ui| {
                                file_field(
                                    ui,
                                    &mut app.ask,
                                    egui::Id::new("unlock-path"),
                                    &app.icons,
                                    &mut app.auth.unlock_path,
                                    "box file",
                                    false,
                                );
                                #[cfg(target_os = "android")]
                                {
                                    if theme::secondary_button(
                                        ui,
                                        &app.icons,
                                        Some(theme::Icon::Folder),
                                        "Choose a box file…",
                                    )
                                    .clicked()
                                    {
                                        app.import_box();
                                    }
                                    ui.add_space(6.0);
                                }
                                ui.add_space(12.0);
                                let resp = secret_field(
                                    ui,
                                    pw_id,
                                    &app.icons,
                                    &mut app.auth.unlock_password,
                                    "master password",
                                );
                                ui.add_space(12.0);
                                file_field(
                                    ui,
                                    &mut app.ask,
                                    egui::Id::new("unlock-keyfile"),
                                    &app.icons,
                                    &mut app.auth.unlock_keyfile,
                                    "keyfile (if this box uses one)",
                                    false,
                                );
                                ui.add_space(12.0);
                                secret_field(
                                    ui,
                                    egui::Id::new("unlock-code"),
                                    &app.icons,
                                    &mut app.auth.unlock_code,
                                    "recovery code (when the password is gone)",
                                );
                                ui.add_space(18.0);
                                submit =
                                    ctx.input(|i| i.key_pressed(Key::Enter)) && resp.lost_focus();
                            });

                            let waiting =
                                (wait > 0).then(|| tf("wait {wait} s", &[("wait", &wait)]));
                            let go = theme::primary_button(
                                ui,
                                &app.icons,
                                Some(theme::Icon::LockKeyholeOpen),
                                "Unlock",
                                busy.then_some("Unlocking…").or(waiting.as_deref()),
                            );
                            if (go.clicked() || submit) && wait == 0 {
                                app.unlock();
                            }
                            // The reader, when this box was set up for it: one touch, and the
                            // keystore hands back the key the password would have derived.
                            if sealed(app) {
                                ui.add_space(8.0);
                                if theme::secondary_button(
                                    ui,
                                    &app.icons,
                                    None,
                                    "Unlock with fingerprint",
                                )
                                .clicked()
                                {
                                    app.unlock_biometric();
                                }
                            }
                        });
                        ui.add_space(18.0);

                        if ui.link(t("Create a new box")).clicked() {
                            app.screen = Screen::Welcome;
                        }
                    });
                },
            );
        });
}

/// Is this box set up to open with the device's reader?
fn sealed(app: &App) -> bool {
    let path = app.auth.unlock_path.trim();
    !path.is_empty()
        && crate::features::auth::biometric::available()
        && crate::features::auth::biometric::sealed(Path::new(path))
}

/// The password field's id, so the sources screen can hand focus to it.
pub(crate) fn password_id() -> egui::Id {
    egui::Id::new("unlock-password")
}

#[cfg(test)]
mod tests;
