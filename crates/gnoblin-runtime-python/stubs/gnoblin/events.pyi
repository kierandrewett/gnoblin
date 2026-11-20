"""The values an event carries, and the handles a registration returns."""

from __future__ import annotations

from dataclasses import dataclass

class Subscription:
    """A registration the core holds for the running generation: an event, a chord, a timer. It lasts until the generation is replaced by a
    reload, or until :meth:`unsubscribe`. Dropping the object does not remove it."""

    def __init__(self, token: int) -> None: ...
    @property
    def active(self) -> bool:
        """Whether :meth:`unsubscribe` has not been called."""
    def unsubscribe(self) -> None:
        """Removes the registration. Removing it twice, or after its generation ended, raises ``StaleError``."""

@dataclass(frozen=True)
class WindowRequest:
    """A request a window's client made that policy decides. It changes nothing until the shell answers it."""

    name: str  #: ``move``, ``resize``, ``maximize``, ``unmaximize``, ``fullscreen``, ``close``, ``menu``, ``activate``, ...
    x: float = 0.0  #: for ``menu``: where it opens
    y: float = 0.0

@dataclass(frozen=True)
class NodeEvent:
    """An input event sent to a node: a key, a pointer press, motion, a wheel turn."""

    kind: str
    x: float
    y: float
    button: int
    key: str
    text: str
    dx: float
    dy: float
    pressed: bool

@dataclass(frozen=True)
class ChordEvent:
    """A key chord the shell claimed was pressed, repeated or released."""

    chord: str
    state: str  #: ``press``, ``release`` or ``repeat``

@dataclass(frozen=True)
class PointerDrag:
    """A pointer chord held and dragged."""

    phase: str  #: ``start``, ``update``, ``end`` or ``cancel``
    x: float
    y: float
    dx: float
    dy: float
    button: int
    modifiers: str
    time: int

@dataclass(frozen=True)
class GestureEvent:
    """A touchpad gesture."""

    kind: str
    fingers: int
    phase: str
    dx: float
    dy: float
    scale: float
    rotation: float

@dataclass(frozen=True)
class PointerButton:
    """A button of a pointing device the shell claimed."""

    button: int
    pressed: bool
    x: float
    y: float
    time: int

@dataclass(frozen=True)
class ProcessExit:
    """How a program the shell started ended."""

    code: int | None
    signal: str | None
    timed_out: bool
    stdout: str
    stderr: str
    stdout_truncated: bool = False
    stderr_truncated: bool = False
    @property
    def success(self) -> bool:
        """Whether it exited normally with status 0."""

class Process:
    """A program the shell started, in a process group of its own."""

    def __init__(self, token: int) -> None: ...
    def signal(self, name: str = "term") -> None:
        """Sends a signal (``term``, ``kill``, ``int``, ``hup``, ``usr1``, ``usr2``, ``stop``, ``cont``) to the process group."""
    def terminate(self) -> None:
        """Asks the program to end."""
    def kill(self) -> None:
        """Ends the program at once."""
