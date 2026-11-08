//! Raw wire types for the Fab JSON API.
//!
//! These mirror the shape Fab actually returns and are kept separate from the
//! domain model in [`assetcraft_core`] so that upstream changes only ripple
//! through [`crate::convert`]. Fields are permissive (`Option`/`#[serde(default)]`)
//! because Fab omits keys freely — and omits them *as `null`* as often as not:
//! a format with no files yet answers `"files": null`, an extension list
//! answers `"extensions": null`. A `null` where a list or a bool was expected
//! deserialises to empty or false (`null_to_empty`, `null_to_false`); refusing
//! it is how a signed-in first real download died on Fab answering honestly
//! about an asset whose Unreal files carry no size.

use serde::Deserialize;

/// A `Vec` field Fab may answer with `null` rather than omit: read it as
/// empty. `#[serde(default)]` covers a *missing* key but chokes on a present
/// `null`, and the live API sends both. Refusing the null is how a signed-in
/// first real download died: the formats page for an Unreal asset carries
/// `"platformsIncluded": null`, and the app had never seen one before.
fn null_to_empty<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

/// A `bool` field Fab may answer with `null`: read it as false.
fn null_to_false<'de, D>(deserializer: D) -> std::result::Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<bool>::deserialize(deserializer)?.unwrap_or(false))
}

/// Response body of `GET /i/listings/search`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchResults {
    #[serde(default, deserialize_with = "null_to_empty")]
    pub results: Vec<SearchListing>,
    pub count: Option<u64>,
    pub cursors: Option<Cursors>,
}

/// Pagination cursors attached to a search response.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Cursors {
    pub next: Option<String>,
    pub previous: Option<String>,
}

/// A listing as it appears in search results.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchListing {
    pub uid: String,
    pub title: Option<String>,
    pub listing_type: Option<String>,
    pub user: Option<User>,
    pub category: Option<Category>,
    #[serde(default, deserialize_with = "null_to_false")]
    pub is_mature: bool,
    #[serde(default, deserialize_with = "null_to_false")]
    pub is_free: bool,
    #[serde(default, deserialize_with = "null_to_false")]
    pub is_discounted: bool,
    pub published_at: Option<String>,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub tags: Vec<Tag>,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub thumbnails: Vec<Thumbnail>,
    pub ratings: Option<Ratings>,
}

/// Full listing detail from `GET /i/listings/{uid}`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListingDetail {
    pub uid: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub listing_type: Option<String>,
    pub user: Option<User>,
    pub category: Option<Category>,
    pub created_at: Option<String>,
    pub published_at: Option<String>,
    pub review_count: Option<u64>,
    #[serde(default, deserialize_with = "null_to_false")]
    pub is_mature: bool,
    #[serde(default, deserialize_with = "null_to_false")]
    pub is_free: bool,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub tags: Vec<Tag>,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub thumbnails: Vec<Thumbnail>,
    pub ratings: Option<Ratings>,
}

/// Seller/user info embedded in a listing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub uid: Option<String>,
    pub seller_name: Option<String>,
    pub seller_id: Option<String>,
    pub profile_url: Option<String>,
    pub is_seller: Option<bool>,
}

/// A category embedded in a listing.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Category {
    pub uid: Option<String>,
    pub name: Option<String>,
    pub path: Option<String>,
    pub slug: Option<String>,
}

/// A tag embedded in a listing or tag group.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Tag {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub uid: Option<String>,
}

/// A thumbnail or preview image.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thumbnail {
    pub uid: Option<String>,
    pub media_url: Option<String>,
    #[serde(rename = "type")]
    pub thumbnail_type: Option<String>,
}

/// Aggregate ratings object.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ratings {
    pub average_rating: Option<f64>,
    pub total: Option<u64>,
}

/// One downloadable format from `GET /i/listings/{uid}/asset-formats`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListingFormat {
    pub asset_format_type: Option<FormatType>,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub files: Vec<FormatFile>,
}

/// Descriptor for a format kind.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FormatType {
    pub code: Option<String>,
    pub icon: Option<String>,
    pub name: Option<String>,
    pub extensions: Option<Vec<String>>,
}

/// A downloadable file inside a format.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatFile {
    pub uid: Option<String>,
    pub name: Option<String>,
    pub file_size: Option<u64>,
    pub asset_type: Option<String>,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub platforms_included: Vec<String>,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub engine_versions: Vec<String>,
}

/// A license type from `GET /i/taxonomy/licenses`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LicenseType {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub url: Option<String>,
}

