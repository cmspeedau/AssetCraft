"""Runs the export the Rust exporter actually wrote, under a stub `bpy`.

This is the contract test between the two halves of the Blender story: the
manifest `assetcraft-dcc-blender` writes, the script it generates, and the
Python that reads and runs them. It needs a real export, built by:

    ASSETCRAFT_ADDON_FIXTURE=/tmp/fixture cargo test -p assetcraft-dcc-blender --test addon_fixture
    ASSETCRAFT_ADDON_FIXTURE=/tmp/fixture python3 -m unittest discover -s blender_addon/tests

(That is exactly what the CI `addon` job does.) Without the fixture the tests
skip — a machine without a fixture says "not checked here", never "checked".
"""

import ast
import os
import runpy
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE / "assetcraft_blender"))

import exports  # noqa: E402
import stub_bpy  # noqa: E402

FIXTURE = os.environ.get("ASSETCRAFT_ADDON_FIXTURE")

REASON = (
    "no Rust-built export fixture; set ASSETCRAFT_ADDON_FIXTURE and run "
    "`cargo test -p assetcraft-dcc-blender --test addon_fixture`"
)


def read_constants(script_path):
    """The generated script's literal data constants, parsed with `ast` — the
    script is read as data here, never executed."""
    with open(script_path, "r", encoding="utf-8") as handle:
        tree = ast.parse(handle.read())
    constants = {}
    for node in tree.body:
        if (
            isinstance(node, ast.Assign)
            and len(node.targets) == 1
            and isinstance(node.targets[0], ast.Name)
        ):
            try:
                constants[node.targets[0].id] = ast.literal_eval(node.value)
            except ValueError:
                pass  # a computed constant, like EXPORT_DIR = _export_dir()
    return constants


