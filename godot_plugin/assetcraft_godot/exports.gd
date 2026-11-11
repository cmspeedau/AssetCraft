## Reading AssetCraft export directories from inside Godot.
##
## The Godot plugin, like the Blender addon, is a *reader*: the Rust exporter
## owns the layout and writes a manifest describing it, and the plugin only
## lists those manifests and copies a chosen export into the project. That
## keeps the export logic in one place and makes the manifest the contract
## between the two sides.
##
## Everything here is pure GDScript over `FileAccess`/`DirAccess`, with no
## editor types, so it can be exercised headlessly by `tests/test_exports.gd`
## without an editor session.

@tool
extends RefCounted

## The manifest's file name inside an export directory.
const MANIFEST_NAME := "assetcraft_export.json"
## The scene the Godot exporter writes beside the script.
const SCENE_NAME := "assetcraft_export.tscn"
## The script the Godot exporter writes.
const SCRIPT_NAME := "assetcraft_export.gd"
## The manifest shape this reader understands. A reader that meets another
## version skips the export rather than guessing what its keys mean.
const MANIFEST_VERSION := 1

## A bound on directories walked below a root, so pointing the dock at a whole
## home directory costs milliseconds and gives up rather than freezing the
## editor. Real export libraries are tens of directories, not thousands.
const MAX_WALKED_DIRECTORIES := 4000


## Whether this Godot is running inside a Flatpak sandbox.
##
## Every Flatpak sandbox has a `/.flatpak-info` file at its root; it is what the
## export root rule below asks before trusting `XDG_DATA_HOME`.
static func inside_flatpak() -> bool:
	return FileAccess.file_exists("/.flatpak-info")


## The AssetCraft app's export directory on this machine, or "".
##
## The app lays exports out under its platform data directory, resolved by
## Rust's `directories` crate as `ProjectDirs::from("dev", "AssetCraft",
## "AssetCraft")`. These rules mirror that crate's per-platform behaviour,
## exactly as the Blender addon's `default_library_root()` does: Linux
## lower-cases the application name, macOS uses the dotted bundle identifier,
## and Windows nests organisation and application under a `data` segment.
static func default_library_root() -> String:
	var os_name := OS.get_name()
	if os_name == "Windows":
		var appdata := OS.get_environment("APPDATA")
		if appdata == "":
			return ""
		return appdata.path_join("AssetCraft").path_join("AssetCraft").path_join("data").path_join("exports")
	if os_name == "macOS":
		var home := OS.get_environment("HOME")
		if home == "":
			return ""
		return home.path_join("Library").path_join("Application Support").path_join("dev.AssetCraft.AssetCraft").path_join("exports")

	var xdg := OS.get_environment("XDG_DATA_HOME")
	if inside_flatpak():
		# A Flatpak sandbox (Godot's on Flathub is the one that matters here)
		# redefines XDG_DATA_HOME to a folder inside the sandbox, while the app
		# on the host writes where the host environment points. The sandbox
		# sees the host filesystem, so ignoring the sandboxed variable finds
		# the real folder. A relocated data directory is named by the dock's
		# path field instead.
		xdg = ""
	if xdg == "":
		var home := OS.get_environment("HOME")
		if home == "":
			return ""
		xdg = home.path_join(".local").path_join("share")
	return xdg.path_join("assetcraft").path_join("exports")


