//! The `assetcraft` command-line interface.
//!
//! This is the headless entry point to the same engine the desktop app will
//! use. For M1 it covers the read side of the catalog plus sign-in.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};

use assetcraft_auth::{ensure_fresh_token, EpicAuth, FileTokenStore, TokenStore};
use assetcraft_catalog::{FabClient, ResponseCache, DEFAULT_TTL};
use assetcraft_core::{
    AssetFormat, AssetId, AssetSummary, Config, LibraryItem, Page, Preference, SearchQuery,
    SelectionCriteria, SortOrder,
};
use assetcraft_download::{
    BandwidthLimit, ChunkStore, DownloadFile, DownloadOptions, Downloader, Manifest,
};
use assetcraft_library::{Library, MediaCache};

/// Browse and download Megascans / Fab assets.
#[derive(Debug, Parser)]
#[command(name = "assetcraft", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Search the Fab catalog.
    Search(SearchArgs),
    /// Sign in to Epic Games so AssetCraft can reach your Fab library.
    Login(LoginArgs),
    /// List the assets in your Fab library (requires sign-in).
    Library(LibraryArgs),
    /// Download a single file, resuming and verifying as it goes.
    Fetch(FetchArgs),
    /// List an asset's downloadable formats and files.
    Formats(FormatsArgs),
    /// Resolve an owned asset into the chunks and files to fetch (requires sign-in).
    Resolve(ResolveArgs),
    /// Download an owned asset's files and reassemble them (requires sign-in).
    Download(DownloadArgs),
    /// Lay out an asset's downloaded files for a DCC and write the import script.
    Export(ExportArgs),
    /// Inspect the catalog's taxonomy.
    Taxonomy(TaxonomyArgs),
}

#[derive(Debug, Args)]
struct SearchArgs {
    /// Free-text query, e.g. "mossy rock".
    query: Option<String>,

    /// Restrict to listing types (repeatable), e.g. `3d-model`.
    #[arg(long = "type")]
    listing_types: Vec<String>,

    /// Restrict to category slugs (repeatable).
    #[arg(long)]
    category: Vec<String>,

    /// Ordering of the results.
    #[arg(long, value_enum)]
    sort: Option<SortArg>,

    /// Maximum number of results to print.
    #[arg(long, default_value_t = 24)]
    limit: u32,

    /// Only show free assets.
    #[arg(long)]
    free: bool,

    /// Continue from a cursor printed by a previous search.
    #[arg(long)]
    cursor: Option<String>,

    /// Print the raw JSON response instead of a summary.
    #[arg(long)]
    json: bool,

    /// Ignore the on-disk response cache and fetch fresh results.
    #[arg(long)]
    no_cache: bool,
}

/// Sort orders exposed on the command line.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum SortArg {
    Relevance,
    Newest,
    Oldest,
    PriceAsc,
    PriceDesc,
}

impl From<SortArg> for SortOrder {
    fn from(value: SortArg) -> Self {
        match value {
            SortArg::Relevance => SortOrder::Relevance,
            SortArg::Newest => SortOrder::Newest,
            SortArg::Oldest => SortOrder::Oldest,
            SortArg::PriceAsc => SortOrder::PriceLowToHigh,
            SortArg::PriceDesc => SortOrder::PriceHighToLow,
        }
    }
}

#[derive(Debug, Args)]
struct LoginArgs {
    /// The authorization code shown after signing in through the browser.
    #[arg(long)]
    code: Option<String>,
}

#[derive(Debug, Args)]
struct LibraryArgs {
    /// Browse the local library without contacting Fab.
    #[arg(long)]
    offline: bool,

    /// Search titles in the local library.
    #[arg(long)]
    search: Option<String>,

    /// Check the local library for unreadable entries and exit.
    #[arg(long)]
    verify: bool,

    /// Repair the local library: realign stale titles, drop unreadable rows.
    #[arg(long)]
    repair: bool,

    /// Cache thumbnails for the local library so it browses offline.
    #[arg(long)]
    thumbnails: bool,

    /// Print JSON instead of a summary.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct FetchArgs {
    /// URL to download.
    url: String,

