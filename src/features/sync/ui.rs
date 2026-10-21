//! The box picker in three views: recents, this machine, GitHub. Each `.pandora` file
//! in a repo is a box, and its working copy under the data dir is what works offline.

use crate::app::{App, Screen, Side, StatusKind};
use crate::features::auth::ui as auth;
use crate::features::settings::model::Source;
use crate::shared::i18n::t;
use crate::shared::theme;
use crate::shared::ui::entry as record;
use crate::shared::ui::responsive;
use crate::vault::store;
use egui::{Layout, RichText, TextEdit};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const BOTTOM: f32 = 24.0;

const AIR: f32 = 4.0;

const INSET: f32 = 14.0;

const CODE: f32 = 22.0;

const CODE_TRACK: f32 = 1.0;

const GAP: f32 = 8.0;

const ROWS: f32 = 7.0;

const BOXES: f32 = ROWS * (ROW + GAP);

fn recents_rows(ui: &egui::Ui) -> f32 {
    BOXES + search_rows(ui)
}

fn search_rows(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body) + 14.0 + ui.spacing().item_spacing.y
}

pub fn run(ui: &mut egui::Ui, app: &mut App) {
    // Repos refresh only while this view is on screen: one read, at most one per window.
    if side(app) == Side::Github {
        if let Some(wait) = app.sources_load(Instant::now()) {
            if app.shell.focused {
                ui.ctx().request_repaint_after(wait);
            }
        }
    }
    egui::CentralPanel::default()
        .frame(theme::pane(responsive::compact(ui.ctx())))
        .show(ui, |ui| {
            ui.add_space(auth::top(ui));
            ui.vertical_centered(|ui| {
                auth::column(ui);
                auth::wordmark(ui, &app.icons);
                picker(ui, app);
                ui.add_space(2.0);
                let card = theme::card(ui, "", |ui| {
                    // The recents carry one fixed band; the other two fill what they are given.
                    if side(app) == Side::Machine {
                        machine(ui, app);
                        return;
                    }
                    if side(app) == Side::Github {
                        github(ui, app);
                        return;
                    }
                    if app.auth.recents.is_empty() {
                        start(ui, app);
                        return;
                    }
                    theme::search(
                        ui,
                        &app.icons,
                        &mut app.auth.recent_search,
                        crate::features::boxes::logic::search_id(Screen::Sources),
                        "search",
                    );
                    let band = BOXES.min((ui.available_height() - BOTTOM).max(0.0));
                    theme::band(
                        ui,
                        egui::ScrollArea::vertical()
                            .id_salt("sources")
                            .auto_shrink([false, false])
                            .max_height(band),
                        |ui| recent(ui, app),
                    );
                });
                // After the card: the wait must not write a row into the list.
                if side(app) == Side::Github && reading(app) {
                    veil(ui, card.response.rect);
                }
            });
        });
    modal(ui.ctx(), app);
    new_box_modal(ui.ctx(), app);
}

/// The three views as icon buttons, in the order the boxes arrive; their names ride
/// the tooltips because three words do not fit a narrow window.
fn picker(ui: &mut egui::Ui, app: &mut App) {
    let icons = &app.icons;
    let now = side(app);
    let mut pick = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.spacing_mut().button_padding = egui::Vec2::ZERO;
        for (view, brand, icon, name, tip) in [
            (
                Side::Recents,
                None,
                theme::Icon::History,
                "Recent boxes",
                None,
            ),
            (
                Side::Machine,
                None,
                theme::Icon::Folder,
                "Boxes on this machine",
                None,
            ),
            (
                Side::Github,
                Some(theme::Brand::Github),
                theme::Icon::Folder,
                "GitHub repos",
                Some("GitHub repos — private repositories that keep copies of your boxes"),
            ),
        ] {
            if view_button(ui, icons, brand, icon, name, tip, now == view) {
                pick = Some(view);
            }
        }
    });
    if let Some(pick) = pick {
        app.auth.side = Some(pick);
    }
}

/// One view button: an icon, its name as screen-reader label and tooltip, and the
/// accent fill on show under a light icon.
fn view_button(
    ui: &mut egui::Ui,
    icons: &theme::Icons,
    brand: Option<theme::Brand>,
    icon: theme::Icon,
    name: &str,
    tip: Option<&str>,
    on: bool,
) -> bool {
    let tint = if on {
        theme::palette().text
    } else {
        theme::palette().muted
    };
    let glyph = responsive::grow(ui.ctx(), 14.0);
    let mark = brand
        .and_then(|b| icons.brand_image(b, glyph, tint))
        .or_else(|| icons.image(icon, glyph, tint));
    let Some(mark) = mark else {
        return false;
    };
    let name = crate::shared::i18n::t(name);
    let tip = tip.map_or(name, crate::shared::i18n::t);
    let side = responsive::target(ui.ctx());
    let button = egui::Button::new(mark.alt_text(name.to_owned()))
        .small()
        .min_size(egui::vec2(side, side));
    let resp = if on {
        theme::chosen(ui, |ui| ui.add(button))
    } else {
        ui.add(button)
    };
    let resp = resp.on_hover_text(tip);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, name));
    resp.clicked()
}

/// Which view is on show; the recents first, where the boxes opened before wait.
fn side(app: &App) -> Side {
    app.auth.side.unwrap_or(Side::Recents)
}

/// Whether the GitHub view is still reading repos nobody read once: only that first
/// pass is veiled, never a re-read or a repo that answered with a failure.
fn reading(app: &App) -> bool {
    app.sync
        .repo_job
        .as_ref()
        .is_some_and(|job| job.source.is_some())
        && app.prefs.sources.iter().any(|s| {
            app.boxes_of(s).is_none()
                && !app
                    .sync
                    .failed
                    .contains_key(&crate::features::sync::logic::source_key(s))
        })
}

/// The wait's scrim: dims without hiding what is under it.
const VEIL: f32 = 0.86;

/// Lighter than the card's: a dialog still has to read as itself under the wait.
const SCRIM: f32 = 0.55;

const SPIN: f32 = 22.0;

/// What dims while something is being read.
fn scrim(ui: &mut egui::Ui, rect: egui::Rect, strength: f32) {
    ui.painter().rect_filled(
        rect,
        theme::radius(12),
        theme::palette().panel.gamma_multiply(strength),
    );
}

/// The spinner that rides it, centred on what it covers.
fn spinner(ui: &egui::Ui, rect: egui::Rect) {
    let at = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(SPIN));
    egui::Spinner::new()
        .size(SPIN)
        .color(theme::ACCENT)
        .paint_at(ui, at);
}

/// The card while its repos are read: a spinner over a scrim, with nothing reserved,
/// so the list never shifts and the working copies under it still open.
fn veil(ui: &mut egui::Ui, rect: egui::Rect) {
    scrim(ui, rect, VEIL);
    spinner(ui, rect);
}

/// The ways into a local file, stacked in a narrow window.
fn file_actions(ui: &mut egui::Ui, app: &mut App) {
    let icons = &app.icons;
    let mut open = false;
    let mut create = false;
    if responsive::compact(ui.ctx()) {
        open = aux_button(ui, icons, theme::Icon::Folder, "Open a box file…");
        ui.add_space(8.0);
        create = aux_button(ui, icons, theme::Icon::Plus, "Create a new box");
    } else {
        ui.columns(2, |cols| {
            open = aux_button(&mut cols[0], icons, theme::Icon::Folder, "Open a box file…");
            create = aux_button(&mut cols[1], icons, theme::Icon::Plus, "Create a new box");
        });
    }
    if open {
        app.auth.unlock_path.clear();
        app.auth.unlock_keyfile.clear();
        app.screen = Screen::Unlock;
    }
    if create {
        app.screen = Screen::Welcome;
    }
}

/// The files on this machine; a repo's working copy belongs to the GitHub view.
fn machine(ui: &mut egui::Ui, app: &mut App) {
    file_actions(ui, app);
    ui.add_space(8.0);
    let boxes: Vec<(PathBuf, String, bool)> = app
        .auth
        .recents
        .iter()
        .filter(|r| app.source_of(&r.path).is_none())
        .map(|r| (r.path.clone(), r.label(), false))
        .collect();
    let current = app.auth.unlock_path.clone();
    let band = recents_rows(ui).min((ui.available_height() - BOTTOM).max(0.0));
    let mut pick: Option<PathBuf> = None;
    theme::band(
        ui,
        egui::ScrollArea::vertical()
            .id_salt("machine")
            .auto_shrink([false, false])
            .max_height(band),
        |ui| {
            ui.set_min_height(band);
            let (clicked, shown) = roster(ui, app, "machine", &boxes, "", &current);
            pick = clicked;
            if shown == 0 {
                theme::hint(
                    ui,
                    "no box files on this computer yet — open one above, or create one",
                );
            }
        },
    );
    if let Some(path) = pick {
        open(ui, app, &path);
    }
}

/// A full-width quiet button, its mark in front of its label.
fn aux_button(ui: &mut egui::Ui, icons: &theme::Icons, icon: theme::Icon, text: &str) -> bool {
    theme::secondary_button(ui, icons, Some(icon), text).clicked()
}

