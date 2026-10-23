use crate::vault::crypto;
use crate::vault::matching::{Rule, Uri};
use crate::vault::reader::BoxReader;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

pub const VAULT_VERSION: u32 = 1;

/// What a row holds, for the screen and the CSV: a projection of the record's
/// type, never something written down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Login,
    Card,
    Identity,
    Passkey,
    Note,
    File,
    Otp,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Login => "password",
            Kind::Card => "card",
            Kind::Identity => "identity",
            Kind::Passkey => "passkey",
            Kind::Note => "note",
            Kind::File => "file",
            Kind::Otp => "otp",
        }
    }
}

/// One line a record carries beyond its own shape: a recovery code, a PIN, the
/// number on a card. `secret` keeps it out of a search and behind the eye.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub label: String,
    #[serde(with = "zstr")]
    pub value: Zeroizing<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
}

impl Field {
    pub fn new(label: &str, value: &str, secret: bool) -> Self {
        Self {
            label: label.to_owned(),
            value: Zeroizing::new(value.to_owned()),
            secret,
        }
    }
}

/// A password the record no longer uses, newest last.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Old {
    #[serde(with = "zstr")]
    pub password: Zeroizing<String>,
    pub changed: u64,
}

/// A login's own type, so a note cannot carry a password.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Login {
    pub id: String,
    pub name: String,
    pub username: String,
    #[serde(with = "zstr")]
    pub password: Zeroizing<String>,
    /// Free text: a recovery hint, the note that came with it from another manager.
    #[serde(with = "zstr")]
    pub notes: Zeroizing<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Old>,
    pub url: String,
    #[serde(default, skip_serializing_if = "Rule::is_domain")]
    pub url_match: Rule,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_urls: Vec<Uri>,
    /// The keys this login types, when auto-type runs: `{USERNAME}`, `{PASSWORD}`,
    /// `{OTP}`, `{ENTER}`, `{TAB}` and `{WAIT 250}`. Empty means the default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sequence: String,
    /// Brand icon slug; `None` keeps the kind icon, then the URL's suggestion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub created: u64,
    pub modified: u64,
}

impl Login {
    /// How many old passwords ride along; the oldest leaves first.
    pub const HISTORY: usize = 20;

    pub fn new(now: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "New password".to_owned(),
            username: String::new(),
            password: Zeroizing::new(String::new()),
            notes: Zeroizing::new(String::new()),
            fields: Vec::new(),
            history: Vec::new(),
            sequence: String::new(),
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

    /// Take a new password, keeping the one it displaces; the same one twice is not a
    /// change. Returns whether the password moved.
    pub fn set_password(&mut self, password: &str) -> bool {
        if self.password.as_str() == password {
            return false;
        }
        let previous = std::mem::take(&mut self.password);
        self.password = Zeroizing::new(password.to_owned());
        self.remember(previous)
    }

    /// Keep `password` as one this login no longer uses. The editor binds the field in
    /// place, so it hands over the value it displaced instead of calling
    /// [`Login::set_password`].
    pub fn remember(&mut self, password: Zeroizing<String>) -> bool {
        if password.is_empty() || password == self.password {
            return false;
        }
        self.history.push(Old {
            password,
            changed: now_secs(),
        });
        let over = self.history.len().saturating_sub(Self::HISTORY);
        self.history.drain(..over);
        true
    }

    /// Main URL first, then the extras, each with its rule; the only reader of both.
    pub fn uris(&self) -> impl Iterator<Item = (&str, Rule)> + '_ {
        let primary = (!self.url.trim().is_empty()).then_some((self.url.as_str(), self.url_match));
        primary.into_iter().chain(
            self.extra_urls
                .iter()
                .map(|uri| (uri.url.as_str(), uri.rule)),
        )
    }
}

/// A card's own type, so a number and an expiry never ride a login's fields.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub name: String,
    #[serde(default, with = "zstr")]
    pub notes: Zeroizing<String>,
    /// The long number, spaces and all: kept as typed, grouped by the UI.
    #[serde(default, with = "zstr")]
    pub number: Zeroizing<String>,
    /// `MM/YY`, as printed on the card.
    #[serde(default)]
    pub expiry: String,
    #[serde(default)]
    pub holder: String,
    /// Brand icon slug, the way a login carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
    pub created: u64,
    pub modified: u64,
}

