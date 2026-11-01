//! The app's end of the bridge: the socket it listens on so the native host can hand
//! it a request instead of opening the vault file itself.
//!
//! The point of it is the master password: when this is listening there is no reason
//! for the browser to ask for one, and none for it to hold. The box is already open
//! here; the extension asks *this* process, the way KeePassXC asks its own app.

pub mod logic;
pub mod model;

pub use model::{Bridge, Effect};

/// The prompt a browser the app has not seen before gets.
use crate::app::{App, StatusKind};
use crate::shared::theme;
use egui::RichText;

pub fn modal(ctx: &egui::Context, app: &mut App) {
    let Some(caller) = app.bridge.asking().map(str::to_owned) else {
        return;
    };
    let mut remember = app.bridge.remember;
    let mut decision: Option<bool> = None;

    theme::dialog(ctx, egui::Id::new("bridge-approval"), |ui| {
        theme::width(ui, 380.0);
        if theme::head(ui, &app.icons, "Browser") {
            decision = Some(false);
        }
        ui.label(RichText::new(&caller).monospace());
        ui.add_space(8.0);
        theme::hint(
            ui,
        "Whatever the browser called it. Allow it and it can read the box you have open: a site's entries, and one password after you pick one.",
        );
        ui.add_space(12.0);
        theme::check(ui, &mut remember, "Remember this browser");
        ui.add_space(12.0);
        let (allowed, refused) = theme::actions(
            ui,
            ctx,
            |ui| choice(ui, "Allow", theme::ACCENT),
            |ui| choice(ui, "Deny", theme::palette().muted),
        );
        if allowed {
            decision = Some(true);
        }
        if refused {
            decision = Some(false);
        }
        decision.is_some()
    });

    if let Some(allow) = decision {
        app.bridge.remember = remember;
        let remembered = app
            .bridge
            .resolve(allow, remember, &mut app.boxes, &mut app.prefs);
        if remembered {
            crate::features::settings::model::save(&app.prefs);
        }
        // Say which way it went, so the prompt's answer outlives the prompt.
        let (text, kind) = if allow {
            ("browser allowed to read this box", StatusKind::Saved)
        } else {
            ("browser refused", StatusKind::Info)
        };
        app.shell.set_status(text, kind);
    }
}

/// One button of that row: the shape `Icons::labeled` gives the app's other dialogs,
/// with no glyph. There is no tick in the icon set, and a padlock reads as "the vault
/// is open" rather than "you may talk to it".
fn choice(ui: &mut egui::Ui, text: &str, color: egui::Color32) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(crate::shared::i18n::t(text)).color(color))
            .small()
            .min_size(egui::vec2(0.0, 28.0)),
    )
}
