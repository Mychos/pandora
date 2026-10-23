//! Passwords pane: the box's credentials as a flat list, or as folders generated
//! from the entries themselves. The editor sits beside the list and opens as a
//! modal when the folders take the whole panel.

use crate::app::{App, Confirm, Drag};
use crate::features::boxes::logic::Secret;
use crate::features::entries::model::{Axis, View};
use crate::features::entries::tree;
use crate::features::entries::ui::editor as entry;
use crate::features::entries::ui::flat;
use crate::features::settings::model as prefs;
use crate::shared::theme;
use crate::shared::ui::drop::{landing, mark, Landing};
use crate::shared::ui::entry as record;
use crate::shared::ui::responsive;
use crate::vault::model::{Kind, Login, Vault};
use egui::{Layout, RichText, Sense, Ui};

pub fn main(ui: &mut Ui, app: &mut App, compact: bool) {
    // A card or an identity is the same pane, with a shape of its own: this one keeps
    // the generated folders and the drag order for logins.
    let pane = app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.tab.kind())
        .unwrap_or(Kind::Login);
    if pane != Kind::Login {
        flat::main(ui, app, compact, pane);
        return;
    }
    if compact {
        egui::CentralPanel::default()
            .frame(theme::pane(true))
            .show(ui, |ui| {
                if app.boxes.selected().is_some() && view_of(app) == View::List {
                    entry::show(ui, app);
                    return;
                }
                head(ui, app);
                finder(ui, app);
                if view_of(app) == View::Folders {
                    content(ui, app, true);
                } else {
                    list(ui, app);
                }
            });
        if view_of(app) == View::Folders {
            entry::modal(ui.ctx(), app);
        }
        return;
    }

    egui::Panel::left("password-list")
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
            head(ui, app);
            finder(ui, app);
            sidebar(ui, app);
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
        .show(ui, |ui| {
            if view_of(app) == View::Folders {
                content(ui, app, false);
            } else {
                entry::show(ui, app);
            }
        });

    if view_of(app) == View::Folders {
        entry::modal(ui.ctx(), app);
    }
}

fn view_of(app: &App) -> View {
    app.boxes
        .session
        .as_ref()
        .map(|s| s.view)
        .unwrap_or_default()
}

/// The pane's title line: the view picker and the new-password button.
fn head(ui: &mut Ui, app: &mut App) {
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };
    let mut view = s.view;
    ui.horizontal(|ui| {
        if view_button(
            ui,
            icons,
            theme::Icon::List,
            View::List.label(),
            view == View::List,
        )
        .clicked()
        {
            view = View::List;
        }
        if view_button(
            ui,
            icons,
            theme::Icon::Folder,
            View::Folders.label(),
            view == View::Folders,
        )
        .clicked()
        {
            view = View::Folders;
        }
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            if icons
                .button(ui, theme::Icon::Plus, 15.0, theme::ACCENT, "New password")
                .clicked()
            {
                s.show_new = true;
            }
        });
    });
    // The two views read the same box differently enough that a kept selection
    // would open an editor over a list the user did not ask for.
    if view != s.view {
        s.view = view;
        s.selected = None;
        s.at.clear();
        app.prefs.view = view;
        prefs::save(&app.prefs);
    }
}

/// The search field, with the grouping picker on its right in the folders view.
/// The picker only means something there, so the list view hands the whole row
/// back to the field.
fn finder(ui: &mut Ui, app: &mut App) {
    let icons = &app.icons;
    let mut pick: Option<Axis> = None;
    {
        let Some(s) = app.boxes.session.as_mut() else {
            return;
        };
        let folders = s.view == View::Folders;
        ui.add_space(8.0);
        // Right to left, so the picker hangs on the row's right edge and the
        // field takes what is left of the width.
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                if folders {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    pick = axis_picker(ui, icons, s.axis);
                    ui.spacing_mut().item_spacing.x = 8.0;
                }
                ui.with_layout(Layout::left_to_right(egui::Align::Center), |ui| {
                    theme::search(
                        ui,
                        icons,
                        &mut s.search,
                        crate::features::boxes::logic::search_id(crate::app::Screen::Main),
                        "search name, user, url, notes",
                    );
                });
            });
        });
        ui.add_space(10.0);
    }
    if let Some(axis) = pick {
        if let Some(s) = app.boxes.session.as_mut() {
            s.axis = axis;
            s.at.clear();
            s.selected = None;
        }
        app.prefs.axis = axis;
        prefs::save(&app.prefs);
    }
}

fn sidebar(ui: &mut Ui, app: &mut App) {
    if view_of(app) == View::Folders {
        tree(ui, app);
    } else {
        list(ui, app);
    }
}

