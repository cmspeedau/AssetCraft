"""Tests for the addon's manifest reading and export discovery.

The manifests here are hand-written rather than Rust's output on purpose:
this file is the Python side's idea of the contract, so a Rust change that
silently renames a key fails the fixture test rather than quietly matching a
test that was changed to match it. The same JSON shape is checked against the
real exporter in `test_generated_script.py`.
"""

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

# Import `exports` the way a test of a bpy-free module must: directly, with no
# Blender and no package import (importing the package would need `bpy`).
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "assetcraft_blender"))

import exports  # noqa: E402


def manifest_text(asset="a-1", fmt="fbx", version=None, import_meshes=True):
    return json.dumps(
        {
            "version": exports.MANIFEST_VERSION if version is None else version,
            "asset": asset,
            "format": fmt,
            "script": "assetcraft_export.py",
            "importMeshes": import_meshes,
            "meshes": ["meshes/rock.fbx"],
            "materials": [
                {
                    "name": "rock",
                    "maps": [
                        {
                            "role": "baseColor",
                            "binding": "BaseColor",
                            "path": "textures/rock_basecolor.png",
                        }
                    ],
                }
            ],
        }
    )


class Library:
    """A temporary export library tree for a test."""

    def __init__(self, case):
        directory = tempfile.TemporaryDirectory()
        case.addCleanup(directory.cleanup)
        self.root = directory.name

    def export(self, asset, fmt="fbx", manifest=None, script=True, version=None):
        directory = os.path.join(self.root, asset, fmt)
        os.makedirs(directory, exist_ok=True)
        body = (
            manifest_text(asset=asset, fmt=fmt, version=version)
            if manifest is None
            else manifest
        )
        with open(os.path.join(directory, exports.MANIFEST_NAME), "w") as handle:
            handle.write(body)
        if script:
            with open(os.path.join(directory, exports.SCRIPT_NAME), "w") as handle:
                handle.write("main()\n")
        return directory


class LoadManifestTest(unittest.TestCase):
    def setUp(self):
        # This module must work with no Blender at all; make a leaked stub
        # impossible, whatever test order discovery chose.
        sys.modules.pop("bpy", None)

    def test_reads_a_manifest_the_exporter_writes(self):
        library = Library(self)
        export = exports.load_manifest(library.export("a-1"))
        self.assertIsNotNone(export)
        self.assertEqual(export.asset, "a-1")
        self.assertEqual(export.format, "fbx")
        self.assertTrue(export.import_meshes)
        self.assertEqual(export.meshes, ["meshes/rock.fbx"])
        self.assertEqual(export.label, "a-1 (fbx)")
        material = export.materials[0]
        self.assertEqual(material.name, "rock")
        self.assertEqual(material.maps[0].role, "baseColor")
        self.assertEqual(material.maps[0].binding, "BaseColor")
        self.assertEqual(material.maps[0].path, "textures/rock_basecolor.png")

    def test_a_directory_without_a_manifest_is_not_an_export(self):
        with tempfile.TemporaryDirectory() as root:
            self.assertIsNone(exports.load_manifest(root))

    def test_unreadable_manifests_answer_none_not_exception(self):
        library = Library(self)
        for broken in ("not json at all", "[]", '{"version": 99}', "{}"):
            directory = library.export("a-1", manifest=broken)
            self.assertIsNone(exports.load_manifest(directory), broken)

    def test_a_future_version_is_skipped_not_guessed(self):
        library = Library(self)
        directory = library.export("a-1", version=exports.MANIFEST_VERSION + 1)
        self.assertIsNone(exports.load_manifest(directory))

    def test_partial_manifests_are_tolerated(self):
        library = Library(self)
        directory = library.export("a-1", manifest=json.dumps({"version": 1}))
        export = exports.load_manifest(directory)
        self.assertIsNotNone(export)
        self.assertEqual(export.meshes, [])
        self.assertEqual(export.materials, [])
        # A manifest with no asset falls back to the directory that holds it.
        self.assertEqual(export.asset, "a-1")
        # importMeshes defaults to true, matching the exporter's default.
        self.assertTrue(export.import_meshes)

    def test_rubbish_inside_a_manifest_is_dropped_not_fatal(self):
        library = Library(self)
        directory = library.export(
            "a-1",
            manifest=json.dumps(
                {
                    "version": 1,
                    "asset": "a-1",
                    "meshes": ["meshes/rock.fbx", 42, ""],
                    "materials": [
                        {"name": "rock", "maps": [{"path": "textures/a.png"}, 7, {"no": 1}]},
                        "not a material",
                    ],
                }
            ),
        )
        export = exports.load_manifest(directory)
        self.assertEqual(export.meshes, ["meshes/rock.fbx"])
        self.assertEqual(len(export.materials), 1)
        self.assertEqual([map_.path for map_ in export.materials[0].maps], ["textures/a.png"])

    def test_runnable_means_the_named_script_is_there(self):
        library = Library(self)
        export = exports.load_manifest(library.export("a-1"))
        self.assertTrue(export.runnable)

        missing = exports.load_manifest(library.export("a-2", script=False))
        self.assertFalse(missing.runnable)
        self.assertEqual(
            missing.script, os.path.join(missing.dir, exports.SCRIPT_NAME)
        )


