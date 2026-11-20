"""The generic base of the typed classes in ``_generated/classes``.

The classes themselves (``Window``, ``Seat``, ``Screen`` and the rest) are generated from the schema catalog; nothing about one of them is
written by hand. What is here is what a generated class is made of:

* :class:`Node`, a handle on one node of the compositor's tree, by id;
* :class:`Property`, :class:`Method` and :class:`Signal`, descriptors that read, call and listen **by name** through ``_gnoblin_core``, so a
  property added to the schema works the day it is generated.

A handle holds the node's id and nothing else. The compositor owns the node: reading a window that has closed raises
:class:`gnoblin.errors.NodeGoneError`, and a handle kept across a reload keeps working because the node does.
"""

from __future__ import annotations

from typing import Any, Callable, ClassVar, Generic, Iterable, TypeVar

from .events import Subscription

_T = TypeVar("_T")
_F = TypeVar("_F", bound=Callable[..., Any])

#: Every generated class by its schema name (``"window"``). Filled as the classes are defined.
CLASSES: dict[str, type[Node]]

class Property(Generic[_T]):
    """A property of a node class, read and written by name."""

    name: str
    type: Any
    readonly: bool
    animated: bool
    attribute: str
    def __init__(self, name: str, type: Any = ..., readonly: bool = ..., animated: bool = ...) -> None: ...
    def __set_name__(self, owner: type, attribute: str) -> None: ...
    def __get__(self, node: Node | None, owner: type | None = None) -> Any: ...
    def __set__(self, node: Node, value: Any) -> None: ...

class Method:
    """A method of a node class. Calling it asks the compositor; it returns ``None`` and the answer, if wanted, goes to
    ``on_answer(result: bytes, error: str | None)``."""

    name: str
    params: tuple[tuple[Any, ...], ...]
    returns: Any
    attribute: str
    def __init__(self, name: str, params: Iterable[tuple[Any, ...]] = ..., returns: Any = ...) -> None: ...
    def __set_name__(self, owner: type, attribute: str) -> None: ...
    def __get__(self, node: Node | None, owner: type | None = None) -> Any: ...

class Signal:
    """A signal of a node class. ``Window.closed`` listens to every window, ``window.closed`` to that one; both are used as a decorator
    (``@Window.closed``) or called with the function (``Window.closed(fn)``), which returns the function. ``.subscribe(fn)`` returns the
    :class:`gnoblin.Subscription` instead."""

    name: str
    fields: tuple[tuple[Any, ...], ...]
    attribute: str
    def __init__(self, name: str, fields: Iterable[tuple[Any, ...]] = ...) -> None: ...
    def __set_name__(self, owner: type, attribute: str) -> None: ...
    def __get__(self, node: Node | None, owner: type | None = None) -> _SignalHook: ...

class _SignalHook:
    """A signal seen from a class or from one node: ``Window.closed`` or ``window.closed``."""

    def subscribe(self, callback: Callable[..., None]) -> Subscription:
        """Registers ``callback(source, values)`` and returns the :class:`Subscription` that removes it."""
    def __call__(self, callback: _F) -> _F:
        """Registers ``callback`` and returns it, so the call works as a decorator."""

class Node:
    """A node of the compositor's tree, by id. The typed subclasses add the schema's properties, methods and signals."""

    __class_name__: ClassVar[str]
    def __init__(self, id: int) -> None: ...
    @property
    def node_id(self) -> int:
        """The node's id in the tree."""
    @staticmethod
    def _of(id: int, class_name: str | None = None) -> Node:
        """The typed handle for node ``id``: the class registered for its schema name, or :class:`Node` for one the generated classes
        do not know (or one that has left the tree and cannot be asked)."""
    def is_alive(self) -> bool:
        """Whether the node is in the tree now."""
    def class_name(self) -> str:
        """The schema name of the node's class."""
    def children(self) -> list[Node]:
        """The node's children, in order."""
    def get(self, name: str) -> Any:
        """Reads a property by its schema name."""
    def set(self, name: str, value: Any, seconds: float = 0.0) -> None:
        """Writes a property by its schema name; ``seconds`` asks the compositor to animate an animated property."""
    def call(self, method: str, *args: Any, on_answer: Callable[[bytes, str | None], None] | None = None) -> None:
        """Calls a method by its schema name."""
    def insert_into(self, parent: Node, index: int | None = None) -> None:
        """Puts the node under ``parent`` at ``index`` (the top when ``None``), moving it from where it was."""
    def destroy(self) -> None:
        """Destroys the node and everything beneath it. To close a window, call its ``close`` method."""
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...