## The export described by `export_dir`, or an empty dictionary when it is not
## one. Never raises at the caller: an unreadable directory, invalid JSON, and
## an unknown version all answer the same way, so one odd folder cannot break
## the dock.
static func load_manifest(export_dir: String) -> Dictionary:
	var path := export_dir.path_join(MANIFEST_NAME)
	if not FileAccess.file_exists(path):
		return {}

	var handle := FileAccess.open(path, FileAccess.READ)
	if handle == null:
		return {}
	var text := handle.get_as_text()
	handle.close()

	var json := JSON.new()
	if json.parse(text) != OK:
		return {}
	var data = json.data
	if typeof(data) != TYPE_DICTIONARY:
		return {}
	var version = data.get("version")
	if typeof(version) != TYPE_INT and typeof(version) != TYPE_FLOAT:
		return {}
	if int(version) != MANIFEST_VERSION or float(version) != MANIFEST_VERSION:
		return {}
	if typeof(data.get("asset")) != TYPE_STRING or typeof(data.get("format")) != TYPE_STRING:
		return {}
	if typeof(data.get("script")) != TYPE_STRING or data["script"] != SCRIPT_NAME:
		return {}
	if typeof(data.get("importMeshes")) != TYPE_BOOL:
		return {}
	if typeof(data.get("meshes")) != TYPE_ARRAY or typeof(data.get("materials")) != TYPE_ARRAY:
		return {}

	var asset := str(data.get("asset", "")).strip_edges()
	if asset == "":
		# The export directory is `<root>/<asset>/<format>`, so the manifest's
		# own folder name is the asset id when the field is hand-made.
		asset = export_dir.get_base_dir().get_file()
	if asset == "":
		asset = "asset"
	var format := str(data.get("format", "asset")).strip_edges()
	if not _is_safe_segment(asset) or not _is_safe_segment(format):
		return {}

	var meshes: Array = []
	var raw_meshes = data.get("meshes", [])
	if typeof(raw_meshes) == TYPE_ARRAY:
		for mesh in raw_meshes:
			if typeof(mesh) == TYPE_STRING and mesh != "" and _is_safe_relative_path(mesh):
				meshes.append(mesh)

	var materials: Array = []
	var raw_materials = data.get("materials", [])
	if typeof(raw_materials) == TYPE_ARRAY:
		for entry in raw_materials:
			if typeof(entry) != TYPE_DICTIONARY:
				continue
			var maps: Array = []
			var raw_maps = entry.get("maps", [])
			if typeof(raw_maps) == TYPE_ARRAY:
				for map in raw_maps:
					if typeof(map) == TYPE_DICTIONARY:
						var map_path := str(map.get("path", ""))
						if _is_safe_relative_path(map_path):
							maps.append(map_path)
			materials.append({"name": str(entry.get("name", "material")), "maps": maps})

	return {
		"dir": export_dir,
		"asset": asset,
		"format": format,
		"import_meshes": bool(data.get("importMeshes", true)),
		"meshes": meshes,
		"materials": materials,
	}


## Every AssetCraft export under `root`, ordered by asset then format.
##
## Walks until it has looked at `MAX_WALKED_DIRECTORIES` directories and never
## descends into an export it has already found: an export inside an export is
## a mistake rather than a find, and its folders are not a place to keep
## looking.
static func discover(root: String) -> Array:
	var found: Array = []
	if root == "":
		return found

	var root_dir := DirAccess.open(root)
	if root_dir == null:
		return found
	var absolute_root := root_dir.get_current_dir()

	var pending: Array = [""]
	var walked := 0
	while not pending.is_empty():
		var relative: String = pending.pop_back()
		walked += 1
		if walked > MAX_WALKED_DIRECTORIES:
			break

		var here := absolute_root if relative == "" else absolute_root.path_join(relative)
		var manifest := load_manifest(here)
		if not manifest.is_empty():
			found.append(manifest)
			continue

		var dir := DirAccess.open(here)
		if dir == null:
			continue
		dir.list_dir_begin()
		var name := dir.get_next()
		while name != "":
			if dir.current_is_dir() and not name.begins_with("."):
				pending.push_back(name if relative == "" else relative.path_join(name))
			name = dir.get_next()
		dir.list_dir_end()

	found.sort_custom(_sorts_before)
	return found


## Manifest path segments are used to construct a path inside the Godot
## project. Reject separators and dot segments rather than letting a malformed
## manifest escape the import root.
static func _is_safe_segment(value: String) -> bool:
	return (
		value != ""
		and value != "."
		and value != ".."
		and not value.contains("/")
		and not value.contains("\\")
		and not value.contains(":")
	)


static func _is_safe_relative_path(value: String) -> bool:
	if value == "" or value.begins_with("/") or value.contains("\\") or value.contains(":"):
		return false
	for part in value.split("/", false):
		if part == "" or part == "." or part == "..":
			return false
	return true


static func _sorts_before(a: Dictionary, b: Dictionary) -> bool:
	var asset_a := str(a["asset"])
	var asset_b := str(b["asset"])
	if asset_a != asset_b:
		return asset_a < asset_b
	var format_a := str(a["format"])
	var format_b := str(b["format"])
	if format_a != format_b:
		return format_a < format_b
	return str(a["dir"]) < str(b["dir"])


## The mesh path an export took, for the dock's second line: glTF when the
## manifest lists it, the raw format otherwise.
static func mesh_kind(record: Dictionary) -> String:
	for mesh in record.get("meshes", []):
		var extension := str(mesh).get_extension().to_lower()
		if extension == "glb" or extension == "gltf":
			return "glTF"
	return "raw"
