@tool
extends EditorPlugin
## The AssetCraft dock: browse exports written by `assetcraft export` and copy
## one into this project.
##
## The plugin deliberately holds no export logic. It reads the same
## `assetcraft_export.json` manifest the Blender addon reads (see `exports.gd`)
## and copies an export's directory into `res://assetcraft_imports/`. Godot
## then imports the meshes and textures, and the exported script builds the
## materials when the scene runs.

const Exports = preload("exports.gd")

const DOCK_TITLE := "AssetCraft"
const Importer = preload("importer.gd")

var _dock: Control
var _path_edit: LineEdit
var _list: VBoxContainer
var _status: Label
var _pending_scene := ""
var _pending_export: Dictionary = {}
var _replace_dialog: ConfirmationDialog


func _enter_tree() -> void:
	_dock = _build_dock()
	add_control_to_dock(DOCK_SLOT_RIGHT_UL, _dock)

	var filesystem := get_editor_interface().get_resource_filesystem()
	if not filesystem.filesystem_changed.is_connected(_on_filesystem_changed):
		filesystem.filesystem_changed.connect(_on_filesystem_changed)

	_refresh()


func _exit_tree() -> void:
	var filesystem := get_editor_interface().get_resource_filesystem()
	if filesystem.filesystem_changed.is_connected(_on_filesystem_changed):
		filesystem.filesystem_changed.disconnect(_on_filesystem_changed)

	if _dock != null:
		remove_control_from_docks(_dock)
		_dock.queue_free()
		_dock = null


func _build_dock() -> Control:
	var margin := MarginContainer.new()
	margin.name = DOCK_TITLE
	margin.custom_minimum_size = Vector2(280, 0)
	margin.add_theme_constant_override("margin_left", 8)
	margin.add_theme_constant_override("margin_right", 8)
	margin.add_theme_constant_override("margin_top", 8)
	margin.add_theme_constant_override("margin_bottom", 8)

	var box := VBoxContainer.new()
	margin.add_child(box)

	var title := Label.new()
	title.text = "AssetCraft exports"
	title.add_theme_font_size_override("font_size", 14)
	box.add_child(title)

	var hint := Label.new()
	hint.text = "Copy an export into this project and open its scene."
	hint.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	hint.add_theme_color_override("font_color", Color(0.62, 0.65, 0.7))
	box.add_child(hint)

	var path_row := HBoxContainer.new()
	_path_edit = LineEdit.new()
	_path_edit.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_path_edit.text = Exports.default_library_root()
	_path_edit.placeholder_text = "Folder AssetCraft exports to"
	_path_edit.tooltip_text = "The --target folder used by the CLI, or the app's export library."
	_path_edit.text_submitted.connect(func(_text): _refresh())
	path_row.add_child(_path_edit)

	var refresh := Button.new()
	refresh.text = "Refresh"
	refresh.pressed.connect(_refresh)
	path_row.add_child(refresh)
	box.add_child(path_row)

	var scroll := ScrollContainer.new()
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	_list = VBoxContainer.new()
	_list.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.add_child(_list)
	box.add_child(scroll)

	_status = Label.new()
	_status.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_status.add_theme_color_override("font_color", Color(0.62, 0.65, 0.7))
	box.add_child(_status)

	_replace_dialog = ConfirmationDialog.new()
	_replace_dialog.title = "Replace imported export?"
	_replace_dialog.dialog_text = "This replaces the existing files for this asset and format."
	_replace_dialog.confirmed.connect(_confirm_replace)
	_replace_dialog.canceled.connect(_cancel_replace)
	margin.add_child(_replace_dialog)

	return margin


func _refresh() -> void:
	for child in _list.get_children():
		child.queue_free()

	var root := _path_edit.text.strip_edges()
	if root == "":
		_status.text = "Set the folder AssetCraft exports to."
		return

	var exports := Exports.discover(root)
	if exports.is_empty():
		_status.text = "No exports found in " + root
		return

	_status.text = "%d export%s found." % [exports.size(), "" if exports.size() == 1 else "s"]
	for record in exports:
		_list.add_child(_make_row(record))


func _make_row(record: Dictionary) -> Control:
	var row := HBoxContainer.new()

	var summary := VBoxContainer.new()
	summary.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	summary.add_theme_constant_override("separation", 0)
	var title := Label.new()
	title.text = "%s (%s)" % [record["asset"], record["format"]]
	summary.add_child(title)
	var detail := Label.new()
	var materials: Array = record["materials"]
	detail.text = "%d material%s · %s" % [
		materials.size(),
		"" if materials.size() == 1 else "s",
		Exports.mesh_kind(record),
	]
	detail.add_theme_font_size_override("font_size", 11)
	detail.add_theme_color_override("font_color", Color(0.62, 0.65, 0.7))
	summary.add_child(detail)
	row.add_child(summary)

	var import_button := Button.new()
	import_button.text = "Import"
	import_button.tooltip_text = "Copy this export into res://assetcraft_imports and open its scene."
	import_button.pressed.connect(_import.bind(record))
	row.add_child(import_button)

	return row


func _import(record: Dictionary) -> void:
	var asset := str(record["asset"])
	var format := str(record["format"])
	var destination := Importer.destination(asset, format)
	if destination == "":
		_status.text = "Rejected unsafe asset or format path in the export manifest."
		return
	if DirAccess.dir_exists_absolute(ProjectSettings.globalize_path(destination)):
		_pending_export = record
		_replace_dialog.dialog_text = (
			"Replace %s (%s)?\n\nThis removes the previous import at %s."
			% [asset, format, destination]
		)
		_replace_dialog.popup_centered()
		return
	_import_now(record)


func _confirm_replace() -> void:
	if _pending_export.is_empty():
		return
	var record := _pending_export
	_pending_export = {}
	_import_now(record)


func _cancel_replace() -> void:
	_pending_export = {}


func _import_now(record: Dictionary) -> void:
	var asset := str(record["asset"])
	var format := str(record["format"])
	var destination := Importer.destination(asset, format)
	if destination == "":
		_status.text = "Rejected unsafe asset or format path in the export manifest."
		return

	_status.text = "Importing %s (%s)…" % [asset, format]
	if DirAccess.dir_exists_absolute(ProjectSettings.globalize_path(destination)):
		Importer.remove_tree(destination)
	if not Importer.mirror_tree(str(record["dir"]), destination):
		Importer.remove_tree(destination)
		_status.text = "Could not copy " + str(record["dir"])
		return
	if not FileAccess.file_exists(destination.path_join(Exports.SCENE_NAME)):
		Importer.remove_tree(destination)
		_status.text = "The export has no %s scene." % Exports.SCENE_NAME
		return

	# Textures are read from disk by the generated script, so they work before
	# import. Meshes are imported by Godot asynchronously — open the scene once
	# the filesystem reports it has settled, or its mesh references are null.
	_pending_scene = destination.path_join(Exports.SCENE_NAME)
	_status.text = "Copied %s; waiting for Godot to import its files…" % asset
	get_editor_interface().get_resource_filesystem().scan()


func _on_filesystem_changed() -> void:
	if _pending_scene == "":
		return
	var scene := _pending_scene
	_pending_scene = ""
	if not ResourceLoader.exists(scene):
		_status.text = "Copied, but the scene was not found at " + scene
		return
	get_editor_interface().open_scene_from_path(scene)
	_status.text = "Opened " + scene
