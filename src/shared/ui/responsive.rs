use egui::Context;

pub const BREAKPOINT: f32 = 620.0;

pub fn compact(ctx: &Context) -> bool {
    ctx.viewport_rect().width() < BREAKPOINT
}

pub fn glyph(ctx: &Context) -> f32 {
    if compact(ctx) {
        18.0
    } else {
        15.0
    }
}

pub fn target(ctx: &Context) -> f32 {
    if compact(ctx) {
        44.0
    } else {
        28.0
    }
}

pub fn row(ctx: &Context) -> f32 {
    if compact(ctx) {
        42.0
    } else {
        34.0
    }
}

pub fn grow(ctx: &Context, size: f32) -> f32 {
    if compact(ctx) {
        size.max(glyph(ctx))
    } else {
        size
    }
}

pub fn grow_target(ctx: &Context, side: f32) -> f32 {
    if compact(ctx) {
        side.max(target(ctx))
    } else {
        side
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(width: f32) -> egui::Context {
        let ctx = egui::Context::default();
        let _ = crate::shared::ui::shot::frame(&ctx, egui::vec2(width, 800.0), Vec::new(), |_| {});
        ctx
    }

    #[test]
    fn a_finger_gets_a_bigger_glyph_than_a_pointer() {
        let phone = at(420.0);
        let desk = at(1000.0);
        assert!(compact(&phone) && !compact(&desk));
        assert_eq!(glyph(&phone), 18.0);
        assert_eq!(glyph(&desk), 15.0);
        assert!(
            glyph(&phone) - glyph(&desk) >= 3.0,
            "the glyph is visibly bigger"
        );
        assert!(target(&phone) >= 40.0, "a target a thumb can land on");
        assert!(row(&phone) >= 40.0, "a row a thumb can land on");
        assert!(
            target(&desk) < 32.0 && row(&desk) < 40.0,
            "the desktop is unchanged"
        );
    }
}
