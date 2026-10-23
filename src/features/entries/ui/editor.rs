//! The record editor, beside its list or over it as a modal: a login, a card and an
//! identity each draw their own fields, and share the head, the notes and the end.

use crate::app::{App, Confirm, Field, StatusKind};
use crate::shared::theme;
use crate::shared::ui::entry as record;
use crate::shared::ui::responsive;

use crate::features::autotype::DEFAULT;
use crate::vault::matching::{self, Rule, Uri};
use crate::vault::model::{now_secs, Field as Custom, Kind};
use egui::{Layout, RichText, TextEdit, TextStyle, Ui};

pub fn show(ui: &mut Ui, app: &mut App) {
    let sel = app.boxes.selected();
    let compact = responsive::compact(ui.ctx());
    record::pane(ui, sel, |ui, sel| {
        let (closed, _) = face(ui, app, sel, compact);
        if closed {
            if let Some(s) = app.boxes.session.as_mut() {
                s.selected = None;
            }
        }
        body(ui, app, sel);
    });
}

/// The selected login as a floating modal; a backdrop click or Esc clears the
/// selection.
pub fn modal(ctx: &egui::Context, app: &mut App) {
    let sel = app.boxes.selected();
    let Some(sel) = sel else { return };
    // folders have their own modal (drive::modal)
    if app
        .boxes
        .session
        .as_ref()
        .is_some_and(|s| s.vault.folders.iter().any(|f| f.id == sel))
    {
        return;
    }

    // Bounded by the window: long content scrolls inside instead of spilling off screen.
    let screen = crate::shared::ui::chrome::body(ctx);
    let max_h = (screen.height() - 48.0).max(200.0);
    let width = 520.0;

    app.boxes.editing(true);
    let open = theme::dialog(ctx, egui::Id::new("entry"), |ui| {
        theme::width(ui, width);
        ui.set_max_height(max_h);
        let (closed, changed) = face(ui, app, &sel, true);
        if changed {
            if let Some(s) = app.boxes.session.as_mut() {
                if let Some(it) = s.vault.logins.iter_mut().find(|i| i.id == sel) {
                    it.touched(now_secs());
                }
                s.mark_dirty();
            }
        }
        let rest = ui.available_height();
        theme::band(
            ui,
            egui::ScrollArea::vertical()
                .max_height(rest)
                .auto_shrink([false, true]),
            |ui| body(ui, app, &sel),
        );
        closed
    });

    if !open {
        if let Some(s) = app.boxes.session.as_mut() {
            s.selected = None;
        }
        app.boxes.editing(false);
        app.boxes.flush(&mut app.shell);
    }
}

