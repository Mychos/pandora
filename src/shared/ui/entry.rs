//! The glyph, eye/copy marks and card every record row and editor share.

use crate::shared::theme;
use crate::shared::ui::responsive;
use crate::vault::model::Kind;
use egui::Ui;

/// The centred card a record's editor sits in, or the hint to pick one.
pub fn pane<T>(ui: &mut Ui, selected: Option<T>, add: impl FnOnce(&mut Ui, &T)) {
    let Some(selected) = selected else {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new(crate::shared::i18n::t("Nothing selected"))
                    .color(theme::palette().muted),
            );
        });
        return;
    };
    theme::scroll_when_needed(
        ui,
        egui::ScrollArea::vertical().auto_shrink([false, false]),
        |ui| {
            let card_w = ui.available_width().max(0.0);
            // The card fills the pane down to the window's edge, whatever the record's
            // own shape leaves unused: a short pane still reads as one card, not a slip.
            // What egui keeps around the card — the spacing it leaves after the last
            // row, inside the card and in the scroll area — plus its hairline and a
            // point of rounding, is given back, so the card never grows a bar of its own.
            let margin = egui::Margin::symmetric(24, 20);
            let air = 2.0 * ui.spacing().item_spacing.y + 4.0;
            let room = (ui.available_height() - margin.sum().y - air).max(0.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                egui::Frame::default()
                    .fill(theme::palette().panel)
                    .corner_radius(theme::radius(10))
                    .stroke(theme::stroke(theme::palette().hair, 1.0))
                    .inner_margin(margin)
                    .show(ui, |ui| {
                        ui.set_max_width((card_w - 48.0).max(0.0));
                        // The card is centred, what it holds is not: a label or a hint
                        // takes the card's left edge, as the form's fields do. Its height
                        // is held to the room, so what a record fills stops at the card's
                        // floor instead of reaching past it for the pane's bottom.
                        ui.set_height(room);
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            add(ui, &selected)
                        });
                    });
            });
        },
    );
}

pub fn glyph(
    ui: &mut Ui,
    icons: &theme::Icons,
    kind: Kind,
    brand: Option<theme::Brand>,
    size: f32,
) {
    if let Some(img) = icons.glyph(brand, theme::kind_icon(kind), size, theme::tag_color(kind)) {
        ui.add(img);
    }
}

/// Eye + copy buttons for a secret field. Returns `(toggle, copied)`.
pub fn secret_actions(ui: &mut Ui, icons: &theme::Icons, shown: bool) -> (bool, bool) {
    let toggle = icons
        .button(
            ui,
            if shown {
                theme::Icon::EyeOff
            } else {
                theme::Icon::Eye
            },
            15.0,
            theme::palette().muted,
            if shown { "hide" } else { "show" },
        )
        .clicked();
    let copied = icons
        .button(ui, theme::Icon::Copy, 15.0, theme::palette().muted, "copy")
        .clicked();
    (toggle, copied)
}

/// Room left for a field after `marks` squares and the gaps between them, never
/// below `least`.
pub fn width(ui: &Ui, marks: usize, least: f32) -> f32 {
    let square = responsive::target(ui.ctx()) + ui.spacing().item_spacing.x;
    (ui.available_width() - marks as f32 * square).max(least)
}

/// A field that hides what it holds, with the eye and copy marks closing its row;
/// `true` comes back when copy was pressed, and the clipboard belongs to a feature.
#[allow(clippy::too_many_arguments)] // the row's own geometry: marks, floor, align
pub fn secret<R>(
    ui: &mut Ui,
    icons: &theme::Icons,
    shown: &mut bool,
    marks: usize,
    least: f32,
    align: egui::Align,
    field: impl FnOnce(&mut Ui, f32, bool) -> R,
) -> (R, bool) {
    // Starting at a row's own height keeps the row from filling the card's leftover
    // height; the field then grows it instead.
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
        egui::Layout::left_to_right(align),
        |ui| {
            let w = width(ui, marks, least);
            let out = field(ui, w, *shown);
            let (toggle, copied) = secret_actions(ui, icons, *shown);
            if toggle {
                *shown = !*shown;
            }
            (out, copied)
        },
    )
    .inner
}

/// `gap`, then the delete mark on the card's right edge. What a deletion means
/// belongs to the feature.
pub fn delete(ui: &mut Ui, icons: &theme::Icons, gap: f32) -> bool {
    ui.add_space(gap);
    // A bare `with_layout` under a top-down card takes the card's leftover height;
    // the `horizontal` row keeps the mark to its own square.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            icons
                .button(
                    ui,
                    theme::Icon::Trash,
                    16.0,
                    theme::palette().error,
                    "Delete entry",
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
        })
        .inner
    })
    .inner
}

/// The mark that ends a record's card, in the card's own corner: whatever the record
/// left unused is spent above it, so a short pane still ends on its own trash.
pub fn corner(ui: &mut Ui, icons: &theme::Icons, gap: f32) -> bool {
    let reserve = gap + responsive::target(ui.ctx());
    ui.add_space((ui.available_height() - reserve).max(0.0));
    delete(ui, icons, gap)
}

