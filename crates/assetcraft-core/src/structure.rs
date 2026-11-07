//! What an asset's files actually contain: its texture maps and levels of detail.
//!
//! Fab's API says an asset is a 3D model available as FBX, glTF, and Unreal. It
//! does not say the asset is one mesh with three levels of detail and four PBR
//! maps. That information is in the *file names* the manifest lists, and every
//! layer wants it: the browser shows it as asset detail, the Blender exporter
//! wires its material graph from it, and the next adapter will do the same. So
//! the reading lives here, once, as pure functions over paths and sizes — no
//! client, no async, no filesystem — which is also what lets an asset already on
//! disk be described with the network off.
//!
//! Two naming conventions carry the information, and both are asset-library
//! standards rather than anything Fab-specific:
//!
//! - a **role token** at the end of the stem names the PBR slot a map fills
//!   ([`classify`]: `rock_basecolor`, `T_Rock_Base_Color`, `boulder_AO`);
//! - a **level token** (`lod` plus digits, at a word boundary) names the level
//!   of detail a file belongs to ([`lod_tag`]: `rock_LOD0`, `rock_lod_2`,
//!   `rock@lod1`). The level token may sit anywhere in the name, so a texture can
//!   be both (`T_Rock_BaseColor_LOD1` is a base colour map, level 1).
//!
//! Two limits are worth stating, because both are visible from the file list and
//! so are honest rather than silent:
//!
//! - A mesh that packs its levels *inside* one file — an FBX containing
//!   `rock_LOD0`, `rock_LOD1` objects — reports as one level-less mesh. Names of
//!   files are all this module can see.
//! - A name that follows neither convention is reported as it reads: a texture
//!   with [`TextureRole::Unknown`], a mesh with no level. Nothing is guessed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Mesh formats AssetCraft recognises as such.
///
/// Deliberately importable formats only: an engine-native mesh (`.uasset`) is
/// not a mesh an exporter can hand to a DCC, so it stays `Other` here and is
/// kept at its original path by a layout.
const MESH_EXTENSIONS: &[&str] = &[
    "fbx", "obj", "glb", "gltf", "usd", "usda", "usdc", "usdz", "blend", "abc", "ply", "stl", "dae",
];

/// Image formats AssetCraft treats as texture maps.
const TEXTURE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "tif", "tiff", "exr", "tga", "bmp", "hdr", "dds", "webp", "psd",
];

/// Characters that separate the tokens in an asset file name.
const SEPARATORS: &[char] = &['_', '-', '.', '@', '+', ' '];

/// What a file is, for describing and laying out an asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    /// A mesh an exporter imports into a scene.
    Mesh,
    /// An image, offered to a material graph as a texture map.
    Texture,
    /// Anything else: descriptions, atlases, engine-specific data.
    Other,
}

impl FileKind {
    /// The subdirectory files of this kind land under in the standard layout.
    ///
    /// The layout is the one the Blender exporter established (roadmap M4) and
    /// every later adapter reuses, so the name of a file's folder means the same
    /// thing whatever exported the asset.
    pub const fn dir(self) -> &'static str {
        match self {
            FileKind::Mesh => "meshes",
            FileKind::Texture => "textures",
            FileKind::Other => "other",
        }
    }
}

/// What a file is, from its extension, case-insensitively.
pub fn file_kind(path: &Path) -> FileKind {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some(ext) if MESH_EXTENSIONS.contains(&ext) => FileKind::Mesh,
        Some(ext) if TEXTURE_EXTENSIONS.contains(&ext) => FileKind::Texture,
        _ => FileKind::Other,
    }
}

