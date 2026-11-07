//! The application shell: left nav, search + filter bar, grid, asset detail
//! panel, and the downloads drawer — the three-pane anatomy documented in
//! `docs/reference/quixel-bridge/README.md`.
//!
//! Data comes from the live Fab catalog through [`CatalogWorker`]; when the
//! marketplace is unreachable (or the user picks it) the demo dataset stands in
//! so the shell stays reviewable offline.

use std::collections::{HashMap, HashSet};

use assetcraft_core::{AssetId, Config, Settings};
use assetcraft_library::{Collection, Library};
use assetcraft_transfer::Phase;

use crate::data::{self, Card, Filters, View};
use crate::worker::{Account, CatalogWorker, Command, Event};
use crate::{grid, icons, theme};

/// Resolution choices for the download flow; the first entry means "no
/// preference" and lets the engine take the first file the asset offers.
/// The settings dialog's default-quality uses the same list, so a stored
/// quality and a picker position are the same thing.
const QUALITY_CHOICES: [&str; 5] = ["No preference", "2K", "4K", "8K", "16K"];
/// How long the search box waits after the last keystroke before searching.
const QUERY_DEBOUNCE: f64 = 0.3;
/// Height reserved for the detail panel's action bar.
const ACTION_BAR: f32 = 46.0;

/// Where the catalog comes from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Live,
    Demo,
}

/// Bridge's first-window rule: the marketplace views wait for a session.
/// Signed out, the grid holds nothing and a panel says what fills it — the
/// way Bridge opens on an empty window rather than a shelf of store data.
/// The local views never wait: they show the user's own disk and library,
/// not the store.
fn store_locked(source: Source, account: &Option<Account>, view: View) -> bool {
    source == Source::Live
        && account.is_none()
        && matches!(view, View::Home | View::Free | View::Purchased)
}

/// A download in the drawer.
/// Where a download has got to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobStatus {
    /// Choosing a format and fetching the manifest.
    Preparing,
    /// Fetching chunks.
    Downloading,
    /// Splicing chunks into files.
    Assembling,
    /// Writing the files out.
    Writing,
    /// Finished, and recorded in the library.
    Done,
    /// Stopped; the message says why.
    Failed(String),
}

pub struct Job {
    /// Asset id, which ties progress events to this row.
    pub id: String,
    pub name: String,
    pub progress: f32,
    pub status: JobStatus,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// How many files landed, once the download finished.
    pub files: usize,
    /// Demo downloads are simulated; live ones come from the worker.
    pub demo: bool,
}

impl Job {
    fn running(&self) -> bool {
        !matches!(self.status, JobStatus::Done | JobStatus::Failed(_))
    }

    /// The right-hand label: the stage, or the byte count while downloading.
    fn label(&self) -> String {
        match &self.status {
            JobStatus::Preparing => "Resolving".to_owned(),
            JobStatus::Downloading if self.bytes_total > 0 => format!(
                "{} / {}",
                bytes_label(self.bytes_done),
                bytes_label(self.bytes_total)
            ),
            JobStatus::Downloading => "Downloading".to_owned(),
            JobStatus::Assembling => "Assembling".to_owned(),
            JobStatus::Writing => "Writing files".to_owned(),
            JobStatus::Done if self.files > 0 => format!("Done: {} files", self.files),
            JobStatus::Done => "Done".to_owned(),
            JobStatus::Failed(_) => "Failed".to_owned(),
        }
    }
}

