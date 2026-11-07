//! The `assetcraft` command-line interface.
//!
//! This is the headless entry point to the same engine the desktop app will
//! use. For M1 it covers the read side of the catalog plus sign-in.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};

use assetcraft_auth::{ensure_fresh_token, DefaultTokenStore, EpicAuth, TokenStore};
use assetcraft_catalog::{FabClient, ResponseCache, DEFAULT_TTL};
use assetcraft_core::{
    AssetFormat, AssetId, AssetStructure, AssetSummary, Config, LibraryItem, Page, Paths,
    Preference, SearchQuery, SelectionCriteria, Settings, SortOrder, TextureMap,
};
use assetcraft_download::{BandwidthLimit, DownloadFile, DownloadOptions, Downloader, Manifest};
use assetcraft_library::{Download, Library, MediaCache};
use assetcraft_transfer::Phase;

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
    /// Describe an asset's texture maps and levels of detail.
    Structure(StructureArgs),
    /// Resolve an owned asset into the chunks and files to fetch (requires sign-in).
    Resolve(ResolveArgs),
    /// Download or re-download an owned asset's files and reassemble them (requires sign-in).
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
struct StructureArgs {
    /// Asset id to describe (as printed by `search` or `library`).
    ///
    /// Either shape works: the hyphenated listing UUID from the catalog or
    /// the bare hex id from your owned library. Fab answers through the
    /// owned entry's artifact, which is where downloads go too.
    asset_id: String,

    /// Describe the files already on disk, with no connection.
    ///
    /// Reads the location `download` recorded for the asset, so an asset you own
    /// can be inspected with the network off and without signing in.
    #[arg(long)]
    offline: bool,

    /// Describe the files in this directory rather than the recorded location.
    #[arg(long, short)]
    source: Option<PathBuf>,

    /// Print the structure as JSON instead of a summary.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ResolveArgs {
    /// Asset id to resolve (as printed by `search` or `library`).
    ///
    /// Fab delivers the owned Unreal build and nothing else to third
    /// parties, so there is nothing to select: the id names the asset and
    /// your library names the build.
    asset_id: String,

    /// Also list every chunk that will be fetched.
    #[arg(long)]
    chunks: bool,
}

#[derive(Debug, Args)]
struct DownloadArgs {
    /// Asset id to download (as printed by `search` or `library`).
    asset_id: String,

    /// Directory to write the asset's files into.
    ///
    /// Omit it to write back to the location the library recorded for the
    /// asset — what makes `assetcraft download <asset-id>` on its own a
    /// re-download — or, for a first download, the standard asset directory.
    #[arg(long, short)]
    output: Option<PathBuf>,

    /// Base URL the manifest's chunk paths hang off, e.g. `…/CloudDir`.
    ///
    /// Omit it to use the manifest URL's own directory, which is where the
    /// live CDN serves chunks. Pass it only to point transfers at another
    /// server — a local test stub — which drops the CDN signature.
    #[arg(long)]
    chunk_base: Option<String>,

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
        Command::Structure(args) => structure(&config, args).await,
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
    let store = DefaultTokenStore::from_config(config);

