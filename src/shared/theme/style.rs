//! Design tokens — two schemes, austere. One place to tune the whole look.
//!
//! The palette is read through the accessors below and not from constants, because
//! the scheme is chosen at runtime: `shared::i18n` solved the same problem for the
//! language, and this follows its shape exactly — an enum, one `AtomicU8`, and a
//! `set` that the settings screen calls. Nothing else in the app knows the scheme;
//! it asks for `theme::palette().panel` and gets whichever one is in force.

use std::sync::atomic::{AtomicU8, Ordering};

use egui::{Color32, CornerRadius, FontFamily, FontId, RichText, TextEdit, TextStyle, Visuals};
use serde::{Deserialize, Serialize};

use super::icons::{Icon, Icons};
use crate::shared::ui::balance;

/// Which way the app is painted. The name is a word the catalogs carry, so it
/// changes with the language like every other label.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    #[default]
    Dark = 0,
    Light = 1,
}

impl Scheme {
    pub const ALL: [Self; 2] = [Self::Dark, Self::Light];

    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => crate::shared::i18n::t("Dark"),
            Self::Light => crate::shared::i18n::t("Light"),
        }
    }
}

static SCHEME: AtomicU8 = AtomicU8::new(Scheme::Dark as u8);

pub fn set(scheme: Scheme) {
    SCHEME.store(scheme as u8, Ordering::Relaxed);
}

pub fn scheme() -> Scheme {
    match SCHEME.load(Ordering::Relaxed) {
        1 => Scheme::Light,
        _ => Scheme::Dark,
    }
}

/// Every colour the app paints with. One struct rather than two lists of constants,
/// so the two schemes cannot drift apart in shape — only in value.
#[derive(Clone, Copy)]
pub struct Palette {
    pub paper: Color32,
    pub panel: Color32,
    pub elev: Color32,
    pub hair: Color32,
    pub hover: Color32,
    pub rail: Color32,
    pub wash: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent_dim: Color32,
    pub accent_deep: Color32,
    pub error: Color32,
    pub ok: Color32,
    pub note: Color32,
    pub file: Color32,
    pub otp: Color32,
    pub shadow: egui::Shadow,
    /// The veil a drag-and-drop lays over the window: dark enough to read the
    /// message on, light enough to still see the window under it.
    pub scrim: Color32,
}

const DARK: Palette = Palette {
    paper: Color32::from_rgb(0x0e, 0x0f, 0x11),
    panel: Color32::from_rgb(0x14, 0x16, 0x19),
    elev: Color32::from_rgb(0x1b, 0x1e, 0x23),
    hair: Color32::from_rgb(0x2a, 0x2e, 0x35),
    hover: Color32::from_rgb(0x25, 0x29, 0x30),
    rail: Color32::from_rgb(0x1f, 0x22, 0x28),
    wash: Color32::from_rgba_premultiplied(10, 10, 10, 10),
    text: Color32::from_rgb(0xe7, 0xe8, 0xea),
    muted: Color32::from_rgb(0x92, 0x98, 0xa3),
    faint: Color32::from_rgb(0x5c, 0x62, 0x6c),
    accent_dim: Color32::from_rgb(0x14, 0x52, 0x2f),
    accent_deep: Color32::from_rgb(0x0f, 0x3d, 0x23),
    error: Color32::from_rgb(0xd9, 0x6b, 0x5f),
    ok: Color32::from_rgb(0x6f, 0xb3, 0xc9),
    note: Color32::from_rgb(0x6f, 0x8f, 0xbf),
    file: Color32::from_rgb(0x9d, 0x8b, 0xd0),
    otp: Color32::from_rgb(0xcf, 0x8b, 0x5e),
    shadow: egui::Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 2,
        color: Color32::from_black_alpha(150),
    },
    scrim: Color32::from_black_alpha(216),
};

/// The light scheme keeps the dark one's relationships, not its numbers: paper is
/// the page, panel the card on it, elev the field inside the card, hair the line
/// between them — and the accent fills turn pale, because they sit behind dark text
/// the way the dark ones sit behind light text.
const LIGHT: Palette = Palette {
    paper: Color32::from_rgb(0xf6, 0xf7, 0xf8),
    panel: Color32::from_rgb(0xff, 0xff, 0xff),
    elev: Color32::from_rgb(0xf0, 0xf2, 0xf4),
    hair: Color32::from_rgb(0xdc, 0xe0, 0xe5),
    hover: Color32::from_rgb(0xe9, 0xec, 0xef),
    rail: Color32::from_rgb(0xe6, 0xe9, 0xed),
    wash: Color32::from_rgba_premultiplied(0, 0, 0, 12),
    text: Color32::from_rgb(0x16, 0x18, 0x1c),
    muted: Color32::from_rgb(0x56, 0x5c, 0x66),
    faint: Color32::from_rgb(0x74, 0x7a, 0x85),
    accent_dim: Color32::from_rgb(0xd3, 0xf0, 0xdf),
    accent_deep: Color32::from_rgb(0xb3, 0xe6, 0xc8),
    error: Color32::from_rgb(0xb3, 0x2e, 0x2e),
    ok: Color32::from_rgb(0x1d, 0x66, 0x7a),
    note: Color32::from_rgb(0x2f, 0x55, 0x94),
    file: Color32::from_rgb(0x5b, 0x45, 0x96),
    otp: Color32::from_rgb(0x8a, 0x4f, 0x1f),
    shadow: egui::Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 1,
        color: Color32::from_black_alpha(40),
    },
    scrim: Color32::from_black_alpha(90),
};

pub fn palette() -> &'static Palette {
    palette_of(scheme())
}

/// One scheme's palette, named rather than looked up. `palette()` is what the app
/// reads; this is what a test reads when it needs both at once.
pub fn palette_of(scheme: Scheme) -> &'static Palette {
    match scheme {
        Scheme::Dark => &DARK,
        Scheme::Light => &LIGHT,
    }
}

/// The accent is the brand's and does not move with the scheme: it is the same
/// green the logo is drawn with, and a test keeps the two in step.
pub const ACCENT: Color32 = Color32::from_rgb(0x00, 0xa6, 0x50);

pub fn radius(r: u8) -> CornerRadius {
    CornerRadius::same(r)
}

pub fn stroke(c: Color32, w: f32) -> egui::Stroke {
    egui::Stroke::new(w, c)
}

pub fn tag_color(kind: crate::vault::model::Kind) -> Color32 {
    use crate::vault::model::Kind;
    let p = palette();
    match kind {
        Kind::Login => ACCENT,
        Kind::Note => p.note,
        Kind::File => p.file,
        Kind::Otp => p.otp,
    }
}

/// The phone's own fonts, read off `/system/fonts`, in place of egui's bundled
/// ones: those are ~1 MB of `.rodata` inside the APK, and every Android carries
/// the same faces anyway. The symbol font is not decoration — the UI draws `✎`,
/// `→` and `⇄` as text, and neither Roboto nor Droid Sans Mono has them.
///
/// A file that is not there is not fatal: whatever does read is what the app
/// draws with, and `logcat` names the ones that did not.
#[cfg(target_os = "android")]
fn system_fonts(ctx: &egui::Context) {
    let mut defs = egui::FontDefinitions::default();
    face(
        &mut defs,
        "sans",
        &[
            "/system/fonts/Roboto-Regular.ttf",
            "/system/fonts/NotoSans-Regular.ttf",
            "/product/fonts/NotoSans-Regular.ttf",
        ],
        FontFamily::Proportional,
    );
    face(
        &mut defs,
        "symbols",
        &[
            // Android 11 and up, then the two names the older releases use: the
            // `Subsetted2` file is a 124-glyph sliver with none of what the UI
            // draws, so it is not asked for.
            "/system/fonts/NotoSansSymbols2-Regular.ttf",
            "/system/fonts/NotoSansSymbols-Regular.ttf",
            "/system/fonts/NotoSansSymbols-Regular-Subsetted.ttf",
        ],
        FontFamily::Proportional,
    );
    face(
        &mut defs,
        "mono",
        &[
            "/system/fonts/DroidSansMono.ttf",
            "/system/fonts/RobotoMono-Regular.ttf",
        ],
        FontFamily::Monospace,
    );
    ctx.set_fonts(defs);
}

