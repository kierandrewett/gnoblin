"""The ``gnoblin`` object: what a shell script talks to.

Everything here is a thin spelling of the core. The typed classes of the tree are generated (``_generated``).

Registering. A *named subscription* is an attribute of ``gnoblin`` that registers a function::

    @gnoblin.on_client_created              # bare: the function is the callback, and it is returned unchanged
    def place(window): ...

    @gnoblin.bind("<Super>Return")           # parameters first: the call returns the decorator
    def terminal(): gnoblin.spawn("foot")

    sub = gnoblin.bind("<Super>d", launcher)       # parameters and the function: returns the Subscription
    sub = gnoblin.on_client_created.subscribe(place)   # a bare event's Subscription

A registration belongs to the generation of the script that made it. A reload removes all of them and runs the script again, so a script
never undoes its own listeners; ``Subscription.unsubscribe()`` and ``gnoblin.off(function)`` are for changing one's mind while the shell
runs.

Callbacks run on the shell thread. An exception in one is logged with its traceback and the next event is delivered.
"""

from __future__ import annotations

import os
from typing import Any, Callable, Mapping, Sequence

from ._base import Node
from .events import Process, ProcessExit, Subscription

class Event:
    """One named subscription: parameters (a class name, a chord) and then the function.

    ``event(fn)`` for an event without parameters, and ``event(*params)(fn)``, register and return the function. ``event(*params, fn)``
    and :meth:`subscribe` register and return the :class:`Subscription`.
    """

    def subscribe(self, *args: Any) -> Subscription:
        """Registers ``args[-1]`` with the parameters before it; returns the :class:`Subscription`."""
    def __call__(self, *args: Any) -> Any: ...

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

    def __init__(self) -> None: ...
    @property
    def root(self) -> Node:
        """The root of the tree."""
    @property
    def windows(self) -> list[Node]:
        """The windows now in the tree, in stacking order (the first is the bottom)."""
    def node(self, id: int) -> Node:
        """The handle for the node with this id."""
    def create_node(self, class_name: str) -> Node:
        """Makes a node of the class that is in no parent yet; put it in the tree with ``node.insert_into(parent)``."""
    def adopt_node(self, class_name: str, name: str, *, ephemeral: bool = False) -> tuple[Node, bool]:
        """Finds the node of this class and name an earlier generation made, or makes it. Returns the node and whether it was found."""
    def flush(self, on_done: Callable[[int | None, str | None], None]) -> None:
        """Calls ``on_done(frame, error)`` once the compositor has applied everything written so far."""
    def after(self, ms: int, function: Callable[[], None]) -> Subscription:
        """Calls ``function()`` once, ``ms`` milliseconds from now."""
    def every(self, ms: int, function: Callable[[], None]) -> Subscription:
        """Calls ``function()`` every ``ms`` milliseconds."""
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

        ``stdout`` and ``stderr`` are ``inherit`` (the session log), ``discard`` or ``capture`` (kept, up to ``output_limit`` bytes, for
        ``on_exit``). ``timeout`` is in seconds. A program the script's top level starts waits until the generation takes over, so a
        reload that fails starts nothing, and one the next generation starts again is adopted, not doubled.
        """
    def claim_device(self, device: str) -> Subscription:
        """Takes a device from the compositor's own input handling."""
    def grab_input(self, node: Node, scope: str = "both") -> Subscription:
        """Sends the ``pointer``, the ``keyboard`` or ``both`` to the node until the claim is removed."""
    def off(self, function: Callable[..., Any]) -> int:
        """Removes every registration made for ``function``; returns how many were removed. One already removed is skipped."""
    def log(self, message: object, level: str = "info") -> None:
        """Writes a line to the compositor's log, from any thread."""
    def post(self, function: Callable[[], None]) -> None:
        """Runs ``function()`` on the shell thread at its next turn. The one call that is safe from any thread."""
    def request_reload(self) -> None:
        """Asks the host to reload this shell after the current callback returns."""
    def request_exit(self, code: int = 0) -> None:
        """Asks the host to exit the compositor with ``code``."""
    def keep_state(self, key: str, data: bytes, version: int = 0) -> None:
        """Keeps ``data`` for the next generation of this shell (a reload). The store is dropped when the compositor restarts."""
    def kept_state(self, key: str) -> tuple[bytes, int] | None:
        """What an earlier generation kept under ``key`` as ``(data, version)``, or ``None``."""
    @property
    def info(self) -> dict[str, Any]:
        """The generation: ``generation``, ``reload_count``, ``module_path``, ``state_dir``, ``isolate``."""
    @property
    def stats(self) -> dict[str, int]:
        """The core's counters: registrations, callbacks, panics, retired generations, stale calls."""

#: The one object a script uses: ``from gnoblin import gnoblin``.
gnoblin: Gnoblin
