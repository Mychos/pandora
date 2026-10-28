//! What an unfocused window shows: only the logo, so a screen share or a look
//! over a shoulder catches nothing of the box. Focusing the window paints it
//! again; the app keeps running behind the curtain.

use crate::app::App;
use crate::shared::theme;
use egui::{Id, Order};

pub fn show(ctx: &egui::Context, app: &App) {
    let screen = ctx.viewport_rect();
    egui::Area::new(Id::new("cover"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .constrain_to(screen)
        .fade_in(false)
        .show(ctx, |ui| {
            ui.set_width(screen.width());
            ui.set_height(screen.height());
            ui.painter()
                .rect_filled(screen, egui::CornerRadius::ZERO, theme::palette().paper);
            let mut middle = ui.new_child(egui::UiBuilder::new().max_rect(screen).layout(
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
            ));
            middle.add(app.icons.logo(96.0, theme::ACCENT));
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::ui::shot;

    #[test]
    fn an_unfocused_window_shows_only_the_logo_until_it_gets_focus_back() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.prefs.hide_unfocused = true;

        let mut away = shot::input(egui::vec2(420.0, 520.0), Vec::new());
        away.focused = false;
        let mut out = ctx.run_ui(away, |ui| app.run_ui(ui));
        out.textures_delta.clear();
        let hidden = out.shapes;
        assert!(
            shot::words(&hidden).is_empty(),
            "an unfocused window leaves only the logo"
        );

        // The compositor owns the pointer during a move, so focus reads as lost;
        // that gesture is not the user walking away.
        app.shell.moved();
        let mut moving = shot::input(egui::vec2(420.0, 520.0), Vec::new());
        moving.focused = false;
        let mut out = ctx.run_ui(moving, |ui| app.run_ui(ui));
        out.textures_delta.clear();
        assert!(
            !shot::words(&out.shapes).is_empty(),
            "a window being moved stays visible"
        );
        app.shell.settle(true, false);

        let shown = shot::frame(&ctx, egui::vec2(420.0, 520.0), Vec::new(), |ui| {
            app.run_ui(ui)
        });
        assert!(
            !shot::words(&shown).is_empty(),
            "focus paints the window again"
        );
    }
}
