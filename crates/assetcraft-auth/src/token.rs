//! The credential the rest of the app carries around.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// An Epic Games OAuth token.
///
/// The access token is short-lived; the refresh token is what keeps a session
/// alive across runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpicToken {
    /// Bearer token for API calls.
    pub access_token: String,
    /// Token used to obtain a fresh access token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Token scheme, normally `bearer`.
    pub token_type: String,
    /// When the access token stops being valid, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Epic account id, when the response included one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// Display name, for showing who is signed in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

impl EpicToken {
    /// Whether the access token is past its expiry.
    ///
    /// An unknown expiry is treated as still valid: we would rather try the
    /// token and handle a 401 than reject a working session.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        match self.expires_at {
            Some(expiry) => expiry <= now,
            None => false,
        }
    }

    /// Whether the access token expires within `seconds`.
    ///
    /// Used to refresh slightly ahead of expiry rather than racing it.
    pub fn expires_within(&self, now: DateTime<Utc>, seconds: i64) -> bool {
        match self.expires_at {
            Some(expiry) => expiry <= now + Duration::seconds(seconds),
            None => false,
        }
    }

    /// The `Authorization` header value for this token.
    pub fn authorization_header(&self) -> String {
        format!("{} {}", self.token_type, self.access_token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_expiring_at(expires_at: Option<DateTime<Utc>>) -> EpicToken {
        EpicToken {
            access_token: "abc".to_string(),
            refresh_token: Some("refresh".to_string()),
            token_type: "bearer".to_string(),
            expires_at,
            account_id: None,
            display_name: None,
        }
    }

    #[test]
    fn expired_when_past_expiry() {
        let now = Utc::now();
        let token = token_expiring_at(Some(now - Duration::minutes(1)));
        assert!(token.is_expired(now));
    }

    #[test]
    fn valid_before_expiry() {
        let now = Utc::now();
        let token = token_expiring_at(Some(now + Duration::hours(1)));
        assert!(!token.is_expired(now));
    }

    #[test]
    fn unknown_expiry_is_treated_as_valid() {
        let token = token_expiring_at(None);
        assert!(!token.is_expired(Utc::now()));
        assert!(!token.expires_within(Utc::now(), 3600));
    }

    #[test]
    fn expires_within_looks_ahead() {
        let now = Utc::now();
        let token = token_expiring_at(Some(now + Duration::minutes(5)));
        assert!(token.expires_within(now, 600));
        assert!(!token.expires_within(now, 60));
    }

    #[test]
    fn authorization_header_uses_the_scheme() {
        let token = token_expiring_at(None);
        assert_eq!(token.authorization_header(), "bearer abc");
    }

    #[test]
    fn round_trips_through_json() {
        let token = token_expiring_at(Some(Utc::now()));
        let json = serde_json::to_string(&token).unwrap();
        let back: EpicToken = serde_json::from_str(&json).unwrap();
        assert_eq!(token, back);
    }
}
