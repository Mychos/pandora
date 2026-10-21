//! Which field of a record a query matches: never a password, a seed or a file's bytes.

use crate::vault::model::{Any, Entry, Folder, Kind, Vault};
use std::borrow::Cow;

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

/// Best first, stable within a score so the box's own order shows through a tie;
/// an empty query matches nothing.
pub fn find<'a>(vault: &'a Vault, query: &str) -> Vec<Hit<'a>> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<Hit<'a>> = Vec::new();
    for it in vault.records() {
        let score = match &it {
            Any::Login(login) => score(
                &login.name,
                [login.username.as_str(), login.url.as_str()],
                [login.notes.as_str()],
                &q,
            ),
            Any::Drive(Entry::Note(note)) => {
                let body = vault.note_text(note).unwrap_or_default();
                score(&note.name, [], [body.as_str()], &q)
            }
            Any::Drive(Entry::File(file)) => score(&file.name, [], [], &q),
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
        Any::Drive(Entry::File(it)) => Cow::Owned(crate::vault::store::human_size(it.bytes())),
        Any::Drive(Entry::Note(_)) | Any::Otp(_) => Cow::Borrowed(""),
    }
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

    #[test]
    fn an_empty_query_matches_nothing() {
        assert!(find(&vault(), "").is_empty());
        assert!(find(&vault(), "   ").is_empty());
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
            find(&v, "hunter2-unique").is_empty(),
            "a password is not indexed"
        );
        assert!(
            find(&v, "JBSWY3DPEHPK3PXP").is_empty(),
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
            names(&find(&v, "inside")).is_empty(),
            "the bytes are not text"
        );
        assert_eq!(names(&find(&v, "notes")), ["notes.txt"]);
    }

    #[test]
    fn the_best_match_comes_first() {
        let v = vault();
        let hits = names(&find(&v, "github"));
        assert_eq!(
            hits,
            ["GitHub", "GitHub", "GitHub recovery", "github-backup.zip"],
            "exact names first (login then code, the box's own order), then the two \
             that only start with it: {hits:?}"
        );
    }

    #[test]
    fn a_field_and_a_body_rank_under_the_name() {
        let by_user = names(&find(&vault(), "octocat"));
        assert_eq!(by_user, ["GitHub"], "the username is searchable");

        let by_notes = names(&find(&vault(), "recovery"));
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
        let hits = find(&v, "github");
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
        assert_eq!(names(&find(&vault(), "GITHUB")).len(), 4);
        assert_eq!(names(&find(&vault(), "OctoCat")).len(), 1);
    }
}
