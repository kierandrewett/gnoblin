"""Loads the shell script, and the modules it imports, one isolated generation at a time.

The runtime calls :func:`load` to *stage* a generation, :func:`activate` when the host commits it, and :func:`abandon` when the host
throws it away. A reload is the same sequence again.

What makes a generation isolated:

* The script and every module found beside it (under its directory, at any depth) are compiled from their source each time, never
  from a ``.pyc``: an edit that keeps the file's size and lands in the same second as the last one is still seen.
* Before a generation is evaluated, the modules the previous one imported are taken out of ``sys.modules``; the new one imports them
  fresh. If the new one fails, they are put back and nothing of the failed attempt stays, so the running generation, which still holds
  its own module objects, goes on with exactly the code it had, and a ``import`` it makes later finds its own modules.
* ``__main__`` is the generation's script while it runs, as under ``python shell.py``, and is restored after a failure.
* Libraries installed on the system (everything not beside the script) are shared by every generation, as in any Python process: a
  generation's monkey-patches of them are not undone.

The listeners a generation registered are not tracked here. They belong to the generation's context in the core, which removes them
when the generation retires; a script that is evaluated again registers exactly what the new text registers.
"""

from __future__ import annotations

import builtins
import importlib
import importlib.abc
import importlib.machinery
import linecache
import os
import sys
import traceback
import types
from typing import Any, Callable

#: A stamp of a file as the loader read it: path, modification time in nanoseconds, size, inode. A file that is not there is
#: ``(path, -1, 0, 0)``.
Stamp = tuple[str, int, int, int]

_RESERVED = ("gnoblin", "_gnoblin_core")


def _stamp(path: str) -> Stamp:
    try:
        status = os.stat(path)
    except OSError:
        return (path, -1, 0, 0)
    return (path, status.st_mtime_ns, status.st_size, status.st_ino)


def _inside(path: str, root: str) -> bool:
    path = os.path.realpath(path)
    return path == root or path.startswith(root + os.sep)


class _FreshLoader(importlib.machinery.SourceFileLoader):
    """Compiles a module from its source on every import and records what the file looked like when it was read."""

    def __init__(self, fullname: str, path: str, record: Callable[[str], None]) -> None:
        super().__init__(fullname, path)
        self._record = record

    def get_code(self, fullname: str) -> types.CodeType:
        path = self.get_filename(fullname)
        self._record(path)  # before the read: an edit during it makes the stamp older than the file, so it is seen
        return self.source_to_code(self.get_data(path), path)


class _Finder(importlib.abc.MetaPathFinder):
    """Finds the modules beside the script, and gives them a loader that never reads a ``.pyc``."""

    def __init__(self, root: str) -> None:
        self.root = root
        self.stamps: dict[str, Stamp] = {}

    def record(self, path: str) -> None:
        self.stamps[path] = _stamp(path)

    def find_spec(self, fullname: str, path: Any = None, target: Any = None) -> importlib.machinery.ModuleSpec | None:
        if fullname.partition(".")[0] in _RESERVED:
            return None
        if path is None:
            search = [self.root]
        else:
            search = [entry for entry in path if _inside(entry, self.root)]
            if not search:
                return None
        spec = importlib.machinery.PathFinder.find_spec(fullname, search, target)
        if spec is not None and isinstance(spec.loader, importlib.machinery.SourceFileLoader):
            spec.loader = _FreshLoader(spec.loader.name, spec.loader.path, self.record)
        return spec


class _Stage:
    """What a generation changed in the process, so a failed one can be undone."""

    def __init__(self) -> None:
        self.previous_modules: dict[str, types.ModuleType] = {}
        self.previous_main: types.ModuleType | None = None
        self.previous_finder: _Finder | None = None
        self.previous_stamps: dict[str, Stamp] = {}
        self.finder: _Finder | None = None
        self.main: types.ModuleType | None = None
        self.path = ""
        self.root = ""


_serving: _Stage | None = None
_staged: _Stage | None = None
_original_main: types.ModuleType | None = None


def _user_modules() -> dict[str, types.ModuleType]:
    return {
        name: module
        for name, module in sys.modules.items()
        if isinstance(getattr(getattr(module, "__spec__", None), "loader", None), _FreshLoader)
    }


def _install(finder: _Finder) -> None:
    sys.meta_path[:] = [entry for entry in sys.meta_path if not isinstance(entry, _Finder)]
    index = next(
        (i for i, entry in enumerate(sys.meta_path) if entry is importlib.machinery.PathFinder), len(sys.meta_path)
    )
    sys.meta_path.insert(index, finder)


