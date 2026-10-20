use crate::vault::matching::{Rule, Uri};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

pub const VAULT_VERSION: u32 = 1;

/// What a row holds, for the screen and the interchange file. The vault stores
/// four collections and no tag at all, so this is a projection of a record's own
/// type, never something written down: a note cannot be saved as a login, and a
/// drive entry is a document exactly while it carries one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Login,
    Note,
    File,
    Otp,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Login => "password",
            Kind::Note => "note",
            Kind::File => "file",
            Kind::Otp => "otp",
        }
    }
}

/// A login: the credentials handed to a site. Its own type, so a note cannot
/// carry a password and the paths that hand one over cannot reach anything else.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Login {
    pub id: String,
    pub name: String,
    pub username: String,
    #[serde(with = "zstr")]
    pub password: Zeroizing<String>,
    /// Free text that goes with the login — a recovery hint, the note that came
    /// with it from another manager.
    #[serde(with = "zstr")]
    pub notes: Zeroizing<String>,
    pub url: String,
    #[serde(default, skip_serializing_if = "Rule::is_domain")]
    pub url_match: Rule,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_urls: Vec<Uri>,
    /// Brand icon slug (e.g. "github"). None keeps the kind icon, falling back
    /// to a URL auto-suggest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub created: u64,
    pub modified: u64,
}

impl Login {
    pub fn new(now: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "New password".to_owned(),
            username: String::new(),
            password: Zeroizing::new(String::new()),
            notes: Zeroizing::new(String::new()),
            url: String::new(),
            url_match: Rule::Domain,
            extra_urls: Vec::new(),
            icon: None,
            created: now,
            modified: now,
        }
    }

    pub fn touched(&mut self, now: u64) {
        self.modified = now;
    }

    /// Every address this login answers to, the main one first, each with the
    /// rule it is matched by. The one reader of `url` + `extra_urls`, so nothing
    /// else has to know both exist.
    pub fn uris(&self) -> impl Iterator<Item = (&str, Rule)> + '_ {
        let primary = (!self.url.trim().is_empty()).then_some((self.url.as_str(), self.url_match));
        primary.into_iter().chain(
            self.extra_urls
                .iter()
                .map(|uri| (uri.url.as_str(), uri.rule)),
        )
    }
}

/// A note: the text the user typed, and nothing else. A document is not this —
/// it is a [`File`] beside it — so a note can never end up carrying one.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub name: String,
    #[serde(with = "zstr")]
    pub body: Zeroizing<String>,
    /// Folder this entry lives in. None is the drive root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    pub created: u64,
    pub modified: u64,
}

impl Note {
    pub fn new(now: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "New note".to_owned(),
            body: Zeroizing::new(String::new()),
            folder: None,
            created: now,
            modified: now,
        }
    }

    pub fn touched(&mut self, now: u64) {
        self.modified = now;
    }

    pub fn kind(&self) -> Kind {
        Kind::Note
    }
}

/// A file: one document the drive holds, bytes and all. Its own type, so a
/// document cannot be edited as text and a note cannot carry one.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct File {
    pub id: String,
    pub name: String,
    /// Folder this entry lives in. None is the drive root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(with = "b64")]
    pub data: Zeroizing<Vec<u8>>,
    pub created: u64,
    pub modified: u64,
}

impl File {
    pub fn new(name: String, data: Vec<u8>, now: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            folder: None,
            data: Zeroizing::new(data),
            created: now,
            modified: now,
        }
    }

    pub fn bytes(&self) -> usize {
        self.data.len()
    }

    pub fn touched(&mut self, now: u64) {
        self.modified = now;
    }

    pub fn kind(&self) -> Kind {
        Kind::File
    }
}

/// What the drive holds: a note or a file, in one ordered list, so the two sit
/// wherever the user dragged them. The type is what the entry is, never a field
/// the app has to keep in step with the content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Entry {
    Note(Note),
    File(File),
}

impl Entry {
    pub fn id(&self) -> &str {
        match self {
            Entry::Note(it) => &it.id,
            Entry::File(it) => &it.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Entry::Note(it) => &it.name,
            Entry::File(it) => &it.name,
        }
    }

