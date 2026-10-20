//! A card's heading belongs to the card, not to the page that hangs it: it starts
//! at the card's own left edge, whatever measure the page gives it — one card's or
//! a phone's.

use egui::{pos2, Rect, Shape};
use pandora::shared::theme;

fn run(draw: impl FnMut(&mut egui::Ui)) -> Vec<egui::epaint::ClippedShape> {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(
            pos2(0.0, 0.0),
            egui::vec2(1000.0, 600.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, draw);
    out.textures_delta.clear();
    out.shapes
}

/// The page as the pane draws it: a centred measure the cards hang on. The page
/// spends its own width up to that measure, as settings and the generator do.
fn page(measure: f32, title: &str) -> Vec<egui::epaint::ClippedShape> {
    run(move |ui| {
        egui::CentralPanel::default()
            .frame(theme::pane(false))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(32.0);
                    ui.vertical_centered(|ui| {
                        ui.set_width(ui.available_width().min(measure));
                        theme::card(ui, title, |ui| {
                            ui.label("body");
                        });
                    });
                });
            });
    })
}

/// The card a heading heads: its panel is painted before the text it holds, so the
/// card is the one sharing the heading's left edge, nearest under it.
fn card_under(shapes: &[egui::epaint::ClippedShape], title: &str) -> (Rect, egui::Pos2, f32) {
    let text = shapes
        .iter()
        .find_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.text() == title.to_uppercase() => Some(t),
            _ => None,
        })
        .expect("the heading is painted");
    let row = text.galley.rows[0].rect();
    let ink = text.pos + row.min.to_vec2();
    let card = shapes
        .iter()
        .filter_map(|s| match &s.shape {
            Shape::Rect(r)
                if r.fill == theme::palette().panel
                    && (r.rect.left() - ink.x).abs() < 1.5
                    && r.rect.top() >= text.pos.y - 1.0 =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("the card under the heading");
    (card, ink, row.width())
}

/// The heading over a card belongs to the card: it starts where the card does, at
/// every measure the app hangs a card on — a phone's, and one card's own.
#[test]
fn the_card_heading_starts_where_the_card_does() {
    for measure in [332.0, theme::COLUMN] {
        let shapes = page(measure, "Master password");
        let (card, ink, width) = card_under(&shapes, "Master password");
        println!(
            "measure {measure}: card {:.1}..{:.1}, heading ink {:.1} (row {width:.1} wide)",
            card.min.x, card.max.x, ink.x,
        );
        assert!(
            (card.width() - measure).abs() < 1.0,
            "the page takes the measure it was given: {:.1} of {measure}",
            card.width()
        );
        assert!(
            (ink.x - card.min.x).abs() < 1.0,
            "the heading starts where the card does: card {:.1} vs heading {:.1}",
            card.min.x,
            ink.x
        );
    }
}

/// The heading reads from the left: its own centre may not be the pane's.
#[test]
fn the_card_heading_is_not_centred_over_the_card() {
    let shapes = page(theme::COLUMN, "Master password");
    let (card, ink, width) = card_under(&shapes, "Master password");
    assert!(
        ink.x + width / 2.0 < card.center().x - 50.0,
        "the heading is pinned to the card's left, not to the pane's middle"
    );
}
