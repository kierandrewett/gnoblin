#!/usr/bin/env python3
"""Extract the compositor's wire and shell contracts into tests/parity/contracts/.

The parity harness compares a Rust candidate with the C reference. That comparison is only worth
anything when both sides are held to the same list of names, so the list is data on disk, taken
from the C sources that own it, rather than prose that drifts. Nothing here runs the compositor:
every entry is read out of a source line, and each entry records the file and line it came from.
An entry that cannot be read out of a source line is a bug in this script, not something to guess,
so an unparsable construct raises instead of being skipped.

Usage:
    python3 scripts/contract-inventory.py           # rewrite tests/parity/contracts/
    python3 scripts/contract-inventory.py --check   # exit 1 when the checked-in output is stale
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "tests/parity/contracts"
README = OUT / "README.md"

OPERATIONS_HEADER = "src/control/gnoblin-api-operations.h"
NATIVE_CONTROL_HEADER = "src/control/gnoblin-native-control.h"
NATIVE_CONTROL_SOURCE = "src/control/gnoblin-native-control.c"
EVENT_SCAN_DIR = "src"
LUA_ENTRIES = "src/config/lua/gnoblin-lua.c"
LUA_CONFIGURE_KEYS = "src/config/lua/gnoblin-lua-input.c"
OVERRIDES_HEADER = "src/control/gnoblin-overrides.h"
CLI_TABLE = "src/cli/gnoblinctl-cli.c"

COUNT_START = "<!-- contract-counts:start -->"
COUNT_END = "<!-- contract-counts:end -->"

STRING = re.compile(r'"((?:[^"\\]|\\.)*)"')


class InventoryError(RuntimeError):
    """A source construct that this script cannot read. Never skip these silently."""


def read(relative: str) -> str:
    path = ROOT / relative
    if not path.is_file():
        raise InventoryError(f"{relative}: missing source file")
    return path.read_text(encoding="utf-8")


def line_at(text: str, index: int) -> int:
    return text.count("\n", 0, index) + 1


def strip_comments(text: str) -> str:
    """Blank out comments so brace and comma scanning only ever sees code.

    Columns and line numbers are preserved, so an offset in the stripped text is the same offset
    in the file the reader has open.
    """
    out: list[str] = []
    index = 0
    while index < len(text):
        char = text[index]
        if char == '"':
            out.append(char)
            index += 1
            while index < len(text):
                out.append(text[index])
                if text[index] == "\\" and index + 1 < len(text):
                    out.append(text[index + 1])
                    index += 2
                    continue
                index += 1
                if text[index - 1] == '"':
                    break
            continue
        if text.startswith("/*", index):
            end = text.find("*/", index + 2)
            end = len(text) if end == -1 else end + 2
            out.append("".join("\n" if c == "\n" else " " for c in text[index:end]))
            index = end
            continue
        if text.startswith("//", index):
            end = text.find("\n", index)
            end = len(text) if end == -1 else end
            out.append(" " * (end - index))
            index = end
            continue
        out.append(char)
        index += 1
    return "".join(out)


def split_top_level(text: str) -> list[str]:
    """Split a C initializer fragment on its own commas, ignoring nested braces and parentheses."""
    fields: list[str] = []
    depth = 0
    current: list[str] = []
    index = 0
    while index < len(text):
        char = text[index]
        if char == '"':
            current.append(char)
            index += 1
            while index < len(text):
                current.append(text[index])
                escape = text[index] == "\\" and index + 1 < len(text)
                if escape:
                    current.append(text[index + 1])
                    index += 2
                    continue
                index += 1
                if text[index - 1] == '"':
                    break
            continue
        if char in "({[":
            depth += 1
        elif char in ")}]":
            depth -= 1
        if char == "," and depth == 0:
            fields.append("".join(current).strip())
            current = []
            index += 1
            continue
        current.append(char)
        index += 1
    tail = "".join(current).strip()
    if tail:
        fields.append(tail)
    return fields


def matching_group(text: str, index: int, opened: str, closed: str) -> int:
    """The offset of the bracket that closes the one at `index`, in comment-free text."""
    depth = 0
    while index < len(text):
        char = text[index]
        if char == '"':
            index += 1
            while index < len(text):
                if text[index] == "\\" and index + 1 < len(text):
                    index += 2
                    continue
                index += 1
                if text[index - 1] == '"':
                    break
            continue
        if char == opened:
            depth += 1
        elif char == closed:
            depth -= 1
            if depth == 0:
                return index
        index += 1
    raise InventoryError(f"unbalanced {opened!r} at offset {index}")


def block_after(text: str, anchor: str) -> tuple[int, int]:
    """Return the span of the brace block that follows `anchor` (the anchors end at their `{`)."""
    start = text.find(anchor)
    if start == -1:
        raise InventoryError(f"anchor not found: {anchor!r}")
    open_brace = text.find("{", start)
    if open_brace == -1:
        raise InventoryError(f"no initializer after anchor: {anchor!r}")
    return open_brace, matching_group(text, open_brace, "{", "}")


def brace_entries(text: str, anchor: str) -> list[tuple[str, int]]:
    """Split the initializer that follows `anchor` into its `{...}` members, with line numbers."""
    open_brace, close_brace = block_after(text, anchor)
    inner = text[open_brace + 1 : close_brace]
    base_line = line_at(text, open_brace)
    entries: list[tuple[str, int]] = []
    depth = 0
    newlines = 0
    current: list[str] = []
    entry_line = base_line
    for char in inner:
        if char == "\n":
            newlines += 1
            if depth > 0:
                current.append(char)
            continue
        if char == "{":
            if depth == 0:
                entry_line = base_line + newlines
                current = []
            depth += 1
            current.append(char)
            continue
        if char == "}":
            depth -= 1
            current.append(char)
            if depth == 0:
                entries.append(("".join(current), entry_line))
                current = []
            continue
        if depth > 0:
            current.append(char)
    return entries


def string_array(text: str, anchor: str, source: str) -> list[dict[str, object]]:
    """Read a `const char* const name[] = {...}` initializer into name/line pairs."""
    open_brace, close_brace = block_after(text, anchor)
    entries: list[dict[str, object]] = []
    for match in STRING.finditer(text, open_brace, close_brace):
        entries.append({"value": match.group(1), "source": source, "line": line_at(text, match.start())})
    if not entries:
        raise InventoryError(f"{source}: {anchor!r} holds no string entries")
    return entries


def string_literals(fragment: str) -> str:
    """Join the adjacent string literals of one C expression."""
    return "".join(match.group(1) for match in STRING.finditer(fragment))


# --------------------------------------------------------------------------------------------------
# Wire operations
# --------------------------------------------------------------------------------------------------


def macro_rows(text: str, name: str) -> list[tuple[str, int]]:
    """Read the `X(...)` rows of a continuation-line macro, keeping each row's first line."""
    marker = f"#define {name}(X)"
    start = text.find(marker)
    if start == -1:
        raise InventoryError(f"macro not found: {name}")
    position = text.find("\n", start) + 1
    body_start = position
    while position < len(text):
        end = text.find("\n", position)
        end = len(text) if end == -1 else end
        continues = text[position:end].rstrip().endswith("\\")
        position = end + 1
        if not continues:
            break
    body = text[body_start:position]
    rows: list[tuple[str, int]] = []
    residue = []
    consumed = 0
    for call in re.finditer(r"X\(", body):
        if call.start() < consumed:
            continue
        row_line = line_at(text, body_start + call.start())
        close = matching_group(body, call.end() - 1, "(", ")")
        residue.append(body[consumed : call.start()])
        rows.append((body[call.end() : close], row_line))
        consumed = close + 1
    residue.append(body[consumed:])
    extra = "".join(residue).replace("\\", "").strip()
    if extra:
        raise InventoryError(f"{name}: text outside a row: {extra[:80]!r}")
    if not rows:
        raise InventoryError(f"{name}: no rows")
    return rows


