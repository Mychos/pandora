//! The drive entry editor: a note's text, or a file's document, plus the folder
//! it sits in. Beside the grid, or over it as a modal.

use crate::app::{App, Confirm, Field, StatusKind};
use crate::features::drive::preview;

use crate::shared::theme;
use crate::shared::ui::entry as record;
use crate::shared::ui::responsive;
use crate::shared::ui::size;

use crate::vault::model::{now_secs, Entry, Kind};
use egui::{Layout, TextEdit, Ui};

/// The gap the entry keeps above the button that deletes it; with `responsive::target`
/// for the button itself, that is the strip the box holds at its foot.
const FOOT: f32 = 28.0;
/// However short the box is, the entry's own body still shows this much: about
/// three rows of text, or a thumbnail.
const LEAST: f32 = 56.0;

/// The share of the visible body the drive editor takes: a note (or a document)
/// is what the modal was opened for, so it grows and shrinks with the window
/// instead of living at one size. The floors keep a small window usable; the
/// ceiling keeps a wide one from stretching a single column across it.
const WIDTH: f32 = 0.62;
const HEIGHT: f32 = 0.8;
const FLOOR: f32 = 320.0;
const CEIL: f32 = 760.0;

pub fn show(ui: &mut Ui, app: &mut App) {
    let sel = app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.editing_entry.clone());
    let compact = responsive::compact(ui.ctx());
    record::pane(ui, sel, |ui, sel| {
        let (closed, _) = face(ui, app, sel, compact);
        if closed {
            if let Some(s) = app.boxes.session.as_mut() {
                s.editing_entry = None;
                s.selected = None;
            }
            app.preview.sync(None);
        }
        body(ui, app, sel, true);
    });
}

/// The selected drive entry as a floating modal; a backdrop click or Esc closes
/// it (`should_close`), which clears the selection.
pub fn modal(ctx: &egui::Context, app: &mut App) {
    let sel = app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.editing_entry.clone());
    let Some(sel) = sel else { return };
    if app
        .boxes
        .session
        .as_ref()
        .is_some_and(|s| s.vault.folders.iter().any(|f| f.id == sel))
    {
        return;
    }

    let screen = crate::shared::ui::chrome::body(ctx);
    let (width, height) = box_for(screen.size());

    let open = theme::dialog(ctx, egui::Id::new("drive-entry"), |ui| {
        theme::width(ui, width);
        theme::height(ui, height);
        let (closed, changed) = face(ui, app, &sel, true);
        if changed {
            if let Some(s) = app.boxes.session.as_mut() {
                s.mark_dirty();
            }
        }
        let rest = ui.available_height();
        // The entry measures itself against the room it is offered — a note's box,
        // a page's board — so a document with a board fills the box, while one with
        // nothing to lay out (an audio player, a hint) takes only its own height.
        theme::band(
            ui,
            egui::ScrollArea::vertical()
                .max_height(rest)
                .auto_shrink([false, true]),
            |ui| body(ui, app, &sel, false),
        );
        closed
    });

    if !open {
        if let Some(s) = app.boxes.session.as_mut() {
            s.editing_entry = None;
            s.selected = None;
        }
        app.preview.sync(None);
        app.boxes.flush(&mut app.shell);
    }
}

/// The box the editor asks for on a body of `screen`: its share of the window,
/// held inside the floors and the ceiling. What a window too small to hold it does
/// with it is the dialog frame's business: `theme::width` and `theme::height` are
/// what keep the card clear of the edge.
fn box_for(screen: egui::Vec2) -> (f32, f32) {
    (
        (screen.x * WIDTH).clamp(FLOOR, CEIL),
        (screen.y * HEIGHT).max(FLOOR),
    )
}

/// What the entry's own document may take: the box bar the foot, which is the gap
/// above the delete button, the button's own square, and the spacing egui puts
/// between the two. The spare gap keeps rounding from pushing the button off the
/// bottom of a box the entry was told to fill exactly.
fn room(ui: &Ui) -> f32 {
    let gap = ui.spacing().item_spacing.y;
    (ui.available_height() - (FOOT + responsive::target(ui.ctx()) + 2.0 * gap)).max(0.0)
}

