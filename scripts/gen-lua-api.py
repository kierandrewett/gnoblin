#!/usr/bin/env python3
"""Generate the Lua API type definitions from annotations in the C sources.

Annotations are LuaCATS (the format Lua Language Server reads) inside C comments that open with
"/*** lua" and close with "*/". They sit beside the code that implements or registers the
function. From them this script writes:

  src/config/lua/gnoblin-lua-signatures.inc  the table the console reads for hints and completion
  docs/public/gnoblin.d.lua                   a typedef file for editors using Lua Language Server

Run it without arguments to regenerate both files, or with --check to fail when they are stale.
"""

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCES = ROOT / "src"
INC = ROOT / "src/config/lua/gnoblin-lua-signatures.inc"
TYPES = ROOT / "docs/public/gnoblin.d.lua"

BUILTIN = {
    "any",
    "boolean",
    "false",
    "fun",
    "function",
    "integer",
    "nil",
    "number",
    "string",
    "table",
    "true",
    "userdata",
}
BLOCK = re.compile(r"/\*\*\* lua\n(.*?)\*/", re.S)
FUNCTION = re.compile(r"^function\s+([A-Za-z_][\w.:]*)\s*\((.*?)\)\s*end\s*$")
NAME = re.compile(r"^([A-Za-z_]\w*)(\?)?$")


class Error(Exception):
    pass


def read_type(text):
    """Split the leading type off the rest of a LuaCATS line. A type may hold spaces inside
    braces, angle brackets, parentheses and quotes."""
    text = text.lstrip()
    depth = 0
    quote = ""
    for index, char in enumerate(text):
        if quote:
            if char == quote:
                quote = ""
        elif char in "\"'":
            quote = char
        elif char in "{<(":
            depth += 1
        elif char in "}>)":
            depth -= 1
        elif char == " " and depth == 0:
            following = text[index:].lstrip()[:1]
            if text[index - 1] in ":|" or following == "|":
                continue  # a union or a fun() return type continues past the space
            return text[:index], text[index:].strip()
    return text, ""


def parse_name(text):
    word, _, rest = text.strip().partition(" ")
    match = NAME.match(word)
    if not match:
        raise Error(f"bad name {word!r}")
    return match.group(1), bool(match.group(2)), rest.strip()


def join_continuations(body):
    """A long annotation may wrap onto lines that lack the leading ---; fold them into the line
    above so the output stays valid Lua."""
    lines = []
    for line in body.split("\n"):
        stripped = line.strip()
        if lines and stripped and not stripped.startswith("---") and not FUNCTION.match(stripped):
            if lines[-1].strip().startswith("---"):
                lines[-1] = lines[-1].rstrip() + " " + stripped
                continue
        lines.append(line)
    return "\n".join(lines)


