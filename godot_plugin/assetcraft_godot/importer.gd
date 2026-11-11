## Filesystem operations shared by the editor dock and headless tests.
##
## Kept outside `EditorPlugin`: Godot only permits that virtual base class to
## be instantiated by the editor, while these path and copy rules are ordinary
## project operations that can be tested headlessly.
extends RefCounted

const IMPORT_ROOT := "res://assetcraft_imports"


static func destination(asset: String, format: String) -> String:
	if not _is_safe_segment(asset) or not _is_safe_segment(format):
		return ""
	return IMPORT_ROOT.path_join(asset).path_join(format)


static func is_within_import_root(path: String) -> bool:
	var relative := path.trim_prefix(IMPORT_ROOT).trim_prefix("/")
	if relative == path:
		return false
	var segments := relative.split("/", false)
	if segments.size() != 2:
		return false
	for segment in segments:
		if not _is_safe_segment(segment):
			return false
	return true


static func mirror_tree(source: String, destination_path: String) -> bool:
	if not is_within_import_root(destination_path):
		return false
	return _mirror_tree(source, destination_path)


static func _mirror_tree(source: String, destination_path: String) -> bool:
	var dir := DirAccess.open(source)
	if dir == null:
		return false
	if DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(destination_path)) != OK:
		return false

	var names: Array = []
	dir.list_dir_begin()
	var name := dir.get_next()
	while name != "":
		if not name.begins_with(".") and name != "project.godot":
			names.append(name)
		name = dir.get_next()
	dir.list_dir_end()

	for entry in names:
		var source_path := source.path_join(entry)
		var target_path := destination_path.path_join(entry)
		if DirAccess.dir_exists_absolute(ProjectSettings.globalize_path(source_path)):
			if not _mirror_tree(source_path, target_path):
				return false
		elif DirAccess.copy_absolute(
			ProjectSettings.globalize_path(source_path),
			ProjectSettings.globalize_path(target_path),
		) != OK:
			return false
	return true


static func remove_tree(path: String) -> void:
	if not is_within_import_root(path):
		return
	_remove_tree_unchecked(path)


static func _remove_tree_unchecked(path: String) -> void:
	if not DirAccess.dir_exists_absolute(ProjectSettings.globalize_path(path)):
		return
	var dir := DirAccess.open(path)
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
		var child := path.path_join(entry)
		if DirAccess.dir_exists_absolute(ProjectSettings.globalize_path(child)):
			_remove_tree_unchecked(child)
		else:
			DirAccess.remove_absolute(ProjectSettings.globalize_path(child))
	DirAccess.remove_absolute(ProjectSettings.globalize_path(path))


static func _is_safe_segment(value: String) -> bool:
	return (
		value != ""
		and value != "."
		and value != ".."
		and not value.contains("/")
		and not value.contains("\\")
		and not value.contains(":")
	)