/// Add the first of `paths` that reads, as one more face of `family`. Later
/// faces are only reached for the glyphs the earlier ones do not have.
///
/// It says what it took, which is the only way to see from a desk that the
/// phone found a font at all (`adb logcat -s pandora`).
#[cfg(target_os = "android")]
fn face(defs: &mut egui::FontDefinitions, name: &str, paths: &[&str], family: FontFamily) {
    let Some((path, bytes)) = paths
        .iter()
        .find_map(|path| std::fs::read(path).ok().map(|bytes| (*path, bytes)))
    else {
        log::warn!("pandora: none of {paths:?} could be read");
        return;
    };
    log::info!("pandora: {name} from {path} ({} bytes)", bytes.len());
    defs.font_data.insert(
        name.to_owned(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );
    defs.families
        .entry(family)
        .or_default()
        .push(name.to_owned());
}

/// The visuals egui's own widgets are painted with, for one scheme. Pure, so both
/// can be read at once — by `apply` here and by a test that checks the light one
/// without flipping the global the rest of the suite runs under.
pub fn visuals(scheme: Scheme) -> Visuals {
    // The base is the scheme's own, so the defaults egui keeps out of sight — a
    // faint widget stroke, an unfocused text edit — are painted for the right
    // background and not corrected one by one here.
    let mut vs = match scheme {
        Scheme::Dark => Visuals::dark(),
        Scheme::Light => Visuals::light(),
    };
    let p = palette_of(scheme);
    vs.panel_fill = p.paper;
    vs.window_fill = p.panel;
    vs.extreme_bg_color = p.elev;
    vs.override_text_color = Some(p.text);
    vs.window_corner_radius = radius(10);
    vs.window_stroke = stroke(p.hair, 1.0);
    vs.window_shadow = p.shadow;
    vs.menu_corner_radius = radius(8);
    vs.selection = egui::style::Selection {
        bg_fill: ACCENT,
        stroke: stroke(p.text, 1.0),
    };
    vs.hyperlink_color = ACCENT;
    vs.interact_cursor = Some(egui::CursorIcon::PointingHand);

    let widget = |bg: Color32| egui::style::WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: bg,
        bg_stroke: egui::Stroke::NONE,
        fg_stroke: stroke(palette().faint, 1.0),
        corner_radius: radius(6),
        expansion: 0.0,
    };
    vs.widgets.noninteractive = widget(palette().panel);
    vs.widgets.inactive = widget(palette().elev);
    vs.widgets.hovered = egui::style::WidgetVisuals {
        bg_fill: palette().hover,
        weak_bg_fill: palette().hover,
        bg_stroke: egui::Stroke::NONE,
        fg_stroke: stroke(palette().text, 1.0),
        corner_radius: radius(6),
        expansion: 0.0,
    };
    vs.widgets.active = egui::style::WidgetVisuals {
        bg_fill: palette().accent_dim,
        weak_bg_fill: palette().accent_dim,
        bg_stroke: egui::Stroke::NONE,
        fg_stroke: stroke(palette().text, 1.0),
        corner_radius: radius(6),
        expansion: 0.0,
    };
    vs.widgets.open = vs.widgets.active;
    vs
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals(visuals(scheme()));

    #[cfg(target_os = "android")]
    system_fonts(ctx);

    ctx.global_style_mut(|style| {
        // Solid scroll bars take their own space instead of floating over content.
        style.spacing.scroll = bar_style();
        style.interaction.selectable_labels = false;
        style.spacing.item_spacing = egui::vec2(12.0, 10.0);
        style.spacing.icon_spacing = 8.0;
        style.spacing.button_padding = egui::vec2(14.0, 7.0);
        style.spacing.interact_size = egui::vec2(64.0, 32.0);
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId {
                    size: 22.0,
                    family: FontFamily::Proportional,
                },
            ),
            (
                TextStyle::Body,
                FontId {
                    size: 15.0,
                    family: FontFamily::Proportional,
                },
            ),
            (
                TextStyle::Button,
                FontId {
                    size: 15.0,
                    family: FontFamily::Proportional,
                },
            ),
            (
                TextStyle::Small,
                FontId {
                    size: 12.5,
                    family: FontFamily::Proportional,
                },
            ),
            (
                TextStyle::Monospace,
                FontId {
                    size: 14.0,
                    family: FontFamily::Monospace,
                },
            ),
        ]
        .into_iter()
        .collect();
    });
}

pub fn field_label(ui: &mut egui::Ui, text: &str) {
    balance::label(
        ui,
        RichText::new(crate::shared::i18n::text(text).to_uppercase())
            .small()
            .color(palette().muted),
    );
}

pub fn hint(ui: &mut egui::Ui, text: &str) {
    balance::label(
        ui,
        RichText::new(crate::shared::i18n::text(text).into_owned())
            .small()
            .color(palette().faint),
    );
}

/// egui paints the tick from the widget's `fg_stroke` and the box from the widget
/// fill, both taken from the state's visuals. A checked box at rest stays on the
/// inactive (dull) visuals, so the tick reads flat and the box unfilled. Force the
/// accent fill and the white tick while the style is scoped to the widget, not the
/// whole ui.
///
/// It is an atom layout, not a `Button`, so `Visuals::interact_cursor` never
/// reaches it either: the hand is asked for here.
pub fn check(ui: &mut egui::Ui, on: &mut bool, text: &str) -> egui::Response {
    ui.scope(|ui| {
        if *on {
            let widgets = &mut ui.style_mut().visuals.widgets;
            for w in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
            ] {
                w.bg_fill = ACCENT;
                w.weak_bg_fill = ACCENT;
                w.bg_stroke = stroke(ACCENT, 1.0);
                w.fg_stroke = stroke(Color32::WHITE, 1.5);
            }
        }
        ui.checkbox(on, crate::shared::i18n::text(text).into_owned())
    })
    .inner
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A select. egui builds it out of `ui.interact`, not a `Button`, so the hand comes
/// from here. Returns what the list builds, in case a caller wants it.
pub fn combo<R>(
    ui: &mut egui::Ui,
    combo: egui::ComboBox,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let out = combo.show_ui(ui, add);
    out.response
        .clone()
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    out
}

/// A number to drag. egui's slider senses drags and sets no cursor: the open hand
/// while it is hovered, the closed one while it is held, like every other drag.
///
/// It fills the row it is given. egui sizes a rail at `Spacing::slider_width`,
/// the same 100 pt in a card four times as wide, so a slider on a page reads as an
/// ornament in the middle of it. The rail takes what the value box beside it
/// leaves of the row. Every slider in the app stands on a row of its own, so what
/// is left of the row is the slider's.
///
/// `widest` is the longest text the value can show — "3600 s", "50 attempts" —
/// and it is the room the rail gives up. It has to be the widest and not the
/// current one: a rail that shrank as the number grew would move the handle under
/// the pointer that is dragging it, and egui lays the value out after the rail.
pub fn slider(ui: &mut egui::Ui, widest: &str, slider: egui::Slider<'_>) -> egui::Response {
    let resp = ui
        .scope(|ui| {
            // The value is a `DragValue` in the same row, no narrower than
            // `interact_size.x` and never narrower than its own text plus the
            // button padding it wears.
            let text = ui
                .painter()
                .layout_no_wrap(
                    widest.to_owned(),
                    TextStyle::Button.resolve(ui.style()),
                    palette().text,
                )
                .size()
                .x;
            let value = (text + 2.0 * ui.spacing().button_padding.x)
                .max(ui.spacing().interact_size.x)
                + ui.spacing().item_spacing.x;
            ui.spacing_mut().slider_width = (ui.available_width() - value).max(MIN_RAIL);
            ui.add(slider)
        })
        .inner;
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else {
        resp.clone().on_hover_cursor(egui::CursorIcon::Grab);
    }
    resp
}

/// The shortest rail a slider may be squeezed to: in a row with no room left, a
/// rail shorter than this is a dot and the value beside it is the whole control.
const MIN_RAIL: f32 = 60.0;

