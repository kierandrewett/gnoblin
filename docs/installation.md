# Install Gnoblin

Gnoblin is a window manager. A desktop shell provides the bar, dock and launcher, so you install both.

## Quick setup

One script adds the Gnoblin repository and signing key for your system. It installs no packages. Read it at
[repo.gnoblin.org/setup.sh](https://repo.gnoblin.org/setup.sh), then run it:

```sh
curl -fsSL https://repo.gnoblin.org/setup.sh | sh
```

The script asks which repositories you want. Gnoblin is selected. Press Space to select Bingux too, then press Enter.
The guide for your system has the same steps by hand.

## Choose your system

::: systems

- [Fedora](install-fedora.md): dnf
- [openSUSE](install-opensuse.md): zypper
- [Debian](install-debian.md): apt
- [Arch Linux](install-arch.md): pacman, also CachyOS
- [Alpine](install-alpine.md): apk
- [Build from source](install-source.md): any other system
- [Platform support](platform-support.md): releases and architectures

:::

Each guide shows how to install, update and remove Gnoblin on that system.

## After installing

1. [Install a desktop shell](bring-your-own-shell.md).
2. Log out. At the login screen, choose your user, open the session menu and choose **Gnoblin**.
3. Log in and [configure Gnoblin](/config).

See [troubleshooting](troubleshooting.md) if the session does not start.

Run `gnoblin --version` to see which version you have installed.

## Optional packages

Gnoblin has optional packages for a portal backend. See
[optional packages](optional-packages.md).
