"""Type declarations of the ``gnoblin`` package. The classes of the tree come from the generated stubs."""

from ._base import CLASSES as CLASSES
from ._base import Method as Method
from ._base import Node as Node
from ._base import Property as Property
from ._base import Signal as Signal
from ._generated.classes import *
from .api import Event as Event
from .api import Gnoblin as Gnoblin
from .api import gnoblin as gnoblin
from .errors import BusyError as BusyError
from .errors import GnoblinError as GnoblinError
from .errors import MemberError as MemberError
from .errors import NodeGoneError as NodeGoneError
from .errors import RefusedError as RefusedError
from .errors import RetiredError as RetiredError
from .errors import StaleError as StaleError
from .errors import ThreadError as ThreadError
from .errors import UnsupportedError as UnsupportedError
from .events import ChordEvent as ChordEvent
from .events import GestureEvent as GestureEvent
from .events import NodeEvent as NodeEvent
from .events import PointerButton as PointerButton
from .events import PointerDrag as PointerDrag
from .events import Process as Process
from .events import ProcessExit as ProcessExit
from .events import Subscription as Subscription
from .events import WindowRequest as WindowRequest

__version__: str
__all__: list[str]
