//! Runtime configuration and platform directories.

use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Default base URL for the Fab marketplace API.
pub const DEFAULT_FAB_BASE_URL: &str = "https://www.fab.com";

/// Default base URL for Epic's account/OAuth service.
pub const DEFAULT_ACCOUNT_BASE_URL: &str = "https://account-public-service-prod03.ol.epicgames.com";

/// Default base URL for the Epic web id service.
pub const DEFAULT_EPIC_ID_BASE_URL: &str = "https://www.epicgames.com";

/// Default update feed: this repository's GitHub Releases, newest page first.
/// The feed is an API URL rather than the release *pages* because the API is
/// what carries each asset's computed `sha256` digest — the thing that makes
/// an update safe to run.
pub const DEFAULT_UPDATE_FEED_URL: &str =
    "https://api.github.com/repos/cmspeedau/AssetCraft/releases?per_page=100";

/// The version every AssetCraft binary reports and updates are judged
/// against. Every crate inherits the workspace version, so one constant
/// describes both the CLI and the GUI.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

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

    /// Where a downloaded-but-not-yet-installed update binary is staged
    /// before it is swapped over the running executable.
    pub fn updates_dir(&self) -> PathBuf {
        self.cache_dir.join("updates")
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

    /// Where DCC exports are laid out, one `<asset id>/<format>` directory per
    /// export under this root.
    ///
    /// This is the standard export root: the app writes there, and the Blender
    /// addon browses there, which is what lets an export made in the app show up
    /// in the addon's list without the user pointing anything at anything.
    pub fn exports_dir(&self) -> PathBuf {
        self.data_dir.join("exports")
    }

    /// The user's own choices, as a TOML file beside the other config.
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
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
    /// Where to look for release announcements: GitHub's Releases API for a
    /// repository, or a mirror of that same JSON shape.
    pub update_feed_url: String,
    /// User agent sent with every request.
    pub user_agent: String,
    /// Platform directories.
    pub paths: Paths,
}

