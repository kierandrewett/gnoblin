"""``gnoblin.std``: the desktop object model above the core (workspaces, layouts, bindings).

It is not part of the core, and the Python binding of it is not written yet: it will be generated from the tables of
``libgnoblin-std.so.1`` (``gnoblin-std-abi``). Until then this module refuses the import, with the text every runtime gives
when std is missing, so a shell that imports it fails at the import and says why.
"""

from __future__ import annotations

import os

LIBRARY = "libgnoblin-std.so.1"


def _searched() -> list[str]:
    """Where the library would be: beside the core library this process mapped, then the system's library directories."""
    found: list[str] = []
    try:
        with open("/proc/self/maps", encoding="utf-8") as maps:
            for line in maps:
                path = line.rstrip("\n").rpartition(" ")[2]
                if os.path.basename(path).startswith("libgnoblin-core.so"):
                    found.append(os.path.dirname(path))
    except OSError:
        pass
    found += [d for d in os.environ.get("LD_LIBRARY_PATH", "").split(":") if d]
    found += ["/usr/lib64", "/usr/lib", "/usr/local/lib", "/lib64", "/lib"]
    return list(dict.fromkeys(found))


_directories = _searched()
if any(os.path.exists(os.path.join(directory, LIBRARY)) for directory in _directories):
    raise ImportError(
        f"gnoblin.std is not available in this release: {LIBRARY} is installed, but its Python binding has not been "
        "generated yet"
    )
raise ImportError(f"gnoblin-std is not installed (looked for {LIBRARY} in {', '.join(_directories)})")