/// A byte count a person can scan, e.g. `4.2 MB`.
fn bytes_label(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if value >= KB * KB * KB {
        format!("{:.1} GB", value / (KB * KB * KB))
    } else if value >= KB * KB {
        format!("{:.1} MB", value / (KB * KB))
    } else if value >= KB {
        format!("{:.0} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

pub struct App {
    worker: Option<CatalogWorker>,
    source: Source,
    /// Bumped per search so stale responses can be ignored.
    generation: u64,
    loading: bool,
    error: Option<String>,
    pub cards: Vec<Card>,
    total: Option<u64>,
    next_cursor: Option<String>,
    textures: HashMap<String, egui::TextureHandle>,
    requested: HashSet<String>,
    details: HashMap<String, assetcraft_core::AssetDetail>,
    pub favorites: HashSet<String>,
    pub local: HashSet<String>,
    pub view: View,
    pub filters: Filters,
    pub query: String,
    sent_query: String,
    dirty_at: f64,
    pub selected: Option<usize>,
    pub checked: HashSet<String>,
    resolution: usize,
    pub jobs: Vec<Job>,
    last_time: Option<f64>,
    account: Option<Account>,
    sign_in_open: bool,
    sign_in_code: String,
    sign_in_busy: bool,
    sign_in_error: Option<String>,
    sign_in_url: String,
    library: Option<Library>,
    local_cards: Vec<Card>,
    library_error: Option<String>,
    /// Asset currently being exported, so its button reads as busy.
    exporting: Option<String>,
    /// Where each asset was last exported to, for the detail panel's row.
    exported: HashMap<String, std::path::PathBuf>,
    /// The last export's one-line result, shown until dismissed.
    export_note: Option<(bool, String)>,
    owned_cards: Vec<Card>,
    owned_cursor: Option<String>,
    owned_loaded: bool,
    /// The user's choices, as read at startup (the worker re-reads them per
    /// download, so editing the file by hand still takes effect).
    settings: Settings,
    /// Kept to know where the settings file lives when the dialog saves it.
    config: Option<Config>,
    settings_error: Option<String>,
    settings_open: bool,
    settings_dir: String,
    settings_quality: usize,
    collections: Vec<Collection>,
    /// The open collection in the Collections view; `None` is the list.
    active_collection: Option<i64>,
    collection_new: String,
    collection_renaming: Option<(i64, String)>,
    collection_delete_armed: Option<i64>,
    collections_error: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            worker: None,
            source: Source::Live,
            generation: 0,
            loading: false,
            error: None,
            cards: Vec::new(),
            total: None,
            next_cursor: None,
            textures: HashMap::new(),
            requested: HashSet::new(),
            details: HashMap::new(),
            favorites: HashSet::new(),
            local: HashSet::new(),
            view: View::Home,
            filters: Filters::default(),
            query: String::new(),
            sent_query: String::new(),
            dirty_at: 0.0,
            selected: None,
            checked: HashSet::new(),
            resolution: 0,
            jobs: Vec::new(),
            last_time: None,
            account: None,
            sign_in_open: false,
            sign_in_code: String::new(),
            sign_in_busy: false,
            sign_in_error: None,
            sign_in_url: String::new(),
            library: None,
            local_cards: Vec::new(),
            library_error: None,
            exporting: None,
            exported: HashMap::new(),
            export_note: None,
            owned_cards: Vec::new(),
            owned_cursor: None,
            owned_loaded: false,
            settings: Settings::default(),
            config: None,
            settings_error: None,
            settings_open: false,
            settings_dir: String::new(),
            settings_quality: 0,
            collections: Vec::new(),
            active_collection: None,
            collection_new: String::new(),
            collection_renaming: None,
            collection_delete_armed: None,
            collections_error: None,
        }
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let mut app = Self::default();

        match Config::load() {
            Ok(mut config) => {
                // Lets the app and the screenshots run against a local stand-in
                // for Fab without changing the build.
                if let Ok(base) = std::env::var("ASSETCRAFT_FAB_BASE_URL") {
                    config.fab_base_url = base;
                }
                if let Ok(base) = std::env::var("ASSETCRAFT_ACCOUNT_BASE_URL") {
                    config.account_base_url = base;
                }
                if let Ok(base) = std::env::var("ASSETCRAFT_EPIC_ID_BASE_URL") {
                    config.epic_id_base_url = base;
                }
                app.sign_in_url = assetcraft_auth::EpicAuth::from_config(&config)
                    .map(|auth| auth.authorization_url())
                    .unwrap_or_default();
                match Library::open(config.paths.library_file()) {
                    Ok(library) => {
                        app.library = Some(library);
                        app.reload_local();
                    }
                    Err(error) => app.library_error = Some(error.to_string()),
                }

                // The user's own choices: where downloads land, which quality
                // the picker starts on. A broken file is said so, not worked
                // around — defaults would quietly undo what was chosen.
                match Settings::load(&config.paths) {
                    Ok(settings) => {
                        if let Some(quality) = settings.default_quality.as_deref() {
                            app.resolution = QUALITY_CHOICES
                                .iter()
                                .position(|r| r.eq_ignore_ascii_case(quality))
                                .unwrap_or(app.resolution);
                        }
                        app.settings = settings;
                    }
                    Err(error) => app.settings_error = Some(error.to_string()),
                }
                app.config = Some(config.clone());

                app.worker = CatalogWorker::spawn(cc.egui_ctx.clone(), config);
                if app.worker.is_none() {
                    app.source = Source::Demo;
                    app.error = Some("The catalog worker could not start.".to_owned());
                }
            }
            Err(error) => {
                app.source = Source::Demo;
                app.error = Some(format!("Could not load the configuration: {error}"));
            }
        }

        // `--demo` opens the offline dataset, for exploring the shell without a
        // marketplace account or network.
        if std::env::args().any(|arg| arg == "--demo") {
            app.source = Source::Demo;
        }

        // Bridge parity: the store opens empty and fills only once a session
        // has landed (or the demo was asked for) — the first window shows no
        // listings, and the sign-in panel says what unlocks them.
        if app.source == Source::Demo {
            app.start_search(&cc.egui_ctx);
        }
        app
    }

    // ----- data flow -------------------------------------------------------

    /// Read the local library into cards and remember which assets are held.
    ///
    /// Favourites and collections are read in the same breath: they live in
    /// the same database, describe the same assets, and a view that showed
    /// half-current organisation over fresh cards would lie about neither.
    fn reload_local(&mut self) {
        let Some(library) = &self.library else {
            self.local_cards.clear();
            return;
        };
        match local_cards(library) {
            Ok(cards) => {
                self.local = cards.iter().map(|card| card.id.clone()).collect();
                self.local_cards = cards;
                self.library_error = None;
            }
            Err(error) => {
                self.local_cards.clear();
                self.library_error = Some(error.to_string());
            }
        }
        match library.favorites() {
            Ok(favorites) => self.favorites = favorites.into_iter().collect(),
            Err(error) => self.library_error = Some(error.to_string()),
        }
        self.reload_collections();
    }

    /// Re-read the collection list, keeping an open collection open while it
    /// still exists (its count just changed, or it was deleted out from under
    /// the view, in which case the list panel takes its place).
    fn reload_collections(&mut self) {
        let Some(library) = &self.library else {
            self.collections.clear();
            self.active_collection = None;
            return;
        };
        match library.list_collections() {
            Ok(collections) => {
                self.collections = collections;
                self.collections_error = None;
                if self
                    .active_collection
                    .is_some_and(|id| !self.collections.iter().any(|c| c.id == id))
                {
                    self.active_collection = None;
                }
            }
            Err(error) => self.collections_error = Some(error.to_string()),
        }
    }

    /// Kick off a fresh search for the current query and filters.
    fn start_search(&mut self, ctx: &egui::Context) {
        self.generation += 1;
        self.error = None;
        self.next_cursor = None;
        self.total = None;
        if self.view == View::Local {
            self.loading = false;
            self.reload_local();
            self.cards = self.local_cards.clone();
            self.selected = None;
            return;
        }
        if self.view == View::Collections {
            self.loading = false;
            self.reload_collections();
            // The list panel draws itself; an open collection gets the cards
            // of the assets it holds, read from the same local library.
            let loaded = match (&self.library, self.active_collection) {
                (Some(library), Some(id)) => Some(collection_cards(library, id)),
                _ => None,
            };
            self.cards = match loaded {
                Some(Ok(cards)) => cards,
                Some(Err(error)) => {
                    self.library_error = Some(error.to_string());
                    Vec::new()
                }
                None => Vec::new(),
            };
            self.selected = None;
            return;
        }
        if self.view == View::Purchased {
            self.loading = false;
            self.cards.clear();
            self.selected = None;
            if self.source == Source::Live && self.account.is_some() {
                if self.owned_loaded {
                    self.cards = self.owned_cards.clone();
                } else {
                    self.loading = true;
                    self.request_owned(None);
                }
            }
            return;
        }
        if self.source == Source::Demo {
            self.loading = false;
            self.cards = data::demo_cards();
            self.selected = None;
            return;
        }

        if store_locked(self.source, &self.account, self.view) {
            // No session, no store: the generation bump above already voids
            // any page in flight, and the grid panel explains the emptiness.
            self.loading = false;
            self.cards.clear();
            self.selected = None;
            ctx.request_repaint();
            return;
        }

        self.loading = true;
        self.cards.clear();
        self.selected = None;
        self.request_page(None, false);
        ctx.request_repaint();
    }

    /// Ask the worker for one page. `append` marks "load more".
    fn request_page(&mut self, cursor: Option<String>, append: bool) {
        let Some(worker) = &self.worker else {
            self.error = Some("The catalog worker is not running.".to_owned());
            self.loading = false;
            return;
        };
        worker.send(Command::Search {
            generation: self.generation,
            query: data::query_for(&self.filters, &self.query, cursor, self.view == View::Free),
            append,
        });
    }

    /// Ask the worker for one page of the owned library.
    fn request_owned(&mut self, cursor: Option<String>) {
        let Some(worker) = &self.worker else {
            self.error = Some("The catalog worker is not running.".to_owned());
            self.loading = false;
            return;
        };
        worker.send(Command::Owned {
            generation: self.generation,
            cursor,
        });
    }

    pub fn can_load_more(&self) -> bool {
        if self.view == View::Purchased {
            return self.source == Source::Live
                && self.account.is_some()
                && self.owned_cursor.is_some()
                && !self.loading;
        }
        self.source == Source::Live && self.next_cursor.is_some() && !self.loading
    }

    pub fn load_more(&mut self, ctx: &egui::Context) {
        if self.view == View::Purchased {
            let Some(cursor) = self.owned_cursor.take() else {
                return;
            };
            self.loading = true;
            self.request_owned(Some(cursor));
            ctx.request_repaint();
            return;
        }
        let Some(cursor) = self.next_cursor.take() else {
            return;
        };
        self.loading = true;
        self.request_page(Some(cursor), true);
        ctx.request_repaint();
    }

    /// Drain whatever the worker finished since the last frame.
    fn poll(&mut self, ctx: &egui::Context) {
        let mut events = Vec::new();
        if let Some(worker) = &mut self.worker {
            while let Some(event) = worker.try_recv() {
                events.push(event);
            }
        }

        for event in events {
            match event {
                Event::Search {
                    generation,
                    append,
                    result,
                } => {
                    if generation != self.generation {
                        continue; // a newer search superseded this one
                    }
                    self.loading = false;
                    match *result {
                        Ok(page) => {
                            if !append {
                                self.cards.clear();
                            }
                            self.cards.extend(page.items.iter().map(Card::from_summary));
                            if let Some(total) = page.total {
                                self.total = Some(total);
                            }
                            self.next_cursor = page.next_cursor;
                            self.error = None;
                        }
                        Err(error) => {
                            self.error = Some(error);
                        }
                    }
                }
                Event::Detail { id, result } => match *result {
                    Ok(detail) => {
                        self.details.insert(id, detail);
                    }
                    Err(error) => {
                        self.error = Some(error);
                    }
                },
                Event::Owned { generation, result } => {
                    if generation != self.generation {
                        continue; // a newer search superseded this one
                    }
                    self.loading = false;
                    match *result {
                        Ok(page) => {
                            self.owned_cards
                                .extend(page.items.iter().map(Card::from_library_item));
                            self.cards = self.owned_cards.clone();
                            self.owned_cursor = page.next_cursor;
                            self.owned_loaded = true;
                            self.error = None;
                        }
                        Err(error) => {
                            self.error = Some(error);
                        }
                    }
                }
                Event::Thumbnail { id, texture } => {
                    self.textures.insert(id, texture);
                }
                Event::DownloadPhase { id, phase } => {
                    if let Some(job) = self.jobs.iter_mut().find(|job| job.id == id) {
                        match phase {
                            Phase::Resolving => {
                                job.status = JobStatus::Preparing;
                                job.progress = 0.02;
                            }
                            Phase::Download {
                                files_done,
                                files_total,
                                bytes_done,
                                bytes_total,
                            } => {
                                job.status = JobStatus::Downloading;
                                job.bytes_done = bytes_done;
                                job.bytes_total = bytes_total;
                                let fraction = if bytes_total > 0 {
                                    bytes_done as f32 / bytes_total as f32
                                } else if files_total > 0 {
                                    files_done as f32 / files_total as f32
                                } else {
                                    0.0
                                };
                                job.progress = 0.02 + 0.93 * fraction.clamp(0.0, 1.0);
                            }
                            Phase::Assembling => {
                                job.status = JobStatus::Assembling;
                                job.progress = 0.96;
                            }
                            Phase::Writing { done, total } => {
                                job.status = JobStatus::Writing;
                                job.progress = 0.96 + 0.04 * (done as f32 / total.max(1) as f32);
                            }
                        }
                    }
                }
                Event::DownloadDone { id, bytes, files } => {
                    if let Some(job) = self.jobs.iter_mut().find(|job| job.id == id) {
                        job.status = JobStatus::Done;
                        job.progress = 1.0;
                        job.bytes_done = bytes;
                        job.bytes_total = bytes;
                        job.files = files;
                    }
                    self.reload_local();
                    if self.view == View::Local {
                        self.cards = self.local_cards.clone();
                    }
                }
                Event::DownloadFailed { id, error } => {
                    if let Some(job) = self.jobs.iter_mut().find(|job| job.id == id) {
                        job.status = JobStatus::Failed(error);
                    }
                }
                Event::Exported { id, result } => {
                    if self.exporting.as_deref() == Some(id.as_str()) {
                        self.exporting = None;
                    }
                    match *result {
                        Ok(summary) => {
                            let next = if summary.blender_found {
                                "Open Blender and pick it in the AssetCraft sidebar."
                            } else {
                                "Blender was not found on this machine; the export is \
                                 complete either way, and the addon brings it in once \
                                 Blender is set up."
                            };
                            self.export_note = Some((
                                true,
                                format!(
                                    "Exported {} files as {} material(s) to {} - {}",
                                    summary.copied,
                                    summary.materials,
                                    summary.dir.display(),
                                    next
                                ),
                            ));
                            self.exported.insert(id, summary.dir);
                        }
                        Err(error) => {
                            self.export_note = Some((false, format!("The export failed: {error}")));
                        }
                    }
                }
                Event::Session(account) => {
                    self.account = account;
                    // A session landing is what unlocks the store: a saved
                    // login fills the empty first window, and the view may
                    // have opened before the session did.
                    if self.account.is_some()
                        && self.source == Source::Live
                        && (matches!(self.view, View::Home | View::Free)
                            || (self.view == View::Purchased && !self.owned_loaded))
                    {
                        self.start_search(ctx);
                    }
                }
                Event::SignedIn(result) => {
                    self.sign_in_busy = false;
                    match result {
                        Ok(account) => {
                            self.account = Some(account);
                            self.sign_in_open = false;
                            self.sign_in_code.clear();
                            self.sign_in_error = None;
                            // Signing in is the unlock: load the store the
                            // empty window was waiting for.
                            if self.source == Source::Live
                                && matches!(self.view, View::Home | View::Free | View::Purchased)
                            {
                                self.start_search(ctx);
                            }
                        }
                        Err(error) => self.sign_in_error = Some(error),
                    }
                }
                Event::SignedOut => {
                    self.account = None;
                    self.owned_cards.clear();
                    self.owned_cursor = None;
                    self.owned_loaded = false;
                    // Signing out locks the store again: the same locked
                    // start_search that fills it empties the window back.
                    if self.source == Source::Live
                        && matches!(self.view, View::Home | View::Free | View::Purchased)
                    {
                        self.start_search(ctx);
                    }
                }
                Event::Fatal(error) => {
                    self.error = Some(error);
                    self.source = Source::Demo;
                    self.start_search(ctx);
                }
            }
        }
    }

    fn switch_source(&mut self, source: Source, ctx: &egui::Context) {
        if self.source == source {
            return;
        }
        self.source = source;
        self.textures.clear();
        self.requested.clear();
        self.details.clear();
        self.owned_cards.clear();
        self.owned_cursor = None;
        self.owned_loaded = false;
        self.start_search(ctx);
    }

    fn hits(&self) -> Vec<usize> {
        let mut hits = data::visible(&self.cards, self.view, &self.favorites, &self.local);
        if self.source == Source::Demo || self.view == View::Purchased {
            // Demo data and the owned library are filtered locally; catalog
            // search results are filtered by the API.
            let needle = self.query.trim().to_lowercase();
            let slug = self.filters.listing_slug();
            hits.retain(|&i| {
                let card = &self.cards[i];
                (needle.is_empty() || card.title.to_lowercase().contains(&needle))
                    && slug.is_none_or(|slug| card.listing_type.as_deref() == Some(slug))
            });
        }
        hits
    }

    fn nav_counts(&self) -> Vec<(View, Option<usize>)> {
        View::ALL
            .iter()
            .map(|&view| {
                let count = match view {
                    View::Home => match self.source {
                        Source::Demo => Some(self.cards.len()),
                        Source::Live => self.total.map(|total| total as usize),
                    },
                    View::Free if self.source == Source::Demo => {
                        Some(self.cards.iter().filter(|card| card.free).count())
                    }
                    View::Favorites => Some(self.favorites.len()),
                    View::Local => Some(self.local_cards.len()),
                    View::Collections => self.library.as_ref().map(|_| self.collections.len()),
                    View::Purchased if self.source == Source::Live => {
                        self.account.as_ref().map(|_| self.owned_cards.len())
                    }
                    _ => None,
                };
                (view, count)
            })
            .collect()
    }

    // ----- card state ------------------------------------------------------

    pub fn texture(&self, id: &str) -> Option<&egui::TextureHandle> {
        self.textures.get(id)
    }

    /// Request a card's thumbnail once, when it first becomes visible.
    pub fn ensure_thumbnail(&mut self, i: usize) {
        let Some(card) = self.cards.get(i) else {
            return;
        };
        if self.textures.contains_key(&card.id) || self.requested.contains(&card.id) {
            return;
        }
        let Some(url) = card.thumbnail.clone() else {
            return;
        };
        self.requested.insert(card.id.clone());
        if let Some(worker) = &self.worker {
            worker.send(Command::Thumbnail {
                id: card.id.clone(),
                url,
            });
        }
    }

    pub fn select(&mut self, i: usize) {
        self.selected = Some(i);
        // Local items (and collection views, which are built from them) have
        // no marketplace listing to fetch.
        if self.source != Source::Live
            || matches!(self.view, View::Local | View::Purchased | View::Collections)
        {
            return;
        }
        let Some(card) = self.cards.get(i) else {
            return;
        };
        if self.details.contains_key(&card.id) {
            return;
        }
        if let Some(worker) = &self.worker {
            worker.send(Command::Detail(AssetId::new(card.id.clone())));
        }
    }

    /// Flip a favourite: in memory so every view updates at once, and in the
    /// library so the same hearts are there next launch. With no library
    /// (demo mode, or a library that failed to open) the heart still works for
    /// the session — the note is the user's, even if the database is not.
    pub fn toggle_favorite(&mut self, id: &str) {
        let added = !self.favorites.remove(id);
        if added {
            self.favorites.insert(id.to_owned());
        }
        if let Some(library) = &self.library {
            let asset = AssetId::new(id.to_owned());
            let result = if added {
                library.favorite(&asset)
            } else {
                library.unfavorite(&asset)
            };
            if let Err(error) = result {
                self.library_error = Some(error.to_string());
            }
        }
    }

    // ----- collections -----------------------------------------------------

    /// Create a collection from the list panel's name box.
    fn create_collection(&mut self) {
        let Some(library) = &self.library else {
            return;
        };
        match library.create_collection(&self.collection_new) {
            Ok(_) => {
                self.collection_new.clear();
                self.collections_error = None;
                self.reload_collections();
            }
            Err(error) => self.collections_error = Some(error.to_string()),
        }
    }

    /// Apply the inline rename editor to its collection.
    fn rename_collection(&mut self) {
        let Some((id, name)) = self.collection_renaming.take() else {
            return;
        };
        let Some(library) = &self.library else {
            return;
        };
        match library.rename_collection(id, &name) {
            Ok(true) => {
                self.collections_error = None;
                self.reload_collections();
            }
            Ok(false) => self.collections_error = Some("that collection is gone".to_owned()),
            Err(error) => {
                self.collections_error = Some(error.to_string());
                self.collection_renaming = Some((id, name));
            }
        }
    }

    /// Delete a collection. The assets keep their files; only the group goes.
    fn delete_collection(&mut self, id: i64) {
        self.collection_delete_armed = None;
        self.collection_renaming = None;
        let Some(library) = &self.library else {
            return;
        };
        match library.delete_collection(id) {
            Ok(_) => {
                self.collections_error = None;
                self.reload_collections();
            }
            Err(error) => self.collections_error = Some(error.to_string()),
        }
    }

    /// Put an asset in or out of a collection from the detail panel's checks.
    fn toggle_membership(&mut self, collection: i64, asset: &str) {
        let Some(library) = &self.library else {
            return;
        };
        let asset_id = AssetId::new(asset.to_owned());
        let result = match library.collections_for(&asset_id) {
            Ok(memberships) if memberships.contains(&collection) => {
                library.remove_from_collection(collection, &asset_id)
            }
            Ok(_) => library.add_to_collection(collection, &asset_id),
            Err(error) => Err(error),
        };
        match result {
            Ok(_) => {
                self.collections_error = None;
                self.reload_collections();
            }
            Err(error) => self.collections_error = Some(error.to_string()),
        }
    }

    pub fn toggle_checked(&mut self, id: &str) {
        if !self.checked.remove(id) {
            self.checked.insert(id.to_owned());
        }
    }

    /// Lay a downloaded asset out for Blender. The worker reads the files the
    /// library recorded and writes the export; nothing is fetched.
    pub fn start_export(&mut self, id: &str) {
        if self.exporting.is_some() {
            return;
        }
        let Some(worker) = &self.worker else {
            self.export_note = Some((false, "The background worker is not running.".to_owned()));
            return;
        };
        worker.send(Command::ExportBlender { id: id.to_owned() });
        self.exporting = Some(id.to_owned());
        self.export_note = None;
    }

    /// Downloads need an Epic session, which lands with the library milestone,
    /// so only the demo catalog can queue one for now.
    pub fn download_enabled(&self) -> bool {
        self.source == Source::Demo || self.account.is_some()
    }

    pub fn start_download(&mut self, i: usize) {
        let Some(card) = self.cards.get(i) else {
            return;
        };
        let id = card.id.clone();
        let name = card.title.clone();
        if self.jobs.iter().any(|job| job.id == id) {
            return;
        }

        let demo = self.source == Source::Demo;
        if !demo {
            let Some(worker) = &self.worker else {
                self.error = Some("The catalog worker is not running.".to_owned());
                return;
            };
            // A stand-in carrying what the card knows: the worker joins it
            // to the account's real library entry, and records that entry,
            // not this one.
            let item = assetcraft_core::LibraryItem {
                asset_id: id.clone(),
                namespace: "fab".to_owned(),
                title: name.clone(),
                description: card.description.clone(),
                url: None,
                source: Some("fab".to_owned()),
                distribution_method: Some("download".to_owned()),
                artifact_id: None,
                categories: card
                    .listing_type
                    .iter()
                    .map(|name| assetcraft_core::Category {
                        name: Some(name.clone()),
                        ..Default::default()
                    })
                    .collect(),
                images: card
                    .thumbnail
                    .iter()
                    .map(|url| assetcraft_core::AssetImage {
                        url: Some(url.clone()),
                        kind: Some("thumbnail".to_owned()),
                        ..Default::default()
                    })
                    .collect(),
            };
            // The download follows the account's library to the artifact
            // Fab delivers, so the resolution picker travels with it but
            // decides nothing: an artifact is the one build, not a set of
            // files to choose between. The preference rides along for the
            // day Fab serves more of the library to bearer tokens.
            worker.send(Command::Download {
                item,
                quality: (self.resolution > 0).then(|| QUALITY_CHOICES[self.resolution].to_owned()),
            });
        }
        self.jobs.push(Job {
            id,
            name,
            progress: 0.0,
            status: if demo {
                JobStatus::Downloading
            } else {
                JobStatus::Preparing
            },
            bytes_done: 0,
            bytes_total: 0,
            files: 0,
            demo,
        });
    }

    fn tick_jobs(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let dt = self
            .last_time
            .replace(now)
            .map_or(0.0, |last| (now - last).max(0.0));
        if self.jobs.is_empty() {
            return;
        }
        // Demo downloads are simulated at a fixed rate; live ones report their
        // own progress through the worker.
        let mut finished = Vec::new();
        for job in &mut self.jobs {
            if !job.demo {
                continue;
            }
            if job.running() {
                job.progress = (job.progress + dt as f32 * 0.18).min(1.0);
                if job.progress >= 1.0 {
                    job.status = JobStatus::Done;
                }
            }
            if job.status == JobStatus::Done {
                finished.push(job.id.clone());
            }
        }
        for id in finished {
            self.local.insert(id);
        }
        if self.jobs.iter().any(|job| job.running()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
    }

    // ----- panels ----------------------------------------------------------

    fn nav(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 6.0, theme::BLUE);
            ui.painter().text(
                rect.center() + egui::vec2(0.5, 0.5),
                egui::Align2::CENTER_CENTER,
                "A",
                egui::FontId::proportional(16.0),
                egui::Color32::WHITE,
            );
            ui.label(
                egui::RichText::new("AssetCraft")
                    .strong()
                    .size(15.0)
                    .color(theme::FG),
            );
        });
        ui.add_space(12.0);

        let counts = self.nav_counts();
        let collections = self.collections.clone();
        for (view, count) in counts {
            if nav_row(ui, self.view == view, view.label(), count).clicked() {
                self.view = view;
                self.active_collection = None;
                self.checked.clear();
                self.start_search(ctx);
            }
            if view == View::Collections && self.view == View::Collections {
                // The open view's collections, indented as its children.
                for collection in &collections {
                    let active = self.active_collection == Some(collection.id);
                    let response = ui
                        .horizontal(|ui| {
                            ui.add_space(14.0);
                            nav_row(ui, active, &collection.name, Some(collection.asset_count))
                        })
                        .inner;
                    if response.clicked() {
                        self.active_collection = Some(collection.id);
                        self.checked.clear();
                        self.start_search(ctx);
                    }
                }
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("TYPES")
                .size(11.0)
                .color(theme::FG_MUTED),
        );
        ui.add_space(2.0);
        if nav_row(ui, self.filters.listing_type.is_none(), "All assets", None).clicked() {
            self.filters.listing_type = None;
            self.start_search(ctx);
        }
        for (index, (_, label)) in data::LISTING_TYPES.iter().enumerate() {
            if nav_row(ui, self.filters.listing_type == Some(index), label, None).clicked() {
                self.filters.listing_type = Some(index);
                self.start_search(ctx);
            }
        }

        let mut open_sign_in = false;
        let mut sign_out = false;
        let mut open_settings = false;
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                if ui.button("\u{2699}  Settings").clicked() {
                    open_settings = true;
                }
            });
            ui.horizontal(|ui| {
                let live = self.source == Source::Live;
                if selectable(ui, live, "Live", None).clicked() {
                    self.switch_source(Source::Live, ctx);
                }
                if selectable(ui, !live, "Demo data", None).clicked() {
                    self.switch_source(Source::Demo, ctx);
                }
            });
            ui.label(
                egui::RichText::new(if self.source == Source::Live {
                    "fab.com catalog"
                } else {
                    "offline demo catalog"
                })
                .size(11.0)
                .color(theme::FG_MUTED),
            );
            ui.add_space(2.0);
            match &self.account {
                Some(account) => {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(grid::ellipsize(&account.name, 18))
                                .size(11.0)
                                .color(theme::FG_SECONDARY),
                        );
                        if ui.small_button("Sign out").clicked() {
                            sign_out = true;
                        }
                    });
                }
                None => {
                    if selectable(ui, false, "Sign in to Epic", None).clicked() {
                        open_sign_in = true;
                    }
                }
            }
        });
        if open_sign_in {
            self.open_sign_in();
        }
        if sign_out {
            if let Some(worker) = &self.worker {
                worker.send(Command::SignOut);
            }
        }
        if open_settings {
            self.open_settings();
        }
    }

    // ----- settings --------------------------------------------------------

    fn open_settings(&mut self) {
        self.settings_dir = self
            .settings
            .download_dir
            .as_ref()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default();
        // The stored quality is a word (`4K`); the picker index is where that
        // word sits in the shared list. `None` is the list's first entry.
        self.settings_quality = self
            .settings
            .default_quality
            .as_deref()
            .and_then(|quality| {
                QUALITY_CHOICES
                    .iter()
                    .position(|choice| choice.eq_ignore_ascii_case(quality))
            })
            .unwrap_or(0);
        self.settings_error = None;
        self.settings_open = true;
    }

    /// Save the dialog's fields: write the file, keep the in-memory copy, and
    /// adopt the chosen quality so the detail panel's picker starts there.
    fn save_settings(&mut self) {
        let Some(config) = &self.config else {
            self.settings_error =
                Some("the settings file location is unknown on this machine".to_owned());
            return;
        };
        let chosen = Settings {
            default_quality: (self.settings_quality > 0)
                .then(|| QUALITY_CHOICES[self.settings_quality].to_owned()),
            download_dir: {
                let text = self.settings_dir.trim();
                (!text.is_empty()).then(|| std::path::PathBuf::from(text))
            },
        };
        match chosen.save(&config.paths) {
            Ok(()) => {
                // The picker and the setting index the same list, so adopting
                // it is moving the index across.
                self.resolution = self.settings_quality;
                self.settings = chosen;
                self.settings_error = None;
                self.settings_open = false;
            }
            Err(error) => self.settings_error = Some(error.to_string()),
        }
    }

    /// The Collections view's list panel: name, count, and every operation.
    /// An open collection is drawn as the card grid by the caller instead;
    /// the list is where the organising happens, and the detail panel is
    /// where assets get ticked into the groups it names.
    fn collections_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(
            egui::RichText::new("Collections")
                .size(17.0)
                .strong()
                .color(theme::FG),
        );
        ui.label(
            egui::RichText::new(
                "Named groups of the assets you hold, kept beside the local library. \
                 Open one to see its assets; tick a collection in an asset's detail panel \
                 to put it in.",
            )
            .size(11.0)
            .color(theme::FG_MUTED),
        );
        ui.add_space(10.0);

        if self.library.is_none() {
            // Demo mode and a failed open both end up here; the notes live in
            // the database, so say which is missing instead of an empty list.
            ui.label(
                egui::RichText::new("Collections live in the local library, which is not open.")
                    .color(theme::FG_MUTED),
            );
            if let Some(error) = &self.library_error {
                ui.label(egui::RichText::new(error).size(11.0).color(theme::RED));
            }
            return;
        }

        if let Some(error) = &self.collections_error {
            ui.label(egui::RichText::new(error).size(11.0).color(theme::RED));
            ui.add_space(4.0);
        }

        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.collection_new)
                    .hint_text("new collection name")
                    .desired_width(220.0),
            );
            let submit = ui.button("Add").clicked()
                || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if submit && !self.collection_new.trim().is_empty() {
                self.create_collection();
            }
        });
        ui.add_space(8.0);

        let collections = self.collections.clone();
        if collections.is_empty() {
            ui.label(
                egui::RichText::new("Nothing organised yet — name a collection above.")
                    .color(theme::FG_MUTED),
            );
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for collection in &collections {
                    let mut open = false;
                    let mut start_rename = false;
                    let mut save_rename = false;
                    let mut cancel_rename = false;
                    let mut arm_delete = false;
                    let mut delete_now = false;

                    if self
                        .collection_renaming
                        .as_ref()
                        .is_some_and(|(id, _)| *id == collection.id)
                    {
                        // The rename editor replaces its row in place.
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            if let Some((_, text)) = self.collection_renaming.as_mut() {
                                let response =
                                    ui.add(egui::TextEdit::singleline(text).desired_width(170.0));
                                if response.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    save_rename = true;
                                }
                            }
                            if ui.small_button("Save").clicked() {
                                save_rename = true;
                            }
                            if ui.small_button("Cancel").clicked() {
                                cancel_rename = true;
                            }
                        });
                    } else {
                        let armed = self.collection_delete_armed == Some(collection.id);
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            let name = grid::ellipsize(&collection.name, 26);
                            if selectable(ui, false, &name, Some(collection.asset_count)).clicked()
                            {
                                open = true;
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let response = ui
                                        .small_button(if armed { "Sure?" } else { "\u{2715}" })
                                        .on_hover_text(if armed {
                                            "click again to delete the collection"
                                        } else {
                                            "delete this collection"
                                        });
                                    if response.clicked() {
                                        if armed {
                                            delete_now = true;
                                        } else {
                                            arm_delete = true;
                                        }
                                    }
                                    if ui
                                        .small_button("\u{270e}")
                                        .on_hover_text("rename")
                                        .clicked()
                                    {
                                        start_rename = true;
                                    }
                                },
                            );
                        });
                    }

                    if save_rename {
                        self.rename_collection();
                    }
                    if cancel_rename {
                        self.collection_renaming = None;
                    }
                    if start_rename {
                        self.collection_renaming = Some((collection.id, collection.name.clone()));
                        self.collection_delete_armed = None;
                    }
                    if arm_delete {
                        self.collection_delete_armed = Some(collection.id);
                    }
                    if delete_now {
                        self.delete_collection(collection.id);
                    }
                    if open {
                        self.active_collection = Some(collection.id);
                        self.checked.clear();
                        self.start_search(ctx);
                    }
                }
            });
    }

    /// Settings as a small modal: the two choices the README promises, and the
    /// standard paths shown read-only so a custom download root can be put
    /// back without hunting through this file.
    fn settings_dialog(&mut self, ctx: &egui::Context) {
        let mut save = false;
        let mut cancel = false;
        let paths = self.config.as_ref().map(|config| config.paths.clone());
        let standard = paths
            .as_ref()
            .map(|paths| paths.assets_dir().display().to_string())
            .unwrap_or_default();
        let file = paths
            .as_ref()
            .map(|paths| paths.settings_file().display().to_string())
            .unwrap_or_default();
        let modal = egui::Modal::new(egui::Id::new("settings")).show(ctx, |ui| {
            ui.set_width(430.0);
            ui.label(
                egui::RichText::new("Settings")
                    .size(17.0)
                    .strong()
                    .color(theme::FG),
            );
            ui.label(
                egui::RichText::new(
                    "Saved as TOML and read again at every download — the command line \
                     reads the same file, and a hand edit here is a hand edit there.",
                )
                .size(11.0)
                .color(theme::FG_MUTED),
            );

            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("Downloads folder")
                    .size(12.0)
                    .color(theme::FG_SECONDARY),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.settings_dir)
                    .hint_text(&standard)
                    .desired_width(f32::INFINITY),
            );
            ui.label(
                egui::RichText::new(format!(
                    "One directory per asset under this root. Empty means the standard \
                     place: {standard}"
                ))
                .size(11.0)
                .color(theme::FG_MUTED),
            );

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Default quality")
                    .size(12.0)
                    .color(theme::FG_SECONDARY),
            );
            egui::ComboBox::from_id_salt("default-quality")
                .width(160.0)
                .selected_text(QUALITY_CHOICES[self.settings_quality])
                .show_ui(ui, |ui| {
                    for (index, choice) in QUALITY_CHOICES.iter().enumerate() {
                        ui.selectable_value(&mut self.settings_quality, index, *choice);
                    }
                });
            ui.label(
                egui::RichText::new(
                    "The quality tier the download picker starts on. Assets without a \
                     file in the chosen tier report it instead of arriving in another one.",
                )
                .size(11.0)
                .color(theme::FG_MUTED),
            );

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!("Settings file: {file}"))
                    .monospace()
                    .size(10.0)
                    .color(theme::FG_MUTED),
            );

            if let Some(error) = &self.settings_error {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(error).size(11.0).color(theme::RED));
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let button = egui::Button::new(
                    egui::RichText::new("Save")
                        .strong()
                        .color(egui::Color32::WHITE),
                )
                .min_size(egui::vec2(96.0, 28.0))
                .fill(theme::GREEN);
                if ui.add_enabled(paths.is_some(), button).clicked() {
                    save = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        });

        if modal.should_close() || cancel {
            self.settings_open = false;
        }
        if save {
            self.save_settings();
        }
    }

    fn open_sign_in(&mut self) {
        self.sign_in_open = true;
        self.sign_in_code.clear();
        self.sign_in_error = None;
        self.sign_in_busy = false;
    }

    /// Step-by-step sign-in: open Epic's page, paste the code it shows.
    fn sign_in_dialog(&mut self, ctx: &egui::Context) {
        let mut submit = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("sign-in")).show(ctx, |ui| {
            ui.set_width(430.0);
            ui.label(
                egui::RichText::new("Sign in to Epic Games")
                    .size(17.0)
                    .strong()
                    .color(theme::FG),
            );
            ui.label(
                egui::RichText::new(
                    "AssetCraft signs in with Epic's launcher client, the same flow the \
                     command-line tool uses. The token stays on this machine.",
                )
                .size(11.0)
                .color(theme::FG_MUTED),
            );

            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("1. Open the sign-in page in your browser")
                    .size(12.0)
                    .color(theme::FG_SECONDARY),
            );
            ui.horizontal(|ui| {
                if ui.button("Open sign-in page").clicked() {
                    ctx.open_url(egui::OpenUrl::new_tab(self.sign_in_url.as_str()));
                }
                if ui.button("Copy link").clicked() {
                    ctx.copy_text(self.sign_in_url.clone());
                }
            });
            ui.label(
                egui::RichText::new(&self.sign_in_url)
                    .monospace()
                    .size(10.0)
                    .color(theme::FG_MUTED),
            );

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("2. Paste the code the page shows")
                    .size(12.0)
                    .color(theme::FG_SECONDARY),
            );
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.sign_in_code)
                    .hint_text("authorization code")
                    .desired_width(f32::INFINITY),
            );
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit = true;
            }

            if let Some(error) = &self.sign_in_error {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(error).size(11.0).color(theme::RED));
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ready = !self.sign_in_code.trim().is_empty() && !self.sign_in_busy;
                let button = egui::Button::new(
                    egui::RichText::new("Sign in")
                        .strong()
                        .color(egui::Color32::WHITE),
                )
                .min_size(egui::vec2(96.0, 28.0))
                .fill(theme::GREEN);
                if ui.add_enabled(ready, button).clicked() {
                    submit = true;
                }
                if self.sign_in_busy {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new("Exchanging the code...")
                            .size(11.0)
                            .color(theme::FG_MUTED),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        });

        if modal.should_close() || cancel {
            self.sign_in_open = false;
        }
        if submit && !self.sign_in_busy {
            self.submit_sign_in();
        }
    }

    fn submit_sign_in(&mut self) {
        let code = self.sign_in_code.trim().to_owned();
        if code.is_empty() {
            return;
        }
        let Some(worker) = &self.worker else {
            self.sign_in_error = Some("The background worker is not running.".to_owned());
            return;
        };
        worker.send(Command::SignIn { code });
        self.sign_in_busy = true;
        self.sign_in_error = None;
    }

    fn search_bar(&mut self, ui: &mut egui::Ui) {
        let frame = egui::Frame::new()
            .fill(theme::PANEL)
            .inner_margin(egui::Margin::symmetric(12, 8));
        frame.show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let field = egui::Frame::new()
                    .fill(theme::ELEVATED)
                    .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
                    .corner_radius(9.0)
                    .inner_margin(egui::Margin::symmetric(10, 6));
                field.show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        icons::search(
                            ui.painter(),
                            r.center(),
                            16.0,
                            egui::Stroke::new(1.4_f32, theme::FG_MUTED),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.query)
                                .hint_text("Search the marketplace")
                                .desired_width(380.0),
                        );
                    })
                });
                if !self.query.is_empty() && ui.small_button("Clear").clicked() {
                    self.query.clear();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(rect.center(), 12.0, theme::ELEVATED);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "A",
                        egui::FontId::proportional(12.0),
                        theme::FG_SECONDARY,
                    );
                    if self.loading {
                        ui.spinner();
                    } else if let Some(total) = self.total {
                        ui.label(
                            egui::RichText::new(format!("{total} assets"))
                                .size(11.0)
                                .color(theme::FG_MUTED),
                        );
                    }
                });
            });
        });
    }

    fn filter_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let frame = egui::Frame::new()
            .fill(theme::PANEL)
            .inner_margin(egui::Margin::symmetric(12, 6));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                let options: Vec<(&'static str, usize)> = data::LISTING_TYPES
                    .iter()
                    .enumerate()
                    .map(|(index, (_, label))| (*label, index))
                    .collect();
                let before = self.filters;
                dropdown(ui, "Type", &options, &mut self.filters.listing_type);
                if self.filters != before {
                    self.start_search(ctx);
                }
                if self.filters != Filters::default() && ui.small_button("Clear filters").clicked()
                {
                    self.filters = Filters::default();
                    self.start_search(ctx);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let shown = self.hits().len();
                    ui.label(
                        egui::RichText::new(format!("{shown} listed"))
                            .size(11.0)
                            .color(theme::FG_MUTED),
                    );
                });
            });
        });
    }

    fn detail(&mut self, ui: &mut egui::Ui, i: usize) {
        let (id, title, seller, rating, listing_type, hue, tags, is_new, card_description) = {
            let card = &self.cards[i];
            (
                card.id.clone(),
                card.title.clone(),
                card.seller.clone(),
                card.rating,
                card.listing_type.clone(),
                card.hue,
                card.tags.clone(),
                card.is_new,
                card.description.clone(),
            )
        };
        self.ensure_thumbnail(i);
        let description = self
            .details
            .get(&id)
            .and_then(|detail| detail.description.clone())
            .or(card_description);
        let published = self
            .details
            .get(&id)
            .and_then(|detail| detail.created_at.clone());
        let favorite = self.favorites.contains(&id);
        let live = self.source == Source::Live;
        let local_view = self.view == View::Local;
        let owned_view = self.view == View::Purchased;
        // Collections organise what you hold, so they are offered for local
        // assets in any view, read once for the frame; the boxes write through
        // to the library the moment they are ticked.
        let is_local = self.local.contains(&id);
        let membership: Vec<i64> = if is_local {
            self.library
                .as_ref()
                .and_then(|library| library.collections_for(&AssetId::new(id.clone())).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let collections_open = is_local && self.library.is_some();
        let collections: Vec<Collection> = if collections_open {
            self.collections.clone()
        } else {
            Vec::new()
        };
        // Any view can export: the record says whether this asset's files are
        // on disk, and the export reads only those files.
        let disk = self
            .library
            .as_ref()
            .and_then(|library| library.download(&AssetId::new(id.clone())).ok().flatten());
        let on_disk = disk
            .as_ref()
            .map(|download| download.path.display().to_string());
        let demo_note = "Offline demo listing: the marketplace catalog is not \
                         connected in demo mode.";

        let frame = egui::Frame::new()
            .fill(theme::PANEL)
            .inner_margin(egui::Margin::same(14));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.small_button("Back").clicked() {
                    self.selected = None;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(if favorite { "\u{2665}" } else { "\u{2661}" })
                        .clicked()
                    {
                        self.toggle_favorite(&id);
                    }
                });
            });

            let list_height = (ui.available_height() - ACTION_BAR).max(140.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), list_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let (preview, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 210.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(preview, 8.0, theme::BG);
                            match self.texture(&id) {
                                Some(texture) => {
                                    grid::draw_thumbnail(ui.painter(), preview, texture)
                                }
                                None => grid::draw_placeholder(ui.painter(), preview, hue),
                            }

                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&title)
                                        .size(18.0)
                                        .strong()
                                        .color(theme::FG),
                                );
                                if is_new {
                                    chip(ui, "NEW");
                                }
                            });
                            let mut line = seller.unwrap_or_else(|| {
                                if local_view {
                                    "Local copy"
                                } else if owned_view {
                                    "Owned"
                                } else {
                                    "Unknown seller"
                                }
                                .to_owned()
                            });
                            if let Some(rating) = rating {
                                line.push_str(&format!("  \u{00b7}  Rating {rating:.1}"));
                            }
                            ui.label(egui::RichText::new(line).size(12.0));

                            ui.add_space(4.0);
                            let mut rows =
                                vec![("Type", data::listing_label(listing_type.as_deref()))];
                            if let Some(published) = &published {
                                rows.push(("Published", short_date(published)));
                            }
                            if let Some(path) = &on_disk {
                                rows.push(("On disk", path.clone()));
                            }
                            if let Some(path) = self.exported.get(&id) {
                                rows.push(("Exported", path.display().to_string()));
                            }
                            for (key, value) in rows {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(key).size(11.0).color(theme::FG_MUTED),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let response = ui.label(
                                                egui::RichText::new(grid::ellipsize(&value, 46))
                                                    .size(12.0)
                                                    .color(theme::FG_SECONDARY),
                                            );
                                            if matches!(key, "On disk" | "Exported") {
                                                response.on_hover_text(&value);
                                            }
                                        },
                                    );
                                });
                            }

                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new("ABOUT")
                                    .size(11.0)
                                    .color(theme::FG_MUTED),
                            );
                            if live && description.is_none() {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new("Loading details...")
                                            .size(12.0)
                                            .color(theme::FG_MUTED),
                                    );
                                });
                            } else {
                                let body = description.as_deref().unwrap_or(if local_view {
                                    "No description was recorded for this asset."
                                } else {
                                    demo_note
                                });
                                ui.label(
                                    egui::RichText::new(strip_tags(body).trim())
                                        .size(12.0)
                                        .color(theme::FG_SECONDARY),
                                );
                            }

                            if !tags.is_empty() {
                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new("TAGS")
                                        .size(11.0)
                                        .color(theme::FG_MUTED),
                                );
                                ui.horizontal_wrapped(|ui| {
                                    for tag in &tags {
                                        chip(ui, tag);
                                    }
                                });
                            }

                            if collections_open {
                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new("COLLECTIONS")
                                        .size(11.0)
                                        .color(theme::FG_MUTED),
                                );
                                if collections.is_empty() {
                                    ui.label(
                                        egui::RichText::new(
                                            "No collections yet — make them from the \
                                             Collections view.",
                                        )
                                        .size(11.0)
                                        .color(theme::FG_MUTED),
                                    );
                                }
                                for collection in &collections {
                                    let mut member = membership.contains(&collection.id);
                                    if ui
                                        .checkbox(
                                            &mut member,
                                            egui::RichText::new(&collection.name).size(12.0),
                                        )
                                        .clicked()
                                    {
                                        self.toggle_membership(collection.id, &id);
                                    }
                                }
                            }

                            if live && !local_view {
                                ui.add_space(8.0);
                                ui.hyperlink_to(
                                    "Open on fab.com",
                                    format!("https://www.fab.com/listings/{id}"),
                                );
                            }
                        });
                },
            );

            ui.separator();
            let downloaded = self.local.contains(&id);
            let can_download = (!live || self.account.is_some()) && !local_view && !downloaded;
            ui.horizontal(|ui| {
                if can_download {
                    egui::ComboBox::from_id_salt("resolution")
                        .width(112.0)
                        .selected_text(QUALITY_CHOICES[self.resolution])
                        .show_ui(ui, |ui| {
                            for (index, label) in QUALITY_CHOICES.iter().enumerate() {
                                ui.selectable_value(&mut self.resolution, index, *label);
                            }
                        });
                }
                let button = egui::Button::new(
                    egui::RichText::new("Download")
                        .strong()
                        .color(egui::Color32::WHITE),
                )
                .min_size(egui::vec2(if can_download { 0.0 } else { 132.0 }, 30.0))
                .fill(if can_download {
                    theme::GREEN
                } else {
                    theme::GREEN.gamma_multiply(0.35)
                });
                let response = ui.add_enabled(can_download, button);
                if !can_download {
                    response.on_disabled_hover_text(if local_view || downloaded {
                        "Already in your local library"
                    } else {
                        "Sign in to Epic to download assets"
                    });
                } else if response.clicked() {
                    self.start_download(i);
                }

                if disk.is_some() {
                    let busy = self.exporting.as_deref() == Some(id.as_str());
                    let button = egui::Button::new(
                        egui::RichText::new(if busy {
                            "Exporting..."
                        } else {
                            "Export to Blender"
                        })
                        .strong()
                        .color(egui::Color32::WHITE),
                    )
                    .min_size(egui::vec2(148.0, 30.0))
                    .fill(if busy {
                        theme::BLUE.gamma_multiply(0.6)
                    } else {
                        theme::BLUE
                    });
                    let response = ui.add_enabled(!busy, button);
                    let clicked = response.clicked();
                    if busy {
                        response.on_disabled_hover_text("Writing the export...");
                    } else {
                        response.on_hover_text(
                            "Lay the downloaded files out for Blender and write the import \
                             script. The files come from disk; nothing is fetched.",
                        );
                    }
                    if clicked {
                        self.start_export(&id);
                    }
                }
            });
        });
    }

    fn downloads(&mut self, ui: &mut egui::Ui) {
        let mut dismiss: Vec<String> = Vec::new();
        let frame = egui::Frame::new()
            .fill(theme::PANEL)
            .inner_margin(egui::Margin::symmetric(12, 6));
        frame.show(ui, |ui| {
            ui.label(
                egui::RichText::new(format!("Downloads ({})", self.jobs.len()))
                    .size(11.0)
                    .color(theme::FG_MUTED),
            );
            for job in &self.jobs {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&job.name).size(12.0));
                    ui.add(
                        egui::ProgressBar::new(job.progress)
                            .desired_width(220.0)
                            .animate(job.running()),
                    );
                    let colour = match &job.status {
                        JobStatus::Failed(_) => theme::RED,
                        JobStatus::Done => theme::GREEN,
                        _ => theme::FG_SECONDARY,
                    };
                    let response =
                        ui.label(egui::RichText::new(job.label()).size(11.0).color(colour));
                    if let JobStatus::Failed(error) = &job.status {
                        response.on_hover_text(error);
                    }
                    if !job.running() && ui.small_button("\u{2715}").clicked() {
                        dismiss.push(job.id.clone());
                    }
                });
            }
        });
        if !dismiss.is_empty() {
            self.jobs.retain(|job| !dismiss.contains(&job.id));
        }
    }

    /// The one-line result of the last export, kept until dismissed.
    fn export_banner(&mut self, ui: &mut egui::Ui) {
        let Some((ok, message)) = self.export_note.clone() else {
            return;
        };
        let (fill, edge, accent) = if ok {
            (
                egui::Color32::from_rgb(36, 48, 30),
                egui::Color32::from_rgb(86, 118, 64),
                theme::GREEN,
            )
        } else {
            (
                egui::Color32::from_rgb(58, 34, 34),
                egui::Color32::from_rgb(128, 62, 62),
                theme::RED,
            )
        };
        let mut dismiss = false;
        let frame = egui::Frame::new()
            .fill(fill)
            .stroke(egui::Stroke::new(1.0_f32, edge))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(12, 10));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(message)
                        .size(12.0)
                        .color(accent)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("\u{2715}").clicked() {
                        dismiss = true;
                    }
                });
            });
        });
        if dismiss {
            self.export_note = None;
        }
        ui.add_space(10.0);
    }

    /// The banner shown when the catalog could not be reached.
    fn error_banner(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(error) = self.error.clone() else {
            return;
        };
        let frame = egui::Frame::new()
            .fill(egui::Color32::from_rgb(58, 34, 34))
            .stroke(egui::Stroke::new(
                1.0_f32,
                egui::Color32::from_rgb(128, 62, 62),
            ))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(12, 10));
        frame.show(ui, |ui| {
            ui.label(
                egui::RichText::new("The catalog could not be reached")
                    .strong()
                    .color(theme::FG),
            );
            ui.label(
                egui::RichText::new(&error)
                    .size(11.0)
                    .color(theme::FG_SECONDARY),
            );
            ui.horizontal(|ui| {
                if ui.button("Retry").clicked() {
                    self.error = None;
                    self.start_search(ctx);
                }
                if self.source == Source::Live && ui.button("Use demo data").clicked() {
                    self.error = None;
                    self.switch_source(Source::Demo, ctx);
                }
            });
        });
        ui.add_space(10.0);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.tick_jobs(ctx);

        // Debounce typing so a search does not fire per keystroke.
        let now = ctx.input(|i| i.time);
        if self.query != self.sent_query {
            self.sent_query = self.query.clone();
            self.dirty_at = now;
        }
        if self.dirty_at > 0.0 && now - self.dirty_at >= QUERY_DEBOUNCE {
            self.dirty_at = 0.0;
            self.start_search(ctx);
        }

        let hits = self.hits();

        if !self.jobs.is_empty() {
            egui::TopBottomPanel::bottom("downloads").show(ctx, |ui| self.downloads(ui));
        }
        egui::TopBottomPanel::top("search").show(ctx, |ui| self.search_bar(ui));
        egui::TopBottomPanel::top("filters").show(ctx, |ui| self.filter_bar(ui, ctx));
        egui::SidePanel::left("nav")
            .exact_width(232.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(10, 12)),
            )
            .show(ctx, |ui| self.nav(ui, ctx));
        if let Some(i) = self.selected {
            egui::SidePanel::right("detail")
                .exact_width(400.0)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(theme::PANEL)
                        .inner_margin(egui::Margin::same(12)),
                )
                .show(ctx, |ui| self.detail(ui, i));
        }

        if self.sign_in_open {
            self.sign_in_dialog(ctx);
        }
        if self.settings_open {
            self.settings_dialog(ctx);
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::same(14)),
            )
            .show(ctx, |ui| {
                self.export_banner(ui);
                self.error_banner(ui, ctx);
                if self.error.is_some() && self.cards.is_empty() {
                    return;
                }

                // Settings that could not be read is said once, plainly, and
                // stays said until dismissed: the app runs on defaults in the
                // meantime, and a silent default is the thing that must not
                // happen quietly.
                if let Some(error) = self.settings_error.clone() {
                    if !self.settings_open {
                        let frame = egui::Frame::new()
                            .fill(egui::Color32::from_rgb(58, 50, 30))
                            .stroke(egui::Stroke::new(
                                1.0_f32,
                                egui::Color32::from_rgb(128, 110, 62),
                            ))
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::symmetric(12, 10));
                        let mut dismiss = false;
                        frame.show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("Settings could not be read")
                                    .strong()
                                    .color(theme::FG),
                            );
                            ui.label(
                                egui::RichText::new(&error)
                                    .size(11.0)
                                    .color(theme::FG_SECONDARY),
                            );
                            ui.horizontal(|ui| {
                                if ui.button("Open settings").clicked() {
                                    self.settings_open = true;
                                }
                                if ui.button("Dismiss").clicked() {
                                    dismiss = true;
                                }
                            });
                        });
                        if dismiss {
                            self.settings_error = None;
                        }
                        ui.add_space(10.0);
                    }
                }

                // The Collections view is local: it organises the assets the
                // library holds, so it works with no session and no network.
                // The list is its home; opening a collection falls through to
                // the card grid with the collection's cards loaded.
                if self.view == View::Collections {
                    if self.active_collection.is_none() {
                        self.collections_panel(ui, ctx);
                        return;
                    }
                    let name = self
                        .collections
                        .iter()
                        .find(|collection| Some(collection.id) == self.active_collection)
                        .map(|collection| collection.name.clone())
                        .unwrap_or_default();
                    ui.horizontal(|ui| {
                        if ui.small_button("All collections").clicked() {
                            self.active_collection = None;
                            self.start_search(ctx);
                        }
                        ui.label(
                            egui::RichText::new(grid::ellipsize(&name, 40))
                                .strong()
                                .color(theme::FG),
                        );
                    });
                    ui.add_space(6.0);
                }

                // Bridge's rule for the whole store, not just Purchased:
                // signed out, the grid is an empty window and this panel says
                // what fills it.
                if store_locked(self.source, &self.account, self.view) {
                    let mut open_sign_in = false;
                    ui.centered_and_justified(|ui| {
                        ui.vertical(|ui| {
                            let (title, body) = if self.view == View::Purchased {
                                (
                                    "Purchased needs an Epic account",
                                    "Sign in to Epic to see the assets you own on Fab. Until \
                                     then, demo data shows what the shell can do.",
                                )
                            } else {
                                (
                                    "Sign in to Epic to browse",
                                    "The marketplace catalog loads once you sign in. Until \
                                     then, demo data shows what the shell can do.",
                                )
                            };
                            ui.label(
                                egui::RichText::new(title)
                                    .size(15.0)
                                    .color(theme::FG_SECONDARY),
                            );
                            ui.label(egui::RichText::new(body).size(12.0).color(theme::FG_MUTED));
                            ui.add_space(6.0);
                            if ui.button("Sign in to Epic").clicked() {
                                open_sign_in = true;
                            }
                        });
                    });
                    if open_sign_in {
                        self.open_sign_in();
                    }
                    return;
                }

                if self.loading && self.cards.is_empty() {
                    ui.centered_and_justified(|ui| ui.spinner());
                    return;
                }

                if hits.is_empty() {
                    let text = match self.view {
                        View::Local => "Nothing downloaded yet. Assets you acquire land here.",
                        View::Collections => {
                            "This collection is empty. Tick a collection in an asset's \
                             detail panel to fill it."
                        }
                        View::Purchased => "Your Epic library is empty.",
                        _ if self.cards.is_empty() => "The catalog returned no listings.",
                        _ => "Nothing matches this view, search, and filters.",
                    };
                    let library_error = self.library_error.clone();
                    ui.centered_and_justified(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(text).color(theme::FG_MUTED));
                            if let Some(error) = &library_error {
                                ui.label(egui::RichText::new(error).size(11.0).color(theme::RED));
                            }
                        });
                    });
                    return;
                }

                grid::show(ui, ctx, self, &hits);
            });
    }
}

