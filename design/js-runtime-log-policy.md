# JS runtime change log: focus, client requests and window policy

The finished work of focus, window rules and snapping, the answers to client requests, window policy and tiling
bindings. Part of the [JS/TS runtime design](js-runtime.md).

## Focus, rules and snapping status

Lane JsStockFocus ported:

- `data/js/gnoblin/focus.ts` and `snap.ts`, from `data/lua/gnoblin/shell/focus.lua` and `snap.lua` and the `Window:on_*`,
  `snap` and `unsnap` wiring of `init.lua`;
- `rules.ts`, from `rules.lua` (a data-only window rules engine, removed with the declarative config layer;
  design/js-spec-native.md, 3.11, says what a replacement needs).

The tests are `crates/gnoblin-js/tests/stock_focus.rs` (the cases of
`crates/gnoblin-lua/tests/lua/{focus,snap}_test.lua`, over the fake link) and one real-compositor test in
`crates/gnoblin/tests/js_shell.rs`. The user page is `docs/config/configure/window_management.md`.

- **Settings.** The compositor publishes `edge_tiling`, `drag_constraint` and the six focus keys on the seat, read-only.
  `JsShell` reads them through `__gnoblinHost.input.settings(seat)` (`crates/gnoblin-js/src/input.rs`, the schema's
  defaults until the compositor says otherwise) and `InputContext.settings` hands them out.

    The shell uses them as published: there is no script override (the `focus`, `edgeTiling` and `dragConstraint` keys
    were dropped, design/js-spec-native.md 3.11).

    The focus modes are `click`, `sloppy` (the seat's `hover`) and `follow-mouse` (`hover-strict`: acts on entry only and
    clears focus when the pointer leaves the focused window).

- **Pure core.**

    `focusSettings`, `opensFocused`, `hoverFocus`, `restFocus`, `leaveClears` are the Lua functions;
    `focusDecision(settings, event)` composes them into one decision per event (`added`, `enter`, `motion`, `leave`,
    `press`), a plain object: focus, clear, raise, and the delays of a rest focus and an auto raise.

    `edgeZone`, `tileRect` and `edgeTilingEnabled` are `snap.lua`'s, with rectangles written `x, y, width, height`.

- **Pointer edges on windows.** The compositor sends the shell `enter`, `motion`, `leave`, `press` and `release` events
  addressed to a window's node (a press on a window is an edge there while a shell holds the tree, so the shell decides
  focus and raise).

    `JsShell::frame` parks them (`JsInput::accept_window_event`) and, after the frame's `plan`, calls
    `entry.pointer(events)`, which queues the clients' `onPointerEnter`, `onPointerMotion`, `onPointerLeave`,
    `onPointerPress` and `onPointerRelease` with `{ x, y, button }` in the window's own coordinates.

    A client listener hears one client's edges, a collection listener every member's. Scroll and key events still have no
    JavaScript API.

- **Wiring.** The runtime attaches a `FocusPolicy` on `gnoblin.clients` for every config. The policy keeps two timers (a
  pointer rest, an auto raise) on `InputManager`'s per-frame hook, with the clock of `__gnoblinHost.input.now`. A new
  window takes focus (`client.focus()`), except a window the shell found at boot: a reload does not take the user's
  focus.

    `client.focus()` is a write of the first seat's `focus` (the Lua shell's `seat.focus = window`), because the
    compositor answers no `focus` call on a window (`run_window_calls` handles `close` and `kill`);

    the call it was before did nothing against the real compositor. The policy reads focus from the seat as well
    (`InputContext.focusedClient`), because an observer connection saw no window's own `focused` set while a shell held
    the tree. The Lua shell never set `from_launcher`, so `focus_new_windows` has no launcher origin to read yet and the
    policy passes `false`.

