//! The catalog view model.
//!
//! The grid renders [`Card`]s, whether they came from a live Fab search or
//! from the demo dataset used when the API is unreachable. Keeping one shape
//! for both means the UI never branches on the source.

use assetcraft_core::{AssetSummary, LibraryItem, SearchQuery};

/// How many listings a search page asks for.
pub const PAGE_SIZE: u32 = 60;

/// One card in the grid.
#[derive(Clone, Debug)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub seller: Option<String>,
    pub listing_type: Option<String>,
    pub free: bool,
    pub rating: Option<f64>,
    pub tags: Vec<String>,
    pub thumbnail: Option<String>,
    /// Stable hue in degrees for the procedural placeholder.
    pub hue: f32,
    /// Whether the listing appeared recently enough for a NEW pill.
    pub is_new: bool,
    /// Description carried by the record itself, when it has one.
    pub description: Option<String>,
}

impl Card {
    /// Project a locally held asset into the grid's card.
    pub fn from_summary(summary: &AssetSummary) -> Self {
        Self {
            id: summary.id.as_str().to_owned(),
            title: summary.display_title(),
            seller: summary.seller_name().map(str::to_owned),
            listing_type: summary.listing_type.clone(),
            free: summary.is_free,
            rating: summary.rating.as_ref().and_then(|r| r.average),
            tags: summary.tags.iter().filter_map(|t| t.name.clone()).collect(),
            thumbnail: summary.thumbnail_url.clone(),
            hue: hue_for(summary.id.as_str()),
            is_new: is_recent(summary.published_at.as_deref()),
            description: None,
        }
    }

    /// Project a library record into the same card the catalog uses.
    ///
    /// Library items carry no seller, rating, or price, so those stay empty;
    /// the category stands in for the listing type.
    pub fn from_library_item(item: &LibraryItem) -> Self {
        let listing_type = item
            .categories
            .first()
            .and_then(|category| category.name.clone().or_else(|| category.slug.clone()));
        Self {
            id: item.asset_id.clone(),
            title: item.title.clone(),
            seller: None,
            listing_type,
            free: false,
            rating: None,
            tags: item
                .categories
                .iter()
                .filter_map(|category| category.name.clone())
                .collect(),
            thumbnail: item.thumbnail_url().map(str::to_owned),
            hue: hue_for(&item.asset_id),
            is_new: false,
            description: item.description.clone(),
        }
    }
}

/// How long a listing stays "new", in days.
const NEW_WITHIN_DAYS: i64 = 60;

/// Whether a publication timestamp is recent enough to show a NEW pill.
fn is_recent(published_at: Option<&str>) -> bool {
    let Some(raw) = published_at else {
        return false;
    };
    let Ok(published) = chrono::DateTime::parse_from_rfc3339(raw) else {
        return false;
    };
    let cutoff = chrono::Utc::now() - chrono::Duration::days(NEW_WITHIN_DAYS);
    published.with_timezone(&chrono::Utc) > cutoff
}

/// A stable 0..360 hue for an id, so placeholders keep their colour across
/// restarts and don't all look alike.
pub fn hue_for(key: &str) -> f32 {
    let mut hash: u32 = 2_166_136_261;
    for byte in key.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    (hash % 360) as f32
}

/// The listing types the filter bar offers, as `(Fab slug, label)`.
pub const LISTING_TYPES: &[(&str, &str)] = &[
    ("3d-model", "3D Model"),
    ("3d-scene", "3D Scene"),
    ("3d-plant", "3D Plant"),
    ("material", "Material"),
    ("decal", "Decal"),
    ("tool-and-plugin", "Tool & Plugin"),
    ("audio", "Audio"),
    ("vfx", "VFX"),
];

/// Human label for a listing-type slug.
pub fn listing_label(slug: Option<&str>) -> String {
    let Some(slug) = slug else {
        return "Listing".to_owned();
    };
    LISTING_TYPES
        .iter()
        .find(|(code, _)| *code == slug)
        .map(|(_, label)| (*label).to_owned())
        .unwrap_or_else(|| {
            let mut chars = slug.replace('-', " ");
            if let Some(first) = chars.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            chars
        })
}

/// The active nav tab.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum View {
    #[default]
    Home,
    Collections,
    Free,
    Purchased,
    Favorites,
    Local,
}

impl View {
    pub const ALL: [View; 6] = [
        View::Home,
        View::Collections,
        View::Free,
        View::Purchased,
        View::Favorites,
        View::Local,
    ];

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

