//! The catalog worker.
//!
//! The `assetcraft-catalog` client is async, the UI is not, and nothing the
//! network does may block a frame. So a background thread owns a small tokio
//! runtime and the `FabClient`; the UI sends [`Command`]s and drains
//! [`Event`]s, and the worker pokes the egui context whenever something lands.

use std::sync::Arc;
use std::time::Duration;

use assetcraft_auth::{ensure_fresh_token, DefaultTokenStore, EpicAuth, EpicToken, TokenStore};
use assetcraft_catalog::FabClient;
use assetcraft_core::{
    AssetDetail, AssetId, AssetSummary, Config, LibraryItem, Page, SearchQuery, Settings,
};
use assetcraft_library::{Library, MediaCache};
use tokio::sync::{mpsc, Semaphore};

/// How many thumbnail downloads may be in flight at once.
const THUMBNAIL_CONCURRENCY: usize = 6;
/// Thumbnails are capped so a hostile or broken URL cannot exhaust memory.
const MAX_THUMBNAIL_BYTES: usize = 8 * 1024 * 1024;
/// Textures are downscaled before upload; cards never show more than this.
const THUMBNAIL_EDGE: u32 = 512;

/// A signed-in Epic account, as the UI needs it.
#[derive(Clone, Debug)]
pub struct Account {
    /// Display name, or the account id when Epic did not send one.
    pub name: String,
}

/// A finished export, as the UI reports it.
#[derive(Clone, Debug)]
pub struct ExportSummary {
    /// The directory the layout was written into.
    pub dir: std::path::PathBuf,
    /// How many files were copied.
    pub copied: usize,
    /// How many materials the generated script builds.
    pub materials: usize,
    /// Whether Blender was found on this machine, which decides the wording of
    /// the next step the UI suggests.
    pub blender_found: bool,
}

impl Account {
    fn from_token(token: &EpicToken) -> Self {
        let name = token
            .display_name
            .clone()
            .or_else(|| token.account_id.clone())
            .unwrap_or_else(|| "Epic account".to_owned());
        Self { name }
    }
}

/// Work the UI hands to the worker.
pub enum Command {
    /// Run a search. `append` marks a "load more" page.
    Search {
        generation: u64,
        query: SearchQuery,
        append: bool,
    },
    /// Fetch one listing's full detail.
    Detail(AssetId),
    /// Fetch one page of the signed-in account's owned library.
    Owned {
        generation: u64,
        cursor: Option<String>,
    },
    /// Download and decode a thumbnail into a texture.
    Thumbnail { id: String, url: String },
    /// Exchange a browser authorization code for a token and store it.
    SignIn { code: String },
    /// Forget the stored token.
    SignOut,
    /// Download an owned asset's Unreal build and record it in the library.
    Download {
        /// The card asked for; its id joins to the account's library entry,
        /// and the *entry* — not this stand-in — is what gets recorded.
        item: LibraryItem,
        /// Resolution token from the detail panel, e.g. `4K`. Unused: an
        /// artifact is the one build Fab delivers, with no files to pick
        /// between; the picker stays for the formats Fab serves at home.
        quality: Option<String>,
    },
    /// Lay a downloaded asset out for Blender under the export root.
    ExportBlender {
        /// The asset to export; its files come from the library record.
        id: String,
    },
}

/// Results the worker hands back.
pub enum Event {
    Search {
        generation: u64,
        append: bool,
        /// Boxed: a page is much larger than the other events.
        result: Box<Result<Page<AssetSummary>, String>>,
    },
    Detail {
        id: String,
        /// Boxed: a detail is much larger than the other events.
        result: Box<Result<AssetDetail, String>>,
    },
    /// One page of the owned library, or why it could not be read.
    Owned {
        generation: u64,
        result: Box<Result<Page<LibraryItem>, String>>,
    },
    Thumbnail {
        id: String,
        texture: egui::TextureHandle,
    },
    /// What the token store held when the worker started.
    Session(Option<Account>),
    /// The result of exchanging a sign-in code.
    SignedIn(Result<Account, String>),
    /// The stored token was cleared.
    SignedOut,
    /// One progress report from a running download.
    DownloadPhase {
        id: String,
        phase: assetcraft_transfer::Phase,
    },
    /// A download finished and was recorded in the library.
    DownloadDone {
        id: String,
        files: usize,
        bytes: u64,
    },
    /// A download failed; `error` is one readable line.
    DownloadFailed { id: String, error: String },
    /// An export finished, or the one readable line about why it did not.
    Exported {
        id: String,
        result: Box<Result<ExportSummary, String>>,
    },
    /// The worker could not be set up at all; the UI should fall back to demo
    /// data.
    Fatal(String),
}

