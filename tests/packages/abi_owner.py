#!/usr/bin/env python3
"""Case I5 of the install layer: every library that an ELF file of a Gnoblin package needs comes from a package that
the Gnoblin package asks for, and (deb and Arch) every library package that it asks for is needed.

usage: abi_owner.py --kind rpm|suse|deb|arch|apk PACKAGE...

Runs in the image of the target after the packages are installed, with the package manager of the system. The rpm and
apk dependencies name each library (libfoo.so.1()(64bit), so:libfoo.so.1): a library is covered when the package that
holds the file that ldd resolves provides that name. deb and Arch dependencies name packages: a library is covered when
the package that holds it is a dependency, or provides one. expectations.json lists the dependencies on packages that
hold libraries but are asked for as programs or services, with the reason.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import elf  # noqa: E402

LIBRARY = re.compile(r"^/(?:usr/)?lib(?:64|32)?(?:/[a-z0-9_]+-linux-[a-z]+)?/lib[^/]+\.so\.\d")


def run(*command: str) -> str:
    return subprocess.run(command, capture_output=True, text=True).stdout


def files_of(kind: str, package: str) -> list[str]:
    if kind in ("rpm", "suse"):
        return run("rpm", "-ql", package).splitlines()
    if kind == "deb":
        return run("dpkg", "-L", package).splitlines()
    if kind == "arch":
        return run("pacman", "-Qlq", package).splitlines()
    return ["/" + line for line in run("apk", "info", "-qL", package).splitlines() if line]


def owner_of(kind: str, path: str) -> str:
    candidates = [path, os.path.realpath(path)]
    for candidate in list(candidates):
        for prefix in ("/usr/lib", "/lib"):
            other = "/lib" if prefix == "/usr/lib" else "/usr/lib"
            if candidate.startswith(prefix + "/"):
                candidates.append(other + candidate[len(prefix) :])
    for candidate in candidates:
        if kind in ("rpm", "suse"):
            out = run("rpm", "-qf", "--qf", "%{NAME}\n", candidate).strip()
            if out and "not owned" not in out:
                return out.splitlines()[0]
        elif kind == "deb":
            out = run("dpkg", "-S", candidate).strip()
            if out and ":" in out:
                return out.splitlines()[0].split(":")[0].split(",")[0].strip()
        elif kind == "arch":
            out = run("pacman", "-Qqo", candidate).strip()
            if out:
                return out.splitlines()[0]
        else:
            out = run("apk", "info", "-W", candidate).strip()
            match = re.search(r"is owned by (\S+)", out)
            if match:
                return re.sub(r"-[0-9][^-]*-r[0-9]+$", "", match.group(1))
    return ""


def requirements(kind: str, package: str) -> list[str]:
    if kind in ("rpm", "suse"):
        return [line.strip() for line in run("rpm", "-q", "--requires", package).splitlines() if line.strip()]
    if kind == "deb":
        text = run("dpkg-query", "-W", "-f=${Pre-Depends}, ${Depends}", package)
        return [part.strip() for part in re.split(r"[,|]", text) if part.strip()]
    if kind == "arch":
        text = run("pacman", "-Qi", package)
        match = re.search(r"^Depends On\s*:\s*(.*?)\n(?=\S)", text, re.MULTILINE | re.DOTALL)
        return match.group(1).split() if match and match.group(1).strip() != "None" else []
    lines = run("apk", "info", "-R", package).splitlines()
    return [line.strip() for line in lines[1:] if line.strip()]


def bare(name: str) -> str:
    """The name of a dependency without its version, and a deb name without its architecture (python3:any). The
    apk names so:libfoo.so.1 and cmd:tool keep their prefix."""
    name = re.split(r"[\s(<>=]", name.strip(), maxsplit=1)[0]
    return re.sub(r":(any|native|amd64|arm64|i386)$", "", name)


def provides(kind: str, package: str) -> set[str]:
    if kind in ("rpm", "suse"):
        lines = run("rpm", "-q", "--provides", package).splitlines()
        return {line.split(" ")[0].strip() for line in lines if line.strip()}
    if kind == "deb":
        text = run("dpkg-query", "-W", "-f=${Provides}", package)
        return {bare(part.strip()) for part in text.split(",") if part.strip()} | {package}
    if kind == "arch":
        text = run("pacman", "-Qi", package)
        match = re.search(r"^Provides\s*:\s*(.*?)\n(?=\S)", text, re.MULTILINE | re.DOTALL)
        names = match.group(1).split() if match and match.group(1).strip() != "None" else []
        return {bare(name) for name in names} | {package}
    lines = run("apk", "info", "-P", package).splitlines()
    return {bare(line.strip()) for line in lines[1:] if line.strip()} | {package}


def resolved(binary: str) -> dict[str, str]:
    out = subprocess.run(["ldd", binary], capture_output=True, text=True, env={**os.environ, "LD_LIBRARY_PATH": ""})
    found = {}
    for line in (out.stdout + out.stderr).splitlines():
        match = re.match(r"\s*(\S+)\s+=>\s+(/\S+)", line)
        if match:
            found[match.group(1)] = match.group(2)
    return found


def check(kind: str, package: str, exempt: dict[str, str], internal: set[str]) -> list[str]:
    errors = []
    elves = [
        path
        for path in files_of(kind, package)
        if os.path.isfile(path) and not os.path.islink(path) and elf.is_elf(Path(path))
    ]
    requires = requirements(kind, package)
    names = {bare(item) for item in requires}
    owners: set[str] = set()
    for binary in elves:
        if "/plugins/" in binary and kind == "apk":
            continue
        paths = resolved(binary)
        for soname in elf.needed_without_tools(Path(binary)):
            if soname in internal or soname.startswith(("ld-linux", "ld-musl")):
                continue
            path = paths.get(soname)
            if not path:
                errors.append(f"{binary}: {soname} does not resolve")
                continue
            owner = owner_of(kind, path)
            if not owner:
                errors.append(f"{binary}: {soname} resolves to {path}, which no package holds")
                continue
            owners.add(owner)
            if owner == package or owner in internal:
                continue
            if kind in ("rpm", "suse"):
                wanted = [item for item in requires if item.startswith(soname + "(")]
                if not wanted:
                    errors.append(f"{binary}: needs {soname} from {owner}, and {package} does not ask for {soname}")
                elif not provides(kind, owner) & {item.split(" ")[0] for item in wanted}:
                    errors.append(f"{binary}: {soname} comes from {owner}, which does not provide {wanted[0]}")
            elif kind == "apk":
                if f"so:{soname}" not in names:
                    errors.append(f"{binary}: needs {soname} from {owner}, and {package} does not ask for so:{soname}")
            elif not (provides(kind, owner) & names):
                errors.append(f"{binary}: needs {soname} from {owner}, which {package} does not ask for")
    if kind in ("deb", "arch") and elves:
        for name in sorted(names):
            if name in exempt or name in owners or name.startswith(("gnoblin", "xdg-desktop-portal-gnoblin")):
                continue
            if not any(LIBRARY.match(path) for path in files_of(kind, name)):
                continue  # a program, data or a service, not a library
            if any(name in provides(kind, owner) for owner in owners):
                continue
            errors.append(f"{package} asks for {name}, which holds a library that no ELF file of {package} needs")
    print(f"{package}: {len(elves)} ELF files, libraries from {len(owners)} packages")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--kind", required=True, choices=("rpm", "suse", "deb", "arch", "apk"))
    parser.add_argument("packages", nargs="+")
    args = parser.parse_args()
    expectations = json.loads((HERE / "expectations.json").read_text())
    exempt = expectations["abi_not_linked"]
    internal = set(args.packages)
    errors = []
    for package in args.packages:
        errors += check(args.kind, package, exempt, internal)
    for error in errors:
        print(error)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