    /// Whether the view needs an Epic account, and so cannot be filled yet.
    pub fn needs_account(self) -> bool {
        matches!(self, View::Collections | View::Purchased)
    }
}

/// The filter bar's state. `None` means "all types".
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Filters {
    /// Index into [`LISTING_TYPES`].
    pub listing_type: Option<usize>,
}

impl Filters {
    pub fn listing_slug(&self) -> Option<&'static str> {
        self.listing_type.map(|i| LISTING_TYPES[i].0)
    }
}

/// Build the API query for the current search box and filters.
///
/// This is the one place UI state turns into protocol, so both "first page" and
/// "next page" go through it and stay in sync.
pub fn query_for(
    filters: &Filters,
    text: &str,
    cursor: Option<String>,
    free_only: bool,
) -> SearchQuery {
    let text = text.trim();
    let mut query = SearchQuery {
        text: (!text.is_empty()).then(|| text.to_owned()),
        free_only,
        limit: Some(PAGE_SIZE),
        cursor,
        ..Default::default()
    };
    if let Some(slug) = filters.listing_slug() {
        query.listing_types.push(slug.to_owned());
    }
    query
}

/// Indices of the cards a view shows, before any API-side filtering.
///
/// Favourites and the local library are client-side, so they filter here even
/// for live results.
pub fn visible(
    cards: &[Card],
    view: View,
    favorites: &std::collections::HashSet<String>,
    local: &std::collections::HashSet<String>,
) -> Vec<usize> {
    cards
        .iter()
        .enumerate()
        .filter(|(_, card)| match view {
            View::Home | View::Collections | View::Purchased => true,
            View::Free => card.free,
            View::Favorites => favorites.contains(&card.id),
            View::Local => local.contains(&card.id),
        })
        .map(|(i, _)| i)
        .collect()
}