/// The scroll bar egui paints is its own widget, not a `Button`, so the hand has to
/// be asked for here: open while the bar is hovered, closed while it is dragged. An
/// area with nowhere to go never registers a bar — only the hand-painted rail — so a
/// list that fits takes no cursor and no drag.
fn bar_cursor(ui: &egui::Ui, area: egui::Id) {
    for axis in 0..2 {
        let Some(bar) = ui.ctx().read_response(area.with(axis)) else {
            continue;
        };
        if bar.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if bar.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
}

pub fn empty(ui: &mut egui::Ui, searching: bool, title: &str, sub: &str) {
    ui.add_space(34.0);
    ui.vertical_centered(|ui| {
        if searching {
            balance::label(
                ui,
                RichText::new(
                    crate::shared::i18n::text("nothing matches your search").into_owned(),
                )
                .color(palette().muted),
            );
        } else {
            balance::label(
                ui,
                RichText::new(crate::shared::i18n::text(title).into_owned()).color(palette().muted),
            );
            balance::label(
                ui,
                RichText::new(crate::shared::i18n::text(sub).into_owned())
                    .small()
                    .color(palette().faint),
            );
        }
    });
}

/// `id` is the one `Ctrl+F` hands the keyboard to.
pub fn search(
    ui: &mut egui::Ui,
    icons: &Icons,
    query: &mut String,
    id: egui::Id,
    placeholder: &str,
) {
    ui.horizontal(|ui| {
        icons
            .image(
                Icon::Search,
                crate::shared::ui::responsive::glyph(ui.ctx()),
                palette().faint,
            )
            .map(|img| ui.add(img));
        ui.add(
            egui::TextEdit::singleline(query)
                .id(id)
                .hint_text(crate::shared::i18n::text(placeholder).into_owned())
                .desired_width(f32::INFINITY)
                .margin(egui::Margin::symmetric(8, 7)),
        );
    });
}

pub fn pane(compact: bool) -> egui::Frame {
    let gutter = if compact { 14 } else { 20 };
    egui::Frame::NONE
        .fill(palette().paper)
        .inner_margin(egui::Margin::symmetric(gutter, gutter))
}

/// An empty `title` draws no heading: a card that is the page's own subject does
/// not need to be named. The response carries the rect the card took, which is
/// what a loading veil is painted over.
pub fn card<R>(
    ui: &mut egui::Ui,
    title: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    if !title.is_empty() {
        // The card's own width, read from the left: a heading is part of the card,
        // not of whatever column the page hangs it on.
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| field_label(ui, title),
        );
        ui.add_space(8.0);
    }
    egui::Frame::default()
        .fill(palette().panel)
        .corner_radius(radius(10))
        .stroke(stroke(palette().hair, 1.0))
        .inner_margin(egui::Margin::symmetric(20, 18))
        .show(ui, |ui| {
            // Takes the column it is given whatever it holds: a card the width
            // of its own slider is not a card.
            ui.set_min_width(ui.available_width());
            add(ui)
        })
}

/// The dialog frame's own margin, each side of its content.
const MARGIN: i8 = 20;

/// The dialog frame's margin over and under its content.
const PAD: i8 = 18;

/// What the dialog frame adds to one side of its content: that margin, plus the
/// hairline the window stroke draws outside it. `width` leaves it out of what a
/// dialog asks for, so the frame a capped width lands in still fits the body whole.
const SIDE: f32 = MARGIN as f32 + 1.0;

/// `SIDE` stood on its head, for the height: a dialog asks for what fits inside
/// the frame that will hold it, not for the frame itself.
const VERT: f32 = PAD as f32 + 1.0;

/// The width a centred form column asks for: the settings cards and the password
/// generator both hang on this one. A form page is one column of these, never two.
pub const COLUMN: f32 = 460.0;

/// The width a dialog asks for, capped to what the window holds, less the room
/// the card keeps off its edge and the frame's own sides: past that its frame is
/// pinned to the screen's edge, and a phone's window is the whole screen.
pub fn width(ui: &mut egui::Ui, want: f32) {
    let body = crate::shared::ui::chrome::body(ui.ctx()).width();
    let room = body - crate::shared::ui::chrome::GAP * 2.0 - SIDE * 2.0;
    let want = if crate::shared::ui::responsive::compact(ui.ctx()) {
        room
    } else {
        want
    };
    ui.set_width(want.min(room.max(0.0)));
}

/// The height a dialog asks for, capped by the same room a card keeps off the
/// window's edge. A ceiling, not a size: a dialog with little to hold stays short,
/// and what is longer than the box scrolls inside it. A floor as well would make
/// every dialog as tall as its share of the window whether or not it had anything
/// to put there.
pub fn height(ui: &mut egui::Ui, want: f32) {
    let body = crate::shared::ui::chrome::body(ui.ctx()).height();
    let room = body - crate::shared::ui::chrome::GAP * 2.0 - VERT * 2.0;
    let want = if crate::shared::ui::responsive::compact(ui.ctx()) {
        room
    } else {
        want
    };
    ui.set_max_height(want.min(room.max(0.0)));
}

/// The framed dialog every floating box shares; `add` returns true when it closed
/// itself (its own cross). The backdrop, Esc and Enter close it too. Enter is the
/// OK: the edits are live, so closing is accepting. A single-line field ends its
/// input on the key and lets the focus go, so the freed key closes; a multiline
/// field keeps the focus and takes the key for the newline, a focused button
/// clicks itself first, and an open popup answers it.
pub fn dialog(ctx: &egui::Context, id: egui::Id, add: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
    let mut open = true;
    let resp = crate::shared::ui::chrome::modal(ctx, id)
        .frame(
            egui::Frame::window(&ctx.style_of(ctx.theme()))
                .corner_radius(radius(12))
                .inner_margin(egui::Margin::symmetric(MARGIN, PAD)),
        )
        .show(ctx, |ui| {
            if add(ui) {
                open = false;
            }
        });
    if resp.should_close() {
        open = false;
    }
    if resp.is_top_modal
        && !resp.any_popup_open
        && ctx.memory(|m| m.focused().is_none())
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
    {
        open = false;
    }
    open
}

/// The action row every dialog ends with: the accept on the right, the safe answer
/// beside it, and the safe one holding the keyboard when nothing else does — a
/// stray Enter or Tab must not be what confirms. Returns `(accepted, safe)`.
pub fn actions(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    accept: impl FnOnce(&mut egui::Ui) -> egui::Response,
    safe: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> (bool, bool) {
    let (mut accepted, mut refused) = (false, false);
    let compact = crate::shared::ui::responsive::compact(ctx);
    if compact {
        accepted = accept(ui).clicked();
        ui.add_space(6.0);
        let button = ui
            .scope(|ui| {
                let w = &mut ui.style_mut().visuals.widgets;
                w.active.bg_fill = w.inactive.bg_fill;
                w.active.weak_bg_fill = w.inactive.weak_bg_fill;
                w.active.bg_stroke = w.inactive.bg_stroke;
                safe(ui)
            })
            .inner;
        if ctx.memory(|m| m.focused().is_none()) {
            button.request_focus();
        }
        focus(ui, &button);
        refused = button.clicked();
    } else {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            accepted = accept(ui).clicked();
            let button = ui
                .scope(|ui| {
                    let w = &mut ui.style_mut().visuals.widgets;
                    w.active.bg_fill = w.inactive.bg_fill;
                    w.active.weak_bg_fill = w.inactive.weak_bg_fill;
                    w.active.bg_stroke = w.inactive.bg_stroke;
                    safe(ui)
                })
                .inner;
            if ctx.memory(|m| m.focused().is_none()) {
                button.request_focus();
            }
            focus(ui, &button);
            refused = button.clicked();
        });
    }
    (accepted, refused)
}

/// The dialog's own cross, right-aligned, as every header carries one. Returns
/// true when it was clicked.
pub fn close(ui: &mut egui::Ui, icons: &Icons) -> bool {
    let mut closed = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        closed = icons
            .button(ui, Icon::X, 15.0, palette().muted, "close")
            .clicked();
    });
    closed
}

/// The dialog's own header: its name on the left, the cross on the right. Returns
/// true when the cross was clicked.
pub fn head(ui: &mut egui::Ui, icons: &Icons, title: &str) -> bool {
    let mut closed = false;
    ui.horizontal(|ui| {
        field_label(ui, title);
        closed = close(ui, icons);
    });
    ui.add_space(12.0);
    closed
}

