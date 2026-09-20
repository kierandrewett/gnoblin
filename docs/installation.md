# Install Gnoblin

Gnoblin manages your windows. A separate desktop shell provides the
bar, dock and launcher. Install both before your first login.

## Choose your system

| System          | Install method                           |
| --------------- | ---------------------------------------- |
| Fedora          | [Install from COPR](install-fedora.md)   |
| NixOS           | [Add the NixOS module](install-nixos.md) |
| Arch / CachyOS  | [Build from source](install-source.md)   |
| Debian / Ubuntu | [Build from source](install-source.md)   |
| openSUSE        | [Build from source](install-source.md)   |

Gnoblin installs alongside GNOME. Your existing GNOME session remains available.

## After installing

1. [Install a desktop shell](bring-your-own-shell.md).
2. Log out. At the login screen, choose your user, open the session selector,
   and choose **Gnoblin**.
3. Log in and [configure Gnoblin](configuration.md).

If there is no bar or launcher, right-click the desktop and choose **Open
Terminal**. See [first-login troubleshooting](troubleshooting.md#no-bar-dock-or-launcher).

## Package versions

Configuration pages describe the current source tree. Published packages can
lag behind it. Check `gnoblinctl version` if an option is rejected.

As of 20 September 2026, Fedora COPR supplies Mutter 49.5 and Shell 49.6;
the source tree targets GNOME 51. The Fedora guide uses the package names
actually published in [COPR](https://copr.fedorainfracloud.org/coprs/kierandrewett/gnoblin/).

For development builds, see [source installation](install-source.md).
Release maintainers should use the [packaging guide](distribution.md).
