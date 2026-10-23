//! The folder half of the mirror: look at one encrypted file, take it when it is
//! newer, and write this box back in place when it is not. Every read, stamp and
//! write blocks, so each one runs on a worker thread and lands in `App::pump_sync`.
//!
//! The state is three-way without asking: the local file's digest says whether
//! this box moved, the provider's stamp says whether the folder did, and the
//! digest kept in `Remote` is the base both are compared against. When both moved
//! the two are merged; nothing is ever discarded to make the question simpler.

use crate::app::{App, Session, StatusKind};
use crate::features::settings::model::{Remote, FOLDER};
use crate::features::sync::folder::local::Local;
#[cfg(target_os = "android")]
use crate::features::sync::folder::saf::Safe;
use crate::features::sync::folder::{Folder, Guard, Write, NAME};
use crate::features::sync::github::PUSH_BACKOFF;
use crate::features::sync::model::{FolderIntent, FolderJob, FolderReply, Seen, Summary};
use crate::shared::i18n::{t, tf};
use crate::vault::crypto::digest;
use crate::vault::merge;
use crate::vault::model::now_secs;
use crate::vault::store;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How long the folder mirror waits between two looks, whether the box just came
/// on screen, the phone came back, or the quiet window asked for a write.
const PROBE_WAIT: Duration = Duration::from_millis(750);

/// A folder the user picked: the SAF tree on a phone, a directory on a desktop.
fn pick() -> Result<Option<String>, String> {
    #[cfg(target_os = "android")]
    return crate::shared::android::text("treeOpen");

    #[cfg(not(target_os = "android"))]
    return Ok(rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.display().to_string()));
}

/// The transport a remote names. A tree uri is only ever a phone's; a desktop
/// path is only ever a disk's, so the platform decides which one it is.
fn open(remote: &Remote) -> Result<Box<dyn Folder + Send>, String> {
    if remote.kind != FOLDER || remote.folder.is_empty() {
        return Err("this box is not mirrored to a folder".to_owned());
    }
    #[cfg(target_os = "android")]
    if remote.folder.starts_with("content://") {
        return Ok(Box::new(Safe::new(remote.folder.clone())));
    }
    Ok(Box::new(Local::new(remote.folder.clone())))
}

/// Where a folder copy waits while it is opened with this box's key: a file that
/// does not open never reaches the real path. The box's path keys it, so two
/// boxes being merged at once never overwrite each other's staging.
fn staging(box_path: &Path) -> PathBuf {
    store::data_dir().join("staging").join(format!(
        "{}.pandora",
        digest(box_path.display().to_string().as_bytes())
    ))
}

fn stage(box_path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let path = staging(box_path);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create dir: {e}"))?;
    }
    store::write_private(&path, bytes)?;
    Ok(path)
}

/// The folder, without taking it. A stamp this box has seen is enough: the bytes
/// are only read when the stamp moved, or when the provider cannot stamp at all.
fn probe(folder: &dyn Folder, seen: u64, first: bool) -> Result<FolderReply, String> {
    if first {
        if !folder.writable()? {
            return Ok(FolderReply::ReadOnly);
        }
        clean(folder);
    }
    let stamp = folder.stamp(NAME).unwrap_or(None);
    if let Some(at) = stamp {
        if seen != 0 && at == seen {
            return Ok(FolderReply::Probed(Seen::Unchanged(at)));
        }
    }
    match folder.read(NAME)? {
        Some(bytes) => Ok(FolderReply::Probed(Seen::Changed(stamp, bytes))),
        None => {
            // A file the platform lists but cannot read is not a missing file:
            // writing over it would be the wrong guess.
            let listed = folder.list()?;
            if listed.iter().any(|name| name == NAME) {
                Err("the folder holds a file the app cannot read".to_owned())
            } else {
                Ok(FolderReply::Probed(Seen::Missing))
            }
        }
    }
}

/// The old versioned protocol left files behind. They are deleted once, when a
/// folder from that build is first attached; a refusal is not this sync's problem.
fn clean(folder: &dyn Folder) {
    let Ok(names) = folder.list() else {
        return;
    };
    for name in names {
        if crate::features::sync::folder::legacy(&name) {
            let _ = folder.remove(&name);
        }
    }
}