/// The offline dataset: real-looking listings so the shell can be reviewed
/// without the marketplace, and so the demo path exercises the same cards.
pub fn demo_cards() -> Vec<Card> {
    /// Title, seller, listing type, free, rating, tags.
    type DemoRow = (
        &'static str,
        &'static str,
        &'static str,
        bool,
        f64,
        &'static [&'static str],
    );
    const ROWS: &[DemoRow] = &[
        (
            "Flagstone Floor 2x2 M",
            "Grim Wilder",
            "material",
            true,
            4.7,
            &["stone", "floor", "pbr"],
        ),
        (
            "Mossy Boulder Cluster",
            "Rock Forge",
            "3d-model",
            true,
            4.9,
            &["rock", "nature"],
        ),
        (
            "Cracked Asphalt Patch",
            "Urban Decay",
            "material",
            false,
            4.4,
            &["asphalt", "road"],
        ),
        (
            "Coastal Cliff Rock",
            "Rock Forge",
            "3d-model",
            false,
            4.8,
            &["cliff", "coast"],
        ),
        (
            "Palm Frond Group",
            "Greenhouse",
            "3d-plant",
            true,
            4.6,
            &["palm", "tropical"],
        ),
        (
            "Rust Decal Set",
            "Urban Decay",
            "decal",
            false,
            4.2,
            &["rust", "decal"],
        ),
        (
            "Desert Dune Sand",
            "Dune Works",
            "material",
            true,
            4.5,
            &["sand", "desert"],
        ),
        (
            "Wooden Chest",
            "Prop Shop",
            "3d-model",
            true,
            4.9,
            &["chest", "wood"],
        ),
        (
            "Fern Cluster",
            "Greenhouse",
            "3d-plant",
            false,
            4.7,
            &["fern", "forest"],
        ),
        (
            "Graffiti Decals",
            "Street Layer",
            "decal",
            true,
            4.1,
            &["graffiti", "urban"],
        ),
        (
            "Sandy Gravel Ground",
            "Dune Works",
            "material",
            false,
            4.3,
            &["gravel", "ground"],
        ),
        (
            "Granite Block",
            "Rock Forge",
            "3d-model",
            false,
            4.6,
            &["granite", "stone"],
        ),
        (
            "Ivy Vine",
            "Greenhouse",
            "3d-plant",
            false,
            4.8,
            &["ivy", "vine"],
        ),
        (
            "Peeling Paint Decal",
            "Urban Decay",
            "decal",
            true,
            4.0,
            &["paint", "grunge"],
        ),
        (
            "Coral Rubble",
            "Reef Lab",
            "3d-model",
            true,
            4.4,
            &["coral", "sea"],
        ),
        (
            "Sandstone Wall",
            "Dune Works",
            "material",
            false,
            4.7,
            &["sandstone", "wall"],
        ),
        (
            "Seagrass Patch",
            "Reef Lab",
            "3d-plant",
            true,
            4.5,
            &["grass", "coast"],
        ),
        (
            "Mud Decals",
            "Street Layer",
            "decal",
            false,
            4.2,
            &["mud", "decal"],
        ),
        (
            "Basalt Cobble",
            "Rock Forge",
            "material",
            false,
            4.8,
            &["basalt", "cobble"],
        ),
        (
            "Pine Sapling",
            "Greenhouse",
            "3d-plant",
            true,
            4.6,
            &["pine", "tree"],
        ),
        (
            "Barrel Crate Stack",
            "Prop Shop",
            "3d-model",
            false,
            4.3,
            &["crate", "barrel"],
        ),
        (
            "Tire Tracks Decal",
            "Street Layer",
            "decal",
            true,
            3.9,
            &["tracks", "dirt"],
        ),
    ];

    ROWS.iter()
        .enumerate()
        .map(
            |(n, (title, seller, listing_type, free, rating, tags))| Card {
                id: format!("demo-{n:02}"),
                title: (*title).to_owned(),
                seller: Some((*seller).to_owned()),
                listing_type: Some((*listing_type).to_owned()),
                free: *free,
                rating: Some(*rating),
                tags: tags.iter().map(|t| (*t).to_owned()).collect(),
                thumbnail: None,
                hue: hue_for(title),
                is_new: n % 5 < 2,
                description: None,
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_carries_text_type_and_free_flag() {
        let filters = Filters {
            listing_type: Some(0),
        };
        let query = query_for(&filters, "  mossy  ", None, true);
        assert_eq!(query.text.as_deref(), Some("mossy"));
        assert_eq!(query.listing_types, vec!["3d-model".to_owned()]);
        assert!(query.free_only);
        assert_eq!(query.limit, Some(PAGE_SIZE));
        assert!(query.cursor.is_none());
    }

    #[test]
    fn a_blank_search_sends_no_text() {
        let query = query_for(&Filters::default(), "   ", None, false);
        assert!(query.text.is_none());
        assert!(query.listing_types.is_empty());
        assert!(!query.free_only);
    }

    #[test]
    fn hue_is_stable_and_in_range() {
        let first = hue_for("abc-123");
        assert_eq!(first, hue_for("abc-123"));
        assert_ne!(first, hue_for("abc-124"));
        assert!((0.0..360.0).contains(&first));
    }

    #[test]
    fn library_items_map_their_categories_and_images() {
        use assetcraft_core::{AssetImage, Category};

        let item = LibraryItem {
            asset_id: "fab-1".to_owned(),
            namespace: "fab".to_owned(),
            title: "Mossy Rock".to_owned(),
            description: Some("A mossy rock.".to_owned()),
            url: None,
            source: Some("fab".to_owned()),
            distribution_method: None,
            categories: vec![Category {
                name: Some("3D Models".to_owned()),
                ..Default::default()
            }],
            images: vec![AssetImage {
                url: Some("http://example.test/1.png".to_owned()),
                kind: Some("thumbnail".to_owned()),
                ..Default::default()
            }],
        };

        let card = Card::from_library_item(&item);
        assert_eq!(card.id, "fab-1");
        assert_eq!(card.title, "Mossy Rock");
        assert_eq!(card.listing_type.as_deref(), Some("3D Models"));
        assert_eq!(card.thumbnail.as_deref(), Some("http://example.test/1.png"));
        assert_eq!(card.description.as_deref(), Some("A mossy rock."));
        assert_eq!(card.tags, vec!["3D Models".to_owned()]);
        assert!(card.seller.is_none());
        assert!(!card.free);
    }

    #[test]
    fn listing_labels_fall_back_to_the_slug() {
        assert_eq!(listing_label(Some("3d-model")), "3D Model");
        assert_eq!(listing_label(Some("custom-kind")), "Custom kind");
        assert_eq!(listing_label(None), "Listing");
    }

    #[test]
    fn views_filter_client_side_sets() {
        let cards = demo_cards();
        let favorites = std::collections::HashSet::from([cards[0].id.clone()]);
        let local = std::collections::HashSet::new();
        assert_eq!(
            visible(&cards, View::Favorites, &favorites, &local),
            vec![0]
        );
        assert!(visible(&cards, View::Local, &favorites, &local).is_empty());
        let free = visible(&cards, View::Free, &favorites, &local);
        assert!(free.iter().all(|&i| cards[i].free));
        assert_eq!(
            visible(&cards, View::Home, &favorites, &local).len(),
            cards.len()
        );
    }
}
