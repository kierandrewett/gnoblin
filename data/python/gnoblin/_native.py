"""The built-in module the Python runtime provides, imported once under a friendlier failure."""

try:
    import _gnoblin_core as core
except ImportError as error:  # pragma: no cover - only outside a running shell
    raise ImportError(
        "the `gnoblin` package runs inside a Gnoblin shell: the built-in module `_gnoblin_core` is provided by "
        "libgnoblin-runtime-python.so when `gnoblin` loads a .py shell. Start the shell with `gnoblin ./shell.py`."
    ) from error

__all__ = ["core"]