    /// Where to write the file.
    #[arg(long, short)]
    output: PathBuf,

    /// Expected size in bytes; a mismatch is a failure.
    #[arg(long)]
    size: Option<u64>,

    /// Expected SHA-256 (hex); a mismatch is a failure.
    #[arg(long)]
    sha256: Option<String>,

    /// Total attempts per file, including the first.
    #[arg(long, default_value_t = 3)]
    attempts: u32,

    /// Cap the download rate, e.g. `500k` or `2M` (bytes per second).
    #[arg(long, value_parser = parse_rate)]
    limit: Option<u64>,
}

#[derive(Debug, Args)]
struct FormatsArgs {
    /// Asset id to inspect (as printed by `search`).
    asset_id: String,

    #[command(flatten)]
    select: SelectionArgs,

    /// Print the raw JSON response instead of a summary.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ResolveArgs {
    /// Asset id to resolve (as printed by `search`).
    asset_id: String,

    #[command(flatten)]
    select: SelectionArgs,

    /// Also list every chunk that will be fetched.
    #[arg(long)]
    chunks: bool,
}

#[derive(Debug, Args)]
struct DownloadArgs {
    /// Asset id to download (as printed by `search`).
    asset_id: String,

    #[command(flatten)]
    select: SelectionArgs,

    /// Directory to write the asset's files into.
    #[arg(long, short)]
    output: PathBuf,

    /// Base URL the manifest's chunk paths hang off, e.g. `…/CloudDir`.
    #[arg(long)]
    chunk_base: String,

    /// Total attempts per chunk, including the first.
    #[arg(long, default_value_t = 3)]
    attempts: u32,

    /// How many chunks to fetch at once.
    #[arg(long)]
    concurrency: Option<usize>,

    /// Cap the download rate, e.g. `500k` or `2M` (bytes per second).
    #[arg(long, value_parser = parse_rate)]
    limit: Option<u64>,
}

#[derive(Debug, Args)]
struct ExportArgs {
    /// Asset id to export (as printed by `search`).
    asset_id: String,

    /// Target application.
    #[arg(long, value_enum, default_value_t = DccArg::Blender)]
    dcc: DccArg,

    /// Directory holding the asset's downloaded files.
    ///
    /// Omit it to use the location `download` recorded in the library.
    #[arg(long, short)]
    source: Option<PathBuf>,

    /// Directory to lay the export out under.
    #[arg(long, short)]
    target: PathBuf,

    /// Format code the files came from, used in the export directory name.
    ///
    /// Defaults to the format `download` recorded when sourcing from the
    /// library, or `asset` otherwise.
    #[arg(long)]
    format: Option<String>,

    /// Only build the materials; do not import the meshes or assign them.
    #[arg(long)]
    no_mesh_import: bool,

    /// Hand the generated script to Blender.
    #[arg(long)]
    run: bool,

    /// The Blender binary to run, or the one on PATH.
    #[arg(long)]
    blender: Option<PathBuf>,
}

/// The DCC targets `export` understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum DccArg {
    Blender,
}

/// Selection flags shared by `formats` and `resolve`.
#[derive(Debug, Args)]
struct SelectionArgs {
    /// Only consider this format code, e.g. `fbx`.
    #[arg(long)]
    format: Option<String>,

    /// Only consider files whose name or asset type contains this text, e.g. `4K`.
    #[arg(long)]
    quality: Option<String>,

    /// Only consider files that support this platform, e.g. `Windows`.
    #[arg(long)]
    platform: Option<String>,

    /// Only consider files that support this engine version, e.g. `5.4`.
    #[arg(long)]
    engine: Option<String>,

    /// How to choose among several matching files.
    #[arg(long, value_enum)]
    prefer: Option<PreferArg>,
}

impl SelectionArgs {
    /// Build the selection criteria these flags describe.
    fn criteria(&self) -> SelectionCriteria {
        SelectionCriteria {
            format: self.format.clone(),
            quality: self.quality.clone(),
            platform: self.platform.clone(),
            engine_version: self.engine.clone(),
            preference: self.prefer.map(Into::into).unwrap_or_default(),
        }
    }
}

