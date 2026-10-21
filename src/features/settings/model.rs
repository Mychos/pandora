//! User preferences — a small JSON in the app data dir, next to the recents and
//! outside any box, so the box format never carries them. Best effort: a missing
//! or unreadable file falls back to the defaults.

use crate::features::entries::model::{Axis, View};
use crate::shared::i18n::Lang;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_LOCK_AFTER: u64 = 15 * 60;
const LEGACY_LOCK_AFTER: u64 = 30;

const DEFAULT_CLIP_CLEAR: u64 = 30;

pub const MAX_LOCK_AFTER: u64 = 30 * 60;

pub const MAX_CLIP_CLEAR: u64 = 600;

pub const MAX_RETRY_DELAY: u64 = 3600;

/// GitHub is the only provider there is, so it is also what anything written
/// before sources had kinds is read as.
pub const GITHUB: &str = "github";

pub const MAX_ATTEMPTS: u32 = 50;

/// Below a few seconds the wait is not felt at all, so a limit with no wait is
/// raised to this instead of refusing a box for zero seconds.
pub const MIN_RETRY: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Seconds of inactivity before locking; 0 disables idle locking.
    pub lock_after: u64,
    /// Seconds a copied secret stays on the system clipboard; 0 never clears it.
    pub clip_clear: u64,
    /// Failed unlocks a box gets before it is refused; 0 never refuses.
    pub max_attempts: u32,
    /// Seconds a box waits after a failed unlock; 0 waits for nothing.
    pub retry_delay: u64,
    /// One entry per box path — failures since its last unlock, and until when it
    /// is refused.
    // ponytail: one entry per path ever failed, never pruned — bounded by the
    // boxes the user has, and a stale entry costs a few bytes. Prune on write if
    // that ever stops being true.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub tries: HashMap<String, Tries>,
    /// One entry per box path — where it is mirrored, when it is. A source holds
    /// any number of boxes, one file each.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub remotes: HashMap<String, Remote>,
    /// The repos the boxes live in, alongside the local files. Any number of
    /// them, each its own folder of boxes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
    /// Whether the GitHub token is handed to the system credential store, so a
    /// signed-in session survives a restart. On by default: a vault that asks for
    /// a device-flow sign-in every morning is a vault nobody signs into. The
    /// checkbox in the GitHub pane turns it off, and turning it off takes the
    /// token back out of the store.
    #[serde(skip_serializing_if = "is_true")]
    pub remember: bool,
    /// Whether a box's derived key is kept for the device's own reader — a
    /// fingerprint, a face — so the next unlock is a touch and not a password.
    /// Off by default: nothing is sealed unless the user asks for it, and the
    /// seal is per box, made the next time that box's password works.
    #[serde(skip_serializing_if = "is_false")]
    pub biometric: bool,
    /// Whether a browser may open a box on its own, with the master password typed
    /// into the extension. Off by default: with the app open the host relays to it
    /// and never needs a password, and the one thing that would put a master password
    /// inside a browser is this.
    #[serde(skip_serializing_if = "is_false")]
    pub unlock_from_browser: bool,
    /// Browsers the user has let talk to the app, named the way the browser named
    /// them — `chrome-extension://<id>/`, or an add-on id. Empty means nothing has
    /// been approved, so the bridge asks before letting one in.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub browsers: Vec<String>,
    /// Whether the phone's own autofill may read the box on screen. On by default:
    /// the copy lives only while the box is unlocked, and the panel is the whole
    /// point of a manager on a phone. Off keeps the box to this window.
    #[serde(skip_serializing_if = "is_true")]
    pub autofill: bool,
    /// How the passwords pane last drew a box, and what it grouped by, so a fresh
    /// unlock lands where the user left off instead of asking again.
    pub view: View,
    pub axis: Axis,
    pub lang: Lang,
    /// Which way the app is painted. Missing in a file written before the choice
    /// existed, and then the default — dark — is what it reads as.
    pub theme: crate::shared::theme::Scheme,
    /// Whether the app may ask GitHub for a newer release on its own — once at
    /// startup and again when the window comes back. On by default, because a
    /// vault nobody is told about is a vault left with a fixed bug unfixed; the
    /// switch in the settings turns it off.
    #[serde(skip_serializing_if = "is_true")]
    pub update_check: bool,
    /// The release the user answered "Later" to, so the notice does not come back
    /// every launch for a version they already decided against.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub skip: String,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The mirror of [`is_false`], for a flag whose default is on: what is written
