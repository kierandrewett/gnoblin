"""Exceptions of the Gnoblin core: a failed call is an exception, never a return code."""

class GnoblinError(Exception):
    """A call of the Gnoblin core failed. `code` says why (`not_found`, `read_only`, ...) and `target` names the object and member, as in `Client#7.x`; both are None for a failure that has neither."""

    code: str | None
    target: str | None

class RetiredError(GnoblinError):
    """The generation that made this call has been replaced by a reload."""

    code: str | None
    target: str | None

class NodeGoneError(GnoblinError):
    """The node is not in the tree: its window closed, or it was destroyed."""

    code: str | None
    target: str | None

class MemberError(GnoblinError):
    """The class has no such property, method or signal."""

    code: str | None
    target: str | None

class RefusedError(GnoblinError):
    """The compositor refused the request."""

    code: str | None
    target: str | None

class StaleError(GnoblinError):
    """The handle was removed already, or never existed."""

    code: str | None
    target: str | None

class UnsupportedError(GnoblinError):
    """The installed core does not have this function."""

    code: str | None
    target: str | None

class BusyError(GnoblinError):
    """A single-holder resource is taken."""

    code: str | None
    target: str | None

class ThreadError(GnoblinError):
    """The call was made from a thread that is not the shell thread."""

    code: str | None
    target: str | None

__all__: list[str]
