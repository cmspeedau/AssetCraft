//! A thin, read-first client for the Fab marketplace catalog API.
//!
//! AssetCraft owns this client rather than wrapping a third-party crate so that
//! our domain types (in [`assetcraft_core`]) stay independent of any upstream
//! shape. Endpoints are added alongside the milestones that need them.
//!
//! Endpoint reference (all relative to the Fab base URL):
//!
//! | Method | Endpoint | Auth |
//! | --- | --- | --- |
//! | search | `GET /i/listings/search` | no |
//! | detail | `GET /i/listings/{uid}` | no |
//! | formats | `GET /i/listings/{uid}/asset-formats` | no |
//! | licenses | `GET /i/taxonomy/licenses` | no |
//! | tag groups | `GET /i/tags/groups` | no |
//! | CSRF handshake | `GET /i/csrf` | yes |
//! | owned library | `GET /e/accounts/{account_id}/ue/library` | yes |
//! | file download info | `GET /p/egl/listings/{id}/asset-formats/{fmt}/files/{f}/download-info` | yes |
//! | artifact manifest | `POST /e/artifacts/{artifact_id}/manifest` | yes |
//!
//! Authenticated calls need a bearer token *and* a Fab CSRF cookie, which the
//! first authenticated call establishes by handshaking against `/i/csrf`.

pub mod cache;
pub mod convert;
pub mod wire;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use reqwest::Url;
use serde::de::DeserializeOwned;

use assetcraft_core::{
    status_error, AssetDetail, AssetFormat, AssetId, AssetSummary, Config, DownloadInfo, Error,
    LibraryItem, LicenseType, Page, Result, SearchQuery, TagGroup,
};

pub use cache::ResponseCache;

/// A client for the Fab catalog and library endpoints.
#[derive(Debug, Clone)]
pub struct FabClient {
    http: reqwest::Client,
    base: Url,
    token: Option<String>,
    cache: Option<ResponseCache>,
    session_ready: Arc<AtomicBool>,
}

