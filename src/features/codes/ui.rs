//! Codes pane: every seed the box holds, and the code it is showing right now.
//! Nothing leaves by itself — a code goes to the clipboard only when the user
//! presses its own button, and the countdown takes it back.

use crate::app::{App, Confirm, Drag, Field, StatusKind};
use crate::shared::theme;
use crate::shared::ui::drop::{landing, mark, Landing};
use crate::shared::ui::entry as record;
use crate::vault::model::{now_secs, Kind};
use crate::vault::totp;
use egui::{Layout, RichText, Sense, TextEdit, Ui};
use std::time::Duration;
use zeroize::Zeroizing;

/// How often the pane asks for a frame while it is on screen: the code turns over
/// with the clock, and nothing else in the app is redrawing.
const TICK: Duration = Duration::from_secs(1);

/// Height of a code's row: the same shape as a row of the passwords list.
const ROW: f32 = 52.0;

/// A code this close to turning over is dimmed, so nobody starts typing one that
/// is about to go.
const FADING: u64 = 5;

/// How much of the frame's short side the ring to aim through takes. Small on
/// purpose: a QR that fills it lands on enough pixels to read, so filling it is
/// the phone being close enough — a generous ring tells the user they are there
/// when they are not.
#[cfg(target_os = "android")]
const RING: f32 = 0.45;

pub fn main(ui: &mut Ui, app: &mut App, compact: bool) {
    egui::CentralPanel::default()
        .frame(theme::pane(compact))
        .show(ui, |ui| {
            // The `+` belongs to the pane, not to the list: it stays where it is
            // while the codes scroll under it.
            head(ui, app);
            ui.add_space(10.0);
            let mut qr = false;
            {
                let icons = &app.icons;
                ui.interact(
                    ui.available_rect_before_wrap(),
                    ui.id().with("codes-empty"),
                    Sense::CLICK,
                )
                .context_menu(|ui| {
                    theme::menu(ui);
                    if icons
                        .labeled(
                            ui,
                            theme::Icon::Plus,
                            15.0,
                            theme::palette().text,
                            "New code",
                        )
                        .clicked()
                    {
                        app.boxes.new_code();
                        ui.close();
                    }
                    if icons
                        .labeled(
                            ui,
                            theme::Icon::QrCode,
                            15.0,
                            theme::palette().text,
                            "Read a QR…",
                        )
                        .clicked()
                    {
                        qr = true;
                        ui.close();
                    }
                });
            }
            if qr {
                if let Some(s) = app.boxes.session.as_mut() {
                    s.show_qr = true;
                }
            }
            theme::scroll(
                ui,
                egui::ScrollArea::vertical().auto_shrink([false, false]),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    rows(ui, app);
                },
            );
        });

    create(ui.ctx(), app);
    modal(ui.ctx(), app);
    ways(ui.ctx(), app);

    ui.ctx().request_repaint_after(TICK);
}

/// The pane's own way in, as every pane carries one: nothing is created by
/// pressing another pane's `+`. Beside it, the same for a seed that only exists
/// as a picture: one is on the clipboard after a screenshot, one is a file, and
/// on a phone the camera reads it as it is aimed.
fn head(ui: &mut Ui, app: &mut App) {
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            if app
                .icons
                .button(ui, theme::Icon::Plus, 15.0, theme::ACCENT, "New code")
                .clicked()
            {
                app.boxes.new_code();
            }
            if app
                .icons
                .button(
                    ui,
                    theme::Icon::QrCode,
                    15.0,
                    theme::palette().muted,
                    "Read a QR",
                )
                .clicked()
            {
                if let Some(s) = app.boxes.session.as_mut() {
                    s.show_qr = true;
                }
            }
        });
    });
    ui.add_space(10.0);
}

