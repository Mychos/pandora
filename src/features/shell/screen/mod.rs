use crate::app::{App, Tab};
use crate::features::codes::ui as codes;
use crate::features::drive::ui as drive;
use crate::features::entries::ui::list as passwords;
use crate::features::health::ui as review;
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
        Tab::Health => review::main(ui, app, compact),
        Tab::Settings => settings::run(ui, app, compact),
    }

    // The entry's own generator, over the modal the drive opens it in.
    crate::features::generator::ui::modal(&ctx, app);

    // Sealing one record out, or opening one in.
    crate::features::share::ui::modal(&ctx, app);

    // Last, so the confirmation sits above the modal it was opened from.
    crate::features::shell::confirm::modal(&ctx, app);
}

/// The strip an error wears: message and way out, wrapping in a narrow window.
fn banner(ui: &mut Ui, id: &'static str, message: &'static str, actions: impl FnOnce(&mut Ui)) {
    strip(
        ui,
        id,
        crate::shared::i18n::t(message),
        theme::palette().error,
        actions,
    );
}

/// The same strip for a notice in another colour, whose text is already built: a
/// version and a percentage are not catalog entries.
pub(crate) fn notice(ui: &mut Ui, id: &'static str, text: &str, actions: impl FnOnce(&mut Ui)) {
    strip(ui, id, text, theme::palette().note, actions);
}

fn strip(ui: &mut Ui, id: &'static str, text: &str, color: Color32, actions: impl FnOnce(&mut Ui)) {
    egui::Panel::top(id)
        .frame(
            egui::Frame::NONE
                .fill(theme::palette().panel)
                .inner_margin(egui::Margin::symmetric(18, 10))
                .stroke(theme::stroke(color, 1.0)),
        )
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
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
                    (Tab::Health, theme::Icon::History, "Review"),
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

/// The pane's own `+`: the drive holds note and file, so it is the one that asks; a code
/// asks for its name and seed in its own pane's form.
fn new_modal(ctx: &Context, app: &mut App) {
    if !app.boxes.session.as_ref().is_some_and(|s| s.show_new) {
        return;
    }
    let Some(tab) = app.boxes.session.as_ref().map(|s| s.tab) else {
        return;
    };
    if tab == Tab::Codes {
        return;
    }
    if tab != Tab::Drive {
        app.boxes.add_login();
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
        app.pick_files(ctx);
    }
    if note || file || !open {
        if let Some(s) = app.boxes.session.as_mut() {
            s.show_new = false;
        }
    }
}

#[cfg(test)]
mod shots;
