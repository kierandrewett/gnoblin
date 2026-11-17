"""What the runtime does once, when the interpreter starts: routes Python's own reporting into the core's log.

An exception nobody caught, a warning, a ``logging`` record and a thread's traceback all end in the compositor's log with the
component ``python``, so a shell is debugged where every other part of the session is.
"""

from __future__ import annotations

import logging
import sys
import threading
import traceback
import warnings

from ._native import core

_LEVELS = {
    logging.CRITICAL: "error",
    logging.ERROR: "error",
    logging.WARNING: "warn",
    logging.INFO: "info",
    logging.DEBUG: "debug",
}


class _CoreHandler(logging.Handler):
    """A ``logging`` handler that writes to the core's log."""

    def emit(self, record: logging.LogRecord) -> None:
        try:
            level = _LEVELS.get(record.levelno - record.levelno % 10, "info")
            core.log(level, f"{record.name}: {self.format(record)}")
        except Exception:  # noqa: BLE001 - logging must never raise into the shell
            self.handleError(record)


def _excepthook(kind: type[BaseException], value: BaseException, trace: object) -> None:
    core.log("error", "python: uncaught exception\n" + "".join(traceback.format_exception(kind, value, trace)))  # type: ignore[arg-type]


def _thread_hook(args: threading.ExceptHookArgs) -> None:
    core.log(
        "error",
        f"python: thread {getattr(args.thread, 'name', '?')} raised\n"
        + "".join(traceback.format_exception(args.exc_type, args.exc_value, args.exc_traceback)),
    )


def _showwarning(message, category, filename, lineno, file=None, line=None):  # type: ignore[no-untyped-def]
    core.log("warn", warnings.formatwarning(message, category, filename, lineno, line).rstrip())


def install() -> None:
    """Routes the interpreter's reporting to the core's log. Called once, by the runtime, before any shell runs."""
    sys.excepthook = _excepthook
    threading.excepthook = _thread_hook
    warnings.showwarning = _showwarning
    handler = _CoreHandler()
    handler.setFormatter(logging.Formatter("%(message)s"))
    root = logging.getLogger()
    root.handlers[:] = [handler]
    root.setLevel(logging.INFO)
