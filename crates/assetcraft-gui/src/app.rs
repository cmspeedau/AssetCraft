//! The application shell: left nav, search + filter bar, grid, asset detail
//! panel, and the downloads drawer — the three-pane anatomy documented in
//! `docs/reference/quixel-bridge/README.md`.

use std::collections::HashSet;

use crate::data::{self, Asset, Filters, Job, View};
use crate::{grid, icons, theme};

const RESOLUTIONS: [&str; 4] = ["2K", "4K", "8K", "16K"];

pub struct App {
    pub assets: Vec<Asset>,
    pub view: View,
    pub filters: Filters,
    pub query: String,
    pub selected: Option<usize>,
    pub checked: HashSet<usize>,
    pub resolution: usize,
    pub jobs: Vec<Job>,
    pub detail_tab: usize,
    last_time: Option<f64>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            assets: data::mock_assets(),
            view: View::Home,
            filters: Filters::default(),
            query: String::new(),
            selected: Some(0),
            checked: HashSet::new(),
            resolution: 1,
            jobs: Vec::new(),
            detail_tab: 0,
            last_time: None,
        }
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        Self::default()
    }

    pub fn toggle_favorite(&mut self, i: usize) {
        self.assets[i].favorite = !self.assets[i].favorite;
    }

    pub fn toggle_checked(&mut self, i: usize) {
        if !self.checked.remove(&i) {
            self.checked.insert(i);
        }
    }

    fn start_download(&mut self, i: usize) {
        if self.jobs.iter().any(|j| j.name == self.assets[i].name) {
            return;
        }
        self.jobs.push(Job {
            name: self.assets[i].name,
            progress: 0.0,
            speed_mbps: 18.0 + (i % 9) as f32,
        });
    }

    fn tick_jobs(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let dt = self
            .last_time
            .replace(now)
            .map_or(0.0, |t| (now - t).max(0.0));
        if self.jobs.is_empty() {
            return;
        }
        for job in &mut self.jobs {
            job.progress = (job.progress + dt as f32 * 0.18).min(1.0);
        }
        let done: Vec<&str> = self
            .jobs
            .iter()
            .filter(|j| j.progress >= 1.0)
            .map(|j| j.name)
            .collect();
        for name in done {
            if let Some(asset) = self.assets.iter_mut().find(|a| a.name == name) {
                asset.downloaded = true;
            }
            self.jobs.retain(|j| j.name != name);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
    }

    fn nav(&mut self, ui: &mut egui::Ui) {
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
        ui.add_space(10.0);

        for (view, count) in data::counts(&self.assets) {
            let active = self.view == view;
            if nav_row(ui, active, view.label(), Some(count)).clicked() {
                self.view = view;
                self.checked.clear();
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("CATEGORIES")
                .size(11.0)
                .color(theme::FG_MUTED),
        );
        ui.add_space(2.0);
        for category in data::Category::ALL {
            let active = self.filters.category == Some(category);
            if nav_row(ui, active, category.label(), None).clicked() {
                self.filters.category = if active { None } else { Some(category) };
            }
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| if ui.button("\u{2699}  Settings").clicked() {});
            ui.label(
                egui::RichText::new("assetcraft.local")
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
                                .hint_text(format!("Search {} assets", self.assets.len()))
                                .desired_width(360.0),
                        );
                    })
                });
                ui.scope(|ui| {
                    if !self.query.is_empty()
                        && ui
                            .small_button(format!("{} \u{00d7}", self.query))
                            .clicked()
                    {
                        self.query.clear();
                    }
                });
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
                });
            });
        });
    }

    fn filter_bar(&mut self, ui: &mut egui::Ui) {
        let frame = egui::Frame::new()
            .fill(theme::PANEL)
            .inner_margin(egui::Margin::symmetric(12, 6));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                dropdown(
                    ui,
                    "Type",
                    &data::Category::ALL.map(|c| (c.label(), c)),
                    &mut self.filters.category,
                );
                dropdown(
                    ui,
                    "State",
                    &data::State::ALL.map(|s| (s.label(), s)),
                    &mut self.filters.state,
                );

                for env in ["Forest", "Desert", "Coast", "Urban"] {
                    if selectable(ui, self.filters.environment == Some(env), env, None).clicked() {
                        self.filters.environment = if self.filters.environment == Some(env) {
                            None
                        } else {
                            Some(env)
                        };
                    }
                }
                if self.filters != Filters::default() && ui.small_button("Clear filters").clicked()
                {
                    self.filters = Filters::default();
                }
            });
        });
    }

    fn detail(&mut self, ui: &mut egui::Ui, i: usize) {
        // Snapshot the fields the panel shows so the closures below can take
        // `&mut self` without holding a borrow of `self.assets`.
        let asset = &self.assets[i];
        let name = asset.name;
        let hue = asset.hue;
        let favorite = asset.favorite;
        let downloaded = asset.downloaded;
        let meta = asset.meta_line();
        let environment = asset.environment;
        let state = asset.state.label();
        let category = asset.category.label();
        let related: Vec<(usize, &'static str, f32)> = (0..self.assets.len())
            .filter(|&j| j != i && self.assets[j].category == asset.category)
            .take(4)
            .map(|j| (j, self.assets[j].name, self.assets[j].hue))
            .collect();
        let related_count = related.len();

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
                        self.assets[i].favorite = !favorite;
                    }
                });
            });

            let bar_height = 42.0;
            let list_height = (ui.available_height() - bar_height).max(120.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), list_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 210.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(rect, 8.0, theme::BG);
                            grid::draw_sphere(ui.painter(), rect, hue, downloaded);

                            ui.horizontal(|ui| {
                                for (tab, label) in ["Renders", "Maps", "3D"].iter().enumerate() {
                                    if selectable(ui, self.detail_tab == tab, label, None).clicked()
                                    {
                                        self.detail_tab = tab;
                                    }
                                }
                            });

                            if self.detail_tab == 1 {
                                for (n, map) in [
                                    "Albedo",
                                    "Normal",
                                    "Roughness",
                                    "Ambient Occlusion",
                                    "Displacement",
                                ]
                                .iter()
                                .enumerate()
                                {
                                    ui.horizontal(|ui| {
                                        let (swatch, _) = ui.allocate_exact_size(
                                            egui::vec2(34.0, 34.0),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().rect_filled(
                                            swatch,
                                            4.0,
                                            theme::hue_color((hue + n as f32 * 27.0) % 360.0),
                                        );
                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new(*map)
                                                    .size(12.0)
                                                    .color(theme::FG_SECONDARY),
                                            );
                                            ui.label(
                                                egui::RichText::new("4096 x 4096  PNG")
                                                    .size(10.0)
                                                    .color(theme::FG_MUTED),
                                            );
                                        });
                                    });
                                }
                            } else if self.detail_tab == 2 {
                                for (key, value) in [
                                    ("Format", "FBX / glTF"),
                                    ("Vertices", "12,840"),
                                    ("Triangles", "25,120"),
                                    ("LODs", "4"),
                                    ("Collision", "Yes"),
                                ] {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(key)
                                                .size(11.0)
                                                .color(theme::FG_MUTED),
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
                            }

                            ui.label(
                                egui::RichText::new(name)
                                    .size(19.0)
                                    .strong()
                                    .color(theme::FG),
                            );
                            ui.label(meta);
                            ui.label(format!("Environment: {environment}"));

                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new("TAGS")
                                    .size(11.0)
                                    .color(theme::FG_MUTED),
                            );
                            ui.horizontal_wrapped(|ui| {
                                for tag in [state, environment, category] {
                                    chip(ui, tag);
                                }
                            });

                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new("RELATED ASSETS")
                                    .size(11.0)
                                    .color(theme::FG_MUTED),
                            );
                            if related_count == 0 {
                                ui.label("No related assets.");
                            }
                            let gap = ui.spacing().item_spacing.x;
                            let mini = ((ui.available_width() - gap * 3.0) / 4.0).min(84.0);
                            ui.horizontal(|ui| {
                                for (j, related_name, related_hue) in &related {
                                    let (rect, resp) = ui.allocate_exact_size(
                                        egui::vec2(mini, mini),
                                        egui::Sense::click(),
                                    );
                                    ui.painter().rect_filled(rect, 6.0, theme::BG);
                                    grid::draw_sphere(ui.painter(), rect, *related_hue, false);
                                    if resp.clicked() {
                                        self.selected = Some(*j);
                                    }
                                    resp.on_hover_text(*related_name);
                                }
                            });
                        });
                },
            );

            ui.separator();
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("resolution")
                    .width(112.0)
                    .selected_text(RESOLUTIONS[self.resolution])
                    .show_ui(ui, |ui| {
                        for (r, label) in RESOLUTIONS.iter().enumerate() {
                            ui.selectable_value(&mut self.resolution, r, *label);
                        }
                    });
                let label = if downloaded { "Downloaded" } else { "Download" };
                let button = egui::Button::new(
                    egui::RichText::new(label)
                        .strong()
                        .color(egui::Color32::WHITE),
                )
                .min_size(egui::vec2(0.0, 30.0))
                .fill(if downloaded {
                    theme::GREEN.gamma_multiply(0.55)
                } else {
                    theme::GREEN
                });
                if ui.add(button).clicked() && !downloaded {
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
                    ui.label(egui::RichText::new(job.name).size(12.0));
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
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick_jobs(ctx);
        let hits = data::visible(&self.assets, self.view, &self.filters, &self.query);

        if !self.jobs.is_empty() {
            egui::TopBottomPanel::bottom("downloads").show(ctx, |ui| self.downloads(ui));
        }
        egui::TopBottomPanel::top("search").show(ctx, |ui| self.search_bar(ui));
        egui::TopBottomPanel::top("filters").show(ctx, |ui| self.filter_bar(ui));
        egui::SidePanel::left("nav")
            .exact_width(232.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(10, 12)),
            )
            .show(ctx, |ui| self.nav(ui));
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
                if !self.checked.is_empty() {
                    let checked: Vec<usize> = self.checked.iter().copied().collect();
                    ui.horizontal(|ui| {
                        ui.label(format!("{} assets selected", checked.len()));
                        let button = egui::Button::new(
                            egui::RichText::new("Download all")
                                .strong()
                                .color(egui::Color32::WHITE),
                        )
                        .fill(theme::GREEN);
                        if ui.add(button).clicked() {
                            for i in checked {
                                self.start_download(i);
                            }
                            self.checked.clear();
                        }
                        if ui.small_button("Clear").clicked() {
                            self.checked.clear();
                        }
                    });
                    ui.separator();
                }
                if hits.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label("No assets match your search and filters.");
                    });
                } else {
                    grid::show(ui, self, &hits);
                }
            });
    }
}