/// A handle to the running worker.
pub struct CatalogWorker {
    commands: mpsc::UnboundedSender<Command>,
    events: mpsc::UnboundedReceiver<Event>,
}

impl CatalogWorker {
    /// Start the worker for `config`, reporting back through `ctx`.
    ///
    /// Returns `None` if the background thread could not be spawned; the caller
    /// then has no network catalog at all.
    pub fn spawn(ctx: egui::Context, config: Config) -> Option<Self> {
        let (commands, mut command_rx) = mpsc::unbounded_channel();
        let (event_tx, events) = mpsc::unbounded_channel();

        std::thread::Builder::new()
            .name("assetcraft-catalog".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = event_tx.send(Event::Fatal(format!(
                            "could not start the catalog runtime: {error}"
                        )));
                        return;
                    }
                };
                runtime.block_on(run(ctx, config, event_tx, &mut command_rx));
            })
            .ok()?;

        Some(Self { commands, events })
    }

    /// Queue work. Work queued after the UI dropped the worker is discarded.
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Take one finished event, if any.
    pub fn try_recv(&mut self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

async fn run(
    ctx: egui::Context,
    config: Config,
    events: mpsc::UnboundedSender<Event>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) {
    let client = match FabClient::from_config(&config) {
        Ok(client) => client,
        Err(error) => {
            let _ = events.send(Event::Fatal(error.to_string()));
            ctx.request_repaint();
            return;
        }
    };

    // Thumbnails live on a CDN behind no auth, and are fetched with a plain
    // client so page-sized images never travel through the API client.
    let http = match reqwest::Client::builder()
        .user_agent(config.user_agent.clone())
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(http) => http,
        Err(error) => {
            let _ = events.send(Event::Fatal(error.to_string()));
            ctx.request_repaint();
            return;
        }
    };

    let auth = match EpicAuth::from_config(&config) {
        Ok(auth) => auth,
        Err(error) => {
            let _ = events.send(Event::Fatal(error.to_string()));
            ctx.request_repaint();
            return;
        }
    };
    let store = DefaultTokenStore::from_config(&config);
    // Thumbnails the download flow already cached are reused, so local library
    // cards render without a network.
    let media = MediaCache::new(config.paths.thumbnail_cache_dir());

    // Tell the UI who is signed in, refreshing a token that is close to expiry.
    {
        let auth = auth.clone();
        let store = store.clone();
        let events = events.clone();
        let ctx = ctx.clone();
        let session_client = client.clone();
        tokio::spawn(async move {
            let session = restore_session(&auth, &store).await;
            if let Some((_, token)) = &session {
                session_client.set_token(Some(token.authorization_header()));
            }
            let account = session.map(|(account, _)| account);
            let _ = events.send(Event::Session(account));
            ctx.request_repaint();
        });
    }

    let permits = Arc::new(Semaphore::new(THUMBNAIL_CONCURRENCY));

    while let Some(command) = commands.recv().await {
        match command {
            Command::Search {
                generation,
                query,
                append,
            } => {
                let client = client.clone();
                let events = events.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let result = client
                        .search(&query)
                        .await
                        .map_err(|error| friendly(&error.to_string()));
                    let _ = events.send(Event::Search {
                        generation,
                        append,
                        result: Box::new(result),
                    });
                    ctx.request_repaint();
                });
            }
            Command::Detail(id) => {
                let client = client.clone();
                let events = events.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let result = client.listing(&id).await.map_err(|error| error.to_string());
                    let _ = events.send(Event::Detail {
                        id: id.into_inner(),
                        result: Box::new(result),
                    });
                    ctx.request_repaint();
                });
            }
            Command::Owned { generation, cursor } => {
                let client = client.clone();
                let store = store.clone();
                let events = events.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    // The library endpoint is keyed by account id, which only
                    // the stored session carries.
                    let account_id = store
                        .load()
                        .ok()
                        .flatten()
                        .and_then(|token| token.account_id);
                    let result = match account_id {
                        Some(account_id) => client
                            .library(&account_id, cursor.as_deref())
                            .await
                            .map_err(|error| friendly(&error.to_string())),
                        None => {
                            Err("Sign in again: the stored session has no account id.".to_owned())
                        }
                    };
                    let _ = events.send(Event::Owned {
                        generation,
                        result: Box::new(result),
                    });
                    ctx.request_repaint();
                });
            }
            Command::SignIn { code } => {
                let auth = auth.clone();
                let store = store.clone();
                let events = events.clone();
                let ctx = ctx.clone();
                let client = client.clone();
                tokio::spawn(async move {
                    let result = match auth.exchange_code(code.trim()).await {
                        Ok(token) => {
                            client.set_token(Some(token.authorization_header()));
                            match store.save(&token) {
                                Ok(()) => Ok(Account::from_token(&token)),
                                Err(error) => Err(friendly(&error.to_string())),
                            }
                        }
                        Err(error) => Err(friendly(&error.to_string())),
                    };
                    let _ = events.send(Event::SignedIn(result));
                    ctx.request_repaint();
                });
            }
            Command::SignOut => {
                let store = store.clone();
                let events = events.clone();
                let ctx = ctx.clone();
                let client = client.clone();
                tokio::spawn(async move {
                    client.set_token(None);
                    let _ = store.clear();
                    let _ = events.send(Event::SignedOut);
                    ctx.request_repaint();
                });
            }
            Command::Download { item, quality } => {
                let client = client.clone();
                let store = store.clone();
                let events = events.clone();
                let ctx = ctx.clone();
                let config = config.clone();
                let id = item.asset_id.clone();
                // An artifact is the one build Fab chose to deliver; the
                // resolution picker has no files left to choose between.
                let _ = quality;
                tokio::spawn(async move {
                    // Fab delivers the Unreal build of an *owned* asset and
                    // nothing else, and only the account's library — keyed
                    // by the account id only the stored session carries —
                    // knows that build's artifact. The card carries a
                    // listing UUID and the library entry a hex uid;
                    // `owned_for` joins the two and says plainly when the
                    // asset is not owned at all.
                    let account_id = store
                        .load()
                        .ok()
                        .flatten()
                        .and_then(|token| token.account_id);
                    let found = match &account_id {
                        Some(account_id) => {
                            Some(assetcraft_transfer::owned_for(&client, account_id, &id).await)
                        }
                        None => None,
                    };
                    let (entry, owned) = match found {
                        Some(Ok(found)) => found,
                        Some(Err(error)) => {
                            let _ = events.send(Event::DownloadFailed {
                                id,
                                error: friendly(&format!("{error:#}")),
                            });
                            ctx.request_repaint();
                            return;
                        }
                        None => {
                            let _ = events.send(Event::DownloadFailed {
                                id,
                                error: "Sign in again: the stored session has no account id."
                                    .to_owned(),
                            });
                            ctx.request_repaint();
                            return;
                        }
                    };
                    // Where the files land is the user's choice, read at the
                    // point of use; a settings file too broken to read falls
                    // back to the standard directory, and the app has already
                    // said so in its banner.
                    let settings = Settings::load(&config.paths).unwrap_or_default();
                    let request = assetcraft_transfer::Request {
                        owned: owned.clone(),
                        output: settings.asset_dir_for(&config.paths, &owned.item_id),
                        chunk_base: std::env::var("ASSETCRAFT_CHUNK_BASE").ok(),
                        options: Default::default(),
                    };

                    let progress_events = events.clone();
                    let progress_id = id.clone();
                    let progress_ctx = ctx.clone();
                    let result = assetcraft_transfer::acquire(&client, &request, move |phase| {
                        let _ = progress_events.send(Event::DownloadPhase {
                            id: progress_id.clone(),
                            phase,
                        });
                        progress_ctx.request_repaint();
                    })
                    .await;

                    match result {
                        Ok(acquired) => {
                            // The real library entry is recorded — not the
                            // card-shaped stand-in — and the download is
                            // keyed by the hex asset id, which is what
                            // `assetcraft download <id>` and the exporters
                            // look a recorded path up by.
                            let recorded = Library::open(config.paths.library_file())
                                .map_err(|error| error.to_string())
                                .and_then(|library| {
                                    library.insert(&entry).map_err(|error| error.to_string())?;
                                    library
                                        .record_download(
                                            &AssetId::new(&owned.item_id),
                                            &acquired.path,
                                            &acquired.format,
                                        )
                                        .map_err(|error| error.to_string())
                                });
                            let event = match recorded {
                                Ok(()) => Event::DownloadDone {
                                    id,
                                    files: acquired.files,
                                    bytes: acquired.bytes,
                                },
                                Err(error) => Event::DownloadFailed {
                                    id,
                                    error: format!(
                                        "the files arrived but could not be recorded: {error}"
                                    ),
                                },
                            };
                            let _ = events.send(event);
                        }
                        Err(error) => {
                            let _ = events.send(Event::DownloadFailed {
                                id,
                                error: friendly(&format!("{error:#}")),
                            });
                        }
                    }
                    ctx.request_repaint();
                });
            }
            Command::Thumbnail { id, url } => {
                let http = http.clone();
                let permits = Arc::clone(&permits);
                let events = events.clone();
                let ctx = ctx.clone();
                let media = media.clone();
                tokio::spawn(async move {
                    let Ok(_permit) = permits.acquire_owned().await else {
                        return;
                    };
                    let Some(image) = download_thumbnail(&http, &media, &url).await else {
                        return;
                    };
                    let texture = ctx.load_texture(
                        format!("thumb/{id}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    let _ = events.send(Event::Thumbnail { id, texture });
                    ctx.request_repaint();
                });
            }
            Command::ExportBlender { id } => {
                let events = events.clone();
                let ctx = ctx.clone();
                let config = config.clone();
                tokio::spawn(async move {
                    // Copying a download's files is blocking disk work; keep it
                    // off the runtime's async workers.
                    let export_id = id.clone();
                    let result =
                        tokio::task::spawn_blocking(move || run_export(&config, &export_id))
                            .await
                            .unwrap_or_else(|_| {
                                Err("the export thread stopped before it finished.".to_owned())
                            });
                    let _ = events.send(Event::Exported {
                        id,
                        result: Box::new(result),
                    });
                    ctx.request_repaint();
                });
            }
        }
    }
}

