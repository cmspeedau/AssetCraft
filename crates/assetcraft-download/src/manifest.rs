//! Parser for Fab's signed binary artifact manifest.
//!
//! When you ask Fab for the files of an owned asset it answers with a signed
//! *binary* manifest in Epic's chunked format (header magic `0x44BEC00C`). The
//! manifest is the index of the download: a list of chunks (the compressed
//! blobs that are actually fetched) and a list of files (the logical outputs),
//! each file assembled from one or more *chunk parts* that point into chunks.
//!
//! This module reads that format into the domain types below. It is a faithful
//! port of the layout documented by the wider Epic tooling ecosystem
//! (`legendary`'s `manifest.py`, `meszmate/manifest`); the byte layout is
//! pinned by a hand-built fixture in the tests rather than by a writer we also
//! wrote, so the reader is checked against the specification, not against
//! itself. That fixture was cross-checked field by field against `legendary`'s
//! reader, which agrees on every value.
//!
//! **It has not been validated against a manifest from a real Fab account.**
//! The shape is documented and internally consistent, but a field whose meaning
//! Fab changed could still surprise us; treat a first run against a real
//! manifest as a validation step. Encrypted manifests (`stored_as & 0x2`, used
//! by some newer builds) are rejected with a clear error rather than guessed at.
//!
//! Reading a manifest resolves an asset into *what to fetch*: [`Manifest::chunks`]
//! gives the blobs to download and [`ChunkInfo::relative_path`] their path under
//! the CDN base, and [`Manifest::files`] gives the outputs and how each is cut
//! from chunks. Reassembling chunks into files (and the CDN base itself) is a
//! separate step and is not done here.

use std::collections::BTreeMap;

use assetcraft_core::{Error, Result};
use sha1::{Digest, Sha1};

use crate::binary::{corrupt, inflate, Reader};

/// The header magic that marks an Epic binary manifest.
pub const HEADER_MAGIC: u32 = 0x44BEC00C;

/// `stored_as` flag: the body is zlib-compressed.
const FLAG_COMPRESSED: u8 = 0x1;
/// `stored_as` flag: the body is encrypted. Unsupported.
const FLAG_ENCRYPTED: u8 = 0x2;

/// File flag: read-only after install.
const FILE_FLAG_READ_ONLY: u8 = 0x1;
/// File flag: stored compressed.
const FILE_FLAG_COMPRESSED: u8 = 0x2;
/// File flag: executable.
const FILE_FLAG_EXECUTABLE: u8 = 0x4;

/// A parsed binary manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The serialisation version from the header (feature level).
    pub version: u32,
    /// The header size the manifest declared.
    pub header_size: u32,
    /// The declared uncompressed body size.
    pub size_uncompressed: u32,
    /// The declared on-the-wire body size.
    pub size_compressed: u32,
    /// The SHA-1 of the uncompressed body, as recorded in the header.
    pub sha_hash: [u8; 20],
    /// The raw `stored_as` flags.
    pub stored_as: u8,
    /// Build metadata.
    pub meta: ManifestMeta,
    /// The chunks to download.
    pub chunks: Vec<ChunkInfo>,
    /// The logical files, each cut from chunk parts.
    pub files: Vec<FileEntry>,
    /// Free-form key/value metadata.
    pub custom_fields: BTreeMap<String, String>,
}

impl Manifest {
    /// Parse a manifest from its raw bytes.
    ///
    /// The body is zlib-decompressed when the header says it is compressed, and
    /// its SHA-1 is checked against the header hash, matching the reference
    /// implementation. An encrypted manifest is rejected.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(data);

        let magic = reader.u32()?;
        if magic != HEADER_MAGIC {
            return Err(corrupt(format!(
                "bad magic {magic:#010X}, expected {HEADER_MAGIC:#010X}"
            )));
        }
        let header_size = reader.u32()?;
        let size_uncompressed = reader.u32()?;
        let size_compressed = reader.u32()?;
        let sha_hash = reader.array::<20>()?;
        let stored_as = reader.u8()?;
        let version = reader.u32()?;
        // Version 22 and up carry a per-manifest secret GUID and encryption tag.
        if version >= 22 {
            let _secret_guid = reader.guid()?;
            let _encryption_tag = reader.array::<16>()?;
        }

        if reader.pos() as u32 != header_size {
            tracing::warn!(
                read = reader.pos(),
                header_size,
                "manifest header is not the declared size; skipping to it"
            );
            reader.seek(header_size as usize)?;
        }

        if stored_as & FLAG_ENCRYPTED != 0 {
            return Err(Error::Artifact(
                "manifest is encrypted, which AssetCraft cannot read".to_string(),
            ));
        }

