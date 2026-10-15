//! Decoder for Epic's chunk files.
//!
//! A manifest's chunks are the compressed blobs actually fetched from the CDN.
//! Each chunk file is a small header followed by the chunk's bytes, usually
//! zlib-compressed, and a manifest's chunk parts slice the *decompressed* bytes
//! at an offset. This module reads the header and exposes the decompressed data,
//! and [`ChunkStore`] collects chunks by GUID for reassembly.
//!
//! Like [`crate::manifest`], the layout is written from the format the wider
//! Epic tooling ecosystem documents and is pinned by a hand-built fixture, but
//! it has not been validated against a chunk from a real Fab account. Encrypted
//! chunks (`stored_as & 0x2`) are rejected rather than guessed at.

use std::collections::HashMap;

use assetcraft_core::Result;

use crate::binary::{corrupt, inflate, Reader};

/// The header magic that marks an Epic chunk file.
pub const CHUNK_MAGIC: u32 = 0xB1FE3AA2;

/// `stored_as` flag: the chunk body is zlib-compressed.
const FLAG_COMPRESSED: u8 = 0x1;
/// `stored_as` flag: the chunk body is encrypted. Unsupported.
const FLAG_ENCRYPTED: u8 = 0x2;

/// A decoded chunk file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The chunk header's version, which gates the optional fields below.
    pub header_version: u32,
    /// The header size the chunk declared.
    pub header_size: u32,
    /// The declared size of the compressed body.
    pub compressed_size: u32,
    /// The chunk's GUID.
    pub guid: [u32; 4],
    /// A 64-bit hash.
    pub hash: u64,
    /// The raw `stored_as` flags.
    pub stored_as: u8,
    /// The SHA-1 of the decompressed data, present from header version 2.
    pub sha_hash: Option<[u8; 20]>,
    /// Which hashes the header carries, present from header version 2.
    pub hash_type: Option<u8>,
    /// The decompressed size, present from header version 3.
    pub uncompressed_size: Option<u32>,
    /// Per-chunk secret GUID, present from header version 4.
    pub secret_guid: Option<[u32; 4]>,
    /// Encryption tag, present from header version 4.
    pub encryption_tag: Option<[u8; 16]>,
    data: Vec<u8>,
}

impl Chunk {
    /// Decode a chunk file from its raw bytes.
    ///
    /// The body is zlib-decompressed when the header says it is compressed. An
    /// encrypted chunk is rejected.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);

        let magic = reader.u32()?;
        if magic != CHUNK_MAGIC {
            return Err(corrupt(format!(
                "bad chunk magic {magic:#010X}, expected {CHUNK_MAGIC:#010X}"
            )));
        }
        let header_version = reader.u32()?;
        let header_size = reader.u32()?;
        let compressed_size = reader.u32()?;
        let guid = reader.guid()?;
        let hash = reader.u64()?;
        let stored_as = reader.u8()?;
        let (sha_hash, hash_type) = if header_version >= 2 {
            (Some(reader.array::<20>()?), Some(reader.u8()?))
        } else {
            (None, None)
        };
        let uncompressed_size = if header_version >= 3 {
            Some(reader.u32()?)
        } else {
            None
        };
        let (secret_guid, encryption_tag) = if header_version >= 4 {
            (Some(reader.guid()?), Some(reader.array::<16>()?))
        } else {
            (None, None)
        };

        if reader.pos() as u32 != header_size {
            tracing::warn!(
                read = reader.pos(),
                header_size,
                "chunk header is not the declared size; skipping to it"
            );
            reader.seek(header_size as usize)?;
        }
        if stored_as & FLAG_ENCRYPTED != 0 {
            return Err(corrupt(
                "chunk is encrypted, which AssetCraft cannot read".to_string(),
            ));
        }

        let remaining = reader.remaining();
        let body = reader.take(remaining)?;
        let data = if stored_as & FLAG_COMPRESSED != 0 {
            inflate(body)?
        } else {
            body.to_vec()
        };
        if let Some(expected) = uncompressed_size {
            if expected as usize != data.len() {
                tracing::warn!(
                    declared = expected,
                    actual = data.len(),
                    "chunk is not the declared uncompressed size"
                );
            }
        }

        Ok(Self {
            header_version,
            header_size,
            compressed_size,
            guid,
            hash,
            stored_as,
            sha_hash,
            hash_type,
            uncompressed_size,
            secret_guid,
            encryption_tag,
            data,
        })
    }

    /// The decompressed chunk bytes.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Whether the body was zlib-compressed on the wire.
    pub fn compressed(&self) -> bool {
        self.stored_as & FLAG_COMPRESSED != 0
    }

    /// Whether the body was encrypted. Always false for a decoded chunk, since
    /// encrypted chunks are rejected.
    pub fn encrypted(&self) -> bool {
        self.stored_as & FLAG_ENCRYPTED != 0
    }

    /// The chunk's GUID as the usual hyphenated lowercase hex.
    pub fn guid_string(&self) -> String {
        self.guid
            .iter()
            .map(|word| format!("{word:08x}"))
            .collect::<Vec<_>>()
            .join("-")
    }
}

