//! A zoomable, pannable sheet: a photo or a PDF page on its own board. Wheel and
//! pinch zoom at the gesture, a drag pans, and `fit` re-centres when the box
//! changes size. The state is keyed by what is on the board, so opening another
//! entry or turning a page starts from the fit again.

use crate::shared::theme;
use egui::{Color32, CursorIcon, Pos2, Rect, Sense, TextureHandle, Ui, Vec2};

/// However small or large the sheet is drawn, it never leaves this range: below
/// the floor it is a speck, above the ceiling it is a smear of texels.
const MIN: f32 = 0.02;
const MAX: f32 = 8.0;
/// Air the fitted sheet keeps off the board's edge.
const MARGIN: f32 = 8.0;
/// How much of the sheet has to stay over the board when it is dragged away.
const SLIVER: f32 = 24.0;
/// One notch of the wheel, as a factor: `e^(delta * STEP)`.
const STEP: f32 = 0.002;

pub struct Board {
    key: Option<egui::Id>,
    fit: bool,
    scale: f32,
    pan: Vec2,
}

impl Default for Board {
    fn default() -> Self {
        Self {
            key: None,
            fit: true,
            scale: 0.0,
            pan: Vec2::ZERO,
        }
    }
}

impl Board {
    /// Draw `texture` as a sheet `natural` units wide and tall, and return the
    /// points-per-unit it ended up at. The caller asks for the room the board has.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        key: egui::Id,
        texture: &TextureHandle,
        natural: Vec2,
        room: f32,
    ) -> f32 {
        if natural.x <= 0.0 || natural.y <= 0.0 {
            return 0.0;
        }
        if self.key != Some(key) {
            self.key = Some(key);
            self.fit = true;
            self.pan = Vec2::ZERO;
        }

        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), room.max(1.0)),
            Sense::click_and_drag(),
        );
        let fit = ((rect.width() - 2.0 * MARGIN) / natural.x)
            .min((rect.height() - 2.0 * MARGIN) / natural.y)
            .clamp(MIN, MAX);
        if self.fit || self.scale <= 0.0 {
            self.scale = fit;
            self.pan = Vec2::ZERO;
        }

        self.gestures(ui, &resp, rect);

        self.clamp(rect, natural);

        let painter = ui.painter();
        painter.rect_filled(rect, theme::radius(8), theme::palette().paper);
        // A clip rect is square, so a zoomed sheet would square off the board's
        // corners. The visible part of the sheet is painted as a rounded, textured
        // rectangle instead: rounded only where it actually reaches a board corner.
        let sheet = Rect::from_center_size(rect.center() + self.pan, natural * self.scale);
        let size = sheet.size();
        if size.x > 0.0 && size.y > 0.0 {
            let visible = sheet.intersect(rect);
            if visible.is_positive() {
                let corner = corners(visible, rect);
                let uv = Rect::from_min_max(
                    egui::pos2(
                        (visible.min.x - sheet.min.x) / size.x,
                        (visible.min.y - sheet.min.y) / size.y,
                    ),
                    egui::pos2(
                        (visible.max.x - sheet.min.x) / size.x,
                        (visible.max.y - sheet.min.y) / size.y,
                    ),
                );
                let mut shape = egui::epaint::RectShape::filled(visible, corner, Color32::WHITE);
                shape.brush = Some(std::sync::Arc::new(egui::epaint::Brush {
                    fill_texture_id: texture.id(),
                    uv,
                }));
                painter.add(shape);
            }
        }
        painter.rect_stroke(
            rect,
            theme::radius(8),
            theme::stroke(theme::palette().hair, 1.0),
            egui::StrokeKind::Inside,
        );

        if resp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }

        self.scale
    }

    /// Wheel, pinch and drag. A two-finger gesture owns both axes at once, so the
    /// pointer's own drag is not added on top of it.
    fn gestures(&mut self, ui: &mut Ui, resp: &egui::Response, rect: Rect) {
        let multi = ui
            .input(|i| i.multi_touch())
            .filter(|touch| rect.contains(touch.center_pos));

        if let Some(touch) = multi {
            if touch.zoom_delta != 1.0 {
                self.zoom(touch.zoom_delta, Some((touch.center_pos, rect)));
            }
            if touch.translation_delta != Vec2::ZERO {
                self.fit = false;
                self.pan += touch.translation_delta;
            }
        } else if resp.dragged() {
            self.fit = false;
            self.pan += resp.drag_delta();
        }

        if resp.hovered() {
            let (wheel, pointer) = ui.input(|i| (i.smooth_scroll_delta.y, i.pointer.hover_pos()));
            // A pinch is already counted above; only a wheel or trackpad zoom
            // that is not a multi-touch gesture is added here.
            let pinch = if multi.is_some() {
                1.0
            } else {
                ui.input(|i| i.zoom_delta())
            };
            if wheel != 0.0 || pinch != 1.0 {
                let anchor = pointer
                    .filter(|p| rect.contains(*p))
                    .unwrap_or(rect.center());
                self.zoom((wheel * STEP).exp() * pinch, Some((anchor, rect)));
                // The board owns the wheel: the modal around it must not scroll.
                ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            }
        }

        if resp.double_clicked() {
            if self.fit {
                // Double click is the shortcut between the fitted sheet and 1:1.
                self.zoom(1.0 / self.scale.max(MIN), Some((rect.center(), rect)));
            } else {
                self.fit = true;
            }
        }
    }

    fn zoom(&mut self, factor: f32, at: Option<(Pos2, Rect)>) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let old = self.scale.max(MIN);
        let new = (old * factor).clamp(MIN, MAX);
        if (new - old).abs() <= f32::EPSILON {
            return;
        }
        match at {
            // Keep the point under the finger where it is: the sheet moves away
            // from the anchor as it grows.
            Some((anchor, rect)) => {
                let offset = anchor - (rect.center() + self.pan);
                self.pan = anchor - rect.center() - offset * (new / old);
            }
            // A button zooms about the middle of the board, which scales the pan.
            None => self.pan *= new / old,
        }
        self.scale = new;
        self.fit = false;
    }

    /// However far the sheet is dragged, a sliver stays over the board.
    fn clamp(&mut self, rect: Rect, natural: Vec2) {
        let half = natural * self.scale * 0.5;
        let limit = rect.size() * 0.5 + half - Vec2::splat(SLIVER);
        self.pan.x = self.pan.x.clamp(-limit.x.max(0.0), limit.x.max(0.0));
        self.pan.y = self.pan.y.clamp(-limit.y.max(0.0), limit.y.max(0.0));
    }

    pub fn zoom_in(&mut self) {
        self.zoom(1.25, None);
    }

    pub fn zoom_out(&mut self) {
        self.zoom(0.8, None);
    }

    pub fn fit(&mut self) {
        self.fit = true;
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The current size, as a share of the sheet's natural one.
    pub fn label(&self) -> String {
        if self.scale <= 0.0 {
            "—".to_owned()
        } else {
            format!("{:.0}%", self.scale * 100.0)
        }
    }
}

