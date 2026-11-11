//! Writing a Godot export to disk.

use std::path::{Path, PathBuf};

use assetcraft_core::{Error, Result};
use assetcraft_dcc_blender::manifest;

use crate::detect;
use crate::plan::ExportPlan;
use crate::script::{material_script, scene, SCENE_NAME, SCRIPT_NAME};

#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    pub import_meshes: bool,
    pub run: bool,
    pub godot: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    pub dir: PathBuf,
    pub script: PathBuf,
    pub scene: PathBuf,
    pub manifest: PathBuf,
    pub copied: usize,
    pub materials: usize,
    pub mesh_kind: String,
    pub ran: bool,
    pub godot: Option<PathBuf>,
}

pub fn export(
    plan: &ExportPlan,
    source_root: &Path,
    options: &ExportOptions,
) -> Result<ExportReport> {
    let dir = plan.dir();
    std::fs::create_dir_all(&dir)?;

    let mut copied = 0usize;
    for file in &plan.files {
        let source = source_root.join(&file.source);
        let target = dir.join(&file.target);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source, &target)?;
        copied += 1;
    }

    let mesh_kind = if plan.meshes.iter().any(|m| {
        m.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("glb") || e.eq_ignore_ascii_case("gltf"))
            .unwrap_or(false)
    }) {
        "gltf".to_owned()
    } else {
        "fallback".to_owned()
    };

    let script = dir.join(SCRIPT_NAME);
    std::fs::write(&script, material_script(plan, options.import_meshes))?;

    let scene_path = dir.join(SCENE_NAME);
    std::fs::write(&scene_path, scene(plan))?;

    let manifest_path = manifest::write(plan, options.import_meshes)?;

    // Make the export directory a tiny Godot project so res:// resolves and
    // the scene opens in the editor without an import step.
    let project_file = dir.join("project.godot");
    if !project_file.exists() {
        std::fs::write(
            &project_file,
            "; Engine configuration file.\nconfig_version=5\n\n[application]\nconfig/name=\"AssetCraft Export\"\n",
        )?;
    }

    let mut ran = false;
    let mut godot = None;
    if options.run {
        let binary = options.godot.clone().or_else(detect::find).ok_or_else(|| {
            Error::Other("Godot was not found; install it, put it on PATH, or pass its path".into())
        })?;
        let status = std::process::Command::new(&binary)
            .arg("--headless")
            .arg("--path")
            .arg(&dir)
            .arg(&scene_path)
            .status()?;
        if !status.success() {
            return Err(Error::Other(format!(
                "Godot exited with {status} while running {}",
                scene_path.display()
            )));
        }
        ran = true;
        godot = Some(binary);
    }

    Ok(ExportReport {
        dir,
        script,
        scene: scene_path,
        manifest: manifest_path,
        copied,
        materials: plan.materials.len(),
        mesh_kind,
        ran,
        godot,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use assetcraft_core::AssetId;
    use assetcraft_dcc_blender::plan;
    use std::path::PathBuf;

    #[test]
    fn export_writes_script_scene_and_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("download");
        let files = [
            "Content/Rock/SM_Rock.glb",
            "Content/Rock/rock_basecolor.png",
            "Content/Rock/rock_normal.png",
        ];
        let mut paths = Vec::new();
        for f in files {
            let p = source.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
            paths.push(PathBuf::from(f));
        }
        let plan = plan(
            temp.path().join("out"),
            &AssetId::new("rock-1"),
            "glb",
            &paths,
        );
        let report = export(
            &plan,
            &source,
            &ExportOptions {
                import_meshes: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report.script.is_file());
        assert!(report.scene.is_file());
        assert!(report.manifest.is_file());
        assert_eq!(report.mesh_kind, "gltf");
        assert_eq!(report.materials, 1);
    }

    #[test]
    fn export_mesh_kind_fallback_when_no_gltf() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("download");
        let files = [
            "Content/Rock/SM_Rock.fbx",
            "Content/Rock/rock_basecolor.png",
        ];
        let mut paths = Vec::new();
        for f in files {
            let p = source.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
            paths.push(PathBuf::from(f));
        }
        let plan = plan(
            temp.path().join("out"),
            &AssetId::new("rock-1"),
            "fbx",
            &paths,
        );
        let report = export(&plan, &source, &ExportOptions::default()).unwrap();
        assert_eq!(report.mesh_kind, "fallback");
    }
}
