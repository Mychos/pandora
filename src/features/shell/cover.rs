//! What an unfocused window shows: only the logo, so a screen share or a look
//! over a shoulder catches nothing of the box. Focusing the window paints it
//! again; the app keeps running behind the curtain.

use crate::app::App;
use crate::shared::theme;
use egui::{Id, Order};

/// Paints the curtain and answers whether the pointer went down on it: the click
/// that brings the window back is spent here, not on what it was hiding.
pub fn show(ctx: &egui::Context, app: &App) -> bool {
    let screen = ctx.viewport_rect();
    let mut pressed = false;
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
            // A widget, not a painting: without something here the press falls through
            // to nothing, and the release that follows focus lands on whatever it hid.
            pressed = ui
                .allocate_rect(screen, egui::Sense::click())
                .is_pointer_button_down_on();
            let mut middle = ui.new_child(egui::UiBuilder::new().max_rect(screen).layout(
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
            ));
            middle.add(app.icons.logo(96.0, theme::ACCENT));
        });
    pressed
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
    /// The click that brings the focus back never reaches the content, whichever frame
    /// it arrives in: the curtain outlives the focus by a moment, and while it is up
    /// there is no widget drawn under it to take the click.
    #[test]
    fn the_click_that_brings_the_focus_back_never_reaches_the_content() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.prefs.hide_unfocused = true;
        app.screen = crate::app::Screen::Welcome;

        let size = egui::vec2(420.0, 520.0);
        let at = egui::pos2(210.0, 260.0);
        let frame = |app: &mut App, focused: bool, events: Vec<egui::Event>| {
            let mut input = shot::input(size, events);
            input.focused = focused;
            let mut out = ctx.run_ui(input, |ui| app.run_ui(ui));
            out.textures_delta.clear();
            out
        };

        let away = frame(&mut app, false, Vec::new());
        assert!(
            shot::words(&away.shapes).is_empty(),
            "an unfocused window leaves only the logo"
        );

        // The compositor hands the focus over first, the click a frame later: that is
        // the sequence that used to paint the content under the pointer.
        let back = frame(&mut app, true, Vec::new());
        assert!(
            shot::words(&back.shapes).is_empty(),
            "the curtain outlives the focus, so the click has not landed on anything yet"
        );

        let click = frame(
            &mut app,
            true,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        assert!(
            shot::words(&click.shapes).is_empty(),
            "the click finds the curtain, not the field it was aimed at"
        );
        assert_eq!(
            ctx.memory(|mem| mem.focused()),
            None,
            "and nothing took the keyboard: no field, no button"
        );
        assert!(!app.prefs_open, "no action ran behind the curtain");

        std::thread::sleep(std::time::Duration::from_millis(230));
        let after = frame(&mut app, true, Vec::new());
        assert!(
            !shot::words(&after.shapes).is_empty(),
            "and the window paints again once the click is over"
        );
    }

    /// A covered window keeps waking itself. The curtain lifts on a clock, so a window
    /// that sleeps until the next mouse move would hold it for as long as the user
    /// takes to touch the pointer — which reads as the app not reacting at all.
    #[test]
    fn a_covered_window_asks_to_be_woken() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.prefs.hide_unfocused = true;

        let mut away = shot::input(egui::vec2(420.0, 520.0), Vec::new());
        away.focused = false;
        let mut out = ctx.run_ui(away, |ui| app.run_ui(ui));
        out.textures_delta.clear();
        let delay = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(std::time::Duration::MAX, |o| o.repaint_delay);
        assert!(
            delay <= std::time::Duration::from_millis(60),
            "a covered window must wake on its own, not on a mouse move: {delay:?}"
        );
    }

    /// A press aimed at the curtain asks the window manager for the window back.
    #[test]
    fn a_press_on_the_curtain_asks_for_the_focus() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.prefs.hide_unfocused = true;

        let size = egui::vec2(420.0, 520.0);
        let at = egui::pos2(210.0, 260.0);
        let mut away = shot::input(size, Vec::new());
        away.focused = false;
        ctx.run_ui(away, |ui| app.run_ui(ui)).textures_delta.clear();

        // egui learns where the pointer is one frame after it moves.
        for _ in 0..2 {
            let mut press = shot::input(
                size,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
            );
            press.focused = false;
            let mut out = ctx.run_ui(press, |ui| app.run_ui(ui));
            out.textures_delta.clear();
            let asked = out
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .is_some_and(|o| {
                    o.commands
                        .iter()
                        .any(|c| matches!(c, egui::ViewportCommand::Focus))
                });
            if asked {
                return;
            }
        }
        panic!("a press on the curtain has to ask for the focus");
    }
}
