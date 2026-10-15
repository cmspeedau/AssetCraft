//! The AssetCraft download engine.
//!
//! This crate gets bytes onto disk reliably. It is deliberately agnostic about
//! *what* it is downloading: it moves [`DownloadFile`]s, and it does not know
//! how an asset id maps to those files.
//!
//! What is implemented:
//!
//! - **[`Downloader`]**, which fetches files with a bounded number of
//!   concurrent transfers, resumes a partial file with an HTTP range request,
//!   retries transient failures with exponential backoff, verifies the result
//!   against an expected size and SHA-256, and only renames the file into place
//!   once it verifies.
//! - **[`BandwidthLimit`]**, an aggregate throughput cap shared by every
//!   transfer a [`Downloader`] runs.
//! - **[`DownloadQueue`]**, an ordered, de-duplicating queue of
//!   [`DownloadRequest`]s that can be persisted between runs.
//! - **[`manifest`]**, a parser for Fab's signed binary artifact manifest, which
//!   resolves an owned asset into the chunks to fetch and the files they make
//!   up.
//! - **[`chunk`]** and the reassembly in [`Manifest::assemble`], which decode a
//!   chunk file and splice the chunk parts back into the files the manifest
//!   describes, checking each file against its SHA-1.
//!
//! What is **not** automated: the CDN base URL Fab pairs with the manifest. The
//! manifest carries each chunk's relative path but not its host, and how Fab
//! supplies the base can only be confirmed against a real signed-in response, so
//! [`Manifest::chunk_downloads`] joins a base you supply with each chunk's
//! documented path; the CLI takes it as `--chunk-base`.

mod assemble;
pub mod bandwidth;
mod binary;
pub mod chunk;
pub mod engine;
pub mod manifest;
pub mod queue;

pub use assemble::AssembledFile;
pub use bandwidth::{BandwidthLimit, BandwidthLimiter};
pub use chunk::{Chunk, ChunkStore};
pub use engine::{DownloadFile, DownloadOptions, DownloadOutcome, Downloader, Progress};
pub use manifest::{ChunkDownload, ChunkInfo, ChunkPart, FileEntry, Manifest, ManifestMeta};
pub use queue::{DownloadQueue, DownloadRequest};
