//! Welcome (create vault) and Unlock screens.

use crate::app::{App, OpKind, Screen, Side};
use crate::features::settings::model as prefs;
use crate::shared::i18n::{t, tf};
use crate::shared::theme;
use crate::shared::ui::balance;
use crate::shared::ui::responsive;
#[cfg(not(target_os = "android"))]
use crate::vault::store;
use egui::{Context, Key, RichText, TextEdit, Ui};
use std::path::Path;

/// The room a field keeps over and under its own text: a phone's line has to
/// stand as tall as the button it shares the row with.
fn room(ui: &Ui) -> i8 {
    if responsive::compact(ui.ctx()) {
        12
    } else {
        9
    }
}

pub(crate) fn wordmark(ui: &mut egui::Ui, icons: &theme::Icons) {
    ui.add(icons.logo(96.0, theme::ACCENT));
    ui.add_space(4.0);
    balance::label(
        ui,
        RichText::new(t("Save your secrets inside Pandora’s box"))
            .small()
            .color(theme::palette().muted),
    );
    ui.add_space(6.0);
}

const TOP: f32 = 32.0;

pub(crate) fn top(ui: &egui::Ui) -> f32 {
    (ui.available_height() * 0.10).clamp(16.0, TOP)
}

pub(crate) fn column(ui: &mut egui::Ui) {
    const GUTTER: f32 = 12.0;
    ui.set_max_width((ui.available_width() - GUTTER * 2.0).min(440.0));
}

/// Path field with a file-picker button; `save_as` writes a new name instead of reading one.
pub fn file_field(
    ui: &mut egui::Ui,
    icons: &theme::Icons,
    path: &mut String,
    hint: &str,
    save_as: bool,
) {
    let margin = egui::Margin::symmetric(12, room(ui));
    // Android has no dialog behind this field: a box file arrives through the
    // activity's own picker (see `auth::pick`), and a keyfile is typed. The path
    // is the app's own, so there is nothing to browse.
    #[cfg(target_os = "android")]
    {
        let _ = (icons, save_as);
        ui.add(
            TextEdit::singleline(path)
                .hint_text(t(hint))
                .desired_width(f32::INFINITY)
                .margin(margin),
        );
    }
    #[cfg(not(target_os = "android"))]
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let side = (ui.text_style_height(&egui::TextStyle::Body) + margin.sum().y)
            .max(responsive::target(ui.ctx()));
        ui.add(
            TextEdit::singleline(path)
                .hint_text(t(hint))
                .desired_width((ui.available_width() - side - gap).max(120.0))
                .margin(margin),
        );
        if icons
            .framed(
                ui,
                theme::Icon::Folder,
                responsive::grow(ui.ctx(), 16.0),
                theme::palette().muted,
                "Choose a file…",
                side,
            )
            .clicked()
        {
            let picked = if save_as {
                let current = Path::new(path);
                let mut dialog = rfd::FileDialog::new();
                if let Some(dir) = current.parent().filter(|d| d.is_dir()) {
                    dialog = dialog.set_directory(dir);
                }
                dialog.set_file_name(default_name(current)).save_file()
            } else {
                rfd::FileDialog::new().pick_file()
            };
            if let Some(p) = picked {
                *path = p.display().to_string();
            }
        }
    });
}

#[cfg(not(target_os = "android"))]
fn default_name(path: &Path) -> String {
    match path.file_name().and_then(|s| s.to_str()) {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => format!("vault.{}", store::EXT),
    }
}

/// A password field with its reveal toggle: the eye keeps the square the file
/// picker wears, so the row ends on the card's edge either way. The reveal lives
/// in the context's temp data under the field's own id, so it survives the frame
/// and forgets itself whenever the app clears its data.
pub fn secret_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    icons: &theme::Icons,
    text: &mut String,
    hint: &str,
) -> egui::Response {
    let margin = egui::Margin::symmetric(12, room(ui));
    let mut show = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(id))
        .unwrap_or_default();
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        // egui rounds a button's padding to whole pixels, so the eye's square is
        // only its own side once that rounding is applied. The field gives up
        // exactly that much: a hair less and the row runs past its column.
        let eye = responsive::grow(ui.ctx(), 16.0);
        let floor = ui.text_style_height(&egui::TextStyle::Body) + margin.sum().y;
        let side = (eye + 2.0 * ((floor - eye) / 2.0).round()).max(responsive::target(ui.ctx()));
        let resp = ui.add(
            TextEdit::singleline(text)
                .id(id)
                .password(!show)
                .hint_text(t(hint))
                .desired_width((ui.available_width() - side - gap).max(0.0))
                .margin(margin),
        );
        if icons
            .framed(
                ui,
                if show {
                    theme::Icon::EyeOff
                } else {
                    theme::Icon::Eye
                },
                eye,
                theme::palette().muted,
                if show { "hide" } else { "show" },
                side,
            )
            .clicked()
        {
            show = !show;
            ui.ctx().data_mut(|d| d.insert_temp(id, show));
        }
        resp
    })
    .inner
}

/// A secret field and its confirmation, one above the other. They never share a
/// row, however wide the form is: the hint ("master password (min 12)") and the
/// reveal eye need the field's whole width to stay readable, and a confirmation
/// drawn beside its password is a row the eye skips. The response is the
/// confirmation's, so the form still submits on Enter from the second field.
pub fn secret_pair(
    ui: &mut egui::Ui,
    icons: &theme::Icons,
    first: (egui::Id, &mut String, &str),
    second: (egui::Id, &mut String, &str),
) -> egui::Response {
    let (first_id, first_text, first_hint) = first;
    let (second_id, second_text, second_hint) = second;
    secret_field(ui, first_id, icons, first_text, first_hint);
    secret_field(ui, second_id, icons, second_text, second_hint)
}

