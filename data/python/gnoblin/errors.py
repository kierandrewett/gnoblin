"""Exceptions of the Gnoblin core: a failed call is an exception, never a return code."""

from ._native import core

GnoblinError = core.GnoblinError
RetiredError = core.RetiredError
NodeGoneError = core.NodeGoneError
MemberError = core.MemberError
RefusedError = core.RefusedError
StaleError = core.StaleError
UnsupportedError = core.UnsupportedError
BusyError = core.BusyError
ThreadError = core.ThreadError

__all__ = [
    "BusyError",
    "GnoblinError",
    "MemberError",
    "NodeGoneError",
    "RefusedError",
    "RetiredError",
    "StaleError",
    "ThreadError",
    "UnsupportedError",
]
