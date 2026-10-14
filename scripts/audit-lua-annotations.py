#!/usr/bin/env python3
"""Compare Lua annotations with the API names registered by gnoblin-lua.c.

The rewrites mirror src/config/lua/gnoblin-lua-install.c. A rule that changes
there must change here too.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

PREFIX_REWRITES = {"workspace.": "workspaces.", "command.": "commands.", "animation.": "animations."}
NAME_REWRITES = {
    "input.select": "input.select_source",
    "privacy.get": "privacy.state",
    "shortcut.capture": "shortcuts.capture",
    "shortcut.bind": "shortcuts.bind",
    "shortcut.unbind": "shortcuts.unbind",
    "shortcut.session.end": "shortcuts.end_session",
    "workspace.switch": "workspaces.activate",
}


def registered_api_paths():
    text = (ROOT / "src/config/lua/gnoblin-lua.c").read_text()
    methods_block = text[text.index("api_methods[] = {") :]
    aliases_block = text[text.index("api_aliases[] = {") :]
    methods_cut = methods_block.index("NULL,")
    NAME_PATTERN = chr(34) + "([a-zA-Z_.]+)" + chr(34)
    methods = re.findall(NAME_PATTERN, methods_block[:methods_cut])
    aliases = re.findall(NAME_PATTERN, aliases_block)
    names = set()
    for name in methods + aliases:
        # Register only Lua functions. These seven names are tables, so their
        # children are annotated without a belong-to entry in the inc file.
        if name in {
            "__gnoblin_array",
            "animations",
            "autostart",
            "callback",
            "rules",
            "shortcuts",
            "sources",
            "workspaces",
        }:
            continue
        if name in NAME_REWRITES:
            names.add(NAME_REWRITES[name])
            continue
        prefix = next((p for p in PREFIX_REWRITES if name.startswith(p)), None)
        if prefix:
            name = PREFIX_REWRITES[prefix] + name[len(prefix) :]
        elif name == "window.match":
            name = "Window:match"
        elif name.startswith("window."):
            name = "Window:" + name[len("window.") :]
        names.add(name)
    return sorted(n for n in names if n)


def annotated_paths():
    out = set()
    inc = (ROOT / "src/config/lua/gnoblin-lua-signatures.inc").read_text()
    functions_start = inc.index("static const ApiFunction api_functions[]")
    for line in inc[functions_start:].splitlines():
        stripped = line.strip()
        if stripped.startswith("{"):
            match = re.match(r'{"([a-zA-Z_.:]+)", ', stripped)
            if match:
                name = match.group(1)
                if not name.startswith("Window:"):
                    name = "gnoblin." + name
                out.add(name)
    return out


def main():
    missing = []
    compar = annotated_paths()
    for path in registered_api_paths():
        prefix = "" if path.startswith("Window:") else "gnoblin."
        if (prefix + path) not in compar:
            missing.append(prefix + path)
    print(f"{len(missing)} registered names without an annotation")
    for path in missing:
        print(f"not annotated: {path}")
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
