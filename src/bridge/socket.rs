//! The bridge's channel: a Unix socket or a Windows named pipe, never a TCP port.
//! `PANDORA_BRIDGE_SOCKET` names it; tests use it to get a private channel.
//! [`crate::shared::socket`] owns the two platform implementations; this module only
//! says where the bridge's own channel lives.

use std::io;
use std::path::PathBuf;

pub use crate::shared::socket::{connect_at, Listener, Stream};

/// `PANDORA_BRIDGE_SOCKET` wins; otherwise a directory of our own under
/// `XDG_RUNTIME_DIR` (per-user, off disk, cleared at logout), else the app's data dir.
/// The directory matters: a flatpak sandbox can only be granted the directory, since
/// a grant on the socket file holds the inode the app has already replaced.
pub fn path() -> PathBuf {
    if let Some(path) = std::env::var_os("PANDORA_BRIDGE_SOCKET").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(base) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(base).join("pandora").join("bridge.sock");
    }
    crate::vault::store::data_dir().join("pandora-bridge.sock")
}

/// Fails when nothing is listening, which callers read as "no bridge, answer it yourself".
pub fn connect() -> io::Result<Stream> {
    let refused = connect_at(&path());
    // An app built before the socket moved still listens under the flat name.
    #[cfg(unix)]
    if refused.is_err() && std::env::var_os("PANDORA_BRIDGE_SOCKET").is_none() {
        if let Some(base) = std::env::var_os("XDG_RUNTIME_DIR") {
            return connect_at(&PathBuf::from(base).join("pandora-bridge.sock"));
        }
    }
    refused
}
