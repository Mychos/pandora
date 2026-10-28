//! Reading another manager's export. One module per format, and a sniff that says
//! which one a file is before anything is parsed: what this does not recognise is
//! refused, not guessed.

pub mod bitwarden;
pub mod keepass;
pub mod onepux;

use crate::shared::i18n::t;
use crate::vault::model::{Entry, Login, Note, Otp, Vault};
use crate::vault::tree;
use serde::{Deserialize, Serialize};

/// Nothing bigger than this is an export; the cap is checked before any parser sees
/// the bytes, including the zip a 1PUX file is.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;

/// Deepest folder path an import will place a note in.
pub const DEPTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Bitwarden,
    OnePux,
    KeePass,
}

/// Everything a file carried, in the box's own shapes and nothing else: no folder
/// ids, no foreign record ids and no ordering this app would have to keep. It is what
/// an import carries, and what a sealed share hands to another box.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Bundle {
    pub logins: Vec<Login>,
    /// A note and the `a/b` path it sat at; empty for the drive root.
    pub notes: Vec<(String, Note)>,
    pub otps: Vec<Otp>,
}

impl Bundle {
    pub fn count(&self) -> usize {
        self.logins.len() + self.notes.len() + self.otps.len()
    }
}

/// Which manager wrote these bytes. `Err` is a file this app will not touch.
pub fn detect(bytes: &[u8]) -> Result<Format, String> {
    if bytes.len() > MAX_BYTES {
        return Err(t("this file is too large to import").to_owned());
    }
    if bytes.starts_with(b"PK\x03\x04") {
        return Ok(Format::OnePux);
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with('<') {
        return Ok(Format::KeePass);
    }
    if head.starts_with('{') || head.starts_with('[') {
        return Ok(Format::Bitwarden);
    }
    Err(t("this file is not a Bitwarden, 1Password or KeePass export").to_owned())
}

/// Sniff the format and read it in one step.
pub fn read(bytes: &[u8]) -> Result<Bundle, String> {
    match detect(bytes)? {
        Format::Bitwarden => bitwarden::parse(bytes),
        Format::OnePux => onepux::parse(bytes),
        Format::KeePass => keepass::parse(bytes),
    }
}

/// A JSON value as text, for the shapes that carry one.
pub(crate) fn text(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(it)) => it.trim().to_owned(),
        Some(serde_json::Value::Number(it)) => it.to_string(),
        _ => String::new(),
    }
}

/// The first of several keys a record may spell the same thing with.
pub(crate) fn first<'a>(
    value: &'a serde_json::Value,
    keys: &[&str],
) -> Option<&'a serde_json::Value> {
    keys.iter().find_map(|key| value.get(*key))
}

/// How many of each shape a file put into the box.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub logins: usize,
    pub notes: usize,
    pub otps: usize,
}

impl Placed {
    pub fn total(&self) -> usize {
        self.logins + self.notes + self.otps
    }
}

/// Put a bundle into a box, additively: nothing already there is replaced and the
/// same record twice is not added twice. A note lands in the folder its path names,
/// creating it level by level up to [`DEPTH`].
pub fn land(vault: &mut Vault, bundle: Bundle) -> Placed {
    let mut placed = Placed::default();
    for it in bundle.logins {
        let same = vault.logins.iter().any(|old| {
            old.name == it.name && old.username == it.username && old.password == it.password
        });
        if same {
            continue;
        }
        vault.logins.push(it);
        placed.logins += 1;
    }
    for (path, mut it) in bundle.notes {
        let at = tree::ensure(vault, &path);
        let digest = it.digest();
        let same = vault.drive.iter().any(|old| {
            old.name() == it.name
                && old.folder() == at.as_deref()
                && matches!(old, Entry::Note(note) if note.digest() == digest)
        });
        if same {
            continue;
        }
        it.folder = at;
        vault.drive.push(Entry::Note(it));
        placed.notes += 1;
    }
    for it in bundle.otps {
        if vault
            .otps
            .iter()
            .any(|old| old.name == it.name && old.secret == it.secret)
        {
            continue;
        }
        vault.otps.push(it);
        placed.otps += 1;
    }
    placed
}