/// A PBR texture slot that a file can fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// Every role a name can be recognised as, in display order.
    ///
    /// `Unknown` is deliberately absent: it is the absence of a role, so a view
    /// listing the slots an asset covers shows the ten real ones and reports
    /// unrecognised maps separately.
    pub const ALL: &'static [TextureRole] = &[
        TextureRole::BaseColor,
        TextureRole::Normal,
        TextureRole::Roughness,
        TextureRole::Metallic,
        TextureRole::AmbientOcclusion,
        TextureRole::Displacement,
        TextureRole::Opacity,
        TextureRole::Specular,
        TextureRole::Glossiness,
        TextureRole::Emissive,
    ];

    /// A stable name for the role, for logs, JSON, and the UI.
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
/// The role token has to be the last thing in the stem. A name that carries a
/// level of detail as well (`rock_BaseColor_LOD1`) is classified through
/// [`without_lod_token`], which takes the level token out first; that is what
/// [`AssetStructure`] does for every file it reads.
///
/// ```
/// use assetcraft_core::{classify, TextureRole};
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
/// use assetcraft_core::{classify_texture, TextureRole};
/// assert_eq!(classify_texture("rock_basecolor.png"), TextureRole::BaseColor);
/// assert_eq!(classify_texture("rock_normal.png"), TextureRole::Normal);
/// assert_eq!(classify_texture("mystery.png"), TextureRole::Unknown);
/// ```
pub fn classify_texture(file_name: &str) -> TextureRole {
    classify(file_name).role
}

/// The level of detail a file name carries, and what the file is called without
/// the level token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LodTag {
    /// The level the digits name; `0` is the highest-detail mesh by convention.
    pub level: u32,
    /// The file name's stem with the level token removed, so the levels of one
    /// mesh group together. `None` when nothing but the token was left.
    pub base: Option<String>,
}

/// Read the level-of-detail token in a file name.
///
/// The token is `lod` at a word boundary followed by digits, and it may sit
/// anywhere in the name: `rock_LOD0`, `rock_lod_2`, `rock@lod1`,
/// `T_Rock_BaseColor_LOD2`. Requiring the boundary is what keeps a mesh called
/// `clod10` from reading as a level of detail; requiring digits is what keeps
/// `rock_lod` from reading as one either.
///
/// ```
/// use assetcraft_core::lod_tag;
/// let lod = lod_tag("rock_LOD2.fbx").unwrap();
/// assert_eq!(lod.level, 2);
/// assert_eq!(lod.base.as_deref(), Some("rock"));
///
/// assert!(lod_tag("rock.fbx").is_none());
/// assert!(lod_tag("clod10.fbx").is_none());
/// ```
pub fn lod_tag(file_name: &str) -> Option<LodTag> {
    let name = file_name_of(Path::new(file_name));
    let token = find_lod(&name)?;
    let level = token.digits(&name).parse().ok()?;
    Some(LodTag {
        level,
        base: lod_base(&cut(&name, token)),
    })
}

/// The name with its level-of-detail token removed, separators tidied up.
///
/// A name with no token comes back unchanged, so this is safe to apply to
/// anything: `rock_LOD0.fbx` → `rock.fbx`, `T_Rock_Roughness_LOD1.png` →
/// `T_Rock_Roughness.png`, `rock.fbx` → `rock.fbx`.
///
/// ```
/// use assetcraft_core::without_lod_token;
/// assert_eq!(without_lod_token("rock_LOD0.fbx"), "rock.fbx");
/// assert_eq!(without_lod_token("rock_lod_2.fbx"), "rock.fbx");
/// assert_eq!(without_lod_token("rock.fbx"), "rock.fbx");
/// ```
pub fn without_lod_token(file_name: &str) -> String {
    match find_lod(file_name) {
        Some(token) => cut(file_name, token),
        None => file_name.to_string(),
    }
}

/// The token every level of detail is named with, lower case.
const LOD_TOKEN: &str = "lod";

/// Where a level token sits in a name, counted in characters: the word, any
/// separators between the word and its digits, and the digits.
#[derive(Debug, Clone, Copy)]
struct LodSpan {
    start: usize,
    len: usize,
}

impl LodSpan {
    /// The index just past the token.
    const fn end(self) -> usize {
        self.start + self.len
    }

    /// The token's digits on their own, without the word or any separators.
    fn digits(self, name: &str) -> String {
        name.chars()
            .skip(self.start)
            .take(self.len)
            .filter(|ch| ch.is_ascii_digit())
            .collect()
    }
}