/// The drive entry's face: its kind glyph and its name, with the "Move to…"
/// menu trailing them and the dialog's cross after them in the modal. Returns
/// `(closed, changed)`.
fn face(ui: &mut Ui, app: &mut App, sel: &str, close: bool) -> (bool, bool) {
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return (false, false);
    };
    let found = s.vault.drive.iter().find(|i| i.id() == sel);
    let glyph = match found {
        Some(Entry::Note(_)) => theme::kind_icon(Kind::Note),
        Some(Entry::File(file)) => preview::icon(preview::kind(ui.ctx(), &s.vault, file)),
        None => return (false, false),
    };
    let tint = theme::tag_color(found.map(|it| it.kind()).unwrap_or(Kind::File));
    let Some(it) = s.vault.drive.iter_mut().find(|i| i.id() == sel) else {
        return (false, false);
    };
    let name = match &mut *it {
        Entry::Note(note) => &mut note.name,
        Entry::File(file) => &mut file.name,
    };
    let mut move_requested = false;
    let (closed, changed, _) = theme::record_head(
        ui,
        icons,
        close,
        |ui| {
            if let Some(img) = icons.image(glyph, responsive::grow(ui.ctx(), 15.0), tint) {
                ui.add(img);
            }
        },
        name,
        "name",
        |ui| {
            if icons
                .button(
                    ui,
                    theme::Icon::Folder,
                    15.0,
                    theme::palette().muted,
                    "Move to…",
                )
                .clicked()
            {
                move_requested = true;
            }
        },
    );
    if move_requested {
        s.moving_entry = Some(sel.to_owned());
    }
    if changed {
        s.mark_dirty();
    }
    let closed = if close { closed } else { false };
    (closed, changed)
}

fn body(ui: &mut Ui, app: &mut App, sel: &str, fill: bool) {
    let is_file = app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.vault.drive.iter().find(|entry| entry.id() == sel))
        .is_some_and(|entry| matches!(entry, Entry::File(_)));
    if is_file {
        body_file(ui, app, sel, fill);
        return;
    }
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };
    let mut changed = false;
    let mut copied = false;
    let mut show_body = s.shown(sel, Field::Notes);
    let body_was = show_body;
    if s.vault
        .drive
        .iter()
        .any(|entry| matches!(entry, Entry::Note(note) if note.id == sel))
    {
        if let Err(e) = s.vault.ensure_note(sel) {
            app.shell.fail(e);
            return;
        }
    }
    let it = s.vault.drive.iter_mut().find(|i| i.id() == sel);
    let Some(it) = it else { return };

    match it {
        // A note is its text, and only its text: a document is its own entry
        // beside it, so there is nothing here to attach one to.
        Entry::Note(note) => {
            // Read out here: inside a horizontal row, the height left over is the
            // row's own, not the room the note is being given.
            let h = room(ui).max(LEAST);
            let (field, copy) = record::secret(
                ui,
                icons,
                &mut show_body,
                2,
                80.0,
                egui::Align::Min,
                |ui, w, shown| {
                    // The box belongs to the entry, not to the text: it is painted out
                    // here, outside the scroll, so a long note moves inside it while its
                    // edges — and the ring that says the field holds the keyboard — stay
                    // where they are. The field inside is frameless and only as tall as
                    // `min_size` asks, never `f32::INFINITY`, which would make the note a
                    // run with no end.
                    ui.allocate_ui_with_layout(
                        egui::vec2(w, h),
                        Layout::top_down(egui::Align::Min),
                        |ui| {
                            let rect = ui.max_rect();
                            let corner = ui.visuals().widgets.inactive.corner_radius;
                            ui.painter().rect_filled(
                                rect,
                                corner,
                                ui.visuals().text_edit_bg_color(),
                            );
                            let field = theme::band(
                                ui,
                                egui::ScrollArea::vertical()
                                    .max_height(h)
                                    .auto_shrink([false, false]),
                                |ui| {
                                    ui.add(
                                        TextEdit::multiline(&mut *note.body)
                                            .password(!shown)
                                            .hint_text(crate::shared::i18n::t("notes…"))
                                            .desired_width(f32::INFINITY)
                                            .frame(
                                                egui::Frame::new()
                                                    .inner_margin(egui::Margin::symmetric(10, 8)),
                                            )
                                            .min_size(egui::vec2(0.0, h)),
                                    )
                                },
                            )
                            .inner;
                            if field.has_focus() {
                                ui.painter().rect_stroke(
                                    rect,
                                    corner,
                                    ui.visuals().selection.stroke,
                                    egui::StrokeKind::Inside,
                                );
                            }
                            field
                        },
                    )
                    .inner
                },
            );
            changed |= field.changed();
            if copy {
                app.shell.copy_text(ui.ctx(), note.body.as_str());
                copied = true;
            }
        }
        // A file is its document: the name is the row above, how big it is sits
        // beside the label, and the only other thing to offer is the way out of
        // the box.
        Entry::File(_) => {}
    }

    let deleted = if fill {
        record::corner(ui, icons, FOOT)
    } else {
        record::delete(ui, icons, FOOT)
    };
    if deleted {
        s.confirm = Some(Confirm::Entry(sel.to_owned()));
    }

    if changed {
        if let Some(it) = s.vault.drive.iter_mut().find(|i| i.id() == sel) {
            it.touched(now_secs());
        }
        s.mark_dirty();
    }
    if show_body != body_was {
        s.show(sel, Field::Notes, show_body);
    }

    if copied {
        app.shell
            .set_status("copied to clipboard", StatusKind::Saved);
    }
}

