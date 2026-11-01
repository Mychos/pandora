//! The GitHub half of the mirror: sign in, then move a box's file between disk and
//! repo; every request blocks on a worker thread and lands in `App::pump_sync`.

use crate::app::{App, Job, Login, Reply, RepoJob, RepoReply, Screen, SignIn, StatusKind};
use crate::features::settings::model::{self as prefs, Remote, Source, GITHUB};
use crate::features::sync::github::{self as sync, Fingerprint, Repo};
use crate::features::sync::remember;
use crate::shared::i18n::tf;
use crate::shared::ui::size;
use crate::vault::model::Vault;
use crate::vault::store;
use crate::vault::Failure;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

/// Wait after a renewal that never reached GitHub: a connection that is down says
/// nothing about the session, and without a wait a frame loop becomes a request storm.
const RENEW_RETRY: Duration = Duration::from_secs(60);

/// `owner/name` — the only shape the Contents API takes.
fn repo_ok(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !owner.is_empty() && !name.is_empty()
}

pub(crate) fn source_key(source: &Source) -> String {
    format!("{}:{}", source.kind, source.target)
}

/// The folder name of every working copy on disk.
fn folders() -> Vec<String> {
    let Ok(entries) = fs::read_dir(store::sources_dir()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect()
}

/// Whether a folder could be a digest at all. An older layout's `sources/<kind>/…`
/// is not one, and no repo list can name what its folder never hashed.
fn hashed(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether a copy is waiting for the repo that names it: its folder is a digest,
/// so only the account's own repos can say which repo it was.
fn stray(folders: &[String], prefs: &prefs::Prefs) -> bool {
    let known: Vec<String> = prefs
        .sources
        .iter()
        .chain(prefs.stopped.iter())
        .map(|s| store::source_digest(&s.kind, &s.target))
        .collect();
    folders
        .iter()
        .any(|name| hashed(name) && !known.contains(name))
}

/// The sources a folder outlived: a copy stays on disk when its source is lost, so
/// an answered repo whose digest is already a folder is offered back. A repo the
/// user stopped following keeps its copy and is not offered again.
fn orphans(prefs: &prefs::Prefs, repos: &[Repo]) -> Vec<Source> {
    let folders = folders();
    repos
        .iter()
        .filter(|repo| repo.private)
        .map(|repo| Source {
            kind: GITHUB.to_owned(),
            target: repo.full_name.clone(),
        })
        .filter(|source| {
            repo_ok(&source.target)
                && !prefs.sources.contains(source)
                && !prefs.stopped.contains(source)
                && folders.contains(&store::source_digest(&source.kind, &source.target))
        })
        .collect()
}

/// Zero when due now, `STALE` for one just read; never read is due immediately.
fn due_in(read_at: Option<Instant>, now: Instant) -> Duration {
    read_at.map_or(Duration::ZERO, |at| {
        sync::STALE.saturating_sub(now.saturating_duration_since(at))
    })
}

/// Download in one step: a half-written box would open as a corrupt vault. The
/// copy it replaces is stashed, so a fetch never loses an unsent edit.
pub(crate) fn place(path: &Path, bytes: &[u8]) -> Result<(), String> {
    store::stash(path);
    store::write_atomic(path, bytes)
        .map_err(|e| tf("cannot place the box: {error}", &[("error", &e)]))
}

/// Where a source's box lands on disk. Only the last component reaches the path, so
/// a crafted name cannot walk out of the source's folder.
fn box_path(source: &Source, file: &str) -> Option<(String, PathBuf)> {
    let name = Path::new(file).file_name()?.to_str()?.to_owned();
    let path = store::source_dir(&source.kind, &source.target).join(&name);
    Some((name, path))
}

/// A box name turned into a file name: whatever a path cannot carry becomes `-`.
/// A name that reduces to nothing is refused.
fn box_file(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_owned();
    (!cleaned.is_empty()).then(|| format!("{cleaned}.{}", store::EXT))
}

/// The working copies a source already has on disk, for listing a repo while offline.
pub(crate) fn cached(source: &Source) -> Vec<String> {
    list_dir(&store::source_dir(&source.kind, &source.target))
}

/// Where a source's copy of `file` sits on disk, when the name is usable.
pub(crate) fn cached_path(source: &Source, file: &str) -> Option<PathBuf> {
    box_path(source, file).map(|(_, path)| path)
}

/// The box files a folder holds, by name; missing is empty.
fn list_dir(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let suffix = format!(".{}", store::EXT);
    let mut files: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.ends_with(&suffix))
        .collect();
    files.sort();
    files
}

/// Show a page in the platform's opener and say why when nothing could take it;
/// the address stays on screen, so a session with no opener can still sign in by hand.
/// Compiled out of a test build, where `App::open_page` only records the address.
#[cfg(not(test))]
pub(crate) fn browse(url: &str) -> Result<(), String> {
    // A phone has no opener command: the activity hands the page to whatever takes it.
    #[cfg(target_os = "android")]
    let opened = crate::shared::android::send("open", url);
    #[cfg(not(target_os = "android"))]
    let opened = desktop(url);
    opened
}

/// The desktop side of `browse`: each system's "open this address" command, run once.
#[cfg(not(any(test, target_os = "android")))]
fn desktop(url: &str) -> Result<(), String> {
    // Linux builds its own commands in `linux`.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let opened = launch(Command::new("open").arg(url));
    // Not `cmd /C start`: cmd re-parses its command line and this URL comes off the wire.
    #[cfg(target_os = "windows")]
    let opened = launch(Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let opened = linux(url);
    opened
}

/// No single opener command is guaranteed: `xdg-open`, then `gio open`, then `$BROWSER`
/// (which may carry arguments, `%s` for the address). The first that starts wins, and
/// nothing waits for a browser the window cannot wait for.
#[cfg(not(any(
    test,
    target_os = "android",
    target_os = "macos",
    target_os = "windows"
)))]
fn linux(url: &str) -> Result<(), String> {
    use std::process::Command;
    let mut attempts = vec![
        {
            let mut open = Command::new("xdg-open");
            open.arg(url);
            open
        },
        {
            let mut open = Command::new("gio");
            open.arg("open").arg(url);
            open
        },
    ];
    if let Some(browser) = std::env::var("BROWSER")
        .ok()
        .filter(|b| !b.trim().is_empty())
    {
        let mut words = browser.split_whitespace();
        if let Some(program) = words.next() {
            let mut open = Command::new(program);
            // A `%s` in any word is the address; without one it goes last.
            let mut placed = false;
            for word in words {
                if word.contains("%s") {
                    open.arg(word.replace("%s", url));
                    placed = true;
                } else {
                    open.arg(word);
                }
            }
            if !placed {
                open.arg(url);
            }
            attempts.push(open);
        }
    }

    let mut failure = "the opener never started".to_owned();
    for mut attempt in attempts {
        match launch(&mut attempt) {
            Ok(()) => return Ok(()),
            Err(e) => failure = e,
        }
    }
    Err(failure)
}

