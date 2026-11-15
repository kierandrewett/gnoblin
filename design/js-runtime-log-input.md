# JS runtime change log: tree binding, input and workspaces

The finished work of the tree binding (P3), key and gesture bindings (P3c), pointer chord claims and workspaces (P3b).
Part of the [JS/TS runtime design](js-runtime.md).

## P3 status

`JsShell` (`crates/gnoblin-js/src/shell.rs`) does for QuickJS what `ShellVm` does for Lua, over `gnoblin-tree-client`.
The object model is TypeScript, embedded as `gnoblin/host`: `data/js/gnoblin/host.ts` lists its public exports and
`data/js/gnoblin/host/*.ts` is the code.

`JsShell` embeds the stock library, which holds it, and appends `export * from "gnoblin/host"` to the stock `gnoblin`
index. One `import { Workspace, ClientListener } from "gnoblin"` reaches both.

The Rust side (`src/bindings/`) is plain host functions behind `globalThis.__gnoblinHost`. It runs against a fake
compositor in `crates/gnoblin-js/tests/shell.rs`, and against the real one by ["P6 role
wiring"](js-runtime-log-config.md#p6-role-wiring).

Structure of the object model (was one `gnoblin.ts` of 5,186 lines with 279 top-level declarations):

- **Layers.** `gnoblin/types` and `gnoblin/errors` are leaves. `host/bridge`, `host/contract` (types of the two sides of
  the bridge) and `host/context` (the shared state, see below) sit on them. Above come per-concept modules
  (`client`, `workspace`, `output`, `collections`, `desks`, `membership`, `work_area`, `fitting`, `placements`,
  `styles`, `layout_driver`, `snapshot`, `adoption`, `frame`), then the wiring (`policy`, `services`, `xr_bridge`,
  `session_bridge`, `rollback`, `staging`, `input_bridge`, `displays`, `roots`) and last `facade`.
- **State.** The 14 reassigned module-level `let`s and the module maps are fields of one `HostContext` (`host/context`);
  no module keeps state of its own. It is an exported instance, not a parameter, because a script constructs
  `new Workspace(...)` and has nothing to pass it.
- **Composition root.** Only `host/facade` runs anything when imported: it calls each `install*` function in order, makes
  `gnoblin`, adopts the existing nodes and hands the shell its `Entry`.
- **No cycles.** `stock::tests` fails when the stripped stock modules import each other in a loop. Type-only imports
  (erased) still form a loop among the object classes, which is inherent to `Client`, `Workspace` and `Output`
  naming each other. A lower module asks `isClient`, `isOutput` or `isWorkspace` instead of `instanceof`.
- **Module table.** `crates/gnoblin-js/build.rs` embeds every `.ts` under `data/js/gnoblin`; a new file needs no edit.
- **Sizes.** The largest host module, `client.ts`, is 577 lines (its `animate` types are in `client_animation.ts`). In Rust, the `shell`, `input`, `ui`,
  `generation`, `reload`, `console`, `host_process` and `bindings/host` modules are directories, none over 600 lines
  (`shell.rs` 1433 became `shell/mod.rs` 571 plus `frame`, `txgate`, `links` and `document`; `input.rs` 984 became
  `input/mod.rs` 226 plus `claims`, `seats` and `install`; the inline tests of each are in `tests.rs`).
- **Split.** No `.ts` file under `data/js` or `.rs` file under `crates/gnoblin-js/src` is over 600 lines. `ui`,
  `display` and `display_manager` are directories (`gnoblin/ui/*` with `state` as the one owner of the host, the
  report and the live canvases; `display/*` with `state` for the shell's hooks; `display_manager/*`), and `ui.ts`,
  `display.ts` and `display_manager.ts` are barrels listing their exports by name. The largest is
  `display_manager/manager` (585). `loader.rs` (677) keeps the resolver and loader; the file resolution and its tests
  are `loader/files.rs`.

- **Boot order.** `JsShell::new` attaches, drains the snapshot and loads `gnoblin/host`, so `gnoblin.clients` exists
  for the entry. `load_entry` evaluates `init.ts`. The first `frame()` announces `ready` for the snapshot's nodes,
  parents first. Then every `frame()` applies the compositor's batches to the mirror, tells the listeners, and sends
  one transaction.
- **Protocol rules.** Copied from `ShellVm`, not shared: one transaction in flight, replies matched by position,
  `stale-base` and `unknown-node` refusals sent again ahead of the next frame's writes, `STALE_RETRIES` (3) in a row.
  The tests are the Lua ones ported (hold, stale resend, retry cap, unknown node, refusal report).

    Two rules are the JavaScript shell's own, because the refusals of the real compositor showed what the Lua VM never
    met: a refusal applies nothing, so **every** op of it goes again whatever its kind (a call too, `Output.apply` among
    them), except a call the schema marks `secret`, which is asked of `gnoblin_proto::tree_call_is_secret` and not of a
    list of names;

    and a `stale-base` that names no op (`failed_op` -1: the base is older than the compositor's 64 batches) sends the
    **whole frame** again on the refreshed base.

    What cannot go again is not left waiting: a call is answered `failed`, which reaches its caller by token (the lock
    screen's check, an `OutputApplication`).

    A property write given up after its resends is forgotten by the builder and handed to the layout driver
    (`lost_writes`), which lays its workspace out again, because a placement is computed only for a workspace that is
    dirty and the pass cleared that before the write went out.

    Tests: `crates/gnoblin-js/tests/write_refusals.rs` over the shared fake compositor, which refuses as the real one does
    (`Fault`), and `crates/gnoblin/tests/js_lock.rs` against the real one.

- **Objects.** One `Client` per window node for the life of the generation (workspaces are the script's, see P3b), so
  the same node is the same JS object.

    A client has `id` (the node id as a string), `appId`, `title`, `focused`, `fullscreen`, `x`, `y`, `width`, `height`,
    `bounds`, `workspace`, `alive`, `close()`, `focus()`. Only the four geometry numbers are writable; the compositor owns
    the rest.

    A write is queued and read back at once, and travels at frame end. A destroyed node keeps its last values, is `alive
=== false`, and every write or call throws a `GnoblinError` (`code` `not_found`, `target` `Client#3.x`).

    A value that is not a finite number throws a `TypeError` naming the member. A `Client` cannot be constructed by a
    script.

- **Collections.** `gnoblin.clients` and `gnoblin.workspaces` are live `Collection`s: `length`, `at`, `get`, `find`,
  `filter` and `map` (snapshots), iteration, `where`, `addListener`, `removeListener`. `gnoblin.clients` has no `add`.

    `where(query)` is a live selection over a data-only query. Each key is a property, and each value is what it must
    equal or an array of values of which one must.

    The properties for clients are `id`, `appId`, `title`, `focused`, `fullscreen`, `x`, `y`, `width` and `height`.

    Its membership follows property changes. It is not tracked until it has a listener.

- **Listeners.** `ClientListener` (`onTitleChanged`, `onFocusChanged`, `onGeometryChanged`, `onClosed`, `onReady`)
  and `CollectionListener` (`onAdded`, `onRemoved`) are plain base classes; a listener may implement any of the
  methods, with or without the base class. Argument 1 is the origin object; `onAdded` and `onRemoved` also get the
  collection. On a collection, a client listener hears every member and stops hearing a member that leaves. On a
  client, it hears that client.
- **Replay on attach (decision for spec-review 1.25).** `collection.addListener(l)` calls `l.onAdded(member,
collection)` at once for each member already in the collection, in collection order, so a config that loads or
  reloads against existing windows sees them. Member events (`ClientListener`) are not replayed: nothing happened.
  Adding the same listener twice is a no-op.
- **When an event fires.**

    Change events are raised from what the compositor published, compared with what the listeners were last told, so a
    batch that sets a title to its current value raises nothing, two batches in one frame raise one event with the last
    value, and the echo of the script's own write raises an event like any other change.

    A window made in a frame is born with its final values and raises no change event. A node that was there at boot
    raises no `onAdded`. Order per client: title, focus, geometry; then membership changes.

- **Errors and budgets.** A listener runs under its own budget (`entry_budget`, `ENTRY_BUDGET`, 1 s), the frame's
  dispatch under `frame_budget` (`FRAME_BUDGET`, 5 s); queued calls that do not fit wait for the next frame.

    A throw is caught in the module, returned as a diagnostic with the `.ts` location, and reported once per site
    (listener class, method and position); the next listener still runs. A budget or memory trip stops that one call, is
    reported, and counts a strike (`JsShell::strikes`); the next call still runs.

    What to do about strikes is the supervisor's.

- **`gnoblin.spawn(command)`.** A string is a command line, an array is an argv. The shell does not start
  processes: `JsShell::set_spawner` takes the host's callback (`JsSpawnRequest::Command` or `Argv`), and a refusal
  or a missing spawner throws `GnoblinError` with the host's code (`not_found`, `busy` or `internal`). A top-level `spawn` in `init.ts` runs on every load.
- **Not done.** Pointer and key events addressed to nodes and call answers from the compositor are consumed and
  dropped; claimed chords and gestures go to the input module (see "P3c status"). `JsShell` owns its generation:
  `swap_generation` and the staging hook that wire it into `JsHost` staging are ["P6 role
  wiring"](js-runtime-log-config.md#p6-role-wiring) (it calls `JsShell::retire` before the swap). The compositor-side
  `where` evaluation is not built.

## P3c status

Key bindings, touchpad gestures and pointer drag bindings: `data/js/gnoblin/bindings/` (the grammar and the binding
classes), `data/js/gnoblin/input/` (`gnoblin.input` and the matchers) and `crates/gnoblin-js/src/input.rs` (the host
functions: seat claims and the seat's published state). The tests are `crates/gnoblin-js/tests/input.rs`, over a fake
tree link that records claims and plays back chords, gestures, pointer positions and window deaths.

- **Trigger grammar (spec-review 1.22).** `parseTrigger` is the only parser, in TypeScript so a binding rejects a bad
  spelling when it is built, with or without a host.

    A trigger is modifiers joined with `+`, then one key, mouse button (`MouseLeft`, `MouseMiddle`, `MouseRight`, with an
    optional `Drag` or `Click`), wheel, or a gesture alone (`Swipe3`, `Swipe3Left`, `Pinch4In`: 2 to 5 fingers, optional
    direction).

    Modifier names are case-insensitive: `Shift`, `Ctrl` (`Control`, `Primary`), `Alt` (`Mod1`), `Meta`, `Super` (`Logo`),
    `Hyper`; `Mod2` to `Mod5` are refused as the compositor refuses them.

    A key is a keysym name, never the character it types, so `Super+1` and `Super+Shift+1` are two triggers and `Super+!`
    is an error.

    The canonical spelling orders the modifiers `Shift, Ctrl, Alt, Meta, Super, Hyper`, writes a single letter in lower
    case and the common key names one way (`Enter` is `Return`, `pagedown` is `Page_Down`), so two spellings of one
    trigger are one trigger.

    A lone `Super` is a tap. A literal plus is `plus`. There are no sequences. An error names the offending token. The
    compositor's own chord spelling (`<Shift><Super>1`) is derived from the canonical one (`claimOf`), and is the only
    place that knows it.

    Key names are not checked against the keymap: the compositor decides, and drops a claim it cannot parse. The
    declaration file types the trigger as template literal unions.

- **Claims.** `InputContext.bind(trigger, fn | Binding)` returns the binding; `binding.unbind()` (or
  `bindings.remove(id)`) takes it away.

    Each distinct chord is one `Seat.claim` (pointer chords with `grab`, so the release comes back; a drag and a click of
    one button share the claim), each gesture kind and finger count one `Seat.claim_gesture`, and a binding for one device
    claims with that device's name. Ids are allocated per shell, from 1, never reused.

    A claim is released with the last binding that needs it, and every claim on `retire`. Claims are queued writes, so
    they travel with the frame's transaction. A function bound to a drag, or to a wheel (no compositor claim exists for
    it), throws at `bind`.

- **Frame loop.** The compositor's `chord`, `pointer_chord` and `gesture` signals are parked by `JsShell::frame` and
  handed to the input module after the listeners are planned.

    The module only schedules: each hook runs as its own call under the per-call budget (`entry_budget`), a throw is
    reported once per site and never stops the others, and calls that do not fit the frame's budget wait for the next
    frame.

    A pointer drag is read from the `pointer_chord` messages (the position travels in them; the seat's `pointer_x` and
    `pointer_y` are not sampled), and at most one sample waits behind the scripts, which read the latest position when
    they run (latest wins). Gesture updates merge the same way.

    `onStart`, `onEnd` and `onCancel` are never merged or dropped.

- **Drags.** `start` records the press place and the client under it (`InputContext.hitTest`: the topmost client in tree
  order whose bounds hold the point and that is not minimised; the real tree's order was not proved against overlapping
  windows, see below).

    Once an `update` is `threshold` (default 4) pixels from the press, the candidates are asked `onStart` in precedence
    order; the first to return a state object owns the drag, and its `event.position` is the press position, so a binding
    that measures from `position` measures from the press.

    `end` is the release: a drag that has started gets `onEnd` at the end place, an `end` with no drag is a click
    (`onPress`), and a pointer that went past the threshold and was declined is neither. The compositor's `cancel` calls
    `onCancel`.

    A drag event carries `x`, `y` (the same as `position`), `dx`, `dy` (from the press), `button` (X11 numbering),
    `modifiers`, the `context`, the `client` under the press, and `workArea` and `dragConstraint` (the primary screen's
    rectangle and the seat's `drag_constraint`).

    State lives in the gesture, per context, never on the binding. A client dying mid-drag, the binding being removed or
    given another trigger, a second `start` on the same claim, an `onUpdate` that throws (that drag alone: it is reported
    once and stops, the other drags run on) and `JsShell::retire` each call `onCancel` once.

    The compositor says nothing of a drag whose claim goes, so the module cancels the drags of a binding it releases
    itself.

- **Gestures.** A directed gesture (`Swipe3Left`) starts when its direction is known (12 touchpad units of centroid
  movement, 0.1 of scale for a pinch); an undirected one starts at the begin. `event.progress` is 0 to 1 along the
  binding's own direction (400 units for a swipe, 0.5 of scale for a pinch). A function bound to a gesture runs once
  when the gesture ends past 48 units (0.12 of scale).
- **Precedence (spec-review 1.21).** Device, then layout, then context, with `priority` ordering only inside a tier; a
  binding that declines (`onStart` returns `false`, `onPress` returns `false`) lets the next candidate have the input.

    Registration refuses two bindings of one collection that match one trigger at one priority (`BindingConflictError`,
    naming both ids); a layout binding and a context binding for one trigger are not a conflict, the layout's wins for its
    clients.

    A claim for one device shadows the all-devices claim in the compositor, so the context's and layouts' bindings for
    that trigger are candidates of the device's claim too.

- **Stock bindings.**

    `ClientMoveBinding` (Super+MouseLeft+Drag) and `ClientResizeBinding` (Super+MouseRight+Drag) are TypeScript over
    `client.bounds` and `event.position`. The resize binding:

    - uses `edges: "quadrant"` by default: the corner is picked from the initial pointer position and never changed;
    - holds a minimum of 100 by 80 against the opposite corner;
    - honours `client.minSize` and `client.maxSize` when the client has them (the tree carries no size limits today, so
      they are absent).

    Both write `floatingBounds` on release and restore the start on cancel.

    A fullscreen or maximised client declines both: `onStart` returns `false`, and the compositor has already withheld the
    press from the client and the shell's nodes, so the press is consumed and ignored (the shell cannot hand a withheld
    press back, so there is no better answer; a maximised window is not floated first as the Lua shell's drag did).

    A move keeps the window where the seat's `drag_constraint` allows. The code is `dragConstraintMode`, `clampAxis` and
    `constrainRect` in `gnoblin/bindings`, ported from `data/lua/gnoblin/shell/drag.lua` with its test cases.

    - `work_area` keeps the window inside the primary screen (a window at least as large as the screen is pinned to its
      origin).
    - `minimum_strip` lets it leave as long as 75 pixels stay inside.

    The work area is the screen, since the shell has no bars of its own yet. `FloatingLayout` registers both as `move` and
    `resize` (`undefined` keeps the default trigger, `null` leaves the binding out, spec-review 1.19).

    The `snap` option and `edges: "nearest"` are written (see ["Focus, rules and snapping
    status"](js-runtime-log-policy.md#focus-rules-and-snapping-status)). `ClientSwapBinding` and `SplitResizeBinding` are
    written (see ["Tiling bindings status"](js-runtime-log-policy.md#tiling-bindings-status)). Not written:
    `WorkspaceSwitchBinding`.

- **Layouts.**

    The layout of every workspace in `gnoblin.workspaces` is attached to every input context while a workspace uses it
    (checked once per frame): its bindings are live for the clients whose workspace's `layout` it is, they follow the
    layout's `bindings` collection, and they go, cancelling a drag they run, when the workspace takes another layout or
    leaves the collection.

    A layout whose bindings the seat refuses is reported once and left unattached. `InputContext.attachLayout(layout,
owns?)` and `detachLayout` do the same by hand for a layout no workspace holds, with the client test `owns` (a layout
    attached by hand is never detached for the workspaces).

- **Contexts and devices.** One `InputContext` per seat node; `focusedClient` and `pointer` read through to the seat,
  `activeDisplay` is `null`. `devices` is parsed from `Seat.device`; a device's `bind` claims for its name.

    A context or device the seat loses is disposed and its drags cancelled. `new InputContext` is not available: contexts
    are the compositor's seats.

- **Proved against the real compositor.** `crates/gnoblin/tests/js_shell.rs`,
  `super_drags_move_and_resize_a_real_window_through_the_pointer_chord_stream` and
  `a_super_drag_takes_the_window_on_top_where_two_overlap`: a real session, a real Wayland window, an `init.ts` with a
  floating workspace and the default `FloatingLayout`, and Super, pointer motion and Button1 and Button3 injected
  through the session's input tap (the seat path a device's events take).

    Super+Button1 moves the window by the pointer's delta, Super+Button3 resizes from the corner nearest the press and
    holds the same corner when the pointer crosses the window, down to the 100 by 80 minimum, all read back from the
    compositor's scene.

    This also proves that the pointer's space is the space of client `x` and `y` (a press at a window's local point hits
    it), and that tree order is stacking order for `hitTest` (of two windows at one place the one mapped last takes the
    press).

    The claims reach the seat before the first press only after the shell's first frames, so the test repeats the gesture
    until a press is claimed.

- **Proved only against the fake link.**

    The decline of a maximised client, and a minimised client being skipped by `hitTest` (`visible` is not read: the
    shell's `Client` has no such member).

    The drag constraints against a seat that publishes `drag_constraint`, a throwing `onUpdate`, and `cancel`.

    Key chord and gesture claims use the calls the Lua shell makes, byte for byte, but were not run against the
    compositor. `Wheel` has no claim.

## Pointer chord claims

What the compositor offers a shell that wants to move or resize a window by dragging with JavaScript only (package
P3c, "Drags"). `crates/gnoblin-wayland/src/pointer_chord.rs` implements it and does not know any scripting language.

**The claim.** The same call as a key chord, on the seat node: `Seat.claim(id, chord, grab, device)`. The call's
arguments are the little-endian `i64`s `id` and `grab`, and its text is the chord, or the device's name, a line feed and
the chord.

`claim` in `crates/gnoblin-js/src/input.rs` already makes exactly this call, so `input.claim(seat, chord, grab, device)`
with a pointer chord is the whole API. `grab` is ignored for a pointer chord: the release always comes back. The
compositor reads a chord that ends in a button name as a pointer chord and any other as a key chord.

- The chord is modifiers in angle brackets and then `Button1` to `Button9` (`<Super>Button1`, `<Control><Alt>Button2`),
  or `Button8` or `Button9` alone.

    The numbers are X11's: 1 left, 2 middle, 3 right, 8 back, 9 forward, so `MouseLeft`, `MouseMiddle` and `MouseRight`
    are 1, 2 and 3. `Button1` to `Button7` need a modifier, and a claim without one is dropped.

    `claimOf` already writes the modifiers in brackets, in the compositor's order.

- One holder per chord and `device`: a second claim of the same chord for the same device is refused as a keyboard
  chord's is, with the same silence (the claim call applies, the compositor logs and drops it). A claim for one device
  wins over a claim for every device on that device. The modifiers match exactly: `<Super>Button1` is not
  `<Shift><Super>Button1`, and Caps Lock and Num Lock do not count.
- `Seat.unclaim(id)` ends the claim. Ids share one number space with `claim_gesture`, as before.

**The stream.** While a claim matches, the compositor withholds the button's press from every client and from the
shell's nodes and delivers the drag as the seat's `pointer_chord` signal. It arrives on the tree link as
`TreeIncoming::PointerChord(TreePointerChord)` (`crates/gnoblin-tree-client/src/tree_mirror.rs`), one message per
signal, in order:

| Field       | Type   | Meaning                                                                                |
| ----------- | ------ | -------------------------------------------------------------------------------------- |
| `phase`     | string | `start`, `update`, `end` or `cancel`.                                                  |
| `claim`     | int    | The id `claim` returned.                                                               |
| `x`, `y`    | double | The pointer in global logical coordinates, the same space as the seat's `pointer_x`.   |
| `dx`, `dy`  | double | The pointer's movement since `start`. Both 0 at `start`.                               |
| `button`    | int    | The chord's button number (1, 2, 3, 8, 9).                                             |
| `modifiers` | string | The chord's modifiers as `claimOf` writes them, `<Super>`; empty for a bare `Button8`. |
| `time`      | int    | The input event's time in milliseconds, wrapping at 2^32.                              |

- `start` is sent when the button is pressed with the chord's exact modifiers held and the session unlocked. `end` is
  sent when the button is released, whether the modifiers are still held or not. Between them `update` is sent when the
  pointer has moved, at most once per frame, with the latest place: a shell that runs once per frame needs no coalescing
  of its own. `start`, `end` and `cancel` are never merged or dropped.
- No client hears the button or the pointer's motion during the drag, and the shell's nodes hear neither: the shell has
  the pointer. The seat's `pointer_x` and `pointer_y` follow it. When the drag ends the pointer is given the surface it
  is over.
- `cancel` is the compositor ending a drag whose owner is alive: the session locked, or the pointer's device was
  removed. It is the last signal of the drag, and there is no `end` after it.
- A claim that goes while its drag is live (`unclaim`, a replaced claim, the seat's destruction, or the shell's
  connection closing) ends the drag silently: no `end` and no `cancel`, because the owner is gone or has said it does
  not want the drag. The button's release is withheld from clients as the press was, so no client sees a button it
  never saw pressed, and none is left with one down. A shell that restarts and claims afresh starts clean.
- A press that does not match (other modifiers, another button, a locked session) is not the claim's: clients get it
  and the shell hears nothing. A second press of a button already captured on the same pointer cancels the first
  drag.
- Everything is per pointer and button: two buttons held together are two drags, each with its own `start` place.

**What the JS side does with it** (`crates/gnoblin-js/src/input.rs`, `data/js/gnoblin/input/`). `JsShell::frame` parks
each `TreeIncoming::PointerChord` in `JsInput::inbox` beside `Chord` and `Gesture`, and hands the frame's batch to the
input module as `{ kind: "pointer", claim, phase, x, y, dx, dy, button, modifiers }`.

`start` is the press, each `update` a drag sample, `end` the release and `cancel` the `onCancel` ("Drags" in "P3c
status"). The pointer's place comes from the message, not from the seat's `pointer_x` and `pointer_y`.

A pointer claim gets no `chord` message.

## P3b status

Workspaces and the layout driver (`data/js/gnoblin/host.ts`, `data/js/gnoblin/workspace.ts`,
`crates/gnoblin-js/src/workspaces.rs`, tests in `crates/gnoblin-js/tests/workspaces.rs`).

- **A workspace is the script's.** `new Workspace({ id, name?, layout?, gaps? })` (`layout` is a `Layout` or a stock
  name; default floating) and `gnoblin.workspaces.add(ws)`. The compositor's own `workspace` tree node is no longer a
  member of anything, and the tree has no property that says which workspace a window is on, so membership lives in the
  JS shell.

    `gnoblin.workspaces` is a `MutableCollection<Workspace>` (`add`, `remove(ws | id)`, `clear`, `show(ws | id)`, plus the
    read API); a second workspace with an id already there throws `GnoblinError` `already_exists`. A workspace has `id`,
    `name`, `index`, `active`, `layout`, `gaps`, `clients`, `show()`; assigning `layout` or `gaps` marks it dirty.

- **`ws.clients`** is a live `MutableCollection<Client>` of the same `Client` objects as `gnoblin.clients`.
  `add(client)` transfers primary membership (`client.workspace` updates, the old workspace loses it,
  `onAdded`/`onRemoved` go to collection listeners). `remove` leaves the client with `workspace === null`.

    Removing a workspace moves its clients to the first workspace left. Nothing closes a client: `alive` stays true.

- **Initial workspace rule (question 1, simplified).** One function, `initialWorkspace` in `workspace.ts`: the first
  workspace flagged active, else the first. A new toplevel joins it. Active is a single flag for now (the first
  workspace added, or the last `show`n), standing in for "the workspace the primary display shows".

    Windows that were there at boot join it on the first frame, not while the entry loads, so a reload can put them back
    first. `gnoblin.workspaces.add(new Workspace(...))` makes real `Workspace` objects; a config that declares none gets
    `desktop` (floating) when its entry has loaded.

- **Layout driver.** At the end of every frame, after all listeners, `entry.layout()` runs.

    For each workspace it diffs the layout's own enumerable fields against the last snapshot (J6: `Object.is` per key, so
    an edit in place is not seen and a `#private` field is not a field), the gaps, and the area (the primary screen's
    `width` and `height` from the tree's `screen` node; no screen, no layout).

    The layout's `onClientRemoved` and `onClientAdded` hear of the clients that left and arrived, and a dirty workspace
    that is the active one calls `layout.compute({ workspace, children, area, gap, focused })` once and writes the
    placements that differ from the client's values (x, y, width, height; `visible` and `tiled` have no tree property
    yet). Placements are validated before any is written.

    A workspace that is not active stays dirty until it is shown. Windows of other workspaces are not hidden: the tree has
    no visibility property to write.

- **Floating.** `FloatingLayout.compute` places nothing; its `onClientAdded` places a window once on arrival (a
  window moved to the workspace by `clients.add`, by removal of another workspace, or created while the shell runs).
  A user-moved window is left alone because no later frame calls the layout for it.
- **Coalescing.** `send_transaction` keeps the last instant `Set` of each (node, property) in a transaction, so a
  handler's repeated writes and the layout's are one op (`workspaces.rs: coalesce_sets`). Animated writes are not
  merged.
- **Reload.** `JsShell::workspace_snapshot()` returns `JsWorkspaceSnapshot` (workspace ids in order, each with its
  client node ids, and the active id).

    After the new generation's entry has loaded, `JsShell::reconcile(&previous)` puts each client back on the workspace of
    the same id without offering it to the layout, so a floating window keeps its place and no second `desktop` appears; a
    client whose workspace is gone moves to the first workspace and is placed as an arrival.

    The active workspace is kept when it still exists. The tests run two shells in turn over one fake link.
    `JsShell::swap_generation` does the same over one shell (["P6 role wiring"](js-runtime-log-config.md#p6-role-wiring)).

- **Not done.** Per-display workspaces (`display.viewport.content`), `where` queries on a `Workspace` beyond its
  scalar properties, a `visible` write, and hooks on `Layout` for the binding lane.
