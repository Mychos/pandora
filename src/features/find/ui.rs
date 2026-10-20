//! The palette's window: a field, the rows under it, and the keyboard.

use crate::app::App;
use crate::features::find::logic::find_id;
use crate::shared::theme;
use crate::shared::ui::entry as record;
use crate::shared::ui::responsive;
use egui::{Label, Layout, RichText, Sense};

pub fn modal(ctx: &egui::Context, app: &mut App) {
    if !app.find.open {
        return;
    }
    // The keys first: with the field focused, an arrow would otherwise walk the
    // cursor and Enter would end the field's input instead of choosing a row.
    let mut down = false;
    let mut up = false;
    let mut go = false;
    ctx.input_mut(|i| {
        down = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
        up = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
        go = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
    });

    let rows = app.find_rows();
    if app.find.at >= rows.len() {
        app.find.at = rows.len().saturating_sub(1);
    }
    if down {
        app.find_move(true, rows.len());
    }
    if up {
        app.find_move(false, rows.len());
    }
    if go {
        app.find_choose(&rows);
        return;
    }

    let mut clicked = None;
    let open = theme::dialog(ctx, egui::Id::new("find"), |ui| {
        theme::width(ui, 460.0);
        let closed = theme::head(ui, &app.icons, "Find anything");
        ui.add_space(2.0);
        theme::search(
            ui,
            &app.icons,
            &mut app.find.query,
            find_id(),
            "search the whole box",
        );
        // After the field, not before: the request has to name a widget that is
        // already part of this pass.
        if std::mem::take(&mut app.find.needs_focus) {
            ui.memory_mut(|m| m.request_focus(find_id()));
        }
        ui.add_space(10.0);
        if rows.is_empty() {
            if app.find.query.trim().is_empty() {
                theme::empty(
                    ui,
                    false,
                    "search the whole box",
                    "names, users, addresses, notes, files and codes",
                );
            } else {
                theme::empty(ui, true, "", "");
            }
        } else {
            let height = responsive::row(ui.ctx());
            let glyph = responsive::glyph(ui.ctx());
            theme::scroll(ui, egui::ScrollArea::vertical().max_height(360.0), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (index, row) in rows.iter().enumerate() {
                    let icon = row.kind.map_or(theme::Icon::Folder, theme::kind_icon);
                    let color = row.kind.map_or(theme::palette().muted, theme::tag_color);
                    let (resp, _, ()) = record::row(
                        ui,
                        ui.id().with(("find-row", index)),
                        height,
                        Sense::CLICK,
                        index == app.find.at,
                        &row.name,
                        |ui| {
                            if let Some(img) = app.icons.image(icon, glyph, color) {
                                ui.add(img);
                            }
                            ui.add(
                                Label::new(RichText::new(&row.name).color(theme::palette().text))
                                    .wrap(),
                            );
                            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                                if !row.detail.is_empty() {
                                    ui.label(
                                        RichText::new(&row.detail)
                                            .small()
                                            .color(theme::palette().faint),
                                    );
                                }
                            });
                            (None, ())
                        },
                    );
                    if resp.clicked() {
                        clicked = Some(index);
                    }
                }
            });
        }
        closed
    });

    if let Some(index) = clicked {
        app.find.at = index;
        app.find_choose(&rows);
    } else if !open {
        app.find = crate::features::find::model::Find::default();
    }
}
