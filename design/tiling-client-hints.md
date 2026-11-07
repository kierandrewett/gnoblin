# Tiling client hints

What a tiling compositor tells a client about its tile, so that client-side
decorations (CSD) adapt, and what Gnoblin sends today. Companion to the layout
side in `layers.md` and the WM notes; this file covers the protocol side only.

Status: research done. Implementation of `tiled_*` states, `configure_bounds`
and `suspended` is in progress in this lane. Transactions and clip-to-geometry
are proposed, not built.

Citations are `path:line` in this repository, or a primary source named in
[Sources](#sources). `[INFERENCE]` marks a claim not checked against code or a
primary source.

## Short answer

Hyprland, sway and niri all send the four `tiled_*` states to a tiled window.
Hyprland also sends `maximized` as a CSD hack. Gnoblin sends only `maximized`
and `fullscreen`, and sends no `configure_bounds`. Gnoblin also has no
`suspended` state and always answers decoration negotiation with client-side
decorations.

## What the compositors send

### xdg-shell tiled states

The four states exist since xdg-shell version 2. Their meaning: the edge is
tiled, so the client should not draw a shadow, corner radius or resize handle
that points at it ([xdg-shell.xml](https://wayland.app/protocols/xdg-shell)).

- **Hyprland** sets all four `TILED` states on a tiled window when the client
  binds version 2 or later ([XDGShell.cpp:181-184], fetched to `/tmp/tsrc/hypr-XDGShell.cpp`).
- **sway** calls `wlr_xdg_toplevel_set_tiled` with all four edges for a tiled
  view, and falls back to `set_maximized` when the client binds below version 2
  ([xdg_shell.c:195-210], `/tmp/tsrc/sway-xdg_shell.c`).
- **niri** computes `should_tile` as "decoration is server side, or the window
  prefers no CSD", and sets all four `TiledX` states from it
  ([utils/mod.rs:421-475], `/tmp/tsrc/niri-utils-mod.rs`).

So the tiled states are unconditional for tiled windows in all three. The
per-edge rule proposed below gives the same answer in a full-coverage layout:
every tile touches the work area or a neighbour on each side. It differs only
when an outer gap leaves a tile's edge bare.

### Per-edge versus all four

Edge-accurate tiled states matter when a tile is not at an edge of the work
area: a client drawn at a split should round or shadow only the free edges.
Nothing in the three compositors sends a partial set that we found; the choice
is [INFERENCE] that they all treat the layout as full coverage.

### configure_bounds

`configure_bounds` (since xdg-shell version 4) tells the client the largest
size it should pick, so it does not ask for a window bigger than the tile
([xdg-shell.xml](https://wayland.app/protocols/xdg-shell)).

- **niri** sets bounds to the output size on move
  ([layout/mod.rs:4901], trait method `set_bounds` at [layout/mod.rs:244]).
- **Hyprland** and **sway**: not checked in this pass [INFERENCE: they send it
  in some form, unverified].

### Hyprland's maximised hack

Hyprland sends `setMaximized(true)` on a window's first commit that has a
texture ([XDGShell.cpp:567], comment: "forces apps to not draw CSD"). It skips
this when `shouldIgnoreInitialMaximizeds` is set ([XDGShell.cpp:313]).

- **Why:** some clients ignore `tiled_*` and keep drawing CSD shadows and
  corners. A `maximized` state makes them drop the decorations.
- **Which clients:** GTK4 and libadwaita honour `tiled_*`. Some Qt and Electron
  builds do not [INFERENCE: from common reports, not checked here].
- **Downside:** the client believes it is maximised. Its maximise button then
  asks to unmaximise, which Hyprland treats as a request to leave the tile, so
  the button toggles the wrong way. A window rule that targets known clients
  limits the damage.

### Decoration mode and prefer-no-csd

- **niri** `prefer-no-csd`: the window rule asks for server-side decorations.
  `should_tile` reads the result of that negotiation ([utils/mod.rs:421-475]).
  The compositor then draws the border itself.
- **Gnoblin** answers every `xdg_decoration` request with client-side mode
  ([zxdg_decoration_v1.rs:22-45]). `Placement.decorated` exists in the WM
  ([placement.rs:123-124]), but the decoration mode is not negotiated from it.

### Atomic resize

- **niri** collects every window of a layout change into one transaction. The
  screen shows the old layout until all tiles have acked and committed, so no
  half-resized frame appears. [INFERENCE: from niri's `tile.rs` and `layout/`
  transaction names; not traced end to end.]
- **sway** calls `wlr_xdg_surface_schedule_configure` and waits for the ack
  before it moves the scene node.
- **Gnoblin** sends each configure as the placement arrives, one window at a
  time (see the audit below). A layout change can show mixed sizes for a frame.

### suspended and clip-to-geometry

- `suspended` (since version 6) says the window is not visible, so the client
  may stop frame callbacks and animations ([xdg-shell.xml](https://wayland.app/protocols/xdg-shell)).
- **niri** `clip-to-geometry` clips the surface to the xdg window geometry and
  rounds its corners in the compositor. CSD shadows and corners then disappear
  without client cooperation ([niri wiki: window rules]).

## Gnoblin audit

| Feature                         | Hyprland / sway / niri           | Gnoblin today                                                              | Gap | Proposed                                                                     | Cost        |
| ------------------------------- | -------------------------------- | -------------------------------------------------------------------------- | --- | ---------------------------------------------------------------------------- | ----------- |
| tiled_* states                  | all four, tiled windows          | none; `state.rs:1228-1240` sends maximised and fullscreen only             | yes | edge rule in `gnoblin-wm`, sent with `PlacementTarget` (planned)             | S           |
| configure_bounds                | niri sets output size            | `state.rs:1237` sets `bounds = None`                                       | yes | tile size for placed, work area for floating (planned)                       | S           |
| suspended                       | [not checked]                    | none; `set_visible` `state.rs:1260` only does output enter and leave       | yes | send `suspended` when a window is hidden (planned)                           | S           |
| maximised CSD hack              | Hyprland only, first commit      | none                                                                       | yes | needs window rules; no `WindowRule` type exists (`grep` in `gnoblin-config`) | M, deferred |
| decoration negotiation          | niri `prefer-no-csd`             | always client side, `zxdg_decoration_v1.rs:22-45`                          | yes | server side for tiled windows; compositor borders                            | M           |
| atomic resize                   | niri transactions, sway ack wait | sends per window, `wm.rs:1747-1755` retains configures then `apply_layout` | yes | transaction per layout run                                                   | L           |
| clip-to-geometry, corner radius | niri window rule                 | none                                                                       | yes | clip and round in the renderer                                               | L           |

Per-row notes:

- `state.rs:1878-1906` (`place_tiled`) reads the window state and calls
  `configure` once per placement.
- `wm.rs:1772-1790` (`apply_layout`) emits `PlacementTarget` for each placement
  and frees windows the layout took out of tiling.
- Visibility changes come from `Effect::SetVisible`, emitted on workspace
  switch and minimise (`wm.rs:1210`, `:1234`, `:1397`).

## Proposed next

These are larger than the first pass. Not built here.

- **Atomic transactions.** Collect the configures of one layout run, send them
  together, and commit the scene only after every tile acks, with a timeout
  that releases the transaction. Cost: L. It touches `wm.rs` apply queue,
  `state.rs` configure and the scene commit, and needs a timeout test.
- **Clip to geometry.** Read the xdg window geometry (`set_window_geometry`) and
  clip the surface to it in the renderer, with a rounded corner mask per
  placement. Cost: L. It needs a shader or stencil path in the renderer and a
  per-window rule to switch it on.
- **csd-squash rule.** `tiled` (default), `maximised` for known clients, or
  `none`. Cost: M, blocked on window rules. Today `gnoblin-config` has a
  `window-rules` callback doc but no `WindowRule` type, so the rule cannot be
  matched yet.

## Sources

- xdg-shell protocol: https://wayland.app/protocols/xdg-shell (copy at `/tmp/tsrc/xdg-shell.xml`).
- xdg-decoration protocol: https://wayland.app/protocols/xdg-decoration-unstable-v1 (`/tmp/tsrc/xdg-decoration.xml`).
- Hyprland `XDGShell.cpp` (`/tmp/tsrc/hypr-XDGShell.cpp`). Upstream path
  `src/protocols/XDGShell.cpp` [INFERENCE: path not confirmed against the tree].
- sway `xdg_shell.c` (`/tmp/tsrc/sway-xdg_shell.c`), upstream `sway/desktop/xdg_shell.c`.
- niri `utils/mod.rs`, `layout/mod.rs`, `tile.rs` (`/tmp/tsrc/niri-*.rs`), upstream `src/window/mod.rs` and `src/layout/` [INFERENCE: file names as saved].
- niri wiki, window rules: `clip-to-geometry`, `prefer-no-csd` (not fetched in this pass).

[XDGShell.cpp:181-184]: /tmp/tsrc/hypr-XDGShell.cpp
[XDGShell.cpp:313]: /tmp/tsrc/hypr-XDGShell.cpp
[XDGShell.cpp:567]: /tmp/tsrc/hypr-XDGShell.cpp
[xdg_shell.c:195-210]: /tmp/tsrc/sway-xdg_shell.c
[utils/mod.rs:421-475]: /tmp/tsrc/niri-utils-mod.rs
[layout/mod.rs:244]: /tmp/tsrc/niri-layout-mod.rs
[layout/mod.rs:4901]: /tmp/tsrc/niri-layout-mod.rs
[niri wiki: window rules]: https://yalter.github.io/niri/Configuration:-Window-Rules
