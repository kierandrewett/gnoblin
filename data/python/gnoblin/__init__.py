"""Gnoblin shells in Python.

A shell is an ordinary script. Run it with ``gnoblin ./shell.py``; the shell reloads it when it or a module it imports is saved::

    from gnoblin import gnoblin

    @gnoblin.on_client_created
    def place(window):
        window.x, window.y = 0, 0

    @gnoblin.bind("<Super>Return")
    def terminal():
        gnoblin.spawn("foot")

See ``docs/python-shell.md``. The typed classes of the tree (``Window``, ``Seat``, ...) are generated from the schema; the rest of
the package is a thin spelling of the core's C ABI and holds no compositor logic.
"""

from ._base import CLASSES, Method, Node, Property, Signal
from ._generated import classes as _classes
from ._generated.classes import *  # noqa: F401,F403
from .api import Event, Gnoblin, gnoblin
from .errors import (
    BusyError,
    GnoblinError,
    MemberError,
    NodeGoneError,
    RefusedError,
    RetiredError,
    StaleError,
    ThreadError,
    UnsupportedError,
)
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
)

__version__ = "0.2.0"

__all__ = [
    "BusyError",
    "CLASSES",
    "ChordEvent",
    "Event",
    "GestureEvent",
    "Gnoblin",
    "GnoblinError",
    "MemberError",
    "Method",
    "Node",
    "NodeEvent",
    "NodeGoneError",
    "PointerButton",
    "PointerDrag",
    "Process",
    "ProcessExit",
    "Property",
    "RefusedError",
    "RetiredError",
    "Signal",
    "StaleError",
    "Subscription",
    "ThreadError",
    "UnsupportedError",
    "WindowRequest",
    "gnoblin",
    *_classes.__all__,
]
