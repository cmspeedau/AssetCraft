extends SceneTree
## Headless tests for the plugin's manifest reader and export discovery.
##
## Run by the `godot-plugin` CI job after the Rust exporter writes a fixture
## tree (`crates/assetcraft-dcc-godot/tests/plugin_fixture.rs`):
##
##     ASSETCRAFT_GODOT_FIXTURE=<dir> \
##       godot --headless --path <project> --script res://tests/test_exports.gd
##
## The fixture is the exporter's real output, not a hand-written manifest, so a
## key renamed on the Rust side fails here rather than in someone's editor.

const Exports = preload("res://addons/assetcraft_godot/exports.gd")

const FULL_ASSET := "alpha-rock"
const GLTF_ASSET := "zulu-rock"

var _passed := 0
var _failed := 0


func _init() -> void:
	var root := OS.get_environment("ASSETCRAFT_GODOT_FIXTURE")
	if root == "":
		print("ASSETCRAFT_GODOT_FIXTURE is not set; skipping the Godot plugin tests.")
		quit(0)
		return

	_test_default_root_follows_xdg()
	_test_discovery_finds_the_real_exports(root)
	_test_discovery_skips_what_is_not_an_export(root)
	_test_manifest_reading(root)
	_test_unsafe_manifest_is_rejected(root)
	_test_export_contents(root)
	_test_generated_script_parses(root)

	print("Godot plugin tests: %d passed, %d failed." % [_passed, _failed])
	quit(1 if _failed > 0 else 0)


func _check(condition: bool, message: String) -> void:
	if condition:
		_passed += 1
	else:
		_failed += 1
		push_error("FAIL: " + message)


func _test_default_root_follows_xdg() -> void:
	if OS.get_name() == "Windows" or OS.get_name() == "macOS":
		return
	var previous := OS.get_environment("XDG_DATA_HOME")
	OS.set_environment("XDG_DATA_HOME", "/tmp/assetcraft-xdg")
	_check(
		Exports.default_library_root() == "/tmp/assetcraft-xdg/assetcraft/exports",
		"the Linux default root should follow XDG_DATA_HOME",
	)
	OS.set_environment("XDG_DATA_HOME", previous)


func _test_discovery_finds_the_real_exports(root: String) -> void:
	var found := Exports.discover(root)
	_check(found.size() == 2, "expected 2 exports, found %d" % found.size())
	if found.size() != 2:
		return

	_check(found[0]["asset"] == FULL_ASSET, "the first export is %s" % FULL_ASSET)
	_check(found[0]["format"] == "fbx", "the first export's format is fbx")
	_check(found[1]["asset"] == GLTF_ASSET, "the second export is %s" % GLTF_ASSET)
	_check(found[1]["format"] == "glb", "the second export's format is glb")
	_check(
		Exports.mesh_kind(found[0]) == "raw",
		"an fbx-only export reports the raw mesh path",
	)
	_check(
		Exports.mesh_kind(found[1]) == "glTF",
		"an export with a glb reports the glTF path",
	)


func _test_discovery_skips_what_is_not_an_export(root: String) -> void:
	var found := Exports.discover(root)
	var assets: Array = []
	for record in found:
		assets.append(record["asset"])
	_check(not assets.has("nested-rock"), "an export inside an export is not listed")
	_check(not assets.has("future"), "an unknown manifest version is not listed")
	_check(not assets.has("../escape"), "unsafe path segments are not listed")

	_check(Exports.discover("").is_empty(), "an empty root finds nothing")
	_check(
		Exports.discover(root.path_join("missing")).is_empty(),
		"a missing root finds nothing",
	)


func _test_manifest_reading(root: String) -> void:
	_check(
		Exports.load_manifest(root.path_join("broken-export")).is_empty(),
		"invalid JSON is not an export",
	)
	_check(
		Exports.load_manifest(root.path_join("future-export")).is_empty(),
		"an unknown manifest version is not an export",
	)
	_check(
		Exports.load_manifest(root.path_join("not-an-export")).is_empty(),
		"a folder without a manifest is not an export",
	)
	_check(
		not Exports.load_manifest(root.path_join(FULL_ASSET).path_join("fbx")).is_empty(),
		"the exporter's manifest is readable",
	)


func _test_unsafe_manifest_is_rejected(root: String) -> void:
	_check(
		Exports.load_manifest(root.path_join("unsafe-export")).is_empty(),
		"manifest paths and ids cannot escape the import root",
	)


func _test_generated_script_parses(root: String) -> void:
	var full := root.path_join(FULL_ASSET).path_join("fbx")
	var generated := full.path_join(Exports.SCRIPT_NAME)
	var script := GDScript.new()
	script.source_code = FileAccess.get_file_as_string(generated)
	var error := script.reload()
	_check(error == OK, "the exporter-generated GDScript parses (error %d)" % error)


func _test_export_contents(root: String) -> void:
	var full := root.path_join(FULL_ASSET).path_join("fbx")
	var record := Exports.load_manifest(full)
	_check(record.get("import_meshes", false), "meshes are imported by default")

	var materials: Array = record.get("materials", [])
	_check(materials.size() == 1, "the fixture has one material")
	if materials.size() != 1:
		return
	var maps: Array = materials[0]["maps"]
	_check(maps.size() == 6, "every map the exporter classified is listed")
	_check(maps.has("textures/rock_basecolor.png"), "the base colour map is listed")
	_check(maps.has("textures/rock_emission.png"), "the emission map is listed")

	_check(
		FileAccess.file_exists(full.path_join("textures/rock_orm.png")),
		"the ORM sources were packed into one texture",
	)
	var scene_path := full.path_join(Exports.SCENE_NAME)
	_check(FileAccess.file_exists(scene_path), "the scene was written")
	_check(
		FileAccess.get_file_as_string(scene_path).contains(Exports.SCRIPT_NAME),
		"the scene references the generated script",
	)
