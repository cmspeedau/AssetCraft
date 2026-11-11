//! Godot integration.

use std::path::{Path, PathBuf};

use assetcraft_core::{AssetId, DccExporter, Result};
use assetcraft_dcc_blender::ExportPlan;

pub mod detect;
pub mod exporter;
pub mod material;
pub mod script;

pub use assetcraft_core::{
    classify, classify_texture, file_kind, without_lod_token, AssetStructure, Classified, FileKind,
    TextureRole,
};
pub use assetcraft_dcc_blender::{collect_files, export_dir, plan};
pub use detect::find as detect_find;
pub use exporter::{export, ExportOptions, ExportReport};
pub use material::{orm_pack, GodotMaterial};
pub use script::{
    material_script, preferred_meshes, scene, scene_with_meshes, SCENE_NAME, SCRIPT_NAME,
};

impl DccExporter for GodotAdapter {
    type Plan = ExportPlan;
    type Options = ExportOptions;
    type Report = ExportReport;

    fn name() -> &'static str {
        "godot"
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

    fn scene(plan: &Self::Plan) -> Option<String> {
        Some(scene(plan))
    }

    fn detect() -> Option<PathBuf> {
        detect::find()
    }
}

pub struct GodotAdapter;
