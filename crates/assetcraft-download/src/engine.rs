//! The transfer engine.
//!
//! The engine's contract is deliberately narrow: given a [`DownloadFile`] it
//! produces a complete, verified file at the destination, or an error — never a
//! half-written file at the destination. Two ideas make that hold:
//!
//! 1. **Work happens in a `.part` file.** The destination only appears when the
//!    bytes have arrived and verified, via an atomic rename.
//! 2. **A `.part` file is a resumable prefix.** Each attempt appends from the
//!    current length using an HTTP range request, so an interrupted transfer
//!    continues rather than restarting.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use assetcraft_core::{status_error, Error, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// One file to fetch.
///
/// `size` and `sha256` are optional because not every source advertises them;
/// when present they are checked, and a mismatch is a failure rather than a
/// silent success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadFile {
    /// Absolute URL to fetch.
    pub url: String,
    /// Where the finished file belongs.
    pub destination: PathBuf,
    /// Expected size in bytes, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Expected lowercase hex SHA-256, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl DownloadFile {
    /// A file with no known size or checksum.
    pub fn new(url: impl Into<String>, destination: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            destination: destination.into(),
            size: None,
            sha256: None,
        }
    }

    /// Record the expected size.
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    /// Record the expected SHA-256 (hex, case-insensitive).
    pub fn with_sha256(mut self, sha256: impl Into<String>) -> Self {
        self.sha256 = Some(sha256.into());
        self
    }
}

/// How the engine behaves.
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// Maximum number of transfers in flight at once.
    pub concurrency: usize,
    /// Total attempts per file, including the first.
    pub max_attempts: u32,
    /// Delay before the first retry; doubles each further attempt.
    pub base_backoff: Duration,
    /// Ceiling on the backoff delay.
    pub max_backoff: Duration,
    /// Whether to check the size and checksum of the finished file.
    pub verify: bool,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            concurrency: 4,
            max_attempts: 3,
            base_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
            verify: true,
        }
    }
}

/// A progress report for one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// Zero-based index of the file within the batch.
    pub index: usize,
    /// Number of files in the batch.
    pub total: usize,
    /// The file being fetched.
    pub destination: PathBuf,
    /// Bytes written so far, including bytes resumed from a previous run.
    pub bytes_done: u64,
    /// Total bytes, when the source advertises them.
    pub bytes_total: Option<u64>,
    /// Which attempt produced this report.
    pub attempt: u32,
}

impl Progress {
    /// Completion between 0 and 1, when the total is known.
    pub fn fraction(&self) -> Option<f32> {
        let total = self.bytes_total?;
        if total == 0 {
            return Some(1.0);
        }
        Some((self.bytes_done as f32 / total as f32).clamp(0.0, 1.0))
    }
}

/// What a completed download did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOutcome {
    /// Where the finished file was written.
    pub destination: PathBuf,
    /// Size of the finished file.
    pub bytes: u64,
    /// How many bytes were already present when the successful attempt began.
    pub resumed_from: u64,
    /// How many attempts the successful one was.
    pub attempts: u32,
}

/// Downloads files with bounded concurrency, resumption, retries, and
/// verification.
#[derive(Debug, Clone)]
pub struct Downloader {
    http: reqwest::Client,
    options: DownloadOptions,
}

impl Downloader {
    /// Build a downloader over an existing HTTP client.
    pub fn new(http: reqwest::Client, options: DownloadOptions) -> Self {
        Self { http, options }
    }

