//! The local asset library.
//!
//! This is the record of what the user owns, kept on disk so it can be browsed
//! with the network down. It is a SQLite database with a single table: an
//! asset's full [`LibraryItem`] as JSON, plus a denormalised `title` so search
//! and ordering can be done in SQL.
//!
//! Storing the JSON as the source of truth means a schema change to the domain
//! model never silently loses a field: what was written comes back, or the row
//! is reported by [`Library::verify`]. The denormalised title is derived data,
//! rebuilt only on insert.
//!
//! Beside the index sits [`MediaCache`], a content-addressed store for the
//! thumbnails and previews that make offline browsing useful. It holds bytes
//! only; fetching them is the caller's job.
//!
//! A second table, `downloads`, records which assets have been brought down to
//! disk and where their files live, so a previously downloaded asset can be
//! re-exported by id with the network off. The path is local state, kept apart
//! from the Fab-sourced [`LibraryItem`] JSON so a re-sync never overwrites it.
//!
//! The third and fourth tables are the user's own organisation, which the app
//! persists next to the index it organises: `favorites` (a set of asset ids)
//! and `collections` with `collection_items` (named groups). These are local
//! state like download records — never Fab-sourced, never rebuilt by a sync —
//! and removing an asset clears it out of all of them.

use std::path::{Path, PathBuf};

use assetcraft_core::{AssetId, Error, LibraryItem, Result};
use rusqlite::{Connection, OptionalExtension};

pub mod media;

pub use media::MediaCache;

/// The schema, applied on every open so an existing library is upgraded in
/// place and a new one is created.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS assets (
    id    TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    item  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_assets_title ON assets (title COLLATE NOCASE);
CREATE TABLE IF NOT EXISTS downloads (
    asset_id TEXT PRIMARY KEY NOT NULL,
    path     TEXT NOT NULL,
    format   TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS favorites (
    asset_id TEXT PRIMARY KEY NOT NULL
);
CREATE TABLE IF NOT EXISTS collections (
    id   INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
);
CREATE TABLE IF NOT EXISTS collection_items (
    collection_id INTEGER NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    asset_id      TEXT NOT NULL,
    PRIMARY KEY (collection_id, asset_id)
);
";

/// The user's local library.
#[derive(Debug)]
pub struct Library {
    conn: Connection,
}

/// A row that could not be read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableRow {
    /// The asset id of the offending row.
    pub id: String,
    /// What was wrong with it.
    pub problem: String,
}

/// The result of checking the library for damage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntegrityReport {
    /// How many rows were examined.
    pub total: usize,
    /// Rows whose stored item could not be read.
    pub unreadable: Vec<UnreadableRow>,
}

impl IntegrityReport {
    /// Whether every row read back cleanly.
    pub fn is_healthy(&self) -> bool {
        self.unreadable.is_empty()
    }
}

/// The result of repairing the library.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepairReport {
    /// How many rows were examined.
    pub total: usize,
    /// Rows whose denormalised title was realigned with the stored item.
    pub titles_repaired: usize,
    /// Unreadable rows that were dropped from the index.
    pub removed: Vec<UnreadableRow>,
}

impl RepairReport {
    /// Whether nothing had to be dropped to make the library healthy.
    pub fn is_healthy(&self) -> bool {
        self.removed.is_empty()
    }
}

/// Where one asset's files were brought down to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    /// The directory holding the asset's downloaded files.
    pub path: PathBuf,
    /// The format code the files came from, when it was recorded.
    pub format: String,
}

/// A named group of assets, as the user drew it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    /// The database row id, stable for the collection's life.
    pub id: i64,
    /// The name, unique case-insensitively.
    pub name: String,
    /// How many assets it currently holds.
    pub asset_count: usize,
}

