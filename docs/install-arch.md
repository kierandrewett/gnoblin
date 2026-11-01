# Arch Linux

Gnoblin publishes Arch packages for x86_64 and aarch64. The release workflow installs them on a
clean Arch container without stock GNOME, checks them beside stock GNOME, and removes them. See
[platform support](platform-support.md) before using the package as a login session.

## Install from the package repository

Gnoblin's signed pacman repository has a `stable` and a `nightly` channel and checks every package
signature. See [package repositories](package-repositories.md), then run:

```sh
sudo pacman -Syu gnoblin
```

Install `xdg-desktop-portal-gnoblin` as well if you want Gnoblin's GTK portal backend.

## Install a release file

Download the `gnoblin` package from the
[Gnoblin release](https://github.com/kierandrewett/gnoblin/releases) and install it with
`sudo pacman -U`. The release writes the package epoch as a dot in the file name, for example
`gnoblin-1.0.1.13-1-x86_64.pkg.tar.zst`. pacman reads the real version from the package.

The package installs Gnoblin under `/usr/lib/gnoblin` and registers its login session.
[Install a desktop shell](bring-your-own-shell.md), log out, and select **Gnoblin** at the login
screen. Keep another session available while testing this package path.

The core package installs the default portal route for Gnoblin sessions but
does not include a backend. The optional `xdg-desktop-portal-gnoblin`
package provides one. It needs GTK4 4.20 or newer and `xdg-desktop-portal`
1.20 or newer.

Install the portal package if you want Gnoblin's GTK backend. The default route uses it when installed and otherwise selects
another installed backend. You can route interfaces to another backend in the
Lua config.

For GNOME applications on a minimal Arch system, install the
[shared desktop services](gnome-apps.md) first. Install the applications you
want through pacman.