def operation_rows() -> tuple[list[dict[str, object]], dict[str, object]]:
    text = strip_comments(read(OPERATIONS_HEADER))
    messages = {
        match.group(1): match.group(2)
        for match in re.finditer(r'^#define\s+(MSG_[A-Z0-9_]+)\s+"((?:[^"\\]|\\.)*)"', text, re.MULTILINE)
    }
    rows = macro_rows(text, "GNOBLIN_API_OPERATIONS")
    operations: list[dict[str, object]] = []
    for index, (row, line) in enumerate(rows):
        fields = split_top_level(row)
        if len(fields) != 7:
            raise InventoryError(f"{OPERATIONS_HEADER}:{line}: expected 7 fields, found {len(fields)}")
        identifier, quoted, gate, message, cli_minor, flags, dispatch = fields
        name_match = STRING.fullmatch(quoted.strip())
        if not name_match:
            raise InventoryError(f"{OPERATIONS_HEADER}:{line}: operation name is not a literal")
        name = name_match.group(1)
        flag_names = sorted(
            flag.strip().removeprefix("GN_OP_") for flag in flags.split("|") if flag.strip() and flag.strip() != "0"
        )
        gate_message = None
        if message != "NULL":
            if message not in messages:
                raise InventoryError(f"{OPERATIONS_HEADER}:{line}: unknown gate message {message}")
            gate_message = messages[message]
        operations.append(
            {
                "name": name,
                "id": identifier.strip(),
                "table_index": index,
                "is_family": name.endswith("."),
                "gate_minor": int(gate),
                "gate_message": gate_message,
                "cli_minor": int(cli_minor),
                "flags": flag_names,
                "hello": "HELLO" in flag_names,
                "dispatch": dispatch,
                "source": OPERATIONS_HEADER,
                "line": line,
            }
        )
    return operations, api_version()


