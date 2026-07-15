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
- `foreign-toplevel-common/` contains helpers shared by the two foreign
  toplevel protocols; it does not advertise a global itself.

All implemented globals are available only in the Gnoblin session. Each
defaults on within that session and can be disabled through its `[protocols]`
key in `gnoblin.conf`.

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
   `scripts/test-protocol-boundaries.sh`.

Run `just gnome-protocol-boundaries-verify` for protocol contracts,
`just gnome-stock-protocol-isolation-verify` for session scoping, and
`just gnome-protocol-gating-verify` for the configuration gate. Run
`just test-mutter` on a real host before release.
