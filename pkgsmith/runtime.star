# The compiled Gnoblin packages: gnoblin, the session, and xdg-desktop-portal-gnoblin, its optional portal backend.
#
# Both come from one prefix, which scripts/build-prefix.sh builds with ./build.sh --layout system. Every file of the
# portal backend is named below. gnoblin is everything else, except the two configuration files that pkgsmith/integrations.star
# packages. Run it with scripts/build-packages.sh, or by hand:
#
#   pkgsmith plan . -f pkgsmith/runtime.star --prefix prefix --build-arg VERSION=0.1.13 --build-arg PORTAL_VERSION=51.0.0 \
#     --build-arg NEEDED_GNOBLIN=libc.so.6,libm.so.6 --build-arg NEEDED_PORTAL=libc.so.6
#
# The package names, the epoch, the Provides and Obsoletes are those of the Fedora and Arch packages that this script
# replaces, so a system that has them upgrades.

load("common.star", "ARCH", "EPOCH", "FEDORA", "FORMATS", "HOMEPAGE", "MAINTAINER", "RELEASE", "VERSION", "numeric_release", "spell")
load("dependencies.star", "library_depends", "split_list")

PRIVATE = "/usr/lib/gnoblin"
SCHEMAS = PRIVATE + "/share/glib-2.0/schemas"

PORTAL_VERSION = arg("PORTAL_VERSION")
if not PORTAL_VERSION:
    fail("PORTAL_VERSION is missing. scripts/build-packages.sh reads it from src/gnome-versions.json.")
GLIBC = arg("GLIBC", "")
NEEDED_GNOBLIN = split_list(arg("NEEDED_GNOBLIN"))
NEEDED_PORTAL = split_list(arg("NEEDED_PORTAL"))
if not NEEDED_GNOBLIN or not NEEDED_PORTAL:
    fail("NEEDED_GNOBLIN and NEEDED_PORTAL are missing. scripts/build-packages.sh reads them from the programs in the prefix.")

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
# Configuration of the optional packages in pkgsmith/integrations.star.
INTEGRATION_PATHS = ["/etc/geoclue", "/etc/ananicy.d"]

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

# Packages that carry data and tools, not libraries. The libraries come from the programs: see pkgsmith/dependencies.star.
CORE = {
    "fedora": [
        "bash",
        "python3",
        "adwaita-cursor-theme",
        "dbus-tools",
        "dconf",
        "glib2 >= 2.86.0",
        "gsettings-desktop-schemas >= 49.1",
        "libwayland-client >= 1.25",
        "pipewire >= 1.4.11",
        "polkit",
        "wireplumber",
    ],
    "opensuse": [
        "bash",
        "python3",
        "adwaita-icon-theme",
        "dbus-1",
        "dconf",
        "glib2 >= 2.86",
        "glib2-tools >= 2.86",
        "gsettings-desktop-schemas >= 49.1",
        "libwayland-client0 >= 1.25",
        "pipewire >= 1.4.11",
        "polkit",
        "systemd",
        "wireplumber",
    ],
    "deb": [
        "bash",
        "python3",
        "adwaita-icon-theme",
        "dbus-bin",
        "dconf-gsettings-backend",
        "dconf-service",
        "libglib2.0-bin (>= 2.86.0)",
        "gsettings-desktop-schemas (>= 49.1)",
        "pipewire (>= 1.4.11)",
        "polkitd",
        "wireplumber",
    ],
    "apk": [
        "bash",
        "python3",
        "adwaita-icon-theme",
        "dbus",
        "dconf",
        "cmd:glib-compile-schemas",
        "gsettings-desktop-schemas",
        "pipewire",
        "polkit",
        "wireplumber",
    ],
    "archlinux": [
        "bash",
        "python",
        "adwaita-cursors",
        "dbus",
        "dconf",
        "glib2>=2.86.0",
        "gsettings-desktop-schemas>=49.1",
        "pipewire>=1.4.11",
        "polkit",
        "wireplumber",
    ],
}
PORTAL = {
    "fedora": ["gtk4 >= 4.20.0", "xdg-desktop-portal >= 1.20.0", "glib2 >= 2.86.0", "libsecret"],
    "opensuse": ["gtk4 >= 4.20.0", "xdg-desktop-portal >= 1.20.0", "glib2-tools >= 2.86", "libsecret-1-0"],
    "deb": ["libgtk-4-1 (>= 4.20.0)", "xdg-desktop-portal (>= 1.20.0)", "libglib2.0-bin (>= 2.86.0)", "libsecret-1-0"],
    "archlinux": ["gtk4>=4.20.0", "xdg-desktop-portal>=1.20.0", "glib2>=2.86.0", "libsecret"],
    "apk": ["xdg-desktop-portal", "cmd:glib-compile-schemas", "libsecret"],
}