/// Write the box in place, guarded by the stamp the probe saw. A provider that
/// refuses writes is not an error: it is the mirror pausing itself.
fn file(folder: &dyn Folder, bytes: &[u8], guard: Guard) -> Result<FolderReply, String> {
    match folder.write(NAME, bytes, guard) {
        Ok(Write::Filed) => Ok(FolderReply::Wrote(folder.stamp(NAME).unwrap_or(None))),
        Ok(Write::Moved) => Ok(FolderReply::Moved),
        Err(e) => {
            if !folder.writable().unwrap_or(true) {
                Ok(FolderReply::ReadOnly)
            } else {
                Err(e)
            }
        }
    }
}

/// What folding the folder's copy into this box did.
enum Fold {
    /// The two already said the same thing.
    Same,
    /// The merge took everything the folder had, so the folder already holds it.
    Adopted(Summary),
    /// This box holds something the folder does not: the merged box has to be
    /// written back in place.
    Owed(Summary),
}

impl App {
    /// Remember the look that just went out, so neither the open box nor a
    /// foreground return asks for another one before the window is up.
    fn folder_started(&mut self, path: &Path) {
        self.sync.folder_at = Some(Instant::now());
        self.sync.folder_seen = Some(path.to_owned());
    }

    /// The folder's own clock: an open box is looked at once when it comes on
    /// screen, a phone coming back is looked at again, and a settled edit is sent
    /// once the quiet window is up.
    pub(crate) fn folder_tick(&mut self, now: Instant) -> Option<Duration> {
        if self.sync.folder_job.is_some() {
            return None;
        }
        let Some(s) = self.boxes.session.as_ref() else {
            self.sync.folder_seen = None;
            return None;
        };
        let folder = s.remote.as_ref().filter(|r| r.kind == FOLDER);
        let Some(remote) = folder else {
            self.sync.folder_seen = None;
            self.sync.folder_resume = false;
            return None;
        };
        if remote.read_only || remote.blocked {
            return None;
        }
        let path = s.path.clone();
        let opened = self.sync.folder_seen.as_deref() != Some(path.as_path());
        let resume = self.sync.folder_resume;
        let due = !s.conflict && s.push_at.is_some_and(|at| at <= now);
        if !opened && !resume && !due {
            return None;
        }
        if let Some(at) = self.sync.folder_at {
            let next = at + PROBE_WAIT;
            if next > now {
                return Some(next - now);
            }
        }
        self.sync.folder_resume = false;
        self.folder_started(&path);
        if due {
            self.folder_push();
        } else {
            self.folder_probe();
        }
        None
    }

    /// Pick the folder this box is mirrored to. A repo mirror is left alone: its
    /// sha names a file that a folder switch would strand.
    pub fn folder_pick(&mut self) {
        if self.sync.folder_job.is_some() {
            return;
        }
        let Some(s) = self.boxes.session.as_ref() else {
            return;
        };
        if s.remote
            .as_ref()
            .is_some_and(|remote| remote.kind != FOLDER)
        {
            self.shell
                .fail("this box is mirrored to a repo — take it out of the repo first");
            return;
        }
        let folder = match pick() {
            Ok(Some(folder)) => folder,
            Ok(None) => return,
            Err(e) => return self.shell.fail(e),
        };
        let path = s.path.clone();
        let Some(s) = self.boxes.find(&path) else {
            return;
        };
        s.remote = Some(Remote {
            kind: FOLDER.to_owned(),
            repo: String::new(),
            file: String::new(),
            sha: String::new(),
            folder,
            digest: String::new(),
            stamp: 0,
            read_only: false,
            blocked: false,
        });
        s.push_at = None;
        let remote = s.remote.clone();
        self.remember(&path, remote);
        self.sync.folder_failed = None;
        self.sync.folder_resume = false;
        self.folder_probe();
    }

