//! Everything the app knows about its mirrors — the GitHub repos it can reach,
//! the folder it writes the box into, the files each holds — and the
//! requests in flight.

use crate::features::settings::model::Source;
use crate::features::sync::github::{self, Avatars, Quota, Repo};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub enum Reply {
    /// The repo's bytes and the blob sha they belong to.
    Pulled(Vec<u8>, String),
    /// The blob sha the repo holds now, and the shape it now holds: what the
    /// commit that took it there described.
    Pushed(String, crate::features::sync::github::Fingerprint),
    /// GitHub is refusing writes for this many seconds.
    Refused(u64),
    /// The file moved since this box last read it.
    Conflict,
    /// The repo's copy of the box is gone, as asked.
    Erased,
}

/// A pull or push running on a worker thread. It carries the file it was started
/// on: the box may leave the screen while the request is out, and the reply must
/// still land on the box that asked for it.
pub struct Job {
    pub path: PathBuf,
    pub rx: mpsc::Receiver<Result<Reply, String>>,
}

/// What a folder probe saw, without decrypting anything.
pub enum Seen {
    /// The file is not in the folder.
    Missing,
    /// The stamp is the one this box last saw; the bytes were not read.
    Unchanged(u64),
    /// The stamp moved, or the provider cannot stamp. These are the bytes now.
    Changed(Option<u64>, Vec<u8>),
}

/// What a folder mirror answered.
pub enum FolderReply {
    /// What the folder holds now.
    Probed(Seen),
    /// A write landed, and the stamp the provider gave it afterwards.
    Wrote(Option<u64>),
    /// A merge was filed, its stamp, and what the merge did to get there.
    Merged(Option<u64>, Summary),
    /// The write was overtaken: another device moved the file. Nothing was written.
    Moved,
    /// The folder refused writes; the mirror is paused.
    ReadOnly,
}

/// What a merge kept and what it had to duplicate, so the view can say a conflict
/// was kept rather than silently swallow one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Records this side took from the folder, because they were newer there.
    pub adopted: usize,
    /// Records both sides changed at once, kept twice.
    pub conflicts: usize,
    /// Live records a tombstone removed.
    pub dropped: usize,
}

/// What a folder worker was asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderIntent {
    /// Report the file's stamp, its bytes when the stamp moved, or that it is gone.
    Probe,
    /// File what is on disk, guarded by the stamp this box last saw.
    Write,
}

/// A folder read, probe, write or prune on a worker thread. Like [`Job`], it
/// carries the file it was started on: a box not on screen any more still gets
/// its own reply, and never another box's version.
pub struct FolderJob {
    pub path: PathBuf,
    pub intent: FolderIntent,
    pub rx: mpsc::Receiver<Result<FolderReply, String>>,
}

/// What a repo request carried back. No box is attached to it — it is the folder
/// being read, not one of the files in it.
pub enum RepoReply {
    /// The repos the token can reach, and each owner's avatar keyed by login.
    Repos(Vec<Repo>, Avatars),
    /// The box files a source holds.
    Boxes(Source, Vec<String>),
    /// A box fetched from a source: where its working copy landed, and the blob
    /// sha it holds.
    Opened(Source, PathBuf, String),
}

pub struct RepoJob {
    /// The source the request is about, when it is about one. A failure still has
    /// to land somewhere: without this an unreadable repo would be asked for
    /// again on every frame.
    pub source: Option<Source>,
    /// Whether the request is the user's and a failure is worth a notice. An
    /// automatic re-read says nothing, so a repo that is unreachable does not
    /// nag once per window.
    pub announce: bool,
    pub rx: mpsc::Receiver<Result<RepoReply, String>>,
}

