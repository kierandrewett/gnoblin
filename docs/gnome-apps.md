# Use GNOME applications

Install each GNOME application with your distribution's package manager. The
application's package should pull in the libraries it needs.

## Install shared services

On a minimal system, also install these shared desktop services:

- `gvfs`: GIO file access, such as mounts and trash. Install it if your file
  manager uses GIO mounts.
- `gnome-keyring`: stores secrets for apps.
- `xdg-user-dirs`: creates the standard user folders, such as Documents and Downloads.

Install them with your distribution's package manager:

- Fedora: `sudo dnf install gvfs gnome-keyring xdg-user-dirs`
- Arch: `sudo pacman -S gvfs gnome-keyring xdg-user-dirs`
- openSUSE Tumbleweed: `sudo zypper install gvfs gnome-keyring xdg-user-dirs`

For example, on a Fedora installation without GNOME, install the services and
the Files app with:

```sh
sudo dnf --setopt=install_weak_deps=False install gvfs gnome-keyring xdg-user-dirs nautilus
```

This does not install a file manager, calendar, browser, GNOME Settings, or the
full GNOME desktop. Install the apps you want separately.

If you want prompts to rename standard folders when your display language
changes, install `xdg-user-dirs-gtk` separately. Gnoblin does not start that
updater at login.

## Keyring and online accounts

GNOME Keyring starts when an app requests its secret service. Some features
need an app-specific service.

Online accounts need an account manager and the relevant GVfs backend.

## Removable media

Gnoblin does not start GNOME Shell's removable-media automount and autorun
components. Open removable media through your file manager when needed.

## Portals

Portal selection affects the Gnoblin session only. Other sessions keep their
own backend selection.

Gnoblin's optional backend handles file selection, Settings, screen sharing,
remote desktop, and inhibition. Its file chooser uses GTK4. Install
`xdg-desktop-portal-gnoblin` to use it. When installed, the core session
package routes Gnoblin to this backend, with another installed backend as
fallback.

Gnoblin routes portal interfaces but does not manage app permissions. The
selected backend decides what each app may do. The Gnoblin backend shows its
normal consent dialogs.

Choose another backend or mix backends in your normal Lua config; see
[`gnoblin.configure.portals`](/config/configure/portals).

## Email, file managers and inhibition

Email requests open your configured email app; install one separately if you
need it. Nautilus is optional; install it only if you want a file manager.

Portal apps can inhibit idle and suspend. How that works depends on the login:

- Lean login: logout and user-switch inhibition return an unsupported response.
- Optional GNOME Session login: GNOME SessionManager handles all four types.
- Standalone login: Gnoblin's idle service handles requests while a GNOME
  SessionManager process remains on the user's D-Bus session bus.

## Source builds

For a [source build](install-source.md), install these services with your
distribution's package manager if you need them. `make` never installs host
packages.
