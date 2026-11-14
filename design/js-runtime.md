# Gnoblin JS/TS runtime (D36)

Status: proposed by the agent on 2026-11-13, after the owner replaced the Lua shell language with
JavaScript on QuickJS. The owner's API spec is `design/js-research/owner-spec.md` and wins on API
shape. This file records the engineering decisions around it. Each decision can be overruled.

Inputs (all in `design/js-research/`):

- `quickjs-embedding.md`: engine, binding crate, hot reload, budgets.
- `ts-in-quickjs.md`: how `.ts` runs, source maps, types.
- `current-runtime-map.md`: what exists in Rust today and what a `gnoblin-js` crate replaces.
- `spec-review.md`: contradictions in the spec, effort ranking, first slice, acceptance tests.
- `popups-portals.md`: who draws built-in dialogs, API for them.

## Decisions

| ID  | Decision                                                                                                                                                                                                                                                                                                                           | Why                                                                                                                                   |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| J1  | Engine is rquickjs 0.14 on quickjs-ng, in a separate supervised `gnoblin-js` process.                                                                                                                                                                                                                                              | Same shape as the Lua shell VM today (owner socket, tree protocol). The compositor never waits on JS. rquickjs only binds quickjs-ng. |
| J2  | `.ts` runs directly. oxc (`=0.158.0`, pinned) strips types in the module loader, with a source map. No embedded type checker.                                                                                                                                                                                                      | A user never runs a build step. oxc matches the workspace MSRV 1.97.0. TypeScript 7 has no embeddable compiler.                       |
| J3  | Editors get a generated `gnoblin.d.ts` and a `tsconfig.json` written on first run. `gnoblinctl check` checks the module graph and syntax itself, and runs an external `tsc` for types when one exists.                                                                                                                             | Types come from the schema, like `gnoblin.d.lua` does today.                                                                          |
| J4  | Two tiers. Tier 1: `export default defineConfig({...})`, plain data, a closed set of key actions, diffable on reload. Tier 2: the full object API from the spec, reached from `setup(gnoblin)` or from any module.                                                                                                                 | A user who wants a key bound or a gap set does not need `new Workspace(...)`. Tier 1 desugars to tier 2.                              |
| J5  | Hot reload swaps a whole generation. New Runtime and Context per generation, stage, validate, commit at one compositor boundary, then drop the old one. A failed reload keeps the old generation and reports file, line and column in `.ts` coordinates. File watching uses inotify.                                               | QuickJS has no module eviction. A fresh Runtime is the only clean invalidation.                                                       |
| J6  | Observable fields on user subclasses use a snapshot diff at transaction boundaries. A Proxy in the base class constructor is the fallback if the diff is not enough.                                                                                                                                                               | Layouts and bindings run in the JS process, so Rust does not have to see every write. No Proxy is visible to users.                   |
| J7  | XR stays an out-of-process program on the owner socket (D34/D35). It is not a `.so` loaded into any process.                                                                                                                                                                                                                       | `extensions/gnoblin-xr` already works that way. Spec section 17 is replaced by the owner-socket model.                                |
| J8  | Rust draws the security-critical built-in dialogs: portal consent, secret and passphrase entry, polkit, lock password entry. JS themes them (`dialogs.style`), can intercept them (`DialogListener`), and can replace them only through the owner connection. The compositor owns each request and its deadline. JS holds a lease. | A crashed or reloading runtime must never leave the keyring, polkit or the lock without a UI. See `popups-portals.md`.                |
| J9  | The Lua shell stays as the reference until the JS shell passes the same golden geometry corpus. Then delete `gnoblin-lua`, `gnoblin-lua-sandbox`, `data/lua/**` and the Lua docs.                                                                                                                                                  | Spec section 22. No half-migrated state.                                                                                              |

## Zero-config baseline

Without any `init.ts`, the stock library runs: one `desktop` workspace, floating layout, one display
per output, and the stock key bindings of the Lua shell. The stock library is itself TypeScript,
shipped in `/usr/share/gnoblin/js/` and embedded as the `gnoblin` module.

## Slice 1 (first vertical slice, smallest proof)

`~/.config/gnoblin/init.ts` with a `./x.ts` import runs in the `gnoblin-js` process, talks to the
compositor over the existing tree protocol, tiles real windows with a `compute()` layout, binds
keys, and reloads on save without losing window ids.

Done criteria (headless, on Agent37):

1. `init.ts` with type annotations and an `enum`, importing `./x.ts`, evaluates. A thrown error
   reports the `.ts` line and column.
2. A `MasterStackLayout` `compute()` gives the exact rectangles of the Lua `tiling.lua` golden
   corpus.
3. Editing `init.ts` triggers a reload. Window ids and floating positions survive.
4. A syntax error or a throw in the new file keeps the previous generation running and reports the
   error.
5. An infinite loop in a handler is interrupted by the CPU budget. The compositor keeps rendering
   and the runtime restarts.
6. Killing the runtime leaves the compositor responsive. The Rust fallback places new windows and
   the supervisor restarts the runtime.
7. `gnoblinctl check init.ts` reports a syntax error with a `.ts` location.

## Work packages

