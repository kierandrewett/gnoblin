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

Gnoblin keeps using its own portal backend. This package does not install or
select the stock GNOME portal backend. Apps continue to use the portal service
selected by their current session.

File selection in a Gnoblin session uses Gnoblin's GTK4 portal dialog. Email
requests open the configured email app; install one separately if you need it.
Nautilus is optional; install it only if you want a file manager.

Gnoblin's portal backend handles file selection, Settings, screen sharing,
remote desktop, and inhibition. The base install does not need
`xdg-desktop-portal-gtk`.

In the lean login, portal apps can inhibit idle and
suspend. Logout and user-switch inhibition return an unsupported response.
The optional GNOME Session login uses GNOME SessionManager for all four types.
The standalone login continues to use Gnoblin's idle service if a GNOME
SessionManager process remains on the user's D-Bus session bus.

For a [source build](install-source.md), install these services with your
distribution's package manager if you need them. `./build.sh` never installs
host packages.