impl Config {
    /// Load configuration using the platform's standard directories.
    ///
    /// The three service base URLs each take an environment override
    /// (`ASSETCRAFT_FAB_BASE_URL`, `ASSETCRAFT_ACCOUNT_BASE_URL`,
    /// `ASSETCRAFT_EPIC_ID_BASE_URL`). That is how the app, the CLI, and the
    /// screenshot driver run against a local stand-in for Fab and Epic
    /// without changing a build.
    pub fn load() -> Result<Self> {
        let dirs = ProjectDirs::from("dev", "AssetCraft", "AssetCraft").ok_or_else(|| {
            Error::Config("could not determine platform data directories".to_string())
        })?;
        Ok(Self {
            fab_base_url: env_override("ASSETCRAFT_FAB_BASE_URL", DEFAULT_FAB_BASE_URL),
            account_base_url: env_override("ASSETCRAFT_ACCOUNT_BASE_URL", DEFAULT_ACCOUNT_BASE_URL),
            epic_id_base_url: env_override("ASSETCRAFT_EPIC_ID_BASE_URL", DEFAULT_EPIC_ID_BASE_URL),
            update_feed_url: env_override("ASSETCRAFT_UPDATE_FEED", DEFAULT_UPDATE_FEED_URL),
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
            update_feed_url: DEFAULT_UPDATE_FEED_URL.to_string(),
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

/// A base URL the environment may point somewhere else, for testing.
fn env_override(var: &str, default: &str) -> String {
    match std::env::var(var) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => default.to_string(),
    }
}

/// The user's own choices, persisted as TOML in [`Paths::settings_file`].
///
/// This is the settings file the README's configuration table promised, with
/// the format it assumed: TOML. Every field is optional — the file describes
/// departures from the defaults, not a full description of the app — and both
/// binaries read it at the point of use, so a change takes effect for the next
/// download without either program being restarted or notified.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Quality tier wanted by default, as the app's resolution picker words
    /// it (`2K`, `4K`, `8K`, `16K`); matched against file names the same way
    /// `--quality` is. `None` means "whatever the asset offers first".
    pub default_quality: Option<String>,
    /// Where downloads land, one directory per asset id under this root.
    /// `None` means [`Paths::assets_dir`]: the standard place.
    pub download_dir: Option<PathBuf>,
    /// Whether the app asks its update feed whether a newer release exists
    /// when it starts. `None` is the promise the README makes — yes, it
    /// checks; `Some(false)` is the user opting out. The check never
    /// installs anything by itself.
    pub auto_update: Option<bool>,
}

impl Settings {
    /// Read the settings file.
    ///
    /// A missing file is not a failure — settings are what the user chose, and
    /// a user who chose nothing has chosen the defaults. A file that exists but
    /// cannot be understood is reported, never silently ignored: defaulting
    /// around a typo would quietly undo the user's choices.
    pub fn load(paths: &Paths) -> Result<Self> {
        let path = paths.settings_file();
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|error| {
                Error::Config(format!(
                    "{} is not readable settings: {error}. Fix or delete the file.",
                    path.display()
                ))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(Error::Config(format!(
                "could not read {}: {error}",
                path.display()
            ))),
        }
    }

    /// Write the settings file, creating the config directory if needed.
    pub fn save(&self, paths: &Paths) -> Result<()> {
        let path = paths.settings_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|error| Error::Config(format!("could not render settings: {error}")))?;
        std::fs::write(&path, text)?;
        Ok(())
    }

    /// Where one asset's files belong: the chosen root, or the standard one.
    ///
    /// Resolution order deliberately stops at the *record* first for
    /// re-downloads (the CLI prefers the recorded path): this answers only
    /// "where do files for a new asset go".
    pub fn asset_dir_for(&self, paths: &Paths, asset_id: &str) -> PathBuf {
        match self.download_dir.as_deref() {
            Some(dir) => dir.join(asset_id),
            None => paths.asset_dir(asset_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_url_moves_only_when_the_env_names_it() {
        // A variable no other test reads, so the process-wide env is safe here.
        std::env::set_var("ASSETCRAFT_TEST_OVERRIDE_URL", "http://127.0.0.1:9/x");
        assert_eq!(
            env_override("ASSETCRAFT_TEST_OVERRIDE_URL", "https://fab.com"),
            "http://127.0.0.1:9/x"
        );
        // Blank is not a pointer somewhere else: the default stands.
        std::env::set_var("ASSETCRAFT_TEST_OVERRIDE_URL", "  ");
        assert_eq!(
            env_override("ASSETCRAFT_TEST_OVERRIDE_URL", "https://fab.com"),
            "https://fab.com"
        );
        std::env::remove_var("ASSETCRAFT_TEST_OVERRIDE_URL");
        assert_eq!(
            env_override("ASSETCRAFT_TEST_OVERRIDE_URL", "https://fab.com"),
            "https://fab.com"
        );
    }

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
            paths.updates_dir(),
            PathBuf::from("/tmp/assetcraft-test/cache/updates")
        );
        assert_eq!(
            paths.assets_dir(),
            PathBuf::from("/tmp/assetcraft-test/data/assets")
        );
        assert_eq!(
            paths.asset_dir("rock-1"),
            PathBuf::from("/tmp/assetcraft-test/data/assets/rock-1")
        );
        assert_eq!(
            paths.exports_dir(),
            PathBuf::from("/tmp/assetcraft-test/data/exports")
        );
        assert_eq!(
            paths.settings_file(),
            PathBuf::from("/tmp/assetcraft-test/config/settings.toml")
        );
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(root.path());
        let settings = Settings {
            default_quality: Some("4K".to_string()),
            download_dir: Some(PathBuf::from("/media/assets")),
            auto_update: Some(false),
        };
        settings.save(&paths).unwrap();
        // The file says what it is, and stays hand-editable.
        let text = std::fs::read_to_string(paths.settings_file()).unwrap();
        assert!(text.contains("default_quality"), "{text}");
        assert!(text.contains("auto_update"), "{text}");
        assert_eq!(Settings::load(&paths).unwrap(), settings);
    }

    #[test]
    fn a_partial_or_missing_settings_file_is_the_defaults() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(root.path());
        // Nothing on disk at all: defaults, not an error.
        assert_eq!(Settings::load(&paths).unwrap(), Settings::default());

        // One field set is enough: the rest keeps its default. `#[serde
        // (default)]` is what makes the file a list of departures rather
        // than a full description the app demands.
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(paths.settings_file(), "default_quality = \"8K\"\n").unwrap();
        let loaded = Settings::load(&paths).unwrap();
        assert_eq!(loaded.default_quality.as_deref(), Some("8K"));
        assert_eq!(loaded.download_dir, None);
    }

    #[test]
    fn a_corrupt_settings_file_is_reported_not_defaulted_around() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(root.path());
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(paths.settings_file(), "default_quality = ").unwrap();
        let error = Settings::load(&paths).unwrap_err().to_string();
        // The message names the file and says what to do: a silent default
        // would undo the user's choices with no clue why they came back.
        assert!(error.contains("settings.toml"), "{error}");
        assert!(error.contains("Fix or delete"), "{error}");
    }

    #[test]
    fn asset_dirs_follow_the_chosen_root() {
        let paths = Paths::rooted_at("/tmp/assetcraft-test");
        assert_eq!(
            Settings::default().asset_dir_for(&paths, "rock-1"),
            paths.asset_dir("rock-1"),
            "no choice, no change"
        );
        let settings = Settings {
            download_dir: Some(PathBuf::from("/media/assets")),
            ..Settings::default()
        };
        assert_eq!(
            settings.asset_dir_for(&paths, "rock-1"),
            PathBuf::from("/media/assets/rock-1")
        );
    }

    #[test]
    fn root_config_points_at_the_public_endpoints() {
        let config = Config::rooted_at("/tmp/assetcraft-test");
        assert_eq!(config.fab_base_url, DEFAULT_FAB_BASE_URL);
        assert_eq!(config.account_base_url, DEFAULT_ACCOUNT_BASE_URL);
        assert_eq!(config.update_feed_url, DEFAULT_UPDATE_FEED_URL);
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