| Package                  | Content                                                                                                                             | Depends on    |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------- | ------------- |
| P1 `gnoblin-js` core     | rquickjs runtime, generation, module loader and resolver, oxc strip with source map, interrupt budget, memory limit, bytecode cache | none          |
| P2 types                 | `gnoblin.d.ts` generator from the schema, `tsconfig.json` writer, `gnoblinctl check`                                                | P1 loader API |
| P3 tree binding          | JS host objects over `tree_link` and `tree_mirror`, `Collection<T>`, `Client`, `Workspace`, listener classes                        | P1            |
| P4 stock library         | `defineConfig`, `FloatingLayout`, `MasterStackLayout`, `GridLayout`, bindings, rules, goldens against Lua                           | P3            |
| P5 reload and supervisor | watcher, generation swap, failure keep-old, restart, Rust fallback                                                                  | P1, P3        |
| P6 dialogs               | `gnoblin.dialogs`, `DialogListener`, `SecureEntry`, lease and `request.release`, Rust built-ins for secret entry and polkit         | P3            |
| P7 console               | JSON-over-socket REPL and inspector, `gnoblinctl console`                                                                           | P3            |
| P8 later, separate sizes | styles and effect stack, Display mirror and span, transition planner, XR program on the new API                                     | P4            |

### P2 status

`crates/gnoblin-js-types` renders `gnoblin.d.ts` from the schema catalog (`render_dts`) and writes it with a
`tsconfig.json` into the config directory (`ensure_user_files`: the declarations are refreshed on every call, the
`tsconfig.json` only when absent). One interface per schema object; a signal `mode_changed` is
`onModeChanged(handler)`. The hand-written stock library (`defineConfig`, `Client`, `Collection<T>`, `Layout`, ...) is
checked in as `static/gnoblin-stock.d.ts` and marked proposed. `gnoblinctl check FILE` (a `.ts`, `.mts`, `.js` or
`.mjs` file loads through the `gnoblin-js` loader in a throwaway generation with a 1 s budget; any other file keeps the
configuration check) and `gnoblinctl types init [DIR]` are done (`crates/gnoblinctl/src/js.rs`). `check --types` runs an
external `$GNOBLIN_TSC`, `node_modules/.bin/tsc`, `tsc`, `tsc6` or `tsgo`, with the generated declarations in a temporary
directory, and says so on one line when none is installed. The check has no host, so the global `gnoblin` and the
`gnoblin` and `gnoblin/<module>` imports are bound to an inert stub; wrong names from the shell are `--types`'s to find.
`gnoblinctl reload` is not done: it needs the supervisor process.

### P4 status

The stock library is TypeScript under `data/js/gnoblin/`, compiled into `gnoblin-js` (`stock_modules()` strips it once
and returns the embedded `gnoblin` module family: `gnoblin` re-exports `gnoblin/bindings`, `gnoblin/layout`,
`gnoblin/layouts/{floating,grid,master-stack,monocle}` and `gnoblin/config`; the modules import one another by those
names). It reaches the host only through `globalThis.gnoblin`.

Parity-tested against the Lua goldens (`crates/gnoblin-js/tests/stock_layouts.rs`, in a real generation with a fake
host): `MasterStackLayout` (the `Tiling` corpus: 0 to 3 and 200 nodes, gap, ratio, top orientation, resize),
`GridLayout` (row and column fill, gap), `MonocleLayout` (focus picks the shown node), and `FloatingLayout.place` for
every case of `tests/shell/placement_cases.toml`. Tiled rectangles are floored to whole pixels as in Lua. Defaults are
the Lua ones (ratio 0.5, gap 0), not the 0.65 and 8 of the spec's illustrative constructors. Derived from the same
algorithm, not in the Lua corpus: master count, right and bottom orientations, fixed grid `columns`.

`LayoutContext` carries the workspace rectangle and `gap: { inner, outer }`; a tiling layout insets by `outer` and uses
`inner` unless its own `gap` is set, so one `defineConfig({ gaps })` setting reaches every stock layout. `defineConfig`
checks the shape (closed key sets, closed key-action union, references to undeclared workspaces) and registers the
config through `globalThis.gnoblin.__register` when the host provides it; `desugarConfig` is the table of
spec-review.md section 4 as a plain-data plan. `defaultConfig` is one floating `desktop` workspace, `Super+1` and the
media keys of `data/default-config`.

`Binding` and `BindingCollection` are wired by the input module (see "P3c status"): the `triggers` option of
`FloatingLayout` registers its `move` and `resize` bindings, which go live when an input context attaches the layout.
The swap and split-resize drag behaviours (`MasterStackLayout`, `GridLayout`) are not written; `style` and `outputs`
are stored and have no native effect; `rules`, `on` and `setup` are in the plan for the host to apply. Not ported:
`Dwindle` (it needs per-workspace tree state), `Strip`, the launcher (`Super+space`), `Super+Left` and `Super+Right`
(Strip only) and the `Swipe3` workspace slide, which depend on shell classes that have no JS counterpart yet.

## Hot reload

Status of P5: the engine-side half is implemented in `crates/gnoblin-js` (`watch.rs`, `reload.rs`,
`supervisor.rs`). It runs without a compositor. Process spawning, the Rust fallback shell and the real
compositor commit are not done: they belong to P3 and the process supervisor.

- **Watcher.** `ConfigWatcher` uses `notify` (inotify, CC0-1.0, no default features) on the config directory and
  on the directory of every module in the last generation's graph (`JsGeneration::module_paths`), non-recursively,
  because editors replace the inode. It drops editor scratch files (`*~`, `*.swp`, `4913`, `.#x`, `#x#`) and
  anything that is not `.ts`, `.mts`, `.js` or `.mjs`. After the first event it waits for 100 ms of quiet and
  returns one `ReloadRequest` with the changed paths; a burst that never goes quiet is cut off after 20 debounces.
  Any module file in a watched directory counts, imported or not: a new `./x.ts` can shadow an import that
  resolved to `./x.js`. A backend error or an inotify queue overflow becomes a request with no paths, because
  "possibly changed" must reload. Not done: comparing content hashes to skip a save that changed nothing.