/// The ways a picture reaches the pane, as a modal: an anchored menu is a moving
/// target on a phone, and what is on offer is a choice, not a menu.
fn ways(ctx: &egui::Context, app: &mut App) {
    if !app.boxes.session.as_ref().is_some_and(|s| s.show_qr) {
        return;
    }
    #[cfg(target_os = "android")]
    let mut scan = false;
    #[cfg(not(target_os = "android"))]
    let mut clip = false;
    let mut file = false;
    let mut picked = false;
    let open = theme::dialog(ctx, egui::Id::new("qr"), |ui| {
        theme::width(ui, 300.0);
        let closed = theme::head(ui, &app.icons, "Read a QR");
        ui.add_space(6.0);
        // A phone's QR is usually on another screen, so its camera is the way —
        // the same live scan every authenticator opens. The picture in its own
        // gallery is the other, for the QR that is on the phone itself, and a
        // desktop reads what a screenshot left instead.
        #[cfg(target_os = "android")]
        {
            scan = theme::secondary_button(ui, &app.icons, Some(theme::Icon::Camera), "Scan a QR")
                .clicked();
            picked |= scan;
        }
        #[cfg(not(target_os = "android"))]
        {
            clip = theme::secondary_button(
                ui,
                &app.icons,
                Some(theme::Icon::Copy),
                "From the clipboard",
            )
            .clicked();
            picked |= clip;
        }
        file = theme::secondary_button(ui, &app.icons, Some(theme::Icon::QrCode), "From an image…")
            .clicked();
        picked |= file;
        closed || picked
    });
    // The scan is its own screen, so the camera gets the whole body: a dialog is
    // no way to aim one.
    #[cfg(target_os = "android")]
    if scan && app.boxes.start_scan(&mut app.shell) {
        app.screen = crate::app::Screen::Scan;
    }
    #[cfg(not(target_os = "android"))]
    if clip {
        app.boxes.grab_code(&mut app.shell);
    }
    if file {
        app.boxes.scan_code(&mut app.shell);
    }
    if !open {
        if let Some(s) = app.boxes.session.as_mut() {
            s.show_qr = false;
        }
    }
}

fn rows(ui: &mut Ui, app: &mut App) {
    let icons = &app.icons;
    let (select, copy, del, land) = {
        let Some(s) = app.boxes.session.as_mut() else {
            return;
        };
        if s.vault.otps.is_empty() {
            theme::empty(ui, false, "no codes yet", "press + to add one");
            return;
        }

        let now = now_secs();
        let selected = s.selected.clone();
        let carrying = egui::DragAndDrop::payload::<Drag>(ui.ctx()).is_some();
        let released = ui.ctx().input(|i| i.pointer.any_released());
        let (mut select, mut copy, mut del, mut land) = (None, None, None, None);
        for it in s.vault.otps.iter() {
            let here = selected.as_deref() == Some(it.id.as_str());
            let shown = totp::at(&it.secret, now);
            let (hit, rect, ()) = record::row(
                ui,
                ui.id().with(("otp", it.id.as_str())),
                ROW,
                egui::Sense::click_and_drag(),
                here,
                it.name.as_str(),
                |ui| {
                    record::glyph(ui, icons, Kind::Otp, None, 15.0);
                    ui.add_space(10.0);
                    let mut marks: Option<egui::Rect> = None;
                    // The marks go in first, so the name takes exactly the room
                    // they leave and elides inside it: a long name must never run
                    // under the code.
                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        match &shown {
                            Ok((code, left)) => {
                                // The code, what the clock is doing, and the copy:
                                // the value at the row's end, the action after it.
                                // The countdown keeps its width as it runs down —
                                // one that shrank from 30 to 9 would jog the code
                                // beside it.
                                let mark = icons.button(
                                    ui,
                                    theme::Icon::Copy,
                                    15.0,
                                    theme::palette().muted,
                                    "Copy code",
                                );
                                marks = Some(mark.rect);
                                if mark.clicked() {
                                    copy = Some(code.clone());
                                }
                                ui.add_space(6.0);
                                theme::hint(ui, &format!("{left:02} s"));
                                ui.add_space(8.0);
                                let tint = if *left <= FADING {
                                    theme::palette().faint
                                } else {
                                    theme::palette().text
                                };
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(code.as_str())
                                            .monospace()
                                            .size(19.0)
                                            .color(tint),
                                    )
                                    .selectable(false),
                                );
                            }
                            Err(_) => {
                                theme::hint(ui, "Invalid TOTP secret or URI");
                            }
                        }
                        ui.with_layout(Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(it.name.clone())
                                        .strong()
                                        .color(theme::palette().text),
                                )
                                .truncate(),
                            );
                        });
                    });
                    (marks, ())
                },
            );

            hit.context_menu(|ui| {
                theme::menu(ui);
                if icons
                    .labeled(
                        ui,
                        theme::Icon::RefreshCw,
                        15.0,
                        theme::palette().text,
                        "Open",
                    )
                    .clicked()
                {
                    select = Some(it.id.clone());
                    ui.close();
                }
                if let Ok((code, _)) = &shown {
                    if icons
                        .labeled(
                            ui,
                            theme::Icon::Copy,
                            15.0,
                            theme::palette().text,
                            "Copy code",
                        )
                        .clicked()
                    {
                        copy = Some(code.clone());
                        ui.close();
                    }
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
                    del = Some(it.id.clone());
                    ui.close();
                }
            });

            hit.clone().dnd_set_drag_payload(Drag::Entry(it.id.clone()));
            if hit.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            // A row carried over another one lands just above or below it, and the
            // line says which — the same rule the other lists follow.
            if carrying {
                if let Some(Landing::Side { after }) = landing(ui, rect, false, false) {
                    mark(ui, rect, after, false);
                    if released {
                        land = Some((it.id.clone(), after));
                    }
                }
            } else if hit.clicked() {
                select = Some(it.id.clone());
            }
        }
        (select, copy, del, land)
    };

    if let Some(id) = del {
        if let Some(s) = app.boxes.session.as_mut() {
            // Deleting a code must not leave its editor open behind the prompt.
            s.selected = None;
            s.confirm = Some(Confirm::Entry(id));
        }
    }
    if let Some((anchor, after)) = land {
        if let Some(Drag::Entry(id)) = egui::DragAndDrop::take_payload::<Drag>(ui.ctx()).as_deref()
        {
            app.boxes.reorder(Drag::Entry(id.clone()), &anchor, after);
        }
    }
    if let Some(id) = select {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = Some(id);
        }
    }
    if let Some(code) = copy {
        app.shell.copy_text(ui.ctx(), &code);
        app.shell.set_status("code copied", StatusKind::Saved);
    }
}

