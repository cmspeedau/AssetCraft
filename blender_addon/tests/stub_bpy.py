"""A stand-in for Blender's `bpy`, enough for the generated script and the addon.

The point is not to fake Blender faithfully. It is to prove the generated
script's code paths actually run, that the addon registers and draws, and that
both use a bpy surface a real Blender offers. Anything the code touches that
this does not model fails loudly — an unknown node type raises, a missing
importer raises — so the stub cannot paper over a real problem.

`install()` builds a fresh tree, registers it as `bpy`, and returns it. The
state the tests read back lives on `bpy.state`: every operator call, every
image load, everything built.
"""

import os
import sys
import types

# The sockets of each node type the generated script builds, modelled on
# Blender 4.x. A type absent from here raises when created: if the script
# starts building a new node, the stub should say so rather than nod along.
NODE_SOCKETS = {
    "ShaderNodeOutputMaterial": (["Surface", "Displacement"], []),
    "ShaderNodeBsdfPrincipled": (
        [
            "Base Color",
            "Roughness",
            "Metallic",
            "Specular IOR Level",
            "Alpha",
            "Normal",
            "Emission Color",
            "Emission Strength",
        ],
        ["BSDF"],
    ),
    "ShaderNodeTexImage": (["Vector"], ["Color", "Alpha"]),
    "ShaderNodeNormalMap": (["Color", "Strength"], ["Normal"]),
    "ShaderNodeMath": (["Value", "Value"], ["Value"]),
    # 4.x removed this node and the generated script handles that with a
    # fallback; the stub offers it (like 3.x did) so the wiring it guards is
    # still exercised.
    "ShaderNodeMixRGB": (["Fac", "Color1", "Color2"], ["Color"]),
    "ShaderNodeDisplacement": (["Height", "Midlevel", "Scale"], ["Displacement"]),
}

# The importer operators the stub offers under `import_scene`. `wm` is absent
# entirely, which is what drives the generated script's fallback between the
# modern and the legacy operator names.
OFFERED_IMPORTERS = ("fbx", "obj", "gltf")


class StubError(AssertionError):
    """The code under test used a bpy surface this stub does not model."""


class Socket:
    def __init__(self, name, node):
        self.name = name
        self.node = node
        self.default_value = None


class Sockets:
    """Nameable and indexable socket collection, like Blender's."""

    def __init__(self, names, node):
        self._list = [Socket(name, node) for name in names]
        self._by_name = {}
        for socket in self._list:
            self._by_name.setdefault(socket.name, socket)

    def get(self, name):
        return self._by_name.get(name)

    def __contains__(self, name):
        return name in self._by_name

    def __getitem__(self, key):
        if isinstance(key, int):
            return self._list[key]
        socket = self._by_name.get(key)
        if socket is None:
            raise KeyError(key)
        return socket


class Node:
    def __init__(self, bl_idname):
        if bl_idname not in NODE_SOCKETS:
            raise StubError("the script built a node the stub does not know: " + bl_idname)
        inputs, outputs = NODE_SOCKETS[bl_idname]
        self.bl_idname = bl_idname
        self.location = (0, 0)
        self.image = None
        self.operation = None
        self.blend_type = None
        self.inputs = Sockets(inputs, self)
        self.outputs = Sockets(outputs, self)


class Links:
    def __init__(self):
        self.created = []

    def new(self, from_socket, to_socket):
        self.created.append((from_socket, to_socket))


class Nodes:
    def __init__(self, tree):
        self._tree = tree

    def new(self, bl_idname):
        node = Node(bl_idname)
        self._tree.built.append(node)
        self._tree.live.append(node)
        return node

    def clear(self):
        self._tree.live = []


class NodeTree:
    def __init__(self, material):
        self.material = material
        self.nodes = Nodes(self)
        self.links = Links()
        self.built = []  # every node ever created, surviving clear()
        self.live = []


class MaterialSlots:
    def __init__(self):
        self._items = []

    def append(self, material):
        self._items.append(material)

    def __setitem__(self, index, material):
        self._items[index] = material

    def __getitem__(self, index):
        return self._items[index]

    def __bool__(self):
        return bool(self._items)


class Mesh:
    def __init__(self, name):
        self.name = name
        self.materials = MaterialSlots()


class Object:
    def __init__(self, name, type_, data):
        self.name = name
        self.type = type_
        self.data = data


class Material:
    def __init__(self, name):
        self.name = name
        self.use_nodes = False
        self.blend_method = None
        self._node_tree = None

    @property
    def node_tree(self):
        if self._node_tree is None:
            self._node_tree = NodeTree(self)
        return self._node_tree