        let remaining = reader.remaining();
        let raw_body = reader.take(remaining)?;
        let body = if stored_as & FLAG_COMPRESSED != 0 {
            let inflated = inflate(raw_body)?;
            let digest = Sha1::digest(&inflated);
            if digest.as_slice() != sha_hash {
                return Err(corrupt("decompressed body does not match its SHA-1"));
            }
            inflated
        } else {
            raw_body.to_vec()
        };
        if size_uncompressed as usize != body.len() {
            tracing::warn!(
                declared = size_uncompressed,
                actual = body.len(),
                "manifest body is not the declared uncompressed size"
            );
        }

        let mut body_reader = Reader::new(&body);
        let meta = parse_meta(&mut body_reader)?;
        let chunks = parse_chunks(&mut body_reader, meta.feature_level)?;
        let files = parse_files(&mut body_reader)?;
        let custom_fields = parse_custom_fields(&mut body_reader)?;
        if body_reader.remaining() > 0 {
            tracing::warn!(
                remaining = body_reader.remaining(),
                "unread bytes at the end of the manifest body"
            );
        }

        Ok(Self {
            version,
            header_size,
            size_uncompressed,
            size_compressed,
            sha_hash,
            stored_as,
            meta,
            chunks,
            files,
            custom_fields,
        })
    }

    /// Whether the body was zlib-compressed on the wire.
    pub fn compressed(&self) -> bool {
        self.stored_as & FLAG_COMPRESSED != 0
    }

    /// Whether the body was encrypted. Always false for a parsed manifest,
    /// since encrypted manifests are rejected.
    pub fn encrypted(&self) -> bool {
        self.stored_as & FLAG_ENCRYPTED != 0
    }

    /// The chunk with the given GUID, if any.
    pub fn chunk_by_guid(&self, guid: &[u32; 4]) -> Option<&ChunkInfo> {
        self.chunks.iter().find(|chunk| &chunk.guid == guid)
    }

    /// The total size of the installed files.
    pub fn total_install_size(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }

    /// The total number of bytes to download, as the chunks report it.
    pub fn total_download_size(&self) -> u64 {
        self.chunks
            .iter()
            .map(|chunk| chunk.file_size.max(0) as u64)
            .sum()
    }

    /// The chunks to fetch, with their full URL under `base_url`.
    ///
    /// The base URL comes from the distribution point Fab returns; this method
    /// only joins it with each chunk's documented relative path.
    pub fn chunk_downloads(&self, base_url: &str) -> Vec<ChunkDownload> {
        let base = base_url.trim_end_matches('/');
        self.chunks
            .iter()
            .map(|chunk| ChunkDownload {
                guid: chunk.guid_string(),
                url: format!("{base}/{}", chunk.relative_path(self.meta.feature_level)),
                size_bytes: chunk.file_size.max(0) as u64,
                sha1: chunk.sha_hash,
            })
            .collect()
    }
}

/// Build metadata from a manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestMeta {
    /// Version of the metadata block, which gates the optional fields below.
    pub data_version: u8,
    /// The serialisation feature level this manifest targets.
    pub feature_level: u32,
    /// Whether this manifest predates chunks and lists file data directly.
    pub is_file_data: bool,
    /// The application id, usually zero.
    pub app_id: u32,
    /// The application name.
    pub app_name: String,
    /// The build version string.
    pub build_version: String,
    /// The executable to launch, if any.
    pub launch_exe: String,
    /// The launch arguments, if any.
    pub launch_command: String,
    /// Prerequisite ids.
    pub prereq_ids: Vec<String>,
    /// Prerequisite display name.
    pub prereq_name: String,
    /// Prerequisite install path.
    pub prereq_path: String,
    /// Prerequisite install arguments.
    pub prereq_args: String,
    /// The build id, present when `data_version >= 1`.
    pub build_id: String,
    /// Uninstall action path, present when `data_version >= 2`.
    pub uninstall_action_path: String,
    /// Uninstall action arguments, present when `data_version >= 2`.
    pub uninstall_action_args: String,
}

/// A downloadable chunk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChunkInfo {
    /// The chunk's GUID, four 32-bit words.
    pub guid: [u32; 4],
    /// A 64-bit hash used in the chunk's path.
    pub hash: u64,
    /// The SHA-1 of the chunk.
    pub sha_hash: [u8; 20],
    /// The group number, part of the chunk's path.
    pub group_num: u8,
    /// The uncompressed size.
    pub window_size: u32,
    /// The compressed size, which is what gets downloaded.
    pub file_size: i64,
    /// Per-chunk secret GUID, present from version 22.
    pub secret_guid: Option<[u32; 4]>,
    /// Compressed window size, present from version 22.
    pub window_size_compressed: Option<u32>,
    /// Encryption tag, present from version 22.
    pub encryption_tag: Option<[u8; 16]>,
}

