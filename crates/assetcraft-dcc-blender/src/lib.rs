//! Blender integration.
//!
//! M4 is the exporter itself: the folder layout, the PBR material graph, a
//! Python script that rebuilds both in Blender, and a manifest that says what
//! an export is so the in-editor addon can find and run it.
//!
//! The vocabulary the layout is written in — which PBR slot a texture name
//! fills, which level of detail a mesh belongs to, what kind of file anything
//! is — lives in [`assetcraft_core::structure`], because the asset browser
//! describes an asset with the same reading and the next adapter reuses it
//! rather than re-deriving it. This crate re-exports that vocabulary, so
//! [`classify`] and [`TextureRole`] keep working from here. What only Blender
//! needs is [`Binding`]: the role-to-socket wiring of a Principled BSDF.

use std::path::{Path, PathBuf};

use assetcraft_core::{AssetId, DccExporter, Result};

pub mod detect;
pub mod exporter;
pub mod manifest;
pub mod material;
pub mod plan;
pub mod script;

pub use assetcraft_core::{
    classify, classify_texture, file_kind, without_lod_token, AssetStructure, Classified, FileKind,
    TextureRole,
};
pub use exporter::{export, ExportOptions, ExportReport, SCRIPT_NAME};
pub use manifest::{ExportManifest, MANIFEST_NAME, MANIFEST_VERSION};
pub use material::{Binding, MaterialSpec, TextureMap};
pub use plan::{plan, ExportPlan, PlannedFile};
pub use script::material_script;

impl DccExporter for BlenderAdapter {
    type Plan = ExportPlan;
    type Options = ExportOptions;
    type Report = ExportReport;

    fn name() -> &'static str {
        "blender"
    }

    fn plan(
        root: impl AsRef<Path>,
        asset: &AssetId,
        format_code: &str,
        files: &[PathBuf],
    ) -> Self::Plan {
        plan(root, asset, format_code, files)
    }

    fn export(
        plan: &Self::Plan,
        source_root: &Path,
        options: &Self::Options,
    ) -> Result<Self::Report> {
        export(plan, source_root, options)
    }

    fn material_script(plan: &Self::Plan, import_meshes: bool) -> String {
        material_script(plan, import_meshes)
    }

    fn scene(_plan: &Self::Plan) -> Option<String> {
        None
    }

    fn detect() -> Option<PathBuf> {
        detect::find()
    }
}

/// Zero-sized marker type implementing [`DccExporter`] for Blender, so the
/// adapter can be named as a type (e.g. `BlenderAdapter: DccExporter`) without
/// carrying any state. All the work lives in the associated free functions.
pub struct BlenderAdapter;

/// The directory an asset's files export into: `<root>/<asset id>/<format>`.
///
/// Keeping this in one place is what makes the on-disk layout predictable for
/// every DCC, rather than each integration inventing its own.
pub fn export_dir(root: impl AsRef<Path>, asset: &AssetId, format_code: &str) -> PathBuf {
    root.as_ref().join(asset.as_str()).join(format_code)
}

/// Every file under `root`, as paths relative to it, sorted for a stable plan.
///
/// [`plan`] takes its file list from outside the crate — a download directory,
/// a directory someone names — and both the CLI and the app build that list the
/// same way, so an export from the app plans exactly like one from a terminal.
pub fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
            } else if let Ok(relative) = path.strip_prefix(root) {
                files.push(relative.to_path_buf());
            }
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_dir_uses_asset_then_format() {
        let dir = export_dir("/lib", &AssetId::new("abc-123"), "fbx");
        assert_eq!(dir, PathBuf::from("/lib/abc-123/fbx"));
    }

    #[test]
    fn collect_files_lists_everything_relative_and_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let touch = |relative: &str| {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        };
        touch("textures/T_Rock_BaseColor.png");
        touch("textures/T_Rock_Roughness.png");
        touch("meshes/SM_Rock.fbx");
        touch("License.txt");

        let files = collect_files(root).unwrap();
        assert_eq!(
            files,
            [
                PathBuf::from("License.txt"),
                PathBuf::from("meshes/SM_Rock.fbx"),
                PathBuf::from("textures/T_Rock_BaseColor.png"),
                PathBuf::from("textures/T_Rock_Roughness.png"),
            ]
        );
    }

    #[test]
    fn collect_files_of_an_empty_directory_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(collect_files(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn the_shared_vocabulary_is_reachable_from_here() {
        // The M4 surface is unchanged by the classification moving to core: an
        // adapter still reads a texture name, gets its slot, and the slot still
        // says how it wires into Blender's graph.
        let classified = classify("rock_basecolor.png");
        assert_eq!(classified.role, TextureRole::BaseColor);
        assert_eq!(classified.material.as_deref(), Some("rock"));
        assert_eq!(classify_texture("rock_normal.png"), TextureRole::Normal);
        assert_eq!(Binding::for_role(classified.role), Some(Binding::BaseColor));
        assert_eq!(file_kind(Path::new("rock.fbx")), FileKind::Mesh);
        assert_eq!(without_lod_token("rock_LOD1.fbx"), "rock.fbx");
        assert!(AssetStructure::from_paths(["rock_LOD0.fbx"]).has_lods());
    }
}
