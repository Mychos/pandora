//! The strip a new release wears, and the settings row that can turn the check off.

use crate::app::App;
use crate::features::update::model::{State, Update};
use crate::shared::i18n::{t, tf};
use crate::shared::theme;
use egui::{RichText, Ui};

/// Draw the strip for a release the user has not dismissed. Nothing is drawn when
/// a check found none, or when the one it found is the one they already said
/// "Later" to.
pub fn strip(ui: &mut Ui, app: &mut App) {
    let Some(info) = app.update.info.clone() else {
        return;
    };
    if app.prefs.skip == info.latest {
        return;
    }

    let downloading = matches!(app.update.state, State::Downloading(_));
    let message = match progress(&app.update) {
        Some(percent) => tf("updating… {percent}%", &[("percent", &percent)]),
        None if downloading => t("updating…").to_owned(),
        None => tf("{version} is out", &[("version", &info.latest)]),
    };
    // The mark says what the click does: fetch the release, or open its page.
    let mark = if info.installable {
        theme::Icon::Download
    } else {
        theme::Icon::Globe
    };

    let mut install = false;
    let mut page = false;
    let mut later = false;
    egui::Panel::top("update")
        .frame(
            egui::Frame::NONE
                .fill(theme::palette().elev)
                .inner_margin(egui::Margin::symmetric(18, 8)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(image) = app.icons.image(mark, 15.0, theme::ACCENT) {
                    ui.add(image);
                }
                ui.add(
                    egui::Label::new(RichText::new(message).color(theme::palette().text)).wrap(),
                );
                if downloading {
                    return;
                }
                // The bar reads left to right like a sentence, and the ways out sit at
                // its end: the one the user came for, then the one that closes it.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if action(ui, &app.icons, "Later", false) {
                        later = true;
                    }
                    if info.installable {
                        if action(ui, &app.icons, "Update", true) {
                            install = true;
                        }
                    } else if action(ui, &app.icons, "Open release page", true) {
                        page = true;
                    }
                });
            });
        });

    if install {
        app.install_update(ui.ctx());
    }
    if page {
        app.open_page(&info.url);
    }
    if later {
        // Said once: the next launch keeps quiet about this version and no other.
        app.prefs.skip = info.latest;
        crate::features::settings::model::save(&app.prefs);
    }
}

/// A button that takes the room of its label, not the room of the row: the theme's
/// own take the whole width, which a form wants and a bar cannot give twice. No mark
/// of its own: the bar already carries the one that says what a release is.
fn action(ui: &mut Ui, icons: &theme::Icons, text: &str, primary: bool) -> bool {
    let palette = theme::palette();
    let (fill, ink) = if primary {
        (theme::ACCENT, palette.paper)
    } else {
        (egui::Color32::TRANSPARENT, palette.text)
    };
    let atoms = icons.trailing(None, 13.0, ink, RichText::new(t(text)).strong().color(ink));
    let stroke = if primary {
        egui::Stroke::NONE
    } else {
        theme::stroke(palette.hair, 1.0)
    };
    ui.add(
        egui::Button::new(atoms)
            .fill(fill)
            .stroke(stroke)
            .min_size(egui::vec2(0.0, 30.0)),
    )
    .clicked()
}

/// The version line's own door in the settings: look now, and the switch that
/// lets the app look by itself. Nothing rides on the answer but a notice.
pub fn row(ui: &mut Ui, app: &mut App) {
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button(t("Check for updates")).clicked() {
            app.update.check(ui.ctx());
        }
        theme::hint(ui, &status(&app.update));
    });
    let mut on = app.prefs.update_check;
    if theme::check(ui, &mut on, "check for new versions on launch").changed() {
        app.prefs.update_check = on;
        crate::features::settings::model::save(&app.prefs);
    }
}

fn progress(update: &Update) -> Option<u64> {
    match update.state {
        State::Downloading(progress) => progress.percent(),
        _ => None,
    }
}

/// What the check last did, in the row's own words.
fn status(update: &Update) -> String {
    match &update.state {
        State::Checking => t("checking…").to_owned(),
        State::Downloading(progress) => match progress.percent() {
            Some(percent) => tf("updating… {percent}%", &[("percent", &percent)]),
            None => t("updating…").to_owned(),
        },
        State::Failed(why) => tf(
            "could not look for a new version: {error}",
            &[("error", why)],
        ),
        State::Idle => match (&update.info, update.checked) {
            (Some(info), _) => tf(
                "Pandora {version} is available",
                &[("version", &info.latest)],
            ),
            (None, true) => t("up to date").to_owned(),
            (None, false) => String::new(),
        },
    }
}
