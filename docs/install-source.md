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

The compositor is built from the public [Gnoblin Mutter fork](https://github.com/kierandrewett/gnoblin-mutter),
based on the Mutter release pinned in `gnome-versions.json`. Git checkouts and
source tarballs pin its source revision.

The installed `gsettings-desktop-schemas` development package must be at least
the version pinned in `gnome-versions.json`. The Gnoblin Mutter fork includes
the remote-desktop compatibility changes used by the build, including
`libpipewire-0.3` version 1.4.11 or newer. PipeWire 1.5.84 and 1.6 add
optional color-capability and device-ID negotiation.

Mutter uses the shared schemas for settings such as keyboard, pointer, and
accessibility behavior. The package also supplies enum headers used while
building Mutter. This does not install or start GNOME Shell.

Gnoblin builds its runtime and patched Mutter into a separate prefix. It uses
the pinned sources and your installed development libraries; you do not need
the distribution's `gnome-shell` or `mutter` packages to build or run Gnoblin.

Mutter reads monitor vendor names from the system's udev hardware database.
Gnoblin builds without the gnome-desktop development package.

### Portal backend

The default build includes Gnoblin's portal backend. Applications continue to
call the standard portal frontend. You can choose an installed backend for Gnoblin with
[`gnoblin.configure.portals`](/config/configure/portals).

Gnoblin's backend uses GTK4 and libadwaita for its dialogs and capture
features. It uses Glycin for wallpaper previews. Build without it with
`./build.sh --without-portal`.

Gnoblin's portal build supports GTK4 4.20 and `xdg-desktop-portal` 1.20 or
newer. It bundles the backend interface definitions needed by its pinned
source. Use `--without-portal` when another backend is preferred.

### Optional features

Mutter's development viewer is built only for `./build.sh --preview`.

The default build uses an installed Xcursor theme. Install a cursor theme if
your system does not have one. To include Gnoblin's optional Adwaita vector
theme, install librsvg, hyprcursor-util and the Adwaita cursor theme, then run
`./build.sh --with-vector-cursors`. Inkscape is not required.

Portal screen sharing uses the compositor's PipeWire support and connects to
the PipeWire service installed on your system.

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

The tarball includes Gnoblin and the pinned, patched Mutter and portal sources.
`./build.sh` unpacks and builds Mutter and Gnoblin's portal backend. Use
`--without-portal` to omit the backend. Neither command needs Git. Keep the
extracted directory if you register it as a login session.

For current development code, install Git and clone the repository instead:

```sh
git clone https://github.com/kierandrewett/gnoblin.git
cd gnoblin
./build.sh
```

The script uses CMake and Ninja and checks the installed library versions
against the pinned source requirements. It never calls your system package
manager. Run it as your normal user.

The default build includes the session, Mutter, and Gnoblin's portal backend.
Use `--without-portal` to build the core session only. Session registration
installs the portal route; without Gnoblin's backend, the route selects another
installed backend.

## 2. Try it in a window

From an existing Wayland desktop:

```sh
./build.sh --preview
```

The first preview builds Mutter's optional development viewer, then opens a
nested compositor view and a terminal on your host desktop. If Waybar is
installed, the preview starts a sample panel with the time, CPU use, and memory
use. Without Waybar, start a shell or layer-shell client from the terminal.

Close the terminal to end the preview. See [Devkit](devkit.md) for help.

## 3. Add a login session {#login-session}

### Choose a session

After the build and nested test succeed, install the session runtime packages
from your distribution: `xdg-desktop-portal` and WirePlumber. Install dconf or
another persistent GSettings backend so desktop
settings survive logout. The source build does not install host packages.
GNOME Settings is optional.

The example configuration binds media keys to `wpctl` and `playerctl`. To add
brightness shortcuts, bind the keys to a command such as `brightnessctl`.
Install the commands used by your configuration.

Register the standalone Gnoblin login:

```sh
./build.sh --register-session
```

Registration adds only the standalone Gnoblin login. The normal GNOME session
remains a separate login-screen choice, so you can switch back to GNOME without
installing a Gnoblin compatibility session.

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
  the compositor applies the delay after idle activation.

Run `gsettings get org.gnome.desktop.session idle-delay` to see the current
timeout. Applications can prevent idle activation through the ScreenSaver
`Inhibit` method or the desktop portal. Portal apps can also prevent suspension.
Portal inhibition ends when the request closes or its caller disconnects.
Direct ScreenSaver inhibition ends at `UnInhibit` or caller disconnect.
The ScreenSaver `Lock` method asks your shell to lock the session; see
[lock requests](session-lock.md#request-a-lock).

The login reports logout and user-switch inhibition as unsupported. It uses
`Gnoblin` as its desktop identity so GNOME-only autostart entries stay out of
the session. The launcher
clears display addresses left by a previous login before the compositor starts.

If Xwayland is absent, the lean login starts without X11 application support.
Install Xwayland through your distribution if you need X11 applications.

Logout ends the compositor and returns to the login manager. Save your work
before logging out.

The Gnoblin login does not start GNOME Settings Daemon services. Select the
separate GNOME login at the login screen when you need those services.

The lean path needs a fresh Wayland login managed by logind. The login manager
must set `XDG_SESSION_TYPE=wayland` when it starts Gnoblin.

`./build.sh --register-session` asks for sudo to install the login entry and
Gnoblin's desktop-specific portal route. When the prefix includes Gnoblin's
optional backend, it also installs the backend descriptor and D-Bus service.

If a systemd user manager is available, registration links Gnoblin's optional
user services. The compositor login entry does not depend on them.

Registration adds the prefix's native `gnoblinctl` command to
`~/.local/bin`. It stops if a different command already uses that name.

[Install a shell](bring-your-own-shell.md), log out, and select **Gnoblin**.
This registration changes the Gnoblin login entry; it does not change a
separate GNOME session.

Registration only adds session files; it does not build a missing runtime.

## Build options

| Command                            | Behaviour                                 |
| ---------------------------------- | ----------------------------------------- |
| `./build.sh`                       | Build Gnoblin and its session data        |
| `./build.sh --jobs N`              | Use N parallel compilation jobs           |
| `./build.sh --prefix DIR`          | Build into DIR instead of `./install`     |
| `./build.sh --without-xwayland`    | Omit X11 application support              |
| `./build.sh --without-portal`      | Omit Gnoblin's GTK portal backend         |
| `./build.sh --with-vector-cursors` | Include the optional vector cursor theme  |
| `./build.sh --dry-run`             | Show what will be built                   |
| `./build.sh --verbose`             | Show all build output as it runs          |
| `./build.sh --preview`             | Try the build in a nested Wayland session |
| `./build.sh --register-session`    | Add the standalone Gnoblin login entry    |
| `./build.sh --layout system`       | Also write a package's public files       |
| `./build.sh --destdir DIR`         | Install below DIR, as a package root      |

## Build a package tree

A distribution package uses the same build. It adds `--layout system` and a staging directory.

`--layout` takes two values:

- `private` is the default. Every file goes below `--prefix`. Use it for a
  development build, `--preview` and `--register-session`.
- `system` also writes the files a package ships outside the private prefix, below
  `--system-prefix` (default `/usr`): links to `gnoblin` and `gnoblinctl` in `bin`,
  the session file, the systemd user units, the portal configuration and the polkit
  action. `scripts/install-system-layout.sh` lists them.

`--destdir DIR` installs below `DIR`, as a package build root. Nothing outside `DIR`
changes.

```sh
./build.sh --layout system --prefix /usr/lib/gnoblin --destdir "$PWD/stage"
```

The runtime is now in `stage/usr/lib/gnoblin/`, and `stage/usr/bin/gnoblin` links to
it. Copy `stage/` into the package root. Add `--without-portal` for a package that
ships the portal backend separately. To check the result against the RPM file lists,
run `tests/system-layout.test.sh --stage stage`.

`--system-prefix` needs `--layout system`. `--preview` and `--register-session` use
the private layout. Set `GNOBLIN_BUILD_DIR` to build in a directory other than
`build/ninja`, so a package build can sit next to a development build.

Use `./build.sh --preview --terminal kitty` to choose a terminal.
Use `--without-xwayland` only if you run Wayland-native applications; X11-only
applications cannot open in that build. Rebuild without the option to restore
XWayland support.

The session and compositor run from one `gnoblin` executable. The build shows
outer Ninja entries and nested build progress at roughly 10%
intervals, plus up to 14 recent lines of other output per entry. It saves
every line under `build/logs/` and points to that log on failure. Use
`--verbose` to watch every command and diagnostic as it runs.

Stage and result colors appear in a terminal. Set `NO_COLOR=1` to disable them;
redirected output and log files contain plain text.

Compiler temporary files use `build/tmp` so a separate `/tmp` quota does not
interrupt a build with space available in the checkout.

## How GNOME stays separate

Gnoblin's patched Mutter stays in the private build prefix. The runtime and
optional portal backend use development libraries installed by your
distribution. The build does not add library paths to your shell profile or
the system loader configuration.

Gnoblin adds its own login entry and leaves the distribution's GNOME session
available. Select the GNOME entry at login to return to the normal GNOME
desktop.

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
used before: `./build.sh --register-session`.

Registration updates Gnoblin's user-unit links to the new build. If a custom
unit uses one of those names, move it aside first.
Log out and back in to use the new compositor.

## Remove the local session

Log into another session. Remove only the files created by local registration.
The `org.gnoblin.Shell*` and `gnome-session@gnoblin` paths below are included to
clean up registrations created by older Gnoblin builds. When you use
`scripts/install-system.sh`, it removes the exact managed GNOME session drop-in
after a successful DNF transaction. It stops if that file was edited.

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
# Remove the system default installed by earlier Gnoblin builds.
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
