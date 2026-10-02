# Use GNOME applications

Install each GNOME application with your distribution's package manager. Its
package should bring the libraries it needs. On a minimal system, you may also
want the optional `gnoblin-gnome-integration` package for shared desktop
services. Install it alongside Gnoblin:

| System                               | Command                                                                                                                                                                                                                                                                   |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Fedora with the Gnoblin COPR enabled | `sudo dnf install gnoblin-gnome-integration`                                                                                                                                                                                                                              |
| Arch                                 | Download the matching `gnoblin-gnome-integration-*.pkg.tar.zst` from the [Gnoblin release](https://github.com/kierandrewett/gnoblin/releases), then run `sudo pacman -U ./gnoblin-gnome-integration-*.pkg.tar.zst` in the download directory.                             |
| openSUSE Tumbleweed                  | Download the matching `opensuse-gnoblin-gnome-integration-*.rpm` from the [Gnoblin release](https://github.com/kierandrewett/gnoblin/releases), then run `sudo zypper install --allow-unsigned-rpm ./opensuse-gnoblin-gnome-integration-*.rpm` in the download directory. |

For example, with the Gnoblin COPR enabled on a Fedora installation without
GNOME, install the shared services and the Files app with:

```sh
sudo dnf --setopt=install_weak_deps=False install gnoblin-gnome-integration nautilus
```

The package installs GNOME Keyring for stored secrets, GVfs for GIO file
access, and XDG user-directory setup. It does not install a file manager,
calendar, browser, GNOME Settings, or the full GNOME desktop. Install the apps
you want separately.

If you want prompts to rename standard folders when your display language
changes, install `xdg-user-dirs-gtk` separately. The integration package does
not start that updater at login.

GNOME Keyring starts when an app requests its secret service. Some features
need an app-specific service: for example,
online accounts need an account manager and the relevant GVfs backend.

Gnoblin does not start GNOME Shell's removable-media automount and autorun
components. Open removable media through your file manager when needed; install
GVfs through the integration package if that file manager uses GIO mounts.

Gnoblin's portal backend is optional. When it is installed, Gnoblin prefers it
for interfaces it implements and falls back to GTK. Otherwise the portal
frontend uses another installed backend. This does not change portal selection
in a GNOME session. Apps continue to use the portal service selected by their
current session.

File selection uses whichever backend is selected for the session. Email
requests open the configured email app; install one separately if you need it.
Nautilus is optional; install it only if you want a file manager.

Gnoblin's optional portal backend handles file selection, Settings, screen
sharing, remote desktop, and inhibition. Its file chooser uses GTK4. Install
the `gnoblin-portal` package to use it. You can instead install another portal
backend and select it below.

## Choose a different portal backend

To route portal requests to another installed backend for a Gnoblin session,
create `~/.config/xdg-desktop-portal/gnoblin-portals.conf`:

```ini
[preferred]
default=gtk
```

Replace `gtk` with an installed backend name such as `kde`. The per-user file
takes precedence over Gnoblin's system default. To keep Gnoblin for screen
sharing while using another file chooser, add an interface-specific entry:

```ini
[preferred]
default=gtk
org.freedesktop.impl.portal.ScreenCast=gnoblin
org.freedesktop.impl.portal.RemoteDesktop=gnoblin
```

Restart `xdg-desktop-portal` after changing this file:

```sh
systemctl --user restart xdg-desktop-portal.service
```

In the lean login, portal apps can inhibit idle and
suspend. Logout and user-switch inhibition return an unsupported response.
The optional GNOME Session login uses GNOME SessionManager for all four types.
The standalone login continues to use Gnoblin's idle service if a GNOME
SessionManager process remains on the user's D-Bus session bus.

For a [source build](install-source.md), install these services with your
distribution's package manager if you need them. `./build.sh` never installs
host packages.
