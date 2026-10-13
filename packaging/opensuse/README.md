# openSUSE Tumbleweed RPM adapter

These recipes package Gnoblin's standalone session and GNOME 51 compositor
stack under `/usr/lib/gnoblin` on Tumbleweed. The only shared paths are the
`gnoblin` display-manager entry, Gnoblin-named systemd user units, `gnoblinctl`,
and the distinct Gnoblin polkit action. The recipes do not replace, conflict
with, obsolete, or provide stock GNOME packages.

`check-buildrequires.sh` runs `rpmspec` for one package and asks Zypper to
resolve its host dependencies. `build-chain.sh` checks the complete Gnoblin
compositor and session build before preparing source archives. It then builds
the optional portal package and the single `gnoblin` runtime package, which
contains Mutter and the Lua supervisor together.

To check dependencies without installing them, run
`packaging/opensuse/check-buildrequires.sh gnoblin`. The accepted package names
are `xdg-desktop-portal-gnoblin` and `gnoblin`. Add `--install` to install the
selected stage's host dependencies. A successful dependency check does not
prove a binary build, installation, GNOME coexistence, login, or removal. Do
not publish this adapter until those gates have passed on a clean Tumbleweed
GNOME installation.

To prepare the sources, use the repository's reproducible staging script and
point `rpmbuild` at the resulting source directory.