/// The entries the search keeps, in the vault's own order.
fn found<'a>(vault: &'a Vault, q: &str) -> Vec<&'a Login> {
    vault
        .logins
        .iter()
        .filter(|it| {
            q.is_empty()
                || it.name.to_lowercase().contains(q)
                || it.username.to_lowercase().contains(q)
                || it.uris().any(|(url, _)| url.to_lowercase().contains(q))
                || it.notes.to_lowercase().contains(q)
        })
        .collect()
}

fn list(ui: &mut Ui, app: &mut App) {
    let icons = &app.icons;
    let wanted = {
        let Some(s) = app.boxes.session.as_ref() else {
            return;
        };
        let q = s.search.trim().to_lowercase();
        let entries = found(&s.vault, &q);
        if entries.is_empty() {
            theme::empty(
                ui,
                !q.is_empty(),
                "no passwords yet",
                "press + new to create one",
            );
            Wanted::default()
        } else {
            rows(ui, icons, &entries, s.selected.as_deref(), true)
        }
    };
    apply(ui, app, wanted);
}

/// The generated folders, one row each. A click enters a folder.
fn tree(ui: &mut Ui, app: &mut App) {
    let icons = &app.icons;
    let mut goto: Option<Vec<String>> = None;
    {
        let Some(s) = app.boxes.session.as_ref() else {
            return;
        };
        let q = s.search.trim().to_lowercase();
        let axis = s.axis;
        let items = found(&s.vault, &q);
        let nodes = tree::folders(&items, axis);
        let here = tree::node_key(axis, &s.at);
        if nodes.is_empty() {
            theme::empty(
                ui,
                !q.is_empty(),
                "no passwords yet",
                "press + new to create one",
            );
        } else {
            let row_h = responsive::row(ui.ctx());
            ui.spacing_mut().item_spacing.y = 6.0;
            theme::scroll_rows(
                ui,
                egui::ScrollArea::vertical().auto_shrink([false, false]),
                row_h,
                nodes.len(),
                |ui, rows| {
                    for index in rows {
                        let node = &nodes[index];
                        if tree_row(ui, icons, node, axis, node.key == here).clicked() {
                            goto = Some(node.path.clone());
                        }
                    }
                },
            );
        }
    }
    if let Some(path) = goto {
        if let Some(s) = app.boxes.session.as_mut() {
            s.at = path;
            s.selected = None;
        }
    }
}

/// The folders view's content: the breadcrumb, and the rows of every entry under
/// the folder on screen. `root_folders` makes the root (empty `at`) drill down
/// through the generated folders instead of listing every entry, which is what a
/// single column needs.
fn content(ui: &mut Ui, app: &mut App, root_folders: bool) {
    let icons = &app.icons;
    let Some(at) = app.boxes.session.as_ref().map(|s| s.at.clone()) else {
        return;
    };

    // The breadcrumb is only there to climb out of a folder, so the root gives
    // its row back to the entries.
    if !at.is_empty() {
        let mut goto: Option<Vec<String>> = None;
        ui.horizontal(|ui| {
            goto = trail(ui, &at);
        });
        ui.add_space(10.0);
        if let Some(path) = goto {
            if let Some(s) = app.boxes.session.as_mut() {
                s.at = path;
                s.selected = None;
            }
        }
    }

    if root_folders && at.is_empty() {
        tree(ui, app);
        return;
    }

    let wanted = {
        let Some(s) = app.boxes.session.as_ref() else {
            return;
        };
        let q = s.search.trim().to_lowercase();
        let items = found(&s.vault, &q);
        let nodes = tree::folders(&items, s.axis);
        let entries: Vec<&Login> = match tree::find(&nodes, &tree::node_key(s.axis, &s.at)) {
            Some(node) if !s.at.is_empty() => {
                let by_id: std::collections::HashMap<&str, &Login> =
                    items.iter().map(|it| (it.id.as_str(), *it)).collect();
                node.ids
                    .iter()
                    .filter_map(|id| by_id.get(id.as_str()).copied())
                    .collect()
            }
            _ => items.clone(),
        };
        if entries.is_empty() {
            theme::empty(
                ui,
                !q.is_empty(),
                "no passwords yet",
                "press + new to create one",
            );
            Wanted::default()
        } else {
            rows(ui, icons, &entries, s.selected.as_deref(), false)
        }
    };
    apply(ui, app, wanted);
}