/// The header a record editor wears: its own face on the left, its name in the
/// place a title would take, whatever trails the name, and the cross. Returns
/// `(closed, changed, name_response)`.
pub fn record_head(
    ui: &mut egui::Ui,
    icons: &Icons,
    dialog: bool,
    face: impl FnOnce(&mut egui::Ui),
    name: &mut String,
    hint: &str,
    trail: impl FnOnce(&mut egui::Ui),
) -> (bool, bool, egui::Response) {
    let (mut closed, mut changed) = (false, false);
    let mut response = None;
    let field = |ui: &mut egui::Ui, name: &mut String, hint: &str| {
        ui.add(
            TextEdit::singleline(name)
                .hint_text(crate::shared::i18n::t(hint))
                .desired_width(ui.available_width())
                .margin(egui::Margin::symmetric(10, 7)),
        )
    };
    if crate::shared::ui::responsive::compact(ui.ctx()) {
        ui.horizontal(|ui| {
            face(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if dialog {
                    closed = close(ui, icons);
                }
                trail(ui);
            });
        });
        ui.add_space(8.0);
        let r = field(ui, name, hint);
        changed = r.changed();
        response = Some(r);
    } else {
        ui.horizontal(|ui| {
            face(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if dialog {
                    closed = close(ui, icons);
                }
                trail(ui);
                let r = field(ui, name, hint);
                changed = r.changed();
                response = Some(r);
            });
        });
    }
    ui.add_space(12.0);
    (
        closed,
        changed,
        response.expect("the name field is painted"),
    )
}

/// Strips a bar's widgets flat; touches only the child style, never the app's.
pub fn flat(ui: &mut egui::Ui) {
    let widgets = &mut ui.style_mut().visuals.widgets;
    for (w, bg) in [
        (&mut widgets.inactive, Color32::TRANSPARENT),
        (&mut widgets.hovered, palette().wash),
        (&mut widgets.active, palette().wash),
        (&mut widgets.open, Color32::TRANSPARENT),
    ] {
        w.corner_radius = CornerRadius::ZERO;
        w.bg_fill = bg;
        w.weak_bg_fill = bg;
    }
}

/// Run `add` with the chosen-accent fills scoped to it: the dim accent at rest,
/// one step deeper under the pointer and while pressed. `Button::fill` pins every
/// state to one colour and egui's own hovered fill is grey, so a control that only
/// sets its resting fill flashes grey on hover. The widget visuals are put back
/// afterwards, so a grid or a combo can scope one cell without touching the rest.
pub fn chosen<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let restore = ui.style().visuals.widgets.clone();
    let w = &mut ui.style_mut().visuals.widgets;
    w.inactive.bg_fill = palette().accent_dim;
    w.inactive.weak_bg_fill = palette().accent_dim;
    for v in [&mut w.hovered, &mut w.active] {
        v.bg_fill = palette().accent_deep;
        v.weak_bg_fill = palette().accent_deep;
    }
    let out = add(ui);
    ui.style_mut().visuals.widgets = restore;
    out
}

/// egui's menu style keeps the form's roomy `item_spacing`, which grows a
/// submenu's trigger rect over its neighbours (`item_spacing / 2`).
pub fn menu(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
    ui.spacing_mut().button_padding = egui::vec2(8.0, 6.0);
    ui.set_min_width(200.0);
}

/// Keyboard focus on a control painted by hand: the accent ring, and a scroll that
/// keeps it on screen. A pointer click never moves the keyboard focus in egui, so
/// the ring only ever answers Tab, the arrows and a screen reader.
pub fn focus(ui: &egui::Ui, resp: &egui::Response) {
    if !resp.has_focus() {
        return;
    }
    if resp.gained_focus() {
        resp.scroll_to_me(Some(egui::Align::Center));
    }
    ui.painter().rect_stroke(
        resp.rect.expand(1.0),
        radius(6),
        stroke(ACCENT, 2.0),
        egui::StrokeKind::Inside,
    );
}

/// Hand a text field the keyboard with its text already selected, so the first
/// keystroke replaces the placeholder instead of appending to it.
pub fn select_all(ui: &egui::Ui, resp: &egui::Response, len: usize) {
    resp.request_focus();
    if let Some(mut state) = egui::widgets::text_edit::TextEditState::load(ui.ctx(), resp.id) {
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(len),
            )));
        state.store(ui.ctx(), resp.id);
    }
}

pub fn marked(ui: &egui::Ui, resp: egui::Response, edge: Edge) -> egui::Response {
    let color = if resp.is_pointer_button_down_on() || resp.has_focus() {
        ACCENT
    } else if resp.hovered() {
        palette().faint
    } else {
        return resp;
    };
    let bar = ui.max_rect();
    let rect = resp.rect;
    let band = match edge {
        Edge::Floor => egui::Rect::from_min_max(
            egui::pos2(rect.min.x, bar.max.y - 2.0),
            egui::pos2(rect.max.x, bar.max.y),
        ),
        Edge::Ceiling => egui::Rect::from_min_max(
            egui::pos2(rect.min.x, bar.min.y),
            egui::pos2(rect.max.x, bar.min.y + 2.0),
        ),
    };
    ui.painter().rect_filled(band, CornerRadius::ZERO, color);
    resp
}

/// Reserve it before the content is laid out, paint it after: painting without
/// the reservation would cover the label.
pub struct Surface {
    bg: egui::layers::ShapeIdx,
    edge: egui::layers::ShapeIdx,
}

impl Surface {
    pub fn reserve(ui: &mut egui::Ui) -> Self {
        Self {
            bg: ui.painter().add(egui::Shape::Noop),
            edge: ui.painter().add(egui::Shape::Noop),
        }
    }

    pub fn paint(
        self,
        ui: &egui::Ui,
        rect: egui::Rect,
        radius: CornerRadius,
        fill: Color32,
        line: egui::Stroke,
    ) {
        ui.painter()
            .set(self.bg, egui::Shape::rect_filled(rect, radius, fill));
        ui.painter().set(
            self.edge,
            egui::Shape::rect_stroke(rect, radius, line, egui::StrokeKind::Inside),
        );
    }
}

pub fn state(selected: bool, hovered: bool) -> (Color32, egui::Stroke) {
    if selected {
        (palette().accent_dim, stroke(ACCENT, 1.0))
    } else if hovered {
        (palette().elev, stroke(palette().hair, 1.0))
    } else {
        (palette().panel, stroke(palette().hair, 1.0))
    }
}

/// Which edge of a bar its slots mark: the one the body is on, so the rule reads
/// as belonging to the tab on show. A bar at the window's top marks its floor, the
/// one at a phone's floor marks its ceiling.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Floor,
    Ceiling,
}

impl Edge {
    /// The two-pixel band a slot's rule lives in.
    fn rule(self, rect: egui::Rect) -> egui::Rect {
        match self {
            Self::Floor => egui::Rect::from_min_max(
                egui::pos2(rect.min.x, rect.max.y - 2.0),
                egui::pos2(rect.max.x, rect.max.y),
            ),
            Self::Ceiling => {
                egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.min.y + 2.0))
            }
        }
    }

    /// The one pixel the hover wash keeps off the same edge.
    fn wash(self, rect: egui::Rect) -> egui::Rect {
        match self {
            Self::Floor => rect.with_max_y(rect.max.y - 1.0),
            Self::Ceiling => rect.with_min_y(rect.min.y + 1.0),
        }
    }
}

