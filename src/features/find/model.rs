//! The Ctrl+K palette: one query over the whole box, and the row the keyboard is
//! on. Searching itself is `vault/search`; this is only what the window keeps.

use crate::vault::model::Kind;

/// The palette is open, what was typed, and which row the keyboard is on.
#[derive(Default)]
pub struct Find {
    pub open: bool,
    pub query: String,
    /// The highlighted row, an index into the rows of the frame it was set in.
    /// Clamped when the list shrinks under it.
    pub at: usize,
    /// The field still wants the keyboard. A request made the frame the palette
    /// opened would be dropped: the field had not been drawn yet.
    pub needs_focus: bool,
}

/// One result, owned: the box is free to change the moment a row is chosen.
pub struct Row {
    pub id: String,
    pub name: String,
    /// The line beside the name; empty when the kind has nothing to add.
    pub detail: String,
    /// The record's own kind, or `None` for a folder.
    pub kind: Option<Kind>,
}
