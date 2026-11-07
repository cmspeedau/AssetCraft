//! From "this asset id" to "files on disk".
//!
//! Only the *owned Unreal build* route works against the live API, so this
//! module follows it and nothing else. It takes a card's listing id, finds the
//! owned library entry that carries an `artifactId`, asks Fab for that
//! artifact's manifest (`POST /e/artifacts/{artifactId}/manifest` with the
//! bare 32-hex asset id), fetches and parses the signed binary manifest, then
//! fetches the manifest's chunks, reassembles them, and writes the files. The
//! CLI and the GUI both need exactly this sequence, so it lives here rather
//! than being written twice.
//!
//! Two facts the API does not spell out, learned the hard way against a real
//! account: the CDN base is the directory of the manifest URL (Fab's own
//! `distributionPointBaseUrls` are unrooted), and every chunk URL must replay
//! the manifest URL's query string, because the CDN signs the whole `CloudDir/`
//! tree with that one token and answers 403 to an unsigned chunk. Chunks are
//! fetched *without* the bearer token — an `Authorization` header on a CDN
//! chunk gets Akamai's 501.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use assetcraft_catalog::FabClient;
use assetcraft_core::{AssetStructure, DownloadInfo, LibraryItem};
use assetcraft_download::{
    ChunkStore, DownloadFile, DownloadOptions, Downloader, Manifest, Progress,
};
use sha1::{Digest, Sha1};

/// The directory chunks land in before they are reassembled.
const SCRATCH_DIR: &str = ".assetcraft-chunks";

/// The owned Unreal build a download fetches, as the library describes it.
///
/// A search card knows its listing by UUID; the owned library entry knows it
/// by a bare 32-hex asset id. The artifact manifest wants the latter, so the
/// two meet in [`owned_for`] before any request is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owned {
    /// The engine build to deliver, from a library entry's `projectVersions`.
    pub artifact_id: String,
    /// The entry's bare 32-hex asset id — the manifest endpoint's `item_id`.
    pub item_id: String,
    /// The namespace the asset lives in.
    pub namespace: String,
}

/// The coordinates of one deliverable build inside a library entry, if it has
/// one. An entry with no artifact has no download route: other formats live
/// behind a Fab web session, not a bearer token.
pub fn owned_from_item(item: &LibraryItem) -> Option<Owned> {
    Some(Owned {
        artifact_id: item.artifact_id.clone()?,
        item_id: item.asset_id.clone(),
        namespace: item.namespace.clone(),
    })
}

/// Does this library entry answer to a card's id? The card carries the
/// listing UUID while the entry is keyed by a bare hex uid, and the entry's
/// marketplace URL is the only place the two shapes meet.
fn matches_id(item: &LibraryItem, id: &str) -> bool {
    let listing_suffix = format!("/listings/{id}");
    item.asset_id == id
        || item
            .url
            .as_deref()
            .is_some_and(|url| url.ends_with(&listing_suffix))
}

/// The library entry that answers to a card's id, among synced entries.
pub fn item_for_id<'a>(items: &'a [LibraryItem], id: &str) -> Option<&'a LibraryItem> {
    items.iter().find(|item| matches_id(item, id))
}

/// Page through the account's library for the entry that answers to `id`.
///
/// The GUI shows cards and the CLI takes ids from `search`, so both hold a
/// listing UUID that only the owned library resolves to a deliverable build.
/// Paging stops at the first hit: an owned asset of any size can sit on page
/// one, and a miss after every page means the asset is simply not owned.
pub async fn owned_for(
    client: &FabClient,
    account_id: &str,
    id: &str,
) -> Result<(LibraryItem, Owned)> {
    let mut cursor = None;
    loop {
        let page = client
            .library(account_id, cursor.as_deref())
            .await
            .context("could not read your Epic library")?;
        if let Some(item) = page.items.iter().find(|item| matches_id(item, id)) {
            let owned =
                owned_from_item(item).ok_or_else(|| {
                    anyhow!(
                    "{} has no downloadable Unreal build. Other formats come from the Fab website.",
                    if item.title.is_empty() { "This asset" } else { &item.title }
                )
                })?;
            return Ok((item.clone(), owned));
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => {
                bail!("This asset is not in your Epic library — claim (free) or download (paid) it on Fab.com first.")
            }
        }
    }
}