impl ChunkInfo {
    /// The chunk's GUID as the usual hyphenated lowercase hex.
    pub fn guid_string(&self) -> String {
        self.guid
            .iter()
            .map(|word| format!("{word:08x}"))
            .collect::<Vec<_>>()
            .join("-")
    }

    /// The directory chunks of this manifest version live under.
    pub fn chunk_dir(manifest_version: u32) -> &'static str {
        if manifest_version >= 22 {
            "ChunksV5"
        } else if manifest_version >= 15 {
            "ChunksV4"
        } else if manifest_version >= 6 {
            "ChunksV3"
        } else if manifest_version >= 3 {
            "ChunksV2"
        } else {
            "Chunks"
        }
    }

    /// The chunk's path relative to the CDN base, as the format lays it out.
    ///
    /// This uses the group number stored in the manifest. `legendary` instead
    /// derives the group from the GUID (CRC-32 modulo 100) when it builds this
    /// path; the two coincide on a well-formed manifest, and the store is what
    /// the format and the Go implementation use.
    pub fn relative_path(&self, manifest_version: u32) -> String {
        if manifest_version >= 22 {
            let secret = self.secret_guid.unwrap_or([0, 0, 0, 0]);
            let secret_part = if secret == [0, 0, 0, 0] {
                "plain".to_string()
            } else {
                base64_url_nopad(&guid_bytes(&secret))
            };
            let hash = base64_url_nopad(&self.hash.to_le_bytes());
            let guid = base64_url_nopad(&guid_bytes(&self.guid));
            format!(
                "{}/{secret_part}/{:02}/{hash}_{guid}.chunk",
                Self::chunk_dir(manifest_version),
                self.group_num
            )
        } else {
            let guid: String = self.guid.iter().map(|word| format!("{word:08X}")).collect();
            format!(
                "{}/{:02}/{:016X}_{guid}.chunk",
                Self::chunk_dir(manifest_version),
                self.group_num,
                self.hash
            )
        }
    }
}

/// A region of a chunk that contributes to a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChunkPart {
    /// The GUID of the chunk this part reads from.
    pub guid: [u32; 4],
    /// The offset into the (decompressed) chunk.
    pub offset: u32,
    /// The number of bytes taken from the chunk.
    pub size: u32,
    /// The offset of this part within the output file.
    pub file_offset: u64,
}

/// A logical file, assembled from chunk parts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileEntry {
    /// The file's path within the install.
    pub filename: String,
    /// A symlink target, if this entry is a symlink.
    pub symlink_target: String,
    /// The file's SHA-1.
    pub sha1: [u8; 20],
    /// The raw file flags.
    pub flags: u8,
    /// Install tags.
    pub install_tags: Vec<String>,
    /// The parts this file is cut from, in order.
    pub chunk_parts: Vec<ChunkPart>,
    /// The assembled size, the sum of the chunk parts.
    pub size: u64,
    /// The file's MD5, present from manifest feature level 19.
    pub md5: Option<[u8; 16]>,
    /// The file's MIME type, present from manifest feature level 19.
    pub mime_type: Option<String>,
    /// The file's SHA-256, present from manifest feature level 20.
    pub sha256: Option<[u8; 32]>,
}

impl FileEntry {
    /// Whether the file is marked read-only.
    pub fn is_read_only(&self) -> bool {
        self.flags & FILE_FLAG_READ_ONLY != 0
    }

    /// Whether the file is stored compressed.
    pub fn is_compressed(&self) -> bool {
        self.flags & FILE_FLAG_COMPRESSED != 0
    }

    /// Whether the file is executable.
    pub fn is_executable(&self) -> bool {
        self.flags & FILE_FLAG_EXECUTABLE != 0
    }
}

/// One chunk to fetch, with the URL it lives at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkDownload {
    /// The chunk's GUID.
    pub guid: String,
    /// The full URL to fetch.
    pub url: String,
    /// The number of bytes to expect.
    pub size_bytes: u64,
    /// The SHA-1 to verify against.
    pub sha1: [u8; 20],
}

/// Parse the metadata block.
fn parse_meta(reader: &mut Reader) -> Result<ManifestMeta> {
    let start = reader.pos();
    let size = reader.u32()? as usize;
    let data_version = reader.u8()?;
    let feature_level = reader.u32()?;
    let is_file_data = reader.u8()? == 1;
    let app_id = reader.u32()?;
    let app_name = reader.fstring()?;
    let build_version = reader.fstring()?;
    let launch_exe = reader.fstring()?;
    let launch_command = reader.fstring()?;

    let prereq_count = reader.u32()? as usize;
    guard_count(reader, prereq_count, 4, "prerequisite")?;
    let mut prereq_ids = Vec::with_capacity(prereq_count);
    for _ in 0..prereq_count {
        prereq_ids.push(reader.fstring()?);
    }
    let prereq_name = reader.fstring()?;
    let prereq_path = reader.fstring()?;
    let prereq_args = reader.fstring()?;

    let build_id = if data_version >= 1 {
        reader.fstring()?
    } else {
        String::new()
    };
    let (uninstall_action_path, uninstall_action_args) = if data_version >= 2 {
        (reader.fstring()?, reader.fstring()?)
    } else {
        (String::new(), String::new())
    };

    finish_section(reader, start, size, "manifest metadata")?;

    Ok(ManifestMeta {
        data_version,
        feature_level,
        is_file_data,
        app_id,
        app_name,
        build_version,
        launch_exe,
        launch_command,
        prereq_ids,
        prereq_name,
        prereq_path,
        prereq_args,
        build_id,
        uninstall_action_path,
        uninstall_action_args,
    })
}

