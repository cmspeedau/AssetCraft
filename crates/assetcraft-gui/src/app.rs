//! The application shell: left nav, search + filter bar, grid, asset detail
//! panel, and the downloads drawer — the three-pane anatomy documented in
//! `docs/reference/quixel-bridge/README.md`.
//!
//! Data comes from the live Fab catalog through [`CatalogWorker`]; when the
//! marketplace is unreachable (or the user picks it) the demo dataset stands in
//! so the shell stays reviewable offline.

use std::collections::{HashMap, HashSet};

use assetcraft_core::{AssetId, Config};

use crate::data::{self, Card, Filters, View};
use crate::worker::{CatalogWorker, Command, Event};
use crate::{grid, icons, theme};

/// Resolution choices for the demo download flow.
const RESOLUTIONS: [&str; 4] = ["2K", "4K", "8K", "16K"];
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

/// A download in the drawer.
pub struct Job {
    pub name: String,
    pub progress: f32,
    pub speed_mbps: f32,
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
            resolution: 1,
            jobs: Vec::new(),
            last_time: None,
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

        app.start_search(&cc.egui_ctx);
        app
    }

    // ----- data flow -------------------------------------------------------

    /// Kick off a fresh search for the current query and filters.
    fn start_search(&mut self, ctx: &egui::Context) {
        self.generation += 1;
        self.error = None;
        self.next_cursor = None;
        self.total = None;
        if self.source == Source::Demo {
            self.loading = false;
            self.cards = data::demo_cards();
            self.selected = None;
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

    pub fn can_load_more(&self) -> bool {
        self.source == Source::Live && self.next_cursor.is_some() && !self.loading
    }

    pub fn load_more(&mut self, ctx: &egui::Context) {
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
                Event::Thumbnail { id, texture } => {
                    self.textures.insert(id, texture);
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
        self.start_search(ctx);
    }

    fn hits(&self) -> Vec<usize> {
        let mut hits = data::visible(&self.cards, self.view, &self.favorites, &self.local);
        if self.source == Source::Demo {
            // Demo data is filtered locally; live data is filtered by the API.
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
                    View::Local => Some(self.local.len()),
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
        if self.source != Source::Live {
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

    pub fn toggle_favorite(&mut self, id: &str) {
        if !self.favorites.remove(id) {
            self.favorites.insert(id.to_owned());
        }
    }

    pub fn toggle_checked(&mut self, id: &str) {
        if !self.checked.remove(id) {
            self.checked.insert(id.to_owned());
        }
    }

    /// Downloads need an Epic session, which lands with the library milestone,
    /// so only the demo catalog can queue one for now.
    pub fn download_enabled(&self) -> bool {
        self.source == Source::Demo
    }

    pub fn start_download(&mut self, i: usize) {
        let Some(card) = self.cards.get(i) else {
            return;
        };
        if self.jobs.iter().any(|job| job.name == card.title) {
            return;
        }
        self.jobs.push(Job {
            name: card.title.clone(),
            progress: 0.0,
            speed_mbps: 18.0 + (i % 9) as f32,
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
        for job in &mut self.jobs {
            job.progress = (job.progress + dt as f32 * 0.18).min(1.0);
        }
        let finished: Vec<String> = self
            .jobs
            .iter()
            .filter(|job| job.progress >= 1.0)
            .map(|job| job.name.clone())
            .collect();
        for name in finished {
            if let Some(card) = self.cards.iter().find(|card| card.title == name) {
                self.local.insert(card.id.clone());
            }
            self.jobs.retain(|job| job.name != name);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
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
        for (view, count) in counts {
            if nav_row(ui, self.view == view, view.label(), count).clicked() {
                self.view = view;
                self.checked.clear();
                self.start_search(ctx);
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

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| if ui.button("\u{2699}  Settings").clicked() {});
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
        });
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
        let (id, title, seller, rating, listing_type, hue, tags, is_new) = {
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
            )
        };
        self.ensure_thumbnail(i);
        let description = self
            .details
            .get(&id)
            .and_then(|detail| detail.description.clone());
        let published = self
            .details
            .get(&id)
            .and_then(|detail| detail.created_at.clone());
        let favorite = self.favorites.contains(&id);
        let live = self.source == Source::Live;
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
                            let mut line = seller.unwrap_or_else(|| "Unknown seller".to_owned());
                            if let Some(rating) = rating {
                                line.push_str(&format!("  \u{00b7}  Rating {rating:.1}"));
                            }
                            ui.label(egui::RichText::new(line).size(12.0));

                            ui.add_space(4.0);
                            for (key, value) in [
                                ("Type", data::listing_label(listing_type.as_deref())),
                                (
                                    "Published",
                                    published
                                        .map(|raw| short_date(&raw))
                                        .unwrap_or_else(|| "\u{2014}".to_owned()),
                                ),
                            ] {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(key).size(11.0).color(theme::FG_MUTED),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(value)
                                                    .size(12.0)
                                                    .color(theme::FG_SECONDARY),
                                            );
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
                                let body = description.as_deref().unwrap_or(demo_note);
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

                            if live {
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
            ui.horizontal(|ui| {
                if !live {
                    egui::ComboBox::from_id_salt("resolution")
                        .width(112.0)
                        .selected_text(RESOLUTIONS[self.resolution])
                        .show_ui(ui, |ui| {
                            for (index, label) in RESOLUTIONS.iter().enumerate() {
                                ui.selectable_value(&mut self.resolution, index, *label);
                            }
                        });
                }
                let button = egui::Button::new(
                    egui::RichText::new("Download")
                        .strong()
                        .color(egui::Color32::WHITE),
                )
                .min_size(egui::vec2(if live { 132.0 } else { 0.0 }, 30.0))
                .fill(if live {
                    theme::GREEN.gamma_multiply(0.35)
                } else {
                    theme::GREEN
                });
                let response = ui.add_enabled(!live, button);
                if live {
                    response.on_disabled_hover_text(
                        "Sign in to Epic to download assets (library milestone)",
                    );
                } else if response.clicked() {
                    self.start_download(i);
                }
            });
        });
    }

    fn downloads(&mut self, ui: &mut egui::Ui) {
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
                            .animate(job.progress < 1.0),
                    );
                    ui.label(format!("{:.0} MB/s", job.speed_mbps));
                });
            }
        });
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

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::same(14)),
            )
            .show(ctx, |ui| {
                self.error_banner(ui, ctx);
                if self.error.is_some() && self.cards.is_empty() {
                    return;
                }

                if self.view.needs_account() && self.source == Source::Live {
                    let what = match self.view {
                        View::Local => "the local library",
                        _ => self.view.label(),
                    };
                    ui.centered_and_justified(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{what} needs an Epic account"))
                                    .size(15.0)
                                    .color(theme::FG_SECONDARY),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "Signing in and the owned library arrive with the \
                                     account milestone; switch to demo data to explore the shell.",
                                )
                                .size(12.0)
                                .color(theme::FG_MUTED),
                            );
                        });
                    });
                    return;
                }

                if self.loading && self.cards.is_empty() {
                    ui.centered_and_justified(|ui| ui.spinner());
                    return;
                }

                if hits.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new(if self.cards.is_empty() {
                                "The catalog returned no listings."
                            } else {
                                "Nothing matches this view, search, and filters."
                            })
                            .color(theme::FG_MUTED),
                        );
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
    use super::{short_date, strip_tags};

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
