#!/usr/bin/env python3
"""Compare the Lua API annotations with the functions the runtime installs.

src/config/lua/gnoblin-lua.c lists every installed function in lua_api[]. This script reads that
table and the LuaCATS annotations that scripts/gen-lua-api.py collects, and reports:

  * an installed function without an annotation, and
  * an annotated function of the gnoblin global or of a record class that the runtime does not
    install, unless it is installed by hand (STRUCTURAL below).
"""

import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TABLE = ROOT / "src/config/lua/gnoblin-lua.c"

# Record classes whose methods come from lua_api[] rows named Class:method.
RECORD_CLASSES = {"Window", "Workspace", "AnimationPreview"}

# Annotated functions that install_api, lua_gnoblin_index or another record builds by hand,
# outside lua_api[]. Each one is a table with a metatable, a declaration closure, or a value that
# exists only on a record of its own type.
STRUCTURAL = {
    "gnoblin.configure",  # table whose __call merges settings
    "gnoblin.window_rule",  # declaration closure
    "gnoblin.animation",  # declaration closure
    "gnoblin.focus.policy",  # built on first read by lua_gnoblin_index
    "gnoblin.focus.history",  # built on first read by lua_gnoblin_index
    "gnoblin.settings.override",  # a method of the gnoblin.settings snapshot, added by lua_gnoblin_index
}


def load_generator():
    spec = importlib.util.spec_from_file_location("gen_lua_api", ROOT / "scripts/gen-lua-api.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def installed_paths():
    text = TABLE.read_text()
    block = text[text.index("const LuaApiEntry lua_api[] = {") :]
    block = block[: block.index("{NULL, NULL, NULL}")]
    paths = set()
    for path in re.findall(r'\{"([A-Za-z_.:]+)",', block):
        paths.add(path if ":" in path else "gnoblin." + path)
    return paths


def main():
    functions, _classes, _aliases, _raw = load_generator().collect()
    annotated = set(functions)
    installed = installed_paths()
    problems = [f"not annotated: {path}" for path in sorted(installed - annotated)]
    for path in sorted(annotated - installed - STRUCTURAL):
        owner = path.split(":")[0] if ":" in path else "gnoblin"
        if owner == "gnoblin" and path.startswith("gnoblin.") or owner in RECORD_CLASSES:
            problems.append(f"annotated but not installed: {path}")
    print(f"{len(problems)} mismatches between lua_api[] and the annotations")
    for problem in problems:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