/// The account the token store already holds, refreshed when it is stale.
///
/// A refresh that fails (offline, expired refresh token) still reports the
/// stored account: the UI should show who is signed in and let the next
/// authenticated call surface the problem.
async fn restore_session(auth: &EpicAuth, store: &impl TokenStore) -> Option<(Account, EpicToken)> {
    let stored = store.load().ok().flatten()?;
    match ensure_fresh_token(auth, store).await {
        Ok(token) => Some((Account::from_token(&token), token)),
        Err(_) => Some((Account::from_token(&stored), stored)),
    }
}

/// Lay one downloaded asset out for Blender under the export root, offline.
///
/// This is the same flow as the CLI's `export`: the files come from the path the
/// library recorded, the layout lands under the standard export root where the
/// Blender addon browses, and nothing here touches the network. Blender itself is
/// never launched from the app: a background run could not touch the user's open
/// scene, and an interactive one would block this thread until Blender quits —
/// the addon is the in-editor path, and it only needs the written export.
fn run_export(config: &Config, id: &str) -> Result<ExportSummary, String> {
    let asset = AssetId::new(id);
    let download = Library::open(config.paths.library_file())
        .map_err(|error| error.to_string())?
        .download(&asset)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "this asset has no downloaded files in the library".to_owned())?;
    if !download.path.is_dir() {
        return Err(format!(
            "the downloaded files are gone from {}; download the asset again",
            download.path.display()
        ));
    }

    let files = assetcraft_dcc_blender::collect_files(&download.path)
        .map_err(|error| format!("could not read {}: {error}", download.path.display()))?;
    if files.is_empty() {
        return Err(format!(
            "{} holds no files to export",
            download.path.display()
        ));
    }
    let format = if download.format.is_empty() {
        "asset".to_owned()
    } else {
        download.format
    };

    let plan = assetcraft_dcc_blender::plan(config.paths.exports_dir(), &asset, &format, &files);
    let report = assetcraft_dcc_blender::export(
        &plan,
        &download.path,
        &assetcraft_dcc_blender::ExportOptions {
            // Same default as the CLI: bring the meshes in and assign the
            // material, not just the material alone.
            import_meshes: true,
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())?;

    Ok(ExportSummary {
        dir: report.dir,
        copied: report.copied,
        materials: report.materials,
        blender_found: assetcraft_dcc_blender::detect::find().is_some(),
    })
}

