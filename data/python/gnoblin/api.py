"""The ``gnoblin`` object: what a shell script talks to.

Everything here is a thin spelling of the core: it calls the built-in module ``_gnoblin_core`` (itself a layer over the installed
``libgnoblin-core``) and adds no compositor logic of its own. The typed classes of the tree are generated (``_generated``).

Registering. A *named subscription* is an attribute of ``gnoblin`` that registers a function::

    @gnoblin.on_client_created              # bare: the function is the callback, and it is returned unchanged
    def place(window): ...

    @gnoblin.bind("Super+Return")           # parameters first: the call returns the decorator
    def terminal(): gnoblin.spawn("foot")

    sub = gnoblin.bind("Super+d", launcher)       # parameters and the function: returns the Subscription
    sub = gnoblin.on_client_created.subscribe(place)   # a bare event's Subscription

A registration belongs to the generation of the script that made it. A reload removes all of them and runs the script again, so
a script never undoes its own listeners; ``Subscription.unsubscribe()`` and ``gnoblin.off(function)`` are for changing one's
mind while the shell runs.

Callbacks run on the shell thread. An exception in one is logged with its traceback and the next event is delivered.
"""

from __future__ import annotations

import os
from typing import Any, Callable, Mapping, Sequence

from ._base import Node
from ._native import core
from .events import (
    ChordEvent,
    GestureEvent,
    NodeEvent,
    PointerButton,
    PointerDrag,
    Process,
    ProcessExit,
    Subscription,
    WindowRequest,
    forget,
    remember,
)

_LEVELS = ("error", "warn", "info", "debug", "trace")


class Event:
    """One named subscription: ``nparams`` parameters (a class name, a chord) and then the function.

    ``event(fn)`` for an event without parameters, and ``event(*params)(fn)``, register and return the function. ``event(*params, fn)``
    and :meth:`subscribe` register and return the :class:`Subscription`.
    """

    __slots__ = ("_name", "_nparams", "_register")

    def __init__(self, name: str, nparams: int, register: Callable[..., int]) -> None:
        self._name = name
        self._nparams = nparams
        self._register = register

    def subscribe(self, *args: Any) -> Subscription:
        """Registers ``args[-1]`` with the parameters before it; returns the :class:`Subscription`."""
        if len(args) != self._nparams + 1 or not callable(args[-1]):
            raise TypeError(f"{self._name} takes {self._nparams} parameter(s) and then a function")
        subscription = Subscription(self._register(*args))
        remember(args[-1], subscription)
        return subscription

    def __call__(self, *args: Any) -> Any:
        if self._nparams == 0:
            if len(args) != 1 or not callable(args[0]):
                raise TypeError(f"{self._name} is a decorator: use @gnoblin.{self._name} on a function")
            self.subscribe(args[0])
            return args[0]
        if len(args) == self._nparams:

            def decorator(function: Callable[..., Any]) -> Callable[..., Any]:
                self.subscribe(*args, function)
                return function

            return decorator
        if len(args) == self._nparams + 1:
            return self.subscribe(*args)
        raise TypeError(f"{self._name} takes {self._nparams} parameter(s), optionally followed by the function")

    def __repr__(self) -> str:
        return f"<gnoblin.{self._name}>"