def api_version() -> dict[str, object]:
    text = read(NATIVE_CONTROL_HEADER)
    major = re.search(r"#define GNOBLIN_NATIVE_CONTROL_API_MAJOR (\d+)", text)
    minor = re.search(r"#define GNOBLIN_NATIVE_CONTROL_API_MINOR (\d+)", text)
    if not major or not minor:
        raise InventoryError(f"{NATIVE_CONTROL_HEADER}: native control API version is missing")
    return {
        "major": int(major.group(1)),
        "minor": int(minor.group(1)),
        "source": NATIVE_CONTROL_HEADER,
        "line": line_at(text, minor.start()),
    }


def resolve_families(operations: list[dict[str, object]]) -> None:
    """Record the family row and the gate and gnoblinctl minor that a client really meets.

    gnoblin-api-operations.h falls back to the family row when a row declares no gate
    (gnoblin_api_operation_gate_error) or no gnoblinctl minor (gnoblin_api_operation_cli_minor,
    which defaults to 8). The parity harness needs the values a client sees, so both the declared
    and the resolved values are recorded.
    """
    families = [operation for operation in operations if operation["is_family"]]
    for operation in operations:
        name = str(operation["name"])
        family = next((item for item in families if name.startswith(str(item["name"]))), None)
        operation["family"] = family["id"] if family else None
        gate = int(operation["gate_minor"]) or (int(family["gate_minor"]) if family else 0)
        operation["effective_gate_minor"] = gate
        cli = int(operation["cli_minor"]) or (int(family["cli_minor"]) if family else 0) or 8
        operation["effective_cli_minor"] = cli


def lua_arguments(fragment: str) -> list[dict[str, object]] | None:
    """Read a LuaApiEntry argument field, which is either lua_no_arguments or ARGS(...)."""
    stripped = fragment.strip()
    if stripped == "lua_no_arguments":
        return []
    if not stripped.startswith("ARGS("):
        return None
    inner = stripped[len("ARGS(") : stripped.rfind(")")]
    arguments: list[dict[str, object]] = []
    for item in split_top_level(inner):
        if not item.startswith("{"):
            raise InventoryError(f"unexpected Lua argument field: {item}")
        parts = split_top_level(item[1:-1])
        if len(parts) not in (2, 3):
            raise InventoryError(f"unexpected Lua argument: {item}")
        field = parts[0].strip()
        field_match = STRING.fullmatch(field)
        limit = int(parts[2]) if len(parts) == 3 else None
        arguments.append(
            {
                "field": field_match.group(1) if field_match else None,
                "kind": parts[1].strip().removeprefix("LUA_ARG_"),
                "limit": limit,
            }
        )
    return arguments


