use super::*;
use crate::app::{Side, Tab};
use crate::features::entries::model::{Axis, View};
use crate::shared::ui::chrome::HEIGHT;
use crate::shared::ui::shot;

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

#[test]
fn the_narrow_bar_puts_the_panes_on_the_floor() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.open("Work");

    let size = vec2(420.0, 780.0);
    // Two frames: the floor is placed on the first pass and read on the second.
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
        .expect("the generator is still in reach")
        .rect;
    assert!(
        tool.max.y <= ceiling,
        "the tool moved up with the app's own settings, off the floor: {tool:?}"
    );
    assert!(tool.height() >= 44.0, "too small for a thumb: {tool:?}");

    let slot = |label: &str| {
        ctx.read_response(Id::new(("nav", label)))
            .unwrap_or_else(|| panic!("{label} is on the floor"))
            .rect
    };
    let mut left = 0.0;
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
            "{label} follows the pane before it: {rect:?}"
        );
        left = rect.max.x;
    }

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

    // The floor switches the box, and the screen it was on.
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

/// The floor is the box's: its gear on one end, its lock and Back on the other, and
/// the three panes centred in the room they leave. The narrowest window the app allows
/// is the one that decides whether all of it fits.
#[test]
fn the_narrow_floor_holds_the_box_and_centres_its_panes() {
    for size in [vec2(420.0, 780.0), vec2(380.0, 780.0)] {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");

        // Two frames, as above: the floor is placed on the first pass.
        frame_at(&ctx, &mut app, size, vec![]);
        frame_at(&ctx, &mut app, size, vec![]);
        let ceiling = size.y - chrome::NAV;

        let slot = |label: &str| {
            ctx.read_response(Id::new(("nav", label)))
                .unwrap_or_else(|| panic!("{label} is on the floor"))
                .rect
        };
        let first = slot("Passwords");
        let last = slot("Codes");
        let span = Rect::from_min_max(first.min, last.max);
        // Targets edge to edge are one wide target: a miss lands on its neighbour.
        for pair in [first, slot("Files"), last].windows(2) {
            let (left, right) = (pair[0], pair[1]);
            let air = right.min.x - left.max.x;
            assert!(
                air >= SPREAD - 1.0,
                "the pane targets {left:?} and {right:?} leave {air} between them"
            );
        }

        let back = ctx
            .read_response(Id::new("nav-back"))
            .expect("Back closes the floor")
            .rect;
        let gear = ctx
            .read_response(Id::new("bar-settings"))
            .expect("the box's gear is on the floor")
            .rect;
        let lock = ctx
            .read_response(Id::new("bar-lock"))
            .expect("the box's lock is on the floor")
            .rect;
        for (name, rect) in [("gear", gear), ("lock", lock)] {
            assert!(
                rect.min.y >= ceiling && rect.max.y <= size.y,
                "the {name} is not on the floor: {rect:?}"
            );
            assert!(rect.height() >= 44.0, "the {name} is too small: {rect:?}");
        }
        assert!(
            (gear.min.x - lead(&ctx)).abs() <= 1.0,
            "the gear does not open the floor: {gear:?}"
        );
        assert!(
            span.min.x > gear.max.x && span.max.x < lock.min.x && lock.max.x <= back.min.x,
            "the floor reads gear, panes, lock, Back: {gear:?} {span:?} {lock:?} {back:?}"
        );
        let air = (span.min.x - gear.max.x - SPREAD) - (lock.min.x - span.max.x);
        assert!(
            air.abs() <= 1.0,
            "the panes are off the middle of the room the ends leave: {air}"
        );
    }
}

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

    // The gear still leads back to the box, wherever it was reached.
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

    // The close button is the rightmost control; the hover lands a frame late.
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

/// The app's own settings are not a page of a box: their way in is on the
/// bar, and it works with nothing unlocked. The gear beside it is the open
/// box's page, so the two are told apart by the glyph, not by the id alone.
#[test]
fn the_apps_own_settings_open_from_the_bar_with_no_box_open() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    assert!(app.boxes.session.is_none());

    frame(&ctx, &mut app, vec![]);
    let at = ctx
        .read_response(Id::new("bar-prefs"))
        .expect("the bar carries the app's own settings")
        .rect
        .center();
    assert!(
        ctx.read_response(Id::new("bar-settings")).is_none(),
        "no box, so no box settings button"
    );

    click(&ctx, &mut app, at);
    assert!(app.prefs_open, "the sliders open the app's settings");
    assert_eq!(
        app.screen,
        Screen::Sources,
        "without opening a box or leaving the screen"
    );
}

