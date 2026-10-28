//! The chunked writer: every record is staged first, so a failed save rolls back.

use super::atomic::{parent_of, private_file, stash, sync_dir};
use super::header::{header_block, read_header, v4_aad, V4Header};
use super::{Credentials, CURRENT_VERSION, EXT};
use crate::vault::codec;
use crate::vault::container::{self, Kind, CHUNK, HEADER_BLOCK};
use crate::vault::crypto::{self, Key};
use crate::vault::model::{BlobRecord, ChunkRef, Deleted, Entry, File, Note, PendingFile, Vault};
use crate::vault::reader::BoxReader;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use zeroize::Zeroizing;

/// Payload padded to 4096 and never below 64 KiB, so its size says little about
/// what is inside; trailing spaces are valid JSON.
pub(super) const PAD_BLOCK: usize = 4096;
pub(super) const PAD_FLOOR: usize = 64 * 1024;

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
    let mut hasher = crypto::Hasher::new();
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
        digest: hasher.finish(),
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
            // A save carries the slot the session holds: clearing it is a decision,
            // not something a write does on its own.
            recovery: creds.recovery.clone(),
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

/// Save bumps the generation and writes the file; staging rolls back on failure.
pub fn save(path: &Path, vault: &mut Vault, creds: &Credentials) -> Result<(), String> {
    crate::vault::store::clear_legacy_stash(path);
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
