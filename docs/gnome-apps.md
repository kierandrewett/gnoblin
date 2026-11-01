# Use GNOME applications

Install each GNOME application with your system's package manager. The package brings in the libraries it needs.

## Shared services

On a minimal system, also install these services:

| Package         | What it does                                                          |
| --------------- | --------------------------------------------------------------------- |
| `gvfs`          | File access such as mounts and trash, for file managers that use them |
| `gnome-keyring` | Stores secrets for apps                                               |
| `xdg-user-dirs` | Creates the standard folders, such as Documents and Downloads         |

Install them with your package manager:

```sh
# Fedora
sudo dnf install gvfs gnome-keyring xdg-user-dirs
# Debian
sudo apt install gvfs gnome-keyring xdg-user-dirs
# Arch Linux
sudo pacman -S gvfs gnome-keyring xdg-user-dirs
# openSUSE Tumbleweed
sudo zypper install gvfs gnome-keyring xdg-user-dirs
```

This does not install a file manager, browser or GNOME Settings. Install the apps you want separately, for
example `nautilus` for the Files app.

## Portals

Apps ask for files, screen sharing and similar things through portals. Install `xdg-desktop-portal-gnoblin` to use
Gnoblin's portal backend. Without it, Gnoblin uses another portal backend that is installed. To choose a different
backend, see [`gnoblin.configure.portals`](/config/configure/portals).

## Removable media and email

- Gnoblin does not mount removable media for you. Open it in your file manager.
- Email links open your default email app. Install one if you need it.
