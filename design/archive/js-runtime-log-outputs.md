# JS runtime change log: outputs, modes and displays

The finished work of outputs and desks, output modes and displays. Part of the [JS/TS runtime design](js-runtime.md).

## Outputs status

Lane JsOutputs. User page: `docs/js-outputs.md`.

Tests:

- `crates/gnoblin-js/tests/outputs.rs` (28 cases over the fake compositor; the Lua cases of `shell_test.lua` for moving,
  unplugging, reconnecting and dialogs with their inputs and expected values);
- `crates/gnoblin-wayland/tests/window_relations.rs` (the compositor publishes `parent_window` and `from_launcher`, with
  a real client);
- `crates/gnoblin/tests/js_shell.rs`,
  `a_window_on_a_second_virtual_output_follows_to_the_first_when_it_is_unplugged_and_parked_windows_return_with_the_next_output`
  (a real compositor and shell; the headless backend takes an output at run time through `OutputTap`, a new
  `SessionConfig::output_tap`).

**Native.**

`window.toml` gains two properties:

- `parent_window` (`object:window`, from `xdg_toplevel.set_parent` and xdg-foreign through `sync_window_role`;
  published once both nodes exist and cleared when the parent closes);
- `from_launcher` (true when the window redeems an `xdg_activation_v1` token a privileged or trusted connection
  minted).

There is no launch attribution for `gnoblin.spawn`: the spawner mints no token, so a spawned program is not marked.

`screen.toml` gains `output_scale` (`scale` is the `node` base's three-axis scale), and the compositor publishes `make`,
`model`, `mode` (`WxH@Hz`), `output_scale`, `transform`, `primary` and `enabled` on each screen node beside `name`, `x`,
`y`, `width` and `height` (`Composer::set_output_info`).

**Model.** `Output` wraps a `screen` node; `gnoblin.outputs` is a live `Collection<Output>` with `OutputListener`
(`onConnected`, `onDisconnected`, `onModeChanged`, `onScaleChanged`, `onGeometryChanged`). A `Desk` (module-private) is
what an output shows.

Workspaces stay the script's own global list, so one workspace is on one desk: each desk starts on the first workspace
no desk shows and `output.show(ws)` takes a workspace another output shows and gives that output the workspace this one
showed.

The Lua desk owned a copy of every workspace, so two outputs could show "workspace 1"; that is the one behavioural
difference, and a workspace set that names the same workspace for two outputs shows it on the last.

- **Areas.** A layout computes in the area of the output that shows its workspace, in global coordinates (an output at
  `x = 1000` has `x >= 1000`). With one output at the origin the numbers are the ones the shell wrote before. A tree with
  no screen has one output-less desk, so a workspace is on show and nothing is laid out, as before.
- **Bars.** `Bar` takes `output`; `barInsets(output)` counts the bars on that output; the fullscreen rule asks the output.
- **Pointer.** `gnoblin.show`, a new window and the stock keys act on the desk under the pointer (the main seat's `screen`,
  read through `host.pointerScreen`) and on the primary desk when the seat names none.
- **Unplug.** The desk's windows move to the workspace a remaining output shows and are refitted (`refitClient`:
  maximised fills, snapped re-snaps, floating shrinks and clamps). With no output left the desk parks: its workspace is
  not active, `syncVisibility` hides the windows, the focus is dropped; the next output adopts the desk, and a window
  takes the focus when nobody holds it.
- **`moveToWorkspace`, `moveToOutput`.** Lua's `Window:move_to`: refuse a fullscreen window, a dialog, a dead window and the
  workspace it is on, hand the focus to a window that stays, refit.
- **Dialogs.** A dialog joins its parent's workspace, is centred in it once, shifts by the parent's displacement
  whatever moved the parent, follows it across workspaces and outputs, and is not offered to a layout.
- **Reload.** The snapshot carries what each output showed (`JsWorkspaceSnapshot::outputs`, by output name).
- **Sets.** `SetsEnvironment.show(workspace, output)` is told every placement's output; the "which this shell does not show
  workspaces on" report is gone.

