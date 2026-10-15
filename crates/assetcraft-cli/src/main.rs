//! The `assetcraft` command-line interface.
//!
//! This is the headless entry point to the same engine the desktop app will
//! use. For M1 it covers the read side of the catalog plus sign-in.

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};

use assetcraft_auth::{EpicAuth, FileTokenStore, TokenStore};
use assetcraft_catalog::FabClient;
use assetcraft_core::{AssetSummary, Config, Page, SearchQuery, SortOrder};

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
        Command::Taxonomy(args) => taxonomy(&config, args).await,
    }
}

async fn search(config: &Config, args: SearchArgs) -> Result<()> {
    let client = FabClient::from_config(config)?;
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
