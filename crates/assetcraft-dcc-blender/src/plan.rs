//! Turning a downloaded asset's files into a standardized PBR export layout.
//!
//! A download is a bag of files at whatever paths the seller chose. The planner
//! sorts them into a predictable tree — meshes, textures, everything else — and
//! works out which texture fills which slot of which material. It is pure: it
//! reads names, never bytes, so the layout can be tested without any files on
//! disk.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use assetcraft_core::AssetId;

use crate::material::{MaterialSpec, TextureMap};
use crate::{classify, export_dir, TextureRole};

/// Mesh formats the exporter knows how to lay out and Blender can import.
const MESH_EXTENSIONS: &[&str] = &[
    "fbx", "obj", "glb", "gltf", "usd", "usda", "usdc", "usdz", "blend", "abc", "ply", "stl", "dae",
];

/// Image formats treated as texture maps.
const TEXTURE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "tif", "tiff", "exr", "tga", "bmp", "hdr", "dds", "webp", "psd",
];

/// What a file is, for layout purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// A mesh the exporter imports into the scene.
    Mesh,
    /// An image, placed under `textures/` and offered to the material graph.
    Texture,
    /// Anything else, kept under `other/` at its original relative path.
    Other,
}

impl FileKind {
    /// The subdirectory files of this kind are placed under.
    pub const fn dir(self) -> &'static str {
        match self {
            FileKind::Mesh => "meshes",
            FileKind::Texture => "textures",
            FileKind::Other => "other",
        }
    }
}

/// One source file and where it lands in the export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// The file's path within the download.
    pub source: PathBuf,
    /// Its path within the export directory.
    pub target: PathBuf,
    /// What the file is.
    pub kind: FileKind,
    /// The PBR role a texture fills, when it was recognised.
    pub role: Option<TextureRole>,
}

/// The full layout for one asset: where every file goes, the materials the
/// textures describe, and the meshes to import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPlan {
    /// The asset being exported.
    pub asset: AssetId,
    /// The format code the files came from, used in the directory name.
    pub format_code: String,
    /// The root the export directory is created under.
    pub root: PathBuf,
    /// Every file, in the order it was planned.
    pub files: Vec<PlannedFile>,
    /// The materials the textures make up, by name.
    pub materials: Vec<MaterialSpec>,
    /// The meshes to import, as export-relative paths.
    pub meshes: Vec<PathBuf>,
}

impl ExportPlan {
    /// The directory the plan exports into.
    pub fn dir(&self) -> PathBuf {
        export_dir(&self.root, &self.asset, &self.format_code)
    }

    /// An export-relative path resolved against the export directory.
    pub fn absolute(&self, target: &Path) -> PathBuf {
        self.dir().join(target)
    }

    /// The meshes as absolute paths.
    pub fn mesh_paths(&self) -> Vec<PathBuf> {
        self.meshes.iter().map(|mesh| self.absolute(mesh)).collect()
    }
}

/// Plan the export layout for `files`, which are paths relative to the download.
///
/// Textures and meshes are flattened into `textures/` and `meshes/`, with a
/// numeric suffix added when two source files share a name; anything else keeps
/// its relative path under `other/`. Textures are grouped into materials by the
/// name in front of their role token.
pub fn plan(
    root: impl AsRef<Path>,
    asset: &AssetId,
    format_code: &str,
    files: &[PathBuf],
) -> ExportPlan {
    let mut used: HashSet<PathBuf> = HashSet::new();
    let mut planned = Vec::with_capacity(files.len());
    let mut materials: BTreeMap<String, Vec<TextureMap>> = BTreeMap::new();
    let mut meshes = Vec::new();

    for source in files {
        let kind = file_kind(source);
        let (target, role) = match kind {
            FileKind::Mesh => {
                let target = unique(
                    &mut used,
                    Path::new(FileKind::Mesh.dir()).join(base(source)),
                );
                meshes.push(target.clone());
                (target, None)
            }
            FileKind::Texture => {
                let target = unique(
                    &mut used,
                    Path::new(FileKind::Texture.dir()).join(base(source)),
                );
                let classified = classify(&base(source).to_string_lossy());
                if let Some(binding) = classified.role.binding() {
                    let name = classified
                        .material
                        .unwrap_or_else(|| asset.as_str().to_string());
                    materials.entry(name).or_default().push(TextureMap {
                        role: classified.role,
                        binding,
                        target: target.clone(),
                    });
                }
                let role = (classified.role != TextureRole::Unknown).then_some(classified.role);
                (target, role)
            }
            FileKind::Other => (Path::new(FileKind::Other.dir()).join(source), None),
        };
        planned.push(PlannedFile {
            source: source.clone(),
            target,
            kind,
            role,
        });
    }

    let materials = materials
        .into_iter()
        .map(|(name, mut maps)| {
            maps.sort_by_key(|map| (map.binding.order(), map.target.clone()));
            // Keep one map per channel: a material cannot use two roughness
            // maps, and after the sort the preferred one comes first.
            let mut seen = HashSet::new();
            maps.retain(|map| seen.insert(map.binding.channel()));
            MaterialSpec { name, maps }
        })
        .collect();

    ExportPlan {
        asset: asset.clone(),
        format_code: format_code.to_string(),
        root: root.as_ref().to_path_buf(),
        files: planned,
        materials,
        meshes,
    }
}

/// What a file is, from its extension.
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

