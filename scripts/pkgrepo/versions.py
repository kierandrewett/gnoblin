"""Version numbers: the nightly naming rule and the three version orderings the package managers use.

The orderings are ports of rpmvercmp (RPM), verrevcmp (dpkg) and rpmvercmp from libalpm (pacman). They decide which
packages retention keeps. tests/pkgrepo-containers.sh compares them with `rpm`, `dpkg` and `vercmp` on real inputs.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from functools import cmp_to_key
from typing import Callable

STABLE = "stable"
NIGHTLY = "nightly"
CHANNELS = (STABLE, NIGHTLY)

_SEMVER_CORE = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")


def parse_core(version: str) -> tuple[int, int, int]:
    match = _SEMVER_CORE.match(version)
    if not match:
        raise ValueError(f"{version!r} is not a plain MAJOR.MINOR.PATCH version")
    return int(match[1]), int(match[2]), int(match[3])


def next_stable(last_stable: str, tree_version: str | None = None) -> str:
    """The version the next stable release will have, which is the base of every nightly until it is tagged.

    A release commit writes the new version into the tree and the tag lands on that commit, so between releases the tree
    still carries the last stable version. Then the next release is the following patch number. When the tree already
    carries a higher version (a release is being prepared), that version wins.
    """
    last = parse_core(last_stable)
    if tree_version is not None and parse_core(tree_version) > last:
        return tree_version
    return f"{last[0]}.{last[1]}.{last[2] + 1}"


@dataclass(frozen=True)
class NightlyVersion:
    """One nightly build, spelled for each package manager."""

    base: str
    date: str
    count: int
    commit: str

    @property
    def rpm(self) -> str:
        # The tilde sorts before the empty string, so 0.1.13~nightly... is older than 0.1.13 and newer than 0.1.12.
        return f"{self.base}~nightly.{self.date}.{self.count}.g{self.commit}"

    @property
    def deb(self) -> str:
        return self.rpm

    @property
    def arch(self) -> str:
        # pacman has no tilde. Its rule is that a trailing letter segment sorts below the same version without it
        # (1.0a < 1.0), but only when the letters follow the number with no separator (1.0.a is newer than 1.0).
        return f"{self.base}nightly{self.date}.{self.count}"

    @property
    def semver(self) -> str:
        return f"{self.base}-nightly.{self.date}.{self.count}.g{self.commit}"


def nightly(last_stable: str, tree_version: str | None, date: str, count: int, commit: str) -> NightlyVersion:
    if not re.fullmatch(r"[0-9]{8}", date):
        raise ValueError(f"the nightly date must be YYYYMMDD, got {date!r}")
    if count < 0:
        raise ValueError("the commit count must not be negative")
    if not re.fullmatch(r"[0-9a-f]{7,40}", commit):
        raise ValueError(f"the commit must be a lower-case hexadecimal abbreviation, got {commit!r}")
    return NightlyVersion(next_stable(last_stable, tree_version), date, count, commit[:7])


_SEMVER_PRERELEASE = re.compile(r"^(\d+\.\d+\.\d+)(?:-([0-9A-Za-z.-]+))?$")


def semver_to_rpm(semver: str) -> str:
    """0.1.13-nightly.20261031.142.gabc1234 -> 0.1.13~nightly.20261031.142.gabc1234; RPM and dpkg versions allow no hyphen."""
    match = _SEMVER_PRERELEASE.match(semver)
    if not match:
        raise ValueError(f"{semver!r} is not MAJOR.MINOR.PATCH[-prerelease]")
    return semver if match[2] is None else f"{match[1]}~{match[2]}"


def semver_to_arch(semver: str) -> str:
    """0.1.13-nightly.20261031.142.gabc1234 -> 0.1.13nightly20261031.142 (see NightlyVersion.arch for why).

    The commit identifier (g plus hexadecimal) is dropped: pacman orders versions only by their segments, and the
    date and the commit count already order builds. A letter identifier is joined to a number that follows it.
    """
    match = _SEMVER_PRERELEASE.match(semver)
    if not match:
        raise ValueError(f"{semver!r} is not MAJOR.MINOR.PATCH[-prerelease]")
    if match[2] is None:
        return semver
    tokens = [t for t in match[2].split(".") if not re.fullmatch(r"g[0-9a-f]{7,40}", t)]
    text = tokens[0]
    for previous, token in zip(tokens, tokens[1:]):
        text += token if previous.isalpha() and token.isdigit() else "." + token
    return match[1] + text


# --- RPM -------------------------------------------------------------------------------------------------------------


def _isalnum(char: str) -> bool:
    return char.isascii() and char.isalnum()


def rpmvercmp(a: str, b: str) -> int:
    """RPM's version comparison, including the tilde and caret rules."""
    if a == b:
        return 0
    i = j = 0
    while i < len(a) or j < len(b):
        while i < len(a) and not (_isalnum(a[i]) or a[i] in "~^"):
            i += 1
        while j < len(b) and not (_isalnum(b[j]) or b[j] in "~^"):
            j += 1
        a_tilde = i < len(a) and a[i] == "~"
        b_tilde = j < len(b) and b[j] == "~"
        if a_tilde or b_tilde:
            if not a_tilde:
                return 1
            if not b_tilde:
                return -1
            i += 1
            j += 1
            continue
        a_caret = i < len(a) and a[i] == "^"
        b_caret = j < len(b) and b[j] == "^"
        if a_caret or b_caret:
            if i >= len(a):
                return -1
            if j >= len(b):
                return 1
            if not a_caret:
                return 1
            if not b_caret:
                return -1
            i += 1
            j += 1
            continue
        if not (i < len(a) and j < len(b)):
            break
        start_a, start_b = i, j
        if a[i].isdigit():
            while i < len(a) and a[i].isascii() and a[i].isdigit():
                i += 1
            while j < len(b) and b[j].isascii() and b[j].isdigit():
                j += 1
            numeric = True
        else:
            while i < len(a) and a[i].isascii() and a[i].isalpha():
                i += 1
            while j < len(b) and b[j].isascii() and b[j].isalpha():
                j += 1
            numeric = False
        segment_a, segment_b = a[start_a:i], b[start_b:j]
        if not segment_a:
            return -1
        if not segment_b:
            return 1 if numeric else -1
        if numeric:
            segment_a, segment_b = segment_a.lstrip("0"), segment_b.lstrip("0")
            if len(segment_a) != len(segment_b):
                return 1 if len(segment_a) > len(segment_b) else -1
        if segment_a != segment_b:
            return 1 if segment_a > segment_b else -1
    if i >= len(a) and j >= len(b):
        return 0
    return -1 if i >= len(a) else 1


