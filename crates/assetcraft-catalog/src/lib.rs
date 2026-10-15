//! A thin, read-first client for the Fab marketplace catalog API.
//!
//! AssetCraft owns this client rather than wrapping a third-party crate so that
//! our domain types (in [`assetcraft_core`]) stay independent of any upstream
//! shape. Only the read endpoints that M1 needs are implemented here; the
//! authenticated library and download endpoints arrive with their milestones.
//!
//! Endpoint reference (all relative to the Fab base URL):
//!
//! | Method | Endpoint |
//! | --- | --- |
//! | search | `GET /i/listings/search` |
//! | detail | `GET /i/listings/{uid}` |
//! | formats | `GET /i/listings/{uid}/asset-formats` |
//! | licenses | `GET /i/taxonomy/licenses` |
//! | tag groups | `GET /i/tags/groups` |

pub mod convert;
pub mod wire;

use reqwest::Url;
use serde::de::DeserializeOwned;

use assetcraft_core::{
    status_error, AssetDetail, AssetFormat, AssetId, AssetSummary, Config, Error, LicenseType,
    Page, Result, SearchQuery, TagGroup,
};

/// A client for the public (unauthenticated) Fab catalog endpoints.
#[derive(Debug, Clone)]
pub struct FabClient {
    http: reqwest::Client,
    base: Url,
}

impl FabClient {
    /// Build a client against `base_url`, reusing an already-configured HTTP
    /// client. Passing the base URL explicitly keeps tests off the network.
    pub fn new(base_url: &str, http: reqwest::Client) -> Result<Self> {
        let base = Url::parse(base_url)
            .map_err(|e| Error::Config(format!("invalid Fab base URL {base_url:?}: {e}")))?;
        Ok(Self { http, base })
    }

    /// Build a client with a default HTTP client and the user agent from
    /// `config`.
    pub fn from_config(config: &Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(config.user_agent.clone())
            .build()?;
        Self::new(&config.fab_base_url, http)
    }

    /// The base URL this client talks to.
    pub fn base_url(&self) -> &str {
        self.base.as_str()
    }

    /// Search the catalog.
    pub async fn search(&self, query: &SearchQuery) -> Result<Page<AssetSummary>> {
        let url = self.search_url(query)?;
        let results: wire::SearchResults = self.get_json(url).await?;
        Ok(convert::page_from_search(results))
    }

    /// Fetch full detail for one listing.
    pub async fn listing(&self, id: &AssetId) -> Result<AssetDetail> {
        let url = self.endpoint_with_id("/i/listings", id);
        let detail: wire::ListingDetail = self.get_json(url).await?;
        Ok(convert::detail_from_listing(detail))
    }

    /// List the downloadable formats available for a listing.
    pub async fn listing_formats(&self, id: &AssetId) -> Result<Vec<AssetFormat>> {
        let url = self.endpoint(&format!("/i/listings/{}/asset-formats", id.as_str()));
        let formats: Vec<wire::ListingFormat> = self.get_json(url).await?;
        Ok(convert::formats_from_wire(formats))
    }

    /// List the license types Fab offers.
    pub async fn licenses(&self) -> Result<Vec<LicenseType>> {
        let url = self.endpoint("/i/taxonomy/licenses");
        let licenses: Vec<wire::LicenseType> = self.get_json(url).await?;
        Ok(convert::licenses_from_wire(licenses))
    }

    /// List the tag groups used for filtering.
    pub async fn tag_groups(&self) -> Result<Vec<TagGroup>> {
        let url = self.endpoint("/i/tags/groups");
        let wrapper: wire::ResultsWrapper<wire::TagGroup> = self.get_json(url).await?;
        Ok(convert::tag_groups_from_wire(wrapper.results))
    }

    /// Build the search URL for a query. Exposed so the mapping from
    /// [`SearchQuery`] to Fab's query parameters can be tested without a
    /// network round-trip.
    pub fn search_url(&self, query: &SearchQuery) -> Result<Url> {
        let mut url = self.endpoint("/i/listings/search");

        let mut pairs: Vec<(&str, String)> = Vec::new();
        if let Some(text) = query.text.as_deref().filter(|t| !t.is_empty()) {
            pairs.push(("q", text.to_string()));
        }
        if !query.channels.is_empty() {
            pairs.push(("channels", query.channels.join(",")));
        }
        if !query.listing_types.is_empty() {
            pairs.push(("listing_types", query.listing_types.join(",")));
        }
        if !query.categories.is_empty() {
            pairs.push(("categories", query.categories.join(",")));
        }
        if let Some(seller) = &query.seller {
            pairs.push(("seller", seller.clone()));
        }
        if let Some(sort) = query.sort_by {
            pairs.push(("sort_by", sort.as_str().to_string()));
        }
        if let Some(limit) = query.limit {
            pairs.push(("count", limit.to_string()));
        }
        if let Some(cursor) = &query.cursor {
            pairs.push(("cursor", cursor.clone()));
        }
        if query.free_only {
            pairs.push(("is_free", "1".to_string()));
        }
        if query.discounted_only {
            pairs.push(("is_discounted", "true".to_string()));
        }
        if let Some(pct) = query.min_discount_percentage {
            pairs.push(("min_discount_percentage", pct.to_string()));
        }

        // Only touch the query string when there is something to add, so an
        // empty query stays a bare path rather than gaining a trailing `?`.
        if !pairs.is_empty() {
            let mut serializer = url.query_pairs_mut();
            for (key, value) in pairs {
                serializer.append_pair(key, &value);
            }
        }
        Ok(url)
    }

    fn endpoint(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        url.set_query(None);
        url
    }

    fn endpoint_with_id(&self, prefix: &str, id: &AssetId) -> Url {
        let mut url = self.base.clone();
        url.set_path(&format!("{prefix}/{}", id.as_str()));
        url.set_query(None);
        url
    }

