"""The addon package itself, imported under a stubbed `bpy`.

No Blender here either — what this proves is that the addon is importable and
registerable against a Blender-shaped API, that its panel draws for a real
export library, and that its operator runs the export's script in place. The
generated scripts themselves are checked in `test_generated_script.py`.
"""

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

import stub_bpy

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))


def import_addon():
    """(module, bpy-stub), with a fresh stub installed for a fresh import.

    `register()` is called to stand in for Blender enabling the addon — the
    import itself registers nothing, exactly as it must not.
    """
    sys.modules.pop("bpy", None)
    sys.modules.pop("assetcraft_blender", None)
    sys.modules.pop("assetcraft_blender.exports", None)
    bpy = stub_bpy.install()
    import assetcraft_blender

    assetcraft_blender.register()
    return assetcraft_blender, bpy


def write_export(root, asset, script_body="main()\n"):
    """A minimal but real export directory, manifest and script and all."""
    directory = os.path.join(root, asset, "fbx")
    os.makedirs(directory, exist_ok=True)
    with open(os.path.join(directory, "assetcraft_export.json"), "w") as handle:
        json.dump(
            {
                "version": 1,
                "asset": asset,
                "format": "fbx",
                "script": "assetcraft_export.py",
                "importMeshes": False,
                "meshes": [],
                "materials": [],
            },
            handle,
        )
    with open(os.path.join(directory, "assetcraft_export.py"), "w") as handle:
        handle.write(script_body)
    return directory


class FakeOperator:
    """Enough of an operator for `execute` to run against."""

    def __init__(self, **props):
        self.export_path = ""
        self.reports = []
        self.__dict__.update(props)

    def report(self, level, message):
        self.reports.append((level, message))


class Layout:
    """A panel layout that records what was drawn, chainable like Blender's."""

    def __init__(self):
        self.records = []

    def label(self, text=None, **kwargs):
        self.records.append(("label", text))

    def operator(self, operator_id, **kwargs):
        self.records.append(("operator", operator_id))
        return self  # so `op.export_path = ...` lands on the recorder

    def box(self):
        child = Layout()
        self.records.append(("box", child))
        return child

    def texts(self):
        texts = [text for kind, text in self.records if kind == "label"]
        for kind, value in self.records:
            if kind == "box":
                texts.extend(value.texts())
        return texts

    def operator_ids(self):
        ids = [value for kind, value in self.records if kind == "operator"]
        for kind, value in self.records:
            if kind == "box":
                ids.extend(value.operator_ids())
        return ids

    def boxes(self):
        return [value for kind, value in self.records if kind == "box"]


class FakeContext:
    """`context.preferences.addons[pkg].preferences.library_root`, and little
    else the panel reads."""

    def __init__(self, library_root, package="assetcraft_blender"):
        preferences = type("Prefs", (), {"library_root": library_root})()
        addons = {package: type("Addon", (), {"preferences": preferences})()}
        self.preferences = type("Preferences", (), {"addons": addons})()


class AddonImportTest(unittest.TestCase):
    def setUp(self):
        self.addon, self.bpy = import_addon()
        self.addCleanup(stub_bpy.forget)
        self.addCleanup(self.forget_addon)

    @staticmethod
    def forget_addon():
        for name in ("assetcraft_blender", "assetcraft_blender.exports"):
            sys.modules.pop(name, None)

    def test_bl_info_promises_what_the_code_delivers(self):
        info = self.addon.bl_info
        self.assertEqual(info["name"], "AssetCraft")
        self.assertIsInstance(info["blender"], tuple)
        self.assertIn("Sidebar", info["location"])

    def test_registration_round_trips_every_class(self):
        registered = self.bpy.state.registered
        self.assertIn(self.addon.AssetCraftPreferences, registered)
        self.assertIn(self.addon.ASSETCRAFT_OT_import_export, registered)
        self.assertIn(self.addon.ASSETCRAFT_PT_exports, registered)
        self.addon.unregister()
        self.assertEqual(self.bpy.state.registered, [])


class PanelDrawTest(unittest.TestCase):
    def setUp(self):
        self.addon, self.bpy = import_addon()
        self.addCleanup(stub_bpy.forget)
        self.addCleanup(AddonImportTest.forget_addon)

    def draw(self, library_root):
        panel = self.addon.ASSETCRAFT_PT_exports()
        panel.layout = Layout()
        panel.draw(FakeContext(library_root))
        return panel.layout

    def test_without_a_library_it_points_at_the_preferences(self):
        layout = self.draw("")
        self.assertTrue(any("preferences" in text.lower() for text in layout.texts()))
        self.assertEqual(layout.operator_ids(), [])

    def test_an_empty_library_says_so_rather_than_staying_blank(self):
        with tempfile.TemporaryDirectory() as root:
            layout = self.draw(root)
        self.assertIn("No exports under this folder", layout.texts())
        self.assertIn("assetcraft.refresh", layout.operator_ids())

    def test_an_export_gets_a_box_an_import_button_and_the_path(self):
        with tempfile.TemporaryDirectory() as root:
            directory = write_export(root, "rock-1")
            layout = self.draw(root)
        self.assertIn("rock-1 (fbx)", layout.texts())
        self.assertIn("assetcraft.import_export", layout.operator_ids())
        # The import button carries the export's own directory.
        self.assertEqual(layout.boxes()[0].export_path, directory)

    def test_a_scriptless_export_offers_no_import(self):
        with tempfile.TemporaryDirectory() as root:
            directory = write_export(root, "rock-1")
            os.remove(os.path.join(directory, "assetcraft_export.py"))
            layout = self.draw(root)
        self.assertIn("Script missing", layout.texts())
        self.assertNotIn("assetcraft.import_export", layout.operator_ids())


class OperatorTest(unittest.TestCase):
    def setUp(self):
        self.addon, self.bpy = import_addon()
        self.addCleanup(stub_bpy.forget)
        self.addCleanup(AddonImportTest.forget_addon)

    def test_execute_runs_the_exports_script_in_place(self):
        # The script here is trivial; that it runs through the same code path
        # as a real generated script is proven in test_generated_script.py.
        script = "import bpy\nbpy.state.ran = getattr(bpy.state, 'ran', 0) + 1\n"
        with tempfile.TemporaryDirectory() as root:
            directory = write_export(root, "rock-1", script_body=script)
            operator = FakeOperator(export_path=directory)
            result = self.addon.ASSETCRAFT_OT_import_export.execute(operator, None)
        self.assertEqual(result, {"FINISHED"})
        self.assertEqual(self.bpy.state.ran, 1)
        self.assertEqual(operator.reports[0][0], {"INFO"})

    def test_execute_cancels_cleanly_when_the_script_is_broken(self):
        with tempfile.TemporaryDirectory() as root:
            directory = write_export(root, "rock-1", script_body="raise ValueError('nope')\n")
            operator = FakeOperator(export_path=directory)
            result = self.addon.ASSETCRAFT_OT_import_export.execute(operator, None)
        self.assertEqual(result, {"CANCELLED"})
        levels = [level for level, _ in operator.reports]
        self.assertIn({"ERROR"}, levels)
        message = operator.reports[-1][1]
        self.assertIn("ValueError", message)

    def test_execute_refuses_directories_that_are_not_exports(self):
        with tempfile.TemporaryDirectory() as root:
            operator = FakeOperator(export_path=root)
            result = self.addon.ASSETCRAFT_OT_import_export.execute(operator, None)
        self.assertEqual(result, {"CANCELLED"})
        self.assertEqual(operator.reports[0][0], {"ERROR"})


if __name__ == "__main__":
    unittest.main()