/// down is the "no", because silence has come to mean "yes".
fn is_true(value: &bool) -> bool {
    *value
}

/// One place boxes live: a provider and what identifies the target inside it.
/// `kind` is `prefs::GITHUB`; `target` is `owner/name`. The local disk is not
/// here — it is implicit, always present, and never written down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub kind: String,
    pub target: String,
}

/// Where a box is mirrored: a source and the file inside it. The token is not
/// here — it only ever lives in memory, for one run.
// ponytail: GitHub is the only provider, so `kind` is always GITHUB. It is part
// of the contract anyway, so the second integration does not need a second
// migration to say which provider a box came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    #[serde(default = "github")]
    pub kind: String,
    pub repo: String,
    pub file: String,
    /// The blob sha this box last saw, so a push can prove it is replacing the
    /// version it read. Empty until the box has been pushed once.
    #[serde(default)]
    pub sha: String,
}

fn github() -> String {
    GITHUB.to_owned()
}

/// What one box's path remembers about failed unlocks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tries {
    /// Failures since the box was last unlocked.
    pub fails: u32,
    /// Unix seconds it stays refused until; 0 is try now.
    pub until: u64,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            lock_after: DEFAULT_LOCK_AFTER,
            clip_clear: DEFAULT_CLIP_CLEAR,
            max_attempts: 0,
            retry_delay: 0,
            tries: HashMap::new(),
            remotes: HashMap::new(),
            sources: Vec::new(),
            remember: true,
            biometric: false,
            unlock_from_browser: false,
            browsers: Vec::new(),
            autofill: true,
            view: View::List,
            axis: Axis::Target,
            lang: Lang::En,
            theme: crate::shared::theme::Scheme::Dark,
            update_check: true,
            skip: String::new(),
        }
    }
}

/// Unix seconds now. The refusal has to survive a restart, so it is wall clock
/// and not an `Instant`.
pub fn now() -> u64 {
    crate::vault::model::now_secs()
}

/// The key a box is remembered under: a digest of its canonical path, so two
/// spellings of one file are one entry — and so no side file writes down where
/// somebody keeps their boxes.
pub fn box_key(path: &str) -> String {
    let path = Path::new(path);
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    crate::vault::crypto::digest(canonical.to_string_lossy().as_bytes())
}

/// A key read back from a file written before keys were digests: those stored the
/// box's path itself. Re-keying it on the way in keeps what the entry holds — a
/// mirror, a wait — instead of losing it to the change.
pub(crate) fn adopt_key(key: &str) -> String {
    if key.contains('/') || key.contains('\\') {
        box_key(key)
    } else {
        key.to_owned()
    }
}

impl Prefs {
    /// Seconds the box at `key` stays refused, 0 when it may be tried now.
    pub fn wait(&self, key: &str, now: u64) -> u64 {
        self.tries
            .get(key)
            .map_or(0, |t| t.until.saturating_sub(now))
    }

    /// Record a failed unlock and return the seconds the box now waits. Every
    /// failure waits; reaching `max_attempts` starts the count over, so the box
    /// is refused for a while and never bricked.
    pub fn failed(&mut self, key: &str, now: u64) -> u64 {
        let t = self.tries.entry(key.to_owned()).or_default();
        t.fails += 1;
        if self.max_attempts > 0 && t.fails >= self.max_attempts {
            t.fails = 0;
        }
        t.until = now + self.retry_delay;
        self.retry_delay
    }