def parse_block(body, where, functions, classes, aliases, raw):
    body = join_continuations(body)
    documentation = []
    params = []
    returns = []
    current = None
    for number, line in enumerate(body.split("\n"), 1):
        line = line.strip()
        here = f"{where}:{number}"
        if not line:
            current = None
            continue
        if not line.startswith("---"):
            match = FUNCTION.match(line)
            if not match:
                raise Error(f"{here}: expected an annotation or a function stub, got {line!r}")
            path, names = match.group(1), [n.strip() for n in match.group(2).split(",") if n.strip()]
            declared = {p["name"] for p in params}
            for name in names:
                if name != "..." and name not in declared:
                    raise Error(f"{here}: {path} has parameter {name} without an @param line")
            for param in params:
                if param["name"] not in names:
                    raise Error(f"{here}: {path} documents @param {param['name']} that the stub lacks")
            order = {n: i for i, n in enumerate(names)}
            params.sort(key=lambda p: order[p["name"]])
            if path in functions:
                raise Error(f"{here}: {path} is annotated twice")
            functions[path] = {
                "summary": " ".join(documentation),
                "params": params,
                "returns": returns,
                "where": here,
            }
            documentation, params, returns, current = [], [], [], None
            continue
        text = line[3:].strip()
        if not text.startswith("@"):
            documentation.append(text)
            continue
        tag, _, rest = text[1:].partition(" ")
        if tag == "class":
            head = rest.split(":")[0].strip()
            name = re.match(r"[\w.]+", head).group(0)
            generics = re.findall(r"[A-Za-z_]\w*", head[len(name) :])
            if name in classes:
                raise Error(f"{here}: class {name} is defined twice")
            current = {
                "summary": " ".join(documentation),
                "fields": [],
                "generics": generics,
                "where": here,
            }
            classes[name] = current
            documentation = []
        elif tag == "field":
            if current is None:
                raise Error(f"{here}: @field outside a @class")
            name, optional, rest = parse_name(rest)
            kind, description = read_type(rest)
            if kind.endswith("?"):
                optional, kind = True, kind[:-1]
            current["fields"].append({"name": name, "type": kind, "optional": optional, "description": description})
        elif tag == "alias":
            name, _, kind = rest.strip().partition(" ")
            aliases[name] = kind.strip()
            documentation = []
        elif tag == "param":
            name, optional, rest = parse_name(rest)
            kind, description = read_type(rest)
            if kind.endswith("?"):
                optional, kind = True, kind[:-1]
            params.append({"name": name, "type": kind, "optional": optional, "description": description})
        elif tag == "return":
            kind, description = read_type(rest)
            returns.append({"type": kind, "description": description})
        else:
            raise Error(f"{here}: unsupported tag @{tag}")
    if documentation or params or returns:
        raise Error(f"{where}: annotation block ends without a function stub")
    raw.append(body.strip("\n"))


def referenced_types(kind):
    kind = re.sub(r'"[^"]*"|\'[^\']*\'', "", kind)
    kind = re.sub(r"[A-Za-z_]\w*\??\s*:", "", kind)  # table keys and fun() parameter names
    return set(re.findall(r"[A-Za-z_][\w.]*", kind))


def collect():
    functions, classes, aliases, raw = {}, {}, {}, []
    for path in sorted(p for p in SOURCES.rglob("*") if p.suffix in (".c", ".h")):
        text = path.read_text()
        for match in BLOCK.finditer(text):
            line = text.count("\n", 0, match.start()) + 2
            parse_block(
                match.group(1),
                f"{path.relative_to(ROOT)}:{line}",
                functions,
                classes,
                aliases,
                raw,
            )
    known = BUILTIN | set(classes) | set(aliases)
    for path, function in functions.items():
        used = set()
        for item in function["params"] + function["returns"]:
            used |= referenced_types(item["type"])
        for name in sorted(used - known):
            raise Error(f"{function['where']}: {path} uses unknown type {name}")
    for name, cls in classes.items():
        for field in cls["fields"]:
            for used in sorted(referenced_types(field["type"]) - known - set(cls["generics"])):
                raise Error(f"{cls['where']}: {name}.{field['name']} uses unknown type {used}")
    return functions, classes, aliases, raw


def c_string(text):
    text = text.replace("\\", "\\\\").replace('"', '\\"')
    return '"' + text + '"'


