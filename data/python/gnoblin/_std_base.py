"""The hand-written base of the generated ``gnoblin.std``: the native module, and how a value crosses.

Everything typed in ``std.py`` is generated from the std table; what is here does not depend on the table. ``call`` is the one
native entry (``_gnoblin_std.call(name, *args)``, generic by table entry), ``dyn`` turns a script value into what the native layer
takes (a record class into a dict, a handle class into its id, a list into a list of those), ``listener`` and ``wrap`` turn what
comes back into the generated classes.

Importing this module opens ``libgnoblin-std.so.1``; when it is not installed the import raises ``ImportError`` with the text every
runtime gives (``gnoblin-std is not installed (looked for libgnoblin-std.so.1 in ...)``).
"""

from __future__ import annotations

from enum import Enum
from typing import Any, Callable, ClassVar, Sequence

import _gnoblin_std as _native

_native.load()

call = _native.call
functions = _native.functions


class Handle:
    """A value the library names by a number: a collection, a workspace, a registration, a node.

    The number is the one attribute the subclass names in ``__slots__``.
    """

    __slots__ = ()

    def _id(self) -> int:
        return getattr(self, self.__slots__[0])

    def _to_dyn(self) -> int:
        return self._id()

    def __eq__(self, other: object) -> bool:
        return type(other) is type(self) and other._id() == self._id()  # type: ignore[attr-defined]

    def __hash__(self) -> int:
        return hash((type(self).__name__, self._id()))

    def __repr__(self) -> str:
        return f"<{type(self).__name__} {self._id()}>"


def ident(value: Any) -> int:
    """The number behind a handle, a node or a plain integer."""
    if isinstance(value, Handle):
        return value._id()
    node = getattr(value, "_gnoblin_node_id", None)
    if node is not None:
        return int(node)
    return int(value)


def dyn(value: Any) -> Any:
    """A script value as the native layer takes it: records and handles as the plain values they carry."""
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    convert = getattr(value, "_to_dyn", None)
    if convert is not None:
        return convert()
    node = getattr(value, "_gnoblin_node_id", None)
    if node is not None:
        return int(node)
    if isinstance(value, dict):
        return {key: dyn(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [dyn(item) for item in value]
    return value


def wrap(cls: Callable[[int], Any], raw: Any) -> Any:
    """``cls(raw)`` unless the library said there is none (``None``)."""
    return None if raw is None else cls(raw)


def listener(function: Callable[..., Any], converters: Sequence[Callable[[Any], Any] | None]) -> Callable[..., Any]:
    """Wraps a script's function so the records the library passes arrive as the generated classes."""
    if not callable(function):
        raise TypeError(f"expected a function to call, got {type(function).__name__}")

    def call_back(*args: Any) -> Any:
        return function(*(convert(arg) if convert is not None else arg for arg, convert in zip(args, converters)))

    return call_back


def kind(enumeration: type[Enum], value: int) -> Any:
    """The member of ``enumeration`` for ``value``, or the number itself when the library is newer than this module."""
    try:
        return enumeration(value)
    except ValueError:
        return value


def _option(name: str, value: Any, shape: tuple[int, str, bool]) -> dict[str, Any]:
    """One typed option as the table carries it: its ``STD_VALUE_*`` kind, its name and the field that holds the value."""
    kind_number, field, _whole = shape
    if kind_number == 1:  # STD_VALUE_BOOL
        value = 1 if value else 0
    elif field == "node":
        value = ident(value)
    elif field == "number":
        value = float(value)
    return {"kind": kind_number, "name": name, field: value}


def _option_value(native: dict[str, Any], whole: bool) -> Any:
    """What an option or an action answers: ``None`` for nothing, else a boolean, a number, a node or text."""
    kind_number = native["kind"]
    if kind_number == 0:
        return None
    if kind_number == 1:
        return bool(native["flag"])
    if kind_number in (2, 3):
        return int(native["number"]) if whole else native["number"]
    if kind_number == 4:
        return native["node"]
    return native["text"]


class BuiltinLayout:
    """A layout the library runs: a kind of ``gnoblin.std.LAYOUT_KINDS`` with its options and actions, written by the generator.

    Options given before ``attach`` are kept here; after it they are the workspace's, read and written through the library.
    """

    KIND: ClassVar[int] = 0
    SHAPES: ClassVar[dict[str, tuple[int, str, bool]]] = {}
    ACTION_SHAPES: ClassVar[dict[str, tuple[int, str, bool]]] = {}

    def __init__(self, options: dict[str, Any]) -> None:
        self._options = options
        self.workspace: Any = None

    def attach(self, workspace: Any) -> Any:
        """Gives ``workspace`` this layout; its options and actions act on that workspace from then on."""
        given = [_option(name, value, self.SHAPES[name]) for name, value in self._options.items() if value is not None]
        call("workspace_set_layout_kind", ident(workspace), self.KIND, given)
        self.workspace = workspace
        return self

    def _get(self, name: str) -> Any:
        if self.workspace is None:
            return self._options.get(name)
        native = call("workspace_layout_get", ident(self.workspace), name)
        return _option_value(native, self.SHAPES[name][2])

    def _set(self, name: str, value: Any) -> None:
        self._options[name] = value
        if self.workspace is not None and value is not None:
            call("workspace_layout_set", ident(self.workspace), _option(name, value, self.SHAPES[name]))

    def _act(self, name: str, args: list[tuple[str, Any]]) -> Any:
        if self.workspace is None:
            raise RuntimeError(f"{name}: the layout is not attached to a workspace")
        given = [_option(arg, value, self.ACTION_SHAPES[f"{name}.{arg}"]) for arg, value in args if value is not None]
        native = call("workspace_layout_act", ident(self.workspace), name, given)
        answer = self.ACTION_SHAPES.get(f"{name}->")
        return _option_value(native, answer[2] if answer else False)


class BuiltinBinding:
    """A pointer binding the library runs: a kind of ``BINDING_KINDS`` with its options, written by the generator."""

    KIND: ClassVar[int] = 0
    SHAPES: ClassVar[dict[str, tuple[int, str, bool]]] = {}
    registration_class: ClassVar[Any] = None

    def __init__(self, options: dict[str, Any]) -> None:
        self._options = options

    def start(self) -> Any:
        """Claims the binding's trigger. ``remove()`` on the result ends it."""
        given = [_option(name, value, self.SHAPES[name]) for name, value in self._options.items() if value is not None]
        raw = call("binding_builtin", self.KIND, given)
        return self.registration_class(raw)