    /// Build a downloader with a default HTTP client, which caps connection
    /// setup but not total transfer time (downloads may be large).
    pub fn with_default_client(options: DownloadOptions) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self::new(http, options))
    }

    /// The options in effect.
    pub fn options(&self) -> &DownloadOptions {
        &self.options
    }

    /// Fetch one file.
    pub async fn download(&self, file: &DownloadFile) -> Result<DownloadOutcome> {
        let progress: ProgressReporter = Arc::new(|_| {});
        fetch_one(&self.http, &self.options, file, 0, 1, &progress).await
    }

    /// Fetch several files with at most `concurrency` transfers in flight.
    ///
    /// Reports progress through `on_progress`, which is called from whichever
    /// task is transferring. It stops at the first failure and abandons the
    /// rest of the batch, so a caller that wants to keep going should retry the
    /// files that did not complete.
    pub async fn download_all<F>(
        &self,
        files: Vec<DownloadFile>,
        on_progress: F,
    ) -> Result<Vec<DownloadOutcome>>
    where
        F: Fn(Progress) + Send + Sync + 'static,
    {
        let total = files.len();
        if total == 0 {
            return Ok(Vec::new());
        }

        let semaphore = Arc::new(Semaphore::new(self.options.concurrency.max(1)));
        let progress: ProgressReporter = Arc::new(on_progress);
        let mut tasks = JoinSet::new();

        for (index, file) in files.into_iter().enumerate() {
            let permit = Arc::clone(&semaphore);
            let progress = Arc::clone(&progress);
            let http = self.http.clone();
            let options = self.options.clone();
            tasks.spawn(async move {
                // The permit is held for the whole transfer, which is what
                // bounds the number of concurrent downloads.
                let _held = permit
                    .acquire_owned()
                    .await
                    .expect("download semaphore is never closed");
                let result = fetch_one(&http, &options, &file, index, total, &progress).await;
                (index, result)
            });
        }

        let mut completed = Vec::with_capacity(total);
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((index, Ok(outcome))) => completed.push((index, outcome)),
                Ok((_, Err(err))) => {
                    tasks.abort_all();
                    return Err(err);
                }
                Err(join_error) => {
                    tasks.abort_all();
                    return Err(Error::Other(format!(
                        "a download task failed to run: {join_error}"
                    )));
                }
            }
        }

        completed.sort_by_key(|(index, _)| *index);
        Ok(completed.into_iter().map(|(_, outcome)| outcome).collect())
    }
}

/// A progress callback shared between tasks.
type ProgressReporter = Arc<dyn Fn(Progress) + Send + Sync>;

/// A failure from one attempt, and whether trying again could help.
struct AttemptError {
    error: Error,
    retryable: bool,
}

impl AttemptError {
    fn transient(error: Error) -> Self {
        Self {
            error,
            retryable: true,
        }
    }

    fn permanent(error: Error) -> Self {
        Self {
            error,
            retryable: false,
        }
    }
}

/// Fetch `file`, retrying transient failures until the attempt budget is spent.
async fn fetch_one(
    http: &reqwest::Client,
    options: &DownloadOptions,
    file: &DownloadFile,
    index: usize,
    total: usize,
    progress: &ProgressReporter,
) -> Result<DownloadOutcome> {
    let destination = file.destination.clone();
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let part = partial_path(&destination);

    let max_attempts = options.max_attempts.max(1);
    let mut attempt = 0;
    loop {
        attempt += 1;
        match attempt_once(
            http,
            options,
            file,
            &part,
            destination.clone(),
            index,
            total,
            attempt,
            progress,
        )
        .await
        {
            Ok(outcome) => return Ok(outcome),
            Err(err) => {
                if !err.retryable || attempt >= max_attempts {
                    return Err(err.error);
                }
                tokio::time::sleep(backoff_delay(
                    options.base_backoff,
                    options.max_backoff,
                    attempt,
                ))
                .await;
            }
        }
    }
}

/// The `.part` path a destination's partial data lives at.
fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".part");
    destination.with_file_name(name)
}