pub fn welcome(ui: &mut egui::Ui, app: &mut App) {
    let busy = app
        .auth
        .op
        .as_ref()
        .is_some_and(|o| o.kind == OpKind::Create);
    let m = egui::Margin::symmetric(12, 9);
    // A form page scrolls like settings and the generator, but with the bar only
    // when it is needed: a card that fits does not wear one. The pane is what
    // gives the page's bar a gutter to sit in when it does come up.
    let compact = responsive::compact(ui.ctx());
    egui::CentralPanel::default()
        .frame(theme::pane(compact))
        .show(ui, |ui| {
            theme::scroll_when_needed(
                ui,
                egui::ScrollArea::vertical()
                    // Each page keeps its own place: the two share the panel, so a
                    // page scrolled to its button must not open the other one there.
                    .id_salt("welcome")
                    .auto_shrink([false, false]),
                |ui| {
                    ui.add_space(top(ui));
                    ui.vertical_centered(|ui| {
                        // The pane already holds the page off the window's edge, so the
                        // form does not ask `column` for a second gutter: 440 is the
                        // width this card has always had.
                        ui.set_max_width(ui.available_width().min(440.0));
                        wordmark(ui, &app.icons);
                        theme::card(ui, "", |ui| {
                            let mut submit = false;
                            ui.add_enabled_ui(!busy, |ui| {
                                ui.add(
                                    TextEdit::singleline(&mut app.auth.welcome.name)
                                        .hint_text(t("box name"))
                                        .desired_width(f32::INFINITY)
                                        .margin(m),
                                );
                                ui.add_space(12.0);
                                file_field(
                                    ui,
                                    &app.icons,
                                    &mut app.auth.welcome.path,
                                    "box file",
                                    true,
                                );
                                ui.add_space(12.0);
                                let confirm = secret_pair(
                                    ui,
                                    &app.icons,
                                    (
                                        egui::Id::new("welcome-password"),
                                        &mut app.auth.welcome.password,
                                        "master password (min 12)",
                                    ),
                                    (
                                        egui::Id::new("welcome-confirm"),
                                        &mut app.auth.welcome.confirm,
                                        "confirm",
                                    ),
                                );
                                submit = ui.ctx().input(|i| i.key_pressed(Key::Enter))
                                    && confirm.lost_focus();
                                ui.add_space(8.0);
                                theme::strength_meter(ui, &app.auth.welcome.password);
                                ui.add_space(12.0);
                                file_field(
                                    ui,
                                    &app.icons,
                                    &mut app.auth.welcome.keyfile,
                                    "keyfile (optional)",
                                    false,
                                );
                                ui.add_space(16.0);
                            });
                            if theme::primary_button(
                                ui,
                                &app.icons,
                                None,
                                "Create box",
                                busy.then_some("Creating box…"),
                            )
                            .clicked()
                                || submit
                            {
                                app.auth.create_vault(&mut app.shell);
                            }
                        });
                        ui.add_space(18.0);
                        if ui.link(t("I have a box file — open it")).clicked() {
                            app.screen = Screen::Unlock;
                        }
                        ui.add_space(6.0);
                        if ui.link(t("Keep the box in a GitHub repo")).clicked() {
                            app.auth.side = Some(Side::Github);
                            app.screen = Screen::Sources;
                            if !app.sync.signed() {
                                app.github_login();
                            }
                        }
                    });
                },
            );
        });
}