def _describe(error: BaseException, root: str) -> tuple[str, str]:
    """The one-line reason (``FILE:LINE: Type: message``) and the full traceback of a failed evaluation."""
    linecache.clearcache()
    detail = "".join(traceback.format_exception(error))
    if isinstance(error, SyntaxError) and error.filename:
        place = f"{error.filename}:{error.lineno or 0}"
        message = error.msg
    else:
        frames = traceback.extract_tb(error.__traceback__)
        user = [frame for frame in frames if _inside(frame.filename, root)] or frames
        place = f"{user[-1].filename}:{user[-1].lineno}" if user else root
        message = str(error)
    first = message.splitlines()[0] if message else ""
    return f"{place}: {type(error).__name__}: {first}".rstrip(": "), detail


def _watch(stage: _Stage, *extra: dict[str, Stamp]) -> list[Stamp]:
    stamps: dict[str, Stamp] = {}
    for source in extra:
        stamps.update(source)
    if stage.finder is not None:
        stamps.update(stage.finder.stamps)
    return list(stamps.values())


def load(script: str) -> tuple[bool, str, str, list[Stamp]]:
    """Stages a generation: evaluates ``script`` in a fresh namespace beside the one that serves.

    Returns ``(ok, reason, detail, watch)``: on success ``reason`` and ``detail`` are empty; on failure they are the one-line reason and
    the traceback, and nothing of the attempt stays. ``watch`` is the files to look at for the next edit: those of the generation
    that would serve if the host commits this one, or, after a failure, those of the one that serves plus the attempt's (and the
    script's directory, so a module that was missing and is then created is seen).
    """
    global _staged, _original_main
    if _staged is not None:
        abandon()
    path = os.path.realpath(script)
    root = os.path.dirname(path)
    stage = _Stage()
    stage.path, stage.root = path, root
    stage.previous_modules = _user_modules()
    stage.previous_main = sys.modules.get("__main__")
    if _original_main is None:
        _original_main = stage.previous_main
    stage.previous_stamps = dict(_serving.finder.stamps) if _serving is not None and _serving.finder is not None else {}
    stage.previous_finder = _serving.finder if _serving is not None else None
    for name in stage.previous_modules:
        sys.modules.pop(name, None)
    importlib.invalidate_caches()
    stage.finder = _Finder(root)
    _install(stage.finder)
    if root not in sys.path:
        sys.path.insert(0, root)
    try:
        stage.finder.record(path)
        with open(path, "rb") as handle:
            source = handle.read()
        code = compile(source, path, "exec", dont_inherit=True)
        main = types.ModuleType("__main__")
        main.__file__ = path
        main.__builtins__ = builtins
        stage.main = main
        sys.modules["__main__"] = main
        sys.argv = [path]
        exec(code, main.__dict__)
    except BaseException as error:  # noqa: BLE001 - a script may raise anything, including SystemExit, and none may end the host
        reason, detail = _describe(error, root)
        watch = _watch(stage, stage.previous_stamps)
        watch.append(_stamp(root))
        _undo(stage)
        return False, reason, detail, watch
    _staged = stage
    return True, "", "", _watch(stage)


def _undo(stage: _Stage) -> None:
    """Puts the process as it was before ``stage`` was evaluated."""
    global _staged
    for name in _user_modules():
        sys.modules.pop(name, None)
    sys.modules.update(stage.previous_modules)
    if stage.previous_main is not None:
        sys.modules["__main__"] = stage.previous_main
    if stage.previous_finder is not None:
        _install(stage.previous_finder)
    else:
        sys.meta_path[:] = [entry for entry in sys.meta_path if not isinstance(entry, _Finder)]
    if _staged is stage:
        _staged = None


def activate() -> None:
    """Makes the staged generation the one that serves."""
    global _serving, _staged
    if _staged is None:
        return
    _serving, _staged = _staged, None


def abandon() -> None:
    """Throws the staged generation away: its modules leave, the serving generation's come back."""
    if _staged is not None:
        _undo(_staged)


def end() -> None:
    """Ends the shell: no module of the script stays, ``__main__`` is as it was before the first generation."""
    global _serving, _staged
    abandon()
    for name in _user_modules():
        sys.modules.pop(name, None)
    sys.meta_path[:] = [entry for entry in sys.meta_path if not isinstance(entry, _Finder)]
    if _serving is not None and _original_main is not None:
        sys.modules["__main__"] = _original_main
    _serving = None
