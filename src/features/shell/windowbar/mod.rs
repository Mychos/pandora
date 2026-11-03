//! Custom window chrome. Under a finger the bar keeps Back and the box actions, and
//! the panes move to the app's floor over the phone's own navigation.

#[cfg(not(target_os = "android"))]
use crate::app::Session;
use crate::app::{App, Screen, StatusKind, Tab};
use crate::shared::theme;
use crate::shared::ui::{chrome, responsive};
/// The pointer state a native move or resize takes with it, which every desktop has.
#[cfg(not(target_os = "android"))]
use egui::PointerState;
/// macOS resizes a borderless window itself and does not support `BeginResize`, so it
/// gets no strips: one would swallow the press the system needs.
#[cfg(not(any(target_os = "android", target_os = "macos")))]
use egui::ResizeDirection;
use egui::{pos2, vec2, Align, Color32, Context, CornerRadius, Id, Layout, Rect, Ui};
#[cfg(not(target_os = "android"))]
use egui::{CursorIcon, PointerButton, RichText, Sense, Stroke, StrokeKind, ViewportCommand};

/// A bar slot under a pointer; a finger asks [`responsive::grow_target`].
const CONTROL: f32 = 38.0;
/// The glyph inside a slot.
const ICON: f32 = 15.0;
/// The Back glyph and its invisible box; `framed` grows both on a narrow screen.
#[cfg(not(target_os = "android"))]
const BACK: (f32, f32) = (15.0, theme::ICON);
/// The window's own edges: drag-region inset, button darkening, tab chip metrics.
#[cfg(not(target_os = "android"))]
const EDGE: f32 = 8.0;
/// As thick as the sides would swallow the middle of every chip, so the top is its own.
#[cfg(not(target_os = "android"))]
const TOP: f32 = 6.0;
/// The corner reaches past its side strip: a diagonal drag is the gesture a mouse
/// cannot aim.
#[cfg(not(any(target_os = "android", target_os = "macos")))]
const CORNER: f32 = 14.0;
#[cfg(not(any(target_os = "android", target_os = "macos")))]
const _: () = assert!(TOP < ICON && CORNER >= EDGE);
#[cfg(not(target_os = "android"))]
const PAD: f32 = 12.0;
#[cfg(not(target_os = "android"))]
const GAP: f32 = 8.0;
#[cfg(not(target_os = "android"))]
const CROSS: f32 = 13.0;
/// The bar's end insets: a thumb needs the buttons clear of the screen edge.
const LEAD: f32 = 10.0;
const TAIL: f32 = 4.0;
const THUMB: f32 = 16.0;
/// Eight is the floor a finger needs between two thumb targets; ten keeps the miss
/// from landing on the pane beside it.
const SPREAD: f32 = 10.0;

/// Back and the panes are all a finger gets on the floor, so every slot holds a thumb target.
const _: () = assert!(chrome::NAV >= 48.0);

/// A bar slot's own width here, and the glyph inside it.
fn slot(ctx: &Context) -> f32 {
    responsive::grow_target(ctx, CONTROL)
}

fn glyph(ctx: &Context) -> f32 {
    responsive::grow(ctx, ICON)
}

fn lead(ctx: &Context) -> f32 {
    if chrome::touch(ctx) {
        THUMB
    } else {
        LEAD
    }
}

fn tail(ctx: &Context) -> f32 {
    if chrome::touch(ctx) {
        THUMB
    } else {
        TAIL
    }
}

/// Called after the window bar, in the same area, so the strips win the outer border.
#[cfg(not(any(target_os = "android", target_os = "macos")))]
pub fn resize(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    for (i, (dir, rect, cursor)) in zones(ctx.viewport_rect()).into_iter().enumerate() {
        let resp = ui
            .interact(rect, Id::new(("pandora-resize", i)), Sense::DRAG)
            .on_hover_cursor(cursor);
        if resp.drag_started() {
            app.shell.moved();
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            handoff(&ctx);
        }
    }
}

/// The compositor keeps the pointer for a native move or resize and never delivers
/// the button release, so egui would hold the press; the native op owns the gesture
/// from here. `emilk/egui#7959`.
#[cfg(not(target_os = "android"))]
fn handoff(ctx: &Context) {
    ctx.input_mut(|i| i.pointer = PointerState::default());
    ctx.stop_dragging();
}

