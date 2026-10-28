//! Which field of a record a query matches: never a password, a seed or a file's bytes.

use crate::vault::model::{Any, Entry, Field, Folder, Kind, Note, Vault};
use std::borrow::Cow;
use std::collections::HashMap;
use zeroize::Zeroizing;

pub struct Hit<'a> {
    pub found: Found<'a>,
    /// A login's user or address, a file's size, nothing otherwise.
    pub detail: Cow<'a, str>,
    /// Lower is better: exact name, prefix, substring, field, then free text.
    pub score: u8,
}

pub enum Found<'a> {
    Record(Any<'a>),
    Folder(&'a Folder),
}

impl<'a> Hit<'a> {
    pub fn id(&self) -> &str {
        match &self.found {
            Found::Record(it) => it.id(),
            Found::Folder(it) => &it.id,
        }
    }

    pub fn name(&self) -> &str {
        match &self.found {
            Found::Record(it) => it.name(),
            Found::Folder(it) => &it.name,
        }
    }

    /// `None` for a folder, which is not a record.
    pub fn kind(&self) -> Option<Kind> {
        match &self.found {
            Found::Record(it) => Some(it.kind()),
            Found::Folder(_) => None,
        }
    }
}

/// Note text a search has already decrypted, bound to the digest it came from: a
/// closed note is read once instead of on every keystroke, and an edited one is read
/// again because its digest no longer matches. It lives as long as the palette.
#[derive(Default)]
pub struct NoteCache(HashMap<String, (String, Zeroizing<String>)>);

impl NoteCache {
    /// Keep a closed note's text unless the digest says it is already there. An open
    /// note needs no copy, and one that will not decrypt simply scores empty.
    fn keep(&mut self, vault: &Vault, note: &Note) {
        let Some(record) = note.record.as_ref() else {
            return;
        };
        if note.loaded {
            return;
        }
        if self
            .0
            .get(&note.id)
            .is_some_and(|(kept, _)| kept == &record.digest)
        {
            return;
        }
        if let Ok(text) = vault.note_text(note) {
            self.0
                .insert(note.id.clone(), (record.digest.clone(), text));
        }
    }

    /// The text to score a note by: its own body while it is open, else the copy
    /// kept for the same digest, else nothing.
    fn text<'a>(&'a self, note: &'a Note) -> &'a str {
        if note.loaded || note.record.is_none() {
            return note.body.as_str();
        }
        let digest = note
            .record
            .as_ref()
            .map_or("", |record| record.digest.as_str());
        match self.0.get(&note.id) {
            Some((kept, text)) if kept == digest => text.as_str(),
            _ => "",
        }
    }
}

/// Best first, stable within a score so the box's own order shows through a tie;
/// an empty query matches nothing. Note text comes from `notes`, which spares the
/// box a decrypt per keystroke.
pub fn find<'a>(vault: &'a Vault, query: &str, notes: &mut NoteCache) -> Vec<Hit<'a>> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<Hit<'a>> = Vec::new();
    for it in vault.records() {
        let score = match &it {
            Any::Login(login) => score(
                &login.name,
                [login.username.as_str(), login.url.as_str()]
                    .into_iter()
                    .chain(labels(&login.fields)),
                std::iter::once(login.notes.as_str()).chain(values(&login.fields)),
                &q,
            ),
            Any::Drive(Entry::Note(note)) => {
                notes.keep(vault, note);
                score(&note.name, [], [notes.text(note)], &q)
            }
            Any::Drive(Entry::File(file)) => score(&file.name, [], [], &q),
            Any::Passkey(passkey) => score(&passkey.name, [passkey.rp_id.as_str()], [], &q),
            Any::Otp(otp) => score(&otp.name, [], [], &q),
        };
        if let Some(score) = score {
            hits.push(Hit {
                detail: detail(&it),
                score,
                found: Found::Record(it),
            });
        }
    }
    for it in &vault.folders {
        if let Some(score) = score(&it.name, [], [], &q) {
            hits.push(Hit {
                found: Found::Folder(it),
                detail: Cow::Borrowed(""),
                score,
            });
        }
    }
    hits.sort_by_key(|hit| hit.score);
    hits
}

/// A login with no user falls back to its address; a file says how big it is.
fn detail<'a>(it: &Any<'a>) -> Cow<'a, str> {
    match it {
        Any::Login(it) if !it.username.is_empty() => Cow::Borrowed(it.username.as_str()),
        Any::Login(it) => Cow::Borrowed(it.url.as_str()),
        Any::Passkey(it) => Cow::Borrowed(it.rp_id.as_str()),
        Any::Drive(Entry::File(it)) => Cow::Owned(crate::shared::ui::size::human(it.bytes())),
        Any::Drive(Entry::Note(_)) | Any::Otp(_) => Cow::Borrowed(""),
    }
}

/// Every label a record's custom fields carry: a label is never a secret.
fn labels(fields: &[Field]) -> impl Iterator<Item = &str> {
    fields.iter().map(|field| field.label.as_str())
}

/// Only the values that are not marked secret; the mark is what keeps a value out
/// of a query.
fn values(fields: &[Field]) -> impl Iterator<Item = &str> {
    fields
        .iter()
        .filter(|field| !field.secret)
        .map(|field| field.value.as_str())
}