/// The camera, full screen: what it sees, a ring to aim through, and nothing
/// else — the same shape as every other camera that reads a QR. The frame is
/// taken before anything is drawn, so what is on screen is what the reader is
/// looking at.
#[cfg(target_os = "android")]
pub fn scan(ui: &mut Ui, app: &mut App) {
    // Nothing else asks for frames: without this the preview would sit still.
    ui.ctx().request_repaint();
    let found = app.boxes.poll_scan(ui.ctx(), std::time::Instant::now());

    egui::CentralPanel::default()
        .frame(theme::pane(false))
        .show(ui, |ui| {
            let body = ui.available_rect_before_wrap();
            match theme::live_frame(ui.ctx(), egui::Id::new("scan-frame")) {
                Some(frame) => {
                    // The frame fills the body as far as its own shape allows, and
                    // sits in the middle of it: a preview that is not centred is a
                    // ring that does not point at anything.
                    let size = frame.size_vec2();
                    let scale =
                        (body.width() / size.x.max(1.0)).min(body.height() / size.y.max(1.0));
                    let rect = egui::Rect::from_center_size(body.center(), size * scale);
                    ui.painter().image(
                        frame.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    let ring = rect.width().min(rect.height()) * RING;
                    ui.painter().rect_stroke(
                        egui::Rect::from_center_size(rect.center(), egui::vec2(ring, ring)),
                        theme::radius(10),
                        theme::stroke(theme::ACCENT, 2.0),
                        egui::StrokeKind::Inside,
                    );
                }
                None => {
                    ui.centered_and_justified(|ui| {
                        theme::hint(ui, "waiting for the camera…");
                    });
                }
            }
            ui.painter().text(
                egui::pos2(body.center().x, body.min.y + 14.0),
                egui::Align2::CENTER_TOP,
                crate::shared::i18n::t("point the camera at the QR"),
                egui::TextStyle::Body.resolve(ui.style()),
                theme::palette().text,
            );
        });

    if let Some(it) = found {
        // Back to the pane first — a return clears what was selected — and then
        // the new code, so it opens selected in its list.
        app.back();
        app.boxes.land(it);
        app.shell
            .set_status("code read from the camera", StatusKind::Saved);
    }
}

/// The form's own ids: the dialog, and the two fields Enter walks between.
const FORM: &str = "new-code";
const NAME: &str = "new-code-name";
const SEED: &str = "new-code-seed";

/// What a code is asked for before it exists. A `+` that wrote the record first left the
/// pane full of "New code" rows with no seed in them, one per press.
#[derive(Clone, Default)]
struct Draft {
    name: String,
    secret: Zeroizing<String>,
    shown: bool,
}

/// The form a code is born in: the name it is filed under and the seed it derives from.
/// Nothing lands in the box until both are there.
fn create(ctx: &egui::Context, app: &mut App) {
    if !app.boxes.session.as_ref().is_some_and(|s| s.show_new) {
        return;
    }
    let id = egui::Id::new(FORM);
    let name_id = egui::Id::new(NAME);
    let seed_id = egui::Id::new(SEED);
    let mut draft = ctx.data(|d| d.get_temp::<Draft>(id)).unwrap_or_default();
    let mut made = false;

    let open = theme::dialog(ctx, id, |ui| {
        theme::width(ui, 360.0);
        let closed = theme::head(ui, &app.icons, "New code");
        ui.add_space(6.0);
        // A form opens on its first field, not on its way out: Enter walks down the
        // fields, and the empty form says nothing it means.
        let fresh = ctx.memory(|m| m.focused()).is_none();
        let name = ui.add(
            TextEdit::singleline(&mut draft.name)
                .id(name_id)
                .hint_text(crate::shared::i18n::t("name"))
                .desired_width(f32::INFINITY)
                .margin(egui::Margin::symmetric(10, 7)),
        );
        if fresh {
            name.request_focus();
        }
        let ready = !draft.name.trim().is_empty() && !draft.secret.trim().is_empty();
        let mut advance = false;
        if name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            // Filled up, Enter is the form's own: otherwise it is the way to the next
            // field.
            if ready {
                made = true;
            } else {
                advance = true;
            }
        }
        ui.add_space(8.0);
        let seed = record::secret(
            ui,
            &app.icons,
            &mut draft.shown,
            2,
            90.0,
            egui::Align::Center,
            |ui, w, shown| {
                ui.add(
                    TextEdit::singleline(&mut *draft.secret)
                        .id(seed_id)
                        .password(!shown)
                        .hint_text(crate::shared::i18n::t("Base32 secret or otpauth:// URI"))
                        .desired_width(w)
                        .margin(egui::Margin::symmetric(10, 7)),
                )
            },
        );
        let seed = seed.0;
        if advance {
            seed.request_focus();
        }
        if seed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && ready {
            made = true;
        }
        ui.add_space(12.0);
        // A form with a hole in it makes no code: the name and the seed are what it is.
        let (accepted, refused) = theme::actions(
            ui,
            ctx,
            |ui| {
                let color = if ready {
                    theme::ACCENT
                } else {
                    theme::palette().muted
                };
                app.icons
                    .labeled(ui, theme::Icon::Plus, 15.0, color, "Create")
            },
            |ui| ui.button(crate::shared::i18n::t("Cancel")),
        );
        made |= accepted && ready;
        closed || refused
    });

    if made {
        app.boxes.add_code(&draft.name, &draft.secret);
    }
    if made || !open {
        if let Some(s) = app.boxes.session.as_mut() {
            s.show_new = false;
        }
        ctx.data_mut(|d| d.remove_temp::<Draft>(id));
    } else {
        ctx.data_mut(|d| d.insert_temp(id, draft));
    }
}

