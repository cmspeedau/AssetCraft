//! Where the token lives between runs.
//!
//! The interface is a trait so that the storage backend can change without
//! touching callers. The default ([`DefaultTokenStore`]) prefers the OS
//! keyring — the macOS Keychain, the Windows Credential Manager, the Linux
//! D-Bus secret service — and falls back to [`FileTokenStore`], a private
//! JSON file, wherever no keyring is usable: CI, containers, and headless
//! Linux have no secret service to answer. A file token found while a
//! keyring is usable is promoted into the keyring, so the plaintext file is
//! a fallback and never a habit.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use assetcraft_core::{Config, Error, Result};

use crate::token::EpicToken;

/// Persists a single Epic token.
pub trait TokenStore {
    /// Read the stored token, if any.
    fn load(&self) -> Result<Option<EpicToken>>;

    /// Persist `token`, replacing any existing one.
    fn save(&self, token: &EpicToken) -> Result<()>;

    /// Remove the stored token. Succeeds even if none was stored.
    fn clear(&self) -> Result<()>;
}

/// A token store backed by a JSON file, created with owner-only permissions.
#[derive(Debug, Clone)]
pub struct FileTokenStore {
    path: PathBuf,
}

impl FileTokenStore {
    /// Store the token at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The file this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl TokenStore for FileTokenStore {
    fn load(&self) -> Result<Option<EpicToken>> {
        match fs::read_to_string(&self.path) {
            Ok(raw) => Ok(Some(serde_json::from_str(&raw)?)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(Error::Io(err)),
        }
    }

    fn save(&self, token: &EpicToken) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(token)?;
        write_private(&self.path, &raw)
    }

    fn clear(&self) -> Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(Error::Io(err)),
        }
    }
}

/// The entry the token is filed under in the OS keyring.
pub const KEYRING_SERVICE: &str = "AssetCraft";
/// The account name paired with [`KEYRING_SERVICE`] in the OS keyring.
pub const KEYRING_ACCOUNT: &str = "epic";

/// The slice of a credential store this module needs.
///
/// A trait so the keyring-or-file logic is testable with a fake: CI has no
/// secret service on Linux, and tests must not read or write the real
/// keychain on macOS and Windows.
trait KeyringSecrets: Send + Sync {
    /// Read the stored secret, `Ok(None)` when nothing is stored.
    fn get(&self) -> Result<Option<String>>;

    /// Store `secret`, replacing whatever was there.
    fn set(&self, secret: &str) -> Result<()>;

    /// Remove the stored secret. Succeeds even if none was stored.
    fn delete(&self) -> Result<()>;
}

/// The platform's credential store, via the `keyring` crate.
struct NativeKeyring {
    entry: keyring::Entry,
}

impl KeyringSecrets for NativeKeyring {
    fn get(&self) -> Result<Option<String>> {
        match self.entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(Error::Other(format!("keyring read failed: {err}"))),
        }
    }

    fn set(&self, secret: &str) -> Result<()> {
        self.entry
            .set_password(secret)
            .map_err(|err| Error::Other(format!("keyring write failed: {err}")))
    }

    fn delete(&self) -> Result<()> {
        match self.entry.delete_credential() {
            Ok(()) => Ok(()),
            // Clearing what was never stored is exactly what clear means.
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(Error::Other(format!("keyring delete failed: {err}"))),
        }
    }
}

/// The store the CLI and the GUI actually use: the OS keyring when one is
/// usable, the private file when it is not.
///
/// Every keyring failure — no secret service answering, a locked collection,
/// a platform with no credential store at all — is logged and falls back to
/// the file, so signing in works wherever AssetCraft runs. Loading checks the
/// keyring first and falls back to the file; a file token found while the
/// keyring answers cleanly is promoted into the keyring and deleted from
/// disk, so a token acquired on a headless box migrates the first time the
/// app runs where a keyring exists.
#[derive(Clone)]
pub struct DefaultTokenStore {
    file: FileTokenStore,
    keyring: Option<Arc<dyn KeyringSecrets>>,
}

