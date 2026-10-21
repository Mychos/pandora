//! Drive pane: notes + files organised in folders. Left: folder tree. Centre:
//! cards of the current folder. A folder and an entry both open in a floating
//! modal (`modal` / `editor::modal`), so the grid is never pushed into a side pane.

pub mod board;
pub mod editor;
pub mod menu;

use crate::app::{App, Confirm, Drag};
use crate::features::boxes::model::NewFolder;
use crate::features::drive::preview;
use crate::features::drive::ui::menu as move_menu;
use crate::shared::theme;
use crate::shared::ui::drop::{landing, mark, Landing};
use crate::shared::ui::responsive;
use crate::vault::model::{now_secs, Entry, Kind};
use crate::vault::tree as folders;
use egui::{Align, Color32, Layout, Rect, RichText, Sense, Ui};

/// Card size. Four fit a 700pt pane, the grid wraps below that.
const CARD: egui::Vec2 = egui::vec2(152.0, 96.0);
const TREE_INDENT: f32 = 20.0;

pub fn main(ui: &mut Ui, app: &mut App, compact: bool) {
    let open = app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.editing_entry.clone());
    let open_is_folder = open.as_deref().is_some_and(|id| app.boxes.is_folder(id));
    if compact {
        egui::CentralPanel::default()
            .frame(theme::pane(true))
            .show(ui, |ui| match &open {
                Some(_) if !open_is_folder => editor::show(ui, app),
                _ => content(ui, app),
            });
        create_modal(ui.ctx(), app);
        modal(ui.ctx(), app);
        move_modal(ui.ctx(), app);
        return;
    }

    egui::Panel::left("drive-tree")
        .resizable(true)
        .default_size(230.0)
        .size_range(180.0..=360.0)
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
            tree(ui, app);
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
        .show(ui, |ui| content(ui, app));

    // After the grid, so a click on a card this frame already set the selection.
    editor::modal(ui.ctx(), app);
    create_modal(ui.ctx(), app);
    modal(ui.ctx(), app);
    move_modal(ui.ctx(), app);
}

/// Ask for a folder inside the selected one (or the folder being browsed). The
/// name is only a pending one; `create_modal` puts it in the box on confirm, so
/// opening the dialog never writes anything.
pub fn new_folder(app: &mut App) {
    let target = match app.boxes.session.as_ref() {
        Some(s) => match s.selected.as_deref() {
            Some(id) if app.boxes.is_folder(id) => Some(id.to_owned()),
            _ => s.folder.clone(),
        },
        None => return,
    };
    if let Some(s) = app.boxes.session.as_mut() {
        s.new_folder = Some(NewFolder {
            parent: target.clone(),
            name: "New folder".to_owned(),
        });
        if let Some(parent) = target {
            s.collapsed.remove(&parent);
        }
    }
}

fn content(ui: &mut Ui, app: &mut App) {
    header(ui, app);
    breadcrumb(ui, app);
    grid(ui, app);
}

fn header(ui: &mut Ui, app: &mut App) {
    let icons = &app.icons;
    let mut want_new = false;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };

    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icons
                .button(
                    ui,
                    theme::Icon::Plus,
                    15.0,
                    theme::ACCENT,
                    "New note or file",
                )
                .clicked()
            {
                s.show_new = true;
            }
            if icons
                .button(
                    ui,
                    theme::Icon::FolderPlus,
                    15.0,
                    theme::ACCENT,
                    "New folder",
                )
                .clicked()
            {
                want_new = true;
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                theme::search(
                    ui,
                    icons,
                    &mut s.search,
                    crate::features::boxes::logic::search_id(crate::app::Screen::Main),
                    "search this folder",
                );
            });
        });
    });
    if want_new {
        new_folder(app);
    }
}