impl Card {
    pub fn new(now: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "New card".to_owned(),
            notes: Zeroizing::new(String::new()),
            number: Zeroizing::new(String::new()),
            expiry: String::new(),
            holder: String::new(),
            icon: None,
            fields: Vec::new(),
            created: now,
            modified: now,
        }
    }

    pub fn touched(&mut self, now: u64) {
        self.modified = now;
    }

    pub fn kind(&self) -> Kind {
        Kind::Card
    }
}

/// Who one is on paper: names, addresses, a passport number. Its own type, so none
/// of it lands in a login or a note.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    pub id: String,
    pub name: String,
    #[serde(default, with = "zstr")]
    pub notes: Zeroizing<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
    pub created: u64,
    pub modified: u64,
}

impl Identity {
    pub fn new(now: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "New identity".to_owned(),
            notes: Zeroizing::new(String::new()),
            fields: Vec::new(),
            created: now,
            modified: now,
        }
    }

    pub fn touched(&mut self, now: u64) {
        self.modified = now;
    }

    pub fn kind(&self) -> Kind {
        Kind::Identity
    }
}

/// An Ed25519 key this box keeps for its SSH agent. `allowed` is the switch the user
/// sets: a key that is not marked is not listed and does not sign.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SshKey {
    pub id: String,
    pub name: String,
    /// Base64 of the 32-byte seed; `Zeroizing`, and never in a `Debug`.
    #[serde(with = "b64")]
    pub private: Zeroizing<Vec<u8>>,
    /// The 32-byte public key, as base64.
    #[serde(with = "b64")]
    pub public: Zeroizing<Vec<u8>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allowed: bool,
    pub created: u64,
    pub modified: u64,
}

impl SshKey {
    /// A fresh Ed25519 key, made here and never imported: a private key file from the
    /// outside is its own parser and its own policy, and this box can make its own.
    pub fn make(now: u64, name: &str, comment: &str) -> Self {
        let seed = crypto::random_bytes::<32>();
        let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
        let public = signing.verifying_key().to_bytes();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_owned(),
            private: Zeroizing::new(seed.to_vec()),
            public: Zeroizing::new(public.to_vec()),
            comment: comment.to_owned(),
            allowed: false,
            created: now,
            modified: now,
        }
    }

    pub fn touched(&mut self, now: u64) {
        self.modified = now;
    }
}

impl fmt::Debug for SshKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshKey")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("private", &Redacted)
            .field(
                "public",
                &crate::vault::codec::display(self.public.as_slice()).to_string(),
            )
            .field("comment", &self.comment)
            .field("allowed", &self.allowed)
            .finish()
    }
}

/// A WebAuthn credential: the private key never leaves this box, and the record is
/// bound to one origin. The id and the handle are bytes, not text, so they ride as
/// base64 like every other byte field.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Passkey {
    pub id: String,
    pub name: String,
    /// The relying party's id — `example.com` — and the only site this answers for.
    pub rp_id: String,
    #[serde(default, with = "b64")]
    pub user_handle: Zeroizing<Vec<u8>>,
    #[serde(with = "b64")]
    pub credential_id: Zeroizing<Vec<u8>>,
    /// COSE algorithm: -7 is ES256, -8 is EdDSA.
    pub alg: i32,
    /// Incremented on every assertion; a relying party may look at it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub counter: u32,
    #[serde(with = "b64")]
    pub private_key: Zeroizing<Vec<u8>>,
    pub created: u64,
    pub modified: u64,
}

impl Passkey {
    pub fn new(now: u64, rp_id: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: rp_id.to_owned(),
            rp_id: rp_id.to_owned(),
            user_handle: Zeroizing::new(Vec::new()),
            credential_id: Zeroizing::new(Vec::new()),
            alg: -7,
            counter: 0,
            private_key: Zeroizing::new(Vec::new()),
            created: now,
            modified: now,
        }
    }

    pub fn touched(&mut self, now: u64) {
        self.modified = now;
    }

    pub fn kind(&self) -> Kind {
        Kind::Passkey
    }
}

/// One encrypted chunk. `nonce` is base64, `len` includes the AEAD tag, `offset`
/// is absolute in the `.pandora` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkRef {
    pub offset: u64,
    pub len: u64,
    pub nonce: String,
}

/// Where a file or note lives. `digest` is the BLAKE2 of the plaintext, so
/// equality never needs the bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlobRecord {
    pub size: u64,
    pub digest: String,
    pub chunks: Vec<ChunkRef>,
}

/// A file picked or dropped but not yet written into the box; `owned` marks the
/// picker copy removed after the save.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingFile {
    pub path: PathBuf,
    pub size: u64,
    pub owned: bool,
}

