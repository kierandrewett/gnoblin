# What the programs in the prefix need from the system, written for each package format.
#
# pkgsmith does not look inside binaries, so scripts/build-packages.sh reads the NEEDED entries of every ELF file in the
# prefix and passes them as build arguments (NEEDED_GNOBLIN, NEEDED_PORTAL). This file turns each library name into the
# package that ships it on each distribution. A library that is missing from the table stops the script, so a new
# library in the build cannot reach a package unnoticed.
#
# rpm names the library itself (libgtk-4.so.1()(64bit)), which Fedora and openSUSE both provide. deb and Arch have no
# such capability, so they get the package name. The packages that carry data and tools, such as dconf, polkit and
# wireplumber, are named by hand where each package is declared.

load("common.star", "LANES")

# The library, the Debian package that ships it, the Arch package that ships it. None: no package of that format ships it.
LIBRARIES = {
    "libadwaita-1.so.0": ("libadwaita-1-0", "libadwaita"),
    "libatk-1.0.so.0": ("libatk1.0-0t64", "at-spi2-core"),
    "libcairo.so.2": ("libcairo2", "cairo"),
    "libcanberra.so.0": ("libcanberra0", "libcanberra"),
    "libcolord.so.2": ("libcolord2", "libcolord"),
    "libc.so.6": ("libc6", "glibc"),
    "libdisplay-info.so.1": ("libdisplay-info1", None),
    "libdisplay-info.so.2": ("libdisplay-info2", None),
    "libdisplay-info.so.3": ("libdisplay-info3", "libdisplay-info"),
    "libdrm.so.2": ("libdrm2", "libdrm"),
    "libEGL.so.1": ("libegl1", "libglvnd"),
    "libeis.so.1": ("libeis1", "libei"),
    "libfontconfig.so.1": ("libfontconfig1", "fontconfig"),
    "libfribidi.so.0": ("libfribidi0", "fribidi"),
    "libgbm.so.1": ("libgbm1", "mesa"),
    "libgcc_s.so.1": ("libgcc-s1", "libgcc"),
    "libgcr-4.so.4": ("libgcr-4-4", "gcr-4"),
    "libgdk_pixbuf-2.0.so.0": ("libgdk-pixbuf-2.0-0", "gdk-pixbuf2"),
    "libgio-2.0.so.0": ("libglib2.0-0t64", "glib2"),
    "libGLESv2.so.2": ("libgles2", "libglvnd"),
    "libglib-2.0.so.0": ("libglib2.0-0t64", "glib2"),
    "libGL.so.1": ("libgl1", "libglvnd"),
    "libglycin-2.so.0": ("libglycin-2-0", "glycin"),
    "libgmodule-2.0.so.0": ("libglib2.0-0t64", "glib2"),
    "libgobject-2.0.so.0": ("libglib2.0-0t64", "glib2"),
    "libgraphene-1.0.so.0": ("libgraphene-1.0-0", "graphene"),
    "libgtk-4.so.1": ("libgtk-4-1", "gtk4"),
    "libgudev-1.0.so.0": ("libgudev-1.0-0", "libgudev"),
    "libharfbuzz.so.0": ("libharfbuzz0b", "harfbuzz"),
    "libibus-1.0.so.5": ("libibus-1.0-5", "libibus"),
    "libinput.so.10": ("libinput10", "libinput"),
    "libjson-glib-1.0.so.0": ("libjson-glib-1.0-0", "json-glib"),
    "liblcms2.so.2": ("liblcms2-2", "lcms2"),
    "libm.so.6": ("libc6", "glibc"),
    "libpango-1.0.so.0": ("libpango-1.0-0", "pango"),
    "libpangocairo-1.0.so.0": ("libpangocairo-1.0-0", "pango"),
    "libpipewire-0.3.so.0": ("libpipewire-0.3-0t64", "libpipewire"),
    "libpixman-1.so.0": ("libpixman-1-0", "pixman"),
    "libpolkit-agent-1.so.0": ("libpolkit-agent-1-0", "polkit"),
    "libpolkit-gobject-1.so.0": ("libpolkit-gobject-1-0", "polkit"),
    "libstartup-notification-1.so.0": ("libstartup-notification0", "startup-notification"),
    "libstdc++.so.6": ("libstdc++6", "libstdc++"),
    "libsystemd.so.0": ("libsystemd0", "systemd-libs"),
    "libudev.so.1": ("libudev1", "systemd-libs"),
    "libwacom.so.9": ("libwacom9", "libwacom"),
    "libwayland-client.so.0": ("libwayland-client0", "wayland"),
    "libwayland-server.so.0": ("libwayland-server0", "wayland"),
    "libX11.so.6": ("libx11-6", "libx11"),
    "libX11-xcb.so.1": ("libx11-xcb1", "libx11"),
    "libXau.so.6": ("libxau6", "libxau"),
    "libxcb-res.so.0": ("libxcb-res0", "libxcb"),
    "libxcb.so.1": ("libxcb1", "libxcb"),
    "libXcomposite.so.1": ("libxcomposite1", "libxcomposite"),
    "libXcursor.so.1": ("libxcursor1", "libxcursor"),
    "libXext.so.6": ("libxext6", "libxext"),
    "libXfixes.so.3": ("libxfixes3", "libxfixes"),
    "libXinerama.so.1": ("libxinerama1", "libxinerama"),
    "libXi.so.6": ("libxi6", "libxi"),
    "libxkbcommon.so.0": ("libxkbcommon0", "libxkbcommon"),
    "libxkbregistry.so.0": ("libxkbregistry0", "libxkbcommon"),
    "libXrandr.so.2": ("libxrandr2", "libxrandr"),
}