/// Vault name › … › current folder. Clicking a crumb opens it; the root crumb is
/// the only drop target that moves an entry or a folder back to the drive root,
/// since the tree lists folders only.
fn breadcrumb(ui: &mut Ui, app: &mut App) {
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };
    let trail: Vec<(String, String)> = folders::path(&s.vault, s.folder.as_deref())
        .iter()
        .map(|f| (f.id.clone(), f.name.clone()))
        .collect();
    let root = s.vault.name.clone();
    let current = s.folder.clone();
    let dragging = ui
        .ctx()
        .input(|i| !i.raw.hovered_files.is_empty() || !i.raw.dropped_files.is_empty());
    let pointer = ui.ctx().input(|i| i.pointer.hover_pos());
    let mut drop_at: Option<Option<String>> = None;
    let mut drop_highlight: Option<(egui::Rect, u8)> = None;

    ui.add_space(12.0);
    let mut go: Option<Option<String>> = None;
    let mut dropped: Option<Drag> = None;
    let mut edit = false;
    ui.horizontal(|ui| {
        let resp = crumb(ui, current.is_none(), &root);
        if dragging && pointer.map(|p| resp.rect.contains(p)).unwrap_or(false) {
            drop_at = Some(None);
            drop_highlight = Some((resp.rect, 8));
        }
        if let Some(p) = drop_on(ui, resp.rect) {
            dropped = Some(p);
        } else if resp.clicked() {
            go = Some(None);
        }
        for (id, name) in &trail {
            icons
                .image(theme::Icon::ChevronRight, 12.0, theme::palette().faint)
                .map(|img| ui.add(img));
            let here = current.as_deref() == Some(id.as_str());
            let crumb_resp = crumb(ui, here, name);
            if dragging
                && pointer
                    .map(|p| crumb_resp.rect.contains(p))
                    .unwrap_or(false)
            {
                drop_at = Some(Some(id.clone()));
                drop_highlight = Some((crumb_resp.rect, 6));
            }
            if crumb_resp.clicked() {
                go = Some(Some(id.clone()));
            }
        }
        // Edit the folder being browsed: its card and tree row are out of reach from inside.
        if current.is_some() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                edit = icons
                    .button(
                        ui,
                        theme::Icon::Pencil,
                        14.0,
                        theme::palette().muted,
                        "Edit this folder",
                    )
                    .clicked();
            });
        }
    });
    ui.add_space(10.0);

    if let Some(target) = dropped {
        app.boxes.move_into(target, None, &mut app.shell);
    } else if edit {
        if let Some(s) = app.boxes.session.as_mut() {
            s.editing_folder = current;
        }
    } else if let Some(dest) = go {
        if let Some(s) = app.boxes.session.as_mut() {
            s.folder = dest;
            s.selected = None;
        }
    }
    if let Some(target) = drop_at {
        app.drop_at = Some(target);
    }
    if let Some(highlight) = drop_highlight {
        app.drop_highlight = Some(highlight);
    }
}

fn crumb(ui: &mut Ui, current: bool, label: &str) -> egui::Response {
    let color = if current {
        theme::palette().text
    } else {
        theme::palette().muted
    };
    let btn = egui::Button::new(RichText::new(label).color(color));
    ui.add(btn.small().selected(current).frame(false))
}

/// Chevron size, also the indent the label gets when a row has no children.
const CHEVRON: f32 = 14.0;

struct Row {
    opened: bool,
    toggled: bool,
    edit: bool,
    act: Option<FolderAct>,
    drop: Option<Landing>,
    rect: Rect,
}

enum FolderAct {
    Open,
    Edit,
    NewSub,
    NewNote,
    NewFile,
    Delete,
}

enum EntryAct {
    Open,
    Move(Option<String>),
    Delete,
}

/// Record what a landing means: a move into a folder, or a reorder next to it.
fn push_action(intent: &mut Intent, carried: Drag, id: &str, l: Landing) {
    match l {
        Landing::Into => intent.moves.push((carried, Some(id.to_owned()))),
        Landing::Side { after } => intent.reorders.push((carried, id.to_owned(), after)),
    }
}

/// Folder actions on right-click; `Delete` opens the same prompt as the keyboard.
fn folder_menu(icons: &theme::Icons, resp: &egui::Response) -> Option<FolderAct> {
    let mut act = None;
    resp.context_menu(|ui| {
        theme::menu(ui);
        if icons
            .labeled(ui, theme::Icon::Folder, 15.0, theme::palette().text, "Open")
            .clicked()
        {
            act = Some(FolderAct::Open);
            ui.close();
        }
        if icons
            .labeled(
                ui,
                theme::Icon::Pencil,
                15.0,
                theme::palette().text,
                "Rename…",
            )
            .clicked()
        {
            act = Some(FolderAct::Edit);
            ui.close();
        }
        ui.separator();
        if icons
            .labeled(
                ui,
                theme::Icon::FolderPlus,
                15.0,
                theme::palette().text,
                "New subfolder",
            )
            .clicked()
        {
            act = Some(FolderAct::NewSub);
            ui.close();
        }
        if icons
            .labeled(
                ui,
                theme::Icon::Note,
                15.0,
                theme::palette().text,
                "New note here",
            )
            .clicked()
        {
            act = Some(FolderAct::NewNote);
            ui.close();
        }
        if icons
            .labeled(
                ui,
                theme::Icon::Paperclip,
                15.0,
                theme::palette().text,
                "New file here…",
            )
            .clicked()
        {
            act = Some(FolderAct::NewFile);
            ui.close();
        }
        ui.separator();
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
            act = Some(FolderAct::Delete);
            ui.close();
        }
    });
    act
}

/// The entry actions offered on right-click. `current` highlights the folder the
/// entry is in, `tree` is the destination list, `kind` is the entry's own icon.
fn entry_menu(
    icons: &theme::Icons,
    resp: &egui::Response,
    tree: &move_menu::Tree,
    current: Option<String>,
    kind: theme::Icon,
) -> Option<EntryAct> {
    let mut act = None;
    resp.context_menu(|ui| {
        theme::menu(ui);
        if icons
            .labeled(ui, kind, 15.0, theme::palette().text, "Open")
            .clicked()
        {
            act = Some(EntryAct::Open);
            ui.close();
        }
        let mut atoms = egui::Atoms::new(RichText::new("Move to…").color(theme::palette().text));
        if let Some(img) = icons.image(
            theme::Icon::Folder,
            responsive::grow(ui.ctx(), 15.0),
            theme::palette().text,
        ) {
            atoms.push_left(img);
        }
        ui.menu_button(atoms, |ui| {
            theme::menu(ui);
            ui.set_min_width(240.0);
            if let Some(dest) = move_menu::rows(ui, icons, tree, current) {
                act = Some(EntryAct::Move(dest));
            }
        });
        ui.separator();
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
            act = Some(EntryAct::Delete);
            ui.close();
        }
    });
    act
}