/// A tag group from `GET /i/tags/groups` (wrapped in `results`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TagGroup {
    pub category: Option<String>,
    pub name: Option<String>,
    pub order: Option<i32>,
    pub slug: Option<String>,
    pub uid: Option<String>,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub tags: Vec<Tag>,
}

/// The `{ "results": [...] }` envelope Fab uses for some endpoints.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct ResultsWrapper<T> {
    #[serde(default, deserialize_with = "null_to_empty")]
    pub results: Vec<T>,
}

/// Response body of `GET /e/accounts/{id}/ue/library`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Library {
    pub cursors: Option<LibraryCursors>,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub results: Vec<LibraryAsset>,
}

/// Pagination cursor for a library response.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryCursors {
    pub next: Option<String>,
}

/// One owned asset in the library.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryAsset {
    pub asset_id: String,
    pub asset_namespace: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub source: Option<String>,
    pub distribution_method: Option<String>,
    /// The type as the library labels it ("3D", "Tools & Plugins"), not the
    /// search API's slug.
    pub listing_type: Option<String>,
    /// The seller or publisher's display name.
    pub seller: Option<String>,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub categories: Vec<LibraryCategory>,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub images: Vec<LibraryImage>,
    /// The engine builds Fab can deliver for this asset. A build's
    /// `artifactId` is what `POST /e/artifacts/{artifactId}/manifest` takes:
    /// the only download route Fab offers third parties for UE-format
    /// assets. An asset with no artifact here has no download route.
    #[serde(default, deserialize_with = "null_to_empty")]
    pub project_versions: Vec<ProjectVersion>,
}

/// One engine build of a library asset, as `projectVersions[]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectVersion {
    pub artifact_id: Option<String>,
}

/// A category on a library asset.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryCategory {
    pub id: Option<String>,
    pub name: Option<String>,
}

/// An image on a library asset. Dimensions arrive as strings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryImage {
    pub url: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub width: Option<String>,
    pub height: Option<String>,
    pub md5: Option<String>,
}

/// Download info for one file, from the `download-info` endpoint or a manifest.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadInfo {
    pub artifact_id: Option<String>,
    pub asset_format: Option<String>,
    pub build_version: Option<String>,
    /// SHA-1 of the manifest blob the distribution points serve, hex. The
    /// live API sends it; checking it catches a truncated CDN body.
    pub manifest_hash: Option<String>,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub distribution_points: Vec<DistributionPoint>,
    #[serde(default, deserialize_with = "null_to_empty")]
    pub distribution_point_base_urls: Vec<String>,
}

/// A signed location a manifest can be fetched from.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistributionPoint {
    pub manifest_url: String,
    pub signature_expiration: Option<String>,
}

