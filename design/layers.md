# Layers

Companion to D28 and D29 in [rust-compositor.md](rust-compositor.md). It
defines the compositor's paint bands, how content claims a band, and the
`window_menu` band that the window menu uses. Decisions live in the main
document; this file holds the detail.

Status: proposed. Nothing here is implemented in the Rust tree yet.

## Today

- Stacking order for windows is `StackLayer::Normal` and `Fullscreen` in
  `gnoblin-wm/src/stack.rs`. Layers never interleave.
- wlr-layer-shell surfaces sit in `Background`, `Bottom`, `Top` and `Overlay`.
  Only `Top` and `Overlay` may take exclusive keyboard focus.
- `OutputFrame` in `gnoblin-backend/src/render/frame.rs` has a `layers` list,
  one `overlay` slot used by the portal and modal path, and one `cursor` slot.
- The `overlay` slot holds one image. Two compositor-drawn things cannot
  coexist on it.

## Bands

A band is a fixed position in paint order. The set is closed and defined by
the compositor, not by configuration. Bands from the bottom:

| Band          | Holds                                                        |
| ------------- | ------------------------------------------------------------ |
| `background`  | Desktop colour and wlr background surfaces.                  |
| `bottom`      | wlr bottom surfaces, such as desktop widgets.                |
| `windows`     | Normal windows, then fullscreen windows (`StackLayer`).      |
| `top`         | wlr top surfaces, such as bars and docks.                    |
| `window_menu` | One window menu, anchored to a window or a point.            |
| `overlay`     | wlr overlay surfaces, search, OSD and notifications.         |
| `modal`       | Per-output modal overlay; blocks input (`wayland/modal.rs`). |
| `cursor`      | The pointer image, above everything.                         |

The `top` band sits above fullscreen windows, as bars do. The `window_menu`
band sits above `top` so a menu opened from a bar is not hidden by it.

[INFERENCE] The position of `top` against fullscreen windows matches wlr
convention. Confirm against the reference session before the parity gate.

## Claims

A layer is a claim on one band. A claim has:

- a band, one of the closed set above;
- content: a layer-shell surface, or an image painted by a compositor
  provider (the built-in egui painter, or a shell surface);
- an owner: the client or provider that made the claim, with its grant
  (D22);
- for `window_menu` only, a target: a window id, or a point in output
  coordinates, and a preferred edge.

Rules:

- Within one band, the most recent claim is on top. Claims in `windows`,
  `background` and `cursor` are not made by clients; the compositor owns them.
- `window_menu` and `modal` allow one live claim per output. A second claim
  replaces the first only when the owner is the same; otherwise it is refused.
- A claim ends when its owner disconnects, its target goes, or its output is
  removed. The end is reported as an event, never by silent removal.
- Captures leave out `modal`. They include `window_menu` only when the frame
  says so. The default is to leave it out, matching the portal dialog.

## Frame contract

`OutputFrame` replaces the single `overlay` slot with one list of band
content, in band order. The modal keeps its own entry. The rule that the
capture path decides per entry stays. This is a breaking change to the frame
type, and the Pixman path and the GLES path read it through one type.

## Object view

Each claim is a `layer` object (in the object model), with a read-only `band`,
`owner`, `target` and `state` property. The existing `gnoblin.layers.list`
keeps reporting wlr surfaces. Band claims that are not wlr surfaces appear
under the same object, so a shell reads one list.

[Needs Kieran: whether `layers.list` gains a `band` filter in the same
change, or only the object model adds it.]

## Window menu band

The `window_menu` band is described in D29. In short: the compositor opens a
claim on this band for a window; the claim's content comes from the chosen
provider; the compositor places it and dismisses it. The band itself knows
nothing about menus. It only holds one targeted claim per output.

## Tests

Each rule above names one test that fails without it:

- band order: a `top` surface stays below a `window_menu` claim, and a
  `window_menu` claim stays below `overlay`;
- one live `window_menu` claim per output; a second owner is refused;
- a claim ends when its owner disconnects, and an event is sent;
- a capture omits `modal` and omits `window_menu` by default;
- an output removal ends every claim on that output.

## Open questions

- Whether a `window_menu` claim may be placed on an output the target window
  is not on. Proposal: no; the claim moves to the window's output or ends.
- Whether `bottom` surfaces may take keyboard focus. Today they may not, which
  matches the protocol.
