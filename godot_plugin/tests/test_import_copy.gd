extends SceneTree
## Smoke-test the editor plugin's actual import/copy methods without opening
## an editor window. The test creates a normal Godot project, loads the plugin
## script, and calls the same copy helper that the dock uses.

const Importer = preload("res://addons/assetcraft_godot/importer.gd")

var _passed := 0
var _failed := 0


func _init() -> void:
	_test_rejects_unsafe_destination()
	_test_mirrors_export_but_skips_project_file()
	_test_removes_existing_tree()
	print("Godot plugin import tests: %d passed, %d failed." % [_passed, _failed])
	quit(1 if _failed > 0 else 0)


func _check(condition: bool, message: String) -> void:
	if condition:
		_passed += 1
	else:
		_failed += 1
		push_error("FAIL: " + message)


func _test_rejects_unsafe_destination() -> void:
	_check(not Importer.is_within_import_root("res://outside/escape"), "rejects destinations outside its root")
	_check(not Importer.is_within_import_root("res://assetcraft_imports/../escape/fbx"), "rejects parent traversal")
	_check(not Importer.is_within_import_root("res://assetcraft_imports/asset/fbx/extra"), "requires exactly asset/format")
	_check(Importer.is_within_import_root("res://assetcraft_imports/alpha-rock/fbx"), "accepts a safe asset/format destination")


func _test_mirrors_export_but_skips_project_file() -> void:
	var source := "user://assetcraft-plugin-test-source"
	var destination := "res://assetcraft_imports/test-copy/fbx"
	_remove_absolute(source)
	Importer.remove_tree(destination)
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(source).path_join("textures"))
	_write(source.path_join("project.godot"), "standalone project")
	_write(source.path_join("assetcraft_export.gd"), "extends Node3D")
	_write(source.path_join("textures/rock_orm.png"), "orm")

	_check(Importer.mirror_tree(source, destination), "copy helper succeeds")
	_check(FileAccess.file_exists(destination.path_join("assetcraft_export.gd")), "copies the scene script")
	_check(FileAccess.file_exists(destination.path_join("textures/rock_orm.png")), "copies nested textures")
	_check(not FileAccess.file_exists(destination.path_join("project.godot")), "does not copy the standalone project file")

	_remove_absolute(source)
	Importer.remove_tree(destination)


func _test_removes_existing_tree() -> void:
	var path := "res://assetcraft_imports/test-remove/fbx"
	Importer.remove_tree(path)
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(path).path_join("nested"))
	_write(path.path_join("nested/file.txt"), "old")
	Importer.remove_tree(path)
	var parent := DirAccess.open("res://assetcraft_imports/test-remove")
	_check(parent != null and not parent.dir_exists("fbx"), "removes the prior import tree")


func _write(path: String, text: String) -> void:
	var file := FileAccess.open(path, FileAccess.WRITE)
	if file != null:
		file.store_string(text)
		file.close()


func _remove_absolute(path: String) -> void:
	var absolute := ProjectSettings.globalize_path(path)
	if not DirAccess.dir_exists_absolute(absolute):
		return
	var dir := DirAccess.open(absolute)
	if dir == null:
		return
	var names: Array = []
	dir.list_dir_begin()
	var name := dir.get_next()
	while name != "":
		if name != "." and name != "..":
			names.append(name)
		name = dir.get_next()
	dir.list_dir_end()
	for entry in names:
		var child := absolute.path_join(entry)
		if DirAccess.dir_exists_absolute(child):
			_remove_absolute("user://" + child.get_file())
		else:
			DirAccess.remove_absolute(child)
	DirAccess.remove_absolute(absolute)
