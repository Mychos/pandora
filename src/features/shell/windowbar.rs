//! Custom window chrome for the OS title bar disabled in `main.rs`: the window's
//! top, where a mouse looks for it. Under a finger the bar stays at the top and
//! keeps Back and the box's own actions, while the panes move to the app's floor —
//! the phone's navigation, standing on the phone's own bar.

#[cfg(not(target_os = "android"))]
use crate::app::Session;
use crate::app::{App, Screen, Tab};
use crate::shared::theme;
use crate::shared::ui::{chrome, responsive};
use egui::{pos2, vec2, Align, Color32, Context, CornerRadius, Id, Layout, Rect, Ui};
#[cfg(not(target_os = "android"))]
use egui::{CursorIcon, PointerButton, RichText, Sense, Stroke, StrokeKind, ViewportCommand};
/// What only the edge strips need. macOS resizes a borderless window with its own
/// border and `BeginResize` is not supported there, so it has no strips: a strip
/// would swallow the press the system needs.
#[cfg(not(any(target_os = "android", target_os = "macos")))]
use egui::{PointerState, ResizeDirection};

/// A bar slot under a pointer: a window button on the desktop, a chip beside it
/// otherwise. A finger asks [`responsive::grow_target`] for its own.
const CONTROL: f32 = 38.0;
/// The glyph inside a slot.
const ICON: f32 = 15.0;
/// The Back glyph and the invisible box behind it. `framed` grows both on a
/// narrow screen, so a phone gets a whole target without a second set of numbers.
/// Desktop only: a phone's bar keeps Back on the floor, and its own numbers live
/// with `nav`.
#[cfg(not(target_os = "android"))]
const BACK: (f32, f32) = (15.0, theme::ICON);
/// The window's own edges: how far the drag region stays off them, how a window
/// button darkens, and the tab chip's metrics. None of it outlives a desktop
/// window.
#[cfg(not(target_os = "android"))]
const EDGE: f32 = 8.0;
/// The top strip is the bar's own edge: the bar's chips are `ICON` tall and sit
/// at its very top, so a strip as thick as the sides would swallow the middle of
/// every one of them.
#[cfg(not(target_os = "android"))]
const TOP: f32 = 6.0;
/// How far a corner strip reaches along the edge it shares with a side strip: a
/// diagonal drag is the one gesture a mouse cannot aim, so the corner is the
/// wider target.
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
/// The bar's own insets at its two ends: a thumb wants the buttons clear of the
/// screen's edge, where a mouse pointer does not care.
const LEAD: f32 = 10.0;
const TAIL: f32 = 4.0;
const THUMB: f32 = 16.0;
/// The air two destinations keep between them on the floor. Three thumb targets
/// edge to edge are one wide target: a miss lands on the pane beside the one
/// meant, and the wash of the one pressed bleeds into its neighbour. Eight is
/// the floor a finger needs; ten makes the break visible as well.
const SPREAD: f32 = 10.0;

/// Back and the panes are all the finger gets on the floor of a phone, so both
/// the bar and every slot on it must hold a target a thumb can land on.
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

/// Called after the window bar, in the same area, so the strips win the outer
/// border.
#[cfg(not(any(target_os = "android", target_os = "macos")))]
pub fn resize(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    for (i, (dir, rect, cursor)) in zones(ctx.viewport_rect()).into_iter().enumerate() {
        let resp = ui
            .interact(rect, Id::new(("pandora-resize", i)), Sense::DRAG)
            .on_hover_cursor(cursor);
        if resp.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            handoff(&ctx);
        }
    }
}

/// The compositor grabs the pointer for a native move or resize and never
/// delivers the button release, so egui would keep the press down — and hover
/// nothing at all — until the next click. The native op owns the gesture from
/// here: egui's pointer state goes with it.
/// `emilk/egui#7959`.
#[cfg(not(any(target_os = "android", target_os = "macos")))]
fn handoff(ctx: &Context) {
    ctx.input_mut(|i| i.pointer = PointerState::default());
    ctx.stop_dragging();
}

/// The eight border strips, clockwise from the top edge. A corner reaches past
/// the strip it sits on, so the two never overlap.
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