fn body_file(ui: &mut Ui, app: &mut App, sel: &str, fill: bool) {
    let icons = &app.icons;
    let preview = &mut app.preview;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };
    let Some(file) = s.vault.drive.iter().find_map(|entry| match entry {
        Entry::File(file) if file.id == sel => Some(file),
        _ => None,
    }) else {
        return;
    };
    let mut export_this = false;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            if icons
                .button(
                    ui,
                    theme::Icon::Download,
                    14.0,
                    theme::palette().muted,
                    "save to disk",
                )
                .clicked()
            {
                export_this = true;
            }
            theme::hint(ui, &size::human(file.bytes()));
        });
    });
    ui.add_space(4.0);
    preview::show(ui, preview, file, &s.vault, room(ui).max(LEAST), icons);

    let deleted = if fill {
        record::corner(ui, icons, FOOT)
    } else {
        record::delete(ui, icons, FOOT)
    };
    if deleted {
        s.confirm = Some(Confirm::Entry(sel.to_owned()));
    }
    if export_this {
        s.confirm = Some(Confirm::File {
            id: sel.to_owned(),
            neutral: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::entries::ui::editor::tests::session;
    use crate::shared::ui::shot;
    use crate::vault::model::{File, Kind, Note, Vault};
    use egui::epaint::ClippedShape;
    use egui::Shape;
    use zeroize::Zeroizing;

    fn bundled(name: &str) -> File {
        File::new(
            name.to_owned(),
            include_bytes!("../../../../assets/icon.png").to_vec(),
            0,
        )
    }

    /// One drive entry painted through the editor twice, the second frame settled.
    fn painted(item: Entry) -> Vec<ClippedShape> {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let id = item.id().to_owned();
        let mut vault = Vault::new("Test");
        vault.drive = vec![item];
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session(vault, Some(id.clone())));
        app.boxes.session.as_mut().unwrap().editing_entry = Some(id.clone());

        let size = egui::vec2(1000.0, 700.0);
        shot::frame(&ctx, size, Vec::new(), |ui| {
            face(ui, &mut app, &id, false);
            body(ui, &mut app, &id, true);
        });
        shot::frame(&ctx, size, Vec::new(), |ui| {
            face(ui, &mut app, &id, false);
            body(ui, &mut app, &id, true);
        })
    }

    fn words(shapes: &[ClippedShape]) -> Vec<String> {
        shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Text(text) => Some(text.galley.text().to_string()),
                _ => None,
            })
            .collect()
    }

    /// A file is its document: the editor shows what it is called and how big it
    /// is, and offers no text area at all. The glyph says what the entry is, so the
    /// name takes the kind's place beside it instead of the kind being spelled out.
    #[test]
    fn a_file_shows_its_document_and_no_text() {
        let shapes = painted(Entry::File(bundled("report.pdf")));
        let said = words(&shapes);
        assert!(said.iter().any(|line| line == "report.pdf"), "{said:?}");
        assert!(
            said.iter().any(|line| line.ends_with("KB")),
            "the size travels with it: {said:?}"
        );
        assert!(
            said.iter().all(|line| !line.contains('\u{2022}')),
            "a file has no text body to mask: {said:?}"
        );
        let head = glyph(&shapes);
        let name = line_y(&shapes, |t| t == "report.pdf");
        assert!(
            (name - head.top()).abs() <= 8.0,
            "the name shares the header row with the glyph: {name} in {head:?}"
        );
    }

    /// A text document is readable in place: the preview paints its words and
    /// selects them, so no export is needed just to read it.
    #[test]
    fn a_text_file_shows_its_words() {
        let item = File::new(
            "notes.txt".to_owned(),
            b"the wifi is in the drawer".to_vec(),
            0,
        );
        let said = words(&painted(Entry::File(item)));
        assert!(
            said.iter().any(|line| line.contains("wifi")),
            "the preview paints the text: {said:?}"
        );
    }

    /// A note is its text, and nothing else: the body is painted masked, and the
    /// section that used to hold documents is gone.
    #[test]
    fn a_note_masks_its_body_and_holds_no_documents() {
        let mut item = Note::new(0);
        item.body = Zeroizing::new("the wifi password is in the drawer".to_owned());
        let shapes = painted(Entry::Note(item));
        let said = words(&shapes);
        assert!(
            said.iter().any(|line| line.contains('\u{2022}')),
            "the body is masked: {said:?}"
        );
        assert!(
            said.iter().all(|line| !line.contains("wifi password")),
            "and its words never reach the screen: {said:?}"
        );
        let head = glyph(&shapes);
        let name = line_y(&shapes, |t| t == "New note");
        assert!(
            (name - head.top()).abs() <= 8.0,
            "the name shares the header row with the glyph: {name} in {head:?}"
        );
        assert!(
            said.iter().all(|line| line != "FILE"),
            "with no document on offer: {said:?}"
        );
    }

    /// Only a file carries bytes, and the entry says which it is.
    #[test]
    fn the_kind_is_the_type_of_the_entry() {
        assert_eq!(Note::new(0).kind(), Kind::Note);
        assert_eq!(bundled("report.pdf").kind(), Kind::File);
    }

    /// The modal takes its share of the window instead of living at one size: a
    /// wide one gets a wide box, a cramped one still gets a usable one.
    #[test]
    fn the_modal_box_follows_the_window() {
        assert_eq!(
            box_for(egui::vec2(1400.0, 900.0)),
            (CEIL, 720.0),
            "a big window gets the ceiling width and its share of the height"
        );
        assert_eq!(
            box_for(egui::vec2(500.0, 400.0)),
            (FLOOR, 320.0),
            "a small one lands on the floors"
        );
        assert_eq!(
            box_for(egui::vec2(300.0, 200.0)),
            (FLOOR, FLOOR),
            "and never shrinks past them"
        );
    }

    /// A long note leaves the box where it was: the share of the body the modal
    /// takes is what holds its card off the window's edge.
    #[test]
    fn the_modal_keeps_clear_of_the_window_edge() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut item = Note::new(0);
        *item.body = "line\n".repeat(400);
        let item = Entry::Note(item);
        let id = item.id().to_owned();
        let mut vault = Vault::new("Test");
        vault.drive = vec![item];
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session(vault, Some(id.clone())));
        app.boxes.session.as_mut().unwrap().editing_entry = Some(id);

        let size = egui::vec2(1000.0, 700.0);
        let mut card = egui::Rect::NOTHING;
        for _ in 0..4 {
            shot::frame(&ctx, size, Vec::new(), |ui| {
                super::modal(ui.ctx(), &mut app)
            });
            card = ctx
                .memory(|m| m.area_rect(egui::Id::new("drive-entry")))
                .unwrap_or_else(|| panic!("the modal has an area"));
        }

        let body = crate::shared::ui::chrome::body(&ctx);
        let gap = crate::shared::ui::chrome::GAP;
        assert!(
            card.min.x >= body.min.x + gap
                && card.max.x <= body.max.x - gap
                && card.min.y >= body.min.y + gap
                && card.max.y <= body.max.y - gap,
            "the card sits flush: {card:?} in {body:?}"
        );
    }

    /// A document with nothing to lay out gets a modal the size of what it says: a
    /// hint is its own height, and the box no longer takes its share of the window
    /// for it.
    #[test]
    fn a_modal_with_nothing_to_lay_out_is_short() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let item = Entry::File(File::new("blob.bin".to_owned(), vec![0, 1, 2, 3], 0));
        let id = item.id().to_owned();
        let mut vault = Vault::new("Test");
        vault.drive = vec![item];
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session(vault, Some(id.clone())));
        app.boxes.session.as_mut().unwrap().editing_entry = Some(id);

        let size = egui::vec2(1000.0, 700.0);
        let mut card = egui::Rect::NOTHING;
        for _ in 0..4 {
            shot::frame(&ctx, size, Vec::new(), |ui| {
                super::modal(ui.ctx(), &mut app)
            });
            card = ctx
                .memory(|m| m.area_rect(egui::Id::new("drive-entry")))
                .unwrap_or_else(|| panic!("the modal has an area"));
        }

        let body = crate::shared::ui::chrome::body(&ctx);
        assert!(
            card.height() < body.height() * 0.5,
            "the modal took its share of the window for a hint: {card:?} in {body:?}"
        );
    }

    /// The mark the entry wears, at the head of the editor: a painted rect carries
    /// the tint as its fill.
    fn glyph(shapes: &[ClippedShape]) -> egui::Rect {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.brush.is_some() => Some(r.rect),
                _ => None,
            })
            .min_by(|a, b| a.min.y.partial_cmp(&b.min.y).unwrap())
            .expect("the entry wears a glyph")
    }

    /// Where the one line `hit` reads, top down.
    fn line_y(shapes: &[ClippedShape], hit: impl Fn(&str) -> bool) -> f32 {
        shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Text(text) if hit(text.galley.text()) => Some(text.pos.y),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no such line: {:?}", words(shapes)))
    }

    /// A long note scrolls inside its own box, not the entry around it: the field is
    /// as tall as its text and the box is the viewport, so the card holds still while
    /// the note moves under it. `min_size` is what keeps the note finite — with
    /// `f32::INFINITY` the note was a run with no end and the bar a hair.
    #[test]
    fn a_long_note_scrolls_inside_its_own_box() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut item = Note::new(0);
        *item.body = "line\n".repeat(400);
        let item = Entry::Note(item);
        let id = item.id().to_owned();
        let mut vault = Vault::new("Test");
        vault.drive = vec![item];
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session(vault, Some(id.clone())));
        app.boxes.session.as_mut().unwrap().editing_entry = Some(id.clone());
        // Read out, the way the eye reads it: masked, the note is one run of
        // bullets no matter how many lines it holds.
        app.boxes
            .session
            .as_mut()
            .unwrap()
            .reveal
            .insert((id, Field::Notes));

        let size = egui::vec2(900.0, 600.0);
        let mut run =
            |events: Vec<egui::Event>| shot::frame(&ctx, size, events, |ui| show(ui, &mut app));
        let over = egui::pos2(300.0, 400.0);
        let before = run(vec![egui::Event::PointerMoved(over)]);
        run(vec![
            egui::Event::PointerMoved(over),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -80.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            },
        ]);
        let after = run(Vec::new());

        let note = |t: &str| t.starts_with("line");
        assert!(
            line_y(&after, note) < line_y(&before, note),
            "the wheel moves the note, not the box: {} -> {}",
            line_y(&before, note),
            line_y(&after, note)
        );
        assert_eq!(
            line_y(&after, |t| t == "New note"),
            line_y(&before, |t| t == "New note"),
            "and the entry around it stays where it is"
        );

        // The box is the entry's, not the text's: the same rectangle before and after
        // the wheel, and the size of the room the note was given.
        let box_of = |shapes: &[ClippedShape]| {
            shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    Shape::Rect(r) if r.fill == theme::palette().elev => Some(r.rect),
                    _ => None,
                })
                .max_by(|a, b| a.height().total_cmp(&b.height()))
                .expect("the note's box is painted")
        };
        let (was, now) = (box_of(&before), box_of(&after));
        assert_eq!(was, now, "the box does not travel with the note");
        assert!(
            (100.0..1000.0).contains(&was.height()),
            "the box is the room the note was given: {was:?}"
        );
    }
}