    /// GET `url`, decode JSON, and turn non-success statuses into errors.
    async fn get_json<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        let response = self.http.get(url).send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(status_error(status, &body));
        }
        serde_json::from_str(&body).map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assetcraft_core::SortOrder;
    use httpmock::prelude::*;

    fn client_for(server: &MockServer) -> FabClient {
        FabClient::new(&server.base_url(), reqwest::Client::new()).unwrap()
    }

    #[test]
    fn search_url_includes_only_the_requested_filters() {
        let client = FabClient::new("https://www.fab.com", reqwest::Client::new()).unwrap();
        let query = SearchQuery::text("hard surface rock")
            .with_limit(24)
            .with_listing_type("3d-model");
        let url = client.search_url(&query).unwrap();
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

        assert_eq!(url.path(), "/i/listings/search");
        assert!(pairs.contains(&("q".to_string(), "hard surface rock".to_string())));
        assert!(pairs.contains(&("count".to_string(), "24".to_string())));
        assert!(pairs.contains(&("listing_types".to_string(), "3d-model".to_string())));
        // Absent filters must not appear as empty parameters.
        assert!(!pairs.iter().any(|(k, _)| k == "seller"));
        assert!(!pairs.iter().any(|(k, _)| k == "is_free"));
    }

    #[test]
    fn search_url_maps_free_and_sort_filters() {
        let client = FabClient::new("https://www.fab.com", reqwest::Client::new()).unwrap();
        let mut query = SearchQuery::text("grass");
        query.free_only = true;
        query.sort_by = Some(SortOrder::Newest);
        let url = client.search_url(&query).unwrap();
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

        assert!(pairs.contains(&("is_free".to_string(), "1".to_string())));
        assert!(pairs.contains(&("sort_by".to_string(), "-createdAt".to_string())));
    }

    #[test]
    fn empty_query_produces_a_bare_search_path() {
        let client = FabClient::new("https://www.fab.com", reqwest::Client::new()).unwrap();
        let url = client.search_url(&SearchQuery::default()).unwrap();
        assert_eq!(url.as_str(), "https://www.fab.com/i/listings/search");
    }

    #[test]
    fn rejects_a_bad_base_url() {
        assert!(FabClient::new("not a url", reqwest::Client::new()).is_err());
    }

    #[tokio::test]
    async fn search_decodes_a_mocked_response() {
        let server = MockServer::start_async().await;
        let search = server.mock_async(|when, then| {
            when.method(GET).path("/i/listings/search").query_param("q", "rock");
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    r#"{
                        "results": [{
                            "uid": "a55fc08e-82ec-4332-8bed-dde44fff7847",
                            "title": "Steam Integration for Unreal Engine",
                            "listingType": "tool-and-plugin",
                            "isFree": false,
                            "user": {"sellerName": "PloxTools", "sellerId": "o-hz52h7n"},
                            "thumbnails": [{"mediaUrl": "https://media.fab.com/img.jpg", "type": "thumbnail"}]
                        }],
                        "count": 1,
                        "cursors": {"next": "cD0yMDI2", "previous": null}
                    }"#,
                );
        })
        .await;

        let client = client_for(&server);
        let page = client.search(&SearchQuery::text("rock")).await.unwrap();

        search.assert_async().await;
        assert_eq!(page.total, Some(1));
        assert_eq!(page.next_cursor.as_deref(), Some("cD0yMDI2"));
        let item = &page.items[0];
        assert_eq!(item.display_title(), "Steam Integration for Unreal Engine");
        assert_eq!(item.seller_name(), Some("PloxTools"));
        assert_eq!(
            item.thumbnail_url.as_deref(),
            Some("https://media.fab.com/img.jpg")
        );
    }

    #[tokio::test]
    async fn search_surfaces_a_non_success_status() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/i/listings/search");
                then.status(403).body("challenge");
            })
            .await;

        let client = client_for(&server);
        let err = client.search(&SearchQuery::text("x")).await.unwrap_err();
        match err {
            Error::HttpStatus { status, body } => {
                assert_eq!(status, 403);
                assert_eq!(body, "challenge");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn listing_decodes_a_mocked_detail() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/i/listings/id-42");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(r#"{"uid": "id-42", "title": "Cliff", "description": "A cliff", "reviewCount": 3}"#);
            })
            .await;

        let client = client_for(&server);
        let detail = client.listing(&AssetId::new("id-42")).await.unwrap();
        assert_eq!(detail.id().as_str(), "id-42");
        assert_eq!(detail.description.as_deref(), Some("A cliff"));
        assert_eq!(detail.review_count, Some(3));
    }

    #[tokio::test]
    async fn listing_formats_decodes_a_mocked_response() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/i/listings/id-42/asset-formats");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(r#"[{"assetFormatType": {"code": "fbx", "name": "FBX"}, "files": [{"uid": "f1", "fileSize": 2048}]}]"#);
            })
            .await;

        let client = client_for(&server);
        let formats = client
            .listing_formats(&AssetId::new("id-42"))
            .await
            .unwrap();
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].code(), Some("fbx"));
        assert_eq!(formats[0].files[0].size_bytes, Some(2048));
    }

    #[tokio::test]
    async fn tag_groups_unwrap_the_results_envelope() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/i/tags/groups");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(r#"{"results": [{"name": "Rendering Style", "slug": "rendering-style", "tags": [{"name": "Realistic", "uid": "t1"}]}]}"#);
            })
            .await;

        let client = client_for(&server);
        let groups = client.tag_groups().await.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name.as_deref(), Some("Rendering Style"));
        assert_eq!(groups[0].tags[0].name.as_deref(), Some("Realistic"));
    }
}
