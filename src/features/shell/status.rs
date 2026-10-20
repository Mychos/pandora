use crate::features::shell::model::{Shell, Status, StatusKind};
use crate::shared::theme;
use std::time::Instant;

const STATUS_LIFE: std::time::Duration = std::time::Duration::from_secs(5);
const ERROR_LIFE: std::time::Duration = std::time::Duration::from_secs(10);
const FADE: std::time::Duration = std::time::Duration::from_millis(250);
/// Redraw pace while a notice is up; the fade itself runs on wall time.
const FADE_STEP: std::time::Duration = std::time::Duration::from_millis(33);
/// Horizontal inset of the notice's own frame.
const MARGIN: f32 = 14.0;

impl Status {
    fn life(&self) -> std::time::Duration {
        if self.kind == StatusKind::Error {
            ERROR_LIFE
        } else {
            STATUS_LIFE
        }
    }

    fn gone(&self) -> std::time::Duration {
        self.life() + FADE
    }

    pub fn alpha(&self) -> f32 {
        let fade = FADE.as_secs_f32();
        let in_ = self.at.elapsed().as_secs_f32() / fade;
        let out = self.gone().saturating_sub(self.at.elapsed()).as_secs_f32() / fade;
        in_.min(out).clamp(0.0, 1.0)
    }
}

impl Shell {
    pub fn set_status(&mut self, text: impl Into<String>, kind: StatusKind) {
        self.status = Some(Status {
            at: Instant::now(),
            text: text.into(),
            kind,
        });
    }

    pub fn fail(&mut self, text: impl Into<String>) {
        self.set_status(text.into(), StatusKind::Error);
    }

    pub fn expire_status(&mut self) {
        // Dropped once the leaving fade is over, not when its life runs out.
        if let Some(st) = &self.status {
            if st.at.elapsed() >= st.gone() {
                self.status = None;
            }
        }
    }

    /// Floats over the screen; nothing is reserved, so the notice displaces nothing.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let Some(st) = &self.status else { return };
        let text = crate::shared::i18n::text(&st.text).into_owned();
        let alpha = st.alpha();
        let color = match st.kind {
            StatusKind::Saved => theme::palette().muted,
            StatusKind::Info => theme::palette().ok,
            StatusKind::Error => theme::palette().error,
        };
        // The fade runs on its own clock: nothing else asks for frames here.
        ui.ctx().request_repaint_after(FADE_STEP);
        egui::Area::new(egui::Id::new("status"))
            .movable(false)
            .interactable(false)
            .order(egui::Order::Tooltip)
            .anchor(
                // A phone's bar is the window's floor: the notice floats above it.
                egui::Align2::CENTER_BOTTOM,
                egui::vec2(0.0, -(14.0 + crate::shared::ui::chrome::floor(ui.ctx()))),
            )
            .show(ui.ctx(), |ui| {
                ui.multiply_opacity(alpha);
                egui::Frame::NONE
                    .fill(theme::palette().panel)
                    .corner_radius(theme::radius(8))
                    .stroke(theme::stroke(theme::palette().hair, 1.0))
                    .inner_margin(egui::Margin::symmetric(MARGIN as i8, 8))
                    .show(ui, |ui| {
                        let wide =
                            ui.ctx().input(|i| i.content_rect().width()) - 2.0 * MARGIN - 12.0;
                        ui.set_max_width(wide.clamp(120.0, 420.0));
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(text).small().color(color).monospace(),
                            )
                            .wrap(),
                        );
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn status(age: Duration) -> Status {
        Status {
            at: Instant::now() - age,
            text: String::new(),
            kind: StatusKind::Saved,
        }
    }

    #[test]
    fn a_long_notice_wraps_on_a_narrow_screen() {
        let ctx = egui::Context::default();
        crate::shared::theme::apply(&ctx);
        let mut shell = Shell::default();
        shell.set_status(
            "getting the box from the repository for a very long owner name",
            StatusKind::Info,
        );
        shell.status.as_mut().unwrap().at = Instant::now() - FADE - Duration::from_secs(1);
        // The area registers on the first pass; the second one has it painted.
        let mut shapes = Vec::new();
        for _ in 0..2 {
            shapes =
                crate::shared::ui::shot::frame(&ctx, egui::vec2(360.0, 640.0), Vec::new(), |ui| {
                    shell.draw(ui)
                });
        }
        let notice = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text().contains("getting the box") => Some(t),
                _ => None,
            })
            .expect("the notice is painted");
        assert!(
            notice.galley.rows.len() > 1,
            "it wraps rather than truncating"
        );
        assert!(
            notice.galley.rect.width() <= 360.0,
            "it stays inside the phone"
        );
    }

    #[test]
    fn the_notice_fades_in_then_out() {
        assert!(
            status(Duration::ZERO).alpha() < 0.01,
            "it arrives invisible"
        );
        assert_eq!(
            status(FADE + Duration::from_secs(1)).alpha(),
            1.0,
            "it settles at full strength"
        );
        assert!(
            status(STATUS_LIFE).alpha() >= 0.99,
            "its life runs out at full strength"
        );
        assert!(
            status(STATUS_LIFE + FADE / 2).alpha() < 0.6,
            "then it fades"
        );
        assert!(
            status(STATUS_LIFE + FADE).alpha() < 0.01,
            "and it is gone, ready to be dropped"
        );
    }
}