/// A full-width nav row: label on the left, count on the right, and a
/// rounded highlight when active or hovered, as in Bridge's left rail.
fn nav_row(ui: &mut egui::Ui, active: bool, text: &str, count: Option<usize>) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, 6.0, theme::ELEVATED);
    } else if resp.hovered() {
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
    resp
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
    let sl = egui::Button::selectable(active, label);
    let resp = ui.add(sl);
    if let Some(count) = count {
        ui.label(
            egui::RichText::new(count.to_string())
                .size(11.0)
                .color(theme::FG_MUTED),
        );
    }
    resp
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
        .find(|(_, v)| slot.as_ref() == Some(v))
        .map(|(label, _)| *label);
    let response = egui::ComboBox::from_id_salt(title)
        .width(92.0)
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

    // egui draws no affordance on a ComboBox, and the fonts have no `▾`, so
    // paint the chevron ourselves.
    let c = response.rect.right_center() + egui::vec2(-10.0, 0.0);
    let stroke = egui::Stroke::new(1.2_f32, theme::FG_MUTED);
    let painter = ui.painter();
    painter.line_segment(
        [c + egui::vec2(-3.5, -2.0), c + egui::vec2(0.0, 1.8)],
        stroke,
    );
    painter.line_segment(
        [c + egui::vec2(3.5, -2.0), c + egui::vec2(0.0, 1.8)],
        stroke,
    );
}