# The newest LIBINPUT_ symbol version that the programs use, such as 1.30. scripts/build-packages.sh reads it from the
# programs, as it reads the glibc floor, because it depends on the libinput of the host that built the prefix.
LIBINPUT = arg("LIBINPUT", "")

# Libraries whose version matters: the symbol version rpm checks, and a minimum for the distributions without symbol
# versions. Each entry is (rpm capability, Debian package, Arch package, minimum version). libwayland and PipeWire have
# no symbol versions, so their floor is the oldest release that the source builds against (Ubuntu 24.04 for both). A
# prefix built on a newer host can use newer API: a Fedora prefix needs libwayland 1.24, which this floor does not say.
FLOORS = [
    ("libwayland-server.so.0()(64bit)", "libwayland-server0", "wayland", "1.22.0"),
    ("libpipewire-0.3.so.0()(64bit)", "libpipewire-0.3-0t64", "libpipewire", "1.0.5"),
] + ([("libinput.so.10(LIBINPUT_" + LIBINPUT + ")(64bit)", "libinput10", "libinput", LIBINPUT)] if LIBINPUT else [])

# Libraries that an older release ships under an older soname. Ubuntu 24.04 has libdisplay-info 0.1 (.so.1). Fedora 42
# and 43, Debian 13 and openSUSE Leap 16.0 have libdisplay-info 0.2 (.so.2); Arch, Tumbleweed and newer Fedora and
# Debian releases have 0.3. An rpm names the library
# itself, so the rpm of a release lane may carry one: the Fedora rpm, and the rpm of the openSUSE lane, whose prefix was
# built on the Leap release it is for. The shared openSUSE rpm is for Tumbleweed, which has no such library. The deb names
# the Debian package from LIBRARIES. Arch has none (None).
OLDER_SONAMES = ["libdisplay-info.so.1", "libdisplay-info.so.2"]

def split_list(text):
    """A build argument that holds a comma separated list."""
    return [item for item in (text or "").split(",") if item]

def library_depends(fmt, sonames, glibc, distro = None):
    """The library requirements of one package in one format.

    sonames are the NEEDED entries of its programs. glibc is the newest glibc symbol version the programs use, such as 2.43,
    which becomes the floor on the C library. distro is "fedora" or "opensuse" for an rpm, and None for the other formats.
    """
    if fmt == "apk":
        # apk resolves a library by its soname, so no table is needed.
        return ["so:" + soname for soname in sonames]
    for soname in sonames:
        if soname in OLDER_SONAMES and fmt == "rpm":
            if distro == "opensuse" and LANES != "opensuse":
                fail("Tumbleweed ships no " + soname + ". A prefix that links it makes no shared openSUSE rpm. Build the rpm of an openSUSE Leap release with LANES=opensuse from a prefix built on that release.")
            continue
        if soname not in LIBRARIES:
            fail("no package is known for " + soname + ". Add it to LIBRARIES in pkgsmith/dependencies.star, with the Debian and Arch package that ship it.")
        if fmt in ["deb", "archlinux"] and LIBRARIES[soname][0 if fmt == "deb" else 1] == None:
            fail("no " + fmt + " package ships " + soname + ". A prefix that links it makes no " + fmt + " package.")
    if fmt == "rpm":
        result = []
        for soname in sonames:
            capability = soname + "()(64bit)"
            if soname == "libc.so.6" and glibc:
                capability = "libc.so.6(GLIBC_" + glibc + ")(64bit)"
            for floor in FLOORS:
                if floor[0].startswith(soname + "("):
                    capability = floor[0]
            if capability not in result:
                result.append(capability)
        return result
    column = 0 if fmt == "deb" else 1
    names = []
    for soname in sonames:
        name = LIBRARIES[soname][column]
        if name not in names:
            names.append(name)
    result = []
    for name in names:
        constraint = ""
        if name == LIBRARIES["libc.so.6"][column] and glibc:
            constraint = glibc
        for floor in FLOORS:
            if floor[1 + column] == name:
                constraint = floor[3]
        if not constraint:
            result.append(name)
        elif fmt == "deb":
            result.append(name + " (>= " + constraint + ")")
        else:
            result.append(name + ">=" + constraint)
    return result
