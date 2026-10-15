//! Keeping a stored session usable.
//!
//! Callers that need an access token should not have to reason about expiry.
//! [`ensure_fresh_token`] loads the stored token and, if it is close to
//! expiring, refreshes it before handing it back — so every command that talks
//! to an authenticated endpoint gets a token it can actually use.

use assetcraft_core::{Error, Result};
use chrono::Utc;

use crate::epic::EpicAuth;
use crate::store::TokenStore;
use crate::token::EpicToken;

/// How many seconds before expiry a token is refreshed.
///
/// Refreshing slightly early means a request never races the expiry: a token
/// that is about to lapse is replaced before it is used.
pub const REFRESH_MARGIN_SECONDS: i64 = 300;

/// Load the stored token, refreshing it first if it is close to expiry.
///
/// Returns [`Error::NotAuthenticated`] when nothing is stored, with a message
/// telling the user how to sign in.
pub async fn ensure_fresh_token(auth: &EpicAuth, store: &impl TokenStore) -> Result<EpicToken> {
    let stored = store.load()?.ok_or_else(|| {
        Error::NotAuthenticated("not signed in; run `assetcraft login` first".to_string())
    })?;

    if !stored.expires_within(Utc::now(), REFRESH_MARGIN_SECONDS) {
        return Ok(stored);
    }

    // Without a refresh token there is nothing we can do here; hand the token
    // back and let the API answer, rather than failing where we cannot help.
    let Some(refresh_token) = stored.refresh_token.clone() else {
        return Ok(stored);
    };

    let mut fresh = auth.refresh(&refresh_token).await?;
    // Epic usually issues a new refresh token, but if it does not, keep the one
    // that is still valid so the session survives the next refresh too.
    if fresh.refresh_token.is_none() {
        fresh.refresh_token = Some(refresh_token);
    }
    store.save(&fresh)?;
    Ok(fresh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::FileTokenStore;
    use httpmock::prelude::*;

    fn token(expires_in_seconds: i64, refresh: Option<&str>) -> EpicToken {
        EpicToken {
            access_token: "old-access".to_string(),
            refresh_token: refresh.map(str::to_string),
            token_type: "bearer".to_string(),
            expires_at: Some(Utc::now() + chrono::Duration::seconds(expires_in_seconds)),
            account_id: Some("acc-1".to_string()),
            display_name: None,
        }
    }

    #[tokio::test]
    async fn nothing_stored_is_an_error_telling_the_user_to_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileTokenStore::new(dir.path().join("token.json"));
        let auth = EpicAuth::new(
            "https://account.example",
            "https://www.epicgames.com",
            reqwest::Client::new(),
        );

        let err = ensure_fresh_token(&auth, &store).await.unwrap_err();
        assert!(matches!(err, Error::NotAuthenticated(_)), "got {err:?}");
        assert!(err.to_string().contains("assetcraft login"));
    }

    #[tokio::test]
    async fn a_fresh_token_is_returned_without_touching_the_network() {
        // No mocks are registered, so any request would fail the test.
        let server = MockServer::start_async().await;
        let dir = tempfile::tempdir().unwrap();
        let store = FileTokenStore::new(dir.path().join("token.json"));
        store.save(&token(3600, Some("refresh"))).unwrap();
        let auth = EpicAuth::new(
            server.base_url(),
            "https://www.epicgames.com",
            reqwest::Client::new(),
        );

        let resolved = ensure_fresh_token(&auth, &store).await.unwrap();
        assert_eq!(resolved.access_token, "old-access");
    }

    #[tokio::test]
    async fn an_expiring_token_is_refreshed_and_resaved() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/account/api/oauth/token")
                    .body("grant_type=refresh_token&refresh_token=refresh-1&token_type=eg1");
                then.status(200)
                    .header("content-type", "application/json")
                    // No refresh_token in the response: the old one must be kept.
                    .body(r#"{"access_token": "new-access", "token_type": "bearer", "expires_in": 3600}"#);
            })
            .await;

        let dir = tempfile::tempdir().unwrap();
        let store = FileTokenStore::new(dir.path().join("token.json"));
        // Expires in one minute, inside the refresh margin.
        store.save(&token(60, Some("refresh-1"))).unwrap();
        let auth = EpicAuth::new(
            server.base_url(),
            "https://www.epicgames.com",
            reqwest::Client::new(),
        );

        let resolved = ensure_fresh_token(&auth, &store).await.unwrap();
        mock.assert_async().await;
        assert_eq!(resolved.access_token, "new-access");
        // The refreshed token was persisted, and the refresh token survived.
        let saved = store.load().unwrap().unwrap();
        assert_eq!(saved.access_token, "new-access");
        assert_eq!(saved.refresh_token.as_deref(), Some("refresh-1"));
    }

    #[tokio::test]
    async fn an_expiring_token_without_a_refresh_token_is_returned_as_is() {
        let server = MockServer::start_async().await;
        let dir = tempfile::tempdir().unwrap();
        let store = FileTokenStore::new(dir.path().join("token.json"));
        store.save(&token(60, None)).unwrap();
        let auth = EpicAuth::new(
            server.base_url(),
            "https://www.epicgames.com",
            reqwest::Client::new(),
        );

        let resolved = ensure_fresh_token(&auth, &store).await.unwrap();
        assert_eq!(resolved.access_token, "old-access");
    }
}