    let Some(code) = args.code else {
        println!("Sign in to Epic Games to let AssetCraft reach your Fab library.\n");
        println!("1. Open this URL in a browser and sign in:");
        println!();
        println!("     {}", auth.authorization_url());
        println!();
        println!("2. The page shows a JSON object; copy the value of \"authorizationCode\".");
        println!("   If it says null, sign in to epicgames.com in that browser tab,");
        println!("   then open the URL again - the code appears once you are signed in.");
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
    println!("Token stored in {}.", store.location());
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
    let store = DefaultTokenStore::from_config(config);
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

/// Describe what an asset's files contain: its texture maps and levels of detail.
///
/// The reading is `assetcraft-core`'s, shared with the exporters and the desktop
/// app; what this command decides is where the file list comes from. Fab's
/// manifest names every file the asset installs and its exact size, which is the
/// answer for an asset you are about to download. The files a previous download
/// left on disk give the same answer with the network off, which is what makes
/// an owned asset inspectable offline.
async fn structure(config: &Config, args: StructureArgs) -> Result<()> {
    let (structure, origin) = match (&args.source, args.offline) {
        (Some(source), _) => (
            structure_on_disk(source)?,
            format!("{} on disk", source.display()),
        ),
        (None, true) => {
            let library = Library::open(config.paths.library_file())
                .context("could not open the local library")?;
            let recorded = library
                .download(&AssetId::new(&args.asset_id))?
                .with_context(|| {
                    format!(
                        "the library has no download recorded for {}; drop --offline to ask Fab",
                        args.asset_id
                    )
                })?;
            (
                structure_on_disk(&recorded.path)?,
                format!("{} on disk", recorded.path.display()),
            )
        }
        (None, false) => {
            let (client, account_id) = signed_in_client(config).await?;
            let (_, owned) =
                assetcraft_transfer::owned_for(&client, &account_id, &args.asset_id).await?;
            let resolved = assetcraft_transfer::resolve(&client, &owned).await?;
            (
                resolved.structure,
                format!("the manifest for the {} build", resolved.format),
            )
        }
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&structure)?);
        return Ok(());
    }
    print_structure(&structure, &origin);
    Ok(())
}

/// Read an asset's texture maps and levels of detail from a directory on disk.
///
/// The sizes come from the files themselves, so the offline answer carries the
/// same detail as the manifest one.
fn structure_on_disk(root: &Path) -> Result<AssetStructure> {
    let files = assetcraft_dcc_blender::collect_files(root)
        .with_context(|| format!("could not read the files under {}", root.display()))?;
    if files.is_empty() {
        bail!("no files found under {}", root.display());
    }
    let sized = files.into_iter().map(|relative| {
        let size = std::fs::metadata(root.join(&relative))
            .ok()
            .map(|meta| meta.len());
        (relative, size)
    });
    Ok(AssetStructure::from_sized(sized))
}

/// The detail behind the summary: every map by material, every mesh by level.
fn print_structure(structure: &AssetStructure, origin: &str) {
    println!("Source:   {origin}");
    println!("Contents: {}", structure.summary());

    if structure.is_empty() {
        return;
    }

    if !structure.textures.is_empty() {
        println!();
        println!("Texture maps ({}):", structure.textures.len());
        for material in structure.material_names() {
            println!("  {material}:");
            print_maps(&structure.maps_for(Some(material.as_str())));
        }
        let unlabelled = structure.maps_for(None);
        if !unlabelled.is_empty() {
            println!("  (no material in the name):");
            print_maps(&unlabelled);
        }
    }

    if !structure.meshes.is_empty() {
        println!();
        let groups = structure.mesh_groups();
        println!("Meshes ({}):", groups.len());
        for group in groups {
            let files = if group.files == 1 { "file" } else { "files" };
            let levels = if group.levels.is_empty() {
                "no level named".to_string()
            } else {
                format!("levels {}", join_levels(&group.levels))
            };
            println!("  {} — {} {files}, {levels}", group.name, group.files);
        }
        if !structure.has_lods() {
            println!(
                "  (no mesh names a level; one may still pack its levels inside its own file)"
            );
        }
    }

    if !structure.other.is_empty() {
        println!();
        println!("Other files ({}):", structure.other.len());
        for path in &structure.other {
            println!("  {}", path.display());
        }
    }
}

/// The maps of one material, aligned by role.
fn print_maps(maps: &[&TextureMap]) {
    for map in maps {
        let level = map
            .lod_level
            .map(|level| format!("  [level {level}]"))
            .unwrap_or_default();
        println!(
            "    {:<18} {}{level}",
            map.role.as_str(),
            map.path.display()
        );
    }
}

