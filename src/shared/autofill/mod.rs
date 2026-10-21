//! The open box, left where the phone's own autofill can read it.
//!
//! A browser asks over the bridge socket; Android's autofill service has no socket
//! to ask through and may run in a process where `android_main` never started, so
//! the app publishes here instead. Nothing reaches disk, and the copy is dropped
//! when the box locks.

use crate::bridge::protocol::{Candidate, CodeInfo};
use crate::bridge::serve;
use crate::vault::model::{now_secs, Login, Otp, Vault};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use zeroize::Zeroizing;

#[cfg(target_os = "android")]
pub(crate) mod jni;

/// `None` while no box is open: the copy lives exactly as long as one does.
static SNAPSHOT: OnceLock<Mutex<Option<Snapshot>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<Snapshot>> {
    SNAPSHOT.get_or_init(|| Mutex::new(None))
}

/// The box as published, with the seeds its codes derive from. Every secret is
/// `Zeroizing`, so dropping the copy erases it, and nothing here is serializable.
struct Snapshot {
    path: PathBuf,
    /// The box's save generation, so a re-read of the same seq is not a change.
    seq: u64,
    logins: Vec<Login>,
    otps: Vec<Otp>,
    /// Logins the service saved and the app has not put in the box yet.
    pending: Vec<Pending>,
}

/// A login the phone's panel saved. The service has no vault to write it into —
/// the app owns the only one — so it waits here for the next frame.
pub struct Pending {
    pub name: String,
    pub username: String,
    pub password: Zeroizing<String>,
    pub url: String,
}

/// Whether a fill has anything to offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Locked,
    Ready,
}

/// Follow the box on screen: published when one is open and changed, gone when
/// none is. One call per frame instead of a hook at every place a vault can move,
/// so a switch, a close or an unlock cannot forget to keep it in step.
pub fn follow(session: Option<(&Path, &Vault)>) {
    let mut slot = cell().lock().unwrap_or_else(|e| e.into_inner());
    let Some((path, vault)) = session else {
        *slot = None;
        return;
    };
    if slot
        .as_ref()
        .is_some_and(|old| old.path == path && old.seq == vault.seq)
    {
        return;
    }
    // A save the panel made is not content: replacing the copy must not drop it.
    let pending = slot.take().map_or_else(Vec::new, |old| old.pending);
    *slot = Some(Snapshot {
        path: path.to_path_buf(),
        seq: vault.seq,
        logins: vault.logins.clone(),
        otps: vault.otps.clone(),
        pending,
    });
}

/// Take a login the panel saved. `false` when no box is open here: a save with
/// nothing published has nowhere to land.
pub fn queue(login: Pending) -> bool {
    let mut slot = cell().lock().unwrap_or_else(|e| e.into_inner());
    match slot.as_mut() {
        Some(snapshot) => {
            snapshot.pending.push(login);
            true
        }
        None => false,
    }
}

/// Whether the box already holds this login. The rule an import uses: same name,
/// same username, same password is the same entry, and a fresh password off the
/// panel is a different one.
pub fn duplicate(vault: &Vault, login: &Pending) -> bool {
    vault.logins.iter().any(|entry| {
        entry.name == login.name
            && entry.username == login.username
            && entry.password == login.password
    })
}

/// Everything the panel saved since the last call, for the app to put in the box.
pub fn take() -> Vec<Pending> {
    let mut slot = cell().lock().unwrap_or_else(|e| e.into_inner());
    slot.as_mut()
        .map(|snapshot| std::mem::take(&mut snapshot.pending))
        .unwrap_or_default()
}

