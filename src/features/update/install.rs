//! Putting a verified download where the running build came from, and starting it.
//! Whether to install at all is `logic`'s business; this only knows how each
//! platform swaps its own file.

use std::path::{Path, PathBuf};

/// The directory the download lands in: the one holding the file about to be
/// replaced, so the final swap is a rename on a single filesystem.
pub fn staging_dir() -> Result<PathBuf, String> {
    platform::staging_dir()
}

/// Swap the staged file in and start it. `Ok(true)` means this process has to exit
/// for the new build to take over; `Ok(false)` means the platform installs it.
pub fn install(staged: &Path) -> Result<bool, String> {
    platform::install(staged)
}

/// Linux (and any other unix a checkout runs on): the AppImage replaces itself.
#[cfg(not(any(windows, target_os = "macos", target_os = "android")))]
mod platform {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// `$APPIMAGE` with its symlinks resolved, so the update replaces the file and
    /// not the link somebody put in their path.
    fn target() -> Result<PathBuf, String> {
        let path = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .ok_or("this build did not come from an AppImage")?;
        Ok(std::fs::canonicalize(&path).unwrap_or(path))
    }

    pub fn staging_dir() -> Result<PathBuf, String> {
        target()?
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "the AppImage has no directory".to_owned())
    }

    pub fn swap(staged: &Path, target: &Path) -> Result<(), String> {
        std::fs::set_permissions(staged, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("update: {e}"))?;
        std::fs::rename(staged, target).map_err(|e| format!("update: {e}"))
    }

    pub fn install(staged: &Path) -> Result<bool, String> {
        let target = target()?;
        swap(staged, &target)?;
        std::process::Command::new(&target)
            .spawn()
            .map_err(|e| format!("update: installed, reopen Pandora ({e})"))?;
        Ok(true)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn dir(tag: &str) -> PathBuf {
            let dir =
                std::env::temp_dir().join(format!("pandora-install-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        #[test]
        fn the_staged_file_replaces_the_target_and_stays_executable() {
            let dir = dir("swap");
            let target = dir.join("Pandora-0.1.0-x86_64.AppImage");
            std::fs::write(&target, b"old").unwrap();
            let staged = dir.join(".pandora-update-1");
            std::fs::write(&staged, b"new").unwrap();

            swap(&staged, &target).unwrap();

            assert_eq!(std::fs::read(&target).unwrap(), b"new");
            assert!(
                !staged.exists(),
                "the staging file is consumed by the rename"
            );
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_ne!(
                mode & 0o100,
                0,
                "an AppImage that is not executable does not start"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_target_that_cannot_be_replaced_leaves_nothing_behind() {
            let dir = dir("blocked");
            let missing = dir.join("not-a-file");
            std::fs::create_dir(&missing).unwrap();
            let staged = dir.join(".pandora-update-2");
            std::fs::write(&staged, b"new").unwrap();

            assert!(
                swap(&staged, &missing).is_err(),
                "a directory is not a file to rename over"
            );
            assert_eq!(
                std::fs::read(&staged).unwrap(),
                b"new",
                "the download survives"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// Windows: a running executable cannot be replaced, only renamed.
#[cfg(windows)]
mod platform {
    use super::*;

    fn target() -> Result<PathBuf, String> {
        std::env::current_exe().map_err(|e| format!("update: {e}"))
    }

    pub fn staging_dir() -> Result<PathBuf, String> {
        target()?
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "the executable has no directory".to_owned())
    }

    pub fn swap(staged: &Path, target: &Path) -> Result<(), String> {
        let old = target.with_extension("exe.old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(target, &old).map_err(|e| format!("update: {e}"))?;
        if let Err(e) = std::fs::rename(staged, target) {
            let _ = std::fs::rename(&old, target);
            return Err(format!("update: {e}"));
        }
        Ok(())
    }

    pub fn install(staged: &Path) -> Result<bool, String> {
        let target = target()?;
        swap(staged, &target)?;
        std::process::Command::new(&target)
            .spawn()
            .map_err(|e| format!("update: installed, reopen Pandora ({e})"))?;
        Ok(true)
    }
}

/// macOS: the `.app` bundle is what replaces itself, unpacked with `ditto` — the
/// archiver the system ships, and the one that keeps a bundle's own metadata.
#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::OsStr;

    fn target() -> Result<PathBuf, String> {
        let exe = std::env::current_exe().map_err(|e| format!("update: {e}"))?;
        exe.ancestors()
            .find(|path| path.extension() == Some(OsStr::new("app")))
            .map(Path::to_path_buf)
            .ok_or_else(|| "this build is not inside a .app bundle".to_owned())
    }

    pub fn staging_dir() -> Result<PathBuf, String> {
        target()?
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "the bundle has no directory".to_owned())
    }

    pub fn swap(staged: &Path, target: &Path) -> Result<(), String> {
        let dir = target
            .parent()
            .ok_or_else(|| "the bundle has no directory".to_owned())?;
        let unpacked = dir.join(".pandora-update");
        let _ = std::fs::remove_dir_all(&unpacked);
        let out = std::process::Command::new("/usr/bin/ditto")
            .arg("-x")
            .arg("-k")
            .arg(staged)
            .arg(&unpacked)
            .output()
            .map_err(|e| format!("update: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "update: unzip: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let fresh = std::fs::read_dir(&unpacked)
            .map_err(|e| format!("update: {e}"))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension() == Some(OsStr::new("app")))
            .ok_or_else(|| "the release holds no .app bundle".to_owned())?;

        // macOS keeps this process alive against the renamed bundle, so moving the
        // old one aside is safe where deleting it while it runs is not.
        let old = target.with_extension("app.old");
        let _ = std::fs::remove_dir_all(&old);
        std::fs::rename(target, &old).map_err(|e| format!("update: {e}"))?;
        if let Err(e) = std::fs::rename(&fresh, target) {
            let _ = std::fs::rename(&old, target);
            return Err(format!("update: {e}"));
        }
        let _ = std::fs::remove_dir_all(&unpacked);
        Ok(())
    }

    pub fn install(staged: &Path) -> Result<bool, String> {
        let target = target()?;
        swap(staged, &target)?;
        std::process::Command::new("/usr/bin/open")
            .arg("-n")
            .arg(&target)
            .spawn()
            .map_err(|e| format!("update: installed, reopen Pandora ({e})"))?;
        Ok(true)
    }
}

/// Android: the system's installer owns the swap. All this does is stage the APK
/// where the FileProvider declares it and hand over the path.
#[cfg(target_os = "android")]
mod platform {
    use super::*;

    pub fn staging_dir() -> Result<PathBuf, String> {
        Ok(crate::vault::store::data_dir().join("updates"))
    }

    pub fn install(staged: &Path) -> Result<bool, String> {
        let apk = staging_dir()?.join("pandora.apk");
        if staged != apk {
            std::fs::rename(staged, &apk).map_err(|e| format!("update: {e}"))?;
        }
        crate::shared::android::send("installApk", &apk.display().to_string())?;
        Ok(false)
    }
}