/// Parse the chunk data list.
fn parse_chunks(reader: &mut Reader, manifest_version: u32) -> Result<Vec<ChunkInfo>> {
    let start = reader.pos();
    let size = reader.u32()? as usize;
    let _version = reader.u8()?;
    let count = reader.u32()? as usize;
    guard_count(reader, count, 57, "chunk")?;

    let mut chunks: Vec<ChunkInfo> = (0..count).map(|_| ChunkInfo::default()).collect();
    // The format stores each field as its own array across all chunks.
    for chunk in &mut chunks {
        chunk.guid = reader.guid()?;
    }
    for chunk in &mut chunks {
        chunk.hash = reader.u64()?;
    }
    for chunk in &mut chunks {
        chunk.sha_hash = reader.array::<20>()?;
    }
    for chunk in &mut chunks {
        chunk.group_num = reader.u8()?;
    }
    for chunk in &mut chunks {
        chunk.window_size = reader.u32()?;
    }
    for chunk in &mut chunks {
        chunk.file_size = reader.i64()?;
    }
    if manifest_version >= 22 {
        for chunk in &mut chunks {
            chunk.secret_guid = Some(reader.guid()?);
        }
        for chunk in &mut chunks {
            chunk.window_size_compressed = Some(reader.u32()?);
        }
        for chunk in &mut chunks {
            chunk.encryption_tag = Some(reader.array::<16>()?);
        }
    }

    finish_section(reader, start, size, "chunk data list")?;
    Ok(chunks)
}

/// Parse the file manifest list.
fn parse_files(reader: &mut Reader) -> Result<Vec<FileEntry>> {
    let start = reader.pos();
    let size = reader.u32()? as usize;
    let version = reader.u8()?;
    let count = reader.u32()? as usize;
    guard_count(reader, count, 37, "file")?;

    let mut files: Vec<FileEntry> = (0..count).map(|_| FileEntry::default()).collect();
    for file in &mut files {
        file.filename = reader.fstring()?;
    }
    for file in &mut files {
        file.symlink_target = reader.fstring()?;
    }
    for file in &mut files {
        file.sha1 = reader.array::<20>()?;
    }
    for file in &mut files {
        file.flags = reader.u8()?;
    }
    for file in &mut files {
        let tags = reader.u32()? as usize;
        guard_count(reader, tags, 4, "install tag")?;
        for _ in 0..tags {
            file.install_tags.push(reader.fstring()?);
        }
    }
    for file in &mut files {
        let parts = reader.u32()? as usize;
        guard_count(reader, parts, 4, "chunk part")?;
        let mut file_offset = 0u64;
        for _ in 0..parts {
            let part_start = reader.pos();
            // Each part is preceded by its own size, which is normally 28.
            let part_size = reader.u32()? as usize;
            let guid = reader.guid()?;
            let offset = reader.u32()?;
            let part_length = reader.u32()?;
            let consumed = reader.pos() - part_start;
            if part_size > consumed {
                reader.skip(part_size - consumed)?;
            }
            file.chunk_parts.push(ChunkPart {
                guid,
                offset,
                size: part_length,
                file_offset,
            });
            file_offset += u64::from(part_length);
        }
    }
    if version >= 1 {
        for file in &mut files {
            if reader.u32()? != 0 {
                file.md5 = Some(reader.array::<16>()?);
            }
        }
        for file in &mut files {
            file.mime_type = Some(reader.fstring()?);
        }
    }
    if version >= 2 {
        for file in &mut files {
            file.sha256 = Some(reader.array::<32>()?);
        }
    }
    for file in &mut files {
        file.size = file
            .chunk_parts
            .iter()
            .map(|part| u64::from(part.size))
            .sum();
    }

    finish_section(reader, start, size, "file manifest list")?;
    Ok(files)
}

