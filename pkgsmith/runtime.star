# The compiled Gnoblin packages: gnoblin, the session, xdg-desktop-portal-gnoblin, its optional portal backend, and
# gnoblin-x11-frames, the optional window frames of X11 apps.
#
# All three come from one prefix, which scripts/build-prefix.sh builds with ./build.sh --layout system. Every file of the
# portal backend and of the frames client is named below. gnoblin is everything else, except the two configuration files
# that pkgsmith/integrations.star packages. Run it with scripts/build-packages.sh, or by hand:
#
#   pkgsmith plan . -f pkgsmith/runtime.star --prefix prefix --build-arg VERSION=0.1.13 --build-arg PORTAL_VERSION=51.0.0 \
#     --build-arg NEEDED_GNOBLIN=libc.so.6,libm.so.6 --build-arg NEEDED_PORTAL=libc.so.6 --build-arg NEEDED_FRAMES=libc.so.6
#
# The package names, the epoch, the Provides and Obsoletes are those of the Fedora and Arch packages that this script
# replaces, so a system that has them upgrades.

load("common.star", "ARCH", "DEB_COPYRIGHT", "EPOCH", "HOMEPAGE", "MAINTAINER", "RELEASE", "VERSION", "gnoblin_package", "lanes", "numeric_release", "release_for", "spell")
load("dependencies.star", "library_depends", "split_list")

PRIVATE = "/usr/lib/gnoblin"
SCHEMAS = PRIVATE + "/share/glib-2.0/schemas"

PORTAL_VERSION = arg("PORTAL_VERSION")
if not PORTAL_VERSION:
    fail("PORTAL_VERSION is missing. scripts/build-packages.sh reads it from src/gnome-versions.json.")
GLIBC = arg("GLIBC", "")
NEEDED_GNOBLIN = split_list(arg("NEEDED_GNOBLIN"))
NEEDED_PORTAL = split_list(arg("NEEDED_PORTAL"))
NEEDED_FRAMES = split_list(arg("NEEDED_FRAMES"))
if not NEEDED_GNOBLIN or not NEEDED_PORTAL or not NEEDED_FRAMES:
    fail("NEEDED_GNOBLIN, NEEDED_PORTAL and NEEDED_FRAMES are missing. scripts/build-packages.sh reads them from the programs in the prefix.")

# The files of the portal backend, as they read on the installed system.
PORTAL_PATHS = [
    PRIVATE + "/libexec/xdg-desktop-portal-gnoblin",
    PRIVATE + "/lib/systemd/user/xdg-desktop-portal-gnoblin.service",
    PRIVATE + "/share/applications/xdg-desktop-portal-gnoblin.desktop",
    PRIVATE + "/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service",
    SCHEMAS + "/xdg-desktop-portal-gnome.gschema.xml",
    PRIVATE + "/share/licenses/xdg-desktop-portal-gnoblin/**",
    PRIVATE + "/share/locale/*/LC_MESSAGES/xdg-desktop-portal-gnome.mo",
    PRIVATE + "/share/xdg-desktop-portal/portals/gnoblin.portal",
    "/usr/lib/systemd/user/xdg-desktop-portal-gnoblin.service",
    "/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service",
    "/usr/share/xdg-desktop-portal/portals/gnoblin.portal",
]

# The X11 frames client. Mutter starts it for Xwayland, and it draws the title bar and the borders of X11 windows. It is the
# only program of the prefix that links GTK 4, so it has a package of its own: gnoblin then asks the system for no GTK.
FRAMES_PATHS = [PRIVATE + "/libexec/mutter-x11-frames"]
# Configuration of the optional packages in pkgsmith/integrations.star.
INTEGRATION_PATHS = ["/etc/geoclue/**", "/etc/ananicy.d/**"]

# Only the private runtime is a folder of these packages. The public folders (/usr/share/dbus-1, /usr/lib/systemd/user and so
# on) belong to the system packages that every desktop has.
OWN_DIRS = [PRIVATE, PRIVATE + "/**"]

# The schema cache is generated on the system, because the two packages share one schema directory. The rpm owns the
# file without carrying it, so rpm removes it with the package. The other formats remove it in the script.
POST_INSTALL = "/usr/bin/glib-compile-schemas " + SCHEMAS + "\n"
POST_REMOVE = """dir=%s
if [ -d "$dir" ]; then
    if ls "$dir"/*.xml "$dir"/*.override >/dev/null 2>&1; then
        /usr/bin/glib-compile-schemas "$dir" || true
    else
        rm -f "$dir/gschemas.compiled"
    fi
    rmdir -p "$dir" 2>/dev/null || true
fi
""" % SCHEMAS