/// What the rows asked for, gathered so the vault is never borrowed while a row
/// is drawn.
#[derive(Default)]
struct Wanted {
    new: bool,
    select: Option<String>,
    del: Option<String>,
    copy: Option<(String, Secret)>,
    /// The record whose share dialog the row's own menu asked for.
    share: Option<String>,
    /// Where a dragged row would land: (anchor id, below it).
    land: Option<(String, bool)>,
}

fn apply(ui: &Ui, app: &mut App, wanted: Wanted) {
    if let Some((anchor, after)) = wanted.land {
        if ui.ctx().input(|i| i.pointer.any_released()) {
            if let Some(Drag::Entry(id)) =
                egui::DragAndDrop::take_payload::<Drag>(ui.ctx()).as_deref()
            {
                app.boxes.reorder(Drag::Entry(id.clone()), &anchor, after);
            }
        }
    }
    if let Some(id) = wanted.select {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = Some(id);
        }
    }
    if let Some(id) = wanted.del {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = Some(id.clone());
            s.confirm = Some(Confirm::Entry(id));
        }
    }
    if wanted.new {
        app.boxes.add_login();
    }
    if let Some(id) = wanted.share {
        app.share.send(&id);
    }
    if let Some((id, secret)) = wanted.copy {
        app.copy_secret(ui.ctx(), &id, secret);
    }
}

/// The entry rows. `draggable` is off in the folders view, where the order comes
/// from the grouping, not from a drag.
fn rows(
    ui: &mut Ui,
    icons: &theme::Icons,
    entries: &[&Login],
    sel: Option<&str>,
    draggable: bool,
) -> Wanted {
    let mut wanted = Wanted::default();

    // Right-click on the empty space creates a password. Registered before the
    // rows so a row under the pointer wins the click.
    ui.interact(
        ui.available_rect_before_wrap(),
        ui.id().with("rows-empty"),
        Sense::CLICK,
    )
    .context_menu(|ui| {
        theme::menu(ui);
        if icons
            .labeled(
                ui,
                theme::Icon::Key,
                15.0,
                theme::palette().text,
                "New password",
            )
            .clicked()
        {
            wanted.new = true;
            ui.close();
        }
    });

    ui.spacing_mut().item_spacing.y = 6.0;
    theme::scroll_rows(
        ui,
        egui::ScrollArea::vertical().auto_shrink([false, false]),
        ROW_HEIGHT,
        entries.len(),
        |ui, rows| {
            for index in rows {
                let it = entries[index];
                let id = it.id.as_str();
                let (resp, rect, quick) = row(
                    ui,
                    icons,
                    id,
                    &it.name,
                    &it.username,
                    theme::brand_of(it),
                    sel == Some(id),
                );
                if draggable {
                    resp.clone()
                        .dnd_set_drag_payload(Drag::Entry(id.to_owned()));
                    // A row carried over another lands above or below it, and the
                    // line says which — the one rule every list in the app follows.
                    if let Some(Landing::Side { after }) = landing(ui, rect, false, false) {
                        mark(ui, rect, after, false);
                        wanted.land = Some((id.to_owned(), after));
                    }
                }
                if let Some(secret) = quick {
                    wanted.copy = Some((id.to_owned(), secret));
                }
                if resp.clicked() {
                    wanted.select = Some(id.to_owned());
                }
                resp.context_menu(|ui| {
                    theme::menu(ui);
                    if icons
                        .labeled(ui, theme::Icon::Key, 15.0, theme::palette().text, "Open")
                        .clicked()
                    {
                        wanted.select = Some(id.to_owned());
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
                        wanted.share = Some(id.to_owned());
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
                        wanted.del = Some(id.to_owned());
                        ui.close();
                    }
                });
            }
        },
    );

    wanted
}

/// A view picker button: the active half is filled, the other stays quiet.
fn view_button(
    ui: &mut Ui,
    icons: &theme::Icons,
    icon: theme::Icon,
    tip: &str,
    active: bool,
) -> egui::Response {
    ui.scope(|ui| {
        let color = if active {
            theme::palette().text
        } else {
            theme::palette().muted
        };
        let resp = if active {
            theme::chosen(ui, |ui| icons.button(ui, icon, 14.0, color, tip))
        } else {
            icons.button(ui, icon, 14.0, color, tip)
        };
        theme::focus(ui, &resp);
        resp
    })
    .inner
}

/// The grouping picker: one button per axis, at the right edge of the search
/// row, so the folders view changes axis without leaving the list it reads.
fn axis_picker(ui: &mut Ui, icons: &theme::Icons, axis: Axis) -> Option<Axis> {
    let mut pick = None;
    // Right to left, so the buttons still read Site, User.
    for option in Axis::ALL.into_iter().rev() {
        if segment(ui, icons, option, option == axis).clicked() {
            pick = Some(option);
        }
    }
    pick
}

