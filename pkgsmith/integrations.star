# The configuration-only packages: gnoblin-geoclue-integration and gnoblin-ananicy-rules.
#
# Both come from the prefix that scripts/build-prefix.sh builds: the build installs their files under /etc, and the
# gnoblin package in pkgsmith/runtime.star leaves them out. They hold no compiled code, so they are for every
# architecture, except that the GeoClue package names the exact gnoblin build it was made for.
# Run it with scripts/build-packages.sh.

load("common.star", "DEB_COPYRIGHT", "EPOCH", "HOMEPAGE", "MAINTAINER", "RELEASE", "VERSION", "gnoblin_package", "lanes", "release_for", "spell")

# gnoblin-geoclue-integration lets Gnoblin's location agent register with GeoClue. It needs the gnoblin build it was
# made for, so it names that build exactly. Its one file is a drop-in in /etc/geoclue/conf.d, which GeoClue reads since
# 2.7.0. The agent calls AddAgent and answers AuthorizeApp, which are older. So 2.7.0 is the floor.
GEOCLUE = "2.7.0"

def geoclue_depends(fmt, distro):
    version = spell(fmt, VERSION)
    if fmt == "rpm":
        gnoblin = "gnoblin = " + str(EPOCH) + ":" + version + "-" + release_for(distro, RELEASE)
        return [gnoblin, "geoclue2 >= " + GEOCLUE]
    if fmt == "deb":
        return ["gnoblin (= " + str(EPOCH) + ":" + version + "-" + RELEASE + ")", "geoclue-2.0 (>= " + GEOCLUE + ")"]
    if fmt == "archlinux":
        return ["gnoblin=" + str(EPOCH) + ":" + version + "-" + RELEASE, "geoclue>=" + GEOCLUE]
    return []

# gnoblin-ananicy-rules gives the Gnoblin processes normal scheduling under ananicy-cpp. The daemon reads its rules at
# start, so each install, upgrade and removal restarts it where it runs.
ANANICY_RESTART = "systemctl try-restart ananicy-cpp.service >/dev/null 2>&1 || true\n"

def ananicy_relations(fmt):
    if fmt == "archlinux":
        return {"recommends": ["ananicy-cpp: applies these rules"]}
    if fmt in ["rpm", "deb"]:
        return {"recommends": ["ananicy-cpp"]}
    return {}

GEOCLUE_DESCRIPTION = """Optional GeoClue agent authorization for Gnoblin
Allows Gnoblin's optional location authorization agent to register with GeoClue. Install this package only when the
session should use Gnoblin as its location agent."""

ANANICY_DESCRIPTION = """Ananicy-cpp rules for the Gnoblin compositor and shell
Ananicy-cpp's default rules rank gdm and its children as background work, with nice 16, idle I/O and SCHED_IDLE. A
Gnoblin session started by gdm inherits that class, and the compositor and shell then freeze or drop frames while other
programs use the CPU.

This package adds rules that give the gnoblin, quickshell and bingux-frame processes normal scheduling with a small
priority boost. It contains only data files for ananicy-cpp. It has no effect where ananicy-cpp is not installed."""

include("/etc/geoclue/**", DEB_COPYRIGHT)
for lane in lanes():
    fmt = lane[0]
    distro = lane[1]
    fields = {"distro": distro} if distro else {}
    gnoblin_package(
        fmt,
        name = "gnoblin-geoclue-integration",
        version = VERSION,
        release = release_for(distro, RELEASE),
        epoch = EPOCH,
        arch = "all",
        description = GEOCLUE_DESCRIPTION,
        maintainer = MAINTAINER,
        homepage = HOMEPAGE,
        license = "GPL-2.0-or-later",
        depends = geoclue_depends(fmt, distro),
        config_files = ["/etc/**"],
        own_dirs = ["/etc/geoclue", "/etc/geoclue/**"],
        **fields
    )

reset()
include("/etc/ananicy.d/**", DEB_COPYRIGHT)
for lane in lanes():
    fmt = lane[0]
    distro = lane[1]
    fields = {"distro": distro} if distro else {}
    scripts = {}
    if fmt in ["rpm", "deb", "archlinux", "apk"]:
        scripts = {"post_install": ANANICY_RESTART, "post_remove": ANANICY_RESTART}
    gnoblin_package(
        fmt,
        name = "gnoblin-ananicy-rules",
        version = VERSION,
        release = release_for(distro, RELEASE),
        arch = "all",
        description = ANANICY_DESCRIPTION,
        maintainer = MAINTAINER,
        homepage = HOMEPAGE,
        license = "GPL-2.0-or-later",
        config_files = ["/etc/**"],
        own_dirs = ["/etc/ananicy.d"],
        scripts = scripts,
        **dict(fields.items() + ananicy_relations(fmt).items())
    )