/// Decompressed chunk data, keyed by GUID, for reassembling files.
#[derive(Debug, Default)]
pub struct ChunkStore {
    chunks: HashMap<[u32; 4], Vec<u8>>,
}

impl ChunkStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store already-decompressed chunk bytes under `guid`.
    pub fn insert(&mut self, guid: [u32; 4], data: Vec<u8>) {
        self.chunks.insert(guid, data);
    }

    /// Decode a chunk file and store its decompressed bytes under its own GUID.
    ///
    /// Returns the GUID the chunk was stored under.
    pub fn insert_file(&mut self, bytes: &[u8]) -> Result<[u32; 4]> {
        let chunk = Chunk::decode(bytes)?;
        let guid = chunk.guid;
        self.chunks.insert(guid, chunk.data);
        Ok(guid)
    }

    /// The decompressed bytes of a chunk, if it is present.
    pub fn get(&self, guid: &[u32; 4]) -> Option<&[u8]> {
        self.chunks.get(guid).map(Vec::as_slice)
    }

    /// How many chunks the store holds.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use assetcraft_core::Error;
    use std::io::Write;

    /// Build a chunk file the way the format lays it out.
    pub(crate) fn chunk_file(guid: [u32; 4], payload: &[u8], compress: bool) -> Vec<u8> {
        let body = if compress {
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(payload).unwrap();
            encoder.finish().unwrap()
        } else {
            payload.to_vec()
        };

        let mut header = Vec::new();
        header.extend_from_slice(&CHUNK_MAGIC.to_le_bytes());
        header.extend_from_slice(&3u32.to_le_bytes()); // header_version
        header.extend_from_slice(&0u32.to_le_bytes()); // header_size placeholder
        header.extend_from_slice(&(body.len() as u32).to_le_bytes()); // compressed_size
        for word in guid {
            header.extend_from_slice(&word.to_le_bytes());
        }
        header.extend_from_slice(&0u64.to_le_bytes()); // hash
        let stored_as = if compress { FLAG_COMPRESSED } else { 0 };
        header.push(stored_as);
        header.extend_from_slice(&[0x11; 20]); // sha_hash
        header.push(0x3); // hash_type
        header.extend_from_slice(&(payload.len() as u32).to_le_bytes()); // uncompressed_size
        let header_size = header.len() as u32;
        header[8..12].copy_from_slice(&header_size.to_le_bytes());

        header.extend_from_slice(&body);
        header
    }

    #[test]
    fn decodes_a_compressed_chunk() {
        let payload = b"the quick brown fox jumps over the lazy dog".repeat(4);
        let bytes = chunk_file([9, 8, 7, 6], &payload, true);
        let chunk = Chunk::decode(&bytes).unwrap();

        assert_eq!(chunk.guid, [9, 8, 7, 6]);
        assert_eq!(chunk.header_version, 3);
        assert!(chunk.compressed());
        assert!(!chunk.encrypted());
        assert_eq!(chunk.data(), payload.as_slice());
        assert_eq!(chunk.uncompressed_size, Some(payload.len() as u32));
        assert_eq!(chunk.sha_hash, Some([0x11; 20]));
        assert_eq!(chunk.guid_string(), "00000009-00000008-00000007-00000006");
    }

    #[test]
    fn decodes_an_uncompressed_chunk() {
        let payload = b"plain bytes";
        let bytes = chunk_file([1, 1, 1, 1], payload, false);
        let chunk = Chunk::decode(&bytes).unwrap();
        assert!(!chunk.compressed());
        assert_eq!(chunk.data(), payload.as_slice());
    }

    #[test]
    fn rejects_a_foreign_chunk_magic() {
        let mut bytes = chunk_file([1, 2, 3, 4], b"data", true);
        bytes[0..4].copy_from_slice(&0xDEADBEEFu32.to_le_bytes());
        let err = Chunk::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::Artifact(ref message) if message.contains("bad chunk magic")),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_an_encrypted_chunk() {
        let mut bytes = chunk_file([1, 2, 3, 4], b"data", true);
        // stored_as follows the 16-byte GUID and 8-byte hash: 4+4+4+4+16+8.
        bytes[40] |= FLAG_ENCRYPTED;
        let err = Chunk::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::Artifact(ref message) if message.contains("encrypted")),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_a_truncated_chunk() {
        let bytes = chunk_file([1, 2, 3, 4], b"data", true);
        assert!(Chunk::decode(&bytes[..10]).is_err());
    }

    #[test]
    fn a_store_indexes_chunks_by_guid() {
        let mut store = ChunkStore::new();
        assert!(store.is_empty());
        let guid = store
            .insert_file(&chunk_file([5, 5, 5, 5], b"payload", true))
            .unwrap();
        assert_eq!(guid, [5, 5, 5, 5]);
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(&[5, 5, 5, 5]), Some(b"payload".as_slice()));
        assert!(store.get(&[0, 0, 0, 0]).is_none());
    }
}
