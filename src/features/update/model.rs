//! Whether a newer Pandora exists, and the state of the download that follows.

use std::sync::mpsc;

/// Where a released Pandora lives. Renaming the repository renames it here, in
/// `github.rs`, and in the notice the user clicks.
pub const REPO: &str = "Pandora-Secrets/pandora";

pub const RELEASES: &str = "https://github.com/Pandora-Secrets/pandora/releases";

/// The build this process is, as `Cargo.toml` writes it.
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// A release newer than this build. `installable` is whether the artifact for
/// this platform exists at all: a build from a package manager can only be told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    pub latest: String,
    pub url: String,
    pub installable: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub written: u64,
    pub total: Option<u64>,
}

impl Progress {
    /// Percent, or nothing while the server has not said how large the file is.
    pub fn percent(&self) -> Option<u64> {
        let total = self.total.filter(|total| *total > 0)?;
        Some(self.written.min(total) * 100 / total)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    AppImage,
    Windows,
    MacOs,
    Android,
    /// A binary from a package manager or a checkout: it can be told, not replaced.
    External,
}

#[cfg(target_os = "android")]
pub fn channel() -> Channel {
    Channel::Android
}

#[cfg(windows)]
pub fn channel() -> Channel {
    Channel::Windows
}

#[cfg(target_os = "macos")]
pub fn channel() -> Channel {
    Channel::MacOs
}

#[cfg(not(any(windows, target_os = "macos", target_os = "android")))]
pub fn channel() -> Channel {
    if std::env::var_os("APPIMAGE").is_some() {
        Channel::AppImage
    } else {
        Channel::External
    }
}

/// What a worker sends back to the frame that started it.
#[derive(Debug)]
pub enum Event {
    /// The newest release when it is newer than this build, `None` when it is not.
    Checked(Option<Info>),
    Progress(Progress),
    /// Staged and swapped. `restart` is whether this process has to exit for the
    /// new build to take over; a phone's installer does that itself.
    Installed {
        restart: bool,
    },
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Checking,
    Downloading(Progress),
    Failed(String),
}

/// The updater as the frame sees it: what it learned, and the one worker allowed
/// to be running.
pub struct Update {
    pub state: State,
    /// The newer release, once a check has found one.
    pub info: Option<Info>,
    /// A check has finished this run, so "up to date" is an answer and not a wait.
    pub checked: bool,
    pub(super) rx: Option<mpsc::Receiver<Event>>,
    /// Focus as the frame before this one saw it: `None` until the first frame,
    /// which is not a window coming back.
    pub(super) focused: Option<bool>,
}

impl Default for Update {
    fn default() -> Self {
        Self {
            state: State::Idle,
            info: None,
            checked: false,
            rx: None,
            // The first look is `App::new`'s; the first frame is not a regain.
            focused: None,
        }
    }
}

impl Update {
    /// Whether a worker owns the updater: a second check would only queue behind it.
    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Start a worker: its receiver stays here and its sender goes to the thread.
    /// Compiled out of a test build, which has no worker to start.
    #[cfg(not(test))]
    pub(super) fn open(&mut self) -> mpsc::Sender<Event> {
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        tx
    }
}

/// The numeric triple a tag carries: `v1.2.3`, `1.2.3-rc1` and `1.2.3+4` all read
/// as `[1, 2, 3]`. Anything else is not a version this can compare.
pub fn parse(tag: &str) -> Option<[u32; 3]> {
    let tag = tag.trim().trim_start_matches('v');
    let tag = tag.split(['-', '+']).next()?;
    let mut out = [0u32; 3];
    let mut parts = tag.split('.');
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    // A fourth part is a version this does not understand, not one it can guess at.
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

/// Whether `latest` is a version after `current`. Equal or older is not an update:
/// a re-uploaded release must never turn into a downgrade.
pub fn newer(latest: &str, current: &str) -> bool {
    match (parse(latest), parse(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_reads_as_its_numbers() {
        assert_eq!(parse("v1.2.3"), Some([1, 2, 3]));
        assert_eq!(parse("1.2.3"), Some([1, 2, 3]));
        assert_eq!(parse("0.10.0-rc1"), Some([0, 10, 0]));
        assert_eq!(parse("2.0.0+build7"), Some([2, 0, 0]));
        assert_eq!(parse(" 1.2.3 "), Some([1, 2, 3]));
        for bad in ["", "v", "1.2", "1.2.3.4", "nightly", "1.2.x"] {
            assert_eq!(parse(bad), None, "{bad:?} is not a version");
        }
    }

    #[test]
    fn only_a_later_version_is_an_update() {
        assert!(newer("0.1.1", "0.1.0"));
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("0.1.0", "0.1.0"), "the same build is not an update");
        assert!(
            !newer("0.0.9", "0.1.0"),
            "an older release is not an update"
        );
        assert!(
            !newer("nightly", "0.1.0"),
            "an unreadable tag is not an update"
        );
    }

    #[test]
    fn progress_is_a_percentage_only_with_a_total() {
        let known = Progress {
            written: 500,
            total: Some(2000),
        };
        assert_eq!(known.percent(), Some(25));
        let unknown = Progress {
            written: 500,
            total: None,
        };
        assert_eq!(unknown.percent(), None);
        let overrun = Progress {
            written: 3000,
            total: Some(2000),
        };
        assert_eq!(
            overrun.percent(),
            Some(100),
            "a lied-about size cannot pass 100"
        );
    }
}
