use crate::app::{App, Tab};
use crate::features::codes::ui as codes;
use crate::features::drive::ui as drive;
use crate::features::entries::ui::list as passwords;
use crate::features::settings::ui as settings;
use crate::shared::theme;
use crate::shared::ui::responsive;
use egui::{Color32, Context, Layout, RichText, Ui};

pub fn main(ui: &mut Ui, app: &mut App) {
    let ctx: Context = ui.ctx().clone();
    new_modal(&ctx, app);

    let compact = responsive::compact(&ctx);
    let (conflict, behind) = app
        .boxes
        .session
        .as_ref()
        .map_or((false, false), |s| (s.conflict, s.behind));

    if conflict {
        banner(
            ui,
            "conflict",
            "box changed on disk — another device synced?",
            |ui| {
                if ui.button(crate::shared::i18n::t("Reload theirs")).clicked() {
                    if let Some(s) = app.boxes.session.as_mut() {
                        s.confirm = Some(crate::app::Confirm::Reload);
                    }
                }
                if ui
                    .button(crate::shared::i18n::t("Overwrite with mine"))
                    .clicked()
                {
                    app.boxes.conflict_overwrite(&mut app.shell);
                }
                if ui.button(crate::shared::i18n::t("Keep editing")).clicked() {
                    app.boxes.conflict_keep_mine();
                }
            },
        );
    } else if behind {
        banner(
            ui,
            "behind",
            "the repo has a newer version of this box",
            |ui| {
                if ui
                    .button(crate::shared::i18n::t("Get the newer version"))
                    .clicked()
                {
                    app.pull();
                }
            },
        );
    }

    // A wide window keeps the panes on the strip under the bar; a narrow one hands them
    // to the floor, where a thumb reaches them.
    if !crate::shared::ui::chrome::touch(&ctx) {
        tabs(ui, app);
    }

    match app
        .boxes
        .session
        .as_ref()
        .map(|s| s.tab)
        .unwrap_or(Tab::Passwords)
    {
        Tab::Passwords => passwords::main(ui, app, compact),
        Tab::Drive => drive::main(ui, app, compact),
        Tab::Codes => codes::main(ui, app, compact),
        Tab::Settings => settings::run(ui, app, compact),
    }

    // The entry's own generator, over the modal the drive opens it in.
    crate::features::generator::ui::modal(&ctx, app);

    // Last, so the confirmation sits above the modal it was opened from.
    crate::features::shell::confirm::modal(&ctx, app);
}

/// The strip an error wears: message and way out, wrapping in a narrow window.
fn banner(ui: &mut Ui, id: &'static str, message: &'static str, actions: impl FnOnce(&mut Ui)) {
    egui::Panel::top(id)
        .frame(
            egui::Frame::NONE
                .fill(theme::palette().panel)
                .inner_margin(egui::Margin::symmetric(18, 10))
                .stroke(theme::stroke(theme::palette().error, 1.0)),
        )
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(crate::shared::i18n::t(message))
                            .color(theme::palette().error),
                    )
                    .wrap(),
                );
                actions(ui);
            });
        });
}

/// Height of the tab strip; tabs span it so their underline sits on the bottom rule.
const BAR: f32 = 44.0;
const PAD: f32 = 14.0;
const GAP: f32 = 10.0;

/// Lock and box settings act on the open box, so they live on its strip; a narrow
/// window moves the panes and settings to the floor.
fn tabs(ui: &mut Ui, app: &mut App) {
    let want = app
        .boxes
        .session
        .as_ref()
        .map(|s| s.tab)
        .unwrap_or(Tab::Passwords);
    let mut picked = want;
    let mut lock = false;
    let mut settings = false;
    egui::Panel::top("tabs")
        .exact_size(BAR)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(theme::palette().panel))
        .show(ui, |ui| {
            let bar = ui.max_rect();
            ui.painter().rect_filled(
                egui::Rect::from_min_max(egui::pos2(bar.min.x, bar.max.y - 1.0), bar.max),
                egui::CornerRadius::ZERO,
                theme::palette().hair,
            );
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                for (pane, icon, label) in [
                    (Tab::Passwords, theme::Icon::Key, "Passwords"),
                    (Tab::Drive, theme::Icon::Folder, "Files"),
                    (Tab::Codes, theme::Icon::RefreshCw, "Codes"),
                ] {
                    if tab(ui, &app.icons, icon, label, want == pane) {
                        picked = pane;
                    }
                }
                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    // Mirrors the strip's left inset, so the far end is not flush with the border.
                    ui.add_space(12.0);
                    // Right to left: lock rightmost, gear beside it.
                    lock = bar_button(
                        ui,
                        &app.icons,
                        theme::Icon::LockKeyhole,
                        theme::palette().error,
                        "Lock box",
                        false,
                    );
                    settings = bar_button(
                        ui,
                        &app.icons,
                        theme::Icon::Settings,
                        theme::palette().muted,
                        crate::shared::i18n::t("Box settings"),
                        want == Tab::Settings,
                    );
                });
            });
        });

    if lock {
        app.lock(ui.ctx());
    } else if settings {
        picked = Tab::Settings;
    }
    app.boxes.pane(picked);
}

/// Reserves the slot before its content, so wash and rule land behind it.
fn slot(
    ui: &mut Ui,
    width: f32,
    id: egui::Id,
    on: bool,
    content: impl FnOnce(&mut Ui),
) -> egui::Response {
    theme::slot(ui, width, id, on, BAR, theme::Edge::Floor, content)
}

fn tab(ui: &mut Ui, icons: &theme::Icons, icon: theme::Icon, label: &str, on: bool) -> bool {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text_w = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, theme::palette().muted)
        .size()
        .x;
    let icon_w = responsive::glyph(ui.ctx());
    let width = PAD + icon_w + GAP + text_w + PAD;

    let color = if on {
        theme::palette().text
    } else {
        theme::palette().muted
    };
    let tint = if on {
        theme::ACCENT
    } else {
        theme::palette().muted
    };
    let resp = slot(ui, width, ui.id().with(("tab", label)), on, |ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        ui.add_space(PAD);
        if let Some(img) = icons.image(icon, icon_w, tint) {
            ui.add(img);
        }
        ui.add(egui::Label::new(RichText::new(label).color(color)));
    });
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, label)
    });
    resp.clicked()
}

fn bar_button(
    ui: &mut Ui,
    icons: &theme::Icons,
    icon: theme::Icon,
    tint: Color32,
    tooltip: &str,
    on: bool,
) -> bool {
    let icon_w = responsive::glyph(ui.ctx());
    let width = PAD + icon_w + PAD;
    let resp = slot(ui, width, ui.id().with(("bar", tooltip)), on, |ui| {
        ui.add_space(PAD);
        if let Some(img) = icons.image(icon, icon_w, tint) {
            ui.add(img);
        }
    });
    resp.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            on,
            tooltip.to_owned(),
        )
    });
    resp.on_hover_text(tooltip).clicked()
}

/// The pane's own `+`: the drive holds note and file, so it is the one that asks.
fn new_modal(ctx: &Context, app: &mut App) {
    if !app.boxes.session.as_ref().is_some_and(|s| s.show_new) {
        return;
    }
    let Some(tab) = app.boxes.session.as_ref().map(|s| s.tab) else {
        return;
    };
    if tab != Tab::Drive {
        match tab {
            Tab::Codes => app.boxes.add_otp(),
            _ => app.boxes.add_login(),
        }
        if let Some(s) = app.boxes.session.as_mut() {
            s.show_new = false;
        }
        return;
    }

    let mut note = false;
    let mut file = false;
    let open = theme::dialog(ctx, egui::Id::new("new-drive-entry"), |ui| {
        theme::width(ui, 300.0);
        let closed = theme::head(ui, &app.icons, "New note or file");
        ui.add_space(6.0);
        note = theme::secondary_button(ui, &app.icons, Some(theme::Icon::Note), "Note").clicked();
        ui.add_space(6.0);
        file =
            theme::secondary_button(ui, &app.icons, Some(theme::Icon::Paperclip), "File").clicked();
        closed || note || file
    });
    if note {
        app.boxes.add_note();
    }
    if file {
        app.boxes.pick_files(&mut app.shell);
    }
    if note || file || !open {
        if let Some(s) = app.boxes.session.as_mut() {
            s.show_new = false;
        }
    }
}

#[cfg(test)]
mod shots {
    use super::*;
    use crate::app::{Drag, Screen, Session, StatusKind};
    use crate::features::entries::model::{Axis, View};
    use crate::shared::ui::shot::{self, words};
    use crate::vault::model::{Entry, Folder, Login, Note, Vault};
    use crate::vault::store::Credentials;
    use egui::{pos2, vec2, Pos2, Rect, Shape};
    use std::path::PathBuf;
    use zeroize::Zeroizing;

    fn shot(
        ctx: &egui::Context,
        name: &str,
        app: &mut App,
        w: u32,
        h: u32,
        held: Option<(Pos2, Pos2)>,
    ) {
        shot::png(ctx, name, w, h, held, |ui| main(ui, app));
    }

