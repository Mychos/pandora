use egui::Context;

pub const BREAKPOINT: f32 = 620.0;

pub fn compact(ctx: &Context) -> bool {
    ctx.viewport_rect().width() < BREAKPOINT
}

/// The glyph a control draws inside its target: a phone's mark is a step up from
/// a pointer's, and a step is all it takes — the desk's 15 reads fine once the
/// target around it is a thumb's.
pub fn glyph(ctx: &Context) -> f32 {
    if compact(ctx) {
        18.0
    } else {
        15.0
    }
}

/// The square a control reserves for its own press: a finger needs more than the
/// 28 a mouse aims at, and 44 is the smallest that still lands a thumb — it is
/// also what a phone's floor nav measures itself against.
pub fn target(ctx: &Context) -> f32 {
    if compact(ctx) {
        44.0
    } else {
        28.0
    }
}

/// The height of one row in a list: a thumb picks a record the way it picks a
/// button, so the row grows with the target.
pub fn row(ctx: &Context) -> f32 {
    if compact(ctx) {
        42.0
    } else {
        34.0
    }
}

/// `size`, lifted to [`glyph`] on a phone. A pointer keeps the size its own row
/// asked for — a tighter mark stays tighter — and a finger is never handed less
/// than the target it can land on.
pub fn grow(ctx: &Context, size: f32) -> f32 {
    if compact(ctx) {
        size.max(glyph(ctx))
    } else {
        size
    }
}

/// [`grow`] for a control's own square, measured against [`target`].
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
