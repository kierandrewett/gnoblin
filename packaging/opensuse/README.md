# openSUSE Tumbleweed RPM adapter

These recipes package Gnoblin's GNOME 51 stack under `/usr/lib/gnoblin` on
Tumbleweed. The only shared paths are the `gnoblin` display-manager entry,
Gnoblin-named systemd user units, `gnoblinctl`, and the distinct Gnoblin polkit
action. The recipes do not replace, conflict with, obsolete, or provide stock
GNOME packages.

`check-buildrequires.sh` runs `rpmspec` and asks Zypper to resolve the host
dependencies in a clean Tumbleweed image. It deliberately omits the internal
Gnoblin schema and compositor packages because an OBS project must build those
in this order:

When installing the external requirements, it retries a failed transaction up
to three times and forces a repository refresh between attempts. Tumbleweed's
rolling mirrors can briefly advertise package metadata before all mirrors have
the corresponding RPMs.

1. `gnoblin-gsettings-desktop-schemas`
2. `gnoblin-mutter` and `gnoblin-mutter-devel`
3. `gnoblin-shell` and `gnoblin-session`
4. `gnoblin`

The check proves that Tumbleweed can resolve the external BuildRequires. It
does not prove a binary build, an installation, GNOME coexistence, login, or
removal. Do not publish this adapter until those gates have passed on a clean
Tumbleweed GNOME installation.

To prepare the sources, use the repository's reproducible staging script and
point `rpmbuild` at the resulting source directory. Build the packages in the
order above, enabling `--with gnoblin_stack` once the preceding local RPMs are
available to the build service.
