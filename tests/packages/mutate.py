#!/usr/bin/env python3
"""Break a copy of a package on purpose, for the negative tests of the package suite.

usage: mutate.py layer NAME        print the layer that must reject the mutation
       mutate.py case NAME         print the case that must fail
       mutate.py apply NAME DIR    break the gnoblin package in DIR (a copy of the packages of one target)

scripts/run-package-tests.sh --mutate NAME runs the layer against the broken copy and passes only when that case
failed. Each mutation works on the formats that it names. design/package-testing.md lists them.
"""

from __future__ import annotations

import io
import subprocess
import sys
import tarfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from formats import format_of  # noqa: E402

# The dependency that each drop-* mutation removes from the Depends of the gnoblin deb.
DROPPED = {"drop-library-dep": "libinput", "drop-gobject-dep": "python3-gi"}
# The dependency that each add-* mutation adds to it.
ADDED = {"add-gtk-dep": "libgtk-4-1"}
# name: (layer, case, formats, what it does)
MUTATIONS = {
    "foreign-file": ("static", "S4", ("deb", "archlinux", "tar"), "gnoblin also installs /usr/bin/mutter"),
    "build-path": (
        "static",
        "S7",
        ("deb", "archlinux", "tar"),
        "a file of gnoblin holds /work/build, a path of the build machine",
    ),
    "drop-copyright": ("static", "S2", ("deb",), "the gnoblin deb loses /usr/share/doc/gnoblin/copyright"),
    "drop-library-dep": ("install", "I5", ("deb",), "the gnoblin deb no longer asks for the package of libinput.so.10"),
    "drop-gobject-dep": ("install", "I6", ("deb",), "the gnoblin deb no longer asks for PyGObject, which gdctl needs"),
    "add-gtk-dep": ("install", "I2", ("deb",), "the gnoblin deb asks for libgtk-4-1, so installing it brings GTK 4"),
    "rename-global": (
        "runtime",
        "R2",
        ("deb", "archlinux"),
        "libmutter names the layer-shell global zwlr_layer_shell_v0",
    ),
    "corrupt-payload": ("repository", "P1", ("rpm",), "one byte of the payload of the gnoblin rpm is changed"),
}


def run(*command: str, data: bytes | None = None) -> bytes:
    return subprocess.run(command, input=data, capture_output=True, check=True).stdout


def read_ar(data: bytes) -> list[tuple[str, bytes]]:
    assert data.startswith(b"!<arch>\n"), "not an ar archive"
    members, position = [], 8
    while position < len(data):
        header = data[position : position + 60]
        name = header[:16].decode().strip().rstrip("/")
        size = int(header[48:58].decode().strip())
        members.append((name, data[position + 60 : position + 60 + size]))
        position += 60 + size + (size % 2)
    return members


def write_ar(members: list[tuple[str, bytes]]) -> bytes:
    out = bytearray(b"!<arch>\n")
    for name, content in members:
        out += f"{name:<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(content):<10}`\n".encode()
        out += content + (b"\n" if len(content) % 2 else b"")
    return bytes(out)


def decompress(name: str, content: bytes) -> bytes:
    if name.endswith(".xz"):
        return run("xz", "-dc", data=content)
    if name.endswith(".zst"):
        return run("zstd", "-dcq", data=content)
    if name.endswith(".gz"):
        return run("gzip", "-dc", data=content)
    return content


def compress(name: str, content: bytes) -> bytes:
    if name.endswith(".xz"):
        return run("xz", "-c", data=content)
    if name.endswith(".zst"):
        return run("zstd", "-cq", data=content)
    if name.endswith(".gz"):
        return run("gzip", "-cn", data=content)
    return content


def edit_tar(data: bytes, edit) -> bytes:
    """Copy a tar stream member by member, through edit(member, content) -> (member, content) or None to drop it,
    then let edit(None, None) return extra members to append."""
    source = tarfile.open(fileobj=io.BytesIO(data), mode="r:")
    out = io.BytesIO()
    with tarfile.open(fileobj=out, mode="w:", format=tarfile.PAX_FORMAT) as target:
        for member in source:
            content = source.extractfile(member).read() if member.isreg() else None
            result = edit(member, content)
            if result is None:
                continue
            member, content = result
            if content is not None:
                member.size = len(content)
            target.addfile(member, io.BytesIO(content) if content is not None else None)
        for member, content in edit(None, None) or []:
            member.size = len(content)
            target.addfile(member, io.BytesIO(content))
    return out.getvalue()