impl Library {
    /// Open (or create) the library at `path`, creating parent directories.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::from_connection(Connection::open(path).map_err(database)?)
    }

    /// Open a library that lives only in memory, for tests and dry runs.
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory().map_err(database)?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        // `collection_items` relies on `ON DELETE CASCADE` from `collections`,
        // and foreign keys are off unless every connection asks for them.
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(database)?;
        conn.execute_batch(SCHEMA).map_err(database)?;
        Ok(Self { conn })
    }

    /// Add an asset, returning `false` if it was already present.
    ///
    /// De-duplication by asset id is what keeps re-acquiring an asset a no-op.
    pub fn insert(&self, item: &LibraryItem) -> Result<bool> {
        let json = serde_json::to_string(item)?;
        let changed = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO assets (id, title, item) VALUES (?1, ?2, ?3)",
                rusqlite::params![item.asset_id, item.title, json],
            )
            .map_err(database)?;
        Ok(changed > 0)
    }

    /// Look up an asset by id.
    pub fn get(&self, id: &AssetId) -> Result<Option<LibraryItem>> {
        let mut rows = self.items(
            "SELECT item FROM assets WHERE id = ?1",
            rusqlite::params![id.as_str()],
        )?;
        Ok(rows.pop())
    }

    /// Whether the library holds this asset.
    pub fn contains(&self, id: &AssetId) -> Result<bool> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM assets WHERE id = ?1",
                rusqlite::params![id.as_str()],
                |row| row.get(0),
            )
            .map_err(database)?;
        Ok(count > 0)
    }

    /// Remove an asset, reporting whether it was there.
    ///
    /// Any recorded download goes with it, so a later re-download is recorded
    /// fresh rather than pointing at a directory that has been cleared out.
    /// Favouriting and collection membership are the user's notes about an
    /// asset, and a removed asset must not linger in them as a dead row.
    pub fn remove(&self, id: &AssetId) -> Result<bool> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM assets WHERE id = ?1",
                rusqlite::params![id.as_str()],
            )
            .map_err(database)?;
        for table in ["downloads", "favorites", "collection_items"] {
            self.conn
                .execute(
                    &format!("DELETE FROM {table} WHERE asset_id = ?1"),
                    rusqlite::params![id.as_str()],
                )
                .map_err(database)?;
        }
        Ok(changed > 0)
    }

    /// How many assets are held.
    pub fn len(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))
            .map_err(database)?;
        Ok(count as usize)
    }

    /// Whether the library is empty.
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    /// One page of the library, in title order.
    pub fn list(&self, limit: usize, offset: usize) -> Result<Vec<LibraryItem>> {
        self.items(
            "SELECT item FROM assets ORDER BY title COLLATE NOCASE, id LIMIT ?1 OFFSET ?2",
            rusqlite::params![limit as i64, offset as i64],
        )
    }

    /// Every asset, in title order.
    pub fn list_all(&self) -> Result<Vec<LibraryItem>> {
        self.items(
            "SELECT item FROM assets ORDER BY title COLLATE NOCASE, id",
            [],
        )
    }

    /// Find assets by title, case-insensitively, for offline browsing.
    ///
    /// Uses `instr` rather than `LIKE` so that `%` and `_` in the query are
    /// matched literally instead of acting as wildcards.
    pub fn search(&self, needle: &str) -> Result<Vec<LibraryItem>> {
        let needle = needle.trim();
        if needle.is_empty() {
            return self.list_all();
        }
        self.items(
            "SELECT item FROM assets
             WHERE instr(lower(title), lower(?1)) > 0
             ORDER BY title COLLATE NOCASE, id",
            rusqlite::params![needle],
        )
    }

    /// Record where an asset's files live on disk, replacing any earlier record.
    ///
    /// Downloading an asset twice re-points the record at the newest location;
    /// the record lives on its own, so an asset need not be in the Fab index to
    /// have its download tracked.
    pub fn record_download(&self, id: &AssetId, path: &Path, format: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO downloads (asset_id, path, format) VALUES (?1, ?2, ?3)
                 ON CONFLICT(asset_id) DO UPDATE SET path = ?2, format = ?3",
                rusqlite::params![id.as_str(), path.to_string_lossy(), format],
            )
            .map_err(database)?;
        Ok(())
    }

    /// Where an asset's files were downloaded to, if they were.
    pub fn download(&self, id: &AssetId) -> Result<Option<Download>> {
        let found = self
            .conn
            .query_row(
                "SELECT path, format FROM downloads WHERE asset_id = ?1",
                rusqlite::params![id.as_str()],
                |row| {
                    Ok(Download {
                        path: PathBuf::from(row.get::<_, String>(0)?),
                        format: row.get::<_, String>(1)?,
                    })
                },
            )
            .optional()
            .map_err(database)?;
        Ok(found)
    }

    /// Whether the library has a record of an asset's downloaded files.
    pub fn is_downloaded(&self, id: &AssetId) -> Result<bool> {
        Ok(self.download(id)?.is_some())
    }

    /// Drop the record of an asset's downloaded files, reporting whether it was
    /// there. The files themselves are left untouched.
    pub fn forget_download(&self, id: &AssetId) -> Result<bool> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM downloads WHERE asset_id = ?1",
                rusqlite::params![id.as_str()],
            )
            .map_err(database)?;
        Ok(changed > 0)
    }

    // ----- favourites and collections ---------------------------------------

    /// Mark an asset as a favourite, reporting whether that added it.
    pub fn favorite(&self, id: &AssetId) -> Result<bool> {
        let changed = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO favorites (asset_id) VALUES (?1)",
                rusqlite::params![id.as_str()],
            )
            .map_err(database)?;
        Ok(changed > 0)
    }

    /// Clear an asset's favourite, reporting whether it was one.
    pub fn unfavorite(&self, id: &AssetId) -> Result<bool> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM favorites WHERE asset_id = ?1",
                rusqlite::params![id.as_str()],
            )
            .map_err(database)?;
        Ok(changed > 0)
    }

    /// The favourite asset ids.
    pub fn favorites(&self) -> Result<Vec<String>> {
        self.strings("SELECT asset_id FROM favorites ORDER BY asset_id", [])
    }

    /// Create a collection. Names are unique case-insensitively — the point is
    /// organising, and two panels that differ only in capitalisation would
    /// undermine that.
    pub fn create_collection(&self, name: &str) -> Result<Collection> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Other("a collection needs a name".to_string()));
        }
        self.require_name_free(name, None)?;
        self.conn
            .execute(
                "INSERT INTO collections (name) VALUES (?1)",
                rusqlite::params![name],
            )
            .map_err(database)?;
        Ok(Collection {
            id: self.conn.last_insert_rowid(),
            name: name.to_owned(),
            asset_count: 0,
        })
    }

    /// Rename a collection, reporting whether the id was one. The same
    /// uniqueness rule as [`create_collection`][Self::create_collection] holds.
    pub fn rename_collection(&self, id: i64, name: &str) -> Result<bool> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Other("a collection needs a name".to_string()));
        }
        self.require_name_free(name, Some(id))?;
        let changed = self
            .conn
            .execute(
                "UPDATE collections SET name = ?1 WHERE id = ?2",
                rusqlite::params![name, id],
            )
            .map_err(database)?;
        Ok(changed > 0)
    }

    /// Delete a collection. Its membership rows go with it (the foreign key
    /// cascades); the assets themselves are untouched — removing an organising
    /// hat is not removing the asset.
    pub fn delete_collection(&self, id: i64) -> Result<bool> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM collections WHERE id = ?1",
                rusqlite::params![id],
            )
            .map_err(database)?;
        Ok(changed > 0)
    }

    /// Every collection, by name, with its current asset count.
    pub fn list_collections(&self) -> Result<Vec<Collection>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.id, c.name, COUNT(i.asset_id)
                 FROM collections c
                 LEFT JOIN collection_items i ON i.collection_id = c.id
                 GROUP BY c.id
                 ORDER BY c.name COLLATE NOCASE",
            )
            .map_err(database)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Collection {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    asset_count: row.get::<_, i64>(2)? as usize,
                })
            })
            .map_err(database)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(database)?);
        }
        Ok(out)
    }

    /// The asset ids a collection holds, in the order the library lists them
    /// (title first), so a collection view reads like the Local view.
    pub fn collection_members(&self, id: i64) -> Result<Vec<String>> {
        self.strings(
            "SELECT i.asset_id FROM collection_items i
             LEFT JOIN assets a ON a.id = i.asset_id
             WHERE i.collection_id = ?1
             ORDER BY COALESCE(a.title, i.asset_id) COLLATE NOCASE, i.asset_id",
            rusqlite::params![id],
        )
    }

    /// The collection ids an asset belongs to, for the detail panel's checks.
    pub fn collections_for(&self, id: &AssetId) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT collection_id FROM collection_items WHERE asset_id = ?1")
            .map_err(database)?;
        let rows = stmt
            .query_map(rusqlite::params![id.as_str()], |row| row.get::<_, i64>(0))
            .map_err(database)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(database)?);
        }
        Ok(out)
    }

    /// Put an asset in a collection, reporting whether that added it. Adding
    /// twice is a no-op, not an error — a checkbox that fired twice should not
    /// complain.
    pub fn add_to_collection(&self, id: i64, asset: &AssetId) -> Result<bool> {
        let changed = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO collection_items (collection_id, asset_id)
                 VALUES (?1, ?2)",
                rusqlite::params![id, asset.as_str()],
            )
            .map_err(database)?;
        Ok(changed > 0)
    }

    /// Take an asset out of a collection, reporting whether it was in it.
    pub fn remove_from_collection(&self, id: i64, asset: &AssetId) -> Result<bool> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM collection_items WHERE collection_id = ?1 AND asset_id = ?2",
                rusqlite::params![id, asset.as_str()],
            )
            .map_err(database)?;
        Ok(changed > 0)
    }

    /// Whether any other collection already answers to `name` (case folding
    /// like the UNIQUE index, so the error arrives as "someone else took this
    /// name" rather than as a raw constraint message).
    fn require_name_free(&self, name: &str, except_id: Option<i64>) -> Result<()> {
        let taken: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM collections WHERE name = ?1 COLLATE NOCASE
                 AND id != ?2",
                rusqlite::params![name, except_id.unwrap_or(-1)],
                |row| row.get(0),
            )
            .map_err(database)?;
        if taken > 0 {
            return Err(Error::Other(format!(
                "there is already a collection called {name}"
            )));
        }
        Ok(())
    }

    /// Run a query returning one string per row from column 0.
    fn strings(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(sql).map_err(database)?;
        let rows = stmt
            .query_map(params, |row| row.get::<_, String>(0))
            .map_err(database)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(database)?);
        }
        Ok(out)
    }

    /// Check every row can be read back.
    pub fn verify(&self) -> Result<IntegrityReport> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, item FROM assets ORDER BY id")
            .map_err(database)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(database)?;

        let mut report = IntegrityReport::default();
        for row in rows {
            let (id, json) = row.map_err(database)?;
            report.total += 1;
            if serde_json::from_str::<LibraryItem>(&json).is_err() {
                report.unreadable.push(UnreadableRow {
                    id,
                    problem: "stored item is not a readable library item".to_string(),
                });
            }
        }
        Ok(report)
    }

    /// Repair the library: realign stale titles and drop unreadable rows.
    ///
    /// Where [`verify`] only reports damage, this acts on it. A row whose stored
    /// item reads back is kept, and its denormalised `title` column is realigned
    /// with the item it was derived from — the title is only rebuilt on insert,
    /// so a manual edit or a schema change can leave it stale. A row whose item
    /// cannot be read back is dropped from the index: it can never be shown,
    /// searched, or re-exported, so it is dead weight. Its `downloads` record is
    /// deliberately left in place, because the files are still on disk and a
    /// later `assetcraft library` sync restores the metadata row.
    pub fn repair(&self) -> Result<RepairReport> {
        // Read the rows out first: mutating the table through the same connection
        // while a cursor is open over it is asking for surprises.
        let rows = {
            let mut stmt = self
                .conn
                .prepare("SELECT id, title, item FROM assets ORDER BY id")
                .map_err(database)?;
            let read = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(database)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(database)?;
            read
        };

        let mut report = RepairReport::default();
        for (id, title, json) in rows {
            report.total += 1;
            match serde_json::from_str::<LibraryItem>(&json) {
                Ok(item) => {
                    if title != item.title {
                        self.conn
                            .execute(
                                "UPDATE assets SET title = ?1 WHERE id = ?2",
                                rusqlite::params![item.title, id],
                            )
                            .map_err(database)?;
                        report.titles_repaired += 1;
                    }
                }
                Err(_) => {
                    self.conn
                        .execute(
                            "DELETE FROM assets WHERE id = ?1",
                            rusqlite::params![id.as_str()],
                        )
                        .map_err(database)?;
                    report.removed.push(UnreadableRow {
                        id,
                        problem: "stored item is not a readable library item".to_string(),
                    });
                }
            }
        }
        Ok(report)
    }

    /// Run a query that returns stored items from column 0.
    fn items(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<LibraryItem>> {
        let mut stmt = self.conn.prepare(sql).map_err(database)?;
        let rows = stmt
            .query_map(params, |row| row.get::<_, String>(0))
            .map_err(database)?;

        let mut items = Vec::new();
        for row in rows {
            let json = row.map_err(database)?;
            items.push(serde_json::from_str(&json)?);
        }
        Ok(items)
    }
}

