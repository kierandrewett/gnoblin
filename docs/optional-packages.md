# Optional packages

Install these only if you need them. `gnoblin-x11-frames` is the exception: your package manager installs it with
Gnoblin on most systems. See [X11 window frames](#x11-window-frames).

Use your system's package manager to install a package by name:

- Fedora: `sudo dnf install NAME`
- Debian: `sudo apt install NAME`
- Arch Linux: `sudo pacman -S NAME`
- openSUSE: `sudo zypper install NAME`
- Alpine: `apk add NAME`

| Package                       | What it adds                                                                                     |
| ----------------------------- | ------------------------------------------------------------------------------------------------ |
| `gnoblin-x11-frames`          | Title bars and borders for X11 apps. It needs GTK 4. See [X11 window frames](#x11-window-frames) |
| `xdg-desktop-portal-gnoblin`  | Gnoblin's portal backend, which shows the permission dialogs of apps                             |
| `gnoblin-geoclue-integration` | Location permission prompts through Gnoblin. Restart GeoClue after you install it.               |
| `gnoblin-ananicy-rules`       | Scheduling rules for Gnoblin, for systems that run `ananicy-cpp`                                 |

Without `xdg-desktop-portal-gnoblin`, Gnoblin uses another portal backend that is installed.

## X11 window frames

Mutter does not draw the title bar of an X11 app. A small program, `mutter-x11-frames`, draws it. It is
the only part of Gnoblin's compositor that needs GTK 4, so it is in its own package, `gnoblin-x11-frames`.

This split is not complete. It is not a promise that an install with no GTK 4 is safe for X11 apps.

Without the package, X11 apps that use the system title bar get no title bar and no borders. Examples are `xterm`,
Qt apps on X11, Java apps and Wine. Some X11 windows may fail to open. No one has tested that case in a graphical
session yet.

Apps that draw their own title bar (GTK 4, libadwaita) and Wayland apps do not need the package.

| System                   | What happens when you install `gnoblin`                                                                                               |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------- |
| Fedora, openSUSE, Debian | The package manager installs `gnoblin-x11-frames` too, as a recommended package                                                       |
| Arch Linux, Alpine       | `gnoblin` depends on `gnoblin-x11-frames`, so it is installed and GTK 4 comes with it                                                 |
| `.tar.xz` packages       | The frames tarball is needed for X11 apps. Extract `gnoblin-x11-frames-VERSION-ARCH.tar.xz` over `/` as well as the `gnoblin` tarball |

To install Gnoblin without GTK 4, refuse the recommended package. Do this only if you run no X11 apps:

- Fedora: `sudo dnf install --setopt=install_weak_deps=False gnoblin`
- openSUSE: `sudo zypper install --no-recommends gnoblin`
- Debian: `sudo apt install --no-install-recommends gnoblin`

After an upgrade from an earlier release, install `gnoblin-x11-frames` once if your X11 apps have lost their title
bars. To remove the package, remove it by name.

## Other software your shell may need

- **Input methods:** install `ibus` and an engine such as `ibus-m17n`. See
  [Type with an input method](/config/configure/input_sources#type-with-an-input-method).

::: details GNOME applications

Apps from the GNOME project work in Gnoblin. On a minimal system, install a few shared services first: `gvfs`,
`gnome-keyring` and `xdg-user-dirs`. [Use GNOME applications](gnome-apps.md) has the commands.

:::
