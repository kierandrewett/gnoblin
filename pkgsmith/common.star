# Shared by the pkgsmith scripts of this repository: the build arguments, the identity of every package, and the
# spelling of a version in the dependency lines that name another Gnoblin package.
#
# scripts/build-packages.sh passes the build arguments. To run pkgsmith by hand, see docs/distribution.md.

MAINTAINER = "Kieran Drewett <kieran@drewett.dev>"
HOMEPAGE = "https://github.com/kierandrewett/gnoblin"

# The Gnoblin version, as SemVer: 0.1.13, or a nightly such as 0.1.13-nightly.20261031.142.gabc1234.
VERSION = arg("VERSION")
if not VERSION:
    fail("VERSION is missing. Pass --build-arg VERSION=0.1.13, or run scripts/build-packages.sh, which reads it from the tree.")

# The architecture and the C library of the files in the prefix. pkgsmith does not look inside them.
ARCH = arg("ARCH", host()["arch"])
LIBC = arg("LIBC", "glibc")

# The revision of one version, for a packaging fix that does not change the version. It starts at 1 for each version.
RELEASE = arg("RELEASE", "1")

# The Fedora release that the prefix was built on, which is the one the Fedora packages are for. The libraries and the
# glibc symbol versions in the programs come from that release, so the rpm asks for what that release has. It is empty
# for a run that writes no Fedora package.
FEDORA = arg("FEDORA", "")

# Which packages this run writes: "fedora" is the rpm for FEDORA only, "other" is everything except the Fedora rpm, and
# "all" is both. scripts/build-packages.sh runs the Fedora lane once for each Fedora prefix and the other lane once, from
# the prefix that the deb, Arch, tar and openSUSE packages come from.
LANES = arg("LANES", "all")
if LANES not in ["all", "fedora", "other"]:
    fail("LANES is " + LANES + ", which is not all, fedora or other.")
if LANES != "other" and not FEDORA.isdigit():
    fail("FEDORA is missing. Pass --build-arg FEDORA=43, or run scripts/build-packages.sh, which takes --fedora.")

# Every package of this repository carries epoch 1 in the formats that have one, as the Fedora packages always did.
# A higher epoch makes the packages of the new build system upgrade the ones that the old recipes made, whatever
# their version numbers.
EPOCH = 1

# Formats a prefix can feed. A glibc prefix does not run on Alpine, which uses musl, so only a prefix built on Alpine makes
# an apk. See docs/distribution.md.
FORMATS = {
    "glibc": ["rpm", "deb", "archlinux", "tar"],
    "musl": ["apk"],
}[LIBC]

def lanes():
    """The (format, distro) pairs that this run writes. Fedora and openSUSE are two rpm lanes, the other formats have one."""
    result = []
    for fmt in FORMATS:
        if fmt == "rpm":
            if LANES != "other":
                result.append(("rpm", "fedora"))
            if LANES != "fedora":
                result.append(("rpm", "opensuse"))
        elif LANES != "fedora":
            result.append((fmt, None))
    if not result:
        fail("LANES=" + LANES + " writes nothing for a " + LIBC + " prefix.")
    return result

def _is_commit(text):
    # A short or long Git hash, written with a leading g: gabc1234.
    if not text.startswith("g") or len(text) < 8 or len(text) > 41:
        return False
    for char in text[1:].elems():
        if char not in "0123456789abcdef":
            return False
    return True

def spell(fmt, version):
    """Write a version the way pkgsmith writes it in the package of this format, without epoch and release.

    0.1.13 stays 0.1.13. 0.1.13-nightly.20261031.142.gabc1234 is 0.1.13~nightly.20261031.142.gabc1234 in rpm and deb, and
    0.1.13nightly20261031.142 in Arch, which has no tilde. See pkgsmith's docs/versions.md.
    """
    if "-" not in version:
        return version
    base, pre = version.split("-", 1)
    if fmt in ["rpm", "deb"]:
        return base + "~" + pre
    if fmt == "archlinux":
        parts = [part for part in pre.split(".") if not _is_commit(part)]
        return base + parts[0] + ".".join(parts[1:])
    return version

def numeric_release(version):
    """An increasing whole number for each Gnoblin release: 0.1.13 is 1013, 0.2.0 is 2000, 1.0.0 is 1000000.

    The portal backend keeps the version of its GNOME source (51.0.0), so its release number is what tells one Gnoblin
    release from the next. The stable repository refuses two packages with one version.
    """
    core = version.split("-", 1)[0].split(".")
    return int(core[0]) * 1000000 + int(core[1]) * 1000 + int(core[2])