/// The eight border strips, clockwise; the corner reaches past its side, so they never overlap.
#[cfg(not(any(target_os = "android", target_os = "macos")))]
fn zones(s: Rect) -> [(ResizeDirection, Rect, CursorIcon); 8] {
    let (w, top, c) = (EDGE, TOP, CORNER);
    [
        (
            ResizeDirection::North,
            Rect::from_min_max(s.min, pos2(s.max.x, s.min.y + top)),
            CursorIcon::ResizeVertical,
        ),
        (
            ResizeDirection::South,
            Rect::from_min_max(pos2(s.min.x, s.max.y - w), s.max),
            CursorIcon::ResizeVertical,
        ),
        (
            ResizeDirection::West,
            Rect::from_min_max(pos2(s.min.x, s.min.y + c), pos2(s.min.x + w, s.max.y - c)),
            CursorIcon::ResizeHorizontal,
        ),
        (
            ResizeDirection::East,
            Rect::from_min_max(pos2(s.max.x - w, s.min.y + c), pos2(s.max.x, s.max.y - c)),
            CursorIcon::ResizeHorizontal,
        ),
        (
            ResizeDirection::NorthWest,
            Rect::from_min_max(s.min, pos2(s.min.x + c, s.min.y + c)),
            CursorIcon::ResizeNwSe,
        ),
        (
            ResizeDirection::NorthEast,
            Rect::from_min_max(pos2(s.max.x - c, s.min.y), pos2(s.max.x, s.min.y + c)),
            CursorIcon::ResizeNeSw,
        ),
        (
            ResizeDirection::SouthWest,
            Rect::from_min_max(pos2(s.min.x, s.max.y - c), pos2(s.min.x + c, s.max.y)),
            CursorIcon::ResizeNeSw,
        ),
        (
            ResizeDirection::SouthEast,
            Rect::from_min_max(pos2(s.max.x - c, s.max.y - c), s.max),
            CursorIcon::ResizeNwSe,
        ),
    ]
}

/// Above `Order::Foreground`, where `egui::Modal` paints its backdrop: the whole
/// chrome must sit above it.
pub fn show(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    // The bar is the window's own edge; a thumb keeps it at the top and the panes on the floor.
    let strip = chrome::strip(&ctx);
    egui::Panel::top("windowbar")
        .exact_size(strip)
        .show_separator_line(false)
        .frame(egui::Frame::NONE)
        .show(ui, |_| {});
    let floor = chrome::floor(&ctx);
    if floor > 0.0 {
        egui::Panel::bottom("nav")
            .exact_size(floor)
            .show_separator_line(false)
            .frame(egui::Frame::NONE)
            .show(ui, |_| {});
    }
    let screen = ctx.viewport_rect();
    egui::Area::new(Id::new("pandora-chrome"))
        .order(egui::Order::Tooltip)
        .fixed_pos(screen.min)
        // The reserved panel moved `content_rect`; the default constraint would drag it back.
        .constrain_to(screen)
        // A fade would dim the bar on its first frames; this bar never fades.
        .fade_in(false)
        .show(&ctx, |ui| {
            ui.set_width(screen.width());
            ui.set_height(strip);
            bar(ui, app);
            // After the bar, so the strips win the outer edge, even under a tab or a button.
            #[cfg(not(any(target_os = "android", target_os = "macos")))]
            resize(ui, app);
        });
    if floor > 0.0 {
        egui::Area::new(Id::new("pandora-nav"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos2(screen.min.x, screen.max.y - floor))
            .constrain_to(screen)
            .fade_in(false)
            .show(&ctx, |ui| {
                ui.set_width(screen.width());
                ui.set_height(floor);
                nav(ui, app);
            });
    }
}

fn bar(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    let finger = chrome::touch(&ctx);
    let strip = ui.max_rect();
    paint(ui, strip);
    // A phone has no window to drag; a narrow desktop is still borderless.
    #[cfg(not(target_os = "android"))]
    {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let grab = if maximized {
            strip
        } else {
            Rect::from_min_max(
                pos2(strip.min.x + EDGE, strip.min.y + TOP),
                pos2(strip.max.x - EDGE, strip.max.y),
            )
        };
        drag(ui, app, &ctx, grab);
    }
    ui.spacing_mut().item_spacing.x = 0.0;
    ui.allocate_ui_with_layout(strip.size(), Layout::top_down(Align::Min), |ui| {
        ui.horizontal_centered(|ui| {
            theme::flat(ui);
            ui.add_space(lead(&ctx));
            ui.add(app.icons.logo(22.0, theme::ACCENT));
            ui.add_space(12.0);
            // A desktop bar carries Back, the tool and the vault tabs; a thumb keeps only Back here.
            #[cfg(not(target_os = "android"))]
            if !finger {
                let can = app.can_back();
                let (glyph, side) = BACK;
                let back = ui
                    .add_enabled_ui(can, |ui| {
                        app.icons.framed(
                            ui,
                            theme::Icon::Back,
                            glyph,
                            theme::palette().muted,
                            "Back",
                            side,
                        )
                    })
                    .inner;
                if can && theme::marked(ui, back, theme::Edge::Floor).clicked() {
                    app.back();
                }
                if tool(ui, app, theme::Edge::Floor) {
                    app.screen = Screen::Generate;
                }
                ui.add_space(12.0);
                strip_tabs(ui, app);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // With no title bar these are the only way to control the window, so they never leave.
                #[cfg(not(target_os = "android"))]
                {
                    let ctx = ui.ctx().clone();
                    controls(ui, &ctx);
                    // Right to left, so added after: the pin sits at the window's edge,
                    // to the left of minimise, and never past close.
                    pin(ui, app);
                }
                ui.add_space(tail(&ctx));
                // Right to left, so the gear is the rightmost.
                if finger {
                    settings_button(ui, app);
                    lock_button(ui, app);
                }
                // The app's settings stand beside the box's; both are read before any box opens.
                crate::features::settings::ui::button(ui, app);
                #[cfg(not(target_os = "android"))]
                if !finger {
                    browsers(ui, app);
                }
            });
        });
    });
}

