//! The Epic Games OAuth flow.
//!
//! AssetCraft uses Epic's launcher client: the user opens a URL that starts
//! at Epic's login page, signs in, and the browser lands on a page carrying
//! the authorization code, which the user pastes back. That code - single-use
//! - is exchanged for an access token, which is refreshed thereafter.
//!
//! Epic's browser *device* flow — where a short code is displayed and the user
//! approves it — is not offered to the launcher client, so this crate does not
//! attempt it; see the crate-level notes and `docs/architecture.md`.

use assetcraft_core::{status_error, Config, Error, Result};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

use crate::token::EpicToken;

/// Epic's public launcher client id.
pub const LAUNCHER_CLIENT_ID: &str = "34a02cf8f4414e29b15921876da36f9a";

/// The matching public launcher client secret. See the note in the crate docs:
/// this identifies the client application and is not a user secret.
const LAUNCHER_CLIENT_SECRET: &str = "daafbccc737745039dffe53d94fc76cf";

/// Path of the token endpoint on the account service.
const TOKEN_PATH: &str = "/account/api/oauth/token";

/// Percent-encodes the reserved characters an Epic redirect target contains,
/// so Epic's login page will accept it as the place to land after sign-in.
/// The input carries no percent signs — the client id is hex and the rest is
/// fixed path and query — which is why plain replaces are safe here.
fn encode_redirect_target(url: &str) -> String {
    url.replace(':', "%3A")
        .replace('/', "%2F")
        .replace('?', "%3F")
        .replace('&', "%26")
        .replace('=', "%3D")
}

/// Accepts whatever the browser page handed the user as the authorization
/// code: the bare code, a redirect URL carrying `code=`, or the whole JSON
/// body of the page after sign-in, which users commonly select-all and
/// paste. Epic names the field `code`; pages have also shown it as
/// `authorizationCode`, so a paste of either yields the code. JSON that
/// holds no code is not a code and comes back empty - callers must say so
/// rather than trade something that is not a code for a token.
pub fn extract_authorization_code(pasted: &str) -> String {
    let trimmed = pasted.trim();
    if trimmed.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            for key in ["code", "authorizationCode"] {
                if let Some(code) = value.get(key).and_then(|code| code.as_str()) {
                    if !code.is_empty() {
                        return code.to_owned();
                    }
                }
            }
        }
        return String::new();
    }
    if let Some((_, tail)) = trimmed.rsplit_once("code=") {
        let code = tail.split(['&', '#']).next().unwrap_or_default();
        if !code.is_empty() {
            return code.to_owned();
        }
    }
    trimmed.to_owned()
}

/// Everything needed to authenticate against Epic.
#[derive(Debug, Clone)]
pub struct EpicAuth {
    http: reqwest::Client,
    account_base_url: String,
    epic_id_base_url: String,
}

impl EpicAuth {
    /// Build an authenticator against explicit endpoints.
    pub fn new(
        account_base_url: impl Into<String>,
        epic_id_base_url: impl Into<String>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            http,
            account_base_url: account_base_url.into(),
            epic_id_base_url: epic_id_base_url.into(),
        }
    }

    /// Build an authenticator using the endpoints and user agent from `config`.
    pub fn from_config(config: &Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(config.user_agent.clone())
            .build()?;
        Ok(Self::new(
            config.account_base_url.clone(),
            config.epic_id_base_url.clone(),
            http,
        ))
    }

    /// The URL the user opens to sign in and obtain an authorization code.
    ///
    /// It starts at Epic's HTML login page and names the JSON redirect
    /// endpoint as its target: after signing in, the browser's fresh session
    /// completes that hop and the page answers with the code. Linking the
    /// JSON endpoint directly instead shows a logged-out browser a page of
    /// nulls next to a warning about sharing codes - true, but nobody reads
    /// "null" as "sign in here first".
    pub fn authorization_url(&self) -> String {
        let base = self.epic_id_base_url.trim_end_matches('/');
        let redirect = encode_redirect_target(&format!(
            "{base}/id/api/redirect?clientId={LAUNCHER_CLIENT_ID}&responseType=code"
        ));
        format!("{base}/id/login?redirectUrl={redirect}")
    }

    /// Exchange an authorization code for a token.
    pub async fn exchange_code(&self, code: &str) -> Result<EpicToken> {
        self.request_token(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("token_type", "eg1"),
        ])
        .await
    }

    /// Exchange a refresh token for a fresh access token.
    pub async fn refresh(&self, refresh_token: &str) -> Result<EpicToken> {
        self.request_token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("token_type", "eg1"),
        ])
        .await
    }

    async fn request_token(&self, form: &[(&str, &str)]) -> Result<EpicToken> {
        let url = format!(
            "{}{TOKEN_PATH}",
            self.account_base_url.trim_end_matches('/')
        );
        let response = self
            .http
            .post(url)
            .form(form)
            .basic_auth(LAUNCHER_CLIENT_ID, Some(LAUNCHER_CLIENT_SECRET))
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(status_error(status, &body));
        }

        let wire: OAuthResponse = serde_json::from_str(&body)?;
        wire.into_token(Utc::now())
    }
}

