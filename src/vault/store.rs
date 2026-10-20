//! Vault file format: ONE portable encrypted file (KeePass-style).
//! v2 authenticates that header as AEAD associated data; v1 vaults still open.

use crate::vault::codec;
use crate::vault::crypto::{self, KdfParams, Key};
use crate::vault::model::Vault;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;
use zeroize::Zeroizing;

pub const CURRENT_VERSION: u32 = 3;
pub const EXT: &str = "pandora";

/// Payloads are padded up to this block, and never below [`PAD_FLOOR`], so the
/// file size says little about what is inside: an empty box and one with a dozen
/// entries are the same size, and a box only grows in 4 KiB steps after that.
/// Trailing spaces are valid JSON.
const PAD_BLOCK: usize = 4096;
const PAD_FLOOR: usize = 64 * 1024;

const KEYFILE_MAX: u64 = 8 * 1024 * 1024;

/// Four times what this app writes itself. The params sit in the plaintext header
/// and only authenticate *after* the derivation, so an unchecked `m_cost` is a
/// multi-gigabyte allocation — a box someone hands you could hang the app.
const MAX_M_COST: u32 = crypto::M_COST * 4;
const MAX_T_COST: u32 = crypto::T_COST * 4;
const MAX_P_COST: u32 = crypto::P_COST * 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfHeader {
    pub algorithm: String, // currently always "argon2id"
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub salt: String, // base64
    /// Whether a keyfile joined the password in the KDF. v2 wrote it here, in
    /// the clear, and authenticated it as part of the header. v3 does not: the
    /// KDF finds out on its own (see [`open`]), so a box no longer tells whoever
    /// holds the file that a second factor is needed. Kept only to authenticate
    /// a v2 box and open it; never written again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyfile: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Envelope {
    version: u32,
    kdf: KdfHeader,
    nonce: String,      // base64
    ciphertext: String, // base64
}

/// Associated data: the canonical form of the header fields that must not move.
/// The salt is base64 and the flag was one digit, so no field can forge a
/// separator. The nonce stays out — tampering with it already fails the tag.
fn payload_aad(version: u32, k: &KdfHeader) -> Vec<u8> {
    let head = format!(
        "pandora|{version}|{}|{}|{}|{}|{}",
        k.algorithm, k.m_cost, k.t_cost, k.p_cost, k.salt
    );
    // v2 spelled the keyfile flag as the header's last field, so its associated
    // data is one field longer than the one v3 authenticates.
    match version {
        2 => format!("{head}|{}", u8::from(k.keyfile.unwrap_or(false))),
        _ => head,
    }
    .into_bytes()
}

/// v1 authenticated nothing; from v2 the header is bound to the payload.
fn aad_for(env: &Envelope) -> Vec<u8> {
    if env.version >= 2 {
        payload_aad(env.version, &env.kdf)
    } else {
        Vec::new()
    }
}

fn header(params: &KdfParams, salt: &[u8]) -> KdfHeader {
    KdfHeader {
        algorithm: "argon2id".to_owned(),
        m_cost: params.m_cost,
        t_cost: params.t_cost,
        p_cost: params.p_cost,
        salt: codec::encode(salt),
        keyfile: None,
    }
}