/// Turn a client error into one readable line for the banner.
///
/// Fab answers non-browser clients with a Cloudflare challenge page, and the
/// HTTP-status error carries that whole page as its message; the UI must not
/// render (or store) 25 KB of markup.
fn friendly(message: &str) -> String {
    let first = message.lines().next().unwrap_or_default().trim();
    if message.contains("<html") || message.contains("<!DOCTYPE") {
        let status = first.split(':').next().unwrap_or(first).trim();
        return format!(
            "{status} from fab.com. The marketplace blocks non-browser clients, so the live \
             catalog cannot be used here. Switch to demo data to explore the shell."
        );
    }
    if first.chars().count() > 160 {
        let mut short: String = first.chars().take(157).collect();
        short.push_str("...");
        return short;
    }
    if first.is_empty() {
        "The catalog request failed.".to_owned()
    } else {
        first.to_owned()
    }
}

/// Fetch, downscale, and decode a thumbnail into an egui image.
///
/// The media cache is consulted first and filled on the way back, so images
/// the download flow stored are reused and restarts are instant.
async fn download_thumbnail(
    http: &reqwest::Client,
    media: &MediaCache,
    url: &str,
) -> Option<egui::ColorImage> {
    if let Ok(Some(bytes)) = media.get(url) {
        if let Some(image) = tokio::task::spawn_blocking(move || decode(&bytes))
            .await
            .ok()
            .flatten()
        {
            return Some(image);
        }
    }

    let response = http.get(url).send().await.ok()?.error_for_status().ok()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_THUMBNAIL_BYTES as u64)
    {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    if bytes.len() > MAX_THUMBNAIL_BYTES {
        return None;
    }
    let _ = media.put(url, &bytes);
    // Decoding is CPU work; keep it off the runtime's async workers.
    tokio::task::spawn_blocking(move || decode(&bytes))
        .await
        .ok()
        .flatten()
}

fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let image = image::load_from_memory(bytes)
        .ok()?
        .thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE);
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{friendly, run_export, Account};
    use assetcraft_auth::EpicToken;
    use assetcraft_core::{AssetId, Config, LibraryItem};
    use assetcraft_library::Library;

    fn token(display: Option<&str>, account: Option<&str>) -> EpicToken {
        EpicToken {
            access_token: "access".to_owned(),
            refresh_token: None,
            token_type: "bearer".to_owned(),
            expires_at: None,
            account_id: account.map(str::to_owned),
            display_name: display.map(str::to_owned),
        }
    }

    #[test]
    fn accounts_prefer_the_display_name() {
        assert_eq!(
            Account::from_token(&token(Some("Player One"), Some("acc-1"))).name,
            "Player One"
        );
        assert_eq!(
            Account::from_token(&token(None, Some("acc-1"))).name,
            "acc-1"
        );
        assert_eq!(Account::from_token(&token(None, None)).name, "Epic account");
    }

    #[test]
    fn challenge_pages_are_reduced_to_one_line() {
        let message =
            "server returned HTTP 403: <html>\n<head><style>body{}</style></head>\n</html>";
        let friendly = friendly(message);
        assert!(friendly.starts_with("server returned HTTP 403 from fab.com."));
        assert!(!friendly.contains("<html"));
        assert!(friendly.len() < 250);
    }

    #[test]
    fn long_messages_are_cut_and_empty_ones_replaced() {
        let long = "x".repeat(400);
        assert_eq!(friendly(&long).chars().count(), 160);
        assert_eq!(friendly(""), "The catalog request failed.");
        assert_eq!(friendly("connection reset"), "connection reset");
    }

    /// A library in a temporary home, with one asset downloaded to disk.
    fn library_with_download(dir: &std::path::Path) -> Config {
        let config = Config::rooted_at(dir);
        let source = config.paths.asset_dir("rock-1");
        for relative in ["textures/T_Rock_BaseColor.png", "meshes/SM_Rock.fbx"] {
            let path = source.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"stub").unwrap();
        }
        let library = Library::open(config.paths.library_file()).unwrap();
        library
            .insert(&LibraryItem {
                asset_id: "rock-1".to_owned(),
                title: "Rock".to_owned(),
                ..Default::default()
            })
            .unwrap();
        library
            .record_download(&AssetId::new("rock-1"), &source, "gltf")
            .unwrap();
        config
    }

    #[test]
    fn exporting_a_downloaded_asset_lays_it_out_under_the_export_root() {
        let dir = tempfile::tempdir().unwrap();
        let config = library_with_download(dir.path());

        let summary = run_export(&config, "rock-1").unwrap();

        assert_eq!(summary.copied, 2);
        assert_eq!(
            summary.dir,
            config.paths.exports_dir().join("rock-1").join("gltf")
        );
        // The addon discovers an export by its script and manifest sidecar.
        assert!(summary
            .dir
            .join(assetcraft_dcc_blender::SCRIPT_NAME)
            .exists());
        assert!(summary
            .dir
            .join(assetcraft_dcc_blender::MANIFEST_NAME)
            .exists());
    }

    #[test]
    fn exporting_what_was_never_downloaded_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::rooted_at(dir.path());
        let error = run_export(&config, "absent").unwrap_err();
        assert!(error.contains("no downloaded files"), "{error}");
    }

    #[test]
    fn exporting_files_the_user_moved_away_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let config = library_with_download(dir.path());
        std::fs::remove_dir_all(config.paths.asset_dir("rock-1")).unwrap();
        let error = run_export(&config, "rock-1").unwrap_err();
        assert!(error.contains("gone from"), "{error}");
    }

    /// The Blender addon works out where the app writes on its own, in Python
    /// (`blender_addon/assetcraft_blender/exports.py::default_library_root`),
    /// because it has no way to ask a running app. The two implementations can
    /// only agree on the shape of the path, so the shape is pinned here and in
    /// that module's tests: the same platform bases the addon uses, the same
    /// trailing segments this side resolves to.
    #[test]
    fn the_app_and_the_addon_agree_on_the_default_export_root() {
        const SUFFIX: &str = if cfg!(target_os = "windows") {
            // %APPDATA%\AssetCraft\AssetCraft\data\exports (organisation and
            // application both "AssetCraft", plus the `data` segment).
            "AssetCraft/AssetCraft/data/exports"
        } else if cfg!(target_os = "macos") {
            // ~/Library/Application Support/dev.AssetCraft.AssetCraft/exports
            "dev.AssetCraft.AssetCraft/exports"
        } else {
            // $XDG_DATA_HOME/assetcraft/exports - lower-cased project path.
            "assetcraft/exports"
        };
        let config = Config::load().expect("platform directories should resolve");
        let exports = config
            .paths
            .exports_dir()
            .display()
            .to_string()
            .replace('\\', "/");
        assert!(
            exports.ends_with(SUFFIX),
            "{exports} does not end with {SUFFIX}"
        );
    }
}