#[allow(clippy::too_many_arguments)]
async fn attempt_once(
    http: &reqwest::Client,
    options: &DownloadOptions,
    file: &DownloadFile,
    part: &Path,
    destination: PathBuf,
    index: usize,
    total: usize,
    attempt: u32,
    progress: &ProgressReporter,
) -> std::result::Result<DownloadOutcome, AttemptError> {
    let mut existing = tokio::fs::metadata(part)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0);

    // If the part file already holds the whole file, finish rather than refetch.
    if let Some(size) = file.size {
        if existing == size {
            verify_file(part, file, options)
                .await
                .map_err(AttemptError::permanent)?;
            return finalize(part, &destination, existing, existing, attempt).await;
        }
        // A part file longer than the target is not a prefix of it; start over.
        if existing > size {
            let _ = tokio::fs::remove_file(part).await;
            existing = 0;
        }
    }

    let mut request = http.get(&file.url);
    if existing > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={existing}-"));
    }
    let response = request
        .send()
        .await
        .map_err(|err| AttemptError::transient(Error::from(err)))?;

    let status = response.status();

    // 416 means our range starts past the end, which for a correctly sized part
    // file means the file is already complete.
    if status == StatusCode::RANGE_NOT_SATISFIABLE {
        if let Some(size) = file.size {
            if existing == size {
                verify_file(part, file, options)
                    .await
                    .map_err(AttemptError::permanent)?;
                return finalize(part, &destination, existing, existing, attempt).await;
            }
        }
        let body = response.text().await.unwrap_or_default();
        return Err(AttemptError::permanent(status_error(status, &body)));
    }

    let resuming = status == StatusCode::PARTIAL_CONTENT;
    let restarting = status == StatusCode::OK;
    if !resuming && !restarting {
        let retryable = is_retryable_status(status);
        let body = response.text().await.unwrap_or_default();
        let error = status_error(status, &body);
        return Err(AttemptError { error, retryable });
    }

    // A server that ignores our range sends the whole file with a 200, which
    // means the existing bytes are not a prefix we can trust.
    let start = if resuming { existing } else { 0 };

    let hashing = options.verify && file.sha256.is_some();
    let (mut handle, mut hasher) = if resuming {
        let hasher = if hashing {
            let (hasher, hashed) = hash_prefix(part, start)
                .await
                .map_err(|err| AttemptError::transient(Error::Io(err)))?;
            if hashed != start {
                // The file shrank under us; the prefix is unreliable.
                let _ = tokio::fs::remove_file(part).await;
                return Err(AttemptError::transient(Error::Other(
                    "the partial file changed while resuming".to_string(),
                )));
            }
            Some(hasher)
        } else {
            None
        };
        let handle = tokio::fs::OpenOptions::new()
            .append(true)
            .open(part)
            .await
            .map_err(|err| AttemptError::transient(Error::Io(err)))?;
        (handle, hasher)
    } else {
        let handle = tokio::fs::File::create(part)
            .await
            .map_err(|err| AttemptError::transient(Error::Io(err)))?;
        (handle, hashing.then(Sha256::new))
    };

    let mut written = start;
    progress(Progress {
        index,
        total,
        destination: destination.clone(),
        bytes_done: written,
        bytes_total: file.size,
        attempt,
    });

    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| AttemptError::transient(Error::from(err)))?
    {
        handle
            .write_all(&chunk)
            .await
            .map_err(|err| AttemptError::transient(Error::Io(err)))?;
        if let Some(hasher) = hasher.as_mut() {
            hasher.update(&chunk);
        }
        written += chunk.len() as u64;
        progress(Progress {
            index,
            total,
            destination: destination.clone(),
            bytes_done: written,
            bytes_total: file.size,
            attempt,
        });
    }

    handle
        .flush()
        .await
        .map_err(|err| AttemptError::transient(Error::Io(err)))?;
    handle
        .sync_all()
        .await
        .map_err(|err| AttemptError::transient(Error::Io(err)))?;
    drop(handle);

    if let (Some(hasher), Some(expected)) = (hasher, file.sha256.as_deref()) {
        let actual = to_hex(hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            // A corrupt file is worse than no file: it would otherwise be
            // resumed from and reported as complete.
            let _ = tokio::fs::remove_file(part).await;
            return Err(AttemptError::permanent(Error::Other(format!(
                "checksum mismatch for {}: expected {expected}, got {actual}",
                destination.display()
            ))));
        }
    }

    if let Some(size) = file.size {
        if written != size {
            let _ = tokio::fs::remove_file(part).await;
            return Err(AttemptError::permanent(Error::Other(format!(
                "size mismatch for {}: expected {size}, got {written}",
                destination.display()
            ))));
        }
    }

    finalize(part, &destination, written, start, attempt).await
}