/// Find a level-of-detail token in a name.
///
/// The token must begin at a word boundary and end at one, so neither `clod10`
/// (the `lod` sits inside a word) nor `lod1x` (its digits run into a word) reads
/// as a level.
fn find_lod(file_name: &str) -> Option<LodSpan> {
    let chars: Vec<char> = file_name.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        let at_word_start = index == 0 || !chars[index - 1].is_alphanumeric();
        if at_word_start && starts_lod(&chars[index..]) {
            let mut cursor = index + LOD_TOKEN.len();
            // `_lod_2` is the same token as `_lod2`: a separator between the word
            // and the digits belongs to the token rather than to the name.
            while cursor < chars.len() && SEPARATORS.contains(&chars[cursor]) {
                cursor += 1;
            }
            let digits_start = cursor;
            while cursor < chars.len() && chars[cursor].is_ascii_digit() {
                cursor += 1;
            }
            let digits = cursor - digits_start;
            let ends_cleanly = cursor == chars.len() || !chars[cursor].is_alphanumeric();
            if digits > 0 && ends_cleanly {
                return Some(LodSpan {
                    start: index,
                    len: cursor - index,
                });
            }
        }
        index += 1;
    }
    None
}

/// Whether the characters at `at` spell `lod`, ignoring case.
fn starts_lod(at: &[char]) -> bool {
    at.len() >= LOD_TOKEN.len()
        && at[..LOD_TOKEN.len()]
            .iter()
            .map(|ch| ch.to_ascii_lowercase())
            .eq(LOD_TOKEN.chars())
}

/// Cut a level token out of a name, absorbing the separator beside it so the
/// remaining name has no doubled or dangling separator where the token was.
fn cut(name: &str, token: LodSpan) -> String {
    let chars: Vec<char> = name.chars().collect();
    let head_end = if token.start > 0 && SEPARATORS.contains(&chars[token.start - 1]) {
        token.start - 1
    } else {
        token.start
    };
    // Absorb a separator after the token too when there was none before it, but
    // only when a stem is still left beyond it: `LOD0_basecolor.png` becomes
    // `basecolor.png`, while `LOD1.fbx` keeps its `.fbx` rather than turning the
    // extension into the name.
    let end = token.end();
    let absorb_after = end < chars.len()
        && SEPARATORS.contains(&chars[end])
        && head_end == token.start
        && (chars[end] != '.' || chars[end + 1..].contains(&'.'));
    let tail_start = if absorb_after { end + 1 } else { end };
    let head: String = chars[..head_end].iter().collect();
    let tail: String = chars[tail_start..].iter().collect();
    head + &tail
}

/// The stem of a name that has had its level token removed, when anything but
/// the token was left.
fn lod_base(stripped: &str) -> Option<String> {
    let stem = Path::new(stripped)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string);
    stem.filter(|stem| !stem.is_empty() && !stem.starts_with('.'))
}

/// The name of a path's file, or the whole path when it has no file name.
fn file_name_of(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// One texture map among an asset's files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureMap {
    /// Where the map is, relative to the asset's files.
    pub path: PathBuf,
    /// The PBR slot its name names, [`TextureRole::Unknown`] when it names none.
    pub role: TextureRole,
    /// The material the map belongs to, from the name before the role token.
    pub material: Option<String>,
    /// The level of detail it belongs to, when the name says.
    pub lod_level: Option<u32>,
    /// Size in bytes, when the caller knew it.
    pub size_bytes: Option<u64>,
}

/// One mesh file among an asset's files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshFile {
    /// Where the mesh is, relative to the asset's files.
    pub path: PathBuf,
    /// The level of detail its name names, `None` when it names none.
    pub lod_level: Option<u32>,
    /// The mesh the file belongs to, its name without the level token, so the
    /// levels of one mesh group together.
    pub group: String,
    /// Size in bytes, when the caller knew it.
    pub size_bytes: Option<u64>,
}