    /// Ask the folder what it holds. The landing decides whether a newer file is
    /// adopted, merged or left alone.
    pub fn folder_probe(&mut self) {
        if self.sync.folder_job.is_some() {
            return;
        }
        let Some(s) = self.boxes.session.as_ref() else {
            return;
        };
        let Some(remote) = s.remote.as_ref().filter(|r| r.kind == FOLDER) else {
            return;
        };
        if remote.read_only || remote.blocked {
            return;
        }
        let folder = match open(remote) {
            Ok(folder) => folder,
            Err(e) => return self.shell.fail(e),
        };
        let path = s.path.clone();
        let seen = remote.stamp;
        let first = remote.digest.is_empty() && remote.stamp == 0;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(probe(folder.as_ref(), seen, first));
        });
        self.folder_started(&path);
        self.sync.folder_job = Some(FolderJob {
            path,
            intent: FolderIntent::Probe,
            rx,
        });
    }

    /// Look at the folder now, announcing it because the user asked.
    pub fn folder_pull(&mut self) {
        if self.sync.folder_job.is_some() {
            return;
        }
        self.folder_probe();
        if self.sync.folder_job.is_some() {
            self.shell
                .set_status("reading the folder…", StatusKind::Info);
        }
    }

    /// File this box in the folder. The stamp guard catches a file another device
    /// moved: asking the provider first would only add a round trip.
    pub fn folder_push(&mut self) {
        if self.sync.folder_job.is_some() {
            return;
        }
        let Some(s) = self.boxes.session.as_ref() else {
            return;
        };
        if s.parked {
            return self.shell.fail("open the box before sending it");
        }
        let Some(remote) = s.remote.as_ref().filter(|r| r.kind == FOLDER) else {
            return self.shell.fail("this box is not mirrored to a folder");
        };
        if remote.read_only {
            return self
                .shell
                .fail(t("the folder is read-only — the mirror is paused"));
        }
        if remote.blocked {
            return self
                .shell
                .fail(t("the folder holds another box — choose another folder"));
        }
        let path = s.path.clone();
        let guard = if remote.stamp != 0 {
            Guard::Stamp(remote.stamp)
        } else if remote.digest.is_empty() {
            Guard::Missing
        } else {
            Guard::Unguarded
        };
        self.folder_write(&path, None, guard);
    }

    /// Stop mirroring this box to its folder. The folder's file is the user's and
    /// stays where it is.
    pub fn folder_forget(&mut self) {
        let Some(s) = self.boxes.session.as_ref() else {
            return;
        };
        if !s.remote.as_ref().is_some_and(|r| r.kind == FOLDER) {
            return;
        }
        let path = s.path.clone();
        if self
            .sync
            .folder_job
            .as_ref()
            .is_some_and(|job| job.path == path)
        {
            self.sync.folder_job = None;
        }
        if let Some(s) = self.boxes.find(&path) {
            s.remote = None;
            s.push_at = None;
        }
        self.remember(&path, None);
        self.sync.folder_failed = None;
        self.sync.folder_seen = None;
        self.sync.folder_resume = false;
        self.shell
            .set_status("the folder mirror was removed", StatusKind::Info);
    }

    /// Write what is on disk, with the merge summary riding back when the write
    /// is the tail of one.
    fn folder_write(&mut self, path: &Path, summary: Option<Summary>, guard: Guard) {
        if self.sync.folder_job.is_some() {
            return;
        }
        let Some(s) = self.boxes.find(path) else {
            return;
        };
        if s.parked {
            return;
        }
        if s.dirty {
            if s.conflict {
                return;
            }
            if let Err(e) = store::save(&s.path, &mut s.vault, &s.creds) {
                return self.shell.fail(e);
            }
            s.settle();
        }
        let Some(remote) = s.remote.as_ref().filter(|r| r.kind == FOLDER) else {
            return;
        };
        if remote.read_only || remote.blocked {
            return;
        }
        let folder = match open(remote) {
            Ok(folder) => folder,
            Err(e) => return self.shell.fail(e),
        };
        let Ok(bytes) = fs::read(&s.path) else {
            return self.shell.fail("cannot read the box file");
        };
        let path = path.to_owned();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let reply = match file(folder.as_ref(), &bytes, guard) {
                Ok(FolderReply::Wrote(stamp)) => Ok(summary
                    .map_or(FolderReply::Wrote(stamp), |summary| {
                        FolderReply::Merged(stamp, summary)
                    })),
                other => other,
            };
            let _ = tx.send(reply);
        });
        self.folder_started(&path);
        self.sync.folder_job = Some(FolderJob {
            path,
            intent: FolderIntent::Write,
            rx,
        });
        self.shell
            .set_status("filing the box in the folder…", StatusKind::Info);
    }

    /// Where a folder worker's answer lands.
    pub(crate) fn land_folder(
        &mut self,
        path: &Path,
        _intent: FolderIntent,
        reply: Result<FolderReply, String>,
    ) {
        match reply {
            Ok(FolderReply::Probed(seen)) => {
                self.sync.folder_failed = None;
                self.land_seen(path, seen);
            }
            Ok(FolderReply::Wrote(stamp)) => {
                self.sync.folder_failed = None;
                self.land_wrote(path, stamp, None);
            }
            Ok(FolderReply::Merged(stamp, summary)) => {
                self.sync.folder_failed = None;
                self.land_wrote(path, stamp, Some(summary));
            }
            Ok(FolderReply::Moved) => {
                // The write was overtaken. Nothing was written, so the next look
                // has to read the folder before it writes again.
                if let Some(s) = self.boxes.find(path) {
                    s.push_at = None;
                }
                self.sync.folder_resume = true;
                self.sync.folder_failed = None;
                self.shell.set_status(
                    "the folder changed while writing — looking again",
                    StatusKind::Info,
                );
            }
            Ok(FolderReply::ReadOnly) => self.land_read_only(path),
            Err(e) => {
                // Nothing is lost: the box keeps its file, and the next pass
                // probes before it writes, so a failure never becomes a blind
                // overwrite. The wait is the retry's.
                if let Some(s) = self.boxes.find(path) {
                    s.push_at = None;
                }
                self.sync.folder_resume = true;
                self.sync.folder_at = Some(Instant::now() + PUSH_BACKOFF);
                self.sync.folder_failed = Some((path.to_owned(), e.clone()));
                self.shell.fail(e);
            }
        }
    }

    /// The provider refused writes: keep the folder attached, stop retrying, and
    /// say why once. Picking a writable folder is the way out.
    fn land_read_only(&mut self, path: &Path) {
        let mut remote = None;
        if let Some(s) = self.boxes.find(path) {
            s.push_at = None;
            if let Some(r) = s.remote.as_mut().filter(|r| r.kind == FOLDER) {
                r.read_only = true;
                remote = Some(r.clone());
            }
        }
        if let Some(remote) = remote {
            self.remember(path, Some(remote));
        }
        let why = t("the folder is read-only — the mirror is paused").to_owned();
        self.sync.folder_failed = None;
        self.shell.set_status(why, StatusKind::Error);
    }

    /// What the folder holds against what this box holds.
    fn land_seen(&mut self, path: &Path, seen: Seen) {
        let Some(s) = self.boxes.find(path) else {
            return;
        };
        if s.parked {
            return;
        }
        let Some(remote) = s.remote.as_ref().filter(|r| r.kind == FOLDER).cloned() else {
            return;
        };
        let Ok(bytes) = fs::read(&s.path) else {
            return self.shell.fail("cannot read the box file");
        };
        let local = digest(&bytes);
        let local_changed =
            s.dirty || s.editing || remote.digest.is_empty() || local != remote.digest;

        match seen {
            // The file disappeared. Putting this box back is the only way the
            // mirror keeps working; a provider that lost it has nothing to merge.
            Seen::Missing => self.folder_write(path, None, Guard::Missing),
            Seen::Unchanged(stamp) => {
                if local_changed && local != remote.digest {
                    self.folder_write(path, None, Guard::Stamp(stamp));
                }
            }
            Seen::Changed(stamp, their_bytes) => {
                let theirs = digest(&their_bytes);
                if !remote.digest.is_empty() && theirs == remote.digest {
                    // The bytes are the ones this box last synced: the provider
                    // only restamped them. A local edit still goes out.
                    self.set_stamp(path, stamp);
                    if local_changed && local != remote.digest {
                        let guard = stamp.map_or(Guard::Unguarded, Guard::Stamp);
                        self.folder_write(path, None, guard);
                    }
                } else if !local_changed {
                    self.take(
                        path,
                        &their_bytes,
                        stamp,
                        theirs,
                        t("box replaced from the folder").to_owned(),
                        StatusKind::Saved,
                    );
                } else {
                    self.merge(path, &their_bytes, stamp);
                }
            }
        }
    }

    /// Take the folder's copy whole: it opened with this box's key, and this box
    /// has nothing the folder does not.
    fn take(
        &mut self,
        path: &Path,
        bytes: &[u8],
        stamp: Option<u64>,
        digest: String,
        note: String,
        kind: StatusKind,
    ) {
        let mut remote = None;
        if let Some(s) = self.boxes.find(path) {
            match store::adopt(&s.path, bytes, &s.creds, None) {
                Ok(vault) => {
                    s.vault = vault;
                    s.settle();
                    s.push_at = None;
                    s.behind = false;
                    if let Some(r) = s.remote.as_mut().filter(|r| r.kind == FOLDER) {
                        r.digest = digest;
                        r.stamp = stamp.unwrap_or(0);
                        remote = Some(r.clone());
                    }
                }
                Err(e) => {
                    self.land_blocked(path, refusal(e));
                    return;
                }
            }
        }
        if let Some(remote) = remote {
            self.remember(path, Some(remote));
        }
        self.shell.set_status(note, kind);
    }

    /// Both sides moved: fold them, and write the result back when the folder
    /// does not already hold it. Nothing is asked, because nothing is lost.
    fn merge(&mut self, path: &Path, bytes: &[u8], stamp: Option<u64>) {
        let staged = match stage(path, bytes) {
            Ok(path) => path,
            Err(e) => return self.shell.fail(e),
        };
        let Some(s) = self.boxes.find(path) else {
            let _ = fs::remove_file(&staged);
            return;
        };
        let folded = fold(s, &staged);
        let _ = fs::remove_file(&staged);
        match folded {
            Ok(Fold::Same) => self.take(
                path,
                bytes,
                stamp,
                digest(bytes),
                t("the folder copy is this box already").to_owned(),
                StatusKind::Info,
            ),
            Ok(Fold::Adopted(summary)) => {
                let said = said(summary);
                self.take(
                    path,
                    bytes,
                    stamp,
                    digest(bytes),
                    if said.is_empty() {
                        t("merged with the folder copy").to_owned()
                    } else {
                        said
                    },
                    StatusKind::Saved,
                );
            }
            Ok(Fold::Owed(summary)) => {
                let guard = stamp.map_or(Guard::Unguarded, Guard::Stamp);
                self.folder_write(path, Some(summary), guard);
            }
            Err(e) => self.land_blocked(path, e),
        }
    }

    /// The folder's copy is not this box: writing over it would destroy another
    /// box, so the mirror stays attached but never writes until it is picked
    /// again.
    fn land_blocked(&mut self, path: &Path, why: String) {
        let mut remote = None;
        if let Some(s) = self.boxes.find(path) {
            s.push_at = None;
            if let Some(r) = s.remote.as_mut().filter(|r| r.kind == FOLDER) {
                r.blocked = true;
                remote = Some(r.clone());
            }
        }
        if let Some(remote) = remote {
            self.remember(path, Some(remote));
        }
        self.sync.folder_resume = false;
        self.sync.folder_failed = Some((path.to_owned(), why.clone()));
        self.shell.set_status(why, StatusKind::Error);
    }

    /// Point this box at the stamp the folder just gave it.
    fn set_stamp(&mut self, path: &Path, stamp: Option<u64>) {
        let mut remote = None;
        if let Some(s) = self.boxes.find(path) {
            if let Some(r) = s.remote.as_mut().filter(|r| r.kind == FOLDER) {
                r.stamp = stamp.unwrap_or(0);
                remote = Some(r.clone());
            }
        }
        if let Some(remote) = remote {
            self.remember(path, Some(remote));
        }
    }

    /// The write landed: the folder now holds exactly what is on disk, and
    /// nothing is waiting to go out.
    fn land_wrote(&mut self, path: &Path, stamp: Option<u64>, summary: Option<Summary>) {
        let mut remote = None;
        if let Some(s) = self.boxes.find(path) {
            s.push_at = None;
            s.behind = false;
            if !s.parked {
                s.settle();
            }
            let digest = fs::read(&s.path).ok().map(|bytes| digest(&bytes));
            if let Some(r) = s.remote.as_mut().filter(|r| r.kind == FOLDER) {
                if let Some(digest) = digest {
                    r.digest = digest;
                }
                r.stamp = stamp.unwrap_or(0);
                remote = Some(r.clone());
            }
        }
        if let Some(remote) = remote {
            self.remember(path, Some(remote));
        }
        let text = summary.map_or_else(
            || "box filed in the folder".to_owned(),
            |summary| {
                let said = said(summary);
                if said.is_empty() {
                    "merged with the folder copy".to_owned()
                } else {
                    said
                }
            },
        );
        self.shell.set_status(text, StatusKind::Saved);
    }
}

