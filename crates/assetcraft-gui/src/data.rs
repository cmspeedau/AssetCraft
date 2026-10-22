//! The catalog the shell browses.
//!
//! M5's first slice runs on a deterministic mock so the UI can be built and
//! reviewed before the live `assetcraft-catalog` client is wired in; the
//! shapes here (`Asset`, `Filters`, `View`) are what the real client will
//! later fill.

/// What kind of thing an asset is, mirroring the catalog taxonomy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Assets3D,
    Plants,
    Surfaces,
    Decals,
}

impl Category {
    pub const ALL: [Category; 4] = [
        Category::Assets3D,
        Category::Plants,
        Category::Surfaces,
        Category::Decals,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Assets3D => "3D Assets",
            Category::Plants => "3D Plants",
            Category::Surfaces => "Surfaces",
            Category::Decals => "Decals",
        }
    }
}

/// How weathered a scan is; the catalog's `state` facet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    New,
    Weathered,
    Damaged,
}

impl State {
    pub const ALL: [State; 3] = [State::New, State::Weathered, State::Damaged];

    pub fn label(self) -> &'static str {
        match self {
            State::New => "New",
            State::Weathered => "Weathered",
            State::Damaged => "Damaged",
        }
    }
}

/// One browsable asset.
#[derive(Clone, Debug)]
pub struct Asset {
    pub id: &'static str,
    pub name: &'static str,
    pub category: Category,
    pub state: State,
    pub environment: &'static str,
    pub size_mb: u32,
    pub free: bool,
    pub is_new: bool,
    pub favorite: bool,
    pub downloaded: bool,
    /// Hue in degrees driving the generated thumbnail.
    pub hue: f32,
}

impl Asset {
    /// Meta line under the detail preview: `Surfaces · Weathered · 820 MB`.
    pub fn meta_line(&self) -> String {
        format!(
            "{} \u{00b7} {} \u{00b7} {} MB",
            self.category.label(),
            self.state.label(),
            self.size_mb
        )
    }
}

/// The active nav tab.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Home,
    Collections,
    Free,
    Purchased,
    Favorites,
    Local,
}

impl View {
    pub fn label(self) -> &'static str {
        match self {
            View::Home => "Home",
            View::Collections => "Collections",
            View::Free => "Free",
            View::Purchased => "Purchased",
            View::Favorites => "Favorites",
            View::Local => "Local",
        }
    }
}

/// The dropdown filter bar. `None` means "All".
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Filters {
    pub category: Option<Category>,
    pub state: Option<State>,
    pub environment: Option<&'static str>,
}

/// The visible list for a view + filters + search text.
pub fn visible(assets: &[Asset], view: View, filters: &Filters, query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    assets
        .iter()
        .enumerate()
        .filter(|(_, a)| match view {
            View::Home | View::Collections => true,
            View::Free => a.free,
            View::Purchased => !a.free,
            View::Favorites => a.favorite,
            View::Local => a.downloaded,
        })
        .filter(|(_, a)| filters.category.is_none_or(|c| a.category == c))
        .filter(|(_, a)| filters.state.is_none_or(|s| a.state == s))
        .filter(|(_, a)| {
            filters
                .environment
                .is_none_or(|e| a.environment.eq_ignore_ascii_case(e))
        })
        .filter(|(_, a)| query.is_empty() || a.name.to_lowercase().contains(&query))
        .map(|(i, _)| i)
        .collect()
}

/// Counts shown next to each nav tab.
pub fn counts(assets: &[Asset]) -> Vec<(View, usize)> {
    let count = |p: fn(&Asset) -> bool| assets.iter().filter(|a| p(a)).count();
    vec![
        (View::Home, assets.len()),
        (View::Collections, 12),
        (View::Free, count(|a| a.free)),
        (View::Purchased, count(|a| !a.free)),
        (View::Favorites, count(|a| a.favorite)),
        (View::Local, count(|a| a.downloaded)),
    ]
}

/// A download in the drawer.
pub struct Job {
    pub name: &'static str,
    pub progress: f32,
    pub speed_mbps: f32,
}