impl DefaultTokenStore {
    /// The store [`Config`] describes: the OS keyring under
    /// [`KEYRING_SERVICE`]/[`KEYRING_ACCOUNT`], falling back to the token
    /// file. Building the keyring entry is where a platform with no
    /// credential store says so, once, up front.
    pub fn from_config(config: &Config) -> Self {
        let keyring = match keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
            Ok(entry) => Some(Arc::new(NativeKeyring { entry }) as Arc<dyn KeyringSecrets>),
            Err(err) => {
                tracing::debug!("no OS keyring to file the token under: {err}");
                None
            }
        };
        Self {
            file: FileTokenStore::new(config.paths.token_file()),
            keyring,
        }
    }

    /// A store wired to a caller-supplied keyring half: the seam tests use
    /// to drive the keyring-or-file logic with a fake.
    #[cfg(test)]
    fn with_keyring(path: impl Into<PathBuf>, keyring: Option<Arc<dyn KeyringSecrets>>) -> Self {
        Self {
            file: FileTokenStore::new(path),
            keyring,
        }
    }

    /// Where the token is kept right now, as a phrase for a message like
    /// "Token stored in ...": the keyring if it holds one, else the file.
    pub fn location(&self) -> String {
        let in_keyring = matches!(
            self.keyring.as_ref().map(|keyring| keyring.get()),
            Some(Ok(Some(_)))
        );
        if in_keyring {
            format!("the OS keyring ({KEYRING_SERVICE} / {KEYRING_ACCOUNT})")
        } else {
            self.file.path().display().to_string()
        }
    }
}

impl TokenStore for DefaultTokenStore {
    fn load(&self) -> Result<Option<EpicToken>> {
        // `keyring_empty` tracks "the keyring answered and holds nothing",
        // the one state a file token may be promoted from.
        let mut keyring_empty = false;
        if let Some(keyring) = &self.keyring {
            match keyring.get() {
                Ok(Some(raw)) => match serde_json::from_str(&raw) {
                    Ok(token) => return Ok(Some(token)),
                    Err(err) => tracing::warn!(
                        "the keyring token is unreadable; falling back to the file: {err}"
                    ),
                },
                Ok(None) => keyring_empty = true,
                Err(err) => tracing::warn!(
                    "the OS keyring is not answering; falling back to the token file: {err}"
                ),
            }
        }

        let Some(token) = self.file.load()? else {
            return Ok(None);
        };
        if keyring_empty {
            let keyring = self.keyring.as_ref().expect("keyring_empty implies one");
            match serde_json::to_string(&token)
                .map_err(Error::from)
                .and_then(|raw| keyring.set(&raw))
            {
                Ok(()) => {
                    if let Err(err) = self.file.clear() {
                        tracing::warn!("could not remove the plaintext token file: {err}");
                    }
                }
                Err(err) => tracing::warn!("could not move the token into the keyring: {err}"),
            }
        }
        Ok(Some(token))
    }

    fn save(&self, token: &EpicToken) -> Result<()> {
        let raw = serde_json::to_string(token)?;
        if let Some(keyring) = &self.keyring {
            match keyring.set(&raw) {
                Ok(()) => {
                    // A token from an earlier keyring-less run must not
                    // linger on disk next to the keyring copy.
                    if let Err(err) = self.file.clear() {
                        tracing::warn!("could not remove the plaintext token file: {err}");
                    }
                    return Ok(());
                }
                Err(err) => tracing::warn!(
                    "the OS keyring refused the token; keeping the private file: {err}"
                ),
            }
        }
        self.file.save(token)
    }

    fn clear(&self) -> Result<()> {
        // Both halves, always: a keyring that only answers now and then must
        // not keep a stale token alive through whichever half missed it.
        if let Some(keyring) = &self.keyring {
            if let Err(err) = keyring.delete() {
                tracing::warn!("could not clear the keyring token: {err}");
            }
        }
        self.file.clear()
    }
}

