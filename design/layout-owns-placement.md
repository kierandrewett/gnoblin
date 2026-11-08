# One layout script decides placement, order and z-order

Status: proposal (D33, revised after the user's comments on the Doop frame).
Nothing here is built.

## The problem

D17 says a layout is the geometry authority for its workspace. The code does
not follow that. `gnoblin-wm` keeps a second set of placement rules outside
the layout, and the layout API has an escape hatch that makes them reachable.
The result is edge cases on top of the one mechanism that should handle all of
it.

Evidence, from `crates/gnoblin-wm/src` and `data/lua`:

- `wm.rs` (about line 1219) places every new window with
  `placement::place(self.options.placement, ...)`. The mode comes from the
  config key `window_management.new_window_placement`. No layout is asked.
- The same function centres a dialog over its parent by a second rule.
- `stack.insert` (about line 1252) sets the initial z-order. `stack.raise` and
  `stack.lower` (lines 1451, 1669, 1683, 1692) change it on focus, click and
  auto-raise. No layout is asked.
- `arrange::params` lists tiled windows from `windows.values()`. That is a
  `BTreeMap` keyed by window id, so a layout always sees creation order and
  cannot choose a window's slot.
- A layout is told only about tiled windows (`Place`, `Move`, `Leave`).
- A layout can answer `Declined` (Lua `on_request` returning `nil`). The shipped
  `floating.lua` does exactly that for every request, and `LayoutKind::Floating`
  is documented as "the layout gives initial placement only". It gives none. A
  decline means "the window manager's own rules apply". Those rules are the edge
  case.

## The idea

There is no special case. One script handles all layout, including floating.

- The layout is asked about every window, tiled or not, for every change that
  affects placement: a window opens, moves, resizes, closes, is raised or
  lowered, changes workspace or output, or gains focus.
- It always answers with an arrangement: for each window its position (a
  rectangle or a 3D transform), its size, whether it is visible, its opacity,
  and the order of the list. The order of the list is the z-order. There is no
  "decline".
- `gnoblin-wm` validates the answer with `settle` and applies it. It decides
  nothing about placement, order or stacking within a layer band.

### What this removes

| Removed                                                        | Replaced by                                                                                       |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `Declined` and a Lua `on_request` that returns `nil`           | An answer, always                                                                                 |
| `placement.rs` rules in the window manager                     | The floating layout                                                                               |
| The config key `window_management.new_window_placement`        | An option of the floating layout (`placement = "free_space"`, `"center"` or `"cascade"`)          |
| The fixed rule that centres a dialog over its parent           | The layout, which receives the parent as input; the floating default centres it                   |
| Creation-order window list                                     | The order the layout last returned                                                                |
| `stack.insert`, `raise`, `lower` decided in the window manager | The layout's order; a click or auto-raise is a request, and the layout answers with the new order |
| A separate "managed" and "free" status for ordinary placement  | A window the layout does not move simply keeps its rectangle in its answer                        |

### The floating layout

Floating becomes a real layout. The native `FloatingEngine` (D17: native Rust,
equal to the shipped module, checked by a parity test) implements the contract
and holds the three modes now in `placement.rs`. A new window gets a position by
the option `placement`. A window the user moved keeps its rectangle. A raise
request moves the window to the top of the order. A tiling layout answers the
same requests differently: it ignores a raise, and it puts a new window after
the focused one or at the master.

### Config

The key `new_window_placement` is removed in one step, by the repository's
clean-cutover rule. The config hook that reports removed settings
(`crates/gnoblin-config/src/hooks.rs`) names the replacement. The default config
and the docs move to the layout option. An existing config that sets the old key
gets that one error naming the new option, not a silent change.

### What stays in the window manager

- **Layer bands.** A fullscreen window, a window marked above or below, and the
  other state-driven bands stay compositor-owned. A fullscreen window must cover
  the panels whatever a layout says. The layout orders windows inside a band.
- **Input policy.** Click-to-focus, focus follows pointer, auto-raise delay and
  `focus_new_windows` decide when a request is made and who takes focus. They do
  not decide the order. A layout may also set focus through a layout effect (the
  LayoutStrip lane is adding `LayoutEffect::Focus`).
- **Validation and safety.** `settle` refuses an invalid answer.

### Failure is not a decline

There is always exactly one active layout per workspace. That keeps one
authority.

If the active layout errors, times out or returns an invalid answer, the
workspace swaps its active layout for the native floating layout. Floating is a
registered layout with the same contract, so a window is still placed by a layout
and never by the window manager. `LayoutFailed` is reported with the reason. The
last good arrangement stays in force until the swap takes effect, so no window
moves. When the configured layout answers a valid arrangement again, it becomes
active again and re-arranges.

The window manager has no placement rule of its own, including during a failure.
The existing `layout/fallback.rs` (centre and cascade) is replaced by this swap, so
it is removed with `Declined`. A window never waits on a layout past the frame
deadline: it opens under the floating layout and is re-arranged when the
configured layout is ready.

Open: if the native floating layout itself fails, that is a compositor bug. It
runs no script, so the only faults are invalid parameters, and those are
refused before the call.

## What it needs

1. The request set grows: `Open { window, size, parent }`, `Raise`, `Lower`, and
   windows that are not tiled are no longer excluded from requests.
2. The reply type loses `Declined`. The Lua loader refuses `nil` from
   `on_request` with a named error, and the shipped layouts return an
   arrangement.
3. The wm keeps the layout's returned order and passes it in `Params`.
4. `placement.rs` moves behind the native floating engine. The Lua `floating`
   module and `FloatingEngine` keep their parity test.
5. The config schema drops the key. Docs and the default config follow.

## Risks

- One more question per new window. A layout runs on a worker with a deadline.
  If it is late, the workspace swaps to the floating layout and the window never waits.
- Raise and focus requests now reach a layout. Under a 3D layout, depth and
  z-order interact. Each layout decides, and the shipped ones document it.
- Every shipped Lua layout must answer every request kind. That is a mechanical
  change and covered by failure-mode tests first.

## Phases

1. Failure-mode tests first for the new contract: late answer, error, invalid
   order, unknown window in an order, missing parent, empty workspace.
2. Add the request kinds and the always-answer reply. Remove `Declined`. Update
   the shipped layouts and the Lua loader.
3. Route `Open` through the layout. Move `placement.rs` behind the floating
   engine and add the `placement` option.
4. Store and pass the layout's order. Route raise and lower through requests.
5. Remove `new_window_placement`, migrate the default config, update docs and the
   changelog.

## Needs the user

One confirmation. Layer bands (fullscreen above panels, above and below states)
stay compositor-owned, and the layout orders windows inside a band. This is the
safe choice because a layout bug must never hide a fullscreen window or a panel.