/// What a click or a context menu asked of the drive. Both surfaces — a tree
/// row and a grid card — collect into this, and one `apply` runs it, so the two
/// offer the same actions with the same precedence.
#[derive(Default)]
struct Intent {
    open: Option<String>,
    enter: Option<String>,
    edit: Option<String>,
    new_sub: Option<String>,
    new_note: Option<String>,
    new_file: Option<String>,
    del: Option<String>,
    moves: Vec<(Drag, Option<String>)>,
    reorders: Vec<(Drag, String, bool)>,
    toggles: Vec<String>,
    clear: bool,
}

impl Intent {
    /// Record what a folder's context menu asked for.
    fn folder(&mut self, id: &str, act: Option<FolderAct>) {
        match act {
            Some(FolderAct::Open) => self.enter = Some(id.to_owned()),
            Some(FolderAct::Edit) => self.edit = Some(id.to_owned()),
            Some(FolderAct::NewSub) => self.new_sub = Some(id.to_owned()),
            Some(FolderAct::NewNote) => self.new_note = Some(id.to_owned()),
            Some(FolderAct::NewFile) => self.new_file = Some(id.to_owned()),
            Some(FolderAct::Delete) => self.del = Some(id.to_owned()),
            None => {}
        }
    }
}

/// Run what a surface asked for. The folder the action names is selected first,
/// so a create or a delete lands inside it.
fn apply(app: &mut App, intent: Intent) {
    if let Some(s) = app.boxes.session.as_mut() {
        for id in intent.toggles {
            if !s.collapsed.remove(&id) {
                s.collapsed.insert(id);
            }
        }
    }
    if let Some((target, anchor, after)) = intent.reorders.into_iter().next() {
        app.boxes.reorder(target, &anchor, after);
        return;
    }
    if let Some(id) = intent.del {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = Some(id);
        }
        app.boxes.ask_delete();
        return;
    }
    if let Some(id) = intent.new_sub {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = Some(id);
        }
        new_folder(app);
        return;
    }
    if let Some(id) = intent.new_note {
        app.boxes.add_note_in(Some(id));
        return;
    }
    if let Some(id) = intent.new_file {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = Some(id);
        }
        app.boxes.pick_files(&mut app.shell);
        return;
    }
    if let Some(id) = intent.edit {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = Some(id.clone());
            s.editing_folder = Some(id);
        }
        return;
    }
    if let Some((target, dest)) = intent.moves.into_iter().next() {
        app.boxes.move_into(target, dest, &mut app.shell);
    }
    if let Some(id) = intent.enter {
        if let Some(s) = app.boxes.session.as_mut() {
            s.folder = Some(id);
            s.selected = None;
            s.editing_entry = None;
        }
        return;
    }
    if intent.clear {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = None;
        }
    }
    if let Some(id) = intent.open {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = Some(id.clone());
            s.editing_entry = Some(id);
        }
    }
}