/// One picker button: the axis's own icon, the active half filled.
fn segment(ui: &mut Ui, icons: &theme::Icons, axis: Axis, active: bool) -> egui::Response {
    let icon = match axis {
        Axis::Target => theme::Icon::Globe,
        Axis::User => theme::Icon::User,
    };
    view_button(ui, icons, icon, axis.label(), active)
}

/// The folder on screen; a click goes back to every entry. The generated tree is
/// one level deep, so there is nothing else to climb through.
fn trail(ui: &mut Ui, at: &[String]) -> Option<Vec<String>> {
    let label = at.last()?;
    crumb(ui, label).clicked().then(Vec::new)
}

fn crumb(ui: &mut Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).color(theme::palette().muted))
            .small()
            .frame(false),
    )
}

/// One generated folder: its mark, its name and how many entries hang under it. A
/// click enters the folder.
fn tree_row(
    ui: &mut Ui,
    icons: &theme::Icons,
    node: &tree::Node,
    axis: Axis,
    current: bool,
) -> egui::Response {
    /// Room the count keeps off the row's right edge, on top of the frame's own.
    const TAIL: f32 = 8.0;
    let row = responsive::row(ui.ctx());
    let glyph = responsive::glyph(ui.ctx());

    let (resp, _, ()) = record::row(
        ui,
        ui.id().with(("password-tree", &node.key)),
        row,
        Sense::CLICK,
        current,
        node.label.as_str(),
        |ui| {
            ui.set_min_height(row);
            // Every gap is placed by hand: the row may not inherit the form's
            // roomy spacing.
            ui.spacing_mut().item_spacing.x = 0.0;
            // The mark the folder's entries suggest, else the axis's own icon, so
            // every folder wears one and the labels stay aligned.
            let fallback = match axis {
                Axis::Target => theme::Icon::Globe,
                Axis::User => theme::Icon::User,
            };
            if let Some(img) = icons.glyph(node.brand, fallback, glyph, theme::ACCENT) {
                ui.add(img);
                ui.add_space(10.0);
            }
            egui::Sides::new()
                .shrink_left()
                .truncate()
                .spacing(12.0)
                .show(
                    ui,
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(node.label.clone()).color(theme::palette().text),
                            )
                            .truncate(),
                        );
                    },
                    |ui| {
                        ui.add_space(TAIL);
                        theme::hint(ui, &node.total().to_string());
                    },
                );
            (None, ())
        },
    );
    resp
}

/// Fixed row height, so one-line and two-line entries are the same size and centring has a known height.
const ROW_HEIGHT: f32 = 52.0;

