//! A cache of remote media (thumbnails and previews) for offline browsing.
//!
//! The cache is content-addressed by a hash of the image URL, so re-caching an
//! item is a no-op and two library items that share a preview share one file.
//! Each image has a sibling `<key>.url` file holding the source URL; on read
//! the stored URL is compared to the requested one, so a hash collision is a
//! cache miss rather than the wrong picture.
//!
//! This module only stores and reads bytes. Fetching them is the caller's job,
//! which keeps the library free of an HTTP dependency.

use std::path::{Path, PathBuf};

use assetcraft_core::Result;

/// A directory of cached thumbnails and previews.
#[derive(Debug, Clone)]
pub struct MediaCache {
    root: PathBuf,
}

impl MediaCache {
    /// Use `root` as the cache directory. Nothing is created until the first
    /// [`MediaCache::put`].
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The directory the cache lives in.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where an image for `url` is stored, whether or not it is there yet.
    pub fn path_for(&self, url: &str) -> PathBuf {
        self.root
            .join(format!("{}.{}", cache_key(url), extension_for(url)))
    }

    /// The path of a verified cached image for `url`, if there is one.
    pub fn cached_path(&self, url: &str) -> Option<PathBuf> {
        let path = self.path_for(url);
        if self.verify(url, &path) {
            Some(path)
        } else {
            None
        }
    }

    /// Whether a verified image for `url` is cached.
    pub fn contains(&self, url: &str) -> bool {
        self.cached_path(url).is_some()
    }

    /// The cached bytes for `url`, if a verified image is present.
    pub fn get(&self, url: &str) -> Result<Option<Vec<u8>>> {
        match self.cached_path(url) {
            Some(path) => Ok(Some(std::fs::read(path)?)),
            None => Ok(None),
        }
    }

    /// Store `bytes` as the image for `url`, returning the path it was written
    /// to. An already-cached URL is left untouched.
    pub fn put(&self, url: &str, bytes: &[u8]) -> Result<PathBuf> {
        if let Some(path) = self.cached_path(url) {
            return Ok(path);
        }
        std::fs::create_dir_all(&self.root)?;

        let path = self.path_for(url);
        // Write through a temporary file so a reader never sees a half-written
        // image, then rename into place.
        let temp = path.with_extension("part");
        std::fs::write(&temp, bytes)?;
        std::fs::rename(&temp, &path)?;

        // The sidecar is written last: if it is missing the entry is treated as
        // a miss and refetched, which is safe.
        std::fs::write(self.sidecar(url), url)?;
        Ok(path)
    }

    /// Whether the entry at `path` belongs to `url`.
    fn verify(&self, url: &str, path: &Path) -> bool {
        if !path.exists() {
            return false;
        }
        match std::fs::read_to_string(self.sidecar(url)) {
            Ok(stored) => stored == url,
            Err(_) => false,
        }
    }

    /// The sidecar file recording which URL an entry holds.
    fn sidecar(&self, url: &str) -> PathBuf {
        self.root.join(format!("{}.url", cache_key(url)))
    }
}

/// A stable, dependency-free key for a URL (FNV-1a, 64-bit).
fn cache_key(url: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in url.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// A file extension to store `url` under, guessed from its path.
///
/// The extension is cosmetic — a hint for anyone browsing the cache directory —
/// so anything unrecognised falls back to `img`.
fn extension_for(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let extension = path.rsplit('.').next().unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "png" => "png",
        "jpg" | "jpeg" => "jpg",
        "webp" => "webp",
        "gif" => "gif",
        "bmp" => "bmp",
        "tga" => "tga",
        "tif" | "tiff" => "tif",
        _ => "img",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://media.fab.com/rock.png";

    fn cache() -> (tempfile::TempDir, MediaCache) {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::new(dir.path().join("thumbnails"));
        (dir, cache)
    }

    #[test]
    fn a_missing_entry_is_a_miss() {
        let (_dir, cache) = cache();
        assert!(!cache.contains(URL));
        assert!(cache.get(URL).unwrap().is_none());
        assert!(cache.cached_path(URL).is_none());
    }

    #[test]
    fn put_then_get_round_trips_the_bytes() {
        let (_dir, cache) = cache();
        let path = cache.put(URL, b"image bytes").unwrap();
        assert!(path.exists());
        assert!(cache.contains(URL));
        assert_eq!(cache.get(URL).unwrap().unwrap(), b"image bytes");
        assert_eq!(cache.cached_path(URL).unwrap(), path);
    }

    #[test]
    fn put_is_idempotent_and_keeps_the_first_bytes() {
        let (_dir, cache) = cache();
        cache.put(URL, b"first").unwrap();
        cache.put(URL, b"second").unwrap();
        assert_eq!(cache.get(URL).unwrap().unwrap(), b"first");
    }

    #[test]
    fn two_urls_do_not_share_an_entry() {
        let (_dir, cache) = cache();
        cache.put("https://media.fab.com/a.png", b"a").unwrap();
        cache.put("https://media.fab.com/b.png", b"b").unwrap();
        assert_eq!(
            cache.get("https://media.fab.com/a.png").unwrap().unwrap(),
            b"a"
        );
        assert_eq!(
            cache.get("https://media.fab.com/b.png").unwrap().unwrap(),
            b"b"
        );
        assert_ne!(
            cache.path_for("https://media.fab.com/a.png"),
            cache.path_for("https://media.fab.com/b.png")
        );
    }

    #[test]
    fn an_entry_whose_stored_url_differs_is_a_miss() {
        let (_dir, cache) = cache();
        cache.put(URL, b"image").unwrap();
        // Simulate a hash collision: the entry's sidecar points at another URL.
        std::fs::write(cache.sidecar(URL), "https://media.fab.com/other.png").unwrap();
        assert!(!cache.contains(URL));
        assert!(cache.get(URL).unwrap().is_none());
    }

    #[test]
    fn a_missing_sidecar_is_a_miss() {
        let (_dir, cache) = cache();
        let path = cache.put(URL, b"image").unwrap();
        std::fs::remove_file(cache.sidecar(URL)).unwrap();
        // The image is still on disk, but without its URL it is not trusted.
        assert!(path.exists());
        assert!(!cache.contains(URL));
    }

    #[test]
    fn extensions_are_guessed_from_the_url() {
        assert_eq!(extension_for("https://m/a.PNG"), "png");
        assert_eq!(extension_for("https://m/a.jpeg?x=1"), "jpg");
        assert_eq!(extension_for("https://m/a.webp#frag"), "webp");
        assert_eq!(extension_for("https://media.fab.com/abc"), "img");
        assert_eq!(extension_for("https://m/a.txt"), "img");
    }

    #[test]
    fn the_key_is_stable_and_url_specific() {
        assert_eq!(cache_key(URL), cache_key(URL));
        assert_ne!(cache_key(URL), cache_key("https://media.fab.com/other.png"));
        assert_eq!(cache_key(URL).len(), 16);
    }
}
