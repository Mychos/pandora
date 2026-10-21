//! The on-disk contract of a v4 box: header, chunked records, encrypted manifest.

pub(crate) const MAGIC: &[u8] = b"PANDORA4\n";

/// Fixed so the header can be read and patched without knowing the manifest's size.
pub(crate) const HEADER_BLOCK: usize = 4096;

/// 1 MiB keeps random reads cheap without an enormous manifest index.
pub(crate) const CHUNK: usize = 1024 * 1024;

/// Part of the chunk AAD: a note chunk cannot pass as a file chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    File,
    Note,
}

impl Kind {
    fn tag(self) -> u8 {
        match self {
            Kind::File => 1,
            Kind::Note => 2,
        }
    }
}

/// AAD for one chunk: id, kind, index, count and length bind it to its record and
/// position.
pub(crate) fn chunk_aad(
    box_id: &str,
    kind: Kind,
    id: &str,
    index: u32,
    count: u32,
    plain_len: u64,
) -> Vec<u8> {
    format!(
        "pandora4-chunk|{box_id}|{}|{id}|{index}|{count}|{plain_len}",
        kind.tag()
    )
    .into_bytes()
}
