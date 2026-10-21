//! The v4 file: fixed header, chunked records, authenticated manifest. The public
//! side of it — paths, credentials, open and save — is here; the header contract is
//! in `header`, the atomic file writes in `atomic`, and the chunked writer in `write`.

mod atomic;
mod header;
mod write;

#[cfg(test)]
mod tests;

pub use atomic::{
    clear_staging, forget_stash, modified_time, private_sink, stash, write_atomic, write_private,
};
pub use header::KdfHeader;
pub use write::save;

use crate::vault::codec;
use crate::vault::crypto::{self, KdfParams, Key};
use crate::vault::model::Vault;
use crate::vault::Failure;
use atomic::{parent_of, sync_dir};
use header::{decrypt_manifest, header, read_header, V4Header};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use zeroize::Zeroizing;

pub const CURRENT_VERSION: u32 = 4;
pub const EXT: &str = "pandora";

const KEYFILE_MAX: u64 = 8 * 1024 * 1024;

/// The derived master key plus the header needed to re-encrypt on save.
pub struct Credentials {
    pub key: Key,
    header: KdfHeader,
    /// `Some` when the KDF proved whether a keyfile was used; `None` when this
    /// session ran none (biometric unlock). The file itself never records it.
    keyfile: Option<bool>,
}

impl Credentials {
    pub fn params(&self) -> KdfParams {
        KdfParams {
            m_cost: self.header.m_cost,
            t_cost: self.header.t_cost,
            p_cost: self.header.p_cost,
        }
    }

    pub fn salt_bytes(&self) -> Vec<u8> {
        codec::decode(&self.header.salt).unwrap_or_default()
    }

    pub fn needs_keyfile(&self) -> Option<bool> {
        self.keyfile
    }

    pub fn rekey(password: &str, keyfile: Option<&[u8]>) -> Result<Credentials, String> {
        let salt = crypto::random_bytes::<{ crypto::SALT_LEN }>();
        let params = KdfParams::default();
        let secret = crypto::material(password, keyfile);
        let key = crypto::derive_key(&secret, &salt, &params)?;
        Ok(Credentials {
            key,
            header: header(&params, &salt),
            keyfile: Some(keyfile.is_some()),
        })
    }
}

/// Where prefs, recents and the default box live. Android has no `dirs`: it hands
/// the app its private directory from `android_main`.
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

pub fn data_dir() -> PathBuf {
    if let Some(dir) = DATA_DIR.get() {
        return dir.clone();
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pandora")
}

pub fn default_path() -> PathBuf {
    data_dir().join(format!("vault.{EXT}"))
}

/// One working copy per box, named by a digest of the source: a folder called
/// `sources/github/octocat/boxes` would say which repo somebody mirrors to.
pub fn source_dir(kind: &str, target: &str) -> PathBuf {
    let dir = data_dir().join("sources").join(crypto::digest(
        format!("{}|{target}", sanitize(kind)).as_bytes(),
    ));
    // The old layout's folder is moved on first look: a copy that never reached the repo
    // is the only copy.
    let legacy = legacy_source_dir(kind, target);
    if !dir.exists() && legacy.is_dir() {
        if let Some(parent) = dir.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::rename(&legacy, &dir);
    }
    dir
}

/// The folder name an older build derived: `sources/<kind>/<owner>/<repo>`.
fn legacy_source_dir(kind: &str, target: &str) -> PathBuf {
    let mut dir = data_dir().join("sources").join(sanitize(kind));
    for segment in target.split('/').filter(|s| !s.is_empty()) {
        dir.push(sanitize(segment));
    }
    dir
}

/// A name that survives a filesystem. A source can be hand-edited, so a `..`
/// segment must never walk out of the boxes dir; an empty name gets a placeholder.
pub(crate) fn sanitize(segment: &str) -> String {
    let cleaned: String = segment
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        return "_".to_owned();
    }
    cleaned
}

/// Refuses an empty keyfile and one larger than [`KEYFILE_MAX`].
pub fn read_keyfile(path: &Path) -> Result<Zeroizing<Vec<u8>>, String> {
    let len = fs::metadata(path)
        .map_err(|e| format!("cannot read keyfile: {e}"))?
        .len();
    if len == 0 {
        return Err("keyfile is empty".to_owned());
    }
    if len > KEYFILE_MAX {
        return Err("keyfile is larger than 8 MB".to_owned());
    }
    fs::read(path)
        .map(Zeroizing::new)
        .map_err(|e| format!("cannot read keyfile: {e}"))
}