- **Reloader.** `JsReloader` runs `JsHost::stage`, passes the staged generation to a caller-supplied `commit`
  closure (the compositor transaction, step 5 of the pipeline above), and only when that returns `Ok` calls
  `JsHost::commit`. A parse error, a throw, an interrupted loop or a rejected commit leaves the running generation
  untouched and returns a `ReloadReport { ok: false, errors }`. Each `JsDiagnostic` carries file, line and column in
  `.ts` coordinates (line and column are 0 when the engine reported no position, and `file` is then the entry).
  `reload_now()` is the manual trigger for `gnoblinctl reload`; `poll()` waits on the watcher, reloads and re-arms
  the watcher from the new graph. A module that failed to load is in no graph, so the file named by the last error
  stays in `watch_set()` until a stage succeeds.
- **Supervisor.** `JsSupervisor` is pure policy over an injectable `JsClock`. A budget interrupt or a panic is a
  strike (`JsFault`); other failures are not, because a thrown exception leaves the generation consistent. After a
  strike `JsReloader::recover` stages a fresh generation from disk and replaces the suspect one; a restage that is
  itself interrupted is another strike. Three strikes within 30 s latch `JsHealth::Degraded`: retrying stops and the
  last good generation, if one was ever committed, keeps running. A fault during the 5 minute cool-down is refused
  without staging. A deliberate reload (file save or `gnoblinctl reload`) clears the strikes and the degraded state.
  The defaults are `JsStrikePolicy::default()`.
- **Not in this package.** Spawning and restarting the `gnoblin-js` process, exponential backoff between process
  restarts, the heartbeat watchdog (L3), the Rust fallback when the runtime is down, and passing `gnoblin.hot.data`
  between generations.

## P3 status

`JsShell` (`crates/gnoblin-js/src/shell.rs`) does for QuickJS what `ShellVm` does for Lua, over `gnoblin-tree-client`.
The object model is TypeScript (`crates/gnoblin-js/js/gnoblin.ts`), embedded as `gnoblin/host`. `JsShell` embeds the
stock library beside it and appends `export * from "gnoblin/host"` to the stock `gnoblin` index, so one
`import { defineConfig, ClientListener } from "gnoblin"` reaches both; the stock sources are unchanged. The Rust side
(`src/bindings/`) is a set of plain host functions behind `globalThis.__gnoblinHost`. It runs against a fake
compositor in `crates/gnoblin-js/tests/shell.rs`, and against the real one by P6 role wiring (below).

- **Boot order.** `JsShell::new` attaches, drains the snapshot and loads `gnoblin/host`, so `gnoblin.clients` exists
  for the entry. `load_entry` evaluates `init.ts`. The first `frame()` announces `ready` for the snapshot's nodes,
  parents first. Then every `frame()` applies the compositor's batches to the mirror, tells the listeners, and sends
  one transaction.
- **Protocol rules.** Copied from `ShellVm`, not shared: one transaction in flight, replies matched by position,
  `stale-base` and `unknown-node` refusals sent again ahead of the next frame's writes, `STALE_RETRIES` (3) in a
  row. The tests are the Lua ones ported (hold, stale resend, retry cap, unknown node, refusal report).
