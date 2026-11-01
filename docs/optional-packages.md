# Optional packages

Install these only if you need them. Use your system's package manager, for example
`sudo dnf install NAME` on Fedora, `sudo apt install NAME` on Debian, `sudo pacman -S NAME` on Arch Linux,
`sudo zypper install NAME` on openSUSE and `apk add NAME` on Alpine.

| Package                       | What it adds                                                                       |
| ----------------------------- | ---------------------------------------------------------------------------------- |
| `xdg-desktop-portal-gnoblin`  | Gnoblin's portal backend, which shows the permission dialogs of apps               |
| `gnoblin-geoclue-integration` | Location permission prompts through Gnoblin. Restart GeoClue after you install it. |
| `gnoblin-ananicy-rules`       | Scheduling rules for Gnoblin, for systems that run `ananicy-cpp`                   |

Without `xdg-desktop-portal-gnoblin`, Gnoblin uses another portal backend that is installed. GNOME sessions
keep their own portal setup.

## Other software your shell may need

- **GNOME apps:** install the [shared desktop services](gnome-apps.md) first.
- **Input methods:** install `ibus` and an engine such as `ibus-m17n`. See
  [Type with an input method](/config/configure/input_sources#type-with-an-input-method).