/// Folder tree — folders only, the drive root is not a row. Rows are drop
/// targets; nested folders can be dragged onto each other (a cycle is refused
/// with a status message). A click opens the folder, the ✎ (on hover) opens its
/// editor, and the chevron folds its children away. Going back to the drive root
/// is the breadcrumb's job.
fn tree(ui: &mut Ui, app: &mut App) {
    let icons = &app.icons;
    let mut want_new = false;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };

    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icons
                .button(
                    ui,
                    theme::Icon::FolderPlus,
                    15.0,
                    theme::ACCENT,
                    "New folder",
                )
                .clicked()
            {
                want_new = true;
            }
        });
    });
    ui.add_space(10.0);

    // Keep the folder being browsed reachable: expand everything above it.
    let chain: Vec<String> = folders::path(&s.vault, s.folder.as_deref())
        .iter()
        .map(|f| f.id.clone())
        .collect();
    for id in chain {
        s.collapsed.remove(&id);
    }

    let collapsed = s.collapsed.clone();
    let rows: Vec<(usize, String, String, bool, usize)> = folders::visible(&s.vault, &collapsed)
        .iter()
        .map(|(d, f, kids)| {
            let count = folders::children(&s.vault, Some(&f.id)).len()
                + folders::entries(&s.vault, Some(&f.id)).len();
            (*d, f.id.clone(), f.name.clone(), *kids, count)
        })
        .collect();
    let current = s.folder.clone();
    let selected = s.selected.clone();
    let carried = egui::DragAndDrop::payload::<Drag>(ui.ctx()).map(|p| (*p).clone());
    let released = ui.ctx().input(|i| i.pointer.any_released());
    let dragging = ui
        .ctx()
        .input(|i| !i.raw.hovered_files.is_empty() || !i.raw.dropped_files.is_empty());
    let pointer = ui.ctx().input(|i| i.pointer.hover_pos());
    let empty_tree_clicked = ui
        .interact(
            ui.available_rect_before_wrap(),
            ui.id().with("tree-empty"),
            Sense::CLICK,
        )
        .clicked();
    let mut intent = Intent::default();
    if empty_tree_clicked {
        intent.clear = true;
    }
    let mut drop_at: Option<Option<String>> = None;
    let mut drop_highlight: Option<(egui::Rect, u8)> = None;

    theme::scroll(
        ui,
        egui::ScrollArea::vertical().auto_shrink([false, false]),
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for (depth, id, name, kids, count) in &rows {
                let row = ui
                    .horizontal(|ui| {
                        ui.add_space(*depth as f32 * TREE_INDENT);
                        tree_row(
                            ui,
                            icons,
                            id,
                            *depth,
                            name,
                            current.as_deref() == Some(id.as_str()),
                            selected.as_deref() == Some(id.as_str()),
                            *kids,
                            collapsed.contains(id),
                            *count,
                        )
                    })
                    .inner;
                if row.toggled {
                    intent.toggles.push(id.clone());
                }
                if let Some(l) = row.drop {
                    if released {
                        if let Some(carried) = carried.clone() {
                            push_action(&mut intent, carried, id, l);
                        }
                    }
                }
                if row.edit {
                    intent.edit = Some(id.clone());
                } else if row.opened {
                    intent.enter = Some(id.clone());
                }
                intent.folder(id, row.act);
                if dragging && pointer.map(|p| row.rect.contains(p)).unwrap_or(false) {
                    drop_at = Some(Some(id.clone()));
                    drop_highlight = Some((row.rect, 3));
                }
            }
            if rows.is_empty() {
                ui.add_space(8.0);
                theme::hint(ui, "no folders yet");
            }
        },
    );

    apply(app, intent);
    if let Some(target) = drop_at {
        app.drop_at = Some(target);
    }
    if let Some(highlight) = drop_highlight {
        app.drop_highlight = Some(highlight);
    }
    if want_new {
        new_folder(app);
    }
}

/// One tree row: `>` chevron folds children, the label opens the folder on a
/// single click. Returns what happened, and paints the line a drop would land on.
#[allow(clippy::too_many_arguments)]
fn tree_row(
    ui: &mut Ui,
    icons: &theme::Icons,
    id: &str,
    _depth: usize,
    label: &str,
    current: bool,
    selected: bool,
    kids: bool,
    folded: bool,
    count: usize,
) -> Row {
    let mut chevron_rect: Option<Rect> = None;
    let mut count_rect: Option<Rect> = None;
    let surface = theme::Surface::reserve(ui);
    let row = responsive::row(ui.ctx());
    let glyph = responsive::grow(ui.ctx(), 15.0);
    const TAIL: f32 = 8.0;
    let rect = egui::Frame::default()
        // The chevron lives in the row's left padding, so the folder's mark starts
        // on the same grid the passwords pane's rows do.
        .inner_margin(egui::Margin {
            left: 0,
            right: 14,
            top: 0,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), row),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_height(row);
                    ui.spacing_mut().item_spacing.x = 0.0;
                    match (kids, chevron(icons, folded)) {
                        (true, Some(img)) => {
                            ui.add_space(6.0);
                            chevron_rect = Some(ui.add(img).rect);
                            ui.add_space(6.0);
                        }
                        _ => {
                            ui.allocate_exact_size(egui::vec2(CHEVRON, CHEVRON), Sense::hover());
                        }
                    }
                    if let Some(img) = icons.image(theme::Icon::Folder, glyph, theme::ACCENT) {
                        ui.add(img);
                        ui.add_space(10.0);
                    }
                    egui::Sides::new().shrink_left().truncate().show(
                        ui,
                        |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(label).color(theme::palette().text))
                                    .truncate(),
                            );
                        },
                        |ui| {
                            ui.add_space(TAIL);
                            // The hint's ink rides a couple of pixels above a row's
                            // middle; the count is set down so it reads centred
                            // against the row, the mark and the label.
                            let font = ui.style().text_styles[&egui::TextStyle::Small].clone();
                            let galley = ui.fonts_mut(|fonts| {
                                fonts.layout_no_wrap(
                                    count.to_string(),
                                    font,
                                    theme::palette().faint,
                                )
                            });
                            let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                            count_rect = Some(rect);
                            ui.place(
                                rect.translate(egui::vec2(0.0, 2.0)),
                                egui::Label::new(egui::WidgetText::Galley(galley)),
                            );
                            // Room for the pencil, so it lands beside the count
                            // instead of over it.
                            ui.add_space(4.0 + EDIT);
                        },
                    );
                },
            );
        })
        .response
        .rect;

    let resp = ui
        .interact(rect, ui.id().with(("tree", id)), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            selected,
            label.to_owned(),
        )
    });
    theme::focus(ui, &resp);
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    resp.clone()
        .dnd_set_drag_payload(Drag::Folder(id.to_owned()));

    // Registered after the row overlay, so the chevron wins the click there.
    let mut toggled = false;
    let mut on_chevron = false;
    if let Some(crect) = chevron_rect {
        let chev = ui.interact(crect, ui.id().with(("tree-chevron", id)), Sense::click());
        chev.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("{} {label}", if folded { "Expand" } else { "Collapse" }),
            )
        });
        theme::focus(ui, &chev);
        toggled = chev.clicked();
        chev.on_hover_cursor(egui::CursorIcon::PointingHand);
        on_chevron = ui
            .ctx()
            .pointer_interact_pos()
            .is_some_and(|p| crect.contains(p));
    }

    let hot = hot(ui, rect);
    let (fill, line) = if selected || current || resp.dragged() || hot {
        (
            theme::palette().accent_dim,
            theme::stroke(theme::ACCENT, 1.0),
        )
    } else {
        theme::state(false, resp.hovered())
    };
    surface.paint(ui, rect, theme::radius(8), fill, line);

    // ✎ on hover/selection: a click on it edits, it never selects or opens. It
    // sits left of the count, in the room the row keeps for it.
    let edit = (selected || ui.rect_contains_pointer(rect))
        && count_rect.is_some_and(|count| {
            edit_button(
                ui,
                icons,
                id,
                egui::pos2(count.left() - 4.0 - EDIT / 2.0, rect.center().y),
            )
        });

    // The ✎ wins the click; otherwise the row offers the folder actions.
    let act = if edit {
        None
    } else {
        folder_menu(icons, &resp)
    };
    let drop = if edit {
        None
    } else {
        let l = landing(ui, rect, false, true);
        if let Some(Landing::Side { after }) = l {
            mark(ui, rect, after, false);
        }
        l
    };

    Row {
        // A click opens the folder it names. `Response::clicked` already folds
        // in a keyboard activation (Space/Enter on a focused row), so there is
        // nothing extra for the pointer.
        opened: !edit && resp.clicked() && !on_chevron,
        toggled,
        edit,
        act,
        drop,
        rect,
    }
}