/// Tie-breaks exposed on the command line.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum PreferArg {
    First,
    Largest,
    Smallest,
}

impl From<PreferArg> for Preference {
    fn from(value: PreferArg) -> Self {
        match value {
            PreferArg::First => Preference::First,
            PreferArg::Largest => Preference::Largest,
            PreferArg::Smallest => Preference::Smallest,
        }
    }
}

#[derive(Debug, Args)]
struct TaxonomyArgs {
    #[command(subcommand)]
    which: TaxonomyCommand,
}

#[derive(Debug, Subcommand)]
enum TaxonomyCommand {
    /// List available license types.
    Licenses,
    /// List tag groups and their tags.
    Tags,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let config = Config::load().context("could not determine AssetCraft directories")?;

    match cli.command {
        Command::Search(args) => search(&config, args).await,
        Command::Login(args) => login(&config, args).await,
        Command::Library(args) => library(&config, args).await,
        Command::Fetch(args) => fetch(args).await,
        Command::Formats(args) => formats(&config, args).await,
        Command::Resolve(args) => resolve(&config, args).await,
        Command::Download(args) => download(&config, args).await,
        Command::Export(args) => export_asset(&config, args),
        Command::Taxonomy(args) => taxonomy(&config, args).await,
    }
}

async fn search(config: &Config, args: SearchArgs) -> Result<()> {
    let mut client = FabClient::from_config(config)?;
    if !args.no_cache {
        client = client.with_cache(ResponseCache::new(
            config.paths.response_cache_dir(),
            DEFAULT_TTL,
        ));
    }
    let query = SearchQuery {
        text: args.query,
        listing_types: args.listing_types,
        categories: args.category,
        sort_by: args.sort.map(Into::into),
        limit: Some(args.limit),
        cursor: args.cursor,
        free_only: args.free,
        ..Default::default()
    };
    tracing::debug!(?query, "searching the Fab catalog");

    let page = client.search(&query).await.context("search failed")?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&page)?);
    } else {
        print_page(&page);
    }
    Ok(())
}

async fn login(config: &Config, args: LoginArgs) -> Result<()> {
    let auth = EpicAuth::from_config(config)?;
    let store = FileTokenStore::new(config.paths.token_file());

    let Some(code) = args.code else {
        println!("Sign in to Epic Games to let AssetCraft reach your Fab library.\n");
        println!("1. Open this URL in a browser and sign in:");
        println!();
        println!("     {}", auth.authorization_url());
        println!();
        println!("2. The page shows a JSON object; copy the value of \"code\".");
        println!("3. Return here and run:");
        println!();
        println!("     assetcraft login --code <CODE>");
        return Ok(());
    };

    let token = auth.exchange_code(&code).await.context("sign-in failed")?;
    store.save(&token).context("could not store the token")?;

    match token.display_name.as_deref() {
        Some(name) => println!("Signed in as {name}."),
        None => println!("Signed in."),
    }
    println!("Token stored at {}.", store.path().display());
    Ok(())
}

