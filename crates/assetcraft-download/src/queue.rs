//! The persistent work queue of downloads.
//!
//! The engine drains this queue; keeping it as data (rather than in-memory
//! state inside the engine) is what lets a queue survive a restart, which M2
//! requires.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use assetcraft_core::{AssetId, Error, Result};
use serde::{Deserialize, Serialize};

/// A request to fetch one downloadable file of one asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadRequest {
    /// The asset the file belongs to.
    pub asset: AssetId,
    /// Format code the file belongs to, e.g. `fbx`.
    pub format_code: String,
    /// File id within that format.
    pub file_id: String,
    /// Where the file should end up on disk.
    pub destination: PathBuf,
}

impl DownloadRequest {
    /// Build a request.
    pub fn new(
        asset: impl Into<AssetId>,
        format_code: impl Into<String>,
        file_id: impl Into<String>,
        destination: impl Into<PathBuf>,
    ) -> Self {
        Self {
            asset: asset.into(),
            format_code: format_code.into(),
            file_id: file_id.into(),
            destination: destination.into(),
        }
    }

    /// A stable key identifying this exact file.
    ///
    /// Used to keep the queue free of duplicates; two requests for the same
    /// file differ only in their destination, so the destination is excluded.
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.asset, self.format_code, self.file_id)
    }
}

/// An ordered queue of downloads that refuses duplicates.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DownloadQueue {
    pending: VecDeque<DownloadRequest>,
    #[serde(skip)]
    keys: HashSet<String>,
}

impl DownloadQueue {
    /// An empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a request, returning `false` if that file is already queued.
    pub fn push(&mut self, request: DownloadRequest) -> bool {
        let key = request.key();
        if self.keys.contains(&key) {
            return false;
        }
        self.keys.insert(key);
        self.pending.push_back(request);
        true
    }

    /// Take the next request, oldest first.
    ///
    /// The file is forgotten once taken, so it can be queued again later.
    pub fn pop(&mut self) -> Option<DownloadRequest> {
        let request = self.pending.pop_front()?;
        self.keys.remove(&request.key());
        Some(request)
    }

    /// Whether this exact file is currently queued.
    pub fn contains(&self, request: &DownloadRequest) -> bool {
        self.keys.contains(&request.key())
    }

    /// Number of queued downloads.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Iterate over queued downloads, oldest first, without removing them.
    pub fn iter(&self) -> impl Iterator<Item = &DownloadRequest> {
        self.pending.iter()
    }

    /// Write the queue to `path` as JSON, creating parent directories.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        std::fs::write(path, raw)?;
        Ok(())
    }

    /// Read a queue written by [`DownloadQueue::save`].
    ///
    /// A missing file is an empty queue, not an error — the first run has no
    /// queue to read.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(err) => return Err(Error::Io(err)),
        };
        let mut queue: Self = serde_json::from_str(&raw)?;
        // `keys` is derived state and is skipped over the wire; rebuild it.
        queue.keys = queue.pending.iter().map(DownloadRequest::key).collect();
        Ok(queue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(file_id: &str) -> DownloadRequest {
        DownloadRequest::new("asset-1", "fbx", file_id, "/tmp/out")
    }

    #[test]
    fn pops_in_first_in_first_out_order() {
        let mut queue = DownloadQueue::new();
        assert!(queue.push(request("a")));
        assert!(queue.push(request("b")));
        assert_eq!(queue.len(), 2);

        assert_eq!(queue.pop().unwrap().file_id, "a");
        assert_eq!(queue.pop().unwrap().file_id, "b");
        assert!(queue.pop().is_none());
        assert!(queue.is_empty());
    }

    #[test]
    fn rejects_a_duplicate_of_the_same_file() {
        let mut queue = DownloadQueue::new();
        assert!(queue.push(request("a")));
        // Same asset, format and file id, even with a different destination.
        let duplicate = DownloadRequest::new("asset-1", "fbx", "a", "/somewhere/else");
        assert!(!queue.push(duplicate));
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn different_files_are_not_duplicates() {
        let mut queue = DownloadQueue::new();
        assert!(queue.push(request("a")));
        assert!(queue.push(DownloadRequest::new("asset-2", "fbx", "a", "/tmp/out")));
        assert!(queue.push(DownloadRequest::new("asset-1", "gltf", "a", "/tmp/out")));
        assert_eq!(queue.len(), 3);
    }

    #[test]
    fn a_popped_file_can_be_queued_again() {
        let mut queue = DownloadQueue::new();
        queue.push(request("a"));
        let taken = queue.pop().unwrap();
        assert!(!queue.contains(&taken));
        assert!(queue.push(taken));
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn key_ignores_the_destination() {
        let one = DownloadRequest::new("asset-1", "fbx", "a", "/one");
        let two = DownloadRequest::new("asset-1", "fbx", "a", "/two");
        assert_eq!(one.key(), two.key());
    }

    #[test]
    fn iterates_without_consuming() {
        let mut queue = DownloadQueue::new();
        queue.push(request("a"));
        queue.push(request("b"));
        let ids: Vec<&str> = queue.iter().map(|r| r.file_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn save_and_load_round_trip_preserving_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");

        let mut queue = DownloadQueue::new();
        queue.push(request("a"));
        queue.push(request("b"));
        queue.save(&path).unwrap();

        let mut loaded = DownloadQueue::load(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        // Order survives, and de-duplication still works after a reload.
        assert_eq!(loaded.pop().unwrap().file_id, "a");
        assert!(loaded.contains(&request("b")));
        assert!(!loaded.push(request("b")));
    }

    #[test]
    fn loading_a_missing_file_is_an_empty_queue() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DownloadQueue::load(dir.path().join("absent.json")).unwrap();
        assert!(queue.is_empty());
    }
}