/// GitHub's half. Without a session the repos and their working copies still show:
/// signing in reaches GitHub, it is never a wall. One repo at a time, from a select.
fn github(ui: &mut egui::Ui, app: &mut App) {
    if app.auth.login.is_some() {
        login(ui, app);
        return;
    }
    let signed = app.sync.signed();
    let sources = app.prefs.sources.clone();
    if !signed {
        theme::hint(
            ui,
            if sources.is_empty() {
                "sign in with GitHub to add a repo and keep your boxes synced"
            } else {
                "this repo is listed below — sign in to sync it"
            },
        );
        ui.add_space(6.0);
        if theme::primary_button(ui, &app.icons, None, "Sign in with GitHub", None).clicked() {
            app.github_login();
        }
        ui.add_space(8.0);
        keep_toggle(ui, app);
        ui.add_space(10.0);
    } else {
        // The add-repo action sits first, above the repo it will become.
        let add = github_atoms(ui, &app.icons, "Add a GitHub repo…");
        if secondary_button(ui, add).clicked() {
            app.sync.adding = true;
        }
        ui.add_space(10.0);
    }

    let Some(source) = showing(app) else {
        theme::hint(ui, "no repo yet — add one above");
        ui.add_space(4.0);
        return;
    };

    repo_head(ui, app, &source, &sources);
    repo_boxes(ui, app, &source);

    ui.add_space(8.0);
    if theme::primary_button(
        ui,
        &app.icons,
        Some(theme::Icon::Plus),
        "New box in this repo…",
        None,
    )
    .clicked()
    {
        app.sync.new_in = Some(source.clone());
    }

    if signed {
        gap(ui);
        keep_toggle(ui, app);
    }
}

/// The repo the GitHub view is showing: the one picked, or the first assigned.
fn showing(app: &App) -> Option<Source> {
    app.sync
        .showing
        .as_ref()
        .filter(|s| app.prefs.sources.contains(s))
        .or_else(|| app.prefs.sources.first())
        .cloned()
}

/// The repo on show, as a select of everything assigned, with read-again and stop-following.
fn repo_head(ui: &mut egui::Ui, app: &mut App, source: &Source, sources: &[Source]) {
    let mut choose: Option<Source> = None;
    let mut refresh = false;
    let mut remove = false;
    let mark = app
        .icons
        .brand_image(theme::Brand::Github, 16.0, theme::palette().muted);
    let icons = &app.icons;

    ui.horizontal(|ui| {
        let width = record::width(ui, 2, 120.0);
        // `ComboBox::width` is a floor: scope the room and truncate the name, or it pushes
        // the marks after it off the card.
        ui.scope(|ui| {
            ui.set_max_width(width);
            theme::combo(
                ui,
                egui::ComboBox::from_id_salt("repo")
                    .width(width)
                    .truncate()
                    .selected_text(&source.target),
                |ui| {
                    theme::menu(ui);
                    for s in sources {
                        let size = egui::vec2(width, 0.0);
                        let entry = match mark.clone() {
                            Some(mark) => egui::Button::image_and_text(mark, &s.target)
                                .min_size(size)
                                .truncate(),
                            None => egui::Button::new(&s.target).min_size(size).truncate(),
                        };
                        let chosen = s == source;
                        let hit = if chosen {
                            theme::chosen(ui, |ui| ui.add(entry))
                        } else {
                            ui.add(entry)
                        };
                        if hit.clicked() {
                            choose = Some(s.clone());
                        }
                    }
                },
            );
        });
        if icons
            .button(
                ui,
                theme::Icon::Trash,
                15.0,
                theme::palette().muted,
                "Remove this repo — the copies already on this computer stay",
            )
            .clicked()
        {
            remove = true;
        }
        ui.add_enabled_ui(app.sync.signed(), |ui| {
            if icons
                .button(
                    ui,
                    theme::Icon::RefreshCw,
                    15.0,
                    theme::palette().muted,
                    "Reload this repo",
                )
                .clicked()
            {
                refresh = true;
            }
        });
    });

    if let Some(s) = choose {
        app.sync.showing = Some(s);
    }
    if remove {
        app.source_remove(source);
    }
    if refresh {
        app.source_refresh(source, true);
    }
}

/// The boxes of the repo on show: the repo read with a session, else the working copies;
/// the band is the recents' own, with the select and create button held still.
fn repo_boxes(ui: &mut egui::Ui, app: &mut App, source: &Source) {
    let live = app.boxes_of(source).is_some();
    let files: Vec<String> = app
        .boxes_of(source)
        .map(<[String]>::to_vec)
        .unwrap_or_else(|| crate::features::sync::logic::cached(source));
    let signed = app.sync.signed();
    // Why the last read failed; the rows still open from the local copies.
    let why = app
        .sync
        .failed
        .get(&crate::features::sync::logic::source_key(source))
        .cloned();
    let mut open: Option<String> = None;
    // The local band: what is left after the tail and the floor gap.
    let band = recents_rows(ui)
        .min(ui.available_height() - TAIL - BOTTOM)
        .max(0.0);

    ui.add_space(8.0);
    theme::band(
        ui,
        egui::ScrollArea::vertical()
            .id_salt(("boxes", &source.target))
            .auto_shrink([false, false])
            .max_height(band),
        |ui| {
            ui.set_min_height(band);
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let icons = &app.icons;
                for file in &files {
                    if row(
                        ui,
                        icons,
                        ("repo", &source.target, file),
                        name(file),
                        None,
                        Some(theme::Brand::Github),
                        RowState::Plain,
                    ) {
                        open = Some(file.clone());
                    }
                }
            });
            // The one line under the rows says which state this is — and there
            // is no line for a read still out, because the veil already says so.
            // A read nobody answered is not one of the empty ones, and a repo
            // this session cannot reach is not a repo without boxes.
            if let Some(why) = why.as_deref() {
                failure(ui, why);
            } else if files.is_empty() && live {
                theme::hint(ui, "this repo has no boxes yet — create one below");
            } else if files.is_empty() && !signed {
                theme::hint(
                    ui,
                    "only the copies saved on this computer — sign in to see the whole repo",
                );
            }
        },
    );

    if let Some(file) = open {
        if live {
            app.source_open(source, &file);
        } else if let Some(path) = crate::features::sync::logic::cached_path(source, &file) {
            // A working copy off the disk opens like any other local file: no
            // repo round trip is needed to get at what is already here.
            app.auth.unlock_path = path.display().to_string();
            app.auth.unlock_keyfile.clear();
            app.screen = Screen::Unlock;
        }
    }
}

/// The reason the boxes are the local ones, in the colour failures wear.
fn failure(ui: &mut egui::Ui, why: &str) {
    crate::shared::ui::balance::label(
        ui,
        RichText::new(crate::shared::i18n::text(why).into_owned())
            .small()
            .color(theme::palette().error),
    );
}

/// The "keep me signed in" choice; off takes the session back out of the store.
fn keep_toggle(ui: &mut egui::Ui, app: &mut App) {
    let resp = theme::check(ui, &mut app.prefs.remember, "Keep me signed in").on_hover_text(
        crate::shared::i18n::t(
            "the refresh token goes to your system keyring, never to a plain file",
        ),
    );
    if resp.changed() {
        crate::features::settings::model::save(&app.prefs);
        if app.sync.signed() {
            app.keep();
        }
    }
}

/// One value to carry, as a field: value left, copy square right, both on the target's
/// own height through [`field`], not the 32 `ui.horizontal` assumes.
fn value(ui: &mut egui::Ui, app: &mut App, text: &str, what: &str) {
    field(ui, |ui| {
        ui.add_space(INSET);
        ui.label(
            RichText::new(text)
                .monospace()
                .size(CODE)
                .extra_letter_spacing(CODE_TRACK)
                .color(theme::ACCENT),
        );
        // The copy square rides the right edge, so different lengths keep their squares in line.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(INSET);
            copy(ui, app, text, what);
        });
    });
}

/// The field a value is read in, and where the view waits: one row at the target's
/// height, the surface behind it.
///
/// Not `ui.horizontal`: it reserves 32 px and grows downwards, so a 44 px square beside
/// it sank the value; one height keeps both centres on one axis.
fn field(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    // The square's own height, never under the style's floor for a control.
    let side = responsive::target(ui.ctx()).max(ui.spacing().interact_size.y);
    // Reserved first, painted last: the surface under the value.
    let surface = theme::Surface::reserve(ui);
    let row = ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), side + 2.0 * AIR),
        egui::Layout::left_to_right(egui::Align::Center),
        add,
    );
    surface.paint(
        ui,
        row.response.rect,
        theme::radius(8),
        theme::palette().elev,
        theme::stroke(theme::palette().hair, 1.0),
    );
}

/// The wait under the action: the worker polls GitHub until the user authorises.
fn waiting(ui: &mut egui::Ui) {
    wait(ui, "waiting for you to authorize…");
}

/// The code's field before there is a code: the same row, held open with the wait inside,
/// so it fills in instead of resizing under the spinner.
fn asking(ui: &mut egui::Ui) {
    field(ui, |ui| {
        ui.add_space(INSET);
        wait(ui, "asking GitHub for a code…");
    });
}

/// A spinner and its line: what the app is waiting for, in words.
fn wait(ui: &mut egui::Ui, text: &str) {
    let glyph = responsive::glyph(ui.ctx());
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(glyph, glyph), egui::Sense::hover());
        egui::Spinner::new()
            .size(glyph)
            .color(theme::palette().muted)
            .paint_at(ui, rect);
        // A spinner and its line are one thing, not two 12 px apart.
        ui.spacing_mut().item_spacing.x = 8.0;
        crate::shared::ui::balance::label(
            ui,
            RichText::new(crate::shared::i18n::t(text))
                .small()
                .color(theme::palette().faint),
        );
    });
}

