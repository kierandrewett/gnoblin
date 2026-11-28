"""Type declarations of ``gnoblin.host``: the tree layer under the object model.

The node classes (``Window``, ``Seat``, ``Screen``, ...) come from the generated stubs. The runtime and its tests use this module; a shell is
written on the package root.
"""

from ._base import CLASSES as CLASSES
from ._base import Method as Method
from ._base import Node as Node
from ._base import Property as Property
from ._base import Signal as Signal
from ._generated.classes import *
from .api import Event as Event
from .events import ChordEvent as ChordEvent
from .events import GestureEvent as GestureEvent
from .events import NodeEvent as NodeEvent
from .events import PointerButton as PointerButton
from .events import PointerDrag as PointerDrag
from .events import Process as Process
from .events import ProcessExit as ProcessExit
from .events import Subscription as Subscription
from .events import WindowRequest as WindowRequest

__all__: list[str]
