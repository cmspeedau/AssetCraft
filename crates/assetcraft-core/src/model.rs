//! Marketplace-agnostic domain model.
//!
//! These types are deliberately decoupled from any single API's wire format so
//! that the catalog, download, and DCC layers share one vocabulary. The catalog
//! crate is responsible for translating raw responses into these types.

use serde::{Deserialize, Serialize};

/// Stable identifier for an asset/listing on its source marketplace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AssetId(String);

impl AssetId {
    /// Wrap a raw marketplace id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the raw id.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the wrapper and return the raw id.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl From<String> for AssetId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for AssetId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl AsRef<str> for AssetId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The seller or publisher behind an asset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seller {
    /// Marketplace-internal seller id.
    pub id: Option<String>,
    /// Display name shown to users.
    pub name: Option<String>,
    /// Public profile URL, when the marketplace exposes one.
    pub profile_url: Option<String>,
}

/// A single category a listing belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    /// Category id.
    pub id: Option<String>,
    /// Human-readable category name.
    pub name: Option<String>,
    /// URL-friendly slug.
    pub slug: Option<String>,
    /// Hierarchical path, when the marketplace provides one.
    pub path: Option<String>,
}

/// A tag attached to a listing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    /// Tag id.
    pub id: Option<String>,
    /// Human-readable tag name.
    pub name: Option<String>,
    /// URL-friendly slug.
    pub slug: Option<String>,
}

/// Aggregate rating for an asset.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Rating {
    /// Mean rating out of five.
    pub average: Option<f64>,
    /// Number of ratings contributing to the mean.
    pub count: Option<u64>,
}

/// A short-form asset record, as returned by search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetSummary {
    /// Marketplace id.
    pub id: AssetId,
    /// Listing title.
    pub title: Option<String>,
    /// Marketplace listing type, e.g. `3d-model`, `tool-and-plugin`.
    pub listing_type: Option<String>,
    /// Seller, when known.
    pub seller: Option<Seller>,
    /// Primary category, when known.
    pub category: Option<Category>,
    /// Tags attached to the listing.
    pub tags: Vec<Tag>,
    /// Aggregate rating, when known.
    pub rating: Option<Rating>,
    /// Whether the asset is free.
    pub is_free: bool,
    /// Whether the listing is flagged mature.
    pub is_mature: bool,
    /// Publication timestamp, as reported by the source.
    pub published_at: Option<String>,
    /// Best available thumbnail URL.
    pub thumbnail_url: Option<String>,
}

impl AssetSummary {
    /// A human-friendly title, falling back to the raw id when absent.
    pub fn display_title(&self) -> String {
        match self.title.as_deref() {
            Some(title) if !title.trim().is_empty() => title.to_string(),
            _ => self.id.to_string(),
        }
    }

    /// The seller's display name, when known.
    pub fn seller_name(&self) -> Option<&str> {
        self.seller
            .as_ref()
            .and_then(|seller| seller.name.as_deref())
    }
}

/// Full asset record, as returned by a detail lookup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetDetail {
    /// The summary fields, always present.
    pub summary: AssetSummary,
    /// Long-form description.
    pub description: Option<String>,
    /// Number of reviews.
    pub review_count: Option<u64>,
    /// Creation timestamp, as reported by the source.
    pub created_at: Option<String>,
}

impl AssetDetail {
    /// The asset's id.
    pub fn id(&self) -> &AssetId {
        &self.summary.id
    }
}

/// One downloadable file within an [`AssetFormat`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetFile {
    /// File id used by the download endpoint.
    pub id: Option<String>,
    /// Display name of the file.
    pub name: Option<String>,
    /// Size in bytes, when reported.
    pub size_bytes: Option<u64>,
    /// Sub-type of the asset inside the file.
    pub asset_type: Option<String>,
    /// Platforms the file targets.
    pub platforms: Vec<String>,
    /// Engine versions the file supports.
    pub engine_versions: Vec<String>,
}