- **Raise.** `Client.raise()` calls `__gnoblinHost.raise`: the tree's order is the stacking order, but a window the
  compositor made has no parent and sorts after the root's tree, so the call first gives every unparented window the
  root for its parent, in the order it already has, and then moves the window last.

    A window already on top queues nothing. Unit-tested on a mirror (`bindings/host.rs`), and proved against the real
    compositor (`a_click_focuses_and_raises_a_real_window_so_the_compositor_picks_it_over_the_one_above`): two overlapping
    windows, a click on the lower one where only it shows, then a click where both show, which the compositor gives to the
    lower window because it was raised.

- **Rules (removed).** `rules.ts` was a data-only engine: queries over app id, title, focus and workspace, and actions on
  placement, style and window state. It was deleted with the declarative config layer. A window rule is now a listener
  plus `gnoblin.clients.where(...).style`. The actions with no writable spec property have no replacement yet
  (design/js-spec-native.md, 3.11).
- **Still on `Client`.** `fullscreen` (writable), `opacity`, `secure`, `floating`, `raise()` and
  `ClientListener.onAppIdChanged`. `opacity` reads 1 and not 0 while the compositor has published none.
- **Snapping.** `ClientMoveBinding({ snap })` tiles a window dropped within `snap` pixels (default 4, `false` never) of
  a screen edge when the seat's `edge_tiling` is on, as `drag` in `init.lua` did.

    `snapClient`, `unsnapClient`, `snappedZone` and `forgetSnap` keep the floating rectangle off the client; a maximised
    window floats first; unsnapping restores the rectangle the window had when it snapped (Lua's `keep_floating`), and a
    drag that takes hold of a snapped window ends the snapped state.

    `ClientResizeBinding({ edges: "nearest" })` moves the one side the press is nearest to; `quadrant` still moves a
    corner.

- **Not done.** `focus_new_windows` launcher origin (no source), a rule's `role`, `xr`, drawing `cornerRadius` and
  `shadows`, the Lua `refit` of a snapped window when the work area changes. Client requests are in "Client requests
  status".
  A press on a titlebar canvas does not focus or raise its window: a canvas's pointer events do not reach the client's
  listeners, so `Titlebar` would have to ask for it.

## Client requests status

Lane JsClientRequests, cutover step 1: `data/js/gnoblin/window_state.ts` answers the requests the compositor sends a
shell and changes nothing for until it is answered (`crates/gnoblin-schema/schema/tree/messages.toml`, the `request`
event of a node: `maximize unmaximize fullscreen unfullscreen minimize unminimize close raise lower activate move resize
menu`).

The policy is `Window:on_request` and its helpers in `data/lua/gnoblin/shell/init.lua` (`present`, `raise`, `lower`,
`maximise`, `enter_fullscreen`, `minimise` and their inverses), ported with the same inputs and expected values.

The tests are:

- `crates/gnoblin-js/tests/client_requests.rs` (the 12 cases of `states_test.lua`, the `shell_test.lua` cases for raising
  with a fullscreen sibling, activation and a repeat fullscreen, and the new cases below, over the fake link);
- three tests in `crates/gnoblin/tests/js_shell.rs` that run a real Wayland client against the real compositor
  (`xdg_toplevel.set_fullscreen`, `set_minimized` and an `xdg_activation_v1` token earned by a click).

- **Delivery.** A `request` event is parked by `JsUiState` whatever its node and reaches `onUiRequest` handlers in the
  toolkit's frame. `windowState.start` registers its handler when `gnoblin.ts` loads, so it runs before the window menu's
  (which also hears `menu`) and a script's answer can stop the menu. Every script has the policy: a request with no
  answer is a window that cannot be maximised.
- **The hook.** `ClientListener.onRequest(client, request)`: the listeners attached to the client first, then each
  collection that holds it, each in the order attached, called synchronously; the first to call `request.handle()` wins,
  later listeners are not called and the built-in answer does not run. A throw is reported (`Name.onRequest`) and the
  next listener is asked.

    `request` is a `ClientRequest` (`kind`, `client`, `x`, `y` for a menu anchor, `handled`, `handle()`). A listener that
    handles a request may call `windowState` (`maximise`, `unmaximise`, `enterFullscreen`, `leaveFullscreen`, `minimise`,
    `unminimise`, `present`, `raise`, `lower`, `beginMove`, `beginResize`), each returning whether it ran.

    The rule is written in the header of `window_state.ts` and the page `docs/typescript-shell.md`.

- **Fullscreen covers the whole output.** The Lua `cover_rect` is the whole `Screen` under the bars (`{ x = 0, y =
-d.space.y, w = d.width, h = d.height }` in the workspace's own space), and `Desk:sync_fullscreen` hides the bars and
  the workspace clip while a fullscreen window shows. The JavaScript model had the screen less the bars (`screenArea`).

    The decision is Lua's: the rectangle is `(0, 0, output width, output height)` in screen coordinates (which is the Lua
    `y = -32` with a 32 pixel bar), and `Bar.shown` is false while a fullscreen window shows on the workspace on show
    (`barsGiveWayTo`, `data/js/gnoblin/widgets.ts`), so it is hidden and takes no room.

    The bars return when the window leaves fullscreen, minimises, closes or its workspace is hidden. There is no workspace
    clip in JavaScript: a window is placed in screen coordinates, so the Lua `space.clip` check has no twin. `maximize`
    still fills `screenArea` (what the bars leave).

- **Rectangles.** `maximise` keeps the floating rectangle, `enterFullscreen` the windowed one (a maximised or snapped
  window keeps its flags, and leaving fullscreen returns the maximised rectangle), as `keep_floating` and
  `data.windowed` did.

    A repeat `fullscreen` is refused so it cannot overwrite the kept rectangle, and a `maximize` of a fullscreen window is
    refused. The rule engine and the menu and titlebar maximise/minimise buttons call the same methods, so a window a rule
    made fullscreen is covered the same way (`cover` and `beforeCover` are gone).

    A tiling layout leaves a fullscreen or maximised window alone (`layout()` filters them from `compute`, and a change of
    the covering set makes the workspace compute again).

- **Raise.** `Client.raise()` keeps a fullscreen sibling on the same workspace on top of the window it raised, as
  `Window:raise` did; a pinned window (`windowMenu.isAbove`) is still kept by the menu's per-frame `#keepPinned`, which
  already put a fullscreen window over the pinned ones.
- **Minimise and activate.** `minimise` writes `minimised`; the compositor hides the window and a layout does not show
  it again (`writtenVisible`). When the window held the keyboard focus the seat's focus goes to the topmost sibling on
  its workspace that is not minimised, or to nobody. `unminimise` focuses the window only when nothing holds the focus.

    `present` (the `activate` request, the compositor's own authority having allowed it) does nothing for a window on a
    workspace that is not shown; else it unminimises, focuses and raises.

- **Move and resize.**

    The request names no pointer and no edge, so `beginMove` and `beginResize` read the seat's pointer and start the stock
    `ClientMoveBinding` or `ClientResizeBinding`.

    They go through the binding's own `onStart`, `onUpdate` and `onEnd` (`constrainRect`, the snap on a drop, the minimum
    size).

    The drag follows the pointer in a frame hook and ends on the next pointer press or release on a window (the compositor
    sends the release to the client that holds the implicit grab), when its window goes, or after `REQUEST_DRAG_LIMIT_MS`
    (30 s) so none follows the pointer for ever.

    A window that a layout tiles (`Client.tiled`), a fullscreen one and a maximised one are not dragged. A pointer within
    `RESIZE_CORNER_ZONE` (16) pixels of two sides resizes that corner, anywhere else the nearest side.

    This is proved over the fake link only: a real `xdg_toplevel.move` needs a client that sends the request with a
    pointer serial, which the test client does not.

- **Rules and the stock workspaces** moved in the same lane. Rule lists merge every matching rule (see "Focus, rules and
  snapping status"); the zero-config baseline is four workspaces, `1` to `4`, named `Workspace 1` to `Workspace 4`, and
  the starter is written to match.
- **Not done.** The Lua `refit` of a maximised or snapped window when the bars change, a request for a window on
  another output (the shell has one primary screen), and the kept rectangles across a reload: they live in the
  generation, so a window that is fullscreen or maximised when the config reloads leaves it where it is.

## Window policy status

Lane JsWindowPolicy (`docs/js-window-policy.md`).

Done, each with the tests named in `design/js-parity.md`:

- keyboard Move and Resize (`data/js/gnoblin/reposition.ts`, one grab node that holds the keyboard; real-compositor test
  with injected keys);
- gesture actions (`gesture_actions.ts`, `gestureActions`, `gnoblin.gestures`);
- reduced motion (`gnoblin.motion`, read from the seat's `reduced_motion`);
- binding-only devices (`claim_device`) and the `false` action;
- the focus hand-off for every layout, `Workspace.walk`;
- workspace removal to the neighbour before it with the last refused;
- titlebar presses reaching the focus policy;
- a refit of the windows when a bar changes the work area;
- kept rectangles across a reload (carried in the workspace snapshot, `JsKeptWindow`);
- workspace sets (removed later with the spec's lack of sets).

- **Decisions.** A gesture bound at the stock slide's trigger replaces it. The hand-off is skipped for a layout that
  defines `onClientRemoved` (a strip's columns are not in window order).
  `workspaces.remove` returns a boolean and refuses the last workspace; `clear()` still empties.
- **Not done.** Nothing of the five lane targets; the real-compositor proof covers only the keyboard move and resize, the rest
  runs over the fake link.

## Tiling bindings status

Lane JsTilingBindings (`docs/shell-api/input-handlers.md`, "Tiling drags"). Done: `ClientSwapBinding` (id `swap`,
`Super+MouseLeft+Drag`) and `SplitResizeBinding` (id `resize`, `Super+MouseRight+Drag`) in
`data/js/gnoblin/bindings/tiling.ts`, registered by `MasterStackLayout` and `DwindleLayout` (both) and `GridLayout`
(swap only) with the `triggers` option `FloatingLayout` has (`null` leaves one out, `layout.bindings.remove("swap")`
removes it). `MonocleLayout` has none.

Tests: `crates/gnoblin-js/tests/tiling_bindings.rs` over the fake link, and the real-compositor test in `js_shell.rs`.

- **Order.** `workspace.clients.swap(a, b)` (new on `MutableCollection`) trades two members' places and marks the workspace
  dirty; the owner spec's `layout.order.swap` is this, since the order lives on the workspace's collection and a layout has
  no handle on its workspace. A layout that keeps its own record of who sits where hears it as `onClientsSwapped(a, b)`
  (`DwindleLayout` trades two leaves of its tree; a tree is placed by its shape, not by the order).
- **Swap.** The drop target is the tiled client of the dragged client's workspace whose bounds hold the pointer, found
  by scanning the workspace and not through `context.hitTest`, which can answer with a hidden window of another
  workspace. Nothing moves while the pointer is down, so the layout computes once, on release.

    A release outside every tile, over the held tile, or a cancel changes nothing. There is no ghost or drop highlight;
    the spec's `previewSwap` is not written. A client whose `tiled` flag is false (floating, a dialog), or that is
    fullscreen or maximised, declines.

- **Split.** A layout with dividers answers `findSplit(client, point)` with a `LayoutSplit`: the axis the pointer moves
  along, the pixels of travel that change the ratio by 1 (`span`), the direction, the limits and `reset`.

    The binding sets `ratio` from the pointer's travel since the press, so the divider follows the pointer exactly, and
    the layout computes per frame because its own field changed. `MasterStackLayout.ratio` is the field (`direction` -1
    for a master on the right or at the bottom, `span` is the extent less the gap).

    `DwindleLayout` gives each split a ratio of its own (`null` follows `layout.ratio`), picks the split above the pressed
    tile whose divider is nearest the press, and counts edits in the public field `revision`, because the tree is not a
    field the workspace could compare. A cancel gives the split back to `layout.ratio`.

- **Not done.** The spec's `previewSwap` preview, and a split chosen by hit-testing the divider's own pixels: the nearest
  divider above the pressed tile takes the drag wherever in the tile the press is.
