//! Starting the app when a browser asks and nothing is listening. The host may open
//! the window, but only the window opens a box — so it starts it, and the panel that
//! asked waits on `status` instead of being told to go somewhere else.

use crate::bridge::protocol::Response;
use crate::bridge::socket;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// The binary's name, as `cargo` and a packaged install spell it.
const APP: &str = if cfg!(windows) {
    "pandora.exe"
} else {
    "pandora"
};

/// Start the app unless it is already listening. `Ok` does not mean unlocked: the
/// window is where a box gets opened, and the caller polls `status` for it.
pub fn open() -> Response {
    if socket::connect().is_ok() {
        return Response::Ok;
    }
    // A flatpak browser keeps the app on the other side of the sandbox wall.
    if in_sandbox(std::env::var_os("FLATPAK_ID").as_deref()) {
        return on_host();
    }
    let Some(app) = resolve() else {
        return Response::Error {
            message: "Pandora is not installed — open a box in the app.".to_owned(),
        };
    };
    start(&app)
}

/// `PANDORA_APP` first, then the directory the host was installed in, then `PATH`: a
/// development build keeps both binaries in `target/release`, and a packaged one is
/// expected to do the same.
fn resolve() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("PANDORA_APP").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(explicit)).filter(|path| is_app(path));
    }
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(APP)));
    let path = std::env::var_os("PATH").unwrap_or_default();
    find(
        sibling
            .into_iter()
            .chain(std::env::split_paths(&path).map(|dir| dir.join(APP))),
    )
}

/// The first candidate that is an executable file. Split out so the order is testable
/// without an install or a window.
fn find(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|path| is_app(path))
}

/// A flatpak app is told its own id and nothing of the host's filesystem.
fn in_sandbox(id: Option<&std::ffi::OsStr>) -> bool {
    id.is_some_and(|id| !id.is_empty())
}

/// A flatpak browser cannot start the app itself, and the path to it does not exist on
/// this side of the wall. `install.sh` names the host-side binary in `PANDORA_APP` and
/// grants `org.freedesktop.Flatpak` so the sandbox may ask the session to run it.
fn on_host() -> Response {
    let app = std::env::var_os("PANDORA_APP")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| APP.into());
    let spawned = Command::new("flatpak-spawn")
        .arg("--host")
        .arg(&app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(_) => Response::Ok,
        Err(e) => {
            log::warn!(
                "pandora: could not ask the host to start {}: {e}",
                std::path::Path::new(&app).display()
            );
            Response::Error {
                message: "Pandora could not be started.".to_owned(),
            }
        }
    }
}

/// Start one process of our own, with nothing of the host's leaking into it: stdout is
/// the browser's channel, and a child holding it would corrupt the protocol.
fn start(app: &std::path::Path) -> Response {
    let spawned = Command::new(app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(_) => Response::Ok,
        Err(e) => {
            log::warn!("pandora: could not start {}: {e}", app.display());
            Response::Error {
                message: "Pandora could not be started.".to_owned(),
            }
        }
    }
}

#[cfg(unix)]
fn is_app(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && std::fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_app(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_candidate_that_is_an_executable_wins() {
        let dir = std::env::temp_dir().join(format!("pandora-launch-{}", uuid::Uuid::new_v4()));
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let app = dir.join(APP);
        std::fs::write(&app, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let missing = empty.join(APP);
        assert_eq!(
            find([missing.clone(), app.clone()]),
            Some(app.clone()),
            "a directory without the binary is skipped"
        );
        assert_eq!(find([missing]), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A flatpak sets the id; nothing else does, and an empty one is not a sandbox.
    #[test]
    fn only_a_flatpak_id_means_we_are_sandboxed() {
        use std::ffi::OsStr;
        assert!(in_sandbox(Some(OsStr::new("app.zen_browser.zen"))));
        assert!(!in_sandbox(Some(OsStr::new(""))));
        assert!(!in_sandbox(None));
    }
}
