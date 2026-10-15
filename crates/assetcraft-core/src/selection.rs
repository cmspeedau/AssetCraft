//! Choosing which format and file of an asset to download.
//!
//! Fab describes an asset's downloads as a list of [`AssetFormat`]s, each with
//! one or more [`AssetFile`]s. There is no single "quality" field: a Megascans
//! listing expresses the tier in the file's name or asset type, and the wording
//! varies between sellers. So a selection matches by *substring* against the
//! fields that carry it rather than hard-coding a tier list — guessing at names
//! we have not seen would be exactly the kind of claim this project avoids. A
//! caller who knows the exact string passes it; a caller who only knows `4K`
//! still gets a match.
//!
//! Selection is pure: it reads the format metadata and returns the format and
//! file to request. Turning that file into concrete bytes needs Fab's binary
//! manifest, which is tracked separately in ROADMAP.md.

use crate::{AssetFile, AssetFormat, Error, Result};

/// Which file to prefer when several match the criteria.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preference {
    /// The first file in the order Fab listed it.
    #[default]
    First,
    /// The largest file by reported size; files with no reported size rank
    /// last.
    Largest,
    /// The smallest file by reported size; files with no reported size rank
    /// last.
    Smallest,
}

/// What to look for when picking a format and file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionCriteria {
    /// Format code to require, e.g. `fbx` or `unreal-engine`.
    pub format: Option<String>,
    /// Text that must appear in the file's name or asset type, e.g. `4K`.
    pub quality: Option<String>,
    /// A platform the file must support, e.g. `Windows`.
    pub platform: Option<String>,
    /// An engine version the file must support, e.g. `5.4`.
    pub engine_version: Option<String>,
    /// How to choose among several matching files.
    pub preference: Preference,
}

impl SelectionCriteria {
    /// Empty criteria, which selects the first file of the first format.
    pub fn new() -> Self {
        Self::default()
    }

    /// Require a format code.
    pub fn with_format(mut self, code: impl Into<String>) -> Self {
        self.format = Some(code.into());
        self
    }

    /// Require `text` to appear in the file's name or asset type.
    pub fn with_quality(mut self, text: impl Into<String>) -> Self {
        self.quality = Some(text.into());
        self
    }

    /// Require a supported platform.
    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = Some(platform.into());
        self
    }

    /// Require a supported engine version.
    pub fn with_engine_version(mut self, version: impl Into<String>) -> Self {
        self.engine_version = Some(version.into());
        self
    }

    /// Choose how to break a tie between matching files.
    pub fn preferring(mut self, preference: Preference) -> Self {
        self.preference = preference;
        self
    }

    /// Whether these criteria narrow anything at all.
    pub fn is_unconstrained(&self) -> bool {
        self.format.is_none()
            && self.quality.is_none()
            && self.platform.is_none()
            && self.engine_version.is_none()
    }
}

/// A format and file chosen from an asset's download options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The chosen format's code, when it has one.
    pub format_code: Option<String>,
    /// The chosen format's display name, when it has one.
    pub format_name: Option<String>,
    /// The chosen file.
    pub file: AssetFile,
}

impl Selection {
    /// The chosen file's id, when Fab reported one.
    pub fn file_id(&self) -> Option<&str> {
        self.file.id.as_deref()
    }
}

/// Pick the format and file from `formats` that best match `criteria`.
pub fn select(formats: &[AssetFormat], criteria: &SelectionCriteria) -> Result<Selection> {
    let matching_formats: Vec<&AssetFormat> = formats
        .iter()
        .filter(|format| format_matches(format, criteria))
        .collect();
    if matching_formats.is_empty() {
        return Err(no_format(criteria, formats));
    }

    let mut candidates: Vec<(&AssetFormat, &AssetFile)> = Vec::new();
    for format in &matching_formats {
        for file in &format.files {
            if file_matches(file, criteria) {
                candidates.push((format, file));
            }
        }
    }
    if candidates.is_empty() {
        return Err(no_file(criteria));
    }

    let (format, file) = match criteria.preference {
        Preference::First => candidates[0],
        // An unknown size is ranked last in both directions by giving it the
        // extreme value on the side the comparison does not want.
        Preference::Largest => *candidates
            .iter()
            .max_by_key(|(_, file)| file.size_bytes.unwrap_or(0))
            .expect("candidates is not empty"),
        Preference::Smallest => *candidates
            .iter()
            .min_by_key(|(_, file)| file.size_bytes.unwrap_or(u64::MAX))
            .expect("candidates is not empty"),
    };

    Ok(Selection {
        format_code: format.code().map(str::to_string),
        format_name: format
            .format_type
            .as_ref()
            .and_then(|kind| kind.name.clone()),
        file: file.clone(),
    })
}

