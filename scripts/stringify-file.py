#!/usr/bin/env python3
"""Embed a text file as a C string-literal header.

Usage: stringify-file.py <input> <output.h> <SYMBOL>

Emits `static const char SYMBOL[] = "...";` so a config/text resource (e.g.
gnoblin.defaults.conf) can be baked into the binary at compile time, mirroring
the Rust side's include_str!(). Keeps shipped defaults in the config file, not
hardcoded in code.
"""
import sys


def main() -> int:
    if len(sys.argv) != 4:
        sys.stderr.write("usage: stringify-file.py <input> <output> <symbol>\n")
        return 2
    src, dst, sym = sys.argv[1], sys.argv[2], sys.argv[3]
    with open(src, "r", encoding="utf-8") as f:
        text = f.read()
    lines = [
        "/* Auto-generated from %s — do not edit. */" % src,
        "static const char %s[] =" % sym,
    ]
    for line in text.split("\n"):
        esc = line.replace("\\", "\\\\").replace('"', '\\"')
        lines.append('    "%s\\n"' % esc)
    lines.append("    ;")
    with open(dst, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