/// The fold chevron: `>` when folded, rotated to `v` when open.
fn chevron(icons: &theme::Icons, folded: bool) -> Option<egui::Image<'static>> {
    icons
        .image(theme::Icon::ChevronRight, CHEVRON, theme::palette().muted)
        .map(|img| {
            if folded {
                img
            } else {
                img.rotate(std::f32::consts::FRAC_PI_2, egui::vec2(0.5, 0.5))
            }
        })
}

/// Cards of the current folder: folders first, then the drive entries.
fn grid(ui: &mut Ui, app: &mut App) {
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };
    let parent = s.folder.clone();
    let q = s.search.trim().to_lowercase();
    let sel = s.selected.clone();
    let tree = move_menu::tree(&s.vault);

    let dirs: Vec<(String, String)> = folders::children(&s.vault, parent.as_deref())
        .iter()
        .filter(|f| q.is_empty() || f.name.to_lowercase().contains(&q))
        .map(|f| (f.id.clone(), f.name.clone()))
        .collect();
    let files: Vec<(String, String, theme::Icon, Color32, String, Option<String>)> =
        folders::entries(&s.vault, parent.as_deref())
            .into_iter()
            .filter(|i| {
                q.is_empty()
                    || i.name().to_lowercase().contains(&q)
                    || matches!(
                        i,
                        Entry::Note(n)
                            if s.vault
                                .note_text(n)
                                .is_ok_and(|body| body.to_lowercase().contains(&q))
                    )
            })
            .map(|i| {
                // The icon already carries the kind, so only a document's own size
                // is worth a subtitle: a note has nothing else to say.
                let sub = match i {
                    Entry::File(f) => crate::vault::store::human_size(f.bytes()),
                    Entry::Note(_) => String::new(),
                };
                let icon = match &i {
                    Entry::Note(_) => theme::kind_icon(Kind::Note),
                    Entry::File(f) => preview::icon(preview::kind(ui.ctx(), &s.vault, f)),
                };
                (
                    i.id().to_owned(),
                    i.name().to_owned(),
                    icon,
                    theme::tag_color(i.kind()),
                    sub,
                    i.folder().map(str::to_owned),
                )
            })
            .collect();

    // Right-click on the empty space creates here. Registered before the cards
    // so a card under the pointer wins the click.
    let mut empty_note = false;
    let mut empty_file = false;
    let mut empty_folder = false;
    let empty_resp = ui.interact(
        ui.available_rect_before_wrap(),
        ui.id().with("grid-empty"),
        Sense::CLICK,
    );
    let empty_clicked = empty_resp.clicked();
    empty_resp.context_menu(|ui| {
        theme::menu(ui);
        if icons
            .labeled(
                ui,
                theme::Icon::Note,
                15.0,
                theme::palette().text,
                "New note",
            )
            .clicked()
        {
            empty_note = true;
            ui.close();
        }
        if icons
            .labeled(
                ui,
                theme::Icon::Paperclip,
                15.0,
                theme::palette().text,
                "New file…",
            )
            .clicked()
        {
            empty_file = true;
            ui.close();
        }
        if icons
            .labeled(
                ui,
                theme::Icon::FolderPlus,
                15.0,
                theme::palette().text,
                "New folder",
            )
            .clicked()
        {
            empty_folder = true;
            ui.close();
        }
    });
    if empty_folder {
        new_folder(app);
        return;
    }
    if empty_note {
        app.boxes.add_note();
        return;
    }
    if empty_file {
        app.boxes.pick_files(&mut app.shell);
        return;
    }

    if dirs.is_empty() && files.is_empty() {
        theme::empty(
            ui,
            !q.is_empty(),
            "this folder is empty",
            "drop files here, or press + to add a note or file",
        );
        return;
    }

    let carried = egui::DragAndDrop::payload::<Drag>(ui.ctx()).map(|p| (*p).clone());
    let released = ui.ctx().input(|i| i.pointer.any_released());
    let dragging = ui
        .ctx()
        .input(|i| !i.raw.hovered_files.is_empty() || !i.raw.dropped_files.is_empty());
    let pointer = ui.ctx().input(|i| i.pointer.hover_pos());
    let mut intent = Intent::default();
    if empty_clicked {
        intent.clear = true;
    }
    let mut drop_at: Option<Option<String>> = None;
    let mut drop_highlight: Option<(egui::Rect, u8)> = None;

    theme::scroll(
        ui,
        egui::ScrollArea::vertical().auto_shrink([false, false]),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
            ui.horizontal_wrapped(|ui| {
                for (id, name) in &dirs {
                    let resp = card(
                        ui,
                        icons,
                        id,
                        theme::Icon::Folder,
                        None,
                        theme::ACCENT,
                        name,
                        "",
                        sel.as_deref() == Some(id.as_str()),
                    );
                    resp.clone().dnd_set_drag_payload(Drag::Folder(id.clone()));
                    let act = folder_menu(icons, &resp);
                    // ✎ only once the card is hovered or selected — a plain click
                    // opens the folder, it does not open the editor.
                    let show_edit =
                        sel.as_deref() == Some(id.as_str()) || ui.rect_contains_pointer(resp.rect);
                    if show_edit
                        && edit_button(
                            ui,
                            icons,
                            id,
                            egui::pos2(
                                resp.rect.max.x - EDIT / 2.0 - 3.0,
                                resp.rect.min.y + EDIT / 2.0 + 3.0,
                            ),
                        )
                    {
                        intent.edit = Some(id.clone());
                    } else if let Some(l) = landing(ui, resp.rect, true, true) {
                        if let Landing::Side { after } = l {
                            mark(ui, resp.rect, after, true);
                        }
                        if released {
                            if let Some(carried) = carried.clone() {
                                push_action(&mut intent, carried, id, l);
                            }
                        }
                    } else if resp.clicked() {
                        intent.enter = Some(id.clone());
                    }
                    if dragging && pointer.map(|p| resp.rect.contains(p)).unwrap_or(false) {
                        drop_at = Some(Some(id.clone()));
                        drop_highlight = Some((resp.rect, 8));
                    }
                    intent.folder(id, act);
                }
                for (id, name, icon, color, sub, folder) in &files {
                    let resp = card(
                        ui,
                        icons,
                        id,
                        *icon,
                        None,
                        *color,
                        name,
                        sub,
                        sel.as_deref() == Some(id.as_str()),
                    );
                    resp.clone().dnd_set_drag_payload(Drag::Entry(id.clone()));
                    match entry_menu(icons, &resp, &tree, folder.clone(), *icon) {
                        Some(EntryAct::Open) => intent.open = Some(id.clone()),
                        Some(EntryAct::Move(dest)) => {
                            intent.moves.push((Drag::Entry(id.clone()), dest));
                        }
                        Some(EntryAct::Delete) => intent.del = Some(id.clone()),
                        None => {}
                    }
                    if let Some(l) = landing(ui, resp.rect, true, false) {
                        if let Landing::Side { after } = l {
                            mark(ui, resp.rect, after, true);
                        }
                        if released {
                            if let Some(carried) = carried.clone() {
                                push_action(&mut intent, carried, id, l);
                            }
                        }
                    } else if resp.clicked() {
                        intent.open = Some(id.clone());
                    }
                }
            });
        },
    );

    if let Some(target) = drop_at {
        app.drop_at = Some(target);
    }
    if let Some(highlight) = drop_highlight {
        app.drop_highlight = Some(highlight);
    }
    apply(app, intent);
}

