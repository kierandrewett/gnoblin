#!/usr/bin/env python3
"""Compare the Lua API annotations with the functions the runtime installs.

src/config/lua/gnoblin-lua.c lists every installed function in lua_api[]. This script reads that
table and the LuaCATS annotations that scripts/gen-lua-api.py collects, and reports:

  * an installed function without an annotation, and
  * an annotated function of the gnoblin global or of a record class that the runtime does not
    install, unless it is installed by hand (STRUCTURAL below), and
  * a method of a class that is built by hand outside lua_api[] (HAND_BUILT below) that is
    annotated but not listed, listed but not annotated, or listed but not in the C source, and
  * a record method whose argument schema in lua_api[] (ARGS(...)) takes a different number of
    positional arguments than its annotation declares, and
  * a name in the console's call table (src/cli/gnoblinctl-lua-api.c) that lua_api[] does not
    hold, and a wire operation of lua_api[] on a record that the console builds but that the
    console has no method for.

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


# The console (`gnoblinctl lua`) reads lua_api[] for the names. Its own implementations are in
# the call tables of this file; every other name raises the "no runtime" error. The records the
# console builds have the wire operations below as methods, so each one needs a console method,
# except the ones that only a runtime can run.
CONSOLE = ROOT / "src/cli/gnoblinctl-lua-api.c"
CONSOLE_RECORD_CLASSES = {
    "Window",
    "Workspace",
    "LayerSurface",
    "Output",
    "AnimationPreview",
    "InputSource",
    "Shortcut",
    "Launch",
}
CONSOLE_RUNTIME_ONLY = {
    "Window:present",  # focuses a window of a process that the runtime started
    "Window:set_effect",  # an override ends with the runtime that holds it
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
    for path in re.findall(r'^\s*\{"([A-Za-z_.:]+)",', block, re.M):
        paths.add(path if ":" in path else "gnoblin." + path)
    return paths


def lua_api_entries():
    """Each lua_api[] entry as (path, has_operation, has_function)."""
    text = TABLE.read_text()
    block = text[text.index("const LuaApiEntry lua_api[] = {") :]
    block = block[: block.index("{NULL, NULL, NULL}")]
    entries = []
    for match in re.finditer(r'^\s*\{"([A-Za-z_.:]+)",\s*(NULL|OPERATION\(\w+\)),\s*(NULL|\w+)', block, re.M):
        entries.append((match.group(1), match.group(2) != "NULL", match.group(3) != "NULL"))
    return entries


def argument_schemas():
    """The positional arguments of each record method with an ARGS(...) schema: path -> (required, option)."""
    text = TABLE.read_text()
    block = text[text.index("const LuaApiEntry lua_api[] = {") :]
    block = block[: block.index("{NULL, NULL, NULL}")]
    # clang-format wraps a long entry onto a second line; read it as one.
    block = re.sub(r",\n\s+(ARGS\(|lua_no_arguments)", r", \1", block)
    schemas = {}
    for match in re.finditer(r'^\s*\{"([A-Za-z_.:]+)",[^\n]*?(ARGS\((.*)\)|lua_no_arguments)\},', block, re.M):
        kinds = re.findall(r"LUA_ARG_(\w+)", match.group(3) or "")
        required = sum(1 for kind in kinds if kind not in ("OPTION_BOOLEAN", "CONSTANT_TRUE"))
        # A method whose first argument only picks the operation still takes it positionally.
        schemas[match.group(1)] = (required, "OPTION_BOOLEAN" in kinds)
    return schemas


def schema_problems(functions):
    problems = []
    for path, (required, option) in sorted(argument_schemas().items()):
        declared = functions.get(path)
        if declared is None:
            continue
        params = declared["params"]
        if len(params) != required + (1 if option else 0):
            problems.append(
                f"{path}: the schema in lua_api[] takes {required + (1 if option else 0)} arguments, "
                f"the annotation declares {len(params)}"
            )
        elif option and not params[-1]["optional"]:
            problems.append(f"{path}: the option argument must be optional in the annotation")
    return problems


def console_table(name):
    """The paths in one `static const ... name[] = {...};` table of the console's source."""
    text = CONSOLE.read_text()
    start = text.index(f"{name}[] = {{")
    body = text[start : text.index("};", start)]
    return re.findall(r'\{"([A-Za-z_.:]+)",', body)


def console_problems(structural):
    problems = []
    entries = lua_api_entries()
    functions = {path for path, _op, _fn in entries if ":" not in path}
    methods = {path for path, _op, _fn in entries if ":" in path}
    for path in console_table("console_functions"):
        if path not in functions:
            problems.append(f"console function is not in lua_api[]: {path}")
    for path in console_table("console_extras"):
        if "gnoblin." + path not in structural:
            problems.append(f"console extra is not a structural name: {path}")
    implemented = set(console_table("console_methods"))
    for path in sorted(implemented - methods):
        problems.append(f"console method is not in lua_api[]: {path}")
    for path, has_operation, has_function in entries:
        owner = path.split(":")[0]
        if ":" not in path or owner not in CONSOLE_RECORD_CLASSES:
            continue
        if has_operation and not has_function and path not in implemented:
            if path not in CONSOLE_RUNTIME_ONLY:
                problems.append(f"console has no method for the wire operation {path}")
    return problems


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
    problems += schema_problems(functions)
    problems += hand_built_problems(annotated)
    problems += console_problems(STRUCTURAL)
    print(f"{len(problems)} mismatches between lua_api[], the annotations and the console")
    for problem in problems:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