/// A note: text and nothing else. A document is a [`File`], so a note can never
/// carry one.
#[derive(Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub name: String,
    #[serde(default, with = "zstr")]
    pub body: Zeroizing<String>,
    /// Where the body lives in a v4 box; `None` until saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<BlobRecord>,
    /// The body is in `body` and may be edited. Not persisted.
    #[serde(skip)]
    pub loaded: bool,
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
            record: None,
            loaded: true,
            folder: None,
            created: now,
            modified: now,
        }
    }

    pub fn bytes(&self) -> u64 {
        if self.loaded || self.record.is_none() {
            return self.body.len() as u64;
        }
        self.record.as_ref().map_or(0, |record| record.size)
    }

    pub fn digest(&self) -> String {
        if self.loaded || self.record.is_none() {
            return crypto::digest(self.body.as_bytes());
        }
        self.record.as_ref().map_or_else(
            || crypto::digest(self.body.as_bytes()),
            |r| r.digest.clone(),
        )
    }

    pub fn touched(&mut self, now: u64) {
        self.modified = now;
    }

    pub fn kind(&self) -> Kind {
        Kind::Note
    }
}

impl PartialEq for Note {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.folder == other.folder
            && self.created == other.created
            && self.modified == other.modified
            && self.bytes() == other.bytes()
            && self.digest() == other.digest()
    }
}

/// One document the drive holds; its own type, so a note cannot carry one.
#[derive(Clone, Serialize, Deserialize)]
pub struct File {
    pub id: String,
    pub name: String,
    /// Folder this entry lives in. None is the drive root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(with = "b64")]
    pub data: Zeroizing<Vec<u8>>,
    /// Where the bytes live in a v4 box; `None` until an inline file is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<BlobRecord>,
    /// Picked or dropped, not yet read into the box.
    #[serde(skip)]
    pub pending: Option<PendingFile>,
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
            record: None,
            pending: None,
            created: now,
            modified: now,
        }
    }

    /// Stays on disk until the save chunks it; `owned` marks a picker copy the app
    /// removes after writing it into the box.
    pub fn new_pending(name: String, path: PathBuf, size: u64, owned: bool, now: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            folder: None,
            data: Zeroizing::new(Vec::new()),
            record: None,
            pending: Some(PendingFile { path, size, owned }),
            created: now,
            modified: now,
        }
    }

    pub fn bytes(&self) -> usize {
        match &self.pending {
            Some(pending) => pending.size as usize,
            None => self
                .record
                .as_ref()
                .map_or(self.data.len(), |record| record.size as usize),
        }
    }

    pub fn digest(&self) -> Option<String> {
        if let Some(record) = &self.record {
            return Some(record.digest.clone());
        }
        if self.pending.is_some() {
            return None;
        }
        Some(crypto::digest(&self.data))
    }

    pub fn touched(&mut self, now: u64) {
        self.modified = now;
    }

    pub fn kind(&self) -> Kind {
        Kind::File
    }
}

impl PartialEq for File {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.folder == other.folder
            && self.created == other.created
            && self.modified == other.modified
            && self.bytes() == other.bytes()
            && self.digest() == other.digest()
    }
}

