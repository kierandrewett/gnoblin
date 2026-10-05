#!/usr/bin/env python3
"""Export executable Mutter keybinding descriptors for the Gnoblin Lua API."""

from __future__ import annotations

import ast
import json
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path


TABLE_GROUPS = {
    "COMMON_KEYBINDINGS": "wm",
    "MUTTER_KEYBINDINGS": "mutter",
    "WAYLAND_KEYBINDINGS": "wayland",
    "NATIVE_KEYBINDINGS": "wayland",
}
SCHEMAS = {
    "wm": ("org.gnome.desktop.wm.keybindings", "schemas/org.gnome.desktop.wm.keybindings.gschema.xml.in"),
    "mutter": ("org.gnome.mutter.keybindings", "data/org.gnome.mutter.gschema.xml.in"),
    "wayland": ("org.gnome.mutter.wayland.keybindings", "data/org.gnome.mutter.wayland.gschema.xml.in"),
}
KEY_RE = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*\Z")


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


def executable_actions(source_path: Path) -> dict[str, list[str]]:
    source = remove_comments(source_path.read_text(encoding="utf-8"))
    catalog: dict[str, set[str]] = {group: set() for group in SCHEMAS}
    for table_name, group in TABLE_GROUPS.items():
        for row in table_rows(source, table_name):
            fields = split_fields(row)
            if len(fields) != 5:
                raise ValueError(f"unexpected initializer in {table_name}: expected 5 fields")
            match = re.fullmatch(r'"([a-z0-9]+(?:-[a-z0-9]+)*)"', fields[0])
            if not match:
                raise ValueError(f"unexpected key name in {table_name}: {fields[0]}")
            if fields[3] != "NULL":
                name = match.group(1)
                if name in catalog[group]:
                    raise ValueError(f"duplicate executable action: {group}.{name}")
                catalog[group].add(name)

    return {group: sorted(names) for group, names in catalog.items()}


def parse_string_array_default(value: str, *, location: str) -> list[str]:
    """Parse the quoted string array subset used by the pinned schemas' `as` defaults."""
    value = value.strip()
    if not (value.startswith("[") and value.endswith("]")):
        raise ValueError(f"{location}: expected a string-array GVariant default")
    body = value[1:-1]
    index = 0
    result: list[str] = []
    while True:
        while index < len(body) and body[index].isspace():
            index += 1
        if index == len(body):
            return result
        quote = body[index]
        if quote not in ("'", '"'):
            raise ValueError(f"{location}: unsupported string-array default syntax")
        start = index
        index += 1
        while index < len(body):
            if body[index] == "\\":
                index += 2
                continue
            if body[index] == quote:
                index += 1
                break
            index += 1
        else:
            raise ValueError(f"{location}: unterminated string in array default")
        try:
            parsed = ast.literal_eval(body[start:index])
        except (SyntaxError, ValueError) as error:
            raise ValueError(f"{location}: invalid string in array default") from error
        if not isinstance(parsed, str):
            raise ValueError(f"{location}: expected string in array default")
        result.append(parsed)
        while index < len(body) and body[index].isspace():
            index += 1
        if index == len(body):
            return result
        if body[index] != ",":
            raise ValueError(f"{location}: expected comma in array default")
        index += 1
        while index < len(body) and body[index].isspace():
            index += 1
        if index == len(body):
            return result


def schema_descriptors(schema_root: Path, group: str) -> dict[str, dict]:
    schema_id, relative_path = SCHEMAS[group]
    path = schema_root / relative_path
    if not path.is_file():
        raise ValueError(f"missing pinned schema source: {path}")
    try:
        root = ET.parse(path).getroot()
    except ET.ParseError as error:
        raise ValueError(f"cannot parse schema source {path}: {error}") from error

    schemas = [schema for schema in root.findall("schema") if schema.get("id") == schema_id]
    if len(schemas) != 1:
        raise ValueError(f"{path}: expected exactly one schema with id {schema_id}")
    descriptors: dict[str, dict] = {}
    for key in schemas[0].findall("key"):
        name = key.get("name")
        if not name or not KEY_RE.fullmatch(name):
            raise ValueError(f"{path}: invalid key name in schema {schema_id}")
        if name in descriptors:
            raise ValueError(f"{path}: duplicate schema key {schema_id}.{name}")
        if key.get("type") != "as":
            descriptors[name] = {"type": key.get("type")}
            continue
        default = key.find("default")
        if default is None or default.text is None:
            raise ValueError(f"{path}: missing default for {schema_id}.{name}")
        summary = key.find("summary")
        description = " ".join("".join(summary.itertext()).split()) if summary is not None else None
        descriptors[name] = {
            "type": "as",
            "description": description or None,
            "default_bindings": parse_string_array_default(
                default.text,
                location=f"{schema_id}.{name}",
            ),
        }
    return descriptors


def extract_catalog(mutter_source: Path, schemas_source: Path) -> dict:
    actions = executable_actions(mutter_source / "src/core/keybindings.c")
    descriptors: dict[str, dict[str, dict]] = {}
    for group in SCHEMAS:
        schema_root = schemas_source if group == "wm" else mutter_source
        descriptors[group] = schema_descriptors(schema_root, group)

    groups: dict[str, dict[str, dict]] = {}
    for group, names in actions.items():
        entries: dict[str, dict] = {}
        for name in names:
            descriptor = descriptors[group].get(name)
            if descriptor is None:
                raise ValueError(f"executable action has no schema key: {group}.{name}")
            if descriptor.get("type") != "as":
                raise ValueError(f"executable action is not a string-array key: {group}.{name}")
            entries[name] = {
                "key": name,
                "description": descriptor["description"],
                "default_bindings": descriptor["default_bindings"],
            }
        groups[group] = entries
    return {"format": 2, "groups": groups}


def main() -> int:
    if len(sys.argv) != 4:
        print(
            "usage: generate-mutter-keybinding-catalog.py <mutter-source-root> "
            "<gsettings-desktop-schemas-source-root> <output.json>",
            file=sys.stderr,
        )
        return 2

    mutter_source, schemas_source, output_path = map(Path, sys.argv[1:])
    try:
        catalog = extract_catalog(mutter_source, schemas_source)
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(json.dumps(catalog, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    except (OSError, ValueError) as error:
        print(f"generate-mutter-keybinding-catalog: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
