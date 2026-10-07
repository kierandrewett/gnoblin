#!/usr/bin/env python3
"""Generate a C translation unit containing the shipped default-config tree."""

from pathlib import Path
import sys


if len(sys.argv) != 3:
    raise SystemExit(f"usage: {Path(sys.argv[0]).name} DEFAULT_CONFIG_DIRECTORY OUTPUT")

source_root = Path(sys.argv[1])
target = Path(sys.argv[2])
if not source_root.is_dir():
    raise SystemExit(f"default config directory does not exist: {source_root}")

files = sorted(path for path in source_root.rglob("*") if path.is_file())
if not files or source_root / "init.lua" not in files:
    raise SystemExit(f"default config directory needs init.lua: {source_root}")


def bytes_literal(source: bytes) -> str:
    return ",\n    ".join(
        ", ".join(f"0x{byte:02x}" for byte in source[offset : offset + 12])
        for offset in range(0, len(source), 12)
    )


arrays = []
entries = []
for number, path in enumerate(files):
    content = path.read_bytes()
    name = f"default_config_{number}"
    arrays.append(f"static const unsigned char {name}[] = {{\n    {bytes_literal(content)}, 0x00\n}};")
    relative = path.relative_to(source_root).as_posix()
    entries.append(
        f'    {{"{relative}", (const char*){name}, sizeof {name} - 1}},'
    )

target.write_text(
    '#include "gnoblin-config.h"\n\n'
    + "\n\n".join(arrays)
    + "\n\nstatic const GnoblinConfigDefaultFile default_config_files[] = {\n"
    + "\n".join(entries)
    + "\n};\n\n"
    + "const GnoblinConfigDefaultFile* gnoblin_config_default_files(gsize* count) {\n"
    + "    if (count)\n"
    + "        *count = G_N_ELEMENTS(default_config_files);\n"
    + "    return default_config_files;\n"
    + "}\n\n"
    + "const GnoblinConfigDefaultFile* gnoblin_config_default_file(const char* path) {\n"
    + "    if (!path)\n"
    + "        return NULL;\n"
    + "    for (gsize i = 0; i < G_N_ELEMENTS(default_config_files); i++)\n"
    + "        if (g_str_equal(default_config_files[i].path, path))\n"
    + "            return &default_config_files[i];\n"
    + "    return NULL;\n"
    + "}\n\n"
    + "const char* gnoblin_config_default_lua(gsize* length) {\n"
    + "    const GnoblinConfigDefaultFile* file = gnoblin_config_default_file(\"init.lua\");\n"
    + "    if (length)\n"
    + "        *length = file ? file->length : 0;\n"
    + "    return file ? file->contents : \"\";\n"
    + "}\n",
    encoding="ascii",
)