impl FabClient {
    /// Build a client against `base_url`, reusing an already-configured HTTP
    /// client. Passing the base URL explicitly keeps tests off the network.
    pub fn new(base_url: &str, http: reqwest::Client) -> Result<Self> {
        let base = Url::parse(base_url)
            .map_err(|e| Error::Config(format!("invalid Fab base URL {base_url:?}: {e}")))?;
        Ok(Self {
            http,
            base,
            token: None,
            cache: None,
            session_ready: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Build a client with a default HTTP client and the user agent from
    /// `config`.
    ///
    /// The cookie store is enabled because the authenticated endpoints depend on
    /// the Fab CSRF cookie being carried between requests.
    pub fn from_config(config: &Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(config.user_agent.clone())
            .cookie_store(true)
            .build()?;
        Self::new(&config.fab_base_url, http)
    }

    /// Attach an `Authorization` header value (see
    /// [`assetcraft_auth::EpicToken::authorization_header`]) for authenticated
    /// calls.
    pub fn with_token(mut self, authorization_header: impl Into<String>) -> Self {
        self.token = Some(authorization_header.into());
        self
    }

    /// Serve public GET requests from `cache` when possible.
    pub fn with_cache(mut self, cache: ResponseCache) -> Self {
        self.cache = Some(cache);
        self
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

    /// Fetch one page of the user's owned library. Requires a token.
    pub async fn library(
        &self,
        account_id: &str,
        cursor: Option<&str>,
    ) -> Result<Page<LibraryItem>> {
        let mut url = self.endpoint(&format!("/e/accounts/{account_id}/ue/library"));
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("count", "100");
            if let Some(cursor) = cursor {
                pairs.append_pair("cursor", cursor);
            }
        }
        let library: wire::Library = self.get_json_authorized(url).await?;
        Ok(convert::library_page_from_wire(library))
    }

    /// Resolve where one file of one format can be downloaded from. Requires a
    /// token.
    pub async fn download_info(
        &self,
        listing_id: &str,
        format_code: &str,
        file_id: &str,
    ) -> Result<DownloadInfo> {
        let url = self.endpoint(&format!(
            "/p/egl/listings/{listing_id}/asset-formats/{format_code}/files/{file_id}/download-info"
        ));
        let info: wire::DownloadInfo = self.get_json_authorized(url).await?;
        Ok(convert::download_info_from_wire(info))
    }

    /// Request the artifact manifest for an owned item. Requires a token.
    pub async fn asset_manifest(
        &self,
        artifact_id: &str,
        item_id: &str,
        namespace: &str,
        platform: &str,
    ) -> Result<Vec<DownloadInfo>> {
        self.ensure_session().await?;
        let url = self.endpoint(&format!("/e/artifacts/{artifact_id}/manifest"));
        let response = self
            .http
            .post(url)
            .header(reqwest::header::AUTHORIZATION, self.authorization()?)
            .json(&serde_json::json!({
                "item_id": item_id,
                "namespace": namespace,
                "platform": platform,
            }))
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;
        let manifest: wire::AssetManifest = decode(status, &body)?;
        Ok(manifest
            .download_info
            .into_iter()
            .map(convert::download_info_from_wire)
            .collect())
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
        self.endpoint(&format!("{prefix}/{}", id.as_str()))
    }

    /// The bearer header value, or an error explaining how to get one.
    fn authorization(&self) -> Result<&str> {
        self.token.as_deref().ok_or_else(|| {
            Error::NotAuthenticated(
                "this request needs an access token; run `assetcraft login` first".to_string(),
            )
        })
    }

    /// Establish the Fab session (CSRF cookie) if it has not been established.
    ///
    /// Fab's authenticated endpoints reject requests without the CSRF cookie, so
    /// the first authenticated call performs the handshake. The flag is only set
    /// once the handshake succeeds.
    async fn ensure_session(&self) -> Result<()> {
        // Check the token first so a missing credential is reported as such
        // rather than as whatever the CSRF call happens to return.
        self.authorization()?;

        if self.session_ready.load(Ordering::Acquire) {
            return Ok(());
        }

        let response = self.http.get(self.endpoint("/i/csrf")).send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(status_error(status, &body));
        }
        self.session_ready.store(true, Ordering::Release);
        Ok(())
    }

    /// GET a public URL, served from cache when possible.
    async fn get_json<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        let body = self.fetch_public(&url).await?;
        serde_json::from_str(&body).map_err(Error::from)
    }

    /// GET an authenticated URL.
    async fn get_json_authorized<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        self.ensure_session().await?;
        let response = self
            .http
            .get(url)
            .header(reqwest::header::AUTHORIZATION, self.authorization()?)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        decode(status, &body)
    }

    async fn fetch_public(&self, url: &Url) -> Result<String> {
        if let Some(cache) = &self.cache {
            if let Some(hit) = cache.get(url.as_str()) {
                return Ok(hit);
            }
        }

        let response = self.http.get(url.clone()).send().await?;
        let status = response.status();
        let body = response.text().await?;
        decode_body(status, &body)?;

        // Only public responses are cached: authenticated ones are user-specific.
        if let Some(cache) = &self.cache {
            let _ = cache.put(url.as_str(), &body);
        }
        Ok(body)
    }
}

/// Turn a non-success status into an error, or hand back the body.
fn decode_body(status: reqwest::StatusCode, body: &str) -> Result<&str> {
    if status.is_success() {
        Ok(body)
    } else {
        Err(status_error(status, body))
    }
}