    /// A box was unlocked: its failures are forgotten. True when there was
    /// anything to forget, so a clean unlock does not rewrite the file.
    pub fn opened(&mut self, key: &str) -> bool {
        self.tries.remove(key).is_some()
    }
}

pub fn load() -> Prefs {
    read(&dir())
}

pub fn save(prefs: &Prefs) {
    write(&dir(), prefs);
}

fn dir() -> PathBuf {
    crate::vault::store::data_dir()
}

fn file(dir: &Path) -> PathBuf {
    dir.join("prefs.json")
}

/// A file older than the current fields keeps the defaults for whatever it is
/// missing (`#[serde(default)]` on the struct). A hand-edited file can only cost
/// the user a wait, so what it cannot be trusted with is clamped here.
fn read(dir: &Path) -> Prefs {
    let Ok(bytes) = fs::read(file(dir)) else {
        return Prefs::default();
    };
    let Ok(mut prefs) = serde_json::from_slice::<Prefs>(&bytes) else {
        // Keep the unreadable file beside itself: writing the defaults over it
        // would turn a version mismatch into lost settings.
        let _ = fs::rename(file(dir), file(dir).with_extension("bad"));
        return Prefs::default();
    };
    if prefs.lock_after == LEGACY_LOCK_AFTER {
        prefs.lock_after = DEFAULT_LOCK_AFTER;
    }
    prefs.retry_delay = prefs.retry_delay.min(MAX_RETRY_DELAY);
    prefs.clip_clear = prefs.clip_clear.min(MAX_CLIP_CLEAR);
    if prefs.max_attempts > 0 {
        prefs.retry_delay = prefs.retry_delay.max(MIN_RETRY);
    }
    // Every key in the file names a box by the digest of its path, never by the
    // path. A file from before that is re-keyed here and written back at once, so
    // the paths it carried do not outlive the first run of this build.
    let mut moved = false;
    fn rekey<T>(keys: HashMap<String, T>, moved: &mut bool) -> HashMap<String, T> {
        keys.into_iter()
            .map(|(k, v)| {
                let key = adopt_key(&k);
                *moved |= key != k;
                (key, v)
            })
            .collect()
    }
    prefs.tries = rekey(std::mem::take(&mut prefs.tries), &mut moved);
    prefs.remotes = rekey(std::mem::take(&mut prefs.remotes), &mut moved);
    if moved {
        write(dir, &prefs);
    }
    prefs
}

