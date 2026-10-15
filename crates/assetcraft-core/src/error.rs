//! The shared error type.

use thiserror::Error;

/// Errors produced across AssetCraft.
///
/// The variants are deliberately coarse: callers usually only need to know
/// whether a call failed because of the network, a bad response, bad local
/// state, or a missing credential.
#[derive(Debug, Error)]
pub enum Error {
    /// The transport itself failed (DNS, TLS, connection, timeout).
    #[error("network request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The server answered, but not with success.
    #[error("server returned HTTP {status}: {body}")]
    HttpStatus {
        /// The HTTP status code.
        status: u16,
        /// A truncated copy of the response body, for diagnostics.
        body: String,
    },

    /// A response body could not be decoded into the expected shape.
    #[error("failed to decode response: {0}")]
    Decode(#[from] serde_json::Error),

    /// A response was missing a field the caller required.
    #[error("response was missing required field: {0}")]
    MissingField(&'static str),

    /// An operation needed credentials that are not present or have expired.
    #[error("not authenticated: {0}")]
    NotAuthenticated(String),

    /// Local filesystem trouble (reading a token, writing the library, ...).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Local database trouble (opening, querying, or a corrupt library).
    #[error("database error: {0}")]
    Database(String),

    /// The configuration could not be determined or was invalid.
    #[error("invalid configuration: {0}")]
    Config(String),

    /// A requested format or file could not be found for an asset.
    #[error("no match: {0}")]
    NoMatch(String),

    /// Anything else, with a human-readable explanation.
    #[error("{0}")]
    Other(String),
}

/// Convenience alias for results carrying [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Keep response bodies in errors from growing without bound.
pub(crate) fn truncate_body(body: &str) -> String {
    const LIMIT: usize = 500;
    if body.len() <= LIMIT {
        return body.to_string();
    }
    let mut end = LIMIT;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… ({} bytes total)", &body[..end], body.len())
}

/// Convert a non-success HTTP response into an [`Error`].
pub fn status_error(status: reqwest::StatusCode, body: &str) -> Error {
    Error::HttpStatus {
        status: status.as_u16(),
        body: truncate_body(body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_leaves_short_bodies_alone() {
        assert_eq!(truncate_body("short"), "short");
    }

    #[test]
    fn truncate_cuts_long_bodies_on_a_char_boundary() {
        let body = "é".repeat(400); // 800 bytes
        let out = truncate_body(&body);
        assert!(out.starts_with('é'));
        assert!(out.contains("bytes total"));
    }

    #[test]
    fn status_error_carries_the_code() {
        let err = status_error(reqwest::StatusCode::FORBIDDEN, "nope");
        match err {
            Error::HttpStatus { status, body } => {
                assert_eq!(status, 403);
                assert_eq!(body, "nope");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
