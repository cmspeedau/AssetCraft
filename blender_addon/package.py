#!/usr/bin/env python3
"""Build the installable zip of the AssetCraft Blender addon.

Blender installs an addon from a zip whose single top-level folder is the
addon's module. This zips exactly `assetcraft_blender/` — never the tests,
never a stray `__pycache__` — into `dist/assetcraft_blender.zip`, and prints
the result. Run it from anywhere:

    python blender_addon/package.py [output-directory]
"""

import sys
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
MODULE = "assetcraft_blender"


def build(dest=None) -> Path:
    """Zip the addon module into `dest` (default: this folder's `dist/`)."""
    dest = Path(dest) if dest else HERE / "dist"
    dest.mkdir(parents=True, exist_ok=True)
    target = dest / (MODULE + ".zip")

    source = HERE / MODULE
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(source.rglob("*")):
            if not path.is_file() or "__pycache__" in path.parts:
                continue
            archive.write(path, path.relative_to(HERE).as_posix())
    return target


if __name__ == "__main__":
    argument = sys.argv[1] if len(sys.argv) > 1 else None
    print(build(argument))