/// Write `contents` to `path`, creating it with owner-only access on Unix.
///
/// Using `OpenOptions::mode` avoids the window where a plain write would leave
/// the token world-readable before a follow-up `chmod`.
#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents.as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::sync::Mutex;

    fn sample_token() -> EpicToken {
        EpicToken {
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            token_type: "bearer".to_string(),
            expires_at: Some(Utc::now()),
            account_id: Some("account-1".to_string()),
            display_name: Some("Tester".to_string()),
        }
    }

    #[test]
    fn load_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileTokenStore::new(dir.path().join("token.json"));
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileTokenStore::new(dir.path().join("nested").join("token.json"));
        let token = sample_token();
        store.save(&token).unwrap();
        assert_eq!(store.load().unwrap(), Some(token));
    }

    #[test]
    fn clear_removes_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileTokenStore::new(dir.path().join("token.json"));
        store.save(&sample_token()).unwrap();
        store.clear().unwrap();
        assert!(store.load().unwrap().is_none());
        // Clearing again is still fine.
        store.clear().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn saved_token_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let store = FileTokenStore::new(dir.path().join("token.json"));
        store.save(&sample_token()).unwrap();
        let mode = fs::metadata(store.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "token file should be owner-only");
    }

    #[test]
    fn corrupt_file_is_reported_as_a_decode_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        fs::write(&path, "not json").unwrap();
        let store = FileTokenStore::new(path);
        assert!(matches!(store.load(), Err(Error::Decode(_))));
    }

    /// A keyring double: holds the secret in memory, and when broken refuses
    /// every call — like a machine with no secret service answering, or a
    /// locked keychain.
    struct FakeKeyring {
        stored: Mutex<Option<String>>,
        broken: bool,
    }

    impl FakeKeyring {
        fn working() -> Arc<Self> {
            Arc::new(Self {
                stored: Mutex::new(None),
                broken: false,
            })
        }

        fn broken() -> Arc<Self> {
            Arc::new(Self {
                stored: Mutex::new(None),
                broken: true,
            })
        }

        fn with_secret(secret: &str) -> Arc<Self> {
            Arc::new(Self {
                stored: Mutex::new(Some(secret.to_string())),
                broken: false,
            })
        }

        fn peek(&self) -> Option<String> {
            self.stored
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    fn refused<T>() -> Result<T> {
        Err(Error::Other(
            "the secret service is not answering".to_string(),
        ))
    }

    impl KeyringSecrets for FakeKeyring {
        fn get(&self) -> Result<Option<String>> {
            if self.broken {
                return refused();
            }
            Ok(self.peek())
        }

        fn set(&self, secret: &str) -> Result<()> {
            if self.broken {
                return refused();
            }
            *self.stored.lock().unwrap_or_else(|p| p.into_inner()) = Some(secret.to_string());
            Ok(())
        }

        fn delete(&self) -> Result<()> {
            if self.broken {
                return refused();
            }
            *self.stored.lock().unwrap_or_else(|p| p.into_inner()) = None;
            Ok(())
        }
    }

    fn store_at(dir: &tempfile::TempDir, keyring: Option<Arc<FakeKeyring>>) -> DefaultTokenStore {
        DefaultTokenStore::with_keyring(
            dir.path().join("token.json"),
            keyring.map(|fake| fake as Arc<dyn KeyringSecrets>),
        )
    }

    fn encoded(token: &EpicToken) -> String {
        serde_json::to_string(token).unwrap()
    }

    #[test]
    fn save_prefers_the_keyring_and_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = FakeKeyring::working();
        let store = store_at(&dir, Some(keyring.clone()));
        let token = sample_token();
        store.save(&token).unwrap();
        assert_eq!(keyring.peek(), Some(encoded(&token)));
        assert!(!dir.path().join("token.json").exists());
    }

    #[test]
    fn save_falls_back_to_the_file_when_the_keyring_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = FakeKeyring::broken();
        let store = store_at(&dir, Some(keyring.clone()));
        let token = sample_token();
        store.save(&token).unwrap();
        assert_eq!(keyring.peek(), None);
        let on_disk = FileTokenStore::new(dir.path().join("token.json"))
            .load()
            .unwrap();
        assert_eq!(on_disk, Some(token));
    }

    #[test]
    fn load_prefers_the_keyring_over_a_stale_file() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = FakeKeyring::with_secret(&encoded(&sample_token()));
        let store = store_at(&dir, Some(keyring));
        let stale = EpicToken {
            access_token: "stale".to_string(),
            ..sample_token()
        };
        FileTokenStore::new(dir.path().join("token.json"))
            .save(&stale)
            .unwrap();
        let loaded = store.load().unwrap().unwrap();
        assert_eq!(loaded.access_token, "access");
    }

    #[test]
    fn a_file_token_is_promoted_into_a_working_keyring() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = FakeKeyring::working();
        let store = store_at(&dir, Some(keyring.clone()));
        let token = sample_token();
        FileTokenStore::new(dir.path().join("token.json"))
            .save(&token)
            .unwrap();
        assert_eq!(store.load().unwrap(), Some(token.clone()));
        // The plaintext is gone and the keyring copy took its place.
        assert!(!dir.path().join("token.json").exists());
        assert_eq!(keyring.peek(), Some(encoded(&token)));
    }

    #[test]
    fn load_falls_back_to_the_file_when_the_keyring_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = FakeKeyring::broken();
        let store = store_at(&dir, Some(keyring.clone()));
        let token = sample_token();
        FileTokenStore::new(dir.path().join("token.json"))
            .save(&token)
            .unwrap();
        assert_eq!(store.load().unwrap(), Some(token));
        // A refusing keyring must not have consumed the file.
        assert!(dir.path().join("token.json").exists());
    }

    #[test]
    fn a_corrupt_keyring_token_falls_back_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = FakeKeyring::with_secret("not json at all");
        let store = store_at(&dir, Some(keyring));
        let token = sample_token();
        FileTokenStore::new(dir.path().join("token.json"))
            .save(&token)
            .unwrap();
        assert_eq!(store.load().unwrap(), Some(token));
    }

    #[test]
    fn clear_empties_both_halves() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = FakeKeyring::working();
        let store = store_at(&dir, Some(keyring.clone()));
        store.save(&sample_token()).unwrap();
        // Also park a stale file: clear must reach both.
        FileTokenStore::new(dir.path().join("token.json"))
            .save(&sample_token())
            .unwrap();
        store.clear().unwrap();
        assert_eq!(keyring.peek(), None);
        assert!(!dir.path().join("token.json").exists());
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn a_refusing_keyring_does_not_block_clearing_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir, Some(FakeKeyring::broken()));
        FileTokenStore::new(dir.path().join("token.json"))
            .save(&sample_token())
            .unwrap();
        store.clear().unwrap();
        assert!(!dir.path().join("token.json").exists());
    }

    #[test]
    fn load_reports_none_when_neither_half_holds_a_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir, Some(FakeKeyring::working()));
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn keyring_and_file_round_trip_the_whole_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir, Some(FakeKeyring::working()));
        let token = sample_token();
        store.save(&token).unwrap();
        assert_eq!(store.load().unwrap(), Some(token));
    }

    #[test]
    fn location_names_the_keyring_when_it_holds_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(&dir, Some(FakeKeyring::working()));
        store.save(&sample_token()).unwrap();
        assert!(store.location().contains("OS keyring"));

        let on_disk = store_at(&dir, Some(FakeKeyring::broken()));
        on_disk.save(&sample_token()).unwrap();
        assert!(on_disk.location().ends_with("token.json"));
    }

    /// Round-trips a token through the *real* OS credential store, under a
    /// scratch service name so it can never touch a real sign-in.
    ///
    /// Ignored because CI must not touch a real keychain, and because the
    /// honest answer depends on the machine: a headless Linux box with no
    /// secret service is supposed to fall back to the file. Run it manually
    /// on each platform (`cargo test -p assetcraft-auth -- --ignored`) to
    /// confirm the keyring answers for the service and account AssetCraft
    /// files under.
    #[test]
    #[ignore = "touches the real OS keyring; run manually per platform"]
    fn the_real_keyring_round_trips_a_token() {
        // Report, without writing, how the real service answers.
        println!(
            "store status for {KEYRING_SERVICE:?}/{KEYRING_ACCOUNT:?}: {:?}",
            keyring::Entry::store_status()
        );

        let keyring = match keyring::Entry::new("AssetCraft-smoke", "smoke-test") {
            Ok(entry) => Arc::new(NativeKeyring { entry }) as Arc<dyn KeyringSecrets>,
            Err(err) => {
                println!("no OS keyring to round-trip through here: {err}");
                return;
            }
        };
        // Whatever answers for the scratch service, the store contract must
        // hold against it.
        let dir = tempfile::tempdir().unwrap();
        let store = DefaultTokenStore::with_keyring(dir.path().join("token.json"), Some(keyring));
        let token = sample_token();
        store.save(&token).unwrap();
        println!("token kept in: {}", store.location());
        assert_eq!(store.load().unwrap(), Some(token.clone()));
        store.clear().unwrap();
        assert!(store.load().unwrap().is_none());
    }
}
