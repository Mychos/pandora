//! The strip a new release wears, and the settings row that can turn the check off.

use crate::app::App;
use crate::features::shell::screen::notice;
use crate::features::update::model::{State, Update};
use crate::shared::i18n::{t, tf};
use crate::shared::theme;
use egui::Ui;

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

    let mut install = false;
    let mut page = false;
    let mut later = false;
    notice(ui, "update", &message, |ui| {
        if downloading {
            return;
        }
        // A build from a package manager cannot replace its own file: the only
        // honest offer is the page the release is on.
        if info.installable {
            if ui.button(t("Update")).clicked() {
                install = true;
            }
        } else if ui.button(t("Open release page")).clicked() {
            page = true;
        }
        if ui.button(t("Later")).clicked() {
            later = true;
        }
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