/// The kind of a downloadable format.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatType {
    /// Machine code, e.g. `unreal-engine`, `fbx`.
    pub code: Option<String>,
    /// Human-readable name.
    pub name: Option<String>,
    /// Icon identifier.
    pub icon: Option<String>,
}

/// A downloadable format (e.g. `unreal-engine`, `fbx`, `blender`) for an asset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetFormat {
    /// The kind of format.
    pub format_type: Option<FormatType>,
    /// Files available in this format.
    pub files: Vec<AssetFile>,
}

impl AssetFormat {
    /// The machine code of this format, when known.
    pub fn code(&self) -> Option<&str> {
        self.format_type.as_ref().and_then(|t| t.code.as_deref())
    }
}

/// A license offered for an asset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseType {
    /// Human-readable license name.
    pub name: Option<String>,
    /// URL-friendly slug.
    pub slug: Option<String>,
    /// Link to the license terms.
    pub url: Option<String>,
}

/// A named group of tags, used to drive filtering UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagGroup {
    /// Group id.
    pub id: Option<String>,
    /// Human-readable group name.
    pub name: Option<String>,
    /// URL-friendly slug.
    pub slug: Option<String>,
    /// Category the group belongs to.
    pub category: Option<String>,
    /// Display order hint.
    pub order: Option<i32>,
    /// Tags in the group.
    pub tags: Vec<Tag>,
}

/// An image attached to a library asset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetImage {
    /// Image kind, e.g. `thumbnail`.
    pub kind: Option<String>,
    /// Image URL.
    pub url: Option<String>,
    /// Pixel width, when reported.
    pub width: Option<u32>,
    /// Pixel height, when reported.
    pub height: Option<u32>,
    /// Content checksum, when reported.
    pub md5: Option<String>,
}

/// An asset the user owns, as reported by their library.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryItem {
    /// Asset id, unique within the namespace.
    pub asset_id: String,
    /// Namespace the asset lives in.
    pub namespace: String,
    /// Display title.
    pub title: String,
    /// Long-form description.
    pub description: Option<String>,
    /// Marketplace URL for the asset.
    pub url: Option<String>,
    /// Where the asset came from.
    pub source: Option<String>,
    /// How the asset is distributed.
    pub distribution_method: Option<String>,
    /// Categories the asset belongs to.
    pub categories: Vec<Category>,
    /// Images for the asset.
    pub images: Vec<AssetImage>,
}

impl LibraryItem {
    /// The best available image URL, preferring one typed `thumbnail`.
    pub fn thumbnail_url(&self) -> Option<&str> {
        self.images
            .iter()
            .find(|image| image.kind.as_deref() == Some("thumbnail"))
            .or_else(|| self.images.first())
            .and_then(|image| image.url.as_deref())
    }
}

/// One place a download manifest can be fetched from, with its signature expiry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionPoint {
    /// URL of the manifest.
    pub manifest_url: String,
    /// When the signed URL stops working, as reported by the source.
    pub signature_expiration: Option<String>,
}

/// Where and how one file of one format can be downloaded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadInfo {
    /// Artifact id needed to request the manifest.
    pub artifact_id: String,
    /// Format this download provides.
    pub asset_format: String,
    /// Build version, when reported.
    pub build_version: Option<String>,
    /// Signed distribution points.
    pub distribution_points: Vec<DistributionPoint>,
}

/// A page of results plus the cursor needed to fetch the next one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    /// The items on this page.
    pub items: Vec<T>,
    /// Total number of matching items, when the source reports it.
    pub total: Option<u64>,
    /// Opaque cursor for the next page, when there is one.
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    /// Build a page from its parts.
    pub fn new(items: Vec<T>, total: Option<u64>, next_cursor: Option<String>) -> Self {
        Self {
            items,
            total,
            next_cursor,
        }
    }

    /// Whether this page has no items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Whether another page can be requested.
    pub fn has_more(&self) -> bool {
        self.next_cursor.is_some()
    }
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total: None,
            next_cursor: None,
        }
    }
}