async fn library(config: &Config, args: LibraryArgs) -> Result<()> {
    let library =
        Library::open(config.paths.library_file()).context("could not open the local library")?;
    let media = MediaCache::new(config.paths.thumbnail_cache_dir());

    if args.verify {
        let report = library.verify().context("could not check the library")?;
        if report.is_healthy() {
            println!("{} assets, all readable.", report.total);
            return Ok(());
        }
        println!(
            "{} assets, {} unreadable:",
            report.total,
            report.unreadable.len()
        );
        for row in &report.unreadable {
            println!("  {}: {}", row.id, row.problem);
        }
        anyhow::bail!("the local library has unreadable entries");
    }

    if args.repair {
        let report = library.repair().context("could not repair the library")?;
        if report.titles_repaired > 0 {
            println!("Realigned {} stale title(s).", report.titles_repaired);
        }
        if report.removed.is_empty() {
            println!("{} assets, nothing to drop.", report.total);
        } else {
            println!(
                "Dropped {} unreadable entr{}:",
                report.removed.len(),
                if report.removed.len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            );
            for row in &report.removed {
                println!("  {}: {}", row.id, row.problem);
            }
        }
        return Ok(());
    }

    if args.thumbnails {
        return cache_thumbnails(config, &library, &media).await;
    }

    if let Some(needle) = args.search.as_deref() {
        let items = library.search(needle).context("search failed")?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&items)?);
        } else {
            print_library_items(&items, &media, &library);
        }
        return Ok(());
    }

    if args.offline {
        let items = library.list_all().context("could not read the library")?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&items)?);
        } else {
            print_library_items(&items, &media, &library);
        }
        return Ok(());
    }

    // Online: pull the whole library from Fab and record it locally.
    let auth = EpicAuth::from_config(config)?;
    let store = FileTokenStore::new(config.paths.token_file());
    let token = ensure_fresh_token(&auth, &store)
        .await
        .context("not signed in; run `assetcraft login` first")?;
    let account_id = token
        .account_id
        .clone()
        .context("the stored token has no account id; run `assetcraft login` again")?;

    let client = FabClient::from_config(config)?.with_token(token.authorization_header());

    let mut fetched = 0usize;
    let mut added = 0usize;
    let mut cursor: Option<String> = None;
    loop {
        let page = client
            .library(&account_id, cursor.as_deref())
            .await
            .context("could not load your Fab library")?;
        fetched += page.items.len();
        for item in &page.items {
            if library.insert(item).context("could not store an asset")? {
                added += 1;
            }
        }
        // Stop when the server offers no further cursor, or repeats an empty
        // page, so a misbehaving cursor cannot spin forever.
        match page.next_cursor {
            Some(next) if !page.items.is_empty() => cursor = Some(next),
            _ => break,
        }
    }

    println!(
        "Fetched {fetched} assets ({added} new). The local library holds {} assets.\n",
        library.len()?
    );

    let items = library.list_all().context("could not read the library")?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        print_library_items(&items, &media, &library);
    }
    Ok(())
}

/// Cache the thumbnail of every library item that has one.
async fn cache_thumbnails(config: &Config, library: &Library, media: &MediaCache) -> Result<()> {
    let client = FabClient::from_config(config)?;
    let items = library.list_all().context("could not read the library")?;

    let mut cached = 0usize;
    let mut already = 0usize;
    let mut without = 0usize;
    let mut failed = 0usize;

    for item in &items {
        let Some(url) = item.thumbnail_url() else {
            without += 1;
            continue;
        };
        if media.contains(url) {
            already += 1;
            continue;
        }
        match client.fetch_bytes(url).await {
            Ok(bytes) => {
                media
                    .put(url, &bytes)
                    .context("could not store a thumbnail")?;
                cached += 1;
            }
            Err(err) => {
                failed += 1;
                eprintln!("  {}: {err}", item.asset_id);
            }
        }
    }

    println!(
        "Cached {cached} thumbnails ({already} already cached, {without} without one, {failed} failed)."
    );
    println!("Cache: {}", media.root().display());
    Ok(())
}

async fn fetch(args: FetchArgs) -> Result<()> {
    let options = DownloadOptions {
        max_attempts: args.attempts,
        bandwidth: args.limit.map(BandwidthLimit::new),
        ..Default::default()
    };
    let downloader = Downloader::with_default_client(options)?;
    let file = DownloadFile {
        url: args.url,
        destination: args.output,
        size: args.size,
        sha256: args.sha256,
    };

    let outcome = downloader
        .download(&file)
        .await
        .context("download failed")?;

    println!(
        "Downloaded {} bytes to {}",
        outcome.bytes,
        outcome.destination.display()
    );
    if outcome.resumed_from > 0 {
        println!(
            "Resumed from {} bytes already on disk.",
            outcome.resumed_from
        );
    }
    if outcome.attempts > 1 {
        println!("Succeeded on attempt {}.", outcome.attempts);
    }
    Ok(())
}