/// The device flow: the code GitHub minted, where it goes, the retry that reopens the
/// page, and the wait. The page opens itself when the code lands.
fn login(ui: &mut egui::Ui, app: &mut App) {
    // The card centres its hints, but this is a form: sentence, field and wait read left.
    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
        // Taken out of the app first, so the rows below can borrow it.
        let held = app.auth.login.as_ref().and_then(|l| l.code.clone());
        match held {
            Some((code, page)) => {
                // The address without its scheme: what one line holds whole on the
                // narrowest phone, and what a user types on another device — the
                // case the device flow exists for.
                let address = page
                    .strip_prefix("https://")
                    .or_else(|| page.strip_prefix("http://"))
                    .unwrap_or(page.as_str());
                crate::shared::ui::balance::label(
                    ui,
                    RichText::new(crate::shared::i18n::tf(
                        "enter this code at {page}",
                        &[("page", &address)],
                    ))
                    .small()
                    .color(theme::palette().faint),
                );
                ui.add_space(8.0);
                value(ui, app, &code, "code");
                ui.add_space(10.0);
                // The page already carries the code, so this one tap is the whole
                // of what the browser side asks — the way back when the page the
                // sign-in opened by itself was closed or never came up.
                if theme::primary_button(ui, &app.icons, None, "Open GitHub", None).clicked() {
                    let url = crate::features::sync::github::device_url(&page, &code);
                    app.open_page(&url);
                }
                ui.add_space(10.0);
                waiting(ui);
            }
            // The code's own row, still empty, with the wait inside it: the row
            // keeps its height, so the spinner is not replaced by a differently
            // sized box.
            None => asking(ui),
        }
        ui.add_space(14.0);
        // A full-width quiet action, like every other action of the panel.
        if theme::secondary_button(ui, &app.icons, None, "Cancel").clicked() {
            // Cancelling aborts the device flow, and the repos come back with it.
            app.auth.login = None;
        }
    });
}

/// Every box opened before; the mark says where it lives: a repo's brand or a local lock.
fn recent(ui: &mut egui::Ui, app: &mut App) {
    let boxes: Vec<(PathBuf, String, bool)> = app
        .auth
        .recents
        .iter()
        .map(|r| {
            (
                r.path.clone(),
                // The row is called after the file it points at: the box's own
                // name is inside the box, and this list is not.
                r.label(),
                app.source_of(&r.path).is_some(),
            )
        })
        .collect();

    let q = app.auth.recent_search.trim().to_lowercase();
    let current = app.auth.unlock_path.clone();
    let (pick, shown) = roster(ui, app, "recent", &boxes, &q, &current);
    if shown == 0 {
        ui.label(
            RichText::new(crate::shared::i18n::t("nothing matches"))
                .small()
                .color(theme::palette().muted),
        );
    }
    if let Some(path) = pick {
        open(ui, app, &path);
    }
}

/// With nothing opened before, the two places a first box starts from.
fn start(ui: &mut egui::Ui, app: &mut App) {
    let icons = &app.icons;
    let mut pick = None;
    ui.add_space(2.0);
    ui.columns(2, |cols| {
        if start_tile(
            &mut cols[0],
            icons,
            None,
            theme::Icon::Folder,
            t("Local"),
            t("Boxes on this machine"),
        ) {
            pick = Some(Side::Machine);
        }
        if start_tile(
            &mut cols[1],
            icons,
            Some(theme::Brand::Github),
            theme::Icon::Folder,
            t("GitHub"),
            t("Your repositories"),
        ) {
            pick = Some(Side::Github);
        }
    });
    if let Some(side) = pick {
        app.auth.side = Some(side);
    }
}

/// One of the two ways into a first box: a tile that answers the click, not a row.
fn start_tile(
    ui: &mut egui::Ui,
    icons: &theme::Icons,
    brand: Option<theme::Brand>,
    icon: theme::Icon,
    title: &str,
    sub: &str,
) -> bool {
    let surface = theme::Surface::reserve(ui);
    let frame = egui::Frame::default()
        .inner_margin(egui::Margin::symmetric(16, 18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(96.0);
            ui.vertical_centered(|ui| {
                if let Some(mark) = icons.glyph(brand, icon, 28.0, theme::palette().text) {
                    ui.add(mark);
                }
                ui.add_space(10.0);
                ui.label(RichText::new(title).strong().color(theme::palette().text));
                ui.add_space(2.0);
                ui.label(RichText::new(sub).small().color(theme::palette().faint));
            });
        });
    let rect = frame.response.rect;
    let resp = ui
        .interact(rect, ui.id().with(("start", title)), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let label = format!("{title}, {sub}");
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, false, label.as_str())
    });
    theme::focus(ui, &resp);
    let (fill, line) = theme::state(false, resp.hovered());
    surface.paint(ui, rect, theme::radius(10), fill, line);
    resp.clicked()
}

/// One box list: the mark says where each box lives, `q` filters names and paths.
/// Returns what was clicked and how many rows were drawn.
fn roster(
    ui: &mut egui::Ui,
    app: &App,
    id: &'static str,
    boxes: &[(PathBuf, String, bool)],
    q: &str,
    current: &str,
) -> (Option<PathBuf>, usize) {
    let icons = &app.icons;
    let mut pick: Option<PathBuf> = None;
    let mut shown = 0usize;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = GAP;
        for (path, name, remote) in boxes {
            let sub = short_path(path);
            if !q.is_empty() && !name.to_lowercase().contains(q) && !sub.to_lowercase().contains(q)
            {
                continue;
            }
            shown += 1;
            let key = path.display().to_string();
            if row(
                ui,
                icons,
                (id, path),
                name,
                Some(&sub),
                remote.then_some(theme::Brand::Github),
                if current == key {
                    RowState::Selected
                } else {
                    RowState::Plain
                },
            ) {
                pick = Some(path.clone());
            }
        }
    });
    (pick, shown)
}

/// Ask for a listed box's password; the keyfile is cleared, since it belongs to another box.
fn open(ui: &egui::Ui, app: &mut App, path: &Path) {
    app.auth.unlock_path = path.display().to_string();
    app.auth.unlock_keyfile.clear();
    app.screen = Screen::Unlock;
    ui.memory_mut(|mem| mem.request_focus(auth::password_id()));
}

/// A full-width quiet button; the caller hands label and mark, sprite or brand.
fn secondary_button(ui: &mut egui::Ui, atoms: egui::Atoms<'static>) -> egui::Response {
    // Force the label+mark block to the middle; the ambient layout is left-aligned.
    ui.with_layout(Layout::top_down(egui::Align::Center), |ui| {
        ui.add(egui::Button::new(atoms).min_size(egui::vec2(ui.available_width(), 40.0)))
    })
    .inner
}

/// Create a box in a repo, floating over the sources: the Welcome form without the
/// path, because the file goes where that source's copies live.
pub fn new_box_modal(ctx: &egui::Context, app: &mut App) {
    let Some(source) = app.sync.new_in.clone() else {
        return;
    };
    let busy = app.auth.op.is_some();
    let m = egui::Margin::symmetric(10, 7);
    let name_id = egui::Id::new("new-box-name");
    let mut submit = false;
    let open = theme::dialog(ctx, egui::Id::new("new-box"), |ui| {
        theme::width(ui, 340.0);
        let closed = theme::head(ui, &app.icons, "New box");
        theme::hint(
            ui,
            &crate::shared::i18n::tf("saved in {target}", &[("target", &source.target)]),
        );
        ui.add_space(14.0);
        // A window that opened with nothing typed asks for the name.
        if ui.memory(|mem| mem.focused().is_none()) {
            ui.memory_mut(|mem| mem.request_focus(name_id));
        }
        ui.add_enabled_ui(!busy, |ui| {
            ui.add(
                TextEdit::singleline(&mut app.auth.welcome.name)
                    .id(name_id)
                    .hint_text(crate::shared::i18n::t("box name"))
                    .desired_width(f32::INFINITY)
                    .margin(m),
            );
            ui.add_space(8.0);
            auth::secret_pair(
                ui,
                &app.icons,
                (
                    egui::Id::new("new-box-password"),
                    &mut app.auth.welcome.password,
                    "master password (min 12)",
                ),
                (
                    egui::Id::new("new-box-confirm"),
                    &mut app.auth.welcome.confirm,
                    "confirm",
                ),
            );
            // Enter in any field creates the box: the field surrenders the focus
            // first, so the key reaches here free to submit, and it is eaten here
            // rather than left for `theme::dialog`'s Enter-as-OK to just close on.
            submit = ctx.memory(|m| m.focused().is_none())
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        });
        ui.add_space(10.0);
        if theme::primary_button(
            ui,
            &app.icons,
            None,
            "Create box",
            busy.then_some("Creating box…"),
        )
        .clicked()
        {
            submit = true;
        }
        closed
    });

    // The create goes out before the close: the dialog only says how it ended.
    if submit {
        app.source_new(&source);
    } else if !open {
        app.sync.new_in = None;
    }
}

fn github_atoms(ui: &egui::Ui, icons: &theme::Icons, text: &str) -> egui::Atoms<'static> {
    let mut atoms =
        egui::Atoms::new(RichText::new(crate::shared::i18n::t(text)).color(theme::palette().text));
    if let Some(mark) = icons.brand_image(
        theme::Brand::Github,
        responsive::grow(ui.ctx(), 15.0),
        theme::palette().text,
    ) {
        atoms.push_left(mark);
    }
    atoms
}

fn copy(ui: &mut egui::Ui, app: &mut App, text: &str, what: &str) {
    let hit = app
        .icons
        .button(ui, theme::Icon::Copy, 15.0, theme::palette().muted, "Copy")
        .clicked();
    if hit {
        app.shell.copy_text(ui.ctx(), text);
        app.shell.set_status(
            crate::shared::i18n::tf("{what} copied", &[("what", &crate::shared::i18n::t(what))]),
            StatusKind::Saved,
        );
    }
}

