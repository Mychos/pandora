//! The plaintext header of a v4 box: what it says, how its params are bounded, and
//! how the manifest behind it is opened.

use crate::vault::codec;
use crate::vault::container::{HEADER_BLOCK, MAGIC};
use crate::vault::crypto::{self, KdfParams, Key};
use crate::vault::model::Vault;
use crate::vault::reader::BoxReader;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use zeroize::Zeroizing;

use super::CURRENT_VERSION;

/// Four times what this app writes. The params sit in the plaintext header and
/// only authenticate after derivation, so an unchecked `m_cost` can allocate gigabytes.
pub(super) const MAX_M_COST: u32 = crypto::M_COST * 4;
pub(super) const MAX_T_COST: u32 = crypto::T_COST * 4;
pub(super) const MAX_P_COST: u32 = crypto::P_COST * 4;

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
pub(super) struct V4Header {
    pub(super) version: u32,
    pub(super) kdf: KdfHeader,
    pub(super) box_id: String,
    pub(super) manifest_offset: u64,
    pub(super) manifest_len: u64,
    pub(super) manifest_nonce: String,
}

pub(super) fn v4_aad(header: &V4Header) -> Vec<u8> {
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

pub(super) fn header_block(header: &V4Header) -> Result<[u8; HEADER_BLOCK], String> {
    let json = serde_json::to_vec(header).map_err(|e| format!("serialize header: {e}"))?;
    if MAGIC.len() + json.len() > HEADER_BLOCK {
        return Err("header too large".to_owned());
    }
    let mut block = [b' '; HEADER_BLOCK];
    block[..MAGIC.len()].copy_from_slice(MAGIC);
    block[MAGIC.len()..MAGIC.len() + json.len()].copy_from_slice(&json);
    Ok(block)
}

pub(super) fn read_header(path: &Path) -> Result<Option<V4Header>, String> {
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

pub(super) fn decrypt_manifest(path: &Path, header: &V4Header, key: &Key) -> Result<Vault, String> {
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

pub(super) fn header(params: &KdfParams, salt: &[u8]) -> KdfHeader {
    KdfHeader {
        algorithm: "argon2id".to_owned(),
        m_cost: params.m_cost,
        t_cost: params.t_cost,
        p_cost: params.p_cost,
        salt: codec::encode(salt),
    }
}