/// Derived master key + the header needed to re-encrypt on save.
/// The password itself is never kept — only the derived key.
pub struct Credentials {
    pub key: Key,
    header: KdfHeader,
    /// Whether the key is the password *and* a keyfile: `Some(true)` when the
    /// KDF proved the box needs one, `Some(false)` when the password alone
    /// opened it, and `None` when this session never ran a KDF — a finger handed
    /// the key back, and nothing here can tell. The file itself never says.
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

/// Where prefs, recents and the default box live; one definition every caller
/// shares. Android has no `dirs` to ask: it hands the app its own private
/// directory from `android_main` (and, on iOS, the app's container).
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

/// One working copy per box, under the app's data dir.
///
/// The folder is named after a digest of the source, never after the source: a dir
/// called `sources/github/octocat/boxes` writes down which repo somebody mirrors
/// their boxes to, and the point of the box is that it does not say.
pub fn source_dir(kind: &str, target: &str) -> PathBuf {
    let dir = data_dir().join("sources").join(crypto::digest(
        format!("{}|{target}", sanitize(kind)).as_bytes(),
    ));
    // This build's first look at a source moves the folder the old layout left
    // behind: a working copy that never reached the repo is the only copy there is.
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

/// A name that survives a filesystem: what a box's own name becomes on the way to
/// a path. A source can be hand-edited, so a `..` segment must never walk out of
/// the boxes dir; an empty or all-dots name gets a placeholder rather than none.
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

/// Refuses an empty keyfile (a file with no entropy in it) and one too big to be
/// a keyfile at all.
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

/// Open a box with its master password. `keyfile` is the second factor, when the
/// one opening the box has it to hand.
///
/// The file does not say whether a keyfile was mixed into its key — v2 said so in
/// the clear, which told whoever held the file which way to attack it. So the KDF
/// asks instead: the password with the keyfile first when one was handed over,
/// then the password alone. The AEAD tag is what says which key the box owns, and
/// a key that does not fit simply fails to decrypt, exactly as a wrong password
/// does. Nothing is written down for an attacker to read.
pub fn open(
    path: &Path,
    password: &str,
    keyfile: Option<&[u8]>,
) -> Result<(Credentials, Vault), String> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read box: {e}"))?;
    let env: Envelope = read_envelope(&bytes)?;
    let salt = codec::decode(&env.kdf.salt)?;
    let params = KdfParams {
        m_cost: env.kdf.m_cost,
        t_cost: env.kdf.t_cost,
        p_cost: env.kdf.p_cost,
    };
    // One derivation in the common cases: a keyfile box is tried with it first,
    // a box without one has no second candidate at all, and the password alone
    // is the last thing tried either way.
    let mut attempts = Vec::with_capacity(2);
    if keyfile.is_some() {
        attempts.push(keyfile);
    }
    attempts.push(None);
    for with in attempts {
        let secret = crypto::material(password, with);
        let key = crypto::derive_key(&secret, &salt, &params)?;
        if let Ok(vault) = decrypt_payload(&env, &key) {
            return Ok((
                Credentials {
                    key,
                    header: env.kdf,
                    keyfile: Some(with.is_some()),
                },
                vault,
            ));
        }
    }
    Err(crypto::WRONG.to_owned())
}

/// Open with a key that was derived earlier — the one a fingerprint just handed
/// back (see `features/auth/biometric`). No KDF runs: the AEAD tag is the whole
/// check, so a key that no longer fits this file — a rotated password, another
/// box — simply fails to decrypt.
pub fn open_keyed(path: &Path, key: Key) -> Result<(Credentials, Vault), String> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read box: {e}"))?;
    let env: Envelope = read_envelope(&bytes)?;
    let vault = decrypt_payload(&env, &key)?;
    Ok((
        Credentials {
            key,
            header: env.kdf,
            // A keyed open runs no KDF, so there is nothing here to learn it
            // from: the session reports "unknown" and the form asks.
            keyfile: None,
        },
        vault,
    ))
}

/// Reload the vault from disk using an already-derived key (no KDF run).
pub fn load(path: &Path, creds: &Credentials) -> Result<Vault, String> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read box: {e}"))?;
    let env: Envelope = read_envelope(&bytes)?;
    decrypt_payload(&env, &creds.key)
}

/// Does `key` decrypt what is on disk? The AEAD tag comparison is constant time,
/// so this is the safe way to check a master password (v1 and v2 alike).
pub fn verify(path: &Path, key: &Key) -> bool {
    fs::read(path)
        .ok()
        .and_then(|b| read_envelope(&b).ok())
        .is_some_and(|env| decrypt_payload(&env, key).is_ok())
}

fn read_envelope(bytes: &[u8]) -> Result<Envelope, String> {
    let env: Envelope =
        serde_json::from_slice(bytes).map_err(|e| format!("not a Pandora box: {e}"))?;
    if env.version > CURRENT_VERSION {
        return Err(format!(
            "box version {} is newer than this app ({CURRENT_VERSION})",
            env.version
        ));
    }
    let k = &env.kdf;
    if k.m_cost > MAX_M_COST || k.t_cost > MAX_T_COST || k.p_cost > MAX_P_COST {
        return Err(format!(
            "box asks for kdf params this app will not run: {} KiB / {} passes / {} lanes",
            k.m_cost, k.t_cost, k.p_cost
        ));
    }
    Ok(env)
}

fn decrypt_payload(env: &Envelope, key: &Key) -> Result<Vault, String> {
    let nonce = codec::decode(&env.nonce)?;
    let ct = codec::decode(&env.ciphertext)?;
    let aad = aad_for(env);
    let plain = Zeroizing::new(crypto::decrypt(&nonce, &ct, key, &aad)?);
    serde_json::from_slice(&plain).map_err(|e| format!("corrupt box payload: {e}"))
}

fn pad(buf: &mut Vec<u8>) {
    let len = buf.len().max(PAD_FLOOR);
    buf.resize(len.div_ceil(PAD_BLOCK) * PAD_BLOCK, b' ');
}

/// Atomic write: tmp file + rename, so a sync (GDrive) never reads a half-written vault.
pub fn save(path: &Path, vault: &mut Vault, creds: &Credentials) -> Result<(), String> {
    // One write, one generation. Nothing else bumps this, so a copy that comes
    // back carrying a smaller one is a box that has been rewound — `adopt` is
    // where that is caught.
    vault.seq = vault.seq.saturating_add(1);
    let mut plain =
        Zeroizing::new(serde_json::to_vec(vault).map_err(|e| format!("serialize: {e}"))?);
    pad(&mut plain);
    // v3 has no keyfile flag in the header — the KDF rediscovers it, and a box
    // that announces which one it is only helps whoever holds the file. A v2
    // header that opened this box loses the field here, on the way back out.
    let header = KdfHeader {
        keyfile: None,
        ..creds.header.clone()
    };
    let aad = payload_aad(CURRENT_VERSION, &header);
    let (nonce, ct) = crypto::encrypt(&plain, &creds.key, &aad)?;
    let env = Envelope {
        version: CURRENT_VERSION,
        kdf: header,
        nonce: codec::encode(&nonce),
        ciphertext: codec::encode(&ct),
    };
    let json = serde_json::to_vec(&env).map_err(|e| format!("serialize: {e}"))?;

    stash(path);
    write_atomic(path, &json)
}