/// One slot of a tab strip: the wash on hover and the accent rule under the one
/// on show. It reserves the slot before its content, so both land behind it, and
/// the rect comes from the allocation, so it sits right in either layout
/// direction. `height` is the strip's, so every screen's tabs are the same shape.
pub fn slot(
    ui: &mut egui::Ui,
    width: f32,
    id: egui::Id,
    on: bool,
    height: f32,
    edge: Edge,
    content: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let bg = ui.painter().add(egui::Shape::Noop);
    let rule = ui.painter().add(egui::Shape::Noop);

    let rect = ui
        .allocate_ui_with_layout(
            egui::vec2(width, height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                // Keeps the strip's advance and the slot's own rect in step.
                ui.set_min_width(width);
                content(ui);
            },
        )
        .response
        .rect;

    let resp = if ui.is_enabled() {
        ui.interact(rect, id, egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        ui.interact(rect, id, egui::Sense::hover())
    };
    focus(ui, &resp);
    if ui.is_enabled() && resp.hovered() && !on {
        ui.painter().set(
            bg,
            egui::Shape::rect_filled(edge.wash(rect), CornerRadius::ZERO, palette().wash),
        );
    }
    let mark = if !ui.is_enabled() {
        None
    } else if on || resp.is_pointer_button_down_on() {
        Some(ACCENT)
    } else if resp.hovered() {
        Some(palette().faint)
    } else {
        None
    };
    if let Some(color) = mark {
        ui.painter().set(
            rule,
            egui::Shape::rect_filled(edge.rule(rect), CornerRadius::ZERO, color),
        );
    }
    resp
}

fn spinner_id() -> egui::Id {
    egui::Id::new("primary-button-spinner")
}

/// A form's primary action: the full-width, accent-filled button. The pointer
/// deepens the fill to the palette's `accent_dim` and a press to `accent_deep`.
/// `Button::fill`
/// would pin every state to one colour, so the fills ride the widget visuals
/// instead. With `busy` the icon slot becomes a spinner,
/// and it stays full strength because a disabled widget would be faded to half
/// alpha, the accent fill included. It centres its own label, since a wide button
/// in a left-aligned column would pin it to the edge. The busy branch senses hover
/// only, so its fill comes from the noninteractive visuals.
pub fn primary_button(
    ui: &mut egui::Ui,
    icons: &Icons,
    icon: Option<Icon>,
    text: &str,
    busy: Option<&str>,
) -> egui::Response {
    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
        let mut atoms = icons.trailing(
            icon.filter(|_| busy.is_none()),
            15.0,
            palette().paper,
            RichText::new(crate::shared::i18n::text(busy.unwrap_or(text)).into_owned())
                .strong()
                .color(palette().paper),
        );
        if busy.is_some() {
            atoms.push_left(egui::Atom::custom(spinner_id(), egui::vec2(15.0, 15.0)));
        }
        let widgets = &mut ui.style_mut().visuals.widgets;
        for w in [&mut widgets.noninteractive, &mut widgets.inactive] {
            w.bg_fill = ACCENT;
            w.weak_bg_fill = ACCENT;
        }
        widgets.hovered.bg_fill = palette().accent_dim;
        widgets.hovered.weak_bg_fill = palette().accent_dim;
        widgets.active.bg_fill = palette().accent_deep;
        widgets.active.weak_bg_fill = palette().accent_deep;
        let button = egui::Button::new(atoms).min_size(egui::vec2(ui.available_width(), 40.0));
        if busy.is_none() {
            return ui.add(button);
        }
        // `Sense::hover` and not a disabled ui: the same shape, minus the clicks.
        let out = button.sense(egui::Sense::hover()).atom_ui(ui);
        if let Some(rect) = out.rect(spinner_id()) {
            egui::Spinner::new()
                .size(15.0)
                .color(palette().paper)
                .paint_at(ui, rect);
        }
        out.response
    })
    .inner
}

/// The quiet twin of [`primary_button`], built the same way so a pair of them
/// read as one row: full width, one height, its own label centred.
pub fn secondary_button(
    ui: &mut egui::Ui,
    icons: &Icons,
    icon: Option<Icon>,
    text: &str,
) -> egui::Response {
    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
        let atoms = icons.trailing(
            icon,
            15.0,
            palette().muted,
            RichText::new(crate::shared::i18n::text(text).into_owned())
                .strong()
                .color(palette().text),
        );
        ui.add(
            egui::Button::new(atoms)
                .stroke(stroke(palette().hair, 1.0))
                .min_size(egui::vec2(ui.available_width(), 40.0)),
        )
    })
    .inner
}

/// The width of the bar itself. egui's floating bar is thin at rest and widens
/// under the pointer; ours is one width in both states, so nothing the pointer
/// touches moves.
const BAR_WIDTH: f32 = 4.0;

/// Air between the content and the bar, held on both sides of the gutter: a
/// solid bar sits this far off the content, a band keeps the same gap whether or
/// not the bar is up.
const BAR_GAP: f32 = 12.0;

/// The gutter a band and a page keep for their bar: `BAR_GAP + BAR_WIDTH +
/// bar_outer_margin`. Held out of the content whether or not the bar is up, so
/// neither reflows when it comes up (CSS `scrollbar-gutter: stable`) — and the bar
/// stays up in it (`band_with`), so the room never reads as a gap.
pub const BAR: f32 = BAR_GAP + BAR_WIDTH + 4.0;

/// The app's one bar: one width at rest and under the pointer. The global style
/// and every band share it; `band_style` is the one that floats it in the gutter
/// it was handed. The extension matches the same numbers in CSS.
fn bar_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        bar_width: BAR_WIDTH,
        bar_inner_margin: BAR_GAP,
        bar_outer_margin: 4.0,
        ..egui::style::ScrollStyle::solid()
    }
}

/// Floating bar style: the bar every band and page rides. The gutter rides the
/// *content margin* (`band_with`) and not `floating_allocated_width`: egui takes that
/// width out of the content only while the bar is up, which would reflow a list the
/// moment it grew past the band. The track and the knob hold full strength at rest
/// and under the pointer alike — the bar is a landmark, not a control, so nothing
/// about it moves.
fn band_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        floating: true,
        floating_width: BAR_WIDTH,
        floating_allocated_width: 0.0,
        foreground_color: false,
        dormant_background_opacity: 1.0,
        active_background_opacity: 1.0,
        interact_background_opacity: 1.0,
        dormant_handle_opacity: 1.0,
        active_handle_opacity: 1.0,
        ..bar_style()
    }
}

/// egui paints the bar from the ui that owns the area — the track from
/// `extreme_bg_color`, the handle from the widget fills — so the bar's palette
/// goes on that ui. Returns the ordinary visuals, which the content is put back
/// on, so a field stays a field and a slider keeps its rail.
fn bars(ui: &mut egui::Ui, style: egui::style::ScrollStyle) -> Visuals {
    let surface = ui.visuals().clone();
    let vs = &mut ui.style_mut().visuals;
    vs.extreme_bg_color = palette().rail;
    vs.widgets.inactive.bg_fill = palette().hair;
    vs.widgets.hovered.bg_fill = palette().faint;
    vs.widgets.active.bg_fill = ACCENT;
    ui.style_mut().spacing.scroll = style;
    surface
}

/// Scroll a page: it hands the bar its gutter whether or not there is anywhere to go,
/// so a page that fits and a page that scrolls keep the same content width and the
/// bar never moves the rows beside it.
pub fn scroll<R>(
    ui: &mut egui::Ui,
    area: egui::ScrollArea,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::containers::scroll_area::ScrollAreaOutput<R> {
    band(ui, area, add)
}

/// The bar's style for a page that is allowed to fit: the strip `scroll` holds
/// always is taken only while the bar is up, so a page that never overflows wears
/// no bar and keeps the room for its card.
fn fit_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        floating_allocated_width: BAR,
        ..band_style()
    }
}

