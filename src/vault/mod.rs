//! The vault: one encrypted file, its crypto, its model and the reads over it.

pub mod codec;
pub(crate) mod container;
pub mod crypto;
pub mod health;
pub mod matching;
pub mod merge;
pub mod model;
pub(crate) mod reader;
pub mod recovery;
pub mod search;
pub mod share;
pub mod store;
pub mod totp;
pub mod tree;

use std::fmt;

/// The failures a caller has to tell apart from every other message. An error
/// crosses the crate as text; these are the ones whose text is decided here, so a
/// caller matches the kind instead of repeating the sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// The key does not open this box, or the box cannot be read.
    Wrong,
    /// A copy from a repo is older than the one on disk.
    RolledBack,
}

impl Failure {
    /// The vault's own words for it; the sentence a screen shows.
    pub fn text(self) -> &'static str {
        match self {
            Failure::Wrong => "wrong master password or corrupt box",
            Failure::RolledBack => "the copy is older than this box",
        }
    }

    /// Which failure a message names, or `None` for the many that are just text.
    pub fn of(message: &str) -> Option<Self> {
        [Failure::Wrong, Failure::RolledBack]
            .into_iter()
            .find(|failure| failure.text() == message)
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_reads_back_from_its_own_words() {
        assert_eq!(Failure::of(Failure::Wrong.text()), Some(Failure::Wrong));
        assert_eq!(
            Failure::of(Failure::RolledBack.text()),
            Some(Failure::RolledBack)
        );
        assert_eq!(Failure::of("cannot read box: no such file"), None);
    }
}
