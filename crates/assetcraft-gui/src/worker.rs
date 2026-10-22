//! The catalog worker.
//!
//! The `assetcraft-catalog` client is async, the UI is not, and nothing the
//! network does may block a frame. So a background thread owns a small tokio
//! runtime and the `FabClient`; the UI sends [`Command`]s and drains
//! [`Event`]s, and the worker pokes the egui context whenever something lands.

use std::sync::Arc;
use std::time::Duration;

use assetcraft_auth::{ensure_fresh_token, EpicAuth, EpicToken, FileTokenStore, TokenStore};
use assetcraft_catalog::FabClient;
use assetcraft_core::{AssetDetail, AssetId, AssetSummary, Config, Page, SearchQuery};
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
    /// Download and decode a thumbnail into a texture.
    Thumbnail { id: String, url: String },
    /// Exchange a browser authorization code for a token and store it.
    SignIn { code: String },
    /// Forget the stored token.
    SignOut,
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
    let store = FileTokenStore::new(config.paths.token_file());

    // Tell the UI who is signed in, refreshing a token that is close to expiry.
    {
        let auth = auth.clone();
        let store = store.clone();
        let events = events.clone();
        let ctx = ctx.clone();
        tokio::spawn(async move {
            let account = restore_session(&auth, &store).await;
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
            Command::SignIn { code } => {
                let auth = auth.clone();
                let store = store.clone();
                let events = events.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let result = match auth.exchange_code(code.trim()).await {
                        Ok(token) => match store.save(&token) {
                            Ok(()) => Ok(Account::from_token(&token)),
                            Err(error) => Err(friendly(&error.to_string())),
                        },
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
                tokio::spawn(async move {
                    let _ = store.clear();
                    let _ = events.send(Event::SignedOut);
                    ctx.request_repaint();
                });
            }
            Command::Thumbnail { id, url } => {
                let http = http.clone();
                let permits = Arc::clone(&permits);
                let events = events.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let Ok(_permit) = permits.acquire_owned().await else {
                        return;
                    };
                    let Some(image) = download_thumbnail(&http, &url).await else {
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
        }
    }
}

/// The account the token store already holds, refreshed when it is stale.
///
/// A refresh that fails (offline, expired refresh token) still reports the
/// stored account: the UI should show who is signed in and let the next
/// authenticated call surface the problem.
async fn restore_session(auth: &EpicAuth, store: &FileTokenStore) -> Option<Account> {
    let stored = store.load().ok().flatten()?;
    match ensure_fresh_token(auth, store).await {
        Ok(token) => Some(Account::from_token(&token)),
        Err(_) => Some(Account::from_token(&stored)),
    }
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
async fn download_thumbnail(http: &reqwest::Client, url: &str) -> Option<egui::ColorImage> {
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
    use super::{friendly, Account};
    use assetcraft_auth::EpicToken;

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
}