/// Scroll a page that may have nothing to scroll. The exception to [`scroll`]:
/// nothing is held for the bar and no rail is drawn while the page fits, so a
/// form that never overflows has no bar at all — a bar down the side of a centred
/// card promises something to scroll that a form usually cannot keep. When the
/// page does overflow, the bar takes the same strip `scroll` keeps and answers a
/// thumb in the same gutter; the page is then laid out as `scroll` lays it, so the
/// content gives up its strip the frame the bar comes up.
pub fn scroll_when_needed<R>(
    ui: &mut egui::Ui,
    area: egui::ScrollArea,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::containers::scroll_area::ScrollAreaOutput<R> {
    let surface = bars(ui, fit_style());
    let room = ui.available_rect_before_wrap();
    let out = area.show(ui, |ui| {
        ui.style_mut().visuals = surface;
        add(ui)
    });
    if out.content_size.y > out.inner_rect.height() {
        bar_cursor(ui, out.id);
        grab(ui, room, &out);
    }
    out
}

/// Scroll a page of uniform rows, drawing only the rows in view. A resize then
/// lays out what the window shows instead of the whole vault.
pub fn scroll_rows<R>(
    ui: &mut egui::Ui,
    area: egui::ScrollArea,
    row_height: f32,
    total_rows: usize,
    add: impl FnOnce(&mut egui::Ui, std::ops::Range<usize>) -> R,
) -> egui::containers::scroll_area::ScrollAreaOutput<R> {
    band_rows(ui, area, row_height, total_rows, add)
}

/// Scroll a band inside a card or a dialog: it keeps the bar's gutter out of its
/// own width whether or not the bar is up, so a long entry never shifts every
/// field above it. A page (`scroll`) is the same treatment over a whole pane.
pub fn band<R>(
    ui: &mut egui::Ui,
    area: egui::ScrollArea,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::containers::scroll_area::ScrollAreaOutput<R> {
    let surface = bars(ui, band_style());
    let room = ui.available_rect_before_wrap();
    let area = area.content_margin(egui::Margin {
        right: BAR as i8,
        ..egui::Margin::ZERO
    });
    let out = area.show(ui, |ui| {
        ui.style_mut().visuals = surface;
        add(ui)
    });
    finish_band(ui, room, out)
}

/// The uniform-row half of [`band`], so a long list draws only the rows the
/// viewport can reach.
pub fn band_rows<R>(
    ui: &mut egui::Ui,
    area: egui::ScrollArea,
    row_height: f32,
    total_rows: usize,
    add: impl FnOnce(&mut egui::Ui, std::ops::Range<usize>) -> R,
) -> egui::containers::scroll_area::ScrollAreaOutput<R> {
    let surface = bars(ui, band_style());
    let room = ui.available_rect_before_wrap();
    let area = area.content_margin(egui::Margin {
        right: BAR as i8,
        ..egui::Margin::ZERO
    });
    let out = area.show_rows(ui, row_height, total_rows, |ui, rows| {
        ui.style_mut().visuals = surface;
        add(ui, rows)
    });
    finish_band(ui, room, out)
}

fn finish_band<R>(
    ui: &mut egui::Ui,
    room: egui::Rect,
    out: egui::containers::scroll_area::ScrollAreaOutput<R>,
) -> egui::containers::scroll_area::ScrollAreaOutput<R> {
    if out.content_size.y > out.inner_rect.height() {
        bar_cursor(ui, out.id);
        grab(ui, room, &out);
    } else {
        let band = out.inner_rect.with_max_y(ui.min_rect().max.y);
        rail(ui, room, band);
    }
    out
}

/// The rail the gutter keeps when there is nowhere to go: the bar's own track, in the
/// place, the width and the corner of the knob egui would paint — and painted by hand
/// because egui paints no bar at all for a list that fits. A knob that cannot move is
/// not a knob, and one drawn under the rail would only show its own antialiased edge.
fn rail(ui: &egui::Ui, room: egui::Rect, band: egui::Rect) {
    let groove = egui::Rect::from_x_y_ranges(
        (room.right() - 4.0 - BAR_WIDTH)..=(room.right() - 4.0),
        band.y_range(),
    )
    .intersect(room);
    if groove.height() > 0.0 {
        ui.painter().rect_filled(
            groove,
            ui.visuals().widgets.inactive.corner_radius,
            palette().rail,
        );
    }
}

/// The knob under a thumb, in the accent a press would give it. The press never
/// reaches egui's bar: this widget sits on top of the eight pixels egui aims at
/// a pointer, so the bar never reads it as held and would paint the knob at
/// rest. The fill is repeated here at the place egui drew the knob, with egui's
/// own geometry — the handle's share of the track, at rest, no shorter than its
/// minimum. Leaving the bar's eight pixels to egui and grabbing only the air
/// beside them would light a thumb that lands exactly on the knob and nothing
/// else, so the whole gutter stays one target.
fn knob<R>(
    ui: &egui::Ui,
    room: egui::Rect,
    out: &egui::containers::scroll_area::ScrollAreaOutput<R>,
) {
    let track = out.inner_rect;
    let travel = (out.content_size.y - track.height()).max(0.0);
    if travel <= 0.0 || out.content_size.y <= 0.0 {
        return;
    }
    let size = (track.height() * track.height() / out.content_size.y)
        .max(ui.spacing().scroll.handle_min_length);
    let start = egui::remap_clamp(
        out.state.offset.y,
        0.0..=travel,
        track.top()..=(track.bottom() - size),
    );
    let grip = egui::Rect::from_x_y_ranges(
        (room.right() - 4.0 - BAR_WIDTH)..=(room.right() - 4.0),
        start..=(start + size),
    );
    let vs = &ui.visuals().widgets.active;
    ui.painter().rect_filled(grip, vs.corner_radius, vs.bg_fill);
}

/// The thumb's grip on the bar, below the breakpoint. egui aims the bar's hit
/// zone at a pointer — the bar's width plus its outer margin, eight pixels
/// here — and no thumb lands on that, so the whole gutter the bar rides
/// answers instead. The drag takes the knob's own travel: the groove's height
/// crosses the whole list wherever on it the finger lands, and nothing about
/// the bar itself moves. The offset is written back into the area's own state,
/// so rows and knob take it together on the next frame. The screen reader is
/// not told: the bar under the strip already names itself. A desk keeps the
/// pointer's own strip and the wheel.
fn grab<R>(
    ui: &mut egui::Ui,
    room: egui::Rect,
    out: &egui::containers::scroll_area::ScrollAreaOutput<R>,
) {
    if !crate::shared::ui::responsive::compact(ui.ctx()) {
        return;
    }
    let gutter = egui::Rect::from_x_y_ranges(
        (room.right() - BAR)..=room.right(),
        out.inner_rect.y_range(),
    )
    .intersect(room);
    if gutter.height() <= 0.0 {
        return;
    }
    let resp = ui.interact(gutter, out.id.with("grab"), egui::Sense::DRAG);
    if resp.is_pointer_button_down_on() {
        knob(ui, room, out);
    }
    if !resp.dragged() {
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        return;
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    let travel = (out.content_size.y - out.inner_rect.height()).max(0.0);
    let ratio = out.content_size.y / out.inner_rect.height().max(1.0);
    let mut state = out.state;
    state.offset.y = (state.offset.y + resp.drag_delta().y * ratio).clamp(0.0, travel);
    state.store(ui.ctx(), out.id);
}

/// Master-password strength: a filled bar plus its label. Advisory, never a gate.
pub fn strength_meter(ui: &mut egui::Ui, pw: &str) {
    use crate::vault::crypto::{strength, Strength};
    if pw.is_empty() {
        return;
    }
    let s = strength(pw);
    let (level, color) = match s {
        Strength::Weak => (1.0, palette().error),
        Strength::Fair => (2.0, palette().ok),
        Strength::Strong => (3.0, ACCENT),
    };
    ui.add(
        egui::ProgressBar::new(level / 3.0)
            .fill(color)
            .corner_radius(2)
            .desired_height(4.0)
            .desired_width(ui.available_width()),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(crate::shared::i18n::t(s.label()))
            .small()
            .color(color),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(ctx: &egui::Context, on: bool) -> (bool, bool) {
        let mut on = on;
        let shapes =
            crate::shared::ui::shot::frame(ctx, egui::vec2(240.0, 60.0), Vec::new(), |ui| {
                check(ui, &mut on, "A-Z");
            });
        let white_tick = shapes.iter().any(|s| match &s.shape {
            egui::Shape::Path(p) => {
                p.stroke.color == egui::epaint::ColorMode::Solid(Color32::WHITE)
            }
            _ => false,
        });
        let green_box = shapes.iter().any(|s| match &s.shape {
            egui::Shape::Rect(r) => r.fill == ACCENT,
            _ => false,
        });
        (white_tick, green_box)
    }

    /// Enter is the dialog's OK: a single-line field ends its input on the key,
    /// so the freed key closes the card; a multiline field keeps the focus and
    /// takes the key for the newline instead.
    #[test]
    fn enter_closes_the_dialog_unless_a_multiline_field_holds_it() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let size = egui::vec2(420.0, 240.0);
        let enter = || {
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]
        };

        let mut text = String::new();
        let mut open = true;
        for (i, events) in [Vec::new(), Vec::new(), enter()].into_iter().enumerate() {
            crate::shared::ui::shot::frame(&ctx, size, events, |ui| {
                open = dialog(ui.ctx(), egui::Id::new("enter-ok"), |ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut text));
                    if i == 0 {
                        r.request_focus();
                    }
                    false
                });
            });
        }
        assert!(!open, "Enter in a single-line field is the dialog's OK");

        let mut body = String::new();
        let mut open = true;
        for (i, events) in [Vec::new(), Vec::new(), enter()].into_iter().enumerate() {
            crate::shared::ui::shot::frame(&ctx, size, events, |ui| {
                open = dialog(ui.ctx(), egui::Id::new("enter-newline"), |ui| {
                    let r = ui.add(egui::TextEdit::multiline(&mut body));
                    if i == 0 {
                        r.request_focus();
                    }
                    false
                });
            });
        }
        assert!(open, "a multiline field keeps the dialog open on Enter");
        assert!(body.contains('\n'), "the newline landed in the field");
    }

    /// The cross uses the same square button as other icons — the desk's square
    /// on a pointer, the phone's target on a narrow screen.
    #[test]
    fn the_cross_uses_the_standard_icon_button() {
        for width in [1000.0, 240.0] {
            let ctx = egui::Context::default();
            apply(&ctx);
            let icons = Icons::load(&ctx);

            let mut gutter = 0.0;
            let shapes =
                crate::shared::ui::shot::frame(&ctx, egui::vec2(width, 80.0), Vec::new(), |ui| {
                    gutter = ui.max_rect().max.x;
                    super::head(ui, &icons, "Entry");
                });
            // Read after the frame: the screen the app is drawn on is what the
            // tokens answer for.
            let side = crate::shared::ui::responsive::target(&ctx);
            let glyph = crate::shared::ui::responsive::glyph(&ctx);

            let sole = |s: &egui::epaint::ClippedShape| match &s.shape {
                egui::Shape::Rect(r) => Some(r.rect),
                _ => None,
            };
            let cross = shapes
                .iter()
                .find(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.brush.is_some()))
                .and_then(sole)
                .expect("the cross is painted");
            let square = shapes
                .iter()
                .filter_map(sole)
                .reduce(|a, b| a.union(b))
                .expect("the cross's square");

            let gap = gutter - cross.max.x;
            let pad = (side - glyph) / 2.0;
            assert!(
                (gap - pad).abs() < 1.0,
                "the cross has standard button padding at {width}: {gap} vs {pad}"
            );
            assert!(
                (square.width() - side).abs() < 1.0 && (square.height() - side).abs() < 1.0,
                "the cross has a standard icon button square at {width}: {square:?}"
            );
        }
    }

    /// One band: its content's width and every bar shape it painted, in paint order —
    /// the last one is the one on top. An id of its own, so two bands in a test never
    /// share the bar's state.
    type Painted = (f32, Vec<(Color32, egui::Rect, egui::CornerRadius)>);

    fn band(ctx: &egui::Context, content: f32) -> Painted {
        let mut wide = 0.0;
        let mut draw = |ui: &mut egui::Ui| {
            let area = egui::ScrollArea::vertical()
                .id_salt(content as i32)
                .auto_shrink([false, false]);
            let add = |ui: &mut egui::Ui| {
                wide = ui.available_width();
                ui.allocate_space(egui::vec2(180.0, content));
            };
            super::band(ui, area, add);
        };
        let size = egui::vec2(200.0, 100.0);
        // The bar reads the last frame's state, so one frame is not the answer.
        crate::shared::ui::shot::frame(ctx, size, Vec::new(), &mut draw);
        let shapes = crate::shared::ui::shot::frame(ctx, size, Vec::new(), &mut draw);
        let bars = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == palette().hair || r.fill == palette().rail => {
                    Some((r.fill, r.rect, r.corner_radius))
                }
                _ => None,
            })
            .collect();
        (wide, bars)
    }

    /// A band keeps the bar's gutter out of its own width whether or not the bar is
    /// up, and shows only what there is to show: the rail with a knob riding it when
    /// there is somewhere to go, and the bare rail when there is not. One that
    /// reserved its width as the list grew would shift every field above it, one that
    /// floated would cover them, and a knob that cannot move cries wolf on a list
    /// that already fits.
    #[test]
    fn a_band_reserves_the_gutter_and_keeps_its_rail() {
        let ctx = egui::Context::default();
        apply(&ctx);

        let (tall, bars) = band(&ctx, 400.0);
        assert!(
            (tall - (200.0 - BAR)).abs() < 1.0,
            "the gutter comes out of the content: {tall}"
        );
        let (ink, knob, _) = *bars.last().expect("the bar is painted");
        assert_eq!(
            ink,
            palette().hair,
            "the knob has the last word while it can move"
        );
        assert!(
            knob.min.x > tall,
            "the knob rides inside the gutter, off the rows: {knob:?}"
        );

        let (short, bars) = band(&ctx, 10.0);
        assert!(
            (short - tall).abs() < 0.5,
            "a band that fits keeps the same width: {short} against {tall}"
        );
        let (ink, rail, _) = *bars.last().expect("the rail is painted");
        assert_eq!(
            ink,
            palette().rail,
            "a bar that cannot move leaves the rail, not a knob"
        );
        assert!(
            rail.min.y < 1.0 && rail.min.x > short,
            "the rail runs the band inside the gutter: {rail:?}"
        );
        assert!(
            rail.height() > knob.height() + 20.0,
            "the rail is the whole groove, where the knob is a knob: {rail:?} against {knob:?}"
        );
        assert_eq!(
            bars.len(),
            1,
            "and nothing under it: a knob egui drew there would only show its own edge"
        );
    }

    /// A band that shrinks to its content does not let its rail run past the
    /// content's own bottom: the offered height is not the band's, so a rail
    /// spanning it would paint over the card behind it.
    #[test]
    fn a_band_that_shrinks_to_its_content_keeps_its_rail_inside_it() {
        let ctx = egui::Context::default();
        apply(&ctx);

        let mut band_bottom = 0.0f32;
        let mut draw = |ui: &mut egui::Ui| {
            let area = egui::ScrollArea::vertical()
                .id_salt("hug")
                .auto_shrink([false, true]);
            super::band(ui, area, |ui| {
                ui.allocate_space(egui::vec2(180.0, 10.0));
            });
            band_bottom = ui.min_rect().max.y;
        };

        let size = egui::vec2(200.0, 100.0);
        crate::shared::ui::shot::frame(&ctx, size, Vec::new(), &mut draw);
        let shapes = crate::shared::ui::shot::frame(&ctx, size, Vec::new(), &mut draw);

        let bars: Vec<_> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == palette().rail => Some(r.rect),
                _ => None,
            })
            .collect();
        let rail = *bars.last().expect("the rail is painted");
        assert!(
            rail.max.y <= band_bottom + 1.0,
            "rail spills past the band: {rail:?} vs band bottom {band_bottom}"
        );
        assert!(
            rail.height() < 50.0,
            "rail spans the offered height, not the content: {rail:?}"
        );
    }

    #[test]
    fn a_page_reserves_the_gutter_and_keeps_its_rail() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let size = egui::vec2(200.0, 100.0);

        let page = |content: f32| -> Painted {
            let mut wide = 0.0;
            let mut draw = |ui: &mut egui::Ui| {
                let area = egui::ScrollArea::vertical()
                    .id_salt(("page", content as i32))
                    .auto_shrink([false, false]);
                super::scroll(ui, area, |ui: &mut egui::Ui| {
                    wide = ui.available_width();
                    ui.allocate_space(egui::vec2(180.0, content));
                });
            };
            // The bar reads the last frame's state, so one frame is not the answer.
            crate::shared::ui::shot::frame(&ctx, size, Vec::new(), &mut draw);
            let shapes = crate::shared::ui::shot::frame(&ctx, size, Vec::new(), &mut draw);
            let bars = shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r)
                        if r.fill == palette().hair || r.fill == palette().rail =>
                    {
                        Some((r.fill, r.rect, r.corner_radius))
                    }
                    _ => None,
                })
                .collect();
            (wide, bars)
        };

        let (tall, bars) = page(400.0);
        assert!(
            (tall - (200.0 - BAR)).abs() < 1.0,
            "the gutter comes out of the page: {tall}"
        );
        let (ink, knob, _) = *bars.last().expect("the bar is painted");
        assert_eq!(
            ink,
            palette().hair,
            "the knob has the last word while it can move"
        );
        assert!(
            knob.min.x > tall,
            "the knob rides inside the gutter, off the rows: {knob:?}"
        );

        let (short, bars) = page(10.0);
        assert!(
            (short - tall).abs() < 0.5,
            "a page that fits keeps the same width: {short} against {tall}"
        );
        let (ink, rail, _) = *bars.last().expect("the rail is painted");
        assert_eq!(
            ink,
            palette().rail,
            "a page that fits leaves the rail, not a knob"
        );
        assert!(
            rail.min.y < 1.0 && rail.min.x > short,
            "the rail runs the page inside the gutter: {rail:?}"
        );
        assert!(
            rail.height() > knob.height() + 20.0,
            "the rail is the whole groove, where the knob is a knob: {rail:?} against {knob:?}"
        );
    }

    /// Below the breakpoint the bar's grip is the whole gutter, not the eight
    /// pixels egui aims at a pointer: a drag that lands in the gutter off the
    /// bar's own zone still takes the list with it, with the knob's travel (the
    /// gutter's height crosses the whole content). A wider window keeps the
    /// gutter inert — the desk's strip and wheel are where they always were.
    #[test]
    fn a_phone_grabs_the_bar_by_the_whole_gutter() {
        let ctx = egui::Context::default();
        apply(&ctx);

        let dragged = |ctx: &egui::Context, size: egui::Vec2| -> f32 {
            let mut id = None;
            let mut draw = |ui: &mut egui::Ui| {
                id = Some(
                    super::scroll(
                        ui,
                        egui::ScrollArea::vertical()
                            .id_salt(size.x as i32)
                            .auto_shrink([false, false]),
                        |ui| {
                            ui.allocate_space(egui::vec2(200.0, 4000.0));
                        },
                    )
                    .id,
                );
            };
            // The bar reads the last frame's state, so one frame is not the answer.
            crate::shared::ui::shot::frame(ctx, size, Vec::new(), &mut draw);
            crate::shared::ui::shot::frame(ctx, size, Vec::new(), &mut draw);
            // Land in the gutter, off the bar's own eight pixels at the edge.
            let at = egui::pos2(size.x - BAR + 3.0, size.y / 2.0);
            crate::shared::ui::shot::frame(
                ctx,
                size,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
                &mut draw,
            );
            crate::shared::ui::shot::frame(
                ctx,
                size,
                vec![egui::Event::PointerMoved(at + egui::vec2(0.0, 40.0))],
                &mut draw,
            );
            egui::containers::scroll_area::State::load(ctx, id.expect("the page drew"))
                .map(|s| s.offset.y)
                .unwrap_or(0.0)
        };

        let phone = dragged(&ctx, egui::vec2(360.0, 480.0));
        assert!(
            phone > 0.0,
            "a drag in the gutter takes the list with it: {phone}"
        );

        let desk = dragged(&ctx, egui::vec2(800.0, 480.0));
        assert_eq!(desk, 0.0, "a desk's gutter off the bar stays inert: {desk}");
    }

    /// The knob under a thumb goes green while it is held: the press lands on the
    /// gutter's own widget, above egui's bar, so the bar would never read it and
    /// `grab` paints the accent itself. A knob nobody holds stays at rest, and the
    /// desk keeps egui's bar and its own eight pixels.
    #[test]
    fn a_thumb_on_the_gutter_paints_the_knob_accent() {
        let accent = |shapes: &[egui::epaint::ClippedShape], right: f32| {
            shapes.iter().any(|s| match &s.shape {
                egui::Shape::Rect(r) => {
                    r.fill == ACCENT
                        && r.rect.width() < 20.0
                        && r.rect.height() > 3.0
                        && (right - r.rect.right()).abs() < 20.0
                }
                _ => false,
            })
        };

        let held = |size: egui::Vec2, press: bool| -> bool {
            let ctx = egui::Context::default();
            apply(&ctx);
            let mut draw = |ui: &mut egui::Ui| {
                super::scroll(
                    ui,
                    egui::ScrollArea::vertical()
                        .id_salt(size.x as i32)
                        .auto_shrink([false, false]),
                    |ui| {
                        ui.allocate_space(egui::vec2(200.0, 4000.0));
                    },
                );
            };
            // The bar reads the last frame's state, so one frame is not the answer.
            crate::shared::ui::shot::frame(&ctx, size, Vec::new(), &mut draw);
            crate::shared::ui::shot::frame(&ctx, size, Vec::new(), &mut draw);
            // Land in the gutter, off the bar's own eight pixels at the edge.
            let at = egui::pos2(size.x - BAR + 3.0, size.y / 2.0);
            let mut events = vec![egui::Event::PointerMoved(at)];
            if press {
                events.push(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                });
            }
            crate::shared::ui::shot::frame(&ctx, size, events, &mut draw);
            let shapes = crate::shared::ui::shot::frame(&ctx, size, Vec::new(), &mut draw);
            accent(&shapes, size.x)
        };

        assert!(
            held(egui::vec2(360.0, 480.0), true),
            "a thumb on the gutter paints the knob in the accent"
        );
        assert!(
            !held(egui::vec2(360.0, 480.0), false),
            "a knob nobody holds stays at rest"
        );
        assert!(
            !held(egui::vec2(800.0, 480.0), true),
            "the desk leaves the knob to egui's own bar"
        );
    }

    /// Hover a control: the pointer goes to the centre of the rect it lays itself
    /// out at.
    fn hover(
        ctx: &egui::Context,
        size: egui::Vec2,
        mut draw: impl FnMut(&mut egui::Ui) -> egui::Rect,
    ) -> egui::CursorIcon {
        let mut rect = egui::Rect::NOTHING;
        for _ in 0..2 {
            crate::shared::ui::shot::frame(ctx, size, Vec::new(), |ui| rect = draw(ui));
        }
        let mut out = ctx.run_ui(
            crate::shared::ui::shot::input(size, vec![egui::Event::PointerMoved(rect.center())]),
            |ui| rect = draw(ui),
        );
        out.textures_delta.clear();
        out.platform_output.cursor_icon
    }

    /// `Visuals::interact_cursor` reaches `Button` and nothing else, so a tick
    /// box, a select and a slider ask for their own cursor; and the bar egui
    /// paints asks for the open hand, closed while it is dragged. Without those
    /// the pointer stays an arrow over a control that is clearly a control.
    #[test]
    fn a_control_egui_does_not_cursor_asks_for_its_own() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let size = egui::vec2(240.0, 90.0);

        let mut on = false;
        assert_eq!(
            hover(&ctx, size, |ui| check(ui, &mut on, "A-Z").rect),
            egui::CursorIcon::PointingHand,
            "a tick box points the cursor"
        );

        let mut pick = 0;
        assert_eq!(
            hover(&ctx, size, |ui| {
                combo(
                    ui,
                    egui::ComboBox::from_id_salt("probe").selected_text("Never"),
                    |ui| {
                        for i in 0..3 {
                            if ui.selectable_label(pick == i, "one").clicked() {
                                pick = i;
                            }
                        }
                    },
                )
                .response
                .rect
            }),
            egui::CursorIcon::PointingHand,
            "a select points the cursor"
        );

        let mut len = 5u32;
        assert_eq!(
            hover(&ctx, size, |ui| {
                slider(ui, "10", egui::Slider::new(&mut len, 1..=10)).rect
            }),
            egui::CursorIcon::Grab,
            "a slider offers the open hand"
        );

        let bar = |ui: &mut egui::Ui| {
            super::band(
                ui,
                egui::ScrollArea::vertical().id_salt("cursor"),
                |ui: &mut egui::Ui| {
                    ui.allocate_space(egui::vec2(180.0, 400.0));
                },
            );
        };
        crate::shared::ui::shot::frame(&ctx, size, Vec::new(), bar);
        let shapes = crate::shared::ui::shot::frame(&ctx, size, Vec::new(), bar);
        // The bar egui paints sits in the gutter at the frame's right edge.
        let at = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) => Some(r.rect),
                _ => None,
            })
            .max_by(|a, b| a.min.x.total_cmp(&b.min.x))
            .expect("the bar is painted")
            .center();
        let press = |held: bool| {
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: held,
                    modifiers: Default::default(),
                },
            ]
        };
        let mut out = ctx.run_ui(
            crate::shared::ui::shot::input(size, vec![egui::Event::PointerMoved(at)]),
            bar,
        );
        out.textures_delta.clear();
        assert_eq!(
            out.platform_output.cursor_icon,
            egui::CursorIcon::Grab,
            "the scroll bar offers the open hand"
        );

        let _ = ctx.run_ui(crate::shared::ui::shot::input(size, press(true)), bar);
        let mut out = ctx.run_ui(
            crate::shared::ui::shot::input(
                size,
                vec![egui::Event::PointerMoved(at + egui::vec2(0.0, 20.0))],
            ),
            bar,
        );
        out.textures_delta.clear();
        assert_eq!(
            out.platform_output.cursor_icon,
            egui::CursorIcon::Grabbing,
            "a scroll bar being dragged closes the hand"
        );
    }

    #[test]
    fn a_checked_box_is_a_green_box_with_a_white_tick() {
        let ctx = egui::Context::default();
        assert_eq!(tick(&ctx, true), (true, true), "green box, white tick");
        assert_eq!(
            tick(&ctx, false),
            (false, false),
            "plain box while it is off"
        );
    }
}