class Gnoblin:
    """The shell's handle on the compositor. Use the instance ``gnoblin``, not the class."""

    on_client_created: Event
    on_client_destroyed: Event
    on_title_changed: Event
    on_app_id_changed: Event
    on_focus_changed: Event
    on_window_property_changed: Event
    on_node_created: Event
    on_node_destroyed: Event
    on_class_property_changed: Event
    on_property_changed: Event
    on_window_request: Event
    on_node_event: Event
    on_signal: Event
    on_shutdown: Event
    bind: Event
    bind_events: Event
    bind_pointer: Event
    bind_gesture: Event
    claim_pointer_buttons: Event

    def __init__(self) -> None:
        events = {
            "on_client_created": (0, lambda fn: core.on_node_created("window", fn)),
            "on_client_destroyed": (0, lambda fn: core.on_node_destroyed("window", fn)),
            "on_title_changed": (0, lambda fn: core.on_class_property_changed("window", "title", fn)),
            "on_app_id_changed": (0, lambda fn: core.on_class_property_changed("window", "app_id", fn)),
            "on_focus_changed": (
                0,
                lambda fn: core.on_class_property_changed("seat", "focus", lambda seat, window: fn(window)),
            ),
            "on_window_property_changed": (1, lambda prop, fn: core.on_class_property_changed("window", prop, fn)),
            "on_node_created": (1, core.on_node_created),
            "on_node_destroyed": (1, core.on_node_destroyed),
            "on_class_property_changed": (2, core.on_class_property_changed),
            "on_property_changed": (2, lambda node, prop, fn: core.on_property_changed(node.node_id, prop, fn)),
            "on_window_request": (
                0,
                lambda fn: core.on_window_request(lambda window, name, x, y: fn(window, WindowRequest(name, x, y))),
            ),
            "on_node_event": (
                1,
                lambda node, fn: core.on_node_event(node.node_id, lambda *fields: fn(NodeEvent(*fields))),
            ),
            "on_signal": (2, core.on_signal),
            "on_shutdown": (0, core.on_shutdown),
            "bind": (1, core.bind),
            "bind_events": (1, lambda chord, fn: core.bind_events(chord, lambda c, s: fn(ChordEvent(c, s)))),
            "bind_pointer": (1, lambda chord, fn: core.bind_pointer(chord, lambda *f: fn(PointerDrag(*f)))),
            "bind_gesture": (
                2,
                lambda kind, fingers, fn: core.bind_gesture(kind, fingers, lambda *f: fn(GestureEvent(*f))),
            ),
            "claim_pointer_buttons": (
                1,
                lambda device, fn: core.claim_pointer_buttons(device, lambda *f: fn(PointerButton(*f))),
            ),
        }
        for name, (nparams, register) in events.items():
            setattr(self, name, Event(name, nparams, register))

    # ---- the tree ---------------------------------------------------------------------------------------------------

    @property
    def root(self) -> Node:
        """The root of the tree."""
        return Node._of(core.root())

    @property
    def windows(self) -> list[Node]:
        """The windows now in the tree, in stacking order (the first is the bottom)."""
        return [Node._of(id, "window") for id in core.windows()]

    def node(self, id: int) -> Node:
        """The handle for the node with this id."""
        return Node._of(id)

    def create_node(self, class_name: str) -> Node:
        """Makes a node of the class that is in no parent yet; put it in the tree with ``node.insert_into(parent)``."""
        return Node._of(core.create_node(class_name), class_name)

    def adopt_node(self, class_name: str, name: str, *, ephemeral: bool = False) -> tuple[Node, bool]:
        """Finds the node of this class and name an earlier generation made, or makes it. Returns the node and whether it was found."""
        id, adopted = core.adopt_node(class_name, name, ephemeral)
        return Node._of(id, class_name), adopted

    def flush(self, on_done: Callable[[int | None, str | None], None]) -> None:
        """Calls ``on_done(frame, error)`` once the compositor has applied everything written so far."""
        core.flush(on_done)

    # ---- timers, programs -------------------------------------------------------------------------------------------

    def after(self, ms: int, function: Callable[[], None]) -> Subscription:
        """Calls ``function()`` once, ``ms`` milliseconds from now."""
        return Subscription(core.after(ms, function))

    def every(self, ms: int, function: Callable[[], None]) -> Subscription:
        """Calls ``function()`` every ``ms`` milliseconds."""
        return Subscription(core.every(ms, function))

    def spawn(
        self,
        program: str | Sequence[str],
        *args: str,
        cwd: str | os.PathLike[str] | None = None,
        env: Mapping[str, str] | None = None,
        stdin: str | None = None,
        stdout: str = "inherit",
        stderr: str = "inherit",
        output_limit: int = 0,
        timeout: float | None = None,
        on_exit: Callable[[ProcessExit], None] | None = None,
    ) -> Process:
        """Starts a program in a process group of its own. The name is looked up on ``PATH``; it is not run through a shell::

            gnoblin.spawn("foot")
            gnoblin.spawn("foot", "-e", "htop")
            gnoblin.spawn(["sh", "-c", "notify-send hi"], stdout="capture", on_exit=lambda exit: print(exit.stdout))

        ``stdout`` and ``stderr`` are ``inherit`` (the session log), ``discard`` or ``capture`` (kept, up to ``output_limit`` bytes,
        for ``on_exit``). ``timeout`` is in seconds. A program the script's top level starts waits until the generation takes over, so a
        reload that fails starts nothing, and one the next generation starts again is adopted, not doubled.
        """
        argv = [program] if isinstance(program, str) else list(program)
        argv += args
        hook = None
        if on_exit is not None:
            hook = lambda *fields: on_exit(ProcessExit(*fields))  # noqa: E731
        token = core.spawn(
            argv,
            None if cwd is None else os.fspath(cwd),
            None if env is None else list(env.items()),
            stdin,
            stdout,
            stderr,
            output_limit,
            0 if timeout is None else int(timeout * 1000),
            hook,
        )
        return Process(token)

    # ---- input claims that return no events ------------------------------------------------------------------------

    def claim_device(self, device: str) -> Subscription:
        """Takes a device from the compositor's own input handling."""
        return Subscription(core.claim_device(device))

    def grab_input(self, node: Node, scope: str = "both") -> Subscription:
        """Sends the ``pointer``, the ``keyboard`` or ``both`` to the node until the claim is removed."""
        return Subscription(core.grab_input(node.node_id, scope))

    # ---- the shell --------------------------------------------------------------------------------------------------

    def off(self, function: Callable[..., Any]) -> int:
        """Removes every registration made for ``function``; returns how many were removed. One already removed is skipped."""
        removed = 0
        for subscription in forget(function):
            if subscription.active:
                try:
                    subscription.unsubscribe()
                    removed += 1
                except core.GnoblinError:
                    pass
        return removed

    def log(self, message: object, level: str = "info") -> None:
        """Writes a line to the compositor's log, from any thread."""
        if level not in _LEVELS:
            raise ValueError(f"a log level is one of {', '.join(_LEVELS)}, not {level!r}")
        core.log(level, str(message))

    def post(self, function: Callable[[], None]) -> None:
        """Runs ``function()`` on the shell thread at its next turn. The one call that is safe from any thread."""
        core.post(function)

    def request_reload(self) -> None:
        """Asks the host to reload this shell after the current callback returns."""
        core.request_reload()

    def request_exit(self, code: int = 0) -> None:
        """Asks the host to exit the compositor with ``code``."""
        core.request_exit(code)

    def keep_state(self, key: str, data: bytes, version: int = 0) -> None:
        """Keeps ``data`` for the next generation of this shell (a reload). The store is dropped when the compositor restarts."""
        core.keep_state(key, bytes(data), version)

    def kept_state(self, key: str) -> tuple[bytes, int] | None:
        """What an earlier generation kept under ``key`` as ``(data, version)``, or ``None``."""
        return core.kept_state(key)

    @property
    def info(self) -> dict[str, Any]:
        """The generation: ``generation``, ``reload_count``, ``module_path``, ``state_dir``, ``isolate``."""
        return core.info()

    @property
    def stats(self) -> dict[str, int]:
        """The core's counters: registrations, callbacks, panics, retired generations, stale calls."""
        return core.stats()


#: The one object a script uses: ``from gnoblin import gnoblin``.
gnoblin = Gnoblin()
