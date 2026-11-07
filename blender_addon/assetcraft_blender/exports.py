"""Reading AssetCraft export directories, written without Blender.

The Blender addon uses this to list the exports under a library folder and to
describe each one. The Rust exporter writes two files per export: a manifest
this module reads, and a script the addon runs. The reading lives here, on pure
standard library and no `bpy`, because the manifest is the contract between the
exporter and the addon — and a contract that can only be checked inside Blender
is checked in CI by nobody.
"""

import json
import os
from dataclasses import dataclass, field
from typing import List, Optional

MANIFEST_NAME = "assetcraft_export.json"
SCRIPT_NAME = "assetcraft_export.py"
MANIFEST_VERSION = 1

# A bound on directories walked below a root, so pointing the addon at a whole
# home directory costs milliseconds and gives up rather than freezing the UI.
# Real export libraries are tens of directories, not thousands.
MAX_WALKED_DIRECTORIES = 4000


@dataclass
class TextureMap:
    """One texture map of a material: the slot it fills and the file that
    fills it, as export-relative paths."""

    role: str
    binding: str
    path: str


@dataclass
class Material:
    """One material the export's script builds."""

    name: str
    maps: List[TextureMap] = field(default_factory=list)


@dataclass
class Export:
    """One AssetCraft export: an asset laid out for a DCC, with its script."""

    dir: str
    asset: str
    format: str
    import_meshes: bool
    meshes: List[str]
    materials: List[Material]

    @property
    def script(self) -> str:
        """The generated script's path inside the export directory."""
        return os.path.join(self.dir, SCRIPT_NAME)

    @property
    def runnable(self) -> bool:
        """Whether the script the manifest names is actually there."""
        return os.path.isfile(self.script)

    @property
    def label(self) -> str:
        """A one-line name for the panel."""
        return "{} ({})".format(self.asset, self.format)


def load_manifest(export_dir: str) -> Optional[Export]:
    """The export described by `export_dir`, or None when it is not one.

    Never raises: a directory the addon cannot read, a manifest that is not
    JSON, and a manifest of a version this addon does not know all answer the
    same way — it is not an export, so it is not listed. An addon must not
    break its own panel because one folder on disk is odd.
    """
    path = os.path.join(export_dir, MANIFEST_NAME)
    try:
        with open(path, "r", encoding="utf-8") as handle:
            data = json.load(handle)
    except (OSError, ValueError):
        return None
    if not isinstance(data, dict) or data.get("version") != MANIFEST_VERSION:
        return None

    materials = []
    for entry in data.get("materials") or []:
        if not isinstance(entry, dict):
            continue
        maps = [
            TextureMap(
                str(map_.get("role") or "unknown"),
                str(map_.get("binding") or ""),
                map_["path"],
            )
            for map_ in entry.get("maps") or []
            if isinstance(map_, dict) and isinstance(map_.get("path"), str) and map_["path"]
        ]
        materials.append(Material(str(entry.get("name") or "material"), maps))

    absolute = os.path.abspath(export_dir)
    return Export(
        dir=absolute,
        # The exporter always writes an asset id; the fallbacks are for a hand
        # made manifest, and keep one from being listed as "(fbx)".
        asset=str(
            data.get("asset")
            or os.path.basename(os.path.dirname(absolute))
            or "asset"
        ),
        format=str(data.get("format") or "asset"),
        import_meshes=bool(data.get("importMeshes", True)),
        meshes=[m for m in data.get("meshes") or [] if isinstance(m, str) and m],
        materials=materials,
    )


def discover(root: str) -> List[Export]:
    """Every AssetCraft export under `root`, ordered by asset then format.

    Walks until it has looked at `MAX_WALKED_DIRECTORIES` directories and
    never descends into an export it has already found: an export inside an
    export is a mistake rather than a find, and its folders are not a place to
    keep looking.
    """
    if not root or not os.path.isdir(root):
        return []

    found = []
    walked = 0
    for dirpath, dirnames, filenames in os.walk(root):
        walked += 1
        if walked > MAX_WALKED_DIRECTORIES:
            break
        if MANIFEST_NAME in filenames:
            export = load_manifest(dirpath)
            if export is not None:
                found.append(export)
            dirnames[:] = []
    found.sort(key=lambda export: (export.asset, export.format, export.dir))
    return found