/// How to order search results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortOrder {
    /// Best match first.
    Relevance,
    /// Most recently published first.
    Newest,
    /// Oldest published first.
    Oldest,
    /// Cheapest first.
    PriceLowToHigh,
    /// Most expensive first.
    PriceHighToLow,
}

impl SortOrder {
    /// The value the source API expects.
    pub const fn as_str(self) -> &'static str {
        match self {
            SortOrder::Relevance => "-relevance",
            SortOrder::Newest => "-createdAt",
            SortOrder::Oldest => "createdAt",
            SortOrder::PriceLowToHigh => "price",
            SortOrder::PriceHighToLow => "-price",
        }
    }
}

/// Filters for a catalog search.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Free-text search terms.
    pub text: Option<String>,
    /// Restrict to channels, e.g. `unreal-engine`.
    pub channels: Vec<String>,
    /// Restrict to listing types, e.g. `3d-model`.
    pub listing_types: Vec<String>,
    /// Restrict to categories (slugs).
    pub categories: Vec<String>,
    /// Restrict to a seller.
    pub seller: Option<String>,
    /// Result ordering.
    pub sort_by: Option<SortOrder>,
    /// Maximum number of items to return.
    pub limit: Option<u32>,
    /// Pagination cursor from a previous page.
    pub cursor: Option<String>,
    /// Only free assets.
    pub free_only: bool,
    /// Only discounted assets.
    pub discounted_only: bool,
    /// Only assets discounted by at least this percentage.
    pub min_discount_percentage: Option<u32>,
}

impl SearchQuery {
    /// A query matching `text`.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            ..Default::default()
        }
    }

    /// Set the page size.
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Restrict to a listing type.
    pub fn with_listing_type(mut self, listing_type: impl Into<String>) -> Self {
        self.listing_types.push(listing_type.into());
        self
    }

    /// Continue from a previous page's cursor.
    pub fn with_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_title_falls_back_to_id() {
        let summary = AssetSummary {
            id: AssetId::new("abc-123"),
            title: None,
            listing_type: None,
            seller: None,
            category: None,
            tags: Vec::new(),
            rating: None,
            is_free: false,
            is_mature: false,
            published_at: None,
            thumbnail_url: None,
        };
        assert_eq!(summary.display_title(), "abc-123");
    }

    #[test]
    fn display_title_prefers_a_non_blank_title() {
        let summary = AssetSummary {
            id: AssetId::new("abc-123"),
            title: Some("  Highland Rocks  ".to_string()),
            listing_type: None,
            seller: None,
            category: None,
            tags: Vec::new(),
            rating: None,
            is_free: false,
            is_mature: false,
            published_at: None,
            thumbnail_url: None,
        };
        assert_eq!(summary.display_title(), "  Highland Rocks  ");
    }

    #[test]
    fn sort_order_maps_to_api_values() {
        assert_eq!(SortOrder::Relevance.as_str(), "-relevance");
        assert_eq!(SortOrder::Newest.as_str(), "-createdAt");
        assert_eq!(SortOrder::PriceHighToLow.as_str(), "-price");
    }

    #[test]
    fn search_query_builder_chains() {
        let query = SearchQuery::text("rock")
            .with_limit(12)
            .with_listing_type("3d-model")
            .with_cursor("next");
        assert_eq!(query.text.as_deref(), Some("rock"));
        assert_eq!(query.limit, Some(12));
        assert_eq!(query.listing_types, vec!["3d-model".to_string()]);
        assert_eq!(query.cursor.as_deref(), Some("next"));
    }

    #[test]
    fn page_reports_more_pages_from_a_cursor() {
        let page: Page<u8> = Page::new(vec![1, 2], Some(10), Some("cursor".to_string()));
        assert!(page.has_more());
        assert!(!page.is_empty());
        assert_eq!(page.total, Some(10));
    }

    #[test]
    fn asset_format_reports_its_code() {
        let format = AssetFormat {
            format_type: Some(FormatType {
                code: Some("fbx".to_string()),
                ..Default::default()
            }),
            files: Vec::new(),
        };
        assert_eq!(format.code(), Some("fbx"));
    }
}
