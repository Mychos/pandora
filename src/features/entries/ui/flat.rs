//! The pane a card or an identity lives in: its list beside its editor. One pane for
//! both — the login keeps the folders view, the generated tree and the drag order to
//! itself, and neither of these shapes needs any of that.

use crate::app::{App, Confirm};
use crate::features::entries::ui::editor as entry;
use crate::shared::theme;
use crate::shared::ui::entry as record;
use crate::vault::model::{Card, Identity, Kind, Vault};
use egui::{Layout, RichText, Sense, Ui};

const ROW: f32 = 52.0;

pub fn main(ui: &mut Ui, app: &mut App, compact: bool, kind: Kind) {
    if compact {
        egui::CentralPanel::default()
            .frame(theme::pane(true))
            .show(ui, |ui| {
                if app.boxes.selected().is_some() {
                    entry::show(ui, app);
                    return;
                }
                head(ui, app, kind);
                finder(ui, app, kind);
                list(ui, app, kind);
            });
        return;
    }

    egui::Panel::left("shape-list")
        .resizable(true)
        .default_size(320.0)
        .size_range(260.0..=480.0)
        .frame(
            egui::Frame::NONE
                .fill(theme::palette().panel)
                .inner_margin(egui::Margin {
                    left: 16,
                    right: 14,
                    top: 14,
                    bottom: 14,
                }),
        )
        .show(ui, |ui| {
            head(ui, app, kind);
            finder(ui, app, kind);
            list(ui, app, kind);
            // Only the divider: a full frame stroke would overlap the tabs hairline above.
            let panel = ui.max_rect().expand2(egui::vec2(15.0, 14.0));
            ui.painter().vline(
                panel.right(),
                panel.y_range(),
                theme::stroke(theme::palette().hair, 1.0),
            );
        });

    egui::CentralPanel::default()
        .frame(theme::pane(false))
        .show(ui, |ui| entry::show(ui, app));
}

/// The pane's title line: what it holds, and the new-record button.
fn head(ui: &mut Ui, app: &mut App, kind: Kind) {
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(match kind {
                Kind::Card => crate::shared::i18n::t("Cards"),
                _ => crate::shared::i18n::t("Identities"),
            })
            .strong()
            .color(theme::palette().text),
        );
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            let label = match kind {
                Kind::Card => crate::shared::i18n::t("New card"),
                _ => crate::shared::i18n::t("New identity"),
            };
            if icons
                .button(ui, theme::kind_icon(kind), 15.0, theme::ACCENT, label)
                .clicked()
            {
                s.show_new = true;
            }
        });
    });
    ui.add_space(8.0);
}

fn finder(ui: &mut Ui, app: &mut App, kind: Kind) {
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };
    let hint = match kind {
        Kind::Card => "search name, holder, notes",
        _ => "search name, notes",
    };
    theme::search(
        ui,
        icons,
        &mut s.search,
        crate::features::boxes::logic::search_id(crate::app::Screen::Main),
        hint,
    );
    ui.add_space(10.0);
}

