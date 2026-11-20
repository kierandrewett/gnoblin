"""Exceptions of the Gnoblin core: a failed call is an exception, never a return code."""

class GnoblinError(Exception):
    """A call of the Gnoblin core failed."""

class RetiredError(GnoblinError):
    """The generation that made this call has been replaced by a reload."""

class NodeGoneError(GnoblinError):
    """The node is not in the tree: its window closed, or it was destroyed."""

class MemberError(GnoblinError):
    """The class has no such property, method or signal."""

class RefusedError(GnoblinError):
    """The compositor refused the request."""

class StaleError(GnoblinError):
    """The handle was removed already, or never existed."""

class UnsupportedError(GnoblinError):
    """The installed core does not have this function."""

class BusyError(GnoblinError):
    """A single-holder resource is taken."""

class ThreadError(GnoblinError):
    """The call was made from a thread that is not the shell thread."""

__all__: list[str]