/// The window that puts a repo among the sources; the token stays in memory.
pub fn modal(ctx: &egui::Context, app: &mut App) {
    if !app.sync.adding {
        return;
    }
    // Sized against the window, so it stays whole in a small one.
    let room = crate::shared::ui::chrome::body(ctx);
    let width = (room.width() - 56.0).clamp(240.0, 440.0);
    // A desk stops the list short of the body; a phone gives it what the chrome leaves.
    let list = if crate::shared::ui::responsive::compact(ctx) {
        room.height()
    } else {
        (room.height() - 200.0).clamp(80.0, 300.0)
    };

    let open = theme::dialog(ctx, egui::Id::new("github-repo"), |ui| {
        theme::width(ui, width);
        // A ceiling, not a size: the window gives way before the body does.
        theme::height(ui, room.height());
        let closed = theme::head(ui, &app.icons, "GitHub");
        if !app.sync.repo_loaded && app.sync.repo_job.is_none() {
            app.repo_repos();
        }
        pick(ui, app, list);
        // The wait rides the whole dialog, and it is the scrim the GitHub half
        // waits behind: everything steps back a step, the shapes the answer will
        // take stay lit on top of it, and nothing stops answering a click.
        if fetching(app) {
            let dialog = ctx
                .memory(|mem| mem.area_rect(egui::Id::new("github-repo")))
                .unwrap_or_else(|| ui.max_rect());
            scrim(ui, dialog, SCRIM);
            spinner(ui, dialog);
        }
        closed
    });

    if !open {
        app.sync.adding = false;
    }
    // Also when a repo was picked, which shuts the window from the inside.
    if !app.sync.adding {
        app.sync.repo_search.clear();
    }
}

fn pick(ui: &mut egui::Ui, app: &mut App, height: f32) {
    let mut chosen: Option<String> = None;
    let mut reload = false;
    let mut out = false;

    // The orgs select comes first and is disabled until GitHub answers; the field
    // under it never moves.
    owners(ui, app);

    theme::search(
        ui,
        &app.icons,
        &mut app.sync.repo_search,
        egui::Id::new("repo-search"),
        "search your repos",
    );
    ui.add_space(10.0);

    // The action row is reserved before the list takes its room; the ceiling then
    // shrinks the list in a short window instead of overflowing.
    let tail = 12.0 + ui.spacing().item_spacing.y + crate::shared::ui::responsive::target(ui.ctx());
    let band = (height - STRIP)
        .max(BAND_FLOOR.min(ui.available_height()))
        .min((ui.available_height() - tail).max(0.0));
    if fetching(app) {
        // The room the rows will take, held open from before they are here. What
        // dims it is the dialog's own scrim.
        ui.allocate_exact_size(egui::vec2(ui.available_width(), band), egui::Sense::hover());
    } else {
        listing(ui, app, band, &mut chosen);
    }

    ui.add_space(12.0);
    // A row that takes its target's height and no more: `with_layout` fills all
    // the room it is handed, which on a phone is everything under the list — the
    // two marks would float in the middle of the card and leave the empty half
    // that made the window look half-finished.
    ui.allocate_ui_with_layout(
        egui::vec2(
            ui.available_width(),
            crate::shared::ui::responsive::target(ui.ctx()),
        ),
        Layout::right_to_left(egui::Align::Center),
        |ui| {
            let icons = &app.icons;
            if icons
                .button(
                    ui,
                    theme::Icon::LogOut,
                    15.0,
                    theme::palette().muted,
                    "Sign out",
                )
                .clicked()
            {
                out = true;
            }
            if icons
                .button(
                    ui,
                    theme::Icon::RefreshCw,
                    15.0,
                    theme::palette().muted,
                    "Reload",
                )
                .clicked()
            {
                reload = true;
            }
        },
    );

    if out {
        app.github_logout();
    }
    if reload {
        app.sync.repo_loaded = false;
        app.sync.repos.clear();
        app.sync.avatars.clear();
        app.sync.avatar_textures.clear();
        app.repo_repos();
    }
    if let Some(repo) = chosen {
        app.source_add(repo);
    }
}

/// Whether the picker is still waiting for its answer: a read is out, and the
/// repos that would fill the list were emptied when it went.
fn fetching(app: &App) -> bool {
    app.sync.repos.is_empty() && app.sync.repo_job.is_some()
}

/// The repos as rows, over the reserved band: a short list leaves the room the
/// spinner already took, so both states are one size.
fn listing(ui: &mut egui::Ui, app: &mut App, height: f32, chosen: &mut Option<String>) {
    let q = app.sync.repo_search.trim().to_lowercase();
    let mut shown = 0usize;
    theme::band(
        ui,
        egui::ScrollArea::vertical()
            .id_salt("repos")
            .max_height(height)
            // egui floors a scroll area at `min_scrolled_size` (64) so a nested one
            // never collapses; here the dialog's own ceiling is the one that must
            // hold, so the list is allowed to give up all of its room.
            .min_scrolled_height(0.0)
            .auto_shrink([false, true]),
        |ui| {
            ui.set_min_height(height);
            ui.spacing_mut().item_spacing.y = 4.0;
            for r in &app.sync.repos {
                if !q.is_empty() && !r.full_name.to_lowercase().contains(&q) {
                    continue;
                }
                shown += 1;
                // A public repo is listed, not offered: it says why it is out
                // rather than hiding the repo the user came looking for. A box
                // only goes to a private one — the write refuses it anyway.
                let sub = if r.private {
                    t("private")
                } else {
                    t("public — choose a private repo")
                };
                if row(
                    ui,
                    &app.icons,
                    ("pick", &r.full_name),
                    &r.full_name,
                    Some(sub),
                    Some(theme::Brand::Github),
                    if r.private {
                        RowState::Plain
                    } else {
                        RowState::Out
                    },
                ) {
                    *chosen = Some(r.full_name.clone());
                }
            }
            if shown == 0 {
                theme::hint(
                    ui,
                    if app.sync.repos.is_empty() {
                        "GitHub returned no repos — reload, or sign in again"
                    } else {
                        "nothing matches"
                    },
                );
            }
        },
    );
}

/// Side of an owner's avatar inside the select.
const AVATAR: f32 = 18.0;

/// What the select reads while no organisation is picking the list.
const ALL: &str = "All repos";

/// Air under the select, before the search field.
const FILTER_GAP: f32 = 10.0;

/// The whole strip: the select's own line and its air. A constant, and that is
/// the point — the field under it is placed at the same height from the first
/// frame, before GitHub has answered anything. `32.0` is the height the style
/// gives every control (`spacing.interact_size`).
const STRIP: f32 = 32.0 + FILTER_GAP;

/// The list never gives up more than this much, however short the window is.
const BAND_FLOOR: f32 = 48.0;

/// What the fixed tail under the boxes costs — the air above the create button,
/// the button (`theme::primary_button` is 40 tall), the gaps and the separator
/// around it, the keep toggle, and the card's own bottom margin. The stack
/// measures 123; the constant is that rounded up, so the last row never clips.
/// A ceiling and not a measurement: the boxes are the only thing that gives
/// room up. `the_github_view_keeps_its_tail_inside_its_card` fails if it drifts.
const TAIL: f32 = 128.0;

/// The orgs behind the list as one select over the search field: `All repos` and
/// one entry per owner, each with the avatar that came back for it. It is the
/// search field's own filter wearing a face — picking types `owner/` into it,
/// which `listing` already matches against the whole `owner/name` — so the picker
/// still has one filter, and what is on is what the field says.
///
/// A select and not a row of chips: the orgs of a real account outgrow one line,
/// and the ones past the edge are ones nobody finds. It keeps the height of any
/// other control, so the search under it never moves.
fn owners(ui: &mut egui::Ui, app: &mut App) {
    let mut pick: Option<String> = None;
    let owner = filter(app);
    ui.add_enabled_ui(!fetching(app), |ui| {
        // The dialog's own width, asked for before the popup is: inside it the
        // available width is the screen's, and a row sized to that would drag
        // the popup out past the window it hangs under. The scope caps the
        // select itself, and `truncate` keeps a long owner from sizing it.
        let width = ui.available_width();
        ui.scope(|ui| {
            ui.set_max_width(width);
            theme::combo(
                ui,
                egui::ComboBox::from_id_salt("owners")
                    .width(width)
                    .truncate()
                    .selected_text(owner.as_deref().unwrap_or(ALL)),
                |ui| {
                    theme::menu(ui);
                    let all = egui::Button::new(ALL)
                        .min_size(egui::vec2(width, 0.0))
                        .truncate();
                    let hit = if owner.is_none() {
                        theme::chosen(ui, |ui| ui.add(all))
                    } else {
                        ui.add(all)
                    };
                    if hit.clicked() {
                        pick = Some(String::new());
                    }
                    for login in logins(app) {
                        let chosen = owner.as_deref() == Some(login.as_str());
                        let bytes = app.sync.avatars.get(&login);
                        let face = avatar(ui.ctx(), &login, bytes, &mut app.sync.avatar_textures)
                            .map(|tex| {
                                egui::Image::new(egui::load::SizedTexture::new(
                                    tex.id(),
                                    egui::vec2(AVATAR, AVATAR),
                                ))
                            });
                        let size = egui::vec2(width, 0.0);
                        let entry = match face {
                            Some(face) => egui::Button::image_and_text(face, &login)
                                .min_size(size)
                                .truncate(),
                            None => egui::Button::new(&login).min_size(size).truncate(),
                        };
                        let hit = if chosen {
                            theme::chosen(ui, |ui| ui.add(entry))
                        } else {
                            ui.add(entry)
                        };
                        if hit.clicked() {
                            pick = Some(format!("{login}/"));
                        }
                    }
                },
            );
        });
    });
    if let Some(pick) = pick {
        app.sync.repo_search = pick;
    }
    ui.add_space(FILTER_GAP);
}

