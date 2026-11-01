# Install Gnoblin

Gnoblin is a window manager. A desktop shell provides the bar, dock and launcher, so you install both.

## Choose your system

| System              | Guide                                  |
| ------------------- | -------------------------------------- |
| Fedora              | [Fedora](install-fedora.md)            |
| openSUSE            | [openSUSE](install-opensuse.md)        |
| Debian              | [Debian](install-debian.md)            |
| Arch Linux, CachyOS | [Arch Linux](install-arch.md)          |
| Alpine              | [Alpine](install-alpine.md)            |
| NixOS               | [NixOS](install-nixos.md)              |
| Anything else       | [Build from source](install-source.md) |

Each guide adds the Gnoblin package repository, installs Gnoblin and shows how to update and remove it.
For the supported releases and architectures, see [platform support](platform-support.md).

## Quick setup

One script adds the Gnoblin repository and signing key for your system. It installs no packages. Read it first at [repo.gnoblin.org/setup.sh](https://repo.gnoblin.org/setup.sh), then run it:

```sh
curl -fsSL https://repo.gnoblin.org/setup.sh | sh
```

Add `-s -- --nightly` after `sh` to use the nightly channel, or `-s -- --bingux` to add the Bingux repository too.
`-s -- --help` lists every option. The guide for your system has the same steps by hand.

## After installing

1. [Install a desktop shell](bring-your-own-shell.md).
2. Log out. At the login screen, choose your user, open the session menu and choose **Gnoblin**.
3. Log in and [configure Gnoblin](/config).

See [troubleshooting](troubleshooting.md) if the session does not start.

Run `gnoblin --version` to see which version you have installed.

## Optional packages

Gnoblin has optional packages for a portal backend, location prompts and scheduling rules. See
[optional packages](optional-packages.md).
