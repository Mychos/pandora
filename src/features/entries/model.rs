//! How the passwords pane draws a box, and what the folders view groups by.

use serde::{Deserialize, Serialize};

/// The two ways the pane can show a box's passwords.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum View {
    /// The vault's own order, one row per entry. What the pane always did.
    #[default]
    List,
    /// The same entries gathered under folders generated from the entries
    /// themselves. Nothing is written to the vault.
    Folders,
}

impl View {
    pub fn label(self) -> &'static str {
        match self {
            View::List => crate::shared::i18n::t("List view"),
            View::Folders => crate::shared::i18n::t("Folders view"),
        }
    }
}

/// What the folders view splits on: the site an entry claims, or the username it
/// carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Axis {
    /// One folder per site.
    #[default]
    Target,
    /// One folder per username.
    User,
}

impl Axis {
    /// Every axis, in the order the picker offers them.
    pub const ALL: [Axis; 2] = [Axis::Target, Axis::User];

    /// Short name for the picker.
    pub fn label(self) -> &'static str {
        match self {
            Axis::Target => crate::shared::i18n::t("Site"),
            Axis::User => crate::shared::i18n::t("User"),
        }
    }

    /// Keeps generated folder keys apart across axes, so a fold in one axis does
    /// not fold a same-named node in another.
    pub fn key(self) -> &'static str {
        match self {
            Axis::Target => "target",
            Axis::User => "user",
        }
    }
}
