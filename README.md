<div align="center">

# Gnoblin

A standalone Wayland compositor and session runtime, written in Rust.

[Documentation](https://gnoblin.org/) · [Install](docs/installation.md) · [Choose a shell](docs/bring-your-own-shell.md) · [Configure](docs/config.md) · [Protocols](#supported-protocols) · [Contribute](CONTRIBUTING.md)

</div>

Gnoblin provides window management and Wayland interfaces for an external
desktop shell such as [Bingux](https://github.com/kierandrewett/bingux), or a
setup built from Waybar and other layer-shell clients.

Gnoblin is built on [Smithay](https://github.com/Smithay/smithay). It does not ship a desktop shell UI.

## Run a shell

Gnoblin starts a compositor and one shell, which you can write in TypeScript, Python or Rust (JavaScript runs too). These
are the ways to choose it:

```sh
gnoblin ./shell.ts                     # run this file; a TypeScript, Python (shell.py) or Rust (libmyshell.so) shell runs the same way
gnoblin ./target/debug/libmyshell.so   # a Rust shell; add --watch to reload it when it is rebuilt
gnoblin --shell tiler                  # run the installed shell called tiler
gnoblin                                # run the shell Gnoblin finds: ~/.config/gnoblin/shell.ts, shell.py or shell.so, or the default
```

`gnoblin --list-shells` prints every shell Gnoblin can see and marks the one that would run. See
[Start Gnoblin and choose a shell](docs/starting-gnoblin.md) for how it chooses, [Build a desktop](docs/build-a-desktop.md)
and the [examples](examples/README.md) for small example shells, and the [TypeScript](docs/typescript-shell.md),
[Python](docs/python-shell.md) and [Rust](docs/rust-shell.md) shell guides.

## Features

- **Bring your own chrome.** Use [Bingux](https://github.com/kierandrewett/bingux),
  [Waybar](https://github.com/Alexays/Waybar) or another layer-shell client
  for the bar, dock, launcher and notifications. Gnoblin does not impose a
  replacement desktop shell.
- **Layer-shell first.** `zwlr_layer_shell_v1` supports panels,
  docks, wallpapers, launchers and overlays, including layer popups and
  exclusive zones. See [bring-your-own-shell](docs/bring-your-own-shell.md).
- **Cursor themes.** [Configure compositor cursors](docs/guides/cursors.md) with
  the generic `cursor.theme` and `cursor.size` settings in Gnoblin's live
  config. Xcursor themes work on every install, and an uninstalled theme falls
  back to the default cursor. Gnoblin also reads compiled Hyprcursor themes, with animated frames that preserve
  hotspots and timing.
- **Compositor effects.** Configure [blur, opacity, rounded corners, borders,
  shadows and layer animations](docs/guides/window_effects.md) with
  window rules.
- **Live configuration.** Edit one file, in [TypeScript, Python or Rust](docs/config.md), for workspaces, shortcuts,
  window style and monitors, written as the statements of a program. Saving the file reloads it.
- **Window control and scripting.** [`gnoblinctl`](docs/cli/index.md) exposes windows, workspaces,
  monitors and input sources. Listeners in your shell and the user-private compositor
  bridge expose supported runtime state and actions.
- **External desktop controls.** A separate shell or desktop clients provide
  bars, docks, launchers, notifications and other visible controls.
- **Input methods.** Gnoblin includes an IBus input method, so GTK4 and Qt apps on
  Wayland get composed text from engines such as Russian transliteration or
  Japanese. It needs a running `ibus-daemon` and an installed engine.
- **Keyring and polkit prompts.** Gnoblin acts as the polkit agent and draws its
  own password dialogs for polkit and the keyring's unlock prompt. For polkit it
  checks the password with polkit's own helper, and no script can answer. A shell
  may draw the keyring prompt instead. See
  [authentication](docs/shell-api/authentication.md) and
  [keyring passphrase prompts](docs/shell-api/requests.md).
- **Portal routing.** Gnoblin routes portal interfaces to the backend you choose.
  It does not manage app permissions; the selected backend decides them. The
  Gnoblin backend shows its normal consent dialogs.
- **Shells in three languages.** Configure the compositor and handle supported runtime events
  in TypeScript, Python or Rust, with the same objects in each. See [event handlers](docs/user-scripts.md) and the
  [TypeScript shell](docs/typescript-shell.md), [Python shell](docs/python-shell.md) and [Rust shell](docs/rust-shell.md).

## Supported protocols

The [Wayland protocol catalogue](docs/wayland-protocols.md) lists every global a session advertises, its version, who can
bind it and the setting that switches it off. It includes:

- `wlr-layer-shell`, `ext-background-effect-v1` and `wlr-screencopy`
- The foreign-toplevel protocols and `ext-data-control`
- `ext-idle-notify`, `wlr-gamma-control` and `wlr-output-power-management`
- `ext-session-lock`

Gnoblin exposes standard `ext-session-lock-v1` in its own session. It does not
ship a built-in lock screen or choose a locker. Bingux supplies its lock client
independently; hyprlock, swaylock, gtklock, waylock and other conforming
clients can use the same protocol. See [session locking](docs/session-lock.md).

Existing portal streams keep running. While locked, those streams show the lock
scene. Remote input is available only after `locked` and the lock scene has
presented, and is routed to the active lock surface; it is refused during
transitions and failsafe. No lock-specific portal setting is required.

## Get started

[Install](docs/installation.md) from the package repository on Fedora, openSUSE, Debian, Arch Linux or Alpine, or from
source. A source build needs Rust, `just` and the system libraries that `scripts/ci-install-system-libraries.sh` installs:

```sh
git clone https://github.com/kierandrewett/gnoblin.git
cd gnoblin
just dist /usr/local        # build in release mode and install the session; `cargo xtask dist uninstall` removes it
```

The [source instructions](docs/install-source.md) cover prerequisites, a nested try-out window and the login session.
`GNOBLIN_BACKEND=nested gnoblin` runs a session inside your current desktop without logging out; see
[backends](docs/backends.md).

[Configuration](docs/config.md) · [Window effects](docs/guides/window_effects.md) · [Event handlers](docs/user-scripts.md) · [All docs](docs/index.md)

Gnoblin is GPL-2.0-or-later.
