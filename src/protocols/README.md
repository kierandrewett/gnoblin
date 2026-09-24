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

`session-lock/` provides the standard `ext-session-lock-v1` global in the
Gnoblin session. It is a compositor security boundary, rather than shell UI:
Gnoblin covers and isolates the session, while Bingux or a compatible external
client supplies the lock screen and authentication. Regular GNOME sessions do
not receive the global and retain GNOME ScreenShield.

The lock controller replaces normal desktop capture with its lock scene for
already authorised portal monitor streams. After `locked` and lock-scene
presentation, authorised remote input reaches the active lock surface; it is
refused during transitions and failsafe. This is automatic; it does not add a
protocol setting or portal opt-in.

`output-management/` remains XML only.

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
