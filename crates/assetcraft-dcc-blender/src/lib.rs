//! Blender integration.
//!
//! M4 builds the exporter itself — folder layout, PBR material graph, and a
//! Python addon bridge. What is here now is the first thing that exporter needs:
//! working out which PBR slot a texture file belongs to from its name, using
//! the naming convention asset libraries follow, and the standard export path.

use std::path::{Path, PathBuf};

use assetcraft_core::AssetId;

pub mod detect;
pub mod exporter;
pub mod material;
pub mod plan;
pub mod script;

pub use exporter::{export, ExportOptions, ExportReport};
pub use material::{Binding, MaterialSpec, TextureMap};
pub use plan::{plan, ExportPlan, FileKind, PlannedFile};
pub use script::material_script;

/// A PBR texture slot that a file can fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureRole {
    /// Albedo / base colour.
    BaseColor,
    /// Tangent-space normal map.
    Normal,
    /// Roughness.
    Roughness,
    /// Metallic.
    Metallic,
    /// Ambient occlusion.
    AmbientOcclusion,
    /// Displacement / height.
    Displacement,
    /// Opacity / alpha.
    Opacity,
    /// Specular.
    Specular,
    /// Glossiness / smoothness.
    Glossiness,
    /// Emissive.
    Emissive,
    /// Not recognised.
    Unknown,
}

impl TextureRole {
    /// A stable name for the role, for logs and the UI.
    pub const fn as_str(self) -> &'static str {
        match self {
            TextureRole::BaseColor => "baseColor",
            TextureRole::Normal => "normal",
            TextureRole::Roughness => "roughness",
            TextureRole::Metallic => "metallic",
            TextureRole::AmbientOcclusion => "ambientOcclusion",
            TextureRole::Displacement => "displacement",
            TextureRole::Opacity => "opacity",
            TextureRole::Specular => "specular",
            TextureRole::Glossiness => "glossiness",
            TextureRole::Emissive => "emissive",
            TextureRole::Unknown => "unknown",
        }
    }
}

/// Suffixes that identify a role, most specific first.
///
/// Order matters: `basecolor` must be tested before `color`, and `glossiness`
/// before `gloss`, or the shorter suffix would win.
const ROLE_SUFFIXES: &[(&str, TextureRole)] = &[
    ("normalgl", TextureRole::Normal),
    ("normaldx", TextureRole::Normal),
    ("normal", TextureRole::Normal),
    ("nrm", TextureRole::Normal),
    ("norm", TextureRole::Normal),
    ("bump", TextureRole::Normal),
    ("basecolor", TextureRole::BaseColor),
    ("albedo", TextureRole::BaseColor),
    ("diffuse", TextureRole::BaseColor),
    ("diff", TextureRole::BaseColor),
    ("colour", TextureRole::BaseColor),
    ("color", TextureRole::BaseColor),
    ("roughness", TextureRole::Roughness),
    ("rough", TextureRole::Roughness),
    ("rgh", TextureRole::Roughness),
    ("metallic", TextureRole::Metallic),
    ("metalness", TextureRole::Metallic),
    ("metal", TextureRole::Metallic),
    ("ambientocclusion", TextureRole::AmbientOcclusion),
    ("occlusion", TextureRole::AmbientOcclusion),
    ("ao", TextureRole::AmbientOcclusion),
    ("displacement", TextureRole::Displacement),
    ("displace", TextureRole::Displacement),
    ("height", TextureRole::Displacement),
    ("disp", TextureRole::Displacement),
    ("hgt", TextureRole::Displacement),
    ("opacity", TextureRole::Opacity),
    ("alpha", TextureRole::Opacity),
    ("glossiness", TextureRole::Glossiness),
    ("gloss", TextureRole::Glossiness),
    ("smoothness", TextureRole::Glossiness),
    ("smooth", TextureRole::Glossiness),
    ("specular", TextureRole::Specular),
    ("spec", TextureRole::Specular),
    ("emissive", TextureRole::Emissive),
    ("emission", TextureRole::Emissive),
    ("emit", TextureRole::Emissive),
];

/// A texture file name, read for the role it fills and the material it belongs
/// to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    /// The role the name's token puts it in.
    pub role: TextureRole,
    /// The part of the stem before the role token, trimmed of the separators
    /// between them — the material the map belongs to. `None` when no role
    /// matched, or when the token is the whole stem.
    pub material: Option<String>,
}

/// Classify a texture file name by the role token it ends with, and recover the
/// material name in front of that token.
///
/// Works on stems like `rock_basecolor`, `T_Rock_Base_Color`, or `boulder_AO`.
/// Only the file name is inspected, so a full path is fine.
///
/// ```
/// use assetcraft_dcc_blender::{classify, TextureRole};
/// let rock = classify("rock_basecolor.png");
/// assert_eq!(rock.role, TextureRole::BaseColor);
/// assert_eq!(rock.material.as_deref(), Some("rock"));
///
/// let unnamed = classify("normal.png");
/// assert_eq!(unnamed.role, TextureRole::Normal);
/// assert_eq!(unnamed.material, None);
/// ```
pub fn classify(file_name: &str) -> Classified {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    let normalized: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();

    match ROLE_SUFFIXES
        .iter()
        .find(|(suffix, _)| normalized.ends_with(suffix))
    {
        Some((suffix, role)) => Classified {
            role: *role,
            material: strip_role_token(stem, suffix),
        },
        None => Classified {
            role: TextureRole::Unknown,
            material: None,
        },
    }
}

