//! The folder mirror on a disk this machine can see: a directory the user picked,
//! or one a Syncthing folder and a mounted WebDAV already keep in step. The
//! mirror is the directory, so nothing here knows how the bytes travel.

use crate::features::sync::folder::{millis, Folder, Guard, Write};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write as _};
use std::path::PathBuf;

/// A directory holding a mirror's one file.
pub struct Local {
    root: PathBuf,
}

impl Local {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn at(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Folder for Local {
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        let path = self.at(name);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn stamp(&self, name: &str) -> Result<Option<u64>, String> {
        let path = self.at(name);
        match fs::metadata(&path) {
            Ok(meta) => Ok(meta.modified().ok().and_then(millis)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn write(&self, name: &str, bytes: &[u8], guard: Guard) -> Result<Write, String> {
        let path = self.at(name);
        let now = match fs::metadata(&path) {
            Ok(meta) => meta.modified().ok().and_then(millis),
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut options = OpenOptions::new();
        options.write(true);
        match guard {
            Guard::Missing if now.is_some() => return Ok(Write::Moved),
            Guard::Missing => {
                options.create_new(true);
            }
            Guard::Stamp(stamp) if now != Some(stamp) => return Ok(Write::Moved),
            Guard::Stamp(_) => {
                options.truncate(true);
            }
            Guard::Unguarded => {
                options.create(true).truncate(true);
            }
        }
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::AlreadyExists) => {
                return Ok(Write::Moved);
            }
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Write::Filed)
    }

    fn writable(&self) -> Result<bool, String> {
        let path = self.at(crate::features::sync::folder::NAME);
        let meta = fs::metadata(&path).or_else(|_| fs::metadata(&self.root));
        Ok(!meta
            .map_err(|e| format!("{}: {e}", self.root.display()))?
            .permissions()
            .readonly())
    }

    fn list(&self) -> Result<Vec<String>, String> {
        let listing =
            fs::read_dir(&self.root).map_err(|e| format!("{}: {e}", self.root.display()))?;
        let mut names = Vec::new();
        for entry in listing {
            let entry = entry.map_err(|e| e.to_string())?;
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
        Ok(names)
    }

    fn remove(&self, name: &str) -> Result<(), String> {
        let path = self.at(name);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}
