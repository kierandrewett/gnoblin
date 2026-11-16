# JS runtime change log: types and the stock library

The finished work of work packages P2 (types) and P4 (the stock library and its layouts), written as each lane landed.
Part of the [JS/TS runtime design](js-runtime.md).

## P2 status

`crates/gnoblin-js-types` renders `gnoblin.d.ts` from the schema catalog (`render_dts`) and writes it with a
`tsconfig.json` into the config directory (`ensure_user_files`: the declarations are refreshed on every call, the
`tsconfig.json` only when absent). The layout described here was replaced by design/js-types.md: the library sources are the types.

One interface per schema object; a signal `mode_changed` is `onModeChanged(handler)`. The hand-written stock library
(`Client`, `Collection<T>`, `Layout`, ...) is checked in as `static/gnoblin-stock.d.ts` and marked proposed.

`gnoblinctl check FILE` (a `.ts`, `.mts`, `.js` or `.mjs` file loads through the `gnoblin-js` loader in a throwaway
generation with a 1 s budget; any other file keeps the configuration check) and `gnoblinctl types init [DIR]` are done
(`crates/gnoblinctl/src/js.rs`).

`check --types` runs an external `$GNOBLIN_TSC`, `node_modules/.bin/tsc`, `tsc`, `tsc6` or `tsgo`, with the generated
declarations in a temporary directory, and says so on one line when none is installed.

The check has no host, so the global `gnoblin` and the `gnoblin` and `gnoblin/<module>` imports are bound to an inert
stub; wrong names from the shell are `--types`'s to find. `gnoblinctl reload` is not done: it needs the supervisor
process.

## P4 status

The stock library is TypeScript under `data/js/gnoblin/`, compiled into `gnoblin-js` (`stock_modules()` strips it once
and returns the embedded `gnoblin` module family: `gnoblin` re-exports `gnoblin/bindings`, `gnoblin/layout`,
`gnoblin/layouts/{floating,grid,master-stack,monocle}`; the modules import one another by those names).

It reaches the host only through `globalThis.gnoblin`.

Parity-tested against the Lua goldens (`crates/gnoblin-js/tests/stock_layouts.rs`, in a real generation with a fake
host):

- `MasterStackLayout` (the `Tiling` corpus: 0 to 3 and 200 nodes, gap, ratio, top orientation, resize);
- `GridLayout` (row and column fill, gap);
- `MonocleLayout` (focus picks the shown node);
- `FloatingLayout.place` for every case of `tests/shell/placement_cases.toml`.

Tiled rectangles are floored to whole pixels as in Lua. Defaults are the Lua ones (ratio 0.5, gap 0), not the 0.65 and 8
of the spec's illustrative constructors. Derived from the same algorithm, not in the Lua corpus: master count, right and
bottom orientations, fixed grid `columns`.

`LayoutContext` carries the workspace rectangle and `gap: { inner, outer }`; a tiling layout insets by `outer` and uses
`inner` unless its own `gap` is set, so a workspace's `gaps` reach every stock layout.

