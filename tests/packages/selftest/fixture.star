# The fixture package set of the package suite self test (selftest.py): the four Gnoblin package names, with a few files
# each, written by the pinned pkgsmith from a fixture prefix that selftest.py makes. The packages are not runnable.

COMMON = {
    "version": "0.1.13",
    "release": "1",
    "maintainer": "Gnoblin package suite <noreply@example.invalid>",
    "homepage": "https://github.com/kierandrewett/gnoblin",
}

def fixture(fmt, name, license, arch, paths, version = "0.1.13", release = "1"):
    reset()
    include(paths)
    fields = dict(COMMON)
    fields["version"] = version
    fields["release"] = release
    package(fmt, name = name, arch = arch, license = license, description = "Package suite fixture " + name, **fields)

for fmt in ["archlinux", "tar"]:
    fixture(fmt, "gnoblin", "GPL-2.0-or-later", "amd64", ["/usr/lib/gnoblin/bin/**", "/usr/bin/**", "/usr/share/wayland-sessions/**"])
    fixture(fmt, "xdg-desktop-portal-gnoblin", "LGPL-2.1-or-later", "amd64", ["/usr/lib/gnoblin/libexec/xdg-desktop-portal-gnoblin"], version = "51.0.0", release = "1013")
    fixture(fmt, "gnoblin-geoclue-integration", "GPL-2.0-or-later", "all", ["/etc/geoclue/**"])
    fixture(fmt, "gnoblin-ananicy-rules", "GPL-2.0-or-later", "all", ["/etc/ananicy.d/**"])
