# gnoblin

A small, self-contained Wayland desktop: a **from-scratch compositor**
(`gnoblin-shell`) that embeds **libmutter-17** as a library and drives it with
its own `MetaPlugin`, plus a set of **Rust layer-shell clients** for the panel,
dock, notifications, launcher and on-screen display.

gnoblin started as a *patched GNOME* (mutter + gnome-shell with the overview
removed and `wlr-layer-shell` added). It has since pivoted: gnome-shell is gone,
replaced by gnoblin-shell — a concrete, owned compositor (not a patch pile) — and
the UI is bring-your-own layer-shell clients, with first-class ones gnoblin
ships. mutter is still used as a library and carries a thin set of protocol
overlays; everything else is gnoblin's own code.

## What's in it

**Compositor** (`src/compositor/`, C++ on `libmutter-17`):

- Window management: edge snapping with customisable regions, a drag-to-edge
  tile preview, a config-driven right-click window menu, Super+drag move/resize.
- Effects (config-tunable curves + durations): open / close / minimize /
  unminimize / resize / workspace-slide animations; opt-in rounded corners and
  soft drop shadows (analytic SDF shaders); solid fallback background behind
  the layer-shell wallpaper.
- A session **lock screen** (overlay + Clutter input grab + PAM).
- A dogfooded D-Bus API, `dev.gnoblin.Shell`: `Dispatch`, `ListActions`,
  `WorkspaceState`, `ListWindows`, `ActivateWindow`, plus `spawn` (Hyprland-style
  exec binds) and `lock` — the same verbs keybindings and `gnoblinctl` use.

**Protocols** (`src/protocols/`, mutter overlays): `wlr-layer-shell`,
`wlr-screencopy`, `ext-idle-notify`, `ext-foreign-toplevel-list`,
`wlr-foreign-toplevel-management`, `wlr-gamma-control` (so `wlsunset` night-light
works), `wlr-output-power-management`, `ext-data-control` (so `cliphist` works).

**Clients** (`src/clients/`, Rust + Slint layer-shell clients, all themeable):

| Client | Role |
|---|---|
| `gnoblin-topbar` | workspaces · clock · system tray (StatusNotifierWatcher + dbusmenu) · volume · network · battery · quick-settings (volume/brightness sliders, Wi-Fi/Bluetooth toggles) · power menu |
| `gnoblin-dock` | taskbar (running windows + favourites), launch / raise |
| `gnoblin-notifyd` | `org.freedesktop.Notifications` daemon + popups |
| `gnoblin-launcher` | Slint app search and grid (Super+Space / Super+A) |
| `gnoblin-osd` | volume / brightness on-screen display (media keys) |
| `gnoblin-wallpaper` | swappable layer-shell wallpaper |

## Layout

```
src/
  compositor/   gnoblin-shell — the C++ libmutter compositor
  config/       shared C config parser (gnoblin.conf)
  protocols/    mutter wayland protocol overlays + aggregator/
  clients/      the Rust layer-shell clients (above)
  data/         gschema, gnoblin.conf.example
subprojects/    upstream submodules pinned to tags, kept PRISTINE (mutter @ 49.5)
patches/        EDITS to upstream files, grouped by purpose
scripts/        apply-patches, copy-overlay, run-devkit, install-userspace, tests
Justfile        build + test orchestration
```

The mutter submodule is **never modified in the repo**. gnoblin's mutter changes
come from two places, applied onto the pinned tag at build time:

- **Overlay (`src/protocols/<feature>/`, `src/config/`)** — large new files we
  author (the protocol implementations, the config parser) live here as real,
  navigable source. `scripts/copy-overlay.sh` finds every `manifest`
  (`<project> <source> <dest>`) under `src/` and copies the files into the
  submodule; they're gitignored there and removed on reset, so it stays pristine.
- **Patches (`patches/`)** — for *editing existing* upstream files (wiring the
  overlays into mutter's build). Applied with `git am`.

## Build

```sh
just init        # fetch the pinned mutter submodule
just dev         # build patched mutter + gnoblin-shell + the Rust clients into ./install
```

`just dev` builds everything into a local `./install` prefix (no system install).
It needs the usual mutter build deps plus a Rust toolchain, `gtk4-layer-shell`,
and `libpam`. The first Rust build compiles all of gtk4-rs (~40s); later builds
are incremental. Individual steps: `just dev-mutter`, `just dev-shell`,
`just dev-userspace`.

## Run

```sh
# Nested/headless smoke (boots gnoblin-shell, lists the protocols it serves):
just devkit-verify

# Windowed nested compositor in your current Wayland session (needs a GPU):
./scripts/run-devkit.sh visible

# Capture a screenshot of the running stack to a PNG (needs a GPU backend):
SHOT_NESTED=1 ./scripts/run-devkit.sh shot   # -> /tmp/gnoblin-shot.png

# Dump the live compositor scene inspector over gnoblinctl's control socket:
gnoblinctl inspect
gnoblinctl inspect /tmp/gnoblin-scene.json
```

For a real session, install a `gnoblin.desktop` session entry pointing at
`gnoblin-shell` and select it at the login manager. The compositor also self-
screenshots on demand: set `GNOBLIN_SHOT=/path.png` (and optionally
`GNOBLIN_SHOT_DELAY` ms) in its environment.

## Configure

gnoblin reads `~/.config/gnoblin/gnoblin.conf` (or `$GNOBLIN_CONFIG`) — a
sectioned INI, watched live (saving re-applies keybinds, animations, wallpaper).
See `src/data/gnoblin.conf.example` for every key: `[startup] exec`,
`[appearance]` (background / wallpaper / rounding / shadow / accent),
`[animations]`, `[input]`, `[bind]` (`Accel = action [arg]`), `[snap]`, `[menu]`,
`[lock]`. Keybinds, `gnoblinctl`, and the `dev.gnoblin.Shell` D-Bus compatibility
API all share one set of actions — `gnoblinctl actions` lists them.

## Tests

```sh
just test            # everything runnable without a display
  test-build         #   patches apply, protocol lints, schema default
  test-clients       #   Rust unit tests: volume/brightness/network/dbusmenu/tray/app-match parsers
  test-logic         #   C tests: lock PAM rejects wrong passwords; shadow/corner SDF geometry
just test-mutter     # mutter in-tree headless layer-shell tests (needs a dev build)
just test-devkit     # gnoblin-shell devkit integration tests (needs a dev build)
just test-devkit-visible # exact `just devkit` visible startup/log/cleanup smoke
```

Build, boot, protocol, D-Bus, and the logic/security/geometry tests run headless.
Pixel-level rendering and interactive input (the visual look of the bar/dock/
shadows, a typed lock password) require a real GPU Wayland session. The visible
devkit smoke also opens a Mutter Devkit window on the host session, so it is kept
separate from the default headless integration recipe.

## Authoring a protocol overlay

Drop the implementation under `src/protocols/<feature>/` with a `manifest`
(`mutter <src> <dest>` lines) and vendored protocol XML, add its init call to
`src/protocols/aggregator/`, regenerate the wiring patch with
`scripts/gen-gnoblin-protocols-patch.sh`, and gate it via
`gnoblin_config_get_bool("protocols", "<name>", TRUE)`.
