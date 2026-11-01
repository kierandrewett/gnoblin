# Install Gnoblin

Gnoblin is a window manager. A desktop shell provides the bar, dock and launcher, so you install both.
Gnoblin installs next to GNOME, so your current session stays available.

## Choose your system

| System              | Guide                                  |
| ------------------- | -------------------------------------- |
| Fedora 45           | [Fedora](install-fedora.md)            |
| openSUSE Tumbleweed | [openSUSE](install-opensuse.md)        |
| Debian testing      | [Debian](install-debian.md)            |
| Arch Linux, CachyOS | [Arch Linux](install-arch.md)          |
| Alpine edge         | [Alpine](install-alpine.md)            |
| NixOS               | [NixOS](install-nixos.md)              |
| Anything else       | [Build from source](install-source.md) |

Each guide adds the Gnoblin package repository, installs Gnoblin and shows how to update and remove it.
Packages are available for amd64 and arm64.

## After installing

1. [Install a desktop shell](bring-your-own-shell.md).
2. Log out. At the login screen, choose your user, open the session menu and choose **Gnoblin**.
3. Log in and [configure Gnoblin](/config).

If you do not get a working desktop, log out and choose your previous session. See
[troubleshooting](troubleshooting.md).

Run `gnoblin --version` to see which version you have installed.

## Optional packages

Gnoblin has optional packages for a portal backend, location prompts and scheduling rules. See
[optional packages](optional-packages.md).