/// The organisation the search field is holding, if it holds one: the select is
/// the field's own text wearing a face, so `uriach/` reads back as `uriach`.
fn filter(app: &App) -> Option<String> {
    let login = app.sync.repo_search.strip_suffix('/')?;
    (!login.is_empty()).then(|| login.to_owned())
}

/// The owners of everything in the list, once each and in order.
fn logins(app: &App) -> Vec<String> {
    let mut logins: Vec<String> = app.sync.repos.iter().map(|r| r.owner.clone()).collect();
    logins.sort();
    logins.dedup();
    logins
}

/// An owner's avatar as a texture, decoded once and kept with the sync state.
/// None while none came back, which leaves the chip with its name alone. The
/// map is cleared whenever the repo list is replaced, so the textures die with
/// the avatars they belong to instead of accumulating in egui's frame cache.
fn avatar(
    ctx: &egui::Context,
    login: &str,
    bytes: Option<&Vec<u8>>,
    textures: &mut HashMap<String, egui::TextureHandle>,
) -> Option<egui::TextureHandle> {
    let bytes = bytes?;
    if let Some(texture) = textures.get(login) {
        return Some(texture.clone());
    }
    let image = theme::decode(bytes).ok()?;
    let texture = ctx.load_texture(
        format!("avatar-{login}"),
        image,
        egui::TextureOptions::LINEAR,
    );
    textures.insert(login.to_owned(), texture.clone());
    Some(texture)
}

fn gap(ui: &mut egui::Ui) {
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(2.0);
}

/// How a row stands: the two ways it can be picked, and the one where it only
/// reads. A row that is out is dimmed, takes no press and no tab stop, and says
/// why in its quiet line.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowState {
    Plain,
    Selected,
    Out,
}

/// One row of a list: the mark that says where the box lives — the repo's brand,
/// a lock for a file on this machine — its name, and the quiet line under it. The
/// recents and the GitHub half both draw through it, so a row reads the same on
/// either side. Painted by hand, so it stays left-aligned whatever layout it is
/// dropped into. Clicking returns true.
fn row(
    ui: &mut egui::Ui,
    icons: &theme::Icons,
    id: impl std::hash::Hash + std::fmt::Debug,
    name: &str,
    sub: Option<&str>,
    brand: Option<theme::Brand>,
    state: RowState,
) -> bool {
    let enabled = state != RowState::Out;
    let selected = state == RowState::Selected;
    let radius = theme::radius(8);
    // Reserved up front so fill and hairline land behind the row content.
    let surface = theme::Surface::reserve(ui);
    let rect = egui::Frame::default()
        .inner_margin(egui::Margin::symmetric(12, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), ROW),
                Layout::left_to_right(egui::Align::Center),
                |ui| {
                    icons
                        .glyph(
                            brand,
                            theme::Icon::LockKeyhole,
                            15.0,
                            theme::palette().muted,
                        )
                        .map(|img| ui.add(img));
                    let name_h = ui.text_style_height(&egui::TextStyle::Body);
                    let sub_h = ui.text_style_height(&egui::TextStyle::Small);
                    // An explicit height, or the row's cross alignment has
                    // nothing to centre the text block in.
                    let block = match sub {
                        Some(_) => name_h + ui.spacing().item_spacing.y + sub_h,
                        None => name_h,
                    };
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), block),
                        Layout::top_down(egui::Align::Min),
                        |ui| {
                            // A row that cannot be picked reads as one: the name
                            // steps back with its sub-line.
                            let name_color = if enabled {
                                theme::palette().text
                            } else {
                                theme::palette().faint
                            };
                            ui.add(
                                egui::Label::new(RichText::new(name).strong().color(name_color))
                                    .truncate(),
                            );
                            if let Some(sub) = sub {
                                let path_color = if selected {
                                    theme::palette().text
                                } else {
                                    theme::palette().faint
                                };
                                ui.add(
                                    egui::Label::new(RichText::new(sub).small().color(path_color))
                                        .truncate(),
                                );
                            }
                        },
                    );
                },
            );
        })
        .response
        .rect;
    // A row that cannot be picked is no click target and no tab stop: it still
    // reads, and the hand only shows where a press would do something.
    let resp = match enabled {
        true => ui
            .interact(rect, ui.id().with(("row", id)), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand),
        false => ui.interact(rect, ui.id().with(("row", id)), egui::Sense::hover()),
    };
    let label = match sub {
        Some(sub) => format!("{name}, {sub}"),
        None => name.to_owned(),
    };
    resp.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            enabled,
            selected,
            label.as_str(),
        )
    });
    if enabled {
        theme::focus(ui, &resp);
    }
    let (fill, line) = theme::state(selected, resp.hovered() && enabled);
    surface.paint(ui, rect, radius, fill, line);
    enabled && resp.clicked()
}

fn name(file: &str) -> &str {
    file.strip_suffix(&format!(".{}", store::EXT))
        .unwrap_or(file)
}

const ROW: f32 = 46.0;