/// Everything needed to bring one asset down.
#[derive(Debug, Clone)]
pub struct Request {
    /// The owned build to fetch.
    pub owned: Owned,
    /// Directory the asset's files are written into.
    pub output: PathBuf,
    /// Base URL the manifest's chunk paths hang off, when the caller knows it
    /// (a local stub, usually). Setting it drops the manifest URL's signature.
    pub chunk_base: Option<String>,
    /// Transfer tuning: concurrency, retries, bandwidth.
    pub options: DownloadOptions,
}

impl Request {
    /// A request with default transfer settings.
    pub fn new(owned: Owned, output: impl Into<PathBuf>) -> Self {
        Self {
            owned,
            output: output.into(),
            chunk_base: None,
            options: DownloadOptions::default(),
        }
    }
}

/// What resolving an asset produced, before any chunk is fetched.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// The format label the manifest's build reports, e.g. `unreal-engine`.
    pub format: String,
    /// The build version the artifact delivers, when Fab reported one.
    pub build_version: Option<String>,
    /// The artifact id the download belongs to.
    pub artifact: String,
    /// The parsed manifest.
    pub manifest: Manifest,
    /// The directory the manifest was served from — the chunk base.
    pub chunk_base: String,
    /// The manifest URL's query string (leading `?` and all), which every
    /// chunk URL must replay; empty when the manifest URL was unsigned.
    pub chunk_query: String,
    /// What the manifest's files contain — its texture maps and levels of detail
    /// — read from their names, before a single chunk has been fetched.
    pub structure: AssetStructure,
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
    /// What the files that were written contain: their texture maps and levels of
    /// detail, read from their names.
    pub structure: AssetStructure,
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

    let resolved = resolve(client, &request.owned).await?;
    let format = resolved.format.clone();
    let manifest = resolved.manifest.clone();
    // A caller-supplied base (a local stub, mostly) arrives unsigned, so the
    // manifest URL's signature goes with it; the live route replays it.
    let (chunk_base, chunk_query) = match &request.chunk_base {
        Some(base) => (base.clone(), String::new()),
        None => (resolved.chunk_base.clone(), resolved.chunk_query.clone()),
    };

    let scratch = request.output.join(SCRATCH_DIR);
    std::fs::create_dir_all(&scratch).context("could not create the scratch directory")?;

    let chunk_files: Vec<DownloadFile> = manifest
        .chunk_downloads(&chunk_base, &chunk_query)
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
    // The manifest hashed every chunk's decompressed payload at build time;
    // checking that here is what proves the bytes Fab indexed are the bytes
    // the CDN actually handed us, before any of them is spliced into a file.
    let expected: std::collections::BTreeMap<[u32; 4], [u8; 20]> = manifest
        .chunks
        .iter()
        .filter(|chunk| chunk.sha_hash != [0u8; 20])
        .map(|chunk| (chunk.guid, chunk.sha_hash))
        .collect();
    let mut store = ChunkStore::new();
    for outcome in &outcomes {
        let bytes = std::fs::read(&outcome.destination).context("could not read a chunk")?;
        let chunk =
            assetcraft_download::Chunk::decode(&bytes).context("could not decode a chunk")?;
        if request.options.verify {
            if let Some(want) = expected.get(&chunk.guid) {
                let got = Sha1::digest(chunk.data());
                if got.as_slice() != *want {
                    bail!(
                        "chunk {} arrived hashing to {}; the manifest promised {}",
                        chunk
                            .guid
                            .iter()
                            .map(|word| format!("{word:08X}"))
                            .collect::<String>(),
                        hex_lower(got),
                        hex_lower(want),
                    );
                }
            }
        }
        store.insert_chunk(chunk);
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
    let structure = structure_of_files(
        files
            .iter()
            .map(|file| (file.filename.as_str(), file.data.len() as u64)),
    );
    Ok(Acquired {
        format,
        files: written,
        bytes,
        path,
        structure,
    })
}