/// Paints and interacts in a layer above `Order::Foreground`, where `egui::Modal`
/// lives: a modal's backdrop is painted over the whole window, so the whole
/// chrome — the bars and the resize strips — must sit above.
pub fn show(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    // The bar is the window's own edge: the top, where a mouse looks for it and
    // where the window buttons are. A thumb gets the same bar there, and its
    // panes at the floor, over the phone's own navigation.
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
        // The reserved panel moved `content_rect` off the bar; the default
        // constraint would drag the bar back into it.
        .constrain_to(screen)
        // A fade would dim the bar on its first frames; this bar never fades.
        .fade_in(false)
        .show(&ctx, |ui| {
            ui.set_width(screen.width());
            ui.set_height(strip);
            bar(ui, app);
            // After the bar, so the strips win the outer border and the very edge
            // resizes even where a tab or a window button reaches it.
            #[cfg(not(any(target_os = "android", target_os = "macos")))]
            resize(ui);
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
    // A phone has no window behind this: nothing to drag. A desktop keeps its
    // drag region at any width — a narrow window is still a borderless window.
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
        drag(ui, &ctx, grab);
    }
    ui.spacing_mut().item_spacing.x = 0.0;
    ui.allocate_ui_with_layout(strip.size(), Layout::top_down(Align::Min), |ui| {
        ui.horizontal_centered(|ui| {
            theme::flat(ui);
            ui.add_space(lead(&ctx));
            ui.add(app.icons.logo(22.0, theme::ACCENT));
            ui.add_space(12.0);
            // A pointer's bar carries Back, the tool and the vault tabs; a
            // thumb's one keeps only Back up here — the panes are on the floor.
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
                // The window's own buttons are the desktop's, narrow or not: with no
                // title bar they are the only way to control the window, so they never
                // leave the bar. They sit flush at the corner, as a window expects.
                #[cfg(not(target_os = "android"))]
                {
                    let ctx = ui.ctx().clone();
                    controls(ui, &ctx);
                }
                ui.add_space(tail(&ctx));
                // A thumb's bar keeps the box's own actions, where the panes would
                // be on the floor: right to left, the gear is the rightmost.
                if finger {
                    settings_button(ui, app);
                    lock_button(ui, app);
                }
                crate::features::settings::ui::language(ui, app);
                #[cfg(not(target_os = "android"))]
                if !finger {
                    browsers(ui, app);
                }
            });
        });
    });
}