**Not done.** `Output.apply` and the `outputs` config key were not done in this lane; see "Output modes status". A snapped
window's remembered floating rectangle is not refitted on an output move. The menu's Move to workspace and the rule `workspace` action move a window
with `transferClient` (no refit). `Workspace:walk` and the removal cases are not part of this lane.

## Output modes status

Lane OutputMirror. Design and decisions: `design/js-outputs.md`. User page: `docs/shell-api/outputs.md`. Tests:
`crates/gnoblin-output/tests/profile.rs`, `crates/gnoblin-wayland/tests/output_modes.rs` (a real compositor and the
software renderer on virtual outputs) and `crates/gnoblin-js/tests/outputs.rs` (the JS surface over the fake compositor).

**Native.** `screen.apply(text)` now runs in the compositor (`crates/gnoblin-wayland/src/output_policy.rs`): the request
is resolved as a whole layout first, the changed outputs are then driven with rollback on a refusal, and only a fully
driven candidate becomes the output's profile. The compositor answers under the call's token.

`screen.toml` gains `connector`, `identity` and `mirror_of`; `transform` and `enabled` are read-only. `gnoblin-output`
gains `ProfileStore` (profiles keyed by connector and EDID identity, `Edid::identity`) and a `Display` for
`LayoutError`; the backend `Output` carries the EDID identity.

A mirror or an output that is off is a shadow: its screen node stays, the scene and the pointer do not see it, and its
frame is the source's readback fitted into its panel, or black.

**JS.** `Output.apply(options)`, `Output.mirror(source)`, `Output.connector`, `identity`, `mirrorOf`,
`OutputListener.onApplied`, and an output setting applied by connector (`outputRequest` in `data/js/gnoblin/display.ts`
checks a request and names the key path).

A mirrored or off output has no desk, so nothing is laid out on it; it gets one back when it is ordinary again.
(`Display`, `Viewport` and the cameras were not added then; see "Displays status" below, which reverses that.)

**Not done.** A DRM modeset (the hardware backend refuses `configure_output`), a transform other than `normal` (no
renderer draws a rotation), the GLES renderer's proof of a mirror, profiles that outlive a session, and
`wlr-output-management` or `gnoblinctl output call apply` reaching the new path (both keep their old, narrower behaviour).

## Displays status

Lane SpecOutputs. Design and decisions: `design/js-outputs.md` ("Displays, viewports and cameras"),
`design/js-spec-native.md` part 2.5. User page: `docs/shell-api/outputs.md` ("Hardware profiles", "Displays and
viewports", "Cameras").

Tests: `crates/gnoblin-js/tests/displays.rs` (fake link), `crates/gnoblin/tests/js_shell.rs` ("logical displays": a span
and a mirror over two real virtual outputs) and the conformance fences of `crates/gnoblin-js/tests/spec_conformance.rs`.

**Built.** `OutputProfile` and `gnoblin.outputs.profiles`; `Display`, `gnoblin.displays` (with `replace`) and
`input.activeDisplay`; `Viewport`; `Camera2D`; the spec's `position`, `size`, `near` and `far` on `Camera3D`. The
classes are `data/js/gnoblin/display.ts`; the reconciler is `display_manager.ts`; the host module wires collections,
desks (a desk may span others) and the output requests.

The requests are the existing `screen.apply` text; nothing changed in the compositor.

**Reported as not applied** (once per reload, as a diagnostic): a `Camera3D` on a viewport, a `Camera2D` that is not the
identity, `Viewport.clearColor`, `OutputProfile.vrr`, a relative profile position whose neighbour is not connected.

**Not done.** A workspace shown on two displays at once (the renderer cannot draw a workspace twice; fence 107 assigns it
and the second assignment swaps), a perspective or zoomed workspace, the span authority policy, profiles that outlive a
session, and undoing a mirror or profile a reload no longer declares.