/// The file name of a path, falling back to the whole path when it has none.
fn base(path: &Path) -> PathBuf {
    path.file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| path.to_path_buf())
}

/// A target path that no earlier file has claimed, adding `_1`, `_2`, … before
/// the extension on a collision.
fn unique(used: &mut HashSet<PathBuf>, target: PathBuf) -> PathBuf {
    if used.insert(target.clone()) {
        return target;
    }
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = target.extension().map(|e| e.to_string_lossy().into_owned());
    for n in 1u32.. {
        let name = match &extension {
            Some(ext) => format!("{stem}_{n}.{ext}"),
            None => format!("{stem}_{n}"),
        };
        let candidate = target.with_file_name(name);
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("the loop only ends by returning a fresh path")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::Binding;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn asset() -> AssetId {
        AssetId::new("rock-1")
    }

    #[test]
    fn sorts_files_into_the_standard_layout() {
        let files = paths(&[
            "Content/Rock/rock_basecolor.png",
            "Content/Rock/rock_normal.png",
            "Content/Rock/rock.fbx",
            "Content/Rock/rock.uasset",
        ]);
        let plan = plan("/lib", &asset(), "fbx", &files);

        assert_eq!(
            plan.files[0].target,
            PathBuf::from("textures/rock_basecolor.png")
        );
        assert_eq!(plan.files[0].kind, FileKind::Texture);
        assert_eq!(plan.files[2].target, PathBuf::from("meshes/rock.fbx"));
        assert_eq!(plan.files[2].kind, FileKind::Mesh);
        assert_eq!(
            plan.files[3].target,
            PathBuf::from("other/Content/Rock/rock.uasset")
        );
        assert_eq!(plan.meshes, vec![PathBuf::from("meshes/rock.fbx")]);
    }

    #[test]
    fn groups_textures_into_a_material_named_for_the_asset() {
        let files = paths(&[
            "rock_basecolor.png",
            "rock_normal.png",
            "rock_roughness.png",
        ]);
        let plan = plan("/lib", &asset(), "fbx", &files);

        assert_eq!(plan.materials.len(), 1);
        assert_eq!(plan.materials[0].name, "rock");
        let roles: Vec<_> = plan.materials[0].maps.iter().map(|m| m.role).collect();
        // Ordered base colour first, then roughness, then normal.
        assert_eq!(
            roles,
            vec![
                TextureRole::BaseColor,
                TextureRole::Roughness,
                TextureRole::Normal
            ]
        );
    }

    #[test]
    fn separates_maps_of_different_materials() {
        let files = paths(&[
            "rock_basecolor.png",
            "moss_basecolor.png",
            "moss_normal.png",
        ]);
        let plan = plan("/lib", &asset(), "fbx", &files);

        assert_eq!(plan.materials.len(), 2);
        assert_eq!(plan.materials[0].name, "moss");
        assert_eq!(plan.materials[1].name, "rock");
    }

    #[test]
    fn a_roughness_map_wins_over_a_glossiness_map() {
        let files = paths(&["rock_roughness.png", "rock_glossiness.png"]);
        let plan = plan("/lib", &asset(), "fbx", &files);

        // Both are exported, but only one drives the roughness channel.
        assert_eq!(plan.files.len(), 2);
        assert_eq!(plan.materials[0].maps.len(), 1);
        assert_eq!(plan.materials[0].maps[0].binding, Binding::Roughness);
    }

    #[test]
    fn an_unrecognised_texture_is_exported_but_not_wired() {
        let files = paths(&["rock_basecolor.png", "rock_mystery.png"]);
        let plan = plan("/lib", &asset(), "fbx", &files);

        assert_eq!(
            plan.files[1].target,
            PathBuf::from("textures/rock_mystery.png")
        );
        assert_eq!(plan.files[1].role, None);
        assert_eq!(plan.materials[0].maps.len(), 1);
    }

    #[test]
    fn colliding_texture_names_are_disambiguated() {
        let files = paths(&["a/rock_basecolor.png", "b/rock_basecolor.png"]);
        let plan = plan("/lib", &asset(), "fbx", &files);

        assert_eq!(
            plan.files[0].target,
            PathBuf::from("textures/rock_basecolor.png")
        );
        assert_eq!(
            plan.files[1].target,
            PathBuf::from("textures/rock_basecolor_1.png")
        );
    }

    #[test]
    fn a_texture_without_a_material_name_falls_back_to_the_asset_id() {
        let files = paths(&["basecolor.png", "normal.png"]);
        let plan = plan("/lib", &asset(), "fbx", &files);

        assert_eq!(plan.materials.len(), 1);
        assert_eq!(plan.materials[0].name, "rock-1");
    }

    #[test]
    fn the_export_directory_is_asset_then_format() {
        let plan = plan("/lib", &asset(), "fbx", &[]);
        assert_eq!(plan.dir(), PathBuf::from("/lib/rock-1/fbx"));
        assert_eq!(
            plan.absolute(Path::new("meshes/rock.fbx")),
            PathBuf::from("/lib/rock-1/fbx/meshes/rock.fbx")
        );
    }

    #[test]
    fn extensions_are_matched_case_insensitively() {
        assert_eq!(file_kind(Path::new("Rock.FBX")), FileKind::Mesh);
        assert_eq!(file_kind(Path::new("Rock.PNG")), FileKind::Texture);
        assert_eq!(file_kind(Path::new("Rock.txt")), FileKind::Other);
        assert_eq!(file_kind(Path::new("Rock")), FileKind::Other);
    }
}