/// The floor navigation: generator far left, the box's panes centred, Back far right;
/// same layer as the top area, so a modal cannot cover it.
fn nav(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    let strip = ui.max_rect();
    // The phone's own bar goes under this one: what is left below is its room.
    let bar = Rect::from_min_max(strip.min, pos2(strip.max.x, strip.min.y + chrome::NAV));
    paint_nav(ui, strip, bar);
    ui.spacing_mut().item_spacing.x = 0.0;
    ui.allocate_ui_with_layout(bar.size(), Layout::top_down(Align::Min), |ui| {
        ui.horizontal_centered(|ui| {
            theme::flat(ui);
            ui.add_space(lead(&ctx));
            // The generator reads nothing from a box, so it opens the floor and
            // not the panes: a thumb finds it without hunting for the box first.
            if tool(ui, app, theme::Edge::Ceiling) {
                app.screen = Screen::Generate;
            }
            // The tool and the scan are not pages of the box: while either is
            // the screen, the pane it was reached from keeps its tab but drops
            // The floor marks one place: the one the user is at.
            let live = app.screen == Screen::Main;
            let mut go = None;
            if let Some(s) = app.boxes.session.as_ref() {
                let want = s.tab;
                // The three panes are the bar's centre; generator left and Back right never shift them.
                let row = ui.max_rect();
                let panes = Rect::from_center_size(
                    row.center(),
                    vec2(slot(&ctx) * 3.0 + SPREAD * 2.0, row.height()),
                );
                let mut middle = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(panes)
                        .layout(Layout::left_to_right(Align::Center)),
                );
                for (i, (pane, icon, label)) in [
                    (Tab::Passwords, theme::Icon::Key, "Passwords"),
                    (Tab::Drive, theme::Icon::Folder, "Files"),
                    (Tab::Codes, theme::Icon::RefreshCw, "Codes"),
                ]
                .into_iter()
                .enumerate()
                {
                    if i > 0 {
                        middle.add_space(SPREAD);
                    }
                    if destination(&mut middle, &app.icons, label, icon, live && want == pane) {
                        go = Some(pane);
                    }
                }
                drop(middle);
            }
            if let Some(pane) = go {
                app.boxes.pane(pane);
                app.screen = Screen::Main;
            }
            // Back sits at the far end, where a thumb finds it without crossing the panes.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(tail(&ctx));
                let can = app.can_back();
                let back = ui
                    .add_enabled_ui(can, |ui| {
                        action(
                            ui,
                            &app.icons,
                            Id::new("nav-back"),
                            theme::Icon::Back,
                            "Back",
                            theme::palette().muted,
                            false,
                            theme::Edge::Ceiling,
                        )
                    })
                    .inner;
                if can && back.clicked() {
                    app.back();
                }
            });
        });
    });
}