/// Start the opener and let go: the frame never waits for a browser. The child is
/// reaped on its own thread, or it sits as a zombie until the app exits.
#[cfg(not(any(test, target_os = "android")))]
fn launch(command: &mut std::process::Command) -> Result<(), String> {
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

impl App {
    /// Hand a page to the platform; the address stays on screen either way.
    pub(crate) fn open_page(&mut self, url: &str) {
        // A test never launches a browser; it records what the app asked for.
        #[cfg(test)]
        {
            self.sync.opened = Some(url.to_owned());
        }
        // The card keeps the code and its page, so the user can finish by hand.
        #[cfg(not(test))]
        if let Err(why) = browse(url) {
            self.shell.fail(tf(
                "no browser could open the page: {why}",
                &[("why", &why)],
            ));
        }
    }

    /// Start the device flow: GitHub mints the code, the frame that draws it opens the
    /// page with the code inside, and nothing is listened on.
    pub fn github_login(&mut self) {
        // A session already in hand, or one on its way from a stored refresh
        // token, must not start a second device flow.
        if self.auth.login.is_some() || self.sync.signed() || self.sync.renew.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let device = match sync::start() {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx.send(SignIn::Failed(e));
                    return;
                }
            };
            let _ = tx.send(SignIn::Code {
                code: device.code().to_owned(),
                page: device.verify.clone(),
                open: device.open.clone(),
            });
            // The page is opened by the frame that copies the code: the browser lands on
            // top, so the code is copied first. The thread polls 15 minutes on its own.
            let landed = sync::await_token(device);
            let _ = tx.send(match landed {
                Ok(session) => SignIn::Token(session),
                Err(e) => SignIn::Failed(e),
            });
        });
        self.auth.login = Some(Login { code: None, rx });
    }

    /// Forget the session; whatever the credential store held goes with it.
    pub fn github_logout(&mut self) {
        self.sync.token = Zeroizing::new(String::new());
        self.sync.renew = None;
        self.sync.expires_at = None;
        self.sync.renewing = None;
        self.sync.renew_at = None;
        self.sync.recall_at = None;
        self.sync.recalls = 0;
        match remember::forget() {
            Ok(()) => self
                .shell
                .set_status("signed out of GitHub", StatusKind::Info),
            Err(e) => self.shell.fail(e),
        }
    }

    /// The session that just landed is the token every request carries, and the
    /// renewal that keeps it alive past its deadline.
    fn adopt(&mut self, session: sync::Session) {
        self.sync.token = session.access;
        self.sync.renew = session.refresh;
        self.sync.expires_at = session.expires_at;
        self.sync.renewing = None;
        self.sync.renew_at = None;
        self.sync.recall_at = None;
        self.sync.recalls = 0;
    }

    /// GitHub refused the token itself, whatever the request was. With a refresh
    /// token in hand that is not the session gone: it is renewed, and the next
    /// request carries the new one. Without it there is nothing to mint another,
    /// so the dead token is dropped and the user signs in again.
    fn expire(&mut self, why: String) {
        if self.sync.renew.is_some() {
            self.sync.expires_at = Some(Instant::now());
            self.sync.renew_at = None;
            self.shell.set_status(
                crate::shared::i18n::t("the session ran out — renewing it"),
                StatusKind::Info,
            );
            return;
        }
        self.github_logout();
        self.shell.fail(why);
    }

    /// Trade the refresh token in before the access token runs out, so no request
    /// ever carries a dead one. One at a time, and a failed attempt waits instead
    /// of spending the frame loop on a GitHub that cannot be reached.
    fn renew(&mut self, ctx: &egui::Context) {
        if self.auth.login.is_some() {
            return;
        }
        if let Some(rx) = self.sync.renewing.take() {
            match rx.try_recv() {
                Ok(Ok(session)) => {
                    self.adopt(session);
                    log::info!("pandora: the GitHub session was renewed");
                    if self.prefs.remember {
                        if let Err(e) = remember::save(&self.sync.credential()) {
                            self.shell.fail(e);
                        }
                    }
                }
                Ok(Err(e)) if sync::expired(&e) => {
                    self.github_logout();
                    self.shell.fail(e);
                    return;
                }
                Ok(Err(e)) => {
                    log::warn!("pandora: the session could not be renewed yet: {e}");
                    self.sync.renew_at = Some(Instant::now() + RENEW_RETRY);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.sync.renewing = Some(rx);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.sync.renew_at = Some(Instant::now() + RENEW_RETRY);
                }
            }
        }
        let Some(refresh) = self.sync.renew.clone() else {
            return;
        };
        let now = Instant::now();
        if let Some(at) = self.sync.renew_at.filter(|at| *at > now) {
            ctx.request_repaint_after(at - now);
            return;
        }
        let left = self.sync.left();
        // A restored pair has no deadline: renewing it once is cheaper than
        // finding out with a 401 which half died while the app was closed.
        if let Some(left) = left {
            if left > sync::RENEW_WINDOW {
                ctx.request_repaint_after(left - sync::RENEW_WINDOW);
                return;
            }
        }
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(sync::refresh(&refresh));
        });
        self.sync.renewing = Some(rx);
    }

    /// The token a request goes out with.
    fn out(&mut self) -> Option<Zeroizing<String>> {
        (!self.sync.token.is_empty()).then(|| Zeroizing::new(self.sync.token.to_string()))
    }

    /// Match the "keep me signed in" choice: hand the session to the store or take it out.
    pub fn keep(&mut self) {
        let result = if self.prefs.remember {
            remember::save(&self.sync.credential())
        } else {
            remember::forget()
        };
        match result {
            Ok(()) => self.shell.set_status(
                if self.prefs.remember {
                    "this session will be here next time"
                } else {
                    "this session is no longer remembered"
                },
                StatusKind::Saved,
            ),
            Err(e) => self.shell.fail(e),
        }
    }

    pub fn repo_repos(&mut self, announce: bool) {
        if self.sync.repo_job.is_some() {
            return;
        }
        if !self.sync.signed() {
            return self.shell.fail("sign in to GitHub first");
        }
        let Some(token) = self.out() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx
                .send(sync::repos(&token).map(|(repos, avatars)| RepoReply::Repos(repos, avatars)));
        });
        self.sync.repo_job = Some(RepoJob {
            source: None,
            announce,
            rx,
        });
        if announce {
            self.shell
                .set_status("asking GitHub for your repos…", StatusKind::Info);
        }
    }

    /// Add a repo as a source, alongside the local boxes and the other repos.
    pub fn source_add(&mut self, target: String) {
        let target = target.trim().to_owned();
        // A repo anyone can read is no place for a box; the write asks GitHub again too.
        if self
            .sync
            .repos
            .iter()
            .any(|r| r.full_name == target && !r.private)
        {
            return self.shell.fail(crate::shared::i18n::t(
                "that repo is public — a box needs a private one",
            ));
        }
        let source = Source {
            kind: GITHUB.to_owned(),
            target,
        };
        self.sync.adding = false;
        if !repo_ok(&source.target) || self.prefs.sources.contains(&source) {
            return;
        }
        self.prefs.sources.push(source.clone());
        self.prefs.stopped.retain(|s| s != &source);
        prefs::save(&self.prefs);
        self.source_refresh(&source, true);
    }

    /// Stop following a source; its working copies stay on disk, out of the local
    /// list and reachable only through the GitHub view. The source is remembered as
    /// stopped, so a repo that answers again is not readopted behind the user's back.
    pub fn source_remove(&mut self, source: &Source) {
        self.prefs.sources.retain(|s| s != source);
        if !self.prefs.stopped.contains(source) {
            self.prefs.stopped.push(source.clone());
        }
        self.sync.listing.remove(&source_key(source));
        self.sync.failed.remove(&source_key(source));
        self.sync.read_at.remove(&source_key(source));
        if self.sync.new_in.as_ref() == Some(source) {
            self.sync.new_in = None;
        }
        prefs::save(&self.prefs);
    }

    /// Read a repo again. `announce` is for the button: an automatic re-read says nothing,
    /// so an unreachable repo does not nag once every window.
    pub fn source_refresh(&mut self, source: &Source, announce: bool) {
        if self.sync.repo_job.is_some() {
            return;
        }
        if !self.sync.signed() {
            if announce {
                self.shell.fail("sign in to GitHub first");
            }
            return;
        }
        if !repo_ok(&source.target) {
            if announce {
                self.shell.fail("that repo is not owner/name");
            }
            return;
        }
        let Some(token) = self.out() else {
            return;
        };
        if announce {
            self.shell.set_status(
                tf("reading {target}…", &[("target", &source.target)]),
                StatusKind::Info,
            );
        }
        // The attempt is what the window measures: a repo nobody answered is retried
        // once per window, not every frame.
        self.sync.read_at.insert(source_key(source), Instant::now());
        let source = source.clone();
        let target = source.target.clone();
        let asked = source.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let listed = sync::list(&target, &token).map(|names| RepoReply::Boxes(source, names));
            let _ = tx.send(listed);
        });
        self.sync.repo_job = Some(RepoJob {
            source: Some(asked),
            announce,
            rx,
        });
    }

    /// Ask the next stale source for its boxes, one at a time, and return how long until
    /// the next one is due, so the frame loop can wake for it.
    pub fn sources_load(&mut self, now: Instant) -> Option<Duration> {
        if !self.sync.signed() || self.sync.repo_job.is_some() {
            return None;
        }
        let mut wait: Option<Duration> = None;
        for source in &self.prefs.sources {
            let left = due_in(self.sync.read_at.get(&source_key(source)).copied(), now);
            if left.is_zero() {
                self.source_refresh(&source.clone(), false);
                return None;
            }
            wait = Some(wait.map_or(left, |w| w.min(left)));
        }
        wait
    }

    /// The box files a source holds; `None` means nobody has read that repo yet.
    pub fn boxes_of(&self, source: &Source) -> Option<&[String]> {
        self.sync
            .listing
            .get(&source_key(source))
            .map(Vec::as_slice)
    }

    /// The source `path` belongs to. The identity comes from the prefs — a folder is
    /// a digest and says nothing — so a copy whose source was lost has none until its
    /// repo answers again and the folder is readopted.
    pub fn source_of(&self, path: &Path) -> Option<Source> {
        self.prefs
            .sources
            .iter()
            .find(|s| path.starts_with(store::source_dir(&s.kind, &s.target)))
            .cloned()
    }

    /// Ask GitHub for the repo list when a copy on disk waits for the repo that
    /// names it: the folder is a digest, so nothing else can say which repo it was.
    /// Once per run — the disk is read once, and never again whatever it held.
    fn repair(&mut self) {
        if self.sync.repaired || !self.sync.signed() {
            return;
        }
        self.sync.repaired = true;
        if self.sync.repo_job.is_none() && stray(&folders(), &self.prefs) {
            self.repo_repos(false);
        }
    }

    /// Bring a source's box to disk and ask for its password; the sha comes with it,
    /// so the next save proves it replaces the version it read.
    pub fn source_open(&mut self, source: &Source, file: &str) {
        if self.sync.repo_job.is_some() {
            return;
        }
        if !self.sync.signed() {
            return self.shell.fail("sign in to GitHub first");
        }
        let Some(token) = self.out() else {
            return;
        };
        let Some((name, path)) = box_path(source, file) else {
            return self.shell.fail("that is not a box file name");
        };
        if let Some(i) = self.boxes.position(&path) {
            self.switch(i);
            return;
        }
        let source = source.clone();
        let target = source.target.clone();
        let (tx, rx) = mpsc::channel();
        let dest = path.clone();
        thread::spawn(move || {
            let landed = (|| {
                let blob = sync::get(&target, &name, &token)?;
                place(&dest, &blob.bytes)?;
                Ok(RepoReply::Opened(source, dest, blob.sha))
            })();
            let _ = tx.send(landed);
        });
        self.sync.repo_job = Some(RepoJob {
            source: None,
            announce: true,
            rx,
        });
        self.shell
            .set_status("getting this box from the repo…", StatusKind::Info);
    }

    /// Create a box inside a source's folder, reusing the create form and its worker;
    /// the path is built here, so the file name cannot leave the folder.
    pub fn source_new(&mut self, source: &Source) {
        let name = self.auth.welcome.name.trim().to_owned();
        if name.is_empty() {
            return self.shell.fail("give your box a name");
        }
        let Some(file) = box_file(&name) else {
            return self.shell.fail("that name cannot be a file name");
        };
        self.auth.welcome.path = store::source_dir(&source.kind, &source.target)
            .join(file)
            .display()
            .to_string();
        self.sync.new_in = None;
        self.auth.create_vault(&mut self.shell);
    }

    /// Bind an open box to a source, so its next save lands there.
    pub(crate) fn mirror(&mut self, source: &Source, path: &Path) {
        let Some(file) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            return;
        };
        let remote = Remote {
            kind: source.kind.clone(),
            repo: source.target.clone(),
            file,
            sha: String::new(),
            folder: String::new(),
            digest: String::new(),
            stamp: 0,
            read_only: false,
            blocked: false,
        };
        self.prefs
            .remotes
            .insert(prefs::box_key(&path.display().to_string()), remote.clone());
        prefs::save(&self.prefs);
        if let Some(s) = self.boxes.find(path) {
            s.remote = Some(remote);
            s.behind = false;
        }
    }

    /// A source's box landed: remember where it came from, then ask for its password.
    fn land_open(&mut self, source: Source, path: PathBuf, sha: String) {
        let file = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        let remote = Remote {
            kind: source.kind,
            repo: source.target,
            file,
            sha,
            folder: String::new(),
            digest: String::new(),
            stamp: 0,
            read_only: false,
            blocked: false,
        };
        self.prefs
            .remotes
            .insert(prefs::box_key(&path.display().to_string()), remote);
        prefs::save(&self.prefs);
        self.auth.unlock_path = path.display().to_string();
        self.auth.unlock_keyfile.clear();
        self.auth.unlock_password.zeroize();
        self.screen = Screen::Unlock;
        self.shell
            .set_status("box ready — unlock it", StatusKind::Info);
    }

    pub fn pull(&mut self) {
        if self.sync.job.is_some() {
            return;
        }
        if !self.sync.signed() {
            return self.shell.fail("sign in to GitHub first");
        }
        let Some(token) = self.out() else {
            return;
        };
        let Some(s) = self.boxes.session.as_ref() else {
            return;
        };
        let Some(remote) = s.remote.clone() else {
            return self.shell.fail("this box is not in a repo yet");
        };
        // A box with unsaved edits is pulled too: the merge below keeps them, and
        // what it keeps is what gets written.
        let (tx, rx) = mpsc::channel();
        let path = s.path.clone();
        thread::spawn(move || {
            let landed = sync::get(&remote.repo, &remote.file, &token)
                .map(|blob| Reply::Pulled(blob.bytes, blob.sha));
            let _ = tx.send(landed);
        });
        self.sync.job = Some(Job { path, rx });
        self.shell
            .set_status("getting the newer version from the repo…", StatusKind::Info);
    }

    pub fn push(&mut self) {
        self.send(true);
    }

    /// The repo gets what is on screen, not what was last saved.
    pub(crate) fn send(&mut self, announce: bool) {
        if self.sync.job.is_some() {
            return;
        }
        if !self.sync.signed() {
            if announce {
                self.shell.fail("sign in to GitHub first");
            }
            return;
        }
        let Some(token) = self.out() else {
            return;
        };
        let Some(s) = self.boxes.session.as_ref() else {
            return;
        };
        let Some(remote) = s.remote.clone() else {
            if announce {
                self.shell.fail("this box is not in a repo yet");
            }
            return;
        };
        let path = s.path.clone();
        let unsaved = s.dirty;
        // One fixed subject: a commit message is plaintext in the repo.
        let message = Fingerprint::SUBJECT.to_owned();

        if unsaved {
            self.boxes.persist(&mut self.shell);
            // Refused means the file moved under the edit: sending it would push another
            // device's box under this one's name.
            if self.boxes.session.as_ref().is_some_and(|s| s.dirty) {
                return;
            }
        }
        // After the write, because a save bumps the generation the fingerprint covers:
        // this is what the landing compares the box against to know it did not move.
        let Some(sent) = self
            .boxes
            .session
            .as_ref()
            .map(|s| Fingerprint::of(&s.vault))
        else {
            return;
        };
        // Size before read: a box past GitHub's ceiling is refused without loading it.
        let size = match fs::metadata(&path) {
            Ok(meta) => meta.len(),
            Err(_) => return self.shell.fail("cannot read the box file"),
        };
        if size > sync::MAX_FILE as u64 {
            if let Some(s) = self.boxes.find(&path) {
                s.push_at = None;
            }
            if announce {
                self.shell.fail(tf(
                    "this box is {size} — a repo file holds at most {max}",
                    &[
                        ("size", &size::human(size as usize)),
                        ("max", &size::human(sync::MAX_FILE)),
                    ],
                ));
            }
            return;
        }
        let Ok(bytes) = fs::read(&path) else {
            return self.shell.fail("cannot read the box file");
        };
        // GitHub would answer 422 ("already in the repo"), so check the ceiling here too.
        if bytes.len() > sync::MAX_FILE {
            if let Some(s) = self.boxes.find(&path) {
                s.push_at = None;
            }
            if announce {
                self.shell.fail(tf(
                    "this box is {size} — a repo file holds at most {max}",
                    &[
                        ("size", &size::human(bytes.len())),
                        ("max", &size::human(sync::MAX_FILE)),
                    ],
                ));
            }
            return;
        }
        let sha = (!remote.sha.is_empty()).then(|| remote.sha.clone());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let landed = sync::put(
                &remote.repo,
                &remote.file,
                &token,
                &bytes,
                sha.as_deref(),
                &message,
            )
            .map(|written| match written {
                sync::Written::Sha(sha) => Reply::Pushed(sha, sent),
                sync::Written::Conflict => Reply::Conflict,
                sync::Written::Later(secs) => Reply::Refused(secs),
            });
            let _ = tx.send(landed);
        });
        self.sync.quota.record(Instant::now());
        self.sync.job = Some(Job { path, rx });
        if announce {
            self.shell
                .set_status("sending the box to the repo…", StatusKind::Info);
        }
    }

    /// Land finished workers and keep frames coming while requests are out; egui idles
    /// otherwise and the reply would wait for the mouse.
    pub fn pump_sync(&mut self, ctx: &egui::Context) {
        let now = crate::vault::model::now_secs();
        if self.prefs.remember
            && !self.sync.signed()
            && self.sync.recall_at.is_some_and(|at| at <= now)
        {
            if let Some(why) = self.sync.recall() {
                log::warn!("pandora: the store still holds nothing: {why}");
                if self.sync.recall_at.is_none() {
                    self.shell.fail(why);
                }
            }
        }
        if let Some(at) = self.sync.recall_at.filter(|at| *at > now) {
            ctx.request_repaint_after(Duration::from_secs(at - now));
        }
        self.renew(ctx);
        // A copy whose source was lost is named by the repos the account holds, so
        // the list is asked for on its own: once a run, not once a frame.
        self.repair();

        let mut signed: Option<SignIn> = None;
        if let Some(login) = self.auth.login.as_mut() {
            match login.rx.try_recv() {
                Ok(step) => signed = Some(step),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    signed = Some(SignIn::Failed("sign-in interrupted".to_owned()));
                }
            }
        }
        match signed {
            Some(SignIn::Code { code, page, open }) => {
                // Copied before the browser opens, so a window on top never hides the code.
                self.shell.copy_text(ctx, &code);
                // The card draws the pair; the page goes out in the same frame.
                if let Some(login) = self.auth.login.as_mut() {
                    login.code = Some((code, page));
                }
                self.open_page(&open);
            }
            Some(SignIn::Token(session)) => {
                self.auth.login = None;
                self.adopt(session);
                if self.prefs.remember {
                    match remember::save(&self.sync.credential()) {
                        Ok(()) => self.shell.set_status(
                            "signed in — kept in the system keyring",
                            StatusKind::Saved,
                        ),
                        Err(e) => self.shell.fail(e),
                    }
                } else {
                    self.shell.set_status(
                        "signed in — the token lives in memory only",
                        StatusKind::Saved,
                    );
                }
            }
            Some(SignIn::Failed(e)) => {
                self.auth.login = None;
                self.shell.fail(e);
            }
            None => {}
        }

        let mut landed: Option<(PathBuf, Result<Reply, String>)> = None;
        if let Some(job) = self.sync.job.as_ref() {
            match job.rx.try_recv() {
                Ok(reply) => landed = Some((job.path.clone(), reply)),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    landed = Some((job.path.clone(), Err("the request was interrupted".into())));
                }
            }
        }
        if let Some((path, reply)) = landed {
            self.sync.job = None;
            let now = Instant::now();
            match reply {
                Ok(Reply::Pulled(bytes, sha)) => self.land_pull(&path, bytes, sha),
                Ok(Reply::Pushed(sha, sent)) => self.land_push(&path, sha, sent),
                Ok(Reply::Conflict) => {
                    // A stale sha would make every later push fail until the repo's
                    // version is in hand. Ask for it now: the pull merges, the merge
                    // writes, and what is left over goes back out with the new sha.
                    if let Some(s) = self.boxes.find(&path) {
                        s.behind = true;
                    }
                    self.retry_later(&path, now);
                    self.pull();
                    self.shell.set_status(
                        "the repo has a newer version — merging it",
                        StatusKind::Info,
                    );
                }
                Ok(Reply::Erased) => {
                    // No copy left: forgetting the sha makes the next push a create.
                    let key = prefs::box_key(&path.display().to_string());
                    if let Some(s) = self.boxes.find(&path) {
                        if let Some(r) = &mut s.remote {
                            r.sha.clear();
                        }
                        s.push_at = None;
                    }
                    if let Some(r) = self.prefs.remotes.get_mut(&key) {
                        r.sha.clear();
                    }
                    prefs::save(&self.prefs);
                    self.shell.set_status(
                        "the repo no longer has this box — the next send creates it again",
                        StatusKind::Saved,
                    );
                }
                Ok(Reply::Refused(secs)) => {
                    self.sync.quota.hold(secs, now);
                    self.retry_later(&path, now);
                    self.shell.fail(tf(
                        "GitHub is not accepting writes for {minutes} min — sending resumes after that",
                        &[("minutes", &secs.div_ceil(60))],
                    ));
                }
                // A 401 is the session and not the operation: nothing this build
                // holds can mint another token, so the dead one is dropped and the
                // user signs in again — the box then sends by itself, its `push_at`
                // left due rather than backed off.
                // shortcut: a pull is not replayed, the user asks again once signed in.
                Err(e) if sync::expired(&e) => self.expire(e),
                Err(e) => {
                    self.retry_later(&path, now);
                    self.shell.fail(e);
                }
            }
        }

        let mut repo_landed: Option<(Option<Source>, bool, Result<RepoReply, String>)> = None;
        if let Some(job) = self.sync.repo_job.as_ref() {
            match job.rx.try_recv() {
                Ok(reply) => repo_landed = Some((job.source.clone(), job.announce, reply)),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    repo_landed = Some((
                        job.source.clone(),
                        job.announce,
                        Err("the request was interrupted".into()),
                    ));
                }
            }
        }
        if let Some((asked, announce, reply)) = repo_landed {
            self.sync.repo_job = None;
            match reply {
                Ok(RepoReply::Repos(mut list, avatars)) => {
                    // Stable order, private repos first: the only ones a box can take.
                    list.sort_by_key(|r| !r.private);
                    // A copy left behind by a lost source comes back with its repo.
                    let adopted = orphans(&self.prefs, &list);
                    if !adopted.is_empty() {
                        self.prefs.sources.extend(adopted);
                        prefs::save(&self.prefs);
                    }
                    self.sync.repos = list;
                    self.sync.avatars = avatars;
                    self.sync.avatar_textures.clear();
                    self.sync.repo_loaded = true;
                }
                Ok(RepoReply::Boxes(source, list)) => {
                    let key = source_key(&source);
                    self.sync.failed.remove(&key);
                    self.sync.listing.insert(key, list);
                }
                Ok(RepoReply::Opened(source, path, sha)) => self.land_open(source, path, sha),
                Err(e) => {
                    // Marked answered even on failure, or an empty answer is asked again
                    // before its window is up.
                    self.sync.repo_loaded = true;
                    if let Some(source) = asked {
                        // A read nobody answered is not a folder without boxes: the listing stays
                        // and the reason is kept.
                        self.sync.failed.insert(source_key(&source), e.clone());
                    }
                    // A dead session is the one refusal no retry fixes, so it is
                    // dropped and said out loud; an automatic read says nothing else.
                    if sync::expired(&e) {
                        self.expire(e);
                    } else if announce {
                        self.shell.fail(e);
                    }
                }
            }
        }

        let mut folder: Option<(
            PathBuf,
            crate::features::sync::model::FolderIntent,
            Result<crate::features::sync::model::FolderReply, String>,
        )> = None;
        if let Some(job) = self.sync.folder_job.as_ref() {
            match job.rx.try_recv() {
                Ok(reply) => folder = Some((job.path.clone(), job.intent, reply)),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    folder = Some((
                        job.path.clone(),
                        job.intent,
                        Err("the folder request was interrupted".to_owned()),
                    ));
                }
            }
        }
        if let Some((path, intent, reply)) = folder {
            self.sync.folder_job = None;
            self.land_folder(&path, intent, reply);
        }

        // Automatic sending: a run of edits lands as one commit once the box settles.
        let now = Instant::now();
        if let Some(wait) = self.folder_tick(now) {
            ctx.request_repaint_after(wait);
        }
        match self.push_wait(now) {
            Some(wait) if wait.is_zero() => self.send(false),
            Some(wait) => ctx.request_repaint_after(wait),
            None => {}
        }

        // Last, because the tick and the send can start a job of their own.
        if self.auth.login.is_some()
            || self.sync.job.is_some()
            || self.sync.folder_job.is_some()
            || self.sync.repo_job.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    /// How long until the box wants a push; zero means due now. Asking for the frame
    /// is what makes the deadline arrive, since egui idles without input.
    fn push_wait(&mut self, now: Instant) -> Option<Duration> {
        if self.sync.job.is_some() || self.sync.token.is_empty() {
            return None;
        }
        if self.sync.quota.held(now).is_some() || self.sync.quota.spent(now) >= sync::WRITE_LIMIT {
            return None;
        }
        let s = self.boxes.session.as_ref()?;
        if !s
            .remote
            .as_ref()
            .is_some_and(|remote| remote.kind == GITHUB)
            || s.conflict
        {
            return None;
        }
        s.push_at.map(|at| at.saturating_duration_since(now))
    }

    /// Back off after a push that did not land; the next change carries the whole file.
    fn retry_later(&mut self, path: &Path, now: Instant) {
        if let Some(s) = self.boxes.find(path) {
            s.push_at = Some(now + sync::PUSH_BACKOFF);
        }
    }

    /// The repo's copy landed, on the box that asked for it; a copy that does not open
    /// never touches the file.
    fn land_pull(&mut self, path: &Path, bytes: Vec<u8>, sha: String) {
        let note: Option<(String, StatusKind)>;
        // A refused copy is not the repo's version, so nothing after this may treat them as agreeing.
        let mut refused = false;
        // A merge that left something only this box holds must not be told the repo
        // already has it: the tail below clears the push for every other landing.
        let mut send = false;
        // None: closed while the request was out. Some(None): open, but no longer mirrored.
        let mut mirror: Option<Option<Remote>> = None;
        match self.boxes.find(path) {
            None => {
                note = Some((
                    "that box was closed — nothing replaced".to_owned(),
                    StatusKind::Error,
                ));
            }
            Some(s) => {
                if s.dirty {
                    // Written while the copy was on its way: replacing the vault would drop
                    // those edits, and a pull is not worth a keystroke.
                    note = Some((
                        "the box changed while its repo copy was on the way — nothing replaced"
                            .to_owned(),
                        StatusKind::Error,
                    ));
                } else if fs::read(&s.path).is_ok_and(|on_disk| on_disk == bytes) {
                    if let Some(r) = &mut s.remote {
                        r.sha = sha;
                    }
                    note = Some((
                        "the repo copy is this box already".to_owned(),
                        StatusKind::Info,
                    ));
                } else if !s.parked && !s.dirty {
                    match reconcile(s, &bytes, &sha) {
                        Ok(Reconcile::Same) => {
                            note = Some((
                                "the repo copy is this box already".to_owned(),
                                StatusKind::Info,
                            ));
                        }
                        Ok(Reconcile::Merged {
                            adopted,
                            conflicts,
                            sending,
                        }) => {
                            send = sending;
                            let mut said = crate::vault::merge::summary(adopted, conflicts);
                            if said.is_empty() {
                                said = crate::shared::i18n::t("this box's own edits were kept")
                                    .to_owned();
                            }
                            let said = if sending {
                                tf("{what} — sending it back", &[("what", &said)])
                            } else {
                                said
                            };
                            note = Some((said, StatusKind::Saved));
                        }
                        Err(e) if Failure::of(&e) == Some(Failure::RolledBack) => {
                            refused = true;
                            note = Some((
                                "the repo copy is older than this box — nothing replaced"
                                    .to_owned(),
                                StatusKind::Error,
                            ));
                        }
                        Err(e) => {
                            note = Some((
                                tf(
                                    "the repo copy does not open with this box's key: {error}",
                                    &[("error", &e)],
                                ),
                                StatusKind::Error,
                            ));
                        }
                    }
                } else {
                    // A parked box is not loaded, and a box with unsaved edits is one
                    // keystroke from losing them: the copy replaces the file, as it
                    // always did, and the merge waits for a moment with neither.
                    let floor = if s.parked {
                        Some(s.parked_seq)
                    } else {
                        Some(s.vault.seq)
                    };
                    match store::adopt(&s.path, &bytes, &s.creds, floor) {
                        Ok(vault) => {
                            if s.parked {
                                // The file remains the source; the tab stays unloaded and remembers where.
                                s.parked_seq = vault.seq;
                            } else {
                                s.vault = vault;
                                s.sanitize();
                            }
                            s.dirty = false;
                            s.last_edit = None;
                            s.conflict = false;
                            s.ignore_mtime = None;
                            s.last_mtime = store::modified_time(&s.path);
                            if let Some(r) = &mut s.remote {
                                r.sha = sha;
                            }
                            note =
                                Some(("box replaced from the repo".to_owned(), StatusKind::Saved));
                        }
                        // Older than what is on screen (rewound, or raced a push): taking it
                        // would undo what happened since.
                        Err(e) if Failure::of(&e) == Some(Failure::RolledBack) => {
                            refused = true;
                            note = Some((
                                "the repo copy is older than this box — nothing replaced"
                                    .to_owned(),
                                StatusKind::Error,
                            ));
                        }
                        Err(e) => {
                            note = Some((
                                tf(
                                    "the repo copy does not open with this box's key: {error}",
                                    &[("error", &e)],
                                ),
                                StatusKind::Error,
                            ));
                        }
                    }
                }
                // Landed: the repo and this box agree. A refused one changed nothing, so a
                // push that was due still is.
                if !refused {
                    s.baseline = Some(Fingerprint::of(&s.vault));
                    // A copy that did not land left this box holding an edit the repo
                    // does not have, so that edit keeps the push it already asked for.
                    if !send && !s.dirty {
                        s.push_at = None;
                    }
                }
                s.behind = false;
                mirror = Some(s.remote.clone());
            }
        }
        if let Some((text, kind)) = note {
            self.shell.set_status(text, kind);
        }
        if let Some(mirror) = mirror {
            self.remember(path, mirror);
        }
    }

    /// The push landed: the repo holds this version, so its sha is the next one named.
    fn land_push(&mut self, path: &Path, sha: String, sent: Fingerprint) {
        let mut mirror = None;
        if let Some(s) = self.boxes.find(path) {
            if let Some(r) = &mut s.remote {
                r.sha = sha;
            }
            // An edit made while the request was out did not leave, so its push stays
            // due instead of being wiped by the landing of the version before it.
            let same = Fingerprint::of(&s.vault) == sent;
            s.baseline = Some(sent);
            if same {
                s.push_at = None;
            }
            s.behind = false;
            mirror = Some(s.remote.clone());
        }
        if let Some(mirror) = mirror {
            self.remember(path, mirror);
        }
        self.shell
            .set_status("box sent to the repo", StatusKind::Saved);
    }

    /// Take the box out of the repo so the old password's version is not served; the
    /// mirror stays, and with no sha the next push creates the file again. The commits
    /// behind it are the repo's history, which no Contents API call rewrites.
    pub(crate) fn erase_remote(&mut self) {
        let Some(token) = self.out() else {
            return;
        };
        let Some(s) = self.boxes.session.as_ref() else {
            return;
        };
        let Some(remote) = s.remote.clone() else {
            return;
        };
        if remote.sha.is_empty() {
            // Nothing was ever pushed, so the repo holds no copy to take away.
            return;
        }
        if self.sync.job.is_some() {
            self.shell
                .fail("another sync is already running — the repo still holds this box's copy");
            return;
        }
        let path = s.path.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let landed = sync::erase(&remote.repo, &remote.file, &remote.sha, &token)
                .map(|()| Reply::Erased);
            let _ = tx.send(landed);
        });
        self.sync.quota.record(Instant::now());
        self.sync.job = Some(Job { path, rx });
        self.shell
            .set_status("taking the box out of the repo…", StatusKind::Info);
    }

    /// Remember where a box is mirrored; the token is never written down.
    pub(crate) fn remember(&mut self, path: &Path, remote: Option<Remote>) {
        let key = prefs::box_key(&path.display().to_string());
        match remote {
            Some(remote) => {
                self.prefs.remotes.insert(key, remote);
            }
            None => {
                self.prefs.remotes.remove(&key);
            }
        }
        prefs::save(&self.prefs);
    }
}

