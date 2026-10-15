//! Core types shared by every AssetCraft crate.
//!
//! This crate is intentionally GUI- and service-agnostic: it holds the domain
//! model, the configuration, and the error type that the catalog, download, and
//! DCC layers all build on.

pub mod config;
pub mod error;
pub mod model;

pub use config::{Config, Paths};
pub use error::{status_error, Error, Result};
pub use model::{
    AssetDetail, AssetFile, AssetFormat, AssetId, AssetSummary, Category, FormatType, LicenseType,
    Page, Rating, SearchQuery, Seller, SortOrder, Tag, TagGroup,
};