/// Parse the custom fields block.
fn parse_custom_fields(reader: &mut Reader) -> Result<BTreeMap<String, String>> {
    // A manifest may end after the file list; an absent block is not an error.
    if reader.remaining() == 0 {
        return Ok(BTreeMap::new());
    }

    let start = reader.pos();
    let size = reader.u32()? as usize;
    let _version = reader.u8()?;
    let count = reader.u32()? as usize;
    guard_count(reader, count, 8, "custom field")?;

    let mut keys = Vec::with_capacity(count);
    for _ in 0..count {
        keys.push(reader.fstring()?);
    }
    let mut fields = BTreeMap::new();
    for key in keys {
        fields.insert(key, reader.fstring()?);
    }

    finish_section(reader, start, size, "custom fields")?;
    Ok(fields)
}

/// Check that a declared count cannot exceed the bytes left to read it.
fn guard_count(reader: &Reader, count: usize, per_item_min: usize, what: &str) -> Result<()> {
    let per_item = per_item_min.max(1);
    if count > reader.remaining() / per_item {
        return Err(corrupt(format!(
            "{what} count {count} does not fit in the {} bytes remaining",
            reader.remaining()
        )));
    }
    Ok(())
}

/// Check that a section read exactly its declared size, or skip the difference.
fn finish_section(
    reader: &mut Reader,
    start: usize,
    declared_size: usize,
    what: &str,
) -> Result<()> {
    let end = start
        .checked_add(declared_size)
        .ok_or_else(|| corrupt(format!("{what} declared size overflows")))?;
    let read = reader.pos();
    if read == end {
        return Ok(());
    }
    if read < end {
        // A newer version may add fields we do not know yet. Skip them.
        tracing::warn!(
            what,
            missing = end - read,
            "manifest section is shorter than declared; skipping the remainder"
        );
        reader.seek(end)?;
        return Ok(());
    }
    Err(corrupt(format!(
        "{what} read {} bytes past its declared size",
        read - end
    )))
}

