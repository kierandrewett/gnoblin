<div align="center">

# Gnoblin

**The GNOME session for people who bring their own shell.**

Mutter and GNOME Shell underneath. Your bar, dock and launcher on top.

[Install](docs/installation.md) · [Quick start](docs/bring-your-own-shell.md) · [Configure](docs/configuration.md) · [Contribute](CONTRIBUTING.md)

</div>

Gnoblin is a drop-in Wayland session built from GNOME. It keeps the parts of
GNOME that make a desktop work — windows, workspaces, input, portals, polkit,
keyring and hardware integration — and leaves the visible desktop chrome to a
layer-shell client.

Use [Bingux](https://github.com/kierandrewett/bingux), Waybar or your own shell
for the top bar, dock, launcher, notifications and OSD. Gnoblin provides the
Wayland protocols and the small `org.gnoblin.Shell` API those clients use.

## Features

- GNOME-compatible login session with GDM and the regular GNOME session kept intact
- Mutter window management, workspaces and keyboard input
- `zwlr_layer_shell_v1` plus compositor protocols for external shells
- D-Bus control for feature ownership, reloads, input sources and window state
- Live TOML configuration for protocol and window behaviour
- External ownership of notifications, OSD and screenshot UI
- Nested devkit so you can test a shell without logging out
- Fedora RPMs, a NixOS module and a source-prefix development path

## Quick start

Build a private prefix and try Gnoblin in a nested session:

```sh
just init
just dev
just gnome-devkit
```

Start a layer-shell client from the terminal that opens:

```sh
qs -p /path/to/your/shell.qml
# or: waybar
```

The nested session is safe to close. For a real login, follow the [installation
guide](docs/installation.md), then enable your shell using [Bring your own
shell](docs/bring-your-own-shell.md).

## Install

- **Fedora:** build the RPMs in [Distribution](docs/distribution.md). The COPR
  project exists, but its builds still need clean-host login verification.
- **NixOS:** use the flake and module in [Installation](docs/installation.md#nixos).
- **Other systems:** build the private source prefix described in
  [Installation](docs/installation.md#get-the-source).

Gnoblin intentionally starts without a top bar, dock or overview until a shell
is running. That is the contract: GNOME underneath, your desktop on top.

## Develop

```sh
just test            # fast deterministic checks
just verify          # build and run the headless session suite
just verify-release  # add host Mutter tests and RPM builds
```

Read [Testing](docs/testing.md) for what each gate proves. The [source
map](src/README.md) explains where compositor protocols, session data and shell
integration live.

## Documentation

- [Installation](docs/installation.md)
- [Bring your own shell](docs/bring-your-own-shell.md)
- [Configuration and `gnoblinctl`](docs/configuration.md)
- [Compositor bridge](docs/compositor-bridge.md)
- [Distribution and COPR](docs/distribution.md)
- [Devkit](docs/devkit.md)
- [Testing](docs/testing.md)
- [Real-hardware verification](docs/real-hardware-verification.md)
- [Contributing](CONTRIBUTING.md)