/// One action on a bar: a slot the bar's height, glyph centred, `off` at rest and
/// `on` in the accent.
#[allow(clippy::too_many_arguments)]
fn action(
    ui: &mut Ui,
    icons: &theme::Icons,
    id: egui::Id,
    icon: theme::Icon,
    label: &str,
    off: Color32,
    on: bool,
    edge: theme::Edge,
) -> egui::Response {
    let ctx = ui.ctx().clone();
    let tint = if on { theme::ACCENT } else { off };
    let tint = if ui.is_enabled() {
        tint
    } else {
        tint.gamma_multiply(ui.visuals().disabled_alpha())
    };
    let enabled = ui.is_enabled();
    let resp = theme::slot(ui, slot(&ctx), id, on, ui.available_height(), edge, |ui| {
        ui.add_space((slot(&ctx) - glyph(&ctx)) * 0.5);
        if let Some(img) = icons.image(icon, glyph(&ctx), tint) {
            ui.add(img);
        }
    });
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, on, label)
    });
    resp.on_hover_text(label)
}

/// One pane of the navigation: icon and the accent rule on the bar's ceiling.
fn destination(
    ui: &mut Ui,
    icons: &theme::Icons,
    label: &str,
    icon: theme::Icon,
    on: bool,
) -> bool {
    action(
        ui,
        icons,
        Id::new(("nav", label)),
        icon,
        label,
        theme::palette().muted,
        on,
        theme::Edge::Ceiling,
    )
    .clicked()
}

/// The lock acts on the whole app, so a phone keeps it in reach at the top.
fn lock_button(ui: &mut Ui, app: &mut App) {
    if app.boxes.session.is_none() {
        return;
    }
    let ctx = ui.ctx().clone();
    if action(
        ui,
        &app.icons,
        Id::new("bar-lock"),
        theme::Icon::LockKeyhole,
        "Lock box",
        theme::palette().error,
        false,
        theme::Edge::Floor,
    )
    .clicked()
    {
        app.lock(&ctx);
    }
}

/// Box settings are a page of the open box, and the only way to it once the panes
/// left the bar; app settings are the sliders beside it.
fn settings_button(ui: &mut Ui, app: &mut App) {
    if app.boxes.session.is_none() {
        return;
    }
    // Settings is a page of the box: the tool or scan leaving takes its mark.
    let on = app.screen == Screen::Main
        && app
            .boxes
            .session
            .as_ref()
            .is_some_and(|s| s.tab == Tab::Settings);
    if action(
        ui,
        &app.icons,
        Id::new("bar-settings"),
        theme::Icon::Settings,
        crate::shared::i18n::t("Box settings"),
        theme::palette().muted,
        on,
        theme::Edge::Floor,
    )
    .clicked()
    {
        app.boxes.pane(Tab::Settings);
        app.screen = Screen::Main;
    }
}

fn paint(ui: &Ui, strip: Rect) {
    // The bar is raised at its top and settles towards the body below it.
    let mut mesh = egui::Mesh::default();
    for (pos, color) in [
        (strip.left_top(), theme::palette().elev),
        (strip.right_top(), theme::palette().elev),
        (strip.left_bottom(), theme::palette().panel),
        (strip.right_bottom(), theme::palette().panel),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: egui::epaint::WHITE_UV,
            color,
        });
    }
    mesh.indices.extend([0, 1, 2, 2, 1, 3]);
    ui.painter().add(egui::Shape::mesh(mesh));
    // The hairline sits on the edge the body is on.
    let seam = Rect::from_min_max(pos2(strip.min.x, strip.max.y - 1.0), strip.max);
    ui.painter()
        .rect_filled(seam, CornerRadius::ZERO, theme::palette().hair);
}

/// The floor's paint: body meets navigation at its ceiling, phone's room below keeps the colour.
fn paint_nav(ui: &Ui, strip: Rect, bar: Rect) {
    ui.painter()
        .rect_filled(strip, CornerRadius::ZERO, theme::palette().panel);
    let seam = Rect::from_min_max(strip.min, pos2(strip.max.x, bar.min.y + 1.0));
    ui.painter()
        .rect_filled(seam, CornerRadius::ZERO, theme::palette().hair);
}

