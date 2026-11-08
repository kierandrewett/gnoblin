# The layout owns placement

Status: proposal (D33). Nothing here is built. It answers one question: why is
`new_window_placement` not handled by the layout, and what should own what.

## The problem

D17 says a layout is the geometry authority for its workspace. D25 says the
layout kind decides placement, and `LayoutKind::Floating` is documented as "The
layout gives initial placement only" (`crates/gnoblin-wm/src/layout/kind.rs`).
The code does not do that. For a new window the layout is never asked where it
opens.

What happens today, in order, in `crates/gnoblin-wm/src/wm.rs`:

1. **Geometry.** `Wm` computes the initial rectangle itself with
   `placement::place(self.options.placement, ...)` (about line 1219). The mode
   comes from the config key `window_management.new_window_placement` and is one
   of `free_space`, `center` or `cascade`. The workspace's layout is not
   consulted. This runs for every normal window, including windows that a tiling
   layout will move a moment later.
2. **Transients.** A dialog is centred over its parent in the same function, by
   a second, separate rule.
3. **Z-order.** `self.stack.insert(window_id, layer, anchor)` (about line 1252)
   decides where the window enters the stack. Later changes are `stack.raise` and
   `stack.lower` from focus, click and auto-raise (lines 1451, 1669, 1683, 1692).
   No layout is asked.
4. **Order in a tiled set.** `arrange::params` builds the window list from
   `windows.values()`, a map keyed by window id. A tiling layout therefore always
   sees windows in creation order. It cannot say where a new window goes in the
   order, such as "after the focused window" or "at the master".
5. **Layout requests.** A layout only hears about a window through `Place`,
   `Move` and `Leave` (`RequestKind`), sent by `request_kinds`. They cover tiled
   windows only. A window under a floating layout never produces a request.
6. **Floating.** D17 makes `floating` the layout that declines every request, so
   "the wm's existing floating logic applies". That logic is items 1 to 3.

So the placement rules for a floating desktop live in `gnoblin-wm` as config
options, and the layout that is called "floating" owns none of them. A user who
swaps `floating` for another layout keeps `new_window_placement` in config, where
it means nothing for tiled windows and something for the rest. The shipped Lua
layouts (`master_stack.lua` and others) decline every request.

## What should own what

| Thing                                                     | Owner today                             | Owner proposed                                                                             |
| --------------------------------------------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------ |
| Where a new window opens (rectangle or 3D transform)      | `gnoblin-wm`, from config               | The workspace layout                                                                       |
| Where a dialog opens relative to its parent               | `gnoblin-wm`, fixed rule                | The workspace layout, with the parent as an input; default is centred over the parent      |
| Where a new window enters the tiled order                 | Creation order, fixed                   | The workspace layout                                                                       |
| Initial z-order of a new window                           | `stack.insert`, fixed                   | The workspace layout, as a hint the wm validates                                           |
| `new_window_placement`                                    | wm config key                           | A parameter of the floating layout                                                         |
| Raising and lowering after use (click, focus, auto-raise) | `gnoblin-wm` options                    | Stays in `gnoblin-wm`. These are input policy. A layout may refuse a raise (kiosk, tiling) |
| Who gets focus when a window opens                        | `gnoblin-wm` option `focus_new_windows` | Stays. It is focus policy, not layout                                                      |
| Edge tiling and snap                                      | `gnoblin-wm`                            | Stays for now. Snap is a user gesture, and a layout already hears `Move` and `Leave`       |

## Proposal

Scope: this covers initial placement and initial order only. Ongoing restacking
(raise on click, auto-raise, lower) stays in `gnoblin-wm` as input policy unless
question 3 below is answered yes. The user asked that order and z-order follow the
layout, so question 3 decides whether that part is in scope.

1. **One new request: `Open`.** The wm sends `Open { window, size, parent }` to
   the workspace layout for every new toplevel that is not covered by another
   request. The reply is one `Placement` for that window (a rectangle or a full
   3D transform), plus an optional stack hint.
    - For a tiling layout, `Open` replaces the current `Place` for new windows. It
      lets the layout choose the window's slot in the order.
    - For a floating layout, `Open` returns the initial rectangle. It holds the
      three modes now in `placement.rs`.
    - A layout may decline. A declined `Open` falls back to the built-in rule that
      exists today (centred, cascaded), so nothing regresses.
2. **Floating owns the three modes.** The native `FloatingEngine` and the shipped
   `gnoblin.layouts.floating` module accept a `placement` parameter with the
   values `free_space`, `center` and `cascade`. The config key
   `window_management.new_window_placement` stays as the default value of that
   parameter, and a per-workspace layout parameter overrides it. The Rust code in
   `placement.rs` moves with it and stays pure and bounded.
3. **Order.** The tiled window list is ordered by an explicit position the layout
   chose in `Open`, not by window id. The wm stores that order and passes it in
   `Params`.
4. **Z-order hint.** The `Open` reply may carry `stack: top | bottom | above(window)`.
   The wm applies it through the existing validated stack functions. A reply that
   names an unknown window is refused with a named error.
5. **Transients.** The parent is an input to `Open`. The default reply, when the
   layout declines, is the current centring.
6. **Docs.** `docs/config/configure/window_management.md` describes
   `new_window_placement` as "the default placement mode of the floating layout".
   The layout guide documents `Open`, the stack hint and the parameter.

## Why not leave it in the wm

- A layout cannot give a consistent experience if the wm places windows by a
  different rule first. Under a tiling layout, every new window is placed once
  by `placement::place` and then moved by the layout. The first step is wasted
  work and, for `free_space`, a scan over the grid.
- `new_window_placement = "center"` is meaningless under `master_stack`. A single
  parameter on the layout that uses it is clearer.
- A Lua layout author cannot implement "open next to the focused window" or "new
  window becomes master" today.

## Risks

- The wm sends one more request for every new window. Each request waits for a
  Lua worker, so the first frame of a new window could be late. Mitigation: the
  layout runs on the existing deadline, and a late answer falls back to the
  built-in rule. The window must never wait on a layout past the deadline.
- Existing user configs set `new_window_placement`. The key stays and keeps its
  meaning for the floating layout, so nothing breaks.
- The shipped Lua layouts decline everything today. They need a default `Open`
  reply, or the floating fallback applies.
- Parity tests under `gnoblin-wm` for placement (free space, centre, cascade)
  must pass unchanged against the new path.

## Phases

1. Add `RequestKind::Open` and the `Open` reply in `gnoblin-wm::layout`, with
   failure-mode tests first: declined, late, invalid placement, unknown stack
   hint, parent missing.
2. Route new windows through it, with the built-in rule as the fallback. Move
   `placement.rs` behind the native floating engine and give the shipped floating
   module the same behaviour.
3. Store the layout's order and pass it in `Params`.
4. Add the stack hint.
5. Update the shipped tiling layouts to answer `Open` (insert after the focused
   window, new master, and so on) and update docs and changelog.

## Needs Kieran

1. Is `new_window_placement` a parameter of the floating layout (proposed), or
   should it be removed in favour of per-workspace layout parameters only?
2. Should a dialog's placement relative to its parent be a layout decision (as
   proposed), or stay a fixed compositor rule so dialogs never surprise anyone?
3. Should raise and lower on click and auto-raise stay as compositor input policy
   (as proposed), or become layout decisions too?