/// The token endpoint's response, including its error shape.
#[derive(Debug, Deserialize)]
struct OAuthResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<i64>,
    expires_at: Option<String>,
    account_id: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "errorCode")]
    error_code: Option<String>,
    #[serde(rename = "errorMessage")]
    error_message: Option<String>,
}

impl OAuthResponse {
    fn into_token(self, now: DateTime<Utc>) -> Result<EpicToken> {
        if let Some(message) = self.error_message {
            let code = self.error_code.unwrap_or_else(|| "unknown".to_string());
            return Err(Error::Other(format!(
                "Epic authentication failed: {message} ({code})"
            )));
        }

        let access_token = self
            .access_token
            .ok_or(Error::MissingField("access_token"))?;
        let expires_at = self
            .expires_in
            .map(|seconds| now + Duration::seconds(seconds))
            .or_else(|| self.expires_at.as_deref().and_then(parse_rfc3339));

        Ok(EpicToken {
            access_token,
            refresh_token: self.refresh_token,
            token_type: self.token_type.unwrap_or_else(|| "bearer".to_string()),
            expires_at,
            account_id: self.account_id,
            display_name: self.display_name,
        })
    }
}

fn parse_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn auth_for(server: &MockServer) -> EpicAuth {
        EpicAuth::new(
            server.base_url(),
            "https://www.epicgames.com",
            reqwest::Client::new(),
        )
    }

    #[test]
    fn authorization_url_opens_the_login_page_with_the_json_hop_as_target() {
        let auth = EpicAuth::new(
            "https://account.example",
            "https://www.epicgames.com",
            reqwest::Client::new(),
        );
        let url = auth.authorization_url();
        assert_eq!(
            url,
            "https://www.epicgames.com/id/login?redirectUrl=https%3A%2F%2Fwww.epicgames.com\
             %2Fid%2Fapi%2Fredirect%3FclientId%3D34a02cf8f4414e29b15921876da36f9a\
             %26responseType%3Dcode"
        );
    }

    #[test]
    fn the_paste_box_accepts_everything_the_browser_page_offers() {
        // The bare code, as Epic's page labels it.
        assert_eq!(extract_authorization_code(" XYZabc "), "XYZabc");
        // The redirect URL, code and all.
        assert_eq!(
            extract_authorization_code("https://localhost/launcher/authorized?code=XYZabc"),
            "XYZabc"
        );
        // The whole JSON body, in either field name the page has used.
        assert_eq!(
            extract_authorization_code(r#"{"code":"XYZabc","sid":"s"}"#),
            "XYZabc"
        );
        assert_eq!(
            extract_authorization_code(r#"{"authorizationCode":"XYZabc"}"#),
            "XYZabc"
        );
        // A page that holds no code - the logged-out nulls - is not a code.
        assert_eq!(
            extract_authorization_code(r#"{"authorizationCode":null,"sid":null}"#),
            ""
        );
    }

    #[tokio::test]
    async fn exchange_code_parses_a_successful_response() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/account/api/oauth/token")
                    .header_exists("authorization");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        r#"{
                            "access_token": "access-123",
                            "refresh_token": "refresh-456",
                            "token_type": "bearer",
                            "expires_in": 28800,
                            "account_id": "acc-1",
                            "displayName": "Tester"
                        }"#,
                    );
            })
            .await;

        let auth = auth_for(&server);
        let token = auth.exchange_code("the-code").await.unwrap();

        mock.assert_async().await;
        assert_eq!(token.access_token, "access-123");
        assert_eq!(token.refresh_token.as_deref(), Some("refresh-456"));
        assert_eq!(token.display_name.as_deref(), Some("Tester"));
        // expires_in was present, so an absolute expiry was computed.
        assert!(token.expires_at.is_some());
        assert!(!token.is_expired(Utc::now()));
    }

    #[tokio::test]
    async fn refresh_uses_the_refresh_grant() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/account/api/oauth/token")
                    .body("grant_type=refresh_token&refresh_token=refresh-456&token_type=eg1");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(r#"{"access_token": "access-789", "token_type": "bearer", "expires_in": 3600}"#);
            })
            .await;

        let auth = auth_for(&server);
        let token = auth.refresh("refresh-456").await.unwrap();
        mock.assert_async().await;
        assert_eq!(token.access_token, "access-789");
        assert!(token.refresh_token.is_none());
    }

    #[tokio::test]
    async fn server_reported_errors_become_a_clear_message() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/account/api/oauth/token");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        r#"{"errorCode": "invalid_grant", "errorMessage": "authorization code is invalid"}"#,
                    );
            })
            .await;

        let auth = auth_for(&server);
        let err = auth.exchange_code("bad").await.unwrap_err();
        let message = err.to_string();
        assert!(message.contains("invalid_grant"), "message was: {message}");
        assert!(message.contains("authorization code is invalid"));
    }

    #[tokio::test]
    async fn a_missing_access_token_is_a_missing_field_error() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/account/api/oauth/token");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(r#"{"token_type": "bearer"}"#);
            })
            .await;

        let auth = auth_for(&server);
        assert!(matches!(
            auth.exchange_code("code").await,
            Err(Error::MissingField("access_token"))
        ));
    }
}