# Packages that carry data and tools, not libraries. PyGObject runs Mutter's gdctl (bin/gdctl), which the QEMU and
# window-effect tests use to set the monitor layout. The libraries come from the programs: see pkgsmith/dependencies.star.
# The version floors here and in PORTAL are those of the oldest systems of each format that the source builds on. For
# rpm, Arch and apk that is Fedora 41 and Alpine 3.21: GLib 2.82, gsettings-desktop-schemas 47.1, libwayland 1.23,
# PipeWire 1.2.7, GTK 4.16 and xdg-desktop-portal 1.18.4. For deb it is Ubuntu 24.04: GLib 2.80, gsettings-desktop-schemas
# 46.1 and GTK 4.14. GTK and libwayland have no symbol versions, so a prefix built on a newer system can need more: a
# Fedora prefix calls libwayland 1.24 and GTK 4.22 API.
CORE = {
    "fedora": [
        "bash",
        "python3",
        "python3-gobject-base",
        "dbus-tools",
        "dconf",
        "glib2 >= 2.82.0",
        "gsettings-desktop-schemas >= 47.1",
        "libwayland-client >= 1.23",
        "pipewire-libs >= 1.2.7",
        "polkit",
    ],
    "opensuse": [
        "bash",
        "python3",
        "python3-gobject",
        "dbus-1",
        "dconf",
        "glib2 >= 2.82",
        "glib2-tools >= 2.82",
        "gsettings-desktop-schemas >= 47.1",
        "libwayland-client0 >= 1.23",
        "libpipewire-0_3-0 >= 1.2.7",
        "polkit",
        "systemd",
    ],
    "deb": [
        "bash",
        "python3",
        "python3-gi",
        "dbus-bin",
        "dconf-gsettings-backend",
        "dconf-service",
        "libglib2.0-bin (>= 2.80.0)",
        "gsettings-desktop-schemas (>= 46.1)",
        "polkitd",
    ],
    "apk": [
        "bash",
        "python3",
        "py3-gobject3",
        "dbus",
        "dconf",
        "cmd:glib-compile-schemas",
        "gsettings-desktop-schemas",
        "polkit",
    ],
    "archlinux": [
        "bash",
        "python",
        "python-gobject",
        "dbus",
        "dconf",
        "glib2>=2.82.0",
        "gsettings-desktop-schemas>=47.1",
        "polkit",
    ],
}
PORTAL = {
    "fedora": ["gtk4 >= 4.16.0", "xdg-desktop-portal >= 1.18.4", "glib2 >= 2.82.0"],
    "opensuse": ["gtk4 >= 4.16.0", "xdg-desktop-portal >= 1.18.4", "glib2-tools >= 2.82"],
    "deb": ["libgtk-4-1 (>= 4.14.0)", "xdg-desktop-portal (>= 1.18.4)", "libglib2.0-bin (>= 2.80.0)"],
    "archlinux": ["gtk4>=4.16.0", "xdg-desktop-portal>=1.18.4", "glib2>=2.82.0"],
    "apk": ["xdg-desktop-portal", "cmd:glib-compile-schemas"],
}

# The GTK 4 floor of the frames client, which is the one of Mutter (meson.build, gtk4_req). The libraries that the client
# needs come from its NEEDED entries. gnoblin itself is added in frames_depends.
FRAMES = {
    "fedora": ["gtk4 >= 4.14.0"],
    "opensuse": ["gtk4 >= 4.14.0"],
    "deb": ["libgtk-4-1 (>= 4.14.0)"],
    "archlinux": ["gtk4>=4.14.0"],
    "apk": [],
}

# What gnoblin does about the frames package in the formats that cannot say "recommended": Arch (pkgsmith writes an
# optional dependency, which pacman prints and never installs) and apk (no weak dependency at all). While this is True,
# gnoblin DEPENDS on gnoblin-x11-frames in those two formats, so a plain `pacman -S gnoblin` or `apk add gnoblin` keeps the
# title bars of X11 apps and installs GTK 4. Mutter makes no frame without the client, and some X11 windows may then fail
# to open. Nobody has tested that case in a graphical session. Flip this to False, so that those two formats leave the
# package optional, only when the missing-helper path is tested or Mutter is patched to run without the client. rpm and
# deb are not affected: they recommend the package, and a user can refuse a recommendation. tar has no dependencies.
FRAMES_HARD_DEPENDENCY_ON_NO_WEAK_DEP_FORMATS = True

def table_key(fmt, distro):
    return distro if distro else fmt