/// Drop the copy. Erasure needs no loop here: every secret in it is already a
/// `Zeroizing`, so dropping the `Option` zeroizes each password, note and seed in
/// place before the memory goes back. `follow` gets the same for free, since
/// replacing a copy drops the old one.
pub fn clear() {
    *cell().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn state() -> State {
    match &*cell().lock().unwrap_or_else(|e| e.into_inner()) {
        Some(_) => State::Ready,
        None => State::Locked,
    }
}

/// The box's save generation as published; 0 while there is no copy. It moves
/// whenever `store::save` does, so a reply built from an older one can be told.
pub fn generation() -> u64 {
    cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map_or(0, |snapshot| snapshot.seq)
}

/// Entries claiming the page, ranked exactly as the browser popup ranks them.
pub fn query(page: &str) -> Vec<Candidate> {
    match &*cell().lock().unwrap_or_else(|e| e.into_inner()) {
        Some(snapshot) => serve::rank(&snapshot.logins, page),
        None => Vec::new(),
    }
}

/// The username and password for one id the user picked; `None` once the copy is
/// gone. This is the only way a password leaves the copy, exactly as the browser
/// bridge reveals one: an id the user chose.
pub fn reveal(id: &str) -> Option<(String, Zeroizing<String>)> {
    let slot = cell().lock().unwrap_or_else(|e| e.into_inner());
    let login = slot.as_ref()?.logins.iter().find(|item| item.id == id)?;
    Some((login.username.clone(), login.password.clone()))
}

/// Every seed by name: never a seed, never a code.
pub fn codes() -> Vec<CodeInfo> {
    match &*cell().lock().unwrap_or_else(|e| e.into_inner()) {
        Some(snapshot) => serve::listing(&snapshot.otps),
        None => Vec::new(),
    }
}

/// The code one seed holds now, and the seconds left on it.
pub fn code(id: &str) -> Option<(Zeroizing<String>, u64)> {
    let slot = cell().lock().unwrap_or_else(|e| e.into_inner());
    let otp = slot.as_ref()?.otps.iter().find(|otp| otp.id == id)?;
    serve::derive(otp, now_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::model::Login;

    const BOX: &str = "/tmp/pandora-autofill.pandora";

    fn boxed() -> Vault {
        let mut vault = Vault::new("Personal");
        vault.seq = 1;
        let mut login = Login::new(1);
        login.name = "Google".to_owned();
        login.username = "me@example.com".to_owned();
        login.password = Zeroizing::new("hunter2".to_owned());
        login.url = "https://accounts.google.com".to_owned();
        vault.logins.push(login);
        vault
    }

    /// One test and not four: the copy is a process-wide static, and four tests
    /// would race over it the moment cargo runs them on more than one thread.
    #[test]
    fn the_copy_follows_the_box() {
        let path = Path::new(BOX);
        let mut vault = boxed();

        // Published: the domain rule claims a page under the same registrable domain.
        follow(Some((path, &vault)));
        assert_eq!(state(), State::Ready);
        assert_eq!(generation(), 1);
        let hits = query("https://mail.google.com");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].username, "me@example.com");
        assert_eq!(
            hits[0].uri.as_deref(),
            Some("https://accounts.google.com"),
            "the panel says which stored URL claimed the page"
        );

        // Nothing that travels carries the secret; one id reveals it, and only it.
        let json = serde_json::to_string(&hits).unwrap();
        assert!(
            json.contains("me@example.com"),
            "a listing that is empty proves nothing: {json}"
        );
        assert!(!json.contains("hunter2"), "{json}");
        assert!(!json.contains("password"), "{json}");
        let id = hits[0].id.clone();
        let (username, password) = reveal(&id).unwrap();
        assert_eq!(username, "me@example.com");
        assert_eq!(password.as_str(), "hunter2");
        assert!(reveal("no-such-id").is_none());

        // A page nothing claims is offered nothing, and the copy stays put.
        assert!(query("https://example.com").is_empty());
        assert_eq!(state(), State::Ready);

        // Edited and saved: a new seq is a new copy, and its generation moved.
        vault.seq = 2;
        vault.logins[0].username = "other@example.com".to_owned();
        follow(Some((path, &vault)));
        assert_eq!(generation(), 2);
        assert_eq!(
            query("https://mail.google.com")[0].username,
            "other@example.com"
        );

        // The same seq again is not a republish: the frame loop calls this often.
        follow(Some((path, &vault)));
        assert_eq!(generation(), 2);

        // A login the panel saved waits for the app, and a republication carries it.
        assert!(queue(Pending {
            name: "GitHub".to_owned(),
            username: "octocat".to_owned(),
            password: Zeroizing::new("hunter3".to_owned()),
            url: "https://github.com".to_owned(),
        }));
        vault.seq = 3;
        follow(Some((path, &vault)));
        let queued = take();
        assert_eq!(
            queued.len(),
            1,
            "the copy that replaced the old one kept it"
        );
        assert_eq!(queued[0].name, "GitHub");
        assert!(take().is_empty(), "a queue is drained once");

        // Locked: nothing left to offer, and the id that was revealed is gone.
        follow(None);
        assert_eq!(state(), State::Locked);
        assert_eq!(generation(), 0);
        assert!(query("https://mail.google.com").is_empty());
        assert!(reveal(&id).is_none());
        assert!(
            !queue(Pending {
                name: "Late".to_owned(),
                username: String::new(),
                password: Zeroizing::new(String::new()),
                url: String::new(),
            }),
            "no box means nowhere to save"
        );
    }

    /// The rule that keeps a save from the panel from landing twice, and that
    /// lets a changed password land: the same criterion an import uses.
    #[test]
    fn a_login_already_in_the_box_is_not_queued_twice() {
        let vault = boxed();
        let mut saved = Pending {
            name: "Google".to_owned(),
            username: "me@example.com".to_owned(),
            password: Zeroizing::new("hunter2".to_owned()),
            url: "https://mail.google.com".to_owned(),
        };
        assert!(duplicate(&vault, &saved), "same three fields, same entry");
        saved.password = Zeroizing::new("hunter3".to_owned());
        assert!(!duplicate(&vault, &saved), "a new password is a new entry");
        saved.name = "Other".to_owned();
        saved.password = Zeroizing::new("hunter2".to_owned());
        assert!(
            !duplicate(&vault, &saved),
            "a different name is a new entry"
        );
    }
}
