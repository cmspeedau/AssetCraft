//! A small on-disk cache for API responses.
//!
//! The cache exists so that browsing a catalog you have already looked at does
//! not hammer the marketplace, and so repeated CLI invocations stay quick. It is
//! deliberately simple: one JSON file per response, keyed by the request URL,
//! with a time-to-live.
//!
//! Two details are deliberate:
//!
//! - The key is a dependency-free FNV-1a hash of the URL rather than a crypto
//!   digest, because the cache is disposable and we would rather not pull in a
//!   hashing crate for it.
//! - The URL is stored *inside* the entry and checked on read, so a hash
//!   collision degrades to a cache miss instead of serving the wrong body.

use std::fs;
use std::path::{Path, PathBuf};

use assetcraft_core::{Error, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// One cached response.
#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    url: String,
    stored_at: DateTime<Utc>,
    body: String,
}

/// A sensible default lifetime for a cached API response.
///
/// Long enough that browsing a catalog twice is cheap, short enough that a
/// listing that changed today is not stale for long.
pub const DEFAULT_TTL: Duration = Duration::seconds(1800);

/// A time-limited, on-disk cache of response bodies keyed by URL.
#[derive(Debug, Clone)]
pub struct ResponseCache {
    dir: PathBuf,
    ttl: Duration,
}

impl ResponseCache {
    /// Cache responses under `dir`, treating an entry older than `ttl` as absent.
    pub fn new(dir: impl Into<PathBuf>, ttl: Duration) -> Self {
        Self {
            dir: dir.into(),
            ttl,
        }
    }

    /// The directory entries are written to.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// How long an entry stays valid.
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Read a cached body for `url`, if one is present and still fresh.
    pub fn get(&self, url: &str) -> Option<String> {
        let raw = fs::read_to_string(self.path_for(url)).ok()?;
        let entry: Entry = serde_json::from_str(&raw).ok()?;
        // Guard against a hash collision: the key must agree with the entry.
        if entry.url != url {
            return None;
        }
        if !is_fresh(entry.stored_at, Utc::now(), self.ttl) {
            return None;
        }
        Some(entry.body)
    }

    /// Store `body` for `url`.
    pub fn put(&self, url: &str, body: &str) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        let entry = Entry {
            url: url.to_string(),
            stored_at: Utc::now(),
            body: body.to_string(),
        };
        let raw = serde_json::to_string(&entry)?;
        fs::write(self.path_for(url), raw)?;
        Ok(())
    }

    /// Remove every cached entry, returning how many were removed.
    pub fn clear(&self) -> Result<usize> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(err) => return Err(Error::Io(err)),
        };

        let mut removed = 0;
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                fs::remove_file(path)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn path_for(&self, url: &str) -> PathBuf {
        self.dir.join(format!("{}.json", fnv1a_hex(url)))
    }
}

/// Whether an entry stored at `stored_at` is still within `ttl` of `now`.
fn is_fresh(stored_at: DateTime<Utc>, now: DateTime<Utc>, ttl: Duration) -> bool {
    now.signed_duration_since(stored_at) <= ttl
}

/// A stable 64-bit FNV-1a hash, rendered as hex.
fn fnv1a_hex(input: &str) -> String {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET_BASIS;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(dir: &Path) -> ResponseCache {
        ResponseCache::new(dir, Duration::hours(1))
    }

    #[test]
    fn missing_entry_is_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        assert!(cache(dir.path()).get("https://example/a").is_none());
    }

    #[test]
    fn put_then_get_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        cache.put("https://example/a", "{\"x\":1}").unwrap();
        assert_eq!(cache.get("https://example/a").as_deref(), Some("{\"x\":1}"));
    }

    #[test]
    fn different_urls_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        cache.put("https://example/a", "A").unwrap();
        cache.put("https://example/b", "B").unwrap();
        assert_eq!(cache.get("https://example/a").as_deref(), Some("A"));
        assert_eq!(cache.get("https://example/b").as_deref(), Some("B"));
        assert!(cache.get("https://example/c").is_none());
    }

    #[test]
    fn an_entry_whose_stored_url_disagrees_is_a_miss() {
        // Simulates a hash collision: the file exists under the key but holds a
        // different URL.
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let path = dir
            .path()
            .join(format!("{}.json", fnv1a_hex("https://example/a")));
        fs::write(
            &path,
            r#"{"url":"https://example/other","stored_at":"2026-01-01T00:00:00Z","body":"wrong"}"#,
        )
        .unwrap();
        assert!(cache.get("https://example/a").is_none());
    }

    #[test]
    fn freshness_respects_the_ttl() {
        let now = Utc::now();
        assert!(is_fresh(
            now - Duration::seconds(1),
            now,
            Duration::seconds(5)
        ));
        assert!(!is_fresh(
            now - Duration::seconds(10),
            now,
            Duration::seconds(5)
        ));
    }

    #[test]
    fn an_expired_entry_is_not_returned() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ResponseCache::new(dir.path(), Duration::seconds(-1));
        cache.put("https://example/a", "A").unwrap();
        assert!(cache.get("https://example/a").is_none());
    }

    #[test]
    fn clear_removes_entries_and_is_safe_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        cache.put("https://example/a", "A").unwrap();
        cache.put("https://example/b", "B").unwrap();
        assert_eq!(cache.clear().unwrap(), 2);
        assert!(cache.get("https://example/a").is_none());
        // Clearing an already-empty cache is not an error.
        assert_eq!(cache.clear().unwrap(), 0);
    }

    #[test]
    fn clear_on_a_missing_directory_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ResponseCache::new(dir.path().join("absent"), Duration::hours(1));
        assert_eq!(cache.clear().unwrap(), 0);
    }

    #[test]
    fn the_hash_is_stable_and_hex() {
        assert_eq!(fnv1a_hex("abc"), fnv1a_hex("abc"));
        assert_ne!(fnv1a_hex("abc"), fnv1a_hex("abd"));
        assert_eq!(fnv1a_hex("abc").len(), 16);
    }
}
