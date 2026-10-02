# Source development

Start with [build from source](install-source.md). The source build uses Ninja
to coordinate Gnoblin, Mutter, and session files. The GTK-based portal backend
is optional.

## Build a component

The default command builds the standalone session. It uses the portal frontend
and backend installed on your system:

```sh
./build.sh
```

To include Gnoblin's GTK-based portal backend, use:

```sh
./build.sh --with-portal
```

It requires GTK4 4.22 or newer and `xdg-desktop-portal` 1.21.1 or newer. You
can instead configure another installed backend for Gnoblin as described in
[Choose a different portal backend](/gnome-apps#choose-a-different-portal-backend).

To rebuild one upstream component, select its CMake target:

```sh
./build.sh --target mutter
./build.sh --target xdg-desktop-portal-gnome
```

Each target prepares its pinned source and checks its own development
dependencies. Use the default command after rebuilding to install the complete
session into the selected prefix.

Native compositor changes require a fresh session. A Lua config reload does not
load rebuilt libraries. For Lua API methods, events, and operation results, use
the [runtime API reference](/config/runtime-api) and [event catalog](/config/lua-events).

## Preview a change

Build the nested viewer when needed and start a private Gnoblin session:

```sh
./build.sh --preview
```

The preview uses a temporary home, config, data, cache, state, runtime
directory, and D-Bus session. It still uses the host Wayland display and may
connect to the host PipeWire socket. This is a development session, not a
sandbox. See the [devkit guide](/devkit) for options and isolation details.

## Choose a prefix

The default build prefix is `./install`, with libraries in `lib64`.

```sh
GNOBLIN_LIBDIR=lib ./build.sh --prefix /tmp/gnoblin
./build.sh --prefix /tmp/gnoblin --preview
```

`GNOBLIN_LIBDIR` is relative to the prefix. Use the same prefix for subsequent
build and preview commands. System prefixes `/usr` and `/usr/local` are
rejected. To add the built session to your login screen, run:

```sh
./build.sh --prefix /tmp/gnoblin --register-session
```

## Portal backend

The default build uses portal services already installed on the system. To
build Gnoblin's optional backend, run `./build.sh --with-portal`. A GNOME
session can continue using GNOME's backend. To choose another backend for
Gnoblin, see [Choose a different portal backend](/gnome-apps#choose-a-different-portal-backend).
See [permission policy](/guides/permissions) when testing remote access.

## Verify changes

Run deterministic checks and the native runtime tests with:

```sh
just check
just test-runtime
```

Use `just test-preview` for an isolated live session. It requires a working
Wayland desktop and a current build. A successful build or CTest run does not
verify login on a real seat; see [hardware verification](real-hardware-verification.md).

Keep upstream submodules at their pinned commits. Put Gnoblin-owned behavior in
`src/`, protocol changes in `src/protocols/`, and narrow upstream fixes in
`patches/`. Export committed subproject changes with
[`scripts/manage-patches.py`](https://github.com/kierandrewett/gnoblin/blob/main/scripts/manage-patches.py)
so patch identity and headers stay consistent.
