# Wayland Protocol Overlays

`src/protocols/` contains Gnoblin-owned Mutter overlay code. These sources are
not built in place. `scripts/copy-overlay.sh` copies the paths declared by each
`manifest` into the Mutter checkout before the patch stack adds Meson and
startup wiring.

## Implemented protocols

- `layer-shell/` and `screencopy/` use dedicated wiring patches under
  `patches/mutter/30-layer-shell/` and `30-screencopy/`.
- `idle-notify/`, `data-control/`, `gamma-control/`,
  `output-power-management/`, `foreign-toplevel-list/`, and
  `foreign-toplevel-management/` and `session-lock/` share the entry point in
  `aggregator/` and the generated `40-gnoblin-protocols` wiring patch.
- GNOME 51 supplies `ext-background-effect-v1`; Gnoblin's
  `patches/mutter/62-background-effect/` adds committed-region handling and
  applies the `protocols.ext_background_effect_v1` gate in Gnoblin mode.
  See [background effects](../../docs/background-effects.md) for semantics and tests.
- `foreign-toplevel-common/` contains helpers shared by the two foreign
  toplevel protocols; it does not advertise a global itself.

All Gnoblin-owned globals are available only in the Gnoblin session. Each
defaults on within that session and can be disabled through its `protocols`
key in `init.lua`.

`session-lock/` registers the secure `ext_session_lock_manager_v1` global in
the Gnoblin session when its protocol gate is enabled. `output-management/`
remains XML only and does not register a global.

## Adding an aggregated protocol

1. Add the implementation, protocol XML, and `manifest` under this directory.
2. Add its init call to `aggregator/meta-gnoblin-protocols.c`.
3. Add its source and XML basenames to `scripts/gen-gnoblin-protocols-patch.sh`.
4. Regenerate and reapply the Mutter wiring patch.
5. Add a `[protocols]` gate and update `src/data/gnoblin.conf.example`.
6. Add a focused protocol client under `tests/` and include it in
   `tests/test-protocol-boundaries.sh`.

Run `just test-protocols` for protocol contracts,
`just test-stock-gnome` for session scoping, and
`just test-protocol-gating` for the configuration gate. Run
`just test-window-manager` on a real host before release.