/// Move the window from the bar; only a desktop has one to move.
#[cfg(not(target_os = "android"))]
fn drag(ui: &mut Ui, app: &mut App, ctx: &Context, grab: Rect) {
    // `CLICK | DRAG`: a press alone must not start a move, or the release never returns.
    let resp = ui
        .interact(
            grab,
            Id::new("pandora-windowbar"),
            Sense::CLICK | Sense::DRAG,
        )
        .on_hover_cursor(CursorIcon::Grab);
    if resp.drag_started_by(PointerButton::Primary) {
        app.shell.moved();
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        handoff(ctx);
    }
    if resp.double_clicked() {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
}

/// Only the vaults get tabs; the pane strip below the bar is a different thing.
#[cfg(not(target_os = "android"))]
fn strip_tabs(ui: &mut Ui, app: &mut App) {
    let Some(current) = app.boxes.session.as_ref() else {
        return;
    };
    // Only the box screen has a vault on show; Sources and the tool leave no active tab.
    let live = app.screen == Screen::Main;
    let at = app.boxes.active.min(app.boxes.tabs.len());
    let mut order: Vec<&Session> = Vec::with_capacity(app.boxes.tabs.len() + 1);
    order.extend(app.boxes.tabs[..at].iter());
    order.push(current);
    order.extend(app.boxes.tabs[at..].iter());

    let (mut go, mut shut) = (None, None);
    for (i, session) in order.iter().enumerate() {
        match vault_tab(ui, &session.vault.name, i == at, live) {
            Press::Select => go = Some(i),
            Press::Close => shut = Some(i),
            Press::None => {}
        }
    }
    plus(ui, app);
    if let Some(i) = shut {
        app.close(i);
    } else if let Some(i) = go {
        app.switch(i);
    }
}

#[cfg(not(target_os = "android"))]
enum Press {
    None,
    Select,
    Close,
}

/// `current`: the tab holding the box on screen, name bright and cross always shown.
/// `live`: whether it is the screen, since the accent rule belongs to the window on show.
#[cfg(not(target_os = "android"))]
fn vault_tab(ui: &mut Ui, name: &str, current: bool, live: bool) -> Press {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let color = if current {
        theme::palette().text
    } else {
        theme::palette().muted
    };
    let text = ui.painter().layout_no_wrap(name.to_owned(), font, color);
    let width = PAD + text.size().x + GAP + CROSS + PAD;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, ui.available_height()), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, current, name)
    });
    theme::focus(ui, &resp);

    if resp.hovered() && !current {
        ui.painter().rect_filled(
            Rect::from_min_max(rect.min, pos2(rect.max.x, rect.max.y - 1.0)),
            CornerRadius::ZERO,
            theme::palette().wash,
        );
    }
    let rule = if (current && live) || resp.is_pointer_button_down_on() {
        Some(theme::ACCENT)
    } else if resp.hovered() {
        Some(theme::palette().faint)
    } else {
        None
    };
    if let Some(color) = rule {
        ui.painter().rect_filled(
            Rect::from_min_max(pos2(rect.min.x, rect.max.y - 2.0), rect.max),
            CornerRadius::ZERO,
            color,
        );
    }
    let cross = Rect::from_center_size(
        pos2(rect.max.x - PAD - CROSS * 0.5, rect.center().y),
        vec2(CROSS, CROSS),
    );
    let pointer = resp.hover_pos();
    if current || pointer.is_some() {
        let over = pointer.is_some_and(|p| cross.contains(p));
        Glyph::Close.paint(
            ui.painter(),
            cross.center(),
            if over {
                theme::palette().text
            } else {
                theme::palette().muted
            },
        );
    }
    ui.painter().galley(
        pos2(rect.min.x + PAD, rect.center().y - text.size().y * 0.5),
        text,
        color,
    );

    if !resp.clicked() {
        return Press::None;
    }
    if pointer.is_some_and(|p| cross.contains(p)) {
        Press::Close
    } else {
        Press::Select
    }
}

#[cfg(not(target_os = "android"))]
fn plus(ui: &mut Ui, app: &mut App) {
    let open = app.icons.button(
        ui,
        theme::Icon::Plus,
        13.0,
        theme::palette().muted,
        "Open another box",
    );
    if theme::marked(ui, open, theme::Edge::Floor).clicked() {
        app.add();
    }
}

