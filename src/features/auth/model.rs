//! Boxes opened before, so the Unlock screen can offer one-click reopen. A plain
//! list of paths, outside any box, so it never touches the box format — and with no
//! name in it: what a row is called is the file's, not the box's.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use crate::vault::model::Vault;
use crate::vault::store::Credentials;

const KEEP: usize = 12;

/// Where a phone's copy of a box came from: the document the picker handed over, and
/// when that document last said it changed. A platform that can open a file in place
/// needs neither — there is nothing stale to compare.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Origin {
    pub uri: String,
    /// Milliseconds since the epoch, in the provider's own stamp.
    pub stamp: u64,
}

/// A box the user has unlocked at least once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recent {
    /// Where it is. The list is the way back into a box, so it has to point at the
    /// file; what it never carries is the box's own name.
    pub path: PathBuf,
    /// Set only for a copy a picker made: a box this machine owns is never stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Recent {
    /// A box this machine owns: no copy behind it, so nothing can ever be older
    /// than anything.
    pub fn at(path: PathBuf) -> Self {
        Self { path, origin: None }
    }

    /// What the row says: the file's own name, without the extension. The name the
    /// box goes under is inside the box, and this list sits outside every box with
    /// nothing encrypted in it — so it does not repeat it.
    pub fn label(&self) -> String {
        self.path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "box".to_owned())
    }
}

#[derive(Default, Serialize, Deserialize)]
struct List {
    #[serde(default)]
    entries: Vec<Recent>,
}

pub fn load() -> Vec<Recent> {
    read(&dir())
}

pub fn remember(path: &Path, origin: Option<Origin>) -> Vec<Recent> {
    let dir = dir();
    match origin {
        Some(origin) => push_from(&dir, path, origin),
        None => push(&dir, path),
    }
    read(&dir)
}

fn dir() -> PathBuf {
    crate::vault::store::data_dir()
}

fn file(dir: &Path) -> PathBuf {
    dir.join("recent.json")
}

fn read(dir: &Path) -> Vec<Recent> {
    let Ok(bytes) = fs::read(file(dir)) else {
        return Vec::new();
    };
    let list: List = serde_json::from_slice(&bytes).unwrap_or_default();
    list.entries
        .into_iter()
        .filter(|e| e.path.exists())
        .collect()
}

/// Writing is best effort: losing the list costs only the convenience, never an unlock.
///
/// `pub(crate)` so the contract test can write this file beside the other two and
/// check what none of them may say.
pub(crate) fn push(dir: &Path, path: &Path) {
    write(dir, path, None);
}

/// The same row, carrying where the copy came from: a phone's copy is the only one
/// that can outlive the document it was made from.
pub(crate) fn push_from(dir: &Path, path: &Path, origin: Origin) {
    write(dir, path, Some(origin));
}

fn write(dir: &Path, path: &Path, origin: Option<Origin>) {
    let mut entries = read(dir);
    // A pick says where the copy now comes from; every other open says nothing about
    // that, and must not forget it — the second unlock of a copy is the same copy.
    let before = entries
        .iter()
        .find(|e| e.path == path)
        .and_then(|e| e.origin.clone());
    entries.retain(|e| e.path != path);
    entries.insert(
        0,
        Recent {
            path: path.to_path_buf(),
            origin: origin.or(before),
        },
    );
    entries.truncate(KEEP);
    let Ok(bytes) = serde_json::to_vec(&List { entries }) else {
        return;
    };
    if fs::create_dir_all(dir).is_ok() {
        let _ = crate::vault::store::write_private(&file(dir), &bytes);
    }
}

#[derive(Default)]
pub struct WelcomeForm {
    pub name: String,
    pub path: String,
    /// The user wrote the file's path, or picked one: from then on the box's name
    /// proposes nothing, because a destination chosen by hand is the user's.
    pub path_locked: bool,
    pub password: String,
    pub confirm: String,
    /// Optional second factor: a file mixed into the KDF.
    pub keyfile: String,
}

impl WelcomeForm {
    /// The file follows the box's name while the path is nobody's yet. Only the
    /// file's own name is replaced: the folder the user is creating in stays.
    pub fn follow(&mut self) {
        if self.path_locked {
            return;
        }
        let Some(file) = crate::vault::store::file_name(&self.name) else {
            return;
        };
        let dir = Path::new(&self.path)
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(crate::vault::store::data_dir);
        self.path = dir.join(file).display().to_string();
    }
}

#[derive(Default)]
pub struct ChangeForm {
    pub current: String,
    pub new: String,
    pub confirm: String,
    /// Required again when the vault already uses a keyfile.
    pub keyfile: String,
    /// Take the box out of the repo as part of the change. Off unless asked:
    /// it removes the copy the repo holds, and the commits behind it stay until
    /// that history is rewritten.
    pub erase_remote: bool,
}

