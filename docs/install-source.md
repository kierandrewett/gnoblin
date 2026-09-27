# Build from source

Use the release source tarball or a Git checkout. This route does not make
Gnoblin supported on your distribution. For distribution package status, see
[platform support](platform-support.md).

The build uses your installed development libraries and writes to `./install`
inside the source tree. Keep that directory there if you register the build as
a login session.

## Prerequisites

### Build tools and libraries

For a native build, you need a C/C++ toolchain, Python 3.11 or newer, CMake,
Meson, Ninja, tar, xz and the development libraries required by the pinned
GNOME sources. A Git checkout also needs Git. Install these packages through
your distribution. The build reports missing libraries and minimum versions.

Python is used while building; the installed control command is a native
GLib/GIO program.

The Shell loads the AccountsService and IBus 1.0 typelibs at startup, even
without their daemons. The build checks for both before compiling the Shell.
Install these runtime packages:

- Fedora: `accountsservice-libs` and `ibus-libs`
- Arch: `accountsservice` and `libibus`
- Debian: `gir1.2-accountsservice-1.0` and `gir1.2-ibus-1.0`

Install the `ibus` daemon separately if you use IBus input methods. The lean
session starts it when an IBus input source is configured.

The installed `gsettings-desktop-schemas` development package must be at least
the version pinned in `gnome-versions.json`.

Gnoblin builds its patched Mutter and GNOME Shell into a separate prefix. It
uses development libraries from your distribution; you do not need its
`gnome-shell` or `mutter` packages.

Mutter reads monitor vendor names from the system's udev hardware database.
Gnoblin builds without the gnome-desktop development package. Its wallpaper
slideshows and input-source lookup use native Shell helpers. The portal's
wallpaper preview uses the Glycin development library already needed by the
Shell.

The portal also needs GTK4 and libadwaita for its dialogs and capture features.
Mutter's development viewer is built only for `./build.sh --preview`.

### Optional features

The default build uses an installed Xcursor theme. Install a cursor theme if
your system does not have one. To include Gnoblin's optional Adwaita vector
theme, install librsvg, hyprcursor-util and the Adwaita cursor theme, then run
`./build.sh --with-vector-cursors`. Inkscape is not required.

GNOME Shell's separate built-in screen recorder needs the GStreamer runtime,
its good plugins, and PipeWire's GStreamer plugin. Install them if you use
that recorder. GStreamer development headers are not needed to build the
Shell. Portal screen sharing uses the compositor's PipeWire support.

When built with NetworkManager support, Gnoblin keeps its network secret
agent but does not need libnma's network menu typelib for its session.
Gnoblin also starts without the GDM UI typelib; it uses its own session lock.

The GNOME Bluetooth panel library is optional too; Gnoblin does not show that
panel.
GNOME's extension tool is disabled, so `gnome-autoar` is not a build
prerequisite. The default build also omits GNOME Shell's test and performance
helpers.

## 1. Get the source