/// Levels as a comma-separated list.
fn join_levels(levels: &[u32]) -> String {
    levels
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve an owned asset into the chunks and files Fab's manifest describes.
///
/// This composes the owned-library lookup, the artifact manifest, and the
/// binary manifest parser. It is the first end-to-end path from an asset id
/// to "what to fetch" — the same path `download` then walks.
async fn resolve(config: &Config, args: ResolveArgs) -> Result<()> {
    let (client, account_id) = signed_in_client(config).await?;
    let (_, owned) = assetcraft_transfer::owned_for(&client, &account_id, &args.asset_id).await?;
    let resolved = assetcraft_transfer::resolve(&client, &owned).await?;

    println!("Format:   {}", resolved.format);
    println!("Artifact: {}", resolved.artifact);
    if let Some(build) = &resolved.build_version {
        println!("Build:    {build}");
    }
    println!("Contents: {}", resolved.structure.summary());
    print_manifest(&resolved.manifest, args.chunks);
    Ok(())
}

/// A client carrying the stored account's token, and the account id it belongs
/// to.
///
/// Fab answers downloads only for the account that owns the asset: the
/// artifact manifest is posted with that account's token, and the library
/// that names the artifact is keyed by the account id the token carries.
async fn signed_in_client(config: &Config) -> Result<(FabClient, String)> {
    let auth = EpicAuth::from_config(config)?;
    let store = DefaultTokenStore::from_config(config);
    let token = ensure_fresh_token(&auth, &store)
        .await
        .context("not signed in; run `assetcraft login` first")?;
    let account_id = token
        .account_id
        .clone()
        .context("the stored token has no account id; run `assetcraft login` again")?;
    Ok((
        FabClient::from_config(config)?.with_token(token.authorization_header()),
        account_id,
    ))
}

/// Download an owned asset's chunks, reassemble its files, and verify them.
///
/// The transfer itself lives in `assetcraft-transfer`, which the GUI uses too;
/// this command supplies the output directory and chunk base, then records
/// where the files landed. With no `--output` the files go back to the
/// location the library holds for the asset — or, the first time, to the
/// standard asset directory — so re-downloading takes only the asset id. With
/// no `--chunk-base` the base and signature are the manifest URL's own, which
/// is how the live CDN serves chunks.
async fn download(config: &Config, args: DownloadArgs) -> Result<()> {
    let (client, account_id) = signed_in_client(config).await?;
    // Fab delivers the Unreal build of an owned asset, and the account's
    // library is what names that build; either id shape joins to an entry.
    let (entry, owned) =
        assetcraft_transfer::owned_for(&client, &account_id, &args.asset_id).await?;
    let library =
        Library::open(config.paths.library_file()).context("could not open the local library")?;
    // Recorded downloads are keyed by the hex asset id, whatever id shape
    // the caller typed.
    let recorded = library
        .download(&AssetId::new(&owned.item_id))
        .context("could not read the library")?;

    let settings = Settings::load(&config.paths).context("could not read the settings")?;
    let (output, source) = download_output(
        args.output,
        recorded,
        &settings,
        &config.paths,
        &owned.item_id,
    );
    match source {
        OutputSource::Flag => {}
        OutputSource::Recorded => println!(
            "Re-downloading to the location the library recorded: {}",
            output.display()
        ),
        OutputSource::Settings => println!(
            "First download: writing under the settings download_dir: {}",
            output.display()
        ),
        OutputSource::Standard => println!(
            "First download: writing to the standard asset directory: {}",
            output.display()
        ),
    }

    let request = assetcraft_transfer::Request {
        owned: owned.clone(),
        output,
        chunk_base: args.chunk_base.clone(),
        options: DownloadOptions {
            max_attempts: args.attempts,
            concurrency: args
                .concurrency
                .unwrap_or_else(|| DownloadOptions::default().concurrency),
            bandwidth: args.limit.map(BandwidthLimit::new),
            ..Default::default()
        },
    };

    // Announce the batch once; progress fires per chunk.
    let announced = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let acquired = assetcraft_transfer::acquire(&client, &request, move |phase| {
        if let Phase::Download {
            files_total,
            bytes_total,
            ..
        } = phase
        {
            if !announced.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("{files_total} chunks ({bytes_total} bytes to download).");
            }
        }
    })
    .await?;

    println!(
        "Wrote {} files ({} bytes) to {}.",
        acquired.files,
        acquired.bytes,
        acquired.path.display()
    );
    println!(
        "Contents: {} — detail with: assetcraft structure {} --offline",
        acquired.structure.summary(),
        args.asset_id
    );

    // Record the library entry and where its files landed, so `export` can
    // find them again by id with the network off. Both are keyed by the hex
    // asset id — the id the library and the download record share, whatever
    // shape the caller typed. The path is stored absolute, so the record
    // survives a different working directory. Re-recording is what keeps a
    // re-download's default output pointing at the newest files.
    library
        .insert(&entry)
        .context("could not store the library entry")?;
    library
        .record_download(
            &AssetId::new(&owned.item_id),
            &acquired.path,
            &acquired.format,
        )
        .context("could not record the download in the library")?;
    println!(
        "Recorded it in the library — re-export offline with: assetcraft export {} --target <dir>",
        args.asset_id
    );
    Ok(())
}

