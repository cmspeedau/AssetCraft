//! The shared export contract every DCC adapter honours.
//!
//! An adapter's whole job is to turn a downloaded asset's files into a
//! standardized layout plus a script the target editor can run, and optionally
//! to hand that script to a detected editor binary. This trait is the shape of
//! that job, written so a reader can check the constraint that defines an
//! adapter by reading the signature: **no client, no async, no network types
//! anywhere in it.** Inputs are a source directory and a plan; outputs are
//! laid-out files, a generated script, and optionally a hand-off.
//!
//! The concrete plan, options, and report types are associated, because Godot
//! and Blender genuinely differ in what a run produces (a `.tscn` scene and an
//! ORM-packed texture versus a Principled BSDF graph). What they share is the
//! *shape* and the purity, which is what the GUI and CLI drive against.

use std::path::{Path, PathBuf};

use crate::{AssetId, Result};

/// A DCC adapter: turns a downloaded asset into an editor-ready export.
pub trait DccExporter {
    /// The adapter's plan type: which files go where, and which materials the
    /// textures describe. Shared in shape with every other adapter.
    type Plan;
    /// How an export is configured.
    type Options: Default;
    /// What an export produced.
    type Report;

    /// A short, stable name for the target (`"blender"`, `"godot"`), used in
    /// CLI output and the export directory's format code.
    fn name() -> &'static str;

    /// Sort a download's files into the standardized layout and work out the
    /// materials. Pure: reads names, never bytes.
    fn plan(
        root: impl AsRef<Path>,
        asset: &AssetId,
        format_code: &str,
        files: &[PathBuf],
    ) -> Self::Plan;

    /// Lay `plan` out under its export directory, reading files from
    /// `source_root`, and optionally run the target editor on the result.
    fn export(
        plan: &Self::Plan,
        source_root: &Path,
        options: &Self::Options,
    ) -> Result<Self::Report>;

    /// The generated script, as text, so it can be inspected without writing
    /// anything to disk. `import_meshes` selects the variant that brings meshes
    /// in and assigns the material, versus materials only.
    fn material_script(plan: &Self::Plan, import_meshes: bool) -> String;

    /// A scene file the target editor can open, when the adapter produces one.
    /// Adapters that only emit a script return `None`.
    fn scene(plan: &Self::Plan) -> Option<String> {
        let _ = plan;
        None
    }

    /// The first editor binary found on this machine, if any.
    fn detect() -> Option<PathBuf>;
}

/// The list of DCC targets AssetCraft knows how to export to.
pub const DCC_TARGETS: &[&str] = &["blender", "godot"];