    /// The folder this entry hangs from; the drive root when None.
    pub fn folder(&self) -> Option<&str> {
        match self {
            Entry::Note(it) => it.folder.as_deref(),
            Entry::File(it) => it.folder.as_deref(),
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Entry::Note(it) => it.kind(),
            Entry::File(it) => it.kind(),
        }
    }

    pub fn touched(&mut self, now: u64) {
        match self {
            Entry::Note(it) => it.touched(now),
            Entry::File(it) => it.touched(now),
        }
    }

    pub fn modified(&self) -> u64 {
        match self {
            Entry::Note(it) => it.modified,
            Entry::File(it) => it.modified,
        }
    }

    pub fn move_to(&mut self, folder: Option<String>) {
        match self {
            Entry::Note(it) => it.folder = folder,
            Entry::File(it) => it.folder = folder,
        }
    }
}
/// A seed for a one-time code. Never handed over: the code is derived from it, so
/// nothing copies it to a clipboard, autofills it or overwrites it with a
/// generated password.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Otp {
    pub id: String,
    pub name: String,
    #[serde(with = "zstr")]
    pub secret: Zeroizing<String>,
    pub created: u64,
    pub modified: u64,
}

impl Otp {
    pub fn new(now: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "New code".to_owned(),
            secret: Zeroizing::new(String::new()),
            created: now,
            modified: now,
        }
    }

    pub fn touched(&mut self, now: u64) {
        self.modified = now;
    }
}

/// A record that left its collection. It is not a registration any more — the
/// panes, the lookup by id and the CSV never see it — but it stays in the box
/// until the trash is emptied, which is the one thing that erases it for good.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trashed {
    /// Unix seconds it was deleted, so the list can say how long ago.
    pub deleted: u64,
    /// Where it sat in its collection, so a restore puts it back. `None` is the
    /// end: a record that was never placed anywhere in particular.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
    #[serde(flatten)]
    pub record: Deleted,
}

/// What the trash holds: the owned mirror of [`Any`], one variant per collection
/// of the box, so nothing has to ask "is this a password?" to put it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Deleted {
    Login(Login),
    Drive(Entry),
    Otp(Otp),
}

impl Deleted {
    pub fn id(&self) -> &str {
        match self {
            Deleted::Login(it) => &it.id,
            Deleted::Drive(it) => it.id(),
            Deleted::Otp(it) => &it.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Deleted::Login(it) => &it.name,
            Deleted::Drive(it) => it.name(),
            Deleted::Otp(it) => &it.name,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Deleted::Login(_) => Kind::Login,
            Deleted::Drive(it) => it.kind(),
            Deleted::Otp(_) => Kind::Otp,
        }
    }
}

/// Put a restored record back in its own collection: where it sat when it could,
/// at the end when the place is unknown or the list has shrunk past it.
fn put<T>(list: &mut Vec<T>, it: T, at: Option<usize>) {
    match at {
        Some(i) => list.insert(i.min(list.len()), it),
        None => list.push(it),
    }
}