/// A full-width nav row: label on the left, count on the right, and a rounded
/// highlight when active or hovered, as in Bridge's left rail.
fn nav_row(ui: &mut egui::Ui, active: bool, text: &str, count: Option<usize>) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, 6.0, theme::ELEVATED);
    } else if response.hovered() {
        painter.rect_filled(rect, 6.0, egui::Color32::from_rgb(38, 38, 38));
    }
    painter.text(
        rect.left_center() + egui::vec2(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(13.0),
        if active {
            theme::FG
        } else {
            theme::FG_SECONDARY
        },
    );
    if let Some(count) = count {
        painter.text(
            rect.right_center() - egui::vec2(10.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::proportional(11.0),
            theme::FG_MUTED,
        );
    }
    response
}

fn selectable(ui: &mut egui::Ui, active: bool, text: &str, count: Option<usize>) -> egui::Response {
    let mut label = egui::RichText::new(text).color(if active {
        theme::FG
    } else {
        theme::FG_SECONDARY
    });
    if active {
        label = label.strong();
    }
    let response = ui.add(egui::Button::selectable(active, label));
    if let Some(count) = count {
        ui.label(
            egui::RichText::new(count.to_string())
                .size(11.0)
                .color(theme::FG_MUTED),
        );
    }
    response
}

fn chip(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(theme::ELEVATED)
        .corner_radius(9.0)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(11.0));
        });
}