def rpm_evr_compare(a: tuple[int, str, str], b: tuple[int, str, str]) -> int:
    """Compare (epoch, version, release) the way RPM does."""
    if a[0] != b[0]:
        return 1 if a[0] > b[0] else -1
    return rpmvercmp(a[1], b[1]) or rpmvercmp(a[2], b[2])


# --- dpkg ------------------------------------------------------------------------------------------------------------


def _dpkg_order(char: str) -> int:
    if char.isascii() and char.isdigit():
        return 0
    if char.isascii() and char.isalpha():
        return ord(char)
    if char == "~":
        return -1
    if char:
        return ord(char) + 256
    return 0


def _verrevcmp(a: str, b: str) -> int:
    i = j = 0
    while i < len(a) or j < len(b):
        first_diff = 0
        while (i < len(a) and not a[i].isdigit()) or (j < len(b) and not b[j].isdigit()):
            order_a = _dpkg_order(a[i]) if i < len(a) else 0
            order_b = _dpkg_order(b[j]) if j < len(b) else 0
            if order_a != order_b:
                return order_a - order_b
            i += 1
            j += 1
        while i < len(a) and a[i] == "0":
            i += 1
        while j < len(b) and b[j] == "0":
            j += 1
        while i < len(a) and a[i].isdigit() and j < len(b) and b[j].isdigit():
            if not first_diff:
                first_diff = ord(a[i]) - ord(b[j])
            i += 1
            j += 1
        if i < len(a) and a[i].isdigit():
            return 1
        if j < len(b) and b[j].isdigit():
            return -1
        if first_diff:
            return first_diff
    return 0


def _split_deb(version: str) -> tuple[int, str, str]:
    epoch = 0
    if ":" in version:
        head, version = version.split(":", 1)
        epoch = int(head)
    revision = ""
    if "-" in version:
        version, revision = version.rsplit("-", 1)
    return epoch, version, revision