/// The levels of detail one mesh was found at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshGroup {
    /// The mesh's name, its file names without their level tokens.
    pub name: String,
    /// The levels its files name, lowest (highest detail) first. Empty when the
    /// mesh ships a single file with no level token, or packs its levels inside
    /// that file where a file list cannot see them.
    pub levels: Vec<u32>,
    /// How many files belong to it.
    pub files: usize,
}

/// What an asset's files contain, read from their names and sizes.
///
/// Built from any list of files: the manifest of an asset about to be downloaded
/// (which names every file and its exact size), or the directory of one already
/// on disk (which is the same information with the network off).
///
/// ```
/// use assetcraft_core::{AssetStructure, TextureRole};
/// let structure = AssetStructure::from_paths([
///     "Content/rock.fbx",
///     "Content/rock_basecolor.png",
///     "Content/rock_normal.png",
///     "Content/rock_README.txt",
/// ]);
///
/// assert_eq!(structure.meshes.len(), 1);
/// assert_eq!(structure.role_counts(), vec![
///     (TextureRole::BaseColor, 1),
///     (TextureRole::Normal, 1),
/// ]);
/// assert_eq!(structure.other.len(), 1);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetStructure {
    /// Image files, each with the role its name names.
    pub textures: Vec<TextureMap>,
    /// Mesh files, each with the level of detail its name names.
    pub meshes: Vec<MeshFile>,
    /// Everything else, kept for its count and its paths.
    pub other: Vec<PathBuf>,
}

impl AssetStructure {
    /// Read a list of file paths, with sizes unknown.
    pub fn from_paths<I, P>(paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        Self::from_sized(paths.into_iter().map(|path| (path.into(), None)))
    }

    /// Read a list of files with their sizes, where known.
    pub fn from_sized<I, S>(files: I) -> Self
    where
        I: IntoIterator<Item = (PathBuf, S)>,
        S: Into<Option<u64>>,
    {
        let mut structure = Self::default();
        for (path, size) in files {
            let size = size.into();
            let name = file_name_of(&path);
            // Reading the name without its level token first is what lets
            // `T_Rock_BaseColor_LOD1` name both its role and its level.
            let role_source = without_lod_token(&name);
            let lod = lod_tag(&name).map(|tag| tag.level);
            match file_kind(&path) {
                FileKind::Mesh => {
                    let group = lod_base(&role_source).unwrap_or_else(|| file_stem(&name));
                    structure.meshes.push(MeshFile {
                        path,
                        lod_level: lod,
                        group,
                        size_bytes: size,
                    });
                }
                FileKind::Texture => {
                    let classified = classify(&role_source);
                    structure.textures.push(TextureMap {
                        path,
                        role: classified.role,
                        material: classified.material,
                        lod_level: lod,
                        size_bytes: size,
                    });
                }
                FileKind::Other => structure.other.push(path),
            }
        }
        structure
    }

    /// How many files were read.
    pub fn file_count(&self) -> usize {
        self.textures.len() + self.meshes.len() + self.other.len()
    }

    /// Whether nothing was read.
    pub fn is_empty(&self) -> bool {
        self.file_count() == 0
    }

    /// The bytes the files occupy, counting only the ones that reported a size.
    pub fn known_size_bytes(&self) -> u64 {
        let textures = self.textures.iter().map(|map| map.size_bytes);
        let meshes = self.meshes.iter().map(|mesh| mesh.size_bytes);
        textures.chain(meshes).flatten().sum()
    }

    /// How many maps fill each role, in [`TextureRole::ALL`] order, roles with no
    /// map left out.
    pub fn role_counts(&self) -> Vec<(TextureRole, usize)> {
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for map in &self.textures {
            *counts.entry(map.role.as_str()).or_default() += 1;
        }
        TextureRole::ALL
            .iter()
            .filter_map(|role| counts.get(role.as_str()).map(|count| (*role, *count)))
            .collect()
    }

    /// How many image files no role token recognised.
    pub fn unrecognised_textures(&self) -> usize {
        self.textures
            .iter()
            .filter(|map| map.role == TextureRole::Unknown)
            .count()
    }

