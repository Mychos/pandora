//! Window chrome geometry: the app paints its own top strip and floor navigation.

use egui::{vec2, Context, Id, Rect};

/// Height of the pointer strip that replaces the title bar.
pub const HEIGHT: f32 = 30.0;

/// Height under a finger, and of the floor navigation in a narrow window.
pub const NAV: f32 = 56.0;

pub fn touch(ctx: &Context) -> bool {
    crate::shared::ui::responsive::compact(ctx) || cfg!(target_os = "android")
}

/// The bar's height on this screen.
pub fn strip(ctx: &Context) -> f32 {
    if touch(ctx) {
        NAV
    } else {
        HEIGHT
    }
}

/// What the floor holds: nothing on a wide window, the navigation (and a phone's own bar) on a narrow one.
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

/// The area between the top bar and the floor, where a modal lives.
pub fn body(ctx: &Context) -> Rect {
    let rect = ctx.content_rect();
    Rect::from_min_max(
        rect.min + vec2(0.0, strip(ctx)),
        rect.max - vec2(0.0, floor(ctx)),
    )
}

/// Taken off the width the card asks for, never off the modal's area: that area is
/// what the backdrop is painted through, and a backdrop with a margin is not one.
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

    #[test]
    fn the_body_keeps_clear_of_the_top_bar() {
        let (ctx, rect) = room(1000.0, 700.0);
        assert_eq!(super::strip(&ctx), super::HEIGHT);
        assert_eq!(super::floor(&ctx), 0.0);
        assert_eq!(rect.min.y, super::HEIGHT, "the body starts under the bar");
        assert_eq!(rect.max.y, 700.0, "nothing is reserved at the floor");
    }

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