/// One card, filling exactly `CARD`. Returns its response; drop handling is up
/// to the caller. Allocates the space itself so `horizontal_wrapped` can wrap
/// the grid (a `Frame` would not — it allocates the rect after the content).
#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut Ui,
    icons: &theme::Icons,
    id: &str,
    icon: theme::Icon,
    brand: Option<theme::Brand>,
    color: Color32,
    title: &str,
    sub: &str,
    selected: bool,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(CARD, Sense::hover());
    let surface = theme::Surface::reserve(ui);

    let mut content = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(12.0, 10.0)))
            .layout(Layout::top_down(Align::Min)),
    );
    if let Some(img) = icons.glyph(brand, icon, responsive::grow(ui.ctx(), 20.0), color) {
        content.add(img);
    }
    content.add_space(6.0);
    content.add(
        egui::Label::new(RichText::new(title).strong().color(theme::palette().text)).truncate(),
    );
    if !sub.is_empty() {
        content.add(
            egui::Label::new(RichText::new(sub).small().color(theme::palette().faint)).truncate(),
        );
    }
    drop(content);

    // Registered after the content: labels sense clicks and drags too (text
    // selection), and only the topmost widget wins.
    let resp = ui
        .interact(rect, ui.id().with(("card", id)), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let label = if sub.is_empty() {
        title.to_owned()
    } else {
        format!("{title}, {sub}")
    };
    resp.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            selected,
            label.as_str(),
        )
    });
    theme::focus(ui, &resp);
    let dragging = resp.dragged();
    if dragging {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    let (fill, line) = theme::state(selected || dragging || hot(ui, rect), resp.hovered());
    surface.paint(ui, rect, theme::radius(8), fill, line);
    resp
}

