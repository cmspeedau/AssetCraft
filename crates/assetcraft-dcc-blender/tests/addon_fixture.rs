//! Emits one complete export for the Blender addon's Python tests to read.
//!
//! The addon's tests (under `blender_addon/tests`) run the generated script
//! against a stub `bpy` and check it against the manifest. What they must not
//! do is parse Rust's output by hand — so this test writes the real thing: run
//! it with `ASSETCRAFT_ADDON_FIXTURE` pointing at the directory to write under,
//! and point the Python tests at the same directory. Without the variable set
//! (every ordinary `cargo test`) the test does nothing, so it costs no one a
//! stray directory.

use std::path::PathBuf;

use assetcraft_core::AssetId;

/// The asset id the fixture exports, for the Python tests to look for.
const FIXTURE_ASSET: &str = "addon-fixture-rock";

#[test]
fn writes_an_export_for_the_addon_tests() {
    let Some(root) = std::env::var_os("ASSETCRAFT_ADDON_FIXTURE") else {
        return;
    };
    let root = PathBuf::from(root);

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("download");
    // A believable little asset: two meshes (the addon's stub has to route
    // `.fbx` and `.obj` through different importers), the full rough/gloss
    // pair the planner dedupes into one roughness channel, an AO map that
    // must precede nothing and follow base colour, an unrecognised map, and
    // a non-file the layout parks under `other/`.
    let files = [
        ("Content/Rock/SM_Rock.fbx", b"fbx".as_slice()),
        ("Content/Rock/SM_Rock.obj", b"obj".as_slice()),
        ("Content/Rock/rock_basecolor.png", b"base".as_slice()),
        ("Content/Rock/rock_normal.png", b"normal".as_slice()),
        ("Content/Rock/rock_roughness.png", b"rough".as_slice()),
        ("Content/Rock/rock_glossiness.png", b"gloss".as_slice()),
        ("Content/Rock/rock_ao.png", b"ao".as_slice()),
        ("Content/Rock/rock_opacity.png", b"alpha".as_slice()),
        ("Content/Rock/rock_mystery.png", b"huh".as_slice()),
        ("Content/Rock/credits.txt", b"credits".as_slice()),
    ];
    let mut paths = Vec::new();
    for (relative, bytes) in files {
        let path = source.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        paths.push(PathBuf::from(relative));
    }

    let plan = assetcraft_dcc_blender::plan(&root, &AssetId::new(FIXTURE_ASSET), "fbx", &paths);
    let report = assetcraft_dcc_blender::export(
        &plan,
        &source,
        &assetcraft_dcc_blender::ExportOptions {
            import_meshes: true,
            ..Default::default()
        },
    )
    .unwrap();

    // Something that is not an export, beside it, so the addon's discovery is
    // proven to look for the manifest rather than for any folder.
    let stray = root.join("not-an-export");
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::write(stray.join("readme.txt"), b"nothing to import here").unwrap();

    assert!(report.script.is_file());
    assert!(report.manifest.is_file());
    assert_eq!(
        report.manifest.file_name().unwrap(),
        assetcraft_dcc_blender::MANIFEST_NAME
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report.manifest).unwrap()).unwrap();
    assert_eq!(manifest["asset"], FIXTURE_ASSET);

    println!(
        "wrote the addon fixture export to {}",
        report.manifest.parent().unwrap().display()
    );
    // Wherever the export went, it is under the requested root.
    assert!(report.dir.starts_with(&root));
    assert!(report.dir.join("textures/rock_basecolor.png").is_file());
}