    fn center_of(shapes: &[egui::epaint::ClippedShape], text: &str, min_x: f32) -> Option<Pos2> {
        shapes.iter().find_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.text() == text => {
                let center = s.shape.visual_bounding_rect().center();
                (center.x >= min_x).then_some(center)
            }
            _ => None,
        })
    }

    const DRIVE_X: f32 = 240.0;

    const ROW: f32 = 270.0;

    /// Nothing may paint past the window's own width (floor 380pt, `main.rs`); an empty
    /// shape paints nothing and carries no bounds.
    fn fits_window(shapes: &[egui::epaint::ClippedShape], width: f32) {
        for s in shapes {
            let r = s.shape.visual_bounding_rect();
            if !r.min.x.is_finite() || !r.max.x.is_finite() {
                continue;
            }
            assert!(
                r.min.x >= -1.0 && r.max.x <= width + 1.0,
                "paints past the window: {r:?}"
            );
        }
    }

    fn login(name: &str, id: &str, user: &str, icon: Option<&str>) -> Login {
        let mut it = Login::new(0);
        it.id = id.to_owned();
        it.name = name.to_owned();
        it.username = user.to_owned();
        it.icon = icon.map(str::to_owned);
        it
    }

    fn note(name: &str, id: &str, folder: &str) -> Note {
        let mut it = Note::new(0);
        it.id = id.to_owned();
        it.name = name.to_owned();
        it.folder = Some(folder.to_owned());
        it
    }

    fn file(name: &str, id: &str, folder: &str, len: usize) -> crate::vault::model::File {
        let mut it = crate::vault::model::File::new(name.to_owned(), vec![0u8; len], 0);
        it.id = id.to_owned();
        it.folder = Some(folder.to_owned());
        it
    }

    fn demo(ctx: &egui::Context, tab: Tab, selected: Option<&str>) -> App {
        let mut vault = Vault::new("Personal");
        let mut docs = Folder::new("Documents", None, 0);
        docs.id = "docs".to_owned();
        let mut tax = Folder::new("Tax 2026", Some("docs".to_owned()), 0);
        tax.id = "tax".to_owned();
        vault.folders = vec![docs, tax];

        vault.logins = vec![
            login("GitHub", "github", "octocat", Some("github")),
            login(
                "Banco Santander",
                "bank",
                "salvador.ruiz@correo-largo.example.com",
                None,
            ),
            login(
                "Netflix",
                "netflix",
                "salvador@example.com",
                Some("netflix"),
            ),
        ];
        // The drive keeps one order: two notes and a document, side by side.
        vault.drive = vec![
            Entry::Note(note("Recovery codes", "codes", "docs")),
            Entry::File(file("pasaporte-2031.pdf", "passport", "docs", 4096)),
            Entry::Note(note("Factura luz", "invoice", "docs")),
        ];
        vault.otps = vec![{
            let mut otp = crate::vault::model::Otp::new(0);
            otp.id = "github-code".to_owned();
            otp.name = "GitHub".to_owned();
            otp.secret = Zeroizing::new("JBSWY3DPEHPK3PXP".to_owned());
            otp
        }];

        let selected = selected.map(str::to_owned);
        let mut app = App::blank(ctx);
        app.screen = Screen::Main;
        app.boxes.session = Some(Session {
            selected,
            tab,
            folder: Some("docs".to_owned()),
            ..Session::fresh(
                PathBuf::from("/tmp/pandora-test.vault"),
                Credentials::rekey("test", None).unwrap(),
                vault,
            )
        });
        app
    }

    /// The body never widens the dialog it sits in: a row past the band grows the
    /// modal's box and leaves the header cross short of it.
    #[test]
    fn the_entry_modal_keeps_the_width_it_was_given() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, Some("bank"));
        {
            let s = app.boxes.session.as_mut().unwrap();
            s.view = View::Folders;
            let it = s
                .vault
                .logins
                .iter_mut()
                .find(|i| i.username.contains("ruiz"))
                .unwrap();
            it.url = "accounts.google.com".to_owned();
            for _ in 0..5 {
                it.extra_urls.push(crate::vault::matching::Uri {
                    url: String::new(),
                    rule: crate::vault::matching::Rule::Domain,
                });
            }
            it.notes = "uno\ndos\ntres".to_owned().into();
            it.password = Zeroizing::new("x".repeat(24));
        }

        frame(&ctx, &mut app, vec![]); // the modal is top from the frame after it opened
        let shapes = frame(&ctx, &mut app, vec![]);
        let panel = shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Vec(v) => v.iter().rev().find_map(|s| match s {
                    Shape::Rect(r) => Some(r.rect),
                    _ => None,
                }),
                _ => None,
            })
            .expect("the entry modal is up");
        // 520 of content and the frame's 20 px a side, plus the stroke a pixel out either side.
        assert!(
            (panel.width() - 560.0).abs() < 3.0,
            "the body widened the dialog, so the header is off its own gutter: {panel:?}"
        );
    }

    /// A debug probe, not a test: it prints the shapes of the top of the list so
    /// a layout can be read by eye, and asserts nothing. Ignored on purpose, so a
    /// plain `cargo test` is green; run it with `cargo test -- --ignored probe_rows
    /// --nocapture` when the question is what the frame actually painted.
    #[test]
    #[ignore = "debug probe: prints shapes, asserts nothing"]
    fn probe_rows() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        let shapes = frame(&ctx, &mut app, vec![]);
        for s in &shapes {
            let r = s.shape.visual_bounding_rect();
            if r.max.x > 320.0 || r.min.y > 480.0 {
                continue;
            }
            match &s.shape {
                Shape::Rect(x) => println!("RECT {:?} fill={:?}", x.rect, x.fill),
                Shape::Text(t) => println!(
                    "TEXT {:?} pos={:?} size={:?} ink={:?}",
                    t.galley.text(),
                    t.pos,
                    t.galley.size(),
                    r
                ),
                Shape::Mesh(m) => {
                    println!("MESH n={} bounds={:?}", m.vertices.len(), r)
                }
                other => println!("OTHER {other:?} {r:?}"),
            }
        }
        drop(shapes);
    }

    /// The text block only centres because it is given its own height: a plain
    /// `ui.vertical` takes the whole row and packs the text against the top.
    #[test]
    fn password_rows_share_one_height_and_centre_their_content() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        app.boxes.session.as_mut().unwrap().vault.logins[0]
            .username
            .clear();
        let shapes = frame(&ctx, &mut app, vec![]);

        let rows: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == theme::palette().panel && r.rect.width() == ROW => {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(rows.len(), 3, "one row per password entry");
        for row in &rows {
            assert_eq!(row.height(), 52.0, "ROW_HEIGHT: {row:?}");

            let inside: Vec<Rect> = shapes
                .iter()
                .filter_map(|s| {
                    let r = s.shape.visual_bounding_rect();
                    (r.width() == 15.0 && r.height() == 15.0 && row.contains(r.center()))
                        .then_some(r)
                })
                .collect();
            assert_eq!(
                inside.len(),
                3,
                "the kind icon and the two copy marks: {row:?}"
            );
            for mark in &inside {
                assert!(
                    (mark.center().y - row.center().y).abs() <= 1.0,
                    "mark off centre: {mark:?} in {row:?}"
                );
            }

            let block: Vec<Rect> = shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    Shape::Text(t) if row.contains(s.shape.visual_bounding_rect().center()) => {
                        Some(t.galley.rect.translate(t.pos.to_vec2()))
                    }
                    _ => None,
                })
                .collect();
            let block = block.iter().fold(Rect::NOTHING, |acc, r| acc.union(*r));
            assert!(
                (block.center().y - row.center().y).abs() <= 1.0,
                "text block off centre: {block:?} in {row:?}"
            );
        }
    }

    /// The two marks on a row are the phone's way to the secret: no keyboard, no
    /// Ctrl+C. Each one takes its own half, and neither selects the entry.
    #[test]
    fn a_rows_marks_copy_the_username_and_the_password() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        *app.boxes.session.as_mut().unwrap().vault.logins[0].password = "s3cret".to_owned();
        let row_id = app.boxes.session.as_ref().unwrap().vault.logins[0]
            .id
            .clone();

        let shapes = frame(&ctx, &mut app, vec![]);
        let row = shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == theme::palette().panel && r.rect.width() == ROW => {
                    Some(r.rect)
                }
                _ => None,
            })
            .expect("the first password row");
        let mut marks: Vec<Pos2> = shapes
            .iter()
            .filter_map(|s| {
                let r = s.shape.visual_bounding_rect();
                (r.width() == 15.0
                    && r.height() == 15.0
                    && row.contains(r.center())
                    && r.center().x > row.center().x)
                    .then_some(r.center())
            })
            .collect();
        assert_eq!(marks.len(), 2, "a user mark and a password mark: {marks:?}");
        marks.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
        drop(shapes);

        let click = |app: &mut App, at: Pos2| {
            let events = |pressed| {
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]
            };
            let mut out = ctx.run_ui(raw(events(true)), |ui| main(ui, app));
            out.textures_delta.clear();
            let mut out = ctx.run_ui(raw(events(false)), |ui| main(ui, app));
            out.textures_delta.clear();
            copied(&out)
        };

        let out = click(&mut app, marks[0]);
        assert_eq!(out, vec!["octocat".to_owned()]);
        assert_eq!(
            app.shell.status.as_ref().unwrap().text,
            "username copied",
            "the status must name the half that went"
        );
        assert!(
            app.shell.clip.is_some(),
            "the countdown over the clipboard is armed"
        );
        assert!(
            app.boxes.session.as_ref().unwrap().selected.is_none(),
            "a mark copies, it does not open the entry"
        );

        let out = click(&mut app, marks[1]);
        assert_eq!(out, vec!["s3cret".to_owned()]);
        assert_eq!(app.shell.status.as_ref().unwrap().text, "password copied");

        // The row itself still selects away from the marks.
        click(&mut app, pos2(row.min.x + 40.0, row.center().y));
        assert_eq!(
            app.boxes.session.as_ref().unwrap().selected.as_deref(),
            Some(row_id.as_str())
        );
    }

    /// The folders view gathers the entries under generated folders, and offers
    /// the grouping picker.
    #[test]
    fn the_folders_view_gathers_the_entries_under_generated_folders() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        app.boxes.session.as_mut().unwrap().view = View::Folders;

        let shapes = frame(&ctx, &mut app, vec![]);
        let words = words(&shapes);
        assert!(
            words.iter().any(|w| w == "no site"),
            "a folder for the entries no URL claims: {words:?}"
        );
        assert!(
            words.iter().any(|w| w == "GitHub"),
            "the entries still list in the centre: {words:?}"
        );

        // The grouping picker is icon-only and lives on the sidebar's search
        // row, right of the field: a site and a user glyph above the entries.
        let field = ctx
            .read_response(crate::features::boxes::logic::search_id(Screen::Main))
            .expect("the search field is laid out")
            .rect;
        let mut picker: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r)
                    if r.brush.is_some()
                        && r.rect.min.x > field.max.x
                        && (r.rect.center().y - field.center().y).abs() < 1.0 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        picker.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
        assert_eq!(picker.len(), 2, "one icon per axis: {picker:?}");
        drop(shapes);

        // Right to left, so the user glyph is the rightmost one on the row.
        let user = picker
            .iter()
            .max_by(|a, b| a.min.x.total_cmp(&b.min.x))
            .copied()
            .unwrap();
        let press = |held: bool| {
            vec![
                egui::Event::PointerMoved(user.center()),
                egui::Event::PointerButton {
                    pos: user.center(),
                    button: egui::PointerButton::Primary,
                    pressed: held,
                    modifiers: Default::default(),
                },
            ]
        };
        frame(&ctx, &mut app, press(true));
        frame(&ctx, &mut app, press(false));
        assert!(
            app.boxes.session.as_ref().unwrap().axis == Axis::User,
            "the user glyph picks the user axis"
        );
    }

    /// On a phone, the folders view drills down through the generated folders
    /// instead of flattening every entry into one list.
    #[test]
    fn the_folders_view_drills_down_on_a_phone() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        app.boxes.session.as_mut().unwrap().view = View::Folders;
        app.boxes.session.as_mut().unwrap().axis = Axis::User;

        let shapes = shot::frame(&ctx, egui::vec2(420.0, 780.0), vec![], |ui| {
            main(ui, &mut app)
        });
        let words = words(&shapes);
        assert!(
            words.iter().any(|w| w == "octocat"),
            "the generated folder is on the phone: {words:?}"
        );
        assert!(
            !words.iter().any(|w| w == "GitHub"),
            "the phone drills down, it does not flatten: {words:?}"
        );
        fits_window(&shapes, 420.0);

        drop(shapes);
    }

    /// Entering a generated folder narrows the centre to the entries under it.
    #[test]
    fn entering_a_generated_folder_narrows_the_centre_to_its_entries() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        app.boxes.session.as_mut().unwrap().view = View::Folders;
        app.boxes.session.as_mut().unwrap().axis = Axis::User;

        let shapes = frame(&ctx, &mut app, vec![]);
        let node = center_of(&shapes, "octocat", 0.0).expect("the user folder in the tree");
        drop(shapes);

        let press = |held: bool| {
            vec![
                egui::Event::PointerMoved(node),
                egui::Event::PointerButton {
                    pos: node,
                    button: egui::PointerButton::Primary,
                    pressed: held,
                    modifiers: Default::default(),
                },
            ]
        };
        frame(&ctx, &mut app, press(true));
        let shapes = frame(&ctx, &mut app, press(false));
        let words = words(&shapes);

        assert_eq!(
            app.boxes.session.as_ref().unwrap().at,
            vec!["octocat".to_owned()],
            "the user folder was entered"
        );
        assert!(
            words.iter().any(|w| w == "GitHub"),
            "its own entry is in the centre: {words:?}"
        );
        assert!(
            !words.iter().any(|w| w == "Netflix"),
            "another user's entry is not: {words:?}"
        );
    }

    /// The folder on screen is the whole breadcrumb: there is no vault crumb, and
    /// clicking the folder's own name returns every entry to the centre.
    #[test]
    fn the_folder_on_screen_is_the_whole_breadcrumb() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        app.boxes.session.as_mut().unwrap().view = View::Folders;
        app.boxes.session.as_mut().unwrap().axis = Axis::User;

        let click = |app: &mut App, p: Pos2| {
            let button = |pressed| egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(&ctx, app, vec![egui::Event::PointerMoved(p)]);
            frame(&ctx, app, vec![button(true)]);
            frame(&ctx, app, vec![button(false)]);
        };

        let shapes = frame(&ctx, &mut app, vec![]);
        let drawn = words(&shapes);
        assert!(
            !drawn.iter().any(|w| w == "Vault"),
            "the vault crumb is gone: {drawn:?}"
        );

        let node = center_of(&shapes, "octocat", 0.0).expect("the user folder in the tree");
        drop(shapes);
        click(&mut app, node);
        assert_eq!(
            app.boxes.session.as_ref().unwrap().at,
            vec!["octocat".to_owned()],
            "the folder was entered"
        );

        let shapes = frame(&ctx, &mut app, vec![]);
        // Right of the tree, the folder's own name is the crumb; the entry row
        // under it carries the same username, lower down.
        let crumb = center_of(&shapes, "octocat", 320.0).expect("the folder's crumb");
        drop(shapes);
        click(&mut app, crumb);
        assert!(
            app.boxes.session.as_ref().unwrap().at.is_empty(),
            "the crumb returns to every entry"
        );
        let drawn = words(&frame(&ctx, &mut app, vec![]));
        assert!(
            drawn.iter().any(|w| w == "GitHub") && drawn.iter().any(|w| w == "Netflix"),
            "both entries are back in the centre: {drawn:?}"
        );
    }

    /// A group row is one compact line and wears the pane's row treatment, the
    /// same surface and hairline the entry rows use.
    #[test]
    fn a_group_row_is_one_compact_line_with_the_panes_hairline() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        app.boxes.session.as_mut().unwrap().view = View::Folders;
        app.boxes.session.as_mut().unwrap().axis = Axis::User;

        let shapes = frame(&ctx, &mut app, vec![]);
        let rows: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == theme::palette().panel && r.rect.width() == ROW => {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(rows.len(), 3, "one row per generated folder: {rows:?}");
        for row in &rows {
            assert_eq!(row.height(), 34.0, "one compact line per group");
        }
        for pair in rows.windows(2) {
            let pitch = pair[1].center().y - pair[0].center().y;
            assert!(
                (pitch - 40.0).abs() <= 1.0,
                "rows sit one compact line apart, plus the tree's own gap: {pitch}"
            );
        }

        let edges: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r)
                    if r.rect.max.x < 320.0
                        && r.stroke.color == theme::palette().hair
                        && r.stroke.width > 0.0 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            edges.len(),
            rows.len(),
            "every row wears the pane's hairline: {edges:?}"
        );
    }

    /// The drive's folder tree wears the same row treatment as the passwords'
    /// grouping rows: a 34 px card with the pane's hairline and the rounded
    /// corner.
    #[test]
    fn the_drive_tree_wears_the_passwords_group_row_treatment() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);

        let shapes = frame(&ctx, &mut app, vec![]);
        let rows: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r)
                    if r.fill == theme::palette().panel
                        && (r.rect.height() - 34.0).abs() < 0.5
                        && r.rect.max.x < DRIVE_X =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        assert!(!rows.is_empty(), "the drive tree paints rows: {rows:?}");
        for row in &rows {
            assert_eq!(row.height(), 34.0, "one compact line per folder: {row:?}");
        }
    }

    #[test]
    fn the_open_tab_gets_a_rule_and_clicking_the_other_switches_panes() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);

        let shapes = frame(&ctx, &mut app, vec![]);
        let rule: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == theme::ACCENT && r.rect.height() == 2.0 => Some(r.rect),
                _ => None,
            })
            .collect();
        let rule = *rule.first().expect("the open tab must carry a rule");
        assert_eq!(rule.max.y, 44.0, "the rule sits on the strip's bottom edge");
        assert_eq!(rule.min.x, 12.0, "the rule spans the first tab");
        assert!(
            rule.max.x < 140.0,
            "the rule must not span the second tab: {rule:?}"
        );

        let target = center_of(&shapes, "Files", 0.0).expect("second tab on screen");
        drop(shapes);
        let press = |held: bool| {
            vec![
                egui::Event::PointerMoved(target),
                egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Primary,
                    pressed: held,
                    modifiers: Default::default(),
                },
            ]
        };
        frame(&ctx, &mut app, press(true));
        frame(&ctx, &mut app, press(false));

        assert!(
            app.boxes.session.as_ref().unwrap().tab == Tab::Drive,
            "clicking the Files tab must open the drive"
        );
    }

    #[test]
    fn settings_is_a_page_of_the_box_with_its_own_button_on_the_strip() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        assert_eq!(app.screen, Screen::Main);

        let click = |app: &mut App, p: Pos2| {
            let button = |pressed| egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(&ctx, app, vec![egui::Event::PointerMoved(p), button(true)]);
            frame(&ctx, app, vec![button(false)]);
        };

        // egui resolves hover from the previous frame's widgets: warm up first
        frame(&ctx, &mut app, vec![]);
        let shapes = frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(pos2(980.0, BAR / 2.0))],
        );
        let rule = shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == theme::palette().faint && r.rect.max.y == BAR => {
                    Some(r.rect)
                }
                _ => None,
            })
            .expect("the lock marks its slot on hover");
        drop(shapes);
        let lock = Rect::from_min_max(pos2(rule.min.x, rule.max.y - BAR), rule.max);
        assert!(
            (1000.0 - lock.max.x - 12.0).abs() < 1.0,
            "the strip is inset the same at both ends: {lock:?}"
        );
        assert_eq!(lock.height(), BAR, "the slot spans the strip whole");

        let gear = pos2(lock.center().x - lock.width() - 12.0, BAR / 2.0);
        click(&mut app, gear);
        assert!(
            app.boxes.session.as_ref().unwrap().tab == Tab::Settings,
            "the gear opens the settings page"
        );
        assert_eq!(app.screen, Screen::Main, "the app screen is untouched");

        let shapes = frame_tall(&ctx, &mut app, vec![]);
        let drawn = words(&shapes);
        for want in ["Passwords", "Files", "MASTER PASSWORD"] {
            assert!(drawn.iter().any(|w| w == want), "missing {want}: {drawn:?}");
        }
        assert!(
            !drawn.iter().any(|w| w == "Settings"),
            "the gear must stay an icon, not a text tab: {drawn:?}"
        );
        assert!(
            shapes.iter().any(|s| matches!(&s.shape, Shape::Rect(r)
                if r.fill == theme::ACCENT
                    && r.rect.height() == 2.0
                    && (r.rect.center().x - gear.x).abs() < 30.0)),
            "the open settings page must light the gear's rule"
        );
        drop(shapes);

        click(&mut app, lock.center());
        assert!(app.boxes.session.is_none(), "the lock drops the box");
    }

    #[test]
    fn settings_heads_its_sections_with_the_apps_micro_label() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Settings, None);
        let shapes = frame_tall(&ctx, &mut app, vec![]);
        let drawn = words(&shapes);

        assert!(
            drawn.iter().any(|w| w == "MASTER PASSWORD"),
            "missing MASTER PASSWORD: {drawn:?}"
        );
        // The first card is the box itself, and a heading over it names nothing:
        // `BOX` must not come back.
        for old in ["BOX", "Box", "Master password", "Auto-lock"] {
            assert!(
                !drawn.iter().any(|w| w == old),
                "still Title Case: {drawn:?}"
            );
        }
        assert!(
            !shapes.iter().any(|s| matches!(&s.shape, Shape::Text(t)
                if t.galley.job.sections.iter().any(|sec| sec.format.color == theme::ACCENT))),
            "no heading in the accent colour: {drawn:?}"
        );
        drop(shapes);
    }

    /// Every form page of the app hangs its column on the pane's centre line, at
    /// the generator's measure. Settings was pinned to the left edge, leaving the
    /// room to its right empty.
    #[test]
    fn the_settings_column_hangs_on_the_centre() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Settings, None);
        let shapes = frame_tall(&ctx, &mut app, vec![]);

        let cards = settings_cards(&shapes);
        assert!(!cards.is_empty(), "the settings cards are painted");
        for card in &cards {
            assert!(
                (card.width() - theme::COLUMN).abs() < 0.5,
                "the card is not the column's width: {card:?}"
            );
            assert!(
                (card.center().x - 490.0).abs() < 1.0,
                "the card hangs off the pane's centre line: {card:?}"
            );
        }
    }

    /// A long file path used to widen the first card past the column: the card
    /// grew and hung off the centre line. The row elides instead.
    #[test]
    fn a_long_file_path_keeps_the_column_centred() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Settings, None);
        app.boxes.session.as_mut().unwrap().path = PathBuf::from(
            "/home/master/.local/share/pandora/sources/github/salvadorruiz/pandora-private/personal.pandora",
        );
        let shapes = frame_tall(&ctx, &mut app, vec![]);

        let cards = settings_cards(&shapes);
        assert!(!cards.is_empty(), "the settings cards are painted");
        for card in &cards {
            assert!(
                (card.width() - theme::COLUMN).abs() < 0.5,
                "a long path widened the card: {card:?}"
            );
            assert!(
                (card.center().x - 490.0).abs() < 1.0,
                "the card hangs off the pane's centre line: {card:?}"
            );
        }
    }

    /// The cards of the settings page: the panels it paints below the strip, the
    /// strip itself being a panel too.
    fn settings_cards(shapes: &[egui::epaint::ClippedShape]) -> Vec<Rect> {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == theme::palette().panel && r.rect.min.y > BAR => {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect()
    }

    /// The window floor is 380pt (`main.rs`): a card's buttons take the card's
    /// width there, never half a row with their label cut, and nothing paints
    /// past the window.
    #[test]
    fn the_settings_page_folds_into_a_narrow_window() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Settings, None);
        let shapes = shot::frame(&ctx, egui::vec2(380.0, 1400.0), Vec::new(), |ui| {
            main(ui, &mut app)
        });

        let buttons: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.rect.height() == 40.0 => Some(r.rect),
                _ => None,
            })
            .collect();
        assert!(
            buttons.len() >= 3,
            "the page's buttons are painted: {buttons:?}"
        );
        // The claim is the relation, not a number: a button takes the card's own
        // content width — its 20 px of padding a side and the hairline inside it
        // — never half a row. The strip above the cards is PANEL too, so the card
        // is the first one below it.
        let card = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == theme::palette().panel && r.rect.min.y > BAR => {
                    Some(r.rect)
                }
                _ => None,
            })
            .next()
            .expect("the settings cards are painted");
        for button in &buttons {
            assert!(
                button.width() >= card.width() - 44.0,
                "a button took half a row instead of the card: {button:?} in {card:?}"
            );
        }
        fits_window(&shapes, 380.0);
    }

    /// The strip is the wide window's: a narrow one hands the panes — and the
    /// strip's own inset — to the app's floor, so nothing of it is left up here.
    #[test]
    fn the_pane_strip_is_the_wide_windows() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);

        let shapes = shot::frame(&ctx, vec2(380.0, 520.0), Vec::new(), |ui| {
            main(ui, &mut app)
        });
        let drawn = words(&shapes);
        for gone in ["Passwords", "Files", "Codes"] {
            assert!(
                !drawn.iter().any(|w| w == gone),
                "the narrow window kept a strip it has no room for: {drawn:?}"
            );
        }
        fits_window(&shapes, 380.0);

        // A window with the room keeps its strip: the floor is the window's width
        // talking, not a label that went missing.
        let shapes = shot::frame(&ctx, vec2(1000.0, 700.0), Vec::new(), |ui| {
            main(ui, &mut app)
        });
        let drawn = words(&shapes);
        for shown in ["Passwords", "Files", "Codes"] {
            assert!(
                drawn.iter().any(|w| w == shown),
                "missing {shown}: {drawn:?}"
            );
        }
    }

    /// An error banner carries a message and the way out of it. The window floor
    /// is 380pt (`main.rs`): the message keeps its whole text and the buttons stay
    /// on screen, so neither runs past the edge.
    #[test]
    fn a_banner_folds_into_a_narrow_window() {
        for (conflict, behind) in [(true, false), (false, true)] {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut app = demo(&ctx, Tab::Passwords, None);
            {
                let s = app.boxes.session.as_mut().unwrap();
                s.conflict = conflict;
                s.behind = behind;
            }
            let want = if conflict {
                "changed on disk"
            } else {
                "newer version of this box"
            };
            for w in [380.0, 1000.0] {
                let shapes = shot::frame(&ctx, vec2(w, 520.0), Vec::new(), |ui| main(ui, &mut app));
                let drawn = words(&shapes);
                assert!(
                    drawn.iter().any(|word| word.contains(want)),
                    "the banner's message is up: {drawn:?}"
                );
                assert!(
                    drawn
                        .iter()
                        .any(|word| word.starts_with("Get the newer version")
                            || word.starts_with("Reload theirs")),
                    "the way out is up: {drawn:?}"
                );
                fits_window(&shapes, w);
            }
        }
    }

    /// The same floor, for the codes row: the code keeps the row's end, so the
    /// name has to elide into what is left. Laid out the other way round, the tail
    /// was handed the remainder and painted back over the name.
    #[test]
    fn a_code_row_elides_its_name_at_the_window_floor() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Codes, None);
        let name = "Banco Santander Empresa".to_owned();
        app.boxes.session.as_mut().unwrap().vault.otps[0].name = name.clone();

        let shapes = shot::frame(&ctx, vec2(380.0, 520.0), Vec::new(), |ui| {
            main(ui, &mut app)
        });
        let painted: Vec<(String, bool, Rect)> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.elided,
                    s.shape.visual_bounding_rect(),
                )),
                _ => None,
            })
            .collect();
        let (_, elided, at) = painted
            .iter()
            .find(|(text, _, _)| text == &name)
            .unwrap_or_else(|| panic!("the row's name is on screen: {painted:?}"));
        assert!(
            *elided,
            "the name ran under the code instead of eliding: {painted:?}"
        );
        let code = painted
            .iter()
            .find(|(text, _, _)| text.len() == 6 && text.bytes().all(|b| b.is_ascii_digit()))
            .expect("the live code is on screen")
            .2;
        assert!(
            at.max.x <= code.min.x,
            "the name and the code overlap: {at:?} {code:?}"
        );
        fits_window(&shapes, 380.0);

        // A seed that cannot be read wears a longer tail, and it has to fit too.
        app.boxes.session.as_mut().unwrap().vault.otps[0].secret =
            Zeroizing::new("otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&encoder=steam".to_owned());
        let shapes = shot::frame(&ctx, vec2(380.0, 520.0), Vec::new(), |ui| {
            main(ui, &mut app)
        });
        let drawn = words(&shapes);
        assert!(
            drawn.iter().any(|w| w.contains("Invalid TOTP")),
            "the broken seed still says so: {drawn:?}"
        );
        fits_window(&shapes, 380.0);
    }

    #[test]
    fn shoot() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, Some("bank"));
        shot(&ctx, "tabs-passwords", &mut app, 1000, 700, None);

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        app.boxes.session.as_mut().unwrap().view = View::Folders;
        shot(&ctx, "tabs-passwords-folders", &mut app, 1000, 700, None);

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        app.boxes.session.as_mut().unwrap().view = View::Folders;
        app.boxes.session.as_mut().unwrap().axis = Axis::User;
        shot(&ctx, "tabs-passwords-compact", &mut app, 420, 780, None);

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        shot(&ctx, "tabs-drive", &mut app, 1000, 700, None);

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, Some("passport"));
        app.boxes.session.as_mut().unwrap().editing_entry = Some("passport".to_owned());
        shot(&ctx, "tabs-drive-folder", &mut app, 1000, 700, None);

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        app.boxes.session.as_mut().unwrap().selected = Some("tax".to_owned());
        app.boxes.session.as_mut().unwrap().editing_folder = Some("tax".to_owned());
        shot(&ctx, "tabs-drive-folder-editor", &mut app, 1000, 700, None);

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        app.boxes.session.as_mut().unwrap().selected = Some("tax".to_owned());
        shot(
            &ctx,
            "tabs-drive-folder-selected",
            &mut app,
            1000,
            700,
            None,
        );

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        shot(&ctx, "tabs-drive-compact", &mut app, 420, 780, None);

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Codes, None);
        if let Some(s) = app.boxes.session.as_mut() {
            s.vault.otps[0].name = "Banco Santander Empresa".to_owned();
        }
        shot(&ctx, "tabs-codes", &mut app, 1000, 700, None);

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Codes, None);
        if let Some(s) = app.boxes.session.as_mut() {
            s.vault.otps[0].name = "Banco Santander Empresa".to_owned();
        }
        shot(&ctx, "tabs-codes-compact", &mut app, 380, 700, None);

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Settings, None);
        shot(&ctx, "tabs-settings", &mut app, 1000, 700, None);

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Settings, None);
        shot(&ctx, "tabs-settings-compact", &mut app, 420, 780, None);

        // The app's own settings, not the box's: no box is open behind the dialog.
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.prefs_open = true;
        shot::png(&ctx, "app-settings", 420, 520, None, |ui| {
            let ctx = ui.ctx().clone();
            crate::features::settings::ui::general(&ctx, &mut app);
        });

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        let shapes = frame(&ctx, &mut app, vec![]);
        let dragging = center_of(&shapes, "Factura luz", 0.0);
        let target = center_of(&shapes, "Tax 2026", DRIVE_X);
        drop(shapes);
        if let (Some(from), Some(to)) = (dragging, target) {
            shot(
                &ctx,
                "tabs-drive-drag",
                &mut app,
                1000,
                700,
                Some((from, to)),
            );
        }

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        app.boxes.session.as_mut().unwrap().show_new = true;
        shot(&ctx, "new-entry", &mut app, 720, 480, None);

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, Some("github"));
        let id = app.boxes.session.as_ref().unwrap().vault.logins[0]
            .id
            .clone();
        app.generate.entry = Some(id);
        shot(&ctx, "generate-modal", &mut app, 720, 520, None);
    }

    /// The entry's dice opens the generator's conditions over the pane it sits
    /// in, on the recipe the tool page edits.
    #[test]
    fn the_entry_generator_opens_over_the_passwords_pane() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, Some("github"));
        let id = app.boxes.session.as_ref().unwrap().vault.logins[0]
            .id
            .clone();
        app.generate.entry = Some(id);

        let mut drawn = Vec::new();
        for _ in 0..3 {
            drawn = words(&frame(&ctx, &mut app, vec![]));
        }
        for want in ["GENERATE", "A-Z", "Generate"] {
            assert!(drawn.iter().any(|w| w == want), "missing {want}: {drawn:?}");
        }
    }

    /// The drive's `+` is the one that asks: that pane holds notes and files, so
    /// the choice is the user's — and the file answer is the picker, never a note
    /// with a document glued to it.
    #[test]
    fn the_new_button_asks_the_drive_for_a_note_or_a_file() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);

        let mut app = demo(&ctx, Tab::Drive, None);
        app.boxes.session.as_mut().unwrap().show_new = true;
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame(&ctx, &mut app, vec![]);
        }
        let said = words(&shapes);
        assert!(
            app.boxes.session.as_ref().unwrap().show_new,
            "the chooser is open"
        );
        assert_eq!(
            app.boxes.session.as_ref().unwrap().vault.drive.len(),
            3,
            "and nothing is created until it is answered"
        );
        for row in ["NEW NOTE OR FILE", "Note", "File"] {
            assert!(said.iter().any(|w| w == row), "no {row}: {said:?}");
        }

        // The note answer lands in the folder being browsed.
        let note_btn = center_of(&shapes, "Note", 0.0).expect("the note row");
        drop(shapes);
        let click = |at: Pos2, pressed: bool| {
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]
        };
        frame(&ctx, &mut app, click(note_btn, true));
        frame(&ctx, &mut app, click(note_btn, false));

        let s = app.boxes.session.as_ref().unwrap();
        assert!(!s.show_new, "the answer closes it");
        assert_eq!(s.vault.drive.len(), 4);
        let last = s.vault.drive.last().unwrap();
        assert!(matches!(last, Entry::Note(_)), "the note answer is a note");
        assert_eq!(last.folder(), Some("docs"), "in the folder being browsed");
        assert_eq!(s.vault.logins.len(), 3, "and nothing else grew");

        // The other panes hold one record each and create it straight away.
        let mut app = demo(&ctx, Tab::Passwords, None);
        app.boxes.session.as_mut().unwrap().show_new = true;
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![]);
        }
        let s = app.boxes.session.as_ref().unwrap();
        assert!(!s.show_new, "no chooser to answer");
        assert_eq!(
            s.vault.logins.len(),
            4,
            "a login lands in the passwords pane"
        );
        assert_eq!(s.vault.drive.len(), 3);

        let mut app = demo(&ctx, Tab::Codes, None);
        app.boxes.session.as_mut().unwrap().show_new = true;
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![]);
        }
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.otps.len(), 2, "and a code in the codes pane");
        assert_eq!(s.vault.logins.len(), 3);
    }

    #[test]
    fn delete_folder_moves_its_content_to_the_parent() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        let note = app.boxes.session.as_ref().unwrap().vault.drive[0]
            .id()
            .to_owned();

        app.boxes.delete_folder("docs");

        let s = app.boxes.session.as_ref().unwrap();
        assert!(s.vault.folders.iter().all(|f| f.id != "docs"));
        assert_eq!(s.vault.folders[0].parent, None, "sub-folder promoted");
        assert_eq!(s.vault.drive[0].folder(), None, "entry promoted");
        assert_eq!(s.folder, None, "browsing the deleted folder falls back");
        assert!(s.vault.has(&note));
        assert!(s.dirty);
    }

    #[test]
    fn new_folder_targets_the_selection_else_the_browsed_folder() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);

        let before = app.boxes.session.as_ref().unwrap().vault.folders.len();

        drive::new_folder(&mut app);
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(
            s.new_folder.as_ref().unwrap().parent.as_deref(),
            Some("docs"),
            "browsed folder"
        );
        assert_eq!(
            s.vault.folders.len(),
            before,
            "opening the create modal writes nothing"
        );

        app.boxes.session.as_mut().unwrap().new_folder = None;
        app.boxes.session.as_mut().unwrap().selected = Some("tax".to_owned());
        drive::new_folder(&mut app);
        assert_eq!(
            app.boxes
                .session
                .as_ref()
                .unwrap()
                .new_folder
                .as_ref()
                .unwrap()
                .parent
                .as_deref(),
            Some("tax"),
            "selection wins"
        );
    }

    #[test]
    fn moving_a_folder_into_its_own_child_is_refused() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);

        app.boxes.move_into(
            Drag::Folder("docs".to_owned()),
            Some("tax".to_owned()),
            &mut app.shell,
        );

        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.folders[0].parent, None);
        assert_eq!(s.vault.folders[1].parent.as_deref(), Some("docs"));
        assert!(!s.dirty, "a refused move must not touch the vault");
    }

    fn raw(events: Vec<egui::Event>) -> egui::RawInput {
        shot::input(vec2(1000.0, 700.0), events)
    }

    fn frame(
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        shot::frame(ctx, vec2(1000.0, 700.0), events, |ui| main(ui, app))
    }

    /// A frame tall enough for a whole settings page. A scroll area paints only
    /// what is on screen, so a heading below the fold is not a heading that went
    /// missing.
    fn frame_tall(
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        shot::frame(ctx, vec2(1000.0, 1400.0), events, |ui| main(ui, app))
    }

    #[test]
    fn clicking_a_card_opens_the_entry() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        let codes = app.boxes.session.as_ref().unwrap().vault.drive[0]
            .id()
            .to_owned();

        let shapes = frame(&ctx, &mut app, vec![]);
        let card = center_of(&shapes, "Recovery codes", 0.0).expect("card on screen");
        drop(shapes);

        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(card),
                egui::Event::PointerButton {
                    pos: card,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: card,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );

        assert_eq!(
            app.boxes.session.as_ref().unwrap().selected.as_deref(),
            Some(codes.as_str())
        );
        assert_eq!(
            app.boxes.session.as_ref().unwrap().editing_entry.as_deref(),
            Some(codes.as_str()),
            "a single click must open the entry, not just select it"
        );
    }

    #[test]
    fn drive_entry_opens_in_a_modal_and_escape_closes_it() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, Some("codes"));
        app.boxes.session.as_mut().unwrap().editing_entry = Some("codes".to_owned());

        let mut drawn = Vec::new();
        for _ in 0..3 {
            let shapes = frame(&ctx, &mut app, vec![]);
            drawn = words(&shapes);
        }
        assert!(
            drawn.iter().any(|w| w == "Recovery codes"),
            "modal missing: {drawn:?}"
        );

        frame(
            &ctx,
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(
            app.boxes.session.as_ref().unwrap().selected.is_none(),
            "Esc must close the modal"
        );
    }

    /// A modal lives in `Order::Foreground` and blocks every layer below it.
    #[test]
    fn the_window_bar_stays_draggable_while_a_modal_is_open() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, Some("codes"));
        app.boxes.session.as_mut().unwrap().editing_entry = Some("codes".to_owned());

        // settle: the modal is only top from the frame after it opened, and the
        // bar only becomes interactable after its sizing pass.
        for _ in 0..2 {
            let mut out = ctx.run_ui(raw(vec![]), |ui| app.run_ui(ui));
            out.textures_delta.clear();
        }

        let bar = pos2(500.0, 15.0);
        let mut out = ctx.run_ui(
            raw(vec![
                egui::Event::PointerMoved(bar),
                egui::Event::PointerButton {
                    pos: bar,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]),
            |ui| app.run_ui(ui),
        );
        out.textures_delta.clear();
        // egui only decides the drag past `InputOptions::max_click_dist`.
        let mut out = ctx.run_ui(
            raw(vec![egui::Event::PointerMoved(pos2(bar.x + 12.0, bar.y))]),
            |ui| app.run_ui(ui),
        );
        out.textures_delta.clear();
        assert!(
            out.viewport_output
                .values()
                .any(|v| v.commands.contains(&egui::ViewportCommand::StartDrag)),
            "a drag on the bar must start the window drag with a modal open"
        );
        assert!(
            app.boxes.session.as_ref().unwrap().selected.is_some(),
            "the press must not fall through to the modal backdrop"
        );
    }

    /// The modal's backdrop covers the whole window, the outer border included:
    /// the resize strips ride the chrome layer above it, so the edge still resizes.
    #[cfg(not(any(target_os = "android", target_os = "macos")))]
    #[test]
    fn the_window_edge_still_resizes_while_a_modal_is_open() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, Some("codes"));
        app.boxes.session.as_mut().unwrap().editing_entry = Some("codes".to_owned());

        for _ in 0..2 {
            let mut out = ctx.run_ui(raw(vec![]), |ui| app.run_ui(ui));
            out.textures_delta.clear();
        }

        let edge = pos2(2.0, 400.0);
        let mut out = ctx.run_ui(
            raw(vec![
                egui::Event::PointerMoved(edge),
                egui::Event::PointerButton {
                    pos: edge,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]),
            |ui| app.run_ui(ui),
        );
        out.textures_delta.clear();
        assert!(
            out.viewport_output.values().any(|v| v.commands.contains(
                &egui::ViewportCommand::BeginResize(egui::ResizeDirection::West)
            )),
            "a drag on the window edge must resize with a modal open"
        );
        assert!(
            app.boxes.session.as_ref().unwrap().selected.is_some(),
            "the press must not fall through to the modal backdrop"
        );
    }

    /// The top-left corner is where the drag region and the resize strip meet:
    /// one press must start one native gesture, never both at once.
    #[cfg(not(any(target_os = "android", target_os = "macos")))]
    #[test]
    fn the_top_left_corner_starts_one_native_gesture() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);

        for _ in 0..2 {
            let mut out = ctx.run_ui(raw(vec![]), |ui| app.run_ui(ui));
            out.textures_delta.clear();
        }

        let corner = pos2(9.0, 13.0);
        let mut out = ctx.run_ui(
            raw(vec![
                egui::Event::PointerMoved(corner),
                egui::Event::PointerButton {
                    pos: corner,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]),
            |ui| app.run_ui(ui),
        );
        out.textures_delta.clear();
        let commands: Vec<&egui::ViewportCommand> = out
            .viewport_output
            .values()
            .flat_map(|v| v.commands.iter())
            .collect();
        // egui only decides a drag past `InputOptions::max_click_dist`; one more
        // frame catches a gesture that starts on the move instead of the press.
        let out = ctx.run_ui(
            raw(vec![egui::Event::PointerMoved(pos2(
                corner.x + 20.0,
                corner.y + 12.0,
            ))]),
            |ui| app.run_ui(ui),
        );
        let later: Vec<&egui::ViewportCommand> = out
            .viewport_output
            .values()
            .flat_map(|v| v.commands.iter())
            .collect();
        let commands: Vec<&egui::ViewportCommand> = commands.into_iter().chain(later).collect();
        let native = commands
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    egui::ViewportCommand::StartDrag | egui::ViewportCommand::BeginResize(_)
                )
            })
            .count();
        assert_eq!(
            native, 1,
            "the corner must start exactly one native gesture: {commands:?}"
        );
    }

    /// The compositor eats the release of a native move, and egui hovers nothing
    /// again until the next click: the bar must hand the pointer back when the
    /// native op takes over, and it must not take over on a bare press.
    #[test]
    fn a_bar_drag_does_not_leave_the_pointer_stuck_down() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);

        for _ in 0..2 {
            let mut out = ctx.run_ui(raw(vec![]), |ui| app.run_ui(ui));
            out.textures_delta.clear();
        }

        let bar = pos2(500.0, 15.0);
        let mut out = ctx.run_ui(
            raw(vec![
                egui::Event::PointerMoved(bar),
                egui::Event::PointerButton {
                    pos: bar,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]),
            |ui| app.run_ui(ui),
        );
        out.textures_delta.clear();
        assert!(
            !out.viewport_output
                .values()
                .any(|v| v.commands.contains(&egui::ViewportCommand::StartDrag)),
            "pressing the bar must not move the window"
        );

        let mut out = ctx.run_ui(
            raw(vec![egui::Event::PointerMoved(pos2(bar.x + 12.0, bar.y))]),
            |ui| app.run_ui(ui),
        );
        out.textures_delta.clear();
        assert!(
            out.viewport_output
                .values()
                .any(|v| v.commands.contains(&egui::ViewportCommand::StartDrag)),
            "moving on the bar must move the window"
        );
        assert!(
            !ctx.input(|i| i.pointer.any_down()),
            "the native move owns the gesture: nothing may stay pressed"
        );
    }

    #[test]
    fn deleting_an_entry_opens_the_confirm_modal() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, Some("github"));
        let id = app.boxes.session.as_ref().unwrap().vault.logins[0]
            .id
            .clone();
        app.boxes.session.as_mut().unwrap().confirm = Some(crate::app::Confirm::Entry(id));

        let mut drawn = Vec::new();
        for _ in 0..3 {
            drawn = words(&frame(&ctx, &mut app, vec![]));
        }
        assert!(
            drawn.iter().any(|w| w == "Delete?"),
            "confirm modal missing: {drawn:?}"
        );
        assert!(drawn.iter().any(|w| w == "Cancel"), "{drawn:?}");
    }

    #[test]
    fn the_folder_editor_opens_as_a_modal_and_escape_closes_it() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        ctx.enable_accesskit();
        let mut app = demo(&ctx, Tab::Drive, None);
        app.boxes.session.as_mut().unwrap().editing_folder = Some("tax".to_owned());

        let mut drawn = Vec::new();
        let mut named = Vec::new();
        for _ in 0..3 {
            let mut out = ctx.run_ui(raw(vec![]), |ui| main(ui, &mut app));
            drawn = words(&out.shapes);
            if let Some(update) = &out.platform_output.accesskit_update {
                for (_, node) in &update.nodes {
                    if node.role() == egui::accesskit::Role::Button {
                        if let Some(name) = node.label() {
                            named.push(name.to_owned());
                        }
                    }
                }
            }
            out.textures_delta.clear();
        }
        assert!(
            drawn.iter().any(|w| w == "Tax 2026"),
            "folder modal missing: {drawn:?}"
        );
        assert!(
            named.iter().any(|name| name == "Delete folder"),
            "the icon-only delete button needs an accessible name: {named:?}"
        );
        assert!(
            drawn.iter().any(|w| w == "Recovery codes"),
            "the grid must stay behind the modal, not be split: {drawn:?}"
        );

        frame(
            &ctx,
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(
            app.boxes.session.as_ref().unwrap().editing_folder.is_none(),
            "Esc must close the folder modal"
        );
    }

    /// Enter in the name field confirms the create: the folder reaches the vault
    /// then, and the dialog itself never writes one.
    #[test]
    fn enter_confirms_the_new_folder() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);

        let before = app.boxes.session.as_ref().unwrap().vault.folders.len();
        drive::new_folder(&mut app);
        assert!(
            app.boxes.session.as_ref().unwrap().new_folder.is_some(),
            "the create modal is pending"
        );
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![]);
        }
        assert_eq!(
            app.boxes.session.as_ref().unwrap().vault.folders.len(),
            before,
            "the dialog itself must not create the folder"
        );
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        let s = app.boxes.session.as_ref().unwrap();
        assert!(s.new_folder.is_none(), "Enter must confirm the folder");
        assert_eq!(
            s.vault.folders.len(),
            before + 1,
            "the confirmed folder reaches the vault"
        );
    }

    /// Cancel discards a folder the create modal was naming: nothing reaches the
    /// vault, and the pending state is dropped.
    #[test]
    fn cancelling_the_new_folder_writes_nothing() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        let before = app.boxes.session.as_ref().unwrap().vault.folders.len();

        drive::new_folder(&mut app);
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![]);
        }
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        let s = app.boxes.session.as_ref().unwrap();
        assert!(
            s.new_folder.is_none(),
            "Esc must discard the pending folder"
        );
        assert_eq!(s.vault.folders.len(), before, "nothing was created");
    }

    #[test]
    fn clicking_a_folder_card_enters_it() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);

        let shapes = frame(&ctx, &mut app, vec![]);
        let card = center_of(&shapes, "Tax 2026", DRIVE_X).expect("folder card on screen");
        drop(shapes);

        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(card),
                egui::Event::PointerButton {
                    pos: card,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        let mut drawn = Vec::new();
        for _ in 0..2 {
            let shapes = frame(
                &ctx,
                &mut app,
                vec![egui::Event::PointerButton {
                    pos: card,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            );
            drawn = words(&shapes);
        }

        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(
            s.folder.as_deref(),
            Some("tax"),
            "the click enters the folder"
        );
        assert!(
            s.editing_folder.is_none(),
            "a click must not open the editor"
        );
        assert!(
            !drawn.iter().any(|w| w == "Delete folder"),
            "no editor modal: {drawn:?}"
        );
    }

    fn right_click(
        ctx: &egui::Context,
        app: &mut App,
        pos: Pos2,
    ) -> Vec<egui::epaint::ClippedShape> {
        let press = |pressed: bool| {
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            }]
        };
        frame(ctx, app, vec![egui::Event::PointerMoved(pos)]);
        frame(ctx, app, press(true));
        frame(ctx, app, press(false));
        frame(ctx, app, vec![])
    }

    #[test]
    fn a_folder_card_menu_offers_delete() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);

        let shapes = frame(&ctx, &mut app, vec![]);
        let card = center_of(&shapes, "Tax 2026", DRIVE_X).expect("folder card on screen");
        drop(shapes);

        let menu = right_click(&ctx, &mut app, card);
        let drawn = words(&menu);
        for label in [
            "Open",
            "Rename…",
            "New subfolder",
            "New note here",
            "Delete",
        ] {
            assert!(
                drawn.iter().any(|w| w == label),
                "the folder menu lacks {label}: {drawn:?}"
            );
        }

        // The rows stay packed: egui reads the layout spacing (roomy on purpose
        // for forms) when it sizes a submenu's trigger rect, so a menu that
        // inherits it opens the submenu from the row above.
        let open = center_of(&menu, "Open", 0.0).expect("the Open row");
        let rename = center_of(&menu, "Rename…", 0.0).expect("the Rename row");
        assert!(
            rename.y - open.y < 34.0,
            "the menu rows drifted {}pt apart",
            rename.y - open.y
        );

        let del = center_of(&menu, "Delete", 0.0).expect("the Delete row");
        drop(menu);
        let click = |pressed: bool| {
            vec![egui::Event::PointerButton {
                pos: del,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]
        };
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(del)]);
        frame(&ctx, &mut app, click(true));
        frame(&ctx, &mut app, click(false));

        assert_eq!(
            app.boxes.session.as_ref().unwrap().confirm,
            Some(crate::app::Confirm::Folder("tax".to_owned())),
            "the folder menu's Delete must ask first"
        );
    }

    #[test]
    fn an_entry_card_menu_offers_move_and_delete() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);

        let shapes = frame(&ctx, &mut app, vec![]);
        let card = center_of(&shapes, "Recovery codes", DRIVE_X).expect("card on screen");
        drop(shapes);

        let drawn = words(&right_click(&ctx, &mut app, card));
        for label in ["Open", "Move to…", "Delete"] {
            assert!(
                drawn.iter().any(|w| w == label),
                "the entry menu lacks {label}: {drawn:?}"
            );
        }
    }

    #[test]
    fn the_edit_button_opens_the_folder_editor() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);

        let shapes = frame(&ctx, &mut app, vec![]);
        let title = center_of(&shapes, "Tax 2026", DRIVE_X).expect("folder card on screen");
        let card = shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Rect(r)
                    if r.rect.width() == 152.0
                        && r.rect.height() == 96.0
                        && r.rect.contains(title) =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .expect("folder card rect");
        drop(shapes);

        let edit = pos2(card.max.x - 14.0, card.min.y + 14.0);
        let click = |held: bool| {
            vec![egui::Event::PointerButton {
                pos: edit,
                button: egui::PointerButton::Primary,
                pressed: held,
                modifiers: Default::default(),
            }]
        };
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(edit)]);
        frame(&ctx, &mut app, click(true));
        frame(&ctx, &mut app, click(false));

        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(
            s.editing_folder.as_deref(),
            Some("tax"),
            "the ✎ opens the folder editor"
        );
    }

    #[test]
    fn the_breadcrumb_edit_button_edits_the_browsed_folder() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None); // browsing "docs"

        let shapes = frame(&ctx, &mut app, vec![]);
        let crumb = center_of(&shapes, "Documents", DRIVE_X).expect("breadcrumb on screen");
        let btn = shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Rect(r)
                    if r.rect.width() == 28.0
                        && r.rect.height() == 28.0
                        && (r.rect.center().y - crumb.y).abs() < 6.0 =>
                {
                    Some(r.rect.center())
                }
                _ => None,
            })
            .expect("breadcrumb edit button");
        drop(shapes);

        let click = |held: bool| {
            vec![egui::Event::PointerButton {
                pos: btn,
                button: egui::PointerButton::Primary,
                pressed: held,
                modifiers: Default::default(),
            }]
        };
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(btn)]);
        frame(&ctx, &mut app, click(true));
        frame(&ctx, &mut app, click(false));

        assert_eq!(
            app.boxes
                .session
                .as_ref()
                .unwrap()
                .editing_folder
                .as_deref(),
            Some("docs"),
            "the breadcrumb ✎ edits the browsed folder"
        );
    }

    #[test]
    fn keyboard_copies_the_selected_secret() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, Some("github"));
        *app.boxes.session.as_mut().unwrap().vault.logins[0].password = "s3cret".to_owned();

        let out = press(&ctx, &mut app, egui::Key::C);
        assert_eq!(copied(&out), vec!["s3cret".to_owned()], "Ctrl+C");
        assert_eq!(app.shell.status.as_ref().unwrap().text, "password copied");

        let out = press(&ctx, &mut app, egui::Key::B);
        assert_eq!(copied(&out), vec!["octocat".to_owned()], "Ctrl+B");
        assert_eq!(app.shell.status.as_ref().unwrap().text, "username copied");
    }

    /// The desktop backend maps Ctrl+C to `Event::Copy`, never a `Key::C`
    /// (egui-winit `is_copy_command`), so the shortcut must listen to the command.
    #[test]
    fn ctrl_c_arrives_as_a_copy_command_not_a_key() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, Some("github"));
        *app.boxes.session.as_mut().unwrap().vault.logins[0].password = "s3cret".to_owned();

        let mut out = ctx.run_ui(raw(vec![egui::Event::Copy]), |ui| app.run_ui(ui));
        out.textures_delta.clear();
        assert_eq!(copied(&out), vec!["s3cret".to_owned()], "Ctrl+C");
        assert_eq!(app.shell.status.as_ref().unwrap().text, "password copied");
    }

    #[test]
    fn the_status_bar_never_moves_the_screen() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);

        let wordmark = |app: &mut App, ctx: &egui::Context| {
            let mut out = ctx.run_ui(raw(vec![]), |ui| app.run_ui(ui));
            out.textures_delta.clear();
            center_of(&out.shapes, "Save your secrets inside Pandora’s box", 0.0)
                .expect("the wordmark is on screen")
        };

        let _ = wordmark(&mut app, &ctx); // first frame of a fresh ctx: fonts unprimed
        let quiet = wordmark(&mut app, &ctx);
        app.shell.set_status("box created", StatusKind::Saved);
        let loud = wordmark(&mut app, &ctx);
        assert_eq!(quiet.y, loud.y, "the notice floats, it pushes nothing");
    }

    fn focus(ctx: &egui::Context, app: &mut App, focused: bool) {
        let input = egui::RawInput {
            focused,
            ..raw(vec![])
        };
        let mut out = ctx.run_ui(input, |ui| app.run_ui(ui));
        out.textures_delta.clear();
    }

    /// One frame of the whole app loop, so the shortcuts in `App::run_ui` run.
    fn press(ctx: &egui::Context, app: &mut App, key: egui::Key) -> egui::FullOutput {
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let event = egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let mut out = ctx.run_ui(
            raw(vec![egui::Event::ModifiersChanged(modifiers), event]),
            |ui| app.run_ui(ui),
        );
        out.textures_delta.clear();
        out
    }

    fn copied(out: &egui::FullOutput) -> Vec<String> {
        out.platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// An empty (masked) field has no selection of its own, so the entry shortcut
    /// must still win Ctrl+C; a live selection in the field keeps the key.
    #[test]
    fn copy_shortcuts_survive_a_focused_field() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, Some("github"));
        *app.boxes.session.as_mut().unwrap().vault.logins[0].password = "s3cret".to_owned();

        let out = with_focused_field(&ctx, &mut app, egui::Key::C, None);
        assert_eq!(
            copied(&out),
            vec!["s3cret".to_owned()],
            "Ctrl+C while typing"
        );

        let out = with_focused_field(&ctx, &mut app, egui::Key::B, None);
        assert_eq!(
            copied(&out),
            vec!["octocat".to_owned()],
            "Ctrl+B while typing"
        );

        let out = with_focused_field(&ctx, &mut app, egui::Key::C, Some((0, 5)));
        assert!(
            !copied(&out).contains(&"s3cret".to_owned()),
            "a live field selection keeps Ctrl+C"
        );
    }

    fn with_focused_field(
        ctx: &egui::Context,
        app: &mut App,
        key: egui::Key,
        select: Option<(usize, usize)>,
    ) -> egui::FullOutput {
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        let event = egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let mut text = "s3cret".to_owned();
        let mut out = ctx.run_ui(
            raw(vec![egui::Event::ModifiersChanged(modifiers), event]),
            |ui| {
                let id = egui::Id::new("focused-field");
                ui.add(egui::TextEdit::singleline(&mut text).id(id))
                    .request_focus();
                if let Some((from, to)) = select {
                    let mut state = egui::widgets::text_edit::TextEditState::load(ui.ctx(), id)
                        .unwrap_or_default();
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(from),
                            egui::text::CCursor::new(to),
                        )));
                    state.store(ui.ctx(), id);
                }
                app.shortcuts(ui.ctx());
            },
        );
        out.textures_delta.clear();
        out
    }

    #[test]
    fn drive_entry_modal_clamps_to_a_small_window() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, Some("codes"));
        {
            let s = app.boxes.session.as_mut().unwrap();
            s.editing_entry = Some("codes".to_owned());
            let sel = s.selected.clone().unwrap();
            let it = s.vault.drive.iter_mut().find(|i| i.id() == sel).unwrap();
            let Entry::Note(note) = it else {
                panic!("the fixture selects a note");
            };
            *note.body = "line\n".repeat(200);
        }

        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 300.0));
        let mut drawn = Vec::new();
        for _ in 0..4 {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| main(ui, &mut app),
            );
            out.textures_delta.clear();
            drawn = words(&out.shapes);
        }
        assert!(
            drawn.iter().any(|w| w == "Recovery codes"),
            "modal must still render when clamped: {drawn:?}"
        );
        assert!(
            app.boxes.session.as_ref().unwrap().selected.is_some(),
            "clamping must not dismiss the modal"
        );
    }

    fn item_named(app: &App, name: &str) -> String {
        app.boxes
            .session
            .as_ref()
            .unwrap()
            .vault
            .records()
            .find(|i| i.name() == name)
            .unwrap_or_else(|| panic!("no entry named {name}"))
            .id()
            .to_owned()
    }

    fn folder_of(app: &App, id: &str) -> Option<String> {
        app.boxes
            .session
            .as_ref()
            .unwrap()
            .vault
            .drive
            .iter()
            .find(|i| i.id() == id)
            .and_then(|i| i.folder().map(str::to_owned))
    }

    /// A drag reorders the pane the row belongs to. Routing every drag into one
    /// collection left the passwords list silently dead, and nothing said so.
    #[test]
    fn dragging_a_password_row_reorders_the_list() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);

        let shapes = frame(&ctx, &mut app, vec![]);
        let source = center_of(&shapes, "GitHub", 0.0).expect("the first row");
        // the lower half of the last row: the drag lands below it
        let target = center_of(&shapes, "Netflix", 0.0).expect("a row below it") + vec2(0.0, 20.0);
        drop(shapes);

        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(source),
                egui::Event::PointerButton {
                    pos: source,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(target)]);
        assert!(
            egui::DragAndDrop::has_payload_of_type::<Drag>(&ctx),
            "the row must publish its drag payload"
        );
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        frame(&ctx, &mut app, vec![]);

        let names: Vec<&str> = app
            .boxes
            .session
            .as_ref()
            .unwrap()
            .vault
            .logins
            .iter()
            .map(|l| l.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["Banco Santander", "Netflix", "GitHub"],
            "the dragged login lands below Netflix"
        );
    }

    #[test]
    fn dragging_an_entry_card_onto_a_folder_card_moves_it() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        let moved = item_named(&app, "Factura luz");

        let shapes = frame(&ctx, &mut app, vec![]);
        // the folder card, not the same-named row in the tree
        let target = center_of(&shapes, "Tax 2026", DRIVE_X).expect("folder card on screen");
        let source = center_of(&shapes, "Factura luz", 0.0).expect("entry card on screen");
        drop(shapes);

        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(source),
                egui::Event::PointerButton {
                    pos: source,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(target)]);
        assert!(
            egui::DragAndDrop::has_payload_of_type::<Drag>(&ctx),
            "the card must publish its drag payload"
        );
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );

        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(
            folder_of(&app, &moved).as_deref(),
            Some("tax"),
            "the entry must land in the folder card it was released on"
        );
        assert!(s.dirty);
    }

    #[test]
    fn dropping_an_entry_on_the_tree_moves_it() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        let moved = item_named(&app, "Recovery codes");

        let shapes = frame(&ctx, &mut app, vec![]);
        let row = center_of(&shapes, "Tax 2026", 0.0).expect("tree row on screen");
        assert!(row.x < DRIVE_X, "that must be the tree row, not the card");
        drop(shapes);

        egui::DragAndDrop::set_payload(&ctx, Drag::Entry(moved.clone()));
        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(row),
                egui::Event::PointerButton {
                    pos: row,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );

        assert_eq!(folder_of(&app, &moved).as_deref(), Some("tax"));
    }

    /// The line that says where a dragged password lands is measured on the row's
    /// own rect. The row's click zone stops where the copy marks begin, and a line
    /// measured on it too ends a third of the job short — with no drop zone there.
    #[test]
    fn the_landing_line_covers_the_whole_row() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        let dragged = item_named(&app, "Netflix");

        let shapes = frame(&ctx, &mut app, vec![]);
        let rows: Vec<Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == theme::palette().panel && r.rect.width() == ROW => {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(rows.len(), 3, "one surface per password row");
        drop(shapes);

        // Carry one row, hover another: the line is the only thing saying where it lands.
        egui::DragAndDrop::set_payload(&ctx, Drag::Entry(dragged));
        let shapes = frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(rows[1].center())],
        );

        let line = shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::LineSegment { points, stroke } if stroke.color == theme::ACCENT => {
                    Some(*points)
                }
                _ => None,
            })
            .expect("the landing line is painted");
        assert_eq!(
            line[0].x, rows[1].min.x,
            "the line starts at the row's edge"
        );
        assert_eq!(line[1].x, rows[1].max.x, "and ends at the other one");
    }

    #[test]
    fn the_tree_has_no_row_for_the_vault() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);

        let shapes = frame(&ctx, &mut app, vec![]);
        let in_tree = shapes.iter().any(|s| match &s.shape {
            Shape::Text(t) => {
                t.galley.text() == "Personal" && s.shape.visual_bounding_rect().center().x < DRIVE_X
            }
            _ => false,
        });
        assert!(!in_tree, "the vault must not be a row of the tree");

        let parent = center_of(&shapes, "Documents", 0.0).expect("tree row on screen");
        let child = center_of(&shapes, "Tax 2026", 0.0).expect("nested tree row on screen");
        assert!(parent.x < DRIVE_X && child.x < DRIVE_X, "both are rows");
        assert!(
            child.x > parent.x,
            "a child folder stays indented under its parent: {parent:?} {child:?}"
        );
    }

    #[derive(Debug)]
    struct Fake(std::path::PathBuf, Vec<u8>);
    impl egui::DroppedFile for Fake {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            Ok(self.1.clone())
        }
    }

    fn drop_probe(ctx: &egui::Context, app: &mut App) {
        // A dropped file is read off its path on the desktop, so it has to be one
        // that is really there.
        let path = std::env::temp_dir().join("probe.txt");
        std::fs::write(&path, b"hi").unwrap();
        let mut input = raw(vec![]);
        input
            .dropped_files
            .push(std::sync::Arc::new(Fake(path, b"hi".to_vec())));
        let mut out = ctx.run_ui(input, |ui| app.run_ui(ui));
        out.textures_delta.clear();
    }

    /// While a file is being dragged over the window, the whole viewport is
    /// covered by a black drop overlay carrying the "Drop the file" message.
    #[test]
    fn a_dragged_file_paints_a_black_drop_overlay() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        let path = std::env::temp_dir().join("probe.txt");
        std::fs::write(&path, b"hi").unwrap();
        let mut input = raw(vec![]);
        input.hovered_files.push(egui::HoveredFile {
            path: Some(path.clone()),
            mime: String::new(),
        });
        let mut out = ctx.run_ui(input, |ui| app.run_ui(ui));
        out.textures_delta.clear();
        let black = shapes_black_rect(&out.shapes, ctx.viewport_rect());
        assert!(black, "no full-viewport black drop overlay painted");
        assert!(
            out.shapes.iter().any(|s| match &s.shape {
                Shape::Text(t) => t.galley.text() == "Drop the file",
                _ => false,
            }),
            "drop overlay missing its message"
        );
    }

    fn shapes_black_rect(shapes: &[egui::epaint::ClippedShape], rect: egui::Rect) -> bool {
        shapes.iter().any(|s| match &s.shape {
            Shape::Rect(r) => r.rect == rect && r.fill == theme::palette().scrim,
            _ => false,
        })
    }

    /// A dropped file is its own entry in the folder the selection names: a note
    /// never carries a document, so nothing is glued onto the selected entry.
    #[test]
    fn dropped_files_land_where_the_selection_says() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);

        let mut app = demo(&ctx, Tab::Drive, None);
        drop_probe(&ctx, &mut app);
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.drive.len(), 4, "one drop is one drive entry");
        assert_eq!(s.vault.drive.last().unwrap().folder(), Some("docs"));
        assert_eq!(s.vault.drive.last().unwrap().name(), "probe.txt");

        let mut app = demo(&ctx, Tab::Drive, None);
        app.boxes.session.as_mut().unwrap().selected = Some("tax".to_owned());
        drop_probe(&ctx, &mut app);
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.drive.len(), 4, "one drop is one drive entry");
        assert_eq!(s.vault.drive.last().unwrap().folder(), Some("tax"));

        // An entry is selected, but a document is never attached to one: the drop
        // lands in the folder being browsed, as the file it is.
        let mut app = demo(&ctx, Tab::Drive, Some("passport"));
        drop_probe(&ctx, &mut app);
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.drive.len(), 4, "the entry keeps its own document");
        let last = s.vault.drive.last().unwrap();
        assert!(
            matches!(last, Entry::File(_)),
            "the drop is a file, not a note: {last:?}"
        );
        assert_eq!(last.folder(), Some("docs"));
        assert_eq!(last.name(), "probe.txt");
    }

    /// Painted, not a `Button`, so `Visuals::interact_cursor` never reaches it:
    /// it must ask for the hand cursor itself.
    #[test]
    fn a_vault_tab_points_the_cursor() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);

        // The bar is an `Area`; its first frame is egui's invisible sizing pass.
        let mut settle = ctx.run_ui(raw(vec![]), |ui| {
            crate::features::shell::windowbar::show(ui, &mut app)
        });
        settle.textures_delta.clear();
        let mut out = ctx.run_ui(raw(vec![]), |ui| {
            crate::features::shell::windowbar::show(ui, &mut app)
        });
        out.textures_delta.clear();
        let tab = center_of(&out.shapes, "Personal", 0.0).expect("the vault tab is on the bar");

        let out = ctx.run_ui(raw(vec![egui::Event::PointerMoved(tab)]), |ui| {
            crate::features::shell::windowbar::show(ui, &mut app)
        });
        assert_eq!(
            out.platform_output.cursor_icon,
            egui::CursorIcon::PointingHand,
            "hovering the vault tab points the cursor"
        );
    }

    /// The row is dragged, not clicked, the moment the pointer is held and moved:
    /// the passwords list owes the closed hand, like the drive and the codes rows.
    #[test]
    fn a_dragged_row_grabs_the_cursor() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);

        let run = |app: &mut App, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(raw(events), |ui| main(ui, app));
            out.textures_delta.clear();
            out
        };
        let row =
            center_of(&run(&mut app, vec![]).shapes, "Netflix", 0.0).expect("the row on screen");

        run(&mut app, vec![egui::Event::PointerMoved(row)]);
        let held = |at: Pos2| {
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }]
        };
        run(&mut app, held(row));
        // Sideways, so the row stays under the pointer and keeps the drag.
        let out = run(
            &mut app,
            vec![egui::Event::PointerMoved(row + vec2(24.0, 0.0))],
        );
        assert_eq!(
            out.platform_output.cursor_icon,
            egui::CursorIcon::Grabbing,
            "a dragged row closes the hand"
        );
    }

    fn bare(ctx: &egui::Context, app: &mut App, key: egui::Key) -> Vec<egui::epaint::ClippedShape> {
        let event = egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        let mut out = ctx.run_ui(raw(vec![event]), |ui| app.run_ui(ui));
        out.textures_delta.clear();
        out.shapes
    }

    /// The accent ring `theme::focus` paints; nothing else draws a 2pt accent stroke.
    fn ring(shapes: &[egui::epaint::ClippedShape]) -> Option<Rect> {
        shapes.iter().find_map(|s| match &s.shape {
            Shape::Rect(r) if r.stroke.color == theme::ACCENT && r.stroke.width == 2.0 => {
                Some(r.rect)
            }
            _ => None,
        })
    }

    /// The drag region and resize strips answer the pointer only: `Sense::CLICK`
    /// and `Sense::DRAG` keep them out of the Tab order.
    #[test]
    fn the_tab_order_skips_the_invisible_widgets() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);

        let hidden: Vec<egui::Id> = std::iter::once(egui::Id::new("pandora-windowbar"))
            .chain((0..8).map(|i| egui::Id::new(("pandora-resize", i))))
            .collect();

        let mut landed = 0;
        for _ in 0..24 {
            bare(&ctx, &mut app, egui::Key::Tab);
            let Some(id) = ctx.memory(|m| m.focused()) else {
                continue;
            };
            assert!(!hidden.contains(&id), "Tab stopped on an invisible widget");
            landed += 1;
        }
        assert!(
            landed > 4,
            "Tab must reach real controls, it landed {landed}"
        );
    }

    #[test]
    fn the_focused_control_wears_a_ring() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);

        let mut ringed = false;
        for _ in 0..12 {
            let shapes = bare(&ctx, &mut app, egui::Key::Tab);
            if ring(&shapes).is_some() {
                ringed = true;
                break;
            }
        }
        assert!(ringed, "no control drew the focus ring");
    }

    #[test]
    fn enter_opens_the_focused_folder() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Drive, None);
        app.boxes.session.as_mut().unwrap().folder = None; // the tree row is on show

        // The tree's row, not the card of the same name in the grid: the card
        // sits in the centre pane, the tree in the left one.
        let row = |shapes: &[egui::epaint::ClippedShape]| -> Pos2 {
            shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    Shape::Text(t) if t.galley.text() == "Documents" => {
                        Some(s.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .min_by(|a, b| a.x.partial_cmp(&b.x).unwrap())
                .expect("the Documents row is on screen")
        };

        let mut reached = false;
        for _ in 0..24 {
            let shapes = bare(&ctx, &mut app, egui::Key::Tab);
            let here = row(&shapes);
            let Some(rect) = ring(&shapes) else { continue };
            if !rect.contains(here) {
                continue;
            }
            reached = true;
            bare(&ctx, &mut app, egui::Key::Enter);
            break;
        }
        assert!(reached, "Tab must reach the folder row");
        assert_eq!(
            app.boxes.session.as_ref().unwrap().folder.as_deref(),
            Some("docs"),
            "Enter on the focused folder opens it"
        );
    }

    #[test]
    fn ctrl_f_focuses_the_search_field() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);

        press(&ctx, &mut app, egui::Key::F);
        focus(&ctx, &mut app, true); // one more frame, so the field exists
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(crate::features::boxes::logic::search_id(Screen::Main)),
            "Ctrl+F must focus the pane's search"
        );
    }

    #[test]
    fn ctrl_k_opens_the_palette_and_escape_closes_it() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);

        press(&ctx, &mut app, egui::Key::K);
        assert!(app.find.open, "Ctrl+K opens the palette");
        // One focused frame, so the field exists and takes the keyboard.
        let mut out = ctx.run_ui(
            egui::RawInput {
                focused: true,
                ..raw(vec![])
            },
            |ui| app.run_ui(ui),
        );
        out.textures_delta.clear();
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(crate::features::find::logic::find_id()),
            "the palette's field holds the keyboard"
        );

        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        let mut out = ctx.run_ui(
            egui::RawInput {
                focused: true,
                ..raw(vec![escape])
            },
            |ui| app.run_ui(ui),
        );
        out.textures_delta.clear();
        assert!(!app.find.open, "Escape closes it");
    }

    /// A phone's window is the whole screen: the palette must not walk off it.
    #[test]
    fn the_palette_fits_a_phone_window() {
        for (w, h) in [(360.0, 640.0), (360.0, 420.0)] {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut app = demo(&ctx, Tab::Passwords, None);
            app.open_find();
            app.find.query = "GitHub".to_owned();
            for _ in 0..3 {
                let mut out = ctx.run_ui(shot::input(vec2(w, h), Vec::new()), |ui| app.run_ui(ui));
                out.textures_delta.clear();
            }

            let frame = ctx
                .memory(|m| m.area_rect(egui::Id::new("find")))
                .expect("the palette is up");
            let body = crate::shared::ui::chrome::body(&ctx);
            assert!(
                frame.min.x >= body.min.x - 0.5 && frame.max.x <= body.max.x + 0.5,
                "off the body at {w}x{h}: {frame:?} vs {body:?}"
            );
            assert!(
                frame.min.y >= body.min.y - 0.5 && frame.max.y <= body.max.y + 0.5,
                "off the body at {w}x{h}: {frame:?} vs {body:?}"
            );
        }
    }

    #[test]
    fn delete_asks_first_and_never_fires_while_typing() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, Some("github"));

        bare(&ctx, &mut app, egui::Key::Delete);
        let s = app.boxes.session.as_ref().unwrap();
        assert!(
            matches!(s.confirm, Some(crate::app::Confirm::Entry(_))),
            "Delete must ask first"
        );
        assert_eq!(
            s.vault.records().count(),
            7,
            "and must not delete on its own"
        );

        let mut app = demo(&ctx, Tab::Passwords, Some("github"));
        let mut text = "typing".to_owned();
        let mut out = ctx.run_ui(raw(vec![]), |ui| {
            let id = egui::Id::new("typing");
            ui.add(egui::TextEdit::singleline(&mut text).id(id))
                .request_focus();
            app.shortcuts(ui.ctx());
        });
        out.textures_delta.clear();
        assert!(
            app.boxes.session.as_ref().unwrap().confirm.is_none(),
            "a focused field keeps Delete"
        );
    }

    /// A modal eats Escape itself, so cancelling one never also drops the
    /// selection behind it.
    #[test]
    fn escape_drops_the_selection_only_when_nothing_else_wants_it() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, Some("github"));

        bare(&ctx, &mut app, egui::Key::Escape);
        assert!(
            app.boxes.session.as_ref().unwrap().selected.is_none(),
            "Esc drops the selection"
        );

        let mut app = demo(&ctx, Tab::Passwords, Some("github"));
        let id = app
            .boxes
            .session
            .as_ref()
            .unwrap()
            .selected
            .clone()
            .unwrap();
        app.boxes.session.as_mut().unwrap().confirm = Some(crate::app::Confirm::Entry(id.clone()));
        // A modal is only the top one from the frame after it opened, and only
        // the top one takes the key for itself.
        frame(&ctx, &mut app, vec![]);

        bare(&ctx, &mut app, egui::Key::Escape);
        let s = app.boxes.session.as_ref().unwrap();
        assert!(s.confirm.is_none(), "Esc cancels the prompt");
        assert_eq!(
            s.selected.as_deref(),
            Some(id.as_str()),
            "and keeps what was selected behind it"
        );
    }

    #[test]
    fn ctrl_tab_and_ctrl_w_walk_the_strip() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = demo(&ctx, Tab::Passwords, None);
        app.open("Work");
        assert_eq!(app.boxes.active, 1, "the box just opened is on screen");

        press(&ctx, &mut app, egui::Key::Tab);
        assert_eq!(app.boxes.active, 0, "Ctrl+Tab walks to the next box");

        press(&ctx, &mut app, egui::Key::W);
        assert!(app.boxes.tabs.is_empty(), "Ctrl+W closes the box on screen");
        assert_eq!(app.boxes.session.as_ref().unwrap().vault.name, "Work");
    }

    /// Hand-painted controls owe `resp.widget_info` a name for the screen reader.
    #[test]
    fn hand_painted_controls_announce_themselves() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        ctx.enable_accesskit();
        let mut app = demo(&ctx, Tab::Passwords, None);

        let mut named: Vec<String> = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(raw(vec![]), |ui| main(ui, &mut app));
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
            "GitHub, octocat",
            "New password",
            "copy username",
            "Passwords",
        ] {
            assert!(named.iter().any(|n| n == want), "missing {want}: {named:?}");
        }
    }
}
