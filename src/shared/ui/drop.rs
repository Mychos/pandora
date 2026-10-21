//! Where a drop lands and the line that marks it, shared by the drive, the tree
//! and the lists so every drop feels the same.

use crate::app::Drag;
use crate::shared::theme;
use egui::{Rect, Ui};

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