/// Convert a SQLite error, keeping the driver's message.
fn database(err: rusqlite::Error) -> Error {
    Error::Database(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use assetcraft_core::Category;

    fn item(id: &str, title: &str) -> LibraryItem {
        LibraryItem {
            asset_id: id.to_string(),
            namespace: "ue".to_string(),
            title: title.to_string(),
            categories: vec![Category {
                id: Some("c1".to_string()),
                name: Some("Rocks".to_string()),
                slug: None,
                path: None,
            }],
            ..Default::default()
        }
    }

    fn library() -> Library {
        Library::open_in_memory().unwrap()
    }

    #[test]
    fn insert_is_idempotent_by_asset_id() {
        let library = library();
        assert!(library.insert(&item("a", "Rocks")).unwrap());
        assert!(!library.insert(&item("a", "Rocks Again")).unwrap());
        assert_eq!(library.len().unwrap(), 1);
        // The first insert wins.
        assert_eq!(
            library.get(&AssetId::new("a")).unwrap().unwrap().title,
            "Rocks"
        );
    }

    #[test]
    fn get_contains_and_remove_track_membership() {
        let library = library();
        library.insert(&item("a", "Rocks")).unwrap();

        assert!(library.contains(&AssetId::new("a")).unwrap());
        assert!(!library.contains(&AssetId::new("b")).unwrap());
        assert!(library.get(&AssetId::new("b")).unwrap().is_none());

        assert!(library.remove(&AssetId::new("a")).unwrap());
        assert!(!library.remove(&AssetId::new("a")).unwrap());
        assert!(library.is_empty().unwrap());
    }

    #[test]
    fn a_round_trip_preserves_the_whole_item() {
        let library = library();
        let original = LibraryItem {
            asset_id: "a".to_string(),
            namespace: "ue".to_string(),
            title: "Highland Rock".to_string(),
            description: Some("A scanned rock".to_string()),
            url: Some("https://www.fab.com/listings/a".to_string()),
            source: Some("fab".to_string()),
            distribution_method: Some("asset_pack".to_string()),
            categories: vec![Category {
                id: Some("c1".to_string()),
                name: Some("Rocks".to_string()),
                slug: Some("rocks".to_string()),
                path: Some("/rocks".to_string()),
            }],
            images: vec![assetcraft_core::AssetImage {
                kind: Some("thumbnail".to_string()),
                url: Some("https://media/rock.jpg".to_string()),
                width: Some(1024),
                height: Some(1024),
                md5: Some("abc".to_string()),
            }],
        };
        library.insert(&original).unwrap();

        let read_back = library.get(&AssetId::new("a")).unwrap().unwrap();
        assert_eq!(read_back, original);
    }

    #[test]
    fn listing_is_ordered_by_title_and_paginates() {
        let library = library();
        library.insert(&item("c", "Cliff")).unwrap();
        library.insert(&item("a", "Arch")).unwrap();
        library.insert(&item("b", "Boulder")).unwrap();

        let all = library.list_all().unwrap();
        let titles: Vec<&str> = all.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, vec!["Arch", "Boulder", "Cliff"]);

        let page = library.list(2, 1).unwrap();
        let titles: Vec<&str> = page.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, vec!["Boulder", "Cliff"]);
    }

    #[test]
    fn search_is_case_insensitive() {
        let library = library();
        library.insert(&item("a", "Highland Rock")).unwrap();
        library.insert(&item("b", "Mossy Boulder")).unwrap();

        let hits = library.search("rock").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].asset_id, "a");
        assert_eq!(library.search("ROCK").unwrap().len(), 1);
    }

    #[test]
    fn an_empty_search_returns_everything() {
        let library = library();
        library.insert(&item("a", "Rock")).unwrap();
        library.insert(&item("b", "Boulder")).unwrap();
        assert_eq!(library.search("   ").unwrap().len(), 2);
    }

    #[test]
    fn search_treats_wildcards_literally() {
        let library = library();
        library.insert(&item("a", "100% Rock")).unwrap();
        library.insert(&item("b", "Plain Stone")).unwrap();

        // `%` is not a wildcard here, so only the literal match is returned.
        let hits = library.search("%").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].asset_id, "a");

        let underscore = library.search("_").unwrap();
        assert!(underscore.is_empty());
    }

    #[test]
    fn the_library_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite");

        {
            let library = Library::open(&path).unwrap();
            library.insert(&item("a", "Highland Rock")).unwrap();
        }

        let reopened = Library::open(&path).unwrap();
        assert_eq!(reopened.len().unwrap(), 1);
        assert_eq!(
            reopened.get(&AssetId::new("a")).unwrap().unwrap().title,
            "Highland Rock"
        );
    }

    #[test]
    fn verify_reports_a_healthy_library() {
        let library = library();
        library.insert(&item("a", "Rock")).unwrap();

        let report = library.verify().unwrap();
        assert!(report.is_healthy());
        assert_eq!(report.total, 1);
    }

    #[test]
    fn verify_catches_an_unreadable_row() {
        let library = library();
        library.insert(&item("a", "Rock")).unwrap();
        // Write a row the domain model cannot read back, as a corrupt or
        // changed schema might.
        library
            .conn
            .execute(
                "INSERT INTO assets (id, title, item) VALUES ('bad', 'Bad', 'not json')",
                [],
            )
            .unwrap();

        let report = library.verify().unwrap();
        assert_eq!(report.total, 2);
        assert!(!report.is_healthy());
        assert_eq!(report.unreadable.len(), 1);
        assert_eq!(report.unreadable[0].id, "bad");
    }

    #[test]
    fn a_download_records_where_the_files_live() {
        let library = library();
        library
            .record_download(&AssetId::new("a"), Path::new("/lib/a/fbx"), "fbx")
            .unwrap();

        let found = library.download(&AssetId::new("a")).unwrap().unwrap();
        assert_eq!(found.path, PathBuf::from("/lib/a/fbx"));
        assert_eq!(found.format, "fbx");
        assert!(library.is_downloaded(&AssetId::new("a")).unwrap());
    }

    #[test]
    fn redownloading_repoints_the_record_rather_than_adding_one() {
        let library = library();
        library
            .record_download(&AssetId::new("a"), Path::new("/old"), "fbx")
            .unwrap();
        library
            .record_download(&AssetId::new("a"), Path::new("/new/glb"), "glb")
            .unwrap();

        let found = library.download(&AssetId::new("a")).unwrap().unwrap();
        assert_eq!(found.path, PathBuf::from("/new/glb"));
        assert_eq!(found.format, "glb");
    }

    #[test]
    fn a_download_is_tracked_without_the_fab_item_being_stored() {
        // The downloads table stands alone, so `download` records work even
        // before `assetcraft library` has synced the asset's metadata.
        let library = library();
        library
            .record_download(&AssetId::new("a"), Path::new("/lib/a"), "fbx")
            .unwrap();

        assert!(library.get(&AssetId::new("a")).unwrap().is_none());
        assert!(library.is_downloaded(&AssetId::new("a")).unwrap());
    }

    #[test]
    fn an_asset_never_downloaded_has_no_record() {
        let library = library();
        assert!(library.download(&AssetId::new("a")).unwrap().is_none());
        assert!(!library.is_downloaded(&AssetId::new("a")).unwrap());
    }

    #[test]
    fn forgetting_a_download_leaves_the_asset_in_place() {
        let library = library();
        library.insert(&item("a", "Rock")).unwrap();
        library
            .record_download(&AssetId::new("a"), Path::new("/lib/a"), "fbx")
            .unwrap();

        assert!(library.forget_download(&AssetId::new("a")).unwrap());
        assert!(!library.forget_download(&AssetId::new("a")).unwrap());
        assert!(!library.is_downloaded(&AssetId::new("a")).unwrap());
        assert!(library.contains(&AssetId::new("a")).unwrap());
    }

    #[test]
    fn removing_an_asset_clears_its_download_record() {
        let library = library();
        library.insert(&item("a", "Rock")).unwrap();
        library
            .record_download(&AssetId::new("a"), Path::new("/lib/a"), "fbx")
            .unwrap();

        assert!(library.remove(&AssetId::new("a")).unwrap());
        assert!(!library.is_downloaded(&AssetId::new("a")).unwrap());
    }

    #[test]
    fn repair_leaves_a_healthy_library_untouched() {
        let library = library();
        library.insert(&item("a", "Highland Rock")).unwrap();

        let report = library.repair().unwrap();
        assert_eq!(report.total, 1);
        assert_eq!(report.titles_repaired, 0);
        assert!(report.removed.is_empty());
        assert!(report.is_healthy());
        assert!(library.contains(&AssetId::new("a")).unwrap());
    }

    #[test]
    fn repair_realigns_a_stale_title() {
        let library = library();
        library.insert(&item("a", "Highland Rock")).unwrap();
        // Drift the denormalised title away from the item's own title.
        library
            .conn
            .execute("UPDATE assets SET title = 'Wrong' WHERE id = 'a'", [])
            .unwrap();

        let report = library.repair().unwrap();
        assert_eq!(report.titles_repaired, 1);
        assert!(report.removed.is_empty());
        assert_eq!(
            library.get(&AssetId::new("a")).unwrap().unwrap().title,
            "Highland Rock"
        );
        // The rebuilt title is what search now runs against.
        assert_eq!(library.search("Highland").unwrap().len(), 1);
    }

    #[test]
    fn repair_drops_an_unreadable_row() {
        let library = library();
        library.insert(&item("a", "Rock")).unwrap();
        library
            .conn
            .execute(
                "INSERT INTO assets (id, title, item) VALUES ('bad', 'Bad', 'not json')",
                [],
            )
            .unwrap();

        let report = library.repair().unwrap();
        assert_eq!(report.total, 2);
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].id, "bad");
        assert!(!report.is_healthy());
        assert_eq!(library.len().unwrap(), 1);
        assert!(library.get(&AssetId::new("a")).unwrap().is_some());
        assert!(library.verify().unwrap().is_healthy());
    }

    #[test]
    fn repair_keeps_the_download_record_of_a_dropped_row() {
        let library = library();
        library
            .record_download(&AssetId::new("bad"), Path::new("/lib/bad"), "fbx")
            .unwrap();
        library
            .conn
            .execute(
                "INSERT INTO assets (id, title, item) VALUES ('bad', 'Bad', 'not json')",
                [],
            )
            .unwrap();

        library.repair().unwrap();
        // The metadata row is gone, but the on-disk files stay tracked.
        assert!(!library.contains(&AssetId::new("bad")).unwrap());
        assert!(library.download(&AssetId::new("bad")).unwrap().is_some());
    }

    #[test]
    fn favorites_round_trip_and_survive_a_reopen() {
        let file = tempfile::tempdir().unwrap().keep().join("lib.sqlite");
        let library = Library::open(&file).unwrap();
        assert!(library.favorite(&AssetId::new("a")).unwrap());
        // Favouriting twice adds nothing.
        assert!(!library.favorite(&AssetId::new("a")).unwrap());
        assert!(library.favorite(&AssetId::new("b")).unwrap());
        assert_eq!(
            library.favorites().unwrap(),
            ["a".to_string(), "b".to_string()]
        );

        drop(library);
        let reopened = Library::open(&file).unwrap();
        assert_eq!(
            reopened.favorites().unwrap(),
            ["a".to_string(), "b".to_string()]
        );
        assert!(reopened.unfavorite(&AssetId::new("a")).unwrap());
        assert!(!reopened.unfavorite(&AssetId::new("a")).unwrap());
        assert_eq!(reopened.favorites().unwrap(), ["b".to_string()]);
    }

    #[test]
    fn collections_need_a_name_and_a_unique_one() {
        let library = library();
        assert!(library.create_collection("   ").is_err());
        let rocks = library.create_collection("  Rocks  ").unwrap();
        assert_eq!(rocks.name, "Rocks", "the name arrives trimmed");
        assert_eq!(rocks.asset_count, 0);
        // Case-insensitive: two panels differing only in capitalisation would
        // defeat the point of organising.
        assert!(library.create_collection("rocks").is_err());
        // Renaming onto its own name (any capitalisation) is a no-op, not a
        // clash with itself.
        assert!(library.rename_collection(rocks.id, "ROCKS ").is_ok());
        assert!(library.rename_collection(rocks.id, "Stones").unwrap());
        assert_eq!(library.list_collections().unwrap()[0].name, "Stones");
        assert!(!library.rename_collection(404, "Whatever").unwrap());
    }

    #[test]
    fn membership_adds_once_orders_by_title_and_can_be_taken_back() {
        let library = library();
        for (id, title) in [("b", "Beta Rock"), ("a", "Alpha Rock")] {
            library.insert(&item(id, title)).unwrap();
        }
        let rocks = library.create_collection("Rocks").unwrap();

        assert!(library
            .add_to_collection(rocks.id, &AssetId::new("b"))
            .unwrap());
        assert!(library
            .add_to_collection(rocks.id, &AssetId::new("a"))
            .unwrap());
        assert!(!library
            .add_to_collection(rocks.id, &AssetId::new("a"))
            .unwrap());

        // Title order, so a collection reads like the Local view rather than in
        // whatever order boxes got clicked.
        assert_eq!(
            library.collection_members(rocks.id).unwrap(),
            ["a".to_string(), "b".to_string()]
        );
        assert_eq!(
            library.collections_for(&AssetId::new("a")).unwrap(),
            vec![rocks.id]
        );

        let listed = library.list_collections().unwrap();
        assert_eq!(listed[0].asset_count, 2);

        assert!(library
            .remove_from_collection(rocks.id, &AssetId::new("a"))
            .unwrap());
        assert!(!library
            .remove_from_collection(rocks.id, &AssetId::new("a"))
            .unwrap());
        assert_eq!(library.list_collections().unwrap()[0].asset_count, 1);
    }

    #[test]
    fn deleting_a_collection_or_an_asset_clears_membership_not_assets() {
        let library = library();
        library.insert(&item("a", "Alpha")).unwrap();
        let rocks = library.create_collection("Rocks").unwrap();
        library
            .add_to_collection(rocks.id, &AssetId::new("a"))
            .unwrap();

        assert!(library.delete_collection(rocks.id).unwrap());
        assert!(!library.delete_collection(rocks.id).unwrap());
        // Cascade took the membership row; the asset stays owned.
        assert!(library.collection_members(rocks.id).unwrap().is_empty());
        assert!(library.contains(&AssetId::new("a")).unwrap());

        // And removing the asset drops it from the other side of the relation.
        let again = library.create_collection("Again").unwrap();
        library
            .add_to_collection(again.id, &AssetId::new("a"))
            .unwrap();
        library.favorite(&AssetId::new("a")).unwrap();
        library.remove(&AssetId::new("a")).unwrap();
        assert!(library.collection_members(again.id).unwrap().is_empty());
        assert!(library.favorites().unwrap().is_empty());
    }

    #[test]
    fn membership_of_an_asset_the_library_lost_still_lists() {
        // A collection can name an asset whose metadata row has gone (a repair
        // dropped it). Listing must not lose the id for the missing join.
        let library = library();
        let rocks = library.create_collection("Rocks").unwrap();
        library
            .add_to_collection(rocks.id, &AssetId::new("ghost"))
            .unwrap();
        assert_eq!(
            library.collection_members(rocks.id).unwrap(),
            ["ghost".to_string()]
        );
        assert_eq!(library.list_collections().unwrap()[0].asset_count, 1);
    }
}
