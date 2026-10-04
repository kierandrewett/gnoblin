#!/usr/bin/env python3
"""Export executable built-in Mutter keybindings for the Gnoblin Lua API."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


TABLE_GROUPS = {
    "COMMON_KEYBINDINGS": "wm",
    "MUTTER_KEYBINDINGS": "mutter",
    "WAYLAND_KEYBINDINGS": "wayland",
    "NATIVE_KEYBINDINGS": "wayland",
}


def remove_comments(source: str) -> str:
    """Remove C comments while preserving strings and source positions."""
    output: list[str] = []
    index = 0
    quote: str | None = None
    while index < len(source):
        char = source[index]
        next_char = source[index + 1] if index + 1 < len(source) else ""
        if quote:
            output.append(char)
            if char == "\\" and index + 1 < len(source):
                index += 1
                output.append(source[index])
            elif char == quote:
                quote = None
        elif char in ('"', "'"):
            quote = char
            output.append(char)
        elif char == "/" and next_char == "*":
            output.extend("  ")
            index += 1
            while index + 1 < len(source) and source[index : index + 2] != "*/":
                output.append("\n" if source[index] == "\n" else " ")
                index += 1
            if index + 1 >= len(source):
                raise ValueError("unterminated C block comment")
            output.extend("  ")
            index += 1
        elif char == "/" and next_char == "/":
            output.extend("  ")
            index += 1
            while index < len(source) and source[index] != "\n":
                output.append(" ")
                index += 1
        else:
            output.append(char)
        index += 1
    return "".join(output)


def split_fields(initializer: str) -> list[str]:
    fields: list[str] = []
    start = 0
    quote: str | None = None
    parens = brackets = braces = 0
    index = 0
    while index < len(initializer):
        char = initializer[index]
        if quote:
            if char == "\\":
                index += 1
            elif char == quote:
                quote = None
        elif char in ('"', "'"):
            quote = char
        elif char == "(":
            parens += 1
        elif char == ")":
            parens -= 1
        elif char == "[":
            brackets += 1
        elif char == "]":
            brackets -= 1
        elif char == "{":
            braces += 1
        elif char == "}":
            braces -= 1
        elif char == "," and parens == brackets == braces == 0:
            fields.append(initializer[start:index].strip())
            start = index + 1
        index += 1
    fields.append(initializer[start:].strip())
    return fields


def table_rows(source: str, table_name: str) -> list[str]:
    declaration = re.search(
        rf"\bstatic\s+BuiltinKeybinding\s+{re.escape(table_name)}\s*\[\s*\]\s*=\s*\{{",
        source,
    )
    if not declaration:
        raise ValueError(f"could not find BuiltinKeybinding table {table_name}")

    start = declaration.end() - 1
    depth = 1
    row_start: int | None = None
    rows: list[str] = []
    quote: str | None = None
    index = start + 1
    while index < len(source):
        char = source[index]
        if quote:
            if char == "\\":
                index += 1
            elif char == quote:
                quote = None
        elif char in ('"', "'"):
            quote = char
        elif char == "{":
            depth += 1
            if depth == 2:
                row_start = index + 1
        elif char == "}":
            if depth == 2 and row_start is not None:
                rows.append(source[row_start:index])
                row_start = None
            depth -= 1
            if depth == 0:
                return rows
        index += 1
    raise ValueError(f"unterminated BuiltinKeybinding table {table_name}")


def extract_catalog(source_path: Path) -> dict[str, list[str]]:
    source = remove_comments(source_path.read_text(encoding="utf-8"))
    catalog = {group: set() for group in ("wm", "mutter", "wayland")}
    for table_name, group in TABLE_GROUPS.items():
        for row in table_rows(source, table_name):
            fields = split_fields(row)
            if len(fields) != 5:
                raise ValueError(f"unexpected initializer in {table_name}: expected 5 fields")
            match = re.fullmatch(r'"([a-z0-9]+(?:-[a-z0-9]+)*)"', fields[0])
            if not match:
                raise ValueError(f"unexpected key name in {table_name}: {fields[0]}")
            if fields[3] != "NULL":
                catalog[group].add(match.group(1))

    return {group: sorted(names) for group, names in catalog.items()}


def main() -> int:
    if len(sys.argv) != 3:
        print(
            "usage: generate-mutter-keybinding-catalog.py <keybindings.c> <output.json>",
            file=sys.stderr,
        )
        return 2

    source_path, output_path = map(Path, sys.argv[1:])
    try:
        catalog = extract_catalog(source_path)
        output = {"format": 1, "groups": catalog}
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    except (OSError, ValueError) as error:
        print(f"generate-mutter-keybinding-catalog: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
