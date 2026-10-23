//! From "this asset id" to "files on disk".
//!
//! Bringing an owned asset down takes three API calls — the format list, the
//! signed download info, and the binary manifest — and then a transfer step
//! that fetches the manifest's chunks, reassembles them, and writes the files.
//! The CLI and the GUI both need exactly this sequence, so it lives here rather
//! than being written twice.
//!
//! The one thing not derivable from a manifest is the CDN base its chunk paths
//! hang off. Callers pass it in ([`Request::chunk_base`]); when they do not,
//! the directory of the manifest URL is assumed, which is what the stub and the
//! documented layout use.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use assetcraft_catalog::FabClient;
use assetcraft_core::{select, AssetId, SelectionCriteria};
use assetcraft_download::{
    ChunkStore, DownloadFile, DownloadOptions, Downloader, Manifest, Progress,
};

/// The directory chunks land in before they are reassembled.
const SCRATCH_DIR: &str = ".assetcraft-chunks";

/// Everything needed to bring one asset down.
#[derive(Debug, Clone)]
pub struct Request {
    /// The asset to fetch.
    pub asset_id: String,
    /// Directory the asset's files are written into.
    pub output: PathBuf,
    /// Base URL the manifest's chunk paths hang off, when the caller knows it.
    pub chunk_base: Option<String>,
    /// Which format and file to select.
    pub criteria: SelectionCriteria,
    /// Transfer tuning: concurrency, retries, bandwidth.
    pub options: DownloadOptions,
}

impl Request {
    /// A request with default selection and transfer settings.
    pub fn new(asset_id: impl Into<String>, output: impl Into<PathBuf>) -> Self {
        Self {
            asset_id: asset_id.into(),
            output: output.into(),
            chunk_base: None,
            criteria: SelectionCriteria::default(),
            options: DownloadOptions::default(),
        }
    }
}

/// What resolving an asset produced, before any chunk is fetched.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// The format code that was chosen, e.g. `fbx`.
    pub format: String,
    /// The chosen file's display name, when the API reported one.
    pub file: Option<String>,
    /// The chosen file's id.
    pub file_id: String,
    /// The artifact id the download belongs to.
    pub artifact: String,
    /// The parsed manifest.
    pub manifest: Manifest,
    /// The directory the manifest was served from, used as the chunk base
    /// unless a request supplies one.
    pub chunk_base: String,
}

/// What a finished transfer produced.
#[derive(Debug, Clone)]
pub struct Acquired {
    /// The format code that was downloaded, e.g. `fbx`.
    pub format: String,
    /// How many files were written.
    pub files: usize,
    /// How many bytes were written.
    pub bytes: u64,
    /// The absolute directory the files were written into.
    pub path: PathBuf,
}

/// Progress a caller can show: what stage a transfer is in, and how far along.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Choosing a format and fetching its manifest.
    Resolving,
    /// Fetching chunks.
    Download {
        /// Chunks that have finished.
        files_done: usize,
        /// Chunks in the batch.
        files_total: usize,
        /// Bytes fetched so far, across the batch.
        bytes_done: u64,
        /// Bytes to fetch, when every chunk reported its size.
        bytes_total: u64,
    },
    /// Splicing chunks back into the manifest's files and verifying them.
    Assembling,
    /// Writing the reassembled files out.
    Writing {
        /// Files written so far.
        done: usize,
        /// Files in the manifest.
        total: usize,
    },
}