/// Losing the file only costs the settings, never a box.
fn write(dir: &Path, prefs: &Prefs) {
    let Ok(bytes) = serde_json::to_vec(prefs) else {
        return;
    };
    if fs::create_dir_all(dir).is_ok() {
        let _ = crate::vault::store::write_private(&file(dir), &bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pandora-prefs-{name}"));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn values_round_trip_and_the_default_survives_a_broken_file() {
        let dir = tmp("roundtrip");
        assert_eq!(
            read(&dir).lock_after,
            DEFAULT_LOCK_AFTER,
            "no file: the shipped 15 min"
        );
        assert_eq!(read(&dir).clip_clear, 30, "and the shipped clipboard grace");

        // An unreadable file is kept beside itself, not overwritten.
        fs::create_dir_all(&dir).unwrap();
        let broken = dir.join("prefs.json");
        fs::write(&broken, b"not json").unwrap();
        assert_eq!(read(&dir).lock_after, DEFAULT_LOCK_AFTER);
        assert!(
            dir.join("prefs.bad").exists(),
            "the unreadable file survives"
        );
        assert_eq!(
            fs::read_to_string(dir.join("prefs.bad")).unwrap(),
            "not json"
        );

        write(
            &dir,
            &Prefs {
                lock_after: 120,
                ..Default::default()
            },
        );
        assert_eq!(read(&dir).lock_after, 120);

        write(
            &dir,
            &Prefs {
                lock_after: 0,
                ..Default::default()
            },
        );
        assert_eq!(read(&dir).lock_after, 0, "0 disables idle locking");

        fs::write(dir.join("prefs.json"), b"not json").unwrap();
        assert_eq!(
            read(&dir).lock_after,
            DEFAULT_LOCK_AFTER,
            "unreadable: back to the default"
        );

        fs::write(dir.join("prefs.json"), br#"{"lock_after":30}"#).unwrap();
        assert_eq!(
            read(&dir).lock_after,
            DEFAULT_LOCK_AFTER,
            "migrates the old default"
        );
        assert_eq!(read(&dir).lang, Lang::En, "old prefs stay in English");

        write(
            &dir,
            &Prefs {
                lang: Lang::Es,
                ..Default::default()
            },
        );
        assert_eq!(read(&dir).lang, Lang::Es, "the selected language persists");
    }

    #[test]
    fn failures_wait_and_the_limit_starts_the_count_over() {
        let mut p = Prefs {
            max_attempts: 3,
            retry_delay: 10,
            ..Default::default()
        };
        let key = "box";

        assert_eq!(p.wait(key, 100), 0, "an untouched box is tryable");
        assert_eq!(p.failed(key, 100), 10, "every failure waits");
        assert_eq!(p.wait(key, 105), 5, "the wait is what is left");
        assert_eq!(p.wait(key, 110), 0, "and it runs out");
        assert_eq!(p.tries[key].fails, 1, "the failure is remembered");

        assert_eq!(p.failed(key, 200), 10);
        assert_eq!(p.tries[key].fails, 2);
        assert_eq!(p.failed(key, 300), 10, "the third failure hits the limit");
        assert_eq!(p.tries[key].fails, 0, "and starts the count over");
        assert_eq!(p.wait(key, 300), 10, "still refused for the full wait");

        assert!(p.opened(key), "a successful unlock has something to forget");
        assert!(!p.opened(key), "and nothing the second time");
        assert_eq!(p.wait(key, 300), 0);
    }

    #[test]
    fn no_limit_never_refuses() {
        let mut p = Prefs::default();
        for at in 0..5 {
            assert_eq!(p.failed("box", at), 0);
        }
        assert_eq!(p.wait("box", 4), 0);
    }

    #[test]
    fn a_hand_edited_file_cannot_brick_a_box() {
        let dir = tmp("clamp");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("prefs.json"),
            br#"{"lock_after":30,"max_attempts":3,"retry_delay":0}"#,
        )
        .unwrap();
        assert_eq!(read(&dir).retry_delay, MIN_RETRY, "a limit needs a wait");

        fs::write(
            dir.join("prefs.json"),
            br#"{"lock_after":30,"max_attempts":3,"retry_delay":99999999}"#,
        )
        .unwrap();
        assert_eq!(read(&dir).retry_delay, MAX_RETRY_DELAY);

        fs::write(dir.join("prefs.json"), br#"{"lock_after":30}"#).unwrap();
        let old = read(&dir);
        assert_eq!(old.max_attempts, 0, "an older file never refuses a box");
        assert_eq!(old.retry_delay, 0);
        assert!(old.tries.is_empty());
        assert!(
            old.autofill,
            "a file written before autofill existed reads as on"
        );

        fs::write(
            dir.join("prefs.json"),
            br#"{"lock_after":30,"clip_clear":99999999}"#,
        )
        .unwrap();
        assert_eq!(read(&dir).clip_clear, MAX_CLIP_CLEAR);
    }

    #[test]
    fn one_file_is_one_counter() {
        let dir = tmp("key");
        fs::create_dir_all(dir.join("sub")).unwrap();
        let file = dir.join("box.pandora");
        fs::write(&file, b"x").unwrap();

        assert_eq!(
            box_key(&file.display().to_string()),
            box_key(
                &dir.join("sub")
                    .join("..")
                    .join("box.pandora")
                    .display()
                    .to_string()
            ),
            "a detour through another directory is the same box"
        );
        assert_eq!(
            box_key("/nowhere/gone.pandora"),
            crate::vault::crypto::digest(b"/nowhere/gone.pandora"),
            "a path that cannot be resolved keys itself — as a digest, never as the path"
        );
        assert!(
            !box_key("/nowhere/gone.pandora").contains("nowhere"),
            "the key never spells the path"
        );
    }

    /// A file written before keys were digests is re-keyed on the way in: the
    /// mirror it recorded is still this box's, and the paths it spelled do not
    /// survive the first read.
    #[test]
    fn a_path_keyed_file_is_re_keyed_and_rewritten() {
        let dir = tmp("legacy-keys");
        let box_path = "/boxes/work.pandora";
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("prefs.json"),
            format!(
                r#"{{"lock_after":30,"tries":{{"{box_path}":{{"fails":2,"until":0}}}},"remotes":{{"{box_path}":{{"repo":"o/n","file":"work.pandora"}}}}}}"#
            ),
        )
        .unwrap();

        let key = box_key(box_path);
        let back = read(&dir);
        assert_eq!(back.tries[&key].fails, 2, "the counter survived");
        assert_eq!(back.remotes[&key].file, "work.pandora");

        let text = fs::read_to_string(dir.join("prefs.json")).unwrap();
        assert!(!text.contains("/boxes/"), "the path is gone: {text}");
    }

    /// What the files beside a box may say about it: nothing of the box's own.
    /// `prefs.json` and `biometric.json` key a box by the digest of its path and
    /// hold neither the path nor a word of what it is called. `recent.json` has to
    /// point at the file — that list is the way back into a box, and a digest
    /// opens nothing — but it calls the row after the file (`Recent::label`),
    /// never after the box. Written through the same writers the app uses.
    #[test]
    fn the_side_files_carry_no_path_or_name() {
        use crate::features::auth::biometric::{self, Sealed};
        use crate::features::auth::model as recents;

        let dir = tmp("side-files");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Divorcio-Laura.pandora");
        fs::write(&path, b"box").unwrap();
        let where_it_lives = path.display().to_string();
        let what_it_is_called = "Divorcio Laura";

        let key = box_key(&where_it_lives);
        assert!(!key.contains("Divorcio"), "{key}");

        let mut p = Prefs::default();
        p.tries.insert(key.clone(), Tries { fails: 1, until: 0 });
        p.remotes.insert(
            key.clone(),
            Remote {
                kind: GITHUB.to_owned(),
                repo: "octocat/boxes".to_owned(),
                file: "work.pandora".to_owned(),
                sha: String::new(),
            },
        );
        write(&dir, &p);
        recents::push(&dir, &path);
        let mut sealed = Sealed::default();
        sealed.boxes.insert(key.clone(), "ciphertext".to_owned());
        biometric::write(&dir, &sealed);

        // The two that only have to find a box again name it by the digest:
        // neither a path nor a word of what the box holds.
        for file in ["prefs.json", "biometric.json"] {
            let text = fs::read_to_string(dir.join(file)).unwrap();
            assert!(
                !text.contains(&where_it_lives),
                "{file} spells the box's path: {text}"
            );
            assert!(
                !text.contains(what_it_is_called),
                "{file} says what the box is called: {text}"
            );
            assert!(
                text.contains(&key),
                "{file} keys the box by its digest: {text}"
            );
        }

        // The recents have to point at the file — that list is the way back into
        // a box, and a digest opens nothing — but the row is called after the file
        // and the box's own name is never written down.
        let list = fs::read_to_string(dir.join("recent.json")).unwrap();
        assert!(
            list.contains(&where_it_lives),
            "the list still opens the box: {list}"
        );
        assert!(
            !list.contains(what_it_is_called) && !list.contains("\"name\""),
            "and carries no name of its own: {list}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// An older file leaves boxes unmirrored instead of failing to load.
    #[test]
    fn a_box_remembers_its_mirror() {
        let dir = tmp("remotes");
        let mut p = Prefs::default();
        p.sources.push(Source {
            kind: GITHUB.to_owned(),
            target: "octocat/boxes".to_owned(),
        });
        let key = box_key("/boxes/work.pandora");
        p.remotes.insert(
            key.clone(),
            Remote {
                kind: GITHUB.to_owned(),
                repo: "octocat/boxes".to_owned(),
                file: "work.pandora".to_owned(),
                sha: "abc".to_owned(),
            },
        );
        write(&dir, &p);

        let back = read(&dir);
        assert_eq!(back.sources, p.sources);
        assert_eq!(back.remotes[&key].file, "work.pandora");
        assert_eq!(back.remotes[&key].sha, "abc");
        fs::write(dir.join("prefs.json"), br#"{"lock_after":30}"#).unwrap();
        let old = read(&dir);
        assert!(old.remotes.is_empty(), "an older file mirrors nothing");
        assert!(old.sources.is_empty(), "and lives in no repo");
    }

    #[test]
    fn a_remote_without_a_kind_is_github() {
        let dir = tmp("kind");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("prefs.json"),
            br#"{"lock_after":30,"remotes":{"/b/x.pandora":{"repo":"o/n","file":"x.pandora"}}}"#,
        )
        .unwrap();
        // An older file, keyed by the box's path and with no `kind` at all.
        let key = box_key("/b/x.pandora");
        let back = read(&dir);
        assert_eq!(back.remotes[&key].kind, GITHUB);
        assert_eq!(back.remotes[&key].file, "x.pandora");
    }

    /// The remembered session is the default, not a choice the user has to find:
    /// a file older than the field keeps the token, and only an explicit "no"
    /// turns it off.
    #[test]
    fn keeping_the_token_is_on_unless_it_is_turned_off() {
        let dir = tmp("remember");
        fs::create_dir_all(&dir).unwrap();

        fs::write(dir.join("prefs.json"), br#"{"lock_after":30}"#).unwrap();
        assert!(read(&dir).remember, "an older file remembers the token");

        write(&dir, &Prefs::default());
        let written = fs::read(dir.join("prefs.json")).unwrap();
        assert!(
            !String::from_utf8(written).unwrap().contains("remember"),
            "the default is silence, which is now yes"
        );

        write(
            &dir,
            &Prefs {
                remember: false,
                ..Default::default()
            },
        );
        let written = fs::read(dir.join("prefs.json")).unwrap();
        assert!(
            String::from_utf8(written)
                .unwrap()
                .contains("\"remember\":false"),
            "an explicit no is written down"
        );
        assert!(!read(&dir).remember, "and it is what comes back");
    }

    /// The pane's last view and grouping ride along, so a fresh unlock lands where
    /// the user left off; an older file lands on the plain list, grouped by site.
    #[test]
    fn the_passwords_pane_remembers_how_it_last_drew_a_box() {
        let dir = tmp("view");
        fs::create_dir_all(&dir).unwrap();

        fs::write(dir.join("prefs.json"), br#"{"lock_after":30}"#).unwrap();
        let old = read(&dir);
        assert_eq!(old.view, View::List, "an older file opens on the list");
        assert_eq!(old.axis, Axis::Target, "and groups by site");

        write(
            &dir,
            &Prefs {
                view: View::Folders,
                axis: Axis::User,
                ..Default::default()
            },
        );
        let back = read(&dir);
        assert_eq!(back.view, View::Folders);
        assert_eq!(back.axis, Axis::User);
    }

    #[test]
    fn a_prefs_file_from_before_the_scheme_reads_as_dark() {
        let prefs: Prefs = serde_json::from_str(r#"{"lock_after":60}"#).unwrap();
        assert_eq!(prefs.theme, crate::shared::theme::Scheme::Dark);
        assert_eq!(prefs.lock_after, 60, "the rest of the file still reads");
    }

    #[test]
    fn the_scheme_is_written_as_a_word_and_comes_back() {
        let prefs = Prefs {
            theme: crate::shared::theme::Scheme::Light,
            ..Prefs::default()
        };
        let json = serde_json::to_string(&prefs).unwrap();
        assert!(json.contains(r#""theme":"light""#), "{json}");
        let back: Prefs = serde_json::from_str(&json).unwrap();
        assert_eq!(back.theme, crate::shared::theme::Scheme::Light);
    }
}
