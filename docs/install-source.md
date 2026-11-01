# Build from source

Use this on a system that has no package for Gnoblin. You build Gnoblin from a release tarball or a Git checkout,
try it in a window, then add a login session.

`make` builds into `build/stage` inside the source directory and installs nothing. `make install` copies the build
to `/usr/local/lib/gnoblin`. After that, you can delete the source directory.

## Prerequisites

Install these with your package manager:

- A C/C++ compiler and GNU Make
- Python 3.11 or newer, CMake, Meson and Ninja
- `tar` and `xz`
- Git, if you build from a Git checkout
- The development libraries for GNOME 51: GLib, GTK4 4.20 or newer, libadwaita, PipeWire 1.4.11 or newer
  (`libpipewire-0.3`), Glycin, `gdk-pixbuf-2.0`, `xdg-desktop-portal` 1.20 or newer, and the
  `gsettings-desktop-schemas` development package

You do not need `gnome-shell` or `mutter`. To check your libraries before you build, run
`python3 scripts/check-build-deps.py` in the source directory.

To build without Gnoblin's portal backend, run `./build.sh --without-portal` instead of `make`. You then need no
GTK4, libadwaita or Glycin.

## 1. Get the source

Download `gnoblin-*-source.tar.xz` from the [Gnoblin releases](https://github.com/kierandrewett/gnoblin/releases),
then build it:

```sh
tar -xf gnoblin-*-source.tar.xz
cd gnoblin-[0-9]*/
make
```

To build the current development code instead, use Git:

```sh
git clone https://github.com/kierandrewett/gnoblin.git
cd gnoblin
make
```

Run `make` as your normal user.

## 2. Try it in a window

From a Wayland desktop, after `make`:

```sh
make preview
```

A nested Gnoblin window opens with a terminal. It needs no root access and changes nothing on your system. Close
the terminal to end the preview. See [Devkit](devkit.md) if it does not open.

## 3. Add a login session {#login-session}

Install these with your package manager first:

- `xdg-desktop-portal` and WirePlumber
- `dconf`, so your settings survive logout
- `dbus-tools` on Fedora, or `dbus` on Arch Linux

Then run:

```sh
make install
```

`make install` asks for `sudo` and installs:

- Gnoblin, in `/usr/local/lib/gnoblin`
- The `gnoblin` and `gnoblinctl` commands, in `/usr/local/bin`
- The **Gnoblin** login entry and the manual pages

GNOME stays as another choice at the login screen.

[Install a desktop shell and log in](installation.md#after-installing). Run `gnoblin --version` to check the
installed version.

Gnoblin does not start the GNOME Settings Daemon. It uses the GNOME desktop settings for idle and lock timing:

| Setting                                      | What it does                                           |
| -------------------------------------------- | ------------------------------------------------------ |
| `org.gnome.desktop.session idle-delay`       | Seconds before the session goes idle. `0` turns it off |
| `org.gnome.desktop.screensaver lock-enabled` | `true` locks the screen when the session goes idle     |
| `org.gnome.desktop.screensaver lock-delay`   | Seconds to wait before it locks                        |

For X11 applications, install Xwayland with your package manager.

## Build options

Add these settings after the command, for example `make JOBS=4`:

| Setting  | Default                  | What it does                                                                        |
| -------- | ------------------------ | ----------------------------------------------------------------------------------- |
| `PREFIX` | `/usr/local/lib/gnoblin` | Where `make install` puts Gnoblin. Use the same value for `make` and `make install` |
| `JOBS`   | number of processors     | Number of parallel compilation jobs                                                 |
| `DEVKIT` | `1`                      | `0` skips the nested viewer that `make preview` needs                               |

`make clean` deletes the `build` directory. An installed copy stays.

To change what is built, run `./build.sh` with these flags instead of `make`:

| Flag                    | What it does                                                              |
| ----------------------- | ------------------------------------------------------------------------- |
| `--without-portal`      | Leave out Gnoblin's portal backend                                        |
| `--without-xwayland`    | Leave out X11 application support                                         |
| `--with-vector-cursors` | Add the Adwaita vector cursor theme (needs librsvg and `hyprcursor-util`) |
| `--jobs N`              | Use N parallel compilation jobs                                           |
| `--dry-run`             | Show what the build would do                                              |

## Update

For a Git checkout:

```sh
git pull --ff-only
make
make install
```

For a release tarball, download the newer tarball, extract it into a new directory, and run `make` and
`make install` there. Log out and back in to use the new version.

## Remove

Log in to another session first. Then remove the files that `make install` created:

```sh
sudo rm -f /usr/local/bin/gnoblin /usr/local/bin/gnoblinctl
sudo rm -f /usr/share/man/man1/gnoblin.1 /usr/share/man/man1/gnoblinctl.1
sudo rm -f /usr/share/wayland-sessions/gnoblin.desktop
sudo rm -f /usr/share/xdg-desktop-portal/portals/gnoblin.portal /usr/share/xdg-desktop-portal/gnoblin-portals.conf
sudo rm -f /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service
sudo rm -f /usr/lib/systemd/user/gnoblin-session.target /usr/lib/systemd/user/gnoblin-idle.service
sudo rm -f /usr/lib/systemd/user/xdg-desktop-portal-gnoblin.service
sudo rm -rf /usr/local/lib/gnoblin
```

If you set another `PREFIX`, remove that directory instead of `/usr/local/lib/gnoblin`. Your shell and your
configuration stay.

See [source development](source-development.md) to rebuild one component.