/// A record's fields, from the `{label, value, secret}` shapes other managers write.
pub(crate) fn field(label: &str, value: &str, secret: bool) -> crate::vault::model::Field {
    crate::vault::model::Field::new(label.trim(), value.trim(), secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_named_by_what_it_starts_with() {
        assert_eq!(
            detect(b"\xef\xbb\xbf<KeePassFile/>").unwrap(),
            Format::KeePass
        );
        assert_eq!(detect(b"  <KeePassFile/>").unwrap(), Format::KeePass);
        assert_eq!(detect(b"{\"items\":[]}").unwrap(), Format::Bitwarden);
        assert_eq!(detect(b"PK\x03\x04rest").unwrap(), Format::OnePux);
        assert!(
            detect(b"name,password\nGitHub,pw").is_err(),
            "CSV has its own door"
        );
        assert!(detect(b"").is_err());
    }

    #[test]
    fn a_file_past_the_cap_is_refused_before_parsing() {
        let mut bytes = vec![b'{'; MAX_BYTES + 1];
        bytes[1] = b'}';
        assert!(detect(&bytes).is_err());
    }

    const BITWARDEN: &[u8] = include_bytes!("../../../../tests/fixtures/import-bitwarden.json");
    const KEEPASS: &[u8] = include_bytes!("../../../../tests/fixtures/import-keepass.xml");
    const ONEPUX: &[u8] = include_bytes!("../../../../tests/fixtures/import-1password.1pux");

    /// The fixtures are committed so a format change shows up as a test, not as a
    /// user's import that came back empty.
    #[test]
    fn the_bitwarden_fixture_lands_in_its_own_collections() {
        let bundle = read(BITWARDEN).unwrap();
        let mut vault = Vault::new("Box");
        let placed = land(&mut vault, bundle);

        assert_eq!(placed.logins, 1, "a login is a login");
        assert_eq!(vault.logins[0].name, "Example Login");
        assert_eq!(vault.logins[0].username, "someone@example.com");
        assert!(vault.logins[0].fields[0].secret);
        assert_eq!(placed.otps, 1, "the seed came out of the login");
        assert_eq!(vault.otps[0].name, "Example Login");
        assert_eq!(placed.notes, 1);
        assert_eq!(
            tree::path(&vault, vault.drive[0].folder())
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            ["Personal"],
            "the note landed in the folder the file named"
        );
    }

    #[test]
    fn the_keepass_fixture_lands_as_logins_and_codes() {
        let bundle = read(KEEPASS).unwrap();
        let mut vault = Vault::new("Box");
        let placed = land(&mut vault, bundle);
        assert_eq!(placed.logins, 2, "a history entry is not a record");
        assert_eq!(placed.otps, 1);
        assert_eq!(vault.logins[0].fields[0].label, "Recovery");
        assert!(vault.logins[0].fields[0].secret);
    }

    #[test]
    fn the_one_password_fixture_lands_in_its_own_collections() {
        let bundle = read(ONEPUX).unwrap();
        let mut vault = Vault::new("Box");
        let placed = land(&mut vault, bundle);
        assert_eq!(placed.logins, 1);
        assert_eq!(placed.notes, 3);
        assert_eq!(vault.logins[0].fields[0].label, "PIN");
        assert!(
            vault.drive.iter().any(|it| it.folder().is_some()),
            "a note has a folder"
        );
    }

    /// Landing the same file twice adds nothing: the second import is a no-op, which
    /// is what makes importing a slightly newer export safe.
    #[test]
    fn landing_the_same_file_twice_adds_nothing() {
        let mut vault = Vault::new("Box");
        let first = land(&mut vault, read(BITWARDEN).unwrap());
        assert_eq!(first.total(), 3);
        let second = land(&mut vault, read(BITWARDEN).unwrap());
        assert_eq!(second.total(), 0, "nothing is duplicated");
        assert_eq!(vault.folders.len(), 1, "and no folder is created twice");
    }
}
