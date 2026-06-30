# Source Map

`src/` is split by ownership boundary, not by language. Start here when trying
to understand where a feature lives.

## Top-Level Areas

- `compositor/` is the standalone `gnoblin-shell` binary. It links against the
  installed patched Mutter and owns window management policy, effects, D-Bus
  control APIs, keybindings, rules, outputs, gestures, overview, lock, and
  app/window animation behavior.
- `protocols/` contains Mutter overlay sources for Wayland protocols that
  gnoblin adds to libmutter. These files are copied into the Mutter submodule by
  `scripts/copy-overlay.sh` and persisted as patch files under `patches/mutter/`.
- `clients/` contains Slint/Rust shell clients and the shared Rust support crate
  used by topbar, dock, wallpaper, notifications, OSD, launcher, and related UI
  surfaces.
- `config/` is the shared C config parser. It is used by both the compositor and
  some Mutter protocol overlays, so keep it C-compatible and byte-compatible
  with the Rust mirror in `clients/shell-ui/src/config.rs`.
- `data/` is installed runtime data: `gnoblin.conf.example`, the embedded
  `gnoblin.defaults.conf` base layer, quick-settings plugin commands, schemas,
  and manifests.

## Common Tasks

- Changing compositor behavior: start in `compositor/gnoblin-shell-plugin.cpp`
  for animation/effects hooks, or `compositor/gnoblin-control.cpp` for D-Bus
  APIs exposed to tests and clients.
- Adding a Wayland protocol: create a protocol directory under `protocols/`, add
  a `manifest`, wire it in `protocols/aggregator/`, then regenerate the Mutter
  protocol patch with `scripts/gen-gnoblin-protocols-patch.sh`.
- Changing topbar/dock/wallpaper UI: start in `clients/<role>/src/main.rs` and
  `clients/<role>/ui/*.slint`; shared Slint widgets live under
  `clients/shell-ui/vendor/slint/`.
- Changing cross-client helpers: use `clients/shell-ui/src/`. That crate owns
  desktop entry launching, app context menus, appmenu/DBusMenu helpers, shell
  D-Bus wrappers, config helpers, theme data, tray, and popout models.
- Changing the config parser or grammar: start in `config/`; keep the C parser,
  Rust mirror, and parser tests in lockstep.
- Changing shipped defaults: update `data/gnoblin.defaults.conf` for the runtime
  base layer, `data/gnoblin.conf.example` for the user-facing reference, and the
  relevant config parser tests.
- Changing built-in quick-settings tiles: update the `[qs-plugin.*]` defaults in
  `data/gnoblin.defaults.conf` and the matching `data/plugins/gnoblin-qs-*`
  command.

## Generated Or Heavy Directories

Cargo output is configured to live under `../build/cargo-target/`, alongside
the Meson build trees, so `src/` stays source-only. If an older checkout created
`clients/target/`, treat it as legacy generated output and remove it.