/// A copy that does not open with this box's key never replaces anything.
fn refusal(e: String) -> String {
    tf(
        "the folder copy does not open with this box's key: {error}",
        &[("error", &e)],
    )
}

/// What a merge did, in the words the status line uses.
fn said(summary: Summary) -> String {
    merge::summary(summary.adopted, summary.conflicts)
}

/// Fold the folder's copy into the box on screen. Both sides keep what they
/// changed, a record both changed at once stays twice, and the write keeps the
/// file it displaced in the stash.
fn fold(s: &mut Session, staged: &Path) -> Result<Fold, String> {
    let theirs = store::load(staged, &s.creds).map_err(refusal)?;
    if merge::equal(&s.vault, &theirs) {
        return Ok(Fold::Same);
    }
    let folded = merge::merge(&s.vault, &theirs, now_secs());
    let summary = Summary {
        adopted: folded.adopted,
        conflicts: folded.conflicts,
        dropped: folded.dropped,
    };
    s.vault = folded.vault;
    s.sanitize();
    if merge::equal(&s.vault, &theirs) {
        return Ok(Fold::Adopted(summary));
    }
    store::save(&s.path, &mut s.vault, &s.creds)?;
    s.settle();
    Ok(Fold::Owed(summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::settings::model::Remote;
    use crate::features::sync::folder::local::Local;
    use crate::vault::crypto::digest;
    use crate::vault::model::Login;
    use std::time::Duration;

    fn login(id: &str, name: &str) -> Login {
        let mut it = Login::new(1);
        it.id = id.to_owned();
        it.name = name.to_owned();
        it.password = zeroize::Zeroizing::new("secret".to_owned());
        it
    }

    fn mirror(app: &mut App, dir: &Path, digest: String, stamp: u64) {
        app.boxes.session.as_mut().expect("a box").remote = Some(Remote {
            kind: FOLDER.to_owned(),
            repo: String::new(),
            file: String::new(),
            sha: String::new(),
            folder: dir.display().to_string(),
            digest,
            stamp,
            read_only: false,
            blocked: false,
        });
    }

    fn their_bytes(
        app: &App,
        dir: &Path,
        edit: impl FnOnce(&mut crate::vault::model::Vault),
    ) -> Vec<u8> {
        let s = app.boxes.session.as_ref().expect("a box");
        let mut theirs = s.vault.clone();
        edit(&mut theirs);
        let path = dir.join(NAME);
        store::save(&path, &mut theirs, &s.creds).expect("saved");
        fs::read(path).expect("read")
    }

    fn wait(app: &mut App) {
        if let Some(job) = app.sync.folder_job.take() {
            let path = job.path;
            let intent = job.intent;
            let reply = job
                .rx
                .recv_timeout(Duration::from_secs(2))
                .expect("a folder reply");
            app.land_folder(&path, intent, reply);
        }
    }

    /// A local save that never reached the folder is found by the digest alone,
    /// even after a restart cleared every in-memory flag.
    #[test]
    fn a_local_save_after_restart_is_filed() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("folder-local-ahead");
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = app.boxes.session.as_ref().expect("a box").path.clone();
        let base = fs::read(&path).expect("read");
        fs::write(dir.path().join(NAME), &base).expect("base");
        let stamp = Local::new(dir.path())
            .stamp(NAME)
            .expect("stamped")
            .expect("a file");
        mirror(&mut app, dir.path(), digest(&base), stamp);

        let s = app.boxes.session.as_mut().expect("a box");
        s.vault.logins.push(login("mine", "Mine"));
        store::save(&s.path, &mut s.vault, &s.creds).expect("saved");

        app.land_seen(&path, Seen::Unchanged(stamp));
        wait(&mut app);
        let remote = fs::read(dir.path().join(NAME)).expect("read");
        let local = fs::read(&path).expect("read");
        assert_eq!(remote, local, "the local save was filed");
        assert_eq!(
            app.boxes
                .session
                .as_ref()
                .expect("a box")
                .remote
                .as_ref()
                .expect("remote")
                .digest,
            digest(&local)
        );
    }

    /// A provider that only restamps the same bytes did not change the box.
    #[test]
    fn a_restamp_changes_nothing_but_the_stamp() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("folder-restamp");
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = app.boxes.session.as_ref().expect("a box").path.clone();
        let base = fs::read(&path).expect("read");
        let stamp = 1_700_000_000_000;
        mirror(&mut app, dir.path(), digest(&base), stamp);

        app.land_seen(&path, Seen::Changed(Some(stamp + 5_000), base.clone()));
        assert!(app.sync.folder_job.is_none(), "nothing was written");
        let s = app.boxes.session.as_ref().expect("a box");
        assert_eq!(s.remote.as_ref().expect("remote").stamp, stamp + 5_000);
        assert_eq!(s.vault.logins.len(), 0);
    }

    /// A clean box with nothing of its own takes the folder's copy whole.
    #[test]
    fn a_clean_box_takes_the_folder_copy() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("folder-adopt");
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = app.boxes.session.as_ref().expect("a box").path.clone();
        let base = fs::read(&path).expect("read");
        let theirs = their_bytes(&app, dir.path(), |vault| {
            vault.logins.push(login("theirs", "Theirs"));
        });
        let stamp = Local::new(dir.path())
            .stamp(NAME)
            .expect("stamped")
            .expect("a file");
        mirror(&mut app, dir.path(), digest(&base), stamp);
        let remote_digest = digest(&theirs);

        app.land_seen(&path, Seen::Changed(Some(stamp), theirs.clone()));
        let s = app.boxes.session.as_ref().expect("a box");
        assert_eq!(s.vault.logins.len(), 1);
        assert_eq!(s.vault.logins[0].id, "theirs");
        assert_eq!(fs::read(&path).expect("read"), theirs);
        assert_eq!(s.remote.as_ref().expect("remote").digest, remote_digest);
    }

    /// Both sides changed: the merge keeps both and files the result back.
    #[test]
    fn a_dirty_box_merges_and_files_the_result() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("folder-merge");
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = app.boxes.session.as_ref().expect("a box").path.clone();
        let base = fs::read(&path).expect("read");
        let theirs = their_bytes(&app, dir.path(), |vault| {
            vault.logins.push(login("theirs", "Theirs"));
        });
        let stamp = Local::new(dir.path())
            .stamp(NAME)
            .expect("stamped")
            .expect("a file");
        mirror(&mut app, dir.path(), digest(&base), stamp);

        let s = app.boxes.session.as_mut().expect("a box");
        s.vault.logins.push(login("mine", "Mine"));
        s.mark_dirty();

        app.land_seen(&path, Seen::Changed(Some(stamp), theirs));
        wait(&mut app);
        let s = app.boxes.session.as_ref().expect("a box");
        assert_eq!(s.vault.logins.len(), 2, "both edits survive");
        let remote = store::load(&dir.path().join(NAME), &s.creds).expect("remote opens");
        assert_eq!(remote.logins.len(), 2);
        assert!(s.push_at.is_none(), "the folder holds the merge");
    }

    /// An open editor counts as unsaved changes even before the first keystroke
    /// reaches disk, so the folder copy must be merged, never adopted under it.
    #[test]
    fn an_open_editor_is_merged_not_replaced() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("folder-editor");
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = app.boxes.session.as_ref().expect("a box").path.clone();
        let base = fs::read(&path).expect("read");
        let theirs = their_bytes(&app, dir.path(), |vault| {
            vault.logins.push(login("theirs", "Theirs"));
        });
        let stamp = Local::new(dir.path())
            .stamp(NAME)
            .expect("stamped")
            .expect("a file");
        mirror(&mut app, dir.path(), digest(&base), stamp);

        let s = app.boxes.session.as_mut().expect("a box");
        s.vault.logins.push(login("mine", "Mine"));
        s.editing = true;

        app.land_seen(&path, Seen::Changed(Some(stamp), theirs));
        wait(&mut app);
        let s = app.boxes.session.as_ref().expect("a box");
        assert_eq!(
            s.vault.logins.len(),
            2,
            "the editor's record was not dropped"
        );
    }

    /// A provider that refuses writes pauses the mirror and keeps the folder's
    /// address, so the user can fix it without picking again.
    #[test]
    fn a_read_only_folder_pauses_the_mirror() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("folder-read-only");
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = app.boxes.session.as_ref().expect("a box").path.clone();
        mirror(&mut app, dir.path(), String::new(), 0);

        app.land_folder(&path, FolderIntent::Probe, Ok(FolderReply::ReadOnly));
        assert!(
            app.boxes
                .session
                .as_ref()
                .expect("a box")
                .remote
                .as_ref()
                .expect("remote")
                .read_only
        );
        app.folder_tick(Instant::now() + Duration::from_secs(5));
        assert!(
            app.sync.folder_job.is_none(),
            "a paused mirror is not probed"
        );
    }

    /// A folder holding a copy this box's key does not open is never written
    /// over: that would destroy another box.
    #[test]
    fn a_foreign_folder_copy_pauses_the_mirror() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("folder-foreign");
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = app.boxes.session.as_ref().expect("a box").path.clone();
        let base = fs::read(&path).expect("read");
        let stamp = 1_700_000_000_000;
        mirror(&mut app, dir.path(), digest(&base), stamp);

        app.land_seen(&path, Seen::Changed(Some(stamp), b"not a box".to_vec()));
        let s = app.boxes.session.as_ref().expect("a box");
        assert!(s.remote.as_ref().expect("remote").blocked);
        assert!(s.push_at.is_none());
        assert!(app.sync.folder_failed.is_some());
        app.folder_tick(Instant::now() + Duration::from_secs(5));
        assert!(
            app.sync.folder_job.is_none(),
            "a blocked mirror is not probed"
        );
    }

    /// The old versioned protocol left files behind; the first probe deletes
    /// them and reads only the one file this build knows.
    #[test]
    fn the_first_probe_drops_legacy_versions() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let local = Local::new(dir.path());
        local
            .write(NAME, b"the box", Guard::Missing)
            .expect("written");
        local
            .write("000004-deadbeef.pandora", b"old", Guard::Missing)
            .expect("written");

        match probe(&local, 0, true).expect("probed") {
            FolderReply::Probed(Seen::Changed(_, bytes)) => assert_eq!(bytes, b"the box"),
            _ => panic!("the one file is read"),
        }
        assert!(!dir.path().join("000004-deadbeef.pandora").exists());
        assert!(dir.path().join(NAME).exists());
    }

    /// An open box is looked at once, and a settled edit is written when its
    /// quiet window ends.
    #[test]
    fn a_folder_box_is_looked_at_once_and_sent_when_its_quiet_window_ends() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("Folder tick");
        let dir = tempfile::tempdir().expect("a temp dir");
        mirror(&mut app, dir.path(), String::new(), 0);

        let now = Instant::now();
        app.folder_tick(now);
        assert!(app.sync.folder_job.is_some(), "an open box is probed");
        app.sync.folder_job = None;

        app.folder_tick(now + Duration::from_millis(100));
        assert!(
            app.sync.folder_job.is_none(),
            "the same box is not looked at twice in one window"
        );
        app.folder_tick(now + Duration::from_millis(800));
        assert!(
            app.sync.folder_job.is_none(),
            "a box that did not move is not looked at again"
        );

        app.boxes.session.as_mut().expect("a box").push_at =
            Some(now + crate::features::sync::github::PUSH_QUIET);
        app.folder_tick(now + crate::features::sync::github::PUSH_QUIET);
        assert!(
            app.sync.folder_job.is_some(),
            "what settled goes out when its window ends"
        );
    }
}
