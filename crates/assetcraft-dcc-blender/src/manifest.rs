//! What an export directory is, in machine-readable form.
//!
//! The generated script is the *how*: it builds the materials and imports the
//! meshes. The manifest is the *what*: which asset and format this export is,
//! which meshes it imports, and which maps make up which material. The Blender
//! addon (M4) reads the manifest to list exports and to describe them, and runs
//! the script to do the work — so it never grows a copy of the export logic,
//! and there is one place the layout can be wrong.
//!
//! Every path is relative to the export directory and uses `/` separators on
//! every OS, the same rule that keeps the generated script byte-identical
//! across platforms. The script resolves its own directory at run time, so an
//! export that has been copied or synced elsewhere still works — the manifest's
//! paths are meaningful wherever the directory now lives.

use std::path::Path;

use serde::Serialize;

use assetcraft_core::{Error, Result};

use crate::plan::ExportPlan;

/// The manifest's file name inside an export directory.
pub const MANIFEST_NAME: &str = "assetcraft_export.json";

/// The manifest's shape version, for a reader to check before trusting it.
///
/// Bump only on a breaking change; a reader that does not recognise the
/// version is expected to skip the export rather than guess.
pub const MANIFEST_VERSION: u32 = 1;

/// One texture map of a material: the slot it fills and the file that fills it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManifestMap {
    /// The PBR role, as [`crate::TextureRole::as_str`] names it.
    pub role: String,
    /// How it wires into the material, as [`crate::material::Binding::as_str`]
    /// names it.
    pub binding: String,
    /// Its path within the export directory, `/`-separated.
    pub path: String,
}

/// One material the export's script builds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManifestMaterial {
    /// The material's name.
    pub name: String,
    /// Its maps, in the order the script wires them.
    pub maps: Vec<ManifestMap>,
}

/// An export directory, described.
///
/// Serialised with camelCase keys (`importMeshes`) because that is the
/// convention the asset world — and the manifest's other JSON consumers — use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportManifest {
    /// The shape version this was written at.
    pub version: u32,
    /// The asset id exported.
    pub asset: String,
    /// The format code the files came from.
    pub format: String,
    /// The script's file name within the export directory.
    pub script: String,
    /// Whether the script imports and assigns meshes, or only builds materials.
    pub import_meshes: bool,
    /// The meshes the script imports, as export-relative paths.
    pub meshes: Vec<String>,
    /// The materials the script builds.
    pub materials: Vec<ManifestMaterial>,
}

/// Describe `plan` the way its export directory will be.
pub fn render(plan: &ExportPlan, import_meshes: bool) -> ExportManifest {
    ExportManifest {
        version: MANIFEST_VERSION,
        asset: plan.asset.as_str().to_string(),
        format: plan.format_code.clone(),
        script: crate::exporter::SCRIPT_NAME.to_string(),
        import_meshes,
        meshes: plan.meshes.iter().map(|mesh| relative(mesh)).collect(),
        materials: plan
            .materials
            .iter()
            .map(|material| ManifestMaterial {
                name: material.name.clone(),
                maps: material
                    .maps
                    .iter()
                    .map(|map| ManifestMap {
                        role: map.role.as_str().to_string(),
                        binding: map.binding.as_str().to_string(),
                        path: relative(&map.target),
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Write the manifest into the plan's export directory, returning its path.
pub fn write(plan: &ExportPlan, import_meshes: bool) -> Result<std::path::PathBuf> {
    let manifest = render(plan, import_meshes);
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| Error::Other(format!("could not serialise the export manifest: {e}")))?;
    let path = plan.dir().join(MANIFEST_NAME);
    std::fs::write(&path, json + "\n")?;
    Ok(path)
}

/// An export-relative path as JSON text: `/`-separated whatever the OS.
fn relative(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assetcraft_core::AssetId;
    use std::path::PathBuf;

    fn plan() -> ExportPlan {
        crate::plan(
            "/lib",
            &AssetId::new("rock-1"),
            "fbx",
            &[
                PathBuf::from("Content/rock_basecolor.png"),
                PathBuf::from("Content/rock_normal.png"),
                PathBuf::from("Content/rock_ao.png"),
                PathBuf::from("Content/rock_mystery.png"),
                PathBuf::from("Content/rock.fbx"),
                PathBuf::from("Content/notes.txt"),
            ],
        )
    }

    #[test]
    fn describes_the_asset_format_and_script() {
        let manifest = render(&plan(), true);
        assert_eq!(manifest.version, MANIFEST_VERSION);
        assert_eq!(manifest.asset, "rock-1");
        assert_eq!(manifest.format, "fbx");
        assert_eq!(manifest.script, crate::exporter::SCRIPT_NAME);
        assert!(manifest.import_meshes);
        assert!(!render(&plan(), false).import_meshes);
    }

    #[test]
    fn lists_meshes_and_material_maps_as_export_relative_paths() {
        let manifest = render(&plan(), true);
        assert_eq!(manifest.meshes, vec!["meshes/rock.fbx".to_string()]);

        assert_eq!(manifest.materials.len(), 1);
        assert_eq!(manifest.materials[0].name, "rock");
        let maps: Vec<(&str, &str, &str)> = manifest.materials[0]
            .maps
            .iter()
            .map(|map| (map.role.as_str(), map.binding.as_str(), map.path.as_str()))
            .collect();
        // In wiring order, base colour first; the unrecognised map is not in
        // any material, because it is not wired.
        assert_eq!(
            maps,
            vec![
                ("baseColor", "BaseColor", "textures/rock_basecolor.png"),
                (
                    "ambientOcclusion",
                    "AmbientOcclusion",
                    "textures/rock_ao.png"
                ),
                ("normal", "Normal", "textures/rock_normal.png"),
            ]
        );
    }

    #[test]
    fn serialises_camel_case_with_forward_slashes() {
        let json = serde_json::to_string(&render(&plan(), true)).unwrap();
        assert!(json.contains("\"importMeshes\":true"));
        assert!(json.contains("\"meshes/rock.fbx\""));
        assert!(json.contains("\"role\":\"baseColor\""));

        let read: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(read["version"], serde_json::json!(MANIFEST_VERSION));
        assert_eq!(read["materials"][0]["maps"][0]["binding"], "BaseColor");
    }

    #[test]
    fn writes_beside_the_plan_s_own_directory() {
        let temp = tempfile::tempdir().unwrap();
        let plan = crate::plan(
            temp.path(),
            &AssetId::new("rock-1"),
            "fbx",
            &[PathBuf::from("rock_basecolor.png")],
        );
        std::fs::create_dir_all(plan.dir()).unwrap();
        let path = write(&plan, true).unwrap();
        assert_eq!(path, plan.dir().join(MANIFEST_NAME));
        assert!(path.is_file());
    }
}
