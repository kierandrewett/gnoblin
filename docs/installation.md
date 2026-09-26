# Install Gnoblin

Gnoblin manages your windows. A separate desktop shell provides the bar, dock
and launcher. Install both before your first login.

No distribution has completed the full graphical-session support gate. Read
[platform support](platform-support.md) before choosing a package path. Package
candidates have passed build, installation, GNOME coexistence and removal
checks, but still need a verified graphical login.

## Choose your system

| System                              | Test path                                                                                        |
| ----------------------------------- | ------------------------------------------------------------------------------------------------ |
| Fedora 43, 44 and 45                | [COPR package candidate](install-fedora.md)                                                      |
| Debian 13, Ubuntu 24.04 and 26.04   | [APT package candidates](install-debian.md)                                                      |
| Arch / CachyOS                      | [Build the PKGBUILD](https://github.com/kierandrewett/gnoblin/blob/main/packaging/arch/PKGBUILD) |
| openSUSE Tumbleweed                 | [Build the RPM](https://github.com/kierandrewett/gnoblin/blob/main/packaging/rpm/README.md)      |
| NixOS 25.05, 25.11, 26.05, unstable | [Flake package candidate and module](install-nixos.md)                                           |
| Other releases                      | [Build from source](install-source.md)                                                           |

The candidate package checks show GNOME can coexist with Gnoblin on the listed
targets. Keep an existing GNOME or other session available while testing.

## After installing

1. [Install a desktop shell](bring-your-own-shell.md).
2. Log out. At the login screen, choose your user, open the session selector,
   and choose **Gnoblin**.
3. Log in and [configure Gnoblin](/config).

If there is no bar or launcher, right-click the desktop and choose **Open
Terminal**. See [first-login troubleshooting](troubleshooting.md#no-bar-dock-or-launcher).

For development builds, see [source installation](install-source.md).
Release maintainers should use the [packaging guide](distribution.md).