/// Brand picker popup; returns `Pick::None` until a brand (or default) is chosen.
fn picker(
    ui: &mut Ui,
    icons: &theme::Icons,
    anchor: egui::Response,
    current: Option<theme::Brand>,
) -> Option<theme::Pick> {
    let ctx = ui.ctx().clone();
    // Eight cells a row is a pointer's grid: on a phone the same rows would be
    // 34 pt squares no thumb can pick, so the grid drops to four wider cells,
    // and the glyph inside them follows.
    let compact = responsive::compact(&ctx);
    let cols = if compact { 4 } else { 8 };
    let cell = if compact {
        responsive::target(&ctx)
    } else {
        34.0
    };
    let glyph = responsive::grow(&ctx, 17.0);
    let id = ui.id().with("brand-picker");
    let popup = egui::Popup::from_toggle_button_response(&anchor)
        .id(id)
        .width(320.0)
        .show(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            ui.label(
                RichText::new(crate::shared::i18n::t("PLATFORM ICON"))
                    .small()
                    .color(theme::palette().muted),
            );
            ui.add_space(2.0);
            let mut pick = theme::Pick::None;
            theme::band(
                ui,
                egui::ScrollArea::vertical()
                    .max_height(210.0)
                    .auto_shrink([false, false]),
                |ui| {
                    egui::Grid::new("brand-grid")
                        .num_columns(cols)
                        .min_col_width(cell)
                        .min_row_height(cell)
                        .spacing(egui::vec2(6.0, 6.0))
                        .show(ui, |ui| {
                            let brands = std::iter::once(None)
                                .chain(theme::Brand::ALL.iter().copied().map(Some));
                            for (i, brand) in brands.enumerate() {
                                let selected = current == brand;
                                let tint = if selected {
                                    theme::ACCENT
                                } else {
                                    theme::palette().muted
                                };
                                let img = match brand {
                                    Some(b) => icons.brand_image(b, glyph, tint),
                                    None => icons.image(theme::Icon::Key, glyph, tint),
                                };
                                let btn = match img {
                                    Some(img) => egui::Button::new(img),
                                    None => egui::Button::new(RichText::new("")),
                                };
                                let mut btn = btn.min_size(egui::vec2(cell, cell)).small();
                                if selected {
                                    btn = btn.stroke(theme::stroke(theme::ACCENT, 1.0));
                                }
                                let tip = match brand {
                                    Some(b) => b.label(),
                                    None => "default — kind icon / URL auto-suggest",
                                };
                                let hit = if selected {
                                    theme::chosen(ui, |ui| ui.add(btn))
                                } else {
                                    ui.add(btn)
                                };
                                if hit.on_hover_text(tip).clicked() {
                                    pick = match brand {
                                        Some(b) => theme::Pick::Set(b),
                                        None => theme::Pick::Clear,
                                    };
                                }
                                if (i + 1) % 8 == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                },
            );
            pick
        });
    popup.map(|inner| {
        if inner.inner != theme::Pick::None {
            egui::Popup::close_id(&ctx, id);
        }
        inner.inner
    })
}

/// The record's face: its icon, the picker when its shape carries a brand, and its
/// name, with the dialog's cross after them when this is the modal's header.
/// Returns `(closed, changed)`.
fn face(ui: &mut Ui, app: &mut App, sel: &str, close: bool) -> (bool, bool) {
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return (false, false);
    };
    let Some(mut it) = s.vault.any_mut(sel) else {
        return (false, false);
    };
    let kind = it.kind();
    let branded = matches!(
        it,
        crate::vault::model::AnyMut::Login(_) | crate::vault::model::AnyMut::Card(_)
    );
    let brand = match &it {
        crate::vault::model::AnyMut::Login(login) => theme::brand_of(login),
        crate::vault::model::AnyMut::Card(card) => {
            card.icon.as_deref().and_then(theme::Brand::from_slug)
        }
        _ => None,
    };
    let mut picked = None;
    let (closed, mut changed, _) = theme::record_head(
        ui,
        icons,
        close,
        |ui| {
            if let Some(img) = icons.glyph(
                brand,
                theme::kind_icon(kind),
                responsive::grow(ui.ctx(), 16.0),
                theme::tag_color(kind),
            ) {
                let side = responsive::target(ui.ctx());
                let resp = ui
                    .add(
                        egui::Button::new(img)
                            .min_size(egui::vec2(side, side))
                            .small(),
                    )
                    .on_hover_text("choose icon");
                if branded {
                    picked = picker(ui, icons, resp, brand);
                }
            }
        },
        it.name_mut(),
        "name",
        |_| {},
    );
    let closed = if close { closed } else { false };
    if let Some(p) = picked {
        if let Some(icon) = it.icon_mut() {
            match p {
                theme::Pick::Set(b) => {
                    *icon = Some(b.slug().to_owned());
                    changed = true;
                }
                theme::Pick::Clear => {
                    *icon = None;
                    changed = true;
                }
                theme::Pick::None => {}
            }
        }
    }
    if changed {
        it.touched(now_secs());
    }
    // `it` borrows the session; its last use is above, so the mark below is free to
    // take the session again.
    if changed {
        if let Some(s) = app.boxes.session.as_mut() {
            s.mark_dirty();
        }
    }
    ui.add_space(0.0);
    (closed, changed)
}

/// The fields a shape draws between the head and the notes, and the end every shape
/// shares. One dispatch, three bodies: a shape is a form, not a pane.
fn body(ui: &mut Ui, app: &mut App, sel: &str) {
    let kind = app
        .boxes
        .session
        .as_ref()
        .and_then(|s| s.vault.any(sel))
        .map(|it| it.kind());
    match kind {
        Some(Kind::Login) => body_login(ui, app, sel),
        Some(Kind::Card) => body_card(ui, app, sel),
        Some(Kind::Identity) => body_identity(ui, app, sel),
        Some(Kind::Passkey) => body_passkey(ui, app, sel),
        _ => {}
    }
}

fn body_login(ui: &mut Ui, app: &mut App, sel: &str) {
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };
    let mut changed = false;
    let mut copied = false;
    let mut show_secret = s.shown(sel, Field::Password);
    let secret_was = show_secret;
    let mut show_notes = s.shown(sel, Field::Notes);
    let notes_was = show_notes;

    let it = s.vault.logins.iter_mut().find(|i| i.id == sel);
    let Some(it) = it else { return };
    // The field is bound in place, so what it displaces is only readable before the
    // edit: copied here, kept when the edit reports itself.
    let mut previous = it.password.clone();

    let m = egui::Margin::symmetric(10, 7);
    changed |= ui
        .add(
            TextEdit::singleline(&mut it.username)
                .hint_text(crate::shared::i18n::t("username"))
                .desired_width(f32::INFINITY)
                .margin(m),
        )
        .changed();
    ui.add_space(14.0);
    // The login's own `url` is the first row; the rest live beside it. One rule
    // each, so a login can pin one host and still answer a whole domain. A picker
    // is as wide as its widest label, never as wide as the one a row happens to
    // show: egui's `ComboBox::width` is a floor, so a rule that outgrows it would
    // size its own button and the column would drift with the rule. The marks go
    // in first, so the field takes exactly the room they leave, and every row
    // gives up the mark's slot, so field and picker hold one column down the list.
    let margin = ui.spacing().button_padding.x;
    let picker = Rule::ALL
        .iter()
        .map(|rule| {
            ui.painter()
                .layout_no_wrap(
                    rule.label().to_owned(),
                    TextStyle::Button.resolve(ui.style()),
                    theme::palette().text,
                )
                .size()
                .x
        })
        .fold(0.0, f32::max)
        + ui.spacing().icon_spacing
        + ui.spacing().icon_width
        + 2.0 * margin;
    let mut drop_url: Option<usize> = None;
    let mut add_url = false;
    for index in 0..=it.extra_urls.len() {
        let (url, rule) = if index == 0 {
            (&mut it.url, &mut it.url_match)
        } else {
            let extra = &mut it.extra_urls[index - 1];
            (&mut extra.url, &mut extra.rule)
        };
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                if index > 0 {
                    if icons
                        .button(ui, theme::Icon::X, 14.0, theme::palette().error, "remove")
                        .clicked()
                    {
                        drop_url = Some(index - 1);
                        changed = true;
                    }
                } else {
                    add_url = icons
                        .button(ui, theme::Icon::Plus, 14.0, theme::ACCENT, "Add URL")
                        .clicked();
                }
                theme::combo(
                    ui,
                    egui::ComboBox::from_id_salt(("rule", index))
                        .width(picker)
                        .selected_text(rule.label()),
                    |ui| {
                        theme::menu(ui);
                        for choice in Rule::ALL {
                            if ui
                                .selectable_label(*rule == choice, choice.label())
                                .clicked()
                            {
                                *rule = choice;
                            }
                        }
                    },
                );
                ui.with_layout(Layout::left_to_right(egui::Align::Center), |ui| {
                    let w = ui.available_width();
                    let unmatched = !url.trim().is_empty() && matching::parse(url).is_none();
                    let resp = ui
                        .scope(|ui| {
                            if unmatched {
                                let widgets = &mut ui.style_mut().visuals.widgets;
                                widgets.inactive.bg_stroke =
                                    theme::stroke(theme::palette().error, 1.0);
                                widgets.hovered.bg_stroke =
                                    theme::stroke(theme::palette().error, 1.0);
                            }
                            ui.add(
                                TextEdit::singleline(url)
                                    .hint_text(crate::shared::i18n::t("https://…"))
                                    .desired_width(w)
                                    .margin(m),
                            )
                        })
                        .inner;
                    let resp = if unmatched {
                        resp.on_hover_text("no site will match this address")
                    } else {
                        resp
                    };
                    changed |= resp.changed();
                });
            });
        });
    }
    if let Some(index) = drop_url {
        it.extra_urls.remove(index);
    }
    if add_url {
        it.extra_urls.push(Uri {
            url: String::new(),
            rule: Rule::Domain,
        });
        changed = true;
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        // The field fills what the three marks give up: they keep their own
        // square, so the row ends flush with the card.
        let w = record::width(ui, 3, 90.0);
        let field = ui.add(
            TextEdit::singleline(&mut *it.password)
                .password(!show_secret)
                .font(TextStyle::Monospace)
                .hint_text(crate::shared::i18n::t("password"))
                .desired_width(w)
                .margin(m),
        );
        changed |= field.changed();
        if field.changed() {
            it.remember(std::mem::take(&mut previous));
        }
        let (toggle, copy) = record::secret_actions(ui, icons, show_secret);
        if toggle {
            show_secret = !show_secret;
        }
        if copy {
            app.shell.copy_text(ui.ctx(), it.password.as_str());
            copied = true;
        }
        if icons
            .button(ui, theme::Icon::Dices, 14.0, theme::ACCENT, "generate")
            .clicked()
        {
            app.generate.entry = Some(sel.to_owned());
        }
    });

    ui.add_space(14.0);
    let (notes_changed, notes_copied) = notes(
        ui,
        &mut app.shell,
        icons,
        sel,
        &mut it.notes,
        &mut show_notes,
    );
    changed |= notes_changed;
    copied |= notes_copied;

    changed |= fields(ui, icons, &mut it.fields);

    ui.add_space(12.0);
    ui.label(
        RichText::new(crate::shared::i18n::t("AUTO-TYPE"))
            .small()
            .color(theme::palette().muted),
    );
    ui.add_space(4.0);
    changed |= ui
        .add(
            TextEdit::singleline(&mut it.sequence)
                .hint_text(DEFAULT)
                .desired_width(f32::INFINITY)
                .margin(m),
        )
        .changed();
    ui.add_space(2.0);
    theme::hint(
        ui,
        crate::shared::i18n::t("{USERNAME}, {PASSWORD}, {OTP}, {ENTER}, {TAB}, {WAIT 250} — Ctrl+T types it into the focused window"),
    );

    // The passkeys this login's sites answer for, read before the list is borrowed.
    let mine: Vec<String> = s
        .vault
        .passkeys
        .iter()
        .filter(|passkey| matching::claims(it, &passkey.rp_id))
        .map(|it| it.id.clone())
        .collect();
    if !mine.is_empty() {
        changed |= passkeys(ui, icons, &mut s.vault.passkeys, &mine, &mut s.confirm);
    }

    tail(
        ui,
        app,
        sel,
        changed,
        &[
            (Field::Password, show_secret, secret_was),
            (Field::Notes, show_notes, notes_was),
        ],
    );

    if copied {
        app.shell
            .set_status("copied to clipboard", StatusKind::Saved);
    }
}

