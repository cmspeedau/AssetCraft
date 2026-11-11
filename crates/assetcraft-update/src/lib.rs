//! In-place self-update from GitHub Releases.
//!
//! The update channel is the repository's releases. Each release carries one
//! raw binary per product per platform, named so a running copy can find its
//! own replacement by exact name — no archive to unpack, because the asset
//! *is* the executable:
//!
//! ```text
//! assetcraft-cli-x86_64-unknown-linux-gnu          (`.exe` on Windows)
//! assetcraft-gui-aarch64-apple-darwin
//! ```
//!
//! The decision is pure: [`plan_update`] answers "which release, if any,
//! supersedes this version for this product on this platform" from the feed's
//! parsed JSON, so the rule — highest semver, never a draft, prereleases only
//! for prereleases, and only an asset with a verifiable checksum for this
//! exact target — is testable without a network. [`UpdateClient`] speaks the
//! feed (GitHub's Releases API, or a mirror of that same JSON shape at
//! [`Config::update_feed_url`]), downloads a chosen asset, and verifies it;
//! [`apply`] swaps a staged binary over the running executable with the
//! rename dance Windows requires for a live image.
//!
//! Nothing installs without a `sha256:` digest. GitHub computes a digest for
//! every asset it stores, so a release made through the workflow always has
//! one; a hand-made mirror that does not is not offering anything verifiable,
//! and an unverifiable binary is never downloaded, let alone run. Fail closed
//! is the whole feature.

use std::path::{Path, PathBuf};

use assetcraft_core::{Config, Error, Result};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

/// Which executable is updating itself. Both ship in every release, so a
/// release carries one asset per product per platform and each product picks
/// its own by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    /// The headless CLI.
    Cli,
    /// The desktop app.
    Gui,
}

impl Product {
    /// The executable's file name, without any path or suffix.
    pub fn bin_name(self) -> &'static str {
        match self {
            Product::Cli => "assetcraft",
            Product::Gui => "assetcraft-gui",
        }
    }

    /// The release asset name carrying this product on this target, exactly
    /// as the release workflow uploads it.
    pub fn asset_name(self, target: &str) -> String {
        let slug = match self {
            Product::Cli => "cli",
            Product::Gui => "gui",
        };
        format!("assetcraft-{slug}-{target}{}", std::env::consts::EXE_SUFFIX)
    }
}

/// The Rust target triple of this machine, in the spelling release assets
/// carry. `None` for a platform this project does not build for: there is
/// nothing to update *to*, which is not an error.
pub fn current_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        _ => None,
    }
}

/// One entry of the Releases API feed, as far as updating cares.
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    /// The tag the release was cut from, e.g. `v0.2.0`.
    pub tag_name: String,
    /// A draft is nobody's release yet; it is never offered.
    #[serde(default)]
    pub draft: bool,
    /// A prerelease is only ever offered to a prerelease.
    #[serde(default)]
    pub prerelease: bool,
    /// The web page a human can read the notes on.
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

/// One uploaded file of a release.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    /// Size as recorded at upload, used as a cheap first check on a download.
    #[serde(default)]
    pub size: u64,
    /// GitHub's own digest of the stored bytes, `sha256:<hex>`. Absent for
    /// servers that do not compute it — and then the asset is not offered.
    #[serde(default)]
    pub digest: Option<String>,
    pub browser_download_url: String,
}

/// What to install: one release's asset, its version, and what its bytes must
/// hash to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: Version,
    pub tag: String,
    /// The release page, for showing a human where this came from.
    pub url: String,
    pub asset: ReleaseAsset,
    pub sha256: [u8; 32],
}

/// The semver a tag names, tolerating the leading `v` every release uses.
fn version_of(tag: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()
}

