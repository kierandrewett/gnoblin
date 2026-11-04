#!/usr/bin/env python3
"""Compare the Lua API annotations with the functions the runtime installs.

src/config/lua/gnoblin-lua.c lists every installed function in lua_api[]. This script reads that
table and the LuaCATS annotations that scripts/gen-lua-api.py collects, and reports:

  * an installed function without an annotation, and
  * an annotated function of the gnoblin global or of a record class that the runtime does not
    install, unless it is installed by hand (STRUCTURAL below), and
  * a method of a class that is built by hand outside lua_api[] (HAND_BUILT below) that is
    annotated but not listed, listed but not annotated, or listed but not in the C source.

A record class is every class that has an annotated `Class:method`; the annotations are the one
declaration, so a new class needs no entry here.
"""

import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TABLE = ROOT / "src/config/lua/gnoblin-lua.c"

# Classes whose methods are built by hand, outside lua_api[]: each has its own metatable or method
# table in the named C file. `methods` lists what the annotations must declare, in both
# directions; `pattern` finds one method in the file. `complete` is a regular expression that
# finds every method the file installs, for the classes whose table can be scanned.
SRC = ROOT / "src/config/lua"
HAND_BUILT = {
    "Operation": {
        "file": SRC / "gnoblin-lua-operation.c",
        "methods": {"on_complete", "map", "timeout", "cancel"},
        "pattern": r'\{{"{name}",\s*lua_operation_\w+\}}',
        "complete": r'\{"(\w+)",\s*lua_operation_\w+\}',
    },
    "Request": {
        "file": SRC / "gnoblin-lua-request.c",
        "methods": {"answer", "cancel", "claim", "finished", "on"},
        "pattern": r'\{{"{name}",\s*lua_request_\w+\}}',
        "complete": r'\{"(\w+)",\s*lua_request_\w+\}',
    },
    "Grant": {
        "file": SRC / "gnoblin-lua-contexts.c",
        "methods": {"narrow"},
        "pattern": r'g_str_equal\(key, "{name}"\)',
    },
    "TextTarget": {
        "file": SRC / "gnoblin-lua-contexts.c",
        "methods": {"insert_text"},
        "pattern": r'g_str_equal\(field, "{name}"\)',
    },
    "SnapContext": {
        "file": SRC / "gnoblin-lua-variant.c",
        "methods": {"commit"},
        "pattern": r'lua_setfield\(state, -2, "{name}"\)',
    },
}

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


def hand_built_problems(annotated):
    problems = []
    for owner, spec in sorted(HAND_BUILT.items()):
        declared = {path.split(":")[1] for path in annotated if path.startswith(owner + ":")}
        text = spec["file"].read_text()
        for name in sorted(declared - spec["methods"]):
            problems.append(f"annotated but not in HAND_BUILT[{owner}]: {owner}:{name}")
        for name in sorted(spec["methods"] - declared):
            problems.append(f"in HAND_BUILT[{owner}] but not annotated: {owner}:{name}")
        for name in sorted(spec["methods"]):
            if not re.search(spec["pattern"].format(name=re.escape(name)), text):
                problems.append(f"in HAND_BUILT[{owner}] but not in {spec['file'].name}: {owner}:{name}")
        if "complete" in spec:
            for name in sorted(set(re.findall(spec["complete"], text)) - spec["methods"]):
                problems.append(f"installed by {spec['file'].name} but not in HAND_BUILT[{owner}]: {owner}:{name}")
    return problems


def main():
    functions, _classes, _aliases, _raw = load_generator().collect()
    annotated = set(functions)
    installed = installed_paths()
    # Every class with an annotated Class:method is a record class, unless it is built by hand.
    record_classes = {path.split(":")[0] for path in annotated if ":" in path} - set(HAND_BUILT)
    problems = [f"not annotated: {path}" for path in sorted(installed - annotated)]
    for path in sorted(annotated - installed - STRUCTURAL):
        owner = path.split(":")[0] if ":" in path else "gnoblin"
        if owner == "gnoblin" and path.startswith("gnoblin.") or owner in record_classes:
            problems.append(f"annotated but not installed: {path}")
    problems += hand_built_problems(annotated)
    print(f"{len(problems)} mismatches between lua_api[] and the annotations")
    for problem in problems:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
