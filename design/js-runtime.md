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

Without any `init.ts`, the stock library runs: the four floating workspaces `1` to `4` of the Lua stock config
(`data/default-config/config/10-windows.lua`), one display per output, and the stock key bindings of the Lua shell.
`Super+1` to `Super+9` show the workspace at that position, so `Super+1` to `Super+4` act and the rest wait for more
workspaces. A script that never calls `defineConfig` is a program with no keys: it gets one floating `desktop`. The stock library is itself TypeScript,
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
| P8 later, separate sizes | styles and effect stack, Display mirror and span, transition planner                                                                | P4            |
| P9 XR                    | `gnoblin.extensions.xr` and `gnoblin/extensions/xr` over the out-of-process XR program (J7): room, ring, View, bindings             | P4            |

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
spec-review.md section 4 as a plain-data plan. `defaultConfig` is four floating workspaces (`1` to `4`), the keys of the Lua
shell (`Super+1` to `Super+9`, `Super+Left` and `Super+Right`, the `Swipe3` slide; see "P4b status") and the media keys
of `data/default-config`.

`Binding` and `BindingCollection` are wired by the input module (see "P3c status"): the `triggers` option of
`FloatingLayout` registers its `move` and `resize` bindings, which go live when an input context attaches the layout.
The swap and split-resize drag behaviours (`MasterStackLayout`, `GridLayout`) are not written; `style` and `outputs`
are stored and have no native effect; `rules` and `focus` are applied (see "Focus, rules and snapping status"); `on` and `setup` are in the plan for the host to apply. The layouts and
keys that were not ported here (`Dwindle`, `Strip`, `Super+space`, `Super+Left` and `Super+Right`, the `Swipe3` slide)
are in "P4b status".

### P4b status

The remaining stock layouts, workspace sets, the slide gesture and the launcher key (lane JsStockLayouts). Tests:
`crates/gnoblin-js/tests/stock_layouts2.rs` (pure, in a real generation with a fake host), `workspaces.rs` and `input.rs`
(the layout driver and the gesture over a fake compositor) and `crates/gnoblin/tests/js_shell.rs` (a real session).