@unittest.skipUnless(FIXTURE, REASON)
class GeneratedExportTest(unittest.TestCase):
    """One Rust-built export, inspected as data and then actually run."""

    @classmethod
    def setUpClass(cls):
        found = exports.discover(FIXTURE)
        if len(found) != 1:
            raise AssertionError(
                "expected exactly one export under the fixture root, found "
                + str([e.dir for e in found])
            )
        cls.export = found[0]

    def setUp(self):
        sys.modules.pop("bpy", None)
        self.bpy = stub_bpy.install()
        self.addCleanup(stub_bpy.forget)
        self.run_script(self.export.script)

    def run_script(self, script_path):
        # The script calls main() itself; runpy gives it __file__ and __main__,
        # which is also exactly how the addon runs it.
        runpy.run_path(script_path, run_name="__main__")

    # ---- the manifest says what the script does ----

    def test_the_manifest_describes_the_script_exactly(self):
        constants = read_constants(self.export.script)
        self.assertEqual(constants["MESHES"], self.export.meshes)
        self.assertEqual(constants["IMPORT_MESHES"], self.export.import_meshes)
        script_materials = {
            name: [(binding, path) for binding, path in bindings]
            for name, bindings in constants["MATERIALS"]
        }
        manifest_materials = {
            material.name: [(map_.binding, map_.path) for map_ in material.maps]
            for material in self.export.materials
        }
        self.assertEqual(script_materials, manifest_materials)

    def test_every_promised_path_exists_where_the_manifest_says(self):
        for path in self.export.meshes:
            self.assertTrue(os.path.isfile(os.path.join(self.export.dir, *path.split("/"))), path)
        for material in self.export.materials:
            for map_ in material.maps:
                self.assertTrue(
                    os.path.isfile(os.path.join(self.export.dir, *map_.path.split("/"))),
                    map_.path,
                )

    # ---- running the script does what the manifest promises ----

    def test_it_imports_every_manifested_mesh(self):
        imported = {path for _operator, path in self.bpy.state.calls}
        expected = {
            os.path.join(self.export.dir, *path.split("/")) for path in self.export.meshes
        }
        self.assertEqual(imported, expected)

    def test_it_routes_each_extension_to_an_importer(self):
        operators = {path: operator for operator, path in self.bpy.state.calls}
        by_extension = {
            os.path.splitext(path)[1]: operator for path, operator in operators.items()
        }
        self.assertEqual(by_extension.get(".fbx"), "import_scene.fbx")
        # The stub has no `bpy.ops.wm`, so the .obj import proves the script's
        # fallback from the modern operator to the legacy one actually works.
        self.assertEqual(by_extension.get(".obj"), "import_scene.obj")

    def test_it_builds_every_manifested_material(self):
        built = {material.name for material in self.bpy.data.materials}
        self.assertEqual(built, {m.name for m in self.export.materials})

    def test_it_loads_every_manifested_map(self):
        expected = {
            os.path.join(self.export.dir, *map_.path.split("/"))
            for material in self.export.materials
            for map_ in material.maps
        }
        self.assertEqual(set(self.bpy.state.images_loaded), expected)

    def test_the_principled_wiring_follows_the_bindings(self):
        material = self.bpy.data.materials.get(self.export.materials[0].name)
        links = {stub_bpy.describe_link(link) for link in material.node_tree.links.created}
        # Base colour straight in; roughness (the survivor of the roughness
        # and glossiness pair) straight in; normal through a Normal Map node;
        # alpha through the Alpha output.
        self.assertIn(
            ("ShaderNodeTexImage", "Color", "ShaderNodeBsdfPrincipled", "Base Color"), links
        )
        self.assertIn(
            ("ShaderNodeTexImage", "Color", "ShaderNodeBsdfPrincipled", "Roughness"), links
        )
        self.assertIn(
            ("ShaderNodeNormalMap", "Normal", "ShaderNodeBsdfPrincipled", "Normal"), links
        )
        self.assertIn(
            ("ShaderNodeTexImage", "Alpha", "ShaderNodeBsdfPrincipled", "Alpha"), links
        )
        built = {node.bl_idname for node in material.node_tree.built}
        self.assertIn("ShaderNodeNormalMap", built)
        # The fixture has no glossiness winner (roughness beat it), no AO
        # without base colour, and no displacement map, so only these nodes
        # should have been built at all.
        self.assertIn("ShaderNodeBsdfPrincipled", built)

    def test_it_assigns_the_material_to_the_imported_objects(self):
        material = self.bpy.data.materials.get(self.export.materials[0].name)
        objects = self.bpy.data.objects
        self.assertEqual(len(objects), len(self.export.meshes))
        for obj in objects:
            self.assertEqual(obj.type, "MESH")
            self.assertTrue(obj.data.materials)
            self.assertIs(obj.data.materials[0], material)

    # ---- and the whole export survives being moved, which is what the
    # ---- sync-between-machines promise needs ----

    def test_a_copied_export_imports_from_where_it_now_lives(self):
        holder = tempfile.TemporaryDirectory()
        self.addCleanup(holder.cleanup)
        copy = os.path.join(holder.name, "moved-export")
        shutil.copytree(self.export.dir, copy)

        sys.modules.pop("bpy", None)
        bpy = stub_bpy.install()
        self.run_script(os.path.join(copy, exports.SCRIPT_NAME))

        imported = {path for _operator, path in bpy.state.calls}
        self.assertTrue(imported, "the copied import should have run")
        for path in imported:
            self.assertTrue(path.startswith(copy), path + " resolved outside the copy")
            self.assertTrue(os.path.isfile(path), path)

    def test_a_script_run_without_file_uses_the_baked_directory(self):
        # `exec` without __file__ is the one way the resolution can fail to
        # find its own folder; the path baked in at export time must then be
        # what the script works from.
        constants = read_constants(self.export.script)
        with open(self.export.script, "r", encoding="utf-8") as handle:
            source = handle.read()
        sys.modules.pop("bpy", None)
        bpy = stub_bpy.install()
        exec(compile(source, "<generated>", "exec"), {"__name__": "__main__"})
        imported = {path for _operator, path in bpy.state.calls}
        self.assertTrue(imported)
        for path in imported:
            self.assertTrue(path.startswith(constants["BAKED_EXPORT_DIR"]), path)


if __name__ == "__main__":
    unittest.main()
