//! Password generator: a tool page of its own, reachable from the bar's own chip
//! with no box open, and the conditions modal an entry's button opens. Both edit
//! one recipe — `app.generate` — and both forge with `logic::password`.

use crate::app::{App, Field, StatusKind};
use crate::features::auth::ui as auth;
use crate::features::generator::logic::{self as password, Recipe};
use crate::features::generator::model::Form;
use crate::shared::theme;
use crate::vault::model::now_secs;
use egui::{RichText, Ui};
use zeroize::Zeroizing;

pub fn main(ui: &mut Ui, app: &mut App) {
    let mut copy: Option<Zeroizing<String>> = None;
    egui::CentralPanel::default()
        .frame(theme::pane(false))
        .show(ui, |ui| {
            theme::scroll(
                ui,
                egui::ScrollArea::vertical().auto_shrink([false, false]),
                |ui| {
                    ui.add_space(auth::top(ui));
                    ui.vertical_centered(|ui| {
                        ui.set_width(ui.available_width().min(theme::COLUMN));
                        let icons = &app.icons;
                        let form = &mut app.generate;
                        if form.out.is_empty() || form.recipe != form.last {
                            forge(form);
                        }

                        theme::card(ui, "Password", |ui| {
                            ui.set_min_width(ui.available_width());
                            if form.recipe.classes().is_empty() {
                                ui.label(
                                    RichText::new(crate::shared::i18n::t(
                                        "pick at least one character set",
                                    ))
                                    .color(theme::palette().muted),
                                );
                                return;
                            }
                            ui.add(
                                egui::Label::new(
                                    RichText::new(form.out.as_str())
                                        .monospace()
                                        .size(19.0)
                                        .color(theme::palette().text),
                                )
                                .wrap()
                                .selectable(true),
                            );
                            ui.add_space(16.0);
                            if crate::shared::ui::responsive::compact(ui.ctx()) {
                                if theme::secondary_button(
                                    ui,
                                    icons,
                                    Some(theme::Icon::Dices),
                                    "Regenerate",
                                )
                                .clicked()
                                {
                                    forge(form);
                                }
                                ui.add_space(6.0);
                                if theme::primary_button(
                                    ui,
                                    icons,
                                    Some(theme::Icon::Copy),
                                    "Copy",
                                    None,
                                )
                                .clicked()
                                {
                                    copy = Some(form.out.clone());
                                }
                            } else {
                                ui.columns(2, |cols| {
                                    if theme::secondary_button(
                                        &mut cols[0],
                                        icons,
                                        Some(theme::Icon::Dices),
                                        "Regenerate",
                                    )
                                    .clicked()
                                    {
                                        forge(form);
                                    }
                                    if theme::primary_button(
                                        &mut cols[1],
                                        icons,
                                        Some(theme::Icon::Copy),
                                        "Copy",
                                        None,
                                    )
                                    .clicked()
                                    {
                                        copy = Some(form.out.clone());
                                    }
                                });
                            }
                        });

                        ui.add_space(20.0);

                        theme::card(ui, "Recipe", |ui| {
                            ui.set_min_width(ui.available_width());
                            recipe(ui, &mut form.recipe);
                        });
                    });
                },
            );
        });

    if let Some(text) = copy {
        app.shell.copy_text(ui.ctx(), &text);
        app.shell.set_status("password copied", StatusKind::Saved);
    }
}

/// The conditions of generation, on their own so the page and the entry's modal
/// edit the same recipe.
pub fn recipe(ui: &mut Ui, recipe: &mut Recipe) {
    // The caller's column may be centred; the controls are a column of their own,
    // so they read from the left.
    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
        theme::slider(
            ui,
            &format!("{} {}", password::MAX_LEN, crate::shared::i18n::t("chars")),
            egui::Slider::new(&mut recipe.len, password::MIN_LEN..=password::MAX_LEN)
                .suffix(format!(" {}", crate::shared::i18n::t("chars"))),
        );
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            theme::check(ui, &mut recipe.upper, "A-Z");
            theme::check(ui, &mut recipe.lower, "a-z");
            theme::check(ui, &mut recipe.digits, "0-9");
            theme::check(ui, &mut recipe.symbols, "!@#");
        });
        ui.add_space(6.0);
        theme::hint(ui, "no look-alikes or shell escapes");
    });
}

