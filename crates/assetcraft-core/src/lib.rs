//! Core types shared by every AssetCraft crate.
//!
//! This crate is intentionally GUI- and service-agnostic: it holds the domain
//! model, the configuration, the error type, and the pure selection logic that
//! the catalog, download, and DCC layers all build on.

pub mod config;
pub mod error;
pub mod model;
pub mod selection;

pub use config::{Config, Paths};
pub use error::{status_error, Error, Result};
pub use model::{
    AssetDetail, AssetFile, AssetFormat, AssetId, AssetImage, AssetSummary, Category,
    DistributionPoint, DownloadInfo, FormatType, LibraryItem, LicenseType, Page, Rating,
    SearchQuery, Seller, SortOrder, Tag, TagGroup,
};
pub use selection::{select, Preference, Selection, SelectionCriteria};
