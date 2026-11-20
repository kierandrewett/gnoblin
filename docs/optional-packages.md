# Optional packages

Install these only if you need them. Use your system's package manager, for example
`sudo dnf install NAME` on Fedora, `sudo apt install NAME` on Debian, `sudo pacman -S NAME` on Arch Linux,
`sudo zypper install NAME` on openSUSE and `apk add NAME` on Alpine.

| Package                      | What it adds                                                         |
| ---------------------------- | -------------------------------------------------------------------- |
| `xdg-desktop-portal-gnoblin` | Gnoblin's portal backend, which shows the permission dialogs of apps |

Without `xdg-desktop-portal-gnoblin`, Gnoblin uses another portal backend that is installed.

## Other software your shell may need

- **Input methods:** install `ibus` and an engine such as `ibus-m17n`.

::: details GNOME applications

Apps from the GNOME project work in Gnoblin. On a minimal system, install a few shared services first: `gvfs`,
`gnome-keyring` and `xdg-user-dirs`. [Use GNOME applications](gnome-apps.md) has the commands.

:::