fn body_card(ui: &mut Ui, app: &mut App, sel: &str) {
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };
    let mut changed = false;
    let mut copied = false;
    let mut show_number = s.shown(sel, Field::Password);
    let number_was = show_number;
    let mut show_notes = s.shown(sel, Field::Notes);
    let notes_was = show_notes;

    let Some(it) = s.vault.cards.iter_mut().find(|i| i.id == sel) else {
        return;
    };
    let m = egui::Margin::symmetric(10, 7);

    ui.horizontal(|ui| {
        let w = record::width(ui, 3, 90.0);
        let field = ui.add(
            TextEdit::singleline(&mut *it.number)
                .password(!show_number)
                .font(TextStyle::Monospace)
                .hint_text(crate::shared::i18n::t("card number"))
                .desired_width(w)
                .margin(m),
        );
        changed |= field.changed();
        let (toggle, copy) = record::secret_actions(ui, icons, show_number);
        if toggle {
            show_number = !show_number;
        }
        if copy {
            app.shell.copy_text(ui.ctx(), it.number.as_str());
            copied = true;
        }
    });

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        let w = record::width(ui, 1, 90.0);
        changed |= ui
            .add(
                TextEdit::singleline(&mut it.holder)
                    .hint_text(crate::shared::i18n::t("cardholder"))
                    .desired_width(w)
                    .margin(m),
            )
            .changed();
        changed |= ui
            .add(
                TextEdit::singleline(&mut it.expiry)
                    .hint_text(crate::shared::i18n::t("MM/YY"))
                    .desired_width(84.0)
                    .margin(m),
            )
            .changed();
    });

    ui.add_space(14.0);
    let (notes_changed, notes_copied) = notes(
        ui,
        &mut app.shell,
        icons,
        sel,
        &mut it.notes,
        &mut show_notes,
    );
    changed |= notes_changed;
    copied |= notes_copied;
    changed |= fields(ui, icons, &mut it.fields);

    tail(
        ui,
        app,
        sel,
        changed,
        &[
            (Field::Password, show_number, number_was),
            (Field::Notes, show_notes, notes_was),
        ],
    );

    if copied {
        app.shell
            .set_status("copied to clipboard", StatusKind::Saved);
    }
}

