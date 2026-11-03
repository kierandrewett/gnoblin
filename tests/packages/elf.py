"""What the static layer needs to know about an ELF file: whether it is a program, its NEEDED entries, its SONAME and
the symbol versions it needs from each library. readelf (binutils) reads any architecture."""

from __future__ import annotations

import dataclasses
import re
import struct
import subprocess
from pathlib import Path

PT_INTERP = 3


@dataclasses.dataclass
class ElfInfo:
    program: bool  # has a program interpreter: it is started, not loaded
    needed: list[str]
    soname: str
    versions: dict[str, set[str]]  # library soname -> symbol versions it needs from it


def is_elf(path: Path) -> bool:
    try:
        with open(path, "rb") as handle:
            return handle.read(4) == b"\x7fELF"
    except OSError:
        return False


def _has_interpreter(data: bytes) -> bool:
    is64 = data[4] == 2
    endian = "<" if data[5] == 1 else ">"
    if is64:
        (phoff,) = struct.unpack_from(endian + "Q", data, 0x20)
        phentsize, phnum = struct.unpack_from(endian + "HH", data, 0x36)
    else:
        (phoff,) = struct.unpack_from(endian + "I", data, 0x1C)
        phentsize, phnum = struct.unpack_from(endian + "HH", data, 0x2A)
    for index in range(phnum):
        (p_type,) = struct.unpack_from(endian + "I", data, phoff + index * phentsize)
        if p_type == PT_INTERP:
            return True
    return False


def read(path: Path) -> ElfInfo:
    data = path.read_bytes()
    dynamic = subprocess.run(["readelf", "-dW", str(path)], capture_output=True, text=True).stdout
    needed = re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", dynamic)
    soname = next(iter(re.findall(r"\(SONAME\)\s+Library soname: \[([^\]]+)\]", dynamic)), "")
    version_text = subprocess.run(["readelf", "-VW", str(path)], capture_output=True, text=True).stdout
    versions: dict[str, set[str]] = {}
    section = version_text.split("Version needs section", 1)[1] if "Version needs section" in version_text else ""
    current = None
    for line in section.splitlines():
        library = re.search(r"File: (\S+)", line)
        if library:
            current = library.group(1)
            versions.setdefault(current, set())
            continue
        name = re.search(r"Name: (\S+)", line)
        if name and current:
            versions[current].add(name.group(1))
    return ElfInfo(program=_has_interpreter(data), needed=needed, soname=soname, versions=versions)


def version_number(node: str, prefix: str) -> tuple[int, ...] | None:
    """GLIBC_2.38 -> (2, 38) for the prefix GLIBC. None for another library or a node without a number."""
    match = re.fullmatch(re.escape(prefix) + r"_(\d+(?:\.\d+)*)", node)
    return tuple(int(part) for part in match.group(1).split(".")) if match else None


def compare_versions(left: str, right: str) -> int:
    """Compare the leading numbers of two version strings: 2.43 against 2.43.0-1 is equal."""

    def numbers(text: str) -> tuple[int, ...]:
        text = text.split(":", 1)[-1]
        match = re.match(r"\d+(?:\.\d+)*", text)
        parts = [int(part) for part in match.group(0).split(".")] if match else []
        while parts and parts[-1] == 0:
            parts.pop()
        return tuple(parts)

    a, b = numbers(left), numbers(right)
    return (a > b) - (a < b)


def needed_without_tools(path: Path) -> list[str]:
    """The NEEDED entries of an ELF file, read from its dynamic segment with the standard library only. The install
    layer runs in an image without binutils."""
    data = path.read_bytes()
    if data[:4] != b"\x7fELF":
        return []
    is64 = data[4] == 2
    endian = "<" if data[5] == 1 else ">"
    if is64:
        (phoff,) = struct.unpack_from(endian + "Q", data, 0x20)
        phentsize, phnum = struct.unpack_from(endian + "HH", data, 0x36)
        header, entry = endian + "IIQQQQQQ", endian + "qQ"
    else:
        (phoff,) = struct.unpack_from(endian + "I", data, 0x1C)
        phentsize, phnum = struct.unpack_from(endian + "HH", data, 0x2A)
        header, entry = endian + "IIIIIIII", endian + "iI"
    loads, dynamic = [], None
    for index in range(phnum):
        fields = struct.unpack_from(header, data, phoff + index * phentsize)
        if is64:
            p_type, _flags, offset, vaddr, _paddr, filesz = fields[:6]
        else:
            p_type, offset, vaddr, _paddr, filesz = fields[:5]
        if p_type == 1:
            loads.append((vaddr, offset, filesz))
        elif p_type == 2:
            dynamic = (offset, filesz)
    if dynamic is None:
        return []

    def file_offset(address: int) -> int:
        for vaddr, offset, size in loads:
            if vaddr <= address < vaddr + size:
                return address - vaddr + offset
        raise ValueError(f"{path}: address {address:#x} is in no loaded segment")

    size = struct.calcsize(entry)
    needed, strtab = [], None
    for position in range(dynamic[0], dynamic[0] + dynamic[1], size):
        tag, value = struct.unpack_from(entry, data, position)
        if tag == 0:
            break
        if tag == 1:
            needed.append(value)
        elif tag == 5:
            strtab = file_offset(value)
    if strtab is None:
        return []
    return [data[strtab + name : data.index(b"\0", strtab + name)].decode() for name in needed]