def lua_table(text: str, anchor: str, source: str) -> list[tuple[str | None, str, bool, list | None, int]]:
    """Read one LuaApiEntry table into (path, operation id, named arguments, arguments, line)."""
    entries: list[tuple[str | None, str, bool, list | None, int]] = []
    for entry, line in brace_entries(text, anchor):
        fields = split_top_level(entry[1:-1])
        if len(fields) < 3:
            raise InventoryError(f"{source}:{line}: LuaApiEntry has fewer than three fields")
        operation = re.fullmatch(r"OPERATION\((\w+)\)", fields[1].strip())
        if not operation:
            if fields[0].strip() == "NULL" and fields[1].strip() == "NULL":
                continue
            if "OPERATION" in fields[1]:
                raise InventoryError(f"{source}:{line}: unreadable operation field {fields[1]!r}")
            continue
        path = None
        if fields[0].strip() != "NULL":
            path_match = STRING.fullmatch(fields[0].strip())
            if not path_match:
                raise InventoryError(f"{source}:{line}: Lua entry path is not a literal")
            path = path_match.group(1)
        named = len(fields) > 3 and fields[3].strip() == "TRUE"
        arguments = lua_arguments(fields[4]) if len(fields) > 4 else None
        entries.append((path, operation.group(1), named, arguments, line))
    return entries


def lua_entries() -> tuple[dict[str, list[dict[str, object]]], set[str]]:
    """Map each wire operation to the Lua names that send it, and to the wire-only bridge."""
    text = strip_comments(read(LUA_ENTRIES))
    installed: dict[str, list[dict[str, object]]] = {}
    for path, identifier, named, arguments, line in lua_table(text, "const LuaApiEntry lua_api[] = {", LUA_ENTRIES):
        if path is None:
            continue
        installed.setdefault(identifier, []).append(
            {
                "path": path,
                "named_arguments": named,
                "arguments": arguments,
                "source": LUA_ENTRIES,
                "line": line,
            }
        )
    bridge = {
        identifier
        for _, identifier, _, _, _ in lua_table(text, "static const LuaApiEntry bridge_operations[] = {", LUA_ENTRIES)
    }
    return installed, bridge


def operations_document() -> dict[str, object]:
    operations, extra = operation_rows()
    resolve_families(operations)
    installed, bridge = lua_entries()
    for operation in operations:
        identifier = str(operation["id"])
        operation["lua"] = sorted(installed.get(identifier, []), key=lambda item: str(item["path"]))
        operation["lua_bridge"] = identifier in bridge
    operations.sort(key=lambda item: str(item["name"]))
    return {
        "format": 1,
        "kind": "operations",
        "generated_by": "scripts/contract-inventory.py",
        "api": extra,
        "count": len(operations),
        "entries": operations,
    }


# --------------------------------------------------------------------------------------------------
# Events
# --------------------------------------------------------------------------------------------------


def socket_events() -> dict[str, dict[str, object]]:
    """Read native_socket_events[], the names a socket client may subscribe to."""
    text = strip_comments(read(NATIVE_CONTROL_SOURCE))
    anchor = "const char* native_socket_events[] ="
    open_brace, close_brace = block_after(text, anchor)
    events: dict[str, dict[str, object]] = {}
    for match in STRING.finditer(text, open_brace, close_brace):
        events[match.group(1)] = {
            "channels": {"socket"},
            "subscription_table": True,
            "source": NATIVE_CONTROL_SOURCE,
            "line": line_at(text, match.start()),
        }
    if not events:
        raise InventoryError(f"{NATIVE_CONTROL_SOURCE}: native_socket_events[] is empty")
    return events