/// Parse a bandwidth rate such as `500k`, `2M`, or a plain byte count.
fn parse_rate(raw: &str) -> Result<u64, String> {
    let raw = raw.trim();
    let (number, multiplier) = match raw.chars().last() {
        Some('k' | 'K') => (&raw[..raw.len() - 1], 1_000u64),
        Some('m' | 'M') => (&raw[..raw.len() - 1], 1_000_000),
        Some('g' | 'G') => (&raw[..raw.len() - 1], 1_000_000_000),
        _ => (raw, 1),
    };
    number
        .trim()
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("`{raw}` is not a rate like `500k` or `2M`"))
}

async fn formats(config: &Config, args: FormatsArgs) -> Result<()> {
    let client = FabClient::from_config(config)?;
    let formats = client
        .listing_formats(&AssetId::new(&args.asset_id))
        .await
        .context("could not load asset formats")?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&formats)?);
        return Ok(());
    }

    print_formats(&formats);

    let criteria = args.select.criteria();
    // Only report a choice when the caller asked for one.
    if criteria.is_unconstrained() && args.select.prefer.is_none() {
        return Ok(());
    }

    let chosen = assetcraft_core::select(&formats, &criteria).context("selection failed")?;
    let label = chosen
        .format_name
        .as_deref()
        .or(chosen.format_code.as_deref())
        .unwrap_or("-");
    println!();
    println!("Selected: {label}");
    match chosen.file_id() {
        Some(id) => println!("  file id: {id}"),
        None => println!("  file id: (none reported)"),
    }
    if let Some(name) = chosen.file.name.as_deref() {
        println!("  name: {name}");
    }
    if let Some(size) = chosen.file.size_bytes {
        println!("  size: {size} bytes");
    }
    Ok(())
}

fn print_formats(formats: &[AssetFormat]) {
    if formats.is_empty() {
        println!("No downloadable formats reported for this asset.");
        return;
    }
    for format in formats {
        let code = format.code().unwrap_or("-");
        let name = format
            .format_type
            .as_ref()
            .and_then(|kind| kind.name.as_deref())
            .unwrap_or(code);
        println!("{name} ({code})");
        if format.files.is_empty() {
            println!("  (no files)");
        }
        for file in &format.files {
            let id = file.id.as_deref().unwrap_or("-");
            let size = match file.size_bytes {
                Some(bytes) => format!("{bytes} bytes"),
                None => "size unknown".to_string(),
            };
            println!("  {id}  {size}");
            if let Some(file_name) = file.name.as_deref() {
                println!("    name: {file_name}");
            }
            if let Some(asset_type) = file.asset_type.as_deref() {
                println!("    type: {asset_type}");
            }
            if !file.platforms.is_empty() {
                println!("    platforms: {}", file.platforms.join(", "));
            }
            if !file.engine_versions.is_empty() {
                println!("    engine: {}", file.engine_versions.join(", "));
            }
        }
        println!();
    }
}

/// Resolve an owned asset into the chunks and files Fab's manifest describes.
///
/// This composes the public format listing, the signed download info, and the
/// binary manifest parser. It is the first end-to-end path from an asset id to
/// "what to fetch".
async fn resolve(config: &Config, args: ResolveArgs) -> Result<()> {
    let client = FabClient::from_config(config)?;
    let criteria = args.select.criteria();
    let (_format_code, manifest) = resolve_manifest(&client, &args.asset_id, &criteria).await?;
    print_manifest(&manifest, args.chunks);
    Ok(())
}