fn body_identity(ui: &mut Ui, app: &mut App, sel: &str) {
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };
    let mut changed = false;
    let mut copied = false;
    let mut show_notes = s.shown(sel, Field::Notes);
    let notes_was = show_notes;

    let Some(it) = s.vault.identities.iter_mut().find(|i| i.id == sel) else {
        return;
    };

    let (notes_changed, notes_copied) = notes(
        ui,
        &mut app.shell,
        icons,
        sel,
        &mut it.notes,
        &mut show_notes,
    );
    changed |= notes_changed;
    copied |= notes_copied;
    changed |= fields(ui, icons, &mut it.fields);

    tail(
        ui,
        app,
        sel,
        changed,
        &[(Field::Notes, show_notes, notes_was)],
    );

    if copied {
        app.shell
            .set_status("copied to clipboard", StatusKind::Saved);
    }
}

/// A passkey as the app shows it: what it answers for, what it signs with, and the
/// two things the user may do here — rename it in the head, delete it through the
/// trash. The key itself is never drawn, printed or copied.
fn body_passkey(ui: &mut Ui, app: &mut App, sel: &str) {
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };
    let Some(it) = s.vault.passkeys.iter().find(|it| it.id == sel) else {
        return;
    };
    let site = it.rp_id.clone();
    let alg = alg_label(it.alg);
    ui.add_space(4.0);
    ui.label(
        RichText::new(crate::shared::i18n::tf(
            "answers for {site}",
            &[("site", &site)],
        ))
        .color(theme::palette().text),
    );
    ui.add_space(2.0);
    ui.label(
        RichText::new(crate::shared::i18n::tf(
            "signed with {alg}, and used from the extension on Chrome",
            &[("alg", &alg)],
        ))
        .small()
        .color(theme::palette().muted),
    );
    tail(ui, app, sel, false, &[]);
}

/// The name a COSE algorithm goes by on screen.
fn alg_label(alg: i32) -> String {
    match alg {
        -8 => "EdDSA".to_owned(),
        -7 => "ES256".to_owned(),
        other => format!("COSE {other}"),
    }
}

/// The passkeys that answer for a login's sites: renamed in place, deleted through the
/// trash like any other record. `mine` is the ids, read before the list is borrowed.
fn passkeys(
    ui: &mut Ui,
    icons: &theme::Icons,
    list: &mut [crate::vault::model::Passkey],
    mine: &[String],
    confirm: &mut Option<Confirm>,
) -> bool {
    let mut changed = false;
    ui.add_space(12.0);
    ui.label(
        RichText::new(crate::shared::i18n::t("PASSKEYS"))
            .small()
            .color(theme::palette().muted),
    );
    ui.add_space(4.0);
    for id in mine {
        let Some(it) = list.iter_mut().find(|it| &it.id == id) else {
            continue;
        };
        ui.horizontal(|ui| {
            let w = record::width(ui, 2, 80.0);
            changed |= ui
                .add(
                    TextEdit::singleline(&mut it.name)
                        .hint_text(crate::shared::i18n::t("name"))
                        .desired_width(w),
                )
                .changed();
            ui.label(
                RichText::new(format!("{} · {}", alg_label(it.alg), it.rp_id))
                    .small()
                    .color(theme::palette().muted),
            );
            if icons
                .button(
                    ui,
                    theme::Icon::Trash,
                    13.0,
                    theme::palette().error,
                    "Delete",
                )
                .clicked()
            {
                *confirm = Some(Confirm::Entry(it.id.clone()));
            }
        });
    }
    if changed {
        for id in mine {
            if let Some(it) = list.iter_mut().find(|it| &it.id == id) {
                it.touched(now_secs());
            }
        }
    }
    changed
}

