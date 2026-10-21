//! The v4 file: fixed header, chunked records, authenticated manifest.

use crate::vault::codec;
use crate::vault::container::{self, Kind, CHUNK, HEADER_BLOCK, MAGIC};
use crate::vault::crypto::{self, KdfParams, Key};
use crate::vault::model::{BlobRecord, ChunkRef, Deleted, Entry, File, Note, PendingFile, Vault};
use crate::vault::reader::BoxReader;
use blake2::{Blake2b512, Digest as _};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;
use zeroize::Zeroizing;

pub const CURRENT_VERSION: u32 = 4;
pub const EXT: &str = "pandora";

/// Payload padded to 4096 and never below 64 KiB, so its size says little about
/// what is inside; trailing spaces are valid JSON.
const PAD_BLOCK: usize = 4096;
const PAD_FLOOR: usize = 64 * 1024;

const KEYFILE_MAX: u64 = 8 * 1024 * 1024;

/// Four times what this app writes. The params sit in the plaintext header and
/// only authenticate after derivation, so an unchecked `m_cost` can allocate gigabytes.
const MAX_M_COST: u32 = crypto::M_COST * 4;
const MAX_T_COST: u32 = crypto::T_COST * 4;
const MAX_P_COST: u32 = crypto::P_COST * 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfHeader {
    pub algorithm: String,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub salt: String, // base64
}

/// The plaintext header, fixed-size on disk and authenticated by the manifest's tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct V4Header {
    version: u32,
    kdf: KdfHeader,
    box_id: String,
    manifest_offset: u64,
    manifest_len: u64,
    manifest_nonce: String,
}

fn v4_aad(header: &V4Header) -> Vec<u8> {
    format!(
        "pandora4|{}|{}|{}|{}|{}|{}|{}|{}",
        header.version,
        header.kdf.algorithm,
        header.kdf.m_cost,
        header.kdf.t_cost,
        header.kdf.p_cost,
        header.kdf.salt,
        header.box_id,
        header.manifest_offset
    )
    .into_bytes()
}

fn header_block(header: &V4Header) -> Result<[u8; HEADER_BLOCK], String> {
    let json = serde_json::to_vec(header).map_err(|e| format!("serialize header: {e}"))?;
    if MAGIC.len() + json.len() > HEADER_BLOCK {
        return Err("header too large".to_owned());
    }
    let mut block = [b' '; HEADER_BLOCK];
    block[..MAGIC.len()].copy_from_slice(MAGIC);
    block[MAGIC.len()..MAGIC.len() + json.len()].copy_from_slice(&json);
    Ok(block)
}

fn read_header(path: &Path) -> Result<Option<V4Header>, String> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read box: {e}")),
    };
    let mut block = [0u8; HEADER_BLOCK];
    let read = file
        .read(&mut block)
        .map_err(|e| format!("cannot read box: {e}"))?;
    if read < MAGIC.len() || &block[..MAGIC.len()] != MAGIC {
        return Ok(None);
    }
    let body = &block[MAGIC.len()..read];
    let end = body.iter().rposition(|b| *b != b' ').map_or(0, |i| i + 1);
    let header: V4Header =
        serde_json::from_slice(&body[..end]).map_err(|e| format!("not a Pandora v4 box: {e}"))?;
    if header.version != CURRENT_VERSION {
        return Err(if header.version > CURRENT_VERSION {
            format!(
                "box version {} is newer than this app ({CURRENT_VERSION})",
                header.version
            )
        } else {
            format!(
                "not a Pandora v4 box: header says version {}",
                header.version
            )
        });
    }
    let k = &header.kdf;
    if k.m_cost > MAX_M_COST || k.t_cost > MAX_T_COST || k.p_cost > MAX_P_COST {
        return Err(format!(
            "box asks for kdf params this app will not run: {} KiB / {} passes / {} lanes",
            k.m_cost, k.t_cost, k.p_cost
        ));
    }
    Ok(Some(header))
}

