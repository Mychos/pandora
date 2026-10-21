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
    theme::scroll(
        ui,
        egui::ScrollArea::vertical().auto_shrink([false, false]),
        |ui| {
            let card_w = ui.available_width().max(0.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                egui::Frame::default()
                    .fill(theme::palette().panel)
                    .corner_radius(theme::radius(10))
                    .stroke(theme::stroke(theme::palette().hair, 1.0))
                    .inner_margin(egui::Margin::symmetric(24, 20))
                    .show(ui, |ui| {
                        ui.set_max_width((card_w - 48.0).max(0.0));
                        add(ui, &selected);
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
    ui.with_layout(egui::Layout::left_to_right(align), |ui| {
        let w = width(ui, marks, least);
        let out = field(ui, w, *shown);
        let (toggle, copied) = secret_actions(ui, icons, *shown);
        if toggle {
            *shown = !*shown;
        }
        (out, copied)
    })
    .inner
}

/// `gap`, then the delete mark. What a deletion means belongs to the feature.
pub fn delete(ui: &mut Ui, icons: &theme::Icons, gap: f32) -> bool {
    ui.add_space(gap);
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
    }
}