Download one `gnoblin-*-source.tar.xz` from the
[Gnoblin releases](https://github.com/kierandrewett/gnoblin/releases) into
`~/Downloads`. From that directory:

```sh
cd ~/Downloads
tar -xf gnoblin-*-source.tar.xz
cd gnoblin-[0-9]*/
./build.sh
```

The tarball includes Gnoblin and the pinned, patched Mutter, Shell and portal
sources. `./build.sh` unpacks those sources and builds them with your installed
development libraries; it does not need Git. Keep the extracted directory if
you register it as a login session.

For current development code, install Git and clone the repository instead:

```sh
git clone https://github.com/kierandrewett/gnoblin.git
cd gnoblin
./build.sh
```

The script uses CMake and Ninja and checks the installed library versions
against the pinned source requirements. It never calls your system package
manager. Run it as your normal user.

The build also includes Gnoblin's portal backend;
the login-session registration step makes that backend available to the portal
frontend.

## 2. Try it in a window

From an existing Wayland desktop:

```sh
./build.sh --preview
```

The first preview builds Mutter's optional development viewer, then opens a
nested desktop and terminal. Launch your layer-shell client from that terminal.
Close the terminal to end the preview. See [Devkit](devkit.md) for help.

## 3. Add a login session {#login-session}

### Choose a session

After the build and nested test succeed, install the session runtime packages
from your distribution: `xdg-desktop-portal` and WirePlumber. Install dconf or
another persistent GSettings backend so desktop
settings survive logout. The source build does not install host packages.
GNOME Settings is optional.

The example configuration binds media keys to `playerctl` and brightness keys
to `brightnessctl`. Install either command if you want those shortcuts, or
change the bindings to commands available on your system.

Register the lean login without GNOME Session or Settings Daemon:

```sh
./build.sh --register-session
```

For a session with GNOME's hardware and accessibility services, also install
`gnome-session`, `gnome-settings-daemon`, and `upower`, then register that login
instead:

```sh
./build.sh --register-session --gnome-session
```

The lean launcher needs `dbus-update-activation-environment` to update the
shared user bus. Install `dbus-tools` on Fedora or `dbus` on Arch before
registering the login.

### Lean session behavior

The lean login launches Gnoblin directly. After the compositor is ready, it gives
user services the Wayland display address and logind session class, then starts
its session target and XDG autostart.

The session target starts Gnoblin's idle service. It uses these installed
desktop settings:

- `org.gnome.desktop.session idle-delay`: seconds before idle activation;
  `0` disables it.
- `org.gnome.desktop.screensaver lock-enabled`: `true` locks the screen after
  activation; `false` leaves it unlocked.
- `org.gnome.desktop.screensaver lock-delay`: requested seconds before locking;
  the Shell finishes its idle fade first.

Run `gsettings get org.gnome.desktop.session idle-delay` to see the current
timeout. Applications can prevent idle activation through the ScreenSaver
`Inhibit` method or the desktop portal. Portal apps can also prevent suspension.
Portal inhibition ends when the request closes or its caller disconnects.
Direct ScreenSaver inhibition ends at `UnInhibit` or caller disconnect.

The lean login reports logout and user-switch inhibition as unsupported. The
GNOME Session login delegates all four inhibition types to GNOME SessionManager.

It uses `Gnoblin` as its desktop identity
so GNOME-only autostart entries stay out of the lean session. The launcher
clears display addresses left by a previous login before the compositor starts.

If Xwayland is absent, the lean login starts without X11 application support.
Install Xwayland through your distribution if you need X11 applications.

Logout ends the compositor and returns to the login manager without GNOME's
logout confirmation dialog. Save your work before logging out.

GNOME Settings Daemon services do not start, so their hardware controls,
accessibility features, and XSettings are unavailable. Use the GNOME Session
option if you need them.

The lean path needs a fresh Wayland login managed by logind. The login manager
must set `XDG_SESSION_TYPE=wayland` when it starts Gnoblin.

Either command asks for sudo to install the login entry and Gnoblin portal
metadata, then links its user services. [Install a shell](bring-your-own-shell.md),
log out, and select **Gnoblin**. This registration changes the login entry for
Gnoblin; it does not change a separate GNOME session.

Registration only adds session files; it does not build a missing runtime.

## Build options

| Command                                         | Behaviour                                 |
| ----------------------------------------------- | ----------------------------------------- |
| `./build.sh`                                    | Build Gnoblin and its session data        |
| `./build.sh --jobs N`                           | Use N parallel compilation jobs           |
| `./build.sh --prefix DIR`                       | Build into DIR instead of `./install`     |
| `./build.sh --without-xwayland`                 | Omit X11 application support              |
| `./build.sh --with-vector-cursors`              | Include the optional vector cursor theme  |
| `./build.sh --dry-run`                          | Show what will be built                   |
| `./build.sh --verbose`                          | Show all build output as it runs          |
| `./build.sh --preview`                          | Try the build in a nested Wayland session |
| `./build.sh --register-session`                 | Add the lean login entry                  |
| `./build.sh --register-session --gnome-session` | Add the GNOME Session login entry         |

Use `./build.sh --preview --terminal kitty` to choose a terminal.
Use `--without-xwayland` only if you run Wayland-native applications; X11-only
applications cannot open in that build. Rebuild without the option to restore
XWayland support.

The build shows each top-level Ninja entry, compilation progress at roughly
10% intervals, and up to 14 recent lines of other output per entry. It saves
every line under `build/logs/` and points to that log on failure. Use
`--verbose` to watch every command and diagnostic as it runs.

Stage and result colors appear in a terminal. Set `NO_COLOR=1` to disable them;
redirected output and log files contain plain text.

Compiler temporary files use `build/tmp` so a separate `/tmp` quota does not
interrupt a build with space available in the checkout.

## How GNOME stays separate

Gnoblin's patched Mutter and GNOME Shell stay in the private build prefix.
It uses development libraries installed by your distribution. The build does not add
library paths to your shell profile or the system loader configuration.

The host still provides the kernel, graphics drivers, system services and
compatible base libraries. The installed PipeWire client connects to the existing
audio service; the build does not register another PipeWire service.

## Update

For a Git checkout:

```sh
git pull --ff-only
./build.sh
```

Preserve local changes if Git refuses the update. Log out and back in after
rebuilding; configuration reload cannot replace compositor libraries.

For a release tarball, download the newer source tarball, extract it into a
new directory, and run `./build.sh` there. If the old build was registered as
a login session, register the new build with the same registration option you
used before: `./build.sh --register-session` or
`./build.sh --register-session --gnome-session`.

Registration updates Gnoblin's user-unit links to the new build. If a custom
unit uses one of those names, move it aside first.
Log out and back in to use the new compositor.

## Remove the local session

Log into another session. Remove only the files created by local registration:

```sh
rm -f ~/.config/systemd/user/gnoblin-session.target
rm -f ~/.config/systemd/user/gnoblin-idle.service
rm -f ~/.config/systemd/user/org.gnoblin.Shell.target
rm -f ~/.config/systemd/user/org.gnoblin.Shell@wayland.service
rm -f ~/.config/systemd/user/xdg-desktop-portal-gnoblin.service
rm -f ~/.config/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf
systemctl --user daemon-reload
sudo rm -f /usr/share/wayland-sessions/gnoblin.desktop
sudo rm -f /usr/share/gnome-session/sessions/gnoblin.session
sudo rm -f /usr/share/xdg-desktop-portal/portals/gnoblin.portal
sudo rm -f /usr/share/xdg-desktop-portal/gnoblin-portals.conf
sudo rm -f /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service
```

You can then remove the checkout's `build` and `install` directories.
For a packaged install, use the package manager instead.

## Missing or outdated dependencies

The build stops at a missing dependency instead of changing host packages.
To check the versioned libraries before building, run:

```sh
python3 scripts/check-build-deps.py
```

This check reads the pinned GNOME source requirements for the default build.
Add `--without-xwayland` if building without X11 application support, or
`--with-vector-cursors` if enabling the optional vector cursor theme. You can
combine the flags. Meson checks other build requirements as it configures each
component. Keep the error and the Meson log path when reporting a build problem.

The pinned GNOME versions are in [gnome-versions.json](https://github.com/kierandrewett/gnoblin/blob/main/gnome-versions.json).
See [source development](source-development.md) for component rebuilds.

## Build reports disk quota exceeded

Check space and your user quota on the filesystem containing the extracted
source:

```sh
df -h .
quota -s
```

Move the extracted source directory to a filesystem with enough quota, then
run `./build.sh` there. The build keeps compiler temporary files under
`build/tmp` inside that directory.