fn decrypt_manifest(path: &Path, header: &V4Header, key: &Key) -> Result<Vault, String> {
    let size = fs::metadata(path)
        .map_err(|e| format!("cannot read box: {e}"))?
        .len();
    let end = header.manifest_offset.saturating_add(header.manifest_len);
    if end > size || header.manifest_len > usize::MAX as u64 {
        return Err("corrupt box: manifest outside the file".to_owned());
    }
    let mut file = fs::File::open(path).map_err(|e| format!("cannot open box: {e}"))?;
    file.seek(SeekFrom::Start(header.manifest_offset))
        .map_err(|e| format!("cannot seek in box: {e}"))?;
    let mut ct = vec![0u8; header.manifest_len as usize];
    file.read_exact(&mut ct)
        .map_err(|e| format!("cannot read box: {e}"))?;
    let nonce = codec::decode(&header.manifest_nonce)?;
    let plain = Zeroizing::new(crypto::decrypt(&nonce, &ct, key, &v4_aad(header))?);
    let mut vault: Vault =
        serde_json::from_slice(&plain).map_err(|e| format!("corrupt box payload: {e}"))?;
    vault.reader = Some(Arc::new(BoxReader::new(
        path.to_path_buf(),
        key.clone(),
        header.box_id.clone(),
    )));
    Ok(vault)
}

fn header(params: &KdfParams, salt: &[u8]) -> KdfHeader {
    KdfHeader {
        algorithm: "argon2id".to_owned(),
        m_cost: params.m_cost,
        t_cost: params.t_cost,
        p_cost: params.p_cost,
        salt: codec::encode(salt),
    }
}

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
    Err(crypto::WRONG.to_owned())
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

fn pad(buf: &mut Vec<u8>) {
    let len = buf.len().max(PAD_FLOOR);
    buf.resize(len.div_ceil(PAD_BLOCK) * PAD_BLOCK, b' ');
}

struct StagedFile {
    id: String,
    old_record: Option<BlobRecord>,
    data: Zeroizing<Vec<u8>>,
    pending: Option<PendingFile>,
}

struct StagedNote {
    id: String,
    old_record: Option<BlobRecord>,
    body: Zeroizing<String>,
    loaded: bool,
}

fn file_mut<'a>(vault: &'a mut Vault, id: &str) -> Option<&'a mut File> {
    if let Some(file) = vault.drive.iter_mut().find_map(|entry| match entry {
        Entry::File(file) if file.id == id => Some(file),
        _ => None,
    }) {
        return Some(file);
    }
    vault
        .trash
        .iter_mut()
        .find_map(|trashed| match &mut trashed.record {
            Deleted::Drive(Entry::File(file)) if file.id == id => Some(file),
            _ => None,
        })
}

fn note_mut<'a>(vault: &'a mut Vault, id: &str) -> Option<&'a mut Note> {
    if let Some(note) = vault.drive.iter_mut().find_map(|entry| match entry {
        Entry::Note(note) if note.id == id => Some(note),
        _ => None,
    }) {
        return Some(note);
    }
    vault
        .trash
        .iter_mut()
        .find_map(|trashed| match &mut trashed.record {
            Deleted::Drive(Entry::Note(note)) if note.id == id => Some(note),
            _ => None,
        })
}

