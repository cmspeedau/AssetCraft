//! Emits a complete export tree for the Godot plugin's GDScript tests to read.
//!
//! The plugin's tests (`godot_plugin/tests/test_exports.gd`) run inside Godot
//! and check that the manifest reader and the discovery walk understand what
//! the exporter actually writes. What they must not do is hand-write a
//! manifest and check the reader against that copy: the real exporter writes
//! this tree, so a rename on the Rust side fails the GDScript test.
//!
//! Run it with `ASSETCRAFT_GODOT_FIXTURE` pointing at the directory to write
//! under, then point Godot and the plugin tests at the same directory. Without
//! the variable set (every ordinary `cargo test`) the test does nothing, so it
//! costs no one a stray directory.

use std::path::{Path, PathBuf};

use assetcraft_core::AssetId;
use image::{ImageBuffer, Rgb};

/// The asset id of the export with every map, including the ORM sources.
const FULL_ASSET: &str = "alpha-rock";
/// The asset id of the glTF-only export, so discovery order is checkable.
const GLTF_ASSET: &str = "zulu-rock";

fn png(path: &Path, channel: u8) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    ImageBuffer::from_pixel(2, 2, Rgb([channel, channel, channel]))
        .save(path)
        .unwrap();
}

fn write_export(root: &Path, source: &Path, asset: &str, format: &str, files: &[(&str, bool, u8)]) {
    let mut paths = Vec::new();
    for (relative, is_png, channel) in files {
        let path = source.join(relative);
        if *is_png {
            png(&path, *channel);
        } else {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"mesh").unwrap();
        }
        paths.push(PathBuf::from(relative));
    }

    let plan = assetcraft_dcc_godot::plan(root, &AssetId::new(asset), format, &paths);
    assetcraft_dcc_godot::export(
        &plan,
        source,
        &assetcraft_dcc_godot::ExportOptions {
            import_meshes: true,
            ..Default::default()
        },
    )
    .unwrap();
}

#[test]
fn writes_an_export_for_the_godot_plugin_tests() {
    let Some(root) = std::env::var_os("ASSETCRAFT_GODOT_FIXTURE") else {
        return;
    };
    let root = PathBuf::from(root);

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("download");

    // The full map set: the ORM sources are here, so the exporter packs a
    // texture and the plugin has something real to list.
    write_export(
        &root,
        &source,
        FULL_ASSET,
        "fbx",
        &[
            ("Content/Rock/SM_Rock.fbx", false, 0),
            ("Content/Rock/rock_basecolor.png", true, 200),
            ("Content/Rock/rock_normal.png", true, 128),
            ("Content/Rock/rock_roughness.png", true, 64),
            ("Content/Rock/rock_metallic.png", true, 32),
            ("Content/Rock/rock_ao.png", true, 16),
            ("Content/Rock/rock_emission.png", true, 8),
        ],
    );

    // A second export, glTF, no ORM sources: discovery's order and the
    // plugin's mesh-kind label both need a second shape.
    write_export(
        &root,
        &source,
        GLTF_ASSET,
        "glb",
        &[
            ("Content/Rock/SM_Rock.glb", false, 0),
            ("Content/Rock/rock_basecolor.png", true, 210),
        ],
    );

    // Directories that look like exports but must be skipped: invalid JSON,
    // and a manifest from a newer shape than the plugin understands.
    let broken = root.join("broken-export");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("assetcraft_export.json"), b"{not json").unwrap();

    let future = root.join("future-export");
    std::fs::create_dir_all(&future).unwrap();
    std::fs::write(
        future.join("assetcraft_export.json"),
        br#"{"version": 99, "asset": "future", "format": "glb", "script": "assetcraft_export.gd", "importMeshes": true, "meshes": [], "materials": []}"#,
    )
    .unwrap();

    // An export nested inside an export: discovery must stop at the outer one
    // rather than list both.
    let nested = root.join(FULL_ASSET).join("fbx").join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(
        nested.join("assetcraft_export.json"),
        br#"{"version": 1, "asset": "nested-rock", "format": "glb"}"#,
    )
    .unwrap();

    // Unsafe paths should not become destination paths in the project.
    let unsafe_dir = root.join("unsafe-export");
    std::fs::create_dir_all(&unsafe_dir).unwrap();
    std::fs::write(
        unsafe_dir.join("assetcraft_export.json"),
        br#"{"version":1,"asset":"../escape","format":"glb","script":"assetcraft_export.gd","importMeshes":true,"meshes":["../../outside.glb"],"materials":[]}"#,
    )
    .unwrap();

    // Not an export at all.
    let stray = root.join("not-an-export");
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::write(stray.join("readme.txt"), b"nothing to import here").unwrap();

    let full = root.join(FULL_ASSET).join("fbx");
    assert!(full.join("assetcraft_export.json").is_file());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(full.join("assetcraft_export.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["script"], "assetcraft_export.gd");
    assert!(full.join("assetcraft_export.tscn").is_file());
    assert!(full.join("textures/rock_orm.png").is_file());

    println!(
        "wrote the Godot plugin fixture exports to {}",
        root.display()
    );
}
