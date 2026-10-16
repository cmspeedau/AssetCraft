//! Writing an export to disk: the standardized layout, the Blender script, and
//! optionally the Blender run itself.
//!
//! The layout is written first and the script second, so a machine without
//! Blender still gets a complete, inspectable export: the files in their
//! standard places and the script to run by hand. Only `--run` needs Blender,
//! and it is the one step this crate cannot prove in tests.

use std::path::{Path, PathBuf};

use assetcraft_core::{Error, Result};

use crate::detect;
use crate::plan::ExportPlan;
use crate::script::material_script;

/// The generated script's file name inside the export directory.
pub const SCRIPT_NAME: &str = "assetcraft_export.py";

/// How to export.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Import the meshes into the scene and assign the material, rather than
    /// only building the materials.
    pub import_meshes: bool,
    /// Hand the script to Blender after writing it.
    pub run: bool,
    /// The Blender binary to run, or `None` to detect one.
    pub blender: Option<PathBuf>,
}

/// What an export did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    /// The directory the asset was laid out in.
    pub dir: PathBuf,
    /// The generated script.
    pub script: PathBuf,
    /// How many files were copied.
    pub copied: usize,
    /// How many materials the script builds.
    pub materials: usize,
    /// Whether Blender was run.
    pub ran: bool,
    /// The Blender binary used, when one was run.
    pub blender: Option<PathBuf>,
}

/// Lay `plan` out under its export directory, reading the files from
/// `source_root`, and optionally run Blender on the generated script.
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

    let script = dir.join(SCRIPT_NAME);
    std::fs::write(&script, material_script(plan, options.import_meshes))?;

    let mut ran = false;
    let mut blender = None;
    if options.run {
        let binary = options
            .blender
            .clone()
            .or_else(detect::find)
            .ok_or_else(|| {
                Error::Other(
                    "Blender was not found; install it, put it on PATH, or pass its path".into(),
                )
            })?;
        let status = std::process::Command::new(&binary)
            .arg("--background")
            .arg("--python")
            .arg(&script)
            .status()?;
        if !status.success() {
            return Err(Error::Other(format!(
                "Blender exited with {status} while running {}",
                script.display()
            )));
        }
        ran = true;
        blender = Some(binary);
    }

    Ok(ExportReport {
        dir,
        script,
        copied,
        materials: plan.materials.len(),
        ran,
        blender,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use assetcraft_core::AssetId;

    /// Write a small asset into `source` and return its relative paths.
    fn source_tree(source: &Path) -> Vec<PathBuf> {
        let files = [
            ("Content/Rock/rock_basecolor.png", b"base".as_slice()),
            ("Content/Rock/rock_normal.png", b"normal".as_slice()),
            ("Content/Rock/rock.fbx", b"mesh".as_slice()),
        ];
        let mut paths = Vec::new();
        for (relative, bytes) in files {
            let path = source.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            paths.push(PathBuf::from(relative));
        }
        paths
    }

    #[test]
    fn copies_the_files_into_the_standard_layout() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("download");
        let files = source_tree(&source);
        let target = temp.path().join("project");

        let plan = crate::plan(&target, &AssetId::new("rock-1"), "fbx", &files);
        let report = export(&plan, &source, &ExportOptions::default()).unwrap();

        assert_eq!(report.copied, 3);
        assert!(!report.ran);
        let dir = target.join("rock-1/fbx");
        assert_eq!(
            std::fs::read(dir.join("textures/rock_basecolor.png")).unwrap(),
            b"base"
        );
        assert_eq!(
            std::fs::read(dir.join("textures/rock_normal.png")).unwrap(),
            b"normal"
        );
        assert_eq!(std::fs::read(dir.join("meshes/rock.fbx")).unwrap(), b"mesh");
        assert!(report.script.exists());
        assert_eq!(report.materials, 1);
    }

    #[test]
    fn a_missing_source_file_is_an_error() {
        let temp = tempfile::tempdir().unwrap();
        let plan = crate::plan(
            temp.path(),
            &AssetId::new("rock-1"),
            "fbx",
            &[PathBuf::from("nope/rock_basecolor.png")],
        );
        let err = export(
            &plan,
            &temp.path().join("download"),
            &ExportOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Io(_)), "got {err:?}");
    }

    #[test]
    fn a_missing_blender_binary_is_an_error() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("download");
        let files = source_tree(&source);
        let plan = crate::plan(temp.path(), &AssetId::new("rock-1"), "fbx", &files);

        let options = ExportOptions {
            run: true,
            blender: Some(temp.path().join("no-such-blender")),
            ..Default::default()
        };
        let err = export(&plan, &source, &options).unwrap_err();
        assert!(matches!(err, Error::Io(_)), "got {err:?}");
    }
}