/// The box's key, enrolled or dropped: the master password is asked again because the
/// key is derived from it, and a keyfile is required again when the box uses one.
#[derive(Default)]
pub struct KeyForm {
    pub password: String,
    pub keyfile: String,
}

/// Long operations run on a worker thread (Argon2 takes seconds), so the UI never freezes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Create,
    Unlock,
    ChangePassword,
    /// Enrolling the YubiKey, or dropping it from a box. A touch and a derivation,
    /// both slow enough that the frame cannot wait for them.
    EnrolKey,
    /// Opening a box with the device's reader. The prompt itself is what takes
    /// the time; the vault then opens with the key the keystore released.
    Biometric,
    /// Keeping a box's key for the device's reader. A prompt too, and for the same
    /// reason: the keystore will not let that key wrap anything without a finger.
    Seal,
    /// Picking a box file on a platform whose picker is not a window.
    Pick,
    /// Asking the document a phone's copy came from whether it moved on since.
    Stale,
}

#[allow(clippy::large_enum_variant)] // the vault rides out of the worker once and is taken, not kept
pub enum OpOutcome {
    Entered(PathBuf, Credentials, Vault),
    /// Carries the file it was rotated on: the vault may have left the screen while the derivation ran.
    /// The flag is the "erase the repo's copy" the form was answered with.
    PasswordChanged(PathBuf, Credentials, bool),
    /// The box was re-derived with the key mixed in, or without it. The flag is which.
    KeyChanged(PathBuf, Credentials, bool),
    /// The box file the picker brought in, to be filled into the form, and where
    /// that copy came from when the platform had to make one.
    Picked(PathBuf, Option<Origin>),
    /// A destination uri the picker chose for a new box.
    Destination(String),
    /// The reader kept the open box's key. Backing out of the prompt is not this:
    /// it is a cancelled operation, and nothing is said about it.
    Kept,
    /// The document a phone's copy came from, and when it says it last changed:
    /// the stamp the copy was taken at, and the answer, if the provider gave one.
    Stale(u64, Option<u64>),
}

pub struct Op {
    pub kind: OpKind,
    pub rx: mpsc::Receiver<Result<OpOutcome, String>>,
}

/// What a sign-in reports as it goes. Only the last two variants end the flow.
pub enum SignIn {
    /// The user code, where it is typed, and the page the browser is handed.
    Code {
        /// The one value the user carries: what the card draws and copies.
        code: String,
        /// The bare page, without the code in it, for the sentence that says
        /// where the code goes.
        page: String,
        /// The address the browser opens, with the code already in it: GitHub's
        /// own `verification_uri_complete` when it sends one, `device_url`
        /// otherwise. Built by the worker because the minted reply is what knows
        /// which spelling GitHub wants.
        open: String,
    },
    /// The session GitHub minted. The signing-in half of the app does not read
    /// it: it is handed straight to the sync state, which is what knows what a
    /// session is.
    Token(crate::features::sync::github::Session),
    Failed(String),
}

/// A GitHub sign-in on a worker thread; the code arrives before the token.
pub struct Login {
    /// `(user code, page)` once GitHub has minted them, held for the card to show.
    pub code: Option<(String, String)>,
    pub rx: mpsc::Receiver<SignIn>,
}

/// Which view of the sources screen is on show. The recents gather every box
/// opened before, whatever view it lives under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    /// The files on this machine.
    Machine,
    Github,
    Recents,
}