/// Ask Fab for the owned artifact's manifest and parse the first working one.
///
/// Every distribution point Fab returns mirrors the same signed blob; each is
/// fetched, checked against `manifestHash` — the SHA-1 of the blob, which
/// catches a truncated or swapped CDN body before a single chunk is fetched
/// — and parsed. The first one to pass wins. Quality criteria play no part:
/// an artifact is the one build Fab chose to deliver.
pub async fn resolve(client: &FabClient, owned: &Owned) -> Result<Resolved> {
    let infos = client
        .asset_manifest(
            &owned.artifact_id,
            &owned.item_id,
            &owned.namespace,
            PLATFORM,
        )
        .await
        .context("could not get the download manifest from Fab")?;
    if infos.is_empty() {
        bail!("Fab has no downloadable build for this artifact");
    }

    let mut last_error = None;
    for info in &infos {
        for point in &info.distribution_points {
            let bytes = match fetch_manifest(client, &point.manifest_url).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    last_error = Some(anyhow!(error).context("the manifest could not be fetched"));
                    continue;
                }
            };
            if let Some(want) = info.manifest_hash.as_deref() {
                let got = hex_lower(Sha1::digest(&bytes));
                if !got.eq_ignore_ascii_case(want) {
                    last_error = Some(anyhow!(
                        "the manifest Fab served hashed to {got}; it promised {want}"
                    ));
                    continue;
                }
            }
            match Manifest::parse(&bytes) {
                Ok(manifest) => return Ok(resolved_from(info, &point.manifest_url, manifest)),
                Err(error) => {
                    last_error = Some(anyhow!(error).context("the manifest could not be parsed"));
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("Fab returned no distribution points to fetch from")))
}

/// The platform the artifact manifest is requested for. Fab delivers UE
/// builds per platform, and Windows is the one every catalog asset carries.
const PLATFORM: &str = "Windows";

/// Shape a parsed manifest into what a caller needs to start fetching.
fn resolved_from(info: &DownloadInfo, manifest_url: &str, manifest: Manifest) -> Resolved {
    let structure = structure_of_files(
        manifest
            .files
            .iter()
            .map(|file| (file.filename.as_str(), file.size)),
    );
    Resolved {
        // "asset-format/game-engine/unreal-engine" reads as its last part.
        format: info
            .asset_format
            .rsplit('/')
            .next()
            .filter(|label| !label.is_empty())
            .unwrap_or(&info.asset_format)
            .to_owned(),
        build_version: info.build_version.clone(),
        artifact: info.artifact_id.clone(),
        chunk_base: base_from_manifest_url(manifest_url),
        chunk_query: query_from_manifest_url(manifest_url),
        structure,
        manifest,
    }
}

/// Fetch a manifest blob from a CDN distribution point.
///
/// No Authorization header: the signed URL is the credential, and the CDN
/// answers a bearer token on the chunk tree with 501.
async fn fetch_manifest(client: &FabClient, url: &str) -> Result<Vec<u8>> {
    client.fetch_bytes(url).await.map_err(Into::into)
}

/// Lowercase hex, the shape SHA-1 digests are compared and reported in.
fn hex_lower(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Describe a set of files by name, with the size each one reported.
///
/// A manifest names every file the asset installs and says how big each is, so
/// the asset's texture maps and levels of detail are known before any of its
/// bytes have been fetched.
fn structure_of_files<'a>(files: impl IntoIterator<Item = (&'a str, u64)>) -> AssetStructure {
    AssetStructure::from_sized(
        files
            .into_iter()
            .map(|(filename, size)| (PathBuf::from(filename), size)),
    )
}

/// The manifest URL's directory, without its file name, query or fragment —
/// the CDN base every chunk path hangs off.
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

/// The manifest URL's query string with its leading `?`, or empty.
///
/// The CDN signs the whole `CloudDir/` tree with this one token, so every
/// chunk URL must replay it verbatim; an unsigned chunk answers 403.
fn query_from_manifest_url(manifest_url: &str) -> String {
    match manifest_url.find('?') {
        Some(start) => match manifest_url[start..].find('#') {
            Some(end) => manifest_url[start..start + end].to_owned(),
            None => manifest_url[start..].to_owned(),
        },
        None => String::new(),
    }
}