fn hot(ui: &Ui, rect: Rect) -> bool {
    let ctx = ui.ctx();
    egui::DragAndDrop::has_payload_of_type::<Drag>(ctx)
        && ctx.pointer_interact_pos().is_some_and(|p| rect.contains(p))
}

/// Side of the pencil's own square: the grid hangs it on a card's corner, the
/// tree parks it left of the folder's count, and neither row moves for it.
const EDIT: f32 = 22.0;

/// Small pencil centred on `at`. Painted and interacted directly (never allocated in the layout), so it does not displace the grid or the tree rows.
fn edit_button(ui: &mut Ui, icons: &theme::Icons, id: &str, at: egui::Pos2) -> bool {
    let Some(img) = icons.image(theme::Icon::Pencil, 13.0, theme::palette().muted) else {
        return false;
    };
    let btn = Rect::from_center_size(at, egui::vec2(EDIT, EDIT));
    let resp = ui.interact(btn, ui.id().with(("folder-edit", id)), Sense::CLICK);
    if resp.hovered() {
        ui.painter()
            .rect_filled(btn, theme::radius(5), theme::palette().elev);
    }
    img.paint_at(
        ui,
        Rect::from_center_size(btn.center(), egui::vec2(13.0, 13.0)),
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("edit folder")
        .clicked()
}

/// The payload a drop on `rect` is carrying, once the pointer lets go over it.
fn drop_on(ui: &Ui, rect: Rect) -> Option<Drag> {
    let ctx = ui.ctx();
    if !ctx.input(|i| i.pointer.any_released()) {
        return None;
    }
    if !ctx.pointer_interact_pos().is_some_and(|p| rect.contains(p)) {
        return None;
    }
    egui::DragAndDrop::take_payload::<Drag>(ctx).map(|p| (*p).clone())
}

/// Folder editor as a floating modal, like the entry editor — the grid keeps
/// the full width. It opens from the ✎ button (`editing_folder`), not from a
/// plain click, which opens the folder it names. A backdrop click or Esc closes
/// it.
/// The "Move to…" picker as a modal: the folder tree with the drive root, plus
/// Cancel. Replaces the popup menu, which was awkward on a phone. Picking a
/// destination moves the entry and closes the modal.
pub fn move_modal(ctx: &egui::Context, app: &mut App) {
    let moving = app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.moving_entry.clone());
    let Some(sel) = moving else { return };

    let screen = crate::shared::ui::chrome::body(ctx);
    let width = (screen.width() - 56.0).clamp(240.0, 420.0);

    let open = theme::dialog(ctx, egui::Id::new("move-entry"), |ui| {
        theme::width(ui, width);
        let mut closed = theme::head(ui, &app.icons, "Move to…");
        let tree = app
            .boxes
            .session
            .as_ref()
            .map(|s| move_menu::tree(&s.vault))
            .unwrap_or_default();
        let current = app.boxes.session.as_ref().and_then(|s| {
            s.vault
                .drive
                .iter()
                .find(|i| i.id() == sel)
                .and_then(|i| i.folder().map(str::to_owned))
        });
        if let Some(dest) = move_menu::rows(ui, &app.icons, &tree, current) {
            app.boxes
                .move_into(crate::app::Drag::Entry(sel.clone()), dest, &mut app.shell);
            closed = true;
        }
        ui.add_space(12.0);
        if ui
            .add(egui::Button::new(crate::shared::i18n::t("Cancel")))
            .clicked()
        {
            closed = true;
        }
        closed
    });

    if !open {
        if let Some(s) = app.boxes.session.as_mut() {
            s.moving_entry = None;
        }
    }
}

