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
  `foreign-toplevel-management/` share the entry point in `aggregator/` and
  the generated `40-gnoblin-protocols` wiring patch.
- GNOME 51 supplies `ext-background-effect-v1`; Gnoblin carries only the
  Shell policy adapter for its committed regions in
  `patches/mutter/62-background-effect/`.
  See [background effects](../../docs/background-effects.md) for semantics and tests.
- `foreign-toplevel-common/` contains helpers shared by the two foreign
  toplevel protocols; it does not advertise a global itself.

All Gnoblin-owned globals are available only in the Gnoblin session. Each
defaults on within that session and can be disabled through its `protocols`
key in `init.lua`.

`session-lock/` and `output-management/` are deferred implementation plans
with vendored protocol XML. They are not compiled, registered, configurable,
or claimed as supported.

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
