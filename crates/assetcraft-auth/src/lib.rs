//! Authentication with Epic Games for access to Fab.
//!
//! M1 ships the pieces needed to sign in and keep a session alive. This crate
//! provides:
//!
//! - [`EpicToken`], the credential the rest of the app carries,
//! - [`TokenStore`] with a default ([`DefaultTokenStore`]) that prefers the
//!   OS keyring and falls back to a private file ([`FileTokenStore`]) where
//!   no keyring is usable (CI, containers, headless Linux),
//! - [`EpicAuth`], which exchanges an authorization code for a token and
//!   refreshes it,
//! - [`ensure_fresh_token`], which loads the stored token and refreshes it when
//!   it is close to expiring.
//!
//! See `docs/architecture.md` for how the keyring preference and its fallback
//! are decided.
//!
//! > **Note on secrets.** The client credentials below are Epic's public
//! > *launcher* credentials, embedded in every Epic Games Launcher install.
//! > They identify the client application, not a user, and are not a secret.
//!
//! > **Note on the device flow.** Epic's OAuth device-authorization grant is not
//! > available to the launcher client (the endpoint answers `401` for those
//! > credentials), so the browser authorization-code flow is the one this crate
//! > implements.

pub mod epic;
pub mod session;
pub mod store;
pub mod token;

pub use epic::{extract_authorization_code, EpicAuth, LAUNCHER_CLIENT_ID};
pub use session::ensure_fresh_token;
pub use store::{DefaultTokenStore, FileTokenStore, TokenStore, KEYRING_ACCOUNT, KEYRING_SERVICE};
pub use token::EpicToken;
