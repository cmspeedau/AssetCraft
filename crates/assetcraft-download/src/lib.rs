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
//!
//! What is **not** implemented yet: turning a parsed manifest's chunks into the
//! bytes of its files. That needs the CDN base URL Fab pairs with the manifest
//! and chunk decompression, and it is tracked as the next download-engine task
//! in ROADMAP.md.

pub mod bandwidth;
pub mod engine;
pub mod manifest;
pub mod queue;

pub use bandwidth::{BandwidthLimit, BandwidthLimiter};
pub use engine::{DownloadFile, DownloadOptions, DownloadOutcome, Downloader, Progress};
pub use manifest::{ChunkDownload, ChunkInfo, ChunkPart, FileEntry, Manifest, ManifestMeta};
pub use queue::{DownloadQueue, DownloadRequest};
