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

use assetcraft_core::AssetId;

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
    fn export_dir_uses_asset_then_format() {
        let dir = export_dir("/lib", &AssetId::new("abc-123"), "fbx");
        assert_eq!(dir, PathBuf::from("/lib/abc-123/fbx"));
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