The stock default (see ["Zero-config baseline"](js-runtime.md#zero-config-baseline)) is:

- four floating workspaces (`1` to `4`);
- the keys of the Lua shell (`Super+1` to `Super+9`, `Super+Left` and `Super+Right`, the `Swipe3` slide; see "P4b
  status");
- the media keys of `data/default-config`.

`Binding` and `BindingCollection` are wired by the input module (see ["P3c status"](js-runtime-log-input.md#p3c-status)).

The `triggers` option of `FloatingLayout` registers its `move` and `resize` bindings, which go live when an input
context attaches the layout.

The swap and split-resize drag behaviours (`MasterStackLayout`, `GridLayout`) were written later (see ["Tiling bindings
status"](js-runtime-log-policy.md#tiling-bindings-status)).

The layouts and keys that were not ported here (`Dwindle`, `Strip`, `Super+space`, `Super+Left` and `Super+Right`, the
`Swipe3` slide) are in "P4b status".

## P4b status

The remaining stock layouts, workspace sets, the slide gesture and the launcher key (lane JsStockLayouts). Tests:
`crates/gnoblin-js/tests/stock_layouts2.rs` (pure, in a real generation with a fake host), `workspaces.rs` and `input.rs`
(the layout driver and the gesture over a fake compositor) and `crates/gnoblin/tests/js_shell.rs` (a real session).

- **Layouts** (`data/js/gnoblin/layouts/`).

    - `DwindleLayout`: the split tree is state the layout keeps between computes, so one instance serves one workspace.
        - A new node splits the cell of the node that had focus when it arrived, which is the focus the layout saw last,
          because the compositor has focused the arrival by the time the layout runs.
        - Every walk is a loop, because a tree of 200 windows that each split the last is 200 deep and the engine's call
          stack is short.
    - `KioskLayout` places as `MonocleLayout`, as the Lua class did.
    - `StripLayout` (`column`, `pin`, `scrollTo(offset)`, `focusLeft(workspace)`, `focusRight(workspace)`):
        - The layout keeps its own column order because a new node opens after the focused column.
        - The focused column's neighbour takes focus when it closes.
        - Rectangles are not floored and nodes are not marked tiled, as in Lua.
    - `Carousel3dLayout` with the pure `ring(count, focus, area, options)`.

    The Lua tests and goldens are ported case for case (`tiling_test.lua` Dwindle and Kiosk, `strip_test.lua`); the cases
    the Lua corpus lacks (a Dwindle gap, closing, focus insertion, a strip's `column`, area offset and a fractional width)
    are marked "derived" in the test file.

- **Carousel3d and depth.** The Lua class wrote only `x`, `y`, `width` and `height`.

    A `Placement` now also carries `visible`, `tiled`, `z`, `rotation` and `scale`, and the Carousel writes the ring's
    depth as `z` and its angle as `rotation` about the vertical axis (radians, `[0, angle, 0]`), which the node schema
    has.

    The projected rectangle is still written, so a renderer that projects depth itself would draw perspective twice: the
    sign of the rotation and that interplay were not seen on a renderer. The node schema has no `transform` object, so the
    flat properties are written.

    The ring's depth fade is not applied (opacity belongs to the rules, as in Lua).

- **The layout driver writes more.**

    `Client` gains `tiled`, `visible`, `opacity`, `z`, `rotation` and `scale` (the host table in `bindings/props.rs`, which
    now has defaults per property and a list kind).

    It also gained `animate(values, { duration, easing })`, a write that carries a duration and 33 samples of an easing,
    as the Lua `node:to` sent its curves (`host.animate`, in milliseconds; the tree's wire counts seconds, and the
    binding converts). `Client.set` replaced it (see "Time units and error codes status"); the wire write stays.

    The easing is the CSS easing `linear`, `ease`, `ease-in`, `ease-out` or `ease-in-out`.

    After each compute the driver writes `tiled` (a client the layout gave no placement is untiled), the depth properties
    (reset when a layout stops using them) and the layout's `visible`; the shown workspace's windows are drawn and the
    others hidden (`syncVisibility`), which is what the Lua shell got from hiding the workspace node.

    A change of the focused client now recomputes the active workspace, so `MonocleLayout`, `KioskLayout`, `StripLayout`
    and `Carousel3dLayout` follow focus.

- **`show` and slides.**

    `gnoblin.workspaces.show(ws, options)` and `gnoblin.show` take `{ duration, progress }`. With a duration:

    - the new workspace's windows start a screen's width away and animate to where the layout put them (`x`, `ease-out`);
    - the old workspace's windows leave the other way and stay drawn until the slide ends, then return to their rest place
      hidden (`gnoblin.timers.after(ms, fn)` runs at the first frame after the time).

    `progress` carries on a slide a gesture has already made. The stock keys slide for `SLIDE_DURATION` (250 ms, as
    `Desk:show`); `show(ws)` with no options switches at once. A window is moved by its own `x`, so a floating window's
    rest place is its `x` when the slide starts.

- **The slide gesture.**

    `WorkspaceSlideBinding` (`gnoblin/gestures`, a `GestureBinding` on `Swipe3`) is `Gestures.workspace_slide`:

    - windows follow the fingers (`travel` 400 units for a screen);
    - fingers moving right bring in the previous workspace and left the next;
    - lifting after `commit` (0.25) with the motion more horizontal than vertical finishes with `250 * (1 - progress)` ms
      left;
    - anything else slides back over 200 ms (`ease-out`) and hides the incoming workspace;
    - a change of direction resets;
    - a workspace switched under the fingers ends the slide.

    The stock default binds it with `input.bind("Swipe3", new WorkspaceSlideBinding())`. The Lua shell claimed the
    undirected `Swipe3`, so one binding covers both directions.

- **Workspace sets** (removed). The first slice ported `sets.lua` as `gnoblin/sets` with `gnoblin.workspaces.sets` and
  `activateSet`. The owner's spec has no sets ("each display chooses independently from the same global workspace
  registry", spec 9E), so the module, the property and the method were deleted with the declarative config layer
  (design/js-spec-native.md, 3.11).
- **Keys.**

    The stock default binds:

    - `Super+1` to `Super+9` (the workspace at that position, whatever its id, nothing when there is none, as
      `desk().space.children[i]`);
    - `Super+Left` and `Super+Right` (the shown workspace's `StripLayout` moves focus one column, other layouts do
      nothing);
    - `Swipe3`.

    `Super+space` opens the stock launcher entry (`openLauncher`); a script that wants another command binds the key
    itself.

- **Proved against the real compositor** (`crates/gnoblin/tests/js_shell.rs`): two real Wayland windows on a
  `DwindleLayout` workspace sit at the two halves of the screen, and on a `StripLayout` workspace in two columns, as
  the compositor's tree holds them.
- **Not ported.**

    - the in-shell launcher (`launcher.lua` is an `Entry` widget that spawns the typed text; only the configured-command
      key is done);
    - reduced motion (`seat.reduced_motion` makes `motion()` zero in Lua; the slides here always animate);
    - the four-finger and other gesture actions of `gestures.lua` (`Gestures.on_swipe`, `on_pinch` and the five actions:
      `workspace_next`, `workspace_previous`, `window_close`, `window_minimise`, `window_toggle_maximise`);
    - the `Desk` (bars, `Desk:remove_workspace`, `sync_fullscreen`);
    - per-output workspaces and sets for outputs other than the primary one;
    - the `Window:on_closed` focus hand-off for layouts other than `StripLayout`.

    A slide moves windows by `x`, so a window that is moved while a slide runs is moved back to where the slide recorded.