def emit_inc(all_functions, classes):
    functions = {p[len("gnoblin.") :]: f for p, f in all_functions.items() if p.startswith("gnoblin.")}
    out = [
        "/* Generated by scripts/gen-lua-api.py from the LuaCATS annotations in src/. Do not edit. */",
        "",
    ]
    fields_of = {}
    for name, cls in sorted(classes.items()):
        label = "api_fields_" + re.sub(r"\W", "_", name)
        fields_of[name] = label
        if cls["fields"]:
            out.append(f"static const ApiMember {label}[] = {{")
            for field in cls["fields"]:
                out.append(
                    f"    {{{c_string(field['name'])}, {c_string(field['type'])}, "
                    f"{'TRUE' if field['optional'] else 'FALSE'}, {c_string(field['description'])}}},"
                )
            out.append("};")
    out.append("")
    out.append("static const ApiClass api_classes[] = {")
    for name, cls in sorted(classes.items()):
        label = fields_of[name] if cls["fields"] else "NULL"
        out.append(f"    {{{c_string(name)}, {c_string(cls['summary'])}, {label}, {len(cls['fields'])}}},")
    out.append("    {NULL, NULL, NULL, 0},")
    out.append("};")
    out.append("")
    for index, (path, function) in enumerate(sorted(functions.items())):
        if function["params"]:
            out.append(f"static const ApiMember api_params_{index}[] = {{")
            for param in function["params"]:
                out.append(
                    f"    {{{c_string(param['name'])}, {c_string(param['type'])}, "
                    f"{'TRUE' if param['optional'] else 'FALSE'}, {c_string(param['description'])}}},"
                )
            out.append("};")
    out.append("")
    out.append("static const ApiFunction api_functions[] = {")
    for index, (path, function) in enumerate(sorted(functions.items())):
        params = f"api_params_{index}" if function["params"] else "NULL"
        returns = " | ".join(r["type"] for r in function["returns"])
        out.append(
            f"    {{{c_string(path)}, {c_string(function['summary'])}, {params}, "
            f"{len(function['params'])}, {c_string(returns)}}},"
        )
    out.append("    {NULL, NULL, NULL, 0, NULL},")
    out.append("};")
    return "\n".join(out) + "\n"


LUA_KEYWORDS = {
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
}  # fmt: skip


def lua_stub(line):
    """A function named like a Lua keyword, such as launches.end, cannot be declared with the
    function statement; declare it by assignment instead."""
    match = FUNCTION.match(line.strip())
    if not match or ":" in match.group(1):
        return line
    parts = match.group(1).split(".")
    if not any(part in LUA_KEYWORDS for part in parts):
        return line
    target = parts[0] + "".join(f'["{p}"]' if p in LUA_KEYWORDS else f".{p}" for p in parts[1:])
    return f"{target} = function({match.group(2)}) end"


def emit_types(functions, raw):
    namespaces = set()
    for path in functions:
        parts = path.split(".")
        for end in range(2, len(parts)):
            namespaces.add(".".join(parts[:end]))
    out = [
        "---@meta",
        "-- Generated by scripts/gen-lua-api.py from the annotations in the Gnoblin sources.",
        "-- Do not edit. Point Lua Language Server at this file with workspace.library.",
        "",
        "---@class gnoblin",
        "gnoblin = {}",
    ]
    for name in sorted(namespaces - {"gnoblin"}):
        out.append(f"{name} = {{}}")
    out.append("")
    for body in raw:
        out.append("\n".join(lua_stub(line) for line in body.split("\n")))
        out.append("")
    return "\n".join(out).rstrip("\n") + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--check", action="store_true", help="fail when the generated files are stale")
    parser.add_argument("--validate", action="store_true", help="parse the annotations and write nothing")
    args = parser.parse_args()
    try:
        functions, classes, _aliases, raw = collect()
    except Error as error:
        print(f"gen-lua-api: {error}", file=sys.stderr)
        return 1
    if args.validate:
        print(f"{len(functions)} functions, {len(classes)} classes: ok")
        return 0
    outputs = {INC: emit_inc(functions, classes), TYPES: emit_types(functions, raw)}
    stale = [p for p, text in outputs.items() if not p.exists() or p.read_text() != text]
    if args.check:
        for path in stale:
            print(f"gen-lua-api: {path.relative_to(ROOT)} is stale; run scripts/gen-lua-api.py", file=sys.stderr)
        return 1 if stale else 0
    for path in stale:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(outputs[path])
        print(f"wrote {path.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
