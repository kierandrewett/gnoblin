# Optional packages

Install these only if you need them.

Use your system's package manager to install a package by name:

- Fedora: `sudo dnf install NAME`
- Debian: `sudo apt install NAME`
- Arch Linux: `sudo pacman -S NAME`
- openSUSE: `sudo zypper install NAME`
- Alpine: `apk add NAME`

| Package                       | What it adds                                                                       |
| ----------------------------- | ---------------------------------------------------------------------------------- |
| `xdg-desktop-portal-gnoblin`  | Gnoblin's portal backend, which shows the permission dialogs of apps               |
| `gnoblin-geoclue-integration` | Location permission prompts through Gnoblin. Restart GeoClue after you install it. |
| `gnoblin-ananicy-rules`       | Scheduling rules for Gnoblin, for systems that run `ananicy-cpp`                   |

Without `xdg-desktop-portal-gnoblin`, Gnoblin uses another portal backend that is installed.

## GTK 4

The `gnoblin` package does not need GTK 4 or libadwaita. Gnoblin draws the title bars and borders of X11 apps
itself, as it does for Wayland apps. See [window frames](/guides/window_frames). Only `xdg-desktop-portal-gnoblin`
needs GTK 4 and libadwaita.

## Other software your shell may need

- **Input methods:** install `ibus` and an engine such as `ibus-m17n`. See
  [Type with an input method](/config/configure/input_sources#type-with-an-input-method).

::: details GNOME applications

Apps from the GNOME project work in Gnoblin. On a minimal system, install a few shared services first: `gvfs`,
`gnome-keyring` and `xdg-user-dirs`. [Use GNOME applications](gnome-apps.md) has the commands.

:::