class DiscoverTest(unittest.TestCase):
    def test_finds_nested_exports_and_orders_them(self):
        library = Library(self)
        library.export("b-asset")
        library.export("a-asset", "gltf")
        library.export("a-asset", "fbx")
        found = exports.discover(library.root)
        self.assertEqual(
            [(e.asset, e.format) for e in found],
            [("a-asset", "fbx"), ("a-asset", "gltf"), ("b-asset", "fbx")],
        )

    def test_ignores_directories_that_are_not_exports(self):
        library = Library(self)
        library.export("a-1")
        stray = os.path.join(library.root, "not-an-export")
        os.makedirs(stray)
        with open(os.path.join(stray, "readme.txt"), "w") as handle:
            handle.write("nothing here")
        self.assertEqual([e.asset for e in exports.discover(library.root)], ["a-1"])

    def test_a_broken_manifest_is_skipped_not_fatal(self):
        library = Library(self)
        library.export("good")
        library.export("broken", manifest="{{ not json")
        self.assertEqual([e.asset for e in exports.discover(library.root)], ["good"])

    def test_never_descends_into_a_found_export(self):
        library = Library(self)
        directory = library.export("a-1")
        # A manifest hidden inside an export's own folders is not a second
        # export to list, even a valid-looking one.
        hidden = os.path.join(directory, "textures", "nested")
        os.makedirs(hidden)
        with open(os.path.join(hidden, exports.MANIFEST_NAME), "w") as handle:
            handle.write(manifest_text(asset="nested"))
        found = exports.discover(library.root)
        self.assertEqual([e.asset for e in found], ["a-1"])

    def test_missing_or_empty_root_finds_nothing(self):
        self.assertEqual(exports.discover(""), [])
        with tempfile.TemporaryDirectory() as root:
            self.assertEqual(exports.discover(os.path.join(root, "nope")), [])

    def test_walk_stops_at_the_bound(self):
        library = Library(self)
        library.export("a-1")
        original = exports.MAX_WALKED_DIRECTORIES
        exports.MAX_WALKED_DIRECTORIES = 1  # only the root itself gets looked at
        try:
            self.assertEqual(exports.discover(library.root), [])
        finally:
            exports.MAX_WALKED_DIRECTORIES = original
        # With the real bound the same tree is found; the walk gave up, it
        # did not miss it for another reason.
        self.assertEqual([e.asset for e in exports.discover(library.root)], ["a-1"])


if __name__ == "__main__":
    unittest.main()