/// Resolve, download, assemble, and write one asset.
///
/// `progress` is called from the transfer tasks, so it must be `Send + Sync`.
/// It is also called on the calling task for the resolving, assembling, and
/// writing phases.
pub async fn acquire<F>(client: &FabClient, request: &Request, progress: F) -> Result<Acquired>
where
    F: Fn(Phase) + Send + Sync + 'static,
{
    let progress = Arc::new(progress);
    progress(Phase::Resolving);

    let resolved = resolve(client, &request.asset_id, &request.criteria).await?;
    let format = resolved.format.clone();
    let manifest = resolved.manifest.clone();
    let chunk_base = request
        .chunk_base
        .clone()
        .unwrap_or_else(|| resolved.chunk_base.clone());

    let scratch = request.output.join(SCRATCH_DIR);
    std::fs::create_dir_all(&scratch).context("could not create the scratch directory")?;

    let chunk_files: Vec<DownloadFile> = manifest
        .chunk_downloads(&chunk_base)
        .into_iter()
        .map(|chunk| DownloadFile {
            url: chunk.url,
            destination: scratch.join(format!("{}.chunk", chunk.guid)),
            size: Some(chunk.size_bytes),
            sha256: None,
        })
        .collect();

    let totals = Arc::new(Mutex::new(Totals::for_chunks(&chunk_files)));
    let ticker = {
        let progress = Arc::clone(&progress);
        let totals = Arc::clone(&totals);
        move |chunk: Progress| {
            let mut totals = totals
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            totals.record(chunk.index, chunk.bytes_done, chunk.bytes_total);
            progress(Phase::Download {
                files_done: totals.finished,
                files_total: totals.total,
                bytes_done: totals.bytes_done,
                bytes_total: totals.bytes_total,
            });
        }
    };

    let downloader = Downloader::with_default_client(request.options.clone())?;
    let outcomes = downloader
        .download_all(chunk_files, ticker)
        .await
        .context("chunk download failed")?;

    progress(Phase::Assembling);
    let mut store = ChunkStore::new();
    for outcome in &outcomes {
        let bytes = std::fs::read(&outcome.destination).context("could not read a chunk")?;
        store
            .insert_file(&bytes)
            .context("could not decode a chunk")?;
    }
    let files = manifest
        .assemble(&store)
        .context("could not reassemble the asset")?;

    let mut written = 0usize;
    let mut bytes = 0u64;
    let total_files = files.len();
    for file in &files {
        let path = safe_join(&request.output, &file.filename)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("could not create a directory")?;
        }
        std::fs::write(&path, &file.data)
            .with_context(|| format!("could not write {}", path.display()))?;
        written += 1;
        bytes += file.data.len() as u64;
        progress(Phase::Writing {
            done: written,
            total: total_files,
        });
    }

    let _ = std::fs::remove_dir_all(&scratch);
    let path = std::fs::canonicalize(&request.output).unwrap_or_else(|_| request.output.clone());
    Ok(Acquired {
        format,
        files: written,
        bytes,
        path,
    })
}

/// Choose a format, fetch its signed info, and parse the first working manifest.
pub async fn resolve(
    client: &FabClient,
    asset_id: &str,
    criteria: &SelectionCriteria,
) -> Result<Resolved> {
    let formats = client
        .listing_formats(&AssetId::new(asset_id))
        .await
        .context("could not load asset formats")?;

    let chosen = select(&formats, criteria).context("selection failed")?;
    let format_code = chosen
        .format_code
        .clone()
        .context("the selected format has no code to request download info with")?;
    let file_id = chosen
        .file_id()
        .map(str::to_string)
        .context("the selected file has no id to request download info with")?;

    let info = client
        .download_info(asset_id, &format_code, &file_id)
        .await
        .context("could not resolve download info")?;
    if info.distribution_points.is_empty() {
        bail!("Fab returned no distribution points for this file");
    }

    // Distribution points are mirrors; use the first one whose manifest parses.
    let mut last_error = None;
    for point in &info.distribution_points {
        let bytes = match client.fetch_bytes(&point.manifest_url).await {
            Ok(bytes) => bytes,
            Err(error) => {
                last_error = Some(anyhow!(error).context("manifest could not be fetched"));
                continue;
            }
        };
        match Manifest::parse(&bytes) {
            Ok(manifest) => {
                return Ok(Resolved {
                    format: format_code,
                    file: chosen.file.name.clone(),
                    file_id: chosen.file_id().unwrap_or_default().to_owned(),
                    artifact: info.artifact_id.clone(),
                    manifest,
                    chunk_base: base_from_manifest_url(&point.manifest_url),
                });
            }
            Err(error) => {
                last_error = Some(anyhow!(error).context("manifest could not be parsed"));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("no distribution point produced a manifest")))
}

/// The manifest URL's directory, without its query or fragment.
fn base_from_manifest_url(manifest_url: &str) -> String {
    let bare = manifest_url
        .split(['?', '#'])
        .next()
        .unwrap_or(manifest_url);
    match bare.rfind('/') {
        Some(index) if index > "https://".len() => bare[..index].to_owned(),
        _ => bare.to_owned(),
    }
}

/// Join a manifest path onto the output root, refusing anything that escapes.
fn safe_join(root: &Path, name: &str) -> Result<PathBuf> {
    let relative = Path::new(name);
    let escapes = relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir));
    if escapes {
        bail!("the manifest lists a path outside the output directory: {name:?}");
    }
    Ok(root.join(relative))
}

