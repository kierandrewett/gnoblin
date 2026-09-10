# gnoblin

Gnoblin is a Wayland desktop session built from Mutter and GNOME Shell.
It manages windows, workspaces and input. A separate shell, such as
[Bingux](https://github.com/kierandrewett/bingux), supplies the top bar, dock,
search and notifications.

Gnoblin removes GNOME's panel and Activities overview from its session. It adds
layer-shell support and window-control interfaces for external desktop shells.
It retains GNOME's login integration, password prompts and hardware services.

The current Fedora packages replace the system Mutter and GNOME Shell.
The regular GNOME session remains available, but both sessions use the patched
packages. See [installation](docs/installation.md) before installing them.
Repository publication is being prepared; see [distribution](docs/distribution.md).

## Prerequisites

Start from a Gnoblin source checkout. You need `git`, [just](https://github.com/casey/just), Meson, Ninja, and the normal Mutter and GNOME Shell build dependencies. Fedora setup and the NixOS path are in the [installation guide](docs/installation.md).

## Quick start

From the checkout, build the local prefix and open a nested session:

```sh
just init
just dev
just gnome-devkit
```

`just init` fetches the pinned source trees and required Meson subprojects. `just dev` builds Mutter, GNOME Shell, and session data into `./install` without changing system files. `just gnome-devkit` opens a visible nested Gnoblin session in a Wayland host session and starts a terminal connected to it. Launch your bar, dock, or other layer-shell client from that terminal. Close the terminal to end the devkit.

For the login-manager registration step, package installation, or NixOS module, use [Installation](docs/installation.md). Test real login, visible chrome, and portal behaviour with the [real-hardware checklist](docs/real-hardware-verification.md).

## Session and safety boundaries

The Gnoblin session removes GNOME Shell chrome but retains its background services, including polkit, keyring, automount, and the network agent. It suppresses MessageTray banners; an external notification daemon can own notification UI.

`org.gnome.Shell.Eval` remains restricted in normal, headless, and default devkit sessions. Set `GNOME_DEVKIT_UNSAFE_MODE=1` only when a development tool requires Mutter's native `--unsafe-mode`; this enables it for that isolated devkit process only and ends when the process exits.

The session configures Mutter's overlay key as `Super`. When Super is released without other input, `org.gnoblin.Shell` emits `SuperReleased` for external chrome. Treat it as an event, not key state; clients must ignore protocol versions they do not support.

## Architecture

Gnoblin keeps owned source separate from the pinned upstream checkouts:

- `src/protocols/` contains Mutter protocol overlays, including `wlr-layer-shell`. `gnoblin.toml` controls whether each implemented protocol is advertised when Mutter starts.
- `src/gnome-shell-overlay/` provides the `org.gnoblin.Shell` component. Its D-Bus API supplies health checks, reloads, and runtime feature controls for an external shell.
- `src/data/` contains the session mode, GSettings schema, and configuration example.
- `patches/` changes existing upstream files. `src/` overlays are copied into the submodules during the build. Keep `subprojects/` pristine.

See the [source map](src/README.md) for ownership boundaries and implementation locations.

## Configuration

Gnoblin has two configuration surfaces:

- `gnoblin.toml` watches `[shell]` settings live, including native window switching and minimise animations. It is read from `$GNOBLIN_CONFIG`, or `$XDG_CONFIG_HOME/gnoblin/gnoblin.toml` by default. `[protocols]` changes require a new compositor session.
- The `org.gnoblin.shell` GSettings `disabled-features` key controls GNOME Shell subsystems at runtime. Use `gnoblinctl` inside a Gnoblin session to inspect or change it:

  ```sh
  gnoblinctl ping
  gnoblinctl features
  gnoblinctl disable osd
  ```

The [configuration reference](docs/configuration.md) lists supported protocols, feature IDs, file grammar, and every `gnoblinctl` command.

## Verification

```sh
just test                       # fast deterministic checks; no compositor boot
just verify                     # build, then run the complete headless suite
just verify-release             # add the real-host Mutter suite and RPM builds
```

`just verify-installed-headless` runs the isolated GNOME Shell integration suite against the existing `./install` prefix without rebuilding. The [testing guide](docs/testing.md) states what every recipe covers and which ones require a real seat or hardware.

## Detailed guides

- [Installation](docs/installation.md): source-prefix setup, login-manager registration, NixOS, optional components, and RPM packaging.
- [Devkit](docs/devkit.md): nested and headless development sessions, environment controls, and isolation.
- [Configuration](docs/configuration.md): protocol gating, feature toggles, and `gnoblinctl`.
- [Testing](docs/testing.md): local, headless, and release gates.
- [Real-hardware verification](docs/real-hardware-verification.md): checks that cannot run in the headless suite.