/// GitHub's `sha256:<hex>` digest string as bytes, or `None` for anything
/// else — including a plain-`<hex>` or `md5:` value, which is not the digest
/// this crate can check.
fn parse_digest(digest: &str) -> Option<[u8; 32]> {
    let hex = digest.strip_prefix("sha256:")?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

/// The newest release that supersedes `current` for this product and target,
/// or `None` when nothing verifiable does.
///
/// A release is a candidate only when all of these hold: it is not a draft;
/// its tag parses as semver; a stable install is never handed a prerelease (a
/// prerelease install may be handed one); it carries the asset named exactly
/// for `product` on `target`; that asset carries a parseable `sha256:`
/// digest; and its version is above `current`. Prerelease parts compare by
/// semver's own rule, so `0.2.0-rc.1` beats `0.1.0` but loses to `0.2.0`.
pub fn plan_update(
    releases: &[Release],
    current: &Version,
    product: Product,
    target: &str,
) -> Option<Update> {
    let wanted = product.asset_name(target);
    releases
        .iter()
        .filter_map(|release| {
            if release.draft {
                return None;
            }
            let version = version_of(&release.tag_name)?;
            if release.prerelease && current.pre.is_empty() {
                return None;
            }
            if version <= *current {
                return None;
            }
            let asset = release.assets.iter().find(|asset| asset.name == wanted)?;
            let sha256 = parse_digest(asset.digest.as_deref()?)?;
            Some(Update {
                version,
                tag: release.tag_name.clone(),
                url: release.html_url.clone(),
                asset: asset.clone(),
                sha256,
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// Talks to the release feed and stages verified updates.
pub struct UpdateClient {
    http: reqwest::Client,
    feed_url: Url,
}

impl UpdateClient {
    /// A client reading releases from `feed_url`.
    pub fn new(feed_url: &str, http: reqwest::Client) -> Result<Self> {
        let feed_url = Url::parse(feed_url)
            .map_err(|err| Error::Config(format!("invalid update feed URL {feed_url:?}: {err}")))?;
        Ok(Self { http, feed_url })
    }

    /// A client for the feed `Config` names, with the shared user agent.
    pub fn from_config(config: &Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(config.user_agent.clone())
            .build()?;
        Self::new(&config.update_feed_url, http)
    }

    /// Ask the feed whether a release supersedes this build, for this
    /// product on this machine.
    pub async fn check(&self, product: Product) -> Result<Option<Update>> {
        let Some(target) = current_target() else {
            return Ok(None);
        };
        let current = version_of(assetcraft_core::VERSION).ok_or_else(|| {
            Error::Config(format!(
                "not a semver version: {}",
                assetcraft_core::VERSION
            ))
        })?;
        let response = self.http.get(self.feed_url.clone()).send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(Error::HttpStatus {
                status: status.as_u16(),
                body: truncate(&body),
            });
        }
        let releases: Vec<Release> = serde_json::from_str(&body).map_err(Error::from)?;
        Ok(plan_update(&releases, &current, product, target))
    }

    /// Fetch an update's binary into `staging_dir`, refusing anything that is
    /// not byte-for-byte the release's asset. Returns the staged file's path.
    pub async fn download(&self, update: &Update, staging_dir: &Path) -> Result<PathBuf> {
        let url = Url::parse(&update.asset.browser_download_url).map_err(|err| {
            Error::Config(format!(
                "invalid download URL {:?}: {err}",
                update.asset.browser_download_url
            ))
        })?;
        let response = self.http.get(url).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::HttpStatus {
                status: status.as_u16(),
                body: truncate(&body),
            });
        }
        let bytes = response.bytes().await?;
        if update.asset.size > 0 && bytes.len() as u64 != update.asset.size {
            return Err(Error::NoMatch(format!(
                "{} arrived as {} bytes, the release recorded {}",
                update.asset.name,
                bytes.len(),
                update.asset.size
            )));
        }
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        if digest != update.sha256 {
            return Err(Error::NoMatch(format!(
                "{} did not match its release checksum; the download was discarded",
                update.asset.name
            )));
        }
        std::fs::create_dir_all(staging_dir)?;
        let staged = staging_dir.join(&update.asset.name);
        std::fs::write(&staged, &bytes)?;
        Ok(staged)
    }
}

/// Swap a staged, verified binary over the executable that is running.
///
/// The staged file may sit on another device (the cache directory can), so it
/// is copied beside the executable first and every disruptive step after that
/// is a same-directory rename: the running image is renamed out of the way
/// (permitted even on Windows) and the new one renamed into its place. If
/// that second rename fails the first is undone, so a failure leaves the old
/// binary exactly where it was. The next process started at this path is the
/// new binary; the one running now keeps its own image until it exits.
pub fn apply(staged: &Path) -> Result<()> {
    let exe = std::env::current_exe()?;
    apply_at(staged, &exe)
}

/// [`apply`] with the destination spelled out, which is what the tests run.
fn apply_at(staged: &Path, exe: &Path) -> Result<()> {
    let tmp = sibling(exe, ".new");
    let old = sibling(exe, ".old");
    std::fs::copy(staged, &tmp)
        .map_err(|error| install_io_error("stage the new executable", &tmp, error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(
            |error| install_io_error("make the staged executable runnable", &tmp, error),
        )?;
    }
    if old.exists() {
        std::fs::remove_file(&old)
            .map_err(|error| install_io_error("remove the previous update backup", &old, error))?;
    }
    std::fs::rename(exe, &old)
        .map_err(|error| install_io_error("move the installed executable aside", exe, error))?;
    if let Err(error) = std::fs::rename(&tmp, exe) {
        // Put back what was there; the update failed, the app did not.
        let _ = std::fs::rename(&old, exe);
        return Err(install_io_error("install the new executable", exe, error));
    }
    // The replaced image unlinks at once on Unix; on Windows it is the still
    // running process's own file and goes when it exits. Neither is a failure.
    let _ = std::fs::remove_file(&old);
    Ok(())
}

/// Attach the install step, affected path, and a useful recovery hint to a
/// filesystem error. Packaged installs under `/usr`, `/opt`, or a mounted
/// AppImage cannot be replaced by an unprivileged self-update.
fn install_io_error(step: &str, path: &Path, error: std::io::Error) -> Error {
    Error::Io(std::io::Error::new(
        error.kind(),
        format!(
            "could not {step} at {}: {error}. The install location may be read-only; update it with your package manager or install AssetCraft in a user-writable location",
            path.display()
        ),
    ))
}

/// `path` with `suffix` appended to its file name, in the same directory.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// A body shortened to one diagnostic line.
fn truncate(body: &str) -> String {
    const LIMIT: usize = 256;
    if body.len() <= LIMIT {
        return body.to_string();
    }
    let mut end = LIMIT;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &body[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: &str = "x86_64-unknown-linux-gnu";

    fn asset(name: &str, digest: Option<&str>) -> ReleaseAsset {
        ReleaseAsset {
            name: name.to_owned(),
            size: 0,
            digest: digest.map(|d| d.to_owned()),
            browser_download_url: format!("https://example.invalid/{name}"),
        }
    }

    /// A hex digest GitHub's wire format would carry for `bytes`.
    fn digest_of(bytes: &[u8]) -> String {
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        format!("sha256:{hex}")
    }

    fn release(tag: &str, assets: Vec<ReleaseAsset>) -> Release {
        Release {
            tag_name: tag.to_owned(),
            draft: false,
            prerelease: false,
            html_url: format!("https://example.invalid/tag/{tag}"),
            assets,
        }
    }

    fn good(tag: &str, product: Product) -> Release {
        release(
            tag,
            vec![asset(
                &product.asset_name(TARGET),
                Some(&digest_of(b"binary")),
            )],
        )
    }

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn the_newest_superseding_release_wins() {
        let releases = vec![
            good("v0.2.0", Product::Cli),
            good("v0.3.0", Product::Cli),
            good("v0.1.0", Product::Cli),
        ];
        let update = plan_update(&releases, &version("0.1.0"), Product::Cli, TARGET).unwrap();
        assert_eq!(update.version, version("0.3.0"));
        assert_eq!(update.tag, "v0.3.0");
    }

    #[test]
    fn nothing_is_offered_when_the_running_version_is_already_newest() {
        let releases = vec![good("v0.2.0", Product::Cli), good("v0.2.1", Product::Cli)];
        assert!(plan_update(&releases, &version("0.2.1"), Product::Cli, TARGET).is_none());
    }

    #[test]
    fn drafts_prereleases_and_unparseable_tags_are_never_offered_to_stable() {
        let mut draft = good("v0.9.0", Product::Cli);
        draft.draft = true;
        let pre = Release {
            prerelease: true,
            ..good("v0.8.0", Product::Cli)
        };
        let odd = good("nightly-2026-11-08", Product::Cli);
        let releases = vec![draft, pre.clone(), odd];
        assert!(plan_update(&releases, &version("0.1.0"), Product::Cli, TARGET).is_none());

        // A prerelease install, though, is exactly who a prerelease is for.
        let releases = vec![pre];
        let update = plan_update(&releases, &version("0.8.0-rc.1"), Product::Cli, TARGET).unwrap();
        assert_eq!(update.tag, "v0.8.0");
    }

    #[test]
    fn an_asset_is_only_ever_the_running_product_on_the_running_target() {
        // The GUI's asset for this target is not something a CLI may install,
        // and the CLI's asset for another target is not something this
        // machine may install either.
        let gui_feed = vec![release(
            "v0.2.0",
            vec![asset(
                &Product::Gui.asset_name(TARGET),
                Some(&digest_of(b"binary")),
            )],
        )];
        assert!(plan_update(&gui_feed, &version("0.1.0"), Product::Cli, TARGET).is_none());
        let update = plan_update(&gui_feed, &version("0.1.0"), Product::Gui, TARGET).unwrap();
        assert_eq!(update.asset.name, Product::Gui.asset_name(TARGET));

        let other_target = vec![release(
            "v0.2.0",
            vec![asset(
                &Product::Cli.asset_name("aarch64-apple-darwin"),
                Some(&digest_of(b"binary")),
            )],
        )];
        assert!(plan_update(&other_target, &version("0.1.0"), Product::Cli, TARGET).is_none());
    }

    #[test]
    fn an_asset_without_a_verifiable_sha256_digest_is_not_offered() {
        // No digest at all (an old or non-GitHub feed), a foreign algorithm,
        // and a malformed hex run all fail the same way.
        for digest in [
            None,
            Some("md5:00000000"),
            Some("sha256:xyz"),
            Some("sha256:dead"),
        ] {
            let releases = vec![release(
                "v0.2.0",
                vec![asset(&Product::Cli.asset_name(TARGET), digest)],
            )];
            assert!(
                plan_update(&releases, &version("0.1.0"), Product::Cli, TARGET).is_none(),
                "digest {digest:?} must not be installable"
            );
        }
    }

    #[test]
    fn digests_parse_back_to_their_bytes() {
        let digest = digest_of(b"binary");
        let parsed = parse_digest(&digest).unwrap();
        assert_eq!(parsed, <[u8; 32]>::from(Sha256::digest(b"binary")));
    }

    #[test]
    fn product_asset_names_carry_product_target_and_suffix() {
        assert_eq!(
            Product::Cli.asset_name(TARGET),
            format!("assetcraft-cli-{TARGET}{}", std::env::consts::EXE_SUFFIX)
        );
        assert_eq!(
            Product::Gui.asset_name(TARGET),
            format!("assetcraft-gui-{TARGET}{}", std::env::consts::EXE_SUFFIX)
        );
        assert_eq!(Product::Gui.bin_name(), "assetcraft-gui");
    }

    #[test]
    fn install_errors_name_the_path_and_suggest_a_read_only_location_recovery() {
        let exe = Path::new("/opt/AssetCraft/assetcraft-gui");
        let error = install_io_error(
            "install the new executable",
            exe,
            std::io::Error::from_raw_os_error(30),
        );
        let message = error.to_string();

        assert!(message.contains(&exe.display().to_string()));
        assert!(message.contains("install the new executable"));
        assert!(message.contains("update it with your package manager"));
    }

    #[cfg(unix)]
    #[test]
    fn applying_moves_the_old_binary_aside_and_the_new_one_in() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("assetcraft-gui");
        std::fs::write(&exe, b"old binary").unwrap();
        let staged = dir.path().join("staged");
        std::fs::write(&staged, b"new binary").unwrap();

        apply_at(&staged, &exe).unwrap();

        assert_eq!(std::fs::read(&exe).unwrap(), b"new binary");
        // The replaced image is unlinked on Unix; nothing litters the side.
        assert!(!dir.path().join("assetcraft-gui.old").exists());
        assert!(!dir.path().join("assetcraft-gui.new").exists());
        // And the new file is executable, not merely present.
        use std::os::unix::fs::PermissionsExt;
        assert!(
            std::fs::metadata(&exe).unwrap().permissions().mode() & 0o100 != 0,
            "a binary that cannot be run is not an installed update"
        );
    }

    #[test]
    fn a_failing_swap_leaves_the_running_binary_where_it_was() {
        let dir = tempfile::tempdir().unwrap();
        // No exe to move aside: the swap cannot start, and nothing changed.
        let exe = dir.path().join("assetcraft");
        let staged = dir.path().join("staged");
        std::fs::write(&staged, b"new").unwrap();

        assert!(apply_at(&staged, &exe).is_err());
        assert!(!exe.exists());
    }

    #[test]
    fn a_sibling_appends_to_the_file_name_in_place() {
        assert_eq!(
            sibling(Path::new("/usr/bin/assetcraft"), ".old"),
            PathBuf::from("/usr/bin/assetcraft.old")
        );
    }

    // ----- over the wire ---------------------------------------------------

    /// A one-release feed whose single asset is the running CLI on the
    /// running target, described as carrying `payload`, downloading from the
    /// literal `BINARY_URL` the test replaces with the mock's path.
    fn feed_with(payload: &[u8], record_size: bool) -> String {
        let name = Product::Cli.asset_name(current_target().unwrap());
        let size = if record_size { payload.len() } else { 0 };
        format!(
            r#"[{{
                "tag_name": "v99.0.0",
                "draft": false,
                "prerelease": false,
                "html_url": "https://example.invalid/release",
                "assets": [{{
                    "name": "{name}",
                    "size": {size},
                    "digest": "{}",
                    "browser_download_url": "BINARY_URL"
                }}]
            }}]"#,
            digest_of(payload)
        )
    }

    #[tokio::test]
    async fn check_reads_the_feed_and_download_delivers_the_verified_bytes() {
        use httpmock::Method::GET;
        use httpmock::MockServer;

        let server = MockServer::start_async().await;
        let payload: &[u8] = b"a freshly built assetcraft";
        let binary = server
            .mock_async(|when, then| {
                when.method(GET).path("/binary");
                then.status(200).body(payload);
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/releases");
                then.status(200).body(
                    feed_with(payload, true)
                        .replace("BINARY_URL", &format!("{}/binary", server.url(""))),
                );
            })
            .await;

        let client = UpdateClient::new(&server.url("/releases"), reqwest::Client::new()).unwrap();
        let update = client.check(Product::Cli).await.unwrap().unwrap();
        assert_eq!(update.tag, "v99.0.0");

        let dir = tempfile::tempdir().unwrap();
        let staged = client.download(&update, dir.path()).await.unwrap();
        binary.assert_calls_async(1).await;
        assert_eq!(std::fs::read(&staged).unwrap(), payload);
    }

    #[tokio::test]
    async fn a_binary_that_does_not_match_its_digest_is_discarded() {
        use httpmock::Method::GET;
        use httpmock::MockServer;

        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/binary");
                // The feed describes `pristine bytes`; the server hands back
                // others. Size is recorded as 0 so it is the digest, not the
                // length, that has to catch the tampering.
                then.status(200).body(b"tampered");
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/releases");
                then.status(200).body(
                    feed_with(b"pristine bytes", false)
                        .replace("BINARY_URL", &format!("{}/binary", server.url(""))),
                );
            })
            .await;

        let client = UpdateClient::new(&server.url("/releases"), reqwest::Client::new()).unwrap();
        let update = client.check(Product::Cli).await.unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let error = client.download(&update, dir.path()).await.unwrap_err();
        assert!(error.to_string().contains("checksum"), "{error}");
        // Nothing staged survives a failed check.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_feed_that_answers_with_an_error_is_reported_not_guessed() {
        use httpmock::Method::GET;
        use httpmock::MockServer;

        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/releases");
                then.status(403).body("rate limited");
            })
            .await;

        let client = UpdateClient::new(&server.url("/releases"), reqwest::Client::new()).unwrap();
        let error = client.check(Product::Cli).await.unwrap_err();
        assert!(matches!(error, Error::HttpStatus { status: 403, .. }));
    }
}
