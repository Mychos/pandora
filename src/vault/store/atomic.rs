//! Every file this crate writes: sibling tmp, fsync, rename, fsync dir, mode 0600.

use crate::vault::store::data_dir;
use crate::vault::store::EXT;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

/// The one way this crate writes a box file: sibling tmp, fsync, rename, fsync dir.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = parent_of(path);
    fs::create_dir_all(dir).map_err(|e| format!("cannot create dir: {e}"))?;

    // A random tmp name: a fixed one can be planted as a symlink and written through.
    let tmp = path.with_extension(format!("{EXT}.{}.tmp", uuid::Uuid::new_v4()));
    let mut f = private_file(&tmp).map_err(|e| format!("cannot write: {e}"))?;
    f.write_all(bytes)
        .map_err(|e| format!("cannot write: {e}"))?;
    f.sync_all().map_err(|e| format!("fsync: {e}"))?;
    drop(f);
    fs::rename(&tmp, path).map_err(|e| format!("cannot finalize: {e}"))?;
    sync_dir(dir)
}

/// Keep the version a box is about to lose in one slot beside it, overwritten each
/// time: a copy, not a link, and never a growing history.
pub fn stash(path: &Path) {
    if !path.is_file() {
        return;
    }
    let backup = path.with_extension(format!("{EXT}.bak"));
    let _ = fs::remove_file(&backup);
    let _ = copy_private(path, &backup);
}

/// `fs::copy` would inherit the box's mode, so a file left at 0644 would leave a
/// world-readable copy of the same secret; 0600 from creation, plus the fsync a stash needs.
pub(super) fn copy_private(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut input = fs::File::open(from)?;
    let mut out = private_file(to)?;
    std::io::copy(&mut input, &mut out)?;
    out.sync_all()
}

/// Drop the stash. A password change owes it: what sits there is sealed with the
/// old key. A deleted box owes it too, since the copy is a version of that box.
pub fn forget_stash(path: &Path) {
    let _ = fs::remove_file(path.with_extension(format!("{EXT}.bak")));
}

/// Every other file the app writes — prefs, recents, an export — goes out private too.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = private_file(path).map_err(|e| format!("cannot write: {e}"))?;
    f.write_all(bytes).map_err(|e| format!("cannot write: {e}"))
}

/// A private file for a streaming export, 0600 from creation like [`write_private`].
pub fn private_sink(path: &Path) -> Result<fs::File, String> {
    private_file(path).map_err(|e| format!("cannot write: {e}"))
}

/// Drop the plaintext copies a phone's picker staged for a save that never happened.
pub fn clear_staging() {
    let _ = fs::remove_dir_all(data_dir().join("staging"));
}

/// Readable and writable by its owner alone, so nobody else reads a box or an export.
#[cfg(unix)]
pub(super) fn private_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // `mode` only bites on creation, so a file an older build left at 0644 is fixed here.
    f.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(f)
}

#[cfg(not(unix))]
pub(super) fn private_file(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

pub(super) fn parent_of(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Windows cannot open a directory for sync, so the guarantee is unix-only.
#[cfg(unix)]
pub(super) fn sync_dir(dir: &Path) -> Result<(), String> {
    fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| format!("fsync dir: {e}"))
}

#[cfg(not(unix))]
pub(super) fn sync_dir(_dir: &Path) -> Result<(), String> {
    Ok(())
}

pub fn modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok().and_then(|m| m.modified().ok())
}