/// Whether a format satisfies the criteria's format code.
fn format_matches(format: &AssetFormat, criteria: &SelectionCriteria) -> bool {
    match criteria.format.as_deref() {
        None => true,
        Some(want) => format
            .code()
            .is_some_and(|code| code.eq_ignore_ascii_case(want)),
    }
}

/// Whether a file satisfies the quality, platform and engine criteria.
fn file_matches(file: &AssetFile, criteria: &SelectionCriteria) -> bool {
    if let Some(quality) = criteria.quality.as_deref().filter(|q| !q.is_empty()) {
        let carries_quality = [file.name.as_deref(), file.asset_type.as_deref()]
            .into_iter()
            .flatten()
            .any(|field| contains_ignore_case(field, quality));
        if !carries_quality {
            return false;
        }
    }
    if let Some(platform) = criteria.platform.as_deref().filter(|p| !p.is_empty()) {
        if !file
            .platforms
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(platform))
        {
            return false;
        }
    }
    if let Some(version) = criteria.engine_version.as_deref().filter(|v| !v.is_empty()) {
        if !file
            .engine_versions
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(version))
        {
            return false;
        }
    }
    true
}

/// Case-insensitive substring test.
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// The error for a format that could not be found.
fn no_format(criteria: &SelectionCriteria, formats: &[AssetFormat]) -> Error {
    match criteria.format.as_deref() {
        Some(want) => {
            let available: Vec<&str> = formats.iter().filter_map(|format| format.code()).collect();
            let available = if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            };
            Error::NoMatch(format!(
                "no format {want:?} for this asset (available: {available})"
            ))
        }
        None => Error::NoMatch("this asset lists no downloadable formats".to_string()),
    }
}

