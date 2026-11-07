//! The first run against a real Fab manifest: the module docs asked for it.
//!
//! `real-manifest.bin` and `real-chunk.bin` were captured from the live API:
//! the artifact manifest for an owned asset and one chunk fetched from the
//! CDN's signed distribution point. They pin the readers against reality —
//! the thing synthetic fixtures could not prove.

use assetcraft_download::chunk::Chunk;
use assetcraft_download::manifest::Manifest;
use sha1::{Digest, Sha1};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn parses_a_manifest_captured_from_the_live_api() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/real-manifest.bin"
    ))
    .unwrap();
    let manifest = Manifest::parse(&bytes).expect("real manifest parses");
    assert_eq!(manifest.files.len(), 667);
    assert_eq!(manifest.chunks.len(), 6914);
    let file = manifest
        .files
        .iter()
        .find(|f| f.filename.ends_with("E_EnclosureLevel.uasset"))
        .expect("file present");
    assert_eq!(file.size, 2908);
    assert_eq!(hex(&file.sha1), "010b6c110d06c5ef594d93849a9d0bcdec18c44c");
    let part = file.chunk_parts.first().unwrap();
    assert_eq!(part.offset, 747157);
    let chunk = manifest.chunk_by_guid(&part.guid).expect("chunk present");
    assert_eq!(
        chunk.relative_path(manifest.version),
        "ChunksV4/49/003111F8F8448D0E_8B076CB9CD4B528342EC89BF4F868A02.chunk"
    );
}

#[test]
fn decodes_a_chunk_captured_from_the_live_cdn() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/real-chunk.bin"
    ))
    .unwrap();
    let chunk = Chunk::decode(&bytes).expect("real chunk decodes");
    assert_eq!(chunk.data().len(), 1_048_576);
    assert_eq!(
        hex(&Sha1::digest(chunk.data())),
        "425ef025ef14e6979e8e1c81513ac2195b7eb75c"
    );
}

#[test]
fn the_manifest_and_chunk_agree_on_one_real_file() {
    // The end-to-end promise of the two readers: the manifest's part points
    // at bytes of the decoded chunk whose SHA-1 is the manifest's file hash.
    let manifest_bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/real-manifest.bin"
    ))
    .unwrap();
    let chunk_bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/real-chunk.bin"
    ))
    .unwrap();
    let manifest = Manifest::parse(&manifest_bytes).unwrap();
    let file = manifest
        .files
        .iter()
        .find(|f| f.filename.ends_with("E_EnclosureLevel.uasset"))
        .unwrap();
    let part = file.chunk_parts.first().unwrap();
    let chunk = Chunk::decode(&chunk_bytes).unwrap();
    let slice = &chunk.data()[part.offset as usize..part.offset as usize + part.size as usize];
    assert_eq!(hex(&Sha1::digest(slice)), hex(&file.sha1));
}
