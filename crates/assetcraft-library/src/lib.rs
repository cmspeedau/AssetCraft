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

/// Where one asset's files were brought down to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    /// The directory holding the asset's downloaded files.
    pub path: PathBuf,
    /// The format code the files came from, when it was recorded.
    pub format: String,
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
    pub fn remove(&self, id: &AssetId) -> Result<bool> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM assets WHERE id = ?1",
                rusqlite::params![id.as_str()],
            )
            .map_err(database)?;
        self.conn
            .execute(
                "DELETE FROM downloads WHERE asset_id = ?1",
                rusqlite::params![id.as_str()],
            )
            .map_err(database)?;
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
}
