# Install Gnoblin

Gnoblin manages your windows. A separate desktop shell provides the bar, dock
and launcher. Install both before your first login.

No distribution has completed the full graphical-session support gate. Read
[platform support](platform-support.md) before choosing a package path.
The new package paths require build, installation, GNOME coexistence, and
removal checks before they can be called package candidates.

## Build from the source tarball

Download the `gnoblin-*-source.tar.xz` asset from a
[Gnoblin release](https://github.com/kierandrewett/gnoblin/releases), extract
it, and run `./build.sh` inside the extracted directory. This is the primary
installation path. It needs development libraries at the versions required by
the pinned sources, but does not need Git or an installed GNOME Shell package.
See [source build prerequisites and login setup](install-source.md).

After building, `./build.sh --register-session` adds the lean login entry.
GNOME Session and Settings Daemon are optional for this source install.
Run `./install/bin/gnoblin --version` to see the installed Gnoblin and GNOME
versions, component versions, and source Git remote and commit. Distribution
packages also expose this command as `gnoblin --version`.
Add `--json` after `--version` for a machine-readable build identity.

`gnoblin` is both the login command and the package to install. Its
`gnoblin-session` systemd target groups services used during the session.

## Distribution packages

| System              | Test path                                                                                   |
| ------------------- | ------------------------------------------------------------------------------------------- |
| Fedora 45           | [COPR build](install-fedora.md)                                                             |
| Arch / CachyOS      | [Build the PKGBUILD](install-arch.md)                                                       |
| openSUSE Tumbleweed | [Build the RPM](https://github.com/kierandrewett/gnoblin/blob/main/packaging/rpm/README.md) |
| NixOS               | [Experimental package and module](install-nixos.md)                                         |
| Other releases      | [Build from source](install-source.md)                                                      |

For Debian and Ubuntu, use the source instructions when installed development
libraries meet the pinned GNOME requirements. There is currently no Gnoblin
APT package.

Keep an existing GNOME or other session available while testing these package
paths.
The Gnoblin session selects its own portal backend. An existing GNOME session
keeps its own portal selection. Distribution packages use the same lean
session launcher as the source tarball; GNOME Session and Settings Daemon are
not required for Gnoblin's login entry.

On a minimal install, add [GNOME application services](gnome-apps.md) only if
you need them. GNOME apps install their own library dependencies; the optional
package adds shared desktop services such as GVfs and GNOME Keyring.

## After installing

1. [Install a desktop shell](bring-your-own-shell.md).
2. Log out. At the login screen, choose your user, open the session selector,
   and choose **Gnoblin**.
3. Log in and [configure Gnoblin](/config).

If there is no bar or launcher, right-click the desktop and choose **Open
Terminal**. See [first-login troubleshooting](troubleshooting.md#no-bar-dock-or-launcher).

For development builds, see [source installation](install-source.md).
Release maintainers should use the [packaging guide](distribution.md).