/// The conditions over the entry whose button was pressed: the field is only
/// written on the modal's own Generate, so nothing changes under the click.
pub fn modal(ctx: &egui::Context, app: &mut App) {
    let Some(entry) = app.generate.entry.clone() else {
        return;
    };
    let empty = app.generate.recipe.classes().is_empty();
    let mut accept = false;
    let open = theme::dialog(ctx, egui::Id::new("entry-generate"), |ui| {
        theme::width(ui, 300.0);
        let closed = theme::head(ui, &app.icons, "Generate");
        recipe(ui, &mut app.generate.recipe);
        ui.add_space(16.0);
        if empty {
            ui.label(
                RichText::new(crate::shared::i18n::t("pick at least one character set"))
                    .color(theme::palette().muted),
            );
        } else if theme::primary_button(ui, &app.icons, Some(theme::Icon::Dices), "Generate", None)
            .clicked()
        {
            accept = true;
        }
        // Enter generates, when nothing else holds the keyboard: it is the modal's
        // OK, and eaten here so `theme::dialog` does not read it as just a close.
        if !empty
            && !accept
            && ctx.memory(|m| m.focused().is_none())
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
        {
            accept = true;
        }
        closed
    });

    if accept {
        app.generate.entry = None;
        apply(app, &entry);
    } else if !open {
        app.generate.entry = None;
    }
}

/// Write a fresh password into the entry the modal was opened for, and reveal it.
fn apply(app: &mut App, entry: &str) {
    let recipe = app.generate.recipe;
    if let Some(s) = app.boxes.session.as_mut() {
        if let Some(it) = s.vault.logins.iter_mut().find(|i| i.id == entry) {
            let fresh = password::generate(&recipe);
            it.set_password(fresh.as_str());
            it.touched(now_secs());
            s.reveal.insert((entry.to_owned(), Field::Password));
            s.mark_now();
        }
    }
}