/// Per-chunk running totals, folded into one batch-wide number.
struct Totals {
    done: Vec<(u64, u64)>,
    finished: usize,
    total: usize,
    bytes_done: u64,
    bytes_total: u64,
}

impl Totals {
    fn for_chunks(files: &[DownloadFile]) -> Self {
        Self {
            done: files
                .iter()
                .map(|file| (0, file.size.unwrap_or(0)))
                .collect(),
            finished: 0,
            total: files.len(),
            bytes_done: 0,
            bytes_total: files.iter().filter_map(|file| file.size).sum(),
        }
    }

    fn record(&mut self, index: usize, done: u64, total: Option<u64>) {
        if let Some(slot) = self.done.get_mut(index) {
            let complete_before = slot.0 >= slot.1 && slot.1 > 0;
            slot.0 = done;
            if let Some(total) = total {
                slot.1 = total;
            }
            let complete_after = slot.0 >= slot.1 && slot.1 > 0;
            if complete_after && !complete_before {
                self.finished += 1;
            }
        }
        self.bytes_done = self.done.iter().map(|(done, _)| done).sum();
        let known: u64 = self.done.iter().map(|(_, total)| total).sum();
        if known > 0 {
            self.bytes_total = known;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{base_from_manifest_url, safe_join};
    use std::path::Path;

    #[test]
    fn a_manifest_url_loses_its_file_name_and_query() {
        assert_eq!(
            base_from_manifest_url("https://cdn.test/CloudDir/Manifest.bin?sig=abc"),
            "https://cdn.test/CloudDir"
        );
        assert_eq!(
            base_from_manifest_url("http://127.0.0.1:8791/stub/manifest.bin"),
            "http://127.0.0.1:8791/stub"
        );
    }

    #[test]
    fn chunk_progress_folds_into_batch_totals() {
        use assetcraft_download::DownloadFile;

        let files = vec![
            DownloadFile {
                url: "http://test/1".to_owned(),
                destination: "1.chunk".into(),
                size: Some(100),
                sha256: None,
            },
            DownloadFile {
                url: "http://test/2".to_owned(),
                destination: "2.chunk".into(),
                size: Some(40),
                sha256: None,
            },
        ];
        let mut totals = super::Totals::for_chunks(&files);
        assert_eq!(totals.bytes_total, 140);

        totals.record(0, 50, Some(100));
        assert_eq!((totals.bytes_done, totals.finished), (50, 0));

        totals.record(0, 100, Some(100));
        assert_eq!((totals.bytes_done, totals.finished), (100, 1));

        totals.record(1, 25, Some(40));
        assert_eq!((totals.bytes_done, totals.finished), (125, 1));
    }

    #[test]
    fn manifest_paths_cannot_escape_the_output_directory() {
        let root = Path::new("/tmp/asset");
        assert_eq!(
            safe_join(root, "Content/rock.uasset").unwrap(),
            root.join("Content/rock.uasset")
        );
        assert!(safe_join(root, "../outside.uasset").is_err());
        assert!(safe_join(root, "/etc/passwd").is_err());
    }
}