def mask_strings(text: str) -> str:
    """Blank the inside of every string literal, keeping the quotes, offsets and line numbers.

    Enclosing-call detection then never mistakes a parenthesis inside a message for code, while a
    literal can still be found at the same offset in the original text.
    """
    out: list[str] = []
    index = 0
    while index < len(text):
        char = text[index]
        if char != '"':
            out.append(char)
            index += 1
            continue
        out.append('"')
        index += 1
        while index < len(text):
            if text[index] == "\\" and index + 1 < len(text):
                out.append(" ")
                out.append("\n" if text[index + 1] == "\n" else " ")
                index += 2
                continue
            if text[index] == '"':
                out.append('"')
                index += 1
                break
            out.append("\n" if text[index] == "\n" else " ")
            index += 1
    return "".join(out)


def enclosing_call(masked: str, offset: int) -> str:
    """The name of the function whose argument list holds `offset`, or an empty string."""
    depth = 0
    index = offset - 1
    while index >= 0:
        char = masked[index]
        if char == ")":
            depth += 1
        elif char == "(":
            if depth == 0:
                start = index
                while start > 0 and (masked[start - 1].isalnum() or masked[start - 1] == "_"):
                    start -= 1
                return masked[start:index]
            depth -= 1
        index -= 1
    return ""


def scanned_sources() -> list[tuple[str, str, str]]:
    """Every C source under src/, with its comments stripped and its string bodies masked."""
    sources: list[tuple[str, str, str]] = []
    for path in sorted((ROOT / EVENT_SCAN_DIR).rglob("*")):
        if path.suffix not in {".c", ".h"}:
            continue
        text = strip_comments(path.read_text(encoding="utf-8"))
        sources.append((path.relative_to(ROOT).as_posix(), text, mask_strings(text)))
    if not sources:
        raise InventoryError(f"{EVENT_SCAN_DIR}: no sources to scan")
    return sources


def function_definitions(sources: list[tuple[str, str, str]]) -> dict[str, list[tuple[str, str, str]]]:
    """Every function defined under src/, as name -> (parameters, body, masked body)."""
    definitions: dict[str, list[tuple[str, str, str]]] = {}
    for _, text, masked in sources:
        for match in re.finditer(r"\b([A-Za-z_]\w*)\s*\(", masked):
            name = match.group(1)
            try:
                close = matching_group(masked, match.end() - 1, "(", ")")
            except InventoryError:
                continue
            body_open = masked.find("{", close)
            if body_open == -1 or masked[close + 1 : body_open].strip():
                continue
            try:
                body_end = matching_group(masked, body_open, "{", "}")
            except InventoryError:
                continue
            body = text[body_open:body_end]
            definitions.setdefault(name, []).append((text[match.end() : close], body, mask_strings(body)))
    return definitions


def helper_channels(
    definitions: dict[str, list[tuple[str, str, str]]],
    memo: dict[str, set[str]],
    name: str,
    stack: frozenset[str] = frozenset(),
) -> set[str]:
    """The channels a helper forwards one of its own parameters to, following other helpers.

    Most events are published by a helper that takes the name as an argument and hands it on:
    dispatch_output_event sends its `event` to the socket publisher and the runtime dispatcher,
    gnoblin_control_publish_event hands its `name` to native_publish_request_event, which does the
    same. The literal is at the call site, so the definitions decide the channels, and a helper
    that forwards to another publisher has to be followed to the end.
    """
    if name in memo:
        return memo[name]
    if name in stack:
        return set()
    channels: set[str] = set()
    for parameters, body, masked_body in definitions.get(name, ()):
        parameter = next(
            (word for word in ("event", "name") if re.search(rf"\b{word}\b", parameters)),
            None,
        )
        if parameter is None:
            continue
        if re.search(rf"native_runtime_dispatch_event\s*\([^;]*\b{parameter}\b", body):
            channels.add("runtime")
        if "publish_native_socket_event" in body and re.search(rf'"name",\s*{parameter}\b', body):
            channels.add("socket")
        for call in re.finditer(r"\b([A-Za-z_]\w*)\s*\(", masked_body):
            callee = call.group(1)
            if callee == name or callee not in definitions:
                continue
            try:
                call_end = matching_group(masked_body, call.end() - 1, "(", ")")
            except InventoryError:
                continue
            if not re.search(rf"\b{parameter}\b", masked_body[call.end() : call_end]):
                continue
            channels |= helper_channels(definitions, memo, callee, stack | {name})
    memo[name] = channels
    return channels