class Materials:
    def __init__(self):
        self._items = []

    def get(self, name):
        for material in self._items:
            if material.name == name:
                return material
        return None

    def new(self, name):
        material = Material(name)
        self._items.append(material)
        return material

    def __iter__(self):
        return iter(self._items)


class Images:
    def __init__(self, state):
        self._state = state

    def load(self, filepath, check_existing=False):
        self._state.images_loaded.append(filepath)
        image = types.SimpleNamespace(name=os.path.basename(filepath))
        self._state.images.append(image)
        return image


class State:
    """Everything a test wants to look at after running code."""

    def __init__(self):
        self.calls = []  # (operator, filepath) for every import operator
        self.images_loaded = []
        self.images = []
        self.registered = []


class Data:
    def __init__(self, state):
        self.materials = Materials()
        self.images = Images(state)
        self.objects = []
        self._state = state

    def _import(self, operator, filepath):
        self._state.calls.append((operator, filepath))
        name = os.path.splitext(os.path.basename(filepath))[0]
        self.objects.append(Object(name, "MESH", Mesh(name)))
        return {"FINISHED"}


class ImportScene:
    """`bpy.ops.import_scene`: the legacy home of the import operators."""

    def __init__(self, data):
        self._data = data

    def __getattr__(self, name):
        if name in OFFERED_IMPORTERS:
            def run(filepath=None):
                return self._data._import("import_scene." + name, filepath)

            return run
        raise AttributeError(name)


class Ops:
    """`bpy.ops`: offers `import_scene` and nothing else, so `bpy.ops.wm.*`
    raises AttributeError and the script's fallbacks do their job."""

    def __init__(self, data):
        self.import_scene = ImportScene(data)


class _PropertyMarker:
    def __init__(self, kind, kwargs):
        self.kind = kind
        self.kwargs = kwargs


class Props:
    """`bpy.props`: every property factory returns a marker object, which is
    all a class body needs for the attribute to exist at all."""

    def __getattr__(self, name):
        if name.endswith("Property"):
            def factory(*args, **kwargs):
                return _PropertyMarker(name, kwargs)

            return factory
        raise AttributeError(name)


class Types:
    """`bpy.types`: the base classes addon code subclasses."""

    class Operator:
        pass

    class Panel:
        pass

    class AddonPreferences:
        pass


class Utils:
    def __init__(self, state):
        self._state = state

    def register_class(self, cls):
        self._state.registered.append(cls)

    def unregister_class(self, cls):
        self._state.registered.remove(cls)


class PathHelpers:
    """`bpy.path`: just enough for the addon's preference handling."""

    @staticmethod
    def abspath(path):
        if not path:
            return ""
        return os.path.normpath(os.path.expanduser(path))


def forget():
    """Undo install(), so a later test cannot inherit a stubbed bpy."""
    for name in list(sys.modules):
        if name == "bpy" or name.startswith("bpy."):
            sys.modules.pop(name, None)


def install():
    """Build a fresh stub `bpy`, register it under the name `bpy`, return it."""
    bpy = types.ModuleType("bpy")
    state = State()
    data = Data(state)
    bpy.state = state
    bpy.data = data
    bpy.ops = Ops(data)
    bpy.props = _submodule("bpy.props", Props())
    bpy.types = _submodule("bpy.types", Types())
    bpy.utils = _submodule("bpy.utils", Utils(state))
    bpy.path = _submodule("bpy.path", PathHelpers())
    sys.modules["bpy"] = bpy
    bpy.__path__ = []  # marks it a package, so `bpy` and `bpy.props` both import
    return bpy


def _submodule(name, contents):
    """Wrap `contents` as a real module under `bpy.<name>`, importably.

    `from bpy.props import StringProperty` goes through the import system, so
    the stub's namespaces have to be registered submodules, not just
    attributes. Names the wrapped object answers through `__getattr__` (the
    property factories) are delegated the same way PEP 562 lets real modules.
    """
    module = types.ModuleType(name)
    for attribute in dir(contents):
        if not attribute.startswith("__"):
            setattr(module, attribute, getattr(contents, attribute))
    if hasattr(type(contents), "__getattr__"):
        module.__getattr__ = lambda attribute, _c=contents: getattr(_c, attribute)
    sys.modules[name] = module
    return module


def describe_link(link):
    """(from node, from socket, to node, to socket) for an asserted-about link."""
    from_socket, to_socket = link
    return (
        from_socket.node.bl_idname,
        from_socket.name,
        to_socket.node.bl_idname,
        to_socket.name,
    )
