# Output modes: configuration, mirror, span and profiles

Lane OutputMirror. Spec: `design/js-research/owner-spec.md` sections 9 and 21; acceptance DW1 to DW8 in
`design/js-research/spec-review.md` section 5. User page: `docs/shell-api/outputs.md`.

## What existed

- **The engine.** `crates/gnoblin-output` resolves a `LayoutConfig` against the connected outputs: mode, scale,
  orientation, absolute and relative placement, mirrors (one logical monitor with several members), overlap and gap
  checks, a transaction with rollback. The compositor used it only as `LayoutConfig::row_over`: every output in a row,
  nothing configurable.
- **`wlr-output-management`.** `BackendOutputConfig` (`crates/gnoblin-wayland/src/output_config.rs`) changes scale
  through `Backend::configure_output` with rollback and refuses a move, a transform, an off output and a mode the
  backend does not list. It still does: it is the protocol's host, not the shell's path.
- **`Screen.apply`.** In `screen.toml` with no parameters and no code behind it. The compositor ran no call on a screen
  node, which is what JsOutputs found.
- **`gnoblinctl output ...`.** It speaks to the model graph of `gnoblin-compositor`, which serves no `apply` on an
  output: `gnoblinctl output call <id> apply` is refused with "does not serve that method".
- **`defineConfig({ outputs })`.** Checked and stored, applied by nothing.
- **Backends.** Only the headless backend can change a live mode (`Headless::resize`). The DRM backend refuses
  `configure_output`: a mode or scale change on hardware is not implemented, and says so. Position, mirror and off need
  no modeset and work on any backend.

## Decision: one native call, `screen.apply`

The `config` document has no outputs section and a screen's properties are read-only, so the smallest surface is the
call the schema already named. `apply` now takes one `text` argument, `key=value` lines (`mode`, `scale`, `transform`,
`position`, `enabled`, `mirror_of`), and the compositor answers under the call's token. JS `Output.apply`,
`Output.mirror` and `defineConfig({ outputs })` all go through it; a config entry is applied to each output with that
connector, when the config loads and when the output is plugged in. Errors carry the key path
(`outputs.DP-1.mode: ...`).

The request is transactional (`crates/gnoblin-wayland/src/output_policy.rs`):

1. Parse, then lay the request over the output's stored profile.
2. Resolve the whole candidate layout for the modes and scales it would drive. An overlap, a gap, a mirror of a missing,
   off or mirrored output, or no output left on is refused by name and no hardware is touched.
3. Drive the changed outputs in order. If the backend refuses one, the earlier ones are put back (DW5): the last good
   arrangement was never replaced.
4. Only a fully driven candidate becomes the profile.

`transform` other than `normal` is refused: no renderer draws a rotation, and accepting one would report what the
pixels do not show.

## Mirror

A mirror is a copy of pixels, not a second layout position. The engine already makes a mirrored pair one logical
monitor, so the mirrored output is a _shadow_: it keeps its screen node (so a shell can change it back) but has no
rectangle in the scene, no `wl_output` and no reach for the pointer. Each frame, after the real outputs are drawn, the
source's readback (`Backend::capture`, pointer included) is fitted into the mirror's panel, keeping its aspect and
centred (letterbox), and drawn as one overlay: byte for byte when the modes are equal, nearest-neighbour scaled
otherwise. The mirror's screen node publishes its source's rectangle and `mirror_of`. An output that is off is the same
shadow with black.

## Span

The extended desktop is the span: one global coordinate space, each output a rectangle in it, a window straddling a
seam drawn once in two pieces, the pointer crossing as one motion. So `Display(mode: 'span')` is an alias of the default
multi-output layout and there is no span renderer. Proven in `crates/gnoblin-wayland/tests/output_modes.rs`: a window
across the seam of outputs at scale 1 and 1.5 is continuous at the seam (no gap, the right physical width on the 1.5
output) and the pointer crosses it, absolute and relative.

## Profiles

`gnoblin_output::ProfileStore` keeps an `OutputProfile` per monitor, keyed by connector and the EDID identity
(`Edid::identity`, `MAN:product:serial`; `None` for a virtual output, which matches on the connector). The store lays
its profiles over the default row, so an output with no profile still gets a place. A profile whose monitor is gone is
parked: it changes no layout, and the same monitor on the same port gets its mode, scale, position and mirror back when
it returns (`settle_outputs` drives a newly connected output to the stored mode). A different monitor on that port does
not inherit it. A stored arrangement that no longer resolves falls back to the row, with a warning, rather than leave
the session without a desktop. Profiles live in the session, not on disk: they survive a hotplug, not a restart, where
`defineConfig({ outputs })` applies them again.

## Not built, on purpose

The spec's `Display`, `Viewport` and `Camera` classes are not added. Per-output desks already give each output its own
workspace, `Output` is the object that carries a profile, and span is the default layout, so a `gnoblin.displays` would
be a thin class over `gnoblin.outputs` with no behaviour of its own. XR owns cameras (`gnoblin/xr`, the `camera` node);
a display-mode camera would be a second owner.

## Proven where

Real compositor, software renderer, virtual outputs (`output_modes.rs`): apply, rollback on an
injected modeset refusal, mirror pixels and letterbox, one monitor for a mirrored pair, off, span, profile restore.
Fake compositor (`crates/gnoblin-js/tests/outputs.rs`): the JS surface and its error paths. Not proven: a DRM modeset
on hardware (unimplemented), the GLES renderer's mirror, a rotation.
