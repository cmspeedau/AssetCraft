//! The local asset library.
//!
//! M3 replaces this with a SQLite-backed store that survives restarts. The
//! interface here is deliberately small and map-like so that swap does not
//! reach into callers: insert, look up, browse, and search offline.

use std::collections::BTreeMap;

use assetcraft_core::{AssetId, AssetSummary};

/// An in-memory index of the assets the user has acquired.
#[derive(Debug, Default)]
pub struct LibraryIndex {
    items: BTreeMap<AssetId, AssetSummary>,
}

impl LibraryIndex {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an asset, returning `false` if that id was already present.
    ///
    /// De-duplicating by id is what lets the download and re-export paths stay
    /// idempotent: acquiring the same asset twice is a no-op.
    pub fn insert(&mut self, item: AssetSummary) -> bool {
        use std::collections::btree_map::Entry;
        match self.items.entry(item.id.clone()) {
            Entry::Occupied(_) => false,
            Entry::Vacant(slot) => {
                slot.insert(item);
                true
            }
        }
    }

    /// Whether the index holds this asset.
    pub fn contains(&self, id: &AssetId) -> bool {
        self.items.contains_key(id)
    }

    /// Look up an asset by id.
    pub fn get(&self, id: &AssetId) -> Option<&AssetSummary> {
        self.items.get(id)
    }

    /// Remove an asset, returning it if it was present.
    pub fn remove(&mut self, id: &AssetId) -> Option<AssetSummary> {
        self.items.remove(id)
    }

    /// Number of assets held.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterate over the index in id order.
    pub fn iter(&self) -> impl Iterator<Item = &AssetSummary> {
        self.items.values()
    }

    /// Case-insensitive search of titles, for offline browsing.
    pub fn search(&self, needle: &str) -> Vec<&AssetSummary> {
        let needle = needle.trim().to_lowercase();
        if needle.is_empty() {
            return self.iter().collect();
        }
        self.items
            .values()
            .filter(|item| item.display_title().to_lowercase().contains(&needle))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(id: &str, title: &str) -> AssetSummary {
        AssetSummary {
            id: AssetId::new(id),
            title: Some(title.to_string()),
            listing_type: None,
            seller: None,
            category: None,
            tags: Vec::new(),
            rating: None,
            is_free: false,
            is_mature: false,
            published_at: None,
            thumbnail_url: None,
        }
    }

    #[test]
    fn insert_is_idempotent_by_id() {
        let mut index = LibraryIndex::new();
        assert!(index.insert(summary("a", "Rocks")));
        assert!(!index.insert(summary("a", "Rocks Again")));
        assert_eq!(index.len(), 1);
        // The first insert wins.
        assert_eq!(
            index.get(&AssetId::new("a")).unwrap().display_title(),
            "Rocks"
        );
    }

    #[test]
    fn get_and_contains_track_membership() {
        let mut index = LibraryIndex::new();
        index.insert(summary("a", "Rocks"));
        assert!(index.contains(&AssetId::new("a")));
        assert!(!index.contains(&AssetId::new("b")));
        assert!(index.get(&AssetId::new("b")).is_none());
    }

    #[test]
    fn remove_returns_the_asset() {
        let mut index = LibraryIndex::new();
        index.insert(summary("a", "Rocks"));
        let removed = index.remove(&AssetId::new("a")).unwrap();
        assert_eq!(removed.display_title(), "Rocks");
        assert!(index.is_empty());
        assert!(index.remove(&AssetId::new("a")).is_none());
    }

    #[test]
    fn search_matches_titles_case_insensitively() {
        let mut index = LibraryIndex::new();
        index.insert(summary("a", "Highland Rock"));
        index.insert(summary("b", "Mossy Boulder"));
        let hits = index.search("rock");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id.as_str(), "a");
    }

    #[test]
    fn an_empty_needle_returns_everything() {
        let mut index = LibraryIndex::new();
        index.insert(summary("a", "Rock"));
        index.insert(summary("b", "Boulder"));
        assert_eq!(index.search("   ").len(), 2);
    }

    #[test]
    fn iterates_in_id_order() {
        let mut index = LibraryIndex::new();
        index.insert(summary("b", "Boulder"));
        index.insert(summary("a", "Rock"));
        let ids: Vec<&str> = index.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }
}