/// Take a copy of the box from somewhere else — the repo, or the stash beside it.
/// The bytes have to open with `creds` *before* they replace the file, so a copy
/// of another box, or of this box under another password, cannot eat the local
/// one.
///
/// `floor` is the generation the file may not go back behind. A copy that comes
/// from the repo carries the one this box is on — a repo rewound to an old blob
/// would otherwise undo everything that happened since, and the sha check does
/// not catch it because an old blob is a perfectly valid blob. A restore from the
/// stash passes `None`: going back is exactly what it is for.
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
        return Err(ROLLED_BACK.to_owned());
    }
    let dir = parent_of(path);
    stash(path);
    fs::rename(&candidate, path).map_err(|e| format!("cannot finalize: {e}"))?;
    sync_dir(dir)?;
    Ok(vault)
}

/// The one way this crate writes a box file: sibling tmp, fsync, rename, fsync dir.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = parent_of(path);
    fs::create_dir_all(dir).map_err(|e| format!("cannot create dir: {e}"))?;

    // A random tmp name: a fixed one is a file name another process can plant a
    // symlink on, and the box would be written through it.
    let tmp = path.with_extension(format!("{EXT}.{}.tmp", uuid::Uuid::new_v4()));
    let mut f = private_file(&tmp).map_err(|e| format!("cannot write: {e}"))?;
    f.write_all(bytes)
        .map_err(|e| format!("cannot write: {e}"))?;
    f.sync_all().map_err(|e| format!("fsync: {e}"))?;
    drop(f);
    fs::rename(&tmp, path).map_err(|e| format!("cannot finalize: {e}"))?;
    sync_dir(dir)
}

/// What a copy older than the box on screen is refused with: the same words a
/// caller can match on, so the repo's answer is told apart from a copy that does
/// not open at all.
pub const ROLLED_BACK: &str = "the copy is older than this box";

/// Keep the copy a box file is about to lose, in one slot beside it: the version
/// another device synced over, or the one a save replaces. One slot, overwritten
/// each time, so a folder holds one extra file and never a growing history. A
/// copy, not a link: the slot must stay a snapshot even if something writes the
/// box in place rather than replacing it.
pub fn stash(path: &Path) {
    if !path.is_file() {
        return;
    }
    let backup = path.with_extension(format!("{EXT}.bak"));
    let _ = fs::remove_file(&backup);
    let _ = copy_private(path, &backup);
}

/// Copy a box into the stash slot, readable by its owner from the moment it
/// exists.
///
/// `fs::copy` would carry over whatever mode the box arrived with, so a file
/// somebody else left at 0644 would leave a second, world-readable copy of the
/// same secret beside it. Going through [`private_file`] means the slot is 0600
/// at creation and the bytes are only ever written into a file nobody else can
/// open. The fsync is the point of a stash: a crash right after this must still
/// find the version the box is about to lose.
fn copy_private(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut input = fs::File::open(from)?;
    let mut out = private_file(to)?;
    std::io::copy(&mut input, &mut out)?;
    out.sync_all()
}

/// Drop the copy `stash` keeps beside a box.
///
/// The one caller that owes it is a password change: what sits in that slot was
/// written under the key the change just replaced, so leaving it would keep the
/// old master password working on this machine for as long as the file is there.
/// A box being deleted owes it too — the copy is a version of that box.
pub fn forget_stash(path: &Path) {
    let _ = fs::remove_file(path.with_extension(format!("{EXT}.bak")));
}

/// Every other file this app writes — prefs, recents, an export — is the user's
/// own too, so it goes out private as well.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = private_file(path).map_err(|e| format!("cannot write: {e}"))?;
    f.write_all(bytes).map_err(|e| format!("cannot write: {e}"))
}

/// Readable and writable by the user alone. Whoever else shares the machine
/// reads neither a box nor a plaintext export.
#[cfg(unix)]
fn private_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // `mode` only bites on creation, so a file an older build left at 0644 is
    // brought down here as well.
    f.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(f)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

