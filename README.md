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

## What's in it

- **Mutter overlays** (`src/protocols/`, C on top of the pinned Mutter tree):
  `wlr-layer-shell` (v5), `wlr-screencopy`, `ext-idle-notify`,
  `ext-foreign-toplevel-list`, `wlr-foreign-toplevel-management`,
  `wlr-gamma-control` (so `wlsunset` night-light works),
  `wlr-output-power-management`, `ext-data-control` (so `cliphist` works),
  plus session-lock and output-management scaffolding. Each is gated by a
  `gnoblin.conf` `[protocols]` key so you can turn any of them off.
- **GNOME Shell patches** (`patches/gnome-shell/`): relaxed extension loading
  (skip shell-version validation), unsafe-mode at startup, portal Access dialog
  auto-grant (unattended screensharing), a `--disable-extensions` runtime flag,
  hidden native top-bar chrome, Gnoblin branding, a Wayland soft-reload
  (`Alt+F2` `r` reloads in-process so windows survive), and the
  `org.gnoblin.shell` feature schema.
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
  data/         session mode, gnome-session, gschema, conf, QS plugin scripts
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
just init        # fetch the pinned mutter + gnome-shell submodules
just dev         # build patched mutter + gnome-shell + session data into ./install
```

`just dev` builds the whole stack into a local `./install` prefix (no system
install). Individual steps: `just dev-mutter`, `just dev-gnome-shell`,
`just dev-session`. It needs the usual mutter + gnome-shell build deps.

## Run

```sh
# Headless smoke: boot gnome-shell in the gnoblin session mode and confirm it
# starts and advertises zwlr_layer_shell_v1 (so any layer-shell client can draw):
just gnome-verify

# Headless: exercise the org.gnoblin.* control protocol over D-Bus
# (Ping / GetVersion / Reload round-trip):
just gnome-dbus-verify
```

For a real session, `just dev-session` installs a `gnoblin.desktop` /
`gnoblin.session` entry into the prefix; select "gnoblin" at the login manager.
Then run whatever chrome you like as layer-shell clients.

## Configure

Two configuration surfaces:

- **`gnoblin.conf`** — a sectioned INI read by the Mutter overlays. Its
  `[protocols]` section gates each Wayland protocol on/off (all on by default,
  a missing file or key falls back to the caller's default). See
  `src/data/gnoblin.conf.example` for the keys.
- **`org.gnoblin.shell` GSettings** — holds `disabled-features`, the runtime
  feature toggles above.

The stock GNOME UI strip is configured entirely by the session mode
(`src/data/session/modes/gnoblin.json`), not by editing shell JS.

## Tests

```sh
just test              # config-parser logic test, no display
just test-mutter       # mutter in-tree headless functional suite (unit/Wayland/native + focus)
just gnome-verify           # boots gnome-shell in gnoblin mode, checks zwlr_layer_shell_v1
just gnome-dbus-verify      # org.gnoblin.* control protocol round-trip (needs ./install)
just gnome-hot-reload-verify  # live extension code hot-reload
just gnome-scripting-verify   # GJS user-scripting layer
```

The suite is headless. For the bits that need a real GPU / login session / root
(logging in at GDM, bring-your-own chrome, unattended screensharing), see
[docs/real-hardware-verification.md](docs/real-hardware-verification.md).

## Authoring a protocol overlay

Drop the implementation under `src/protocols/<feature>/` with a `manifest`
(`mutter <src> <dest>` lines) and vendored protocol XML, add its init call to
`src/protocols/aggregator/`, regenerate the wiring patch with
`scripts/gen-gnoblin-protocols-patch.sh`, and gate it via a `[protocols]`
`gnoblin.conf` key.