    /// The materials the maps named, in name order, unlabelled maps left out.
    pub fn material_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .textures
            .iter()
            .filter_map(|map| map.material.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        names.sort();
        names
    }

    /// The maps of one material, unlabelled maps when `material` is `None`.
    pub fn maps_for(&self, material: Option<&str>) -> Vec<&TextureMap> {
        self.textures
            .iter()
            .filter(|map| map.material.as_deref() == material)
            .collect()
    }

    /// Every level of detail found among the meshes, lowest first.
    pub fn lod_levels(&self) -> Vec<u32> {
        self.mesh_groups()
            .into_iter()
            .flat_map(|group| group.levels)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Whether any mesh file names a level of detail.
    pub fn has_lods(&self) -> bool {
        self.meshes.iter().any(|mesh| mesh.lod_level.is_some())
    }

    /// The meshes, grouped by name, each with the levels found for it.
    ///
    /// A single mesh file with no level token is one group with no levels: that
    /// is what the file list says, whether the mesh truly has no levels or packs
    /// them inside its own file.
    pub fn mesh_groups(&self) -> Vec<MeshGroup> {
        let mut grouped: BTreeMap<&str, Vec<&MeshFile>> = BTreeMap::new();
        for mesh in &self.meshes {
            grouped.entry(mesh.group.as_str()).or_default().push(mesh);
        }
        grouped
            .into_iter()
            .map(|(name, files)| MeshGroup {
                name: name.to_string(),
                levels: files
                    .iter()
                    .filter_map(|file| file.lod_level)
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                files: files.len(),
            })
            .collect()
    }

    /// The meshes that name no level of detail.
    pub fn unlevelled_meshes(&self) -> Vec<PathBuf> {
        self.meshes
            .iter()
            .filter(|mesh| mesh.lod_level.is_none())
            .map(|mesh| mesh.path.clone())
            .collect()
    }

    /// One line saying roughly what the asset is, for a log line or a header.
    ///
    /// ```
    /// use assetcraft_core::AssetStructure;
    /// let structure = AssetStructure::from_paths([
    ///     "rock_LOD0.fbx",
    ///     "rock_LOD1.fbx",
    ///     "rock_basecolor.png",
    ///     "rock_normal.png",
    ///     "license.txt",
    /// ]);
    /// assert_eq!(structure.summary(), "2 texture maps · 1 mesh (LODs 0-1) · 1 other file");
    /// ```
    pub fn summary(&self) -> String {
        if self.is_empty() {
            return "no files".to_string();
        }
        let mut parts = Vec::new();
        if !self.textures.is_empty() {
            parts.push(plural(self.textures.len(), "texture map"));
        }
        if !self.meshes.is_empty() {
            let groups = self.mesh_groups();
            let levels = self.lod_levels();
            // Levels that only one of several meshes carries are worth saying so:
            // "2 meshes (LODs 0-1)" would read as though both had them.
            let named = groups
                .iter()
                .filter(|group| !group.levels.is_empty())
                .count();
            let detail = match levels.first() {
                None => String::new(),
                Some(_) if groups.len() > 1 && named == 1 => {
                    format!(" (LODs {} on one of them)", format_levels(&levels))
                }
                Some(_) => format!(" (LODs {})", format_levels(&levels)),
            };
            parts.push(format!("{}{}", plural(groups.len(), "mesh"), detail));
        }
        if !self.other.is_empty() {
            parts.push(plural(self.other.len(), "other file"));
        }
        parts.join(" · ")
    }
}

/// The stem of a file name, or the name itself when it has no stem.
fn file_stem(file_name: &str) -> String {
    Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| file_name.to_string())
}

/// `rock.fbx` and `mesh` count as "2 files" and "2 meshes" respectively.
fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        return format!("{count} {noun}");
    }
    let word = if noun.ends_with("sh") || noun.ends_with("ch") || noun.ends_with(['s', 'x', 'z']) {
        format!("{noun}es")
    } else {
        format!("{noun}s")
    };
    format!("{count} {word}")
}