/// The mock catalog, stable across runs so screenshots compare.
pub fn mock_assets() -> Vec<Asset> {
    const NAMES: &[(&str, &str, Category, State, &str)] = &[
        (
            "A",
            "Flagstone Floor 2x2 M",
            Category::Surfaces,
            State::Weathered,
            "Urban",
        ),
        (
            "B",
            "Mossy Boulder Cluster",
            Category::Assets3D,
            State::Weathered,
            "Forest",
        ),
        (
            "C",
            "Cracked Asphalt Patch",
            Category::Surfaces,
            State::Damaged,
            "Urban",
        ),
        (
            "D",
            "Coastal Cliff Rock",
            Category::Assets3D,
            State::New,
            "Coast",
        ),
        (
            "E",
            "Palm Frond Group",
            Category::Plants,
            State::New,
            "Coast",
        ),
        (
            "F",
            "Rust Decal Set",
            Category::Decals,
            State::Damaged,
            "Urban",
        ),
        (
            "G",
            "Desert Dune Sand",
            Category::Surfaces,
            State::New,
            "Desert",
        ),
        (
            "H",
            "Wooden Chest",
            Category::Assets3D,
            State::Weathered,
            "Urban",
        ),
        ("I", "Fern Cluster", Category::Plants, State::New, "Forest"),
        (
            "J",
            "Graffiti Decals",
            Category::Decals,
            State::New,
            "Urban",
        ),
        (
            "K",
            "Sandy Gravel Ground",
            Category::Surfaces,
            State::New,
            "Desert",
        ),
        (
            "L",
            "Granite Block",
            Category::Assets3D,
            State::Damaged,
            "Forest",
        ),
        (
            "M",
            "Ivy Vine",
            Category::Plants,
            State::Weathered,
            "Forest",
        ),
        (
            "N",
            "Peeling Paint Decal",
            Category::Decals,
            State::Damaged,
            "Urban",
        ),
        ("O", "Coral Rubble", Category::Assets3D, State::New, "Coast"),
        (
            "P",
            "Sandstone Wall",
            Category::Surfaces,
            State::Weathered,
            "Desert",
        ),
        ("Q", "Seagrass Patch", Category::Plants, State::New, "Coast"),
        ("R", "Mud Decals", Category::Decals, State::New, "Forest"),
        (
            "S",
            "Basalt Cobble",
            Category::Surfaces,
            State::Damaged,
            "Coast",
        ),
        ("T", "Pine Sapling", Category::Plants, State::New, "Forest"),
        (
            "U",
            "Barrel Crate Stack",
            Category::Assets3D,
            State::Weathered,
            "Urban",
        ),
        (
            "V",
            "Tire Tracks Decal",
            Category::Decals,
            State::Weathered,
            "Desert",
        ),
    ];

    NAMES
        .iter()
        .enumerate()
        .map(|(i, (id, name, category, state, environment))| Asset {
            id,
            name,
            category: *category,
            state: *state,
            environment,
            size_mb: 120 + (i as u32 * 137) % 1400,
            free: i % 3 != 0,
            is_new: i % 5 < 2,
            favorite: i % 7 == 0,
            downloaded: i % 6 == 0,
            hue: (i as f32 * 47.0) % 360.0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_view_tabs_partition_the_catalog() {
        let assets = mock_assets();
        let all = visible(&assets, View::Home, &Filters::default(), "");
        assert_eq!(all.len(), assets.len());
        let local = visible(&assets, View::Local, &Filters::default(), "");
        assert!(local.len() < all.len());
        assert!(local.iter().all(|&i| assets[i].downloaded));
    }

    #[test]
    fn search_and_filters_narrow_the_grid() {
        let assets = mock_assets();
        let hits = visible(&assets, View::Home, &Filters::default(), "rock");
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|&i| assets[i].name.contains("Rock")));

        let surfaces = visible(
            &assets,
            View::Home,
            &Filters {
                category: Some(Category::Surfaces),
                ..Default::default()
            },
            "",
        );
        assert!(surfaces
            .iter()
            .all(|&i| assets[i].category == Category::Surfaces));
    }
}
