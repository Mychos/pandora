//! Everything the app knows about GitHub: the repos it can reach, the box files
//! each source holds, and the requests in flight.

use crate::features::settings::model::Source;
use crate::features::sync::github::{self, Avatars, Quota, Repo};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;
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
    /// The refresh token GitHub sent with it, when it sent one: what mints the
    /// next access token once this one runs out. Empty for a session that does
    /// not run out, and then there is nothing to renew with.
    pub refresh: Zeroizing<String>,
    /// When the access token runs out, or `None` when it does not claim to.
    pub expires_at: Option<Instant>,
    /// A renewal on its way back, so a second one is never asked for: a refresh
    /// token is good for one trade, and two in flight would race for it.
    pub renew_job: Option<mpsc::Receiver<Result<github::Session, String>>>,
    /// The next renewal waits until this, because the last one never reached
    /// GitHub. A refused renewal is the session being gone; one that never landed
    /// is not, so what is in hand stays and only the ask is held back.
    pub renew_after: Option<Instant>,
    /// This run's spend against GitHub's hourly write budget.
    pub quota: Quota,
    pub job: Option<Job>,
    pub repo_job: Option<RepoJob>,
    /// The last page the app asked the platform to open. Only a test reads it:
    /// no test may launch a browser, so what would have been opened is written
    /// down instead.
    #[cfg(test)]
    pub opened: Option<String>,
}

impl Sync {
    /// Whether a session is in hand at all: the access token requests carry, or
    /// the refresh token that will mint one.
    pub fn signed(&self) -> bool {
        !self.token.is_empty() || !self.refresh.is_empty()
    }

    /// Whether the access token must not go out again: past, or about to pass.
    /// A session with no refresh token is never due — nothing could renew it,
    /// and GitHub does not expire those. `renew_after` holds the ask itself back
    /// while a renewal that never landed waits for its retry.
    pub fn due(&self, now: Instant) -> bool {
        if self.refresh.is_empty() || self.renew_after.is_some_and(|at| at > now) {
            return false;
        }
        match self.expires_at {
            // A refresh token and no access token yet: what the credential
            // store gives back at boot, one renewal away from being a session.
            None => self.token.is_empty(),
            Some(at) => at <= now + github::RENEW_WINDOW,
        }
    }

    /// Take back what a previous run kept. A refresh token is the whole session
    /// — the access token that goes with it is minted on the next frame — and a
    /// plain access token is one that does not expire, used as it is.
    pub fn restore(&mut self, kept: String) {
        if kept.starts_with(github::REFRESH_PREFIX) {
            self.refresh = Zeroizing::new(kept);
        } else {
            self.token = Zeroizing::new(kept);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A session is due exactly when it can be renewed and must not be used
    /// again: a renewal is asked for at boot, never for a token that does not
    /// expire, and never twice at once.
    #[test]
    fn a_session_is_due_when_it_can_be_renewed_and_has_run_out() {
        let now = Instant::now();
        let mut sync = Sync::default();
        assert!(!sync.signed() && !sync.due(now), "nothing is signed in");

        sync.restore("gho_plain".to_owned());
        assert!(sync.signed());
        assert!(
            !sync.due(now),
            "a session with nothing to renew it with is not due, ever"
        );

        // What the store gives back once a sign-in asked for an expiring
        // session: the refresh half alone, minted into one on the first frame.
        let mut sync = Sync::default();
        sync.restore("ghr_kept".to_owned());
        assert!(sync.signed());
        assert!(sync.due(now), "a kept refresh token wants a token at once");

        sync.token = Zeroizing::new("gho_a".to_owned());
        sync.expires_at = Some(now + github::RENEW_WINDOW + Duration::from_secs(1));
        assert!(!sync.due(now), "time left on it");
        sync.expires_at = Some(now + github::RENEW_WINDOW);
        assert!(sync.due(now), "inside the window it is traded in");
        sync.expires_at = Some(now - Duration::from_secs(1));
        assert!(sync.due(now), "and once it is gone");
    }
}