def lane_fields(fmt, distro):
    return {"distro": distro} if distro else {}

def scripts_for(fmt, install, remove):
    if fmt in ["rpm", "deb", "archlinux", "apk"]:
        return {"post_install": install, "post_remove": remove}
    return {}

# The libraries that the programs need, the packages that bring the data and the tools, and the version floor on
# GTK that only the portal backend and the frames client have.
def core_depends(fmt, distro):
    if fmt == "tar":
        return []
    return library_depends(fmt, NEEDED_GNOBLIN, GLIBC, distro) + CORE[table_key(fmt, distro)] + core_frames(fmt)

# The hard dependency on the frames package, for Arch and apk while the constant above is True.
def core_frames(fmt):
    if not frames_hard_dependency(fmt):
        return []
    if fmt == "archlinux":
        return ["gnoblin-x11-frames=" + str(EPOCH) + ":" + spell(fmt, VERSION) + "-" + RELEASE]
    return ["gnoblin-x11-frames"]

def portal_depends(fmt, distro):
    if fmt == "tar":
        return []
    libraries = library_depends(fmt, NEEDED_PORTAL, GLIBC, distro)
    if fmt == "deb":
        libraries = [name for name in libraries if not name.startswith("libgtk-4-1")]
    if fmt == "archlinux":
        libraries = [name for name in libraries if not name.startswith("gtk4")]
    return libraries + PORTAL[table_key(fmt, distro)]

# The frames client talks to Mutter through X properties (_MUTTER_NEEDS_FRAME, _MUTTER_FRAME_FOR, _MUTTER_FRAME_EXTENTS)
# and Mutter starts it from its own libexec folder. So it needs the exact gnoblin build that it was made with. apk spells
# a nightly version in a way that common.star does not know, so apk names the package only. Where gnoblin depends on the
# frames package (see the constant above), it pins the build from that side, and this package asks for nothing, because
# two packages that need each other make a dependency cycle.
def frames_hard_dependency(fmt):
    return FRAMES_HARD_DEPENDENCY_ON_NO_WEAK_DEP_FORMATS and fmt in ["archlinux", "apk"]

def frames_gnoblin(fmt, distro):
    if frames_hard_dependency(fmt):
        return []
    version = spell(fmt, VERSION)
    if fmt == "rpm":
        return ["gnoblin = " + str(EPOCH) + ":" + version + "-" + release_for(distro, RELEASE)]
    if fmt == "deb":
        return ["gnoblin (= " + str(EPOCH) + ":" + version + "-" + RELEASE + ")"]
    if fmt == "archlinux":
        return ["gnoblin=" + str(EPOCH) + ":" + version + "-" + RELEASE]
    if fmt == "apk":
        return ["gnoblin"]
    return []

def frames_depends(fmt, distro):
    if fmt == "tar":
        return []
    libraries = library_depends(fmt, NEEDED_FRAMES, GLIBC, distro)
    if fmt == "deb":
        libraries = [name for name in libraries if not name.startswith("libgtk-4-1")]
    if fmt == "archlinux":
        libraries = [name for name in libraries if not name.startswith("gtk4")]
    return libraries + FRAMES[table_key(fmt, distro)] + frames_gnoblin(fmt, distro)

# The frames package takes over the file from the gnoblin package of the earlier releases, which carried it. dpkg refuses
# a file that two packages hold unless the new package says so. rpm and pacman replace the file in one transaction.
def frames_relations(fmt):
    if fmt == "deb":
        return {"replaces": ["gnoblin (<< " + str(EPOCH) + ":" + spell(fmt, VERSION) + ")"]}
    return {}

# Names that this package takes over from, and the capability it keeps for packages that ask for it.
def core_relations(fmt):
    version = spell(fmt, VERSION)
    if fmt == "rpm":
        return {
            "provides": ["gnoblin-session = " + version],
            "replaces": ["gnoblin-session < 51", "gnoblin-shell < 51", "gnoblin-mutter < 52", "gnoblin-mutter-devel < 52"],
        }
    if fmt == "deb":
        old = ["gnoblin-session (<< 51)", "gnoblin-shell (<< 51)", "gnoblin-mutter (<< 52)", "gnoblin-mutter-devel (<< 52)"]
        return {"provides": ["gnoblin-session (= " + version + ")"], "replaces": old, "conflicts": old}
    if fmt == "archlinux":
        old = ["gnoblin-mutter", "gnoblin-mutter-devel"]
        return {"provides": ["gnoblin-session=" + version], "replaces": old, "conflicts": old}
    return {}

