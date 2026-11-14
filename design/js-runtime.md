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

Declared only: `Binding` (the four lifecycle hooks; `onStart` declines) and `BindingCollection` have no input wiring, so
the `triggers` option of the layouts is stored resolved but binds nothing and the stock drag behaviours (move, resize,
swap, split resize) are not written; `style` and `outputs` are stored and have no native effect; `rules`, `on` and
`setup` are in the plan for the host to apply. Not ported: `Dwindle` (it needs per-workspace tree state), `Strip`, the
launcher (`Super+space`), `Super+Left` and `Super+Right` (Strip only) and the `Swipe3` workspace slide, which depend on
shell classes that have no JS counterpart yet.

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
compositor in `crates/gnoblin-js/tests/shell.rs`; it has not been run against the real compositor.

- **Boot order.** `JsShell::new` attaches, drains the snapshot and loads `gnoblin/host`, so `gnoblin.clients` exists
  for the entry. `load_entry` evaluates `init.ts`. The first `frame()` announces `ready` for the snapshot's nodes,
  parents first. Then every `frame()` applies the compositor's batches to the mirror, tells the listeners, and sends
  one transaction.
- **Protocol rules.** Copied from `ShellVm`, not shared: one transaction in flight, replies matched by position,
  `stale-base` and `unknown-node` refusals sent again ahead of the next frame's writes, `STALE_RETRIES` (3) in a
  row. The tests are the Lua ones ported (hold, stale resend, retry cap, unknown node, refusal report).
- **Objects.** One `Client` per window node and one `Workspace` per workspace node for the life of the generation,
  so the same node is the same JS object. A client has `id` (the node id as a string), `appId`, `title`, `focused`,
  `fullscreen`, `x`, `y`, `width`, `height`, `bounds`, `workspace`, `alive`, `close()`, `focus()`. Only the four
  geometry numbers are writable; the compositor owns the rest. A write is queued and read back at once, and travels
  at frame end. A destroyed node keeps its last values, is `alive === false`, and every write or call throws a
  `GnoblinError` (`code` `destroyed`, `target` `Client#3.x`). A value that is not a finite number throws a
  `TypeError` naming the member. `Client` and `Workspace` cannot be constructed by a script.
- **Collections.** `gnoblin.clients` and `gnoblin.workspaces` are live `Collection`s: `length`, `at`, `get`, `find`,
  `filter` and `map` (snapshots), iteration, `where`, `addListener`, `removeListener`. They have no `add`.
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
- **Not done.** Input events, chords, gestures and call answers from the compositor are consumed and dropped: the
  binding lane owns them. `Workspace` has no `clients`, layout or `add`. No coalescing of repeated writes to one
  property in a frame. `JsShell` owns its generation: wiring it into `JsHost` staging, so a reload swaps shells,
  is P5's. The compositor-side `where` evaluation is not built.

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
