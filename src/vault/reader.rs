//! Reads v4 chunks by path: the box is opened per read, so a save can rename over
//! the file on Windows without a stale handle.

use crate::vault::codec;
use crate::vault::container::{self, Kind};
use crate::vault::crypto::{self, Key};
use crate::vault::model::BlobRecord;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use zeroize::Zeroizing;

pub(crate) struct BoxReader {
    path: PathBuf,
    key: Key,
    box_id: String,
}

impl fmt::Debug for BoxReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxReader").finish_non_exhaustive()
    }
}

impl BoxReader {
    pub(crate) fn new(path: PathBuf, key: Key, box_id: String) -> Self {
        Self { path, key, box_id }
    }

    #[allow(clippy::too_many_arguments)]
    fn chunk(
        &self,
        file: &mut File,
        kind: Kind,
        id: &str,
        chunk: &crate::vault::model::ChunkRef,
        index: u32,
        count: u32,
        plain_len: u64,
    ) -> Result<Zeroizing<Vec<u8>>, String> {
        let nonce = codec::decode(&chunk.nonce)?;
        let mut bytes = vec![0u8; usize::try_from(chunk.len).map_err(|_| "chunk too large")?];
        file.seek(SeekFrom::Start(chunk.offset))
            .map_err(|e| format!("cannot seek in the box: {e}"))?;
        file.read_exact(&mut bytes)
            .map_err(|e| format!("cannot read the box: {e}"))?;
        let aad = container::chunk_aad(&self.box_id, kind, id, index, count, plain_len);
        let plain = crypto::decrypt(&nonce, &bytes, &self.key, &aad)?;
        Ok(Zeroizing::new(plain))
    }

    pub(crate) fn read_record(
        &self,
        kind: Kind,
        id: &str,
        record: &BlobRecord,
    ) -> Result<Zeroizing<Vec<u8>>, String> {
        let mut out = Zeroizing::new(Vec::with_capacity(
            usize::try_from(record.size).unwrap_or(0),
        ));
        if record.size == 0 {
            return Ok(out);
        }
        let mut file = File::open(&self.path).map_err(|e| format!("cannot open the box: {e}"))?;
        let count = u32::try_from(record.chunks.len()).map_err(|_| "too many chunks")?;
        for (index, chunk) in record.chunks.iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| "too many chunks")?;
            let plain_len = (record.size - u64::from(index) * container::CHUNK as u64)
                .min(container::CHUNK as u64);
            let plain = self.chunk(&mut file, kind, id, chunk, index, count, plain_len)?;
            out.extend_from_slice(&plain[..usize::try_from(plain_len).unwrap_or(0)]);
        }
        Ok(out)
    }

    /// One chunk's plaintext, trimmed. Used by rotation, which re-encrypts a record
    /// without assembling it first.
    pub(crate) fn read_chunk_plain(
        &self,
        kind: Kind,
        id: &str,
        record: &BlobRecord,
        index: usize,
    ) -> Result<Zeroizing<Vec<u8>>, String> {
        let mut file = File::open(&self.path).map_err(|e| format!("cannot open the box: {e}"))?;
        let count = u32::try_from(record.chunks.len()).map_err(|_| "too many chunks")?;
        let chunk = record
            .chunks
            .get(index)
            .ok_or_else(|| "missing chunk".to_owned())?;
        let plain_len =
            (record.size - index as u64 * container::CHUNK as u64).min(container::CHUNK as u64);
        let mut plain = self.chunk(
            &mut file,
            kind,
            id,
            chunk,
            u32::try_from(index).map_err(|_| "too many chunks")?,
            count,
            plain_len,
        )?;
        plain.truncate(usize::try_from(plain_len).unwrap_or(0));
        Ok(plain)
    }

    pub(crate) fn read_range(
        &self,
        kind: Kind,
        id: &str,
        record: &BlobRecord,
        start: u64,
        len: u64,
    ) -> Result<Zeroizing<Vec<u8>>, String> {
        if start >= record.size || len == 0 {
            return Ok(Zeroizing::new(Vec::new()));
        }
        let end = start.saturating_add(len).min(record.size);
        let mut out = Zeroizing::new(Vec::with_capacity(
            usize::try_from(end - start).unwrap_or(0),
        ));
        let mut file = File::open(&self.path).map_err(|e| format!("cannot open the box: {e}"))?;
        let count = u32::try_from(record.chunks.len()).map_err(|_| "too many chunks")?;
        let chunk_size = container::CHUNK as u64;
        let first = start / chunk_size;
        let last = (end.saturating_sub(1)) / chunk_size;
        for index in first..=last {
            let index_u32 = u32::try_from(index).map_err(|_| "too many chunks")?;
            let Some(chunk) = record.chunks.get(index as usize) else {
                break;
            };
            let plain_len = (record.size - index * chunk_size).min(chunk_size);
            let plain = self.chunk(&mut file, kind, id, chunk, index_u32, count, plain_len)?;
            let from = start.saturating_sub(index * chunk_size).min(plain_len);
            let to = (end - index * chunk_size).min(plain_len);
            out.extend_from_slice(
                &plain[usize::try_from(from).unwrap_or(0)..usize::try_from(to).unwrap_or(0)],
            );
        }
        Ok(out)
    }

    pub(crate) fn copy_record_to(
        &self,
        kind: Kind,
        id: &str,
        record: &BlobRecord,
        out: &mut impl Write,
    ) -> Result<(), String> {
        if record.size == 0 {
            return Ok(());
        }
        let mut file = File::open(&self.path).map_err(|e| format!("cannot open the box: {e}"))?;
        let count = u32::try_from(record.chunks.len()).map_err(|_| "too many chunks")?;
        let chunk_size = container::CHUNK as u64;
        for (index, chunk) in record.chunks.iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| "too many chunks")?;
            let plain_len = (record.size - u64::from(index) * chunk_size).min(chunk_size);
            let plain = self.chunk(&mut file, kind, id, chunk, index, count, plain_len)?;
            out.write_all(&plain[..usize::try_from(plain_len).unwrap_or(0)])
                .map_err(|e| format!("cannot write the file: {e}"))?;
        }
        Ok(())
    }
}
