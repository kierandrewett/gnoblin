#!/usr/bin/env python3
"""Compare scripts/pkgrepo/versions.py with the programs that define the orderings: rpm, dpkg and pacman's vercmp.

usage: compare-versions.py rpm|dpkg|pacman ...   (each needs its program: `rpm`, `dpkg`, `vercmp`)

Random version strings plus the nightly spellings are compared in pairs. Any difference is a bug in versions.py.
"""

from __future__ import annotations

import random
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "scripts"))
from pkgrepo import versions  # noqa: E402

rng = random.Random(20261031)


def strings(alphabet: str, count: int, first_digit: bool) -> list[str]:
    out = []
    for _ in range(count):
        text = "".join(rng.choice(alphabet) for _ in range(rng.randint(1, 9)))
        if first_digit:
            text = rng.choice("0123456789") + text
        out.append(text)
    return out


def nightly_set() -> list[str]:
    out = []
    for base in ("0.1.12", "0.1.13", "0.2.0", "1.0.0"):
        out.append(base)
        for date, count in (("20261031", 142), ("20261101", 7), ("20261031", 143)):
            n = versions.nightly(base, base, date, count, "abc1234")
            out += [n.rpm, n.arch]
    return out


def pairs(items: list[str], count: int) -> list[tuple[str, str]]:
    result = [(a, b) for a in items[:30] for b in items[:30]]
    result += [(rng.choice(items), rng.choice(items)) for _ in range(count)]
    return result


def check_rpm() -> list[str]:
    items = nightly_set() + strings("0123456789abcxyzXYZ.~^+_", 400, False)
    pair_list = pairs(items, 1500)
    table = ",".join('{"%s","%s"}' % (a.replace('"', ""), b.replace('"', "")) for a, b in pair_list)
    lua = (
        "local r = {} for i, p in ipairs({%s}) do r[#r+1] = rpm.vercmp(p[1], p[2]) end print(table.concat(r, ','))"
        % table
    )
    output = subprocess.run(
        ["rpm", "--eval", "%{lua:" + lua.replace("%", "%%") + "}"], capture_output=True, text=True, check=True
    ).stdout
    results = [int(x) for x in output.strip().split(",")]
    assert len(results) == len(pair_list), (len(results), len(pair_list))
    return [
        f"rpm {a!r} {b!r}: tool {versions.rpmvercmp(a, b)} rpm {r}"
        for (a, b), r in zip(pair_list, results)
        if versions.rpmvercmp(a, b) != r
    ]


def check_dpkg() -> list[str]:
    items = [n for n in nightly_set() if ":" not in n] + strings("0123456789abcxyzXYZ.+~-", 200, True)
    mismatches = []
    # dpkg rejects an empty revision ("1.0-"), so every string gets a non-empty one.
    pair_list = pairs([i.rstrip("-") + "-1" if not i.rstrip("-").count("-") else i.rstrip("-") for i in items], 500)
    for a, b in pair_list:

        def cmp(op: str) -> bool:
            return subprocess.run(["dpkg", "--compare-versions", a, op, b], capture_output=True).returncode == 0

        real = -1 if cmp("lt") else 0 if cmp("eq") else 1
        if versions.dpkg_compare(a, b) != real:
            mismatches.append(f"dpkg {a!r} {b!r}: tool {versions.dpkg_compare(a, b)} dpkg {real}")
    return mismatches


def check_pacman() -> list[str]:
    items = nightly_set() + strings("0123456789abcxyz.+_~-", 300, False)
    items += [f"{rng.randint(0, 3)}:{x}" for x in strings("0123456789abc.", 30, True)]
    mismatches = []
    for a, b in pairs([i if "-" in i else i + "-1" for i in items], 800):
        real = int(subprocess.run(["vercmp", a, b], capture_output=True, text=True, check=True).stdout.strip())
        if versions.pacman_compare(a, b) != real:
            mismatches.append(f"pacman {a!r} {b!r}: tool {versions.pacman_compare(a, b)} vercmp {real}")
    return mismatches


CHECKS = {"rpm": check_rpm, "dpkg": check_dpkg, "pacman": check_pacman}

failed = False
for name in sys.argv[1:]:
    problems = CHECKS[name]()
    print(f"{name}: {'OK' if not problems else str(len(problems)) + ' differences'}")
    for line in problems[:15]:
        print("  " + line)
    failed |= bool(problems)
raise SystemExit(1 if failed else 0)