pub fn create(
    path: &Path,
    name: &str,
    password: &str,
    keyfile: Option<&[u8]>,
) -> Result<(Credentials, Vault), String> {
    let salt = crypto::random_bytes::<{ crypto::SALT_LEN }>();
    let params = KdfParams::default();
    let secret = crypto::material(password, keyfile);
    let key = crypto::derive_key(&secret, &salt, &params)?;
    let mut vault = Vault::new(name);
    let creds = Credentials {
        key,
        header: header(&params, &salt),
        keyfile: Some(keyfile.is_some()),
    };
    save(path, &mut vault, &creds)?;
    Ok((creds, vault))
}

/// The v4 header of `path`, or an error when the file is missing or not v4.
fn require_header(path: &Path) -> Result<V4Header, String> {
    match read_header(path)? {
        Some(header) => Ok(header),
        None if path.exists() => Err("not a Pandora v4 box".to_owned()),
        None => Err(format!("cannot read box: {} not found", path.display())),
    }
}

/// Open with the master password. The file does not say whether a keyfile joined
/// the key — that would tell an attacker which way to go — so the KDF tries the
/// keyfile first when handed one, then the password alone, and the AEAD tag decides.
pub fn open(
    path: &Path,
    password: &str,
    keyfile: Option<&[u8]>,
) -> Result<(Credentials, Vault), String> {
    let header = require_header(path)?;
    let salt = codec::decode(&header.kdf.salt)?;
    let params = KdfParams {
        m_cost: header.kdf.m_cost,
        t_cost: header.kdf.t_cost,
        p_cost: header.kdf.p_cost,
    };
    // One derivation in the common case: keyfile first when handed over, password alone last.
    let mut attempts = Vec::with_capacity(2);
    if keyfile.is_some() {
        attempts.push(keyfile);
    }
    attempts.push(None);
    for with in attempts {
        let secret = crypto::material(password, with);
        let key = crypto::derive_key(&secret, &salt, &params)?;
        if let Ok(vault) = decrypt_manifest(path, &header, &key) {
            return Ok((
                Credentials {
                    key,
                    header: header.kdf,
                    keyfile: Some(with.is_some()),
                },
                vault,
            ));
        }
    }
    Err(Failure::Wrong.to_string())
}

/// Open with a key derived earlier, for biometric unlock: no KDF runs, and the
/// AEAD tag is the whole check, so a rotated key simply fails to decrypt.
pub fn open_keyed(path: &Path, key: Key) -> Result<(Credentials, Vault), String> {
    let header = require_header(path)?;
    let vault = decrypt_manifest(path, &header, &key)?;
    Ok((
        Credentials {
            key,
            header: header.kdf,
            keyfile: None,
        },
        vault,
    ))
}

/// Reload the vault from disk using an already-derived key (no KDF run).
pub fn load(path: &Path, creds: &Credentials) -> Result<Vault, String> {
    let header = require_header(path)?;
    decrypt_manifest(path, &header, &creds.key)
}

/// Constant-time AEAD check: the safe way to test a master password.
pub fn verify(path: &Path, key: &Key) -> bool {
    match read_header(path) {
        Ok(Some(header)) => decrypt_manifest(path, &header, key).is_ok(),
        _ => false,
    }
}

/// Bytes must open with `creds` before replacing the file, so a foreign box or a
/// re-keyed one cannot eat the local copy. `floor` is the generation a repo copy may
/// not go behind; a stash restore passes `None`.
pub fn adopt(
    path: &Path,
    bytes: &[u8],
    creds: &Credentials,
    floor: Option<u64>,
) -> Result<Vault, String> {
    let candidate = path.with_extension(format!("{EXT}.candidate"));
    write_atomic(&candidate, bytes)?;
    let vault = match load(&candidate, creds) {
        Ok(v) => v,
        Err(e) => {
            let _ = fs::remove_file(&candidate);
            return Err(e);
        }
    };
    if floor.is_some_and(|floor| vault.seq < floor) {
        let _ = fs::remove_file(&candidate);
        return Err(Failure::RolledBack.to_string());
    }
    let dir = parent_of(path);
    stash(path);
    fs::rename(&candidate, path).map_err(|e| format!("cannot finalize: {e}"))?;
    sync_dir(dir)?;
    // The candidate's reader names a file that no longer exists; reload from the final path.
    load(path, creds)
}