/// Forge a password from the recipe and remember the recipe it came from, so the
/// same one is never forged twice.
fn forge(form: &mut Form) {
    form.out = Zeroizing::new(password::generate(&form.recipe));
    form.last = form.recipe;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Screen;
    use crate::shared::ui::shot::{self, words};
    use egui::{Rect, Shape};

    fn app(ctx: &egui::Context) -> App {
        let mut app = App::blank(ctx);
        app.screen = Screen::Generate;
        app
    }

    fn frame(
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        shot::frame(ctx, egui::vec2(1000.0, 700.0), events, |ui| main(ui, app))
    }

    fn label(shapes: &[egui::epaint::ClippedShape], text: &str) -> Rect {
        shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text() == text => Some(s.shape.visual_bounding_rect()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text} must be on screen"))
    }

    /// A box with one password entry, open in the Passwords pane.
    fn open(ctx: &egui::Context) -> App {
        let mut app = App::blank(ctx);
        app.screen = Screen::Main;
        app.open("Test");
        app.boxes.add_login();
        app
    }

    fn selected(app: &App) -> String {
        app.boxes.selected().expect("a selected entry")
    }

    fn password_of(app: &App, id: &str) -> String {
        app.boxes
            .session
            .as_ref()
            .unwrap()
            .vault
            .logins
            .iter()
            .find(|i| i.id == id)
            .unwrap()
            .password
            .to_string()
    }

    fn modal_shapes(
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        shot::frame(ctx, egui::vec2(1000.0, 700.0), events, |ui| {
            modal(ui.ctx(), app)
        })
    }

    fn click(at: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![egui::Event::PointerMoved(at), shot::press(at, pressed)]
    }

    fn escape() -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]
    }

    /// The entry's button asks first: the conditions over the entry, and the
    /// field waits for the modal's own Generate.
    #[test]
    fn the_conditions_come_up_and_only_generate_writes() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = open(&ctx);
        let id = selected(&app);
        app.generate.recipe.len = 32;
        app.generate.entry = Some(id.clone());

        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = modal_shapes(&ctx, &mut app, Vec::new());
        }
        let drawn = words(&shapes);
        for want in ["GENERATE", "A-Z", "a-z", "0-9", "!@#"] {
            assert!(drawn.iter().any(|w| w == want), "missing {want}: {drawn:?}");
        }
        assert!(
            password_of(&app, &id).is_empty(),
            "opening the conditions must not touch the field"
        );

        let target = label(&shapes, "Generate").center();
        modal_shapes(&ctx, &mut app, click(target, true));
        modal_shapes(&ctx, &mut app, click(target, false));

        assert!(app.generate.entry.is_none(), "Generate closes the modal");
        let pw = password_of(&app, &id);
        assert_eq!(pw.chars().count(), 32, "{pw}");
        assert!(
            app.boxes
                .session
                .as_ref()
                .unwrap()
                .reveal
                .contains(&(id, Field::Password)),
            "the new password is shown, not left masked"
        );
    }

    /// Esc and the backdrop close it; nothing is written without Generate.
    #[test]
    fn escape_closes_the_conditions_without_writing() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = open(&ctx);
        let id = selected(&app);
        app.generate.entry = Some(id.clone());
        for _ in 0..3 {
            modal_shapes(&ctx, &mut app, Vec::new());
        }

        modal_shapes(&ctx, &mut app, escape());

        assert!(app.generate.entry.is_none(), "Esc closes the conditions");
        assert!(password_of(&app, &id).is_empty(), "and writes nothing");
    }

    /// The tool reads nothing from a box, so it must paint with none open.
    #[test]
    fn the_tool_forges_a_password_with_no_box_open() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = app(&ctx);
        assert!(app.boxes.session.is_none());

        let mut drawn = Vec::new();
        for _ in 0..3 {
            drawn = words(&frame(&ctx, &mut app, Vec::new()));
        }
        for want in ["PASSWORD", "RECIPE", "A-Z"] {
            assert!(drawn.iter().any(|w| w == want), "missing {want}: {drawn:?}");
        }
        let pw = app.generate.out.as_str().to_owned();
        assert_eq!(pw.chars().count(), 20, "{pw}");
        assert!(pw.chars().any(|c| c.is_ascii_uppercase()), "{pw}");
        assert!(pw.chars().any(|c| c.is_ascii_lowercase()), "{pw}");
        assert!(pw.chars().any(|c| c.is_ascii_digit()), "{pw}");
        assert!(pw.chars().any(|c| !c.is_ascii_alphanumeric()), "{pw}");
    }

    /// The page is a column on the middle, not a left-pinned one: it used to hug
    /// the pane's left edge with the whole right half of the window empty.
    #[test]
    fn the_page_is_a_centred_column() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = app(&ctx);
        frame(&ctx, &mut app, Vec::new());
        let shapes = frame(&ctx, &mut app, Vec::new());

        let cards: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == theme::palette().panel && r.rect.height() > 60.0 => {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(cards.len(), 2, "the tool is two cards: {cards:?}");
        for card in &cards {
            assert!(
                (card.center().x - 490.0).abs() <= 1.0,
                "a card sits off the middle: {card:?}"
            );
        }

        assert!(
            label(&shapes, "PASSWORD").min.y < label(&shapes, "RECIPE").min.y,
            "the result is the page's head, the recipe follows it"
        );
    }

    /// A change has to show: the output follows the recipe the controls left
    /// behind, wherever the cards sit.
    #[test]
    fn a_recipe_change_forges_another_password() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = app(&ctx);
        frame(&ctx, &mut app, Vec::new());
        let first = app.generate.out.to_string();

        app.generate.recipe.len = 40;
        frame(&ctx, &mut app, Vec::new());
        let second = app.generate.out.as_str().to_owned();
        assert_eq!(second.chars().count(), 40, "{second}");
        assert_ne!(first, second);

        frame(&ctx, &mut app, Vec::new());
        assert_eq!(
            app.generate.out.as_str(),
            second,
            "an untouched recipe keeps its password"
        );
    }

    /// The two actions are one row of equal halves: regenerate on the left, copy
    /// on the right.
    #[test]
    fn the_actions_split_the_row_in_two() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = app(&ctx);
        frame(&ctx, &mut app, Vec::new());
        let shapes = frame(&ctx, &mut app, Vec::new());

        let copy = shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Rect(r)
                    if r.fill == theme::ACCENT && (r.rect.height() - 40.0).abs() < 0.5 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .expect("the copy button");
        let regen = shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Rect(r)
                    if r.fill == theme::palette().elev
                        && (r.rect.height() - 40.0).abs() < 0.5
                        && (r.rect.max.y - copy.max.y).abs() < 0.5
                        && r.rect.max.x <= copy.min.x =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .expect("the regenerate button beside it");

        assert_eq!(regen.min.y, copy.min.y, "the pair shares a row");
        assert!(
            (regen.width() - copy.width()).abs() <= 1.0,
            "equal halves: {regen:?} vs {copy:?}"
        );
        assert!(
            label(&shapes, "Regenerate").center().x < label(&shapes, "Copy").center().x,
            "regenerate leads, copy follows"
        );
    }

    /// The page keeps a shot to eyeball like every other; it is no pane of a box,
    /// so it renders on its own.
    #[test]
    fn shots() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = app(&ctx);
        shot::png(&ctx, "generate", 1000, 700, None, |ui| main(ui, &mut app));
        shot::png(&ctx, "generate-compact", 420, 780, None, |ui| {
            main(ui, &mut app)
        });
    }
}