/// Verify an already-complete part file against the expected size and checksum.
async fn verify_file(path: &Path, file: &DownloadFile, options: &DownloadOptions) -> Result<()> {
    if !options.verify {
        return Ok(());
    }
    if let Some(expected) = file.sha256.as_deref() {
        let length = tokio::fs::metadata(path).await.map(|meta| meta.len())?;
        let (hasher, _) = hash_prefix(path, length).await?;
        let actual = to_hex(hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(Error::Other(format!(
                "checksum mismatch for {}: expected {expected}, got {actual}",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Move the verified part file into place.
async fn finalize(
    part: &Path,
    destination: &Path,
    bytes: u64,
    resumed_from: u64,
    attempts: u32,
) -> std::result::Result<DownloadOutcome, AttemptError> {
    tokio::fs::rename(part, destination)
        .await
        .map_err(|err| AttemptError::transient(Error::Io(err)))?;
    Ok(DownloadOutcome {
        destination: destination.to_path_buf(),
        bytes,
        resumed_from,
        attempts,
    })
}

/// Hash the first `limit` bytes of `path`.
///
/// Used to fold an existing partial file into the checksum of the whole file
/// when resuming, rather than re-downloading the prefix to hash it.
async fn hash_prefix(path: &Path, limit: u64) -> std::io::Result<(Sha256, u64)> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut hashed = 0u64;

    while hashed < limit {
        let want = ((limit - hashed) as usize).min(buffer.len());
        let read = file.read(&mut buffer[..want]).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        hashed += read as u64;
    }
    Ok((hasher, hashed))
}

/// Whether a status is worth retrying.
///
/// Client errors are permanent: a 404 will not become a 200 by asking again.
/// Rate limiting and server-side failures are transient.
fn is_retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 425 | 429) || status.is_server_error()
}

/// Exponential backoff: `base * 2^(attempt - 1)`, capped at `max`.
fn backoff_delay(base: Duration, max: Duration, attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(10);
    base.saturating_mul(1u32 << shift).min(max)
}

/// Lowercase hex of a byte string.
fn to_hex(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    for byte in bytes.as_ref() {
        // Writing into a String cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use std::sync::Mutex;

    /// SHA-256 of "hello world".
    const HELLO_WORLD_SHA: &str =
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    fn downloader(options: DownloadOptions) -> Downloader {
        Downloader::new(reqwest::Client::new(), options)
    }

    fn fast_options() -> DownloadOptions {
        DownloadOptions {
            base_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(5),
            ..Default::default()
        }
    }

    #[test]
    fn backoff_doubles_and_then_caps() {
        let base = Duration::from_millis(100);
        let max = Duration::from_secs(5);
        assert_eq!(backoff_delay(base, max, 1), Duration::from_millis(100));
        assert_eq!(backoff_delay(base, max, 2), Duration::from_millis(200));
        assert_eq!(backoff_delay(base, max, 3), Duration::from_millis(400));
        assert_eq!(backoff_delay(base, max, 10), max);
    }

    #[test]
    fn transient_statuses_are_classified() {
        assert!(is_retryable_status(StatusCode::REQUEST_TIMEOUT));
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(!is_retryable_status(StatusCode::NOT_FOUND));
        assert!(!is_retryable_status(StatusCode::FORBIDDEN));
    }

    #[test]
    fn partial_path_appends_a_suffix() {
        assert_eq!(
            partial_path(Path::new("/tmp/asset.zip")),
            PathBuf::from("/tmp/asset.zip.part")
        );
    }

    #[test]
    fn fraction_is_clamped_and_handles_unknown_total() {
        let base = Progress {
            index: 0,
            total: 1,
            destination: PathBuf::from("/tmp/x"),
            bytes_done: 50,
            bytes_total: Some(100),
            attempt: 1,
        };
        assert_eq!(base.fraction(), Some(0.5));
        assert_eq!(
            Progress {
                bytes_total: None,
                ..base.clone()
            }
            .fraction(),
            None
        );
        assert_eq!(
            Progress {
                bytes_done: 200,
                ..base
            }
            .fraction(),
            Some(1.0)
        );
    }

    #[tokio::test]
    async fn downloads_a_file_and_leaves_no_part_file() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/asset.bin");
                then.status(200).body("hello world");
            })
            .await;

        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("asset.bin");
        let file = DownloadFile::new(format!("{}/asset.bin", server.base_url()), &destination);

        let outcome = downloader(fast_options()).download(&file).await.unwrap();

        assert_eq!(outcome.bytes, 11);
        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(outcome.attempts, 1);
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "hello world"
        );
        // The part file is renamed away, not left behind.
        assert!(!partial_path(&destination).exists());
    }

    #[tokio::test]
    async fn accepts_a_matching_checksum() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/asset.bin");
                then.status(200).body("hello world");
            })
            .await;

        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("asset.bin");
        let file = DownloadFile::new(format!("{}/asset.bin", server.base_url()), &destination)
            .with_size(11)
            .with_sha256(HELLO_WORLD_SHA);

        let outcome = downloader(fast_options()).download(&file).await.unwrap();
        assert_eq!(outcome.bytes, 11);
    }

    #[tokio::test]
    async fn a_checksum_mismatch_fails_and_leaves_nothing_behind() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/asset.bin");
                then.status(200).body("hello world");
            })
            .await;

        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("asset.bin");
        let file = DownloadFile::new(format!("{}/asset.bin", server.base_url()), &destination)
            .with_sha256("0000000000000000000000000000000000000000000000000000000000000000");

        let err = downloader(fast_options())
            .download(&file)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"), "got {err}");
        // Neither the destination nor a corrupt part file survives.
        assert!(!destination.exists());
        assert!(!partial_path(&destination).exists());
    }

    #[tokio::test]
    async fn a_size_mismatch_fails_and_leaves_nothing_behind() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/asset.bin");
                then.status(200).body("hello world");
            })
            .await;

        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("asset.bin");
        // Claim a size the body will not match.
        let file = DownloadFile::new(format!("{}/asset.bin", server.base_url()), &destination)
            .with_size(999);

        let err = downloader(fast_options())
            .download(&file)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("size mismatch"), "got {err}");
        assert!(!destination.exists());
        assert!(!partial_path(&destination).exists());
    }

    #[tokio::test]
    async fn resumes_from_a_partial_file_with_a_range_request() {
        let server = MockServer::start_async().await;
        let ranged = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/asset.bin")
                    .header("range", "bytes=5-");
                then.status(206)
                    .header("content-range", "bytes 5-10/11")
                    .body(" world");
            })
            .await;

        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("asset.bin");
        std::fs::write(partial_path(&destination), "hello").unwrap();

        let file = DownloadFile::new(format!("{}/asset.bin", server.base_url()), &destination)
            .with_size(11)
            .with_sha256(HELLO_WORLD_SHA);

        let outcome = downloader(fast_options()).download(&file).await.unwrap();

        ranged.assert_async().await;
        assert_eq!(outcome.resumed_from, 5);
        assert_eq!(outcome.bytes, 11);
        // The resumed prefix plus the remainder hash to the whole file.
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "hello world"
        );
    }

    #[tokio::test]
    async fn restarts_when_the_server_ignores_the_range() {
        let server = MockServer::start_async().await;
        // Matches regardless of the Range header, and answers 200 with the whole
        // body — what a server without range support does.
        server
            .mock_async(|when, then| {
                when.method(GET).path("/asset.bin");
                then.status(200).body("hello world");
            })
            .await;

        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("asset.bin");
        // Five bytes of junk stand in for an unusable prefix.
        std::fs::write(partial_path(&destination), "XXXXX").unwrap();

        let file = DownloadFile::new(format!("{}/asset.bin", server.base_url()), &destination);

        let outcome = downloader(fast_options()).download(&file).await.unwrap();

        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "hello world"
        );
    }

    #[tokio::test]
    async fn a_complete_part_file_is_finished_without_refetching() {
        let server = MockServer::start_async().await;
        let request = server
            .mock_async(|when, then| {
                when.method(GET).path("/asset.bin");
                then.status(500).body("should not be called");
            })
            .await;

        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("asset.bin");
        std::fs::write(partial_path(&destination), "hello world").unwrap();

        let file = DownloadFile::new(format!("{}/asset.bin", server.base_url()), &destination)
            .with_size(11)
            .with_sha256(HELLO_WORLD_SHA);

        let outcome = downloader(fast_options()).download(&file).await.unwrap();

        assert_eq!(outcome.bytes, 11);
        assert_eq!(outcome.resumed_from, 11);
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "hello world"
        );
        // No request was made: the existing bytes were trusted after verifying.
        assert_eq!(request.calls_async().await, 0);
    }

    #[tokio::test]
    async fn retries_a_server_error_up_to_the_attempt_budget() {
        let server = MockServer::start_async().await;
        let request = server
            .mock_async(|when, then| {
                when.method(GET).path("/asset.bin");
                then.status(500).body("boom");
            })
            .await;

        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("asset.bin");
        let file = DownloadFile::new(format!("{}/asset.bin", server.base_url()), &destination);

        let options = DownloadOptions {
            max_attempts: 3,
            ..fast_options()
        };
        let err = downloader(options).download(&file).await.unwrap_err();

        assert!(
            matches!(err, Error::HttpStatus { status: 500, .. }),
            "got {err:?}"
        );
        assert_eq!(request.calls_async().await, 3);
    }

    #[tokio::test]
    async fn does_not_retry_a_client_error() {
        let server = MockServer::start_async().await;
        let request = server
            .mock_async(|when, then| {
                when.method(GET).path("/asset.bin");
                then.status(404).body("gone");
            })
            .await;

        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("asset.bin");
        let file = DownloadFile::new(format!("{}/asset.bin", server.base_url()), &destination);

        let err = downloader(fast_options())
            .download(&file)
            .await
            .unwrap_err();

        assert!(
            matches!(err, Error::HttpStatus { status: 404, .. }),
            "got {err:?}"
        );
        // A 404 will not become a 200 by asking again.
        assert_eq!(request.calls_async().await, 1);
    }

    #[tokio::test]
    async fn downloads_a_batch_and_reports_progress() {
        let server = MockServer::start_async().await;
        for (path, body) in [("/a.bin", "aaaa"), ("/b.bin", "bbbb"), ("/c.bin", "cccc")] {
            server
                .mock_async(move |when, then| {
                    when.method(GET).path(path);
                    then.status(200).body(body);
                })
                .await;
        }

        let dir = tempfile::tempdir().unwrap();
        let files: Vec<DownloadFile> = ["a", "b", "c"]
            .iter()
            .map(|name| {
                DownloadFile::new(
                    format!("{}/{name}.bin", server.base_url()),
                    dir.path().join(format!("{name}.bin")),
                )
                .with_size(4)
            })
            .collect();

        let seen: Arc<Mutex<Vec<Progress>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);

        let options = DownloadOptions {
            concurrency: 2,
            ..fast_options()
        };
        let outcomes = downloader(options)
            .download_all(files, move |progress| {
                recorder.lock().unwrap().push(progress);
            })
            .await
            .unwrap();

        assert_eq!(outcomes.len(), 3);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("b.bin")).unwrap(),
            "bbbb"
        );

        let reports = seen.lock().unwrap();
        // Every file reported reaching its full size, and progress carried the
        // batch size so a UI can render "2 of 3".
        for name in ["a", "b", "c"] {
            assert!(
                reports
                    .iter()
                    .any(|p| p.destination.ends_with(format!("{name}.bin"))
                        && p.bytes_done == 4
                        && p.bytes_total == Some(4)
                        && p.total == 3
                        && p.fraction() == Some(1.0)),
                "no completed progress report for {name}.bin"
            );
        }
    }

    #[tokio::test]
    async fn an_empty_batch_is_fine() {
        let outcomes = downloader(fast_options())
            .download_all(Vec::new(), |_| {})
            .await
            .unwrap();
        assert!(outcomes.is_empty());
    }
}