/// The password tool reads nothing from a box, so it rides the bar, not a pane strip.
fn tool(ui: &mut Ui, app: &App, edge: theme::Edge) -> bool {
    let on = app.screen == Screen::Generate;
    action(
        ui,
        &app.icons,
        Id::new("bar-generate"),
        theme::Icon::Dices,
        "Generate a password",
        theme::palette().muted,
        on,
        edge,
    )
    .clicked()
}

/// Browsers the user allowed: a chip while any is open, naming the family, because a
/// `chrome-extension://` origin does not say which Chromium browser it is.
#[cfg(not(target_os = "android"))]
fn browsers(ui: &mut Ui, app: &mut App) {
    if app.prefs.browsers.is_empty() {
        return;
    }
    let linked = app.prefs.browsers.clone();
    let mut forget: Option<usize> = None;
    let ctx = ui.ctx().clone();
    let resp = theme::slot(
        ui,
        slot(&ctx),
        Id::new("bar-browsers"),
        false,
        ui.available_height(),
        theme::Edge::Floor,
        |ui| {
            ui.add_space((slot(&ctx) - glyph(&ctx)) * 0.5);
            if let Some(img) = app
                .icons
                .image(theme::Icon::Puzzle, glyph(&ctx), theme::ACCENT)
            {
                ui.add(img);
            }
        },
    );
    let count = match linked.len() {
        1 => crate::shared::i18n::t("1 browser allowed").to_owned(),
        n => crate::shared::i18n::tf("{count} browsers allowed", &[("count", &n)]),
    };
    let said = count.as_str();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, said));
    let resp = resp.on_hover_text(crate::shared::i18n::tf(
        "{count} — click to manage",
        &[("count", &count)],
    ));

    egui::Popup::menu(&resp).show(|ui| {
        theme::menu(ui);
        // `with_layout` claims every pixel; pin the width so the action column lands on one x.
        ui.set_width(220.0);
        theme::field_label(ui, "Allowed browsers");
        ui.add_space(10.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        for (i, caller) in linked.iter().enumerate() {
            let (name, note) = crate::features::bridge::model::browser(caller);
            ui.horizontal(|ui| {
                // `Min`, so the action sits on the row's first line, where the name is.
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    // `framed` and not `button`: a 28pt slot leaves a dead band under the last row.
                    if app
                        .icons
                        .framed(
                            ui,
                            theme::Icon::Trash,
                            14.0,
                            theme::palette().muted,
                            &crate::shared::i18n::tf(
                                "Forget {name} — it would have to be allowed again",
                                &[("name", &name)],
                            ),
                            20.0,
                        )
                        .clicked()
                    {
                        forget = Some(i);
                    }
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(name).color(theme::palette().text));
                        if let Some(note) = note {
                            ui.label(RichText::new(note).small().color(theme::palette().muted));
                        }
                    });
                });
            });
        }
    });

    if let Some(i) = forget {
        app.prefs.browsers.remove(i);
        crate::features::settings::model::save(&app.prefs);
    }
}

/// The pin: this window above every other, for a box consulted while something else
/// is being filled in. Remembered, because a window the user pinned once is a window
/// they expect pinned the next time.
#[cfg(not(target_os = "android"))]
fn pin(ui: &mut Ui, app: &mut App) {
    let on = app.prefs.on_top;
    // Wayland gives a client no way to sit above the others, and winit answers that
    // request there with an empty function. `main.rs` dropped `WAYLAND_DISPLAY` when
    // the session was forced onto X11, so this reads the backend the window really has.
    #[cfg(target_os = "linux")]
    let stuck = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    #[cfg(not(target_os = "linux"))]
    let stuck = false;
    let (rect, _) = ui.allocate_exact_size(vec2(CONTROL, ui.available_height()), Sense::HOVER);
    let resp = ui.interact(rect, Id::new("pandora-pin"), Sense::click());
    let tooltip = if stuck {
        "this session does not let an app stay on top: start it with PANDORA_X11=1"
    } else if on {
        "stop keeping this window on top"
    } else {
        "keep this window on top"
    };
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            crate::shared::i18n::t(tooltip),
        )
    });
    // The mark leans while the window is like every other and stands up when it is
    // pinned, the way Jota's does; the colour follows the same reading.
    let turned = ui.ctx().animate_bool_with_time(
        Id::new("pandora-pin"),
        on && !stuck,
        ui.style().animation_time,
    );
    let colour = if stuck {
        theme::palette().faint
    } else {
        theme::palette().muted.lerp_to_gamma(theme::ACCENT, turned)
    };
    if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(
            Rect::from_min_max(rect.min, pos2(rect.max.x, rect.max.y - 1.0)),
            CornerRadius::ZERO,
            theme::palette().hover,
        );
    }
    // Painted, not placed: the window controls beside it are vectors, and a 14pt
    // sprite between them reads as a smudge whatever the filter.
    let angle = 25f32.to_radians() * (1.0 - turned);
    let lean = egui::emath::Rot2::from_angle(angle);
    let centre = rect.center() + vec2(0.0, turned * 1.5);
    let at = |x: f32, y: f32| centre + lean * vec2(x, y);
    let ink = egui::Stroke::new(1.3, colour);
    let painter = ui.painter();
    painter.add(egui::Shape::convex_polygon(
        vec![at(-3.0, -7.0), at(3.0, -7.0), at(4.2, 0.0), at(-4.2, 0.0)],
        Color32::TRANSPARENT,
        ink,
    ));
    painter.line_segment([at(-5.2, 0.0), at(5.2, 0.0)], ink);
    painter.line_segment([at(0.0, 0.0), at(0.0, 6.6)], ink);
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if resp.clicked() {
        // A session that cannot pin still answers the click: a dimmed mark that says
        // nothing back is a button the user keeps pressing.
        if stuck {
            app.shell.set_status(
                "this session does not let an app stay on top: start it with PANDORA_X11=1",
                StatusKind::Info,
            );
        } else {
            app.prefs.on_top = !on;
            crate::features::settings::model::save(&app.prefs);
            app.level(ui.ctx());
        }
    }
}