/// Everything that happens while no box is open: unlocking, creating, remembering.
#[derive(Default)]
pub struct Auth {
    /// The view of the sources screen the user picked. None lets the screen
    /// choose: the local files when there is one to reopen, GitHub otherwise.
    pub side: Option<Side>,
    pub unlock_path: String,
    pub unlock_password: String,
    /// Optional keyfile path on the unlock screen. Never stored in the box.
    pub unlock_keyfile: String,
    /// A printed recovery code, when the password is what was lost.
    pub unlock_code: String,
    /// The code a box just made, shown once and zeroized when the card is done with it.
    pub code_shown: Option<zeroize::Zeroizing<String>>,
    /// Boxes opened before, most recent first. Outside any box.
    pub recents: Vec<Recent>,
    pub recent_search: String,
    pub welcome: WelcomeForm,
    pub change: ChangeForm,
    pub key: KeyForm,
    pub op: Option<Op>,
    pub login: Option<Login>,
    /// The origin of the box the picker just brought in, waiting for the unlock
    /// that will put it in the recents beside the path it belongs to.
    pub picked: Option<(PathBuf, Origin)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pandora-recent-{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, b"x").unwrap();
        p
    }

    fn labels(list: &[Recent]) -> Vec<String> {
        list.iter().map(Recent::label).collect()
    }

    #[test]
    fn remembers_newest_first_by_its_file_and_never_twice() {
        let dir = tmp("order");
        let a = touch(&dir, "a.box");
        let b = touch(&dir, "b.box");

        push(&dir, &a);
        push(&dir, &b);
        assert_eq!(labels(&read(&dir)), ["b", "a"]);

        push(&dir, &a); // reopening lifts it back, no duplicate
        let list = read(&dir);
        assert_eq!(labels(&list), ["a", "b"]);
        assert_eq!(list[0].path, a, "the row keeps the file it opens");
    }

    /// What the list may say about a box: where the file is, and nothing from
    /// inside it. The name a box goes under is sealed in the box itself.
    #[test]
    fn the_list_carries_no_box_name() {
        let dir = tmp("names");
        let p = touch(&dir, "personal.box");
        push(&dir, &p);

        let text = fs::read_to_string(file(&dir)).unwrap();
        assert!(text.contains("personal.box"), "it has to point at the file");
        assert!(
            !text.contains("\"name\""),
            "and it carries no name at all: {text}"
        );
        assert_eq!(labels(&read(&dir)), ["personal"]);
    }

    /// A list written by a build that stored the box's name still loads: the field
    /// it spelled is ignored, and the next write leaves it out for good.
    #[test]
    fn a_list_that_still_spells_a_name_is_read_and_rewritten_without_it() {
        let dir = tmp("legacy-name");
        let p = touch(&dir, "work.box");
        fs::write(
            file(&dir),
            format!(
                r#"{{"entries":[{{"path":"{}","name":"Divorce lawyer"}}]}}"#,
                p.display()
            ),
        )
        .unwrap();

        assert_eq!(
            labels(&read(&dir)),
            ["work"],
            "the row keeps the file's name"
        );
        push(&dir, &p);
        let text = fs::read_to_string(file(&dir)).unwrap();
        assert!(!text.contains("Divorce"), "and the name left: {text}");
    }

    /// The row remembers the document a phone's copy came from, so a later open can
    /// say whether that document moved on; and it keeps that memory when the same
    /// copy is opened again, which says nothing about where it came from.
    #[test]
    fn a_picked_copy_remembers_its_document_until_it_is_picked_again() {
        let dir = tmp("origin");
        let p = touch(&dir, "personal.box");
        let origin = |stamp: u64| Origin {
            uri: "content://docs/personal".to_owned(),
            stamp,
        };

        push_from(&dir, &p, origin(1_700_000_000_000));
        assert_eq!(
            read(&dir)[0].origin.as_ref().map(|o| o.stamp),
            Some(1_700_000_000_000),
            "the stamp the copy was taken at travels with the row"
        );

        push(&dir, &p);
        assert!(
            read(&dir)[0].origin.is_some(),
            "unlocking the copy again does not forget its document"
        );

        push_from(&dir, &p, origin(1_800_000_000_000));
        assert_eq!(
            read(&dir)[0].origin.as_ref().map(|o| o.stamp),
            Some(1_800_000_000_000),
            "a new pick is a new copy, and a new stamp"
        );
    }

    /// A row with no copy behind it never grows one: this machine's own file has
    /// nothing older to be behind.
    #[test]
    fn a_local_box_carries_no_origin() {
        let dir = tmp("plain");
        let p = touch(&dir, "work.box");
        push(&dir, &p);
        assert!(read(&dir)[0].origin.is_none());
        assert!(
            !fs::read_to_string(file(&dir)).unwrap().contains("origin"),
            "and the field is not even written"
        );
    }

    #[test]
    fn drops_files_that_are_gone_and_caps_the_list() {
        let dir = tmp("cap");
        let real = touch(&dir, "real.box");

        push(&dir, &dir.join("gone.box")); // never created
        push(&dir, &real);
        assert_eq!(labels(&read(&dir)), ["real"], "the missing file is dropped");

        for i in 0..KEEP + 3 {
            let p = touch(&dir, &format!("f{i}.box"));
            push(&dir, &p);
        }
        assert_eq!(read(&dir).len(), KEEP);
        assert_eq!(labels(&read(&dir))[0], format!("f{}", KEEP + 2));
    }
}

/// The file follows the box's name while nobody has touched the path; a path the
/// user wrote is theirs, and the folder stays whatever it was.
#[test]
fn the_file_follows_the_box_name_until_the_path_is_the_users() {
    let mut form = WelcomeForm {
        name: "Casa de Ana".to_owned(),
        path: "/tmp/vault.pandora".to_owned(),
        ..Default::default()
    };
    form.follow();
    assert_eq!(
        form.path, "/tmp/casa-de-ana.pandora",
        "the folder stays and the file takes the name"
    );

    form.path_locked = true;
    form.name = "Otra".to_owned();
    form.follow();
    assert_eq!(
        form.path, "/tmp/casa-de-ana.pandora",
        "a path the user owns is not rewritten"
    );

    let mut unnamed = WelcomeForm {
        name: "···".to_owned(),
        path: "/tmp/vault.pandora".to_owned(),
        ..Default::default()
    };
    unnamed.follow();
    assert_eq!(
        unnamed.path, "/tmp/vault.pandora",
        "a name with nothing in it proposes nothing"
    );
}
