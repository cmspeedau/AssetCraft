//! Authentication with Epic Games for access to Fab.
//!
//! M1 ships the pieces needed to sign in and keep a session alive; the
//! browser-based device flow and OS keyring storage land in a later pass. Today
//! this crate provides:
//!
//! - [`EpicToken`], the credential the rest of the app carries,
//! - [`TokenStore`] with a private, file-backed default ([`FileTokenStore`]),
//! - [`EpicAuth`], which exchanges an authorization code for a token and
//!   refreshes it.
//!
//! > **Note on secrets.** The client credentials below are Epic's public
//! > *launcher* credentials, embedded in every Epic Games Launcher install.
//! > They identify the client application, not a user, and are not a secret.

pub mod epic;
pub mod store;
pub mod token;

pub use epic::{EpicAuth, LAUNCHER_CLIENT_ID};
pub use store::{FileTokenStore, TokenStore};
pub use token::EpicToken;