fn controls(ui: &mut Ui, ctx: &Context) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    if control(ui, Glyph::Close, theme::palette().error).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    let glyph = if maximized {
        Glyph::Restore
    } else {
        Glyph::Maximize
    };
    if control(ui, glyph, theme::palette().hover).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
    if control(ui, Glyph::Minimize, theme::palette().hover).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

#[cfg(not(target_os = "android"))]
fn control(ui: &mut Ui, glyph: Glyph, hover: Color32) -> egui::Response {
    let (rect, mut resp) =
        ui.allocate_exact_size(vec2(CONTROL, ui.available_height()), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, glyph.label()));
    if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(
            Rect::from_min_max(rect.min, pos2(rect.max.x, rect.max.y - 1.0)),
            CornerRadius::ZERO,
            hover,
        );
    }
    let color = match (resp.hovered() || resp.has_focus(), glyph) {
        (true, Glyph::Close) => Color32::WHITE,
        (true, _) => theme::palette().text,
        (false, _) => theme::palette().muted,
    };
    glyph.paint(ui.painter(), rect.center(), color);
    resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    resp
}

#[cfg(not(target_os = "android"))]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Glyph {
    Minimize,
    Maximize,
    Restore,
    Close,
}

#[cfg(not(target_os = "android"))]
impl Glyph {
    fn label(self) -> &'static str {
        match self {
            Self::Minimize => "Minimize window",
            Self::Maximize => "Maximize window",
            Self::Restore => "Restore window",
            Self::Close => "Close window",
        }
    }

    fn paint(self, painter: &egui::Painter, center: egui::Pos2, color: Color32) {
        let stroke = Stroke::new(1.1, color);
        match self {
            Self::Minimize => {
                painter.line_segment([center + vec2(-4.5, 0.0), center + vec2(4.5, 0.0)], stroke);
            }
            Self::Maximize => {
                painter.rect_stroke(
                    Rect::from_center_size(center, vec2(9.0, 9.0)),
                    CornerRadius::ZERO,
                    stroke,
                    StrokeKind::Inside,
                );
            }
            Self::Restore => {
                painter.rect_stroke(
                    Rect::from_center_size(center + vec2(-1.8, 1.8), vec2(8.0, 8.0)),
                    CornerRadius::ZERO,
                    stroke,
                    StrokeKind::Inside,
                );
                painter.rect_stroke(
                    Rect::from_center_size(center + vec2(1.8, -1.8), vec2(8.0, 8.0)),
                    CornerRadius::ZERO,
                    stroke,
                    StrokeKind::Inside,
                );
            }
            Self::Close => {
                let d = 4.0;
                painter.line_segment([center + vec2(-d, -d), center + vec2(d, d)], stroke);
                painter.line_segment([center + vec2(-d, d), center + vec2(d, -d)], stroke);
            }
        }
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests;