/// The corner radius the visible part of the sheet wears: the board's only on
/// the corners the sheet actually reaches. A zoomed sheet touching every edge is
/// closed by the board's rounding; a fitted one keeps its own square corners.
fn corners(visible: Rect, board: Rect) -> egui::CornerRadius {
    let at = |a: f32, b: f32| a <= b + 0.5;
    let r = theme::radius(8);
    egui::CornerRadius {
        nw: if at(visible.min.x, board.min.x) && at(visible.min.y, board.min.y) {
            r.nw
        } else {
            0
        },
        ne: if at(board.max.x, visible.max.x) && at(visible.min.y, board.min.y) {
            r.ne
        } else {
            0
        },
        sw: if at(visible.min.x, board.min.x) && at(board.max.y, visible.max.y) {
            r.sw
        } else {
            0
        },
        se: if at(board.max.x, visible.max.x) && at(board.max.y, visible.max.y) {
            r.se
        } else {
            0
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Board {
        Board {
            key: None,
            fit: false,
            scale: 2.0,
            pan: Vec2::ZERO,
        }
    }

    fn rect() -> Rect {
        Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0))
    }

    /// The point under the cursor is the one the zoom holds still.
    #[test]
    fn a_zoom_holds_the_point_under_the_pointer() {
        let mut b = board();
        let anchor = egui::pos2(320.0, 200.0);
        let before = (anchor - (rect().center() + b.pan)) / b.scale;
        b.zoom(2.0, Some((anchor, rect())));
        let after = (anchor - (rect().center() + b.pan)) / b.scale;
        assert!(
            (before - after).length() < 0.001,
            "the sheet point slipped: {before:?} -> {after:?}"
        );
    }

    /// A button has no pointer to hold: the sheet grows about the board's middle.
    #[test]
    fn a_button_zoom_keeps_the_centre() {
        let mut b = board();
        b.pan = egui::vec2(40.0, 0.0);
        b.zoom_in();
        assert_eq!(b.scale, 2.5);
        assert_eq!(b.pan, egui::vec2(50.0, 0.0));
    }

    /// Dragging never drops the sheet entirely off its board.
    #[test]
    fn dragging_keeps_a_sliver_on_the_board() {
        let mut b = board();
        b.pan = egui::vec2(10_000.0, -10_000.0);
        b.clamp(rect(), egui::vec2(100.0, 100.0));
        let sheet =
            Rect::from_center_size(rect().center() + b.pan, egui::vec2(100.0, 100.0) * b.scale);
        assert!(sheet.intersects(rect()), "the sheet left the board");
    }

    /// A zoomed sheet reaching the board's edges is closed by the board's
    /// rounding; a fitted one keeps its own square corners.
    #[test]
    fn only_the_corners_the_sheet_reaches_are_rounded() {
        let board = Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 100.0));
        let zoomed = corners(board, board);
        assert_eq!((zoomed.nw, zoomed.ne, zoomed.sw, zoomed.se), (8, 8, 8, 8));

        let fitted = corners(
            Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(80.0, 80.0)),
            board,
        );
        assert_eq!((fitted.nw, fitted.ne, fitted.sw, fitted.se), (0, 0, 0, 0));

        let left = corners(
            Rect::from_min_max(Pos2::ZERO, egui::pos2(50.0, 100.0)),
            board,
        );
        assert_eq!((left.nw, left.sw), (8, 8));
        assert_eq!((left.ne, left.se), (0, 0));
    }

    #[test]
    fn the_scale_stays_inside_its_range() {
        let mut b = board();
        for _ in 0..100 {
            b.zoom_in();
        }
        assert_eq!(b.scale, MAX);
        for _ in 0..100 {
            b.zoom_out();
        }
        assert_eq!(b.scale, MIN);
    }

    /// The wheel is wired to the board, not to the modal's scroll: one notch
    /// over the sheet changes the zoom.
    #[test]
    fn the_wheel_zooms_the_board() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let png = include_bytes!("../../../../assets/icon.png");
        let image = theme::decode(png).unwrap();
        let texture = ctx.load_texture("wheel-test", image, egui::TextureOptions::LINEAR);
        let mut b = Board::default();
        let size = egui::vec2(400.0, 300.0);
        let over = egui::pos2(200.0, 150.0);

        crate::shared::ui::shot::frame(&ctx, size, vec![egui::Event::PointerMoved(over)], |ui| {
            b.show(
                ui,
                egui::Id::new("wheel"),
                &texture,
                egui::vec2(400.0, 300.0),
                280.0,
            );
        });
        let fit = b.scale();
        crate::shared::ui::shot::frame(
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(over),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 100.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Default::default(),
                },
            ],
            |ui| {
                b.show(
                    ui,
                    egui::Id::new("wheel"),
                    &texture,
                    egui::vec2(400.0, 300.0),
                    280.0,
                );
            },
        );
        assert!(
            b.scale() > fit,
            "the wheel did not zoom: {fit} -> {}",
            b.scale()
        );
    }
}