def dpkg_compare(a: str, b: str) -> int:
    """Compare two Debian version strings (`[epoch:]upstream[-revision]`)."""
    epoch_a, upstream_a, revision_a = _split_deb(a)
    epoch_b, upstream_b, revision_b = _split_deb(b)
    if epoch_a != epoch_b:
        return 1 if epoch_a > epoch_b else -1
    for left, right in ((upstream_a, upstream_b), (revision_a, revision_b)):
        result = _verrevcmp(left, right)
        if result:
            return 1 if result > 0 else -1
    return 0


# --- pacman ----------------------------------------------------------------------------------------------------------


def alpm_rpmvercmp(a: str, b: str) -> int:
    """The segment comparison of libalpm, which differs from RPM's: no tilde, and a trailing letter segment is older."""
    if a == b:
        return 0
    i = j = 0
    previous_a = previous_b = 0
    while i < len(a) and j < len(b):
        while i < len(a) and not _isalnum(a[i]):
            i += 1
        while j < len(b) and not _isalnum(b[j]):
            j += 1
        if i >= len(a) or j >= len(b):
            break
        # Different separator lengths between two segments end the comparison.
        if i - previous_a != j - previous_b:
            return -1 if i - previous_a < j - previous_b else 1
        start_a, start_b = i, j
        if a[i].isdigit():
            while i < len(a) and a[i].isascii() and a[i].isdigit():
                i += 1
            while j < len(b) and b[j].isascii() and b[j].isdigit():
                j += 1
            numeric = True
        else:
            while i < len(a) and a[i].isascii() and a[i].isalpha():
                i += 1
            while j < len(b) and b[j].isascii() and b[j].isalpha():
                j += 1
            numeric = False
        previous_a, previous_b = i, j
        segment_a, segment_b = a[start_a:i], b[start_b:j]
        if not segment_b:
            return 1 if numeric else -1
        if numeric:
            segment_a, segment_b = segment_a.lstrip("0"), segment_b.lstrip("0")
            if len(segment_a) != len(segment_b):
                return 1 if len(segment_a) > len(segment_b) else -1
        if segment_a != segment_b:
            return 1 if segment_a > segment_b else -1
    rest_a, rest_b = a[i:], b[j:]
    if not rest_a and not rest_b:
        return 0
    # Whatever is left decides: a remaining letter segment is older than nothing, anything else is newer.
    if (not rest_a and not rest_b[0].isalpha()) or (rest_a and rest_a[0].isalpha()):
        return -1
    return 1


def _split_evr(version: str) -> tuple[str, str, str | None]:
    epoch = "0"
    if ":" in version and version.split(":", 1)[0].isdigit():
        epoch, version = version.split(":", 1)
    release = None
    if "-" in version:
        version, release = version.rsplit("-", 1)
    return epoch, version, release


def pacman_compare(a: str, b: str) -> int:
    """Compare `[epoch:]pkgver[-pkgrel]` the way pacman's vercmp does."""
    if a == b:
        return 0
    epoch_a, version_a, release_a = _split_evr(a)
    epoch_b, version_b, release_b = _split_evr(b)
    result = alpm_rpmvercmp(epoch_a, epoch_b)
    if result == 0:
        result = alpm_rpmvercmp(version_a, version_b)
        if result == 0 and release_a is not None and release_b is not None:
            result = alpm_rpmvercmp(release_a, release_b)
    return result


# --- ordering helpers ------------------------------------------------------------------------------------------------

Comparator = Callable[[str, str], int]


def comparator(package_format: str) -> Comparator:
    if package_format == "rpm":
        return lambda a, b: rpm_evr_compare(_parse_rpm(a), _parse_rpm(b))
    if package_format == "deb":
        return dpkg_compare
    if package_format == "arch":
        return pacman_compare
    raise ValueError(f"unknown package format {package_format!r}")


def _parse_rpm(evr: str) -> tuple[int, str, str]:
    epoch = 0
    if ":" in evr:
        head, evr = evr.split(":", 1)
        epoch = int(head)
    version, _, release = evr.rpartition("-")
    if not version:
        version, release = release, ""
    return epoch, version, release


def sort_versions(package_format: str, versions: list[str]) -> list[str]:
    """Oldest first."""
    return sorted(versions, key=cmp_to_key(comparator(package_format)))