def dispatch_events() -> dict[str, dict[str, object]]:
    """Find every name passed to a publish or dispatch function anywhere under src/.

    src/control owns most of them, but a few publishers live elsewhere (the idle watches are
    dispatched from src/portal/grants), so the whole tree is scanned. A literal is a name only
    when the call that holds it publishes or dispatches it, and that call's own name is a fallback
    for a publisher whose definition this scan would otherwise miss. Documentation strings,
    comparisons and subscription tests are left out, and so are names ending in `.` or `-`, which
    are the prefixes of comparisons.
    """
    events: dict[str, dict[str, object]] = {}
    name_pattern = re.compile(r'"(gnoblin\.[a-z0-9][a-z0-9._-]*)"')
    sources = scanned_sources()
    definitions = function_definitions(sources)
    memo: dict[str, set[str]] = {}
    for relative, text, masked in sources:
        for match in name_pattern.finditer(text):
            name = match.group(1)
            if name.endswith((".", "-")):
                continue
            caller = enclosing_call(masked, match.start())
            channels = helper_channels(definitions, memo, caller) if caller else set()
            if not channels and ("publish" in caller or "dispatch" in caller):
                channels = {"runtime"} if "runtime" in caller else {"socket"}
            if not channels:
                continue
            line = line_at(text, match.start())
            entry = events.setdefault(
                name,
                {"channels": set(), "subscription_table": False, "source": relative, "line": line},
            )
            entry["channels"].update(channels)  # type: ignore[union-attr]
    return events


def events_document() -> dict[str, object]:
    events = socket_events()
    for name, entry in dispatch_events().items():
        existing = events.get(name)
        if existing is None:
            events[name] = entry
            continue
        existing["channels"].update(entry["channels"])  # type: ignore[union-attr]
    entries = []
    for name in sorted(events):
        entry = events[name]
        entries.append(
            {
                "name": name,
                "channels": sorted(entry["channels"]),  # type: ignore[arg-type]
                "subscription_table": entry["subscription_table"],
                "source": entry["source"],
                "line": entry["line"],
            }
        )
    return {
        "format": 1,
        "kind": "events",
        "generated_by": "scripts/contract-inventory.py",
        "count": len(entries),
        "entries": entries,
    }


# --------------------------------------------------------------------------------------------------
# gnoblinctl
# --------------------------------------------------------------------------------------------------


def help_entries(text: str) -> dict[tuple[str, str | None], dict[str, object]]:
    entries: dict[tuple[str, str | None], dict[str, object]] = {}
    for entry, line in brace_entries(text, "static const HelpEntry help_entries[] = {"):
        fields = split_top_level(entry[1:-1])
        if len(fields) != 3:
            raise InventoryError(f"{CLI_TABLE}:{line}: HelpEntry has {len(fields)} fields")
        command = STRING.fullmatch(fields[0].strip())
        if not command:
            raise InventoryError(f"{CLI_TABLE}:{line}: HelpEntry command is not a literal")
        action = None if fields[1].strip() == "NULL" else string_literals(fields[1])
        entries[(command.group(1), action)] = {
            "summary": string_literals(fields[2]),
            "source": CLI_TABLE,
            "line": line,
        }
    return entries