fn digest_hex(hasher: Blake2b512) -> String {
    let digest = hasher.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn chunk_count(size: u64) -> Result<u32, String> {
    if size == 0 {
        return Ok(0);
    }
    let count = size.div_ceil(CHUNK as u64);
    u32::try_from(count).map_err(|_| "file has too many chunks".to_owned())
}

#[allow(clippy::too_many_arguments)]
fn write_chunk(
    out: &mut fs::File,
    plain: &mut Vec<u8>,
    original: usize,
    kind: Kind,
    id: &str,
    box_id: &str,
    key: &Key,
    index: u32,
    count: u32,
) -> Result<ChunkRef, String> {
    let padded = if original == 0 {
        0
    } else {
        original.div_ceil(PAD_BLOCK) * PAD_BLOCK
    };
    plain.resize(padded, 0);
    let aad = container::chunk_aad(box_id, kind, id, index, count, original as u64);
    let offset = out
        .stream_position()
        .map_err(|e| format!("cannot write box: {e}"))?;
    let nonce = crypto::encrypt_in_place(plain, key, &aad)?;
    out.write_all(plain)
        .map_err(|e| format!("cannot write box: {e}"))?;
    Ok(ChunkRef {
        offset,
        len: plain.len() as u64,
        nonce: codec::encode(&nonce),
    })
}

fn write_blob_from_bytes(
    out: &mut fs::File,
    bytes: &[u8],
    kind: Kind,
    id: &str,
    box_id: &str,
    key: &Key,
) -> Result<BlobRecord, String> {
    let size = bytes.len() as u64;
    let count = chunk_count(size)?;
    let digest = crypto::digest(bytes);
    let mut chunks = Vec::with_capacity(count as usize);
    for (index, part) in bytes.chunks(CHUNK).enumerate() {
        let mut plain = part.to_vec();
        chunks.push(write_chunk(
            out,
            &mut plain,
            part.len(),
            kind,
            id,
            box_id,
            key,
            u32::try_from(index).map_err(|_| "too many chunks")?,
            count,
        )?);
    }
    Ok(BlobRecord {
        size,
        digest,
        chunks,
    })
}

fn write_blob_from_path(
    out: &mut fs::File,
    path: &Path,
    kind: Kind,
    id: &str,
    box_id: &str,
    key: &Key,
) -> Result<BlobRecord, String> {
    let size = fs::metadata(path)
        .map_err(|e| format!("cannot read {path:?}: {e}"))?
        .len();
    let count = chunk_count(size)?;
    let mut hasher = Blake2b512::new();
    let mut input = fs::File::open(path).map_err(|e| format!("cannot read {path:?}: {e}"))?;
    let mut chunks = Vec::with_capacity(count as usize);
    for index in 0..count {
        let remaining = size - u64::from(index) * CHUNK as u64;
        let len = remaining.min(CHUNK as u64) as usize;
        let mut plain = vec![0u8; len];
        input
            .read_exact(&mut plain)
            .map_err(|e| format!("cannot read {path:?}: {e}"))?;
        hasher.update(&plain);
        chunks.push(write_chunk(
            out, &mut plain, len, kind, id, box_id, key, index, count,
        )?);
    }
    Ok(BlobRecord {
        size,
        digest: digest_hex(hasher),
        chunks,
    })
}

fn copy_record(
    out: &mut fs::File,
    old_path: &Path,
    record: &BlobRecord,
) -> Result<BlobRecord, String> {
    let mut input = fs::File::open(old_path).map_err(|e| format!("cannot read old box: {e}"))?;
    let mut chunks = Vec::with_capacity(record.chunks.len());
    let mut buffer = vec![0u8; 64 * 1024];
    for chunk in &record.chunks {
        let offset = out
            .stream_position()
            .map_err(|e| format!("cannot write box: {e}"))?;
        input
            .seek(SeekFrom::Start(chunk.offset))
            .map_err(|e| format!("cannot seek old box: {e}"))?;
        let mut left = chunk.len;
        while left > 0 {
            let take = left.min(buffer.len() as u64) as usize;
            input
                .read_exact(&mut buffer[..take])
                .map_err(|e| format!("cannot read old box: {e}"))?;
            out.write_all(&buffer[..take])
                .map_err(|e| format!("cannot write box: {e}"))?;
            left -= take as u64;
        }
        chunks.push(ChunkRef {
            offset,
            len: chunk.len,
            nonce: chunk.nonce.clone(),
        });
    }
    Ok(BlobRecord {
        size: record.size,
        digest: record.digest.clone(),
        chunks,
    })
}

#[allow(clippy::too_many_arguments)]
fn reencrypt_record(
    out: &mut fs::File,
    reader: &BoxReader,
    record: &BlobRecord,
    kind: Kind,
    id: &str,
    box_id: &str,
    key: &Key,
) -> Result<BlobRecord, String> {
    let count = u32::try_from(record.chunks.len()).map_err(|_| "too many chunks")?;
    let mut chunks = Vec::with_capacity(record.chunks.len());
    for index in 0..record.chunks.len() {
        let mut plain = reader.read_chunk_plain(kind, id, record, index)?;
        let original = plain.len();
        chunks.push(write_chunk(
            out,
            &mut plain,
            original,
            kind,
            id,
            box_id,
            key,
            u32::try_from(index).map_err(|_| "too many chunks")?,
            count,
        )?);
    }
    Ok(BlobRecord {
        size: record.size,
        digest: record.digest.clone(),
        chunks,
    })
}

#[allow(clippy::too_many_arguments)]
fn stage_file(
    file: &mut File,
    out: &mut fs::File,
    old_path: &Path,
    old_reader: Option<&BoxReader>,
    same_key: bool,
    box_id: &str,
    key: &Key,
    staged: &mut Vec<StagedFile>,
) -> Result<(), String> {
    let id = file.id.clone();
    staged.push(StagedFile {
        id: id.clone(),
        old_record: file.record.clone(),
        data: std::mem::take(&mut file.data),
        pending: file.pending.take(),
    });
    // Rollback state is staged before anything can fail, so a full disk leaves memory untouched.
    let entry = staged.last().expect("just staged");
    let new_record = if !entry.data.is_empty() {
        write_blob_from_bytes(out, &entry.data, Kind::File, &id, box_id, key)?
    } else if let Some(pending) = &entry.pending {
        write_blob_from_path(out, &pending.path, Kind::File, &id, box_id, key)?
    } else if let Some(record) = &entry.old_record {
        if same_key {
            copy_record(out, old_path, record)?
        } else {
            let reader =
                old_reader.ok_or_else(|| "the old box is not open to re-encrypt it".to_owned())?;
            reencrypt_record(out, reader, record, Kind::File, &id, box_id, key)?
        }
    } else {
        write_blob_from_bytes(out, &[], Kind::File, &id, box_id, key)?
    };
    file.record = Some(new_record);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn stage_note(
    note: &mut Note,
    out: &mut fs::File,
    old_path: &Path,
    old_reader: Option<&BoxReader>,
    same_key: bool,
    box_id: &str,
    key: &Key,
    staged: &mut Vec<StagedNote>,
) -> Result<(), String> {
    let id = note.id.clone();
    staged.push(StagedNote {
        id: id.clone(),
        old_record: note.record.clone(),
        body: std::mem::take(&mut note.body),
        loaded: note.loaded,
    });
    // As with a file, the staging row exists before the write can fail.
    let entry = staged.last().expect("just staged");
    let new_record = if let Some(record) = &entry.old_record {
        let unchanged = same_key
            && if entry.loaded {
                crypto::digest(entry.body.as_bytes()) == record.digest
            } else {
                entry.body.is_empty()
            };
        if unchanged {
            copy_record(out, old_path, record)?
        } else if entry.loaded || same_key {
            write_blob_from_bytes(out, entry.body.as_bytes(), Kind::Note, &id, box_id, key)?
        } else {
            let reader =
                old_reader.ok_or_else(|| "the old box is not open to re-encrypt it".to_owned())?;
            reencrypt_record(out, reader, record, Kind::Note, &id, box_id, key)?
        }
    } else {
        write_blob_from_bytes(out, entry.body.as_bytes(), Kind::Note, &id, box_id, key)?
    };
    note.record = Some(new_record);
    note.loaded = false;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn stage_entry(
    entry: &mut Entry,
    out: &mut fs::File,
    old_path: &Path,
    old_reader: Option<&BoxReader>,
    same_key: bool,
    box_id: &str,
    key: &Key,
    files: &mut Vec<StagedFile>,
    notes: &mut Vec<StagedNote>,
) -> Result<(), String> {
    match entry {
        Entry::File(file) => stage_file(
            file, out, old_path, old_reader, same_key, box_id, key, files,
        ),
        Entry::Note(note) => stage_note(
            note, out, old_path, old_reader, same_key, box_id, key, notes,
        ),
    }
}

fn restore_staged(vault: &mut Vault, files: &mut Vec<StagedFile>, notes: &mut Vec<StagedNote>) {
    for staged in files.drain(..) {
        if let Some(file) = file_mut(vault, &staged.id) {
            file.record = staged.old_record;
            file.data = staged.data;
            file.pending = staged.pending;
        }
    }
    for staged in notes.drain(..) {
        if let Some(note) = note_mut(vault, &staged.id) {
            note.record = staged.old_record;
            note.body = staged.body;
            note.loaded = staged.loaded;
        }
    }
}

fn finish_staged(vault: &mut Vault, files: &mut Vec<StagedFile>, notes: &mut Vec<StagedNote>) {
    for staged in files.drain(..) {
        drop(staged.data);
        if let Some(pending) = staged.pending {
            if pending.owned {
                let _ = fs::remove_file(&pending.path);
            }
        }
    }
    for staged in notes.drain(..) {
        if let Some(note) = note_mut(vault, &staged.id) {
            if staged.loaded {
                note.body = staged.body;
                note.loaded = true;
            } else {
                // A closed note is now chunks; keeping its plaintext body would defeat the point.
                note.body = Zeroizing::new(String::new());
                note.loaded = false;
            }
        }
    }
}

/// Save bumps the generation and writes the file; staging rolls back on failure.
pub fn save(path: &Path, vault: &mut Vault, creds: &Credentials) -> Result<(), String> {
    vault.seq = vault.seq.saturating_add(1);
    let mut files = Vec::new();
    let mut notes = Vec::new();
    let result = write_v4(path, vault, creds, &mut files, &mut notes);
    match result {
        Ok(box_id) => {
            finish_staged(vault, &mut files, &mut notes);
            vault.reader = Some(Arc::new(BoxReader::new(
                path.to_path_buf(),
                creds.key.clone(),
                box_id,
            )));
            Ok(())
        }
        Err(e) => {
            restore_staged(vault, &mut files, &mut notes);
            Err(e)
        }
    }
}

fn write_v4(
    path: &Path,
    vault: &mut Vault,
    creds: &Credentials,
    files: &mut Vec<StagedFile>,
    notes: &mut Vec<StagedNote>,
) -> Result<String, String> {
    let old_header = read_header(path)?;
    if old_header.is_none() && path.exists() {
        return Err(format!(
            "not a Pandora v4 box: refusing to overwrite {}",
            path.display()
        ));
    }
    let new_kdf = creds.header.clone();
    let same_key = old_header.as_ref().is_some_and(|h| h.kdf == new_kdf);
    let box_id = match &old_header {
        Some(header) if same_key => header.box_id.clone(),
        _ => codec::encode(&crypto::random_bytes::<16>()),
    };
    let old_reader = vault.reader.clone();
    let old_path = path.to_path_buf();
    let dir = parent_of(path);
    fs::create_dir_all(dir).map_err(|e| format!("cannot create dir: {e}"))?;
    let tmp = path.with_extension(format!("{EXT}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), String> {
        let mut out = private_file(&tmp).map_err(|e| format!("cannot write: {e}"))?;
        out.write_all(&[b' '; HEADER_BLOCK])
            .map_err(|e| format!("cannot write: {e}"))?;
        for entry in vault.drive.iter_mut() {
            stage_entry(
                entry,
                &mut out,
                &old_path,
                old_reader.as_deref(),
                same_key,
                &box_id,
                &creds.key,
                files,
                notes,
            )?;
        }
        for trashed in vault.trash.iter_mut() {
            let Deleted::Drive(entry) = &mut trashed.record else {
                continue;
            };
            stage_entry(
                entry,
                &mut out,
                &old_path,
                old_reader.as_deref(),
                same_key,
                &box_id,
                &creds.key,
                files,
                notes,
            )?;
        }
        let mut manifest = serde_json::to_vec(vault).map_err(|e| format!("serialize: {e}"))?;
        pad(&mut manifest);
        let manifest_offset = out
            .stream_position()
            .map_err(|e| format!("cannot write: {e}"))?;
        let mut header = V4Header {
            version: CURRENT_VERSION,
            kdf: new_kdf,
            box_id: box_id.clone(),
            manifest_offset,
            manifest_len: 0,
            manifest_nonce: String::new(),
        };
        let nonce = crypto::encrypt_in_place(&mut manifest, &creds.key, &v4_aad(&header))?;
        header.manifest_len = manifest.len() as u64;
        header.manifest_nonce = codec::encode(&nonce);
        out.write_all(&manifest)
            .map_err(|e| format!("cannot write: {e}"))?;
        let end = out
            .stream_position()
            .map_err(|e| format!("cannot write: {e}"))?;
        let block = header_block(&header)?;
        out.seek(SeekFrom::Start(0))
            .map_err(|e| format!("cannot write: {e}"))?;
        out.write_all(&block)
            .map_err(|e| format!("cannot write: {e}"))?;
        out.seek(SeekFrom::Start(end))
            .map_err(|e| format!("cannot write: {e}"))?;
        let padded_end = end.div_ceil(PAD_BLOCK as u64) * PAD_BLOCK as u64;
        let target = padded_end.max(PAD_FLOOR as u64);
        if target > end {
            out.write_all(&vec![0u8; (target - end) as usize])
                .map_err(|e| format!("cannot write: {e}"))?;
        }
        out.sync_all().map_err(|e| format!("fsync: {e}"))?;
        drop(out);
        stash(path);
        fs::rename(&tmp, path).map_err(|e| format!("cannot finalize: {e}"))?;
        let _ = sync_dir(dir);
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(box_id)
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
        return Err(ROLLED_BACK.to_owned());
    }
    let dir = parent_of(path);
    stash(path);
    fs::rename(&candidate, path).map_err(|e| format!("cannot finalize: {e}"))?;
    sync_dir(dir)?;
    // The candidate's reader names a file that no longer exists; reload from the final path.
    load(path, creds)
}

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

/// What a rewound copy is refused with; callers match on it to tell it apart from
/// a copy that does not open.
pub const ROLLED_BACK: &str = "the copy is older than this box";

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
fn copy_private(from: &Path, to: &Path) -> std::io::Result<()> {
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
fn private_file(path: &Path) -> std::io::Result<fs::File> {
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

    fn header(path: &Path) -> V4Header {
        read_header(path).unwrap().unwrap()
    }

    #[test]
    fn a_source_dir_is_derived_and_stays_inside() {
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

        let v3 = load(&p, &creds2).unwrap();
        assert_eq!(v3.logins.len(), 1);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn hostile_kdf_params_are_refused_before_deriving() {
        let p = tmp_path("kdf-bomb");
        let salt = crypto::random_bytes::<{ crypto::SALT_LEN }>();
        let header = V4Header {
            version: CURRENT_VERSION,
            kdf: KdfHeader {
                algorithm: "argon2id".to_owned(),
                m_cost: u32::MAX,
                t_cost: u32::MAX,
                p_cost: MAX_P_COST * 2,
                salt: codec::encode(&salt),
            },
            box_id: "bomb".to_owned(),
            manifest_offset: HEADER_BLOCK as u64,
            manifest_len: 0,
            manifest_nonce: String::new(),
        };
        fs::write(&p, header_block(&header).unwrap()).unwrap();

        assert!(matches!(open(&p, "pw", None), Err(e) if e.contains("kdf params")));
        assert!(!verify(&p, &Zeroizing::new([0u8; crypto::KEY_LEN])));
        let _ = fs::remove_file(&p);
    }

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
        let header = header(&p);
        let at = (header.manifest_offset + header.manifest_len - 1) as usize;
        bytes[at] ^= 0x20;
        fs::write(&p, &bytes).unwrap();
        assert!(open(&p, "pw", None).is_err());
        let _ = fs::remove_file(&p);
    }

    /// `algorithm` does not feed the KDF; the header AAD catches the edit.
    #[test]
    fn tampered_header_fails() {
        let p = tmp_path("hdrtamper");
        let _ = create(&p, "My Vault", "pw", None).unwrap();
        let mut bytes = fs::read(&p).unwrap();
        let at = bytes
            .windows(b"argon2id".len())
            .position(|window| window == b"argon2id")
            .expect("the header names the KDF");
        bytes[at..at + 8].copy_from_slice(b"argon2ii");
        fs::write(&p, &bytes).unwrap();
        assert!(open(&p, "pw", None).is_err());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn a_legacy_box_is_refused_and_not_overwritten() {
        let p = tmp_path("legacy-refused");
        let legacy = br#"{"version":3,"kdf":{"algorithm":"argon2id","m_cost":65536,"t_cost":3,"p_cost":4,"salt":""},"nonce":"","ciphertext":""}"#.to_vec();
        fs::write(&p, &legacy).unwrap();

        let err = open(&p, "pw", None)
            .err()
            .expect("a legacy box must not open");
        assert!(err.contains("v4"), "{err}");
        assert!(!verify(&p, &Zeroizing::new([0u8; crypto::KEY_LEN])));

        let creds = Credentials::rekey("pw", None).unwrap();
        let mut vault = Vault::new("New");
        let err = save(&p, &mut vault, &creds).unwrap_err();
        assert!(err.contains("refusing to overwrite"), "{err}");
        assert_eq!(fs::read(&p).unwrap(), legacy, "the old box is untouched");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn payload_is_padded_to_a_block() {
        let p = tmp_path("pad");
        let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
        save(&p, &mut vault, &creds).unwrap();
        let floor = fs::metadata(&p).unwrap().len();
        assert!(
            floor >= PAD_FLOOR as u64,
            "an empty box is never its own few hundred bytes: {floor}"
        );
        assert_eq!(floor % PAD_BLOCK as u64, 0);

        let mut big = sample_item("big");
        big.notes = Zeroizing::new("x".repeat(PAD_FLOOR));
        vault.logins.push(big);
        save(&p, &mut vault, &creds).unwrap();
        let size = fs::metadata(&p).unwrap().len();
        assert_eq!(size % PAD_BLOCK as u64, 0);
        assert!(size > floor, "a bigger box grows: {size} vs {floor}");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn v4_chunks_files_and_notes() {
        let p = tmp_path("v4chunks");
        let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
        let payload: Vec<u8> = (0..(2 * CHUNK + 123)).map(|i| (i % 251) as u8).collect();
        let body = "hello ".repeat(30_000);
        let mut note = Note::new(1);
        note.name = "Note".to_owned();
        note.body = Zeroizing::new(body.clone());
        vault.drive.push(Entry::Note(note));
        vault.drive.push(Entry::File(File::new(
            "doc.bin".to_owned(),
            payload.clone(),
            1,
        )));
        save(&p, &mut vault, &creds).unwrap();

        let saved = match &vault.drive[1] {
            Entry::File(file) => file,
            _ => panic!("the drive keeps the file"),
        };
        assert!(saved.record.is_some(), "the file has a record");
        assert!(saved.data.is_empty(), "the inline bytes left memory");
        assert_eq!(saved.bytes(), payload.len());

        let (_c, opened) = open(&p, "pw", None).unwrap();
        let (note, file) = match (&opened.drive[0], &opened.drive[1]) {
            (Entry::Note(note), Entry::File(file)) => (note, file),
            _ => panic!("the drive keeps both entries"),
        };
        assert!(file.record.is_some());
        assert!(file.data.is_empty());
        assert_eq!(
            opened.read_file(file).unwrap().as_slice(),
            payload.as_slice(),
            "the chunks come back byte for byte"
        );
        assert_eq!(opened.note_text(note).unwrap().as_str(), body);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn a_failed_write_restores_every_staged_entry() {
        let p = tmp_path("rollback");
        let missing = tmp_path("rollback-missing");
        let creds = Credentials::rekey("pass", None).unwrap();
        let mut vault = Vault::new("Box");
        vault.drive.push(Entry::File(File::new(
            "kept.bin".to_owned(),
            vec![7; 100],
            1,
        )));
        let mut note = Note::new(1);
        note.body = Zeroizing::new("kept note".to_owned());
        vault.drive.push(Entry::Note(note));
        vault.drive.push(Entry::File(File::new_pending(
            "gone.bin".to_owned(),
            missing,
            10,
            false,
            1,
        )));

        assert!(
            save(&p, &mut vault, &creds).is_err(),
            "the missing file fails it"
        );
        let Entry::File(file) = &vault.drive[0] else {
            panic!("the file is still there");
        };
        assert_eq!(
            file.data.as_slice(),
            &[7u8; 100],
            "the inline bytes came back"
        );
        assert!(file.record.is_none());
        let Entry::Note(note) = &vault.drive[1] else {
            panic!("the note is still there");
        };
        assert_eq!(note.body.as_str(), "kept note");
        assert!(note.loaded);
        let Entry::File(pending) = &vault.drive[2] else {
            panic!("the pending file is still there");
        };
        assert!(pending.pending.is_some(), "the pending path came back");
        assert!(pending.record.is_none());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn a_pending_file_is_chunked_on_save() {
        let p = tmp_path("pending");
        let src = temp_dir().join(format!("pandora-src-{}.bin", uuid::Uuid::new_v4()));
        let data = vec![3u8; CHUNK + 17];
        fs::write(&src, &data).unwrap();
        let (creds, mut vault) = create(&p, "My Vault", "pw", None).unwrap();
        vault.drive.push(Entry::File(File::new_pending(
            "src.bin".to_owned(),
            src.clone(),
            data.len() as u64,
            true,
            1,
        )));
        save(&p, &mut vault, &creds).unwrap();
        assert!(!src.exists(), "the owned copy is removed after the save");

        let (_c, opened) = open(&p, "pw", None).unwrap();
        let Entry::File(file) = &opened.drive[0] else {
            panic!("the pending file became a file");
        };
        assert_eq!(opened.read_file(file).unwrap().as_slice(), data.as_slice());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn a_rotated_key_reencrypts_records() {
        let p = tmp_path("rekey-records");
        let (creds, mut vault) = create(&p, "My Vault", "old-pass", None).unwrap();
        let data = vec![9u8; 4096];
        vault.drive.push(Entry::File(File::new(
            "doc.bin".to_owned(),
            data.clone(),
            1,
        )));
        let mut note = Note::new(1);
        note.body = Zeroizing::new("old note".to_owned());
        vault.drive.push(Entry::Note(note));
        save(&p, &mut vault, &creds).unwrap();

        // A loaded note edited before the rotation travels with its edit.
        if let Entry::Note(note) = &mut vault.drive[1] {
            note.body = Zeroizing::new("new note".to_owned());
            note.loaded = true;
        }
        let new = Credentials::rekey("new-pass", None).unwrap();
        save(&p, &mut vault, &new).unwrap();
        assert!(open(&p, "old-pass", None).is_err());

        let (_c, opened) = open(&p, "new-pass", None).unwrap();
        let Entry::File(file) = &opened.drive[0] else {
            panic!("the file survived the rotation");
        };
        assert_eq!(opened.read_file(file).unwrap().as_slice(), data.as_slice());
        let Entry::Note(note) = &opened.drive[1] else {
            panic!("the note survived the rotation");
        };
        assert_eq!(opened.note_text(note).unwrap().as_str(), "new note");
        let _ = fs::remove_file(&p);
    }

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

        let salt = codec::decode(&header(&p).kdf.salt).unwrap();
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

        let text = serde_json::to_string(&header(&p)).unwrap();
        assert!(
            !text.contains("keyfile"),
            "nothing in the header says a second factor is needed: {text}"
        );
        let _ = fs::remove_file(&p);
    }

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

    #[test]
    fn read_keyfile_rejects_empty_and_huge() {
        let p = tmp_path("kf-read");
        fs::write(&p, b"").unwrap();
        assert!(read_keyfile(&p).unwrap_err().contains("empty"));

        fs::write(&p, vec![7u8; (KEYFILE_MAX + 1) as usize]).unwrap();
        assert!(read_keyfile(&p).unwrap_err().contains("8 MB"));

        let _ = fs::remove_file(&p);
    }

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

        // The same bytes are what a restore wants: going back on purpose.
        let restored = adopt(&p, &older, &creds, None).unwrap();
        assert_eq!(restored.seq, 2);
        assert_eq!(restored.logins.len(), 1, "the older box, as it was");

        let _ = fs::remove_file(&p);
        let _ = fs::remove_file(p.with_extension(format!("{EXT}.bak")));
    }

    #[test]
    fn the_header_reveals_no_kdf_flag() {
        for (name, keyfile) in [
            ("flag-plain", None),
            ("flag-keyfile", Some(&b"the keyfile"[..])),
        ] {
            let p = tmp_path(name);
            let _ = create(&p, "My Vault", "pw", keyfile).unwrap();

            let header = header(&p);
            let text = serde_json::to_string(&header).unwrap();
            assert!(
                !text.contains("keyfile"),
                "the header says too much: {text}"
            );

            let kdf = serde_json::to_value(&header.kdf).unwrap();
            let kdf = kdf.as_object().expect("a kdf object");
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
