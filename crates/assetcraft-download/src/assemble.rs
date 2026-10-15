//! Reassembling an asset's files from its chunks.
//!
//! A manifest's file is not a single blob: it is cut from one or more chunk
//! parts, each naming a chunk, an offset into that chunk's *decompressed* bytes,
//! a length, and where the slice belongs in the output. Reassembly gathers the
//! chunks, splices the parts into place, and checks the result against the
//! SHA-1 the manifest records for the file — the integrity check the download
//! engine's definition of done rests on.

use assetcraft_core::Result;
use sha1::{Digest, Sha1};

use crate::binary::corrupt;
use crate::chunk::ChunkStore;
use crate::manifest::{FileEntry, Manifest};

/// A file reassembled from chunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembledFile {
    /// The file's path within the install.
    pub filename: String,
    /// The reassembled bytes.
    pub data: Vec<u8>,
}

impl Manifest {
    /// Reassemble one file from its chunk parts and verify its SHA-1.
    ///
    /// Every chunk a part names must already be in `chunks`, and every part must
    /// fit both its chunk and the output file.
    pub fn assemble_file(&self, file: &FileEntry, chunks: &ChunkStore) -> Result<Vec<u8>> {
        let mut out = vec![0u8; file.size as usize];
        for part in &file.chunk_parts {
            let chunk = chunks.get(&part.guid).ok_or_else(|| {
                corrupt(format!(
                    "file {:?} needs chunk {} which is not in the store",
                    file.filename,
                    guid_string(&part.guid)
                ))
            })?;

            let source_start = part.offset as usize;
            let source_end = source_start
                .checked_add(part.size as usize)
                .ok_or_else(|| corrupt("chunk part range overflow"))?;
            if source_end > chunk.len() {
                return Err(corrupt(format!(
                    "chunk part for {:?} reads {source_end} bytes from a {} byte chunk",
                    file.filename,
                    chunk.len()
                )));
            }

            let dest_start = part.file_offset as usize;
            let dest_end = dest_start
                .checked_add(part.size as usize)
                .ok_or_else(|| corrupt("file offset overflow"))?;
            if dest_end > out.len() {
                return Err(corrupt(format!(
                    "chunk part for {:?} writes past the end of the file",
                    file.filename
                )));
            }

            out[dest_start..dest_end].copy_from_slice(&chunk[source_start..source_end]);
        }

        let digest = Sha1::digest(&out);
        if digest.as_slice() != file.sha1 {
            return Err(corrupt(format!(
                "file {:?} does not match its SHA-1",
                file.filename
            )));
        }
        Ok(out)
    }

    /// Reassemble every file the manifest lists, in order.
    pub fn assemble(&self, chunks: &ChunkStore) -> Result<Vec<AssembledFile>> {
        self.files
            .iter()
            .map(|file| {
                let data = self.assemble_file(file, chunks)?;
                Ok(AssembledFile {
                    filename: file.filename.clone(),
                    data,
                })
            })
            .collect()
    }
}

/// A GUID as the usual hyphenated lowercase hex.
fn guid_string(guid: &[u32; 4]) -> String {
    guid.iter()
        .map(|word| format!("{word:08x}"))
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::tests::chunk_file;
    use crate::manifest::tests::manifest_with_file;
    use assetcraft_core::Error;

    /// A SHA-1 to hand to a manifest that will not reach the hash check.
    fn unchecked_hash() -> [u8; 20] {
        [0u8; 20]
    }

    #[test]
    fn assembles_a_file_from_two_chunk_parts() {
        let first = b"AAAA".to_vec();
        let second = b"BBBBBBBB".to_vec();
        let expected = b"AAAABBBBBB".to_vec();
        let hash: [u8; 20] = Sha1::digest(&expected).into();

        let parts = [([1, 0, 0, 0], 0, 4), ([2, 0, 0, 0], 2, 6)];
        let manifest = Manifest::parse(&manifest_with_file(&parts, hash)).unwrap();

        let mut chunks = ChunkStore::new();
        chunks
            .insert_file(&chunk_file([1, 0, 0, 0], &first, true))
            .unwrap();
        chunks
            .insert_file(&chunk_file([2, 0, 0, 0], &second, false))
            .unwrap();

        let files = manifest.assemble(&chunks).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "Content/rock.uasset");
        assert_eq!(files[0].data, expected);
    }

    #[test]
    fn a_missing_chunk_is_reported() {
        let parts = [([7, 0, 0, 0], 0, 4)];
        let manifest = Manifest::parse(&manifest_with_file(&parts, unchecked_hash())).unwrap();
        let err = manifest.assemble(&ChunkStore::new()).unwrap_err();
        assert!(
            matches!(err, Error::Artifact(ref message) if message.contains("not in the store")),
            "got {err:?}"
        );
    }

    #[test]
    fn a_part_that_reads_past_its_chunk_is_reported() {
        let parts = [([1, 0, 0, 0], 2, 8)];
        let manifest = Manifest::parse(&manifest_with_file(&parts, unchecked_hash())).unwrap();
        let mut chunks = ChunkStore::new();
        chunks.insert([1, 0, 0, 0], b"short".to_vec());
        let err = manifest.assemble(&chunks).unwrap_err();
        assert!(
            matches!(err, Error::Artifact(ref message) if message.contains("byte chunk")),
            "got {err:?}"
        );
    }

    #[test]
    fn a_wrong_hash_is_reported() {
        let parts = [([1, 0, 0, 0], 0, 4)];
        let good = Sha1::digest(b"GOOD");
        let manifest = Manifest::parse(&manifest_with_file(&parts, good.into())).unwrap();
        let mut chunks = ChunkStore::new();
        chunks.insert([1, 0, 0, 0], b"XXXX".to_vec());
        let err = manifest.assemble(&chunks).unwrap_err();
        assert!(
            matches!(err, Error::Artifact(ref message) if message.contains("SHA-1")),
            "got {err:?}"
        );
    }
}