def lanes():
    """Each package is written once for every lane: Fedora and openSUSE are two rpm lanes, the others have one."""
    result = []
    for fmt in FORMATS:
        if fmt == "rpm":
            result.append(("rpm", "fedora"))
            result.append(("rpm", "opensuse"))
        else:
            result.append((fmt, None))
    return result

def table_key(fmt, distro):
    return distro if distro else fmt

def release_for(distro, number):
    if distro == "fedora":
        return str(number) + ".fc" + FEDORA
    if distro == "opensuse":
        return str(number) + ".tumbleweed"
    return str(number)

def lane_fields(fmt, distro):
    return {"distro": distro} if distro else {}

def scripts_for(fmt, install, remove):
    if fmt in ["rpm", "deb", "archlinux", "apk"]:
        return {"post_install": install, "post_remove": remove}
    return {}

# The libraries that the programs need, the packages that bring the data and the tools, and the version floor on
# GTK that only the portal backend has.
def core_depends(fmt, distro):
    if fmt == "tar":
        return []
    return library_depends(fmt, NEEDED_GNOBLIN, GLIBC) + CORE[table_key(fmt, distro)]

def portal_depends(fmt, distro):
    if fmt == "tar":
        return []
    libraries = library_depends(fmt, NEEDED_PORTAL, GLIBC)
    if fmt == "deb":
        libraries = [name for name in libraries if not name.startswith("libgtk-4-1")]
    if fmt == "archlinux":
        libraries = [name for name in libraries if not name.startswith("gtk4")]
    return libraries + PORTAL[table_key(fmt, distro)]

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

CORE_DESCRIPTION = """Standalone Gnoblin desktop session
Installs the Lua-supervised Gnoblin session, the private runtime tools, the login entry and the patched
Mutter runtime in one package. It does not require GNOME Shell or GJS. The GTK-based Gnoblin portal backend is
optional. It replaces the earlier gnoblin-session, gnoblin-mutter and gnoblin-mutter-devel packages on upgrade."""

PORTAL_DESCRIPTION = """Gnoblin desktop portal backend
Provides a separate portal backend for Gnoblin sessions. An existing GNOME portal service remains available for GNOME
logins."""

# gnoblin: everything except the portal backend and the two optional configuration packages.
exclude(PORTAL_PATHS + INTEGRATION_PATHS)
for lane in lanes():
    fmt = lane[0]
    distro = lane[1]
    fields = lane_fields(fmt, distro)
    contents = []
    if fmt == "rpm":
        contents.append(ghost(SCHEMAS + "/gschemas.compiled"))
    package(
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
        contents = contents,
        **dict(fields.items() + core_relations(fmt).items())
    )

# xdg-desktop-portal-gnoblin: the backend keeps the version of its GNOME source. Its release number follows the Gnoblin
# release, so each Gnoblin release has a distinct portal package.
reset()
include(PORTAL_PATHS)
for lane in lanes():
    fmt = lane[0]
    distro = lane[1]
    fields = lane_fields(fmt, distro)
    package(
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
        **fields
    )