/// A note or a file in one ordered list; the type is what it is, never a field.
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

    pub fn name_mut(&mut self) -> &mut String {
        match self {
            Entry::Note(it) => &mut it.name,
            Entry::File(it) => &mut it.name,
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
/// A one-time seed: never handed over, only the derived code is.
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

/// Deleted but not erased; emptying the trash is the only erase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trashed {
    pub deleted: u64,
    /// Where it sat, so a restore puts it back; `None` is the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
    #[serde(flatten)]
    pub record: Deleted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Deleted {
    Login(Login),
    Card(Card),
    Identity(Identity),
    Passkey(Passkey),
    Drive(Entry),
    Otp(Otp),
}

impl Deleted {
    pub fn id(&self) -> &str {
        match self {
            Deleted::Login(it) => &it.id,
            Deleted::Card(it) => &it.id,
            Deleted::Identity(it) => &it.id,
            Deleted::Passkey(it) => &it.id,
            Deleted::Drive(it) => it.id(),
            Deleted::Otp(it) => &it.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Deleted::Login(it) => &it.name,
            Deleted::Card(it) => &it.name,
            Deleted::Identity(it) => &it.name,
            Deleted::Passkey(it) => &it.name,
            Deleted::Drive(it) => it.name(),
            Deleted::Otp(it) => &it.name,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Deleted::Login(_) => Kind::Login,
            Deleted::Card(_) => Kind::Card,
            Deleted::Identity(_) => Kind::Identity,
            Deleted::Passkey(_) => Kind::Passkey,
            Deleted::Drive(it) => it.kind(),
            Deleted::Otp(_) => Kind::Otp,
        }
    }
}

/// Where it sat, or at the end when the place is gone or the list shrank.
fn put<T>(list: &mut Vec<T>, it: T, at: Option<usize>) {
    match at {
        Some(i) => list.insert(i.min(list.len()), it),
        None => list.push(it),
    }
}

/// Any kind, borrowed; the kind-blind operations go through here.
pub enum Any<'a> {
    Login(&'a Login),
    Card(&'a Card),
    Identity(&'a Identity),
    Passkey(&'a Passkey),
    Drive(&'a Entry),
    Otp(&'a Otp),
}

pub enum AnyMut<'a> {
    Login(&'a mut Login),
    Card(&'a mut Card),
    Identity(&'a mut Identity),
    Passkey(&'a mut Passkey),
    Drive(&'a mut Entry),
    Otp(&'a mut Otp),
}

impl<'a> Any<'a> {
    pub fn id(&self) -> &str {
        match self {
            Any::Login(it) => &it.id,
            Any::Card(it) => &it.id,
            Any::Identity(it) => &it.id,
            Any::Passkey(it) => &it.id,
            Any::Drive(it) => it.id(),
            Any::Otp(it) => &it.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Any::Login(it) => &it.name,
            Any::Card(it) => &it.name,
            Any::Identity(it) => &it.name,
            Any::Passkey(it) => &it.name,
            Any::Drive(it) => it.name(),
            Any::Otp(it) => &it.name,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Any::Login(_) => Kind::Login,
            Any::Card(_) => Kind::Card,
            Any::Identity(_) => Kind::Identity,
            Any::Passkey(_) => Kind::Passkey,
            Any::Drive(it) => it.kind(),
            Any::Otp(_) => Kind::Otp,
        }
    }

    pub fn modified(&self) -> u64 {
        match self {
            Any::Login(it) => it.modified,
            Any::Card(it) => it.modified,
            Any::Identity(it) => it.modified,
            Any::Passkey(it) => it.modified,
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
            AnyMut::Card(it) => &it.id,
            AnyMut::Identity(it) => &it.id,
            AnyMut::Passkey(it) => &it.id,
            AnyMut::Drive(it) => it.id(),
            AnyMut::Otp(it) => &it.id,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            AnyMut::Login(_) => Kind::Login,
            AnyMut::Card(_) => Kind::Card,
            AnyMut::Identity(_) => Kind::Identity,
            AnyMut::Passkey(_) => Kind::Passkey,
            AnyMut::Drive(it) => it.kind(),
            AnyMut::Otp(_) => Kind::Otp,
        }
    }

    pub fn name_mut(&mut self) -> &mut String {
        match self {
            AnyMut::Login(it) => &mut it.name,
            AnyMut::Card(it) => &mut it.name,
            AnyMut::Identity(it) => &mut it.name,
            AnyMut::Passkey(it) => &mut it.name,
            AnyMut::Drive(it) => it.name_mut(),
            AnyMut::Otp(it) => &mut it.name,
        }
    }

    /// The brand slug this shape carries; a note, a code and an identity carry none.
    pub fn icon_mut(&mut self) -> Option<&mut Option<String>> {
        match self {
            AnyMut::Login(it) => Some(&mut it.icon),
            AnyMut::Card(it) => Some(&mut it.icon),
            _ => None,
        }
    }

    /// Whether this record sits in `parent` (the drive root when None).
    pub fn sits_in(&self, parent: Option<&str>) -> bool {
        match self {
            AnyMut::Drive(it) => it.folder() == parent,
            _ => false,
        }
    }

    /// Move it into `folder`. A login, a card, an identity or a code has nowhere to
    /// move to.
    pub fn move_to(&mut self, folder: Option<String>) {
        if let AnyMut::Drive(it) = self {
            it.move_to(folder);
        }
    }

    pub fn touched(&mut self, now: u64) {
        match self {
            AnyMut::Login(it) => it.touched(now),
            AnyMut::Card(it) => it.touched(now),
            AnyMut::Identity(it) => it.touched(now),
            AnyMut::Passkey(it) => it.touched(now),
            AnyMut::Drive(it) => it.touched(now),
            AnyMut::Otp(it) => it.touched(now),
        }
    }
}

/// A secret reads as `[REDACTED]` in `Debug`, so a panic or a log cannot print it.
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
            .field("fields", &format_args!("{} field(s)", self.fields.len()))
            .field(
                "history",
                &format_args!("{} old password(s)", self.history.len()),
            )
            .field("url", &self.url)
            .field("url_match", &self.url_match)
            .field("extra_urls", &self.extra_urls)
            .field("icon", &self.icon)
            .field("created", &self.created)
            .field("modified", &self.modified)
            .finish()
    }
}

impl fmt::Debug for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Card")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("notes", &Redacted)
            .field("number", &Redacted)
            .field("expiry", &self.expiry)
            .field("holder", &self.holder)
            .field("icon", &self.icon)
            .field("fields", &format_args!("{} field(s)", self.fields.len()))
            .field("created", &self.created)
            .field("modified", &self.modified)
            .finish()
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("notes", &Redacted)
            .field("fields", &format_args!("{} field(s)", self.fields.len()))
            .field("created", &self.created)
            .field("modified", &self.modified)
            .finish()
    }
}

impl fmt::Debug for Passkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Passkey")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("rp_id", &self.rp_id)
            .field(
                "user_handle",
                &format_args!("{} bytes", self.user_handle.len()),
            )
            .field(
                "credential_id",
                &format_args!("{} bytes", self.credential_id.len()),
            )
            .field("alg", &self.alg)
            .field("counter", &self.counter)
            .field("private_key", &Redacted)
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
            .field("data", &format_args!("{} bytes", self.bytes()))
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

/// One portable box: the drive's folders and one collection per kind of record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vault {
    pub version: u32,
    /// Generation, bumped by `store::save`; 0 reads as absent in older boxes. It
    /// lives inside the payload so a rewound copy can be told from a newer one.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub seq: u64,
    pub name: String,
    /// Folders only organise the drive view, so only a drive entry lives in one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<Folder>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub logins: Vec<Login>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cards: Vec<Card>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identities: Vec<Identity>,
    /// WebAuthn credentials; a private key lives here and nowhere else.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passkeys: Vec<Passkey>,
    /// Ed25519 keys for the SSH agent; only the ones marked `allowed` ever answer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ssh_keys: Vec<SshKey>,
    /// The keypair this box receives sealed shares with; absent until one is asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share: Option<crate::vault::share::Keypair>,
    /// The drive: notes and files in the one order the user dragged them into.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drive: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub otps: Vec<Otp>,
    /// Deleted but not erased; it travels inside the box, and empty writes no key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trash: Vec<Trashed>,
    /// The v4 reader for the chunks on disk; not part of the manifest.
    #[serde(skip)]
    pub(crate) reader: Option<Arc<BoxReader>>,
}

impl PartialEq for Vault {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
            && self.seq == other.seq
            && self.name == other.name
            && self.folders == other.folders
            && self.logins == other.logins
            && self.cards == other.cards
            && self.identities == other.identities
            && self.passkeys == other.passkeys
            && self.ssh_keys == other.ssh_keys
            && self.share == other.share
            && self.drive == other.drive
            && self.otps == other.otps
            && self.trash == other.trash
    }
}

/// A record-backed note's text, read out of the box: the one place the note's
/// chunks are decrypted and decoded.
fn load_note(reader: &BoxReader, note: &Note) -> Result<Zeroizing<String>, String> {
    let record = note
        .record
        .as_ref()
        .ok_or_else(|| "the note has no chunks to read".to_owned())?;
    let bytes = reader.read_record(crate::vault::container::Kind::Note, &note.id, record)?;
    let text =
        String::from_utf8(bytes.to_vec()).map_err(|_| "the note is not valid text".to_owned())?;
    Ok(Zeroizing::new(text))
}

impl Vault {
    pub fn new(name: &str) -> Self {
        Self {
            version: VAULT_VERSION,
            seq: 0,
            name: name.to_owned(),
            folders: Vec::new(),
            logins: Vec::new(),
            cards: Vec::new(),
            identities: Vec::new(),
            passkeys: Vec::new(),
            ssh_keys: Vec::new(),
            share: None,
            drive: Vec::new(),
            otps: Vec::new(),
            trash: Vec::new(),
            reader: None,
        }
    }

    /// Inline, pending path or chunks on disk; check `file.bytes()` against the
    /// caller's limit first.
    pub fn read_file(&self, file: &File) -> Result<Zeroizing<Vec<u8>>, String> {
        if let Some(pending) = &file.pending {
            let bytes = std::fs::read(&pending.path)
                .map_err(|e| format!("cannot read {}: {e}", file.name))?;
            return Ok(Zeroizing::new(bytes));
        }
        if let Some(record) = &file.record {
            let reader = self
                .reader
                .as_ref()
                .ok_or_else(|| "the box is no longer open".to_owned())?;
            return reader.read_record(crate::vault::container::Kind::File, &file.id, record);
        }
        Ok(file.data.clone())
    }

    /// The key this box receives sealed shares with, made the first time it is asked
    /// for: a keypair is not a thing to walk the user through.
    pub fn share_key(&mut self) -> &crate::vault::share::Keypair {
        if self.share.is_none() {
            self.share = Some(crate::vault::share::Keypair::make());
        }
        self.share.as_ref().expect("just made")
    }

    /// The first `len` bytes; never reads more than the box holds.
    pub fn read_prefix(&self, file: &File, len: usize) -> Result<Zeroizing<Vec<u8>>, String> {
        if let Some(pending) = &file.pending {
            let mut out = vec![0u8; len.min(pending.size as usize)];
            let mut input = std::fs::File::open(&pending.path)
                .map_err(|e| format!("cannot read {}: {e}", file.name))?;
            use std::io::Read as _;
            input
                .read_exact(&mut out)
                .map_err(|e| format!("cannot read {}: {e}", file.name))?;
            return Ok(Zeroizing::new(out));
        }
        if let Some(record) = &file.record {
            let reader = self
                .reader
                .as_ref()
                .ok_or_else(|| "the box is no longer open".to_owned())?;
            return reader.read_range(
                crate::vault::container::Kind::File,
                &file.id,
                record,
                0,
                len as u64,
            );
        }
        Ok(Zeroizing::new(
            file.data[..len.min(file.data.len())].to_vec(),
        ))
    }

    /// Stream a file to a writer without assembling it in memory.
    pub fn copy_file_to(&self, file: &File, out: &mut impl std::io::Write) -> Result<(), String> {
        if let Some(pending) = &file.pending {
            let mut input = std::fs::File::open(&pending.path)
                .map_err(|e| format!("cannot read {}: {e}", file.name))?;
            std::io::copy(&mut input, out)
                .map_err(|e| format!("cannot write {}: {e}", file.name))?;
            return Ok(());
        }
        if let Some(record) = &file.record {
            let reader = self
                .reader
                .as_ref()
                .ok_or_else(|| "the box is no longer open".to_owned())?;
            return reader.copy_record_to(
                crate::vault::container::Kind::File,
                &file.id,
                record,
                out,
            );
        }
        out.write_all(&file.data)
            .map_err(|e| format!("cannot write {}: {e}", file.name))
    }

    /// A note's text, loading its chunks when the note is record-backed.
    pub fn note_text(&self, note: &Note) -> Result<Zeroizing<String>, String> {
        if note.loaded || note.record.is_none() {
            return Ok(note.body.clone());
        }
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| "the box is no longer open".to_owned())?;
        load_note(reader, note)
    }

    /// Bring a record-backed note into memory for the editor. Idempotent.
    pub fn ensure_note(&mut self, id: &str) -> Result<(), String> {
        let Some(reader) = self.reader.clone() else {
            return Ok(());
        };
        let Some(note) = self.drive.iter_mut().find_map(|entry| match entry {
            Entry::Note(note) if note.id == id => Some(note),
            _ => None,
        }) else {
            return Ok(());
        };
        if note.record.is_none() {
            note.loaded = true;
            return Ok(());
        }
        if note.loaded {
            return Ok(());
        }
        note.body = load_note(&reader, note)?;
        note.loaded = true;
        Ok(())
    }

    /// Every record in pane order; folders are not records.
    pub fn records(&self) -> impl Iterator<Item = Any<'_>> {
        self.logins
            .iter()
            .map(Any::Login)
            .chain(self.cards.iter().map(Any::Card))
            .chain(self.identities.iter().map(Any::Identity))
            .chain(self.passkeys.iter().map(Any::Passkey))
            .chain(self.drive.iter().map(Any::Drive))
            .chain(self.otps.iter().map(Any::Otp))
    }

    /// The one lookup by id: ids are unique across collections, so callers never
    /// ask which collection a record came from.
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
        if self.cards.iter().any(|it| it.id == id) {
            return self
                .cards
                .iter_mut()
                .find(|it| it.id == id)
                .map(AnyMut::Card);
        }
        if self.identities.iter().any(|it| it.id == id) {
            return self
                .identities
                .iter_mut()
                .find(|it| it.id == id)
                .map(AnyMut::Identity);
        }
        if self.passkeys.iter().any(|it| it.id == id) {
            return self
                .passkeys
                .iter_mut()
                .find(|it| it.id == id)
                .map(AnyMut::Passkey);
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

    /// Permanent erase; the trash goes through [`Vault::take`].
    pub fn remove(&mut self, id: &str) -> bool {
        let count = |v: &Self| {
            v.logins.len()
                + v.cards.len()
                + v.identities.len()
                + v.passkeys.len()
                + v.drive.len()
                + v.otps.len()
        };
        let before = count(self);
        self.logins.retain(|it| it.id != id);
        self.cards.retain(|it| it.id != id);
        self.identities.retain(|it| it.id != id);
        self.passkeys.retain(|it| it.id != id);
        self.drive.retain(|it| it.id() != id);
        self.otps.retain(|it| it.id != id);
        count(self) != before
    }

    /// Take a record into the trash, remembering where it sat; `None` when the id
    /// names nothing, and the caller must not mark the box dirty.
    pub fn take(&mut self, id: &str, now: u64) -> Option<Trashed> {
        let (record, at) = if let Some(i) = self.logins.iter().position(|it| it.id == id) {
            (Deleted::Login(self.logins.remove(i)), i)
        } else if let Some(i) = self.cards.iter().position(|it| it.id == id) {
            (Deleted::Card(self.cards.remove(i)), i)
        } else if let Some(i) = self.identities.iter().position(|it| it.id == id) {
            (Deleted::Identity(self.identities.remove(i)), i)
        } else if let Some(i) = self.passkeys.iter().position(|it| it.id == id) {
            (Deleted::Passkey(self.passkeys.remove(i)), i)
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

    /// Put a record back and say what it was; a drive entry whose folder is gone
    /// lands at the root, since the folder was not restored with it.
    pub fn restore(&mut self, id: &str) -> Option<Kind> {
        let i = self.trash.iter().position(|t| t.record.id() == id)?;
        let Trashed { at, record, .. } = self.trash.remove(i);
        let kind = record.kind();
        match record {
            Deleted::Login(it) => put(&mut self.logins, it, at),
            Deleted::Card(it) => put(&mut self.cards, it, at),
            Deleted::Identity(it) => put(&mut self.identities, it, at),
            Deleted::Passkey(it) => put(&mut self.passkeys, it, at),
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

    /// The only place a deleted file's bytes leave the box.
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

/// serde helper: a zero is the field's absence, whatever its width.
fn is_zero<T: PartialEq + From<u8>>(value: &T) -> bool {
    *value == T::from(0)
}

/// serde helper: `secret: false` is the field's absence.
fn is_false(value: &bool) -> bool {
    !*value
}

/// serde helper: bytes are stored base64, not as a JSON array of numbers.
mod b64 {
    use crate::vault::codec;
    use serde::{Deserialize, Deserializer, Serializer};
    use zeroize::Zeroizing;

    pub fn serialize<S: Serializer>(bytes: &Zeroizing<Vec<u8>>, ser: S) -> Result<S::Ok, S::Error> {
        ser.collect_str(&codec::display(bytes.as_ref()))
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
        let mut card = Card::new(1);
        card.notes = Zeroizing::new("card-note-secret".to_owned());
        let mut identity = Identity::new(1);
        identity.notes = Zeroizing::new("identity-note-secret".to_owned());
        login.fields.push(Field::new("PIN", "pin-secret", true));
        login.remember(Zeroizing::new("old-password-secret".to_owned()));

        let said = format!("{login:?} {note:?} {otp:?} {card:?} {identity:?}");
        for secret in [
            "password-secret",
            "body-secret",
            "seed-secret",
            "card-note-secret",
            "identity-note-secret",
            "pin-secret",
            "old-password-secret",
        ] {
            assert!(!said.contains(secret), "{secret} leaked: {said}");
        }
        assert!(said.contains("Login"), "and still says what it is: {said}");
    }

    #[test]
    fn a_replaced_password_is_kept_behind_and_capped() {
        let mut login = Login::new(1);
        assert!(!login.set_password("first"), "no password to displace yet");
        assert!(login.history.is_empty(), "the first one is not history");

        assert!(login.set_password("second"));
        assert_eq!(login.history.len(), 1);
        assert_eq!(login.history[0].password.as_str(), "first");
        assert!(
            !login.set_password("second"),
            "the same one twice is not a change"
        );
        assert_eq!(login.history.len(), 1);

        for i in 0..(Login::HISTORY + 5) {
            login.set_password(&format!("pass-{i}"));
        }
        assert_eq!(
            login.history.len(),
            Login::HISTORY,
            "the list has a ceiling"
        );
        let last = format!("pass-{}", Login::HISTORY + 3);
        assert_eq!(login.history.last().unwrap().password.as_str(), last);
    }

    #[test]
    fn the_editor_hands_over_what_it_displaced() {
        let mut login = Login::new(1);
        login.password = Zeroizing::new("in-place".to_owned());
        let previous = login.password.clone();
        login.password = Zeroizing::new("in-place-2".to_owned());

        assert!(login.remember(previous));
        assert_eq!(login.history[0].password.as_str(), "in-place");
        assert!(
            !login.remember(Zeroizing::new("in-place-2".to_owned())),
            "the password in force is not history"
        );
    }

    /// A passkey is a record like any other, and its private key never shows up in a
    /// `Debug`, a listing or a field the screen reads.
    #[test]
    fn a_passkey_is_a_record_and_hides_its_key() {
        let mut vault = Vault::new("Box");
        let mut it = Passkey::new(1, "example.com");
        it.id = "pk".to_owned();
        it.credential_id = Zeroizing::new(vec![1, 2, 3]);
        it.private_key = Zeroizing::new(b"private-key-bytes".to_vec());
        vault.passkeys.push(it);

        assert_eq!(vault.records().count(), 1);
        assert_eq!(vault.any("pk").unwrap().kind(), Kind::Passkey);
        assert_eq!(vault.any("pk").unwrap().name(), "example.com");
        assert_eq!(vault.any_mut("pk").unwrap().kind(), Kind::Passkey);

        let said = format!("{vault:?}");
        assert!(!said.contains("private-key-bytes"), "{said}");
        let json = serde_json::to_string(&vault.passkeys[0]).unwrap();
        assert!(!json.contains("private-key-bytes"), "bytes ride as base64");
        assert!(json.contains("\"alg\":-7"));

        let back: Passkey = serde_json::from_str(&json).unwrap();
        assert_eq!(back.credential_id.as_slice(), &[1, 2, 3]);
        assert_eq!(back.private_key.as_slice(), b"private-key-bytes");

        let trashed = vault.take("pk", 5).expect("a passkey goes to the trash");
        assert_eq!(trashed.record.kind(), Kind::Passkey);
        assert_eq!(vault.restore("pk"), Some(Kind::Passkey));
        assert_eq!(vault.passkeys.len(), 1);
        assert!(vault.remove("pk"));
        assert!(vault.passkeys.is_empty());
    }

    #[test]
    fn a_card_and_an_identity_are_records_of_their_own() {
        let mut vault = Vault::new("Box");
        let mut card = Card::new(1);
        card.id = "card".to_owned();
        let mut identity = Identity::new(1);
        identity.id = "identity".to_owned();
        vault.cards.push(card);
        vault.identities.push(identity);

        assert_eq!(vault.records().count(), 2);
        assert_eq!(vault.any("card").unwrap().kind(), Kind::Card);
        assert_eq!(vault.any("identity").unwrap().kind(), Kind::Identity);
        assert_eq!(
            vault.any("card").unwrap().folder(),
            None,
            "a folder organises the drive and nothing else"
        );
        assert_eq!(vault.any_mut("identity").unwrap().kind(), Kind::Identity);

        let trashed = vault.take("card", 5).expect("a card goes to the trash");
        assert_eq!(trashed.record.kind(), Kind::Card);
        assert!(vault.cards.is_empty(), "and leaves its collection");
        assert_eq!(vault.trash.len(), 1);
        assert_eq!(vault.restore("card"), Some(Kind::Card));
        assert_eq!(vault.cards.len(), 1, "and comes back to its own");

        assert!(vault.remove("identity"), "the erase finds it");
        assert!(vault.identities.is_empty());
        assert_eq!(vault.records().count(), 1);
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

    #[test]
    fn an_old_box_reads_with_an_empty_trash() {
        let v: Vault = serde_json::from_str(r#"{"version":1,"name":"Old"}"#).unwrap();
        assert!(v.trash.is_empty());
    }

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

    #[test]
    fn an_empty_box_writes_no_empty_collection() {
        let round = serde_json::to_string(&Vault::new("Box")).unwrap();
        for key in ["folders", "logins", "drive", "otps", "trash"] {
            assert!(!round.contains(key), "{key} in {round}");
        }
    }

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