/// One record of any kind, borrowed. The operations that are blind to what a
/// record is — selecting, dragging, moving, deleting, committing, exporting — go
/// through here, so nothing has to ask "is this a password?" to reach a name.
pub enum Any<'a> {
    Login(&'a Login),
    Drive(&'a Entry),
    Otp(&'a Otp),
}

pub enum AnyMut<'a> {
    Login(&'a mut Login),
    Drive(&'a mut Entry),
    Otp(&'a mut Otp),
}

impl<'a> Any<'a> {
    pub fn id(&self) -> &str {
        match self {
            Any::Login(it) => &it.id,
            Any::Drive(it) => it.id(),
            Any::Otp(it) => &it.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Any::Login(it) => &it.name,
            Any::Drive(it) => it.name(),
            Any::Otp(it) => &it.name,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Any::Login(_) => Kind::Login,
            Any::Drive(it) => it.kind(),
            Any::Otp(_) => Kind::Otp,
        }
    }

    pub fn modified(&self) -> u64 {
        match self {
            Any::Login(it) => it.modified,
            Any::Drive(it) => it.modified(),
            Any::Otp(it) => it.modified,
        }
    }

    /// The folder this record hangs from; only a drive entry lives in one.
    pub fn folder(&self) -> Option<&str> {
        match self {
            Any::Drive(it) => it.folder(),
            _ => None,
        }
    }
}

impl<'a> AnyMut<'a> {
    pub fn id(&self) -> &str {
        match self {
            AnyMut::Login(it) => &it.id,
            AnyMut::Drive(it) => it.id(),
            AnyMut::Otp(it) => &it.id,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            AnyMut::Login(_) => Kind::Login,
            AnyMut::Drive(it) => it.kind(),
            AnyMut::Otp(_) => Kind::Otp,
        }
    }

    /// Whether this record is the one sitting in `parent` — the drive root when
    /// that is None. Only a drive entry answers yes.
    pub fn sits_in(&self, parent: Option<&str>) -> bool {
        match self {
            AnyMut::Drive(it) => it.folder() == parent,
            _ => false,
        }
    }

    /// Move it into `folder`. A login or a code has nowhere to move to.
    pub fn move_to(&mut self, folder: Option<String>) {
        if let AnyMut::Drive(it) = self {
            it.move_to(folder);
        }
    }

    pub fn touched(&mut self, now: u64) {
        match self {
            AnyMut::Login(it) => it.touched(now),
            AnyMut::Drive(it) => it.touched(now),
            AnyMut::Otp(it) => it.touched(now),
        }
    }
}

/// A secret reads as `[REDACTED]` in `Debug`: a panic or a log line is plaintext
/// the box was meant to keep shut.
struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"[REDACTED]\"")
    }
}

impl fmt::Debug for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Login")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("username", &self.username)
            .field("password", &Redacted)
            .field("notes", &Redacted)
            .field("url", &self.url)
            .field("url_match", &self.url_match)
            .field("extra_urls", &self.extra_urls)
            .field("icon", &self.icon)
            .field("created", &self.created)
            .field("modified", &self.modified)
            .finish()
    }
}

impl fmt::Debug for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Note")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("body", &Redacted)
            .field("folder", &self.folder)
            .field("created", &self.created)
            .field("modified", &self.modified)
            .finish()
    }
}

impl fmt::Debug for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("File")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("folder", &self.folder)
            .field("data", &format_args!("{} bytes", self.data.len()))
            .field("created", &self.created)
            .field("modified", &self.modified)
            .finish()
    }
}

impl fmt::Debug for Otp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Otp")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("secret", &Redacted)
            .field("created", &self.created)
            .field("modified", &self.modified)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    /// Parent folder id. None is the drive root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub created: u64,
    pub modified: u64,
}

impl Folder {
    pub fn new(name: &str, parent: Option<String>, now: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_owned(),
            parent,
            created: now,
            modified: now,
        }
    }
}

/// One portable box: the folders of the drive, and one collection per kind of
/// record. The shape of the file is the shape of the app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vault {
    pub version: u32,
    /// The generation of what this box holds: `store::save` bumps it on every
    /// write. It is inside the box and never in the header, so it costs an
    /// attacker nothing to read — it is here so a copy *older* than the one on
    /// screen can be told apart from a newer one, which is what a rewound repo
    /// hands over. A box written before this field reads as 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub seq: u64,
    pub name: String,
    /// Folders only organise the drive view, so only a drive entry lives in one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<Folder>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub logins: Vec<Login>,
    /// The drive: notes and files in the one order the user dragged them into.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drive: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub otps: Vec<Otp>,
    /// The records deleted but not yet erased. It is part of the box and travels
    /// with it; nothing else reads it, and an empty trash writes no key at all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trash: Vec<Trashed>,
}

impl Vault {
    pub fn new(name: &str) -> Self {
        Self {
            version: VAULT_VERSION,
            seq: 0,
            name: name.to_owned(),
            folders: Vec::new(),
            logins: Vec::new(),
            drive: Vec::new(),
            otps: Vec::new(),
            trash: Vec::new(),
        }
    }

