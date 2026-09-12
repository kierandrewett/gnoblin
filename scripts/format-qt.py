#!/usr/bin/env python3
"""Check or format QML and JavaScript that contains Qt directives."""

import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true")
    parser.add_argument("--javascript", action="store_true")
    parser.add_argument("files", nargs="+")
    args = parser.parse_args()
    tool = shutil.which("prettier" if args.javascript else os.environ.get("QMLFORMAT", "qmlformat"))
    if not tool and not args.javascript and "QMLFORMAT" not in os.environ:
        tool = next(
            (str(p) for p in (Path("/usr/lib64/qt6/bin/qmlformat"), Path("/usr/lib/qt6/bin/qmlformat")) if p.is_file()),
            None,
        )
    if not tool:
        print(
            "Missing prettier or Qt 6 qmlformat. Install Qt QML tools, or set QMLFORMAT to its executable path.",
            file=sys.stderr,
        )
        return 2
    failed = False
    for name in args.files:
        path = Path(name)
        original = path.read_bytes()
        if args.javascript:
            # Preserve Qt directives byte-for-byte; Prettier treats them as comments.
            source = original.decode("utf-8")
            directives = []

            def mask(match):
                marker = f"// __QT_DIRECTIVE_{len(directives)}__"
                directives.append((marker, match.group(0)))
                return marker

            source = re.sub(r"^\.(?:pragma|import)\b[^\r\n]*", mask, source, flags=re.MULTILINE)
            result = subprocess.run([tool, "--stdin-filepath", name], input=source.encode(), capture_output=True)
            formatted = result.stdout.decode("utf-8")
            for marker, directive in directives:
                formatted = formatted.replace(marker, directive, 1)
            output = formatted.encode()
        else:
            command = [tool, "--ignore-settings", "--indent-width", "4", "--newline", "unix"]
            if name.endswith(".inc.qml"):
                # Test fragments contain sibling objects inserted into an existing root.
                with tempfile.NamedTemporaryFile(suffix=".qml") as wrapper:
                    wrapper.write(b"Item {\n" + original + b"\n}\n")
                    wrapper.flush()
                    result = subprocess.run([*command, wrapper.name], capture_output=True)
                output = b"".join(result.stdout.splitlines(keepends=True)[1:-1])
            else:
                result = subprocess.run([*command, name], capture_output=True)
                output = result.stdout
        if result.stderr:
            sys.stderr.buffer.write(result.stderr)
        if result.returncode:
            print(f"{name}: {tool} failed (exit {result.returncode})", file=sys.stderr)
            failed = True
            continue
        if output != original:
            if args.write:
                path.write_bytes(output)
                print(f"Formatted {name}")
            else:
                print(f"Needs formatting: {name}")
                failed = True
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
