//! Sealed shares, from the app's side: one record sealed for somebody else, and one
//! file opened into this box. The crypto and the file format are `vault::share`; this
//! only asks the two questions a person has to answer.

pub mod ui;

use crate::shared::utils::import::Bundle;
use crate::vault::model::{Any, Entry, Vault};

/// What the modal is in the middle of: sealing one record, or opening one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Form {
    Send { id: String },
    Open { path: std::path::PathBuf },
}

#[derive(Default)]
pub struct Share {
    pub form: Option<Form>,
    /// The passphrase, for either direction. Empty means "seal to a key".
    pub passphrase: String,
    /// The recipient's public key, as they handed it over.
    pub to: String,
    /// What went wrong, said inside the dialog so the fields stay where they are.
    pub error: Option<String>,
}

impl Share {
    pub fn send(&mut self, id: &str) {
        self.clear();
        self.form = Some(Form::Send { id: id.to_owned() });
    }

    pub fn open(&mut self, path: std::path::PathBuf) {
        self.clear();
        self.form = Some(Form::Open { path });
    }

    pub fn clear(&mut self) {
        self.form = None;
        self.passphrase.clear();
        self.to.clear();
        self.error = None;
    }
}

/// The one record a share carries, as an import bundle: the same shapes, so the same
/// landing code puts it in another box. A file's bytes and a passkey's private key are
/// not shareable this way, and say so instead of travelling half-read.
pub fn bundle(vault: &Vault, id: &str) -> Result<Bundle, String> {
    let mut bundle = Bundle::default();
    match vault.any(id) {
        Some(Any::Login(it)) => bundle.logins.push(it.clone()),
        Some(Any::Otp(it)) => bundle.otps.push(it.clone()),
        Some(Any::Drive(Entry::Note(it))) => {
            let mut note = it.clone();
            note.body = vault.note_text(it)?;
            note.record = None;
            note.loaded = true;
            bundle.notes.push((String::new(), note));
        }
        Some(Any::Drive(Entry::File(_))) => {
            return Err(crate::shared::i18n::t(
                "a document is not sealed in a share — save it out and send it yourself",
            )
            .to_owned())
        }
        Some(Any::Passkey(_)) => {
            return Err(crate::shared::i18n::t(
                "a passkey cannot be shared: it answers for this box alone",
            )
            .to_owned())
        }
        None => return Err(crate::shared::i18n::t("no such record").to_owned()),
    }
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::model::{Login, Note, Passkey, Vault};
    use crate::vault::share::{self, SealTo};
    use zeroize::Zeroizing;

    fn vault() -> Vault {
        let mut v = Vault::new("Ana's box");
        let mut login = Login::new(1);
        login.id = "a".to_owned();
        login.name = "Bank".to_owned();
        login.password = Zeroizing::new("hunter2".to_owned());
        v.logins.push(login);
        let mut note = Note::new(1);
        note.id = "n".to_owned();
        note.name = "Ideas".to_owned();
        note.body = Zeroizing::new("one two".to_owned());
        v.drive.push(Entry::Note(note));
        let mut key = Passkey::new(1, "example.com");
        key.id = "p".to_owned();
        v.passkeys.push(key);
        let mut file = crate::vault::model::File::new("scan.pdf".to_owned(), b"bytes".to_vec(), 1);
        file.id = "f".to_owned();
        v.drive.push(Entry::File(file));
        v
    }

    /// One record leaves as the shape it is, and a document or a passkey says why it
    /// does not go: half a file, or a credential for this box alone, is not a share.
    #[test]
    fn one_record_leaves_and_the_two_that_cannot_say_so() {
        let v = vault();
        assert_eq!(bundle(&v, "a").unwrap().logins.len(), 1);
        assert_eq!(bundle(&v, "n").unwrap().notes[0].1.body.as_str(), "one two");
        assert!(bundle(&v, "f").is_err(), "a document has no share");
        assert!(bundle(&v, "p").is_err(), "a passkey has none either");
        assert!(bundle(&v, "ghost").is_err());
    }

    /// What is sealed here opens there, as the same records: the share is the same
    /// bundle an import lands, so nothing about it is a second path into the box.
    #[test]
    fn a_sealed_record_lands_in_another_box() {
        let from = vault();
        let bundle = bundle(&from, "a").unwrap();
        let bytes =
            share::seal(&bundle, &from.name, &SealTo::Passphrase("correct horse"), 5).unwrap();

        let mut there = Vault::new("Bob's box");
        let opened = share::open(&bytes, None, Some("correct horse")).unwrap();
        let placed = crate::shared::utils::import::land(&mut there, opened);
        assert_eq!(placed.total(), 1);
        assert_eq!(there.logins[0].password.as_str(), "hunter2");
        assert_eq!(there.logins[0].name, "Bank");

        // The same file again adds nothing: landing is idempotent like every import.
        let again = share::open(&bytes, None, Some("correct horse")).unwrap();
        assert_eq!(
            crate::shared::utils::import::land(&mut there, again).total(),
            0
        );
    }

    #[test]
    fn the_dialog_is_one_form_at_a_time() {
        let mut share = Share {
            passphrase: "x".to_owned(),
            ..Share::default()
        };
        share.send("a");
        assert_eq!(share.form, Some(Form::Send { id: "a".to_owned() }));
        assert!(share.passphrase.is_empty(), "a new dialog starts empty");

        share.error = Some("nope".to_owned());
        share.open(std::path::PathBuf::from("/tmp/x.pandora-share"));
        assert!(share.form.is_some());
        assert!(share.error.is_none(), "and forgets the last failure");
        share.clear();
        assert!(share.form.is_none());
    }
}
