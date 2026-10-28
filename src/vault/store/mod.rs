//! The v4 file: fixed header, chunked records, authenticated manifest. The public
//! side of it — paths, credentials, open and save — is here; the header contract is
//! in `header`, the atomic file writes in `atomic`, and the chunked writer in `write`.

mod atomic;
mod header;
mod write;

#[cfg(test)]
mod tests;

pub use atomic::{
    clear_legacy_stash, clear_orphan_tmps, clear_staging, forget_stash, modified_time,
    private_sink, stash, stash_path, write_atomic, write_private,
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

pub const CURRENT_VERSION: u32 = 5;

/// A second factor that answers a challenge: the YubiKey's HMAC-SHA1 response. It is a
/// parameter rather than a call so a test can derive with an answer no hardware gives.
///
/// One open asks it exactly once — the answer is reused for every combination the box
/// could have been written with — so a key that waits for a finger waits once.
pub trait Factor {
    /// The response to one challenge, blocking while the key waits to be touched.
    fn respond(&self, challenge: &[u8]) -> Result<Vec<u8>, String>;
}
pub const EXT: &str = "pandora";

const KEYFILE_MAX: u64 = 8 * 1024 * 1024;

/// The derived master key plus the header needed to re-encrypt on save.
#[derive(Clone)]
pub struct Credentials {
    pub key: Key,
    header: KdfHeader,
    /// The recovery slot this box carries, kept across saves; a rekey drops it.
    pub recovery: Option<crate::vault::recovery::Slot>,
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
        Self::rekey_with(password, keyfile, None)
    }

    /// Derive a new key from the password and whatever factors are enrolled. The order
    /// is the contract: the keyfile's digest, then the key's response. A box enrolled
    /// with a key and opened with one is derived the same way both times.
    pub fn rekey_with(
        password: &str,
        keyfile: Option<&[u8]>,
        response: Option<&[u8]>,
    ) -> Result<Credentials, String> {
        let salt = crypto::random_bytes::<{ crypto::SALT_LEN }>();
        let params = KdfParams::default();
        let secret = crypto::material(password, &secrets(keyfile, response));
        let key = crypto::derive_key(&secret, &salt, &params)?;
        Ok(Credentials {
            key,
            header: header(&params, &salt),
            recovery: None,
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
    let secret = crypto::keyed(password, keyfile);
    let key = crypto::derive_key(&secret, &salt, &params)?;
    let mut vault = Vault::new(name);
    let creds = Credentials {
        key,
        header: header(&params, &salt),
        recovery: None,
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
    clear_legacy_stash(path);
    open_with(path, password, keyfile, None)
}

/// The box's own id, which its header carries in the clear and every copy keeps.
pub fn box_id(path: &Path) -> Result<String, String> {
    Ok(require_header(path)?.box_id)
}

/// The challenge this box asks its key for: derived from the id the plaintext header
/// carries, so every copy of the file asks the same question and nothing new is
/// written down for the key.
pub fn challenge(path: &Path) -> Result<Vec<u8>, String> {
    let header = require_header(path)?;
    Ok(crate::shared::yubikey::challenge(&header.box_id))
}

/// The same, with a key that answers a challenge. The challenge comes from the box's
/// own id, so nothing has to be stored for it, and a copy of the box asks the same
/// question. The factor is asked once; every combination it can form is then derived
/// until the AEAD tag says which one the box was written with — the box never says.
pub fn open_with(
    path: &Path,
    password: &str,
    keyfile: Option<&[u8]>,
    factor: Option<&dyn Factor>,
) -> Result<(Credentials, Vault), String> {
    clear_legacy_stash(path);
    let header = require_header(path)?;
    let salt = codec::decode(&header.kdf.salt)?;
    let params = KdfParams {
        m_cost: header.kdf.m_cost,
        t_cost: header.kdf.t_cost,
        p_cost: header.kdf.p_cost,
    };
    // The password first: a box with no key opens without touching one, and a key
    // plugged in for something else costs a frame rather than a touch.
    for (attempt, used_keyfile) in attempts(keyfile, None) {
        if let Some(found) = derive_into(
            path,
            &header,
            &salt,
            &params,
            password,
            &attempt,
            used_keyfile,
        ) {
            return Ok(found);
        }
    }
    // Then the key is asked — once — and the combinations it forms are tried.
    let Some(factor) = factor else {
        return Err(Failure::Wrong.to_string());
    };
    let response = factor.respond(&crate::shared::yubikey::challenge(&header.box_id))?;
    for (attempt, used_keyfile) in with_response(keyfile, &response) {
        if let Some(found) = derive_into(
            path,
            &header,
            &salt,
            &params,
            password,
            &attempt,
            used_keyfile,
        ) {
            return Ok(found);
        }
    }
    Err(Failure::Wrong.to_string())
}

/// One derivation and the one question it answers: does this material open the box?
#[allow(clippy::too_many_arguments)] // the derivation's own inputs, already read
fn derive_into(
    path: &Path,
    header: &V4Header,
    salt: &[u8],
    params: &KdfParams,
    password: &str,
    attempt: &[&[u8]],
    used_keyfile: bool,
) -> Option<(Credentials, Vault)> {
    let secret = crypto::material(password, attempt);
    let key = crypto::derive_key(&secret, salt, params).ok()?;
    let vault = decrypt_manifest(path, header, &key).ok()?;
    Some((
        Credentials {
            key,
            header: header.kdf.clone(),
            recovery: header.recovery.clone(),
            keyfile: Some(used_keyfile),
        },
        vault,
    ))
}

/// The material a box may have been written with when it never enrolled a key: the
/// keyfile, then the plain password.
fn attempts<'a>(
    keyfile: Option<&'a [u8]>,
    _response: Option<&'a [u8]>,
) -> Vec<(Vec<&'a [u8]>, bool)> {
    let mut out: Vec<(Vec<&'a [u8]>, bool)> = Vec::with_capacity(2);
    if let Some(keyfile) = keyfile {
        out.push((vec![keyfile], true));
    }
    out.push((Vec::new(), false));
    out
}

/// The material a box enrolled with a key was written with, in the one order the
/// factors are mixed: the keyfile's digest first, then the response.
fn with_response<'a>(keyfile: Option<&'a [u8]>, response: &'a [u8]) -> Vec<(Vec<&'a [u8]>, bool)> {
    let mut out: Vec<(Vec<&'a [u8]>, bool)> = Vec::with_capacity(2);
    if let Some(keyfile) = keyfile {
        out.push((vec![keyfile, response], true));
    }
    out.push((vec![response], false));
    out
}

/// The secret list `crypto::material` takes, in the one order every derivation uses.
fn secrets<'a>(keyfile: Option<&'a [u8]>, response: Option<&'a [u8]>) -> Vec<&'a [u8]> {
    keyfile.into_iter().chain(response).collect()
}

/// Open a box with a printed recovery code instead of the password. The header says
/// whether there is one; a box without a slot says so, and the wrong code is the tag
/// refusing the key it unwrapped.
pub fn open_with_code(path: &Path, code: &str) -> Result<(Credentials, Vault), String> {
    let header = require_header(path)?;
    let slot = header
        .recovery
        .as_ref()
        .ok_or_else(|| crate::shared::i18n::t("this box has no recovery code").to_owned())?;
    let key = crate::vault::recovery::unwrap(slot, code, &header.box_id)?;
    open_keyed(path, key)
}

/// Put a recovery slot on a box: the key it already holds, wrapped under a new code.
/// The box is written by the caller, which is where the new header is made.
pub fn set_recovery(creds: &mut Credentials, code: &str, box_id: &str) -> Result<(), String> {
    creds.recovery = Some(crate::vault::recovery::wrap(&creds.key, code, box_id)?);
    Ok(())
}

/// Whether this box carries a recovery code, from the header alone.
pub fn has_recovery(path: &Path) -> bool {
    read_header(path)
        .ok()
        .flatten()
        .is_some_and(|header| header.recovery.is_some())
}

/// Open with a key derived earlier, for biometric unlock: no KDF runs, and the
/// AEAD tag is the whole check, so a rotated key simply fails to decrypt.
pub fn open_keyed(path: &Path, key: Key) -> Result<(Credentials, Vault), String> {
    clear_legacy_stash(path);
    let header = require_header(path)?;
    let vault = decrypt_manifest(path, &header, &key)?;
    Ok((
        Credentials {
            key,
            header: header.kdf,
            recovery: header.recovery,
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