    /// Every record of the box, in the order the panes show them. Folders are not
    /// records: they organise the drive.
    pub fn records(&self) -> impl Iterator<Item = Any<'_>> {
        self.logins
            .iter()
            .map(Any::Login)
            .chain(self.drive.iter().map(Any::Drive))
            .chain(self.otps.iter().map(Any::Otp))
    }

    /// The one lookup by id: ids are unique across the collections, so a click,
    /// a drag or a commit never has to know which collection it came from.
    pub fn any(&self, id: &str) -> Option<Any<'_>> {
        self.records().find(|record| record.id() == id)
    }

    pub fn any_mut(&mut self, id: &str) -> Option<AnyMut<'_>> {
        if self.logins.iter().any(|it| it.id == id) {
            return self
                .logins
                .iter_mut()
                .find(|it| it.id == id)
                .map(AnyMut::Login);
        }
        if self.drive.iter().any(|it| it.id() == id) {
            return self
                .drive
                .iter_mut()
                .find(|it| it.id() == id)
                .map(AnyMut::Drive);
        }
        self.otps.iter_mut().find(|it| it.id == id).map(AnyMut::Otp)
    }

    pub fn has(&self, id: &str) -> bool {
        self.any(id).is_some()
    }

    /// Take a record out, wherever it lives. True when something came out. A
    /// permanent erase: the trash goes through [`Vault::take`] instead.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.logins.len() + self.drive.len() + self.otps.len();
        self.logins.retain(|it| it.id != id);
        self.drive.retain(|it| it.id() != id);
        self.otps.retain(|it| it.id != id);
        self.logins.len() + self.drive.len() + self.otps.len() != before
    }

    /// Take a record out of its collection and into the trash, remembering where
    /// it sat. `None` when the id names nothing — the caller must not mark the
    /// box dirty for a click that hit no record.
    pub fn take(&mut self, id: &str, now: u64) -> Option<Trashed> {
        let (record, at) = if let Some(i) = self.logins.iter().position(|it| it.id == id) {
            (Deleted::Login(self.logins.remove(i)), i)
        } else if let Some(i) = self.drive.iter().position(|it| it.id() == id) {
            (Deleted::Drive(self.drive.remove(i)), i)
        } else {
            let i = self.otps.iter().position(|it| it.id == id)?;
            (Deleted::Otp(self.otps.remove(i)), i)
        };
        let trashed = Trashed {
            deleted: now,
            at: Some(at),
            record,
        };
        self.trash.push(trashed.clone());
        Some(trashed)
    }

    /// Put a record back from the trash and say what it was. A drive entry whose
    /// folder is gone lands at the root: the folder is not part of the record and
    /// was not restored with it.
    pub fn restore(&mut self, id: &str) -> Option<Kind> {
        let i = self.trash.iter().position(|t| t.record.id() == id)?;
        let Trashed { at, record, .. } = self.trash.remove(i);
        let kind = record.kind();
        match record {
            Deleted::Login(it) => put(&mut self.logins, it, at),
            Deleted::Otp(it) => put(&mut self.otps, it, at),
            Deleted::Drive(mut it) => {
                if let Some(folder) = it.folder() {
                    if !self.folders.iter().any(|f| f.id == folder) {
                        it.move_to(None);
                    }
                }
                put(&mut self.drive, it, at);
            }
        }
        Some(kind)
    }

    /// Erase everything the trash holds. The only place a deleted file's bytes
    /// leave the box.
    pub fn empty_trash(&mut self) -> usize {
        std::mem::take(&mut self.trash).len()
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// serde helper: a generation of 0 is the field's absence, so a box that was
/// never written twice carries nothing extra.
fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// serde helper: bytes are stored base64, not as a JSON array of numbers.
mod b64 {
    use crate::vault::codec;
    use serde::{Deserialize, Deserializer, Serializer};
    use zeroize::Zeroizing;

    pub fn serialize<S: Serializer>(bytes: &Zeroizing<Vec<u8>>, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&codec::encode(bytes.as_ref()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Zeroizing<Vec<u8>>, D::Error> {
        let s = String::deserialize(de)?;
        let bytes = codec::decode(&s).map_err(serde::de::Error::custom)?;
        Ok(Zeroizing::new(bytes))
    }
}

mod zstr {
    use serde::{Deserialize, Deserializer, Serializer};
    use zeroize::Zeroizing;

    pub fn serialize<S: Serializer>(s: &Zeroizing<String>, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(s.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Zeroizing<String>, D::Error> {
        let s = String::deserialize(de)?;
        Ok(Zeroizing::new(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_reads_as_redacted() {
        let mut login = Login::new(1);
        login.password = Zeroizing::new("password-secret".to_owned());
        let mut note = Note::new(1);
        note.body = Zeroizing::new("body-secret".to_owned());
        let mut otp = Otp::new(1);
        otp.secret = Zeroizing::new("seed-secret".to_owned());

        let said = format!("{login:?} {note:?} {otp:?}");
        for secret in ["password-secret", "body-secret", "seed-secret"] {
            assert!(!said.contains(secret), "{secret} leaked: {said}");
        }
        assert!(said.contains("Login"), "and still says what it is: {said}");
    }

    #[test]
    fn a_record_is_found_by_id_wherever_it_lives() {
        let mut vault = Vault::new("Box");
        let mut login = Login::new(1);
        login.id = "login".to_owned();
        let mut note = Note::new(1);
        note.id = "note".to_owned();
        note.folder = Some("docs".to_owned());
        let mut otp = Otp::new(1);
        otp.id = "otp".to_owned();
        vault.logins.push(login);
        vault.drive.push(Entry::Note(note));
        vault.otps.push(otp);

        assert_eq!(vault.any("otp").unwrap().kind(), Kind::Otp);
        assert_eq!(vault.any("note").unwrap().folder(), Some("docs"));
        assert_eq!(vault.any("login").unwrap().folder(), None);
        assert!(vault.any("ghost").is_none());

        assert_eq!(vault.records().count(), 3);
        assert!(vault.remove("note"), "a note comes out of the drive");
        assert!(!vault.remove("note"), "and not twice");
        assert_eq!(vault.drive.len(), 0);
        assert_eq!(vault.records().count(), 2);
    }

    /// A delete is a move, not a loss: the record leaves the box the panes see
    /// and waits in the trash with the place it held.
    #[test]
    fn a_trashed_record_leaves_the_box_but_keeps_its_place() {
        let mut vault = Vault::new("Box");
        for (id, name) in [("one", "One"), ("two", "Two"), ("three", "Three")] {
            let mut it = Login::new(1);
            it.id = id.to_owned();
            it.name = name.to_owned();
            vault.logins.push(it);
        }

        let trashed = vault.take("two", 42).expect("the record goes in");
        assert_eq!(trashed.deleted, 42);
        assert_eq!(trashed.record.name(), "Two");
        assert_eq!(vault.logins.len(), 2);
        assert_eq!(vault.trash.len(), 1);
        assert!(
            vault.any("two").is_none(),
            "a trashed record is not a record of the box"
        );
        assert!(vault.take("ghost", 42).is_none(), "nothing to take");

        assert_eq!(vault.restore("two"), Some(Kind::Login));
        assert_eq!(
            vault
                .logins
                .iter()
                .map(|it| it.name.as_str())
                .collect::<Vec<_>>(),
            ["One", "Two", "Three"],
            "it sits where it did"
        );
        assert!(vault.trash.is_empty(), "and the trash lets it go");
        assert_eq!(vault.restore("two"), None, "once");
    }

    /// The folder is not restored with the entry: one that is gone leaves the
    /// entry at the drive root instead of pointing at nothing.
    #[test]
    fn a_restore_whose_folder_is_gone_lands_at_the_root() {
        let mut vault = Vault::new("Box");
        let mut note = Note::new(1);
        note.id = "note".to_owned();
        note.folder = Some("docs".to_owned());
        vault.drive.push(Entry::Note(note));

        vault.take("note", 2).unwrap();
        assert_eq!(vault.restore("note"), Some(Kind::Note));
        assert_eq!(vault.drive[0].folder(), None, "`docs` was never there");
    }

    /// Empting the trash is the one erase: what it held is gone from the box and
    /// writes nothing more.
    #[test]
    fn emptying_the_trash_erases_what_it_held() {
        let mut vault = Vault::new("Box");
        vault.drive.push(Entry::File(File::new(
            "report.pdf".to_owned(),
            b"bytes".to_vec(),
            1,
        )));
        let id = vault.drive[0].id().to_owned();

        vault.take(&id, 3).unwrap();
        assert_eq!(vault.empty_trash(), 1);
        assert!(vault.trash.is_empty());
        assert!(vault.restore(&id).is_none(), "there is nothing to put back");
        let round = serde_json::to_string(&vault).unwrap();
        assert!(!round.contains("trash"), "and nothing to write: {round}");
    }

    /// An older box knows nothing of the trash, and a box nobody deleted from
    /// writes no key at all.
    #[test]
    fn an_old_box_reads_with_an_empty_trash() {
        let v: Vault = serde_json::from_str(r#"{"version":1,"name":"Old"}"#).unwrap();
        assert!(v.trash.is_empty());
    }

    /// The trash is part of the encrypted payload, so it round-trips like every
    /// other record — and the seed of a trashed code stays hidden.
    #[test]
    fn trash_round_trips() {
        let mut vault = Vault::new("Box");
        let mut otp = Otp::new(1);
        otp.id = "otp".to_owned();
        otp.name = "GitHub".to_owned();
        otp.secret = Zeroizing::new("seed-secret".to_owned());
        vault.otps.push(otp);
        vault.take("otp", 7).unwrap();

        let round: Vault = serde_json::from_str(&serde_json::to_string(&vault).unwrap()).unwrap();
        assert_eq!(round.trash, vault.trash);
        assert_eq!(round.trash[0].record.name(), "GitHub");
        assert!(round.trash[0].at.is_some(), "the place travels too");
    }

    #[test]
    fn only_a_drive_entry_changes_folder() {
        let mut vault = Vault::new("Box");
        let mut login = Login::new(1);
        login.id = "login".to_owned();
        let mut note = Note::new(1);
        note.id = "note".to_owned();
        vault.logins.push(login);
        vault.drive.push(Entry::Note(note));

        vault
            .any_mut("note")
            .unwrap()
            .move_to(Some("docs".to_owned()));
        vault
            .any_mut("login")
            .unwrap()
            .move_to(Some("docs".to_owned()));

        assert!(vault.any_mut("note").unwrap().sits_in(Some("docs")));
        assert!(!vault.any_mut("login").unwrap().sits_in(Some("docs")));
        assert_eq!(vault.drive[0].folder(), Some("docs"));
    }

    /// An empty box writes none of the four collections, so a box nobody put a
    /// note in is not a box with an empty note list.
    #[test]
    fn an_empty_box_writes_no_empty_collection() {
        let round = serde_json::to_string(&Vault::new("Box")).unwrap();
        for key in ["folders", "logins", "drive", "otps", "trash"] {
            assert!(!round.contains(key), "{key} in {round}");
        }
    }

    /// A note and a file are two types, and the list they share says which one
    /// each entry is: a note cannot carry a document, a file cannot carry text.
    #[test]
    fn a_drive_entry_says_what_it_is() {
        let mut note = Note::new(1);
        note.id = "note".to_owned();
        let mut file = File::new("report.pdf".to_owned(), b"bytes".to_vec(), 1);
        file.id = "file".to_owned();
        file.folder = Some("docs".to_owned());
        let vault = Vault {
            drive: vec![Entry::Note(note), Entry::File(file)],
            ..Vault::new("Box")
        };

        assert_eq!(vault.any("note").unwrap().kind(), Kind::Note);
        assert_eq!(vault.any("file").unwrap().kind(), Kind::File);
        assert_eq!(vault.any("file").unwrap().folder(), Some("docs"));

        let round = serde_json::to_string(&vault).unwrap();
        assert!(round.contains(r#""type":"note""#), "{round}");
        assert!(round.contains(r#""type":"file""#), "{round}");
        let back: Vault = serde_json::from_str(&round).unwrap();
        assert_eq!(back, vault, "a document survives the round trip");
    }
}