/// The free text every shape carries. Returns `(changed, copied)`.
fn notes(
    ui: &mut Ui,
    shell: &mut crate::features::shell::model::Shell,
    icons: &theme::Icons,
    _sel: &str,
    text: &mut zeroize::Zeroizing<String>,
    shown: &mut bool,
) -> (bool, bool) {
    let (field, copy) = record::secret(
        ui,
        icons,
        shown,
        2,
        80.0,
        egui::Align::Center,
        |ui, w, shown| {
            ui.add(
                TextEdit::multiline(&mut **text)
                    .password(!shown)
                    .desired_rows(7)
                    .hint_text(crate::shared::i18n::t("notes…"))
                    .desired_width(w)
                    .margin(egui::Margin::symmetric(10, 8)),
            )
        },
    );
    if copy {
        shell.copy_text(ui.ctx(), text.as_str());
    }
    (field.changed(), copy)
}

/// The rows a record carries beyond its own shape: a label, a value, and whether the
/// value is a secret. Returns whether anything about them changed.
fn fields(ui: &mut Ui, icons: &theme::Icons, list: &mut Vec<Custom>) -> bool {
    let mut changed = false;
    let mut drop: Option<usize> = None;
    ui.add_space(12.0);
    ui.label(
        RichText::new(crate::shared::i18n::t("FIELDS"))
            .small()
            .color(theme::palette().muted),
    );
    ui.add_space(4.0);
    egui::Grid::new(ui.id().with("fields"))
        .num_columns(4)
        .spacing(egui::vec2(6.0, 6.0))
        .show(ui, |ui| {
            for (index, field) in list.iter_mut().enumerate() {
                changed |= ui
                    .add(
                        TextEdit::singleline(&mut field.label)
                            .hint_text(crate::shared::i18n::t("label"))
                            .desired_width(104.0),
                    )
                    .changed();
                let id = ui.id().with(("field-shown", index));
                let shown = ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
                changed |= ui
                    .add(
                        TextEdit::singleline(&mut *field.value)
                            .password(field.secret && !shown)
                            .hint_text(crate::shared::i18n::t("value")),
                    )
                    .changed();
                ui.horizontal(|ui| {
                    if field.secret
                        && icons
                            .button(
                                ui,
                                if shown {
                                    theme::Icon::EyeOff
                                } else {
                                    theme::Icon::Eye
                                },
                                13.0,
                                theme::palette().muted,
                                if shown { "hide" } else { "show" },
                            )
                            .clicked()
                    {
                        ui.data_mut(|d| d.insert_temp(id, !shown));
                    }
                    if icons
                        .button(
                            ui,
                            if field.secret {
                                theme::Icon::LockKeyhole
                            } else {
                                theme::Icon::LockKeyholeOpen
                            },
                            13.0,
                            if field.secret {
                                theme::ACCENT
                            } else {
                                theme::palette().muted
                            },
                            "secret",
                        )
                        .clicked()
                    {
                        field.secret = !field.secret;
                        changed = true;
                    }
                    if icons
                        .button(
                            ui,
                            theme::Icon::Trash,
                            13.0,
                            theme::palette().error,
                            "Delete",
                        )
                        .clicked()
                    {
                        drop = Some(index);
                    }
                });
                ui.end_row();
            }
        });
    if let Some(index) = drop {
        list.remove(index);
        changed = true;
    }
    if icons
        .labeled(ui, theme::Icon::Plus, 13.0, theme::ACCENT, "add field")
        .clicked()
    {
        list.push(Custom::new("", "", false));
        changed = true;
    }
    changed
}