/// Select a format and file, fetch the signed download info, and parse the
/// manifest. Shared by `resolve` and `download`.
async fn resolve_manifest(
    client: &FabClient,
    asset_id: &str,
    criteria: &SelectionCriteria,
) -> Result<(String, Manifest)> {
    let formats = client
        .listing_formats(&AssetId::new(asset_id))
        .await
        .context("could not load asset formats")?;

    let chosen = assetcraft_core::select(&formats, criteria).context("selection failed")?;
    let format_code = chosen
        .format_code
        .clone()
        .context("the selected format has no code to request download info with")?;
    let file_id = chosen
        .file_id()
        .map(str::to_string)
        .context("the selected file has no id to request download info with")?;

    println!("Format:   {format_code}");
    println!(
        "File:     {} ({file_id})",
        chosen.file.name.as_deref().unwrap_or("-")
    );

    let info = client
        .download_info(asset_id, &format_code, &file_id)
        .await
        .context("could not resolve download info")?;
    println!("Artifact: {}", info.artifact_id);
    if info.distribution_points.is_empty() {
        anyhow::bail!("Fab returned no distribution points for this file");
    }

    // Distribution points are mirrors; use the first one whose manifest parses.
    let mut last_error = None;
    for point in &info.distribution_points {
        let bytes = match client.fetch_bytes(&point.manifest_url).await {
            Ok(bytes) => bytes,
            Err(err) => {
                last_error = Some(anyhow::Error::new(err).context("manifest could not be fetched"));
                continue;
            }
        };
        match Manifest::parse(&bytes) {
            Ok(manifest) => return Ok((format_code, manifest)),
            Err(err) => {
                last_error = Some(anyhow::Error::new(err).context("manifest could not be parsed"));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no distribution point produced a manifest")))
}

/// Download an owned asset's chunks, reassemble its files, and verify them.
///
/// The chunk base URL is not derivable from the manifest, so it is passed in;
/// once a real signed-in response confirms how Fab pairs it with the manifest,
/// this can be filled in automatically.
async fn download(config: &Config, args: DownloadArgs) -> Result<()> {
    let client = FabClient::from_config(config)?;
    let criteria = args.select.criteria();
    let (format_code, manifest) = resolve_manifest(&client, &args.asset_id, &criteria).await?;

    println!(
        "{} files, {} chunks ({} bytes to download).",
        manifest.files.len(),
        manifest.chunks.len(),
        manifest.total_download_size()
    );

    // Chunks land in a scratch directory beside the output, then are removed.
    let scratch = args.output.join(".assetcraft-chunks");
    std::fs::create_dir_all(&scratch).context("could not create the scratch directory")?;

    let chunk_files: Vec<DownloadFile> = manifest
        .chunk_downloads(&args.chunk_base)
        .into_iter()
        .map(|chunk| DownloadFile {
            url: chunk.url,
            destination: scratch.join(format!("{}.chunk", chunk.guid)),
            size: Some(chunk.size_bytes),
            sha256: None,
        })
        .collect();

    let options = DownloadOptions {
        max_attempts: args.attempts,
        concurrency: args
            .concurrency
            .unwrap_or_else(|| DownloadOptions::default().concurrency),
        bandwidth: args.limit.map(BandwidthLimit::new),
        ..Default::default()
    };
    let downloader = Downloader::with_default_client(options)?;
    let outcomes = downloader
        .download_all(chunk_files, |progress| {
            if progress.bytes_total == Some(progress.bytes_done) {
                eprintln!("  fetched chunk {}/{}", progress.index + 1, progress.total);
            }
        })
        .await
        .context("chunk download failed")?;

    let mut store = ChunkStore::new();
    for outcome in &outcomes {
        let bytes =
            std::fs::read(&outcome.destination).context("could not read a downloaded chunk")?;
        store
            .insert_file(&bytes)
            .context("could not decode a downloaded chunk")?;
    }

    let files = manifest
        .assemble(&store)
        .context("could not reassemble the asset")?;
    for file in &files {
        let path = safe_join(&args.output, &file.filename)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("could not create a directory")?;
        }
        std::fs::write(&path, &file.data)
            .with_context(|| format!("could not write {}", path.display()))?;
    }

    let _ = std::fs::remove_dir_all(&scratch);
    println!("Wrote {} files to {}.", files.len(), args.output.display());

    // Record where the files landed so `export` can find them again by id with
    // the network off. The path is stored absolute, so the record survives a
    // different working directory.
    let stored = std::fs::canonicalize(&args.output).unwrap_or_else(|_| args.output.clone());
    let library =
        Library::open(config.paths.library_file()).context("could not open the local library")?;
    library
        .record_download(&AssetId::new(&args.asset_id), &stored, &format_code)
        .context("could not record the download in the library")?;
    println!(
        "Recorded it in the library — re-export offline with: assetcraft export {} --target <dir>",
        args.asset_id
    );
    Ok(())
}

/// Join a manifest path onto the output root, refusing anything that escapes.
fn safe_join(root: &Path, name: &str) -> Result<PathBuf> {
    let relative = Path::new(name);
    let escapes = relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir));
    if escapes {
        anyhow::bail!("the manifest lists a path outside the output directory: {name:?}");
    }
    Ok(root.join(relative))
}