/// Why `download` writes where it writes.
#[derive(Debug, PartialEq, Eq)]
enum OutputSource {
    /// `--output` named the directory.
    Flag,
    /// The library's recorded download location for the asset.
    Recorded,
    /// The `download_dir` the settings file configures.
    Settings,
    /// The standard per-asset directory under the data directory.
    Standard,
}

/// Where a download's files land: `--output` wins; otherwise the directory the
/// library recorded for the asset — even when its files are gone, so a
/// re-download restores them where `export` already looks; otherwise the
/// `download_dir` from the settings file, the same choice the app's settings
/// dialog writes; otherwise the standard asset directory, the same home the
/// GUI downloads into.
fn download_output(
    requested: Option<PathBuf>,
    recorded: Option<Download>,
    settings: &Settings,
    paths: &Paths,
    asset_id: &str,
) -> (PathBuf, OutputSource) {
    if let Some(output) = requested {
        return (output, OutputSource::Flag);
    }
    if let Some(record) = recorded.filter(|record| !record.path.as_os_str().is_empty()) {
        return (record.path, OutputSource::Recorded);
    }
    if settings.download_dir.is_some() {
        return (
            settings.asset_dir_for(paths, asset_id),
            OutputSource::Settings,
        );
    }
    (paths.asset_dir(asset_id), OutputSource::Standard)
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

    let files = assetcraft_dcc_blender::collect_files(&source)
        .with_context(|| format!("could not read {}", source.display()))?;
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
        // Name the Blender that was actually found: on a machine whose only
        // Blender is Flatpak's, `blender` answers to nobody, and the copied
        // line must be the command that runs — the exported
        // `org.blender.Blender` wrapper takes the same arguments.
        match assetcraft_dcc_blender::detect::find() {
            Some(blender) => println!(
                "Run it with: {} --background --python {}",
                blender.display(),
                report.script.display()
            ),
            None => println!(
                "Run it with: blender --background --python {}",
                report.script.display()
            ),
        }
        println!(
            "   …or install the Blender addon (blender_addon/) and point it \
             at this export's target folder."
        );
    }
    Ok(())
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

    #[test]
    fn download_accepts_a_bare_asset_id() {
        // What makes a re-download straight from the library possible: the
        // output and the chunk base are both optional now.
        let cli = Cli::try_parse_from(["assetcraft", "download", "an-asset-id"]).unwrap();
        let Command::Download(args) = cli.command else {
            panic!("expected the download command");
        };
        assert_eq!(args.asset_id, "an-asset-id");
        assert_eq!(args.output, None);
        assert_eq!(args.chunk_base, None);
    }

    #[test]
    fn download_still_accepts_its_overrides() {
        let cli = Cli::try_parse_from([
            "assetcraft",
            "download",
            "an-asset-id",
            "--output",
            "/tmp/out",
            "--chunk-base",
            "https://cdn.example.com/CloudDir",
        ])
        .unwrap();
        let Command::Download(args) = cli.command else {
            panic!("expected the download command");
        };
        assert_eq!(args.output, Some(PathBuf::from("/tmp/out")));
        assert_eq!(
            args.chunk_base.as_deref(),
            Some("https://cdn.example.com/CloudDir")
        );
    }

    #[test]
    fn download_output_prefers_the_flag_then_the_record_then_the_settings_then_the_default() {
        let paths = Paths::rooted_at("/tmp/assetcraft-cli-test");
        let recorded = Download {
            path: PathBuf::from("/data/where-it-landed"),
            format: "fbx".to_string(),
        };
        let settings = Settings {
            download_dir: Some(PathBuf::from("/media/assets")),
            ..Settings::default()
        };

        let (output, source) = download_output(
            Some(PathBuf::from("/tmp/hand")),
            Some(recorded.clone()),
            &settings,
            &paths,
            "a",
        );
        assert_eq!(output, PathBuf::from("/tmp/hand"));
        assert_eq!(source, OutputSource::Flag);

        // The record still beats a chosen download_dir: re-downloads restore
        // files where export already looks, wherever new ones go.
        let (output, source) =
            download_output(None, Some(recorded.clone()), &settings, &paths, "a");
        assert_eq!(output, PathBuf::from("/data/where-it-landed"));
        assert_eq!(source, OutputSource::Recorded);

        let (output, source) = download_output(None, None, &settings, &paths, "a");
        assert_eq!(output, PathBuf::from("/media/assets/a"));
        assert_eq!(source, OutputSource::Settings);

        // With no chosen directory, the standard place answers, as before.
        let (output, source) = download_output(None, None, &Settings::default(), &paths, "a");
        assert_eq!(output, paths.asset_dir("a"));
        assert_eq!(source, OutputSource::Standard);

        // A record with no usable path is treated as no record.
        let blank = Download {
            path: PathBuf::new(),
            format: String::new(),
        };
        let (output, source) =
            download_output(None, Some(blank), &Settings::default(), &paths, "a");
        assert_eq!(output, paths.asset_dir("a"));
        assert_eq!(source, OutputSource::Standard);
    }

    #[test]
    fn structure_defaults_to_asking_fab() {
        let cli = Cli::try_parse_from(["assetcraft", "structure", "an-asset-id"]).unwrap();
        let Command::Structure(args) = cli.command else {
            panic!("expected the structure command");
        };
        assert_eq!(args.asset_id, "an-asset-id");
        assert!(!args.offline);
        assert_eq!(args.source, None);
    }

    #[test]
    fn structure_can_read_the_files_on_disk_instead() {
        let cli = Cli::try_parse_from(["assetcraft", "structure", "a", "--offline"]).unwrap();
        let Command::Structure(args) = cli.command else {
            panic!("expected the structure command");
        };
        assert!(args.offline);

        let cli =
            Cli::try_parse_from(["assetcraft", "structure", "a", "--source", "./down"]).unwrap();
        let Command::Structure(args) = cli.command else {
            panic!("expected the structure command");
        };
        assert_eq!(args.source, Some(PathBuf::from("./down")));
        assert!(!args.offline);
    }

    #[test]
    fn a_downloaded_directory_describes_itself_with_the_network_off() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(root, "meshes/SM_Rock_LOD0.fbx");
        touch(root, "meshes/SM_Rock_LOD1.fbx");
        touch(root, "meshes/SM_Rock_LOD2.fbx");
        touch(root, "textures/T_Rock_BaseColor.png");
        touch(root, "textures/T_Rock_Roughness_LOD2.png");
        touch(root, "README.md");

        let read = structure_on_disk(root).unwrap();
        let groups = read.mesh_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "SM_Rock");
        assert_eq!(groups[0].levels, vec![0, 1, 2]);
        assert_eq!(read.material_names(), vec!["T_Rock".to_string()]);
        // The level token on the second map did not hide its role.
        assert_eq!(read.textures.len(), 2);
        assert_eq!(read.other, vec![PathBuf::from("README.md")]);
        // Sizes come off the disk: three meshes and two maps of five bytes each,
        // and the README is not counted because it is neither.
        assert_eq!(read.known_size_bytes(), 25);
    }

    #[test]
    fn a_directory_with_no_files_describes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(structure_on_disk(dir.path()).is_err());
    }

    /// A file at `relative`, with the directories above it.
    fn touch(root: &Path, relative: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"bytes").unwrap();
    }
}