def payload_editor(mutation: str):
    def edit(member, content):
        if member is None:
            if mutation == "foreign-file":
                extra = tarfile.TarInfo("./usr/bin/mutter")
                extra.mode, extra.uname, extra.gname = 0o755, "root", "root"
                return [(extra, b"#!/bin/sh\nexit 0\n")]
            return []
        name = member.name.lstrip("./")
        if mutation == "build-path" and name == "usr/share/wayland-sessions/gnoblin.desktop":
            content += b"X-Build-Root=/work/build/prefix-x86_64-linux-gnu\n"
        if mutation == "drop-copyright" and name == "usr/share/doc/gnoblin/copyright":
            return None
        if mutation == "rename-global" and name.startswith("usr/lib/gnoblin/lib/libmutter-") and content:
            if b"\0zwlr_layer_shell_v1\0" in content:
                content = content.replace(b"\0zwlr_layer_shell_v1\0", b"\0zwlr_layer_shell_v0\0")
        return member, content

    return edit


def mutate_deb(path: Path, mutation: str):
    members = read_ar(path.read_bytes())
    result = []
    for name, content in members:
        if name.startswith("data.tar") and mutation in (
            "foreign-file",
            "build-path",
            "drop-copyright",
            "rename-global",
        ):
            content = compress(name, edit_tar(decompress(name, content), payload_editor(mutation)))
        if name.startswith("control.tar") and (mutation in DROPPED or mutation in ADDED):

            def edit(member, data):
                if member is None:
                    return []
                if member.name.lstrip("./") == "control":
                    lines = []
                    for line in data.decode().splitlines():
                        if line.startswith("Depends:"):
                            items = [item.strip() for item in line[len("Depends:") :].split(",")]
                            if mutation in ADDED:
                                kept = [*items, ADDED[mutation]]
                            else:
                                kept = [item for item in items if not item.startswith(DROPPED[mutation])]
                                if len(kept) == len(items):
                                    raise SystemExit(f"the gnoblin deb asks for no {DROPPED[mutation]} package")
                            line = "Depends: " + ", ".join(kept)
                        lines.append(line)
                    data = ("\n".join(lines) + "\n").encode()
                return member, data

            content = compress(name, edit_tar(decompress(name, content), edit))
        result.append((name, content))
    path.write_bytes(write_ar(result))


def mutate_tar_package(path: Path, mutation: str, zst: bool):
    data = run("zstd", "-dcq", str(path)) if zst else run("xz", "-dc", str(path))
    data = edit_tar(data, payload_editor(mutation))
    path.write_bytes(run("zstd", "-cq", data=data) if zst else run("xz", "-c", data=data))


def mutate_rpm(path: Path, mutation: str):
    data = bytearray(path.read_bytes())
    # The payload is at the end of the file. Changing a byte there breaks its digest, not the header.
    data[-200] ^= 0xFF
    path.write_bytes(bytes(data))


def apply(mutation: str, directory: Path) -> int:
    layer, case, formats, description = MUTATIONS[mutation]
    candidates = [
        path
        for path in sorted(directory.iterdir())
        if format_of(path) in formats
        and path.name.startswith(("gnoblin_", "gnoblin-0", "gnoblin-1", "gnoblin-2"))
        and not path.name.startswith(("gnoblin-geoclue", "gnoblin-ananicy"))
    ]
    if not candidates:
        raise SystemExit(f"{mutation} needs a gnoblin package in one of the formats {', '.join(formats)}")
    for path in candidates:
        fmt = format_of(path)
        if fmt == "deb":
            mutate_deb(path, mutation)
        elif fmt == "archlinux":
            mutate_tar_package(path, mutation, zst=True)
        elif fmt == "tar":
            mutate_tar_package(path, mutation, zst=False)
        elif fmt == "rpm":
            mutate_rpm(path, mutation)
        print(f"{mutation}: {path.name}: {description} (the {layer} layer must fail {case})")
    return 0


def main() -> int:
    if len(sys.argv) < 3 or sys.argv[2] not in MUTATIONS:
        print(__doc__ + "\nmutations: " + ", ".join(MUTATIONS))
        return 2
    command, mutation = sys.argv[1], sys.argv[2]
    if command == "layer":
        print(MUTATIONS[mutation][0])
        return 0
    if command == "case":
        print(MUTATIONS[mutation][1])
        return 0
    if command == "apply":
        return apply(mutation, Path(sys.argv[3]))
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