/// The selected code as a modal: its name, its seed, and the code it is showing.
pub fn modal(ctx: &egui::Context, app: &mut App) {
    let Some(sel) = app.boxes.selected() else {
        return;
    };
    if app.boxes.session.as_ref().is_some_and(|s| s.show_new) {
        return;
    }
    if !app
        .boxes
        .session
        .as_ref()
        .is_some_and(|s| s.vault.otps.iter().any(|o| o.id == sel))
    {
        return;
    }

    let mut changed = false;
    let mut copied = false;
    let open = theme::dialog(ctx, egui::Id::new("otp"), |ui| {
        theme::width(ui, 360.0);
        let Some(s) = app.boxes.session.as_mut() else {
            return true;
        };
        let mut show = s.shown(&sel, Field::Secret);
        let was = show;
        let Some(it) = s.vault.otps.iter_mut().find(|o| o.id == sel) else {
            return true;
        };

        let (closed, named, _) = theme::record_head(
            ui,
            &app.icons,
            true,
            |ui| {
                record::glyph(ui, &app.icons, Kind::Otp, None, 15.0);
            },
            &mut it.name,
            "name",
            |_| {},
        );
        changed |= named;

        ui.add_space(12.0);
        theme::field_label(ui, "Secret");
        ui.add_space(4.0);
        let (field, copy) = record::secret(
            ui,
            &app.icons,
            &mut show,
            2,
            90.0,
            egui::Align::Center,
            |ui, w, shown| {
                ui.add(
                    TextEdit::singleline(&mut *it.secret)
                        .password(!shown)
                        .hint_text(crate::shared::i18n::t("Base32 secret or otpauth:// URI"))
                        .desired_width(w)
                        .margin(egui::Margin::symmetric(10, 7)),
                )
            },
        );
        changed |= field.changed();
        if copy {
            app.shell.copy_text(ctx, it.secret.as_str());
            copied = true;
        }

        ui.add_space(10.0);
        match totp::at(&it.secret, now_secs()) {
            Ok((code, left)) => {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(code.as_str())
                            .monospace()
                            .size(22.0)
                            .color(theme::palette().text),
                    );
                    theme::hint(ui, &format!("{left:02} s"));
                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        if app
                            .icons
                            .button(
                                ui,
                                theme::Icon::Copy,
                                15.0,
                                theme::palette().muted,
                                "Copy code",
                            )
                            .clicked()
                        {
                            app.shell.copy_text(ctx, &code);
                            copied = true;
                        }
                    });
                });
            }
            Err(_) => theme::hint(ui, "Invalid TOTP secret or URI"),
        }

        if record::delete(ui, &app.icons, 20.0) {
            s.confirm = Some(Confirm::Entry(sel.clone()));
        }
        if show != was {
            s.show(&sel, Field::Secret, show);
        }
        closed
    });

    if changed {
        if let Some(s) = app.boxes.session.as_mut() {
            if let Some(it) = s.vault.otps.iter_mut().find(|o| o.id == sel) {
                it.touched(now_secs());
            }
            s.mark_dirty();
        }
    }
    if copied {
        app.shell
            .set_status("copied to clipboard", StatusKind::Saved);
    }
    if !open {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = None;
        }
        app.boxes.flush(&mut app.shell);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::entries::ui::editor::tests::session;
    use crate::shared::ui::shot;
    use crate::vault::model::{Otp, Vault};
    use egui::Shape;
    use zeroize::Zeroizing;

    fn painted(otp: Otp) -> (Vec<String>, App) {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let id = otp.id.clone();
        let mut vault = Vault::new("Test");
        vault.otps = vec![otp];
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session(vault, Some(id)));

        let size = egui::vec2(900.0, 600.0);
        shot::frame(&ctx, size, Vec::new(), |ui| rows(ui, &mut app));
        let shapes = shot::frame(&ctx, size, Vec::new(), |ui| rows(ui, &mut app));
        let said = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Text(text) => Some(text.galley.text().to_string()),
                _ => None,
            })
            .collect();
        (said, app)
    }

    /// The pane shows the code the seed is holding right now, and never the seed
    /// itself: what is on screen is the derived six digits.
    #[test]
    fn the_pane_shows_the_code_and_never_the_seed() {
        let mut otp = Otp::new(0);
        otp.name = "GitHub".to_owned();
        otp.secret = Zeroizing::new("JBSWY3DPEHPK3PXP".to_owned());
        let (said, _) = painted(otp);

        assert!(said.iter().any(|line| line == "GitHub"), "{said:?}");
        assert!(
            said.iter()
                .any(|line| line.len() == 6 && line.bytes().all(|byte| byte.is_ascii_digit())),
            "the live code is on screen: {said:?}"
        );
        assert!(
            said.iter().any(|line| line.strip_suffix(" s").is_some_and(
                |digits| digits.len() == 2 && digits.bytes().all(|b| b.is_ascii_digit())
            )),
            "the countdown keeps its width: {said:?}"
        );
        assert!(
            said.iter().all(|line| !line.contains("JBSWY3DPEHPK3PXP")),
            "the seed stays in the box: {said:?}"
        );
    }

    #[test]
    fn a_broken_seed_says_so_instead_of_inventing_a_code() {
        let mut otp = Otp::new(0);
        otp.name = "Broken".to_owned();
        otp.secret =
            Zeroizing::new("otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&encoder=steam".to_owned());
        let (said, _) = painted(otp);

        assert!(
            said.iter().any(|line| line.contains("Invalid TOTP")),
            "{said:?}"
        );
    }

    /// The copy mark on a row copies, and never doubles as the row's own click:
    /// the row is found and clicked where it is painted, not at a guessed point.
    #[test]
    fn copying_a_code_does_not_open_its_editor() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut otp = Otp::new(0);
        otp.name = "GitHub".to_owned();
        otp.secret = Zeroizing::new("JBSWY3DPEHPK3PXP".to_owned());
        let mut vault = Vault::new("Test");
        vault.otps.push(otp);
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session(vault, None));

        let size = egui::vec2(900.0, 600.0);
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            shot::frame(&ctx, size, events, |ui| rows(ui, app))
        };
        frame(&mut app, Vec::new());
        let shapes = frame(&mut app, Vec::new());
        // The copy mark is the last square of the row: the code is to its left.
        let row = shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text() == "GitHub" => {
                    Some(s.shape.visual_bounding_rect().center().y)
                }
                _ => None,
            })
            .expect("the row on screen");
        let at = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if (r.rect.center().y - row).abs() <= 3.0 => Some(r.rect.center()),
                _ => None,
            })
            .max_by(|a, b| a.x.partial_cmp(&b.x).unwrap())
            .expect("the mark at the end of the row");

        let click = |pressed| {
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
        frame(&mut app, click(true));
        frame(&mut app, click(false));

        assert!(
            app.shell.clip.is_some(),
            "the current code reaches the clipboard"
        );
        assert!(
            app.boxes.session.as_ref().unwrap().selected.is_none(),
            "copying does not select the row or open its editor"
        );
    }

    /// The QR mark opens the two ways a picture reaches the pane: what a
    /// screenshot left on the clipboard, and a file.
    #[cfg(not(target_os = "android"))]
    #[test]
    fn the_qr_mark_offers_both_ways_in() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session(Vault::new("Test"), None));

        let size = egui::vec2(900.0, 600.0);
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            shot::frame(&ctx, size, events, |ui| {
                egui::CentralPanel::default().show(ui, |ui| head(ui, app));
                ways(ui.ctx(), app);
            })
        };
        frame(&mut app, Vec::new());
        let shapes = frame(&mut app, Vec::new());
        let mut marks: Vec<egui::Pos2> = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Rect(r) if r.brush.is_some() && r.rect.center().y < 30.0 => {
                    Some(r.rect.center())
                }
                _ => None,
            })
            .collect();
        marks.sort_by(|a, b| b.x.partial_cmp(&a.x).unwrap());
        let qr = *marks.get(1).expect("the QR mark, left of the +");

        let click = |at: egui::Pos2, pressed: bool| {
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
        frame(&mut app, click(qr, true));
        frame(&mut app, click(qr, false));
        let shapes = frame(&mut app, Vec::new());
        let said: Vec<String> = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Text(text) => Some(text.galley.text().to_string()),
                _ => None,
            })
            .collect();
        for row in ["From the clipboard", "From an image…"] {
            assert!(said.iter().any(|line| line == row), "no {row}: {said:?}");
        }
    }

    /// A right click on a code offers the same actions as its siblings: open it,
    /// copy the code, and delete it.
    #[test]
    fn a_code_row_offers_open_copy_and_delete() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut otp = Otp::new(0);
        otp.name = "GitHub".to_owned();
        otp.secret = Zeroizing::new("JBSWY3DPEHPK3PXP".to_owned());
        let mut vault = Vault::new("Test");
        vault.otps.push(otp);
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session(vault, None));

        let size = egui::vec2(900.0, 600.0);
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            shot::frame(&ctx, size, events, |ui| rows(ui, app))
        };
        frame(&mut app, Vec::new());
        let shapes = frame(&mut app, Vec::new());
        let row = shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text() == "GitHub" => {
                    Some(s.shape.visual_bounding_rect().center())
                }
                _ => None,
            })
            .expect("the row on screen");
        drop(shapes);

        let press = |pressed: bool| {
            vec![egui::Event::PointerButton {
                pos: row,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            }]
        };
        frame(&mut app, vec![egui::Event::PointerMoved(row)]);
        frame(&mut app, press(true));
        frame(&mut app, press(false));
        let shapes = frame(&mut app, Vec::new());
        let said: Vec<String> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
        for label in ["Open", "Copy code", "Delete"] {
            assert!(
                said.iter().any(|line| line == label),
                "no {label}: {said:?}"
            );
        }
    }

    /// Dragging a row onto another one reorders the list, as every other list in
    /// the app does: the payload is the id, and the drop says above or below.
    #[test]
    fn a_dragged_code_reorders_the_list() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut vault = Vault::new("Test");
        for (id, name) in [("a", "Uno"), ("b", "Dos")] {
            let mut otp = Otp::new(0);
            otp.id = id.to_owned();
            otp.name = name.to_owned();
            otp.secret = Zeroizing::new("JBSWY3DPEHPK3PXP".to_owned());
            vault.otps.push(otp);
        }
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session(vault, None));

        let size = egui::vec2(900.0, 600.0);
        let press = |at: egui::Pos2, pressed: bool| {
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
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            shot::frame(&ctx, size, events, |ui| {
                egui::CentralPanel::default().show(ui, |ui| rows(ui, app));
            })
        };
        frame(&mut app, Vec::new());
        let shapes = frame(&mut app, Vec::new());
        let at = |text: &str| {
            shapes
                .iter()
                .find_map(|s| match &s.shape {
                    Shape::Text(t) if t.galley.text() == text => {
                        Some(s.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{text} sits on screen"))
        };
        let (from, onto) = (at("Uno"), at("Dos") + egui::vec2(0.0, 20.0));

        frame(&mut app, press(from, true));
        frame(&mut app, vec![egui::Event::PointerMoved(onto)]);
        assert!(
            egui::DragAndDrop::has_payload_of_type::<Drag>(&ctx),
            "the row must publish its drag payload"
        );
        frame(&mut app, press(onto, false));
        frame(&mut app, Vec::new());

        let names: Vec<&str> = app
            .boxes
            .session
            .as_ref()
            .unwrap()
            .vault
            .otps
            .iter()
            .map(|o| o.name.as_str())
            .collect();
        assert_eq!(names, ["Dos", "Uno"], "the dragged code lands below Dos");
    }

    /// The form is the only way in, and it writes nothing until it is filled: an empty
    /// name or an empty seed is not a code.
    #[test]
    fn the_form_writes_the_code_only_once_it_is_filled() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session(Vault::new("Test"), None));
        app.boxes.session.as_mut().unwrap().show_new = true;

        let size = egui::vec2(900.0, 600.0);
        let draft = |name: &str, secret: &str| Draft {
            name: name.to_owned(),
            secret: Zeroizing::new(secret.to_owned()),
            shown: false,
        };
        fn create_click(ctx: &egui::Context, app: &mut App, size: egui::Vec2, draft: Draft) {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new(FORM), draft));
            let mut at = None;
            for _ in 0..2 {
                let shapes = shot::frame(ctx, size, Vec::new(), |ui| create(ui.ctx(), app));
                at = shapes.iter().find_map(|shape| match &shape.shape {
                    Shape::Text(text) if text.galley.text() == "Create" => {
                        Some(shape.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                });
            }
            let at = at.expect("the form offers its own action");
            let click = |pressed| {
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
            shot::frame(ctx, size, click(true), |ui| create(ui.ctx(), app));
            shot::frame(ctx, size, click(false), |ui| create(ui.ctx(), app));
            shot::frame(ctx, size, Vec::new(), |ui| create(ui.ctx(), app));
        }

        create_click(&ctx, &mut app, size, draft("Vercel", ""));
        {
            let s = app.boxes.session.as_ref().unwrap();
            assert!(
                s.vault.otps.is_empty(),
                "a seed is what the name does not stand in for"
            );
            assert!(s.show_new, "and the form stays open for it");
        }

        create_click(&ctx, &mut app, size, draft("Vercel", "JBSWY3DPEHPK3PXP"));
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.otps.len(), 1, "the filled form makes the code");
        assert_eq!(s.vault.otps[0].name, "Vercel");
        assert_eq!(s.vault.otps[0].secret.as_str(), "JBSWY3DPEHPK3PXP");
        assert!(!s.show_new, "and the form is done with");
    }

    /// Enter walks the form instead of leaving it: it opens on the name, goes on to the
    /// seed, and only saves once both are there.
    #[test]
    fn enter_walks_the_form_and_saves_it_filled() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session(Vault::new("Test"), None));
        app.boxes.session.as_mut().unwrap().show_new = true;

        let size = egui::vec2(900.0, 600.0);
        fn frame(ctx: &egui::Context, app: &mut App, size: egui::Vec2, events: Vec<egui::Event>) {
            shot::frame(ctx, size, events, |ui| create(ui.ctx(), app));
        }
        let enter = || {
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]
        };
        let typed = |text: &str| vec![egui::Event::Text(text.to_owned())];

        frame(&ctx, &mut app, size, Vec::new());
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(egui::Id::new(NAME)),
            "the form opens on its first field"
        );

        frame(&ctx, &mut app, size, typed("Vercel"));
        frame(&ctx, &mut app, size, enter());
        assert!(
            app.boxes.session.as_ref().unwrap().show_new,
            "Enter is the way on, not the way out of the form"
        );
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(egui::Id::new(SEED)),
            "Enter lands in the next field, not on the way out"
        );

        frame(&ctx, &mut app, size, typed("JBSWY3DPEHPK3PXP"));
        frame(&ctx, &mut app, size, enter());
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.otps.len(), 1, "a filled form saves on Enter");
        assert_eq!(s.vault.otps[0].name, "Vercel");
        assert!(!s.show_new, "and closes itself");
    }

    /// An empty box says where to start, and the pane's own `+` is what it points
    /// at: it opens the form a code is born in, and writes nothing until that form is
    /// filled, so pressing it twice leaves one empty pane and no "New code" rows.
    #[test]
    fn an_empty_box_offers_its_own_way_in() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = crate::app::Screen::Main;
        app.boxes.session = Some(session(Vault::new("Test"), None));

        let size = egui::vec2(900.0, 600.0);
        let mut frame = |events: Vec<egui::Event>| {
            shot::frame(&ctx, size, events, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.set_width(ui.available_width().min(560.0));
                        head(ui, &mut app);
                        rows(ui, &mut app);
                    });
                });
            })
        };
        frame(Vec::new());
        let shapes = frame(Vec::new());
        let said: Vec<String> = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Text(text) => Some(text.galley.text().to_string()),
                _ => None,
            })
            .collect();
        assert!(
            said.iter().any(|line| line.contains("no codes yet")),
            "{said:?}"
        );

        // Both ways in paint as images on the pane's own top row: the `+` sits
        // rightmost, the QR reader beside it.
        let marks: Vec<egui::Pos2> = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Rect(r) if r.brush.is_some() && r.rect.center().y < 30.0 => {
                    Some(r.rect.center())
                }
                _ => None,
            })
            .collect();
        assert_eq!(marks.len(), 2, "a new code, and a seed to read");
        let at = marks
            .into_iter()
            .max_by(|a, b| a.x.partial_cmp(&b.x).unwrap())
            .expect("the pane's own +");
        let click = |pressed| {
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
        frame(click(true));
        frame(click(false));
        frame(click(true));
        frame(click(false));

        let s = app.boxes.session.as_ref().unwrap();
        assert!(
            s.vault.otps.is_empty(),
            "the + asks for a name and a seed instead of writing a record"
        );
        assert!(s.show_new, "and the pane opens its form");
    }
}