/// The end every shape shares: the delete mark, the save, and the eye's memory.
fn tail(ui: &mut Ui, app: &mut App, sel: &str, changed: bool, flags: &[(Field, bool, bool)]) {
    let icons = &app.icons;
    let Some(s) = app.boxes.session.as_mut() else {
        return;
    };
    if record::delete(ui, icons, 28.0) {
        s.confirm = Some(Confirm::Entry(sel.to_owned()));
    }
    if changed {
        if let Some(mut it) = s.vault.any_mut(sel) {
            it.touched(now_secs());
        }
        s.mark_dirty();
    }
    for (field, show, was) in flags {
        if show != was {
            s.show(sel, *field, *show);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::app::{Screen, Session, Tab};
    use crate::features::entries::model::{Axis, View};
    use crate::shared::ui::shot;
    use crate::vault::model::{Login, Vault};
    use crate::vault::store::Credentials;
    use egui::epaint::ClippedShape;
    use egui::Shape;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use zeroize::Zeroizing;

    pub(crate) fn session(vault: Vault, selected: Option<String>) -> Session {
        Session {
            path: PathBuf::from("/tmp/editor-test.pandora"),
            creds: Credentials::rekey("test", None).unwrap(),
            vault,
            parked: false,
            parked_seq: 0,
            dirty: false,
            immediate: false,
            editing: false,
            last_edit: None,
            last_mtime: None,
            conflict: false,
            ignore_mtime: None,
            remote: None,
            baseline: None,
            saved: None,
            push_at: None,
            behind: false,
            selected,
            search: String::new(),
            last_delete: None,
            show_trash: false,
            reveal: HashSet::new(),
            show_new: false,
            show_qr: false,
            scan: None,
            confirm: None,
            tab: Tab::Passwords,
            folder: None,
            collapsed: HashSet::new(),
            view: View::List,
            axis: Axis::Target,
            at: Vec::new(),
            editing_folder: None,
            new_folder: None,
            editing_entry: None,
            moving_entry: None,
        }
    }

    /// One login painted through the editor twice, the second frame settled.
    pub(crate) fn painted(item: Login) -> Vec<ClippedShape> {
        let id = item.id.clone();
        let mut vault = Vault::new("Test");
        vault.logins = vec![item];
        painted_with(vault, id)
    }

    /// Any box painted through the editor, for the records that need their neighbours.
    pub(crate) fn painted_with(vault: Vault, id: String) -> Vec<ClippedShape> {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Main;
        app.boxes.session = Some(session(vault, Some(id.clone())));

        let size = egui::vec2(1000.0, 700.0);
        shot::frame(&ctx, size, Vec::new(), |ui| {
            face(ui, &mut app, &id, false);
            body(ui, &mut app, &id);
        });
        shot::frame(&ctx, size, Vec::new(), |ui| {
            face(ui, &mut app, &id, false);
            body(ui, &mut app, &id);
        })
    }

    /// One card painted through the editor twice, the second frame settled.
    pub(crate) fn painted_card(item: crate::vault::model::Card) -> Vec<ClippedShape> {
        let id = item.id.clone();
        let mut vault = Vault::new("Test");
        vault.cards = vec![item];
        painted_with(vault, id)
    }

    /// One passkey painted the way the pane shows it.
    pub(crate) fn painted_passkey(item: crate::vault::model::Passkey) -> Vec<ClippedShape> {
        let id = item.id.clone();
        let mut vault = Vault::new("Test");
        vault.passkeys = vec![item];
        painted_with(vault, id)
    }

    /// Every string a frame painted.
    pub(crate) fn words(shapes: &[ClippedShape]) -> Vec<String> {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    /// A passkey is drawn as what it answers for and what signs, never as a key.
    #[test]
    fn a_passkey_says_what_it_answers_for() {
        let mut item = crate::vault::model::Passkey::new(1, "example.com");
        item.id = "pk".to_owned();
        item.name = "Example".to_owned();
        let said = words(&painted_passkey(item));
        for line in [
            "answers for example.com",
            "signed with ES256, and used from the extension on Chrome",
        ] {
            assert!(
                said.iter().any(|word| word == line),
                "{line:?} is missing: {said:?}"
            );
        }
    }

    /// The login a passkey belongs to lists it, by the site they share, and lets it be
    /// The section draws only the ids it is given — the login's own sites — and each
    /// row is the name, editable, with the algorithm and the site beside it.
    #[test]
    fn the_passkey_section_draws_only_the_ids_it_is_given() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut list = vec![
            crate::vault::model::Passkey::new(1, "bank.example"),
            crate::vault::model::Passkey::new(1, "other.example"),
        ];
        list[0].id = "mine".to_owned();
        list[0].name = "ana".to_owned();
        list[1].id = "not-mine".to_owned();
        list[1].name = "someone else".to_owned();
        let mine = ["mine".to_owned()];
        let mut confirm: Option<Confirm> = None;
        let shapes = shot::frame(&ctx, egui::vec2(900.0, 300.0), Vec::new(), |ui| {
            let app = App::blank(&ctx);
            passkeys(ui, &app.icons, &mut list, &mine, &mut confirm);
        });
        let said = words(&shapes);
        assert!(said.iter().any(|word| word == "PASSKEYS"), "{said:?}");
        assert!(said.iter().any(|word| word == "ana"), "the name is drawn");
        assert!(
            !said.iter().any(|word| word == "someone else"),
            "another site's passkey stays out: {said:?}"
        );
    }

    #[test]
    fn a_card_draws_its_own_fields_beside_the_shared_ones() {
        let mut card = crate::vault::model::Card::new(1);
        card.id = "card".to_owned();
        card.fields
            .push(crate::vault::model::Field::new("PIN", "", true));
        let said = words(&painted_card(card));
        for hint in ["card number", "cardholder", "MM/YY", "notes…", "value"] {
            assert!(
                said.iter().any(|word| word == hint),
                "{hint:?} is missing: {said:?}"
            );
        }
    }

    /// The field under the glyphs reading `text`: the smallest box around them,
    /// the lower one when the same string is painted twice.
    pub(crate) fn field(shapes: &[ClippedShape], text: &str) -> egui::epaint::RectShape {
        let hint = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text() == text => Some(s.shape.visual_bounding_rect()),
                _ => None,
            })
            .max_by(|a, b| a.center().y.partial_cmp(&b.center().y).unwrap())
            .unwrap_or_else(|| panic!("{text:?} sits on screen"));
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.rect.contains(hint.center()) => Some(r.clone()),
                _ => None,
            })
            .min_by(|a, b| a.rect.area().partial_cmp(&b.rect.area()).unwrap())
            .expect("the hint sits inside its field")
    }

    /// The right edge of the row's last mark: the control that trails the field,
    /// never the field itself, which spans the row.
    fn end_x(shapes: &[ClippedShape], y: f32) -> f32 {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r)
                    if r.rect.width() <= 200.0 && (r.rect.center().y - y).abs() <= 3.0 =>
                {
                    Some(r.rect.max.x)
                }
                _ => None,
            })
            .fold(f32::NEG_INFINITY, f32::max)
    }

    /// The dice on the password row opens the generator's conditions for this
    /// entry instead of overwriting the field with a default password.
    #[test]
    fn the_dice_opens_the_conditions_for_this_entry() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let item = Login::new(0);
        let id = item.id.clone();
        let mut vault = Vault::new("Test");
        vault.logins = vec![item];
        let mut app = App::blank(&ctx);
        app.screen = Screen::Main;
        app.boxes.session = Some(session(vault, Some(id.clone())));

        let size = egui::vec2(1000.0, 700.0);
        let mut frame = |events: Vec<egui::Event>| {
            shot::frame(&ctx, size, events, |ui| body(ui, &mut app, &id))
        };
        frame(Vec::new());
        let shapes = frame(Vec::new());

        // The dice trails the eye and the copy mark on the password row.
        let y = field(&shapes, "password").rect.center().y;
        let dice = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r)
                    if r.rect.width() <= 200.0 && (r.rect.center().y - y).abs() <= 3.0 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .max_by(|a, b| a.max.x.partial_cmp(&b.max.x).unwrap())
            .expect("the mark that trails the field");
        let at = dice.center();
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

        assert_eq!(
            app.generate.entry.as_deref(),
            Some(id.as_str()),
            "the conditions open on this entry"
        );
        assert!(
            app.boxes.session.as_ref().unwrap().vault.logins[0]
                .password
                .is_empty(),
            "the dice must not write a password of its own"
        );
    }

    /// Both secret rows give up exactly one square per trailing mark, so the
    /// card ends flush after the last one instead of leaving a gap.
    #[test]
    fn the_secret_rows_end_flush() {
        let shapes = painted(Login::new(0));

        // The kind tag also reads "password": the hint is the lower of the two.
        let password = end_x(&shapes, field(&shapes, "password").rect.center().y);
        let notes = end_x(&shapes, field(&shapes, "notes…").rect.center().y);
        assert!(
            (password - notes).abs() <= 3.0,
            "the secret rows end apart: password {password}, notes {notes}"
        );
    }

    /// Every URL row gives up the same room, its own mark included: the field
    /// and the rule picker hold one column down the list, so the rule of a new
    /// row lands where the rule above it sits.
    #[test]
    fn the_url_rows_share_one_column() {
        let mut item = Login::new(0);
        item.url = "example.com".to_owned();
        item.extra_urls.push(Uri {
            url: "example.org".to_owned(),
            rule: Rule::Host,
        });
        item.extra_urls.push(Uri {
            url: "example.net".to_owned(),
            rule: Rule::Prefix,
        });
        let shapes = painted(item);

        // The picker is the one mark on the row that is neither the field (which
        // spans it) nor the 28 px cross, so the row is told by its value alone.
        let picker = |value: &str| {
            let y = field(&shapes, value).rect.center().y;
            shapes
                .iter()
                .find_map(|s| match &s.shape {
                    Shape::Rect(r)
                        if (60.0..200.0).contains(&r.rect.width())
                            && (r.rect.center().y - y).abs() <= 3.0 =>
                    {
                        Some(r.rect.min.x)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the picker of {value:?} sits on its row"))
        };
        let columns = [
            picker("example.com"),
            picker("example.org"),
            picker("example.net"),
        ];
        let spread = columns.iter().cloned().fold(f32::MIN, f32::max)
            - columns.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread <= 1.0, "the rule pickers sit apart: {:?}", columns);

        // The last row still carries a mark, and it ends where the secret rows
        // do: the column is straight, not short of the card.
        let card = end_x(&shapes, field(&shapes, "password").rect.center().y);
        let first = end_x(&shapes, field(&shapes, "example.com").rect.center().y);
        let last = end_x(&shapes, field(&shapes, "example.net").rect.center().y);
        assert!(
            (first - card).abs() <= 3.0,
            "the first url row ends apart: mark {first}, card {card}"
        );
        assert!(
            (last - card).abs() <= 3.0,
            "the url row ends apart: mark {last}, card {card}"
        );
    }

    #[test]
    fn the_first_url_mark_adds_a_domain() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let item = Login::new(0);
        let id = item.id.clone();
        let mut vault = Vault::new("Test");
        vault.logins = vec![item];
        let mut app = App::blank(&ctx);
        app.screen = Screen::Main;
        app.boxes.session = Some(session(vault, Some(id.clone())));

        let size = egui::vec2(1000.0, 700.0);
        let mut frame = |events: Vec<egui::Event>| {
            shot::frame(&ctx, size, events, |ui| body(ui, &mut app, &id))
        };
        frame(Vec::new());
        let shapes = frame(Vec::new());
        let y = field(&shapes, "https://…").rect.center().y;
        let at = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r)
                    if (theme::ICON - 1.0..=theme::ICON + 1.0).contains(&r.rect.width())
                        && (r.rect.center().y - y).abs() <= 3.0 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .max_by(|a, b| a.max.x.partial_cmp(&b.max.x).unwrap())
            .expect("the first URL mark")
            .center();
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

        let extra = &app.boxes.session.as_ref().unwrap().vault.logins[0].extra_urls;
        assert_eq!(extra.len(), 1);
        assert_eq!(extra[0].rule, Rule::Domain);
    }

    /// A URL the matcher cannot read claims nothing, so it is painted as a
    /// problem instead of being skipped in silence.
    #[test]
    fn a_dead_url_is_marked() {
        let mut item = Login::new(0);
        item.url = "not a url".to_owned();
        let shapes = painted(item);

        let marked = field(&shapes, "not a url");
        assert_eq!(
            marked.stroke.color,
            theme::palette().error,
            "a dead url is marked"
        );
    }

    /// Picking a platform in the popup writes the entry's icon and marks the
    /// vault dirty, so the choice survives the repaint instead of being lost.
    #[test]
    fn picking_a_platform_writes_the_icon() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut item = Login::new(0);
        item.icon = Some("google".to_owned());
        let id = item.id.clone();
        let mut vault = Vault::new("Test");
        vault.logins = vec![item];
        let mut app = App::blank(&ctx);
        app.screen = Screen::Main;
        app.boxes.session = Some(session(vault, Some(id.clone())));

        let size = egui::vec2(1000.0, 700.0);
        let mut frame = |events: Vec<egui::Event>| {
            shot::frame(&ctx, size, events, |ui| {
                face(ui, &mut app, &id, false);
                body(ui, &mut app, &id);
            })
        };
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
        frame(Vec::new());
        let shapes = frame(Vec::new());
        let at = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.brush.is_some() && r.rect.min.y < 40.0 => Some(r.rect),
                _ => None,
            })
            .next()
            .expect("the header glyph")
            .center();
        frame(click(at, true));
        frame(click(at, false));

        let shapes = frame(Vec::new());
        let tile = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.brush.is_some() && r.rect.center().y > 40.0 => Some(r.rect),
                _ => None,
            })
            .min_by(|a, b| {
                a.center()
                    .y
                    .partial_cmp(&b.center().y)
                    .unwrap()
                    .then(a.center().x.partial_cmp(&b.center().x).unwrap())
            })
            .expect("the popup's first tile")
            .center();
        frame(click(tile, true));
        frame(click(tile, false));
        frame(Vec::new());

        let s = app.boxes.session.as_ref().unwrap();
        assert!(
            s.vault.logins[0].icon.is_none(),
            "the default tile clears the entry's icon"
        );
        assert!(s.dirty, "the pick marks the vault dirty");
    }

    /// The image on the editor's first row, where the icon sits. egui paints a
    /// texture as a rect carrying a brush and tints it through the fill.
    fn header_glyph(shapes: &[ClippedShape]) -> (egui::TextureId, egui::Color32) {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.rect.min.x < 40.0 && r.rect.min.y < 40.0 => {
                    let brush = r.brush.as_ref()?;
                    Some((brush.fill_texture_id, r.fill))
                }
                _ => None,
            })
            .next()
            .expect("the header paints a glyph")
    }

    /// A login that pins no icon of its own wears the brand its URL suggests, so
    /// a Gmail login shows Google instead of the bare kind icon. An explicit icon
    /// still wins.
    #[test]
    fn the_header_wears_the_brand_the_url_suggests() {
        let mut gmail = Login::new(0);
        gmail.url = "https://account.google.com".to_owned();

        let mut pinned = Login::new(0);
        pinned.icon = Some("google".to_owned());

        let plain = Login::new(0);

        let (suggested, tint) = header_glyph(&painted(gmail));
        assert_eq!(
            tint,
            theme::tag_color(Kind::Login),
            "a brand wears the kind's colour, not a grey of its own"
        );
        assert_eq!(
            suggested,
            header_glyph(&painted(pinned)).0,
            "the URL suggests the glyph the explicit icon would paint"
        );
        assert_ne!(
            suggested,
            header_glyph(&painted(plain)).0,
            "with no brand the kind icon is painted instead"
        );
    }

    /// A login's own notes are a secret like any other: painted masked until the
    /// user asks for them.
    #[test]
    fn a_login_masks_the_notes_it_keeps() {
        let mut item = Login::new(0);
        item.notes = Zeroizing::new("ssh key in the drawer".to_owned());
        let shapes = painted(item);
        let said: Vec<_> = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Text(text) => Some(text.galley.text().to_string()),
                _ => None,
            })
            .collect();
        assert!(
            said.iter().any(|line| line.contains('\u{2022}')),
            "the notes row is masked: {said:?}"
        );
        assert!(
            said.iter()
                .all(|line| !line.contains("ssh key in the drawer")),
            "and the words never reach the screen: {said:?}"
        );
    }
}