fn decode<T: DeserializeOwned>(status: reqwest::StatusCode, body: &str) -> Result<T> {
    serde_json::from_str(decode_body(status, body)?).map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use assetcraft_core::SortOrder;
    use chrono::Duration;
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
    async fn authenticated_calls_without_a_token_say_so() {
        // No network: the client must fail before sending anything.
        let client = FabClient::new("http://127.0.0.1:1", reqwest::Client::new()).unwrap();
        let err = client.library("acc-1", None).await.unwrap_err();
        assert!(matches!(err, Error::NotAuthenticated(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn search_decodes_a_mocked_response() {
        let server = MockServer::start_async().await;
        let search = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/i/listings/search")
                    .query_param("q", "rock");
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
                    .body(
                        r#"{"uid": "id-42", "title": "Cliff", "description": "A cliff", "reviewCount": 3}"#,
                    );
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
                    .body(
                        r#"[{"assetFormatType": {"code": "fbx", "name": "FBX"}, "files": [{"uid": "f1", "fileSize": 2048}]}]"#,
                    );
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
                    .body(
                        r#"{"results": [{"name": "Rendering Style", "slug": "rendering-style", "tags": [{"name": "Realistic", "uid": "t1"}]}]}"#,
                    );
            })
            .await;

        let client = client_for(&server);
        let groups = client.tag_groups().await.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name.as_deref(), Some("Rendering Style"));
        assert_eq!(groups[0].tags[0].name.as_deref(), Some("Realistic"));
    }

    #[tokio::test]
    async fn library_handshakes_csrf_then_sends_the_bearer_token() {
        let server = MockServer::start_async().await;
        let csrf = server
            .mock_async(|when, then| {
                when.method(GET).path("/i/csrf");
                then.status(200).body("{}");
            })
            .await;
        let library = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/e/accounts/acc-1/ue/library")
                    .query_param("count", "100")
                    .header("authorization", "bearer token-abc");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        r#"{
                            "cursors": {"next": "lib-2"},
                            "results": [{
                                "assetId": "asset-1",
                                "assetNamespace": "ue",
                                "title": "Highland Rock",
                                "images": [{"url": "https://media/rock.jpg", "type": "thumbnail", "width": "1024"}]
                            }]
                        }"#,
                    );
            })
            .await;

        let client = client_for(&server).with_token("bearer token-abc");
        let page = client.library("acc-1", None).await.unwrap();

        csrf.assert_async().await;
        library.assert_async().await;
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.next_cursor.as_deref(), Some("lib-2"));
        let item = &page.items[0];
        assert_eq!(item.title, "Highland Rock");
        assert_eq!(item.namespace, "ue");
        assert_eq!(item.thumbnail_url(), Some("https://media/rock.jpg"));
    }

    #[tokio::test]
    async fn download_info_decodes_a_mocked_response() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/i/csrf");
                then.status(200).body("{}");
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/p/egl/listings/lst-1/asset-formats/fbx/files/file-1/download-info");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        r#"{"artifactId": "art-1", "assetFormat": "fbx", "distributionPoints": [{"manifestUrl": "https://cdn/manifest"}]}"#,
                    );
            })
            .await;

        let client = client_for(&server).with_token("bearer token-abc");
        let info = client
            .download_info("lst-1", "fbx", "file-1")
            .await
            .unwrap();
        assert_eq!(info.artifact_id, "art-1");
        assert_eq!(info.asset_format, "fbx");
        assert_eq!(
            info.distribution_points[0].manifest_url,
            "https://cdn/manifest"
        );
    }

    #[tokio::test]
    async fn asset_manifest_posts_and_decodes() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/i/csrf");
                then.status(200).body("{}");
            })
            .await;
        let manifest = server
            .mock_async(|when, then| {
                when.method(POST).path("/e/artifacts/art-1/manifest");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        r#"{"downloadInfo": [{"artifactId": "art-1", "assetFormat": "fbx", "distributionPoints": [{"manifestUrl": "https://cdn/m"}]}]}"#,
                    );
            })
            .await;

        let client = client_for(&server).with_token("bearer token-abc");
        let infos = client
            .asset_manifest("art-1", "item-1", "ns", "Windows")
            .await
            .unwrap();

        manifest.assert_async().await;
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].artifact_id, "art-1");
    }

    #[tokio::test]
    async fn a_cached_public_response_avoids_a_second_request() {
        let server = MockServer::start_async().await;
        let search = server
            .mock_async(|when, then| {
                when.method(GET).path("/i/listings/search");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(r#"{"results": [{"uid": "a"}], "count": 1}"#);
            })
            .await;

        let dir = tempfile::tempdir().unwrap();
        let cache = ResponseCache::new(dir.path(), Duration::hours(1));
        let client = client_for(&server).with_cache(cache);

        let first = client.search(&SearchQuery::text("rock")).await.unwrap();
        let second = client.search(&SearchQuery::text("rock")).await.unwrap();

        assert_eq!(first.items.len(), 1);
        assert_eq!(second.items.len(), 1);
        // The second call was served from disk.
        assert_eq!(search.calls_async().await, 1);
    }
}