/// A compact filter dropdown like Bridge's filter bar.
fn dropdown<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    title: &str,
    options: &[(&'static str, T)],
    slot: &mut Option<T>,
) {
    let current = options
        .iter()
        .find(|(_, value)| slot.as_ref() == Some(value))
        .map(|(label, _)| *label);
    let response = egui::ComboBox::from_id_salt(title)
        .width(112.0)
        .selected_text(current.unwrap_or("All"))
        .show_ui(ui, |ui| {
            for (label, value) in options {
                if ui
                    .selectable_label(slot.as_ref() == Some(value), *label)
                    .clicked()
                {
                    *slot = Some(*value);
                }
            }
            if ui
                .selectable_label(slot.is_none(), format!("All {title}"))
                .clicked()
            {
                *slot = None;
            }
        })
        .response;

    // egui draws no affordance on a ComboBox, and the fonts have no `▾`.
    let center = response.rect.right_center() + egui::vec2(-10.0, 0.0);
    let stroke = egui::Stroke::new(1.2_f32, theme::FG_MUTED);
    let painter = ui.painter();
    painter.line_segment(
        [
            center + egui::vec2(-3.5, -2.0),
            center + egui::vec2(0.0, 1.8),
        ],
        stroke,
    );
    painter.line_segment(
        [
            center + egui::vec2(3.5, -2.0),
            center + egui::vec2(0.0, 1.8),
        ],
        stroke,
    );
}

/// The Local view's cards: every asset the library holds, by title.
fn local_cards(library: &Library) -> assetcraft_core::Result<Vec<Card>> {
    let mut cards: Vec<Card> = library
        .list_all()?
        .iter()
        .map(Card::from_library_item)
        .collect();
    cards.sort_by_key(|card| card.title.to_lowercase());
    Ok(cards)
}

/// One collection's cards: the local library's cards restricted to its
/// members, in the membership's own title order — a collection reads like
/// the Local view it is drawn from. Members whose metadata row is gone
/// (a dropped row) leave a gap rather than an error: the files may still be
/// on disk, but there is no card to draw without a title.
fn collection_cards(library: &Library, id: i64) -> assetcraft_core::Result<Vec<Card>> {
    let owned: HashMap<String, Card> = local_cards(library)?
        .into_iter()
        .map(|card| (card.id.clone(), card))
        .collect();
    Ok(library
        .collection_members(id)?
        .iter()
        .filter_map(|member| owned.get(member))
        .cloned()
        .collect())
}

/// `2026-02-14T09:30:00.000Z` -> `2026-02-14`, leaving anything unexpected alone.
fn short_date(raw: &str) -> String {
    match raw.split_once('T') {
        Some((date, _)) if date.len() == 10 => date.to_owned(),
        _ => raw.to_owned(),
    }
}

/// Marketplace descriptions arrive as HTML; the shell shows plain text.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for character in html.chars() {
        match character {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(character),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{local_cards, short_date, store_locked, strip_tags, Source, View};
    use assetcraft_core::LibraryItem;
    use assetcraft_library::Library;

    #[test]
    fn the_store_waits_for_a_session() {
        use super::Account;
        let signed_out = &None::<Account>;
        // A stand-in session: only its presence matters to the rule.
        let signed_in = &Some(Account {
            name: "Tester".to_owned(),
        });
        // Bridge parity: signed out, the marketplace views are empty windows.
        for view in [View::Home, View::Free, View::Purchased] {
            assert!(store_locked(Source::Live, signed_out, view), "{view:?}");
        }
        // A session unlocks them; the demo dataset was asked for on purpose.
        for view in [View::Home, View::Free, View::Purchased] {
            assert!(!store_locked(Source::Live, signed_in, view), "{view:?}");
        }
        assert!(!store_locked(Source::Demo, signed_out, View::Home));
        // What is already the user's own never waits: the library and its
        // views work signed out.
        for view in [View::Local, View::Collections, View::Favorites] {
            assert!(!store_locked(Source::Live, signed_out, view), "{view:?}");
        }
    }

    #[test]
    fn the_local_view_lists_library_items_by_title() {
        let library = Library::open_in_memory().unwrap();
        for (id, title) in [("b", "Beta Asset"), ("a", "Alpha Asset")] {
            library
                .insert(&LibraryItem {
                    asset_id: id.to_owned(),
                    title: title.to_owned(),
                    ..Default::default()
                })
                .unwrap();
        }

        let cards = local_cards(&library).unwrap();
        let titles: Vec<&str> = cards.iter().map(|card| card.title.as_str()).collect();
        assert_eq!(titles, ["Alpha Asset", "Beta Asset"]);
        assert_eq!(cards[0].id, "a");
    }

    #[test]
    fn timestamps_are_shown_as_dates() {
        assert_eq!(short_date("2026-02-14T09:30:00.000Z"), "2026-02-14");
        assert_eq!(short_date("2026-02-14"), "2026-02-14");
        assert_eq!(short_date("unknown"), "unknown");
    }

    #[test]
    fn descriptions_lose_their_markup() {
        assert_eq!(strip_tags("<p>Hello <b>world</b></p>"), "Hello world");
        assert_eq!(strip_tags("Fish &amp; chips"), "Fish & chips");
        assert_eq!(strip_tags("a<br/>b"), "a b");
    }
}