/// The create-folder modal: the name is held outside the vault, so opening it
/// writes nothing. Create (or Enter in the field) puts the folder in the box;
/// Cancel, the cross, Esc and the backdrop all discard it.
pub fn create_modal(ctx: &egui::Context, app: &mut App) {
    let Some(pending) = app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.new_folder.clone())
    else {
        return;
    };

    let screen = crate::shared::ui::chrome::body(ctx);
    let width = (screen.width() - 56.0).clamp(240.0, 460.0);
    let mut name = pending.name.clone();
    let mut created = false;
    let mut discarded = false;

    let open = theme::dialog(ctx, egui::Id::new("drive-new-folder"), |ui| {
        theme::width(ui, width);
        let (closed, _changed, field) = theme::record_head(
            ui,
            &app.icons,
            true,
            |ui| {
                if let Some(img) = app.icons.image(
                    theme::Icon::Folder,
                    responsive::grow(ui.ctx(), 15.0),
                    theme::palette().text,
                ) {
                    ui.add(img);
                }
            },
            &mut name,
            "folder name",
            |_| {},
        );
        // Enter in the field is the Create: the single-line field gives the key
        // up, and the dialog would otherwise just close on it.
        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            created = true;
        }
        if ui.ctx().memory(|m| m.focused().is_none())
            && !ui.ctx().input(|i| i.key_pressed(egui::Key::Enter))
        {
            let len = name.chars().count();
            theme::select_all(ui, &field, len);
        }
        ui.add_space(24.0);
        let (accept, refuse) = theme::actions(
            ui,
            ctx,
            |ui| {
                app.icons
                    .labeled(ui, theme::Icon::FolderPlus, 15.0, theme::ACCENT, "Create")
            },
            |ui| ui.button(crate::shared::i18n::t("Cancel")),
        );
        created |= accept;
        discarded |= refuse;
        closed || created || discarded
    });

    if created {
        if let Some(id) = app.boxes.add_folder(pending.parent.clone(), name.clone()) {
            if let Some(s) = app.boxes.session.as_mut() {
                s.selected = Some(id);
            }
        }
    }
    if let Some(s) = app.boxes.session.as_mut() {
        if created || discarded || !open {
            s.new_folder = None;
        } else if let Some(pending) = s.new_folder.as_mut() {
            // Keep what was typed while the dialog stays open.
            pending.name = name;
        }
    }
}

pub fn modal(ctx: &egui::Context, app: &mut App) {
    let editing = app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.editing_folder.clone());
    let Some(id) = editing.filter(|id| app.boxes.is_folder(id)) else {
        return;
    };

    let screen = crate::shared::ui::chrome::body(ctx);
    let width = (screen.width() - 56.0).clamp(240.0, 460.0);

    let open = theme::dialog(ctx, egui::Id::new("drive-folder"), |ui| {
        theme::width(ui, width);
        folder(ui, app, &id)
    });

    if !open {
        if let Some(s) = app.boxes.session.as_mut() {
            s.editing_folder = None;
        }
    }
}

fn folder(ui: &mut Ui, app: &mut App, id: &str) -> bool {
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return false;
    };
    let Some(idx) = s.vault.folders.iter().position(|f| f.id == id) else {
        s.selected = None;
        return false;
    };

    let dirs = folders::children(&s.vault, Some(id)).len();
    let files = folders::entries(&s.vault, Some(id)).len();
    let trail = folders::path(&s.vault, Some(id))
        .iter()
        .map(|f| f.name.clone())
        .collect::<Vec<_>>()
        .join(" › ");
    let mut changed = false;

    let (closed, named, name) = theme::record_head(
        ui,
        icons,
        true,
        |ui| {
            if let Some(img) = icons.image(
                theme::Icon::Folder,
                responsive::grow(ui.ctx(), 15.0),
                theme::palette().text,
            ) {
                ui.add(img);
            }
        },
        &mut s.vault.folders[idx].name,
        "folder name",
        |_| {},
    );
    // Give the editor the keyboard with the name picked, but never take focus back
    // from a field — and never after Enter has just let it go, or the dialog's
    // Enter-as-OK would find the focus re-taken and not close.
    if ui.ctx().memory(|m| m.focused().is_none())
        && !ui.ctx().input(|i| i.key_pressed(egui::Key::Enter))
    {
        let len = s.vault.folders[idx].name.chars().count();
        theme::select_all(ui, &name, len);
    }
    changed |= named;
    // `closed` is returned to the dialog's closure.
    let _ = closed;
    ui.add_space(0.0);
    theme::hint(
        ui,
        &crate::shared::i18n::tf(
            "{trail} · {dirs} folders · {files} items",
            &[("trail", &trail), ("dirs", &dirs), ("files", &files)],
        ),
    );
    ui.add_space(24.0);
    if icons
        .button(
            ui,
            theme::Icon::Trash,
            16.0,
            theme::palette().error,
            "Delete folder",
        )
        .clicked()
    {
        s.confirm = Some(Confirm::Folder(id.to_owned()));
    }

    if changed {
        s.vault.folders[idx].modified = now_secs();
        s.mark_dirty();
    }
    closed
}
