# gnoblin

gnoblin is **patched GNOME** — a pinned **Mutter** (49.5) and **GNOME Shell**
(49.6) with a thin, purpose-built patch set — turned into a compositor you can
build a desktop *on top of*. Mutter gains `wlr-layer-shell` and a stack of
Wayland protocol overlays so any layer-shell client can draw the UI. GNOME Shell
keeps running as the compositor and session manager, but its stock chrome (top
bar, Activities overview, dash, app grid) is stripped by a **session mode**, not
by heavy JS surgery. What draws the bar, dock and launcher is **bring-your-own**
— Quickshell, waybar, a custom layer-shell client, or nothing at all.

> gnoblin previously shipped a from-scratch C++ compositor plus Rust/Slint
> layer-shell clients. That stack was **retired**; recover it from the git tag
> `archive/cpp-compositor`. Everything below describes the current
> patched-GNOME direction.

## Documentation

- [Installation](docs/installation.md) — dependencies, building, installing
  the session, picking Gnoblin at the login manager.
- [Devkit](docs/devkit.md) — the fast loop for iterating on your own chrome
  without logging out.
- [Testing](docs/testing.md) — what each `just *-verify` recipe proves.
- [Configuration](docs/configuration.md) — `gnoblin.conf`, the
  `org.gnoblin.shell` GSettings schema, `gnoblinctl`.
- [Real-hardware verification](docs/real-hardware-verification.md) — what
  the headless suite can't cover (GDM login, unattended screensharing).

## What's in it

- **Mutter overlays** (`src/protocols/`, C on top of the pinned Mutter tree):
  `wlr-layer-shell` (v5), `wlr-screencopy`, `ext-idle-notify`,
  `ext-foreign-toplevel-list`, `wlr-foreign-toplevel-management`,
  `wlr-gamma-control` (so `wlsunset` night-light works),
  `wlr-output-power-management`, and `ext-data-control` (so `cliphist` works).
  These globals are registered only in the Gnoblin session; within that boundary,
  each defaults on and has a `gnoblin.conf` `[protocols]` off switch.
  `session-lock/` and `output-management/` contain deferred design and protocol
  scaffolding only; they are not built or advertised.
- **GNOME Shell patches** (`patches/gnome-shell/`): relaxed extension loading
  in Gnoblin mode (skip shell-version validation), correct portal Access
  request cancellation, mode-scoped notification-daemon ownership, a
  `--disable-extensions` runtime flag, Gnoblin-only native top-bar suppression,
  Gnoblin branding, a Wayland soft-reload (`Alt+F2` `r` reloads in-process so
  windows survive), and the `org.gnoblin.shell` feature schema.
- **Shell privilege policy**: `org.gnome.Shell.Eval` keeps its upstream guard
  in normal, headless and login sessions. Developers can deliberately enable
  Mutter's native `--unsafe-mode` for one isolated devkit process with
  `GNOME_DEVKIT_UNSAFE_MODE=1`.
- **Session mode** (`src/data/session/modes/gnoblin.json`): inherits the stock
  `user` mode, empties the panel, sets `hasOverview: false`, and keeps only
  the essential background components (polkit, keyring, automount, network
  agent) plus `gnoblinControl` — the control protocol below.
- **`org.gnoblin.Shell` control protocol** — a first-class GNOME Shell session
  component (`src/gnome-shell-overlay/js/ui/components/gnoblinControl.js`,
  registered via `patches/gnome-shell/30-gnoblin-control`). It exposes over
  D-Bus:
  - `Ping` / `GetVersion` — health + version.
  - `Reload` — Wayland soft-reload (reloads theme + extensions in-process;
    windows survive). Also bound to `Alt+F2` `r`.
  - `ListFeatures` / `GetFeature` / `SetFeature` / `FeatureChanged` — toggle
    gnome-shell subsystems (e.g. `osd`, `screenshot`) on/off at runtime so an
    external userspace can own them. Persisted in the `org.gnoblin.shell`
    `disabled-features` GSettings key.