fn list(ui: &mut Ui, app: &mut App, kind: Kind) {
    let icons = &app.icons;
    let mut select: Option<String> = None;
    let mut del: Option<String> = None;
    let mut share: Option<String> = None;
    {
        let Some(s) = app.boxes.session.as_ref() else {
            return;
        };
        let q = s.search.trim().to_lowercase();
        let sel = s.selected.as_deref();
        let rows: Vec<(&str, &str, &str, Option<&str>)> = match kind {
            Kind::Card => cards(&s.vault, &q)
                .into_iter()
                .map(|it| {
                    (
                        it.id.as_str(),
                        it.name.as_str(),
                        it.holder.as_str(),
                        it.icon.as_deref(),
                    )
                })
                .collect(),
            _ => identities(&s.vault, &q)
                .into_iter()
                .map(|it| (it.id.as_str(), it.name.as_str(), first_line(it), None))
                .collect(),
        };

        if rows.is_empty() {
            theme::empty(
                ui,
                !q.is_empty(),
                match kind {
                    Kind::Card => "no cards yet",
                    _ => "no identities yet",
                },
                "press + new to create one",
            );
            return;
        }

        ui.spacing_mut().item_spacing.y = 6.0;
        theme::scroll_rows(
            ui,
            egui::ScrollArea::vertical().auto_shrink([false, false]),
            ROW,
            rows.len(),
            |ui, shown| {
                for index in shown {
                    let (id, name, detail, icon) = rows[index];
                    let brand = icon.and_then(theme::Brand::from_slug);
                    let (resp, _) = row(ui, icons, id, name, detail, kind, brand, sel == Some(id));
                    if resp.clicked() {
                        select = Some(id.to_owned());
                    }
                    resp.context_menu(|ui| {
                        theme::menu(ui);
                        if icons
                            .labeled(
                                ui,
                                theme::kind_icon(kind),
                                15.0,
                                theme::palette().text,
                                "Open",
                            )
                            .clicked()
                        {
                            select = Some(id.to_owned());
                            ui.close();
                        }
                        ui.separator();
                        if icons
                            .labeled(
                                ui,
                                theme::Icon::Upload,
                                15.0,
                                theme::palette().text,
                                "Share…",
                            )
                            .clicked()
                        {
                            share = Some(id.to_owned());
                            ui.close();
                        }
                        if icons
                            .labeled(
                                ui,
                                theme::Icon::Trash,
                                15.0,
                                theme::palette().error,
                                "Delete",
                            )
                            .clicked()
                        {
                            del = Some(id.to_owned());
                            ui.close();
                        }
                    });
                }
            },
        );
    }

    if let Some(id) = select {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = Some(id);
        }
    }
    if let Some(id) = del {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = Some(id.clone());
            s.confirm = Some(Confirm::Entry(id));
        }
    }
    if let Some(id) = share {
        app.share.send(&id);
    }
}

/// The rows the search keeps, in the vault's own order.
fn cards<'a>(vault: &'a Vault, q: &str) -> Vec<&'a Card> {
    vault
        .cards
        .iter()
        .filter(|it| {
            q.is_empty()
                || it.name.to_lowercase().contains(q)
                || it.holder.to_lowercase().contains(q)
                || it.notes.to_lowercase().contains(q)
        })
        .collect()
}

fn identities<'a>(vault: &'a Vault, q: &str) -> Vec<&'a Identity> {
    vault
        .identities
        .iter()
        .filter(|it| {
            q.is_empty()
                || it.name.to_lowercase().contains(q)
                || it.notes.to_lowercase().contains(q)
        })
        .collect()
}

/// What an identity says under its name: the first line of its notes, which is where
/// a person puts a role or an address. Never a field's value, which may be a secret.
fn first_line(it: &Identity) -> &str {
    it.notes
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

#[allow(clippy::too_many_arguments)] // display-only fields extracted to drop the vault borrow
fn row(
    ui: &mut Ui,
    icons: &theme::Icons,
    id: &str,
    name: &str,
    detail: &str,
    kind: Kind,
    brand: Option<theme::Brand>,
    selected: bool,
) -> (egui::Response, egui::Rect) {
    let label = if detail.is_empty() {
        name.to_owned()
    } else {
        format!("{name}, {detail}")
    };
    let (resp, rect, ()) = record::row(
        ui,
        ui.id().with(("shape-row", id)),
        ROW,
        Sense::click(),
        selected,
        &label,
        |ui| {
            record::glyph(ui, icons, kind, brand, 15.0);
            let w = record::width(ui, 0, 0.0);
            let name_h = ui.text_style_height(&egui::TextStyle::Body);
            let detail_h = ui.text_style_height(&egui::TextStyle::Small);
            let block_h = if detail.is_empty() {
                name_h
            } else {
                name_h + ui.spacing().item_spacing.y + detail_h
            };
            ui.allocate_ui_with_layout(
                egui::vec2(w, block_h),
                Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(name).strong().color(theme::palette().text))
                            .truncate(),
                    );
                    if !detail.is_empty() {
                        ui.add(
                            egui::Label::new(
                                RichText::new(detail).small().color(theme::palette().muted),
                            )
                            .truncate(),
                        );
                    }
                },
            );
            (None, ())
        },
    );
    (resp, rect)
}
