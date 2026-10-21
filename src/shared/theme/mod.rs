mod brands;
mod icons;
mod style;

pub use brands::{brand_of, Brand, Pick};
pub use icons::{decode, kind_icon, live_frame, live_texture, window_icon, Icon, Icons, ICON};
pub use style::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::ui::shot;
    use egui::{Color32, RichText};

    #[test]
    fn a_trailing_icon_follows_its_label() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let icons = Icons::load(&ctx);
        let atoms = icons.trailing(
            Some(Icon::LockKeyhole),
            15.0,
            Color32::WHITE,
            RichText::new("Lock"),
        );
        let order: Vec<&str> = atoms
            .iter_kinds()
            .map(|kind| match kind {
                egui::AtomKind::Text(_) => "label",
                egui::AtomKind::Image(_) => "icon",
                _ => "other",
            })
            .collect();
        assert_eq!(order, ["label", "icon"]);
    }
    #[test]
    fn the_primary_button_centres_its_label() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let icons = Icons::load(&ctx);
        let input = || shot::input(egui::vec2(400.0, 200.0), Vec::new());

        let mut rect: Option<egui::Rect> = None;
        let mut draw = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(input(), |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    rect = Some(primary_button(ui, &icons, None, "Update password", None).rect);
                });
            });
            out.textures_delta.clear();
            out.shapes
        };
        draw(&ctx); // first frame of a fresh ctx: fonts are laid out lazily
        let shapes = draw(&ctx);
        drop(draw);

        let label = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "Update password" => {
                    Some(t.galley.rect.translate(t.pos.to_vec2()))
                }
                _ => None,
            })
            .expect("the label is painted");
        let button = rect.expect("the button is laid out");
        assert!(
            (label.center().x - button.center().x).abs() < 1.0,
            "the label sits at {:?}, not centred in {button:?}",
            label.center(),
        );
    }
    #[test]
    fn an_icon_button_centres_its_icon_whatever_the_row_asks_for() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let icons = Icons::load(&ctx);
        let input = || shot::input(egui::vec2(200.0, 60.0), Vec::new());

        let mut square: Option<egui::Rect> = None;
        let mut draw = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(input(), |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    square = Some(icons.button(ui, Icon::X, 15.0, palette().muted, "").rect);
                });
            });
            out.textures_delta.clear();
            out.shapes
        };
        draw(&ctx);
        let shapes = draw(&ctx);
        drop(draw);

        let icon = shapes
            .iter()
            .map(|s| s.shape.visual_bounding_rect())
            .find(|r| {
                let glyph = crate::shared::ui::responsive::glyph(&ctx);
                (r.width() - glyph).abs() < 0.5 && (r.height() - glyph).abs() < 0.5
            })
            .expect("the icon is painted");
        let square = square.expect("the button is laid out");
        assert!(
            (icon.center() - square.center()).length() < 1.0,
            "the icon sits at {:?}, not centred in {square:?}",
            icon.center(),
        );
    }
    // egui reads a widget's state from the previous frame, hence the third frame before a change shows.
    #[test]
    fn the_primary_button_deepens_on_hover_and_press() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let icons = Icons::load(&ctx);
        let size = egui::vec2(400.0, 200.0);

        let mut rect: Option<egui::Rect> = None;
        let mut rest = Vec::new();
        let mut hover = Vec::new();
        let mut press = Vec::new();
        for frame in 0..6 {
            let mut events = Vec::new();
            if frame > 0 {
                let at = rect.expect("the button is laid out").center();
                events.push(egui::Event::PointerMoved(at));
                if frame >= 3 {
                    events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    });
                }
            }
            let shapes = shot::frame(&ctx, size, events, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    rect = Some(primary_button(ui, &icons, None, "Unlock", None).rect);
                });
            });
            let button = rect.expect("the button is laid out");
            let painted: Vec<Color32> = shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r) if r.rect.contains(button.center()) => Some(r.fill),
                    _ => None,
                })
                .collect();
            match frame {
                0 => rest = painted,
                2 => hover = painted,
                5 => press = painted,
                _ => {}
            }
        }

        assert!(
            rest.contains(&ACCENT),
            "the button is not accent-filled at rest: {rest:?}"
        );
        assert!(
            hover.contains(&palette().accent_dim),
            "the accent fill does not deepen on hover: {hover:?}"
        );
        assert!(
            press.contains(&palette().accent_deep),
            "the accent fill does not deepen again on press: {press:?}"
        );
    }

    #[test]
    fn the_secondary_button_changes_on_hover_and_press() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let icons = Icons::load(&ctx);
        let size = egui::vec2(400.0, 200.0);
        let mut rect: Option<egui::Rect> = None;
        let mut rest = Vec::new();
        let mut hover = Vec::new();
        let mut press = Vec::new();

        for frame in 0..6 {
            let mut events = Vec::new();
            if frame > 0 {
                let at = rect.expect("the button is laid out").center();
                events.push(egui::Event::PointerMoved(at));
                if frame >= 3 {
                    events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    });
                }
            }
            let shapes = shot::frame(&ctx, size, events, |ui| {
                rect = Some(secondary_button(ui, &icons, None, "Open a box").rect);
            });
            let button = rect.expect("the button is laid out");
            let painted: Vec<Color32> = shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.rect.contains(button.center()) => {
                        Some(rect.fill)
                    }
                    _ => None,
                })
                .collect();
            match frame {
                0 => rest = painted,
                2 => hover = painted,
                5 => press = painted,
                _ => {}
            }
        }

        assert!(
            rest.contains(&palette().elev),
            "secondary button lacks its rest fill: {rest:?}"
        );
        assert!(
            hover.contains(&ctx.style_of(ctx.theme()).visuals.widgets.hovered.bg_fill),
            "secondary button does not use hover fill: {hover:?}"
        );
        assert!(
            press.contains(&palette().accent_dim),
            "secondary button does not use pressed fill: {press:?}"
        );
    }

    /// Focused is not pressed: egui paints focus with the `active` visuals, so the
    /// safe answer would wear the accent fill and read as the primary action.
    #[test]
    fn the_safe_answer_keeps_its_quiet_fill_while_focused() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let mut safe: Option<egui::Rect> = None;
        let mut draw = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(shot::input(egui::vec2(400.0, 200.0), Vec::new()), |ui| {
                actions(
                    ui,
                    ctx,
                    |ui| ui.button(RichText::new("Delete").color(palette().error)),
                    |ui| {
                        let r = ui.button(crate::shared::i18n::t("Cancel"));
                        safe = Some(r.rect);
                        r
                    },
                );
            });
            out.textures_delta.clear();
            out.shapes
        };
        draw(&ctx);
        let shapes = draw(&ctx);
        drop(draw);

        assert!(
            ctx.memory(|m| m.focused().is_some()),
            "the safe answer holds the keyboard"
        );
        let safe = safe.expect("the safe answer is laid out");
        let painted: Vec<Color32> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.rect.contains(safe.center()) => Some(r.fill),
                _ => None,
            })
            .collect();
        assert!(
            !painted.contains(&palette().accent_dim),
            "the focused safe answer wears the accent fill: {painted:?}"
        );
        assert!(
            painted.contains(&palette().elev),
            "and keeps its quiet fill: {painted:?}"
        );
    }

    /// `text` and `muted` hold AA on every surface; `faint` is only for hints.
    #[test]
    fn both_schemes_keep_their_text_readable() {
        for scheme in Scheme::ALL {
            let p = palette_of(scheme);
            for (name, fg) in [("text", p.text), ("muted", p.muted)] {
                for (surface, bg) in [("paper", p.paper), ("panel", p.panel), ("elev", p.elev)] {
                    let ratio = contrast(fg, bg);
                    assert!(
                        ratio >= 4.5,
                        "{scheme:?}: {name} on {surface} is {ratio:.2}:1, under AA"
                    );
                }
            }
            let selected = contrast(p.text, p.accent_dim);
            assert!(
                selected >= 4.5,
                "{scheme:?}: text on the selected fill is {selected:.2}:1"
            );
            assert!(
                contrast(p.text, p.paper) > contrast(p.muted, p.paper)
                    && contrast(p.muted, p.paper) > contrast(p.faint, p.paper),
                "{scheme:?}: text > muted > faint is what makes a hierarchy"
            );
        }
    }

    #[test]
    fn a_scheme_keeps_its_surfaces_apart() {
        for scheme in Scheme::ALL {
            let p = palette_of(scheme);
            for (a, b) in [
                (p.paper, p.panel),
                (p.panel, p.elev),
                (p.elev, p.hair),
                (p.paper, p.hair),
            ] {
                assert_ne!(a, b, "{scheme:?}: two surfaces share a colour");
            }
            assert_ne!(p.paper, p.text, "{scheme:?}: the page is not its own text");
        }
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        fn luminance(c: Color32) -> f32 {
            fn channel(v: u8) -> f32 {
                let s = v as f32 / 255.0;
                if s <= 0.03928 {
                    s / 12.92
                } else {
                    ((s + 0.055) / 1.055).powf(2.4)
                }
            }
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        }
        let (x, y) = (luminance(a), luminance(b));
        let (high, low) = if x > y { (x, y) } else { (y, x) };
        (high + 0.05) / (low + 0.05)
    }

    #[test]
    fn a_scheme_reaches_the_widgets_it_did_not_paint() {
        let light = visuals(Scheme::Light);
        assert!(!light.dark_mode, "the light scheme is not dark visuals");
        assert_eq!(light.panel_fill, palette_of(Scheme::Light).paper);
        assert_eq!(light.window_fill, palette_of(Scheme::Light).panel);

        let dark = visuals(Scheme::Dark);
        assert!(dark.dark_mode);
        assert_eq!(dark.panel_fill, palette_of(Scheme::Dark).paper);
        assert_eq!(dark.window_fill, palette_of(Scheme::Dark).panel);
    }

    /// The accent hex is duplicated in `scripts/build-logo.sh`; drifting breaks the icon.
    #[test]
    fn accent_hex_matches_the_logo_script() {
        let script = include_str!("../../../scripts/build-logo.sh");
        let hex = format!("#{:02x}{:02x}{:02x}", ACCENT.r(), ACCENT.g(), ACCENT.b());
        assert!(
            script.contains(&hex),
            "build-logo.sh accent drifted from theme::ACCENT ({hex})"
        );
    }

    /// A regenerated logo with another shape stretches the mark; 137x128 is the asset contract.
    #[test]
    fn logo_asset_has_the_expected_shape() {
        let logo = icons::decode(include_bytes!("../../../assets/logo.png")).unwrap();
        assert_eq!(logo.size, [137, 128]);
    }
}