/// The app's own navigation at the floor of a narrow window: the generator opens
/// it at the far left, the box's panes occupy its centre and Back closes it at
/// the far right. It is on the top area's layer, so a modal cannot cover it.
fn nav(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    let strip = ui.max_rect();
    // The phone's navigation bar goes under this one rather than over it: the
    // app's own row is the strip's first `NAV` points, and what is left below is
    // the room the phone asked for.
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
            // the mark. The floor marks one place, the one the user is at.
            let live = app.screen == Screen::Main;
            let mut go = None;
            if let Some(s) = app.boxes.session.as_ref() {
                let want = s.tab;
                // The three panes are the bar's centre: the generator holds the
                // left end and Back the right one, so neither shifts them. Their
                // own rect keeps them centred on the whole bar, not on what the
                // generator leaves, and its width carries the air between them.
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
            // Back belongs at the far end of the floor, where a thumb finds it
            // without crossing the panes.
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

/// One action on a bar: a slot the bar's own height, so its wash and its rule
/// reach the edges instead of floating inside it, with the glyph centred. `off`
/// is the tint at rest; `on` lifts it to the accent.
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

/// One pane of the app's navigation: the icon, and the accent rule on the bar's
/// ceiling — the edge the body is on, since this bar is the window's floor.
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

/// The lock acts on the whole app, so a phone keeps it in reach at the top, on
/// the bar's floor where a click's mark lands.
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

/// Settings is a page of the open box, and the only way to it once the panes have
/// left the bar.
fn settings_button(ui: &mut Ui, app: &mut App) {
    if app.boxes.session.is_none() {
        return;
    }
    // Settings is a page of the box too, so the tool or the scan leaving the
    // box behind takes its mark with them.
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
        "Settings",
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

/// The floor's own paint: the body meets the navigation at its ceiling, and the
/// room the phone asked for below keeps the bar's colour.
fn paint_nav(ui: &Ui, strip: Rect, bar: Rect) {
    ui.painter()
        .rect_filled(strip, CornerRadius::ZERO, theme::palette().panel);
    let seam = Rect::from_min_max(strip.min, pos2(strip.max.x, bar.min.y + 1.0));
    ui.painter()
        .rect_filled(seam, CornerRadius::ZERO, theme::palette().hair);
}

/// Move the window from the bar; only a desktop has one to move.
#[cfg(not(target_os = "android"))]
fn drag(ui: &mut Ui, ctx: &Context, grab: Rect) {
    // `CLICK | DRAG`: the press alone must not start a move, or the compositor
    // takes the pointer and the release never comes back.
    let resp = ui
        .interact(
            grab,
            Id::new("pandora-windowbar"),
            Sense::CLICK | Sense::DRAG,
        )
        .on_hover_cursor(CursorIcon::Grab);
    if resp.drag_started_by(PointerButton::Primary) {
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
    // Only the box screen has a vault on show: on Sources (the "+") or the tool,
    // no tab is the active window.
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

/// `current` is the tab holding the box on screen: its name stays bright and its
/// close cross always shows, whatever screen is up. `live` is whether that box is
/// the screen — the accent rule belongs to the window on show, so Sources and the
/// tool leave the strip unmarked without taking the cross away.
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

/// The password tool reads nothing from a box, so it rides the bar and not a box's
/// pane strip: it must open with none. On a phone it is one of the destinations on
/// the floor.
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

/// The browsers the user has allowed to read the box open here: a chip appears
/// while any is, and opens the list. A `chrome-extension://` origin is shared by
/// Chrome, Brave and Edge and the browser never says which, so a row names the
/// family, not the browser — never more than one Chromium row, plus Firefox.
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
        // `with_layout` inside claims every pixel it is not given back, so the
        // width is pinned: the row's action column then lands on one x for every
        // row instead of the popup stretching to the screen and stranding the
        // trash a hand's width from the name.
        ui.set_width(220.0);
        theme::field_label(ui, "Allowed browsers");
        ui.add_space(10.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        for (i, caller) in linked.iter().enumerate() {
            let (name, note) = crate::features::bridge::model::browser(caller);
            ui.horizontal(|ui| {
                // `Min`, so the action sits on the row's first line, where the name
                // is, instead of floating between two lines of a wrapped row.
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    // `framed` and not `button`: a 28pt slot leaves a dead band
                    // under the last row that the popup's own padding cannot even
                    // out against its ceiling.
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

#[cfg(not(target_os = "android"))]
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
mod tests {
    use super::*;
    use crate::app::{Side, Tab};
    use crate::features::entries::model::{Axis, View};
    use crate::shared::ui::chrome::HEIGHT;
    use crate::shared::ui::shot;

    /// egui lays an `Area` out on a first, invisible sizing pass, so an idle
    /// frame runs in front before the shot.
    fn frame(
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        frame_at(ctx, app, vec2(1000.0, 700.0), events)
    }

    fn frame_at(
        ctx: &egui::Context,
        app: &mut App,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        let mut settle = ctx.run_ui(shot::input(size, Vec::new()), |ui| show(ui, app));
        settle.textures_delta.clear();
        let mut out = ctx.run_ui(shot::input(size, events), |ui| show(ui, app));
        out.textures_delta.clear();
        out.shapes
    }

    fn click(ctx: &egui::Context, app: &mut App, at: egui::Pos2) {
        frame(
            ctx,
            app,
            vec![egui::Event::PointerMoved(at), shot::press(at, true)],
        );
        frame(ctx, app, vec![shot::press(at, false)]);
    }

    /// Every accent rule the chrome paints: the one under the place on show,
    /// and nothing else.
    fn rules(shapes: &[egui::epaint::ClippedShape]) -> Vec<Rect> {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == theme::ACCENT && r.rect.height() == 2.0 => {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect()
    }

    fn rule(shapes: &[egui::epaint::ClippedShape]) -> Rect {
        *rules(shapes)
            .first()
            .expect("the tab on screen must carry an accent rule")
    }

    fn center_of(shapes: &[egui::epaint::ClippedShape], text: &str) -> egui::Pos2 {
        shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(s.shape.visual_bounding_rect().center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text} must be on screen"))
    }

    #[test]
    fn vault_tabs_switch_and_close_by_click() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Personal");
        app.open("Work");

        let shapes = frame(&ctx, &mut app, vec![]);
        let open = rule(&shapes);
        let parked = center_of(&shapes, "Personal");
        drop(shapes);
        assert_eq!(
            app.boxes.active, 1,
            "the vault unlocked last is the one on screen"
        );
        assert_eq!(open.max.y, HEIGHT, "the rule sits on the bar's floor");
        assert!(parked.x < open.min.x, "the parked tab sits to its left");

        click(&ctx, &mut app, parked);
        assert_eq!(
            app.boxes.active, 0,
            "clicking a name brings that vault forward"
        );
        assert_eq!(app.boxes.session.as_ref().unwrap().vault.name, "Personal");

        let shapes = frame(&ctx, &mut app, vec![]);
        let open = rule(&shapes);
        drop(shapes);
        let open = Rect::from_min_max(pos2(open.min.x, open.max.y - HEIGHT), open.max);
        let cross = pos2(open.max.x - PAD - CROSS * 0.5, open.center().y);
        click(&ctx, &mut app, cross);
        assert!(app.boxes.tabs.is_empty(), "the cross closes the tab");
        assert_eq!(app.boxes.session.as_ref().unwrap().vault.name, "Work");
    }

    /// A narrow window has no room for a strip under the bar, so the panes move to
    /// the app's floor: the generator opens it at the far left, the panes sit
    /// centred, Back closes it at the far right, and the rule of the pane on show
    /// sits on the bar's ceiling.
    #[test]
    fn the_narrow_bar_puts_the_panes_on_the_floor() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");

        let size = vec2(420.0, 780.0);
        // Two frames: the floor is placed on the first pass and is only where it
        // ends up on the second, and the read below answers with what the pass
        // before the last one left behind.
        frame_at(&ctx, &mut app, size, vec![]);
        let shapes = frame_at(&ctx, &mut app, size, vec![]);
        assert_eq!(chrome::strip(&ctx), chrome::NAV, "the bar holds a thumb");
        assert_eq!(chrome::floor(&ctx), chrome::NAV);
        let ceiling = size.y - chrome::NAV;

        let rule = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == theme::ACCENT && r.rect.height() == 2.0 => {
                    Some(r.rect)
                }
                _ => None,
            })
            .expect("the pane on show carries its rule");
        assert_eq!(
            rule.min.y, ceiling,
            "the rule belongs on the bar's ceiling, where the body begins"
        );

        let tool = ctx
            .read_response(Id::new("bar-generate"))
            .expect("the generator opens the floor")
            .rect;
        assert!(
            tool.min.x < 40.0,
            "the generator is the leftmost of the floor: {tool:?}"
        );
        assert!(tool.height() >= 44.0, "too small for a thumb: {tool:?}");

        let slot = |label: &str| {
            ctx.read_response(Id::new(("nav", label)))
                .unwrap_or_else(|| panic!("{label} is on the floor"))
                .rect
        };
        let mut left = tool.max.x;
        for label in ["Passwords", "Files", "Codes"] {
            let rect = slot(label);
            assert!(
                rect.min.y >= ceiling && rect.max.y <= size.y,
                "{label} is not on the floor: {rect:?}"
            );
            assert!(
                rect.height() >= 44.0,
                "{label} is too small for a thumb: {rect:?}"
            );
            assert!(
                rect.min.x >= left,
                "{label} follows the generator and the pane before it: {rect:?}"
            );
            left = rect.max.x;
        }

        // Back is the far end of the floor, and the only mark there: its glyph
        // sits inside its own square, so the gap is the bar's inset plus the
        // square's own padding.
        let indoor = (crate::shared::ui::responsive::grow_target(&ctx, BACK.1) - glyph(&ctx)) * 0.5;
        let back = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.brush.is_some() && r.rect.center().y >= ceiling => {
                    Some(r.rect)
                }
                _ => None,
            })
            .map(|r| r.max.x)
            .fold(0.0, f32::max);
        assert!(
            (size.x - back - THUMB - indoor).abs() <= 1.0,
            "Back is not at the floor's own end: {back}"
        );
        drop(shapes);

        // The floor is what switches the box, and the screen it was on.
        let codes = slot("Codes").center();
        app.screen = Screen::Generate;
        for held in [true, false] {
            frame_at(
                &ctx,
                &mut app,
                size,
                vec![egui::Event::PointerMoved(codes), shot::press(codes, held)],
            );
        }
        assert_eq!(
            app.screen,
            Screen::Main,
            "a destination brings the box back on screen"
        );
        assert!(
            app.boxes.session.as_ref().unwrap().tab == Tab::Codes,
            "and opens the pane it names"
        );
    }

    /// The panes are the floor's centre, not what the generator leaves behind:
    /// the three stand on the bar's own middle at any width a thumb meets, keep
    /// a finger's worth of air between them, and still clear the generator on
    /// the left and Back on the right.
    #[test]
    fn the_narrow_floor_centres_the_panes_on_the_bar() {
        for size in [vec2(420.0, 780.0), vec2(380.0, 780.0)] {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut app = App::blank(&ctx);
            app.open("Work");

            // Two frames, as above: the floor is placed on the first pass and
            // the response read below answers with what the pass before left.
            frame_at(&ctx, &mut app, size, vec![]);
            frame_at(&ctx, &mut app, size, vec![]);

            let slot = |label: &str| {
                ctx.read_response(Id::new(("nav", label)))
                    .unwrap_or_else(|| panic!("{label} is on the floor"))
                    .rect
            };
            let first = slot("Passwords");
            let last = slot("Codes");
            let span = Rect::from_min_max(first.min, last.max);
            assert!(
                (span.center().x - size.x * 0.5).abs() <= 1.0,
                "the panes are not centred on the bar: {span:?} vs {size:?}"
            );
            // Targets edge to edge are one wide target: a thumb that misses the
            // one it aimed at lands on its neighbour.
            for pair in [first, slot("Files"), last].windows(2) {
                let (left, right) = (pair[0], pair[1]);
                let air = right.min.x - left.max.x;
                assert!(
                    air >= SPREAD - 1.0,
                    "the pane targets {left:?} and {right:?} leave {air} between them"
                );
            }

            let tool = ctx
                .read_response(Id::new("bar-generate"))
                .expect("the generator opens the floor")
                .rect;
            let back = ctx
                .read_response(Id::new("nav-back"))
                .expect("Back closes the floor")
                .rect;
            assert!(
                span.min.x > tool.max.x && span.max.x < back.min.x,
                "the centred panes do not clear the bar's ends: {span:?} {tool:?} {back:?}"
            );
        }
    }

    /// The tool is not a page of the box, so opening it leaves the pane behind
    /// it unmarked: the floor marks the one place the user is at.
    #[test]
    fn the_tool_leaves_the_pane_behind_it_unmarked() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");

        let size = vec2(420.0, 780.0);
        frame_at(&ctx, &mut app, size, vec![]);
        app.screen = Screen::Generate;
        let shapes = frame_at(&ctx, &mut app, size, vec![]);

        let marks = rules(&shapes);
        assert_eq!(marks.len(), 1, "one place on show, one mark: {marks:?}");
        let tool = ctx
            .read_response(Id::new("bar-generate"))
            .expect("the generator opens the floor")
            .rect;
        assert!(
            tool.contains(marks[0].center()),
            "the mark belongs to the tool, not the pane behind it: {marks:?} vs {tool:?}"
        );
    }

    /// Settings is a page of the box too: once the tool takes the screen, the
    /// gear it left behind loses its mark, and a click still brings the box back
    /// to it.
    #[test]
    fn the_tool_darkens_the_gear_it_left_behind() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");
        app.boxes.pane(Tab::Settings);

        let size = vec2(420.0, 780.0);
        frame_at(&ctx, &mut app, size, vec![]);
        let shapes = frame_at(&ctx, &mut app, size, vec![]);
        let marks = rules(&shapes);
        let gear = ctx
            .read_response(Id::new("bar-settings"))
            .expect("a phone keeps the gear on its bar")
            .rect;
        assert_eq!(marks.len(), 1, "the gear alone marks the box: {marks:?}");
        assert!(
            gear.contains(marks[0].center()),
            "the gear marks the box while it is on show: {marks:?} vs {gear:?}"
        );

        app.screen = Screen::Generate;
        let shapes = frame_at(&ctx, &mut app, size, vec![]);
        let marks = rules(&shapes);
        let gear = ctx
            .read_response(Id::new("bar-settings"))
            .expect("the gear stays in reach on the tool")
            .rect;
        assert!(
            !marks.iter().any(|r| gear.contains(r.center())),
            "the gear is not the place on show: {marks:?} vs {gear:?}"
        );

        // The gear still leads back to the box, from wherever it was reached.
        let at = gear.center();
        for held in [true, false] {
            frame_at(
                &ctx,
                &mut app,
                size,
                vec![egui::Event::PointerMoved(at), shot::press(at, held)],
            );
        }
        assert_eq!(app.screen, Screen::Main, "the gear brings the box back");
        assert!(
            app.boxes.session.as_ref().unwrap().tab == Tab::Settings,
            "and lands on settings"
        );
    }

    /// A narrow desktop window is still a borderless window: its bar keeps the
    /// drag region and the min/max/close buttons even though the panes moved to
    /// the floor.
    #[test]
    fn the_narrow_desktop_bar_keeps_the_window_controls() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");

        let size = egui::vec2(420.0, 780.0);
        let mut o = ctx.run_ui(crate::shared::ui::shot::input(size, Vec::new()), |ui| {
            crate::features::shell::windowbar::show(ui, &mut app)
        });
        o.textures_delta.clear();
        let mut o = ctx.run_ui(crate::shared::ui::shot::input(size, Vec::new()), |ui| {
            crate::features::shell::windowbar::show(ui, &mut app)
        });
        o.textures_delta.clear();

        assert!(
            ctx.read_response(egui::Id::new("pandora-windowbar"))
                .is_some(),
            "the drag region must survive a narrow desktop window"
        );

        // The close button is the rightmost control: hovering its centre must
        // paint the ERROR wash. The hover lands a frame late.
        let at = egui::pos2(420.0 - 16.0 - 19.0, 28.0);
        let mut o = ctx.run_ui(
            crate::shared::ui::shot::input(size, vec![egui::Event::PointerMoved(at)]),
            |ui| crate::features::shell::windowbar::show(ui, &mut app),
        );
        o.textures_delta.clear();
        let mut out = ctx.run_ui(
            crate::shared::ui::shot::input(size, vec![egui::Event::PointerMoved(at)]),
            |ui| crate::features::shell::windowbar::show(ui, &mut app),
        );
        out.textures_delta.clear();
        assert!(
            out.shapes
                .iter()
                .any(|s| matches!(&s.shape, egui::Shape::Rect(r)
                if r.fill == theme::palette().error && r.rect.height() < 40.0)),
            "the close button must still be on the narrow bar"
        );
    }

    /// The "+" lands on the new-box screen, where no vault is the window on
    /// show: every tab drops its accent rule — but the close cross of the box
    /// still open must stay put.
    #[test]
    fn opening_another_box_clears_the_active_tab() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");

        let marked = |shapes: &[egui::epaint::ClippedShape]| {
            shapes.iter().any(|s| {
                matches!(&s.shape, egui::Shape::Rect(r)
                    if r.fill == theme::ACCENT && r.rect.height() == 2.0)
            })
        };
        // The close cross is painted as line segments; the bar's window buttons
        // draw some too, so the count is what matters.
        let strokes = |shapes: &[egui::epaint::ClippedShape]| {
            shapes
                .iter()
                .filter(|s| matches!(&s.shape, egui::Shape::LineSegment { .. }))
                .count()
        };

        let before = frame(&ctx, &mut app, vec![]);
        assert!(marked(&before), "the box on screen carries its tab's rule");
        let cross = strokes(&before);
        drop(before);

        app.add();
        assert_eq!(
            app.screen,
            Screen::Sources,
            "the + opens the new-box screen"
        );
        let after = frame(&ctx, &mut app, vec![]);
        assert!(
            !marked(&after),
            "no tab is the active window while the new-box screen is up"
        );
        assert_eq!(
            strokes(&after),
            cross,
            "the open box's close cross must not disappear with the rule"
        );
    }

    /// A row has to hug its own lines: one that centres its action against a taller
    /// box stretches, and the last row's slack lands under it as a dead strip no
    /// padding can even out. The shapes are what caught it, so the shapes keep it.
    #[test]
    fn the_browser_popup_keeps_its_floor_off_the_last_row() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");
        app.prefs.browsers = vec![
            "chrome-extension://faked/".to_owned(),
            "pandora@pandora.dev".to_owned(),
        ];
        frame(&ctx, &mut app, vec![]);
        let at = ctx
            .read_response(Id::new("bar-browsers"))
            .expect("the browsers chip is on the bar")
            .rect
            .center();
        click(&ctx, &mut app, at);

        let shapes = frame(&ctx, &mut app, vec![]);
        // The panel is painted over its own shadow, which wears the same radius and
        // is the same rect inflated: the last one found is the menu itself.
        let mut panels: Vec<Rect> = Vec::new();
        for shape in &shapes {
            if let egui::Shape::Vec(group) = &shape.shape {
                for inner in group {
                    if let egui::Shape::Rect(r) = inner {
                        if r.corner_radius == theme::radius(8) && r.rect.width() > 200.0 {
                            panels.push(r.rect);
                        }
                    }
                }
            }
        }
        let panel = panels
            .last()
            .copied()
            .expect("the popup paints a panel of its own");
        // The bar's own labels are painted in the same frame, so the popup's lines
        // are the ones that land inside its panel.
        let lines: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.rect.translate(t.pos.to_vec2())),
                _ => None,
            })
            .filter(|r| panel.contains_rect(*r))
            .collect();
        let last = lines
            .iter()
            .max_by(|a, b| a.max.y.total_cmp(&b.max.y))
            .expect("the popup has lines");
        let first = lines
            .iter()
            .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
            .expect("the popup has lines");

        assert!(
            panel.max.y - last.max.y <= 14.0,
            "the popup's floor is {} below its last line: {panel:?} vs {last:?}",
            panel.max.y - last.max.y
        );
        assert!(
            first.min.y - panel.min.y >= 4.0,
            "the header is glued to the ceiling: {panel:?} vs {first:?}"
        );
    }

    /// The extension link shows on the bar only while a browser has been let in:
    /// with none, there is nothing to say.
    #[test]
    fn the_bar_shows_linked_browsers_only_once_one_is_let_in() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");

        // The chip is the only 15pt mark tinted with the accent.
        let chip = |shapes: &[egui::epaint::ClippedShape]| {
            shapes.iter().any(|s| {
                matches!(&s.shape, egui::Shape::Rect(r)
                if r.fill == theme::ACCENT
                    && (r.rect.width() - 15.0).abs() < 0.5
                    && (r.rect.height() - 15.0).abs() < 0.5)
            })
        };

        assert!(!chip(&frame(&ctx, &mut app, vec![])), "no browser, no chip");

        app.prefs.browsers = vec!["chrome-extension://faked/".to_owned()];
        assert!(
            chip(&frame(&ctx, &mut app, vec![])),
            "a linked browser shows the chip"
        );
    }

    #[test]
    fn action_buttons_fit_inside_the_bar() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let icons = theme::Icons::load(&ctx);
        let mut rows: Vec<(Rect, Rect)> = Vec::new();
        let input = shot::input(vec2(1000.0, 700.0), Vec::new());
        let mut output = ctx.run_ui(input, |ui| {
            egui::Panel::top("windowbar-fit")
                .exact_size(HEIGHT)
                .show_separator_line(false)
                .frame(egui::Frame::NONE.fill(theme::palette().panel))
                .show(ui, |ui| {
                    let bar = ui.max_rect();
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.horizontal_centered(|ui| {
                        icons.button(
                            ui,
                            theme::Icon::LockKeyhole,
                            15.0,
                            theme::palette().error,
                            "",
                        );
                        icons.button(ui, theme::Icon::Settings, 15.0, theme::palette().muted, "");
                        icons.button(ui, theme::Icon::Back, 13.0, theme::palette().muted, "Back");
                    });
                    rows.push((bar, ui.min_rect()));
                });
        });
        output.textures_delta.clear();
        for (bar, content) in rows {
            assert!(
                content.max.y <= bar.max.y && content.min.y >= bar.min.y,
                "action buttons overflow the {HEIGHT}pt bar: content {content:?} vs bar {bar:?}"
            );
        }
    }

    #[test]
    fn bar_buttons_are_flat_while_the_rest_keeps_its_chrome() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let icons = theme::Icons::load(&ctx);

        let mut washed: Vec<CornerRadius> = Vec::new();
        let mut filled: Vec<CornerRadius> = Vec::new();
        // Hover lands a frame late, so the second frame is where the wash appears.
        for frame in 0..2 {
            let mut input = shot::input(vec2(1000.0, 700.0), Vec::new());
            if frame == 1 {
                input
                    .events
                    .push(egui::Event::PointerMoved(pos2(986.0, 15.0)));
            }
            let mut out = ctx.run_ui(input, |ui| {
                egui::Panel::top("windowbar-flat")
                    .exact_size(HEIGHT)
                    .show_separator_line(false)
                    .frame(egui::Frame::NONE.fill(theme::palette().panel))
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                theme::flat(ui);
                                icons.button(
                                    ui,
                                    theme::Icon::LockKeyhole,
                                    15.0,
                                    theme::palette().error,
                                    "",
                                );
                            });
                        });
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(theme::palette().panel))
                    .show(ui, |ui| {
                        ui.add(egui::Button::new("rounded"));
                    });
            });
            out.textures_delta.clear();
            for s in &out.shapes {
                if let egui::Shape::Rect(r) = &s.shape {
                    if r.rect.height() < 20.0 {
                        continue;
                    }
                    if r.fill == theme::palette().wash {
                        washed.push(r.corner_radius);
                    } else if r.fill == theme::palette().elev {
                        filled.push(r.corner_radius);
                    }
                }
            }
        }
        assert!(
            washed.contains(&CornerRadius::ZERO),
            "a hovered bar button must be flat: {washed:?}"
        );
        assert!(
            filled.contains(&theme::radius(6)),
            "a button outside the bar keeps its corners: {filled:?}"
        );
    }

    #[test]
    fn the_bar_is_flat_end_to_end() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");

        let shapes = frame(&ctx, &mut app, vec![]);
        let corners: Vec<CornerRadius> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.rect.max.y <= HEIGHT + 1.0 => Some(r.corner_radius),
                _ => None,
            })
            .collect();
        assert!(
            corners.iter().all(|c| *c == CornerRadius::ZERO),
            "a bar chip kept rounded corners: {corners:?}"
        );
    }

    #[test]
    fn the_back_arrow_stays_dimmed_with_nowhere_to_go() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.record_place(); // home: nothing behind it
        assert!(!app.can_back());

        let alpha = ctx.style_of(egui::Theme::Dark).visuals.disabled_alpha();
        let dim = theme::palette().muted.gamma_multiply(alpha);
        // An icon paints as a tinted rect, so the arrow shows its tint as fill.
        let arrow = |shapes: &[egui::epaint::ClippedShape]| {
            shapes.iter().any(|s| match &s.shape {
                egui::Shape::Rect(r) => r.fill == dim && r.rect.width() == 15.0,
                _ => false,
            })
        };

        assert!(
            arrow(&frame(&ctx, &mut app, vec![])),
            "the back arrow must stay on the bar, dimmed"
        );

        // one step to undo arms it
        app.screen = Screen::Generate;
        app.record_place();
        assert!(app.can_back());
        assert!(
            !arrow(&frame(&ctx, &mut app, vec![])),
            "an armed back arrow is not dimmed"
        );
    }

    #[test]
    fn back_closes_the_overlay_before_it_navigates() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");
        app.record_place(); // at the box, nothing behind yet

        let s = app.boxes.session.as_mut().unwrap();
        s.selected = Some("entry".to_owned());
        assert!(app.can_back(), "the open entry is a step");
        app.back();
        assert!(
            app.boxes.session.as_ref().unwrap().selected.is_none(),
            "Back closes the entry, not the box"
        );
        assert_eq!(app.screen, Screen::Main, "and stays on the box");
    }

    #[test]
    fn back_walks_pane_and_folder() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");
        app.record_place(); // Passwords, root
        app.boxes.pane(Tab::Drive);
        app.record_place(); // Files, root
        app.boxes.session.as_mut().unwrap().folder = Some("docs".to_owned());
        app.record_place(); // Files, docs

        app.back(); // -> Files, root
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.folder, None, "Back steps out of the folder");
        assert!(s.tab == Tab::Drive, "still in Files");

        app.back(); // -> Passwords, root
        assert!(app.boxes.session.as_ref().unwrap().tab == Tab::Passwords);
        assert!(!app.can_back(), "the places behind are spent");
    }

    #[test]
    fn back_walks_out_of_a_generated_password_folder() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");
        app.record_place(); // Passwords, root

        {
            let s = app.boxes.session.as_mut().unwrap();
            s.view = View::Folders;
            s.axis = Axis::User;
        }
        app.record_place(); // Folders, root
        app.boxes.session.as_mut().unwrap().at = vec!["octocat".to_owned()];
        app.record_place(); // inside the generated folder

        assert!(app.can_back(), "inside a generated folder is a step");
        app.back();
        let s = app.boxes.session.as_ref().unwrap();
        assert!(s.at.is_empty(), "Back steps out of the folder");
        assert_eq!(s.view, View::Folders, "still in the folders view");

        app.back();
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.view, View::List, "Back leaves the folders view");
        assert!(!app.can_back(), "the places behind are spent");
    }

    #[test]
    fn the_phone_back_gesture_walks_back() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");
        app.record_place(); // Passwords, root
        app.boxes.pane(Tab::Drive);
        app.record_place(); // Files, root

        assert!(app.can_back());
        app.back_or_leave(&ctx);
        assert!(
            app.boxes.session.as_ref().unwrap().tab == Tab::Passwords,
            "BrowserBack steps back like the bar's arrow"
        );

        // nothing left behind: back leaves the app (viewport closes) — no panic
        app.back();
        assert!(!app.can_back());
        app.back_or_leave(&ctx);
    }

    #[test]
    fn back_returns_to_the_previous_box() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Personal");
        app.open("Work");
        app.record_place(); // Work on screen
        app.switch(0); // Personal
        app.record_place();

        app.back();
        assert_eq!(
            app.boxes.session.as_ref().unwrap().vault.name,
            "Work",
            "Back brings the box just left forward"
        );
    }

    #[test]
    fn back_returns_to_the_landing_view_of_sources() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Machine);
        app.record_place();
        app.auth.side = Some(Side::Github);
        app.record_place();

        app.back();
        assert_eq!(
            app.auth.side,
            Some(Side::Machine),
            "Back returns to the view left"
        );
    }

    #[test]
    fn back_closes_a_sources_overlay() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.record_place(); // the picker, nothing behind yet

        app.sync.adding = true; // the repo picker is up
        assert!(app.can_back(), "an open overlay is a step");
        app.back();
        assert!(!app.sync.adding, "Back closes the picker");
        assert_eq!(app.screen, Screen::Sources, "and stays on Sources");
    }

    #[test]
    fn a_bar_action_draws_a_grey_rule_on_hover_and_the_accent_when_pressed() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let icons = theme::Icons::load(&ctx);
        let at = pos2(986.0, 15.0);

        let mut frames: Vec<Option<(Color32, Rect)>> = Vec::new();
        for frame in 0..3 {
            let mut input = shot::input(vec2(1000.0, 700.0), Vec::new());
            if frame >= 1 {
                input.events.push(egui::Event::PointerMoved(at));
            }
            if frame == 2 {
                input.events.push(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                });
            }
            let mut out = ctx.run_ui(input, |ui| {
                egui::Panel::top("windowbar-mark")
                    .exact_size(HEIGHT)
                    .show_separator_line(false)
                    .frame(egui::Frame::NONE.fill(theme::palette().panel))
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let resp = icons.button(
                                    ui,
                                    theme::Icon::LockKeyhole,
                                    15.0,
                                    theme::palette().error,
                                    "",
                                );
                                theme::marked(ui, resp, theme::Edge::Floor);
                            });
                        });
                    });
            });
            out.textures_delta.clear();
            frames.push(out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.rect.height() == 2.0
                        && (r.fill == theme::palette().faint || r.fill == theme::ACCENT) =>
                {
                    Some((r.fill, r.rect))
                }
                _ => None,
            }));
        }

        assert!(frames[0].is_none(), "an idle bar draws no rule");
        let (fill, hover) = frames[1].expect("a hovered action must draw a rule");
        assert_eq!(fill, theme::palette().faint, "hover draws the grey rule");
        let (fill, rule) = frames[2].expect("a pressed action must draw a rule");
        assert_eq!(fill, theme::ACCENT, "pressing keeps the accent rule");

        assert_eq!(rule.max.y, HEIGHT, "the rule sits on the bar's floor");
        assert!(
            (rule.width() - 28.0).abs() <= 1.0,
            "the rule spans the button: {rule:?}"
        );
        assert_eq!(rule.max.x, 1000.0, "the button is last on the right");
        assert_eq!(hover, rule, "the rule keeps its span across states");
    }

    /// The tool belongs to no box: it opens from the bar with nothing unlocked,
    /// and wears the accent rule while it is the screen on show.
    #[test]
    fn the_password_tool_opens_from_the_bar_with_no_box_open() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        assert!(app.boxes.session.is_none());
        app.record_place(); // home

        let at = chip(&ctx, &mut app);
        click(&ctx, &mut app, at);
        assert_eq!(app.screen, Screen::Generate, "the chip opens the tool");

        let shapes = frame(&ctx, &mut app, vec![]);
        assert!(
            shapes
                .iter()
                .any(|s| matches!(&s.shape, egui::Shape::Rect(r)
                if r.fill == theme::ACCENT && r.rect.height() == 2.0 && r.rect.max.y == HEIGHT)),
            "the chip shows the tool is up"
        );

        app.record_place();
        app.back();
        assert_eq!(app.screen, Screen::Sources, "and Back leads home");
    }

    /// The chip carries no text, so it is reached by the id its own slot wears.
    fn chip(ctx: &egui::Context, app: &mut App) -> egui::Pos2 {
        frame(ctx, app, vec![]); // the pass that paints it is the one that registers it
        ctx.read_response(Id::new("bar-generate"))
            .expect("the tool chip is on the bar")
            .rect
            .center()
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn resize_zones_are_positive_and_inside_the_window() {
        let screen = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 700.0));
        for (dir, rect, _) in zones(screen) {
            assert!(
                rect.width() > 0.0 && rect.height() > 0.0,
                "{dir:?} collapsed"
            );
            assert!(screen.contains_rect(rect), "{dir:?} escapes the window");
        }
    }
}
