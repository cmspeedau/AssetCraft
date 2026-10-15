//! Where the token lives between runs.
//!
//! The interface is a trait so that the storage backend can change without
//! touching callers: today a private JSON file, later the OS keyring. The file
//! backend exists because the keyring needs a running secret service, which is
//! not available in every environment (CI, containers, headless Linux).

use std::fs;
use std::path::{Path, PathBuf};

use assetcraft_core::{Error, Result};

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
}