- **Objects.** One `Client` per window node for the life of the generation (workspaces are the script's, see P3b),
  so the same node is the same JS object. A client has `id` (the node id as a string), `appId`, `title`, `focused`,
  `fullscreen`, `x`, `y`, `width`, `height`, `bounds`, `workspace`, `alive`, `close()`, `focus()`. Only the four
  geometry numbers are writable; the compositor owns the rest. A write is queued and read back at once, and travels
  at frame end. A destroyed node keeps its last values, is `alive === false`, and every write or call throws a
  `GnoblinError` (`code` `destroyed`, `target` `Client#3.x`). A value that is not a finite number throws a
  `TypeError` naming the member. A `Client` cannot be constructed by a script.
- **Collections.** `gnoblin.clients` and `gnoblin.workspaces` are live `Collection`s: `length`, `at`, `get`, `find`,
  `filter` and `map` (snapshots), iteration, `where`, `addListener`, `removeListener`. `gnoblin.clients` has no `add`.
  `where(query)` is a live selection over a data-only query: each key is a property (`id`, `appId`, `title`,
  `focused`, `fullscreen`, `x`, `y`, `width`, `height` for clients), each value what it must equal or an array of
  values of which one must. Its membership follows property changes. It is not tracked until it has a listener.
- **Listeners.** `ClientListener` (`onTitleChanged`, `onFocusChanged`, `onGeometryChanged`, `onClosed`, `onReady`)
  and `CollectionListener` (`onAdded`, `onRemoved`) are plain base classes; a listener may implement any of the
  methods, with or without the base class. Argument 1 is the origin object; `onAdded` and `onRemoved` also get the
  collection. On a collection, a client listener hears every member and stops hearing a member that leaves. On a
  client, it hears that client.
- **Replay on attach (decision for spec-review 1.25).** `collection.addListener(l)` calls `l.onAdded(member,
collection)` at once for each member already in the collection, in collection order, so a config that loads or
  reloads against existing windows sees them. Member events (`ClientListener`) are not replayed: nothing happened.
  Adding the same listener twice is a no-op.
- **When an event fires.** Change events are raised from what the compositor published, compared with what the
  listeners were last told, so a batch that sets a title to its current value raises nothing, two batches in one
  frame raise one event with the last value, and the echo of the script's own write raises an event like any
  other change. A window made in a frame is born with its final values and raises no change event. A node that
  was there at boot raises no `onAdded`. Order per client: title, focus, geometry; then membership changes.
- **Errors and budgets.** A listener runs under its own budget (`entry_budget`, 100 ms), the frame's dispatch under
  `frame_budget` (500 ms); queued calls that do not fit wait for the next frame. A throw is caught in the module,
  returned as a diagnostic with the `.ts` location, and reported once per site (listener class, method and
  position); the next listener still runs. A budget or memory trip stops that one call, is reported, and counts a
  strike (`JsShell::strikes`); the next call still runs. What to do about strikes is the supervisor's.
- **`gnoblin.spawn(command)`.** A string is a command line, an array is an argv. The shell does not start
  processes: `JsShell::set_spawner` takes the host's callback (`JsSpawnRequest::Command` or `Argv`), and a refusal
  or a missing spawner throws `GnoblinError` `spawn-failed`. A top-level `spawn` in `init.ts` runs on every load.
- **Not done.** Pointer and key events addressed to nodes and call answers from the compositor are consumed and
  dropped; claimed chords and gestures go to the input module (see "P3c status"). `JsShell` owns its generation:
  `swap_generation` and the staging hook that wire it into `JsHost` staging are "P6 role wiring" (it calls
  `JsShell::retire` before the swap). The compositor-side `where` evaluation is not built.

## P3c status

Key bindings, touchpad gestures and pointer drag bindings: `data/js/gnoblin/bindings.ts` (the grammar and the binding
classes), `data/js/gnoblin/input.ts` (`gnoblin.input` and the matcher) and `crates/gnoblin-js/src/input.rs` (the host
functions: seat claims and the seat's published state). The tests are `crates/gnoblin-js/tests/input.rs`, over a fake
tree link that records claims and plays back chords, gestures, pointer positions and window deaths.

- **Trigger grammar (spec-review 1.22).** `parseTrigger` is the only parser, in TypeScript so a binding rejects a bad
  spelling when it is built, with or without a host. A trigger is modifiers joined with `+`, then one key, mouse
  button (`MouseLeft`, `MouseMiddle`, `MouseRight`, with an optional `Drag` or `Click`), wheel, or a gesture alone
  (`Swipe3`, `Swipe3Left`, `Pinch4In`: 2 to 5 fingers, optional direction). Modifier names are case-insensitive:
  `Shift`, `Ctrl` (`Control`, `Primary`), `Alt` (`Mod1`), `Meta`, `Super` (`Logo`), `Hyper`; `Mod2` to `Mod5` are
  refused as the compositor refuses them. A key is a keysym name, never the character it types, so `Super+1` and
  `Super+Shift+1` are two triggers and `Super+!` is an error. The canonical spelling orders the modifiers
  `Shift, Ctrl, Alt, Meta, Super, Hyper`, writes a single letter in lower case and the common key names one way
  (`Enter` is `Return`, `pagedown` is `Page_Down`), so two spellings of one trigger are one trigger. A lone `Super`
  is a tap. A literal plus is `plus`. There are no sequences. An error names the offending token. The compositor's
  own chord spelling (`<Shift><Super>1`) is derived from the canonical one (`claimOf`), and is the only place that
  knows it. Key names are not checked against the keymap: the compositor decides, and drops a claim it cannot parse.
  The declaration file types the trigger as template literal unions.
- **Claims.** `InputContext.bind(trigger, fn | Binding)` returns the binding; `binding.unbind()` (or
  `bindings.remove(id)`) takes it away. Each distinct chord is one `Seat.claim` (pointer chords with `grab`, so the
  release comes back; a drag and a click of one button share the claim), each gesture kind and finger count one
  `Seat.claim_gesture`, and a binding for one device claims with that device's name. Ids are allocated per shell,
  from 1, never reused. A claim is released with the last binding that needs it, and every claim on `retire`. Claims
  are queued writes, so they travel with the frame's transaction. A function bound to a drag, or to a wheel (no
  compositor claim exists for it), throws at `bind`.
- **Frame loop.** The compositor's `chord`, `pointer_chord` and `gesture` signals are parked by `JsShell::frame` and
  handed to the input module after the listeners are planned. The module only schedules: each hook runs as its own
  call under the per-call budget (`entry_budget`), a throw is reported once per site and never stops the others, and
  calls that do not fit the frame's budget wait for the next frame. A pointer drag is read from the `pointer_chord`
  messages (the position travels in them; the seat's `pointer_x` and `pointer_y` are not sampled), and at most one
  sample waits behind the scripts, which read the latest position when they run (latest wins). Gesture updates merge
  the same way. `onStart`, `onEnd` and `onCancel` are never merged or dropped.
- **Drags.** `start` records the press place and the client under it (`InputContext.hitTest`: the topmost client in
  tree order whose bounds hold the point and that is not minimised; the real tree's order was not proved against
  overlapping windows, see below). Once an `update` is `threshold` (default 4) pixels from the press, the candidates
  are asked `onStart` in precedence order; the first to return a state object owns the drag, and its `event.position`
  is the press position, so a binding that measures from `position` measures from the press. `end` is the release: a
  drag that has started gets `onEnd` at the end place, an `end` with no drag is a click (`onPress`), and a pointer that
  went past the threshold and was declined is neither. The compositor's `cancel` calls `onCancel`. A drag event
  carries `x`, `y` (the same as `position`), `dx`, `dy` (from the press), `button` (X11 numbering), `modifiers`, the
  `context`, the `client` under the press, and `workArea` and `dragConstraint` (the primary screen's rectangle and the
  seat's `drag_constraint`). State lives in the gesture, per context, never on the binding. A client dying mid-drag, the
  binding being removed or given another trigger, a second `start` on the same claim, an `onUpdate` that throws (that
  drag alone: it is reported once and stops, the other drags run on) and `JsShell::retire` each call `onCancel`
  once. The compositor says nothing of a drag whose claim goes, so the module cancels the drags of a binding it
  releases itself.
- **Gestures.** A directed gesture (`Swipe3Left`) starts when its direction is known (12 touchpad units of centroid
  movement, 0.1 of scale for a pinch); an undirected one starts at the begin. `event.progress` is 0 to 1 along the
  binding's own direction (400 units for a swipe, 0.5 of scale for a pinch). A function bound to a gesture runs once
  when the gesture ends past 48 units (0.12 of scale).
- **Precedence (spec-review 1.21).** Device, then layout, then context, with `priority` ordering only inside a tier; a
  binding that declines (`onStart` returns `false`, `onPress` returns `false`) lets the next candidate have the input.
  Registration refuses two bindings of one collection that match one trigger at one priority
  (`BindingConflictError`, naming both ids); a layout binding and a context binding for one trigger are not a conflict,
  the layout's wins for its clients. A claim for one device shadows the all-devices claim in the compositor, so the
  context's and layouts' bindings for that trigger are candidates of the device's claim too.
- **Stock bindings.** `ClientMoveBinding` (Super+MouseLeft+Drag) and `ClientResizeBinding` (Super+MouseRight+Drag,
  `edges: "quadrant"` by default, the corner picked from the initial pointer position and never changed, a minimum of
  100 by 80 held against the opposite corner, `client.minSize` and `client.maxSize` when the client has them; the tree
  carries no size limits today, so they are absent) are TypeScript over `client.bounds` and `event.position`; they
  write `floatingBounds` on release and restore the start on cancel. A fullscreen or maximised client declines both:
  `onStart` returns `false`, and the compositor has already withheld the press from the client and the shell's nodes,
  so the press is consumed and ignored (the shell cannot hand a withheld press back, so there is no better answer; a
  maximised window is not floated first as the Lua shell's drag did). A move keeps the window where the seat's
  `drag_constraint` allows (`dragConstraintMode`, `clampAxis`, `constrainRect` in `gnoblin/bindings`, ported from
  `data/lua/gnoblin/shell/drag.lua` with its test cases): `work_area` keeps the window inside the primary screen (a
  window at least as large as the screen is pinned to its origin), and `minimum_strip` lets it leave as long as 75
  pixels stay inside. The work area is the screen, since the shell has no bars of its own yet. `FloatingLayout`
  registers both as `move` and `resize` (`undefined` keeps the default trigger, `null` leaves the binding out,
  spec-review 1.19). Not written: the `snap` option, `edges: "nearest"`, `ClientSwapBinding`, `SplitResizeBinding`,
  `WorkspaceSwitchBinding`.
- **Layouts.** The layout of every workspace in `gnoblin.workspaces` is attached to every input context while a
  workspace uses it (checked once per frame): its bindings are live for the clients whose workspace's `layout` it is,
  they follow the layout's `bindings` collection, and they go, cancelling a drag they run, when the workspace takes
  another layout or leaves the collection. A layout whose bindings the seat refuses is reported once and left
  unattached. `InputContext.attachLayout(layout, owns?)` and `detachLayout` do the same by hand for a layout no
  workspace holds, with the client test `owns` (a layout attached by hand is never detached for the workspaces).
- **Contexts and devices.** One `InputContext` per seat node; `focusedClient` and `pointer` read through to the seat,
  `activeDisplay` is `null`. `devices` is parsed from `Seat.device`; a device's `bind` claims for its name. A context
  or device the seat loses is disposed and its drags cancelled. `new InputContext` is not available: contexts are the
  compositor's seats.
- **Proved against the real compositor.** `crates/gnoblin/tests/js_shell.rs`, `super_drags_move_and_resize_a_real_window_through_the_pointer_chord_stream`
  and `a_super_drag_takes_the_window_on_top_where_two_overlap`: a real session, a real Wayland window, an `init.ts`
  with a floating workspace and the default `FloatingLayout`, and Super, pointer motion and Button1 and Button3 injected
  through the session's input tap (the seat path a device's events take). Super+Button1 moves the window by the pointer's
  delta, Super+Button3 resizes from the corner nearest the press and holds the same corner when the pointer crosses the
  window, down to the 100 by 80 minimum, all read back from the compositor's scene. This also proves that the pointer's
  space is the space of client `x` and `y` (a press at a window's local point hits it), and that tree order is stacking
  order for `hitTest` (of two windows at one place the one mapped last takes the press). The claims reach the seat before
  the first press only after the shell's first frames, so the test repeats the gesture until a press is claimed.
- **Proved only against the fake link.** The decline of a maximised client (the `maximised` flag is not a `Client` member
  until the UI lane's `Client.maximised` lands; the bindings read it when it is there), the drag constraints against a
  seat that publishes `drag_constraint`, a throwing `onUpdate`, and `cancel`. Key chord and gesture claims use the calls
  the Lua shell makes, byte for byte, but were not run against the compositor. `Wheel` has no claim. A minimised
  window is skipped by `hitTest` only once `Client.minimised` exists, and `visible` is not read.

## Pointer chord claims

What the compositor offers a shell that wants to move or resize a window by dragging with JavaScript only (package
P3c, "Drags"). `crates/gnoblin-wayland/src/pointer_chord.rs` implements it and does not know any scripting language.

**The claim.** The same call as a key chord, on the seat node: `Seat.claim(id, chord, grab, device)`. The call's
arguments are the little-endian `i64`s `id` and `grab`, and its text is the chord, or the device's name, a line feed and
the chord. `claim` in `crates/gnoblin-js/src/input.rs` already makes exactly this call, so `input.claim(seat, chord,
grab, device)` with a pointer chord is the whole API. `grab` is ignored for a pointer chord: the release always comes
back. The compositor reads a chord that ends in a button name as a pointer chord and any other as a key chord.

- The chord is modifiers in angle brackets and then `Button1` to `Button9` (`<Super>Button1`,
  `<Control><Alt>Button2`), or `Button8` or `Button9` alone. The numbers are X11's: 1 left, 2 middle, 3 right, 8 back, 9
  forward, so `MouseLeft`, `MouseMiddle` and `MouseRight` are 1, 2 and 3. `Button1` to `Button7` need a modifier, and a
  claim without one is dropped. `claimOf` already writes the modifiers in brackets, in the compositor's order.
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

**What the JS side does with it** (`crates/gnoblin-js/src/input.rs`, `data/js/gnoblin/input.ts`). `JsShell::frame`
parks each `TreeIncoming::PointerChord` in `JsInput::inbox` beside `Chord` and `Gesture`, and hands the frame's batch
to the input module as `{ kind: "pointer", claim, phase, x, y, dx, dy, button, modifiers }`. `start` is the press, each
`update` a drag sample, `end` the release and `cancel` the `onCancel` ("Drags" in "P3c status"). The pointer's place
comes from the message, not from the seat's `pointer_x` and `pointer_y`. A pointer claim gets no `chord` message.

## P3b status

Workspaces and the layout driver (`crates/gnoblin-js/js/gnoblin.ts`, `data/js/gnoblin/workspace.ts`,
`crates/gnoblin-js/src/workspaces.rs`, tests in `crates/gnoblin-js/tests/workspaces.rs`).

- **A workspace is the script's.** `new Workspace({ id, name?, layout?, gaps? })` (`layout` is a `Layout` or a stock
  name; default floating) and `gnoblin.workspaces.add(ws)`. The compositor's own `workspace` tree node is no longer
  a member of anything, and the tree has no property that says which workspace a window is on, so membership lives in
  the JS shell. `gnoblin.workspaces` is a `MutableCollection<Workspace>` (`add`, `remove(ws | id)`, `clear`,
  `show(ws | id)`, plus the read API); a second workspace with an id already there throws `GnoblinError`
  `duplicate`. A workspace has `id`, `name`, `index`, `active`, `layout`, `gaps`, `clients`, `show()`; assigning
  `layout` or `gaps` marks it dirty.
- **`ws.clients`** is a live `MutableCollection<Client>` of the same `Client` objects as `gnoblin.clients`. `add(client)`
  transfers primary membership (`client.workspace` updates, the old workspace loses it, `onAdded`/`onRemoved` go to
  collection listeners). `remove` leaves the client with `workspace === null`. Removing a workspace moves its clients
  to the first workspace left. Nothing closes a client: `alive` stays true.
- **Initial workspace rule (question 1, simplified).** One function, `initialWorkspace` in `workspace.ts`: the first
  workspace flagged active, else the first. A new toplevel joins it. Active is a single flag for now (the first
  workspace added, or the last `show`n), standing in for "the workspace the primary display shows". Windows that were
  there at boot join it on the first frame, not while the entry loads, so a reload can put them back first.
  `defineConfig({ workspaces })` calls `gnoblin.__register`, which makes real `Workspace` objects; a config that
  declares none gets `desktop` (floating) when its entry has loaded.
- **Layout driver.** At the end of every frame, after all listeners, `entry.layout()` runs. For each workspace it
  diffs the layout's own enumerable fields against the last snapshot (J6: `Object.is` per key, so an edit in place is
  not seen and a `#private` field is not a field), the gaps, and the area (the primary screen's `width` and `height`
  from the tree's `screen` node; no screen, no layout). The layout's `onClientRemoved` and `onClientAdded` hear of the
  clients that left and arrived, and a dirty workspace that is the active one calls `layout.compute({ workspace,
children, area, gap, focused })` once and writes the placements that differ from the client's values (x, y, width,
  height; `visible` and `tiled` have no tree property yet). Placements are validated before any is written. A workspace
  that is not active stays dirty until it is shown. Windows of other workspaces are not hidden: the tree has no
  visibility property to write.
- **Floating.** `FloatingLayout.compute` places nothing; its `onClientAdded` places a window once on arrival (a
  window moved to the workspace by `clients.add`, by removal of another workspace, or created while the shell runs).
  A user-moved window is left alone because no later frame calls the layout for it.
- **Coalescing.** `send_transaction` keeps the last instant `Set` of each (node, property) in a transaction, so a
  handler's repeated writes and the layout's are one op (`workspaces.rs: coalesce_sets`). Animated writes are not
  merged.
- **Reload.** `JsShell::workspace_snapshot()` returns `JsWorkspaceSnapshot` (workspace ids in order, each with its
  client node ids, and the active id). After the new generation's entry has loaded, `JsShell::reconcile(&previous)`
  puts each client back on the workspace of the same id without offering it to the layout, so a floating window keeps
  its place and no second `desktop` appears; a client whose workspace is gone moves to the first workspace and is
  placed as an arrival. The active workspace is kept when it still exists. The tests run two shells in turn over one
  fake link. `JsShell::swap_generation` does the same over one shell ("P6 role wiring").
- **Not done.** Per-display workspaces (`display.viewport.content`), `where` queries on a `Workspace` beyond its
  scalar properties, a `visible` write, and hooks on `Layout` for the binding lane.

## P6 role wiring

`JsRole` (`crates/gnoblin-js/src/role.rs`) is what `ShellRole` is for Lua: the shell the runtime role runs, one frame per
turn of its loop. It is tested against a real compositor session in `crates/gnoblin/tests/js_shell.rs`; a fake
compositor stands in only for the unit tests of `swap_generation` (`crates/gnoblin-js/tests/shell.rs`).

- **Selection.** `GNOBLIN_SHELL_VM=js` runs the JavaScript shell and `GNOBLIN_SHELL_VM=js:<dir>` reads the stock
  `gnoblin` modules from `<dir>` (the layout of `data/js/gnoblin`; a file that is not there is the compiled-in one).
  Unset, `0` or a directory are what they were: the Lua shell VM. The Lua host still loads `init.lua` and commits the
  configuration, so only the shell changes. The user's entry is `init.ts` in `$XDG_CONFIG_HOME/gnoblin` (else
  `$HOME/.config/gnoblin`), falling back to `init.js`. With neither file the shell runs bare and loads `init.ts` when it
  appears.
- **Wiring without a cycle.** `gnoblin-js` and `gnoblin-lua` are siblings and neither names the other. `gnoblin-lua`
  holds a `ShellFrame` trait (`tick(now)`), `ShellSlot::from_env_or_js` and `RoleRunner::run_with_js`; the `gnoblin`
  binary, which already names both ends, wraps a `JsRole` in a `ShellFrame` and passes the launcher. A build that
  passes none refuses `js` with one warning and serves without a shell. `xtask deps` lists `gnoblin-js` among the
  binary's role entry crates for this.
- **The loop.** `JsRole::connect` reads the socket by the `GNOBLIN_IPC_SOCKET` rule, handshakes as a client at this
  build's version, and hands the handshaken socket to a `SocketTreeLink`. Each `tick` acts on a reload request, runs
  `JsShell::frame`, and then acts on strikes. A link failure ends the shell: `tick` returns it, the slot empties and the
  role keeps serving, as for Lua. There is no second connection for slots and dialogs yet: they belong to the dialogs
  package. `gnoblin.spawn` starts the program in a process group of its own with no terminal and reaps it.
- **Reload.** The role owns a `ConfigWatcher` (100 ms debounce), a `JsReloader` and a `JsSupervisor`. The reloader
  stages the entry in a generation of its own (`JsHost::set_prepare` gives it the shell's host objects over the shell's
  tree mirror, `JsStaging::prepare`), and the commit closure keeps a second handle on it (`JsGeneration::share`).
  When the reload is accepted the role calls `JsShell::swap_generation`: it reads the old generation's
  `workspace_snapshot`, `retire`s it (drags cancelled, seat claims released, in the transaction that carries the new
  generation's own claims), drops it with its listeners and queued calls, runs `finishLoad` on the new one and
  `reconcile`s the windows back onto the workspaces of the same id. The new generation has already told its
  listeners of the windows that exist (replay on `addListener`) and is announced `ready` for every window on the
  next frame. Windows are nodes of the compositor's
  tree, so their ids do not change and nothing is re-created or moved. `JsShell::begin_stage` and `abandon_stage` take
  back a write the staged entry made at its top level (`client.x = 5`) when the stage fails after it.
- **Failure.** A syntax error, a throw, a budget trip while loading or a stage that is rejected keeps the running
  generation and logs one warning, `the running generation stays: <file>:<line>:<column>: <message>`, in the
  author's `.ts` coordinates. The file the error named stays watched, so the edit that fixes it is seen.
- **Runaway handlers.** A listener call that runs past `entry_budget` (100 ms) is interrupted, reported, and counts a
  strike on the shell. The role then asks the supervisor: it restages the generation from disk, and three strikes in
  30 s latch `Degraded`, after which the last generation keeps running and a deliberate reload (a save, or
  `JsRole::reload_now`) clears the strikes. The shell is a process of its own, so the compositor never waits for a
  script: the test runs the role in a thread, spins a handler for ever, reads the scene through an observer connection
  while it spins, and maps one more window afterwards.
- **Proven against the real compositor** (`crates/gnoblin/tests/js_shell.rs`; slice criteria 1, 3, 4 and 5): `init.ts`
  with annotations and an enum importing `./slots.ts` places two mapped windows; a rewrite reloads with the same window
  ids and new places; a syntax error leaves the old behaviour in force (a window mapped afterwards is placed by it) and
  is logged once as `init.ts:3:<col>`; fixing the file recovers; a `while (true)` in `onAdded` is interrupted and the
  generation restarts (the restaged one reads the mark the first wrote and does not hang), the scene answers an
  observer meanwhile and still updates; a handler that hangs in every generation (it hangs the restage too, while it
  loads) latches `Degraded` after three strikes and the last generation keeps placing windows; a shell with no
  `init.ts` runs bare and loads the file when it appears. Not proven here: slice criterion 6
  (killing the process and the Rust fallback), which needs the guardian and the fallback shell.
- **What the real tree changed in the binding.** The fake compositor's ids are small and its windows hang under the
  root; the real tree's are not. (1) The compositor's own nodes have ids with bit 63 set, which a script number
  cannot hold, so a node crosses into script as a number from `NodeHandles` (`bindings/mod.rs`): an id below
  2^53 - 2^24 is its own number, any other gets a stable handle in the band above, for the life of the shell, so a
  reloaded generation sees the same number for the same window. `client.id` and the error targets print the real
  decimal id (`host.label`), the one `gnoblinctl` prints. (2) The compositor makes a window with no parent and the shell
  parents it, so the collections are built from every node of the mirror (`TreeMirror::ids`), the root's tree first,
  and not from a walk of the root.
- **Protocol facts the real compositor showed.** It sends no change message for a transaction that changed no node (a
  call-only one still advances its serial), so `JsShell` moves the mirror to the serial of an applied reply
  (`TreeMirror::advance_serial`) and opens the in-flight gate for an applied transaction that published nothing;
  without it the next batch is a `Gap` or every later write waits for ever. The stock floating layout places an
  arrival after the listeners have run, so a config that wants its own listener to place windows declares a workspace
  with the base `Layout`, as the tests do.
- **Not done.** The second owner connection for slots and dialogs, input events and chords (the binding lane), the
  `gnoblinctl reload` request path into `JsRole::reload_now`, and the stock config plan applied as a runtime (the
  shell with no `init.ts` runs bare, not as `defaultConfig`).

## P7 UI toolkit status

The shell UI toolkit (lane JsUi): `Canvas`, `Box`, `Text`, `Icon`, `Grid` (`data/js/gnoblin/ui.ts`) and the stock
`Titlebar` and `Decorations` (`data/js/gnoblin/titlebar.ts`), over the host functions of `crates/gnoblin-js/src/ui.rs`
(`__gnoblinHost.ui`). User page: `docs/js-ui.md`. Ordinary ES classes and fields: no class registry, no bound
properties.

- **Frame.** After the layouts, `JsShell::frame` calls the module's `ui` entry (`runUiFrame`): it takes the answers to
  measurements and the pointer events parked for the toolkit, then updates every canvas tree that has no canvas parent:
  `refresh()` (read live state), `arrange()` (a `Box` row or column, or a `Grid`), then the node's geometry and paint
  list are written when they differ from the last write. A change is therefore one frame of latency and a frame that
  changed nothing sends nothing (`nothing_is_written_again_for_a_frame_that_changed_nothing`). The Lua shell
  re-evaluated a binding when a property it read changed; the polling here costs one `draw()` per canvas per frame and
  needs no change tracking.
- **Nodes.** A canvas makes its node when it is constructed (`ui.create`: `New` in the frame's transaction, the id made
  from the attach epoch like `gnoblin-lua`'s) and is destroyed with `destroy()`. A script can write only nodes the
  toolkit made (`not-owned` otherwise); the geometry keys are `x`, `y`, `width`, `height`, `opacity` and the flags
  `visible`, `input`, `clip`, and `content` is the paint list. When the compositor destroys a window its canvases go
  with it, and the frame's destroyed list tells the toolkit (`uiNodeGone`); a reload's `retire` destroys the old
  generation's canvases (`cancelAll` calls `uiReleaseAll`).
- **Paint list.** `encodePaintList` writes the bytes the Lua encoder wrote (`"GNPL"`, version, op count, tag and fields,
  little endian). The ops are TypeScript objects (`{ op: "rect", x, y, width, height, radius, fill, stroke,
strokeWidth }`, `line`, `text`, `icon`, `pushClip`, `popClip`, `pushOpacity`, `popOpacity`), not the Lua tables.
- **Text.** `Text` asks the compositor to measure its font, size and text (`ui.measure` is the `measure` call of the
  canvas class: the size as the bits of an `f64`, the text `font\ntext`). `JsShell` keeps the answers of its own tokens
  (`TreeIncoming::Answer`) for the toolkit; any other answer is still dropped. Until the answer a text is sized by the
  Lua estimate, and a late answer for text that has since changed is ignored.
- **Titlebar.** A `Box` row of a drag area (holding the title), minimise, maximise and close buttons, mounted under the
  window's node (decision 2a). Every number and colour is a field of `TitlebarStyle`. `defineConfig({ decorations: true
})` attaches a `Decorations` listener to `gnoblin.clients`: one titlebar per client, there at attach or arriving
  later. `defaultConfig` sets the key; a config without it has no titlebars, so a config that does not ask is unchanged.
  The tree has no server-side decoration flag a window could be asked for (a window's decoration mode is always
  client-side today, `zxdg_decoration_v1.rs`), so every client is decorated. `Client` gains `maximised` and
  `minimised` (writable) for the buttons.
- **Proven against the fake link** (`crates/gnoblin-js/tests/ui.rs`, 24 tests): row, column, grow, nesting and grid
  rectangles (derived from `Box:arrange`; the Lua corpus has no layout case); the measure call's node, size and text; the
  estimate, the answer, a late answer and a refused measure; the Lua test's `GNPL` bytes for a box, a text, an icon and a
  squeezed text; one list of every op equal to `gnoblin_paint::encode` and decoded by `gnoblin_paint::decode`; a bad op
  named and the content left alone; the titlebar's geometry and colours from its style; the buttons, drag and double
  click through pointer events; a window gets exactly one titlebar, it follows the title, and it is gone with the
  window; one bar per window including those at boot; no titlebar without the key; the close button closes the window;
  `retire` destroys the bars.
- **Proven against the real compositor** (`crates/gnoblin/tests/js_shell.rs`,
  `a_script_draws_a_box_and_a_titlebar_and_the_compositor_holds_what_it_painted`): a script's `Box` is a canvas node of
  120 by 40 whose `content` the compositor accepted and the observer reads back, and a mapped window has exactly one
  titlebar canvas under it with its title and the close icon painted in the tree. A pixel is read by drawing the
  decoded list with `gnoblin-paint`'s painter (the one the compositor links), not from the compositor's output: a
  `View` capture was not tried. Real pointer events on a titlebar are not exercised, so the drag is proven against the
  fake link only.
- **Not done.** The right and middle click gestures and the fade on close of `decorations.lua`; the titlebar reserves no
  room above the client (it draws over the top, as the Lua one did); `Bar`, `Clock` and `Entry`; declarations for the
  toolkit in `gnoblin-stock.d.ts`.

## Questions for the owner (the agent has a default for each)

1. Initial workspace rule for a new toplevel. Default: the workspace shown by the display under the
   pointer.
2. Inherited style lists: copy on write when an element is mutated. Default: yes.
3. `where()` queries are data only, so Rust can evaluate them. Default: yes.
4. Trigger grammar. Default: the existing Lua shortcut grammar, typed as template literal types.
5. Port `prompts.lua`? Default: no. Rust draws consent. See `popups-portals.md` section 8 for the
   other eleven questions.
6. Window-rule patterns: `gnoblin-config` depends on `gnoblin-lua-pattern` today. Default: plain
   globs and regular expressions in `rules`.