- **Layouts** (`data/js/gnoblin/layouts/`). `DwindleLayout` (the split tree is state the layout keeps between
  computes, so one instance serves one workspace; a new node splits the cell of the node that had focus when it
  arrived, which is the focus the layout saw last, because the compositor has focused the arrival by the time the
  layout runs; every walk is a loop, because a tree of 200 windows that each split the last is 200 deep and the
  engine's call stack is short), `KioskLayout` (places as `MonocleLayout`, as the Lua class did), `StripLayout`
  (`column`, `pin`, `scrollTo(offset)`, `focusLeft(workspace)`, `focusRight(workspace)`; the layout keeps its own
  column order because a new node opens after the focused column; the focused column's neighbour takes focus when it
  closes; rectangles are not floored and nodes are not marked tiled, as in Lua) and `Carousel3dLayout` with the pure
  `ring(count, focus, area, options)`. The Lua tests and goldens are ported case for case
  (`tiling_test.lua` Dwindle and Kiosk, `strip_test.lua`); the cases the Lua corpus lacks (a Dwindle gap, closing,
  focus insertion, a strip's `column`, area offset and a fractional width) are marked "derived" in the test file.
- **Carousel3d and depth.** The Lua class wrote only `x`, `y`, `width` and `height`. A `Placement` now also carries
  `visible`, `tiled`, `z`, `rotation` and `scale`, and the Carousel writes the ring's depth as `z` and its angle as
  `rotation` about the vertical axis (radians, `[0, angle, 0]`), which the node schema has. The projected rectangle is
  still written, so a renderer that projects depth itself would draw perspective twice: the sign of the rotation and
  that interplay were not seen on a renderer. The node schema has no `transform` object, so the flat properties are
  written. The ring's depth fade is not applied (opacity belongs to the rules, as in Lua).
- **The layout driver writes more.** `Client` gains `tiled`, `visible`, `opacity`, `z`, `rotation` and `scale`
  (the host table in `bindings/props.rs`, which now has defaults per property and a list kind) and `animate(values,
{ duration, easing })`, a write that carries `seconds` and 33 samples of `ease.out`, `out-cubic`, `in-cubic`,
  `in-out` or `linear`, as the Lua `node:to` sends them (`host.animate`). After each compute the driver writes `tiled`
  (a client the layout gave no placement is untiled), the depth properties (reset when a layout stops using them) and
  the layout's `visible`; the shown workspace's windows are drawn and the others hidden (`syncVisibility`), which is
  what the Lua shell got from hiding the workspace node. A change of the focused client now recomputes the active
  workspace, so `MonocleLayout`, `KioskLayout`, `StripLayout` and `Carousel3dLayout` follow focus.
- **`show` and slides.** `gnoblin.workspaces.show(ws, options)` and `gnoblin.show` take `{ duration, progress }`: with
  a duration the new workspace's windows start a screen's width away and animate to where the layout put them (`x`,
  out-cubic), the old workspace's windows leave the other way and stay drawn until the slide ends, then return to
  their rest place hidden (`gnoblin.after(seconds, fn)` runs at the first layout pass after the time). `progress`
  carries on a slide a gesture has already made. The stock keys slide for `SLIDE_SECONDS` (0.25, as `Desk:show`);
  `show(ws)` with no options switches at once. A window is moved by its own `x`, so a floating window's rest place is
  its `x` when the slide starts.
- **The slide gesture.** `WorkspaceSlideBinding` (`gnoblin/gestures`, a `GestureBinding` on `Swipe3`) is
  `Gestures.workspace_slide`: windows follow the fingers (`travel` 400 units for a screen), fingers moving right bring
  in the previous workspace and left the next, lifting after `commit` (0.25) with the motion more horizontal than
  vertical finishes with `0.25 * (1 - progress)` seconds left, anything else slides back over 0.2 s (`out-cubic`) and
  hides the incoming workspace, a change of direction resets, a workspace switched under the fingers ends the slide.
  `defaultConfig` binds it with `binding(() => new WorkspaceSlideBinding())`, a new key action (`kind: "binding"`)
  that the host binds whole. The Lua shell claimed the undirected `Swipe3`, so one binding covers both directions.
- **Workspace sets** (`gnoblin/sets`, `gnoblin.workspaces.sets`). `defineSets` and `activateSet` are the pure functions of
  `sets.lua` with its results and errors (`sets_test.lua` ported); `WorkspaceSets` keeps the list in force:
  `define(list)` throws `WorkspaceSetError` and leaves the sets in force when a list is refused, `activate(name)`
  shows each entry's workspace, a missing output falls back to the primary with a report, a live set again changes
  nothing. A workspace for an output that is not the primary one is reported and not shown: workspaces are per
  shell, not per output, so per-display workspaces are not ported.
- **Keys.** `defaultConfig` binds `Super+1` to `Super+9` (`show.position(n)`: the workspace at that position, whatever
  its id, nothing when there is none, as `desk().space.children[i]`), `Super+Left` and `Super+Right`
  (`focusColumn`: the shown workspace's `StripLayout` moves focus one column, other layouts do nothing) and `Swipe3`.
  `launcher` is a new config key, a command line or an argv: `Super+space` runs it through `spawn` (a `keys` entry for
  `Super+space` wins). There is no default command, because the repository names no stock launcher program.
- **Proved against the real compositor** (`crates/gnoblin/tests/js_shell.rs`): two real Wayland windows on a
  `DwindleLayout` workspace sit at the two halves of the screen, and on a `StripLayout` workspace in two columns, as
  the compositor's tree holds them.
- **Not ported.** The in-shell launcher (`launcher.lua` is an `Entry` widget that spawns the typed text; only the
  configured-command key is done), reduced motion (`seat.reduced_motion` makes `motion()` zero in Lua; the slides here
  always animate), the four-finger and other gesture actions of `gestures.lua` (`Gestures.on_swipe`, `on_pinch` and
  the five actions: `workspace_next`, `workspace_previous`, `window_close`, `window_minimise`,
  `window_toggle_maximise`), the `Desk` (bars, `Desk:remove_workspace`, `sync_fullscreen`), per-output workspaces and
  sets for outputs other than the primary one, and the `Window:on_closed` focus hand-off for layouts other than
  `StripLayout`. A slide moves windows by `x`, so a window that is moved while a slide runs is moved back to where
  the slide recorded.

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

### Seat claims across a reload

Lane JsReloadClaims. The rule: **a reload releases the old generation's claims and takes the new generation's in one
transaction, releases first.** Nothing the shell holds on the compositor may be refused for being held, and none may be
held by nobody after the swap.

What went wrong. A reload stages the new generation while the old one still runs, so the new generation's entry made its
claims (`bind` calls `Seat.claim`, a gesture `claim_gesture`, a layout's drags pointer chords, `bindingOnly`
`claim_device`) at stage time, queued in the shell's pending writes ahead of the old generation's `unclaim` calls that
`retire` queued at the swap. The compositor applies a transaction's calls in order, and a chord that another claim holds
is refused (`a seat claim's chord is already bound for that device`; for a gesture, `a seat gesture claim was refused`).
The first calls were refused because the old claims were still there, and the old claims were then released: the chord
was held by nobody. The live run logged 19 refusals and one refused gesture for the stock keys alone.

Designs considered, and why the third:

- _Diff by chord_ (keep an identical chord's claim id, swap the handler, unclaim only what went). It sends the least, but
  it needs the claims to be shared state between two generations that have nothing else in common, a refcount that the
  old generation's `unclaim` and the new one's `claim` both move, and still refuses a chord whose grab flag or device
  changed. The id space is shared with gestures, device claims and drags, so every kind needs its own equality.
- _A compositor `replace` verb_ changes the protocol and the schema for a problem that one transaction already solves.
- _Release first, claim behind it, in one transaction_ (chosen). It uses the order the compositor already keeps, needs no
  new verb, treats chords, gestures, pointer chords and device claims alike, and cannot refuse anything the new
  generation claims, because a generation's own claims never collide with each other (the input module shares one claim
  among the bindings of a chord).

How it is done (`crates/gnoblin-js/src/input.rs`, `shell.rs`):

- `JsShell::begin_stage` starts keeping claims. A claim made while staging gets its id at once (the script goes on) and
  is held in `JsInput::staged`, not sent. A claim the staged generation gives up again is dropped without a call.
  `abandon_stage` drops what was kept, so a failed stage leaves the compositor and the host's table as they were, and a
  claim of a failed stage is never mistaken for a held one.
- `swap_generation` ends the old generation (`cancelAll`, its drags' `onCancel`, its `unclaim` calls), releases any claim
  the old script did not (it threw first), then sends the kept claims, and sends the one transaction. A transaction still
  in flight holds all of it back together.
- Ids stay one number space per shell, from 1, never reused: a kept claim's id is allocated when it is made, so the new
  generation's ids are above the old generation's.
- **Slots.** The `dialogs.observe` claim is made on the shell's slot connection and is not part of the transaction.
  `JsDialogs::hand_over` answers the old generation's unanswered requests `pass` and keeps the slot claimed; the new
  generation's first frame keeps the claim if it has a dialog listener and releases it if it has none, so the
  compositor never offers a request to nobody in between. A shutdown still releases it (`retire`).
- The `idle` holds and the XR program's controller claims are not seat claims. A reload gives the first back by design
  and the XR program's are released by the generation's `retire` as before.

Proof. `crates/gnoblin-js/tests/input.rs` runs the role over a fake compositor that refuses a held chord, gesture or
device claim like the real one: chords kept, added and removed, a swipe, a layout's drags and a device across reloads,
five in a row; every release is sent before every claim; the compositor holds exactly the new generation's set when
the swap returns; and a stage that claims and then throws sends nothing. `crates/gnoblin/tests/js_shell.rs`,
`a_reload_hands_every_seat_claim_over_so_each_shortcut_fires_once_and_a_removed_one_is_released`, runs the stock shell in
a real compositor and presses every default chord (`Super+1` to `Super+9`, `Super+Left`, `Super+Right`, `Super+space`, the
seven media keys), the user's keys, a three-finger swipe and a `Super+Button1` drag on the generation it booted and
after each of six reloads (each adds a key and takes the last one's away; the last also takes `Super+y` away). Each
chord must fire exactly once, the client's keyboard must not hear a claimed key, a key the reload removed must reach the
client again and not fire, the drag must move the window, and neither the compositor's nor the shell's log may hold a
refused claim. A real client's keyboard is the witness of a release (`Mapper::keyboard`), since a claim that nobody
holds is a key that types. The generation's actions are replaced by counters in the test's `init.ts`: running them
would start `wpctl`, open the launcher and slide the window the test drags.

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
  spec-review 1.19). The `snap` option and `edges: "nearest"` are written (see "Focus, rules and snapping status").
  `ClientSwapBinding` and `SplitResizeBinding` are written (see "Tiling bindings status"). Not written:
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
- **Proved only against the fake link.** The decline of a maximised client, a minimised client being skipped by
  `hitTest` (`visible` is not read: the shell's `Client` has no such member), the drag constraints against a seat that
  publishes `drag_constraint`, a throwing `onUpdate`, and `cancel`. Key chord and gesture claims use the calls the Lua
  shell makes, byte for byte, but were not run against the compositor. `Wheel` has no claim.

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

- **Selection.** Superseded by "JS-owned runtime role": `GNOBLIN_SHELL_VM` unset or `js` runs the JavaScript role,
  `js:<dir>` reads the stock `gnoblin` modules from `<dir>` (the layout of `data/js/gnoblin`; a file that is not there
  is the compiled-in one), and `lua` runs the Lua role. The user's entry is the first of `init.ts`, `init.js` in
  `$XDG_CONFIG_HOME/gnoblin` (else `$HOME/.config/gnoblin`). With neither file the shell runs the stock default config
  (below) and loads `init.ts` when it appears.
- **Wiring without a cycle.** `gnoblin-js` and `gnoblin-lua` are siblings and neither names the other. The `gnoblin`
  binary, which already names both ends, asks `gnoblin_js::choose_runtime_from_environment` which role to run and
  calls `JsRoleRunner::run` or `RoleRunner::run`. `xtask deps` lists `gnoblin-js` among the binary's role entry crates
  for this. (Until the JS-owned role the Lua host ran the JavaScript shell through a `ShellFrame` trait; that trait,
  `ShellSlot::from_env_or_js` and `RoleRunner::run_with_js` are gone.)
- **The loop.** `JsRole::connect` reads the socket by the `GNOBLIN_IPC_SOCKET` rule, handshakes as a client at this
  build's version, and hands the handshaken socket to a `SocketTreeLink`. Each `tick` acts on a reload request, runs
  `JsShell::frame`, and then acts on strikes. A link failure ends the shell: `tick` returns it, the slot empties and the
  role keeps serving, as for Lua. Slots and the configuration document use two more owner connections the role opens
  (see "P6 config document and slots status"). `gnoblin.spawn` starts the program in a process group of its own with
  no terminal and reaps it.
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
- **Zero-config and layering.** `JsShell::load_default_config` (the `loadDefault` hook of the host module) registers
  `defaultConfig` alone, as a real runtime config: the `desktop` workspace with `FloatingLayout` (and its move and
  resize bindings) and the stock keys, each claimed on the first seat. The role runs it when there is no entry file
  and when the entry fails to load at startup, so a shell is never bare. `defineConfig` no longer registers only
  the workspaces: the host (`__register`) lays the user's config over `defaultConfig` with `mergeConfig`
  (`data/js/gnoblin/config.ts`), applies the merged workspaces, and binds the merged keys on the first seat
  (`spawn`, `show`, `"close"` and function actions; a tree with no seat has no keyboard, so the keys are left out). The merge
  rule: `workspaces` and `rules` replace as a whole (an absent or empty `workspaces` keeps the stock one); `keys`
  merge by canonical trigger, the user's replacing the stock key, and a stock key that shows a workspace the merged
  config does not declare is dropped; `gaps`, `style`, `outputs` and `on` merge by key; `setup` is the user's when
  given; `decorations` is not layered (a config that does not ask for titlebars has none, as `desugarConfig` says). An `init.ts` that never calls `defineConfig` is a program: it runs as written and gets no stock keys, only
  the stock workspace when it declared none (`finishLoad`). `on` and `setup` are applied and the config document
  sections reach the compositor (see "P6 config document and slots status"); `style`, `outputs` and `rules` are not.
- **Reload request path (`gnoblinctl config reload`).** Superseded by "JS-owned runtime role": the JavaScript role
  itself watches `session.reload_requested`, reloads the config, and answers the request with a `config.commit` or a
  `config.reject`. `GNOBLIN_JS_WATCH=0` (`JsRoleConfig::watch`) turns the watcher off. With no entry file `reload_now` is
  a successful no-op, which the role answers as a refusal because nothing was committed.
- **Proven against the real compositor** (`crates/gnoblin/tests/js_shell.rs`, in-process shell over a real
  headless compositor; `crates/gnoblin/tests/js_guardian.rs`, the shipped binary under its guardian; slice criteria 1,
  3, 4, 5 and 6): `init.ts` with annotations and an enum importing
  `./slots.ts` places two mapped windows; a rewrite reloads with the same window ids and new places; a syntax error
  leaves the old behaviour in force (a window mapped afterwards is placed by it) and is logged once as
  `init.ts:3:<col>`; fixing the file recovers; a `while (true)` in `onAdded` is interrupted and the generation
  restarts (the restaged one reads the mark the first wrote and does not hang), the scene answers an observer
  meanwhile and still updates; a handler that hangs in every generation (it hangs the restage too, while it loads)
  latches `Degraded` after three strikes and the last generation keeps placing windows. Criterion 6: with the shell
  gone (its owner connection closed in `js_shell.rs`; the runtime process `SIGKILL`ed in `js_guardian.rs`, where the
  guardian restarts it) the compositor answers a fresh connection, the Rust fallback policy places a window mapped
  meanwhile at the next step of the cascade (96, 96 after (32, 32) and (64, 64)) and takes the two windows the shell
  had moved back to the places it gave them (it never knew the shell's), and the restarted shell reattaches, is told
  every window is ready, learns of each and places them again with their ids unchanged. Zero-config: with no
  `init.ts` the shell runs `desktop` with `FloatingLayout` and the stock keys, and a `defineConfig` layered over it
  has its workspaces, its keys, the stock keys it left alone and not the stock key that showed the dropped
  workspace; the file's appearing is a reload. `gnoblinctl config reload` against the real session with the watcher
  off: an edit alone changes nothing; the request exits 0 and the shell takes the file with the same window id; a
  syntax error exits 1 with `init.ts:3:` and the old generation places a new window; the fixed file recovers.
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
- **Not done.** Input events and chords (the binding lane). The document, slots, `on` and `setup` are in "P6 config
  document and slots status"; `style` is in "P8a style status"; `outputs` is still not applied. `rules` are in
  "Focus, rules and snapping status". Removing `init.ts` under a running
  shell keeps its generation until the session restarts.

## JS-owned runtime role

Lane JsOwnConfig, cutover steps 5 and 6 of `design/js-parity.md`. Before it the JavaScript shell ran inside the Lua host:
the Lua host located `init.lua`, validated and committed the whole document, answered `config.reload`, sent the
guardian's `autostart` list and read the keyring first-boot input, and the JavaScript shell laid its settings over what
the Lua host committed. Now the JavaScript role does all of it and starts no Lua. Code: `crates/gnoblin-js/src/runner.rs`
(`JsRoleRunner`, `choose_runtime`), `crates/gnoblin-js/src/document.rs` (`evaluate_document`),
`crates/gnoblin-config/src/locate.rs`, `crates/gnoblin/src/main.rs`.

- **Selection (`GNOBLIN_SHELL_VM`).** `choose_runtime` is a pure function of the environment and the search path. Unset or
  empty: the JavaScript role, unless the located config is an `init.lua` (that is, there is no `init.ts` and no
  `init.js`), in which case the Lua role runs for this release and the binary logs one warning that names
  `docs/migrating-from-lua.md`. `js` and `js:<dir>`: the JavaScript role, always (with only an `init.lua` it runs the stock
  config and logs that the Lua file is not read). `lua`, `0` and any other value (a Lua shell directory): the Lua role, as
  before, with no warning because the user asked. The decision not to switch a Lua user silently is the owner's: no
  silent loss of their config. The variable is documented in `docs/typescript-shell.md`, and the Lua role is removed
  with step 8.
- **Locating the config.** `gnoblin_config::locate` takes the directory `GNOBLIN_CONFIG`, `$XDG_CONFIG_HOME/gnoblin` or
  `$HOME/.config/gnoblin` names and returns the first of `init.ts`, `init.js`, `init.lua` that is a file, with the others
  that exist in `ConfigPath::ignored()` (the JavaScript role warns that they are not read). With none it names
  `init.ts`. `GNOBLIN_CONFIG` names one file and its extension is its language. Every compositor-side reader of the
  last-good store uses the same function, so the store key and the file agree. `locate_lua` is the Lua host's: `init.lua`
  alone. There is no search of system directories: there is none today.
- **What the role does** (`JsRoleRunner::run`). It adopts the privileged connection the guardian passes, subscribes to the
  object graph (the `session` object's `reload_requested` is the signal it watches), builds a `JsRoleConfig` with
  `owns_document` set and a `JsCommitLog`, and boots `JsRole::connect`. The shell commits during that boot; the role then
  reports to the guardian, in the Lua role's order: one `autostart` line per enabled entry of the document, then
  `config-accepted REVISION`, then `ready` with `GNOBLIN_CONFIG_REVISION`. It then serves one frame per turn
  (`FRAME_WAIT`, 16 ms, is how long a turn waits on the compositor), reloads on `reload_requested`, and reports every
  commit the watcher's reload made.
- **The shell owns the document** (`JsShell::own_document`, `ConfigState::owns_document`). Each commit is the generation's
  document patch laid over an empty document: nothing the compositor held before is read, so a key deleted from `init.ts`
  is gone after a reload and after a runtime restart. Every load commits, even when the result equals the document in
  force, because the compositor holds a `config.reload` caller until a commit or a reject answers it. `autostart`
  commands become the document's `autostart` section, named by the program's file name (`foot`, then `foot-2`) so the
  same list gives the same names, and the shell starts none itself: the guardian starts them and matches a running one by
  name and command (so a reload, or a restart of the runtime role, starts no second copy), stops one a reload no longer
  lists, and does not restart one that exits (`ChildSpec::autostart`). The keyring conflict rule (`enabled_autostart`, moved to `gnoblin-config`) runs at `defineConfig`
  time (`host.config.check` takes the autostart list too), so a refused autostart fails the stage and the old generation
  keeps running, which is stronger than the Lua host's refusal after the evaluation.
- **Reload.** A request evaluates the entry in a staged generation and swaps it in when it evaluated cleanly (the machinery
  of "Hot reload"). If the stage failed, or it evaluated and nothing was committed (the compositor refused the commit, or
  the file is gone), the role calls `config.reject` with the reason, which `gnoblinctl config reload` prints with the
  file, line and column, and exit 1. The accepted document and the running generation stay. A document refused by the
  compositor after the stage leaves the new generation running, as it did before. A shell link failure ends the role
  non-zero and the guardian starts it again.
- **Keyring first boot** (`crates/gnoblin-services/src/keyring.rs`, `decide`). Order: a configuration the compositor
  already accepted; the user's `init.ts` or `init.js` evaluated by `gnoblin_js::evaluate_document`; an accepted
  configuration waited for (20 s); the last accepted file; the schema default. `evaluate_document` evaluates the entry in
  a throwaway generation with no tree and no host objects (the probe module `gnoblin-document-probe` stands in for the
  host's `__register`), a 1 s budget and 32 MiB, merges what `defineConfig` was given over the stock config and
  desugars it as the shell does, and returns the document. A config that needs the compositor while it loads cannot
  be read that way; the error is kept as the reason and the wait goes on. `gnoblin-services` therefore depends on
  `gnoblin-js` (`xtask deps` accepts the edge: both are role entry crates and neither names an engine).
- **`gnoblinctl config validate FILE`** routes `.ts`, `.mts`, `.js` and `.mjs` to the same in-process check as
  `gnoblinctl check`.
- **Proven against the real compositor.** `crates/gnoblin/tests/js_runtime_role.rs` runs the shipped `JsRoleRunner`
  in a thread over a headless compositor: commit, then `config-accepted`, then `ready`; autostart lines before the
  revision; a config the validator refuses runs the stock config and still commits; the twins of the Lua role's
  `runtime_reload.rs` (a script's `reloadConfig()` commits what the file says now, a broken file keeps the accepted
  document and the role, an autostart the check refuses keeps the old handlers); `config.reload` answers with the
  revision and a removed file is refused with why; the role ends cleanly when the compositor goes; and the shipped
  starter loads (the twin of `starter_config.rs`). `crates/gnoblin/tests/js_guardian.rs` runs the binary under its
  guardian with `GNOBLIN_SHELL_VM` unset and no Lua anywhere: `init.ts` committed as revision 1, `config reload`, the
  watcher, a killed runtime committing what the file says and not what it left behind, `autostart` started by the
  guardian, the Lua fallback with one warning, an `init.ts` beside
  an `init.lua`, and `GNOBLIN_SHELL_VM=lua` and `js` with only an `init.lua`. `crates/gnoblin/tests/guardian.rs` is
  unchanged and now runs the JavaScript role (no `GNOBLIN_SHELL_VM`): the killed runtime restarts and commits revision 2.
- **Faked.** Nothing in those tests. The unit tests of `ConfigState` use a fake document link (no compositor), and
  `choose_runtime` is tested as a pure function.
- **Not done.** The shared-document mode of `ConfigState` (the shell lays its settings over a document another host
  committed, `owns_document = false`, `JsRole::config_committed`) has no product caller any more; the shell tests still
  run in it (about fifty tests in `crates/gnoblin-js/tests` and `js_shell.rs`), so it goes when they move to the owned
  mode. `scripts/xr-smoke.sh` still selects the Lua stock directory (a Lua shell directory still selects the Lua role).
  The first-boot Secret portal still reads the last accepted file (`design/e2e-keyring.md`).

## P6 config document and slots status

Lane JsConfigSlots. `defineConfig` keys that are settings of the config document now reach it, a script can observe
and refuse the compositor's own dialogs, and `on` and `setup` run. Code: `crates/gnoblin-js/src/config.rs` (the
document), `crates/gnoblin-js/src/dialogs.rs` and `data/js/gnoblin/dialogs.ts` (`gnoblin.dialogs`),
`JsShell::apply_document`/`run_dialogs`, `JsRole` (the connections). Tests: `crates/gnoblin-js/tests/config_slots.rs`
over fakes, `crates/gnoblin/tests/js_shell.rs` against the real compositor.

### What each `defineConfig` key becomes

| `defineConfig` key                                                    | Becomes                                                                                                                                                                    | State                                                                                                        |
| --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `gaps`                                                                | `Workspace.gaps`, a script property (no document key, no tree property)                                                                                                    | applied                                                                                                      |
| `workspaces`                                                          | `Workspace` objects in `gnoblin.workspaces`. The document's `workspaces` section is the Lua host's list, read at start                                                     | applied as script objects; not written to the document                                                       |
| `keys`                                                                | `gnoblin.input.primary.bind`: seat claims, a tree call (the document's `keybindings` is carried and read by nothing)                                                       | applied                                                                                                      |
| `decorations`                                                         | a `Titlebar` canvas node per client, a tree write                                                                                                                          | applied                                                                                                      |
| `menus`, `alerts`                                                     | flags: the stock window menu answers menu requests, the stock alert watches pings (see the shell UI status section)                                                        | applied                                                                                                      |
| `on` (`clientAdded`, `clientRemoved`, `titleChanged`, `focusChanged`) | listeners on `gnoblin.clients`: a `CollectionListener` and a `ClientListener`                                                                                              | applied                                                                                                      |
| `setup(gnoblin)`                                                      | called once after everything above, inside the generation's load budget                                                                                                    | applied                                                                                                      |
| `dialogs.style`                                                       | document `dialogs.style` (`radius`, `accent`, `font.family`, `font.size`, `button-layout`, `scale`)                                                                        | applied: live, the next dialog uses it                                                                       |
| `appearance`                                                          | document `appearance` (`color-scheme`, `accent`, `reduced-motion`, `high-contrast`)                                                                                        | applied: live                                                                                                |
| `cursor`                                                              | document `cursor` (`theme`, `size`, `providers`)                                                                                                                           | applied: live                                                                                                |
| `windowManagement`                                                    | document `window-management` (focus, raise, edge tiling, titlebar actions, ...)                                                                                            | applied: live, except `attach-modal-dialogs` and `disable-workarounds` (unused)                              |
| `input`                                                               | document `input`: `keyboard` is live; `mouse`, `touchpad`, `trackball`, `pointing-stick`, `tablets`, `styluses`, `orientation-lock` are accepted and read by no device yet | `keyboard` applied; the rest not applied (notice from the compositor)                                        |
| `inputSources`                                                        | document `input-sources` (`sources`, `per-window`)                                                                                                                         | applied: live                                                                                                |
| `accessibility`                                                       | document `accessibility` (magnifier, colour filter)                                                                                                                        | applied: live                                                                                                |
| `compositor`                                                          | document `compositor`: `visual-bell`, `visual-bell-type` live; `renderer` at the next compositor start; the rest unused                                                    | partly applied                                                                                               |
| `monitors`                                                            | document `monitors`: `global-scale` live; `privacy-screen` unused                                                                                                          | partly applied                                                                                               |
| `xwayland`                                                            | document `xwayland`                                                                                                                                                        | applied: live (restarts the X11 server when no X11 client is connected)                                      |
| `keyring`, `location`, `compat`, `capture`, `logging`                 | document sections read when a process starts                                                                                                                               | committed; takes effect at the next start of that process (the compositor says so in a `not applied` notice) |
| `layerShell`, `auth`                                                  | document sections nothing reads yet                                                                                                                                        | committed; not applied (notice)                                                                              |
| `style`                                                               | `gnoblin.clients.style`, the global scope of the style cascade; `rules[].style` is the client's own style                                                                  | applied: opacity, outer shadows, backdrop blur; the rest is noticed, not drawn (P8a)                         |
| `outputs`                                                             | stored as written. The document has only the global `monitors.global-scale`, and a screen's tree properties are read-only                                                  | **not applied: needs native style objects (P8)**                                                             |
| `rules`                                                               | stored as written; lane JsStockFocus owns `rules.ts`                                                                                                                       | not applied by this lane                                                                                     |
| `launcher`                                                            | a `Super+space` key running the command, unless `keys` binds that trigger                                                                                                  | applied (a key, see `keys`)                                                                                  |
| `focus`, `edgeTiling`, `dragConstraint`                               | script-side overrides of the seat's published settings (see "Focus, rules and snapping status")                                                                            | applied                                                                                                      |
| `autostart`                                                           | argv lists the shell starts through `gnoblin.spawn` once per session (see "Lists, maps and the first run"); not the document's `autostart`                                 | applied: a command already started this shell is never started again                                         |
| `protocols`                                                           | document `protocols` (camelCase protocol names to kebab-case, booleans)                                                                                                    | committed; takes effect at the next compositor start                                                         |
| `touchpadGestures`                                                    | document `touchpad-gestures`, one list value (a reload that drops the key removes the list)                                                                                | applied: live                                                                                                |

The per-setting rule behind the last columns is `crates/gnoblin-config/src/apply.rs` (`RULES`).

Every top-level section of the document is either a `defineConfig` key or recorded here as not one. The test
`every_section_of_the_config_document_is_a_key_or_a_recorded_non_key` (`crates/gnoblin-js/tests/config_keys.rs`) fails
when the schema gains a section that has neither:

| Document section                                                                                                | `defineConfig`                                                                                                                                |
| --------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `compositor`, `window-management`, `cursor`, `appearance`, `dialogs`, `input`, `input-sources`, `accessibility` | a key of the camelCase name                                                                                                                   |
| `layer-shell`, `keyring`, `auth`, `location`, `compat`, `capture`, `monitors`, `xwayland`, `logging`            | a key of the camelCase name                                                                                                                   |
| `protocols`, `touchpad-gestures`                                                                                | a key (`protocols`, `touchpadGestures`): their inner names are fixed (protocol names, gesture fields), so camelCase maps back to the document |
| `workspaces`                                                                                                    | the key `workspaces`, as script objects; the document's list is the Lua host's and is not written                                             |
| `autostart`                                                                                                     | the key `autostart`, but shell-side (below), not the document section: its entries are keyed by names the user chooses                        |
| `shortcuts`                                                                                                     | **not a key**: `keys` is the one way to bind a key. Naming it is refused with the mapping in the message (docs/typescript-config.md)          |
| `keybindings`                                                                                                   | **not a key**: carried and read by nothing; `keys`                                                                                            |
| `animations`, `portals`                                                                                         | **not keys**: carried and read by nothing                                                                                                     |
| environment, per-output settings                                                                                | the schema has no environment section. A screen's tree properties are read-only, so `outputs` stays stored as written (native style objects)  |

A table keyed by names the user chooses (`autostart`, `shortcuts`) cannot be a document key: turning `camelCase` keys back
into the user's own names is not possible. A list (`touchpad-gestures`) can, because it is one value and the keys inside an
item are fixed.

### The document commit

- **Keys.** A section key is camelCase in a config and kebab-case in the document (`windowManagement` is
  `window-management`, `buttonLayout` is `button-layout`); a key already kebab-case is left alone, and two spellings
  of one key are a `TypeError`. `desugarConfig` puts the result in `ConfigPlan.document`, plain JSON, and
  `mergeConfig` merges the sections by key (a list or a scalar of the user's replaces).
- **The shell's connections.** `JsRole::connect` opens two owner connections besides the tree's: a `ClientSlotLink` (the
  one the Lua role uses) and a `JsClientConfigLink`, which reads `config.current` and writes `config.commit`, the two
  verbs the Lua host uses. `JsRole::over_link` takes them as `JsRoleLinks`; each is optional and the role runs without
  it (the compositor's own dialogs serve, and the document keeps what the Lua host committed), reported once.
- **Full document, not a patch.** The compositor holds one document and derives its settings from each one it
  accepts, so a commit must carry everything. The shell commits the document in force with the generation's patch
  laid over it, and remembers the key paths the patch set. The next commit starts from the document in force with those
  paths taken out (a table the removal empties goes with it) and the new patch laid on. That is the
  generation-overlay rule of spec-review 1.27: a reload that drops `dialogs.style.radius` puts it back to its default,
  and a setting the shell never made (the Lua file's) is not touched. A document that would not change is not
  committed.
- **Validation and where it is reported.** `Config::from_value`, the validator the compositor and the Lua host use,
  runs on the document the commit would send. It runs when `defineConfig` is called, which is while a reload is
  staging: `host.config.check` throws a `GnoblinError` (`defineConfig: dialogs.style.radius: ...`, the document's key
  path), the entry fails, `ReloadReport.errors` carries it with the file and line, and the running generation and the
  document stay. The compositor refuses nothing here itself: it records and parses a committed document and keeps its
  last good settings when one does not decode, so the check is the runtime's. A commit the compositor does refuse
  (it can, for `xwayland`) happens after the swap, which cannot be taken back: it is a diagnostic with the compositor's
  reason, and the notices of an accepted commit (`not applied: ...`) are diagnostics too.
- **The Lua host's reload commits too.** `gnoblinctl config reload` reloads the shell first and then the Lua host,
  which commits a document with none of the shell's settings. `ShellFrame::config_committed` (called by the Lua
  role right after its commit, before it announces the reload) makes the shell lay its settings back over the document
  in force. For the sliver between the two commits the settings are at their defaults.

### Slots and `gnoblin.dialogs`

- **The slot.** `dialogs.observe`, from design `popups-portals.md` 7.5: the compositor shows the shell each consent and
  passphrase request for 250 ms before its own dialog opens, under the title `consent:<interface>` or `secret`. The only
  answer it reads is `deny`, which refuses the request by policy (the requester sees `cancelled(user)`); anything
  else, silence, a decline or a departure leaves it to the built-in dialog.
- **Claim and release.** The shell claims the slot while the running generation has a dialog listener and gives it back
  when it has none, so a config with no listener never delays a dialog. A refused claim, or no connection, is reported
  once and not retried until the listeners are gone. `JsShell::retire` (a reload swap, a restage, a shutdown) answers each
  observation it still holds `pass` and releases the slot; the next generation claims it again on its first frame. A
  crash releases it, because the compositor releases a connection's claims when it closes.
- **Per frame.** `JsShell::frame` polls the `SlotLink` once, hands the offers to `entry.dialogs`, runs the queued
  listener calls under the usual budgets, takes `entry.takeVerdicts` and sends each: `claim_request`, then `answer`
  with the fixed bytes `deny` or `pass`. Nothing a script wrote is ever an answer, and the slot has no way to carry a
  passphrase. A `finished` event (the window ended) closes the request, so a late `decline` refuses nothing.
- **API.** `DialogListener` with optional `onConsentRequest(request)` and `onSecretRequest(request)`;
  `gnoblin.dialogs.addListener/removeListener`. A request has `kind`, `appId`, `subject`, `deadline` and
  `decline(reason = "policy")`. `appId` is the compositor's verified id. `subject` is the portal interface of a consent
  request and the app's own prompt text of a passphrase request. There is no method that allows a request and no
  access to a secret. One queued call per listener, then one that settles the request, so a listener that throws or
  runs out its budget costs the others nothing; the failure is reported once per site.
- **Budget.** A listener that runs past `entry_budget` is interrupted, counts a strike (the supervisor restages the
  generation after it, as for any listener) and the request passes. The compositor's window is 250 ms in any case: a
  shell that is wedged costs the user at most that.

### Proof

Against the real compositor (`crates/gnoblin/tests/js_shell.rs`): `defineConfig({ dialogs })` is in the document the
compositor holds, read back through a second owner connection (`button-layout` camelCase to kebab-case); a radius of 5000
is refused at the reload with `dialogs.style.radius` in the shell's diagnostics, the generation stays and the document
is untouched; a reload without the key removes `dialogs` from the document. A listener that declines a request from
one app refuses it (the requester gets `cancelled(user)` without anything withdrawing it) while a request from another
app stays open for the built-in dialog; a listener that never returns is interrupted, the request is not refused and
the built-in dialog serves it, the generation restarts and the next request is refused again. Against fakes
(`crates/gnoblin-js/tests/config_slots.rs`): the patch, the removal and the no-op commits, a foreign commit, a refused
commit, notices, the missing links, `on`, `setup`, the claim and release, the verdict words, isolation, the budget, a
held request at `retire`, a lost connection.

What the real tests do not show: that the built-in dialog opens is shown by the request staying open until its
requester withdraws it, not by a screenshot of the dialog; `retire` answering a held request is proven against the
fake link only (the real compositor answers within 250 ms, so a request is rarely held across a swap).

### Lists, maps and the first run

Lane JsConfigLists. Code: `data/js/gnoblin/config.ts` (`CONFIG_KEYS`, `DOCUMENT_LISTS`, `checkShape`, `planAutostart`),
`data/js/gnoblin/workspace.ts` (`rejectUnknown`, `nearestKey`), `crates/gnoblin-js/src/autostart.rs`,
`crates/gnoblinctl/src/js.rs` (`config_init`), `data/js/starter/init.ts`. Tests: `crates/gnoblin-js/tests/config_keys.rs`
(plans and messages), `config_slots.rs` (the document and autostart over fakes), `crates/gnoblin/tests/js_shell.rs`
(the starter against the real compositor), `crates/gnoblinctl/tests/js_check.rs` (`config init`).

- **`protocols` and `touchpadGestures`.** They go through the document commit unchanged: the first is a section in
  `DOCUMENT_SECTIONS`, the second a list in `DOCUMENT_LISTS`, which `planDocument` writes as one value. Add, change,
  removal (the section is gone again) and a refused value (`protocols.wlr-layer-shell`, `touchpad-gestures`) are
  tested in `config_slots.rs`.
- **`autostart` is the shell's, not the document's.** The guardian starts the document's `autostart` entries, but it
  learns them from the Lua host's commit (`commit` in `gnoblin-lua/src/role.rs` sends the `autostart` lines) and retires
  any child that commit did not list, so a section the shell wrote would be stopped by the next Lua reload. The shell
  instead starts each command through the spawner `gnoblin.spawn` uses (process group of its own, reaped).
  `JsShell::apply_settings` runs after a generation is swapped in or loaded, so a failed stage starts nothing.
- **Identity and limits.** A command is its argv. `JsAutostart` keeps the set of argv lists started by this shell; a
  reload starts only the new ones, removal stops nothing (the shell keeps no handle to stop), and listing a removed
  command again does not start it again. A command that cannot be started is a diagnostic and is not remembered, so the
  next reload tries again. The set is the shell's memory: a restarted runtime role (the guardian restarts it) starts
  every command again and a survivor from before runs twice. Documented in docs/typescript-config.md.
- **One way to bind a key.** The document's `shortcuts` (`<Super>Return` chords, `command`, `trigger`, `enable`) is not a
  key. `RETIRED_KEYS` gives the message for `shortcuts`, `keybindings` and the kebab/snake spellings of
  `touchpad-gestures`. `trigger = "release"` has no `keys` equivalent (a key acts on press), so such a shortcut stays in
  `init.lua`.
- **Messages.** `rejectUnknown` names the place and the key, suggests the nearest valid key within two edits (case, `-`
  and `_` do not count as edits; ties go to the first in the list), and lists the valid ones. `checkShape` runs first
  and checks that a table key holds a table and a list key a list, naming `config.<key>`, what it should be and what it
  got. A nested document value is checked by `Config::from_value` with the document's key path
  (`dialogs.style.radius`), and a nested name it does not know is listed with the accepted ones.
- **`gnoblinctl config init [DIR] [--force]`.** Writes `STARTER_CONFIG` (`data/js/starter/init.ts`, embedded in
  `gnoblin-js`), refuses when `init.ts` or `init.js` exists (an `init.ts` is read first and would hide `init.js`),
  runs `types init` (`ensure_user_files`), prints three steps, and has a `--json` form. The starter as written is
  `defaultConfig` (empty `keys`, `rules`, `autostart`, the stock gaps, `decorations: true`, the one `desktop`
  workspace); its examples are comments that end the way a config line ends, so a test can switch them on.
- **Proof.** `the_starter_as_written_plans_exactly_what_the_stock_config_plans` compares plans;
  `the_starter_is_the_stock_config_as_written_and_its_examples_work_switched_on` (real compositor) loads the starter
  over a no-`init.ts` shell and asserts the same workspaces, the same bound keys and the same titlebar count, then
  switches every example on and sees the second workspace, the example keys and an autostart command that ran once
  and not again after a reload. `gnoblinctl check --types` is run on the starter, as written and switched on: it passes
  when no TypeScript compiler is present (the step is skipped, and the test prints that) and it type-checks when one is.
- **Not done.** Typed completions for the nested settings of the document sections (`input.keyboard`, ...): they are
  `DocumentSection` tables in `gnoblin-stock.d.ts`, so an editor does not complete them, and a wrong nested name is a
  load error that lists the accepted names. A TypeScript emitter in `gnoblin-config-codegen` would generate them from
  the schema; `style` and `outputs` still need native style objects.

## Focus, rules and snapping status

Lane JsStockFocus: `data/js/gnoblin/focus.ts`, `rules.ts` and `snap.ts`, ported from `data/lua/gnoblin/shell/focus.lua`,
`rules.lua` and `snap.lua` and the `Window:on_*`, `apply_rules`, `snap` and `unsnap` wiring of `init.lua`. The tests are
`crates/gnoblin-js/tests/stock_focus.rs` (the cases of `crates/gnoblin-lua/tests/lua/{focus,rules,snap}_test.lua`, over
the fake link) and one real-compositor test in `crates/gnoblin/tests/js_shell.rs`. User pages: `docs/config/window_rule.md`
(the rule fields and the table from Lua patterns) and `docs/config/configure/window_management.md`.

- **Settings.** The compositor publishes `edge_tiling`, `drag_constraint` and the six focus keys on the seat, read-only.
  `JsShell` reads them through `__gnoblinHost.input.settings(seat)` (`crates/gnoblin-js/src/input.rs`, the schema's
  defaults until the compositor says otherwise) and `InputContext.settings` hands them out. `defineConfig` has three
  keys over them: `focus` (`mode`, `newWindows`, `raiseOnClick`, `autoRaise`, `autoRaiseDelay`, `changeOnPointerRest`),
  `edgeTiling` and `dragConstraint`. A key the config leaves out is the seat's, so the compositor's configuration is
  the default and the script overrides it for the shell only (a seat prop cannot be written). The focus modes are
  `click`, `sloppy` (the seat's `hover`) and `follow-mouse` (`hover-strict`: acts on entry only and clears focus when the
  pointer leaves the focused window); `desugarConfig` checks the keys and the plan holds them as plain data.
- **Pure core.** `focusSettings`, `opensFocused`, `hoverFocus`, `autoRaiseSeconds`, `restFocus`, `leaveClears` are the
  Lua functions; `focusDecision(settings, event)` composes them into one decision per event (`added`, `enter`, `motion`,
  `leave`, `press`), a plain object: focus, clear, raise, and the delays of a rest focus and an auto raise. `edgeZone`,
  `tileRect` and `edgeTilingEnabled` are `snap.lua`'s, with rectangles written `x, y, width, height`.
- **Pointer edges on windows.** The compositor sends the shell `enter`, `motion`, `leave`, `press` and `release` events
  addressed to a window's node (a press on a window is an edge there while a shell holds the tree, so the shell decides
  focus and raise). `JsShell::frame` parks them (`JsInput::accept_window_event`) and, after the frame's `plan`, calls
  `entry.pointer(events)`, which queues the clients' `onPointerEnter`, `onPointerMotion`, `onPointerLeave`,
  `onPointerPress` and `onPointerRelease` with `{ x, y, button }` in the window's own coordinates. A client listener
  hears one client's edges, a collection listener every member's. Scroll and key events still have no JavaScript API.
- **Wiring.** `defineConfig` registers a `RuleEngine` and a `FocusPolicy` on `gnoblin.clients`, rules first, so
  `skipFocus` is known when the focus policy hears the window. The policy keeps two timers (a pointer rest, an auto
  raise) on `InputManager`'s per-frame hook, with the clock of `__gnoblinHost.input.now`. A new window takes focus
  (`client.focus()`), except a window the shell found at boot: a reload does not take the user's focus. `client.focus()`
  is a write of the first seat's `focus` (the Lua shell's `seat.focus = window`), because the compositor answers no
  `focus` call on a window (`run_window_calls` handles `close` and `kill`); the call it was before did nothing against
  the real compositor. The policy and the rules read focus from the seat as well (`InputContext.focusedClient`), because
  an observer connection saw no window's own `focused` set while a shell held the tree. The Lua shell
  never set `from_launcher`, so `focus_new_windows` has no launcher origin to read yet and the policy passes `false`.
  A script that never calls `defineConfig` runs bare, with neither.
- **Raise.** `Client.raise()` calls `__gnoblinHost.raise`: the tree's order is the stacking order, but a window the
  compositor made has no parent and sorts after the root's tree, so the call first gives every unparented window the root
  for its parent, in the order it already has, and then moves the window last. A window already on top queues nothing.
  Unit-tested on a mirror (`bindings/host.rs`), and proved against the real compositor
  (`a_click_focuses_and_raises_a_real_window_so_the_compositor_picks_it_over_the_one_above`): two overlapping windows,
  a click on the lower one where only it shows, then a click where both show, which the compositor gives to the lower
  window because it was raised.
- **Rules.** A rule is data: the query keys `appId` and `title` (a glob, matched against the whole text, or a RegExp tested
  anywhere in it), `focused`, `onWorkspace`; the actions `workspace`, `floating`, `size`, `position`, `style`
  (`opacity` is applied, `cornerRadius` and `shadows` are checked and kept with no native effect, like `config.style`),
  `fullscreen`, `maximised`, `secure`, `skipFocus`; and `first`, `reapply`. Every matching rule applies in list order and
  the later wins per field, as every Lua rule list did (`rules_test.lua`: "the last matching rule wins for a field", "rules
  merge in list order"); a rule with `first: true` ends the search where it matches. A rule applies when its window
  appears and, unless it says `reapply: false`, again when appId, title or focus changes (`ClientListener.onAppIdChanged`
  is new; `RuleEngine.onFocusChanged` is the focus read), and undoes `secure`, `fullscreen`, `maximised`, `opacity`, the
  corner radius and the shadows when it stops matching, to what a rule that still matches sets (else the default). A rule
  that still matches writes nothing. A window the shell found at boot is not touched. Workspace changes of a window are
  not a rule input yet: the client has no event for them.
  `compileRules` runs in `desugarConfig`, so a bad rule fails the stage naming `rules[i]` and the field. `role` is
  refused by name: the tree publishes no window role. The Lua `xr` field and the app-id fallback to the WM class are not
  ported (the tree carries no class). A `position` holds the window there (`floatingBounds`), a lone `size` is what the
  floating layout places it at (`client.size`), and `floating` keeps a window out of the layout's `compute`
  (`Client.floating`, new; a script-side flag). `maximised` and `fullscreen` write the flag and fill the primary
  screen's area, and turning the rule off puts the window back. New `Client` members: `fullscreen` (writable), `opacity`,
  `secure`, `floating`, `raise()`; `opacity` reads 1 and not 0 while the compositor has published none.
- **Snapping.** `ClientMoveBinding({ snap })` tiles a window dropped within `snap` pixels (default 4, `false` never) of a
  screen edge when the seat's `edge_tiling` (or the config's `edgeTiling`) is on, as `drag` in `init.lua` did.
  `snapClient`, `unsnapClient`, `snappedZone` and `forgetSnap` keep the floating rectangle off the client; a maximised
  window floats first; unsnapping restores the rectangle the window had when it snapped (Lua's `keep_floating`), and
  a drag that takes hold of a snapped window ends the snapped state. `ClientResizeBinding({ edges: "nearest" })` moves the
  one side the press is nearest to; `quadrant` still moves a corner.
- **Not done.** `focus_new_windows` launcher origin (no source), a rule's `role`, `xr`, drawing `cornerRadius` and
  `shadows`, the Lua `refit` of a snapped window when the work area changes. Client requests are in "Client requests
  status".
  A press on a titlebar canvas does not focus or raise its window: a canvas's pointer events do not reach the client's
  listeners, so `Titlebar` would have to ask for it.

## Client requests status

Lane JsClientRequests, cutover step 1: `data/js/gnoblin/window_state.ts` answers the requests the compositor sends a shell
and changes nothing for until it is answered (`crates/gnoblin-schema/schema/tree/messages.toml`, the `request` event of a
node: `maximize unmaximize fullscreen unfullscreen minimize unminimize close raise lower activate move resize menu`). The
policy is `Window:on_request` and its helpers in `data/lua/gnoblin/shell/init.lua` (`present`, `raise`, `lower`,
`maximise`, `enter_fullscreen`, `minimise` and their inverses), ported with the same inputs and expected values. The
tests are `crates/gnoblin-js/tests/client_requests.rs` (the 12 cases of `states_test.lua`, the `shell_test.lua` cases for
raising with a fullscreen sibling, activation and a repeat fullscreen, and the new cases below, over the fake link) and
three tests in `crates/gnoblin/tests/js_shell.rs` that run a real Wayland client against the real compositor
(`xdg_toplevel.set_fullscreen`, `set_minimized` and an `xdg_activation_v1` token earned by a click).

- **Delivery.** A `request` event is parked by `JsUiState` whatever its node and reaches `onUiRequest` handlers in the
  toolkit's frame. `windowState.start` registers its handler when `gnoblin.ts` loads, so it runs before the window menu's
  (which also hears `menu`) and a script's answer can stop the menu. A script that never calls `defineConfig` has the
  policy too: it is not a config key, and a request with no answer is a window that cannot be maximised.
- **The hook.** `ClientListener.onRequest(client, request)`: the listeners attached to the client first, then each
  collection that holds it, each in the order attached, called synchronously; the first to call `request.handle()` wins,
  later listeners are not called and the built-in answer does not run. A throw is reported (`Name.onRequest`) and the
  next listener is asked. `request` is a `ClientRequest` (`kind`, `client`, `x`, `y` for a menu anchor, `handled`,
  `handle()`). A listener that handles a request may call `windowState` (`maximise`, `unmaximise`, `enterFullscreen`,
  `leaveFullscreen`, `minimise`, `unminimise`, `present`, `raise`, `lower`, `beginMove`, `beginResize`), each returning
  whether it ran. The rule is written in the header of `window_state.ts` and the page `docs/typescript-shell.md`.
- **Fullscreen covers the whole output.** The Lua `cover_rect` is the whole `Screen` under the bars (`{ x = 0, y = -d.space.y,
w = d.width, h = d.height }` in the workspace's own space), and `Desk:sync_fullscreen` hides the bars and the workspace
  clip while a fullscreen window shows. The JavaScript model had the screen less the bars (`screenArea`). The decision is
  Lua's: the rectangle is `(0, 0, output width, output height)` in screen coordinates (which is the Lua `y = -32` with a 32
  pixel bar), and `Bar.shown` is false while a fullscreen window shows on the workspace on show (`barsGiveWayTo`,
  `data/js/gnoblin/widgets.ts`), so it is hidden and takes no room. The bars return when the window leaves fullscreen,
  minimises, closes or its workspace is hidden. There is no workspace clip in JavaScript: a window is placed in screen
  coordinates, so the Lua `space.clip` check has no twin. `maximize` still fills `screenArea` (what the bars leave).
- **Rectangles.** `maximise` keeps the floating rectangle, `enterFullscreen` the windowed one (a maximised or snapped
  window keeps its flags, and leaving fullscreen returns the maximised rectangle), as `keep_floating` and
  `data.windowed` did. A repeat `fullscreen` is refused so it cannot overwrite the kept rectangle, and a `maximize` of a
  fullscreen window is refused. The rule engine and the menu and titlebar maximise/minimise buttons call the same
  methods, so a window a rule made fullscreen is covered the same way (`cover` and `beforeCover` are gone).
  A tiling layout leaves a fullscreen or maximised window alone (`layout()` filters them from `compute`, and a change of
  the covering set makes the workspace compute again).
- **Raise.** `Client.raise()` keeps a fullscreen sibling on the same workspace on top of the window it raised, as
  `Window:raise` did; a pinned window (`windowMenu.isAbove`) is still kept by the menu's per-frame `#keepPinned`, which
  already put a fullscreen window over the pinned ones.
- **Minimise and activate.** `minimise` writes `minimised`; the compositor hides the window and a layout does not show it
  again (`writtenVisible`). When the window held the keyboard focus the seat's focus goes to the topmost sibling on its
  workspace that is not minimised, or to nobody. `unminimise` focuses the window only when nothing holds the focus.
  `present` (the `activate` request, the compositor's own authority having allowed it) does nothing for a window on a
  workspace that is not shown; else it unminimises, focuses and raises.
- **Move and resize.** The request names no pointer and no edge, so `beginMove` and `beginResize` read the seat's pointer
  and start the stock `ClientMoveBinding` or `ClientResizeBinding` through their own `onStart`, `onUpdate` and `onEnd`
  (`constrainRect`, the snap on a drop, the minimum size). The drag follows the pointer in a frame hook and ends on the
  next pointer press or release on a window (the compositor sends the release to the client that holds the implicit
  grab), when its window goes, or after `REQUEST_DRAG_LIMIT_MS` (30 s) so none follows the pointer for ever. A window that
  a layout tiles (`Client.tiled`), a fullscreen one and a maximised one are not dragged. A pointer within
  `RESIZE_CORNER_ZONE` (16) pixels of two sides resizes that corner, anywhere else the nearest side. This is proved over the
  fake link only: a real `xdg_toplevel.move` needs a client that sends the request with a pointer serial, which the
  test client does not.
- **Rules and the stock workspaces** moved in the same lane. Rule lists merge every matching rule (see "Focus, rules and
  snapping status"); the zero-config baseline is four workspaces, `1` to `4`, named `Workspace 1` to `Workspace 4`
  (`defineConfig({ workspaces: [{ id, name, layout }] })` gained `name`), and the starter is rewritten to match.
- **Not done.** The Lua `refit` of a maximised or snapped window when the bars change, a request for a window on
  another output (the shell has one primary screen), and the kept rectangles across a reload: they live in the
  generation, so a window that is fullscreen or maximised when the config reloads leaves it where it is.

## Window policy status

Lane JsWindowPolicy (`docs/js-window-policy.md`). Done, each with the tests named in `design/js-parity.md`: keyboard Move and Resize
(`data/js/gnoblin/reposition.ts`, one grab node that holds the keyboard; real-compositor test with injected keys), gesture
actions (`gesture_actions.ts`, `gestureActions`, `gnoblin.gestures`), reduced motion (`gnoblin.motion`, read from the seat's
`reduced_motion`), binding-only devices (`claim_device`) and the `false` action, the focus hand-off for every layout,
`Workspace.walk`, workspace removal to the neighbour before it with the last refused, titlebar presses reaching the focus
policy, a refit of the windows when a bar changes the work area, kept rectangles across a reload (carried in the workspace
snapshot, `JsKeptWindow`), and `workspaceSets` with `activateSet`.

- **Decisions.** A gesture bound at the stock slide's trigger replaces it; `gestureActions` and `keys` for one gesture is an
  error. The hand-off is skipped for a layout that defines `onClientRemoved` (a strip's columns are not in window order).
  `workspaces.remove` returns a boolean and refuses the last workspace; `clear()` still empties.
- **Not done.** Nothing of the five lane targets; the real-compositor proof covers only the keyboard move and resize, the rest
  runs over the fake link.

## Tiling bindings status

Lane JsTilingBindings (`docs/shell-api/input-handlers.md`, "Tiling drags"). Done: `ClientSwapBinding` (id `swap`,
`Super+MouseLeft+Drag`) and `SplitResizeBinding` (id `resize`, `Super+MouseRight+Drag`) in `data/js/gnoblin/bindings.ts`,
registered by `MasterStackLayout` and `DwindleLayout` (both) and `GridLayout` (swap only) with the `triggers` option
`FloatingLayout` has (`null` leaves one out, `layout.bindings.remove("swap")` removes it). `MonocleLayout` has none. Tests:
`crates/gnoblin-js/tests/tiling_bindings.rs` over the fake link, and the real-compositor test in `js_shell.rs`.

- **Order.** `workspace.clients.swap(a, b)` (new on `MutableCollection`) trades two members' places and marks the workspace
  dirty; the owner spec's `layout.order.swap` is this, since the order lives on the workspace's collection and a layout has
  no handle on its workspace. A layout that keeps its own record of who sits where hears it as `onClientsSwapped(a, b)`
  (`DwindleLayout` trades two leaves of its tree; a tree is placed by its shape, not by the order).
- **Swap.** The drop target is the tiled client of the dragged client's workspace whose bounds hold the pointer, found by
  scanning the workspace and not through `context.hitTest`, which can answer with a hidden window of another workspace.
  Nothing moves while the pointer is down, so the layout computes once, on release. A release outside every tile, over the
  held tile, or a cancel changes nothing. There is no ghost or drop highlight; the spec's `previewSwap` is not written.
  A client whose `tiled` flag is false (floating, a dialog), or that is fullscreen or maximised, declines.
- **Split.** A layout with dividers answers `findSplit(client, point)` with a `LayoutSplit`: the axis the pointer moves
  along, the pixels of travel that change the ratio by 1 (`span`), the direction, the limits and `reset`. The binding sets
  `ratio` from the pointer's travel since the press, so the divider follows the pointer exactly, and the layout computes
  per frame because its own field changed. `MasterStackLayout.ratio` is the field (`direction` -1 for a master on the
  right or at the bottom, `span` is the extent less the gap). `DwindleLayout` gives each split a ratio of its own
  (`null` follows `layout.ratio`), picks the split above the pressed tile whose divider is nearest the press, and counts
  edits in the public field `revision`, because the tree is not a field the workspace could compare. A cancel gives the split
  back to `layout.ratio`.
- **Not done.** The spec's `previewSwap` preview, and a split chosen by hit-testing the divider's own pixels: the nearest
  divider above the pressed tile takes the drag wherever in the tile the press is.

## Outputs status

Lane JsOutputs. User page: `docs/js-outputs.md`. Tests: `crates/gnoblin-js/tests/outputs.rs` (28 cases over the fake
compositor; the Lua cases of `shell_test.lua` for moving, unplugging, reconnecting and dialogs with their inputs and
expected values), `crates/gnoblin-wayland/tests/window_relations.rs` (the compositor publishes `parent_window` and
`from_launcher`, with a real client), and `crates/gnoblin/tests/js_shell.rs`
`a_window_on_a_second_virtual_output_follows_to_the_first_when_it_is_unplugged_and_parked_windows_return_with_the_next_output`
(a real compositor and shell; the headless backend takes an output at run time through `OutputTap`, a new
`SessionConfig::output_tap`).

**Native.** `window.toml` gains `parent_window` (`object:window`, from `xdg_toplevel.set_parent` and xdg-foreign through
`sync_window_role`; published once both nodes exist and cleared when the parent closes) and `from_launcher` (true when
the window redeems an `xdg_activation_v1` token a privileged or trusted connection minted). There is no launch
attribution for `gnoblin.spawn`: the spawner mints no token, so a spawned program is not marked. `screen.toml` gains
`output_scale` (`scale` is the `node` base's three-axis scale), and the compositor publishes `make`, `model`, `mode`
(`WxH@Hz`), `output_scale`, `transform`, `primary` and `enabled` on each screen node beside `name`, `x`, `y`, `width` and
`height` (`Composer::set_output_info`).

**Model.** `Output` wraps a `screen` node; `gnoblin.outputs` is a live `Collection<Output>` with `OutputListener`
(`onConnected`, `onDisconnected`, `onModeChanged`, `onScaleChanged`, `onGeometryChanged`). A `Desk` (module-private) is
what an output shows. Workspaces stay the script's own global list, so one workspace is on one desk: each desk starts on
the first workspace no desk shows and `output.show(ws)` takes a workspace another output shows and gives that output the
workspace this one showed. The Lua desk owned a copy of every workspace, so two outputs could show "workspace 1"; that is
the one behavioural difference, and a workspace set that names the same workspace for two outputs shows it on the last.

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

**Not done.** `Output.apply`, and applying the `outputs` config key: the schema declares `screen.apply` but the compositor
runs no call on a screen node, so a TypeScript declaration would be a no-op. A snapped window's remembered floating
rectangle is not refitted on an output move. The menu's Move to workspace and the rule `workspace` action move a window
with `transferClient` (no refit). `Workspace:walk` and the removal cases are not part of this lane.

## Host API B status

Lane JsHostApiB. User page: `docs/js-idle-auth-launches.md`. The Lua pages are `docs/shell-api/{idle,privacy,input-grab,text-insertion,
launch-feedback,tablet-pad-help,window-snapping,authentication}.md`; every one of them is marked "not available yet" and
the Lua host implements none of them (`design/docs-parity/allow.toml` lists them), so the Lua host was no reference for
behaviour here: the page and the schema were. What is built is what the compositor serves today; the rest is named below
and not invented.

**Built.**

- **`gnoblin.idle`** (`data/js/gnoblin/idle.ts`, `crates/gnoblin-js/src/host_idle.rs`). `watch(ms, { onIdle, onResume,
onClosed })` returns an `IdleWatch`; `inhibit(holder)` returns an `IdleInhibitor`. A watch is a threshold over the
  compositor's `idle` object (`idle_ms`, `inhibitors`): idle is `inhibitors === 0 && idle_ms >= ms`, so a hold delays
  idleness as it delays a screen blank. The shell reads the object over an owner connection of its own
  (`JsClientIdleLink`: one lookup of the object's id on a throwaway connection, then a subscription to that object
  alone), opened when the running generation first has a watch or a hold and closed when it has neither, because every
  change of `idle_ms` goes to every subscriber. The connection is a `JsIdleConnector` in `JsRoleLinks`, so a test
  passes a fake. `JsShell::retire` gives every hold back (`uninhibit`) before it drops the connection. The compositor
  keeps each hold for the connection that made it (`idle.toml`; `Dispatcher::close` and `lost` release them), so a shell
  killed without retiring leaves none behind, and a lost idle connection ends the script's holds (`IdleMonitor.lost`).
- **`gnoblin.authentication`** (`authentication.ts`). Replaced by the observation-only API of the "Polkit status" section: there is no provider, no `allow` and no second slot connection.
- **`gnoblin.launches`** (`launches.ts`). The shell's own registry: `begin`, `list`, `finish`, four handlers, the page's
  limits (token 128, application 512, 100 to 10000 ms, 64 records). Windows answer a launch through two listeners on
  `gnoblin.clients` attached when the module loads (an arrival, a focus, an application ID that arrives late); the
  deadline is checked once a frame from `plan`. The compositor has no launch state (`allow.toml` says "documented, not
  implemented"), so this record is not shared with any other client.
- One entry call a frame, `wants()`, says whether a script needs the idle connection (bit 1); a shell that does not
  pays for that call. `JsShell::run_host_api` is the rest.

**Proven.** Against fakes: `crates/gnoblin-js/tests/host_idle.rs` (16 cases), `host_auth.rs` (16) and `host_launches.rs`
(13), covering the page's rules, the validation tables and the failures first (no connection, a lost one, a refused
hold or claim, a throwing handler, a request open at `retire`). Against the real compositor (`crates/gnoblin/tests/js_shell.rs`, the
idle and launch tests): a watch hears the compositor's own idle clock pass its time, the input that ends it, and the next idle
period; a hold is counted in the compositor's `inhibitors`, keeps the watch from firing with the session idle for far
longer than the watch's time, and a reload that retires the generation gives it back; a shell that dies without retiring
leaves the compositor counting no hold and the next shell's watch hears the session go idle; a launch is started by a
real mapped window and another is timed out by its deadline.

**Gaps.**

- `privacy`, `input-grab`, `text-insertion` and `tablet-pad-help` are not built, because the compositor serves nothing to
  build them on: no privacy object (the PipeWire monitor is a session-bus service, `org.gnoblin.Privacy`, and carries
  microphone and camera only), no key-event stream and no keyboard lease (a seat claim takes a chord), no text-input
  target or `text_target` grant, no `pad-help-requested` event. A guaranteed release of a grab on `retire` or a crash, and a
  test of it, wait for the lease to exist. A privacy API must be read-only whatever it is built on.
- The snapping grant has no counterpart because there is nothing to grant: the shell owns the tree and writes `bounds`
  (`gnoblin/snap`), where the Lua callback holds a one-use capability over a window it does not own.
- Idle: no `watch:watch(field)`, and the `closed` reasons `config_reloaded` and `runtime_stopped` do not exist (a
  generation's watches end with it, without a signal); times are milliseconds. Launch feedback: a launch is not linked to
  the window or process that answered it, as on the Lua page.

## Polkit status

Design: `design/js-research/polkit.md`. A script never decides whether a password was right (J8).

- **The compositor verifies.** `gnoblin-service-polkit` offers each `BeginAuthentication` on `auth.polkit` with a payload
  (`gnoblin-polkit-auth::AuthRequest`). The compositor holds the request itself, draws `PolkitUi` as a secure modal
  (`ipc/secret.rs`, `ipc/polkit.rs`) and runs polkit's `polkit-agent-helper-1` (`HelperSession`) with the cookie on its
  standard input. Only the helper's `SUCCESS` answers the request; the helper has already told polkitd.
- **No script in the path.** A shell cannot claim `auth.polkit` or offer to it (`ipc/requests.rs`). `gnoblin.authentication
.addListener(new AuthenticationListener())` observes through `dialogs.observe` (`authentication.ts` over
  `dialogs.ts`): `actionId`, `message`, `subject`, `identities`, `deadline`, never the cookie, and `request.decline(reason)`
  is the only verdict. `setProvider`, `allow` and `host_auth.rs` are gone, with no alias.
- **Proven**, with a fake helper script and a fake polkitd on a private bus: `gnoblin-polkit-auth` tests, the dialog and
  service unit tests, `gnoblin-compositor/tests/polkit.rs`, `gnoblin-service-polkit/tests/dbus.rs`, and the
  `js_shell.rs` authentication cases. **Not proven:** a real `polkit-agent-helper-1`, a real polkitd, fprintd.

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
- **Not done.** The keyboard Move and Resize modes of the window menu (`reposition.lua`); a fade for a window its client
  closes itself (the node is gone from the compositor at once); dragging a titlebar with a real pointer.

## Stock shell UI status

Lane JsStockShellUi: `Entry`, `Clock` and `Bar` (`data/js/gnoblin/widgets.ts`), the window menu (`menus.ts`), the
not-responding alert (`alerts.ts`), `toast` (`panel.ts`), the overview (`overview.ts`), the launcher (`launcher.ts`), and
the titlebar's gestures, fade and reserved room. User page: `docs/js-ui.md`.

- **Host.** The toolkit's events now include a `key` sent to the canvas that holds the seat's focus (`Canvas.focus()`,
  `ui.focus`, written as the seat's `focus`) and a `request` made of any window (`menu`, with its anchor); a key event
  is never printed. `ui.top` (`Canvas.mountTop`) parents the windows the compositor left without a parent to the root,
  as `Client.raise` does (`adopt_windows`, shared), and puts a canvas last, over them. The host reads the seat's
  `pointer_x`, `pointer_y`, `double_click`, `middle_click` and `right_click`, whether the lock is engaged or its `state`
  is not `not-locked`, and the primary screen's size. A canvas may be `secure`. `Window` gains `unresponsive`, `kill`
  (callable now) and the host's `lower` (`Client.lower()`).
- **Menu.** The pure parts are `menuHeight`, `placeCard`, `MenuNavigator` and `menuKeyAction`; the controller is
  `windowMenu`. It answers `menu` requests, opens one menu at a time on a layer over the screen, keeps the keyboard where
  it was and closes with its window and on a lock. `defineConfig({ menus: false })` never starts it: the compositor still
  sends the requests, and nothing answers them. The compositor does not wait for the shell, so there is no timeout to
  add. A shell that never calls `defineConfig` has no menu, as it has no stock keys. Always on top is kept by the menu
  controller (a pinned window is raised again when another window comes over it), not by the compositor.
- **Alert.** `AlertTracker` is the state machine. The alert reads `unresponsive` a few times a second (the property is
  written, not signalled) and its Force quit calls `Client.kill()`.
- **Titlebar room.** Decision: the compositor's decoration mode is client-side and the tree has no server-side flag, so
  decoration cannot be opted into per client; the stock library keeps the Lua behaviour (every window gets a bar) and
  reserves the room instead of drawing over the content. `client.bounds` is the content, `client.frameInset` the bar,
  layouts place frames; see `docs/js-ui.md`, "The room above the client".
- **Launcher.** With no `launcher` command and no key bound to `Super+space`, the host binds the stock entry
  (`planStockLauncher`; `plan.bindings` is unchanged).
- **Proven against the fake link** (`crates/gnoblin-js/tests/stock_ui.rs`, the cases of `menus_test.lua`,
  `alerts_test.lua` and `ui_test.lua`; the fake compositor moved to `tests/common`): menu opening, anchor and clamping,
  keyboard navigation, pointer selection, submenu, Restore, Minimise, Close, Always on top, secure, lock, replacing
  `items` and `show`, a throwing entry, `menus: false`; the alert tracker, Wait, Force quit and its arming, the alert
  closing on recovery, a closed window and a lock, `alerts: false`; `EntryModel`, `Entry` keys, `formatTime`, `Clock`,
  `Bar`, toast, overview, launcher; the titlebar's seat actions, the content below its bar, `frameInset`.
- **Proven on the real compositor** (`crates/gnoblin/tests/js_shell.rs`, the last two tests): a mapped decorated window's
  content starts where its bar ends, with the bar on screen above it and no overlap; a right click on the bar through the
  session's input path opens the menu, which the compositor holds as painted canvases, and Escape closes it. A client
  that answers no ping gets the alert, Escape (Wait) closes it, and Force quit (armed) closes it: the compositor refuses
  `kill` for its own process, which is this test's, so the kill itself is proven against the fake link only.

## P8a style status

Lane JsStyle. The style model of owner-spec section 6 (`design/js-research/owner-spec.md`) over the existing node
`effects` and `opacity`, with no renderer feature added. Code: `data/js/gnoblin/style.ts` (the classes, the cascade, the
mapping to effects; no host import), the style section of `crates/gnoblin-js/js/gnoblin.ts` (scopes, `Client.style`,
the frame pass), `effects` and `skipped` in `crates/gnoblin-js/src/bindings/props.rs`. Tests:
`crates/gnoblin-js/tests/style.rs` over fakes, `a_global_style_becomes_the_effect_list_...` in
`crates/gnoblin/tests/js_shell.rs` against the real compositor. User page: `docs/js-style.md`.

- **Classes.** `Style`, `ClientStyle`, `Border`, `Shadow`, `BlurFilter`, `SaturateFilter`, `BrightnessFilter` and
  `ContrastFilter` (the spec's cookbook uses `{ contrast }`) are ordinary ES classes exported from `gnoblin`. A literal
  in a list is normalised to its class; `{ blur: 18 }` in a filter list is a `BlurFilter` of radius 18.
- **Scopes and the cascade.** `Collection.style` is the scope's defaults: `gnoblin.clients.style` (global), a workspace's
  `clients.style`, a `where()` selection's `.style` (`where(query, { priority })`), and `client.style` (the client's
  own overrides). A scope holds only what was set on it, so `client.style.cornerRadius` is `undefined` when the client
  does not override it. Precedence, lowest first: built-in defaults, global, workspace, selections by priority and then
  by the order their style was first used (later wins), client. A selection of `workspace.clients` is ranked as a
  selection, which answers spec-review 1.12. `computedStyle` is resolved on every read from the live scopes: a frozen
  object with the value of each property and `origin` naming the scope it came from. `Node.style` (spec-review ST2 `(+)`)
  is not implemented.
- **Unset, `null`, `[]`.** Each property of a scope is a value or unset; `undefined` is unset, so assigning `undefined`
  and `unset(name)` are the same, and `reset()` unsets all. `null` is a value (`background = null` hides what the scopes
  above set), and `[]` is an explicit empty list.
- **Copy on write (decision for spec-review 1.13, owner question 2).** Reading a list a scope does not override returns a
  copy of the inherited list whose elements know they are copies. The first write to one of them makes the copy the
  scope's own list, then applies the write, so nothing leaks to the scope the list came from or to other clients. The
  client detaches from later edits of the inherited list. A copy handed out earlier and written later commits as it was
  read. The cost is that `client.style.shadows[0]` is readable without an override (the review's other option made it
  throw).
- **Observable changes, no Proxy.** A field of an element and a scalar of a style is an accessor: it checks the value,
  throws with the property path (`Client#42.style.borders[2].width: must be finite and >= 0`) and leaves the previous
  value, bumps one module counter (the style epoch) and, on a copy, commits it. An array a script changed in place
  (`push`, `splice`, `list[0] = {...}`) cannot be intercepted: the frame pass (`syncStyleLists`) compares each array
  with its entries at the last check, validates new entries, makes literals elements, and puts the old entries back and
  reports an error for a bad change. Assigning a list copies it: `style.shadows = array` does not adopt `array`.
- **Limits.** `MAX_STYLE_LIST` is 8 per list and 8 effects per node (the backdrop blur and the outer shadows). Numbers
  take the renderer's ranges: blur and shadow blur 0 to 100, spread -100 to 100, grades 0 to 2, opacity 0 to 1.
- **Application.** `applyStyles` runs at the end of `layout()` each frame: it checks in-place changes, then for each
  client compares the style epoch and the names of the scopes it sits under (a `where()` it joined or left, a
  workspace move) with the last application, and only a client that moved is resolved and written. Writes happen only
  when the value changed: `effects` as JSON against the last write, `opacity` against the last resolved value. The
  first application of a client compares with what the node holds (a window an earlier generation styled), writing an
  empty list when the style has no effects and the node has some, and the opacity when it differs: that is how a style
  line removed on reload takes its effect away. `client.opacity` stays the raw node property (animations and the
  overview write it); a style opacity is written only when it changes, so a later animation of the node is not undone
  by an unrelated style change.
- **What a style becomes.** `opacity` is the node's `opacity`. Outer `shadows` are `shadow` effects (`x`, `y` to
  `offset_x`, `offset_y`; `blur` to `blur_radius`; `spread`; the colour as `#rrggbbaa`), listed last to first so the
  first shadow is on top (paint order is first to last). `backdropFilter` is one `blur` effect (documented defaults,
  `downscale` 2, `passes` 3) when a `BlurFilter` has a radius above 0, listed before the shadows; the chain's
  saturate, brightness and contrast multiply into its `saturation`, `brightness` and `contrast`.
- **Not applied, and said so.** The tree has no corner radius, border, background or content filter, and an inset
  shadow has no effect kind; the blur radius is fixed at 12 by the compositor (`BLUR_RADIUS`), so a radius only
  switches the blur on; a grade above 2 is drawn as 2; a saturate, brightness or contrast without a blur has nothing to
  draw. Each stays in `computedStyle`, becomes a line of `client.styleWarnings`, and is reported once per generation as
  a diagnostic `not applied: style.<property>: <reason>` (`takeNotices`, read by `collect_reports`). The compositor's
  own list of effects it skipped (`skipped` in `node.toml`; the task called it `effects_unsupported`) is appended to
  `client.styleWarnings`. GLES draws blur and shadow; the software and Vulkan renderers draw none.
- **Config.** `defineConfig({ style })` is validated by `desugarConfig` (errors name `config.style`) and set on
  `gnoblin.clients.style` before `setup` runs, so `setup` can change it. `rules[].style` (`opacity`, `cornerRadius`,
  `shadows`) is set on the matched client's own style through `RuleHost.setStyle` and `setOpacity`, which replaces
  `RuleEngine.styleOf`: a rule beats every scope, and a rule that stops matching unsets what it set. A rule's `opacity`
  of 1 (what an undone rule restores) unsets the style's opacity instead of pinning it.
- **Proven on the real compositor.** A config with `style: { opacity, shadows }` and a window rule's `opacity` ends as the
  `effects` list and `opacity` of three mapped windows' nodes in the scene (one with a `where()` selection's own
  shadow), and a reload without the lines takes both off. **Proven against the fake link only:** the cascade
  (ST1 to ST5, ST7), copy on write, the mapping, the notices, the 8-effect cap and the rule routing. **Not proven:**
  that a renderer draws the effects (GLES only, not exercised here), `ST6` (secure masking).
- **Not done.** `Node.style` and `workspace.style`; any renderer
  feature (corner radius, borders, content filters, inset shadows, a blur radius, a background), which the owner must
  schedule as renderer work; a titlebar restore animation still ends at opacity 1, not the style's opacity.

## Transitions status

Lane JsTransitions. `gnoblin.requestTransition(callback, options)` of owner-spec section 14 over the existing tween player,
with no native change. Code: `data/js/gnoblin/transition.ts` (the staging, the engine, `Transition`; no host import),
`crates/gnoblin-js/src/transition.rs` (`__gnoblinHost.check`, the tree's validation of a write without queuing it, the
same `prepare` that a queued write goes through), the staging block at the top of `crates/gnoblin-js/js/gnoblin.ts`
(replaces `host.get/set/animate/call/raise/lower` and the spawn, kill and session verbs of `host.api`), `snapshotStyles` in
`style.ts`. Tests: `crates/gnoblin-js/tests/transition.rs` over the fake link, `a_layout_change_in_a_transition_...` in
`crates/gnoblin/tests/js_shell.rs` against the real compositor. User page: `docs/js-transitions.md`.

- **Staging.** While the callback runs every node write is checked at once (so the setter that made it throws and names its
  own path) and kept; reads give the staged value. Then the layout and style passes run once against the staged state
  (`layout()` of `gnoblin.ts`, with a layout's error failing the transition and not just a report), and the staged writes
  are sent in the frame's one transaction, each as a timed write (`host.animate`) when the property is animatable and the
  node has a place, and as a plain write otherwise. The tree starts each track at the value the transform has, which is
  the drawn one, so a retargeted window does not jump.
- **Failure.** The staged writes are dropped, and `snapshotScriptState` puts back each workspace's layout, gaps, name and
  dirty flag, the own fields of the layout it had, and every style (`snapshotStyles`). A layout object the callback
  assigned keeps field changes made to it; an array edited in place is not seen (`fieldsChanged` is shallow).
- **Decisions.** Durations are milliseconds, as the spec says (`Client.animate` keeps seconds). Easing names are those of
  `Client.animate` plus `ease-out`, `ease-in-out` and `ease-in`. The nested call, an async callback and a side effect
  (`call`, `raise`, `lower`, spawn, kill, a session verb) are errors. `set(props)` without options is atomic and immediate.
  `finished` is a promise that never rejects and `onFinished` a queued callback, both with `{ cancelled }`. The end is the
  compositor's `track_ended`, not the shell's clock.
- **Acceptance rows** (`spec-review.md`, "Animation"):

| Row | Status                                                                                                                                                                                                                                                                                                                                |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| AN1 | Done: once, one transaction, a returned Promise refused (`tests/transition.rs`).                                                                                                                                                                                                                                                      |
| AN2 | Done at the player and the shell: the layout is computed once and every window gets timed writes by node id; golden poses at 0, 0.25, 0.5 and 1 for position, opacity, scale and rotation, and a size tween in whole pixels.                                                                                                          |
| AN3 | Not done: there is no camera (native and spec work).                                                                                                                                                                                                                                                                                  |
| AN4 | Done: no JS runs between the commit and the end (the callback count stays 1 across frames); a scrub is one write and `JsShell::dispatches` stays put over three scrubs (fake link); against a real compositor a held transition is scrubbed 0, 0.4, 0.1, 1 and let go, and it ends on the compositor's `track_ended` (`js_shell.rs`). |
| AN5 | Done at the scene: while a track is seeked, `Scene::hit_test` uses the presented rectangle, position, scale and rotation, and a point inside only the target does not hit (`tween_scrub.rs`).                                                                                                                                         |
| AN6 | Done over the fake link: `cancel` from running and from ended, `{ cancelled }` once; `progress`, `play()`, `pause()` and `reverse()` write the `tween`, a transition run back to its start settles cancelled and puts the start values back.                                                                                          |
| AN7 | Done: an invalid value, a stale (destroyed) client and a throwing layout abort with the previous state and the path in the error.                                                                                                                                                                                                     |
| AN8 | Done: the newest owns a property and the older goes on with the rest, one with none left ends cancelled, a nested call is an error.                                                                                                                                                                                                   |

- **Native tweens** (lane TweenNative). `TweenPlayer` seeks to a point of a track, pauses, runs backwards and settles
  (`finish`: forward, back or cancel); a track interpolates `width` and `height` (whole pixels), and `rotation` and `scale`
  per axis as the node stores them; the timed write has `delay`; a `tween` node (`tween.toml`, `design/node-core.md`,
  "Tweens") holds the play state of the tracks that join it; the compositor sends `track_ended`, which `finished` settles
  on. `crates/gnoblin-render/tests/tween_scrub.rs` has the golden poses and the hit tests, `crates/gnoblin-js/tests/transition.rs`
  the scrub, reverse, delay and end over the fake link.
- **Not built**: presented positions read from a running compositor (a tween's pose is not in the tree an observer
  reads, so `crates/gnoblin/tests/js_shell.rs` proves the scrub through the `tween` node and the end through
  `track_ended`, and `crates/gnoblin-wayland/tests/tree_live.rs` proves the poses on the composer with an injected
  clock), tweening effects and corner radius, `Camera2D`/`Camera3D`, a tree surface for `TweenPlayer::finish`, a
  quaternion rotation path. Spec examples 076 to 079 (`arcLayout.radius`, `camera3D`, `display.viewport`, a `View`) need
  objects the runtime does not have.

## P8 console status

Lane JsConsole: `crates/gnoblin-js/src/console.rs` (the server), `data/js/gnoblin/inspect.ts` (the renderer and the
helpers), `crates/gnoblinctl/src/console.rs` (the client). User page: `docs/js-console.md`; the first of the
introspection tools of owner-spec section 18.

- **Server.** `JsConsole` listens on `js.sock` in the directory of the owner socket (`console_socket_path`: the path
  `socket_path` gives, with the file name replaced, so it follows `GNOBLIN_IPC_SOCKET` and `XDG_RUNTIME_DIR`), mode 0600.
  `JsRole::connect` binds it unless `GNOBLIN_JS_CONSOLE=0` (`JsRoleConfig::console`: `Environment`, `Off` or `At(path)`;
  a role built over a fake link has none unless told where). A shell that cannot bind one (the path is served by another
  shell, or not writable) logs a warning and runs without it. A stale socket file is replaced; a live one is refused. The
  file goes with the server.
- **Peer check.** On every accept the peer's uid is read with `SO_PEERCRED` (`rustix`, as `ipc/privileged.rs` does) and
  compared with the shell's own: another uid, or credentials that cannot be read, is closed unanswered (`peer_allowed`,
  unit-tested; a second uid cannot be made in a test). At most 16 connections; a request line is at most 1 MiB.
- **No thread.** The server is nonblocking and polled by `JsRole::tick` before `JsShell::frame`, so an expression runs on
  the shell's own thread between two frames, never beside one (J5's single-threaded runtime needs no lock), and its
  writes (`gnoblin.clients.at(0).x = 10`) leave in the very next frame's transaction. At most four expressions are
  evaluated per turn, and a connection that has not read its replies is not evaluated for. The cost is the loop's own
  cadence: a request waits for the next turn.
- **Budget, no strike.** An expression runs under `JsShellConfig::entry_budget` (100 ms), the listeners' budget (not the
  2 s the research note suggested for a REPL: a console that holds the shell for 2 s is a visible stall). A trip answers
  with a `budget` error and is not counted as a strike (`console_target` does not call `count_strike`): the person at the
  prompt is not a listener the supervisor should restage the generation over, and the restage would drop the session.
  The interrupt is uncatchable, so `for (;;) { try { while (true) {} } catch {} }` is stopped too.
- **Session.** A script is evaluated in sloppy mode as `<console>`: `var`, `let` and `const` at the prompt, and an
  assignment to an undeclared name, are globals of the running generation and persist for every connection until a
  reload; the reload makes a new generation, so nothing the console defined survives it and nothing is written to disk.
  Not per connection: a connection-scoped session would need a scope object per socket and a `with` wrapper, which makes
  `let` at the prompt behave unlike a script. A leading `{` is read as an object literal when the text parses that way (oxc),
  so `{a: 1}` is the object. A promise the expression returns is shown by its result after the pending jobs run (at most
  1000), and `await` at the top level is not supported. TypeScript is not stripped.
- **Rendering.** `entry.render` (the host's `inspect.render`) walks the value once and returns the JSON text of
  `{ type, value?, text, truncated }`: `value` is plain JSON data, `text` is how a terminal shows it. A cycle is
  `[Circular]`; a function is `[Function name]`; `NaN`, the infinities and `-0` travel as their names; a class instance
  carries `$class`. The host registers its kinds with `inspect.register`: a `Client` is
  `Client#3 { appId, title, focused, ... }` (data `{ "$class": "Client", "id": "3", ... }`) and only app id and title
  when it is one of many; a `Workspace` likewise; a `Collection` is an array of those briefs. A getter that throws shows as
  `[Error: message]` for that field. The cap is 64 KiB of text (separators count): what does not fit is replaced by
  `... N more` and `truncated` is true. Strings are cut at 4096 characters.
- **Errors.** `{ id, ok: false, error: { kind, message, location, stack } }`; `location` is the first stack frame in
  `<console>` (so `null.x` on line 3 says line 3, not a frame of the library under it), else the error's own location.
  A syntax error has one too.
- **Inspector.** `inspect.clients()`, `inspect.workspaces()`, `inspect.bindings()` and `inspect.config()` return plain
  data. `bindings()` is `InputContext.describeBindings()` over every seat: id, trigger, kind, owner (`context`, `layout` or
  `device`), the layout's class or the device's name, `priority`, `enabled` and `rank`, which orders the registrations
  of one trigger by the comparator a press uses (tier, then priority, then registration order). `config()` is the config
  `__register` merged over the stock one, kept in the host (`__effectiveConfig`); `null` for an entry that never called
  `defineConfig`. `gnoblin.clients.focused` (a getter on `Collection`) is the seat's focus, and with no seat the member whose
  own `focused` is set. `inspect` is a global of the generation (non-enumerable, like `gnoblin`) and is exported by
  `gnoblin` for `init.ts`.
- **`gnoblinctl console`.** A plain line loop over standard input (`gnoblinctl` has no line-editor dependency and the
  deps gate would count one): the terminal's own editing, an in-memory history (`.history`, `!!`, `!N`), `.exit` and
  Ctrl-D, a `...` continuation after an error that says "end of input", error blocks on standard error with the location
  and a hint. `--eval` prints the value as JSON and exits 0 or 1 (`undefined` prints `null`; `--json` prints the whole
  reply); with no shell it is an error block and exit 1, not 3, because the compositor may well be running. Without a
  terminal on standard input there are no prompts and a failing line exits 1. Arrow-key history needs `rlwrap`.
- **Proven against the real compositor** (`crates/gnoblin/tests/js_shell.rs`): with the stock config over two mapped
  windows, the socket is mode 0600 and the console answers `gnoblin.clients.length` (2), the app ids, a `Client` and
  a collection, `clients.focused` equal to the seat's focus, `inspect.bindings()` with the stock `XF86AudioMute` (owner
  `context`, rank 1) and `inspect.config()`; a syntax error is an error at `<console>:1` and the shell is as it was;
  `while (true) {}` is a `budget` error, then the console answers, a window mapped afterwards is seen (3 clients) and the
  generation id has not changed; `gnoblin.clients.at(0).x = 10` is in the compositor's scene. `GNOBLIN_JS_CONSOLE=0`
  binds no socket, and the socket goes when the role is dropped.
- **Proven against the fake link** (`crates/gnoblin-js/tests/shell.rs`, `crates/gnoblin-js/src/console.rs`,
  `crates/gnoblinctl/tests/console.rs`): the values and their rendering listed above; the cap and `truncated`;
  error locations on lines 1 and 3; bad requests answered without closing; a runaway with a 50 ms budget, a stubborn one,
  and no strike; session variables; a role that serves the console between frames and drops the session on reload;
  `inspect.bindings()` empty on a tree with no seat; the uid predicate; a stale socket file replaced and a live one
  refused. The client against a scripted server: the value as JSON, `undefined` as `null`, failures with the place and a
  hint, the truncation warning, history and recall, continuation, piped scripts, `--json`, a shell that goes away.
- **Not done.** `complete`, `inspect{path|id}`, `gens`, `modules`, `handlers`, `timers`, `stats`, `reload` and `kill` of
  the research note's protocol; top-level `await`; TypeScript at the prompt; per-connection sessions; the inspector's
  views of computed styles, transition tracks and layout placements (the object model has none yet); `displays` and
  `extensions` expressions of spec section 18 (no such objects).

## Console global status

Lane JsConsoleGlobal: `crates/gnoblin-js/js/console.js` (the formatting), `crates/gnoblin-js/src/script_console.rs` (the
limit, the call site and the log), `crates/gnoblin-js/tests/script_console.rs`. User pages: `docs/typescript-shell.md`
("Debugging") and `docs/js-console.md`. A user's first line `console.log("hi")` threw `ReferenceError` until this.

- **Every generation has it.** `JsGeneration::new` evaluates `console.js` as a function expression and calls it with three
  host functions (`admit`, `emit`, `remap`), so `init.ts`, a module, a listener, a console expression and the throwaway
  generation of `evaluate_document` all share one global, defined non-enumerable. It formats in JavaScript because the work
  is about JavaScript values, and it never throws at its caller: a value that cannot be printed (a getter that throws, a
  `toString` that throws) prints a note instead.
- **Methods.** `log`, `info`, `warn`, `error`, `debug`, `assert` (error level, `Assertion failed`, no throw) and `table`
  (the value as one line of JSON, not a drawn table). No `trace`, `dir`, `group` or `time`: the d.ts declares what exists.
- **Formatting.** Like Node for strings, numbers (`-0`), `%s %d %i %f %j %o %O %c %%` (a first-string format applies only
  when there are further arguments), and an `Error`, which prints `Name: message` and its stack with every frame remapped to
  `.ts` coordinates (QuickJS's `stack` has frames only, so the head is added). An object is one line of JSON-like text:
  keys and strings quoted, `undefined`, functions (`[Function: name]`), symbols and `NaN` named, a cycle `[Circular]`, a
  `Map`, `Set` and `Date` shown, a class instance prefixed by its class name, depth 6 and 100 items per level. It does not
  reuse `inspect.render`: that is the host's renderer (a `Client` as `Client#3 { ... }`) and is not there in a bare
  generation, and one formatter that behaves the same in every generation is worth more than the nicer text.
- **The log.** Component `js`, field `area=script`; `log` and `info` at info, `warn` at warn, `error` at error and `debug`
  at debug (the process default is info, so `console.debug` shows with `GNOBLIN_LOG=js:debug`). The message is
  `file:line:col: text`. One call is one record, which may hold newlines.
- **The call site.** The first frame of `new Error().stack` that is not in `console.js`, remapped through the generation's
  source maps (the same `remap` the error stacks use, now a free function `remap_stack`), with the file shown relative to
  the config root. The engine reports a call at its end and the source map rounds that to the nearest token before it, so
  the line is the call's, and the column is a token inside it: the last argument when that is an identifier (`init.ts:2:29`
  for `console.log("a", gnoblin.clients.length)`, the start of `gnoblin`), else the callee's property (`log`). A call that
  spans lines reports the line of its end.
- **Caps.** At most 4096 bytes of text per call, cut on a character boundary with `... (N more bytes)`. At most 200 calls a
  second per generation: the rest are dropped before they are formatted (`admit`), and when the next second starts, or a
  frame ends after it, one warn line says `N console lines dropped`. The second is counted on `JsConfig::clock`, which
  `JsShell::new` sets to the shell's clock (`JsShellConfig::clock`), so a test moves it and no real time passes; the frame
  calls `flush_console` so the notice does not wait for a script that went quiet.
- **The prompt.** `JsConsole` starts a capture around each expression (`capture_console`, `take_console`), and every reply,
  ok or not, carries `output`: `[{ level, site, text }]` in order (`site` is `<console>:line:col`). `gnoblinctl console`
  prints `log`, `info` and `debug` lines on standard output and `warn` and `error` on standard error, before the value; with
  `--eval` all of them go to standard error so standard output stays the value. The lines are in the log too.
- **d.ts honesty.** `declare const console` is in `gnoblin-stock.d.ts`. `tests/dts_match.rs` now compares the value exports
  of `declare module "gnoblin"` (parsed from the d.ts: class, function, const, namespace, enum and re-export names) with
  `Object.keys` of the runtime module namespace in a booted shell, both ways, each exception in an allow-list with its reason
  (`DECLARED_NOT_IN_RUNTIME` names the lane that lands it and fails once it has; `RUNTIME_NOT_DECLARED` is plumbing the stock
  modules share). The first run found: `ArcLayout` and `WorkspaceSwitchBinding` declared with no runtime (removed; they live on
  in owner-spec.md only), a second `class Transition extends NativeObject` and `Easing` from the proposal beside the real
  `Transition` (removed, with the `delay` and `autoplay` options the real one refuses), `NativeObject`, `MemberListener` and
  `DragBinding` declared as importable base classes the runtime does not have (now not exported: types only), `StyleElement`
  real but not re-exported (now exported by `gnoblin`), and `GnoblinError` exported and documented but not declared (now
  declared). The header of the d.ts no longer says "PROPOSED API".
- **Proven.** `tests/script_console.rs` (bare generation and shell over the fake link, reading the log with the level of each
  record): the levels, the `.ts` call site with a type annotation stripped from the line, the specifiers, objects, cycles,
  errors with stacks, `assert` and `table`, a hostile value, the byte cap, the 200-a-second limit and its notice on the
  injected clock, and the notice at a frame end; `tests/shell.rs` (the reply's `output` and the log from a console
  expression, including a throw after output); `crates/gnoblinctl/tests/console.rs` (where each level is printed);
  `crates/gnoblin/tests/js_shell.rs` (a real compositor: a load-time call and a listener's call are in the session log as
  `init.ts:2:29` and `init.ts:8:34`).
- **Not done.** `console.trace`, `dir`, `group`, `time` and `count`; a table drawn as columns; the host's `inspect` text for
  a `Client` in `console.log` (it prints its own enumerable fields); a per-generation log of the last lines for
  `gnoblinctl console` to replay.

## P9 XR status

Design: `design/js-xr.md` (which spec class is a tree node, a JS class or new native work). User docs: `docs/xr.md`,
"Scripting XR in TypeScript". No headset, OpenXR runtime or `gnoblin-xr` binary was used; the program was not run.

- **Scene classes.** `data/js/gnoblin/scene.ts` has `Node`, `Camera3D` and `View` over the tree (`__gnoblinHost.xr`,
  `crates/gnoblin-js/src/xr.rs`: make, insert, destroy and find nodes of those three classes; ids come from the UI
  toolkit's count; `bindings/props.rs` gains the `node`, `view`, `camera` and `controller` property tables). A write of
  what a node already holds is left out, because the tree sends every write it is given. `Vector3`, `Transform3D`, `Ray`
  and `Plane` are the pose arithmetic; a rotation is the program's, Euler about X, then Y, then Z.
- **The module.** `data/js/gnoblin/xr.ts` is the port of `xr.lua` (`arrangeRing` with the Lua golden numbers, `XRRoom`,
  `SpatialLayout`, panels for clients, none for dialogs or secure clients, head-locked panels) and the spec's classes
  (`XRCamera3D`, `XRViewport`, `XRSession`, `XRExtension`, `XRController`, `ViewMoveBinding`, `ViewResizeBinding`). It
  runs once a frame from the host's UI entry and costs one `seats()` call while no program has a session.
- **Presence.** `gnoblin.extensions.xr` is null until the seat named `xr` exists and null again after it goes; a held
  session reads `alive === false`. `gnoblin/extensions/xr` fails at load unless `JsShellConfig::xr_declared`
  (`GNOBLIN_XR=1`, read in `crates/gnoblin/src/main.rs`) or the seat exists.
- **Bindings.** `Binding.readTrigger` is a static hook, so an XR binding keeps the `Binding` lifecycle, `enabled`,
  `priority` and the `BindingCollection` API with its own trigger grammar (`ControllerTriggerParts`, `Grip+Drag`). No seat
  claims one. They are in `SpatialLayout.bindings`, which a script turns off and replaces like any collection.
- **Proven against the fake tree** (`crates/gnoblin-js/tests/xr.rs`, 50 cases): absent XR, the import gate, XR1, the
  Lua room cases and ring goldens, XR3 move (View only), the ray-drag, XR4 resize (zero window writes while dragging,
  one transaction on release, none when aspect and density hold), cancel, throw, disable and replace, XR6 secure, the
  `docs/xr.md` snippets.
- **Proven on the real compositor** (`crates/gnoblin/tests/js_shell.rs`, the two XR tests): an observer makes the `xr` seat,
  the stock shell builds the room with a head camera and a View per real window, and the room goes with the seat. The
  seat is the only thing of the program in that test.
- **Controllers (lane XrControllers).** The program publishes `controller` nodes (`design/js-xr.md`, "New native work",
  now built): the `controller` schema class, an observer may make and write it, `extensions/gnoblin-xr` writes one for
  each hand and obeys `controller.claimed`. Proven against the fake runtime and a fake owner, and on the compositor's
  wire (`tests/tree_ipc.rs`).
- **Claims (lane JsXrClaims).** The host converts a script's array of strings to the tree's `list<string>` value
  (`JsScriptValue::Texts`, checked against the property's allowed names) and `controller.claimed` is writable.
  `XRController.claim(owner, names)` and `release(owner)` keep one claim for each owner and write the union; the runtime
  claims the buttons of a drag binding's trigger when `onStart` accepts and releases them on end, cancel, a throw, the
  binding's removal, the session's end and `retire`. Proven on the fake link (`crates/gnoblin-js/tests/xr.rs`, the
  "Claims" cases: claim and release of both stock bindings and a ray drag, two owners, the typed errors, every way a
  drag ends, `retire`) and on a real compositor (`crates/gnoblin/tests/js_shell.rs`, `the_stock_grab_claims_the_grip_...`:
  an observer plays the program, the stock shell's `ViewMoveBinding` writes `claimed = ["grip"]` on the real `controller`
  node, the compositor refuses the observer the same write, and the list is empty again when the drag ends and when the
  shell dies). A shell's detach now clears the `claimed` of every controller (`reap_transactions`). The program's
  withholding of a claimed grip from the window is proven against the fake runtime and a fake owner only
  (`extensions/gnoblin-xr`, `bridge/controller_nodes.rs`, `xr-input/tests/hands.rs`); no real program, runtime or headset
  ran it. **The claim now comes before the press (lane XrGrip; design/js-xr.md item 6).** The program holds every press
  of the trigger, grip, face button and stick for `CLAIM_HOLD` (150 ms of display time) and forwards it only if no claim
  came, a tap as a press then a release, so a grip the shell claims never reaches the window and an unclaimed one reaches
  it at most one frame after the hold. `session.chordSettleMs` is a getter and setter capped at 60 ms, so the claim fits
  inside the hold (`the_chord_settle_time_is_a_number_...`, `a_drag_claims_its_grip_after_the_settle_time_...` in
  `tests/xr.rs`). The real-compositor claims test is unchanged: an observer plays the program there, so it cannot show
  the program's hold, which is fake-runtime only. No headset was used.
- **Not built.** The TypeScript rules have no `xr = false` key (`accept` replaces it); `ViewRotateBinding`,
  `ViewScaleBinding` and `ThrowViewBinding`; a View of a workspace. XR5 (a stalled script does not block pose writes or
  layers) is unverified.

## Host API A status

User docs: `docs/js-host-api.md`. Rust: `crates/gnoblin-js/src/host_{api,process,session}.rs` (`__gnoblinHost.api`, JSON in and
out). TypeScript: `data/js/gnoblin/{operation,process,session}.ts`, wired in `js/gnoblin.ts` (`HostApi`, `apiFrame`).

- **Session.** The shell holds one more owner connection (`JsClientSessionLink`, a thread of its own). It subscribes to the
  whole graph, keeps the `session` object's lock properties for `status()`, calls `logout`, `poweroff`, `reboot`, `suspend`
  and `hibernate` on it and forwards every signal. `lock` and `reload-config` run on short connections of their own, so a
  reload that waits for this very process cannot hold the status thread. A refusal is an `Operation` failure with the
  code of `docs/shell-api/errors.md`.
- **Frame.** `run_host_api` runs before the layouts: it drains the session and process events into `apiFrame`, which settles
  operations, delivers signals and exits and fires due timers, all as queued calls under the call budget.
- **Processes** are children of the runtime (a waiter thread each, process groups, timeout, captured output). A captured
  stream is a string. `gnoblin.spawn` returns the handle; the host spawner now serves `autostart` only.
- **Reload.** A successful reload replaces the generation that asked, so its callbacks are gone; a refused one reaches them.
- **Proven on the real compositor** (`js_shell.rs`): status, a timer, two processes with exit events and captured output,
  and `reloadConfig` through the compositor's `reload_requested` signal to a typed `config_rejected`. The power verbs and
  `lock` are proven against a fake link only: calling them for real would suspend the machine.
- **Not built.** `process.on("changed")`/`watch`, `Window.process`, `session.activity`, `capabilities`, `focus.policy`,
  `runtime.status`, `settings`. `gnoblin.requestTransition` landed afterwards (see "Transitions status").

## Real-compositor tests

`crates/gnoblin/tests/js_shell.rs` runs the shell against a headless compositor in the test's own process, and
`crates/gnoblin/tests/support/mod.rs` holds what that file and `js_guardian.rs` share. The tests of a file run in
parallel, one compositor each, so nothing is shared between them: every session, config directory and file lives in a
temporary directory of the test's own, and no test reads the machine's runtime directory or config.

- **`Session`** is `Compositor::run` on a thread, with its IPC, Wayland and EIS sockets in a private directory and an
  input tap that feeds the seat what a device would send. Its liveness pings are off, because a test thread that a busy
  machine did not schedule is not a hung application. The one test about the not-responding alert turns them on.
- **`Shell`** is a `JsRole` ticking in a thread of its own, over a config directory of the test's. `Shell::start`
  returns once the first frame is over. What `generation()`, `diagnostics()`, `view()` and `probe()` read is the snapshot
  the thread published when its last frame ended. `probe()` is what `globalThis.probe()` returned in the test's
  `init.ts`, for a state that the compositor's tree does not show.
- **The scene** is read back through an observer connection (`scene`, `rect_of`, and `Watcher`, which stays attached and
  follows the changes), so a test asserts what the compositor holds and not what the shell believes.
- **`Mapper`** is a real Wayland client that maps toplevels with a buffer and answers pings. `Mapper::pointer()` says
  what its pointer was told: whether the compositor has the pointer over one of its windows, and how many presses it was
  given. A press that a claimed chord took for the shell is never given to the client, which is how a test tells a
  claimed press from one that came before the claim.
  `Mapper::keyboard()` says the same of the keyboard: the key presses the compositor gave the client. It withholds the
  last key of a claimed chord, so a key the client heard is one nobody holds, which is how a test tells a released claim
  from one that is still held. `Session::start_logged()` keeps what the compositor logs (every event, so only a test that
  reads the log asks for it) and `Session::finish()` returns it, for a test that asserts a refused claim never logged.

### Wait on state, with a deadline

A test never sleeps for a time it guessed, and never counts frames to decide that something happened. It waits for the
state it needs with `wait_for` (the shell and the scene) or `wait_for_scene` (a `Watcher`), bounded by `PATIENCE`. A run
that passes never waits near it: it is the margin a loaded machine gets, so that a failure is a state that never came and
not a slow turn. The failure names what it waited for and the last state it saw: the shell's diagnostics and view, the
compositor's scene and what the shell logged.

| The test needs                         | It waits for                                                            |
| -------------------------------------- | ----------------------------------------------------------------------- |
| A window placed                        | the window's rectangle in the scene                                     |
| A reload                               | `generation()` above the one the shell had, and the effect, in one wait |
| A refusal or an error                  | `diagnostics()` to hold the text                                        |
| A press the shell must hear            | `Mapper::pointer().over()`, then one click                              |
| A claimed pointer chord                | the window to follow, or the press to reach the client (`begin_drag`)   |
| The compositor to take a shell's close | the fallback policy's placements                                        |
| A state only the script knows          | `Shell::probe()`                                                        |
| A dialog request judged                | the compositor to answer it with something other than `busy`            |

The rules behind the table:

- **A snapshot is behind the compositor.** The effects of a frame reach the compositor while the frame runs, and the
  shell publishes its snapshot when the frame is over. A test that has seen an effect in the scene and then reads
  `generation()` may read the snapshot from before the frame that made it, so the generation is waited for with the effect.
- **Budgets are wall-clock.** The shell stops a call, a frame or a staging that outlives its budget, and a loaded machine
  that does not schedule the thread spends it on an honest script. `Shell::start` gives every budget `HONEST_BUDGET`. Only
  a test about a runaway script sets its own (`RUNAWAY_CALL`, `RUNAWAY_STAGE`), and keeps them far above what an honest
  staging takes.
- **Save a config file the way an editor does.** `save` writes under a name the watcher ignores and renames over the file,
  so a reload reads the old text or all of the new. A save of two files is two events: when a reload between them must
  not read a broken `init.ts` with new places, save `init.ts` first.
- **Do not hammer a booting compositor.** The shell waits a few seconds for its attach, and a compositor that is busy with
  a test's requests answers it late. A test asks nothing of the dialogs before `Shell::start` has returned.
- **A check that something did not happen waits for what must have happened first**, such as the next window placed or the
  reload's generation, and then looks. Where there is no such state it lets the client answer for a short span (`idle`).

A change to these tests is proven with `cargo test -p gnoblin` run several times in a row, once while two busy loops per
CPU run beside it, and once with `--test-threads=1`.

## Lock status

The session lock screen (lane JsLock; `docs/session-lock.md`, `design/js-research/popups-portals.md` row 9, decision J8).

- **Lock screen.** `data/js/gnoblin/lock.ts` ports `lock.lua` and adds the clock: `LockScreen` (`frame()` once a frame, by
  `runLockFrame` ahead of the toolkit's frame), `PasswordEntry` (a `Box` that keeps typed characters as code points and
  overwrites them when it submits), `LockStyle`. It reads the `lock` node's `state` (`__gnoblinHost.lock`,
  `crates/gnoblin-js/src/lock.rs`), mounts its scrim under the lock node so the compositor draws it over the cover, writes
  the node's `engaged`, takes the seat's focus for the entry and gives it back, and sends the password as the call's secret
  text. It uses the toolkit's `Canvas.onKey` (`UiKeyEvent`), `uiFocus` and `uiFocused`; a key event reaches only the canvas
  that holds the seat's focus.
- **Rust side.** The cover now draws, for the compositor's own lock only, the canvases under the `lock` node once the shell
  holds the tree and has engaged the lock (`Composer::compose_lock_ui`; it redraws when the subtree changes). The lock card
  (`LockUi`, `LockCardService`, `lock_card.rs` in `gnoblin-wayland`) is wanted when the lock is the compositor's own and
  proven and no shell holds the lock screen, with a 1 s grace while a shell is attached. A shell that detaches has its
  `engaged` released. The card is the last layer of the frame and takes every key while up; a shell may not use the card's
  reserved check token. `ipc/localtime.rs` reads the time zone (TZif and its POSIX footer) because the compositor links no
  time library and forbids unsafe code.
- **Limits, honestly.** A JavaScript string cannot be wiped: the password is in the script engine's heap in the string
  built for the `authenticate` call and in the key events that carried it, until the collector frees them. The entry's own
  buffer is cleared, and the Rust card, which has no script, wipes. A `SecureEntry` node (popups-portals.md, 4.3) would
  remove it. The lock screen is part of the runtime: there is no config key to turn it off yet.
- **Proven against the fake link** (`crates/gnoblin-js/tests/lock.rs`, 23 tests): the Lua test's cases (engage, wait for a
  screen, attach while locked, unlock before the screen, external unlock and a later lock, Enter sends the typed password as
  secret text, ok, wrong password, rate limited) and the clock, focus, keys after the unlock and password-in-the-tree cases.
- **Proven against the real compositor** (`crates/gnoblin-compositor/tests/lock_card.rs`, `crates/gnoblin/tests/js_lock.rs`,
  scripted password check in place of PAM): no shell, right password unlocks; wrong password stays locked and the card
  still works; a shell that holds the lock screen keeps the card away and its death brings the card; a shell that attaches
  takes over; the JavaScript shell paints its screen under the lock node, answers a wrong password on it and unlocks.
- **Not proven.** Pixels: no test reads the output picture of the card or of the shell's screen. Real PAM. Several
  outputs: both draw on the first output. A shell reload while locked (the grace period is tested only by the policy, not
  by a reload).

### Unit test timing

The tests of `crates/gnoblin-js` (`tests/**` and the `#[cfg(test)]` modules of `src/`) run on the same busy machines, and
they follow the rule of the section above: no assertion may rest on a wall-clock assumption under one second, and nothing
sleeps for a time it guessed. A test that passes alone and fails one run in twenty hides the regression it would have
caught, so this was measured, not assumed: `cargo test -p gnoblin-js` ten times in a row passes, and four times while two
busy loops per CPU run beside it passes. Both are the proof a change to these tests needs.

- **Time is injected.** `JsShellConfig::clock` is the one clock the scripts read: `gnoblin.timers.now`, and through it every
  timer, `after`, fade, slide, alert arming window, launch deadline and focus delay. It is a `JsClock`: `JsSystemClock` in
  the product, and in a test a `JsManualClock`, which stands at zero until the test calls `advance`. The rigs of
  `tests/common` build one (`timed_config`, `Rig::advance`), so a test of a slide or an arming window says how much time
  passed, and no real time does.
- **An honest script gets `HONEST_BUDGET`, 5 s,** for each call and 10 s for a frame (`honest_config`, and
  `honest_role_config` for a role). The product's 100 ms per listener call and 500 ms per frame are a limit on a script
  that hangs the shell, not on a machine that did not schedule the thread for a tenth of a second.
- **A test about a budget runs a script that never ends,** under a short budget (`RUNAWAY_BUDGET`, 500 ms: not shorter,
  because the honest calls the same budget covers must not be able to trip it), and asserts that the script was
  interrupted, never when. A script that must outlast a frame budget outlasts it by a factor of two, and the frame budget
  is a few hundred milliseconds so that no other frame of the test comes near it. Where a loop could hang the run if the
  interrupt failed, the loop ends by itself after 30 s.
- **The shell installs its bundled library under at least 5 s** (`HOST_INSTALL_BUDGET`), whatever `stage_budget` says, so
  a stage budget shortened to trip a runaway entry does not fail the boot.
- **Real time stays only where the code under test reads the real clock:** a file watcher's debounce (one second in the
  tests: the burst is written in microseconds, and a stall longer than the debounce would split it) and a spawned
  process's own timeout. Those tests wait for a state, with a deadline of seconds.

## Live verification

Lane JsLiveVerify. Until now the shell was proven by scene-tree assertions; nobody had looked at it. This section is what was
seen on pictures of the shipped binary, how to take them again, and what is wrong with them.

### How the pictures are taken

`scripts/js-shell-screenshots.sh [OUT_DIR [SCENARIO...]]` boots the release `gnoblin` as a session (the guardian, which
starts the compositor on its headless backend with the software renderer, the services role and the runtime role that runs
the JavaScript shell), starts real Wayland clients (`foot`, running `gnoblinctl` commands so the windows show real output)
and takes each picture with `grim` on the compositor's privileged socket (`gnoblin-wayland-privileged.sock`, the same path
as `docs/troubleshooting.md`), which is the compositor's own screencopy. No `init.ts` is present unless a scenario writes
one, so the zero-config scenarios run the stock library. Every picture is saved with `NAME.windows.json` (the compositor's
window list), `NAME.clients.json` (the shell's `x`, `y`, `width`, `height`, `frameInset`, `tiled` and workspace for each
client, from `gnoblinctl console --eval`) and `NAME.log` (the session log), so a picture that looks wrong has its numbers
beside it. It needs `grim`, `foot` and `dbus-daemon` (and `wtype` for the menu); `shellcheck` is the lint.

On an Agent37 instance (Debian 12, gVisor, no GPU):

```sh
apt-get install -y --no-install-recommends grim foot wtype dbus dbus-daemon fonts-dejavu-core libegl1 libgles2 libegl-mesa0 libgl1-mesa-dri
a37-gate.sh run ID TREE build --release -p gnoblin -p gnoblinctl
a37-gate.sh in ID TREE scripts/js-shell-screenshots.sh /home/node/shots
a37 pull ID /home/node/shots/master-stack-decorated.png master-stack-decorated.png
```

Any Linux box with the build works the same way: `GNOBLIN_BIN_DIR=target/release scripts/js-shell-screenshots.sh ./shots`.
Each scenario boots its own session over a fresh directory, so one can be run alone by name.

| Scenario                                      | What it shows                                                                             |
| --------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `one-window`                                  | zero config, one window, its titlebar                                                     |
| `cascade`                                     | zero config, three windows of the default size                                            |
| `master-stack`, `dwindle`                     | three windows, then a reload to the owner's `defineConfig` (no `decorations` key)         |
| `master-stack-decorated`, `dwindle-decorated` | the same with `decorations: true`                                                         |
| `shadows-software`, `shadows-gles`            | `style.shadows` on each renderer (the renderer is read at start, so the session restarts) |
| `csd-double`, `csd-own`, `csd-tiled`          | foot as it comes: the shell's one bar; with decorations off, foot's own bar (off screen)  |
| `reload-tiled`, `transition-mid`              | a reload of three tiled windows (before and after compared); a frame mid-transition       |
| `window-menu`                                 | the window menu, opened with `Alt+Space` sent by `wtype`                                  |
| `lock`                                        | `gnoblinctl lock`, then a capture, which is refused                                       |

The pictures (`/home/kieran/dev/lanes/JsLiveVerify-shots/` on the workstation of the lane) are not in the repository except
four that are clean product states and are in `docs/typescript-shell.md`, "What it looks like": `docs/images/js-shell/`.

### What was seen

- **Zero config, one window (`one-window`).** A 700 by 500 window under a 28 px titlebar (`#414559` while focused, title in
  `#c6d0f5`, minimise, maximise and close at the right), content directly below it, nothing over it, and black everywhere
  else: the stock shell draws no wallpaper and no bar. The window sits at the screen's top-left corner. Text is crisp and
  not clipped.
- **Master-stack and dwindle with decorations.** The bars lie above each tile, the focused bar is lighter than the others,
  the 12 px gap is exactly 12 px black, windows touch the screen's edges, and no tile overlaps another.
- **Window menu.** A card with rounded corners, two groups of entries and a submenu mark, text not clipped, opened at the
  top-left corner of the window's content because `Alt+Space` has no pointer to anchor to. The titlebar's right click is
  not drawn: a shipped session has no way to inject a pointer (its EIS socket stays shut until a remote desktop session is
  approved, and `gnoblin-wayland` offers no virtual pointer), so it is proved by the scene in
  `crates/gnoblin/tests/js_shell.rs`, `a_decorated_windows_content_lies_below_its_bar_and_a_titlebar_right_click_opens_the_window_menu`.
- **Shadows.** The software renderer draws none (as `docs/js-style.md` says). GLES draws them: the session was started
  with `compositor.renderer = "gles"` on Mesa's llvmpipe (surfaceless EGL works under gVisor), and the shadow is visible on
  the window below it.
- **The lock screen and the lock card (`lock`).** Not drawn. A locked session refuses every capture (`grim` says
  `failed to copy output HEADLESS-1`), by design: no pixel leaves a locked compositor. `docs/session-lock.md` says no test
  reads their pixels and that is still so; an in-process capture that skips the embargo would be a way around the rule
  being proved, so it was not added.

### Defects

| #   | Picture                           | Diagnosis                                                                                                                                                                                                                                                                                                                                                          | State                                                                                                                                                                                                                                                                                                                                             |
| --- | --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `master-stack.png`                | A black column at x = 1023: `tile()` floored each width on its own, so a master of 607.2 and a stack of 404.8 at 619.2 ended at 619 + 404 = 1023. Every tiled layout had it (grid with 3 columns on 1000 px ended at 999; the Lua shell had the same).                                                                                                             | Fixed in `data/js/gnoblin/layout.ts` (`tile` floors edges). Tests: `stock_layouts.rs`, real: `a_master_stack_workspace_with_a_ratio_and_a_gap_fills_the_screen_to_both_edges`. The Lua goldens that pinned the sliver are changed.                                                                                                                |
| 2   | `cascade.png` (before the fix)    | Three windows of the default size: the second and the third landed at exactly (324, 268). `placeRect` pushes a cascade that found no free spot back inside the area, pinning every later window to the far corner, so the third hid the second.                                                                                                                    | Fixed in `FloatingLayout.onClientAdded` (`apartFrom` steps back by the cascade step). `placeRect` and its golden table are unchanged. Tests: `stock_layouts.rs`, real: `windows_too_big_for_a_free_spot_cascade_and_never_land_exactly_on_each_other`.                                                                                            |
| 3   | `master-stack.png`, `dwindle.png` | The owner's `defineConfig({ workspaces })` draws no titlebars. `docs/typescript-config.md` says so in its table (`decorations` is "not layered") and contradicts itself two paragraphs above ("A key you leave out keeps its default"). With foot as the client, foot then draws its own title bar, which is outside its window and off screen for the top row.    | Open, by design, owner's call: should `decorations` default to `true` for a config that leaves it out? The doc and the screenshots in `docs/` say what is.                                                                                                                                                                                        |
| 4   | `csd-own.png`                     | Fixed: the window is its geometry, so foot's bar is on screen and the window is 700 by 500 (below, "The 52 px, fixed").                                                                                                                                                                                                                                            | Fixed in the compositor: the window node is the window geometry. Test: `a_client_side_decorated_window_is_held_at_its_window_geometry_not_its_buffer_less_its_bar`, no longer ignored.                                                                                                                                                            |
| 5   | `shadows-gles.png`                | The shadow is the window node's, so it is cast by the content and not by the frame: it has nothing at the titlebar's side. The shadow effect has no rectangle of its own to grow.                                                                                                                                                                                  | Open. Needs the effect to take an extent, in the compositor.                                                                                                                                                                                                                                                                                      |
| 6   | `master-stack.log`                | A reload logged 19 `a seat claim's chord is already bound for that device` and one `a seat gesture claim was refused` at WARN. The new generation claimed each stock key while the old one still held it, so the compositor refused the duplicate, and the old generation's release then freed the chord: after the reload nobody held it and the key did nothing. | Fixed (lane JsReloadClaims). A staged generation's claims wait for the swap and go out behind the old generation's releases, in one transaction ("Hot reload", "Seat claims across a reload"). Tests: `crates/gnoblin-js/tests/input.rs`, real: `a_reload_hands_every_seat_claim_over_so_each_shortcut_fires_once_and_a_removed_one_is_released`. |
| 7   | every `.log`                      | The guardian wrapped each INFO line a child prints on stderr as `WARN guardian/raw INFO ...`, so a healthy start was a page of warnings: `is_structured` looked for the bracketed form `INFO  [role] ...`, and the console prints `INFO   role/component  message`.                                                                                                | Fixed (lane JsReloadClaims) in `crates/gnoblin-session/src/raw.rs`: a line the child formatted itself is passed on with its own level. Tests in the same file.                                                                                                                                                                                    |

**Defect 4 (double title bars) is fixed** (lane SsdNegotiation; `docs/js-ui.md`, "Who draws the frame"). The compositor now
negotiates `xdg-decoration`: a client that asks for server-side is told so only while a shell that holds the tree has
written the main seat's `decorations_available` (the stock shell does when `decorations` is on, after each generation
starts, and takes it back when a generation retires; the compositor clears it when the shell's connection drops), and is
told client-side otherwise. The window node publishes `decoration` and `decoration_wanted`; the stock `Decorations`
mounts a `Titlebar`, and `frameInset` reserves room, only on a `server` window. The answer is sent again to existing
windows when the declaration changes. Proven by scene and protocol tests: `crates/gnoblin-wayland/tests/decoration.rs`
(a real `xdg-decoration` client: no shell, shell without declaration, declared, client-side ask, no preference, no
manager, decorations off and on, shell connection drop), `crates/gnoblin/tests/js_shell.rs` (a client that draws its own bar
gets none, a reload toggling decorations) and `crates/gnoblin-js/tests/stock_ui.rs`. It is re-taken on pixels below.

### Re-verified on pixels (second run)

The release binary of the tree at `origin/rust-compositor` after SsdNegotiation, the reload claim hand-off, the console
global, tiling bindings, transitions, outputs and the idle fixes, on an Agent37 instance, every scenario of
`scripts/js-shell-screenshots.sh` (pictures in `/home/kieran/dev/lanes/JsLiveVerify-shots/`, four in `docs/images/js-shell/`):

- `one-window`, `csd-double`, `csd-tiled`: foot as it comes asks for server-side and gets exactly one bar, the shell's
  (28 px, above the content). `csd-tiled` (decorations and a dwindle workspace) fills the screen under one bar.
- `master-stack-decorated`, `dwindle-decorated`: one bar above each tile, 12 px gaps, edge to edge (the 1 px column of
  defect 1 stays fixed). `cascade`: the third window no longer hides the second (defect 2 stays fixed).
- `csd-own` (foot, `decorations: false`): **no bar is visible.** Foot draws its own, but its bar is a subsurface above its
  main surface and the window sits at y = 0, so the bar is off screen, and the window is 448 high, not 500. That is the
  height loss of defect 4's first run, which was still there on this run and is fixed since: see "The 52 px, fixed".
- `reload-tiled`: three tiled windows before and after `gnoblinctl config reload` are byte-identical PNGs.
- `transition-mid`: a `requestTransition` moving a window 600 px right over 6 s, captured 2 s in, shows the window at
  x = 212 (linear: 200), and `transition-end` shows it at 600. The script catches the middle frame by a long duration on
  the real clock; the injected clock is not reachable from a shipped session.
- `window-menu`: as before, opened with `Alt+Space`. Not run: the `Super`+drag swap and the titlebar right click, because a
  shipped session has no pointer injection (the EIS socket is shut until a remote desktop session is approved and no
  virtual pointer is offered); both are proved in `crates/gnoblin/tests/js_shell.rs`.
- `lock`: still refused (`failed to copy output HEADLESS-1`). The lock card cannot be read back on pixels from this tree:
  `ControlHandle::screenshot` reads the frame while locked (`crates/gnoblin-wayland/tests/session_lock.rs` does), but the
  card is drawn by the compositor crate's `LockCardService`, whose test `Session` runs `Compositor::run` and does not hand
  out the control handle. A test would need that handle exposed; the card's colours are therefore unproven.

### The 52 px, fixed

A client that draws its bar in a subsurface above its main surface (foot: `set_window_geometry(0, -26, 700, 500)`, a 700 by
474 buffer) has two rectangles, and the tree knew one. `publish_geometry` (`crates/gnoblin-backend/src/render/compose.rs`)
published the window node's `width` and `height` as the **buffer** size, 700 by 474, and the node's position as the main
surface's origin. The shell writes a window's `width` and `height` back, and the compositor sends them as the
`xdg_toplevel.configure` size, which is a **window geometry** size: 474 asked of foot gave a 448 buffer, with its 26 px bar
inside the 474. Each round trip lost the bar's height, and a window placed at y = 0 had its bar at y = -26.

The fix is in the compositor, at the root: **the window node is the window geometry.**

- `x`, `y`, `width` and `height` of a window node are the `xdg_surface.set_window_geometry` rectangle in global
  coordinates, and the whole surface while the client set none (xdg-shell's default). `publish_geometry` publishes that,
  `window_node_size` (the size a configure asks for) is that size, and `window_frame` is that rectangle. A configure then
  asks for what the tree publishes, so the round trip is stable.
- The surface is drawn at the node's origin less the geometry's offset (`Drawn::local` in `push_surface`, 3D models
  included), a surface hung from it (a subsurface or a popup of the window's own surface) is placed in the node's plane
  (`tree_origin`), and what lies outside the node is the node's **draw extent**: it is drawn and damaged with the node
  (`Scene::set_draw_extent`; a commit damages the shadow) and is not the window. The node's hit extent is the surface
  tree's bounds in the same plane, so the pointer still reaches what the client's input region admits outside the
  geometry (a GTK resize border), and `surface_under` turns the node's point back into the surface's coordinates before
  the input region and the subsurfaces are asked. A popup is found from the surface's own origin, the frame less the
  geometry's offset (`window_popup_under`).
- Fallback placement, cascade, centring, tiling, maximise and fullscreen already worked in geometry (the policy's record is
  the geometry, adopted from `set_window_geometry`), so a maximised window fills the output with its geometry and the
  client's shadow lies past the screen's edge.
- A capture of a window (screencopy, toplevel capture, a cast) copies the output's pixels over the window geometry, so
  the shadow outside it is not in it; a View of a window draws its own surface cut to the geometry
  (`view_crop`, `geometry_crop`), and its subsurfaces and popups whole. `docs/shell-api/windows.md`, "Window geometry and
  capture", states the rule.

Proven by (each case fails on the tree before the fix, except the pixel cases of a shell-less session, which were already
right and are kept as guards):

- `crates/gnoblin-backend/src/render/surface_tree_tests.rs`: the published rectangle for geometry origins (0, 0),
  (0, -26) and (12, 12) with larger buffers, an unset geometry, a changed geometry republished, the surface drawn at the
  frame less the offset, damage of the shadow, a bar above the surface, a popup, the pick and its hit extent, maximise
  filling the output, and a View cut to the geometry.
- `crates/gnoblin-wayland/tests/window_geometry.rs` (real client, a shell that holds the tree): the node a shell reads for
  each origin, an unset geometry, a geometry committed later, five configure round trips that lose no bar, and the
  pointer in surface coordinates with and without an input region over the shadow; `tests/surface_tree.rs`: the pixels of
  a shadow margin, a bar above the surface and a popup anchored in the geometry, on both renderers.
- `crates/gnoblin/tests/js_shell.rs`, with the shipped shell: `a_client_side_decorated_window_is_held_at_its_window_geometry_not_its_buffer_less_its_bar`
  (no longer ignored), a bar, a shadow margin and no geometry, five round trips, and fullscreen filling the screen.

Live, `scripts/js-shell-screenshots.sh csd-own` on the fixed tree (release build on an Agent37 instance): foot's own bar is
on the screen at the top-left corner, and `csd-own.windows.json` and `csd-own.clients.json` say the window is 700 by 500 at
(0, 0) (before: 700 by 448, its bar at y = -26). `csd-double` is unchanged: the shell's 28 px bar, the window at
(0, 28), 700 by 500. The picture is `docs/images/js-shell/csd-own.png`.

## Questions for the owner (the agent has a default for each)

1. Initial workspace rule for a new toplevel. Default: the workspace shown by the display under the
   pointer.
2. Inherited style lists: copy on write when an element is mutated. Default: yes. Done: "P8a style status".
3. `where()` queries are data only, so Rust can evaluate them. Default: yes.
4. Trigger grammar. Default: the existing Lua shortcut grammar, typed as template literal types.
5. Port `prompts.lua`? Default: no. Rust draws consent. See `popups-portals.md` section 8 for the
   other eleven questions.
6. Window-rule patterns: `gnoblin-config` depends on `gnoblin-lua-pattern` today. Default: plain
   globs and regular expressions in `rules`. Done: see "Focus, rules and snapping status".