/// The error for criteria no file satisfied.
fn no_file(criteria: &SelectionCriteria) -> Error {
    let mut wanted = Vec::new();
    if let Some(quality) = &criteria.quality {
        wanted.push(format!("quality {quality:?}"));
    }
    if let Some(platform) = &criteria.platform {
        wanted.push(format!("platform {platform:?}"));
    }
    if let Some(version) = &criteria.engine_version {
        wanted.push(format!("engine {version:?}"));
    }
    if wanted.is_empty() {
        Error::NoMatch("the matching format has no files".to_string())
    } else {
        Error::NoMatch(format!("no file matched {}", wanted.join(" and ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FormatType;

    fn format(code: &str, files: Vec<AssetFile>) -> AssetFormat {
        AssetFormat {
            format_type: Some(FormatType {
                code: Some(code.to_string()),
                name: Some(code.to_uppercase()),
                icon: None,
            }),
            files,
        }
    }

    fn file(id: &str, name: &str, size: Option<u64>) -> AssetFile {
        AssetFile {
            id: Some(id.to_string()),
            name: Some(name.to_string()),
            size_bytes: size,
            ..Default::default()
        }
    }

    /// Two formats: UE with a big pack and FBX with 2K and 4K zips.
    fn catalog() -> Vec<AssetFormat> {
        let mut two_k = file("fbx-2k", "rock_2K.zip", Some(1_000));
        two_k.platforms = vec!["Windows".to_string(), "Mac".to_string()];
        two_k.engine_versions = vec!["5.4".to_string()];

        let mut four_k = file("fbx-4k", "rock_4K.zip", Some(4_000));
        four_k.platforms = vec!["Windows".to_string()];
        four_k.engine_versions = vec!["5.4".to_string(), "5.5".to_string()];

        vec![
            format("unreal-engine", vec![file("ue-1", "RockPack", Some(9_000))]),
            format("fbx", vec![two_k, four_k]),
        ]
    }

    #[test]
    fn no_criteria_picks_the_first_file_of_the_first_format() {
        let chosen = select(&catalog(), &SelectionCriteria::new()).unwrap();
        assert_eq!(chosen.format_code.as_deref(), Some("unreal-engine"));
        assert_eq!(chosen.file_id(), Some("ue-1"));
    }

    #[test]
    fn format_is_matched_case_insensitively() {
        let chosen = select(&catalog(), &SelectionCriteria::new().with_format("FBX")).unwrap();
        assert_eq!(chosen.format_code.as_deref(), Some("fbx"));
        assert_eq!(chosen.format_name.as_deref(), Some("FBX"));
        assert_eq!(chosen.file_id(), Some("fbx-2k"));
    }

    #[test]
    fn an_unknown_format_reports_what_is_available() {
        let err = select(&catalog(), &SelectionCriteria::new().with_format("gltf")).unwrap_err();
        match err {
            Error::NoMatch(message) => {
                assert!(message.contains("gltf"), "got {message}");
                assert!(message.contains("unreal-engine, fbx"), "got {message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn quality_matches_a_substring_of_the_name() {
        let chosen = select(
            &catalog(),
            &SelectionCriteria::new()
                .with_format("fbx")
                .with_quality("4k"),
        )
        .unwrap();
        assert_eq!(chosen.file_id(), Some("fbx-4k"));
    }

    #[test]
    fn quality_also_matches_the_asset_type() {
        let formats = vec![format(
            "fbx",
            vec![AssetFile {
                id: Some("f1".to_string()),
                asset_type: Some("High Detail".to_string()),
                ..Default::default()
            }],
        )];
        let chosen = select(&formats, &SelectionCriteria::new().with_quality("high")).unwrap();
        assert_eq!(chosen.file_id(), Some("f1"));
    }

    #[test]
    fn a_quality_with_no_match_fails() {
        let err = select(&catalog(), &SelectionCriteria::new().with_quality("8k")).unwrap_err();
        assert!(
            matches!(err, Error::NoMatch(ref m) if m.contains("8k")),
            "got {err:?}"
        );
    }

    #[test]
    fn platform_narrows_the_files() {
        let chosen = select(
            &catalog(),
            &SelectionCriteria::new()
                .with_format("fbx")
                .with_quality("4k")
                .with_platform("windows"),
        )
        .unwrap();
        assert_eq!(chosen.file_id(), Some("fbx-4k"));

        let err = select(
            &catalog(),
            &SelectionCriteria::new()
                .with_format("fbx")
                .with_quality("4k")
                .with_platform("linux"),
        )
        .unwrap_err();
        assert!(matches!(err, Error::NoMatch(_)), "got {err:?}");
    }

    #[test]
    fn engine_version_narrows_the_files() {
        let chosen = select(
            &catalog(),
            &SelectionCriteria::new()
                .with_format("fbx")
                .with_engine_version("5.5"),
        )
        .unwrap();
        assert_eq!(chosen.file_id(), Some("fbx-4k"));
    }

    #[test]
    fn preference_largest_and_smallest_pick_by_size() {
        let criteria = SelectionCriteria::new().with_format("fbx");
        let largest = select(
            &catalog(),
            &criteria.clone().preferring(Preference::Largest),
        )
        .unwrap();
        assert_eq!(largest.file_id(), Some("fbx-4k"));
        let smallest = select(&catalog(), &criteria.preferring(Preference::Smallest)).unwrap();
        assert_eq!(smallest.file_id(), Some("fbx-2k"));
    }

    #[test]
    fn files_without_a_size_rank_last_in_both_directions() {
        let formats = vec![format(
            "fbx",
            vec![
                file("unknown", "no size", None),
                file("small", "small", Some(10)),
                file("large", "large", Some(1_000)),
            ],
        )];
        let largest = select(
            &formats,
            &SelectionCriteria::new().preferring(Preference::Largest),
        )
        .unwrap();
        assert_eq!(largest.file_id(), Some("large"));
        let smallest = select(
            &formats,
            &SelectionCriteria::new().preferring(Preference::Smallest),
        )
        .unwrap();
        assert_eq!(smallest.file_id(), Some("small"));
    }

    #[test]
    fn an_empty_format_list_fails_cleanly() {
        let err = select(&[], &SelectionCriteria::new()).unwrap_err();
        assert!(
            matches!(err, Error::NoMatch(ref m) if m.contains("no downloadable formats")),
            "got {err:?}"
        );
    }

    #[test]
    fn a_matching_format_with_no_files_fails_cleanly() {
        let formats = vec![format("fbx", Vec::new())];
        let err = select(&formats, &SelectionCriteria::new().with_format("fbx")).unwrap_err();
        assert!(
            matches!(err, Error::NoMatch(ref m) if m.contains("no files")),
            "got {err:?}"
        );
    }

    #[test]
    fn unconstrained_reports_whether_anything_is_narrowed() {
        assert!(SelectionCriteria::new().is_unconstrained());
        assert!(!SelectionCriteria::new()
            .with_quality("4k")
            .is_unconstrained());
        assert!(!SelectionCriteria::new()
            .with_format("fbx")
            .is_unconstrained());
    }
}
