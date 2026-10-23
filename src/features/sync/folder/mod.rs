//! A folder mirror is one encrypted file, written in place and guarded by the
//! provider's own stamp. There is no version to file, no name to follow and no
//! update window: the folder is the transport, and its history is the provider's.
//!
//! Writing in place means a write cut in half leaves a truncated remote until the
//! next save. That is the trade a single-file mirror makes; the provider's history
//! is what recovers it.

use std::time::SystemTime;

pub mod local;
pub mod logic;

#[cfg(target_os = "android")]
pub mod saf;

/// The one file a folder mirror reads and writes.
pub const NAME: &str = "vault.pandora";

/// What a guarded write did.
pub enum Write {
    /// The bytes landed.
    Filed,
    /// The file was not where this box left it; nothing was written.
    Moved,
}

/// What a write expects to find before it lands.
pub enum Guard {
    /// The folder holds no file yet.
    Missing,
    /// The file still carries this provider stamp.
    Stamp(u64),
    /// The provider cannot stamp, so the bytes just read are the only check.
    Unguarded,
}

pub trait Folder {
    /// The file's bytes, or `None` when it is not there.
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, String>;

    /// The provider's stamp in Unix milliseconds, or `None` when the file is not
    /// there or the provider cannot say.
    fn stamp(&self, name: &str) -> Result<Option<u64>, String>;

    /// Write in place under the guard the probe earned.
    fn write(&self, name: &str, bytes: &[u8], guard: Guard) -> Result<Write, String>;

    /// Whether the provider lets this app write the folder. Optimistic when the
    /// platform does not say.
    fn writable(&self) -> Result<bool, String>;

    /// Every name in the folder. Only the legacy cleanup reads it.
    fn list(&self) -> Result<Vec<String>, String>;

    /// Drop a name. Only the legacy cleanup calls it.
    fn remove(&self, name: &str) -> Result<(), String>;
}

/// Whether `name` is one the old versioned protocol filed. Those files are
/// deleted once, when a folder from an older build is first attached.
pub fn legacy(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 23
        && bytes[..6].iter().all(u8::is_ascii_digit)
        && bytes[6] == b'-'
        && bytes[7..15].iter().all(u8::is_ascii_hexdigit)
        && &bytes[15..] == b".pandora"
}

/// The Unix milliseconds a filesystem stamp names.
pub fn millis(stamp: SystemTime) -> Option<u64> {
    stamp
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_legacy_version_is_recognised_and_nothing_else_is() {
        assert!(legacy("000004-deadbeef.pandora"));
        assert!(!legacy("vault.pandora"));
        assert!(!legacy("000004-deadbeef.pandora.bak"));
        assert!(!legacy("00004-deadbeef.pandora"));
        assert!(!legacy("000004-deadbee.pandora"));
        assert!(!legacy("000004-deadbeeg.pandora"));
    }
}
