//! Runtime configuration and platform directories.

use std::path::PathBuf;

use directories::ProjectDirs;

use crate::error::{Error, Result};

/// Default base URL for the Fab marketplace API.
pub const DEFAULT_FAB_BASE_URL: &str = "https://www.fab.com";

/// Default base URL for Epic's account/OAuth service.
pub const DEFAULT_ACCOUNT_BASE_URL: &str = "https://account-public-service-prod03.ol.epicgames.com";

/// Default base URL for the Epic web id service.
pub const DEFAULT_EPIC_ID_BASE_URL: &str = "https://www.epicgames.com";

/// User agent sent with every request. Fab sits behind a bot filter that
/// rejects some default clients, so a descriptive agent matters.
pub const DEFAULT_USER_AGENT: &str = concat!(
    "AssetCraft/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/cmspeedau/AssetCraft)"
);

/// Platform directories AssetCraft reads and writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Settings and credentials.
    pub config_dir: PathBuf,
    /// Cached API responses and thumbnails.
    pub cache_dir: PathBuf,
    /// The local asset library.
    pub data_dir: PathBuf,
}

impl Paths {
    /// Build a set of paths rooted at a single directory.
    ///
    /// Useful in tests, where writing to the real user directories would be
    /// surprising.
    pub fn rooted_at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            config_dir: root.join("config"),
            cache_dir: root.join("cache"),
            data_dir: root.join("data"),
        }
    }

    /// Where the Epic access token is stored.
    pub fn token_file(&self) -> PathBuf {
        self.config_dir.join("token.json")
    }

    /// The SQLite database holding the local asset library.
    pub fn library_file(&self) -> PathBuf {
        self.data_dir.join("library.sqlite")
    }

    /// Where cached API responses live.
    pub fn response_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("api")
    }

    /// Where cached thumbnails and previews live.
    pub fn thumbnail_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("thumbnails")
    }

    /// Where downloaded assets are laid out, one directory per asset id.
    ///
    /// This is the standard home for a download's reassembled files: the place
    /// [`Self::asset_dir`] points an asset at, and the default a `download`
    /// writes to and an `export` reads from. Keeping it under `data_dir` rather
    /// than `cache_dir` marks the files as something the user owns and keeps,
    /// not disposable cache.
    pub fn assets_dir(&self) -> PathBuf {
        self.data_dir.join("assets")
    }

    /// The directory one asset's downloaded files live in.
    pub fn asset_dir(&self, asset_id: &str) -> PathBuf {
        self.assets_dir().join(asset_id)
    }
}

/// Everything needed to talk to the outside world and to store local state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Base URL for the Fab marketplace API.
    pub fab_base_url: String,
    /// Base URL for Epic's account/OAuth service.
    pub account_base_url: String,
    /// Base URL for the Epic web id service (used to start a login).
    pub epic_id_base_url: String,
    /// User agent sent with every request.
    pub user_agent: String,
    /// Platform directories.
    pub paths: Paths,
}

impl Config {
    /// Load configuration using the platform's standard directories.
    pub fn load() -> Result<Self> {
        let dirs = ProjectDirs::from("dev", "AssetCraft", "AssetCraft").ok_or_else(|| {
            Error::Config("could not determine platform data directories".to_string())
        })?;
        Ok(Self {
            fab_base_url: DEFAULT_FAB_BASE_URL.to_string(),
            account_base_url: DEFAULT_ACCOUNT_BASE_URL.to_string(),
            epic_id_base_url: DEFAULT_EPIC_ID_BASE_URL.to_string(),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            paths: Paths {
                config_dir: dirs.config_dir().to_path_buf(),
                cache_dir: dirs.cache_dir().to_path_buf(),
                data_dir: dirs.data_dir().to_path_buf(),
            },
        })
    }

    /// Build a configuration rooted at an explicit directory.
    pub fn rooted_at(root: impl Into<PathBuf>) -> Self {
        Self {
            fab_base_url: DEFAULT_FAB_BASE_URL.to_string(),
            account_base_url: DEFAULT_ACCOUNT_BASE_URL.to_string(),
            epic_id_base_url: DEFAULT_EPIC_ID_BASE_URL.to_string(),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            paths: Paths::rooted_at(root),
        }
    }

    /// A user agent suitable for a `reqwest` default header.
    pub fn user_agent_header(&self) -> Result<reqwest::header::HeaderValue> {
        reqwest::header::HeaderValue::from_str(&self.user_agent)
            .map_err(|_| Error::Config(format!("invalid user agent: {}", self.user_agent)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooted_at_places_paths_under_the_root() {
        let paths = Paths::rooted_at("/tmp/assetcraft-test");
        assert_eq!(
            paths.config_dir,
            PathBuf::from("/tmp/assetcraft-test/config")
        );
        assert_eq!(paths.cache_dir, PathBuf::from("/tmp/assetcraft-test/cache"));
        assert_eq!(paths.data_dir, PathBuf::from("/tmp/assetcraft-test/data"));
        assert_eq!(
            paths.token_file(),
            PathBuf::from("/tmp/assetcraft-test/config/token.json")
        );
        assert_eq!(
            paths.library_file(),
            PathBuf::from("/tmp/assetcraft-test/data/library.sqlite")
        );
        assert_eq!(
            paths.response_cache_dir(),
            PathBuf::from("/tmp/assetcraft-test/cache/api")
        );
        assert_eq!(
            paths.thumbnail_cache_dir(),
            PathBuf::from("/tmp/assetcraft-test/cache/thumbnails")
        );
        assert_eq!(
            paths.assets_dir(),
            PathBuf::from("/tmp/assetcraft-test/data/assets")
        );
        assert_eq!(
            paths.asset_dir("rock-1"),
            PathBuf::from("/tmp/assetcraft-test/data/assets/rock-1")
        );
    }

    #[test]
    fn root_config_points_at_the_public_endpoints() {
        let config = Config::rooted_at("/tmp/assetcraft-test");
        assert_eq!(config.fab_base_url, DEFAULT_FAB_BASE_URL);
        assert_eq!(config.account_base_url, DEFAULT_ACCOUNT_BASE_URL);
        assert!(config.user_agent.starts_with("AssetCraft/"));
    }

    #[test]
    fn load_uses_platform_directories() {
        // Exercises the real `directories` lookup; it should always resolve on
        // a supported platform.
        let config = Config::load().expect("platform directories should resolve");
        assert!(config.paths.config_dir.is_absolute());
    }
}
