//! Window chrome geometry: no OS title bar, so the app paints its own strip at the
//! window's top — where a phone's own navigation is answered by a bar at its
//! floor — and the body dialogs live in. The bars and the dialog chrome both read
//! this.

use egui::{vec2, Context, Id, Rect};

/// The strip the app paints in place of a title bar under a pointer: the window's
/// own edge, with Back and the tools on it.
pub const HEIGHT: f32 = 30.0;

/// The same strip under a finger, and the app's own navigation at the floor of a
/// narrow window: Back and the panes have to be a target a thumb can land on, so
/// the bar grows.
pub const NAV: f32 = 56.0;

/// Whether a finger drives the chrome: a window narrow enough to be a phone, or a
/// phone itself, which is always narrow enough whatever it reports.
pub fn touch(ctx: &Context) -> bool {
    crate::shared::ui::responsive::compact(ctx) || cfg!(target_os = "android")
}

/// The bar's own height on the screen in front of us: it stands on the window's
/// top, and a thumb gets the taller one.
pub fn strip(ctx: &Context) -> f32 {
    if touch(ctx) {
        NAV
    } else {
        HEIGHT
    }
}

/// What the app keeps under the body: the navigation a narrow screen holds at its
/// floor, plus the phone's own bar it stands on. Nothing on a wide one, where the
/// bar is the top.
pub fn floor(ctx: &Context) -> f32 {
    if !touch(ctx) {
        return 0.0;
    }
    #[cfg(target_os = "android")]
    {
        NAV + crate::shared::android::footer()
    }
    #[cfg(not(target_os = "android"))]
    {
        NAV
    }
}

/// Where a modal lives: the bar and the floor are the frames the body is not
/// allowed to touch, so a tall one does not end up flush against either.
pub fn body(ctx: &Context) -> Rect {
    let rect = ctx.content_rect();
    Rect::from_min_max(
        rect.min + vec2(0.0, strip(ctx)),
        rect.max - vec2(0.0, floor(ctx)),
    )
}

/// The room a dialog keeps from the window's edge: a phone's body is the whole
/// screen, so without it a modal would sit flush against both sides. It is taken
/// off the width the card asks for (`theme::width`), never off the area the modal
/// lives in: that area is what the backdrop is painted through, and a backdrop
/// with a margin is not a backdrop.
pub const GAP: f32 = 16.0;

pub fn modal(ctx: &Context, id: Id) -> egui::Modal {
    egui::Modal::new(id).area(egui::Modal::default_area(id).constrain_to(body(ctx)))
}

#[cfg(test)]
mod tests {
    use crate::shared::ui::shot;

    fn room(width: f32, height: f32) -> (egui::Context, egui::Rect) {
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        let mut out = ctx.run_ui(shot::input(egui::vec2(width, height), Vec::new()), |ui| {
            rect = super::body(ui.ctx())
        });
        out.textures_delta.clear();
        (ctx, rect)
    }

    /// A wide window keeps the bar on its top: the body starts under it and runs
    /// to the window's own floor.
    #[test]
    fn the_body_keeps_clear_of_the_top_bar() {
        let (ctx, rect) = room(1000.0, 700.0);
        assert_eq!(super::strip(&ctx), super::HEIGHT);
        assert_eq!(super::floor(&ctx), 0.0);
        assert_eq!(rect.min.y, super::HEIGHT, "the body starts under the bar");
        assert_eq!(rect.max.y, 700.0, "nothing is reserved at the floor");
    }

    /// A narrow one hands the panes to its floor, so the body gives up both bars:
    /// the top one and the navigation below.
    #[test]
    fn the_body_keeps_clear_of_both_bars() {
        let (ctx, rect) = room(420.0, 780.0);
        assert_eq!(super::strip(&ctx), super::NAV);
        assert_eq!(super::floor(&ctx), super::NAV);
        assert_eq!(rect.min.y, super::NAV, "the body starts under the bar");
        assert_eq!(
            rect.max.y,
            780.0 - super::NAV,
            "the body ends above the navigation"
        );
        assert!(
            rect.height() < 780.0 - super::HEIGHT - super::NAV + 1.0,
            "a tall modal would sit flush against a bar: {rect:?}"
        );
    }
}