/// A row, its whole rect — the click zone stops short of the marks, the rect the
/// drop line is measured on does not — and the secret a copy mark took, if one
/// was pressed.
#[allow(clippy::too_many_arguments)] // display-only fields extracted to drop the vault borrow
fn row(
    ui: &mut Ui,
    icons: &theme::Icons,
    id: &str,
    name: &str,
    user: &str,
    brand: Option<theme::Brand>,
    selected: bool,
) -> (egui::Response, egui::Rect, Option<Secret>) {
    let label = if user.is_empty() {
        name.to_owned()
    } else {
        format!("{name}, {user}")
    };
    record::row(
        ui,
        ui.id().with(("row", id)),
        ROW_HEIGHT,
        egui::Sense::click_and_drag(),
        selected,
        &label,
        |ui| {
            record::glyph(ui, icons, Kind::Login, brand, 15.0);
            // The marks keep their own square, so the text gives up what they take.
            let w = record::width(ui, 2, 0.0);
            // An explicit height, or the row's cross alignment has nothing to centre.
            let name_h = ui.text_style_height(&egui::TextStyle::Body);
            let user_h = ui.text_style_height(&egui::TextStyle::Small);
            let block_h = if user.is_empty() {
                name_h
            } else {
                name_h + ui.spacing().item_spacing.y + user_h
            };
            ui.allocate_ui_with_layout(
                egui::vec2(w, block_h),
                Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(name).strong().color(theme::palette().text))
                            .truncate(),
                    );
                    if !user.is_empty() {
                        ui.add(
                            egui::Label::new(
                                RichText::new(user).small().color(theme::palette().muted),
                            )
                            .truncate(),
                        );
                    }
                },
            );
            // Always on, not on hover: a phone has no pointer to hover with. Right
            // to left, so they hang on the row's right edge whatever the name and
            // the user over them take.
            let (marks, taken) = ui
                .with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    let password = icons.button(
                        ui,
                        theme::Icon::Copy,
                        15.0,
                        theme::palette().muted,
                        "copy password",
                    );
                    let username = icons.button(
                        ui,
                        theme::Icon::User,
                        15.0,
                        theme::palette().muted,
                        "copy username",
                    );
                    let taken = if username.clicked() {
                        Some(Secret::Username)
                    } else if password.clicked() {
                        Some(Secret::Password)
                    } else {
                        None
                    };
                    (
                        egui::Rect::from_min_max(username.rect.min, password.rect.max),
                        taken,
                    )
                })
                .inner;
            (Some(marks), taken)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::entries::ui::editor;
    use crate::shared::ui::shot;

    /// The search row of a sidebar-width column inside a desktop window, settled
    /// over two frames: what it painted, the field's own rect, and the column's
    /// width.
    fn search_row(view: View) -> (Vec<egui::epaint::ClippedShape>, egui::Rect, f32) {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut session = editor::tests::session(Vault::new("Test"), None);
        session.view = view;
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session);

        // The pane's own width is what the sidebar reserves, not the window's.
        let sidebar = 320.0;
        let size = egui::vec2(900.0, 600.0);
        let mut row = |ui: &mut Ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(sidebar, size.y),
                Layout::top_down(egui::Align::Min),
                |ui| finder(ui, &mut app),
            );
        };
        shot::frame(&ctx, size, Vec::new(), &mut row);
        let shapes = shot::frame(&ctx, size, Vec::new(), &mut row);
        let field = ctx
            .read_response(crate::features::boxes::logic::search_id(
                crate::app::Screen::Main,
            ))
            .expect("the search field is laid out")
            .rect;
        (shapes, field, sidebar)
    }

    /// The picker that groups the folders view lives on the search row, to the
    /// right of the field — never on a row of its own.
    #[test]
    fn the_grouping_picker_sits_right_of_the_search_field() {
        let (shapes, field, sidebar) = search_row(View::Folders);
        let filled: Vec<egui::Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == theme::palette().accent_dim => Some(r.rect),
                _ => None,
            })
            .collect();

        assert_eq!(
            filled.len(),
            1,
            "the active axis is not the one dim-accent mark: {filled:?}"
        );
        let picker = filled[0];
        assert!(
            picker.min.x > field.max.x,
            "the picker is not right of the field: {picker:?} vs {field:?}"
        );
        assert!(
            picker.center().y > field.min.y && picker.center().y < field.max.y,
            "the picker is not on the search row: {picker:?} vs {field:?}"
        );
        assert!(
            field.max.x < sidebar - 40.0,
            "the field did not hand the picker any room: {field:?}"
        );
    }

    /// Nothing groups a flat list, so the list view hands the whole row back to
    /// the field.
    #[test]
    fn the_list_view_hands_the_whole_row_to_the_search() {
        let (shapes, field, sidebar) = search_row(View::List);
        let filled = shapes.iter().filter(|s| match &s.shape {
            egui::Shape::Rect(r) => r.fill == theme::palette().accent_dim,
            _ => false,
        });
        assert_eq!(filled.count(), 0, "the list view still paints a picker");
        assert!(
            field.max.x > sidebar - 16.0,
            "the field does not take the whole row: {field:?}"
        );
    }

    /// The active half of the view picker is filled with the dim accent; the
    /// pointer must deepen it, never swap it for egui's grey hovered fill.
    #[test]
    fn the_active_view_button_deepens_on_hover() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let icons = theme::Icons::load(&ctx);
        let size = egui::vec2(200.0, 60.0);

        let mut rect: Option<egui::Rect> = None;
        let mut rest = Vec::new();
        let mut hover = Vec::new();
        for frame in 0..3 {
            let mut events = Vec::new();
            if frame > 0 {
                events.push(egui::Event::PointerMoved(
                    rect.expect("the button is laid out").center(),
                ));
            }
            let shapes = shot::frame(&ctx, size, events, |ui| {
                rect =
                    Some(view_button(ui, &icons, theme::Icon::List, View::List.label(), true).rect);
            });
            let at = rect.expect("the button is laid out").center();
            let painted: Vec<egui::Color32> = shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r) if r.rect.contains(at) => Some(r.fill),
                    _ => None,
                })
                .collect();
            match frame {
                0 => rest = painted,
                2 => hover = painted,
                _ => {}
            }
        }

        assert!(
            rest.contains(&theme::palette().accent_dim),
            "the active toggle is not the dim accent at rest: {rest:?}"
        );
        assert!(
            hover.contains(&theme::palette().accent_deep),
            "the active toggle does not deepen on hover: {hover:?}"
        );
    }
}