pub fn unlock(ui: &mut egui::Ui, app: &mut App) {
    let ctx: Context = ui.ctx().clone();
    // Any of the three — the derivation, the reader's prompt, the picker — owns
    // the form while it runs, so none of them can start on top of another.
    let busy = app.auth.op.is_some();
    let wait = app
        .prefs
        .wait(&prefs::box_key(app.auth.unlock_path.trim()), prefs::now());
    let pw_id = password_id();
    // Without a repaint request the countdown sits stale until the next mouse move.
    if wait > 0 {
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
    egui::CentralPanel::default()
        .frame(theme::pane(responsive::compact(ui.ctx())))
        .show(ui, |ui| {
            theme::scroll_when_needed(
                ui,
                egui::ScrollArea::vertical()
                    // Its own place, like the create page's: see `welcome`.
                    .id_salt("unlock")
                    .auto_shrink([false, false]),
                |ui| {
                    ui.add_space(top(ui));
                    ui.vertical_centered(|ui| {
                        // The pane already holds the page off the window's edge, so the
                        // form does not ask `column` for a second gutter: 440 is the
                        // width this card has always had.
                        ui.set_max_width(ui.available_width().min(440.0));
                        wordmark(ui, &app.icons);
                        theme::card(ui, "", |ui| {
                            let mut submit = false;
                            ui.add_enabled_ui(!busy, |ui| {
                                file_field(
                                    ui,
                                    &app.icons,
                                    &mut app.auth.unlock_path,
                                    "box file",
                                    false,
                                );
                                ui.add_space(12.0);
                                let resp = secret_field(
                                    ui,
                                    pw_id,
                                    &app.icons,
                                    &mut app.auth.unlock_password,
                                    "master password",
                                );
                                ui.add_space(12.0);
                                file_field(
                                    ui,
                                    &app.icons,
                                    &mut app.auth.unlock_keyfile,
                                    "keyfile (if this box uses one)",
                                    false,
                                );
                                ui.add_space(18.0);
                                submit =
                                    ctx.input(|i| i.key_pressed(Key::Enter)) && resp.lost_focus();
                            });

                            let waiting =
                                (wait > 0).then(|| tf("wait {wait} s", &[("wait", &wait)]));
                            let go = theme::primary_button(
                                ui,
                                &app.icons,
                                Some(theme::Icon::LockKeyholeOpen),
                                "Unlock",
                                busy.then_some("Unlocking…").or(waiting.as_deref()),
                            );
                            if (go.clicked() || submit) && wait == 0 {
                                app.unlock();
                            }
                            // The reader, when this box was set up for it: one touch, and the
                            // keystore hands back the key the password would have derived.
                            if sealed(app) {
                                ui.add_space(8.0);
                                if theme::secondary_button(
                                    ui,
                                    &app.icons,
                                    None,
                                    "Unlock with fingerprint",
                                )
                                .clicked()
                                {
                                    app.unlock_biometric();
                                }
                            }
                        });
                        ui.add_space(18.0);
                        #[cfg(target_os = "android")]
                        {
                            if ui.link(t("Import a box file from this phone")).clicked() {
                                app.import_box();
                            }
                            ui.add_space(6.0);
                        }
                        if ui.link(t("Create a new box")).clicked() {
                            app.screen = Screen::Welcome;
                        }
                    });
                },
            );
        });
}

/// Is this box set up to open with the device's reader?
fn sealed(app: &App) -> bool {
    let path = app.auth.unlock_path.trim();
    !path.is_empty()
        && crate::features::auth::biometric::available()
        && crate::features::auth::biometric::sealed(Path::new(path))
}

/// The password field's id, so the sources screen can hand focus to it.
pub(crate) fn password_id() -> egui::Id {
    egui::Id::new("unlock-password")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Login;
    use crate::features::auth::model::Recent;
    use crate::features::settings::model::{Source, GITHUB};
    use crate::shared::ui::shot;
    use egui::Shape;
    use std::path::PathBuf;
    use zeroize::Zeroizing;

    fn unlock_app(ctx: &egui::Context) -> App {
        let mut app = App::blank(ctx);
        app.screen = Screen::Unlock;
        app
    }

    fn draw(ui: &mut egui::Ui, app: &mut App) {
        match app.screen {
            Screen::Welcome => welcome(ui, app),
            Screen::Sources => crate::features::sync::ui::run(ui, app),
            _ => unlock(ui, app),
        }
    }

    fn shapes(app: &mut App, ctx: &egui::Context) -> Vec<egui::epaint::ClippedShape> {
        shapes_at(app, ctx, egui::vec2(1000.0, 700.0))
    }

    fn shapes_at(
        app: &mut App,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> Vec<egui::epaint::ClippedShape> {
        frame(app, ctx, size, Vec::new())
    }

    fn frame(
        app: &mut App,
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        shot::frame(ctx, size, events, |ui| draw(ui, app))
    }

    /// Press an icon of the sources picker and release a frame later: the picker
    /// carries no text, so it is reached by the id its own slot wears.
    /// Where the picker's `i`-th button paints, left to right: recent, this
    /// machine, GitHub. They carry no text to look for.
    fn pick_button(app: &mut App, ctx: &egui::Context, i: usize) -> egui::Pos2 {
        let shapes = shapes_at(app, ctx, egui::vec2(1000.0, 700.0));
        let mut rects: Vec<egui::Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r)
                    if (r.rect.height() - theme::ICON).abs() < 2.0
                        && (r.rect.width() - theme::ICON).abs() < 2.0
                        && (r.fill == theme::palette().elev
                            || r.fill == theme::palette().accent_dim
                            || r.fill == theme::palette().accent_deep) =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        rects.sort_by(|a, b| a.center().x.partial_cmp(&b.center().x).unwrap());
        rects
            .get(i)
            .unwrap_or_else(|| panic!("the picker paints {} buttons", rects.len()))
            .center()
    }

    /// Press the `i`-th view button and release a frame later: egui resolves
    /// clicks against the previous frame's widgets.
    fn press_pick(app: &mut App, ctx: &egui::Context, i: usize) {
        let size = egui::vec2(1000.0, 700.0);
        let at = pick_button(app, ctx, i);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            app,
            ctx,
            size,
            vec![egui::Event::PointerMoved(at), button(true)],
        );
        frame(app, ctx, size, vec![button(false)]);
    }

    /// Press on a painted string and release a frame later: egui resolves clicks against the previous frame's widgets.
    fn click(app: &mut App, ctx: &egui::Context, text: &str) {
        let shapes = shapes_at(app, ctx, egui::vec2(1000.0, 700.0));
        click_at(app, ctx, hint(&shapes, text).center());
    }

    fn click_at(app: &mut App, ctx: &egui::Context, at: egui::Pos2) {
        let size = egui::vec2(1000.0, 700.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            app,
            ctx,
            size,
            vec![egui::Event::PointerMoved(at), button(true)],
        );
        frame(app, ctx, size, vec![button(false)]);
    }

    fn hint(shapes: &[egui::epaint::ClippedShape], text: &str) -> egui::Rect {
        shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text() == text => Some(s.shape.visual_bounding_rect()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text:?} is on screen"))
    }

    fn has(shapes: &[egui::epaint::ClippedShape], text: &str) -> bool {
        shapes
            .iter()
            .any(|s| matches!(&s.shape, Shape::Text(t) if t.galley.text() == text))
    }

    /// A repo the screen has already read, so a test can frame it without
    /// anything going out to GitHub.
    fn answered(app: &mut App, source: &Source, files: Vec<String>) {
        let key = crate::features::sync::logic::source_key(source);
        app.sync.listing.insert(key.clone(), files);
        app.sync.read_at.insert(key, std::time::Instant::now());
    }

    fn field(shapes: &[egui::epaint::ClippedShape], at: egui::Rect) -> egui::Rect {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.rect.contains(at.center()) => Some(r.rect),
                _ => None,
            })
            .min_by(|a, b| a.area().partial_cmp(&b.area()).unwrap())
            .expect("the hint sits inside a field")
    }

    /// The square control standing at the right end of a field's row: the file
    /// picker, or the reveal eye.
    fn beside(shapes: &[egui::epaint::ClippedShape], field: egui::Rect) -> egui::Rect {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.rect.min.x > field.max.x => Some(r.rect),
                _ => None,
            })
            .filter(|r| (r.center().y - field.center().y).abs() <= 1.0)
            .max_by(|a, b| a.area().partial_cmp(&b.area()).unwrap())
            .expect("a control beside the field")
    }

    /// A masked field paints bullets, so the eye is found between them and the
    /// card's edge.
    #[test]
    fn the_eye_reveals_the_master_password() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.auth.unlock_password = "hunter2".to_owned();

        let masked = shapes(&mut app, &ctx);
        assert!(!has(&masked, "hunter2"), "the password starts masked");
        let dots = masked
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text().starts_with('•') => {
                    Some(s.shape.visual_bounding_rect())
                }
                _ => None,
            })
            .expect("the masked field wears bullets");

        click_at(
            &mut app,
            &ctx,
            beside(&masked, field(&masked, dots)).center(),
        );
        assert!(
            has(&shapes(&mut app, &ctx), "hunter2"),
            "the eye reveals the password"
        );
    }

    #[test]
    fn the_file_picker_matches_the_field_it_sits_beside() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        let shapes = shapes(&mut app, &ctx);

        let path = field(&shapes, hint(&shapes, "box file"));
        let picker = beside(&shapes, path);
        assert_eq!(picker.min.y, path.min.y, "picker top flush with the field");
        assert_eq!(
            picker.max.y, path.max.y,
            "picker bottom flush with the field"
        );
        assert!(
            (picker.width() - picker.height()).abs() <= 1.0,
            "picker is square: {picker:?}"
        );

        let password = field(&shapes, hint(&shapes, "master password"));
        let eye = beside(&shapes, password);
        assert!(
            (eye.max.x - picker.max.x).abs() <= 1.0,
            "the password row ends where the path row does: {eye:?} vs {picker:?}"
        );
    }

    #[test]
    fn previously_opened_boxes_are_listed() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Recents);
        app.auth.recents = vec![Recent {
            path: PathBuf::from("/tmp/work.box"),
        }];
        let shapes = shapes(&mut app, &ctx);
        hint(&shapes, "work");
        hint(&shapes, "/tmp/work.box");
        hint(&shapes, "search");
    }

    #[test]
    fn the_search_hides_boxes_that_do_not_match() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Recents);
        app.auth.recents = vec![
            Recent {
                path: PathBuf::from("/tmp/work.box"),
            },
            Recent {
                path: PathBuf::from("/tmp/home.box"),
            },
        ];
        app.auth.recent_search = "work".to_owned();
        let shapes = shapes(&mut app, &ctx);
        hint(&shapes, "work");
        assert!(!has(&shapes, "home"), "the filter must hide the rest");
    }

    #[test]
    fn the_scroll_bar_shows_a_handle_that_turns_accent() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let size = egui::vec2(700.0, 560.0);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Recents);
        app.auth.recents = (0..12)
            .map(|i| Recent {
                path: PathBuf::from(format!("/tmp/box{i}.pandora")),
            })
            .collect();

        let bar = |shapes: &[egui::epaint::ClippedShape], fill| {
            shapes.iter().find_map(|s| match &s.shape {
                Shape::Rect(r)
                    if r.fill == fill
                        && r.rect.width() < 20.0
                        && r.rect.height() > 3.0
                        && r.rect.center().x > size.x * 0.75 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
        };

        shapes_at(&mut app, &ctx, size); // the bar animates in: the first frame paints nothing
        let shapes = shapes_at(&mut app, &ctx, size);
        let handle = bar(&shapes, theme::palette().hair).expect("the handle is painted");
        assert!(
            bar(&shapes, theme::palette().elev).is_none(),
            "the track must not be filled like the handle"
        );
        assert!(
            shapes.iter().any(|s| matches!(&s.shape, Shape::Rect(r)
                if r.fill == theme::palette().elev && r.rect.width() > 100.0)),
            "the search field above the band keeps its own fill"
        );
        drop(shapes);

        let at = handle.center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(at), button(true)],
        );
        let shapes = frame(&mut app, &ctx, size, vec![]);
        assert!(
            bar(&shapes, theme::ACCENT).is_some(),
            "the handle goes accent while it is dragged"
        );
    }

    #[test]
    fn the_recents_carry_every_source_with_its_own_mark() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Recents);
        let work = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/work".to_owned(),
        };
        app.prefs.sources = vec![work.clone()];
        app.auth.recents = vec![
            Recent {
                path: crate::vault::store::source_dir(&work.kind, &work.target)
                    .join("bank.pandora"),
            },
            Recent {
                path: PathBuf::from("/tmp/plain.pandora"),
            },
        ];

        let shapes = shapes_at(&mut app, &ctx, egui::vec2(1000.0, 1600.0));
        // The row is called after the file: the box's own name is inside it.
        hint(&shapes, "bank");
        hint(&shapes, "plain");
    }

    #[test]
    fn the_picker_switches_between_the_three_views() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;
        app.auth.recents = vec![Recent {
            path: PathBuf::from("/tmp/plain.pandora"),
        }];
        let work = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/work".to_owned(),
        };
        app.prefs.sources = vec![work.clone()];
        app.sync.repo_loaded = true;
        answered(&mut app, &work, vec!["bank.pandora".to_owned()]);

        // Untouched, the screen opens on the recents.
        let start = shapes(&mut app, &ctx);
        hint(&start, "plain");
        assert!(
            !has(&start, "Open a box file…") && !has(&start, "octocat/work"),
            "the other two views stay out of the recents"
        );

        press_pick(&mut app, &ctx, 1);
        assert_eq!(app.auth.side, Some(Side::Machine));
        let machine = shapes(&mut app, &ctx);
        hint(&machine, "Open a box file…");
        hint(&machine, "Create a new box");
        assert!(
            !has(&machine, "octocat/work"),
            "the repos are their own view"
        );

        press_pick(&mut app, &ctx, 2);
        assert_eq!(app.auth.side, Some(Side::Github));
        let out = shapes(&mut app, &ctx);
        hint(&out, "Sign in with GitHub");
        // The repo the app already knows shows without a session — only reaching
        // GitHub needs the token, so the sign-in is an action, not a wall.
        hint(&out, "octocat/work");
        hint(&out, "bank");
        assert!(
            !has(&out, "Add a GitHub repo…"),
            "adding a repo is what the session is for"
        );

        app.sync.token = Zeroizing::new("t".to_owned());
        let github = shapes(&mut app, &ctx);
        hint(&github, "octocat/work");
        hint(&github, "bank");
        hint(&github, "Add a GitHub repo…");
        assert!(
            !has(&github, "Open a box file…"),
            "the local actions stayed behind"
        );

        press_pick(&mut app, &ctx, 0);
        assert_eq!(app.auth.side, Some(Side::Recents));
        let recents = shapes(&mut app, &ctx);
        hint(&recents, "plain");
        assert!(
            !has(&recents, "octocat/work") && !has(&recents, "Open a box file…"),
            "the recents hold only boxes"
        );
    }

    /// The picker has no labels to read: each icon owes the screen reader its name.
    #[test]
    fn the_picker_icons_announce_their_names() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        ctx.enable_accesskit();
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;

        let mut named: Vec<String> = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(shot::input(egui::vec2(1000.0, 700.0), Vec::new()), |ui| {
                draw(ui, &mut app)
            });
            out.textures_delta.clear();
            if let Some(update) = &out.platform_output.accesskit_update {
                for (_, node) in &update.nodes {
                    if node.role() == egui::accesskit::Role::Button {
                        if let Some(name) = node.label() {
                            named.push(name.to_owned());
                        }
                    }
                }
            }
        }
        for want in ["Recent boxes", "GitHub repos", "Boxes on this machine"] {
            assert!(named.iter().any(|n| n == want), "missing {want}: {named:?}");
        }
    }

    /// The repo's own controls carry no text: their names ride the tooltips,
    /// which is also what a screen reader has to announce.
    #[test]
    fn the_repo_icons_announce_their_names() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        ctx.enable_accesskit();
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        app.sync.token = Zeroizing::new("t".to_owned());
        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/work".to_owned(),
        };
        app.prefs.sources = vec![source.clone()];
        answered(&mut app, &source, Vec::new());

        let mut named: Vec<String> = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(shot::input(egui::vec2(1000.0, 700.0), Vec::new()), |ui| {
                draw(ui, &mut app)
            });
            out.textures_delta.clear();
            if let Some(update) = &out.platform_output.accesskit_update {
                for (_, node) in &update.nodes {
                    if node.role() == egui::accesskit::Role::Button {
                        if let Some(name) = node.label() {
                            named.push(name.to_owned());
                        }
                    }
                }
            }
        }
        for want in [
            "Remove this repo — the copies already on this computer stay",
            "Reload this repo",
        ] {
            assert!(named.iter().any(|n| n == want), "missing {want}: {named:?}");
        }
    }

    #[test]
    fn an_empty_recents_offers_the_way_in() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;

        let shapes = shapes(&mut app, &ctx);
        hint(&shapes, "Local");
        hint(&shapes, "Boxes on this machine");
        hint(&shapes, "GitHub");
        hint(&shapes, "Your repositories");
        assert!(
            !has(&shapes, "Open a box file…") && !has(&shapes, "Create a new box"),
            "the local actions live one step deeper, in the Local view"
        );
        assert!(
            !has(&shapes, "search"),
            "the recents list, and its search, are not there to show"
        );
    }

    #[test]
    fn the_empty_recents_navigate_to_local_and_github() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;

        click(&mut app, &ctx, "Local");
        assert_eq!(
            app.auth.side,
            Some(Side::Machine),
            "Local opens the machine view"
        );
        hint(&shapes(&mut app, &ctx), "Open a box file…");

        app.auth.side = Some(Side::Recents);
        click(&mut app, &ctx, "GitHub");
        assert_eq!(
            app.auth.side,
            Some(Side::Github),
            "GitHub opens its own view"
        );
    }

    #[test]
    fn the_add_a_repo_button_opens_its_own_window() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        app.sync.token = Zeroizing::new("t".to_owned());
        app.sync.repo_loaded = true;
        app.sync.repos = vec![crate::features::sync::github::Repo {
            full_name: "octocat/boxes".to_owned(),
            owner: "octocat".to_owned(),
            private: false,
        }];
        assert!(
            !has(&shapes(&mut app, &ctx), "GITHUB"),
            "the window stays shut until the button is pressed"
        );

        click(&mut app, &ctx, "Add a GitHub repo…");
        assert!(app.sync.adding, "the button opens the window");
        // egui fades an area in: the window only paints whole on its second frame.
        shapes(&mut app, &ctx);
        let open = shapes(&mut app, &ctx);
        hint(&open, "GITHUB");
        hint(&open, "octocat/boxes");
    }

    #[test]
    fn the_welcome_link_opens_the_github_half() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Welcome;

        click(&mut app, &ctx, "Keep the box in a GitHub repo");
        assert_eq!(app.screen, Screen::Sources);
        assert_eq!(app.auth.side, Some(Side::Github));
        assert!(app.auth.login.is_some(), "the link starts the sign-in");
        assert!(!app.sync.adding, "no window is asked for");

        let shapes = shapes(&mut app, &ctx);
        hint(&shapes, "asking GitHub for a code…");
    }

    #[test]
    fn the_pending_code_is_shown_with_its_page() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        let (_tx, rx) = std::sync::mpsc::channel();
        app.auth.login = Some(Login {
            code: Some((
                "ABCD-1234".to_owned(),
                "https://github.com/login/device".to_owned(),
            )),
            rx,
        });

        shapes(&mut app, &ctx);
        let shapes = shapes(&mut app, &ctx);
        let code = hint(&shapes, "ABCD-1234");
        // The page rides in the sentence above the code, without its scheme, and
        // the copy sits at the field's far edge instead of behind the value.
        hint(&shapes, "enter this code at github.com/login/device");
        let copy = shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Rect(r)
                    if r.brush.is_some() && (r.rect.center().y - code.center().y).abs() < 30.0 =>
                {
                    Some(r.rect.center())
                }
                _ => None,
            })
            .expect("the code carries a copy beside it");

        click_at(&mut app, &ctx, copy);
        let clip = app.shell.clip.expect("the button copies through the app");
        assert!(clip.holds("ABCD-1234"));
        assert!(!clip.holds("enter this code at github.com/login/device"));
    }

    #[test]
    fn the_add_a_repo_window_fits_a_small_window() {
        for size in [
            egui::vec2(1000.0, 700.0),
            egui::vec2(420.0, 480.0),
            // The floor `main.rs` puts on the window: nothing narrower ever opens,
            // and below it the picker has no room left for a repo row.
            egui::vec2(380.0, 520.0),
        ] {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut app = unlock_app(&ctx);
            app.screen = Screen::Sources;
            app.auth.side = Some(Side::Github);
            app.sync.adding = true;
            app.sync.token = Zeroizing::new("t".to_owned());
            app.sync.repo_loaded = true;
            app.sync.repos = vec![crate::features::sync::github::Repo {
                full_name: "otracuenta/un-nombre-de-repo-largo".to_owned(),
                owner: "otracuenta".to_owned(),
                private: false,
            }];

            shapes_at(&mut app, &ctx, size);
            let shapes = shapes_at(&mut app, &ctx, size);
            for text in ["GITHUB", "otracuenta/un-nombre-de-repo-largo"] {
                let r = hint(&shapes, text);
                assert!(
                    r.min.x >= 0.0 && r.max.x <= size.x && r.min.y >= 0.0 && r.max.y <= size.y,
                    "{text:?} paints at {r:?}, outside a {size:?} window"
                );
            }

            app.sync.adding = false;
            app.sync.token = Zeroizing::new(String::new());
            let (_tx, rx) = std::sync::mpsc::channel();
            app.auth.login = Some(Login {
                code: Some((
                    "ABCD-1234".to_owned(),
                    "https://github.com/login/device".to_owned(),
                )),
                rx,
            });
            // The code card lives in the view, which scrolls: it needs a window
            // tall enough to hold the wordmark, the picker and the card.
            let tall = egui::vec2(size.x, size.y.max(560.0));
            shapes_at(&mut app, &ctx, tall);
            let shapes = shapes_at(&mut app, &ctx, tall);
            for text in ["ABCD-1234", "enter this code at github.com/login/device"] {
                let r = hint(&shapes, text);
                assert!(
                    r.min.x >= 0.0 && r.max.x <= tall.x && r.min.y >= 0.0 && r.max.y <= tall.y,
                    "{text:?} paints at {r:?}, outside a {tall:?} window"
                );
            }
        }
    }

    /// The sources are the app's front door: recents, local files and GitHub
    /// repos, whether or not a box was ever opened.
    #[test]
    fn the_app_lands_on_the_sources() {
        let ctx = egui::Context::default();
        let mut app = unlock_app(&ctx);
        assert_eq!(
            app.auth.landing(),
            Screen::Sources,
            "with nothing to reopen"
        );
        app.auth.recents = vec![Recent {
            path: PathBuf::from("/tmp/work.box"),
        }];
        assert_eq!(app.auth.landing(), Screen::Sources, "with recents");
    }

    #[test]
    fn back_returns_to_the_screen_left_behind() {
        let ctx = egui::Context::default();
        let mut app = unlock_app(&ctx);
        app.record_place();
        assert!(!app.can_back(), "home has nowhere to go back to");

        app.auth.recents = vec![Recent {
            path: PathBuf::from("/tmp/work.box"),
        }];
        app.screen = Screen::Sources;
        app.record_place();
        app.back();
        assert_eq!(
            app.screen,
            Screen::Unlock,
            "Back returns to the screen left behind"
        );

        app.back();
        assert_eq!(app.screen, Screen::Unlock, "nothing more is behind it");
        assert!(!app.can_back());
    }

    #[test]
    fn the_recents_card_never_overflows_a_short_window() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Recents);
        app.auth.recents = (0..12)
            .map(|i| Recent {
                path: PathBuf::from(format!("/tmp/box{i}.box")),
            })
            .collect();

        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(380.0, 520.0));
        for _ in 0..3 {
            let shapes = shapes_at(&mut app, &ctx, screen.size());
            for s in &shapes {
                let r = s.shape.visual_bounding_rect().intersect(s.clip_rect);
                assert!(
                    !r.is_positive() || screen.contains_rect(r),
                    "a shape spills off the screen: {r:?}"
                );
            }
        }
    }

    /// The empty card is the two ways in and nothing else: they fit the same
    /// narrow window the full recents list does.
    #[test]
    fn the_empty_recents_start_never_overflows_a_short_window() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Recents);

        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(380.0, 520.0));
        for _ in 0..3 {
            let shapes = shapes_at(&mut app, &ctx, screen.size());
            for s in &shapes {
                let r = s.shape.visual_bounding_rect().intersect(s.clip_rect);
                assert!(
                    !r.is_positive() || screen.contains_rect(r),
                    "a shape spills off the screen: {r:?}"
                );
            }
        }
        let shapes = shapes_at(&mut app, &ctx, screen.size());
        hint(&shapes, "Local");
        hint(&shapes, "GitHub");
    }

    #[test]
    fn the_password_and_its_confirmation_never_share_a_row() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Welcome;

        let pair = |app: &mut App, width: f32| {
            let shapes = shapes_at(app, &ctx, egui::vec2(width, 700.0));
            (
                field(&shapes, hint(&shapes, "master password (min 12)")),
                field(&shapes, hint(&shapes, "confirm")),
            )
        };

        // A wide window included: the pair stacks whatever room the form has.
        for width in [380.0, 1000.0] {
            let (password, confirm) = pair(&mut app, width);
            assert!(
                confirm.top() >= password.bottom(),
                "the confirmation shares the password's row at {width}: {password:?} {confirm:?}"
            );
        }
    }

    /// The three views fit a narrow window, and a short one only clips: whatever
    /// the page cannot show is cut at the pane, never painted past the window. The
    /// short size is the window's own floor (`main.rs`: 380 by 520), where the auth
    /// forms scroll — the bar and the card it moves stay inside it either way.
    #[test]
    fn the_auth_forms_fit_a_narrow_window() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.auth.recents = vec![Recent {
            path: PathBuf::from("/tmp/work.box"),
        }];

        let gutter = 12.0;
        for size in [
            egui::vec2(380.0, 640.0),
            egui::vec2(320.0, 640.0),
            egui::vec2(380.0, 520.0),
        ] {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            for which in [Screen::Welcome, Screen::Unlock, Screen::Sources] {
                app.screen = which;
                for s in shapes_at(&mut app, &ctx, size) {
                    let r = s.shape.visual_bounding_rect();
                    if !r.is_positive() || !r.is_finite() {
                        continue;
                    }
                    // Only the panel's own background may reach the edges.
                    if r.min.x <= 1.0 && r.max.x >= size.x - 1.0 {
                        continue;
                    }
                    assert!(
                        r.min.x >= gutter - 1.0 && r.max.x <= size.x - gutter + 1.0,
                        "{which:?} at {size:?} hugs the edge: {r:?}"
                    );
                    // What a short page cannot show is clipped at the pane, so the
                    // part that is painted is on the window.
                    let shown = s.clip_rect.intersect(r);
                    assert!(
                        !shown.is_positive() || screen.contains_rect(shown),
                        "{which:?} at {size:?} paints past the window: {shown:?}"
                    );
                }
            }
        }
    }

    /// The card is taller than a short window, so the page has to say there is
    /// more below it and let the wheel reach it. The bar is the one egui paints
    /// for the area, in the app's own gutter — the pane's, never the window's edge.
    #[test]
    fn a_short_window_scrolls_the_auth_forms() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);

        // The create form is held to the smallest window the app opens (`main.rs`:
        // 380 by 520). Opening asks for less room, so it is held to a phone on its
        // side instead, where its button is cut and the link under it is not
        // painted at all.
        for (which, size, control) in [
            (Screen::Welcome, egui::vec2(380.0, 520.0), "Create box"),
            (Screen::Unlock, egui::vec2(380.0, 440.0), "Unlock"),
        ] {
            app.screen = which;
            let page = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let shown = |shapes: &[egui::epaint::ClippedShape]| {
                shapes.iter().find_map(|s| match &s.shape {
                    Shape::Text(t) if t.galley.text() == control => {
                        Some(s.shape.visual_bounding_rect())
                    }
                    _ => None,
                })
            };

            // The bar is drawn off the layout of the frame before it, so it takes
            // a couple of frames to come up.
            shapes_at(&mut app, &ctx, size);
            let shapes = shapes_at(&mut app, &ctx, size);
            assert!(
                shapes.iter().any(|s| match &s.shape {
                    Shape::Rect(r) =>
                        (r.fill == theme::palette().hair || r.fill == theme::palette().rail)
                            && r.rect.width() < 20.0
                            && r.rect.height() > 100.0
                            && r.rect.center().x > size.x * 0.75,
                    _ => false,
                }),
                "{which:?} is taller than its window, so it wears the bar"
            );
            assert!(
                shown(&shapes).is_none_or(|at| !page.contains_rect(at)),
                "{which:?}: {control:?} starts off the page: {:?}",
                shown(&shapes)
            );

            // The wheel over the page brings the card up until the control is on
            // it. The gesture is lifted and the page let to settle afterwards: a
            // wheel left mid-gesture keeps smoothing into the frames after it, and
            // the next page would open already scrolled.
            let over = egui::pos2(size.x / 2.0, size.y / 2.0);
            let wheel = |delta: f32, phase| egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, delta),
                phase,
                modifiers: Default::default(),
            };
            let mut shapes = shapes;
            for _ in 0..12 {
                shapes = frame(
                    &mut app,
                    &ctx,
                    size,
                    vec![
                        egui::Event::PointerMoved(over),
                        wheel(-60.0, egui::TouchPhase::Move),
                    ],
                );
            }
            frame(
                &mut app,
                &ctx,
                size,
                vec![wheel(0.0, egui::TouchPhase::End)],
            );
            for _ in 0..3 {
                shapes = frame(&mut app, &ctx, size, Vec::new());
            }
            let at =
                shown(&shapes).unwrap_or_else(|| panic!("{which:?}: {control:?} is on the page"));
            assert!(
                page.contains_rect(at),
                "{which:?}: the wheel brings {control:?} onto the page: {at:?}"
            );
        }
    }

    #[test]
    fn the_auth_screens_share_one_top_anchor() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let size = egui::vec2(1000.0, 700.0);
        let mut app = unlock_app(&ctx);
        app.auth.recents = vec![Recent {
            path: PathBuf::from("/tmp/work.box"),
        }];

        let wordmark = "Save your secrets inside Pandora’s box";
        let mut at = |screen: Screen| {
            app.screen = screen;
            let shapes = shapes_at(&mut app, &ctx, size);
            hint(&shapes, wordmark).min.y
        };
        let welcome = at(Screen::Welcome);
        let unlock = at(Screen::Unlock);
        let recents = at(Screen::Sources);
        assert_eq!(welcome, unlock, "create and open share the wordmark's top");
        assert_eq!(unlock, recents, "the way in wears it too");
    }

    /// The three views stand under the wordmark, and the one on show holds its
    /// boxes in the card below them.
    #[test]
    fn the_sources_screen_picks_a_view_under_its_wordmark() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let size = egui::vec2(1000.0, 700.0);
        let mut app = unlock_app(&ctx);
        app.screen = Screen::Sources;
        app.auth.recents = vec![Recent {
            path: PathBuf::from("/tmp/work.box"),
        }];

        let shapes = shapes_at(&mut app, &ctx, size);
        hint(&shapes, "Save your secrets inside Pandora’s box");
        let first = hint(&shapes, "work");
        assert!(
            first.min.y > 0.0 && first.min.y < size.y,
            "the recents sit inside the window: {first:?}"
        );
        assert_eq!(
            pick_button(&mut app, &ctx, 0).y,
            pick_button(&mut app, &ctx, 1).y,
            "the three views stand on one line"
        );
    }

    #[test]
    fn save_dialog_keeps_the_name_it_was_given() {
        assert_eq!(
            default_name(Path::new("/data/pandora/work.pandora")),
            "work.pandora"
        );
        assert_eq!(default_name(Path::new("/")), "vault.pandora");
        assert_eq!(default_name(Path::new("")), "vault.pandora");
    }

    #[test]
    fn a_refused_box_counts_down_instead_of_unlocking() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        app.auth.unlock_path = "/nowhere/refused.pandora".to_owned();
        app.prefs.retry_delay = 40;
        app.prefs
            .failed(&prefs::box_key(&app.auth.unlock_path), prefs::now());

        let shapes = shapes(&mut app, &ctx);
        assert!(
            shapes.iter().any(
                |s| matches!(&s.shape, Shape::Text(t) if t.galley.text().starts_with("wait "))
            ),
            "the button counts the wait down"
        );
        assert!(!has(&shapes, "Unlock"), "it is no longer the way in");

        app.unlock();
        assert!(app.auth.op.is_none(), "a refused box starts no derivation");
        assert!(
            app.shell
                .status
                .as_ref()
                .is_some_and(|s| s.text.contains("too many attempts")),
            "and says why: {:?}",
            app.shell.status.as_ref().map(|s| &s.text)
        );
    }

    #[test]
    fn the_unlock_button_holds_the_spinner() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = unlock_app(&ctx);
        let (_tx, rx) = std::sync::mpsc::channel();
        app.auth.op = Some(crate::app::Op {
            kind: OpKind::Unlock,
            rx,
        });
        let shapes = shapes(&mut app, &ctx);

        assert!(
            !has(&shapes, "unlocking — deriving key…"),
            "no loader row below the card"
        );
        let button = field(&shapes, hint(&shapes, "Unlocking…"));
        let spinner = shapes.iter().any(|s| match &s.shape {
            Shape::Path(p) => p.points.iter().all(|pt| button.contains(*pt)),
            _ => false,
        });
        assert!(spinner, "the spinner is painted inside the button");
        let accent = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.rect.contains(button.center()) => Some(r.fill),
                _ => None,
            })
            .any(|fill| fill == theme::ACCENT);
        assert!(accent, "the accent fill is not faded while busy");
    }
}
