//! Searching the whole box from one field: Ctrl+K opens a palette over every
//! pane, and choosing a row shows that record where it lives. The matching is
//! `vault/search`; this owns the window.

pub mod logic;
pub mod model;
pub mod ui;