/// Lay an asset's downloaded files out for a DCC and write the import script.
///
/// The files come from `--source` when given, otherwise from the location
/// `download` recorded in the library, which is what makes an asset you already
/// own re-exportable with the network off.
fn export_asset(config: &Config, args: ExportArgs) -> Result<()> {
    let dcc = match args.dcc {
        DccArg::Blender => "blender",
    };

    let asset = AssetId::new(&args.asset_id);
    let recorded = if args.source.is_none() || args.format.is_none() {
        Library::open(config.paths.library_file())
            .context("could not open the local library")?
            .download(&asset)
            .context("could not read the library")?
    } else {
        None
    };

    let source = match (args.source, &recorded) {
        (Some(source), _) => source,
        (None, Some(download)) => download.path.clone(),
        (None, None) => anyhow::bail!(
            "{} is not in the library as a download; pass --source, or run \
             `assetcraft download {}` first",
            args.asset_id,
            args.asset_id
        ),
    };
    let format = args
        .format
        .clone()
        .or_else(|| recorded.map(|d| d.format).filter(|f| !f.is_empty()))
        .unwrap_or_else(|| "asset".to_string());

    let files =
        collect_files(&source).with_context(|| format!("could not read {}", source.display()))?;
    if files.is_empty() {
        anyhow::bail!("{} holds no files to export", source.display());
    }

    let plan = assetcraft_dcc_blender::plan(&args.target, &asset, &format, &files);
    let options = assetcraft_dcc_blender::ExportOptions {
        import_meshes: !args.no_mesh_import,
        run: args.run,
        blender: args.blender.clone(),
    };
    let report = assetcraft_dcc_blender::export(&plan, &source, &options)
        .context("could not export the asset")?;

    println!(
        "Laid out {} files as {} material(s) for {dcc} in {}",
        report.copied,
        report.materials,
        report.dir.display()
    );
    println!("Script: {}", report.script.display());
    if report.ran {
        match &report.blender {
            Some(blender) => println!("Ran {}", blender.display()),
            None => println!("Ran Blender"),
        }
    } else {
        println!(
            "Run it with: blender --background --python {}",
            report.script.display()
        );
    }
    Ok(())
}

/// Every file under `root`, as paths relative to it, sorted for a stable plan.
fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
            } else if let Ok(relative) = path.strip_prefix(root) {
                files.push(relative.to_path_buf());
            }
        }
    }
    files.sort();
    Ok(files)
}

fn print_manifest(manifest: &Manifest, show_chunks: bool) {
    println!();
    println!("Manifest version {}", manifest.version);
    if !manifest.meta.app_name.is_empty() {
        println!("  app: {}", manifest.meta.app_name);
    }
    if !manifest.meta.build_version.is_empty() {
        println!("  build: {}", manifest.meta.build_version);
    }
    println!(
        "  chunks: {} ({} bytes to download)",
        manifest.chunks.len(),
        manifest.total_download_size()
    );
    println!(
        "  files: {} ({} bytes installed)",
        manifest.files.len(),
        manifest.total_install_size()
    );

    if show_chunks {
        println!();
        println!("Chunks:");
        for chunk in &manifest.chunks {
            println!(
                "  {}  {} bytes",
                chunk.relative_path(manifest.meta.feature_level),
                chunk.file_size.max(0)
            );
        }
    }

    println!();
    println!("Files:");
    for file in &manifest.files {
        let executable = if file.is_executable() {
            " (executable)"
        } else {
            ""
        };
        println!("  {}  {} bytes{executable}", file.filename, file.size);
    }
}

