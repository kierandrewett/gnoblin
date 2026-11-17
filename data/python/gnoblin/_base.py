"""The generic base of the typed classes in ``_generated/classes.py``.

The classes themselves (``Window``, ``Seat``, ``Screen`` and the rest) are generated from the schema catalog by
``cargo xtask gen-sdks``; nothing about one of them is written here. What is here is what a generated class is made of:

* :class:`Node`, a handle on one node of the compositor's tree, by id;
* :class:`Property`, :class:`Method` and :class:`Signal`, descriptors that read, call and listen **by name** through
  ``_gnoblin_core``, so a property added to the schema works the day it is generated.

A handle holds the node's id and nothing else. The compositor owns the node: reading a window that has closed raises
:class:`gnoblin.errors.NodeGoneError`, and a handle kept across a reload keeps working because the node does.
"""

from __future__ import annotations

from typing import Any, Callable, ClassVar, Generic, Iterable, TypeVar

from ._native import core

T = TypeVar("T")

#: Every generated class by its schema name (``"window"``). Filled as the classes are defined.
CLASSES: dict[str, type[Node]] = {}


class Property(Generic[T]):
    """A property of a node class, read and written by name."""

    __slots__ = ("name", "type", "readonly", "animated", "attribute")

    def __init__(self, name: str, type: Any = object, readonly: bool = False, animated: bool = False) -> None:
        self.name = name
        self.type = type
        self.readonly = readonly
        self.animated = animated
        self.attribute = name

    def __set_name__(self, owner: type, attribute: str) -> None:
        self.attribute = attribute

    def __get__(self, node: Node | None, owner: type | None = None) -> Any:
        if node is None:
            return self
        return core.node_get(node._id, self.name)

    def __set__(self, node: Node, value: Any) -> None:
        if self.readonly:
            raise AttributeError(f"{type(node).__name__}.{self.attribute} is read-only")
        core.node_set(node._id, self.name, value)

    def __repr__(self) -> str:
        return f"Property({self.name!r}, readonly={self.readonly}, animated={self.animated})"


class Method:
    """A method of a node class. Calling it asks the compositor; it returns ``None`` and the answer, if wanted, goes to
    ``on_answer(result: bytes, error: str | None)``."""

    __slots__ = ("name", "params", "returns", "attribute")

    def __init__(self, name: str, params: Iterable[tuple[Any, ...]] = (), returns: Any = None) -> None:
        self.name = name
        self.params = tuple(params)
        self.returns = returns
        self.attribute = name

    def __set_name__(self, owner: type, attribute: str) -> None:
        self.attribute = attribute

    def __get__(self, node: Node | None, owner: type | None = None) -> Any:
        if node is None:
            return self
        return _BoundMethod(self, node)

    def __repr__(self) -> str:
        return f"Method({self.name!r}, {self.params!r})"


class _BoundMethod:
    __slots__ = ("_method", "_node")

    def __init__(self, method: Method, node: Node) -> None:
        self._method = method
        self._node = node

    def __call__(self, *args: Any, on_answer: Callable[[bytes, str | None], None] | None = None) -> None:
        method = self._method
        required = sum(1 for param in method.params if len(param) < 3 or not param[2])
        if not required <= len(args) <= len(method.params):
            count = f"{required}" if required == len(method.params) else f"{required} to {len(method.params)}"
            raise TypeError(f"{method.name}() takes {count} argument(s), {len(args)} given")
        core.node_call(self._node._id, method.name, tuple(args), on_answer)

    def __repr__(self) -> str:
        return f"<method {self._method.name} of {self._node!r}>"


