# openSUSE Tumbleweed RPM adapter

These recipes package Gnoblin's standalone session and GNOME 51 compositor
stack under `/usr/lib/gnoblin` on Tumbleweed. The only shared paths are the
`gnoblin` display-manager entry, Gnoblin-named systemd user units, `gnoblinctl`,
and the distinct Gnoblin polkit action. The recipes do not replace, conflict
with, obsolete, or provide stock GNOME packages.

`check-buildrequires.sh` runs `rpmspec` for one package and asks Zypper to
resolve only that package's host dependencies. `build-chain.sh` installs the
Mutter host requirements before preparing source archives, then checks each
later stage when it is ready. The session package is not resolved before the
private Mutter development RPM has been built and installed:

1. `gnoblin-mutter` and `gnoblin-mutter-devel`
2. `gnoblin-portal`
3. `gnoblin` and its optional `gnoblin-gnome-integration` subpackage

To check dependencies without installing them, run
`packaging/opensuse/check-buildrequires.sh mutter`. The accepted package names
are `mutter`, `gnoblin-portal`, and `gnoblin`. Add `--install` to install the
selected stage's host dependencies. A successful dependency check does not
prove a binary build, installation, GNOME coexistence, login, or removal. Do
not publish this adapter until those gates have passed on a clean Tumbleweed
GNOME installation.

To prepare the sources, use the repository's reproducible staging script and
point `rpmbuild` at the resulting source directory. Build the packages in the
order above; the chain installs each private RPM before resolving the next
package's requirements.