/// What a pull's merge did with the box on screen.
enum Reconcile {
    /// The repo says what this box says: nothing was written.
    Same,
    /// The two were folded: what the other copy had is here now.
    Merged {
        adopted: usize,
        conflicts: usize,
        /// This box holds something the repo does not, so a push is due.
        sending: bool,
    },
}

/// The repo's copy, decrypted with the key this session holds, without touching the
/// box: chunks are read through a file, so the bytes land in a private temp beside it.
fn remote_vault(path: &Path, bytes: &[u8], creds: &store::Credentials) -> Result<Vault, String> {
    let candidate = path.with_extension(format!("{}.incoming", store::EXT));
    store::write_atomic(&candidate, bytes)?;
    let opened = store::load(&candidate, creds);
    let _ = fs::remove_file(&candidate);
    opened
}

/// Fold the repo's copy into the box on screen and write the result: this device keeps
/// its edits, the repo's arrive, and a record both changed at once stays twice. The
/// write is the normal one — atomic, and it keeps the version it displaces in the
/// stash — so a merge is never the last word on a box.
fn reconcile(s: &mut crate::app::Session, bytes: &[u8], sha: &str) -> Result<Reconcile, String> {
    let theirs = remote_vault(&s.path, bytes, &s.creds)?;
    // The pull names the generation: a rewound repo hands a valid old blob with a
    // valid sha, and nothing else would notice.
    if theirs.seq < s.vault.seq {
        return Err(Failure::RolledBack.to_string());
    }

    if crate::vault::merge::equal(&s.vault, &theirs) {
        if let Some(remote) = &mut s.remote {
            remote.sha = sha.to_owned();
        }
        s.baseline = Some(Fingerprint::of(&s.vault));
        s.push_at = None;
        s.behind = false;
        return Ok(Reconcile::Same);
    }

    let merged = crate::vault::merge::merge(&s.vault, &theirs, crate::vault::model::now_secs());
    let (adopted, conflicts) = (merged.adopted, merged.conflicts);
    s.vault = merged.vault;
    s.sanitize();
    store::save(&s.path, &mut s.vault, &s.creds)?;
    s.dirty = false;
    s.last_edit = None;
    s.conflict = false;
    s.ignore_mtime = None;
    s.last_mtime = store::modified_time(&s.path);
    if let Some(remote) = &mut s.remote {
        remote.sha = sha.to_owned();
    }
    s.baseline = Some(Fingerprint::of(&s.vault));
    s.behind = false;
    // The repo does not hold the merged box yet; if it holds something else than what
    // this box now says, the next push carries it under the sha just read.
    let sending = !crate::vault::merge::equal(&s.vault, &theirs);
    s.push_at = sending.then(std::time::Instant::now);
    Ok(Reconcile::Merged {
        adopted,
        conflicts,
        sending,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session already in hand must not start a second device flow.
    #[test]
    fn github_login_does_not_run_when_signed_in() {
        let ctx = egui::Context::default();
        let mut app = crate::app::App::blank(&ctx);
        app.sync.restore("gho_kept".to_owned());
        assert!(app.sync.signed());
        app.github_login();
        assert!(app.auth.login.is_none(), "no device flow while signed in");
    }

    /// A stored refresh token is already a session on its way: a second device
    /// flow would mint one over it.
    #[test]
    fn github_login_waits_for_a_stored_renewal() {
        let ctx = egui::Context::default();
        let mut app = crate::app::App::blank(&ctx);
        app.sync.restore("ghr_kept".to_owned());
        app.github_login();
        assert!(
            app.auth.login.is_none(),
            "the renewal is what brings the token back"
        );
    }

    /// A 401 is not the session gone while a refresh token can mint another: the
    /// token is kept and the next frame renews it, instead of asking to sign in.
    #[test]
    fn an_expired_token_with_a_renewal_keeps_the_session() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.sync.restore("gho_a\nghr_b".to_owned());
        app.expire(sync::EXPIRED.to_owned());
        assert!(app.sync.signed(), "the renewal is still in hand");
        assert!(
            app.sync.left().is_some_and(|left| left.is_zero()),
            "the renewal is due now, not at the old deadline"
        );
        assert!(app.sync.renew.is_some());
    }

    #[test]
    fn a_repo_is_owner_slash_name_and_nothing_else() {
        assert!(repo_ok("octocat/boxes"));
        assert!(repo_ok("me/private-vault"));
        for bad in ["", "boxes", "/boxes", "octocat/", "a/b/c", "/"] {
            assert!(!repo_ok(bad), "{bad:?} is not a repo");
        }
    }

    /// The disk is looked at once per run: a copy still waiting does not make the
    /// next frame ask again.
    #[test]
    fn the_disk_is_read_once_a_run() {
        let ctx = egui::Context::default();
        let mut app = crate::app::App::blank(&ctx);
        app.sync.restore("gho_kept".to_owned());
        app.sync.repaired = true;

        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/latched".to_owned(),
        };
        let dir = store::source_dir(&source.kind, &source.target);
        fs::create_dir_all(&dir).unwrap();

        app.repair();
        assert!(
            app.sync.repo_job.is_none(),
            "a run that has already looked asks nothing"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    fn repo(full_name: &str, private: bool) -> Repo {
        Repo {
            full_name: full_name.to_owned(),
            owner: full_name.split('/').next().unwrap().to_owned(),
            private,
        }
    }

    /// A copy outlives the source that made it: when the repo answers again, the
    /// folder it left behind is what brings it back.
    #[test]
    fn a_copy_whose_source_was_lost_is_readopted() {
        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/readopted".to_owned(),
        };
        let dir = store::source_dir(&source.kind, &source.target);
        fs::create_dir_all(&dir).unwrap();

        let listed = [
            repo("octocat/readopted", true),
            repo("octocat/public", false),
            repo("octocat/absent", true),
        ];
        assert_eq!(
            orphans(&prefs::Prefs::default(), &listed),
            vec![source.clone()],
            "only a private repo with a folder is readopted"
        );

        let mut held = prefs::Prefs::default();
        held.sources.push(source.clone());
        assert!(
            orphans(&held, &listed).is_empty(),
            "a source still followed is not adopted twice"
        );

        let mut stopped = prefs::Prefs::default();
        stopped.stopped.push(source);
        assert!(
            orphans(&stopped, &listed).is_empty(),
            "the repo the user walked away from stays away"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// A folder nobody follows asks for the repo list; one the user walked away
    /// from, and one an older layout left, do not.
    #[test]
    fn a_folder_nobody_follows_asks_for_the_repos() {
        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/stray".to_owned(),
        };
        let digest = store::source_digest(&source.kind, &source.target);
        let plain = prefs::Prefs::default();

        assert!(stray(std::slice::from_ref(&digest), &plain));
        assert!(
            !stray(&["github".to_owned()], &plain),
            "an older layout's folder is not a digest"
        );

        let mut followed = prefs::Prefs::default();
        followed.sources.push(source.clone());
        assert!(!stray(std::slice::from_ref(&digest), &followed));

        let mut stopped = prefs::Prefs::default();
        stopped.stopped.push(source);
        assert!(
            !stray(&[digest], &stopped),
            "the repo the user walked away from stays quiet"
        );
    }

    /// A pull folds the repo's copy into this one: the edit made here and the edit
    /// made there both survive, the file is written once, and the merged box is
    /// scheduled to go back out under the sha just read.
    #[test]
    fn a_pull_merges_both_sides_and_schedules_the_send() {
        let ctx = egui::Context::default();
        let mut app = crate::app::App::blank(&ctx);
        app.open("merge-pull");

        let (path, theirs_path) = {
            let s = app.boxes.session.as_mut().unwrap();
            s.remote = Some(crate::features::settings::model::Remote {
                kind: "github".to_owned(),
                repo: "me/boxes".to_owned(),
                file: "box.pandora".to_owned(),
                sha: String::new(),
                folder: String::new(),
                digest: String::new(),
                stamp: 0,
                read_only: false,
                blocked: false,
            });
            s.vault.logins.push(crate::vault::model::Login::new(1));
            store::save(&s.path, &mut s.vault, &s.creds).unwrap();
            (s.path.clone(), s.path.with_extension("other"))
        };

        // The repo's copy: this box, plus a login written on another device.
        let bytes = {
            let s = app.boxes.session.as_mut().unwrap();
            let mut theirs = s.vault.clone();
            let mut far = crate::vault::model::Login::new(1);
            far.name = "From the repo".to_owned();
            far.modified = crate::vault::model::now_secs();
            theirs.logins.push(far);
            store::save(&theirs_path, &mut theirs, &s.creds).unwrap();
            fs::read(&theirs_path).unwrap()
        };

        // And this device's own edit, made while the pull was in flight.
        {
            let s = app.boxes.session.as_mut().unwrap();
            let mut here = crate::vault::model::Login::new(1);
            here.name = "From here".to_owned();
            here.modified = crate::vault::model::now_secs();
            s.vault.logins.push(here);
            store::save(&s.path, &mut s.vault, &s.creds).unwrap();
            s.last_mtime = store::modified_time(&s.path);
        }

        app.land_pull(&path, bytes, "sha-2".to_owned());

        let s = app.boxes.session.as_ref().unwrap();
        let names: Vec<&str> = s.vault.logins.iter().map(|it| it.name.as_str()).collect();
        assert_eq!(
            names.len(),
            3,
            "the login, the repo's and this device's: {names:?}"
        );
        assert!(names.contains(&"From the repo"));
        assert!(names.contains(&"From here"));
        assert!(s.push_at.is_some(), "the merged box goes back out");
        assert!(!s.behind, "and the repo's version is in hand");
        assert_eq!(s.remote.as_ref().unwrap().sha, "sha-2", "the push names it");

        let on_disk = store::load(&path, &s.creds).unwrap();
        assert_eq!(on_disk.logins.len(), 3, "the file holds the merge");

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&theirs_path);
    }

    /// A push carries what left. An edit made while the request was out did not
    /// leave, so it keeps its own push instead of being wiped by the landing.
    #[test]
    fn an_edit_made_while_a_push_is_out_keeps_its_own_push() {
        let (mut app, path, _) = diverged(|_| {}, |_| {});
        let sent = app
            .boxes
            .session
            .as_ref()
            .map(|s| Fingerprint::of(&s.vault))
            .unwrap();
        {
            let s = app.boxes.session.as_mut().unwrap();
            s.vault.logins.push(crate::vault::model::Login::new(1));
            s.mark_dirty();
        }

        app.land_push(&path, "sha-9".to_owned(), sent);

        let s = app.boxes.session.as_ref().unwrap();
        assert!(
            s.push_at.is_some(),
            "the edit that did not leave is still owed"
        );
        assert_eq!(
            s.remote.as_ref().unwrap().sha,
            "sha-9",
            "and the next push names what landed"
        );

        let _ = fs::remove_file(&path);
    }

    /// A landing nobody overtook is the version the repo holds: the wait is over.
    #[test]
    fn a_push_that_lands_unchanged_clears_the_wait() {
        let (mut app, path, _) = diverged(|_| {}, |_| {});
        let sent = app
            .boxes
            .session
            .as_ref()
            .map(|s| Fingerprint::of(&s.vault))
            .unwrap();
        app.boxes.find(&path).unwrap().push_at = Some(std::time::Instant::now());

        app.land_push(&path, "sha-1".to_owned(), sent);

        assert!(
            app.boxes.find(&path).unwrap().push_at.is_none(),
            "nothing is waiting to go out"
        );

        let _ = fs::remove_file(&path);
    }

    /// A pull that leaves an edit alone changed nothing on disk, so the edit keeps
    /// the push it was already waiting for.
    #[test]
    fn a_pull_that_leaves_an_edit_alone_keeps_its_push() {
        let (mut app, path, bytes) = diverged(|_| {}, |_| {});
        {
            let s = app.boxes.session.as_mut().unwrap();
            s.vault.logins[0].name = "Mine".to_owned();
            s.mark_dirty();
        }

        app.land_pull(&path, bytes, "sha-2".to_owned());

        let s = app.boxes.session.as_ref().unwrap();
        assert!(s.dirty, "the edit stayed");
        assert!(s.push_at.is_some(), "and the repo still owes it a push");

        let _ = fs::remove_file(&path);
    }

    /// The repo being ahead is not a reason to park the queue: a push under a stale
    /// sha is refused as a conflict and pulled again, so it heals by itself.
    #[test]
    fn the_repo_being_ahead_does_not_stop_the_next_push() {
        let (mut app, path, _) = diverged(|_| {}, |_| {});
        app.sync.restore("gho_x".to_owned());
        {
            let s = app.boxes.session.as_mut().unwrap();
            s.behind = true;
            s.push_at = Some(std::time::Instant::now());
        }

        assert!(
            app.push_wait(std::time::Instant::now())
                .is_some_and(|wait| wait.is_zero()),
            "a conflict is retried, not parked"
        );

        let _ = fs::remove_file(&path);
    }

    /// This box, opened and saved, with a remote copy made by saving a changed clone
    /// beside it under the same key: what another device would have written.
    fn diverged(
        edit_here: impl FnOnce(&mut Vault),
        edit_there: impl FnOnce(&mut Vault),
    ) -> (crate::app::App, std::path::PathBuf, Vec<u8>) {
        let ctx = egui::Context::default();
        let mut app = crate::app::App::blank(&ctx);
        app.open("diverged");
        let (path, theirs_path, bytes) = {
            let s = app.boxes.session.as_mut().unwrap();
            s.remote = Some(crate::features::settings::model::Remote {
                kind: "github".to_owned(),
                repo: "me/boxes".to_owned(),
                file: "box.pandora".to_owned(),
                sha: String::new(),
                folder: String::new(),
                digest: String::new(),
                stamp: 0,
                read_only: false,
                blocked: false,
            });
            let mut mine = crate::vault::model::Login::new(1);
            mine.id = "bank".to_owned();
            mine.name = "Bank".to_owned();
            mine.password = zeroize::Zeroizing::new("mine".to_owned());
            mine.modified = crate::vault::model::now_secs();
            s.vault.logins.push(mine);
            edit_here(&mut s.vault);
            store::save(&s.path, &mut s.vault, &s.creds).unwrap();

            let mut theirs = s.vault.clone();
            edit_there(&mut theirs);
            let theirs_path = s.path.with_extension("other");
            store::save(&theirs_path, &mut theirs, &s.creds).unwrap();
            let bytes = fs::read(&theirs_path).unwrap();
            s.last_mtime = store::modified_time(&s.path);
            (s.path.clone(), theirs_path, bytes)
        };
        // The remote copy is a file only while the test runs; the box it was made from
        // is the one on screen.
        let _ = theirs_path;
        (app, path, bytes)
    }

    fn said(app: &crate::app::App) -> String {
        app.shell
            .status
            .as_ref()
            .map(|s| s.text.clone())
            .unwrap_or_default()
    }

    fn passwords(app: &crate::app::App) -> Vec<String> {
        let s = app.boxes.session.as_ref().unwrap();
        s.vault
            .logins
            .iter()
            .map(|it| it.password.to_string())
            .collect()
    }

    /// The same login edited on both sides at nearly the same second is not a race to
    /// win: both passwords stay, the older one under a name that says so, and the bar
    /// says how many were kept twice.
    #[test]
    fn both_edits_of_one_record_survive_a_pull() {
        let now = crate::vault::model::now_secs();
        let (mut app, path, bytes) = diverged(
            |vault| vault.logins[0].password = zeroize::Zeroizing::new("mine-2".to_owned()),
            |vault| {
                vault.logins[0].password = zeroize::Zeroizing::new("theirs".to_owned());
                vault.logins[0].modified = now;
            },
        );
        app.boxes.session.as_mut().unwrap().vault.logins[0].modified = now;

        app.land_pull(&path, bytes, "sha-2".to_owned());

        let mut kept = passwords(&app);
        kept.sort();
        assert_eq!(kept, ["mine-2", "theirs"], "neither password is lost");
        let s = app.boxes.session.as_ref().unwrap();
        assert!(
            s.vault
                .logins
                .iter()
                .any(|it| it.name.ends_with(" (conflict)")),
            "the copy says it is one"
        );
        assert!(
            said(&app).contains("kept twice"),
            "the bar counts them: {}",
            said(&app)
        );
        let on_disk = store::load(&path, &s.creds).unwrap();
        assert_eq!(on_disk.logins.len(), 2, "and the file holds both");

        let _ = fs::remove_file(&path);
    }

    /// A deletion written on the other side reaches this one, and the bar says a record
    /// came over: the tombstone is not silently dropped.
    #[test]
    fn a_deletion_on_the_other_side_removes_what_it_outlived() {
        let (mut app, path, bytes) = diverged(
            |_| {},
            |vault| {
                let at = vault.logins[0].modified + 5;
                vault.take("bank", at).unwrap();
            },
        );

        app.land_pull(&path, bytes, "sha-2".to_owned());

        let s = app.boxes.session.as_ref().unwrap();
        assert!(s.vault.logins.is_empty(), "the delete reached this side");
        assert_eq!(s.vault.trash.len(), 1, "and the stone is kept");
        assert!(
            said(&app).contains("came over"),
            "the bar says what moved: {}",
            said(&app)
        );
        let on_disk = store::load(&path, &s.creds).unwrap();
        assert!(on_disk.logins.is_empty());

        let _ = fs::remove_file(&path);
    }

    /// An edit made after the other side deleted the record is newer than the stone,
    /// and stays: deleting is not a way to overwrite a later change.
    #[test]
    fn an_edit_after_the_other_sides_deletion_survives() {
        let now = crate::vault::model::now_secs();
        let (mut app, path, bytes) = diverged(
            |vault| vault.logins[0].modified = now,
            |vault| {
                vault.take("bank", now.saturating_sub(100)).unwrap();
            },
        );

        app.land_pull(&path, bytes, "sha-2".to_owned());

        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.logins.len(), 1, "the later edit is the truth");
        assert_eq!(s.vault.logins[0].password.as_str(), "mine");
        assert!(s.vault.trash.is_empty(), "the stone is dropped");

        let _ = fs::remove_file(&path);
    }

    /// The same version pulled twice duplicates nothing, and a record both copies hold
    /// identically — touched at different times — is still one record.
    #[test]
    fn a_merge_never_duplicates_records() {
        let now = crate::vault::model::now_secs();
        let (mut app, path, bytes) = diverged(
            |_| {},
            |vault| {
                // The same record, touched later and otherwise untouched.
                vault.logins[0].modified = now + 60;
            },
        );

        app.land_pull(&path, bytes.clone(), "sha-2".to_owned());
        assert_eq!(app.boxes.session.as_ref().unwrap().vault.logins.len(), 1);
        assert!(
            said(&app).contains("already"),
            "and it says the repo had it: {}",
            said(&app)
        );

        // The file is where the merge wrote it: the same blob again changes nothing.
        let again = fs::read(&path).unwrap();
        app.land_pull(&path, again, "sha-3".to_owned());
        assert_eq!(
            app.boxes.session.as_ref().unwrap().vault.logins.len(),
            1,
            "a second pull of the same version adds no copy"
        );

        let _ = fs::remove_file(&path);
    }

    /// A pull older than the box is refused: a rewound repo hands a valid old blob, and
    /// its sha says nothing about it; nothing on screen moves.
    #[test]
    fn a_repo_copy_older_than_the_box_never_replaces_it() {
        let ctx = egui::Context::default();
        let mut app = crate::app::App::blank(&ctx);
        app.open("rollback");

        let path = {
            let s = app.boxes.session.as_mut().unwrap();
            store::save(&s.path, &mut s.vault, &s.creds).unwrap();
            s.path.clone()
        };
        let old = fs::read(&path).unwrap();

        let here = {
            let s = app.boxes.session.as_mut().unwrap();
            s.vault.logins.push(crate::vault::model::Login::new(1));
            store::save(&s.path, &mut s.vault, &s.creds).unwrap();
            s.last_mtime = store::modified_time(&path);
            s.vault.seq
        };

        app.land_pull(&path, old, "sha".to_owned());

        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.seq, here, "the box on screen did not move");
        assert_eq!(s.vault.logins.len(), 1, "nor did what it holds");
        let said = app.shell.status.as_ref().map(|s| s.text.clone());
        assert!(
            said.as_deref()
                .is_some_and(|t| t.contains("older than this box")),
            "and it says so: {said:?}"
        );
        let on_disk = store::load(&path, &s.creds).unwrap();
        assert_eq!(on_disk.seq, here, "the file was not replaced either");
        let _ = fs::remove_file(&path);
    }

    /// The window keeps the automatic re-read from asking every frame; a source never
    /// read is due at once, so the first look fills itself in.
    #[test]
    fn a_listing_is_due_only_once_its_window_is_up() {
        let now = Instant::now();
        assert_eq!(due_in(None, now), Duration::ZERO, "never read is due now");
        assert_eq!(due_in(Some(now), now), sync::STALE, "fresh waits it out");

        let old = now - sync::STALE - Duration::from_secs(1);
        assert_eq!(due_in(Some(old), now), Duration::ZERO, "the window is up");
    }

    /// A repo just read is left alone; the view gets the wait back.
    #[test]
    fn a_fresh_repo_is_not_read_again() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.sync.token = Zeroizing::new("t".to_owned());
        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/boxes".to_owned(),
        };
        app.prefs.sources = vec![source.clone()];
        app.sync.read_at.insert(source_key(&source), Instant::now());

        let wait = app
            .sources_load(Instant::now())
            .expect("a wait is handed back");
        assert!(wait <= sync::STALE, "wait {wait:?} outran the window");
        assert!(app.sync.repo_job.is_none(), "nothing was asked for");
    }

    /// Only the last path component is used, so a crafted name cannot escape.
    #[test]
    fn a_source_box_lands_inside_its_folder() {
        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/boxes".to_owned(),
        };
        let dir = store::source_dir(&source.kind, &source.target);

        let (name, path) = box_path(&source, "work.pandora").unwrap();
        assert_eq!(name, "work.pandora");
        assert_eq!(path.file_name().unwrap(), "work.pandora");
        assert!(path.starts_with(&dir));

        let (name, path) = box_path(&source, "../../etc/passwd").unwrap();
        assert_eq!(name, "passwd");
        assert!(path.starts_with(&dir), "{path:?} escaped the folder");
    }

    #[test]
    fn two_repos_are_two_sources() {
        let one = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/boxes".to_owned(),
        };
        let two = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/work".to_owned(),
        };
        assert_eq!(source_key(&one), "github:octocat/boxes");
        assert_ne!(source_key(&one), source_key(&two));
    }

    /// A read nobody answered is not a folder without boxes: the listing stays and the
    /// reason is kept. A dead token ends the session, or every read fails in silence.
    #[test]
    fn a_read_that_did_not_land_is_not_an_empty_repo() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.sync.token = Zeroizing::new("t".to_owned());
        let source = Source {
            kind: GITHUB.to_owned(),
            target: "octocat/unanswered".to_owned(),
        };
        app.prefs.sources = vec![source.clone()];
        app.sync
            .listing
            .insert(source_key(&source), vec!["kept.pandora".to_owned()]);

        let refused = "github: rate limited — wait 1 min";
        let (tx, rx) = mpsc::channel();
        app.sync.repo_job = Some(RepoJob {
            source: Some(source.clone()),
            announce: false,
            rx,
        });
        tx.send(Err(refused.to_owned())).unwrap();
        app.pump_sync(&ctx);

        assert_eq!(
            app.sync
                .failed
                .get(&source_key(&source))
                .map(String::as_str),
            Some(refused),
            "the reason is kept, however automatic the read was"
        );
        assert_eq!(
            app.boxes_of(&source),
            Some(["kept.pandora".to_owned()].as_slice()),
            "a listing that answered before is left alone"
        );
        assert_eq!(
            app.sync.token.as_str(),
            "t",
            "a rate limit is not a dead session"
        );
    }

    #[test]
    fn a_box_name_becomes_a_file_name_inside_the_folder() {
        assert_eq!(box_file("Work").unwrap(), "Work.pandora");
        assert_eq!(box_file("My Box").unwrap(), "My Box.pandora");
        let hostile = box_file("../../etc/passwd").unwrap();
        assert!(!hostile.contains('/'), "{hostile} kept a separator");
        assert!(box_file("   ").is_none());
        assert!(box_file("...").is_none());
    }

    /// A repo never listed still shows what the disk holds: only `.pandora` files.
    #[test]
    fn the_working_copies_come_off_the_disk() {
        let dir = std::env::temp_dir().join("pandora-cached");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("work.pandora"), b"x").unwrap();
        fs::write(dir.join("home.pandora"), b"x").unwrap();
        fs::write(dir.join("notes.txt"), b"x").unwrap();

        assert_eq!(list_dir(&dir), ["home.pandora", "work.pandora"]);
        assert!(list_dir(&dir.join("gone")).is_empty());
    }

    /// The frame that lands the code opens the page, with the code on the clipboard
    /// first; a test build records the address instead of launching a browser.
    #[test]
    fn the_code_opens_the_page_it_names() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        let (tx, rx) = mpsc::channel();
        app.auth.login = Some(Login { code: None, rx });

        app.pump_sync(&ctx);
        assert!(app.shell.clip.is_none(), "there is nothing to copy yet");
        assert!(app.sync.opened.is_none(), "and nothing to open either");

        let page = "https://github.com/login/device";
        tx.send(SignIn::Code {
            code: "ABCD-1234".to_owned(),
            page: page.to_owned(),
            open: sync::device_url(page, "ABCD-1234"),
        })
        .unwrap();
        app.pump_sync(&ctx);

        let shown = app
            .auth
            .login
            .as_ref()
            .and_then(|login| login.code.as_ref())
            .map(|(code, _)| code.clone());
        assert_eq!(shown.as_deref(), Some("ABCD-1234"));
        let clip = app.shell.clip.expect("the code arms the countdown");
        assert!(clip.holds("ABCD-1234"), "the code went out, not the page");
        assert_eq!(
            app.sync.opened.as_deref(),
            Some(sync::device_url(page, "ABCD-1234").as_str()),
            "the page goes out with the code already in it"
        );
    }

    /// GitHub's own `verification_uri_complete` goes out unchanged when it sends one.
    #[test]
    fn the_page_the_worker_minted_is_the_one_opened() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        let (tx, rx) = mpsc::channel();
        app.auth.login = Some(Login { code: None, rx });

        let page = "https://github.com/login/device";
        // Deliberately not what `device_url` builds: only GitHub could choose this spelling.
        let complete = "https://github.com/login/device/verify?code=ABCD-1234";
        tx.send(SignIn::Code {
            code: "ABCD-1234".to_owned(),
            page: page.to_owned(),
            open: complete.to_owned(),
        })
        .unwrap();
        app.pump_sync(&ctx);

        assert_eq!(
            app.sync.opened.as_deref(),
            Some(complete),
            "the minted address wins over one the app would have composed"
        );
        assert_ne!(
            app.sync.opened.as_deref(),
            Some(sync::device_url(page, "ABCD-1234").as_str()),
            "and it is not `device_url`'s spelling"
        );
        // The card keeps the bare page: its button rebuilds the address for a retry.
        assert_eq!(
            app.auth.login.as_ref().and_then(|l| l.code.clone()),
            Some(("ABCD-1234".to_owned(), page.to_owned()))
        );
    }
}