/// Levels as a range when they are unbroken, and as a list when they are not.
fn format_levels(levels: &[u32]) -> String {
    let unbroken = levels.windows(2).all(|pair| pair[1] == pair[0] + 1);
    match (levels.first(), levels.last(), unbroken) {
        (Some(_), Some(_), true) if levels.len() > 1 => {
            format!("{}-{}", levels[0], levels[levels.len() - 1])
        }
        _ => levels
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn files_named(names: &[&str]) -> AssetStructure {
        AssetStructure::from_paths(paths(names))
    }

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
    fn every_known_role_names_itself() {
        let mut names: Vec<&str> = TextureRole::ALL.iter().map(|role| role.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TextureRole::ALL.len());
        assert_eq!(TextureRole::Unknown.as_str(), "unknown");
    }

    #[test]
    fn reads_a_level_token_at_the_end_of_a_name() {
        let lod = lod_tag("rock_LOD0.fbx").unwrap();
        assert_eq!(lod.level, 0);
        assert_eq!(lod.base.as_deref(), Some("rock"));

        assert_eq!(lod_tag("rock_lod_2.fbx").unwrap().level, 2);
        assert_eq!(lod_tag("rock-lod3.fbx").unwrap().level, 3);
        assert_eq!(lod_tag("rock@lod1.fbx").unwrap().level, 1);
        assert_eq!(lod_tag("rock.lod10.fbx").unwrap().level, 10);
        assert_eq!(lod_tag("SM_Rock_LOD00.fbx").unwrap().level, 0);
    }

    #[test]
    fn reads_a_level_token_in_the_middle_of_a_name() {
        let lod = lod_tag("T_Rock_LOD1_BaseColor.png").unwrap();
        assert_eq!(lod.level, 1);
        assert_eq!(lod.base.as_deref(), Some("T_Rock_BaseColor"));
    }

    #[test]
    fn a_level_token_has_to_be_its_own_word_with_digits_after_it() {
        // `clod10` is a word, not a level.
        assert!(lod_tag("clod10.fbx").is_none());
        // A token with no digits names nothing.
        assert!(lod_tag("rock_lod.fbx").is_none());
        // Digits running into a word are not a level either.
        assert!(lod_tag("rock_lod1x.fbx").is_none());
        assert!(lod_tag("rock.fbx").is_none());
        assert!(lod_tag("").is_none());
    }

    #[test]
    fn an_absurd_level_number_is_not_a_level() {
        assert!(lod_tag("rock_LOD99999999999999.fbx").is_none());
    }

    #[test]
    fn a_name_can_be_just_a_level() {
        let lod = lod_tag("LOD1.fbx").unwrap();
        assert_eq!(lod.level, 1);
        assert_eq!(lod.base, None);
    }

    #[test]
    fn dropping_a_level_token_leaves_a_clean_name() {
        assert_eq!(without_lod_token("rock_LOD0.fbx"), "rock.fbx");
        assert_eq!(without_lod_token("rock_lod_2.fbx"), "rock.fbx");
        assert_eq!(without_lod_token("rock@lod1.fbx"), "rock.fbx");
        assert_eq!(without_lod_token("rock.lod10.fbx"), "rock.fbx");
        assert_eq!(
            without_lod_token("T_Rock_LOD1_BaseColor.png"),
            "T_Rock_BaseColor.png"
        );
        // A name with no token is untouched, so this is safe to apply to any file.
        assert_eq!(without_lod_token("rock.fbx"), "rock.fbx");
        assert_eq!(without_lod_token("license.txt"), "license.txt");
    }

    #[test]
    fn sorts_files_by_kind() {
        let structure = files_named(&[
            "Content/rock.fbx",
            "Content/rock_basecolor.png",
            "Content/license.txt",
        ]);
        assert_eq!(structure.meshes.len(), 1);
        assert_eq!(structure.textures.len(), 1);
        assert_eq!(structure.other, paths(&["Content/license.txt"]));
        assert_eq!(structure.file_count(), 3);
    }

    #[test]
    fn groups_the_levels_of_one_mesh() {
        let structure = files_named(&[
            "meshes/SM_Rock_LOD0.fbx",
            "meshes/SM_Rock_LOD1.fbx",
            "meshes/SM_Rock_LOD2.fbx",
        ]);
        let groups = structure.mesh_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "SM_Rock");
        assert_eq!(groups[0].levels, vec![0, 1, 2]);
        assert_eq!(groups[0].files, 3);
        assert!(structure.has_lods());
        assert!(structure.unlevelled_meshes().is_empty());
    }

    #[test]
    fn keeps_meshes_apart_and_counts_their_levels_together() {
        let structure = files_named(&[
            "SM_Rock_LOD0.fbx",
            "SM_Rock_LOD1.fbx",
            "SM_Grass_LOD0.fbx",
            "SM_Grass_LOD1.fbx",
            "SM_Grass_LOD2.fbx",
        ]);
        let groups = structure.mesh_groups();
        let names: Vec<&str> = groups.iter().map(|group| group.name.as_str()).collect();
        assert_eq!(names, vec!["SM_Grass", "SM_Rock"]);
        assert_eq!(groups[0].levels, vec![0, 1, 2]);
        assert_eq!(groups[1].levels, vec![0, 1]);
        assert_eq!(structure.lod_levels(), vec![0, 1, 2]);
    }

    #[test]
    fn a_mesh_without_a_level_token_reports_no_levels() {
        let structure = files_named(&["SM_Rock.fbx"]);
        let groups = structure.mesh_groups();
        assert_eq!(groups[0].name, "SM_Rock");
        assert!(groups[0].levels.is_empty());
        assert!(!structure.has_lods());
        assert_eq!(structure.unlevelled_meshes(), paths(&["SM_Rock.fbx"]));
    }

    #[test]
    fn an_unbroken_level_list_prints_as_a_range() {
        assert_eq!(format_levels(&[0, 1, 2]), "0-2");
        assert_eq!(format_levels(&[0, 1]), "0-1");
        assert_eq!(format_levels(&[3]), "3");
        assert_eq!(format_levels(&[0, 2, 7]), "0, 2, 7");
    }

    #[test]
    fn a_level_bearing_texture_names_both_its_role_and_its_level() {
        let structure = files_named(&["T_Rock_BaseColor_LOD1.png"]);
        assert_eq!(structure.textures.len(), 1);
        assert_eq!(structure.textures[0].role, TextureRole::BaseColor);
        assert_eq!(structure.textures[0].lod_level, Some(1));
        assert_eq!(structure.textures[0].material.as_deref(), Some("T_Rock"));
    }

    #[test]
    fn groups_an_assets_maps_by_material() {
        let structure = files_named(&[
            "rock_basecolor.png",
            "rock_normal.png",
            "moss_basecolor.png",
        ]);
        assert_eq!(structure.material_names(), vec!["moss", "rock"]);
        assert_eq!(structure.maps_for(Some("rock")).len(), 2);
        assert_eq!(structure.maps_for(Some("moss")).len(), 1);
        assert!(structure.maps_for(None).is_empty());
    }

    #[test]
    fn counts_maps_by_role_in_display_order() {
        let structure = files_named(&[
            "rock_normal.png",
            "rock_normaldx.png",
            "rock_basecolor.png",
            "rock_ao.png",
            "rock_mystery.png",
        ]);
        assert_eq!(
            structure.role_counts(),
            vec![
                (TextureRole::BaseColor, 1),
                (TextureRole::Normal, 2),
                (TextureRole::AmbientOcclusion, 1),
            ]
        );
        // The unrecognised map is counted as itself, not quietly dropped.
        assert_eq!(structure.unrecognised_textures(), 1);
        assert_eq!(structure.textures.len(), 5);
    }

    #[test]
    fn keeps_sizes_and_sums_the_ones_it_knows() {
        let structure = AssetStructure::from_sized([
            (PathBuf::from("rock.fbx"), Some(2000u64)),
            (PathBuf::from("rock_basecolor.png"), Some(1000u64)),
            (PathBuf::from("rock_normal.png"), None),
            (PathBuf::from("license.txt"), Some(10u64)),
        ]);
        assert_eq!(structure.meshes[0].size_bytes, Some(2000));
        assert_eq!(structure.textures[1].size_bytes, None);
        // The mesh and one map reported sizes; the other map and the text did not.
        assert_eq!(structure.known_size_bytes(), 3000);
    }

    #[test]
    fn accepts_plain_numbers_as_sizes() {
        let structure = AssetStructure::from_sized([(PathBuf::from("rock.fbx"), 5u64)]);
        assert_eq!(structure.meshes[0].size_bytes, Some(5));
    }

    #[test]
    fn describes_the_asset_in_one_line() {
        let structure = files_named(&[
            "rock_LOD0.fbx",
            "rock_LOD1.fbx",
            "rock_basecolor.png",
            "rock_normal.png",
            "license.txt",
        ]);
        assert_eq!(
            structure.summary(),
            "2 texture maps · 1 mesh (LODs 0-1) · 1 other file"
        );

        assert_eq!(files_named(&[]).summary(), "no files");
        assert_eq!(files_named(&["rock.fbx"]).summary(), "1 mesh");
        assert_eq!(
            files_named(&["rock_basecolor.png"]).summary(),
            "1 texture map"
        );
    }

    #[test]
    fn says_when_only_one_of_several_meshes_has_levels() {
        let structure = files_named(&["rock_LOD0.fbx", "rock_LOD1.fbx", "grass.fbx"]);
        assert_eq!(structure.summary(), "2 meshes (LODs 0-1 on one of them)");

        // When every mesh has levels, the plain form says it well enough.
        let both = files_named(&["rock_LOD0.fbx", "rock_LOD1.fbx", "grass_LOD0.fbx"]);
        assert_eq!(both.summary(), "2 meshes (LODs 0-1)");
    }

    #[test]
    fn counts_pluralise_in_english() {
        assert_eq!(plural(1, "mesh"), "1 mesh");
        assert_eq!(plural(2, "mesh"), "2 meshes");
        assert_eq!(plural(3, "texture map"), "3 texture maps");
        assert_eq!(plural(1, "other file"), "1 other file");
    }

    #[test]
    fn an_empty_asset_has_no_structure() {
        let structure = files_named(&[]);
        assert!(structure.is_empty());
        assert_eq!(structure.file_count(), 0);
        assert!(structure.role_counts().is_empty());
        assert!(structure.material_names().is_empty());
        assert!(structure.lod_levels().is_empty());
        assert_eq!(structure.known_size_bytes(), 0);
    }

    #[test]
    fn extensions_are_matched_case_insensitively() {
        assert_eq!(file_kind(Path::new("Rock.FBX")), FileKind::Mesh);
        assert_eq!(file_kind(Path::new("Rock.PNG")), FileKind::Texture);
        assert_eq!(file_kind(Path::new("Rock.txt")), FileKind::Other);
        assert_eq!(file_kind(Path::new("Rock")), FileKind::Other);
    }

    #[test]
    fn an_engine_native_mesh_is_not_an_importable_mesh() {
        // `.uasset` is a mesh to Unreal and nothing to a DCC importer, so it
        // stays out of the layout's `meshes/` folder.
        assert_eq!(file_kind(Path::new("SM_Rock.uasset")), FileKind::Other);
    }

    #[test]
    fn kind_dirs_are_the_standard_layout() {
        assert_eq!(FileKind::Mesh.dir(), "meshes");
        assert_eq!(FileKind::Texture.dir(), "textures");
        assert_eq!(FileKind::Other.dir(), "other");
    }

    #[test]
    fn a_structure_survives_json_round_trip() {
        let structure = files_named(&["rock_LOD0.fbx", "rock_basecolor.png", "notes.txt"]);
        let json = serde_json::to_string(&structure).unwrap();
        let read_back: AssetStructure = serde_json::from_str(&json).unwrap();
        assert_eq!(read_back, structure);
        // Roles read as the same names `as_str` gives them.
        assert!(json.contains("\"baseColor\""));
    }
}
