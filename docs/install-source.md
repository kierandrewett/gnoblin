# Build from source

Use this route for current development code. For a packaged install, use
[Fedora's COPR packages](install-fedora.md) or the
[Debian/Ubuntu downloads](install-debian.md).

The build goes into `./install` inside your checkout.
Keep the checkout there if you register it as a login session.

## Prerequisites

This is a native source build. You still need a C/C++ toolchain, Python 3.11 or
newer, Meson, Ninja, Git, Just and the base development libraries for GNOME.
The private build supplies GLib, GJS, Wayland, Wayland protocols, libinput,
mtdev, Lua, gnome-desktop and the PipeWire client libraries. Other development libraries must already be installed on the host.

Fedora, Arch, Debian/Ubuntu and openSUSE use the same private build path.
The required library versions are checked during the build. If a base dependency
is missing or too old, the build stops and reports it without changing host packages.

## 1. Get the source

Install Git with your distribution's package manager, then:

```sh
git clone https://github.com/kierandrewett/gnoblin.git
cd gnoblin
./build.sh
```

The script builds pinned dependency versions in `./install/deps`, then builds
Mutter, GNOME Shell and the Gnoblin session in `./install`. It never calls your
system package manager. Run it as your normal user.

The dependency sources and checksums are in `build-dependencies.json`. Completed dependency
builds are reused. Settings and the patched portal remain
[optional builds](source-development.md#optional-components).

## 2. Try it in a window

From an existing Wayland desktop:

```sh
GNOBLIN_PREFIX="$PWD/install" just gnome-devkit
```

A nested desktop and terminal open. Launch your layer-shell client from that
terminal. Close the terminal to end the test. See [Devkit](devkit.md) for help.

## 3. Add a login session

After the build and nested test succeed:

```sh
GNOBLIN_PREFIX="$PWD/install" just dev-session-register
```

Run the `sudo install` commands it prints. Then
[install a shell](bring-your-own-shell.md), log out and select **Gnoblin**.

Registration only adds session files; it does not build a missing runtime.

## Build options

| Command                  | Behaviour                                |
| ------------------------ | ---------------------------------------- |
| `./build.sh`             | Build private dependencies, then Gnoblin |
| `./build.sh --deps-only` | Build only the private dependencies      |
| `./build.sh --no-deps`   | Reuse dependencies and rebuild Gnoblin   |
| `./build.sh --dry-run`   | Show what will be built                  |

`--yes` and `--install-deps` remain accepted for older scripts. Neither enables
host package installation.

## How GNOME stays separate

Dependency headers, libraries and tools stay in `./install/deps`. Gnoblin's
binaries link to that directory. Its library paths are not written to your
shell profile or the system loader configuration. The dependency tool directory
is not added to application launch paths.

The host still provides the kernel, graphics drivers, system services and
compatible base libraries. The private PipeWire client connects to the existing
audio service; the build does not register another PipeWire service.

## Update

From the checkout:

```sh
git pull --ff-only
./build.sh
```

Preserve local changes if Git refuses the update. Log out and back in after
rebuilding; configuration reload cannot replace compositor libraries.

## Remove the local session

Log into another session. Remove only the files created by local registration:

```sh
rm ~/.config/systemd/user/org.gnoblin.Shell.target
rm ~/.config/systemd/user/org.gnoblin.Shell@wayland.service
rm ~/.config/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf
systemctl --user daemon-reload
sudo rm /usr/share/wayland-sessions/gnoblin.desktop
sudo rm /usr/share/gnome-session/sessions/gnoblin.session
```

You can then remove the checkout's `build` and `install` directories.
For a packaged install, use the package manager instead.

## Missing or outdated dependencies

The build stops at a missing dependency instead of changing host packages.
Keep the error and the Meson log path when reporting a build problem.

The pinned GNOME versions are in [gnome-versions.json](../gnome-versions.json).
See [source development](source-development.md) for component rebuilds.