def cli_document() -> dict[str, object]:
    text = strip_comments(read(CLI_TABLE))
    summaries = help_entries(text)
    entries = []
    for entry, line in brace_entries(text, "const CommandSpec commands[] = {"):
        fields = split_top_level(entry[1:-1])
        if len(fields) != 2:
            raise InventoryError(f"{CLI_TABLE}:{line}: CommandSpec has {len(fields)} fields")
        command = STRING.fullmatch(fields[0].strip())
        if not command:
            raise InventoryError(f"{CLI_TABLE}:{line}: command name is not a literal")
        actions = fields[1].strip()
        words = string_literals(actions).split() if actions != "NULL" else []
        action_entries = []
        for action in words:
            summary = summaries.get((command.group(1), action))
            action_entries.append(
                {
                    "action": action,
                    "summary": summary["summary"] if summary else None,
                    "source": summary["source"] if summary else CLI_TABLE,
                    "line": summary["line"] if summary else line,
                }
            )
        own = summaries.get((command.group(1), None))
        entries.append(
            {
                "command": command.group(1),
                "summary": own["summary"] if own else None,
                "actions": action_entries,
                "source": CLI_TABLE,
                "line": line,
            }
        )
    entries.sort(key=lambda item: str(item["command"]))
    return {
        "format": 1,
        "kind": "cli",
        "generated_by": "scripts/contract-inventory.py",
        "count": len(entries),
        "actions": sum(len(entry["actions"]) for entry in entries),  # type: ignore[arg-type]
        "entries": entries,
    }


# --------------------------------------------------------------------------------------------------
# Lua configuration keys
# --------------------------------------------------------------------------------------------------


def config_keys_document() -> dict[str, object]:
    """Read the top-level keys gnoblin.configure accepts and the keys that can change at runtime."""
    keys = string_array(
        strip_comments(read(LUA_CONFIGURE_KEYS)),
        "static const char* const known_configure_sections[] =",
        LUA_CONFIGURE_KEYS,
    )
    protocols = string_array(
        strip_comments(read(LUA_CONFIGURE_KEYS)),
        "static const char* const known_protocols[] =",
        LUA_CONFIGURE_KEYS,
    )
    overrides_text = strip_comments(read(OVERRIDES_HEADER))
    overridable: dict[str, list[str]] = {}
    for entry, line in brace_entries(overrides_text, "} allowed[] = {"):
        fields = split_top_level(entry[1:-1])
        if len(fields) != 2:
            raise InventoryError(f"{OVERRIDES_HEADER}:{line}: allowed entry has {len(fields)} fields")
        section = STRING.fullmatch(fields[0].strip())
        if not section:
            raise InventoryError(f"{OVERRIDES_HEADER}:{line}: section is not a literal")
        overridable.setdefault(section.group(1), [])
        if fields[1].strip() != "NULL":
            overridable[section.group(1)].append(string_literals(fields[1]))
    entries = []
    known = {str(item["value"]) for item in keys}
    for index, item in enumerate(keys):
        key = str(item["value"])
        allowed_keys = overridable.get(key, [])
        entries.append(
            {
                "key": key,
                "table_index": index,
                "runtime_overridable": key in overridable,
                "runtime_override_keys": allowed_keys,
                "source": item["source"],
                "line": item["line"],
            }
        )
    unknown = sorted(set(overridable) - known)
    if unknown:
        raise InventoryError(f"{OVERRIDES_HEADER}: runtime overrides for unknown sections: {unknown}")
    entries.sort(key=lambda item: str(item["key"]))
    return {
        "format": 1,
        "kind": "config-keys",
        "generated_by": "scripts/contract-inventory.py",
        "count": len(entries),
        "entries": entries,
        "enumerations": [
            {
                "name": "protocols",
                "values": [str(item["value"]) for item in protocols],
                "source": LUA_CONFIGURE_KEYS,
                "line": protocols[0]["line"],
            }
        ],
    }


# --------------------------------------------------------------------------------------------------
# Output
# --------------------------------------------------------------------------------------------------


def documents() -> dict[str, dict[str, object]]:
    return {
        "operations.json": operations_document(),
        "events.json": events_document(),
        "cli.json": cli_document(),
        "config-keys.json": config_keys_document(),
    }


