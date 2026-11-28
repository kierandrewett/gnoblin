"""Type declarations of the ``gnoblin`` package: the classes of the object model, the ``gnoblin`` object, the verbs and the errors.

The tree layer under them is ``gnoblin.host``.
"""

from ._generated import verbs as verbs
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
from .objects.model import *

__version__: str
__all__: list[str]
