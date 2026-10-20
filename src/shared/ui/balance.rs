//! `text-wrap: balance` for the app's prose: `label` cuts a string at the
//! narrowest width that still holds it on as many rows as the full width would,
//! so a heading comes out even instead of stranding a word on its own line.
//!
//! It lays the text out the way [`egui::Label`] does — same job, same
//! alignment, same galley — with only the wrap width narrowed first. A wrapping
//! horizontal row, where `Label` indents the first row around other widgets, is
//! handed straight back to it: there is nothing to even out there.

use egui::{Direction, FontSelection, Label, Ui, WidgetText};
use std::sync::Arc;

/// Paints `text` as [`Ui::label`] would, with its lines balanced.
pub fn label(ui: &mut Ui, text: impl Into<WidgetText>) {
    let text = text.into();
    if ui.layout().main_dir() == Direction::LeftToRight && ui.layout().main_wrap() {
        ui.label(text);
        return;
    }

    let mut job = Arc::unwrap_or_clone(text.into_layout_job(
        ui.style(),
        FontSelection::Default,
        ui.text_valign(),
    ));
    job.halign = ui.layout().horizontal_placement();
    job.justify = ui.layout().horizontal_justify();
    job.wrap.max_width = ui.available_width();

    let lines = rows(ui, &job);
    if lines > 1 {
        // One pixel narrower and a word drops to the next row, so the tightest
        // width that keeps the line count is also the evenest one.
        let (mut lo, mut hi) = (0.0, job.wrap.max_width);
        for _ in 0..12 {
            let mid = (lo + hi) / 2.0;
            job.wrap.max_width = mid;
            if rows(ui, &job) == lines {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        job.wrap.max_width = hi;
    }

    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    ui.add(Label::new(WidgetText::Galley(galley)));
}

fn rows(ui: &Ui, job: &egui::text::LayoutJob) -> usize {
    ui.fonts_mut(|fonts| fonts.layout_job(job.clone()))
        .rows
        .len()
}

#[cfg(test)]
mod tests {
    use crate::shared::theme;
    use crate::shared::ui::shot;
    use egui::{RichText, Shape};

    /// Two rows of prose break where the words allow; balancing may not add a
    /// row, may not widen the block, and leaves the rows no less even.
    #[test]
    fn balancing_keeps_the_rows_and_evens_them() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let text = "Save your secrets inside Pandora’s box";

        let shapes = shot::frame(&ctx, egui::vec2(320.0, 240.0), Vec::new(), |ui| {
            ui.set_max_width(140.0);
            ui.label(RichText::new(text).small());
            super::label(ui, RichText::new(text).small());
        });
        let galleys: Vec<_> = shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text() == text => Some(t.galley.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(galleys.len(), 2, "both labels are painted");

        let widths = |g: &egui::Galley| {
            g.rows
                .iter()
                .map(|row| row.rect().width())
                .collect::<Vec<_>>()
        };
        let (plain, even) = (widths(&galleys[0]), widths(&galleys[1]));
        assert!(plain.len() > 1, "the text has to wrap for this to say much");
        assert_eq!(even.len(), plain.len(), "no word was pushed to a new row");
        assert!(
            even.iter().cloned().fold(0.0, f32::max) <= plain.iter().cloned().fold(0.0, f32::max),
            "the block is no wider: {even:?} against {plain:?}"
        );
        let evenness = |w: &[f32]| {
            w.iter().cloned().fold(f32::MAX, f32::min) / w.iter().cloned().fold(0.0, f32::max)
        };
        assert!(
            evenness(&even) > evenness(&plain),
            "the rows are less ragged: {even:?} against {plain:?}"
        );
    }
}
