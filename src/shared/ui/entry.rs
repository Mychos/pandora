//! The pieces every record's row and editor share: its glyph, the eye and copy
//! pair that guards a secret, and the pane an editor draws its card on. What a
//! record *is* decides the rest, and that lives in the pane that owns it.

use crate::shared::theme;
use crate::vault::model::Kind;
use egui::Ui;

/// The pane a record's editor draws on: the centred card the record sits in, or
/// the hint that asks for one to be picked. The card is measured against the room
/// the pane really has, the bar's gutter already taken off it.
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

/// The mark a record wears: the brand it was given or the URL suggests, else the
/// icon of its kind.
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

/// One row of a pane: the surface reserved behind the content, the frame inset
/// every row shares, the hit zone with its focus ring and pointer, and the fill
/// and hairline the state picks. `content` returns the rect the row's own clicks
/// must stop at — a mark on the right that answers its own clicks — and whatever
/// the caller needs back. The response is the row's, `rect` is what a drop line
/// is measured on.
pub fn row<R>(
    ui: &mut Ui,
    id: egui::Id,
    height: f32,
    sense: egui::Sense,
    selected: bool,
    name: &str,
    content: impl FnOnce(&mut Ui) -> (Option<egui::Rect>, R),
) -> (egui::Response, egui::Rect, R) {
    // Reserved up front so the fill and hairline land behind the row content —
    // painting them afterwards covers the label on hover.
    let surface = theme::Surface::reserve(ui);
    let frame = egui::Frame::default()
        // The pane's own row inset: every list starts on the same grid.
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
    // The row's own click zone stops where the marks begin: interacted after them
    // and over them too, it would take their clicks for itself.
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
