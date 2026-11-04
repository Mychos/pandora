//! Where a drop lands and the line that marks it, shared by the drive, the tree
//! and the lists so every drop feels the same.

use crate::app::Drag;
use crate::shared::theme;
use egui::{Rect, Ui};

/// Whether a drag is a reorder at all. A mouse drags a row; a finger scrolls the list
/// it landed on, because a phone has no hover, no second button and no other way to
/// move a pane — with the row claiming the gesture, the `ScrollArea` under it never
/// sees the drag and the list cannot be scrolled at all.
pub fn reorder(ctx: &egui::Context) -> bool {
    !ctx.input(|i| i.has_touch_screen())
}

/// The sense a draggable record takes. `Sense::click` on a touch screen, so the drag
/// falls through to the scroll area that contains it instead of picking the row up.
pub fn sense(ctx: &egui::Context) -> egui::Sense {
    if reorder(ctx) {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::click()
    }
}

/// Next to the target (`Side`) or inside it (`Into`, folders only).
#[derive(Clone, Copy)]
pub enum Landing {
    Into,
    Side { after: bool },
}

/// Resolve a drop over `rect`; `horizontal` is the target's axis. A folder takes a
/// drop into its centre and a reorder on its edge, an entry only reorders; an entry
/// over a folder always moves into it, a folder over an entry does nothing.
pub fn landing(ui: &Ui, rect: Rect, horizontal: bool, target_folder: bool) -> Option<Landing> {
    let carried = egui::DragAndDrop::payload::<Drag>(ui.ctx())?;
    let p = ui.ctx().pointer_interact_pos()?;
    if !rect.contains(p) {
        return None;
    }
    let half = || {
        if horizontal {
            (p.x - rect.min.x) / rect.width().max(1.0)
        } else {
            (p.y - rect.min.y) / rect.height().max(1.0)
        }
    };
    match (&*carried, target_folder) {
        (Drag::Entry(_), true) => Some(Landing::Into),
        (Drag::Folder(_), false) => None,
        (_, true) => {
            const EDGE: f32 = 0.3;
            let f = half();
            if f < EDGE {
                Some(Landing::Side { after: false })
            } else if f > 1.0 - EDGE {
                Some(Landing::Side { after: true })
            } else {
                Some(Landing::Into)
            }
        }
        (_, false) => Some(Landing::Side {
            after: half() > 0.5,
        }),
    }
}

/// The insertion line for a `Side` landing: vertical for a grid card, horizontal
/// for a row.
pub fn mark(ui: &Ui, rect: Rect, after: bool, horizontal: bool) {
    let (a, b) = if horizontal {
        let x = if after { rect.max.x } else { rect.min.x };
        (egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y))
    } else {
        let y = if after { rect.max.y } else { rect.min.y };
        (egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y))
    };
    ui.painter()
        .line_segment([a, b], theme::stroke(theme::ACCENT, 2.0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::ui::shot;

    /// A finger and a mouse part ways here: the same press-and-move scrolls a list on
    /// the phone and picks a row up with a mouse, and it is egui's own knowledge of
    /// what touched the screen that decides, not a compile-time guess about the device.
    #[test]
    fn a_finger_scrolls_where_a_mouse_reorders() {
        let ctx = egui::Context::default();
        let size = egui::vec2(400.0, 300.0);
        let mut mouse = (false, egui::Sense::click());
        let _ = shot::frame(&ctx, size, Vec::new(), |ui| {
            mouse = (reorder(ui.ctx()), sense(ui.ctx()));
        });
        assert!(mouse.0, "a mouse has no scroll area to hand the drag to");
        assert_eq!(mouse.1, egui::Sense::click_and_drag());

        let touch = egui::Event::Touch {
            device_id: egui::TouchDeviceId(0),
            id: egui::TouchId(0),
            phase: egui::TouchPhase::Start,
            pos: egui::pos2(10.0, 10.0),
            force: None,
        };
        let mut finger = (true, egui::Sense::click_and_drag());
        let _ = shot::frame(&ctx, size, vec![touch], |ui| {
            finger = (reorder(ui.ctx()), sense(ui.ctx()));
        });
        assert!(
            !finger.0,
            "a finger's drag belongs to the list, not the row"
        );
        assert_eq!(finger.1, egui::Sense::click(), "so the row is only a click");
    }
}