#[derive(Default)]
pub struct Sync {
    /// The repos the token can reach, once fetched.
    pub repos: Vec<Repo>,
    /// Each owner's avatar, by login, as it came back with the repos. Only what
    /// the picker draws: a missing one falls back to the owner's name.
    pub avatars: Avatars,
    /// The textures decoded from `avatars`. Owned here so a refresh replaces the
    /// set instead of leaving every old avatar in egui's frame cache.
    pub avatar_textures: HashMap<String, egui::TextureHandle>,
    /// Whether the repo picker's list has been fetched at all, so an empty answer
    /// is not asked for again every frame.
    pub repo_loaded: bool,
    /// Whether this run already looked at the copies on disk for one whose source
    /// was lost. Latched on the first signed frame, so the folder is read once a run
    /// and not once a frame.
    pub repaired: bool,
    /// The box files each source holds, keyed by `kind:target`. A key that is
    /// present has been asked for; an empty list is an empty folder.
    pub listing: HashMap<String, Vec<String>>,
    /// Why a source's last read failed, keyed the same way. A read nobody
    /// answered is not an empty folder, and the view has to say which it has.
    pub failed: HashMap<String, String>,
    /// When each source was last read from GitHub, keyed the same way. What
    /// paces the automatic re-read: a repo older than `github::STALE` is asked
    /// for again while its view is on screen. Written whenever a read goes out,
    /// so a repo nobody answered is retried once per window and not per frame.
    pub read_at: HashMap<String, Instant>,
    /// Whether the "Add a repo" section is showing the picker.
    pub adding: bool,
    /// The source whose "new box" form is open, if one is. One form at a time,
    /// however many repos there are.
    pub new_in: Option<Source>,
    /// The repo the GitHub view is showing, once the user picked one. The first
    /// source until then, and a source removed from under it falls back to the
    /// first rather than to an empty view.
    pub showing: Option<Source>,
    /// Filter over `repos` while picking one.
    pub repo_search: String,
    /// The token GitHub handed back. Zeroizing and never written down: it opens
    /// nothing on its own — the box still needs its password — and staying put
    /// across a lock is what keeps automatic sending alive.
    pub token: Zeroizing<String>,
    /// The companion GitHub sends for a token that runs out: what mints the next.
    pub renew: Option<Zeroizing<String>>,
    /// When the access token stops working, when GitHub said. A session minted
    /// without an expiry has none.
    pub expires_at: Option<Instant>,
    /// The renewal in flight, if one is. One at a time, so a frame loop cannot
    /// spend the same refresh token twice.
    pub renewing: Option<mpsc::Receiver<Result<github::Session, String>>>,
    /// Earliest next renewal after an attempt that never reached GitHub, so a
    /// connection that is down is not asked once per frame.
    pub renew_at: Option<Instant>,
    /// When the credential store is asked again after failing to hand back the
    /// remembered session at boot, in epoch seconds. A locked keyring is not a
    /// signed-out run, so the session is looked for again instead of written off.
    pub recall_at: Option<u64>,
    /// How many times this run has asked the store; a store that never answers is
    /// left alone instead of costing a request a minute for the whole run.
    pub recalls: u8,
    /// This run's spend against GitHub's hourly write budget.
    pub quota: Quota,
    pub job: Option<Job>,
    /// The folder mirror's one request in flight. It is separate from `job`
    /// because a repo and a folder are two different transports, and a box only
    /// ever uses one of them.
    pub folder_job: Option<FolderJob>,
    /// Why the folder mirror last failed, by box path. The next answer clears it.
    pub folder_failed: Option<(PathBuf, String)>,
    /// When the folder was last looked at, and which box, so an open box and a
    /// return to the foreground are probed once per window and not per frame.
    pub folder_at: Option<Instant>,
    pub folder_seen: Option<PathBuf>,
    /// A return to the foreground asks for a fresh probe.
    pub folder_resume: bool,
    pub repo_job: Option<RepoJob>,
    /// The last page the app asked the platform to open. Only a test reads it:
    /// no test may launch a browser, so what would have been opened is written
    /// down instead.
    #[cfg(test)]
    pub opened: Option<String>,
}

impl Sync {
    /// Whether a session is in hand: the token every request carries.
    pub fn signed(&self) -> bool {
        !self.token.is_empty()
    }

    /// Take back what a previous run kept: the token every request carries and,
    /// on the next line, what renews it. A bare refresh token is older still, and
    /// it is enough to mint a fresh pair.
    pub fn restore(&mut self, kept: String) {
        let mut lines = kept.split('\n');
        let access = lines.next().unwrap_or_default();
        if access.starts_with(github::REFRESH_PREFIX) {
            self.renew = Some(Zeroizing::new(access.to_owned()));
            return;
        }
        self.token = Zeroizing::new(access.to_owned());
        self.renew = lines
            .next()
            .filter(|token| !token.is_empty())
            .map(|token| Zeroizing::new(token.to_owned()));
    }

    /// What the credential store holds: the token and, under it, its renewal, so a
    /// restart picks the pair up instead of asking for a new one.
    pub fn credential(&self) -> String {
        match &self.renew {
            Some(renew) => format!("{}\n{}", self.token.as_str(), renew.as_str()),
            None => self.token.as_str().to_owned(),
        }
    }

    /// How long the access token has left, when GitHub said. `None` is a token
    /// with no deadline, which has nothing to renew.
    pub fn left(&self) -> Option<Duration> {
        self.expires_at
            .map(|at| at.saturating_duration_since(Instant::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A restored session carries its renewal too, and a bare refresh token left
    /// by an older build is enough to mint one: only a token is ever a bearer.
    #[test]
    fn a_restored_session_is_the_pair_the_store_holds() {
        let mut sync = Sync::default();
        assert!(!sync.signed(), "nothing is signed in");

        sync.restore("gho_plain".to_owned());
        assert!(sync.signed());
        assert_eq!(sync.token.as_str(), "gho_plain");
        assert!(
            sync.renew.is_none(),
            "a token that never runs out needs none"
        );
        assert_eq!(sync.credential(), "gho_plain", "and it is what is kept");

        let mut expiring = Sync::default();
        expiring.restore("gho_a\nghr_b".to_owned());
        assert!(expiring.signed());
        assert_eq!(
            expiring.renew.as_ref().map(|t| t.as_str()),
            Some("ghr_b"),
            "the renewal comes back with the token"
        );
        assert_eq!(expiring.credential(), "gho_a\nghr_b", "pair in, pair out");

        let mut older = Sync::default();
        older.restore("ghr_renewable".to_owned());
        assert!(!older.signed(), "a refresh token opens nothing as a bearer");
        assert!(
            older.renew.is_some(),
            "but it is still good for minting one"
        );
    }
}