/// Drop the role token from the end of `stem`, keeping what came before it.
///
/// The token was matched against a stem with its non-alphanumerics removed, so
/// the match is re-found here character by character from the end, which lets
/// the original separators (and case) survive in the material name.
fn strip_role_token(stem: &str, suffix: &str) -> Option<String> {
    let mut matched = 0usize;
    let mut cut = 0usize;
    for (index, ch) in stem.char_indices().rev() {
        if ch.is_ascii_alphanumeric() {
            matched += 1;
            if matched == suffix.len() {
                cut = index;
                break;
            }
        }
    }
    if matched != suffix.len() {
        return None;
    }
    let material = stem[..cut]
        .trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_string();
    (!material.is_empty()).then_some(material)
}

/// Classify a texture file name by the role token it ends with.
///
/// A thin wrapper over [`classify`] for callers that only want the role.
///
/// ```
/// use assetcraft_dcc_blender::{classify_texture, TextureRole};
/// assert_eq!(classify_texture("rock_basecolor.png"), TextureRole::BaseColor);
/// assert_eq!(classify_texture("rock_normal.png"), TextureRole::Normal);
/// assert_eq!(classify_texture("mystery.png"), TextureRole::Unknown);
/// ```
pub fn classify_texture(file_name: &str) -> TextureRole {
    classify(file_name).role
}

/// The directory an asset's files export into: `<root>/<asset id>/<format>`.
///
/// Keeping this in one place is what makes the on-disk layout predictable for
/// every DCC, rather than each integration inventing its own.
pub fn export_dir(root: impl AsRef<Path>, asset: &AssetId, format_code: &str) -> PathBuf {
    root.as_ref().join(asset.as_str()).join(format_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_common_base_colour_names() {
        assert_eq!(
            classify_texture("rock_basecolor.png"),
            TextureRole::BaseColor
        );
        assert_eq!(
            classify_texture("rock_base_color.png"),
            TextureRole::BaseColor
        );
        assert_eq!(classify_texture("Rock_Albedo.jpg"), TextureRole::BaseColor);
        assert_eq!(
            classify_texture("T_sand_diffuse.tif"),
            TextureRole::BaseColor
        );
        assert_eq!(classify_texture("bark_color.png"), TextureRole::BaseColor);
    }

    #[test]
    fn recognises_normal_maps_including_directx_variants() {
        assert_eq!(classify_texture("rock_normal.png"), TextureRole::Normal);
        assert_eq!(classify_texture("rock_nrm.png"), TextureRole::Normal);
        assert_eq!(classify_texture("rock_normalgl.png"), TextureRole::Normal);
        assert_eq!(classify_texture("rock_normaldx.png"), TextureRole::Normal);
    }

    #[test]
    fn recognises_the_remaining_pbr_slots() {
        assert_eq!(
            classify_texture("rock_roughness.png"),
            TextureRole::Roughness
        );
        assert_eq!(classify_texture("rock_metallic.png"), TextureRole::Metallic);
        assert_eq!(
            classify_texture("rock_ao.png"),
            TextureRole::AmbientOcclusion
        );
        assert_eq!(
            classify_texture("rock_displacement.png"),
            TextureRole::Displacement
        );
        assert_eq!(
            classify_texture("rock_height.exr"),
            TextureRole::Displacement
        );
        assert_eq!(classify_texture("rock_opacity.png"), TextureRole::Opacity);
        assert_eq!(classify_texture("rock_specular.png"), TextureRole::Specular);
        assert_eq!(
            classify_texture("rock_glossiness.png"),
            TextureRole::Glossiness
        );
        assert_eq!(classify_texture("rock_emissive.png"), TextureRole::Emissive);
    }

    #[test]
    fn prefers_the_more_specific_suffix() {
        // `basecolor` ends with `color`; only one role should win.
        assert_eq!(classify_texture("x_basecolor"), TextureRole::BaseColor);
        // `glossiness` ends with `gloss`.
        assert_eq!(classify_texture("x_glossiness"), TextureRole::Glossiness);
    }

    #[test]
    fn unknown_names_are_reported_as_unknown() {
        assert_eq!(classify_texture("mystery.png"), TextureRole::Unknown);
        assert_eq!(classify_texture(""), TextureRole::Unknown);
    }

    #[test]
    fn accepts_a_full_path() {
        assert_eq!(
            classify_texture("/assets/rock/rock_basecolor.png"),
            TextureRole::BaseColor
        );
    }

    #[test]
    fn recovers_the_material_name_before_the_role_token() {
        assert_eq!(
            classify("rock_basecolor.png").material.as_deref(),
            Some("rock")
        );
        assert_eq!(
            classify("T_Rock_Base_Color.png").material.as_deref(),
            Some("T_Rock")
        );
        assert_eq!(
            classify("boulder-AO.png").material.as_deref(),
            Some("boulder")
        );
    }

    #[test]
    fn a_token_with_no_name_before_it_has_no_material() {
        assert_eq!(classify("normal.png").material, None);
        assert_eq!(classify("basecolor.png").material, None);
    }

    #[test]
    fn an_unknown_name_has_neither_role_nor_material() {
        let classified = classify("mystery.png");
        assert_eq!(classified.role, TextureRole::Unknown);
        assert_eq!(classified.material, None);
    }

    #[test]
    fn export_dir_uses_asset_then_format() {
        let dir = export_dir("/lib", &AssetId::new("abc-123"), "fbx");
        assert_eq!(dir, PathBuf::from("/lib/abc-123/fbx"));
    }
}