/// `fields` are shown beside the name, `body` is free text; both rank under it.
fn score<'a>(
    name: &str,
    fields: impl IntoIterator<Item = &'a str>,
    body: impl IntoIterator<Item = &'a str>,
    q: &str,
) -> Option<u8> {
    let name = name.to_lowercase();
    if name == q {
        return Some(0);
    }
    if name.starts_with(q) {
        return Some(1);
    }
    if name.contains(q) {
        return Some(2);
    }
    if fields.into_iter().any(|f| f.to_lowercase().contains(q)) {
        return Some(3);
    }
    if body.into_iter().any(|f| f.to_lowercase().contains(q)) {
        return Some(4);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::model::{Login, Note, Otp};
    use zeroize::Zeroizing;

    fn login(name: &str, username: &str, url: &str, notes: &str) -> Login {
        let mut it = Login::new(1);
        it.name = name.to_owned();
        it.username = username.to_owned();
        it.url = url.to_owned();
        it.notes = Zeroizing::new(notes.to_owned());
        it
    }

    fn vault() -> Vault {
        let mut v = Vault::new("Box");
        v.logins = vec![
            login("GitHub", "octocat", "https://github.com", "ssh key"),
            login(
                "Correo",
                "me@example.com",
                "https://mail.example",
                "recovery",
            ),
        ];
        let mut note = Note::new(1);
        note.name = "GitHub recovery".to_owned();
        note.body = Zeroizing::new("codes".to_owned());
        v.drive.push(Entry::Note(note));
        v.drive.push(Entry::File(crate::vault::model::File::new(
            "github-backup.zip".to_owned(),
            b"secret-bytes".to_vec(),
            1,
        )));
        let mut otp = Otp::new(1);
        otp.name = "GitHub".to_owned();
        otp.secret = Zeroizing::new("SEED".to_owned());
        v.otps.push(otp);
        v
    }

    fn names(hits: &[Hit<'_>]) -> Vec<String> {
        hits.iter().map(|h| h.name().to_owned()).collect()
    }

    /// One search with a cold cache: what the palette gets on its first keystroke.
    fn search<'a>(vault: &'a Vault, query: &str) -> Vec<Hit<'a>> {
        find(vault, query, &mut NoteCache::default())
    }

    /// A closed note's text is kept for the digest it was read from, and a digest
    /// that moved is never answered with the old text.
    #[test]
    fn a_closed_notes_text_survives_only_while_its_digest_does() {
        let mut notes = NoteCache::default();
        let mut note = Note::new(1);
        note.loaded = false;
        note.record = Some(crate::vault::model::BlobRecord {
            size: 5,
            digest: "old".to_owned(),
            chunks: Vec::new(),
        });
        notes.0.insert(
            note.id.clone(),
            ("old".to_owned(), Zeroizing::new("alpha".to_owned())),
        );
        assert_eq!(notes.text(&note), "alpha", "the copy is the note's text");

        let mut moved = note;
        moved.record.as_mut().unwrap().digest = "new".to_owned();
        notes.keep(&Vault::new("Box"), &moved);
        assert_eq!(
            notes.text(&moved),
            "",
            "a note that could not be read again is not answered from the old copy"
        );
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        assert!(search(&vault(), "").is_empty());
        assert!(search(&vault(), "   ").is_empty());
    }

    #[test]
    fn a_password_and_a_seed_are_never_found() {
        let mut v = Vault::new("Box");
        v.logins = vec![login("Mail", "me", "", "")];
        v.logins[0].password = Zeroizing::new("hunter2-unique".to_owned());
        let mut otp = Otp::new(1);
        otp.name = "Mail".to_owned();
        otp.secret = Zeroizing::new("JBSWY3DPEHPK3PXP".to_owned());
        v.otps.push(otp);

        assert!(
            search(&v, "hunter2-unique").is_empty(),
            "a password is not indexed"
        );
        assert!(
            search(&v, "JBSWY3DPEHPK3PXP").is_empty(),
            "a seed is not indexed"
        );
    }

    #[test]
    fn a_file_matches_only_by_name() {
        let mut v = Vault::new("Box");
        v.drive.push(Entry::File(crate::vault::model::File::new(
            "notes.txt".to_owned(),
            b"a word inside the document".to_vec(),
            1,
        )));
        assert!(
            names(&search(&v, "inside")).is_empty(),
            "the bytes are not text"
        );
        assert_eq!(names(&search(&v, "notes")), ["notes.txt"]);
    }

    #[test]
    fn the_best_match_comes_first() {
        let v = vault();
        let hits = names(&search(&v, "github"));
        assert_eq!(
            hits,
            ["GitHub", "GitHub", "GitHub recovery", "github-backup.zip"],
            "exact names first (login then code, the box's own order), then the two \
             that only start with it: {hits:?}"
        );
    }

    #[test]
    fn a_field_and_a_body_rank_under_the_name() {
        let by_user = names(&search(&vault(), "octocat"));
        assert_eq!(by_user, ["GitHub"], "the username is searchable");

        let by_notes = names(&search(&vault(), "recovery"));
        assert_eq!(
            by_notes,
            ["GitHub recovery", "Correo"],
            "a note's name beats the login's notes: {by_notes:?}"
        );
    }

    #[test]
    fn every_kind_and_a_folder_are_reachable() {
        let mut v = vault();
        v.folders.push(Folder::new("GitHub Docs", None, 1));
        let hits = search(&v, "github");
        assert_eq!(
            hits.iter().filter(|h| h.kind() == Some(Kind::Otp)).count(),
            1
        );
        let folder = hits
            .iter()
            .find(|h| h.kind().is_none())
            .expect("the folder is a hit");
        assert_eq!(folder.name(), "GitHub Docs");
    }

    #[test]
    fn a_match_ignores_case() {
        assert_eq!(names(&search(&vault(), "GITHUB")).len(), 4);
        assert_eq!(names(&search(&vault(), "OctoCat")).len(), 1);
    }
}
