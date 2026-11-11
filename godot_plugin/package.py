#!/usr/bin/env python3
"""Build the installable zip of the AssetCraft Godot plugin.

The Godot project manager can extract the packaged zip; it contains an `addons/`
directory containing the plugin. This zips `assetcraft_godot/` under `addons/`
into `dist/assetcraft_godot.zip`:

    addons/
      assetcraft_godot/
        plugin.cfg
        plugin.gd

Run it from anywhere:

    python godot_plugin/package.py [output-directory]
"""

import sys
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
MODULE = "assetcraft_godot"
FILES = ("plugin.cfg", "plugin.gd", "exports.gd", "importer.gd")


def build(dest=None) -> Path:
    """Zip the plugin module under addons/ into `dest`."""
    dest = Path(dest) if dest else HERE / "dist"
    dest.mkdir(parents=True, exist_ok=True)
    target = dest / (MODULE + ".zip")

    source = HERE / MODULE
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as archive:
        for name in FILES:
            archive.write(source / name, Path("addons") / MODULE / name)
        readme = HERE / "README.md"
        archive.write(readme, Path("addons") / MODULE / "README.md")
    return target


if __name__ == "__main__":
    argument = sys.argv[1] if len(sys.argv) > 1 else None
    print(build(argument))