class Signal:
    """A signal of a node class. ``Window.closed`` listens to every window, ``window.closed`` to that one; both are used as
    a decorator (``@Window.closed``) or called with the function (``Window.closed(fn)``), which returns the function.
    ``.subscribe(fn)`` returns the :class:`gnoblin.Subscription` instead."""

    __slots__ = ("name", "fields", "attribute")

    def __init__(self, name: str, fields: Iterable[tuple[Any, ...]] = ()) -> None:
        self.name = name
        self.fields = tuple(fields)
        self.attribute = name

    def __set_name__(self, owner: type, attribute: str) -> None:
        self.attribute = attribute

    def __get__(self, node: Node | None, owner: type | None = None) -> Any:
        if node is None:
            return _SignalHook(owner, self, None)
        return _SignalHook(type(node), self, node)

    def __repr__(self) -> str:
        return f"Signal({self.name!r}, {self.fields!r})"


class _SignalHook:
    def __init__(self, owner: type | None, signal: Signal, node: Node | None) -> None:
        self._owner = owner
        self._signal = signal
        self._node = node

    def subscribe(self, callback: Callable[..., None]) -> Any:
        from .events import Subscription

        node = self._node
        if node is not None:
            inner = callback
            callback = lambda source, values: inner(source, values) if source == node else None  # noqa: E731
        class_name = getattr(self._owner, "__class_name__", "node")
        return Subscription(core.on_signal(class_name, self._signal.name, callback))

    def __call__(self, callback: Callable[..., None]) -> Callable[..., None]:
        from .events import remember

        remember(callback, self.subscribe(callback))
        return callback


class Node:
    """A node of the compositor's tree, by id. The typed subclasses add the schema's properties, methods and signals."""

    __slots__ = ("_id",)
    __class_name__: ClassVar[str] = "node"

    def __init_subclass__(cls, **kwargs: Any) -> None:
        super().__init_subclass__(**kwargs)
        name = cls.__dict__.get("__class_name__")
        if name is not None:
            CLASSES[name] = cls

    def __init__(self, id: int) -> None:
        self._id = id

    @property
    def _gnoblin_node_id(self) -> int:
        return self._id

    @property
    def node_id(self) -> int:
        """The node's id in the tree."""
        return self._id

    @staticmethod
    def _of(id: int, class_name: str | None = None) -> Node:
        """The typed handle for node ``id``: the class registered for its schema name, or :class:`Node` for one the generated
        classes do not know (or one that has left the tree and cannot be asked)."""
        if class_name is None:
            try:
                class_name = core.node_class(id)
            except core.NodeGoneError:
                return Node(id)
        return CLASSES.get(class_name, Node)(id)

    def is_alive(self) -> bool:
        """Whether the node is in the tree now."""
        return core.node_exists(self._id)

    def class_name(self) -> str:
        """The schema name of the node's class."""
        return core.node_class(self._id)

    def children(self) -> list[Node]:
        """The node's children, in order."""
        return [Node._of(child) for child in core.node_children(self._id)]

    def get(self, name: str) -> Any:
        """Reads a property by its schema name."""
        return core.node_get(self._id, name)

    def set(self, name: str, value: Any, seconds: float = 0.0) -> None:
        """Writes a property by its schema name; ``seconds`` asks the compositor to animate an animated property."""
        core.node_set(self._id, name, value, seconds)

    def call(self, method: str, *args: Any, on_answer: Callable[[bytes, str | None], None] | None = None) -> None:
        """Calls a method by its schema name."""
        core.node_call(self._id, method, tuple(args), on_answer)

    def insert_into(self, parent: Node, index: int | None = None) -> None:
        """Puts the node under ``parent`` at ``index`` (the top when ``None``), moving it from where it was."""
        core.node_insert(self._id, parent._id, index)

    def destroy(self) -> None:
        """Destroys the node and everything beneath it. To close a window, call its ``close`` method."""
        core.node_destroy(self._id)

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Node) and other._id == self._id

    def __hash__(self) -> int:
        return hash(self._id)

    def __repr__(self) -> str:
        return f"<{type(self).__name__} {self._id}>"


core.set_node_factory(Node._of)
