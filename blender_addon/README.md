# The AssetCraft Blender addon

One-click import of AssetCraft exports from inside Blender.

The addon deliberately contains no export logic. Each AssetCraft export already
carries everything needed to import it — a generated `assetcraft_export.py`
that builds the PBR materials and imports the meshes, and an
`assetcraft_export.json` manifest that says what the export is. The addon
discovers exports through the manifest and runs the script in place. One
recipe, written by one crate, and the addon cannot drift from it.

## Install

```bash
python3 blender_addon/package.py        # writes dist/assetcraft_blender.zip
```

In Blender: **Edit ▸ Preferences ▸ Add-ons ▸ Install…**, pick the zip, and
enable *AssetCraft*. (On Blender 4.2+ you can instead copy the
`assetcraft_blender/` folder into Blender's add-ons directory and enable it.)

By default the addon browses the folder the AssetCraft app writes its exports
to, so an export made with the app's **Export to Blender** button appears with
nothing else to configure. To browse a different export root — the `--target`
of a CLI export, say — set the addon's **Export Library** preference to it:

```bash
assetcraft export <asset-id> --target /path/to/exports
```

## Use

Open the 3D Viewport sidebar (**N**) and the **AssetCraft** tab. Each export
under the library folder is listed with its mesh and material counts; **Import**
runs that export's script in the current scene. No sign-in, no network, no
terminal — the export folder on disk is the whole requirement.

Notes:

- Imports do not register undo steps; undo is the whole import as one action
  only if you wrap it yourself. (Use `File ▸ Revert` if an import goes wrong.)
- An export folder can be copied or synced to another machine and still
  imports: the generated script resolves paths against its own location, with
  the exported path only as a fallback.
- **Refresh** rescans the library; the list otherwise updates whenever the
  panel redraws.

## Layout

```
blender_addon/
├── assetcraft_blender/     # the addon: __init__.py (UI) + exports.py (reading)
│   ├── __init__.py         # preferences, sidebar panel, import operator
│   └── exports.py          # manifest reading and discovery — no bpy at all
├── tests/                  # stdlib unittest; a stub bpy stands in for Blender
│   ├── stub_bpy.py         # enough bpy to run generated scripts and register the addon
│   ├── test_exports.py     # the manifest contract, hand-written on the Python side
│   ├── test_addon.py       # the addon registers, draws, and runs scripts
│   ├── test_generated_script.py  # runs the *real* export Rust wrote (needs fixture)
│   └── test_package.py     # the zip installs the way Blender wants it
└── package.py              # builds dist/assetcraft_blender.zip
```

## Tests

```bash
python3 -m unittest discover -s blender_addon/tests
```

The pure tests need nothing but Python. The contract tests
(`test_generated_script.py`) run the export the Rust exporter actually wrote —
built by `cargo test -p assetcraft-dcc-blender --test addon_fixture` with
`ASSETCRAFT_ADDON_FIXTURE` set, exactly as the CI `addon` job does — and skip
without a fixture rather than pretend. There is no Blender binary in CI, so a
live "does it look right in the viewport" check stays manual.