def inline_json(value: object) -> str | None:
    """The one-line form of a value, or None when prettier would keep it on several lines.

    The checked-in files are formatted by prettier through .pre-commit-config.yaml, so this writer
    follows its JSON rules: an object always breaks, and an array stays on one line only when it
    holds scalars that fit.
    """
    if isinstance(value, dict):
        return "{}" if not value else None
    if isinstance(value, list):
        if not value:
            return "[]"
        parts = [inline_json(item) for item in value]
        if any(part is None for part in parts):
            return None
        return "[" + ", ".join(parts) + "]"
    return json.dumps(value)


def json_lines(value: object, level: int, width: int, prefix: str) -> list[str]:
    """Render `value` in the shape prettier gives it, indented under `prefix`."""
    text = inline_json(value)
    indent = " " * (2 * level)
    if text is not None and len(indent) + len(prefix) + len(text) <= width:
        return [indent + prefix + text]
    if isinstance(value, dict):
        lines = [indent + prefix + "{"]
        members = list(value.items())
        for index, (key, member) in enumerate(members):
            member_prefix = json.dumps(key) + ": "
            member_lines = json_lines(member, level + 1, width, member_prefix)
            member_lines[-1] += "," if index + 1 < len(members) else ""
            lines.extend(member_lines)
        lines.append(indent + "}")
        return lines
    if isinstance(value, list):
        lines = [indent + prefix + "["]
        for index, member in enumerate(value):
            member_lines = json_lines(member, level + 1, width, "")
            member_lines[-1] += "," if index + 1 < len(value) else ""
            lines.extend(member_lines)
        lines.append(indent + "]")
        return lines
    return [indent + prefix + json.dumps(value)]


def counts_block(docs: dict[str, dict[str, object]]) -> str:
    """The counts a reader sees first, written so that a stale file fails --check."""
    lines = [COUNT_START, ""]
    for name in ("operations.json", "events.json", "cli.json", "config-keys.json"):
        document = docs[name]
        detail = f" ({document['actions']} actions)" if name == "cli.json" else ""
        sources = sorted({str(entry["source"]) for entry in document["entries"]})  # type: ignore[union-attr]
        lines.append(
            f"- `{name}`: {document['count']} entries{detail}, from {', '.join(f'`{item}`' for item in sources)}"
        )
    lines.extend(["", COUNT_END])
    return "\n".join(lines)


def render(docs: dict[str, dict[str, object]]) -> dict[Path, str]:
    rendered = {OUT / name: "\n".join(json_lines(document, 0, 120, "")) + "\n" for name, document in docs.items()}
    if not README.is_file():
        raise InventoryError(f"{README.relative_to(ROOT)}: missing; the counts block lives there")
    readme = README.read_text(encoding="utf-8")
    start = readme.find(COUNT_START)
    end = readme.find(COUNT_END)
    if start == -1 or end == -1:
        raise InventoryError(f"{README.relative_to(ROOT)}: missing the contract-counts markers")
    block = counts_block(docs)
    rendered[README] = readme[:start] + block + readme[end + len(COUNT_END) :]
    return rendered


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail when a checked-in file differs from what the sources say",
    )
    arguments = parser.parse_args()
    try:
        docs = documents()
        rendered = render(docs)
    except InventoryError as error:
        print(f"contract-inventory: {error}", file=sys.stderr)
        return 1
    stale = []
    for path, content in sorted(rendered.items()):
        current = path.read_text(encoding="utf-8") if path.is_file() else None
        if current != content:
            stale.append(path)
    for document in docs.values():
        print(f"{document['kind']}: {document['count']} entries")
    if not arguments.check:
        OUT.mkdir(parents=True, exist_ok=True)
        for path in stale:
            path.write_text(rendered[path], encoding="utf-8")
            print(f"wrote {path.relative_to(ROOT)}")
        if not stale:
            print("contract inventory is up to date")
        return 0
    for path in stale:
        print(f"stale: {path.relative_to(ROOT)}", file=sys.stderr)
    if stale:
        print("contract-inventory: run python3 scripts/contract-inventory.py", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