fn parent_of(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Windows cannot open a directory for sync, so the guarantee is unix-only.
#[cfg(unix)]
fn sync_dir(dir: &Path) -> Result<(), String> {
    fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| format!("fsync dir: {e}"))
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> Result<(), String> {
    Ok(())
}

pub fn modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

pub fn human_size(bytes: usize) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::matching::{Rule, Uri};
    use crate::vault::model::{Login, Otp};
    use std::env::temp_dir;

    fn tmp_path(name: &str) -> PathBuf {
        let p = temp_dir().join(format!(
            "pandora-test-{name}-{}.{EXT}",
            uuid::Uuid::new_v4()
        ));
        let _ = fs::remove_file(&p);
        p
    }

    /// The write that follows a stash leaves the version it replaced beside it,
    /// so a replacement is never the last word.
    #[test]
    fn a_stash_keeps_what_a_write_replaces() {
        let p = tmp_path("stash");
        fs::write(&p, b"first").unwrap();
        stash(&p);
        fs::write(&p, b"second").unwrap();
        let backup = p.with_extension(format!("{EXT}.bak"));

        assert_eq!(fs::read(&p).unwrap(), b"second", "the write landed");
        assert_eq!(
            fs::read(&backup).unwrap(),
            b"first",
            "and what it replaced is still readable"
        );

        let missing = tmp_path("stash-none");
        stash(&missing);
        assert!(
            !missing.with_extension(format!("{EXT}.bak")).exists(),
            "nothing to keep, nothing written"
        );

        let _ = fs::remove_file(&p);
        let _ = fs::remove_file(&backup);
    }

    /// A password change drops the copy beside the box: what it holds is sealed
    /// with the key that change just replaced, and leaving it would keep the old
    /// master password working on this machine.
    #[test]
    fn the_copy_beside_a_box_can_be_dropped() {
        let p = tmp_path("forget-stash");
        fs::write(&p, b"old").unwrap();
        stash(&p);
        let backup = p.with_extension(format!("{EXT}.bak"));
        assert!(backup.exists(), "the write before the change kept one");

        forget_stash(&p);
        assert!(!backup.exists(), "and the change takes it away");
        assert!(p.exists(), "the box itself is not what this removes");
        // Dropping a copy that is not there is not a failure.
        forget_stash(&p);

        let _ = fs::remove_file(&p);
    }

    fn sample_item(id: &str) -> Login {
        let mut it = Login::new(42);
        it.id = id.to_owned();
        it.name = "GitHub".to_owned();
        it.username = "octocat".to_owned();
        it.password = Zeroizing::new("hunter2".to_owned());
        it.url = "https://github.com".to_owned();
        it
    }

    fn sample_branded_item(id: &str) -> Login {
        let mut it = sample_item(id);
        it.icon = Some("github".to_owned());
        it
    }

    fn envelope(path: &Path) -> Envelope {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    /// Derived folder never walks out of the boxes dir, even with a hostile target.
    #[test]
    fn a_source_dir_is_derived_and_stays_inside() {
        // The folder is a digest of the source: a path through `sources/github/`
        // naming the owner and the repo is what this build stopped writing.
        let dir = source_dir("github", "octocat/boxes");
        assert_eq!(dir.parent(), Some(data_dir().join("sources").as_path()));
        let leaf = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(leaf.len(), 32, "128 bits of digest in hex: {leaf}");
        assert!(leaf.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!leaf.contains("octocat") && !leaf.contains("boxes"));
        assert_ne!(dir, source_dir("github", "octocat/other"));

        let base = data_dir().join("sources");
        let hostile = source_dir("github", "../../etc/passwd");
        assert!(
            !hostile.to_string_lossy().contains(".."),
            "{hostile:?} kept a parent hop"
        );
        assert!(
            hostile.starts_with(&base),
            "{hostile:?} must stay under {base:?}"
        );
    }

    /// The folder an older build derived is moved once, so a working copy that
    /// never reached the repo is not what the change strands.
    #[test]
    fn a_legacy_source_dir_is_moved_to_its_digest() {
        let target = "octocat/moved-for-the-test";
        let legacy = legacy_source_dir("github", target);
        let _ = fs::remove_dir_all(legacy.parent().unwrap().parent().unwrap());
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("work.pandora"), b"locked").unwrap();

        let dir = source_dir("github", target);
        assert!(dir.join("work.pandora").is_file(), "the copy came along");
        assert!(
            !legacy.exists(),
            "and the folder that named the repo is gone"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_open_roundtrip() {
        let p = tmp_path("roundtrip");
        let (creds, mut vault) = create(&p, "My Vault", "master-pass", None).unwrap();
        assert!(vault.records().next().is_none());
        vault.logins.push(sample_item("abc"));
        save(&p, &mut vault, &creds).unwrap();

        let (creds2, vault2) = open(&p, "master-pass", None).unwrap();
        assert_eq!(vault2.logins[0].id, "abc");
        assert_eq!(vault2.logins[0].password.as_str(), "hunter2");

        // reload with cached key, no re-derivation
        let v3 = load(&p, &creds2).unwrap();
        assert_eq!(v3.logins.len(), 1);
        let _ = fs::remove_file(&p);
    }

    /// The params are plaintext and unauthenticated until after the derivation, so
    /// they are checked first, and refused with a message instead of being run.
    #[test]
    fn hostile_kdf_params_are_refused_before_deriving() {
        let p = tmp_path("kdf-bomb");
        let salt = crypto::random_bytes::<{ crypto::SALT_LEN }>();
        let env = serde_json::json!({
            "version": 2,
            "kdf": {
                "algorithm": "argon2id",
                "m_cost": u32::MAX,
                "t_cost": u32::MAX,
                "p_cost": MAX_P_COST * 2,
                "salt": codec::encode(&salt),
            },
            "nonce": codec::encode(&[0u8; crypto::NONCE_LEN]),
            "ciphertext": codec::encode(&[0u8; 32]),
        });
        fs::write(&p, serde_json::to_vec(&env).unwrap()).unwrap();

        assert!(matches!(open(&p, "pw", None), Err(e) if e.contains("kdf params")));
        assert!(!verify(&p, &Zeroizing::new([0u8; crypto::KEY_LEN])));
        let _ = fs::remove_file(&p);
    }

    /// A box is the user's own file: nobody else on the machine reads it, and a
    /// file an older build left at 0644 is brought down on the next write.
    #[cfg(unix)]
    #[test]
    fn a_box_lands_readable_by_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;

        let p = tmp_path("mode");
        let _ = create(&p, "My Vault", "pw", None).unwrap();
        assert_eq!(mode(&p), 0o600, "{p:?} came out readable by anyone");

        fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&p, b"{}").unwrap();
        assert_eq!(mode(&p), 0o600, "{p:?} kept its old, open mode");
        let _ = fs::remove_file(&p);
    }

    /// The copy beside a box is the same secret twice, so it is as private as the
    /// box should have been — even when the box itself arrived at 0644.
    #[cfg(unix)]
    #[test]
    fn a_backup_of_a_loose_box_is_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;

        let p = tmp_path("stash-mode");
        fs::write(&p, b"old box").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();

        stash(&p);
        let backup = p.with_extension(format!("{EXT}.bak"));
        assert_eq!(
            mode(&backup),
            0o600,
            "{backup:?} came out readable by anyone"
        );
        assert_eq!(
            fs::read(&backup).unwrap(),
            b"old box",
            "and it holds the box"
        );

        let _ = fs::remove_file(&p);
        let _ = fs::remove_file(&backup);
    }

    #[test]
    fn wrong_password_fails() {
        let p = tmp_path("wrongpass");
        let _ = create(&p, "My Vault", "right", None).unwrap();
        assert!(open(&p, "wrong", None).is_err());
        assert!(open(&p, "", None).is_err());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn tampered_payload_fails() {
        let p = tmp_path("tamper");
        let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
        save(&p, &mut vault, &creds).unwrap();
        let mut bytes = fs::read(&p).unwrap();
        let n = bytes.len();
        bytes[n - 3] ^= 0x20; // flip a bit inside the ciphertext
        fs::write(&p, &bytes).unwrap();
        assert!(open(&p, "pw", None).is_err());
        let _ = fs::remove_file(&p);
    }

    /// `algorithm` does not feed the KDF, so only the v2 associated data catches this edit.
    #[test]
    fn tampered_header_fails() {
        let p = tmp_path("hdrtamper");
        let _ = create(&p, "My Vault", "pw", None).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("argon2id"));
        fs::write(&p, text.replace("argon2id", "argon2i")).unwrap();
        assert!(open(&p, "pw", None).is_err());
        let _ = fs::remove_file(&p);
    }

    /// Padding hides the payload size: a small box is the floor and not its own
    /// size, and a bigger one only shows itself in whole blocks.
    #[test]
    fn payload_is_padded_to_a_block() {
        let p = tmp_path("pad");
        let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
        save(&p, &mut vault, &creds).unwrap();
        let ct = codec::decode(&envelope(&p).ciphertext).unwrap();
        assert_eq!((ct.len() - 16) % PAD_BLOCK, 0);
        assert_eq!(
            ct.len(),
            PAD_FLOOR + 16,
            "an empty box is the floor, never its own few hundred bytes"
        );

        // Past the floor, the size is the block it belongs to and nothing finer.
        let mut big = sample_item("big");
        big.notes = Zeroizing::new("x".repeat(PAD_FLOOR));
        vault.logins.push(big);
        save(&p, &mut vault, &creds).unwrap();
        let ct = codec::decode(&envelope(&p).ciphertext).unwrap();
        let padded = ct.len() - 16;
        assert_eq!(padded % PAD_BLOCK, 0);
        assert!(
            padded > PAD_FLOOR,
            "a box this size is above the floor: {padded}"
        );
        let _ = fs::remove_file(&p);
    }

    /// A vault written by the previous format (no associated data) still opens.
    #[test]
    fn legacy_v1_vault_without_aad_opens() {
        let p = tmp_path("v1");
        let salt = crypto::random_bytes::<{ crypto::SALT_LEN }>();
        let params = KdfParams::default();
        let key = crypto::derive_key(&crypto::material("pw", None), &salt, &params).unwrap();
        let plain = serde_json::to_vec(&Vault::new("Old")).unwrap();
        let (nonce, ct) = crypto::encrypt(&plain, &key, b"").unwrap(); // v1: no AAD
        let env = serde_json::json!({
            "version": 1,
            "kdf": {
                "algorithm": "argon2id",
                "m_cost": params.m_cost,
                "t_cost": params.t_cost,
                "p_cost": params.p_cost,
                "salt": codec::encode(&salt),
            },
            "nonce": codec::encode(&nonce),
            "ciphertext": codec::encode(&ct),
        });
        fs::write(&p, serde_json::to_vec(&env).unwrap()).unwrap();

        let (creds, mut v) = open(&p, "pw", None).unwrap();
        assert_eq!(v.name, "Old");
        assert!(verify(&p, &key), "a v1 vault must verify too");

        // the next save migrates it to the current version, header authenticated
        save(&p, &mut v, &creds).unwrap();
        assert_eq!(envelope(&p).version, CURRENT_VERSION);
        assert!(verify(&p, &creds.key));
        let (_c2, v2) = open(&p, "pw", None).unwrap();
        assert_eq!(v2.name, "Old");
        let _ = fs::remove_file(&p);
    }

    /// What a fingerprint unlock does: the key comes back from the platform and
    /// the file opens with no KDF run at all. And what a rotated password does to
    /// that key — the new salt means it opens nothing, so the form comes back.
    #[test]
    fn a_key_from_earlier_opens_the_box_until_its_password_changes() {
        let p = tmp_path("keyed");
        let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
        let key = creds.key.clone();

        let (_creds, opened) = open_keyed(&p, key.clone()).unwrap();
        assert_eq!(opened.name, "My Vault");

        let rotated = Credentials::rekey("other-password", None).unwrap();
        save(&p, &mut vault, &rotated).unwrap();
        assert!(
            open_keyed(&p, key).is_err(),
            "the key the reader kept no longer fits this file"
        );
        let (_creds, after) = open_keyed(&p, rotated.key).unwrap();
        assert_eq!(after.name, "My Vault");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn verify_accepts_only_the_right_key() {
        let p = tmp_path("verify");
        let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
        save(&p, &mut vault, &creds).unwrap();
        assert!(verify(&p, &creds.key));

        let salt = codec::decode(&envelope(&p).kdf.salt).unwrap();
        let wrong = crypto::derive_key(
            &crypto::material("nope", None),
            &salt,
            &KdfParams::default(),
        )
        .unwrap();
        assert!(!verify(&p, &wrong));
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn creates_parent_dirs() {
        let p = temp_dir()
            .join("pandora-test-nested")
            .join("a")
            .join("b")
            .join(format!("vault.{EXT}"));
        let _ = fs::remove_dir_all(p.parent().unwrap());
        let (c, mut v) = create(&p, "My Vault", "pw", None).unwrap();
        assert!(p.exists());
        save(&p, &mut v, &c).unwrap();
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn create_keeps_the_name() {
        let p = tmp_path("name");
        let (_c, v) = create(&p, "Work", "pw", None).unwrap();
        assert_eq!(v.name, "Work");
        let (_c2, v2) = open(&p, "pw", None).unwrap();
        assert_eq!(v2.name, "Work");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn icon_roundtrip() {
        let p = tmp_path("icon");
        let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
        vault.logins.push(sample_branded_item("a"));
        save(&p, &mut vault, &creds).unwrap();
        let (_c, v) = open(&p, "pw", None).unwrap();
        assert_eq!(v.logins[0].icon.as_deref(), Some("github"));
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn a_code_roundtrips() {
        let p = tmp_path("totp");
        let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
        let mut code = Otp::new(1);
        code.name = "GitHub".to_owned();
        code.secret = Zeroizing::new("JBSWY3DPEHPK3PXP".to_owned());
        vault.otps.push(code);
        save(&p, &mut vault, &creds).unwrap();
        let (_creds, opened) = open(&p, "pw", None).unwrap();
        assert_eq!(opened.otps[0].name, "GitHub");
        assert_eq!(opened.otps[0].secret.as_str(), "JBSWY3DPEHPK3PXP");
        let _ = fs::remove_file(&p);
    }

    /// A login written before it could pin an icon or a rule for its address
    /// still opens, and writes none of those keys back.
    #[test]
    fn a_login_without_its_optional_fields_loads() {
        let old = r#"{"id":"a","name":"GitHub","username":"u","password":"p","url":"https://github.com","notes":"","created":1,"modified":1}"#;
        let it: Login = serde_json::from_str(old).unwrap();
        assert_eq!(it.icon, None);
        assert_eq!(it.url_match, Rule::Domain);
        assert!(it.extra_urls.is_empty());

        let round = serde_json::to_string(&it).unwrap();
        for key in ["icon", "url_match", "extra_urls"] {
            assert!(!round.contains(key), "{key} in {round}");
        }
    }

    /// A drive entry that carries nothing writes neither its folder nor its
    /// files, and a box nobody put anything in writes no collection at all.
    #[test]
    fn an_empty_box_writes_no_empty_collection() {
        let old = r#"{"version":1,"name":"Old"}"#;
        let v: Vault = serde_json::from_str(old).unwrap();
        assert!(v.records().next().is_none());
        assert!(v.folders.is_empty());

        let round = serde_json::to_string(&v).unwrap();
        for key in ["folders", "logins", "drive", "otps", "trash"] {
            assert!(!round.contains(key), "{key} in {round}");
        }
    }

    /// An entry nobody gave a rule to writes the same JSON it did before rules
    /// existed, and an old vault still reads as a plain domain match.
    #[test]
    fn the_default_rule_and_no_extra_urls_stay_out_of_the_payload() {
        let legacy = r#"{"id":"a","name":"GitHub","username":"u","password":"p","url":"https://github.com","notes":"","created":1,"modified":1}"#;
        let it: Login = serde_json::from_str(legacy).unwrap();
        assert_eq!(it.url_match, Rule::Domain);
        assert!(it.extra_urls.is_empty());
        assert_eq!(it.uris().count(), 1, "the stored URL is the one address");

        let round = serde_json::to_string(&it).unwrap();
        assert!(!round.contains("url_match"), "{round}");
        assert!(!round.contains("extra_urls"), "{round}");
    }

    #[test]
    fn rules_and_extra_urls_survive_a_round_trip() {
        let p = tmp_path("rules");
        let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
        let mut it = sample_item("a");
        it.url = "example.com".to_owned();
        it.url_match = Rule::Prefix;
        it.extra_urls = vec![
            Uri {
                url: "login.example.com".to_owned(),
                rule: Rule::Host,
            },
            Uri {
                url: "evil.example.com".to_owned(),
                rule: Rule::Never,
            },
        ];
        vault.logins.push(it);
        save(&p, &mut vault, &creds).unwrap();

        let (_c, v) = open(&p, "pw", None).unwrap();
        assert_eq!(v.logins[0].url_match, Rule::Prefix);
        assert_eq!(v.logins[0].extra_urls.len(), 2);
        assert_eq!(v.logins[0].extra_urls[1].rule, Rule::Never);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn set_password_reencrypts() {
        let p = tmp_path("rekey");
        let (mut creds, mut vault) = create(&p, "My Vault", "old-pass", None).unwrap();
        vault.logins.push(sample_item("x"));
        save(&p, &mut vault, &creds).unwrap();
        creds = Credentials::rekey("new-pass", None).unwrap();
        save(&p, &mut vault, &creds).unwrap();

        assert!(open(&p, "old-pass", None).is_err());
        let (_c2, v2) = open(&p, "new-pass", None).unwrap();
        assert_eq!(v2.logins[0].name, "GitHub");
        assert_eq!(v2.logins[0].password.as_str(), "hunter2");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn keyfile_roundtrip() {
        let p = tmp_path("keyfile");
        let kf = b"pretend this is a 4 KiB keyfile";
        let (creds, mut vault) = create(&p, "My Vault", "pw", Some(kf)).unwrap();
        assert_eq!(creds.needs_keyfile(), Some(true));
        vault.logins.push(sample_item("k"));
        save(&p, &mut vault, &creds).unwrap();

        let (back, v) = open(&p, "pw", Some(kf)).unwrap();
        assert_eq!(v.logins[0].name, "GitHub");
        assert_eq!(
            back.needs_keyfile(),
            Some(true),
            "the KDF is what notices, not the file"
        );
        let _ = fs::remove_file(&p);
    }

    /// A box that needs its keyfile does not open without it — and says nothing
    /// about why, because the file no longer carries that bit.
    #[test]
    fn a_keyfile_box_does_not_open_without_its_keyfile() {
        let p = tmp_path("needs-keyfile");
        let (_, _) = create(&p, "My Vault", "pw", Some(b"keyfile")).unwrap();
        let refused = open(&p, "pw", None).err().expect("no keyfile, no key");
        assert_eq!(
            refused,
            crypto::WRONG,
            "the same words a wrong password gets: the box does not say"
        );
        assert!(open(&p, "pw", Some(b"other")).is_err());

        let text = fs::read_to_string(&p).unwrap();
        assert!(
            !text.contains("keyfile"),
            "nothing in the header says a second factor is needed: {text}"
        );
        let _ = fs::remove_file(&p);
    }

    /// A vault created without a keyfile opens with a stray one too: the password
    /// is the whole secret, and the KDF finds that out by itself.
    #[test]
    fn a_stray_keyfile_does_not_lock_a_password_box() {
        let p = tmp_path("stray");
        let _ = create(&p, "My Vault", "pw", None).unwrap();
        let (creds, v) = open(&p, "pw", Some(b"keyfile")).unwrap();
        assert_eq!(v.name, "My Vault");
        assert_eq!(
            creds.needs_keyfile(),
            Some(false),
            "the password alone did it"
        );
        let _ = fs::remove_file(&p);
    }

    /// A v2 box wrote whether a keyfile was used in the clear, and authenticated
    /// that flag. v3 dropped it from the header, so a v2 file has to be opened by
    /// its own spelling and re-saved without it.
    #[test]
    fn a_v2_box_still_opens_and_is_written_without_the_flag() {
        let p = tmp_path("v2");
        let salt = crypto::random_bytes::<{ crypto::SALT_LEN }>();
        let params = KdfParams::default();
        let secret = crypto::material("pw", Some(b"the keyfile"));
        let key = crypto::derive_key(&secret, &salt, &params).unwrap();
        let mut plain = serde_json::to_vec(&Vault::new("Old")).unwrap();
        pad(&mut plain);
        let kdf = KdfHeader {
            algorithm: "argon2id".to_owned(),
            m_cost: params.m_cost,
            t_cost: params.t_cost,
            p_cost: params.p_cost,
            salt: codec::encode(&salt),
            keyfile: Some(true),
        };
        let (nonce, ct) = crypto::encrypt(&plain, &key, &payload_aad(2, &kdf)).unwrap();
        let env = serde_json::json!({
            "version": 2,
            "kdf": serde_json::to_value(&kdf).unwrap(),
            "nonce": codec::encode(&nonce),
            "ciphertext": codec::encode(&ct),
        });
        fs::write(&p, serde_json::to_vec(&env).unwrap()).unwrap();
        assert!(fs::read_to_string(&p).unwrap().contains("keyfile"));

        let (creds, mut v) = open(&p, "pw", Some(b"the keyfile")).unwrap();
        assert_eq!(v.name, "Old");
        assert_eq!(creds.needs_keyfile(), Some(true));

        save(&p, &mut v, &creds).unwrap();
        assert_eq!(envelope(&p).version, CURRENT_VERSION);
        let text = fs::read_to_string(&p).unwrap();
        assert!(
            !text.contains("keyfile"),
            "the flag left the header with the rewrite: {text}"
        );
        let (_c, again) = open(&p, "pw", Some(b"the keyfile")).unwrap();
        assert_eq!(again.name, "Old");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn read_keyfile_rejects_empty_and_huge() {
        let p = tmp_path("kf-read");
        fs::write(&p, b"").unwrap();
        assert!(read_keyfile(&p).unwrap_err().contains("empty"));

        fs::write(&p, vec![7u8; (KEYFILE_MAX + 1) as usize]).unwrap();
        assert!(read_keyfile(&p).unwrap_err().contains("8 MB"));

        let _ = fs::remove_file(&p);
    }

    /// A copy that is older than the box is a box rewound. A repo can hand one
    /// over — an old blob is a perfectly valid blob, so the sha says nothing — and
    /// putting it in place would undo everything that happened since.
    #[test]
    fn a_copy_older_than_the_box_is_refused() {
        let p = tmp_path("rollback");
        let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
        vault.logins.push(sample_item("github"));
        save(&p, &mut vault, &creds).unwrap();
        let older = fs::read(&p).unwrap();
        vault.logins.push(sample_item("bank"));
        save(&p, &mut vault, &creds).unwrap();
        assert_eq!(vault.seq, 3, "a write is a generation");

        let refused =
            adopt(&p, &older, &creds, Some(vault.seq)).expect_err("a rewound copy is not taken");
        assert_eq!(refused, ROLLED_BACK);
        assert_eq!(load(&p, &creds).unwrap().seq, 3, "the box is untouched");
        assert!(
            !p.with_extension(format!("{EXT}.candidate")).exists(),
            "and nothing of the refused copy is left beside it"
        );

        // The same bytes are exactly what a restore wants: going back on purpose.
        let restored = adopt(&p, &older, &creds, None).unwrap();
        assert_eq!(restored.seq, 2);
        assert_eq!(restored.logins.len(), 1, "the older box, as it was");

        let _ = fs::remove_file(&p);
        let _ = fs::remove_file(p.with_extension(format!("{EXT}.bak")));
    }

    /// What an envelope tells whoever holds the file: the KDF it asks for, a salt
    /// and a nonce. Nothing about how it was opened — v2 wrote whether a keyfile
    /// joined the password, and that bit told an attacker which way to attack it.
    #[test]
    fn the_envelope_reveals_no_kdf_flag() {
        for (name, keyfile) in [
            ("flag-plain", None),
            ("flag-keyfile", Some(&b"the keyfile"[..])),
        ] {
            let p = tmp_path(name);
            let _ = create(&p, "My Vault", "pw", keyfile).unwrap();

            let text = fs::read_to_string(&p).unwrap();
            assert!(
                !text.contains("keyfile"),
                "the header says too much: {text}"
            );

            let env: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
            let kdf = env["kdf"].as_object().expect("a kdf object");
            let mut fields: Vec<&str> = kdf.keys().map(String::as_str).collect();
            fields.sort_unstable();
            assert_eq!(
                fields,
                ["algorithm", "m_cost", "p_cost", "salt", "t_cost"],
                "nothing else rides the header, whatever the box needs"
            );
            let _ = fs::remove_file(&p);
        }
    }

    /// A copy that does not open never replaces what is on disk.
    #[test]
    fn a_copy_that_does_not_open_never_replaces_the_box() {
        let src = tmp_path("adopt-src");
        let dst = tmp_path("adopt-dst");
        let (creds, mut vault) = create(&src, "Repo", "pw", None).unwrap();
        save(&src, &mut vault, &creds).unwrap();
        let bytes = fs::read(&src).unwrap();

        assert_eq!(adopt(&dst, &bytes, &creds, None).unwrap().name, "Repo");

        let (own, _) = create(&dst, "Local", "other", None).unwrap();
        let local = fs::read(&dst).unwrap();
        assert!(adopt(&dst, &bytes, &own, None).is_err(), "another password");
        assert_eq!(fs::read(&dst).unwrap(), local, "the local box is intact");
        assert!(
            !dst.with_extension(format!("{EXT}.candidate")).exists(),
            "and no half-written candidate is left behind"
        );

        let _ = fs::remove_file(&src);
        let _ = fs::remove_file(&dst);
    }
}
