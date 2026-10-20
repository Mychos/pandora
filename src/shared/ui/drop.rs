//! Where a dragged row or card would land, and the line that says so. The drive
//! grid, the folder tree and the lists all resolve their drops the same way, so
//! the rule lives here once and every list keeps the same feel.

use crate::app::Drag;
use crate::shared::theme;
use egui::{Rect, Ui};

/// Where a dragged card or row would land: next to the target (`Side`) or inside
/// it (`Into`, folders only).
#[derive(Clone, Copy)]
pub enum Landing {
    Into,
    Side { after: bool },
}

/// Resolve a drop over `rect`. `horizontal` is the target's own axis: grid cards
/// lay out left to right, tree rows and list rows top to bottom. A folder takes a
/// drop into its centre and a reorder on its edge; an entry only reorders (by
/// half). An entry over a folder is always a move into it; a folder over an entry
/// is a no-op.
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

/// The insertion line for a `Side` landing: a vertical rule for a grid card, a
/// horizontal one for a tree row or a list row.
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