## Layout

```
src/
  protocols/    Mutter Wayland protocol overlays + aggregator/
  config/       shared C ini parser (gnoblin.conf), compiled into the overlays
  data/         session mode, gnome-session, gschema, conf
  gnome-shell-overlay/  the org.gnoblin.Shell control component
subprojects/    pinned upstream submodules, kept PRISTINE (mutter 49.5, gnome-shell 49.6)
patches/        edits to upstream files, grouped by purpose (mutter/, gnome-shell/)
scripts/        apply-patches, copy-overlay, install-session, verify + test scripts
Justfile        build + test orchestration
```

The submodules are **never modified in the repo**. gnoblin's changes come from
two places, applied onto the pinned tags at build time:

- **Overlay (`src/protocols/`, `src/config/`, `src/gnome-shell-overlay/`)** —
  large new files we author live here as real, navigable source with a
  `manifest` (`<project> <src> <dest>` lines). `scripts/copy-overlay.sh` copies
  them into the submodule tree; they're gitignored there and removed on reset,
  so the submodule stays pristine.
- **Patches (`patches/`)** — for *editing existing* upstream files (wiring the
  overlays into the build, the shell changes above). Applied with `git am`.

## Build

```sh
just init        # fetch pinned GNOME checkouts and mandatory Meson subprojects
just dev         # build patched mutter + gnome-shell + session data into ./install
```

`just dev` builds the whole stack into a local `./install` prefix (no system
install). Individual steps: `just dev-mutter`, `just dev-gnome-shell`,
`just dev-session`. It needs the usual mutter + gnome-shell build deps.

## Run

```sh
just gnome-devkit
```

Fastest way to try it: a visible nested gnoblin session (a window in your
current Wayland session) plus a terminal wired to it, so you can run your
own chrome against a real gnoblin compositor without touching your login
session. See [docs/devkit.md](docs/devkit.md).

For a real login session: `just dev-session` installs the session data into
the prefix, `just dev-session-register` links it with your systemd --user
instance and prints the (root) command to add "Gnoblin" to your login
manager's picker. See [docs/installation.md](docs/installation.md).

## Configure

Two configuration surfaces:

- **`gnoblin.conf`** — a sectioned INI read by the Mutter overlays. In the
  Gnoblin session, its `[protocols]` section can disable any implemented
  Gnoblin protocol (all implemented entries default on). Stock session modes
  never register these globals. See `src/data/gnoblin.conf.example` for the keys.
- **`org.gnoblin.shell` GSettings** — holds `disabled-features`, the runtime
  feature toggles above.

The session mode (`src/data/session/modes/gnoblin.json`) removes the overview,
dash, app grid and panel contents. A small mode-scoped Shell patch also makes
the native panel non-interactive and non-strutting in Gnoblin only; stock GNOME
keeps its upstream chrome. Full reference (feature ids, `gnoblinctl`, grammar):
[docs/configuration.md](docs/configuration.md).

## Tests

```sh
just test            # deterministic fast checks; no compositor integration
just verify          # rebuild + complete isolated headless integration suite
just verify-release  # verify + Mutter real-host suite + both RPM builds
```

`just verify-installed-headless` reruns the integration suite against the
current prefix without rebuilding. The GNOME Shell integration recipes are
headless; `test-mutter` in the release gate still needs a real seat and working
local file monitoring. Full recipe reference (what each command proves and
what it needs): [docs/testing.md](docs/testing.md). For GDM login, a visible
bring-your-own chrome check, and unattended screensharing, see
[docs/real-hardware-verification.md](docs/real-hardware-verification.md).

## Authoring a protocol overlay

Drop the implementation under `src/protocols/<feature>/` with a `manifest`
(`mutter <src> <dest>` lines) and vendored protocol XML, add its init call to
`src/protocols/aggregator/`, regenerate the wiring patch with
`scripts/gen-gnoblin-protocols-patch.sh`, and gate it via a `[protocols]`
`gnoblin.conf` key.
