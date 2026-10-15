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
//! - **[`DownloadQueue`]**, an ordered, de-duplicating queue of
//!   [`DownloadRequest`]s that can be persisted between runs.
//!
//! What is **not** implemented yet: resolving an owned asset into the concrete
//! files to fetch. Fab hands that back as a signed *binary* manifest (Epic's
//! chunked manifest format, magic `0x44BEC00C`), whose parser can only be
//! validated against a manifest from a real signed-in account. It is tracked as
//! the next download-engine task in ROADMAP.md rather than guessed at here.

pub mod engine;
pub mod queue;

pub use engine::{DownloadFile, DownloadOptions, DownloadOutcome, Downloader, Progress};
pub use queue::{DownloadQueue, DownloadRequest};