/// Join a manifest path onto the output root, refusing anything that escapes.
fn safe_join(root: &Path, name: &str) -> Result<PathBuf> {
    let relative = Path::new(name);
    // `is_absolute` is not enough on Windows, where `/etc/passwd` is rooted but
    // not absolute; RootDir and Prefix cover every rooted form on both.
    let escapes = relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        });
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
    use super::{
        base_from_manifest_url, item_for_id, owned_from_item, query_from_manifest_url, safe_join,
        structure_of_files,
    };
    use assetcraft_core::{LibraryItem, TextureRole};
    use std::path::Path;

    #[test]
    fn a_manifest_names_its_maps_and_levels_before_anything_is_fetched() {
        let structure = structure_of_files([
            ("Cloud/Rock/SM_Rock_LOD0.fbx", 4000u64),
            ("Cloud/Rock/SM_Rock_LOD1.fbx", 900),
            ("Cloud/Rock/T_Rock_BaseColor.png", 3000),
            ("Cloud/Rock/T_Rock_Roughness_LOD1.png", 200),
            ("Cloud/Rock/README.txt", 40),
        ]);

        let groups = structure.mesh_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "SM_Rock");
        assert_eq!(groups[0].levels, vec![0, 1]);
        assert_eq!(
            structure.role_counts(),
            vec![(TextureRole::BaseColor, 1), (TextureRole::Roughness, 1),]
        );
        // The README is not a map or a mesh, and its bytes are not in the total.
        assert_eq!(structure.other.len(), 1);
        assert_eq!(structure.known_size_bytes(), 8100);
        assert_eq!(
            structure.summary(),
            "2 texture maps · 1 mesh (LODs 0-1) · 1 other file"
        );
    }

    #[test]
    fn manifest_paths_read_correctly_whatever_separators_they_use() {
        // A manifest lists paths with `/` on every platform.
        let structure = structure_of_files([("Content/Rock/rock_normal.png", 10)]);
        assert_eq!(structure.textures[0].role, TextureRole::Normal);
        assert_eq!(structure.textures[0].material.as_deref(), Some("rock"));
    }

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
    fn a_manifest_url_shares_its_signature_with_every_chunk() {
        // The shape Fab really answers: a policy blob and a key pair id, all
        // of it one signature over the whole CloudDir tree.
        let url = "https://cdn.test/CloudDir/m.manifest?Policy=eyJhIjoxfQ~KM~abc&Key-Pair-Id=K2X";
        assert_eq!(base_from_manifest_url(url), "https://cdn.test/CloudDir");
        assert_eq!(
            query_from_manifest_url(url),
            "?Policy=eyJhIjoxfQ~KM~abc&Key-Pair-Id=K2X"
        );
        // The stub serves unsigned manifests; nothing replays then.
        assert_eq!(
            query_from_manifest_url("http://127.0.0.1:8791/stub/manifest.bin"),
            ""
        );
    }

    #[test]
    fn a_card_finds_its_owned_entry_through_the_listing_url() {
        // Live shapes: the card knows the hyphenated listing UUID, the
        // library entry is keyed by a bare hex uid, and the entry's URL is
        // the only place the two meet.
        let entry = LibraryItem {
            asset_id: "798525662a374bd8ab58f27a5b8619a5".to_owned(),
            namespace: "89efe5924d3d467c839449ab6ab52e7f".to_owned(),
            title: "Warehouse".to_owned(),
            url: Some(
                "https://www.fab.com/listings/edeb5874-d6e8-48e3-868e-a5cce4d2b828".to_owned(),
            ),
            artifact_id: Some("Warehous7e371bf75594V1".to_owned()),
            ..Default::default()
        };
        let items = [entry.clone()];
        let found = item_for_id(&items, "edeb5874-d6e8-48e3-868e-a5cce4d2b828")
            .expect("the listing UUID joins to the entry");
        assert_eq!(found.asset_id, entry.asset_id);
        // A hex asset id finds the same entry directly (the CLI's `library`).
        assert!(item_for_id(&items, "798525662a374bd8ab58f27a5b8619a5").is_some());
        assert!(item_for_id(&items, "someone-elses-listing").is_none());

        let owned = owned_from_item(found).expect("the entry is deliverable");
        assert_eq!(owned.artifact_id, "Warehous7e371bf75594V1");
        assert_eq!(owned.item_id, "798525662a374bd8ab58f27a5b8619a5");
        assert_eq!(owned.namespace, entry.namespace);
        // An entry without an artifact has no download route at all.
        assert!(owned_from_item(&LibraryItem::default()).is_none());
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
        #[cfg(windows)]
        assert!(safe_join(root, "C:\\Windows\\System32\\drivers\\etc\\hosts").is_err());
    }
}