/// One row of a pane: the shared inset and surface, the hit zone with its focus ring,
/// and the fill and hairline the state picks. `content` returns the rect where the
/// row's own clicks stop (a mark answers its own); the response is the row's, `rect`
/// is what a drop line is measured on.
pub fn row<R>(
    ui: &mut Ui,
    id: egui::Id,
    height: f32,
    sense: egui::Sense,
    selected: bool,
    name: &str,
    content: impl FnOnce(&mut Ui) -> (Option<egui::Rect>, R),
) -> (egui::Response, egui::Rect, R) {
    // Reserved first so the fill and hairline land behind the content, not over the label.
    let surface = theme::Surface::reserve(ui);
    let frame = egui::Frame::default()
        // The pane's own row inset, so every list starts on the same grid.
        .inner_margin(egui::Margin::symmetric(14, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), height),
                egui::Layout::left_to_right(egui::Align::Center),
                content,
            )
            .inner
        });

    let rect = frame.response.rect;
    let (stop, out) = frame.inner;
    // The click zone stops where the marks begin, so their clicks stay theirs.
    let zone = stop.map_or(rect, |mark| {
        egui::Rect::from_min_max(rect.min, egui::pos2(mark.min.x, rect.max.y))
    });
    let resp = ui
        .interact(zone, id, sense)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, name)
    });
    theme::focus(ui, &resp);
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }

    let (fill, line) = theme::state(selected, resp.hovered());
    surface.paint(ui, rect, theme::radius(8), fill, line);
    (resp, rect, out)
}

#[cfg(test)]
mod tests {
    use crate::shared::ui::responsive;
    use crate::shared::ui::shot;

    fn room(ctx: &egui::Context, want: f32, marks: usize, least: f32) -> (f32, f32, f32) {
        let mut out = (0.0, 0.0, 0.0);
        let mut frame = ctx.run_ui(shot::input(egui::vec2(800.0, 200.0), Vec::new()), |ui| {
            ui.set_width(want);
            out = (
                super::width(ui, marks, least),
                ui.available_width(),
                ui.spacing().item_spacing.x,
            );
        });
        frame.textures_delta.clear();
        out
    }

    #[test]
    fn the_marks_take_their_own_square_off_the_field() {
        let ctx = egui::Context::default();
        super::theme::apply(&ctx);

        let (field, avail, gap) = room(&ctx, 400.0, 2, 90.0);
        let square = responsive::target(&ctx) + gap;

        assert_eq!(avail, 400.0, "the fixture is not the row it claims to be");
        assert_eq!(
            field,
            400.0 - 2.0 * square,
            "the marks were not taken off the row"
        );
    }

    #[test]
    fn a_row_too_narrow_for_the_marks_keeps_the_floor() {
        let ctx = egui::Context::default();
        super::theme::apply(&ctx);

        let (field, avail, _) = room(&ctx, 10.0, 2, 90.0);

        assert_eq!(avail, 10.0, "the fixture is not the row it claims to be");
        assert_eq!(field, 90.0, "the field was crushed instead of floored");
    }

    #[test]
    fn the_delete_footer_is_one_mark_that_only_answers_a_press() {
        let ctx = egui::Context::default();
        super::theme::apply(&ctx);
        let icons = super::theme::Icons::load(&ctx);
        let mut pressed = None;
        let size = egui::vec2(900.0, 200.0);

        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = shot::frame(&ctx, size, Vec::new(), |ui| {
                pressed = Some(super::delete(ui, &icons, 20.0));
            });
        }
        let glyphs: Vec<egui::Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.brush.is_some() => Some(r.rect),
                _ => None,
            })
            .collect();

        assert_eq!(pressed, Some(false), "the footer answered without a press");
        assert_eq!(glyphs.len(), 1, "the footer is not one mark: {glyphs:?}");
        assert_eq!(
            glyphs[0].height(),
            16.0,
            "the mark is not 16 tall: {glyphs:?}"
        );
        assert!(
            glyphs[0].right() > size.x - 40.0,
            "the mark is not on the right edge: {glyphs:?}"
        );
    }

    /// A short record still gets a card the pane's own height, not a slip that ends
    /// where its fields do.
    #[test]
    fn the_card_reaches_the_bottom_of_the_pane() {
        let ctx = egui::Context::default();
        super::theme::apply(&ctx);
        let size = egui::vec2(800.0, 600.0);

        let mut shapes = Vec::new();
        for _ in 0..2 {
            shapes = shot::frame(&ctx, size, Vec::new(), |ui| {
                super::pane(ui, Some("one"), |ui, _: &&str| {
                    ui.label("body");
                });
            });
        }
        let card = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == super::theme::palette().panel => Some(r.rect),
                _ => None,
            })
            .max_by(|a, b| a.height().total_cmp(&b.height()))
            .expect("the card is painted");

        assert!(
            card.height() > size.y * 0.9,
            "the card stops short of the pane: {card:?}"
        );
    }

    /// The card's own mark ends it: the trash sits on the floor, hard right, whatever
    /// the record left unused above it.
    #[test]
    fn the_corner_mark_ends_the_card_at_its_bottom_right() {
        let ctx = egui::Context::default();
        super::theme::apply(&ctx);
        let icons = super::theme::Icons::load(&ctx);
        let size = egui::vec2(900.0, 300.0);

        let mut glyphs: Vec<egui::Rect> = Vec::new();
        for _ in 0..3 {
            let shapes = shot::frame(&ctx, size, Vec::new(), |ui| {
                super::corner(ui, &icons, 20.0);
            });
            glyphs = shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r) if r.brush.is_some() => Some(r.rect),
                    _ => None,
                })
                .collect();
        }

        assert_eq!(glyphs.len(), 1, "the corner is one mark: {glyphs:?}");
        let pad = (responsive::target(&ctx) - 16.0) / 2.0;
        let mark = glyphs[0];
        assert!(
            (mark.right() + pad - size.x).abs() <= 2.0,
            "the mark does not hug the right edge: {mark:?}"
        );
        assert!(
            (mark.bottom() + pad - size.y).abs() <= 2.0,
            "the mark does not sit on the card's floor: {mark:?}"
        );
    }
}