/// Response of `POST /e/artifacts/{id}/manifest`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetManifest {
    #[serde(default, deserialize_with = "null_to_empty")]
    pub download_info: Vec<DownloadInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sample captured from the live Fab API.
    const REAL_SEARCH: &str = r#"{
        "aggregations": null,
        "cursors": {"next": "cD0yMDI2", "previous": null},
        "results": [{
            "uid": "a55fc08e-82ec-4332-8bed-dde44fff7847",
            "title": "Steam Integration for Unreal Engine",
            "listingType": "tool-and-plugin",
            "isMature": false,
            "isFree": false,
            "isDiscounted": false,
            "publishedAt": "2026-02-16T10:00:00Z",
            "category": {"uid": "afd8c50c", "name": "Engine Tools", "path": "engine-tools", "slug": "engine-tools"},
            "user": {"sellerName": "PloxTools", "sellerId": "o-hz52h7n", "uid": "86b03c21", "isSeller": true},
            "ratings": {"averageRating": 4.5, "total": 12},
            "tags": [{"name": "Steam", "slug": "steam", "uid": "tag-001"}],
            "thumbnails": [{"uid": "thumb-001", "mediaUrl": "https://media.fab.com/img.jpg", "type": "thumbnail"}]
        }]
    }"#;

    #[test]
    fn deserialize_real_search_result() {
        let results: SearchResults = serde_json::from_str(REAL_SEARCH).unwrap();
        assert_eq!(results.results.len(), 1);
        assert_eq!(results.count, None);
        assert_eq!(
            results.cursors.as_ref().unwrap().next.as_deref(),
            Some("cD0yMDI2")
        );

        let listing = &results.results[0];
        assert_eq!(listing.uid, "a55fc08e-82ec-4332-8bed-dde44fff7847");
        assert_eq!(listing.listing_type.as_deref(), Some("tool-and-plugin"));
        assert!(!listing.is_free);
        assert_eq!(
            listing.user.as_ref().unwrap().seller_name.as_deref(),
            Some("PloxTools")
        );
        assert_eq!(
            listing.category.as_ref().unwrap().name.as_deref(),
            Some("Engine Tools")
        );
        assert_eq!(listing.ratings.as_ref().unwrap().average_rating, Some(4.5));
        assert_eq!(listing.tags[0].name.as_deref(), Some("Steam"));
        assert_eq!(
            listing.thumbnails[0].thumbnail_type.as_deref(),
            Some("thumbnail")
        );
    }

    #[test]
    fn deserialize_search_tolerates_missing_keys() {
        let json = r#"{"results": [{"uid": "abc"}]}"#;
        let results: SearchResults = serde_json::from_str(json).unwrap();
        let listing = &results.results[0];
        assert_eq!(listing.uid, "abc");
        assert!(listing.title.is_none());
        assert!(!listing.is_free);
        assert!(listing.tags.is_empty());
        assert!(results.cursors.is_none());
    }

    #[test]
    fn deserialize_empty_search() {
        let results: SearchResults =
            serde_json::from_str(r#"{"results": [], "count": 0}"#).unwrap();
        assert!(results.results.is_empty());
        assert_eq!(results.count, Some(0));
    }

    #[test]
    fn deserialize_listing_detail() {
        let json = r#"{
            "uid": "listing-xyz",
            "title": "Sci-Fi Weapon Set",
            "description": "50 weapons with PBR materials",
            "listingType": "3d-model",
            "user": {"sellerName": "GameArt Studio", "uid": "seller-042"},
            "createdAt": "2025-03-01T00:00:00Z",
            "publishedAt": "2025-03-01T00:00:00Z",
            "reviewCount": 23
        }"#;
        let detail: ListingDetail = serde_json::from_str(json).unwrap();
        assert_eq!(detail.uid, "listing-xyz");
        assert_eq!(detail.review_count, Some(23));
        assert_eq!(
            detail.description.as_deref(),
            Some("50 weapons with PBR materials")
        );
        assert_eq!(
            detail.user.as_ref().unwrap().seller_name.as_deref(),
            Some("GameArt Studio")
        );
    }

    #[test]
    fn deserialize_listing_formats() {
        let json = r#"[
            {
                "assetFormatType": {"code": "unreal-engine", "icon": "unreal-engine", "name": "Unreal Engine"},
                "files": [{"uid": "file-001", "name": "MyAsset V1", "fileSize": 3444643477}]
            },
            {
                "assetFormatType": {"code": "fbx", "name": "FBX"},
                "files": [{"uid": "file-002", "name": "myasset.zip", "fileSize": 1234567,
                           "platformsIncluded": ["Windows"], "engineVersions": ["5.4"]}]
            }
        ]"#;
        let formats: Vec<ListingFormat> = serde_json::from_str(json).unwrap();
        assert_eq!(formats.len(), 2);
        assert_eq!(
            formats[0]
                .asset_format_type
                .as_ref()
                .unwrap()
                .code
                .as_deref(),
            Some("unreal-engine")
        );
        assert_eq!(formats[0].files[0].file_size, Some(3444643477));
        assert_eq!(
            formats[1].files[0].platforms_included,
            vec!["Windows".to_string()]
        );
        assert_eq!(formats[1].files[0].engine_versions, vec!["5.4".to_string()]);
    }

    #[test]
    fn deserialize_listing_formats_with_live_nulls() {
        // Captured from the live API: a real asset whose Unreal file carries
        // no size and whose platform and engine lists are null, not absent.
        // Refusing those nulls is what killed a signed-in first download.
        let json = r#"[
            {
                "assetFormatType": {
                    "code": "unreal-engine",
                    "extensions": ["uasset", "uproject", "uplugin"],
                    "icon": "unreal-engine",
                    "name": "Unreal Engine",
                    "groupName": "Game Engine Formats"
                },
                "files": [{
                    "fileSize": null,
                    "name": "Unreal Engine 5.6",
                    "uid": "7c93129d-abf1-4138-b274-7d427acd4e0b",
                    "assetType": null,
                    "platformsIncluded": null,
                    "initialEngineVersion": null,
                    "engineVersions": null
                }]
            },
            {
                "assetFormatType": {
                    "code": "uefn-source",
                    "extensions": null,
                    "icon": "unreal-engine",
                    "name": "UEFN Source",
                    "groupName": "Game Engine Formats"
                },
                "files": [{
                    "fileSize": 420414054,
                    "name": "warehouse.zip",
                    "uid": "edfb82cc-f946-459e-8b7d-fe56e31a5537",
                    "assetType": null,
                    "platformsIncluded": null,
                    "initialEngineVersion": null,
                    "engineVersions": null
                }]
            }
        ]"#;
        let formats: Vec<ListingFormat> = serde_json::from_str(json).unwrap();
        assert_eq!(formats.len(), 2);
        // A null list reads as empty, not as an error and not as a missing file.
        assert_eq!(formats[0].files.len(), 1);
        assert_eq!(formats[0].files[0].file_size, None);
        assert!(formats[0].files[0].platforms_included.is_empty());
        assert!(formats[0].files[0].engine_versions.is_empty());
        assert_eq!(formats[1].files[0].file_size, Some(420414054));
        assert_eq!(
            formats[1].asset_format_type.as_ref().unwrap().extensions,
            None
        );
    }

    #[test]
    fn deserialize_licenses_array() {
        let json = r#"[{"name": "Standard License", "slug": "standard", "url": "https://fab.com/licenses/standard"}]"#;
        let licenses: Vec<LicenseType> = serde_json::from_str(json).unwrap();
        assert_eq!(licenses[0].slug.as_deref(), Some("standard"));
    }

    #[test]
    fn deserialize_tag_groups_wrapper() {
        let json = r#"{"results": [
            {"category": "style", "name": "Rendering Style", "order": 0, "slug": "rendering-style",
             "tags": [{"name": "Realistic", "slug": "realistic", "uid": "t1"}], "uid": "g1"}
        ]}"#;
        let wrapper: ResultsWrapper<TagGroup> = serde_json::from_str(json).unwrap();
        assert_eq!(wrapper.results.len(), 1);
        assert_eq!(
            wrapper.results[0].tags[0].name.as_deref(),
            Some("Realistic")
        );
    }

    #[test]
    fn deserialize_library_page() {
        let json = r#"{
            "cursors": {"next": "lib-cursor-2"},
            "results": [{
                "assetId": "asset-1",
                "assetNamespace": "ue",
                "title": "Highland Rock",
                "description": "A scanned rock",
                "url": "https://www.fab.com/listings/asset-1",
                "source": "fab",
                "distributionMethod": "asset_pack",
                "categories": [{"id": "cat-1", "name": "Rocks"}],
                "images": [{"url": "https://media/rock.jpg", "type": "thumbnail", "width": "1024", "height": "1024"}]
            }]
        }"#;
        let library: Library = serde_json::from_str(json).unwrap();
        assert_eq!(library.results.len(), 1);
        assert_eq!(
            library.cursors.as_ref().unwrap().next.as_deref(),
            Some("lib-cursor-2")
        );

        let asset = &library.results[0];
        assert_eq!(asset.asset_id, "asset-1");
        assert_eq!(asset.asset_namespace.as_deref(), Some("ue"));
        assert_eq!(asset.categories[0].name.as_deref(), Some("Rocks"));
        assert_eq!(asset.images[0].width.as_deref(), Some("1024"));
        assert_eq!(asset.images[0].kind.as_deref(), Some("thumbnail"));
    }

    #[test]
    fn deserialize_download_info() {
        let json = r#"{
            "artifactId": "art-1",
            "assetFormat": "fbx",
            "buildVersion": "v3",
            "distributionPoints": [
                {"manifestUrl": "https://cdn.example/manifest", "signatureExpiration": "2026-10-17T00:00:00.000Z"}
            ]
        }"#;
        let info: DownloadInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.artifact_id.as_deref(), Some("art-1"));
        assert_eq!(info.build_version.as_deref(), Some("v3"));
        assert_eq!(
            info.distribution_points[0].manifest_url,
            "https://cdn.example/manifest"
        );
        assert!(info.distribution_points[0].signature_expiration.is_some());
    }

    #[test]
    fn deserialize_asset_manifest_tolerates_empty_download_info() {
        let manifest: AssetManifest = serde_json::from_str(r#"{"downloadInfo": []}"#).unwrap();
        assert!(manifest.download_info.is_empty());
        let empty: AssetManifest = serde_json::from_str(r#"{}"#).unwrap();
        assert!(empty.download_info.is_empty());
    }
}
