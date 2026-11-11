# AssetCraft Godot editor plugin

The plugin adds an **AssetCraft exports** dock to Godot 4. It discovers the
Rust exporter's `assetcraft_export.json` manifests, shows each asset and format,
and imports a selected export into `res://assetcraft_imports/<asset>/<format>/`.
The imported scene is opened after Godot has scanned and imported the files.

The plugin only reads manifests and copies files. The Rust exporter owns the
layout and generated GDScript; see [the architecture note](../docs/architecture.md)
for the reader/exporter contract.

## Requirements

- Godot 4.4.1 or later in the Godot 4 series (the plugin uses Godot 4's editor
  and resource-filesystem APIs).
- An AssetCraft Godot export. The plugin reads the app's default export library
  path, or a custom CLI `--target` folder entered in the dock.

## Build and install

Build the archive:

```sh
python3 godot_plugin/package.py
```

This creates `godot_plugin/dist/assetcraft_godot.zip`. Extract it into the Godot
project root; the archive contains `addons/assetcraft_godot/`. Alternatively,
copy `godot_plugin/assetcraft_godot/plugin.cfg`, `plugin.gd`, `exports.gd`, and
`importer.gd` into the project's `addons/assetcraft_godot/` directory.
Then open **Project → Project Settings → Plugins** and enable **AssetCraft**.

## Use

Open the **AssetCraft** dock on the right side of the editor. Its path field
defaults to the application export library. Enter a different folder when
browsing exports made with the CLI's `--target` option, then click **Refresh**.
Each discovered export lists its asset id, format, number of classified
materials, and whether the mesh path is glTF or raw. **Import** copies the
export into the current project and opens its `.tscn` after Godot's filesystem
scan finishes. Re-importing an existing asset/format asks before replacing it.

The exporter writes a minimal `project.godot` so the export can also be opened
directly in the Godot editor without the plugin.

## Tests

The plugin reader tests use a fixture written by the Rust exporter, not a
hand-authored manifest. Run from the repository root with Godot on `PATH`:

```sh
ASSETCRAFT_GODOT_FIXTURE=/tmp/assetcraft-godot-fixture \
  cargo test -p assetcraft-dcc-godot --test plugin_fixture
mkdir -p /tmp/assetcraft-godot-project/addons
cp -r godot_plugin/assetcraft_godot /tmp/assetcraft-godot-project/addons/
cp -r godot_plugin/tests /tmp/assetcraft-godot-project/tests
printf 'config_version=5\n\n[application]\nconfig/name="AssetCraft tests"\n' \
  > /tmp/assetcraft-godot-project/project.godot
ASSETCRAFT_GODOT_FIXTURE=/tmp/assetcraft-godot-fixture \
  godot --headless --path /tmp/assetcraft-godot-project \
  --script res://tests/test_exports.gd
```

CI pins Godot 4.4.1 and runs this same fixture-to-reader contract, plus parse
checks for both plugin scripts and package tests.