async fn taxonomy(config: &Config, args: TaxonomyArgs) -> Result<()> {
    let client = FabClient::from_config(config)?;
    match args.which {
        TaxonomyCommand::Licenses => {
            for license in client.licenses().await.context("could not load licenses")? {
                println!(
                    "{}\t{}",
                    license.slug.as_deref().unwrap_or("-"),
                    license.name.as_deref().unwrap_or("-")
                );
            }
        }
        TaxonomyCommand::Tags => {
            for group in client.tag_groups().await.context("could not load tags")? {
                println!("{}", group.name.as_deref().unwrap_or("-"));
                for tag in &group.tags {
                    println!("  {}", tag.name.as_deref().unwrap_or("-"));
                }
            }
        }
    }
    Ok(())
}

fn print_page(page: &Page<AssetSummary>) {
    match page.total {
        Some(total) => println!("{total} results (showing {})\n", page.items.len()),
        None => println!("{} results\n", page.items.len()),
    }

    for item in &page.items {
        println!("{}", item.display_title());
        let mut meta = format!("  id: {}", item.id);
        if let Some(seller) = item.seller_name() {
            meta.push_str(&format!("  seller: {seller}"));
        }
        if item.is_free {
            meta.push_str("  [free]");
        }
        println!("{meta}");

        if let Some(listing_type) = &item.listing_type {
            println!("  type: {listing_type}");
        }
        if let Some(category) = item.category.as_ref().and_then(|c| c.name.as_deref()) {
            println!("  category: {category}");
        }
        let tags: Vec<&str> = item.tags.iter().filter_map(|t| t.name.as_deref()).collect();
        if !tags.is_empty() {
            println!("  tags: {}", tags.join(", "));
        }
        if let Some(average) = item.rating.as_ref().and_then(|r| r.average) {
            println!("  rating: {average:.1}");
        }
        println!();
    }

    if let Some(cursor) = &page.next_cursor {
        println!("Next page: assetcraft search --cursor {cursor}");
    }
}

fn print_library_items(items: &[LibraryItem], media: &MediaCache, library: &Library) {
    println!("{} assets in your library\n", items.len());

    for item in items {
        let title = if item.title.is_empty() {
            item.asset_id.as_str()
        } else {
            item.title.as_str()
        };
        println!("{title}");
        println!("  id: {}", item.asset_id);
        if !item.namespace.is_empty() {
            println!("  namespace: {}", item.namespace);
        }
        if let Some(url) = item.url.as_deref() {
            println!("  url: {url}");
        }
        if let Some(url) = item.thumbnail_url() {
            match media.cached_path(url) {
                Some(path) => println!("  thumbnail: {} (cached)", path.display()),
                None => println!("  thumbnail: {url}"),
            }
        }
        let categories: Vec<&str> = item
            .categories
            .iter()
            .filter_map(|category| category.name.as_deref())
            .collect();
        if !categories.is_empty() {
            println!("  categories: {}", categories.join(", "));
        }
        // Show what is on disk and re-exportable offline, and flag a record
        // whose files have since been moved or deleted.
        let on_disk = library
            .download(&AssetId::new(&item.asset_id))
            .ok()
            .flatten()
            .map(|download| {
                if download.path.exists() {
                    format!("  downloaded: {} [exportable]", download.path.display())
                } else {
                    format!("  downloaded: {} [files missing]", download.path.display())
                }
            });
        if let Some(on_disk) = on_disk {
            println!("{on_disk}");
        }
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rate_understands_suffixes() {
        assert_eq!(parse_rate("512").unwrap(), 512);
        assert_eq!(parse_rate("500k").unwrap(), 500_000);
        assert_eq!(parse_rate("2M").unwrap(), 2_000_000);
        assert_eq!(parse_rate("1g").unwrap(), 1_000_000_000);
        assert_eq!(parse_rate(" 10 K ").unwrap(), 10_000);
    }

    #[test]
    fn parse_rate_rejects_nonsense() {
        assert!(parse_rate("").is_err());
        assert!(parse_rate("fast").is_err());
        assert!(parse_rate("0").is_err());
        assert!(parse_rate("-5k").is_err());
    }
}
