"""The AssetCraft addon: one-click import of AssetCraft exports in Blender.

The addon does not know how to build a PBR material graph. It never needs to:
the AssetCraft exporter generates that recipe per export
(`assetcraft_export.py`) and writes a manifest beside it
(`assetcraft_export.json`) that says what the export is. The addon discovers
exports through the manifest and runs the script in place with `runpy`, so the
recipe lives in exactly one place and the addon cannot drift from it — which
is also what keeps the offline promise: this addon never touches the network,
never shells out, and needs nothing but the export folder on disk.

Install: build the zip with `python blender_addon/package.py`, then in Blender
choose Edit ▸ Preferences ▸ Add-ons ▸ Install…, enable "AssetCraft", and set
the *Export Library* preference to the directory `assetcraft export --target`
laid the exports out under. The panel appears in the 3D Viewport sidebar (N)
under "AssetCraft".
"""

import os
import runpy
import traceback

import bpy
from bpy.props import StringProperty
from bpy.types import AddonPreferences, Operator, Panel

from . import exports

bl_info = {
    "name": "AssetCraft",
    "author": "AssetCraft contributors",
    "version": (0, 1, 0),
    "blender": (3, 0, 0),
    "location": "View3D > Sidebar > AssetCraft",
    "description": "Import AssetCraft exports with one click",
    "category": "Import-Export",
}


class AssetCraftPreferences(AddonPreferences):
    bl_idname = __package__

    library_root: StringProperty(
        name="Export Library",
        description=(
            "The directory AssetCraft laid exports out under. Defaults to "
            "where the AssetCraft app writes its exports; set it to use "
            "another export root, such as the --target of `assetcraft export`"
        ),
        subtype="DIR_PATH",
        default=exports.default_library_root(),
    )


class ASSETCRAFT_OT_import_export(Operator):
    bl_idname = "assetcraft.import_export"
    bl_label = "Import AssetCraft Export"
    bl_description = "Run this export's AssetCraft script in the current scene"

    export_path: StringProperty(name="Export Directory")

    def execute(self, context):
        export = exports.load_manifest(self.export_path)
        if export is None:
            self.report(
                {"ERROR"}, "{} is not an AssetCraft export".format(self.export_path)
            )
            return {"CANCELLED"}
        if not export.runnable:
            self.report(
                {"ERROR"}, "{} has no {}".format(export.dir, exports.SCRIPT_NAME)
            )
            return {"CANCELLED"}
        try:
            runpy.run_path(export.script, run_name="__main__")
        except Exception:
            # The script is generated and re-runnable; show enough of a failure
            # to act on without swallowing Blender's session.
            self.report({"ERROR"}, traceback.format_exc(limit=4))
            return {"CANCELLED"}
        self.report({"INFO"}, "Imported {}".format(export.label))
        return {"FINISHED"}


class ASSETCRAFT_OT_refresh(Operator):
    bl_idname = "assetcraft.refresh"
    bl_label = "Refresh"
    bl_description = "Scan the export library again"

    def execute(self, context):
        # Discovery is a fresh walk on every redraw and nothing is cached, so
        # this operator exists only to give the panel a deliberate re-scan and
        # the redraw that a finished operator brings with it.
        return {"FINISHED"}


class ASSETCRAFT_PT_exports(Panel):
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "AssetCraft"
    bl_label = "AssetCraft"

    def draw(self, context):
        layout = self.layout
        preferences = context.preferences.addons[__package__].preferences
        # A saved-but-empty preference (an add-on enabled before this default
        # existed) still browses the app's export directory, not nothing.
        root = bpy.path.abspath(preferences.library_root) or exports.default_library_root()
        if not root or not os.path.isdir(root):
            layout.label(text="Set the export library first:", icon="INFO")
            layout.label(text="Preferences > Add-ons > AssetCraft")
            return

        found = exports.discover(root)
        if not found:
            layout.label(text="No exports under this folder", icon="ERROR")
            layout.operator("assetcraft.refresh", icon="FILE_REFRESH")
            return

        for export in found:
            box = layout.box()
            box.label(text=export.label, icon="MATERIAL")
            if export.import_meshes:
                detail = "{} meshes, {} materials".format(
                    len(export.meshes), len(export.materials)
                )
            else:
                detail = "materials only (no mesh import)"
            box.label(text=detail)
            if not export.runnable:
                box.label(text="Script missing", icon="ERROR")
                continue
            operator = box.operator(
                "assetcraft.import_export", text="Import", icon="IMPORT"
            )
            operator.export_path = export.dir

        layout.operator("assetcraft.refresh", icon="FILE_REFRESH")


classes = (
    AssetCraftPreferences,
    ASSETCRAFT_OT_import_export,
    ASSETCRAFT_OT_refresh,
    ASSETCRAFT_PT_exports,
)


def register():
    for cls in classes:
        bpy.utils.register_class(cls)


def unregister():
    for cls in reversed(classes):
        bpy.utils.unregister_class(cls)