# The weak dependency of gnoblin on the frames package. rpm and deb install a recommended package by default, and a user
# who wants no GTK can refuse it. Arch has no such dependency: pkgsmith writes it as an optional dependency, which pacman
# prints and does not install. So Arch gets the optional dependency only when gnoblin does not depend on the package.
# apk and tar have no weak dependency at all, so they say nothing.
def core_recommends(fmt):
    if fmt in ["rpm", "deb"]:
        return {"recommends": ["gnoblin-x11-frames"]}
    if fmt == "archlinux" and not frames_hard_dependency(fmt):
        return {"recommends": ["gnoblin-x11-frames: title bars and borders for X11 apps under Xwayland"]}
    return {}

CORE_DESCRIPTION = """Standalone Gnoblin desktop session
Installs the Lua-supervised Gnoblin session, the private runtime tools, the login entry and the patched
Mutter runtime in one package. The package itself needs no GTK. The title bars of X11 apps come from
gnoblin-x11-frames, which it recommends (on Arch and Alpine it depends on it), and the GTK-based Gnoblin portal
backend is the optional xdg-desktop-portal-gnoblin. It replaces the earlier gnoblin-session, gnoblin-mutter and gnoblin-mutter-devel packages on upgrade."""

FRAMES_DESCRIPTION = """Window frames for X11 apps in the Gnoblin session
Draws the title bar and the borders of X11 apps that run under Xwayland. Mutter starts this client for each X11 display.
Without it, X11 apps that need Mutter to draw their frame get no title bar and no borders. Wayland apps and apps that
draw their own title bar are not affected. This is the only part of Gnoblin that needs GTK 4."""

PORTAL_DESCRIPTION = """Gnoblin desktop portal backend
Provides a separate portal backend for Gnoblin sessions. An existing GNOME portal service remains available for GNOME
logins."""

# gnoblin: everything except the portal backend, the frames client and the two optional configuration packages.
exclude(PORTAL_PATHS + FRAMES_PATHS + INTEGRATION_PATHS)
for lane in lanes():
    fmt = lane[0]
    distro = lane[1]
    fields = lane_fields(fmt, distro)
    contents = []
    if fmt == "rpm":
        contents.append(ghost(SCHEMAS + "/gschemas.compiled"))
    gnoblin_package(
        fmt,
        name = "gnoblin",
        version = VERSION,
        release = release_for(distro, RELEASE),
        epoch = EPOCH,
        arch = ARCH,
        description = CORE_DESCRIPTION,
        maintainer = MAINTAINER,
        homepage = HOMEPAGE,
        license = "GPL-2.0-or-later",
        depends = core_depends(fmt, distro),
        scripts = scripts_for(fmt, POST_INSTALL, POST_REMOVE),
        own_dirs = OWN_DIRS,
        contents = contents,
        **dict(fields.items() + core_relations(fmt).items() + core_recommends(fmt).items())
    )

# xdg-desktop-portal-gnoblin: the backend keeps the version of its GNOME source. Its release number follows the Gnoblin
# release, so each Gnoblin release has a distinct portal package.
reset()
include(PORTAL_PATHS + [DEB_COPYRIGHT])
for lane in lanes():
    fmt = lane[0]
    distro = lane[1]
    fields = lane_fields(fmt, distro)
    gnoblin_package(
        fmt,
        name = "xdg-desktop-portal-gnoblin",
        version = PORTAL_VERSION,
        release = release_for(distro, numeric_release(VERSION)),
        arch = ARCH,
        description = PORTAL_DESCRIPTION,
        maintainer = MAINTAINER,
        homepage = HOMEPAGE,
        license = "LGPL-2.1-or-later",
        depends = portal_depends(fmt, distro),
        scripts = scripts_for(fmt, POST_INSTALL, POST_REMOVE),
        own_dirs = OWN_DIRS,
        **fields
    )

# gnoblin-x11-frames: the X11 frames client of Mutter. It has the version of Gnoblin.
reset()
include(FRAMES_PATHS + [DEB_COPYRIGHT])
for lane in lanes():
    fmt = lane[0]
    distro = lane[1]
    fields = lane_fields(fmt, distro)
    gnoblin_package(
        fmt,
        name = "gnoblin-x11-frames",
        version = VERSION,
        release = release_for(distro, RELEASE),
        epoch = EPOCH,
        arch = ARCH,
        description = FRAMES_DESCRIPTION,
        maintainer = MAINTAINER,
        homepage = HOMEPAGE,
        license = "GPL-2.0-or-later",
        depends = frames_depends(fmt, distro),
        **dict(fields.items() + frames_relations(fmt).items())
    )