/// A phone's bar carries both, side by side and apart: the box's gear, and
/// the app's sliders.
#[test]
fn a_phone_bar_tells_the_boxs_settings_from_the_apps() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    app.open("Work");
    app.boxes.pane(Tab::Passwords);
    app.screen = Screen::Main;

    let size = vec2(420.0, 780.0);
    frame_at(&ctx, &mut app, size, vec![]);
    let gear = ctx
        .read_response(Id::new("bar-settings"))
        .expect("the box's gear is on a phone's bar")
        .rect;
    let sliders = ctx
        .read_response(Id::new("bar-prefs"))
        .expect("so are the app's sliders")
        .rect;
    assert!(
        !gear.intersects(sliders),
        "the two settings are not one button: {gear:?} and {sliders:?}"
    );

    let at = sliders.center();
    for held in [true, false] {
        frame_at(
            &ctx,
            &mut app,
            size,
            vec![egui::Event::PointerMoved(at), shot::press(at, held)],
        );
    }
    assert!(app.prefs_open, "the sliders open the app's settings");
    assert!(
        app.boxes.session.as_ref().unwrap().tab == Tab::Passwords,
        "and leave the box on the page it was on"
    );
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

/// The session's display is the environment's, and every test here shares one process:
/// these two take turns and hand the variable back the way they found it.
struct Session {
    _turn: std::sync::MutexGuard<'static, ()>,
    saved: Option<std::ffi::OsString>,
}

impl Drop for Session {
    fn drop(&mut self) {
        match &self.saved {
            Some(name) => std::env::set_var("WAYLAND_DISPLAY", name),
            None => std::env::remove_var("WAYLAND_DISPLAY"),
        }
    }
}

fn session(wayland: Option<&str>) -> Session {
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let saved = std::env::var_os("WAYLAND_DISPLAY");
    match wayland {
        Some(name) => std::env::set_var("WAYLAND_DISPLAY", name),
        None => std::env::remove_var("WAYLAND_DISPLAY"),
    }
    Session { _turn: turn, saved }
}

/// The frame its events landed in, so a test can read what the bar asked the window for.
fn event_frame(
    ctx: &egui::Context,
    app: &mut App,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut settle = ctx.run_ui(shot::input(size, Vec::new()), |ui| show(ui, app));
    settle.textures_delta.clear();
    let mut out = ctx.run_ui(shot::input(size, events), |ui| show(ui, app));
    out.textures_delta.clear();
    out
}

/// Every level the bar asked the window for in that frame.
fn levels(out: &egui::FullOutput) -> Vec<egui::WindowLevel> {
    out.viewport_output
        .values()
        .flat_map(|v| v.commands.iter())
        .filter_map(|c| match c {
            egui::ViewportCommand::WindowLevel(level) => Some(*level),
            _ => None,
        })
        .collect()
}

/// The pin carries no text, so it is reached by the id its own slot wears.
fn pin_at(ctx: &egui::Context, app: &mut App) -> egui::Pos2 {
    frame(ctx, app, vec![]); // the pass that paints it is the one that registers it
    ctx.read_response(Id::new("pandora-pin"))
        .expect("the pin is on the bar")
        .rect
        .center()
}

#[test]
fn the_pin_puts_the_window_on_top() {
    let _session = session(None);
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    let size = vec2(1000.0, 700.0);
    let at = pin_at(&ctx, &mut app);

    event_frame(
        &ctx,
        &mut app,
        size,
        vec![egui::Event::PointerMoved(at), shot::press(at, true)],
    );
    let out = event_frame(&ctx, &mut app, size, vec![shot::press(at, false)]);

    assert!(app.prefs.on_top, "a click must pin the window");
    assert_eq!(
        levels(&out),
        vec![egui::WindowLevel::AlwaysOnTop],
        "the bar must ask the window for the top level"
    );
}

#[test]
fn the_pin_says_why_a_wayland_session_cannot_pin() {
    // Wayland gives a client no way to sit above the others, so the click changes
    // nothing and answers instead of leaving the mark a button that does nothing.
    let _session = session(Some("wayland-0"));
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut app = App::blank(&ctx);
    let size = vec2(1000.0, 700.0);
    let at = pin_at(&ctx, &mut app);

    event_frame(
        &ctx,
        &mut app,
        size,
        vec![egui::Event::PointerMoved(at), shot::press(at, true)],
    );
    let out = event_frame(&ctx, &mut app, size, vec![shot::press(at, false)]);

    assert!(!app.prefs.on_top, "nothing is pinned where it cannot be");
    assert!(
        levels(&out).is_empty(),
        "no level is asked of a compositor that would drop it"
    );
    let status = app.shell.status.as_ref().expect("the click must answer");
    assert!(
        status.kind == StatusKind::Info,
        "a limitation is not a failure"
    );
    assert_eq!(
        status.text,
        "this session does not let an app stay on top: start it with PANDORA_X11=1"
    );
}

/// A debug probe, not a test: it draws the chrome at a phone's size and, when
/// `PANDORA_SHOTS` names a folder, writes the pixels there. Run it with
/// `PANDORA_SHOTS=/tmp/shots cargo test -- --ignored shoot_chrome`.
#[test]
#[ignore = "debug probe: writes a shot, asserts nothing"]
fn shoot_chrome() {
    for (name, size) in [
        ("chrome-phone", vec2(420.0, 780.0)),
        ("chrome-floor", vec2(380.0, 780.0)),
    ] {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.open("Work");
        shot::png(&ctx, name, size.x as u32, size.y as u32, None, |ui| {
            show(ui, &mut app)
        });
    }
}
