# Source development

Start with [build from source](install-source.md). This page covers rebuilding
individual components after that first build.

## Rebuild

Use the Ninja targets when rebuilding a component:

```sh
./build.sh --target mutter
```

Use `gnome-shell` or `xdg-desktop-portal-gnome` for those components. Each
target checks its own development libraries. Mutter must be built before Shell;
the portal target does not build either one. Run `./build.sh` for the full
session, including Gnoblin's portal backend.

Native compositor changes need a fresh session. A config reload does not load
rebuilt libraries.

The private `mutter` executable accepts `--gnoblin-config PATH` for a native
compositor preview. The Lua file supports:

- [`protocols`](/config/configure/protocols) and
  [`layer_shell`](/config/configure/layer_shell) for Wayland features.
- [Top-level `workspaces`](/recipes/writing-workspace) for initial IDs and names.
- [`window_management`](/config/configure/window_management) for focus,
  placement, and titlebar policy.
- [`compositor`](/config/configure/compositor) for animations and bell behavior.
- [`autostart`](/config/configure/autostart) for launching separate clients
  after the Wayland display is available.
- [`shortcuts`](/config/configure/shortcuts) for command bindings on key press
  or release, including bare Super on release.
- [`input`](/config/configure/input) for mouse, touchpad, keyboard, tablet,
  stylus, and orientation overrides.
- [`keybindings`](/config/configure/keybindings) for Mutter's `wm`, `mutter`,
  and `wayland` action groups.

Those references give each field's values and defaults. The compositor applies
the settings at startup. Autostart commands launch once per name and inherit
the Wayland display. Launch failures appear in the compositor journal. A config
reload does not restart or replace a launched command.

The native preview can load the same config file as the Shell session. It
applies the sections listed above and logs a warning for known Shell-only
sections and Lua event handlers that it skips. Unknown top-level sections still
stop startup, which helps catch misspelled settings.

The native shortcut path accepts named actions for the `wm`, `mutter`, and
`wayland` groups, alongside command shortcuts.

The native preview logs and skips `keybindings.shell`, `gnome:shell` actions,
and command shortcuts with `capture_input = true`; they need the Shell session.
Native input overrides apply at startup. A new session is needed after editing
them; the native preview does not reload the config.

The preview answers `gnoblinctl ping`, `gnoblinctl monitor list`,
`gnoblinctl window list`, workspace management by ID or number, and basic window
actions. It keeps connections open for multiple requests. Socket
clients can subscribe to window snapshots with
`{"op":"windows"}`. A Gnoblin login is still needed for the other bridge
streams, Shell commands, and lock screen.

## Choose a prefix

Default: `./install`, with libraries in `lib64`.

```sh
GNOBLIN_LIBDIR=lib ./build.sh --prefix /tmp/gnoblin
./build.sh --prefix /tmp/gnoblin --preview
```

`GNOBLIN_LIBDIR` is relative to the prefix. Use the same prefix for subsequent
build and devkit commands. System prefixes `/usr` and `/usr/local` are rejected.

## Portal backend

`./build.sh` builds the Gnoblin portal backend. Register the built session with
`./build.sh --register-session` to install its portal metadata. The portal
frontend selects it in a Gnoblin session; a GNOME session keeps GNOME's backend.
See [permission policy](/guides/permissions) before testing remote access.

## Verify

For a source build:

```sh
just test-session
```

For a Nix build, use `nix flake check` and `nix build .#gnoblin`. See [testing](testing.md) for the
full suite and [hardware verification](real-hardware-verification.md) for login checks.