/// The 16 little-endian bytes of a GUID.
fn guid_bytes(guid: &[u32; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (index, word) in guid.iter().enumerate() {
        out[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// Base64 (URL-safe, unpadded) as the version 22 chunk path uses it.
fn base64_url_nopad(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for block in data.chunks(3) {
        let b0 = u32::from(block[0]);
        let b1 = block.get(1).copied().map(u32::from).unwrap_or(0);
        let b2 = block.get(2).copied().map(u32::from).unwrap_or(0);
        let combined = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((combined >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((combined >> 12) & 0x3F) as usize] as char);
        if block.len() > 1 {
            out.push(ALPHABET[((combined >> 6) & 0x3F) as usize] as char);
        }
        if block.len() > 2 {
            out.push(ALPHABET[(combined & 0x3F) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // A hand-built manifest. Every byte is written from the specification, and
    // the section sizes are asserted against values computed by hand, so this
    // pins the layout independently of the reader. The same bytes were parsed
    // with `legendary`'s reader during development, which agreed on every field.
    struct Fixture {
        body: Vec<u8>,
    }

    fn put_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn put_u64(out: &mut Vec<u8>, value: u64) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn put_i64(out: &mut Vec<u8>, value: i64) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn put_guid(out: &mut Vec<u8>, guid: [u32; 4]) {
        for word in guid {
            put_u32(out, word);
        }
    }

    fn put_fstring(out: &mut Vec<u8>, text: &str) {
        if text.is_empty() {
            put_u32(out, 0);
            return;
        }
        put_u32(out, text.len() as u32 + 1);
        out.extend_from_slice(text.as_bytes());
        out.push(0);
    }

    /// Write a section, patching in its total size (including the size field).
    fn section(out: &mut Vec<u8>, write: impl FnOnce(&mut Vec<u8>)) {
        let start = out.len();
        put_u32(out, 0); // placeholder
        write(out);
        let total = (out.len() - start) as u32;
        out[start..start + 4].copy_from_slice(&total.to_le_bytes());
    }

    impl Fixture {
        /// A version 18, uncompressed manifest with one chunk and one file.
        fn simple() -> Self {
            let mut body = Vec::new();

            // Metadata (data_version 0).
            section(&mut body, |out| {
                out.push(0); // data_version
                put_u32(out, 18); // feature_level
                out.push(0); // is_file_data
                put_u32(out, 0); // app_id
                put_fstring(out, "TestApp");
                put_fstring(out, "1.0");
                put_fstring(out, "game.exe");
                put_fstring(out, ""); // launch_command
                put_u32(out, 0); // prereq count
                put_fstring(out, ""); // prereq_name
                put_fstring(out, ""); // prereq_path
                put_fstring(out, ""); // prereq_args
            });

            // Chunk data list (one chunk).
            section(&mut body, |out| {
                out.push(0); // cdl version
                put_u32(out, 1); // count
                put_guid(out, [1, 2, 3, 4]);
                put_u64(out, 0x1122_3344_5566_7788);
                out.extend_from_slice(&[0xAB; 20]); // sha1
                out.push(7); // group_num
                put_u32(out, 1000); // window_size
                put_i64(out, 300); // file_size
            });

            // File manifest list (one file, one chunk part).
            section(&mut body, |out| {
                out.push(0); // fml version
                put_u32(out, 1); // count
                put_fstring(out, "Content/rock.uasset");
                put_fstring(out, ""); // symlink_target
                out.extend_from_slice(&[0xCD; 20]); // sha1
                out.push(0); // flags
                put_u32(out, 0); // install tags
                put_u32(out, 1); // chunk parts
                put_u32(out, 28); // part size
                put_guid(out, [1, 2, 3, 4]);
                put_u32(out, 0); // offset
                put_u32(out, 300); // size
            });

            // Custom fields (empty).
            section(&mut body, |out| {
                out.push(0); // version
                put_u32(out, 0); // count
            });

            Self { body }
        }

        fn body_section_sizes(&self) -> [u32; 4] {
            // Each section's declared size is its first four bytes; the sections
            // follow one another in the body.
            let mut sizes = [0u32; 4];
            let mut pos = 0usize;
            for size in &mut sizes {
                *size = u32::from_le_bytes(self.body[pos..pos + 4].try_into().unwrap());
                pos += *size as usize;
            }
            sizes
        }

        /// The full manifest: a 41-byte header followed by the body.
        fn bytes(&self) -> Vec<u8> {
            let mut out = Vec::new();
            put_u32(&mut out, HEADER_MAGIC);
            put_u32(&mut out, 41); // header_size
            put_u32(&mut out, self.body.len() as u32); // size_uncompressed
            put_u32(&mut out, self.body.len() as u32); // size_compressed
            out.extend_from_slice(&[0u8; 20]); // sha_hash (unchecked when uncompressed)
            out.push(0); // stored_as: uncompressed
            put_u32(&mut out, 18); // version
            out.extend_from_slice(&self.body);
            out
        }
    }

    /// A version-18 manifest with one file assembled from the given chunk parts.
    /// `parts` are `(guid, offset, size)`, and `file_sha1` is the hash the
    /// manifest records for the file. The chunk data list is empty: the chunks
    /// are supplied separately when reassembling.
    pub(crate) fn manifest_with_file(
        parts: &[([u32; 4], u32, u32)],
        file_sha1: [u8; 20],
    ) -> Vec<u8> {
        let mut body = Vec::new();

        section(&mut body, |out| {
            out.push(0);
            put_u32(out, 18);
            out.push(0);
            put_u32(out, 0);
            put_fstring(out, "App");
            put_fstring(out, "1");
            put_fstring(out, "");
            put_fstring(out, "");
            put_u32(out, 0);
            put_fstring(out, "");
            put_fstring(out, "");
            put_fstring(out, "");
        });
        section(&mut body, |out| {
            out.push(0);
            put_u32(out, 0); // no chunks in the manifest itself
        });
        section(&mut body, |out| {
            out.push(0);
            put_u32(out, 1); // one file
            put_fstring(out, "Content/rock.uasset");
            put_fstring(out, "");
            out.extend_from_slice(&file_sha1);
            out.push(0); // flags
            put_u32(out, 0); // install tags
            put_u32(out, parts.len() as u32);
            for (guid, offset, size) in parts {
                put_u32(out, 28);
                put_guid(out, *guid);
                put_u32(out, *offset);
                put_u32(out, *size);
            }
        });
        section(&mut body, |out| {
            out.push(0);
            put_u32(out, 0);
        });

        let mut bytes = Vec::new();
        put_u32(&mut bytes, HEADER_MAGIC);
        put_u32(&mut bytes, 41);
        put_u32(&mut bytes, body.len() as u32);
        put_u32(&mut bytes, body.len() as u32);
        bytes.extend_from_slice(&[0u8; 20]);
        bytes.push(0);
        put_u32(&mut bytes, 18);
        bytes.extend_from_slice(&body);
        bytes
    }

    #[test]
    fn hand_built_section_sizes_match_the_specification() {
        let fixture = Fixture::simple();
        // Computed by hand from the documented layout.
        assert_eq!(fixture.body_section_sizes(), [67, 66, 94, 9]);
        assert_eq!(fixture.body.len(), 236);
        assert_eq!(fixture.bytes().len(), 41 + 236);
    }

    #[test]
    fn parses_a_hand_built_manifest() {
        let manifest = Manifest::parse(&Fixture::simple().bytes()).unwrap();

        assert_eq!(manifest.version, 18);
        assert_eq!(manifest.header_size, 41);
        assert!(!manifest.compressed());
        assert!(!manifest.encrypted());

        assert_eq!(manifest.meta.feature_level, 18);
        assert_eq!(manifest.meta.app_name, "TestApp");
        assert_eq!(manifest.meta.build_version, "1.0");
        assert_eq!(manifest.meta.launch_exe, "game.exe");
        assert!(manifest.meta.prereq_ids.is_empty());
        assert_eq!(manifest.meta.data_version, 0);

        assert_eq!(manifest.chunks.len(), 1);
        let chunk = &manifest.chunks[0];
        assert_eq!(chunk.guid, [1, 2, 3, 4]);
        assert_eq!(chunk.hash, 0x1122_3344_5566_7788);
        assert_eq!(chunk.sha_hash, [0xAB; 20]);
        assert_eq!(chunk.group_num, 7);
        assert_eq!(chunk.window_size, 1000);
        assert_eq!(chunk.file_size, 300);
        assert_eq!(chunk.guid_string(), "00000001-00000002-00000003-00000004");

        assert_eq!(manifest.files.len(), 1);
        let file = &manifest.files[0];
        assert_eq!(file.filename, "Content/rock.uasset");
        assert_eq!(file.sha1, [0xCD; 20]);
        assert_eq!(file.flags, 0);
        assert_eq!(file.size, 300);
        assert_eq!(file.chunk_parts.len(), 1);
        assert_eq!(file.chunk_parts[0].file_offset, 0);
        assert_eq!(file.chunk_parts[0].size, 300);
        assert!(file.md5.is_none());
        assert!(file.sha256.is_none());

        assert!(manifest.custom_fields.is_empty());
        assert_eq!(manifest.total_install_size(), 300);
        assert_eq!(manifest.total_download_size(), 300);

        assert_eq!(
            manifest.chunk_by_guid(&[1, 2, 3, 4]).map(|c| c.group_num),
            Some(7)
        );
        assert!(manifest.chunk_by_guid(&[9, 9, 9, 9]).is_none());
    }

    #[test]
    fn chunk_relative_path_uses_the_documented_layout() {
        let manifest = Manifest::parse(&Fixture::simple().bytes()).unwrap();
        let chunk = &manifest.chunks[0];
        assert_eq!(
            chunk.relative_path(manifest.meta.feature_level),
            "ChunksV4/07/1122334455667788_00000001000000020000000300000004.chunk"
        );

        let downloads = manifest.chunk_downloads("https://cdn.example.com/build/");
        assert_eq!(downloads.len(), 1);
        assert_eq!(
            downloads[0].url,
            "https://cdn.example.com/build/ChunksV4/07/1122334455667788_00000001000000020000000300000004.chunk"
        );
        assert_eq!(downloads[0].size_bytes, 300);
        assert_eq!(downloads[0].sha1, [0xAB; 20]);
    }

    #[test]
    fn chunk_dir_follows_the_version() {
        assert_eq!(ChunkInfo::chunk_dir(2), "Chunks");
        assert_eq!(ChunkInfo::chunk_dir(3), "ChunksV2");
        assert_eq!(ChunkInfo::chunk_dir(6), "ChunksV3");
        assert_eq!(ChunkInfo::chunk_dir(15), "ChunksV4");
        assert_eq!(ChunkInfo::chunk_dir(22), "ChunksV5");
    }

    #[test]
    fn version_22_paths_use_base64_and_a_secret_part() {
        let chunk = ChunkInfo {
            guid: [1, 2, 3, 4],
            hash: 0x1122_3344_5566_7788,
            group_num: 5,
            secret_guid: Some([0, 0, 0, 0]),
            ..Default::default()
        };
        assert!(chunk.relative_path(22).starts_with("ChunksV5/plain/05/"));
        assert!(chunk.relative_path(22).ends_with(".chunk"));
    }

    #[test]
    fn base64_url_is_unpadded_and_url_safe() {
        // "M" -> "TQ", and bytes that would be +/ become -_.
        assert_eq!(base64_url_nopad(b"M"), "TQ");
        assert_eq!(base64_url_nopad(&[0xFB, 0xFF, 0xBF]), "-_-_");
    }

    #[test]
    fn fstring_reads_ascii_and_empty() {
        let mut bytes = Vec::new();
        put_fstring(&mut bytes, "");
        put_fstring(&mut bytes, "hello");
        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.fstring().unwrap(), "");
        assert_eq!(reader.fstring().unwrap(), "hello");
    }

    #[test]
    fn rejects_a_foreign_magic() {
        let mut bytes = Fixture::simple().bytes();
        bytes[0..4].copy_from_slice(&0xDEADBEEFu32.to_le_bytes());
        let err = Manifest::parse(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::Artifact(ref message) if message.contains("bad magic")),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_an_encrypted_manifest() {
        let mut bytes = Fixture::simple().bytes();
        bytes[36] |= FLAG_ENCRYPTED; // stored_as
        let err = Manifest::parse(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::Artifact(ref message) if message.contains("encrypted")),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_a_truncated_manifest() {
        let bytes = Fixture::simple().bytes();
        let err = Manifest::parse(&bytes[..50]).unwrap_err();
        assert!(matches!(err, Error::Artifact(_)), "got {err:?}");
    }

    #[test]
    fn a_count_that_cannot_fit_is_rejected() {
        // Corrupt the chunk count to something absurd.
        let mut bytes = Fixture::simple().bytes();
        // Body starts at 41; the metadata section is 67 bytes; the chunk count
        // is at body + 67 + 4 (size) + 1 (version).
        let count_at = 41 + 67 + 4 + 1;
        bytes[count_at..count_at + 4].copy_from_slice(&1_000_000u32.to_le_bytes());
        let err = Manifest::parse(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::Artifact(ref message) if message.contains("does not fit")),
            "got {err:?}"
        );
    }

    #[test]
    fn reads_a_compressed_manifest_and_verifies_its_hash() {
        // Compress the hand-built body and record the real SHA-1.
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        let fixture = Fixture::simple();
        let digest = Sha1::digest(&fixture.body);

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&fixture.body).unwrap();
        let compressed = encoder.finish().unwrap();

        let mut bytes = Vec::new();
        put_u32(&mut bytes, HEADER_MAGIC);
        put_u32(&mut bytes, 41);
        put_u32(&mut bytes, fixture.body.len() as u32);
        put_u32(&mut bytes, compressed.len() as u32);
        bytes.extend_from_slice(&digest);
        bytes.push(FLAG_COMPRESSED);
        put_u32(&mut bytes, 18);
        bytes.extend_from_slice(&compressed);

        let manifest = Manifest::parse(&bytes).unwrap();
        assert!(manifest.compressed());
        assert_eq!(manifest.files[0].filename, "Content/rock.uasset");
    }

    #[test]
    fn a_compressed_manifest_with_a_wrong_hash_is_rejected() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        let fixture = Fixture::simple();
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&fixture.body).unwrap();
        let compressed = encoder.finish().unwrap();

        let mut bytes = Vec::new();
        put_u32(&mut bytes, HEADER_MAGIC);
        put_u32(&mut bytes, 41);
        put_u32(&mut bytes, fixture.body.len() as u32);
        put_u32(&mut bytes, compressed.len() as u32);
        bytes.extend_from_slice(&[0u8; 20]); // wrong hash
        bytes.push(FLAG_COMPRESSED);
        put_u32(&mut bytes, 18);
        bytes.extend_from_slice(&compressed);

        let err = Manifest::parse(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::Artifact(ref message) if message.contains("SHA-1")),
            "got {err:?}"
        );
    }

    #[test]
    fn version_2_file_fields_are_read() {
        // A file manifest list with version 2 carries MD5, MIME and SHA-256.
        let mut body = Vec::new();
        section(&mut body, |out| {
            out.push(0);
            put_u32(out, 18);
            out.push(0);
            put_u32(out, 0);
            put_fstring(out, "App");
            put_fstring(out, "1");
            put_fstring(out, "");
            put_fstring(out, "");
            put_u32(out, 0);
            put_fstring(out, "");
            put_fstring(out, "");
            put_fstring(out, "");
        });
        section(&mut body, |out| {
            out.push(0);
            put_u32(out, 0); // no chunks
        });
        section(&mut body, |out| {
            out.push(2); // fml version 2
            put_u32(out, 1);
            put_fstring(out, "a.txt");
            put_fstring(out, "");
            out.extend_from_slice(&[0x01; 20]);
            out.push(FILE_FLAG_EXECUTABLE);
            put_u32(out, 0); // install tags
            put_u32(out, 0); // chunk parts
            put_u32(out, 1); // has md5
            out.extend_from_slice(&[0x02; 16]);
            put_fstring(out, "text/plain");
            out.extend_from_slice(&[0x03; 32]);
        });
        section(&mut body, |out| {
            out.push(0);
            put_u32(out, 1);
            put_fstring(out, "key");
            put_fstring(out, "value");
        });

        let mut bytes = Vec::new();
        put_u32(&mut bytes, HEADER_MAGIC);
        put_u32(&mut bytes, 41);
        put_u32(&mut bytes, body.len() as u32);
        put_u32(&mut bytes, body.len() as u32);
        bytes.extend_from_slice(&[0u8; 20]);
        bytes.push(0);
        put_u32(&mut bytes, 20);
        bytes.extend_from_slice(&body);

        let manifest = Manifest::parse(&bytes).unwrap();
        let file = &manifest.files[0];
        assert_eq!(file.md5, Some([0x02; 16]));
        assert_eq!(file.mime_type.as_deref(), Some("text/plain"));
        assert_eq!(file.sha256, Some([0x03; 32]));
        assert!(file.is_executable());
        assert_eq!(file.size, 0);
        assert_eq!(
            manifest.custom_fields.get("key").map(String::as_str),
            Some("value")
        );
    }
}