/// The home prefix as `~`, so a path reads short in a row.
pub(crate) fn short_path(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(&home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::settings::model::GITHUB;
    use crate::shared::ui::shot;
    use std::fs;

    #[test]
    fn avatar_textures_live_with_the_sync_state() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let png = include_bytes!("../../../assets/icon.png").to_vec();
        let base = ctx.tex_manager().read().num_allocated();
        let mut textures = std::collections::HashMap::new();

        {
            let first = avatar(&ctx, "octocat", Some(&png), &mut textures).expect("decodes");
            let again = avatar(&ctx, "octocat", Some(&png), &mut textures).expect("reuses");
            assert_eq!(first.id(), again.id(), "one texture per owner");
            assert_eq!(ctx.tex_manager().read().num_allocated(), base + 1);
        }

        assert_eq!(textures.len(), 1);
        textures.clear();
        assert_eq!(
            ctx.tex_manager().read().num_allocated(),
            base,
            "the refresh drops the old avatar texture"
        );
    }

    fn frame(
        app: &mut App,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        shot::frame(ctx, egui::vec2(900.0, 700.0), events, |ui| run(ui, app))
    }

    fn click(app: &mut App, ctx: &egui::Context, text: &str) {
        let shapes = frame(app, ctx, Vec::new());
        let at = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(s.shape.visual_bounding_rect().center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text:?} is on screen"));
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(app, ctx, vec![egui::Event::PointerMoved(at), button(true)]);
        frame(app, ctx, vec![button(false)]);
    }

    #[test]
    fn temp_page_shots() {
        for (w, h) in [
            (393.0, 851.0),
            (393.0, 690.0),
            (360.0, 640.0),
            (780.0, 360.0),
            (900.0, 700.0),
        ] {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut app = App::blank(&ctx);
            app.screen = Screen::Sources;
            app.auth.side = Some(Side::Github);
            app.sync.token = zeroize::Zeroizing::new("t".to_owned());
            app.sync.repo_loaded = true;
            app.sync.repos = (0..3)
                .map(|i| crate::features::sync::github::Repo {
                    full_name: format!("uriach/repo-{i}"),
                    owner: "uriach".to_owned(),
                    private: false,
                })
                .collect();
            let carbon = Source {
                kind: GITHUB.to_owned(),
                target: "uriach/carbon".to_owned(),
            };
            app.prefs.sources = vec![carbon.clone()];
            let dir = store::source_dir(&carbon.kind, &carbon.target);
            fs::create_dir_all(&dir).unwrap();
            for file in ["bank.pandora", "personal.pandora", "work.pandora"] {
                fs::write(dir.join(file), b"locked").unwrap();
            }
            let name = format!("page-{}x{}", w as u32, h as u32);
            crate::shared::ui::shot::png(&ctx, &name, w as u32, h as u32, None, |ui| {
                run(ui, &mut app)
            });
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_working_copy_opens_without_a_session() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);

        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/pandora-ui-test".to_owned(),
        };
        app.prefs.sources = vec![source.clone()];
        let dir = store::source_dir(&source.kind, &source.target);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("personal.pandora");
        fs::write(&file, b"locked").unwrap();

        click(&mut app, &ctx, "personal");
        assert_eq!(app.screen, Screen::Unlock, "the copy opens off the disk");
        assert_eq!(app.auth.unlock_path, file.display().to_string());
        assert!(
            app.sync.repo_job.is_none(),
            "no request goes out without a token"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_repo_that_did_not_answer_says_why_instead_of_looking_empty() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);

        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/unanswered".to_owned(),
        };
        app.prefs.sources = vec![source.clone()];
        app.sync.failed.insert(
            crate::features::sync::logic::source_key(&source),
            crate::features::sync::github::EXPIRED.to_owned(),
        );

        let words = texts(&frame(&mut app, &ctx, Vec::new()));
        assert!(
            words.iter().any(|w| w.contains("no longer valid")),
            "the reason is on screen: {words:?}"
        );
        assert!(
            !words.contains(&"this repo has no boxes yet — create one below".to_owned()),
            "a read nobody answered is not an empty folder: {words:?}"
        );

        // Nothing answered and the disk holds nothing either: with no session
        // the place is the sign-in hint, and not an empty repo.
        app.sync.failed.clear();
        let words = texts(&frame(&mut app, &ctx, Vec::new()));
        assert!(
            words
                .iter()
                .any(|w| w.contains("sign in to see the whole repo")),
            "the sign-in hint gets its turn: {words:?}"
        );
    }

    #[test]
    fn the_machine_view_lists_local_files_and_leaves_working_copies_alone() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Machine);

        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/machine-ui-test".to_owned(),
        };
        app.prefs.sources = vec![source.clone()];
        let inside = store::source_dir(&source.kind, &source.target).join("repo-box.pandora");
        let dir = std::env::temp_dir().join("pandora-machine-local");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let outside = dir.join("plain.pandora");
        fs::write(&outside, b"locked").unwrap();

        app.auth.recents = vec![
            crate::features::auth::model::Recent { path: inside },
            crate::features::auth::model::Recent {
                path: outside.clone(),
            },
        ];

        let words = texts(&frame(&mut app, &ctx, Vec::new()));
        // The row is the file's name: the box's own name never leaves the box.
        assert!(
            words.contains(&"plain".to_owned()),
            "the local file is listed: {words:?}"
        );
        assert!(
            !words.contains(&"repo-box".to_owned()),
            "a working copy is not a local file: {words:?}"
        );

        click(&mut app, &ctx, "plain");
        assert_eq!(app.screen, Screen::Unlock, "the row opens the box");
        assert_eq!(app.auth.unlock_path, outside.display().to_string());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_local_list_keeps_the_bar_gutter_beside_its_buttons() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Machine);
        app.auth.recents.push(crate::features::auth::model::Recent {
            path: PathBuf::from("/tmp/local-box.pandora"),
        });

        let shapes = frame(&mut app, &ctx, Vec::new());
        let row_name = rect_of(&shapes, "local-box");
        let row = surface_of(&shapes, row_name);
        let button = |text: &str| {
            let label = rect_of(&shapes, text);
            shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect)
                        if rect.fill == theme::palette().elev
                            && rect.rect.contains(label.center()) =>
                    {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .min_by(|a, b| a.area().total_cmp(&b.area()))
                .unwrap_or_else(|| panic!("{text:?} has a button surface"))
        };
        let buttons = button("Open a box file…").union(button("Create a new box"));

        assert!(
            (row.left() - buttons.left()).abs() < 1.0,
            "left edges differ: {row:?} {buttons:?}"
        );
        assert!(
            (buttons.right() - row.right() - theme::BAR).abs() < 1.0,
            "the row pays the bar's gutter on its right: {row:?} {buttons:?}"
        );
    }

    #[test]
    fn the_code_and_its_copy_sit_on_one_line() {
        for (w, h) in [(393.0, 851.0), (900.0, 700.0)] {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut app = login_app(&ctx);
            let shapes = shot::frame(&ctx, egui::vec2(w, h), Vec::new(), |ui| run(ui, &mut app));

            let code = rect_of(&shapes, DEVICE);
            let copy = mark_beside(&shapes, code);
            let off = code.center().y - copy.center().y;
            assert!(
                off.abs() <= 1.0,
                "the code sits {off:.1} px off its copy at {w}x{h}: {code:?} against {copy:?}"
            );
        }
    }

    #[test]
    fn the_sign_in_view_says_where_the_code_goes() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let size = egui::vec2(393.0, 851.0);

        let mut app = login_app(&ctx);
        let shapes = shot::frame(&ctx, size, Vec::new(), |ui| run(ui, &mut app));
        let words = texts(&shapes);
        for said in [
            "enter this code at github.com/login/device",
            DEVICE,
            "Open GitHub",
            "Cancel",
        ] {
            assert!(
                words.iter().any(|w| w == said),
                "{said:?} is on screen: {words:?}"
            );
        }

        // Before the code lands the same field holds the wait: a spinner, the
        // line saying what is being asked for, and nothing to act on yet.
        let mut app = login_app(&ctx);
        app.auth.login.as_mut().expect("a sign-in is on").code = None;
        let shapes = shot::frame(&ctx, size, Vec::new(), |ui| run(ui, &mut app));
        let words = texts(&shapes);
        assert!(
            words.iter().any(|w| w == "asking GitHub for a code…"),
            "the wait says what it waits for: {words:?}"
        );
        assert!(
            !words.iter().any(|w| w == "Open GitHub"),
            "there is nothing to open before there is a page: {words:?}"
        );
        assert!(!words.iter().any(|w| w == DEVICE), "no code yet: {words:?}");
        assert!(
            shapes.iter().any(|s| matches!(
                &s.shape,
                egui::Shape::Path(p)
                    if matches!(p.stroke.color, egui::epaint::ColorMode::Solid(c) if c == theme::palette().muted)
            )),
            "the field holds a spinner while it waits"
        );
    }

    fn rect_of(shapes: &[egui::epaint::ClippedShape], text: &str) -> egui::Rect {
        shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(s.shape.visual_bounding_rect())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text:?} is on screen"))
    }

    fn surface_of(shapes: &[egui::epaint::ClippedShape], name: egui::Rect) -> egui::Rect {
        shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.fill == theme::palette().panel
                        && (r.rect.height() - ROW).abs() < 1.0
                        && r.rect.contains(name.center()) =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("the row paints a surface under {name:?}"))
    }

    const DEVICE: &str = "WDJB-MJHT";
    const PAGE: &str = "https://github.com/login/device";

    fn login_app(ctx: &egui::Context) -> App {
        let mut app = App::blank(ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        let (_worker, rx) = std::sync::mpsc::channel();
        app.auth.login = Some(crate::features::auth::model::Login {
            code: Some((DEVICE.to_owned(), PAGE.to_owned())),
            rx,
        });
        app
    }

    fn mark_beside(shapes: &[egui::epaint::ClippedShape], value: egui::Rect) -> egui::Rect {
        shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.brush.is_some()
                        && r.rect.left() > value.right()
                        && (r.rect.center().y - value.center().y).abs() < 60.0 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("the value carries a copy beside it: {value:?}"))
    }

    #[test]
    fn a_repo_row_carries_its_mark_on_the_left() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);

        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/row-layout".to_owned(),
        };
        app.prefs.sources = vec![source.clone()];
        let dir = store::source_dir(&source.kind, &source.target);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("personal.pandora");
        fs::write(&file, b"locked").unwrap();

        let shapes = frame(&mut app, &ctx, Vec::new());
        let name = rect_of(&shapes, "personal");
        let surface = surface_of(&shapes, name);
        // The mark is an image: a rect filled with a texture brush.
        let mark = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.brush.is_some() => (r.rect.width() <= 20.0
                    && surface.contains(r.rect.center())
                    && r.rect.right() < name.left())
                .then_some(r.rect),
                _ => None,
            })
            .expect("the row carries a mark, and it comes before the name");

        assert!(
            mark.left() - surface.left() < 14.0,
            "the mark opens the row: {mark:?} in {surface:?}"
        );
        assert!(
            name.left() - mark.right() < 16.0,
            "the mark sits against the name: {mark:?} vs {name:?}"
        );
        // Centred, a 400pt row would start the name some 170pt in.
        assert!(
            name.left() - surface.left() < 80.0,
            "the name follows the mark instead of floating mid-row: {name:?} in {surface:?}"
        );
        assert!(
            name.left() < surface.center().x,
            "the row reads from the left: {name:?} in {surface:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_first_read_veils_the_card_instead_of_pushing_a_row() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        app.sync.token = zeroize::Zeroizing::new("t".to_owned());

        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/veiled".to_owned(),
        };
        app.prefs.sources = vec![source.clone()];
        // The read has gone out and has not come back.
        let (_tx, rx) = std::sync::mpsc::channel();
        app.sync.repo_job = Some(crate::app::RepoJob {
            source: Some(source),
            announce: false,
            rx,
        });

        let shapes = frame(&mut app, &ctx, Vec::new());
        let words: Vec<String> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert!(
            !words
                .iter()
                .any(|w| w.starts_with("reading the repo")
                    || w.starts_with("this repo has no boxes")),
            "the wait is not a row, and an unread repo is not called empty: {words:?}"
        );

        let veil = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == theme::palette().panel.gamma_multiply(VEIL) => {
                    Some(r.rect)
                }
                _ => None,
            })
            .expect("the card wears the veil");
        let spinner = shapes.iter().any(|s| match &s.shape {
            egui::Shape::Path(p) => p.points.iter().all(|pt| veil.contains(*pt)),
            _ => false,
        });
        assert!(spinner, "the spinner rides the veil over {veil:?}");
    }

    #[test]
    fn the_new_box_form_floats_and_closes() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);

        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/pandora-ui-test".to_owned(),
        };
        app.prefs.sources = vec![source.clone()];
        app.sync.new_in = Some(source);

        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame(&mut app, &ctx, Vec::new());
        }
        let words: Vec<String> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert!(
            words.iter().any(|w| w.eq_ignore_ascii_case("new box")),
            "the modal is up: {words:?}"
        );
        assert!(
            words.iter().any(|w| w.contains("octocat/pandora-ui-test")),
            "it says where the box lands"
        );
        assert!(words.iter().any(|w| w == "box name"), "{words:?}");

        // No Cancel button: the dialog's own X, the backdrop and Esc close it.
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(app.sync.new_in.is_none(), "Esc drops the form");
    }

    #[test]
    fn the_home_prefix_reads_as_a_tilde() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        assert_eq!(short_path(&home.join("boxes/work.box")), "~/boxes/work.box");
        assert_eq!(short_path(Path::new("/etc/x")), "/etc/x");
    }

    #[test]
    fn a_working_copy_sits_in_the_recents_beside_a_local_file() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/boxes".to_owned(),
        };
        let inside = store::source_dir(&source.kind, &source.target).join("work.pandora");
        let outside = PathBuf::from("/tmp/plain.pandora");
        app.prefs.sources.push(source.clone());

        assert_eq!(app.source_of(&inside), Some(source));
        assert_eq!(app.source_of(&outside), None);

        app.auth.recents = vec![
            crate::features::auth::model::Recent { path: inside },
            crate::features::auth::model::Recent { path: outside },
        ];
        let listed: Vec<_> = app.auth.recents.iter().map(|r| r.label()).collect();
        assert_eq!(
            listed,
            ["work", "plain"],
            "the file's own name, not the box's"
        );
    }

    fn picker_rect(ctx: &egui::Context) -> egui::Rect {
        ctx.memory(|mem| mem.area_rect(egui::Id::new("github-repo")))
            .expect("the picker is up")
    }

    #[test]
    fn a_modal_keeps_off_the_screen_edges() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.sync.new_in = Some(Source {
            kind: GITHUB.to_owned(),
            target: "octocat/boxes".to_owned(),
        });

        // A screen narrower than the width the dialog asks for.
        let (w, h) = (360.0, 780.0);
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = shot::frame(&ctx, egui::vec2(w, h), Vec::new(), |ui| run(ui, &mut app));
        }

        // The heading is laid inside the frame's own margin, past the room the
        // card keeps: 16 off the screen, 20 of frame margin.
        let gap = crate::shared::ui::chrome::GAP;
        let heading = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "NEW BOX" => {
                    Some(t.galley.rect.translate(t.pos.to_vec2()))
                }
                _ => None,
            })
            .expect("the dialog's heading is painted");
        assert!(
            heading.left() >= gap + 20.0 - 1.0,
            "the card keeps {gap} and its own margin off the screen: {heading:?}",
        );

        // The modal paints its backdrop over the whole window: it is what the
        // modal's area is seen through, so the room the card keeps must not be
        // taken off it.
        let backdrop = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.fill.r() == 0
                        && r.fill.g() == 0
                        && r.fill.b() == 0
                        && r.fill.a() > 0
                        && r.rect.width() >= w =>
                {
                    Some((r.rect, s.clip_rect))
                }
                _ => None,
            })
            .expect("the modal paints its backdrop");
        assert!(
            backdrop.0.left() == 0.0
                && backdrop.0.right() == w
                && backdrop.1.left() == 0.0
                && backdrop.1.right() == w,
            "the backdrop covers the window edge to edge, not {:?} seen through {:?}",
            backdrop.0,
            backdrop.1,
        );
    }

    #[test]
    fn the_repo_picker_holds_its_size_while_loading() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        app.sync.token = zeroize::Zeroizing::new("t".to_owned());
        app.sync.adding = true;

        // The read has gone out and has not come back.
        let (_tx, rx) = std::sync::mpsc::channel();
        app.sync.repo_job = Some(crate::app::RepoJob {
            source: None,
            announce: true,
            rx,
        });
        // The dialog lays its own heading out lazily, so the first frames are not
        // yet its final shape; both states get the same number of them.
        for _ in 0..6 {
            frame(&mut app, &ctx, Vec::new());
        }
        let loading = picker_rect(&ctx).height();
        let searching = search_rect(&ctx).top();

        // The repos land: a second owner adds a chip, and neither the window nor
        // the field under the chips so much as twitches.
        app.sync.repo_job = None;
        app.sync.repo_loaded = true;
        app.sync.repos = vec![
            crate::features::sync::github::Repo {
                full_name: "octocat/one".to_owned(),
                owner: "octocat".to_owned(),
                private: false,
            },
            crate::features::sync::github::Repo {
                full_name: "sponsor/two".to_owned(),
                owner: "sponsor".to_owned(),
                private: false,
            },
        ];
        for _ in 0..6 {
            frame(&mut app, &ctx, Vec::new());
        }
        let landed = picker_rect(&ctx).height();
        let search = search_rect(&ctx).top();

        assert!(
            (landed - loading).abs() < 0.5,
            "the window is already its final size: {loading} while loading, {landed} once they land"
        );
        assert!(
            (search - searching).abs() < 0.5,
            "the select is above the search field and holds its room:\
             {searching} while loading, {search} once the repos land"
        );
    }

    #[test]
    fn a_public_repo_is_listed_but_cannot_be_picked() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.sync.repos = vec![
            crate::features::sync::github::Repo {
                full_name: "octocat/closed".to_owned(),
                owner: "octocat".to_owned(),
                private: true,
            },
            crate::features::sync::github::Repo {
                full_name: "octocat/open".to_owned(),
                owner: "octocat".to_owned(),
                private: false,
            },
        ];

        let size = egui::vec2(360.0, 300.0);
        let mut chosen: Option<String> = None;
        let mut draw = |events: Vec<egui::Event>, chosen: &mut Option<String>| {
            shot::frame(&ctx, size, events, |ui| {
                listing(ui, &mut app, 200.0, chosen);
            })
        };

        let shapes = draw(Vec::new(), &mut chosen);
        let words = shot::words(&shapes);
        assert!(words.iter().any(|w| w == "private"), "{words:?}");
        assert!(
            words
                .iter()
                .any(|w| w == t("public — choose a private repo")),
            "a public repo says why it is out: {words:?}"
        );

        // Press each row in turn: only the private one answers.
        for (name, pickable) in [("octocat/open", false), ("octocat/closed", true)] {
            let at = rect_of(&shapes, name).center();
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            draw(
                vec![egui::Event::PointerMoved(at), button(true)],
                &mut chosen,
            );
            draw(vec![button(false)], &mut chosen);
            assert_eq!(
                chosen.is_some(),
                pickable,
                "{name} answered the press: {chosen:?}"
            );
            chosen = None;
        }
    }

    #[test]
    fn the_repo_picker_fits_the_body_it_is_given() {
        for (w, h) in [(360.0, 640.0), (360.0, 420.0), (800.0, 360.0)] {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut app = App::blank(&ctx);
            app.screen = Screen::Sources;
            app.auth.side = Some(Side::Github);
            app.sync.token = zeroize::Zeroizing::new("t".to_owned());
            app.sync.adding = true;
            app.sync.repo_loaded = true;
            app.sync.repos = vec![
                crate::features::sync::github::Repo {
                    full_name: "octocat/one".to_owned(),
                    owner: "octocat".to_owned(),
                    private: false,
                },
                crate::features::sync::github::Repo {
                    full_name: "sponsor/two".to_owned(),
                    owner: "sponsor".to_owned(),
                    private: false,
                },
            ];
            let mut shapes = Vec::new();
            for _ in 0..8 {
                shapes = shot::frame(&ctx, egui::vec2(w, h), Vec::new(), |ui| run(ui, &mut app));
            }

            // The dialog's own frame: the `radius(12)` panel the card is drawn in.
            let mut frames: Vec<egui::Rect> = Vec::new();
            fn walk(shape: &egui::Shape, out: &mut Vec<egui::Rect>) {
                match shape {
                    egui::Shape::Rect(r) if r.corner_radius == theme::radius(12) => {
                        out.push(r.rect)
                    }
                    egui::Shape::Vec(v) => {
                        for inner in v {
                            walk(inner, out);
                        }
                    }
                    _ => {}
                }
            }
            for s in &shapes {
                walk(&s.shape, &mut frames);
            }
            let frame = *frames.first().expect("the picker paints its frame");

            let body = crate::shared::ui::chrome::body(&ctx);
            assert!(
                frame.min.y >= body.min.y - 0.5 && frame.max.y <= body.max.y + 0.5,
                "the picker is off the body at {w}x{h}: frame {frame:?} vs body {body:?}"
            );
            assert!(
                frame.min.x >= body.min.x - 0.5 && frame.max.x <= body.max.x + 0.5,
                "the picker is off the body at {w}x{h}: frame {frame:?} vs body {body:?}"
            );
        }
    }

    #[test]
    fn a_phones_repo_list_reaches_the_action_row() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        app.sync.token = zeroize::Zeroizing::new("t".to_owned());
        app.sync.adding = true;
        app.sync.repo_loaded = true;
        app.sync.repos = (0..12)
            .map(|i| crate::features::sync::github::Repo {
                full_name: format!("uriach/repo-{i}"),
                owner: "uriach".to_owned(),
                private: false,
            })
            .collect();

        let mut shapes = Vec::new();
        for _ in 0..4 {
            shapes = shot::frame(&ctx, egui::vec2(393.0, 851.0), Vec::new(), |ui| {
                run(ui, &mut app)
            });
        }

        // The first row's painted text rides the scroll area's own clip, which is
        // the viewport the list was given.
        let viewport = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "uriach/repo-0" => Some(s.clip_rect),
                _ => None,
            })
            .expect("the first row is painted");
        let card = picker_rect(&ctx);
        let under = card.max.y - viewport.max.y;
        let target = crate::shared::ui::responsive::target(&ctx);
        // Under the list there are only the air above the action row, the row
        // itself and the frame's own bottom margin.
        let tail = 12.0 + ctx.style_of(ctx.theme()).spacing.item_spacing.y + target + 18.0;
        assert!(
            under <= tail + 2.0,
            "the list gives up {under} under itself, more than the {tail} of its tail:\
             rows stop at {viewport:?} in a card ending at {card:?}"
        );
        assert!(
            viewport.height() > card.height() / 2.0,
            "the scroll is the card, not a band in it: {viewport:?} in {card:?}"
        );
    }

    fn search_rect(ctx: &egui::Context) -> egui::Rect {
        ctx.read_response(egui::Id::new("repo-search"))
            .expect("the search field is up")
            .rect
    }

    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_org_select_filters_the_list() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        app.sync.adding = true;
        app.sync.repo_loaded = true;
        app.sync.repos = vec![
            crate::features::sync::github::Repo {
                full_name: "octocat/one".to_owned(),
                owner: "octocat".to_owned(),
                private: false,
            },
            crate::features::sync::github::Repo {
                full_name: "uriach/two".to_owned(),
                owner: "uriach".to_owned(),
                private: false,
            },
            crate::features::sync::github::Repo {
                full_name: "uriach/three".to_owned(),
                owner: "uriach".to_owned(),
                private: false,
            },
        ];

        // The dialog needs a frame to place itself before it paints.
        for _ in 0..2 {
            frame(&mut app, &ctx, Vec::new());
        }
        let shot = frame(&mut app, &ctx, Vec::new());
        assert!(
            rect_of(&shot, ALL).bottom() < search_rect(&ctx).top(),
            "the orgs pick the list, the search narrows what is left"
        );
        assert!(
            texts(&shot).contains(&ALL.to_owned()),
            "the select says what is picking the list"
        );

        click(&mut app, &ctx, ALL);
        let open = texts(&frame(&mut app, &ctx, Vec::new()));
        assert!(
            open.contains(&"uriach".to_owned()),
            "the orgs are one entry each, and one org is one entry"
        );

        click(&mut app, &ctx, "uriach");
        assert_eq!(
            app.sync.repo_search, "uriach/",
            "picking an org types it into the field"
        );

        let narrowed = texts(&frame(&mut app, &ctx, Vec::new()));
        assert!(narrowed.contains(&"uriach/two".to_owned()));
        assert!(
            !narrowed.contains(&"octocat/one".to_owned()),
            "the other org is filtered out"
        );
        assert!(
            narrowed.contains(&"uriach".to_owned()),
            "and the select reads back what it is holding"
        );

        // Back through the select: the button wears the org now, so `All repos`
        // is only in the list it opens.
        click(&mut app, &ctx, "uriach");
        click(&mut app, &ctx, ALL);
        assert_eq!(
            app.sync.repo_search, "",
            "`All repos` lets the rest back in"
        );
        let widened = texts(&frame(&mut app, &ctx, Vec::new()));
        assert!(widened.contains(&"octocat/one".to_owned()));
    }

    fn painted(shapes: &[egui::epaint::ClippedShape], fill: egui::Color32) -> egui::Rect {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == fill => Some(r.rect),
                _ => None,
            })
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap_or_else(|| panic!("a rect painted in {fill:?}"))
    }

    #[test]
    fn the_github_view_keeps_its_tail_inside_its_card() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        app.sync.token = zeroize::Zeroizing::new("t".to_owned());
        let carbon = Source {
            kind: GITHUB.to_owned(),
            target: "uriach/carbon".to_owned(),
        };
        app.prefs.sources = vec![
            carbon.clone(),
            Source {
                kind: GITHUB.to_owned(),
                target: "Jota-Music/jota".to_owned(),
            },
        ];
        let dir = store::source_dir(&carbon.kind, &carbon.target);
        fs::create_dir_all(&dir).unwrap();
        for file in ["bank.pandora", "personal.pandora", "work.pandora"] {
            fs::write(dir.join(file), b"locked").unwrap();
        }

        let mut shapes = Vec::new();
        for _ in 0..4 {
            shapes = frame(&mut app, &ctx, Vec::new());
        }
        let card = painted(&shapes, theme::palette().panel);
        let create = painted(&shapes, theme::ACCENT);
        assert!(
            create.height() < 48.0,
            "the way to create a box is the one accent block: {create:?}"
        );
        assert!(
            card.max.y <= 700.0 - BOTTOM,
            "the card holds the window it is drawn in, and leaves the floor alone: {card:?}"
        );
        assert!(
            card.contains_rect(create),
            "the way to create a box sits on the card, not under it: {card:?} {create:?}"
        );
        assert!(
            card.contains_rect(rect_of(&shapes, "Keep me signed in")),
            "and so does the toggle under it"
        );
        assert!(
            rect_of(&shapes, "uriach/carbon").bottom() < rect_of(&shapes, "bank").top(),
            "the select picks the list, above the boxes it holds"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrapped_empty_hint_keeps_the_repo_tail_on_the_card() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/a-rather-long-repo-name".to_owned(),
        };
        app.prefs.sources = vec![source.clone()];

        // A phone's width, and a repo read with nothing in it.
        let narrow = egui::vec2(380.0, 700.0);
        app.sync.token = zeroize::Zeroizing::new("t".to_owned());
        app.sync.listing.insert(
            crate::features::sync::logic::source_key(&source),
            Vec::new(),
        );
        let mut shapes = Vec::new();
        for _ in 0..4 {
            shapes = shot::frame(&ctx, narrow, Vec::new(), |ui| run(ui, &mut app));
        }
        let words = texts(&shapes);
        assert!(
            words
                .iter()
                .any(|w| w == "this repo has no boxes yet — create one below"),
            "the empty line is up: {words:?}"
        );
        let card = painted(&shapes, theme::palette().panel);
        assert!(
            card.max.y <= narrow.y - BOTTOM,
            "the card keeps the floor gap after the line wraps: {card:?}"
        );
        assert!(
            card.contains_rect(painted(&shapes, theme::ACCENT)),
            "the way to create a box stays on the card: {card:?}"
        );
        assert!(
            card.contains_rect(rect_of(&shapes, "Keep me signed in")),
            "and so does the toggle under it: {card:?}"
        );

        // Without a session the line is the longest one there is: it names the
        // copies on this machine and points at the sign-in above.
        app.sync.token = zeroize::Zeroizing::new(String::new());
        app.sync.listing.clear();
        let mut shapes = Vec::new();
        for _ in 0..4 {
            shapes = shot::frame(&ctx, narrow, Vec::new(), |ui| run(ui, &mut app));
        }
        let words = texts(&shapes);
        assert!(
            words
                .iter()
                .any(|w| w
                    == "only the copies saved on this computer — sign in to see the whole repo"),
            "the longest line is up: {words:?}"
        );
        let card = painted(&shapes, theme::palette().panel);
        assert!(
            card.max.y <= narrow.y - BOTTOM,
            "the card keeps the floor gap with the longest line: {card:?}"
        );
        assert!(
            card.contains_rect(rect_of(&shapes, "Sign in with GitHub")),
            "and the way in stays on it: {card:?}"
        );

        // A window too short for the card's own floor gap — a small desktop, not
        // the phone this is written for. The band gives up its room, and the way
        // in and the toggle still ride the card instead of falling off it.
        let short = egui::vec2(380.0, 620.0);
        let mut shapes = Vec::new();
        for _ in 0..4 {
            shapes = shot::frame(&ctx, short, Vec::new(), |ui| run(ui, &mut app));
        }
        let card = painted(&shapes, theme::palette().panel);
        assert!(
            card.contains_rect(rect_of(&shapes, "Sign in with GitHub")),
            "the way in holds the card even when the floor gap does not: {card:?}"
        );
        assert!(
            card.contains_rect(rect_of(&shapes, "Keep me signed in")),
            "and the toggle under it does too: {card:?}"
        );
    }

    #[test]
    fn a_long_repo_name_stays_inside_the_github_card() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.side = Some(Side::Github);
        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/a-very-long-repository-name-that-does-not-fit".to_owned(),
        };
        // Signed, so both marks are live, but already read, so no request goes
        // out over the network while the row is laid out.
        app.sync.token = zeroize::Zeroizing::new("t".to_owned());
        app.sync.read_at.insert(
            crate::features::sync::logic::source_key(&source),
            Instant::now(),
        );
        app.prefs.sources = vec![source];

        let mut shapes = Vec::new();
        for _ in 0..4 {
            shapes = shot::frame(&ctx, egui::vec2(393.0, 851.0), Vec::new(), |ui| {
                run(ui, &mut app)
            });
        }

        let card = painted(&shapes, theme::palette().panel);
        // The select and the two marks that act on the repo share one line, and
        // that line is the one this is about: the select's own label finds it.
        let row = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text().starts_with("octocat/") => {
                    Some(s.shape.visual_bounding_rect().center().y)
                }
                _ => None,
            })
            .expect("the repo's name is in the select");
        let controls: Vec<egui::Rect> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.fill == theme::palette().elev
                        && (r.rect.center().y - row).abs() < responsive::target(&ctx) / 2.0 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            controls.len(),
            3,
            "the select and its two marks are all on the line: {controls:?}"
        );
        for control in controls {
            assert!(
                card.contains_rect(control),
                "a control leaves the card: {control:?} in {card:?}"
            );
        }
    }

    #[test]
    fn the_recents_card_keeps_the_floor_gap_of_the_local_view() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = App::blank(&ctx);
        app.screen = Screen::Sources;
        app.auth.recents = (0..12)
            .map(|i| crate::features::auth::model::Recent {
                path: PathBuf::from(format!("/tmp/floor{i}.pandora")),
            })
            .collect();

        let card = |app: &mut App, side: Side| {
            app.auth.side = Some(side);
            let mut shapes = Vec::new();
            for _ in 0..2 {
                shapes = frame(app, &ctx, Vec::new());
            }
            painted(&shapes, theme::palette().panel)
        };
        let recents = card(&mut app, Side::Recents);
        let local = card(&mut app, Side::Machine);

        assert!(
            (recents.max.y - local.max.y).abs() < 1.0,
            "recents and local leave a different floor gap: {recents:?} {local:?}"
        );
    }
}
