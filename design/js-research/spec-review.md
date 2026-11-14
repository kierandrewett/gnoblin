# Review of the Gnoblin JS specification (implementer's view)

Sources read in full: `/home/kieran/dev/lanes/js-spec/SPECIFICATION.md` (2663 lines) and `init.js`. `L123` = spec line 123. Repo facts come from a read-only look at `/home/kieran/dev/gnoblin-rust-wt` (crate list, `crates/gnoblin-lua/src/lib.rs`, `tree_link.rs`, `process.rs`, `crates/gnoblin-wayland/src/fallback.rs`, `design/layout-owns-placement.md`, `design/xr-extension.md`). Effort numbers are my estimates, marked [INFERENCE].

## 0. Repo facts that change the review

- The architecture the spec wants largely exists for Lua: `gnoblin-lua` is already an IPC client process ("a fault in a script is a restart of this process", `lib.rs:3-9`), with a sandbox crate, a tree socket (`tree_link.rs`: attach/transact/poll, length-prefixed frames), a local mirror of the graph (`tree_mirror.rs`, 1069 lines), a schema catalog (`crates/gnoblin-schema/schema/*.toml`) that generates bindings plus `gnoblin.d.lua`, `gnoblin.configure`, hot-reload (`ReloadOutcome`), and `--role runtime`. About 11k lines in total.
- The stock shell is 5.6k lines of Lua (`data/lua/gnoblin/shell/*.lua` + `node.lua`; `init.lua` 980, `menus` 606, `prompts` 427, `xr` 327, `tiling` 292, `floating` 142). It also covers rules, focus, snap, decorations, gestures, lock, prompts and menus. The spec covers none of rules, focus policy, decorations or lock UI.
- The compositor already has a native fallback policy when no shell holds the tree (`fallback.rs`: cascade placement, maximise, fullscreen, focus handoff). The spec never mentions it, yet its crash story needs it (finding 1.14).
- XR is already an out-of-process program on the owner socket (`extensions/gnoblin-xr`, D35): quad layers per View, stereo `Camera`, `head` camera written by an observer connection, an `xr` seat for rays. Three render backends exist (`gnoblin-render-gles/-vulkan/-software`), so every effect in section 6 is built three times.

## 1. Contradictions and ambiguities

### Display / Output / OutputProfile

- 1.1 **"Output does not own a workspace or camera" (L88) vs `OutputListener` seeing "viewport-related changes" (L193).** `Output` has no viewport. The listener spec predates the Display refactor.
- 1.2 **Overlap policy is two different rules (L668).** "atomically replace or reject the old assignment" is stated as one sentence. `displays.add()` has no return or throw contract. §21 L2565 says only "not silently driven twice". Choose: reject with an error naming both displays, and make `displays.replace()` the only way to steal an output.
- 1.3 **A `Display` selects by `OutputProfile` (L603) but `display.outputs` resolves to `Collection<Output>`.** Nothing says what a Display with zero matching hardware is (L603: "parked"), whether it appears in `gnoblin.displays`, or what `activeDisplay` (L688) is when the pointer is on an output that belongs to no Display. `mode: "single"` with two outputs is undefined (the default mode accepts an array).
- 1.4 **Two geometries for pointer space.** `OutputProfile.position` (L550, `rightOf` L558) places hardware. `span` (L649) defines "logical union". Which wins when a span Display's two profiles are positioned with a gap, or at different scales? Mirror (L636) "does not create a second pointer-space rectangle", so a pointer can leave the mirrored pair at an undefined edge.
- 1.5 **`VirtualOutput` (L234, L2263) is a `gnoblin.outputs` member but physical `Output` "is not manually instantiated" (L533).** `VirtualOutput` is not in the glossary or any class listing. `attachment: "viewer"` is XR-only but appears in core `outputs.add`.
- 1.6 **One transaction or two?** Profiles commit hardware atomically (L564). Displays are replaced atomically (L668). A hotplug that changes both has no stated single transaction boundary, and §18 hot reload "commit staged changes atomically" (L1973) includes both.
- 1.7 **Same workspace on two displays (L623) says "a Wayland client is not configured at two sizes".** It never says what happens to the _scale_. A surface has one preferred fractional scale; on the second output it is resampled. Span (L649) mentions scale; the independent-display case B does not. Also the workspace "authority" display policy (L666) is "can be added": `fixed` is recommended but `workspace.bounds` is not in the `Workspace` class and has no default.

### Workspace membership vs reference collections

- 1.8 **`workspace.clients` is both "membership, not copies" (L69) and has `add()` = transfer (L234, L396).** So it is a mutable _ownership_ collection, while `where()` / `gnoblin.clients` are reference collections. `Collection<T>` (L220) has no way to say which. The type contract needs `MutableCollection` vs `ReadonlyCollection` vs `LiveSelection`.
- 1.9 **`Workspace.children` (L70) vs `Workspace.clients`.** Are clients also children (`client.parent === workspace`)? `LayoutContext` has both `clients` and `children` (L1137). `ArcLayout.compute` uses `children` and `n.content` (L1239), which only makes sense for Views, yet init.js L2499 assigns `ArcLayout` to the _floating desktop workspace of Clients_. Undefined.
- 1.10 **No initial-workspace rule.** A new toplevel has `workspace: null` (L162). Who assigns it: the focused display's `viewport.content`? `workspaces.at(0)`? A `Layout` hook? The spec has `onClientAdded(client, ctx)` on the layout, but a client cannot reach a layout until it is in a workspace. This is the most important missing behaviour for a first slice. Also unspecified: removing a workspace that has clients; a client in two places (a `View` is a reference, a client is not, L429, but the diagram L16 invites confusion); `workspace.clients.add` on a client that is `fullscreen`.
- 1.11 **`Node.layout?` (L147) lets any node own a layout (XR room, init.js L2510), but `Layout.workspace: Workspace` is non-null (L1130).** A layout on a plain `Node` has no workspace. `layout.workspace.bounds` (L963, L1076) then fails. Also `ratio`, `order`, `previewSwap`, `hitTest`, `minSize`, `secure`, `bounds`-on-Workspace, `Scene.metresPerUnit` vs `room.metresPerUnit` (L1369, init L149) are all _used_ in normative code but _absent_ from the declared types.

### Style cascade

- 1.12 **Stack depth is not closed.** Cascade is: defaults → `gnoblin.clients.style` → `workspace.clients.style` → `where()` → `client.style` (L307). Gaps: • `Node.style` (L146) exists on `Workspace` and `View`. Where does it sit, and does `workspace.style` cascade into its clients? • `workspace.clients.where(...)` is a legal selection. Is it ranked as workspace-scoped or `where()`-scoped? • A `where()` selection's `priority` (L307) is not on `Collection` (L220-232). Registration order of selections is undefined for selections made in modules loaded in different order or on reload. • `where(query)`: `Query<T>` is never defined. The native cascade (§21 L2556: "inherited layers don't duplicate on every client") can only be evaluated in Rust if queries are _data_ (appId, title regex, workspace id), never JS predicates. The spec's own `filter(fn)` vs `where({})` split suggests this but never says so. • Selection lifetime: init.js L2423 creates a `where()` and discards the handle. Native must keep the selection alive after JS GC, and drop it on reload when the line is removed from the file. Same for `development.clients.style.cornerRadius = 4`.
- 1.13 **Mutating an inherited list element is undefined (L293-295, L1708, L2501).** `client.style.shadows[1].blur = 40` on a client with no override: the glossary says `client.style` holds "explicit overrides" only (L634), so `shadows[1]` is `undefined` and the line throws. If it returns the inherited element, the write changes _every_ client. If it copies on write, the client detaches from later global edits. Pick one (I recommend: `client.style.shadows` returns the _override list_; to edit an inherited layer, assign a clone explicitly, or use `computedStyle` read-only + `client.style.shadows = [...computedStyle.shadows, ...]`). Related: `[]` as "explicitly no shadows" (L357) vs `unset()` (L309) is fine, but `gnoblin.clients.style = new ClientStyle(...)` (L349) replaces vs merges and orphans the old object's identity. `client.opacity` (L1659) vs `client.style.opacity` (L1697) vs `Node.opacity` (L144) vs `Style.opacity` (L320): four spellings; multiplicative or not? `computedStyle` "and origin" (L2556) is not in the type (L166).

### Transitions staged by running a callback once

- 1.14 **The staging model needs machinery the spec does not describe.** • Writes inside the callback must be buffered, but plain writes outside are immediate (L19, L1659). The runtime needs a global "staging" flag, and reads inside the callback must return the staged value. • Side-effect bans (`spawn`, I/O, `await`, L1668) are unenforceable in JS beyond rejecting a returned Promise. State a dev-mode check. • The target scene needs layout results, which are JS (`compute`, L1768: "computes the current and target placements"). So `requestTransition` is not "callback once then native": it is staged writes → run user `compute` for the after state with the _new_ field values → send. Field values must read as old for the "before" compute and as new for the "after" compute (L1715 animates `arcLayout.radius`). That is a snapshot/restore of user fields, not just a transaction. • A layout _field_ animation interpolates between endpoint placements (L1811), so an arc's `radius` animation moves nodes on straight lines, not along the swept arc. Fine, but document it. • **Floating has no `compute()` (L903-916, hooks only), yet init.js L2497-2503 and L1749 transition `flat → arc → flat`.** Returning to `FloatingLayout` has no target placement to interpolate to. Either Floating gets a `compute` that returns stored `floatingBounds` (recommended), or layouts without `compute` cannot be a transition endpoint. • Manual transitions (`autoplay:false`, L1785): "stage without changing the committed logical state", but the staging writes already ran in JS. Two truths exist (logical vs presented). State which one reads return; `ClientMoveBinding` reads `bounds` as drag start (L938). • `Transition.finished` "rejects" on cancel (L1814, L1785). Most callers drop the return value, so every cancellation is an unhandled rejection. Resolve with `{ cancelled: true }` instead. • Overlap policy (newest wins per component, L1814) is fine; nested `requestTransition` inside a callback, or inside a listener fired by a staged write, is not defined. • `Camera2D → Camera3D` (L1768) needs a common parametrisation. "Do not interpolate matrices" is right; the adapter is real research work.

### Observable user fields

- 1.15 **The spec poses the problem wrongly (L1882).** It says plain `this.radius` writes are "not inherently visible to Rust". But layout `compute` runs _in the JS runtime process_ (L1966), next to the fields. Rust never needs to see them; only the runtime's own scheduler needs "this layout is dirty". • Proxy-wrapping in a base-class constructor (`return new Proxy(this, ...)`) breaks `#private` fields on user subclasses, which the spec claims to preserve (L1882). It also makes `instanceof`, identity and `Object.assign(this, …)` (L2304) behave subtly. • Alternative without Proxy: after every handler turn (and around each transition callback) shallow-compare an own-enumerable snapshot of each live `Layout` / `Binding`. A diff marks it dirty and yields old/new for transitions. It has the same limit the spec admits (nested objects, privates), no engine hooks, and is portable across QuickJS/anything. • Decide and document: "observed = own enumerable data properties, one level".

### XR as `.so` vs the existing `gnoblin-xr` process

- 1.16 **Spec says `.so` loaded into an "isolated extension host" (L1275, L1952, L1909-1961). The repo already has a process on the owner socket (D35).** A C-ABI `dlopen` plugin hosted in a _second_ process adds an ABI to freeze, a host binary, capability grants and a manifest, for one Rust consumer that is already a socket client. Also unclear: who is the peer of the JS runtime for `gnoblin.extensions.xr`, the compositor or the ext host (L1961 "client-side projection")? Recommendation: **an extension is an owner-socket program plus manifest plus an optional JS module**, with the ABI being the tree/IPC protocol version. Keep `dlopen` out of v1 (L1952's "trusted in-process tier" stays out).
- 1.17 **Naming.** `xr.Camera3D` (init L2526), `XRCamera3D` (L1349), `{ Camera3D: XRCamera3D }` (L1305), `gnoblin/extensions/xr` named exports (L1322). Also `xr.session.head` (spec) vs the repo's tree node `camera head` with observer write rights; `ControllerDragBinding` is defined in the extension but core `DragEvent` already carries `controller?: XRController` (L820): a core type referencing an optional module.
- 1.18 **JS in the XR loop.** Goal 13 and L1358 say JS never drives tracking. `ViewMoveBinding.onUpdate` (L1450) is JS reading `event.controller.pose` every sample. That is allowed by L2009 ("coalesce input"), but §21 L2597 "tracking does not wait on JS" must be tested as _head pose and compositing continue while a binding handler is stalled_, and the spec should state a pose-update rate (the repo caps the observer at 60-120/s, `design/xr-extension.md`).

### Other internal problems

- 1.19 **Binding code contradicts the binding text.** L848 says an omitted _or `undefined`_ trigger uses the default. The stock constructors (L889, L1178-1182) do `{ ...defaults, ...triggers }`; an explicit `{ move: undefined }` overwrites the default with `undefined`, and `actual.move !== null` then installs a binding with no trigger. Same for `triggers.resize ?? default`. Fix in the stock code, and test it.
- 1.20 **Latency vs goal 13.** Dragging a window (L941-951) is: input in compositor → IPC to runtime → JS `onUpdate` → write `bounds` → IPC → applied "at frame boundary" (L1895). The window trails the (compositor-drawn) cursor by at least one frame plus IPC. The Lua shell already pays this, but the spec asserts the opposite design goal (L26, L1966). §21 has no number. Add a budget (e.g. p95 pointer-event-to-applied-rect ≤ 2 frames at 60 Hz in a headless run) and a coalescing rule (latest-wins for `onUpdate`, L2009).
- 1.21 **Binding precedence (L1120 vs L852).** L1120: device bindings → target layout → context. L852: context bindings "fallback unless higher-priority overrides". Does `priority` (L790) cross tiers? Equal-priority conflict "refused with a diagnostic": at registration time or at match time? A context-level `Super+MouseLeft+Drag` conflicts with every layout's default; the init.js comment L2474 says "no need", which hides that it silently wins or loses.
- 1.22 **Trigger grammar is absent.** `Super+MouseLeft+Drag`, `Swipe3Left`, `MouseMiddle`, `Grip+Trigger+Drag`, `Left.Grip`: no grammar, no modifier names (`Super`/`Mod4`/`Logo`), no rule for key identity (keysym vs keycode vs layout-dependent: `Super+1` vs `Super+!`), nothing for hold/release/repeat/chords/sequences. The most user-facing string in the API is unspecified.
- 1.23 **Property errors are sync or async?** "Reject cleanly" (L129) and "named property path" (L1905) imply sync throws, but validation happens at the frame boundary (L1895). Split: shape/finite/range → sync `TypeError`/`RangeError` from the setter; ownership/cycles/hardware → reported on the transaction (`Transition.finished`, or `gnoblin.onError`).
- 1.24 **`bind` overload and reactive graph (L1825-1840).** The doc settles on `properties.bind` but `NativeObject` (L131-138) has no `properties`, and the first example still uses `client.bind`. Reactive callbacks are "host-evaluated" JS: during a native camera transition the rendered `camera3D.position` changes natively, so either the callback runs per frame (contradicts L26) or it only tracks logical values (surprising). Recommend cutting §15 from v1 or restricting it to a small native expression DSL.
- 1.25 **Listener semantics.** Adding a `CollectionListener` does not say whether `onAdded` replays for existing members (init and §13 L1389 imply no replay, and the code loops manually). With hot reload (new generation, existing clients) a replay-on-attach mode is required or every config re-implements it. Cross-object event ordering is undefined (only per-object order, L193).
- 1.26 **`client.bounds`, `client.size`, `floatingBounds`, `committedSize`, `transform` (L143-168).** `bounds` is "layout geometry" and written by user drag bindings; in a tiling workspace `compute` owns it. Who wins? `onClientAdded` guards with `floatingBounds` (L905) but a client moved between workspaces keeps a stale `floatingBounds` from the old area. When both `bounds` and `transform` are returned in a `Placement` (L100, L1144) there is no precedence.
- 1.27 **Hot reload by imperative replay (L1970-1974).** `init.js` is imperative (`gnoblin.clients.style.x = 12`, `gnoblin.workspaces.add`). "Stage in a fresh generation, reconcile by id, commit" requires every write in staging to go to a generation-owned overlay, and to be _reverted_ when the line disappears next time (style defaults, selections, bindings, listeners, workspace removal). The spec reconciles _objects with ids_, but not _property writes on shared singletons_ such as `gnoblin.clients.style`. Also top-level `gnoblin.spawn("...")` would re-run on every reload.
- 1.28 **Resilience gap.** The stock shell is itself JS (L2011, "stock JS library"). If the runtime crashes, no layout runs. The existing native fallback (`fallback.rs`) must be named as the runtime-down policy, together with a native-only emergency set (spawn terminal, close, reload). Otherwise "compositor continues" (L1966) means "windows appear at the default size and nothing responds".
- 1.29 **TypeScript.** The spec (L14, L121) says editor-only. The owner now wants `.ts` run directly. Consequences the spec needs: relative-import resolution with `.ts` extensions (L106 shows `.js`), strip-only transforms (no `enum`/`namespace`/parameter properties unless a transform is accepted), diagnostics with original line numbers (source maps or whitespace-preserving strip), and `gnoblin.d.ts` generated from the schema as `gnoblin.d.lua` is today.
- 1.30 **Missing capability areas.** Timers (`setTimeout`), `console`, async I/O/`spawn` with output (the Lua host has `gnoblin.process.spawn`), clipboard, notifications, window rules, focus policy, decorations/titlebars (compositor `borders` exist, titlebars do not), session lock, screenshots/capture, key-repeat per-device config (present L736), per-output DPMS. L119 says the sandbox allows only module loading and "granted services", but L1668 already lists HTTP and file writes.

## 2. Effort ranking (relative units; 1 ≈ one engineer-week) [INFERENCE]

| Rank | Work package                                                                                                                      | Units                         | Notes                                                                                               |
| ---- | --------------------------------------------------------------------------------------------------------------------------------- | ----------------------------- | --------------------------------------------------------------------------------------------------- |
| 1    | Style effect stack natively (multiple borders/shadows/inset, filter and backdrop chains, caps, secure masking) x3 render backends | 8-10                          | Cascade is ~2, rendering is the bulk                                                                |
| 2    | Display / Viewport / mirror / span with mixed scales, pointer mapping, damage                                                     | 8-10                          | `span` plus fractional scale plus damage is the hard part; mirror alone ~2                          |
| 3    | Transition planner: staging, before/after by id, layout and camera 2D↔3D blend, scrubbing, retarget, hit-test on presented state  | 8-10                          | Existing `tween.rs`/`transaction.rs` help; planner and camera adapter are new                       |
| 4    | Binding framework (trigger grammar, matcher, tiers, capture, per-gesture state, device/context routing, cancel on reload/crash)   | 5-6                           | Existing chord/gesture payloads in the tree proto; grammar and multi-seat are new                   |
| 5    | Hot reload with generation overlay and reconcile                                                                                  | 5-6                           | Existing `ReloadOutcome` covers whole-config swap; per-write overlay is new                         |
| 6    | Schema + generated JS classes + `.d.ts` for the whole object zoo, compositor object store for new types                           | 6-8                           | Mechanical but wide; Lua generator is the template                                                  |
| 7    | Extension system, literal `.so` ABI + host + manifest + capability grants + JS module generation                                  | 5                             | Drops to 1-2 if reinterpreted as owner-socket programs (1.16)                                       |
| 8    | Layout host (compute scheduling, hooks, dirty detect) + port floating/tiling/grid/monocle/dwindle/arc                             | 4                             | Lua `floating.lua` 142 + `tiling.lua` 292 are references; invalidation is the subtle part           |
| 9    | XR JS API on top of existing `gnoblin-xr` + 6 bindings                                                                            | 4                             | Binding maths (`Pose3D`, `Ray3`, `Transform3D`) is ~1 on its own                                    |
| 10   | QuickJS embedding: runtime process, limits, ES module loader, event pump                                                          | 3                             | Largely a port of the Lua host/sandbox; engine-neutral parts of `tree_link`/mirror/lifecycle reused |
| 11   | Collections, live `where()` selections evaluated natively, listener forwarding                                                    | 3                             |                                                                                                     |
| 12   | Inspector / console / input-debug / transition inspector                                                                          | 3                             | Defer                                                                                               |
| 13   | Security: secure client masking across views, capture, XR, backdrop                                                               | 3                             | Cross-cutting, not separable                                                                        |
| 14   | Parity and acceptance harness vs Lua corpus                                                                                       | 3                             |                                                                                                     |
| 15   | Reactive property bindings (§15)                                                                                                  | 3                             | Cut from v1 (1.24)                                                                                  |
| 16   | Observable-field bridge                                                                                                           | 2 (snapshot diff) / 4 (Proxy) | See 1.15                                                                                            |
| 17   | TS type-strip + source positions                                                                                                  | 1-2                           | Another lane is researching this                                                                    |

Total ≈ 75-90 units. The three XL items (1-3) are roughly half. A flat floating/tiling desktop with JS keybindings needs only items 4 (key chords only), 5 (basic), 6 (small), 8, 10 and 11, about 20 units.

## 3. Proposed first vertical slice

Goal: prove **TS init → runtime process → tree protocol → real window geometry change → hot reload**, nothing else. Spec §22 stages 1+2+5 minus everything visual. Cut: styles, displays/viewports/outputs, transitions, XR, `.so`, inspector, gestures, pointer drag bindings, property bindings.

### Scope

1. New crate (e.g. `gnoblin-js`, alongside `gnoblin-lua`; Lua untouched) with a QuickJS runtime role: `gnoblin --role runtime-js` adopting the same privileged connection the Lua runtime uses, over the existing `tree` namespace. Extract the engine-neutral part of `tree_link` / `tree_mirror` rather than copying it.
2. Load `~/.config/gnoblin/init.ts` (and `init.js`). `.ts` is type-stripped before evaluation; relative `./x.ts` imports work; the bare `gnoblin` module exports the slice's classes.
3. Objects (read from the mirror, write via transactions): `gnoblin.clients` (read-only `Collection<Client>` with `length/at/get/find/filter/[Symbol.iterator]`), `Client { id, appId, title, alive, bounds (rw), close() }`, `ClientListener` with `onAdded`-style forwarding via `CollectionListener` (`onAdded`, `onRemoved`) and `onTitleChanged`.
4. One implicit workspace: all toplevels belong to it (answers 1.10 for the slice), with one `Layout` slot. Ship `FloatingLayout` (hook-only, cascade) and `MasterStackLayout` (`compute`, verbatim from L1173-1206) as JS modules. A `Layout` subclass with a plain `this.gap` field must re-lay out when `gap` is edited (snapshot-diff dirty check, 1.15).
5. Input: `gnoblin.input.primary.bind("Super+Return", fn)` for key chords only, plus `"close"`/`spawn` actions via the simple tier below (`keys:`). Pointer drags are out.
6. Hot reload: file watcher or `gnoblinctl reload` runs a _new_ QuickJS context (generation N+1), stages all writes, validates, commits in one tree transaction, then drops generation N's bindings and listeners. Failure (syntax error, thrown exception, bad value) leaves generation N fully intact and reports file:line (original `.ts` line).
7. Runtime-down behaviour: kill the runtime process; compositor keeps running with `fallback.rs` policy; restart re-applies config without disturbing open clients.

### Example the slice must run (plain TS)

```ts
import { defineConfig, MasterStackLayout, spawn } from "gnoblin";
export default defineConfig({
    layout: new MasterStackLayout({ ratio: 0.6, gap: 12 }),
    keys: { "Super+Return": spawn("foot"), "Super+Q": "close" },
});
```

### Done criteria (all headless, no GPU, no real input device)

- D1. `init.ts` with type annotations and a `./helper.ts` import runs; an error in it reports the original `.ts` line.
- D2. With the testkit's null compositor/fake owner (as in the `lib.rs` doctest: `Server::new(Graph::new(catalog))` + `connect`), opening 3 fake toplevels yields `MasterStackLayout.compute` rectangles applied through a single tree transaction (assert exact rects for gap 12, ratio 0.6, 1/2/3 windows).
- D3. Editing `gap` in `init.ts` and reloading changes applied rects without closing or re-creating any toplevel, and the `Window` ids are unchanged.
- D4. A reload with a syntax error keeps the old rects and key bindings, and the error names file and line. A reload that removes a `keys` entry removes that binding.
- D5. `Super+Q` (synthetic chord through the owner socket) closes the focused toplevel; `Super+Return` spawns the configured command (recorded by a stub spawner).
- D6. `ClientListener.onTitleChanged` fires exactly once per change with the client as argument 1; a listener from generation N never fires after generation N+1 is live.
- D7. Killing the runtime process leaves the compositor responsive; windows opened meanwhile are placed by `fallback.rs`; a restarted runtime re-lays out all existing toplevels.
- D8. An infinite loop in a handler is interrupted by the budget and the next event still runs (port of the Lua `failure_modes` tests).
- D9. Latency instrument exists: p95 from synthetic `bounds` write to applied rect recorded in a headless run; no pass/fail threshold yet, but the number is printed (feeds 1.20).

Estimated size: ~12-15 units [INFERENCE], mostly items 10, 11, 6 (small), 8 and the engine-neutral extraction of the tree link.

## 4. API UX problems and a simple tier

### Problems (user who just wants a key bound or a gap)

- U1. **A gap needs a `Workspace`, a layout instance, an `add` to a global collection, and an import list** (L372-390). No global/outer gap exists; `gap` is per layout instance (L1161). init.js is 170 lines for "stock behaviour with a theme". The spec never says what exists _without_ init.js (L2011 only says it "must function").
- U2. **Bind a key = `gnoblin.input.primary.bind(...)`** (four identifiers), and every workspace switch repeats `if (input.activeDisplay) input.activeDisplay.viewport.content = ws` (init L2464-2472). Need `gnoblin.show(ws)` or an action value.
- U3. **One-off listener = a class plus `addListener(new X())`** (L174-190, L2478). Typed-class listeners are fine for rich cases, but a `title` log needs 6 lines. The spec rejected event-name strings (L20); generated typed sugar `clients.onAdded(fn)` / `client.onTitleChanged(fn)` keeps the types and removes the class.
- U4. **Monitor config is 27 lines for scale 1.25** (`OutputProfile` + `Display` + `Viewport` + `Camera2D`, init L2426-2452). A single-output default `Display` should be implicit.
- U5. **Three ways to write a property** (assignment, `set()`, `requestTransition`) plus four opacity spellings (1.13). Document one primary path per use.
- U6. **Stringly tokens:** triggers, `"quadrant"`, `unset("cornerRadius")`, colour strings (`"#ffffff40"`, `"#0006"` both appear, L2385 vs L347). TypeScript can type triggers (template literal types) and `keyof ClientStyle`.

### Proposed simple tier: `defineConfig` (plain TS, pure sugar)

`defineConfig(config)` is an identity function with a typed parameter (so editors complete it) and a runtime registration. Each key _desugars to the object API in §4-12_; there is no second system. `export default defineConfig(...)` makes hot reload a diff of plain data. The callback tier stays available: `setup(gnoblin)` runs after the declarative part and may use any class.

```ts
import { defineConfig, spawn, show, MasterStackLayout } from "gnoblin";

export default defineConfig({
    gaps: { inner: 12, outer: 8 }, // applied to every stock layout via LayoutContext.area/gap
    style: { cornerRadius: 12, shadows: [{ y: 8, blur: 24, color: "#00000066" }] },
    workspaces: [
        // creates Workspace objects, ids stable for reload
        { id: "desktop", layout: "floating" },
        { id: "dev", layout: new MasterStackLayout({ ratio: 0.65 }) },
    ],
    outputs: { "DP-1": { mode: "3440x1440@144", scale: 1.25 } }, // OutputProfile + implicit Display/Viewport
    keys: {
        "Super+Return": spawn("ghostty"),
        "Super+Q": "close",
        "Super+Tab": ({ gnoblin }) =>
            gnoblin.requestTransition(
                () => {
                    /* … */
                },
                { duration: 300 },
            ),
        ...show.numbered("Super+{n}", ["desktop", "dev"]), // Super+1, Super+2
    },
    rules: [{ appId: "firefox", workspace: "web", style: { cornerRadius: 18 } }],
    on: {
        clientAdded(c) {
            /* typed ClientListener sugar */
        },
    },
    setup(gnoblin) {
        /* escape hatch: full OOP API */
    },
});
```

Requirements for the tier:

- Desugaring table published (`gaps.inner` → `gap` on each stock layout, `rules[].style` → `clients.where(query).style`, `keys` → `input.primary.bind`). Missing keys do nothing; unknown keys are type errors.
- **Gap needs a real core concept:** add `LayoutContext.gap`/`inset` supplied by the workspace (`Workspace.gaps`) so one setting applies to all stock layouts and custom layouts can read it.
- `keys` actions are a closed union (`"close"`, `spawn(...)`, `show(ws)`, `fn`) so bindings are inspectable and diffable.
- Rules use data-only queries (1.12), which keeps them native-evaluable.
- Zero-config baseline spelled out: one `desktop` workspace, floating layout, one implicit display per output, stock bindings (the existing Lua shell's set).

## 5. Proposed acceptance tests (headless)

Harness tiers: **F** = fake model (in-process runtime against `gnoblin-model::Graph` + loopback `Server`, as in the Lua host doctest); **N** = null/headless compositor (`gnoblin-testkit`/`gnoblin-wlcs`, software renderer, scripted Wayland clients and synthetic seat input); **X** = simulated OpenXR (`FakeRuntime`); **P** = pixel readback on the software backend.
IDs map to §21 bullets; **(+)** marks tests I added because §21 leaves the behaviour undefined.

### JS object model (L2541-2548)

| ID  | Test                                                                                                                                                                          | Tier |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---- |
| OM1 | `gnoblin` is the same object across three imported modules; `import { Client, … } from "gnoblin"` resolves and types match `.d.ts`                                            | F    |
| OM3 | `gnoblin.clients.find(id) === workspace.clients.find(id)`; `workspace.clients.remove` / workspace destroy leaves `alive === true`                                             | F    |
| OM4 | `class R extends Layout { constructor(){super();this.radius=9} }`: set, change, `compute` re-run once per change, no proxy code in user source; `#private` field works (1.15) | F    |
| OM5 | `where()` follows add/remove/appId change; `filter()` is a frozen array snapshot                                                                                              | F    |
| OM6 | Listener on a collection fires once per member, arg 1 is the origin; removal detaches all; replay-on-attach per decision 1.25 (+)                                             | F    |
| OM7 | A write on a destroyed object throws a typed error naming `Client#id.field`; sync-vs-async classification of errors per 1.23 (+)                                              | F    |

### Styles (L2550-2556)

| ID  | Test                                                                                                                                                                                                           | Tier  |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- |
| ST1 | Global radius applies to current and future clients; an explicit `client.style.cornerRadius` survives later global edits                                                                                       | F + P |
| ST2 | Precedence matrix: defaults < global < workspace < `where()` (priority, then registration order) < client, per property; table-driven over all pairs, including `workspace.clients.where` and `Node.style` (+) | F     |
| ST3 | `unset` reveals the inherited value; `null` stays `null`; `[]` is an explicit empty list (1.13)                                                                                                                | F     |
| ST4 | Mutating an element of an _inherited_ list per the decided semantics does or does not leak to other clients (+)                                                                                                | F     |
| ST5 | `computedStyle` reports value and origin; animated global `shadows[0].blur` moves every non-overriding client and no overriding one                                                                            | F + P |
| ST6 | Backdrop blur over a secure client renders masked pixels                                                                                                                                                       | P     |
| ST7 | Invalid style value (`NaN`, negative blur, > pass cap) rejects with the property path and leaves previous state                                                                                                | F     |

### Displays and workspaces (L2558-2565)

| ID  | Test                                                                                                                                                              | Tier  |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- |
| DW1 | One workspace on two Displays with different cameras: client count unchanged, exactly one configure sequence, layout area unchanged (+ explicit authority policy) | N     |
| DW2 | `mirror`: both outputs byte-identical at equal modes; letterboxed at different aspect; pointer-space has one rect                                                 | N + P |
| DW3 | `span`: clipped halves compose to the full frame; a straddling client's pixels continuous across the seam; pointer and damage correct, with mixed scale 1 and 1.5 | N + P |
| DW4 | Switching `viewport.content` on display A does not affect display B                                                                                               | N     |
| DW5 | Forced modeset failure and hotplug-in/out: last-good arrangement preserved; no-enabled-output rejected (L564)                                                     | N     |
| DW6 | `displays.add` overlapping an owned output rejects with both ids in the message; `replace` swaps atomically (1.2) (+)                                             | F     |
| DW7 | Display with no matching hardware is parked; plugging the output in activates it; workspace clients untouched (L603) (+)                                          | N     |
| DW8 | New toplevel is assigned to the workspace per the initial-workspace rule (1.10) (+)                                                                               | N     |

### Bindings and layouts (L2567-2576)

| ID   | Test                                                                                                                                                               | Tier |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---- |
| BL1  | `new FloatingLayout()` installs ids `move`, `resize` with the documented triggers; `{ resize: null }` omits it; `{ move: undefined }` keeps the default (1.19)     | F    |
| BL2  | `bindings.remove("move")` keeps the layout; `clear()`, `get`, `add` behave                                                                                         | F    |
| BL3  | Right-drag quadrant is chosen from the initial pointer position and does not change mid-drag (synthetic pointer events)                                            | N    |
| BL5  | The four stock drag bindings are real JS classes; a Rust-side assertion shows only generic scene/input calls were made                                             | F    |
| BL6  | Two input contexts dragging two clients keep separate state; cancel of one leaves the other                                                                        | N    |
| BL7  | Runtime killed mid-grab: native capture released, client receives button release, no stuck button; `onCancel` once if the runtime is alive                         | N    |
| BL8  | Floating preserves user bounds when another client opens; stale `floatingBounds` after workspace transfer handled (1.26)                                           | N    |
| BL9  | Tiling recomputes in one transaction when `ratio` changes or a client leaves; golden rects equal the Lua `tiling.lua` corpus for the same inputs (L2618)           | F    |
| BL10 | Precedence: device > layout > context; equal-priority conflict diagnostic at registration (1.21) (+)                                                               | F    |
| BL11 | Trigger grammar parser accepts/rejects a golden list, including `Super+1` vs `Super+Shift+1` (1.22) (+)                                                            | F    |
| BL12 | Latency: p95 pointer-event-to-applied-rect for a scripted 5 s drag under a stated budget; coalescing keeps press/release order under a stalled handler (L2009) (+) | N    |
| BL13 | Layout removed or reloaded during a grab → `onCancel` once, capture released                                                                                       | N    |

### Animation (L2578-2586)

| ID  | Test                                                                                                                                                     | Tier  | Status                                                              |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- | ------------------------------------------------------------------- |
| AN1 | Callback invoked exactly once; N writes yield exactly one tree transaction; a returned Promise or `await` in the callback is rejected in dev mode (1.14) | F     | done                                                                |
| AN2 | `flat → arc` (including a _floating_ start, 1.14) interpolates by client id; no bbox jump at t=0 and t=1; `progress` 0.5 matches the golden pose         | F + P | partly: by-id timed writes and one compute; no scrub, no size tween |
| AN3 | Camera 2D↔3D blend: at several `progress` values, projected corner positions are continuous and monotone (no clip pop)                                   | P     | not done: no camera                                                 |
| AN4 | No JS runs during a 750 ms transition (counter on handler dispatches stays 0)                                                                            | N     | partly: no JS after commit; native counter not built                |
| AN5 | Hit-test at `progress=0.5` returns the client under the _drawn_ pixel, not the target rect                                                               | N + P | not done: native                                                    |
| AN6 | Scrub 0→0.4→0.1→1, `reverse`, `cancel` from several states: logical state and promise result (`{cancelled}`, 1.14) as specified                          | F     | partly: cancel and `{cancelled}`; no scrub or `reverse`             |
| AN7 | Invalid or stale target aborts the whole transaction; previous state intact; error names path                                                            | F     | done                                                                |
| AN8 | Overlapping transitions: newest wins per component, others continue; nested call inside a callback follows the decided rule (+)                          | F     | done                                                                |

### Runtime, hot reload, extensions, XR (L2588-2598, L1964-2011)

| ID  | Test                                                                                                                                                                            | Tier  |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- |
| RT1 | Reload with the same `Workspace({id})` reconciles: clients, floating bounds, membership kept; no duplicate workspace                                                            | N     |
| RT2 | A failed reload (syntax, exception, bad value, missing import) keeps the whole prior generation; error has module path, line, object, field                                     | F     |
| RT3 | Removing a style line, `where()` rule, binding or listener from the file reverts its effect on reload (1.27) (+)                                                                | F     |
| RT4 | Stored closures from generation N never run after the swap; in-flight transitions retarget or finish cleanly                                                                    | F     |
| RT6 | Kill runtime: compositor responsive, `fallback.rs` policy places new windows, restart re-lays out (1.28) (+)                                                                    | N     |
| RT7 | `init.ts` with types and `./x.ts` import runs; diagnostics map to `.ts` lines (1.29) (+)                                                                                        | F     |
| EX1 | No XR extension installed: `gnoblin.extensions.xr` is nullish; static import fails with a clear message; desktop unaffected                                                     | N     |
| EX2 | Extension crash or ABI/protocol mismatch: compositor alive, session objects marked `alive=false`, restart restores                                                              | X     |
| XR1 | `XRCamera3D instanceof Camera3D`, `XRViewport instanceof Viewport`; core objects have no OpenXR fields                                                                          | F     |
| XR2 | Synthetic controller ray hits a `View` and forwards `Trigger` to the source client when no binding consumes it                                                                  | X     |
| XR3 | `ViewMoveBinding` changes only the View transform; the source client's desktop rect is unchanged                                                                                | X     |
| XR4 | `ViewResizeBinding`: zero configures during drag; at most one on release, only past aspect/density thresholds; two Views of one client keep rendering during the pending buffer | X     |
| XR5 | Stall the JS binding handler for 500 ms: head pose writes and layer submission continue; binding cancels safely                                                                 | X     |
| XR6 | Secure client never appears in a View, capture, or XR layer                                                                                                                     | X + P |

## Recommendation

Build the existing Lua runtime's architecture again with QuickJS rather than starting from the spec's object zoo: first the slice in section 3 (a plain-TS `defineConfig` front, one implicit workspace, `compute`-based layouts, key bindings, generation-based hot reload over the existing tree socket), and treat styles, displays, transitions and XR as later, separately sized projects. Before coding, the owner must decide: the initial-workspace rule (1.10), copy-on-write semantics for inherited style lists (1.13), the observable-field mechanism (1.15, recommend snapshot diff), `.so` vs owner-socket extensions (1.16, recommend the latter), data-only `where()` queries (1.12), the trigger grammar (1.22), and the latency budget (1.20).

## Open risks

- JS-in-the-input-loop latency for pointer drags and 90 Hz XR manipulation is untested; the design depends on it being acceptable.
- Generation-overlay hot reload (1.27) turns an imperative script into a reconcilable program. It may force a declarative front (`defineConfig`) as the primary form.
- The transition planner needs user `compute` results for the after-state, so a transition is no longer "one callback then native". Complexity and failure modes (compute throws mid-transition) are underspecified.
- Span/mirror with mixed scales and the effect stack across three backends are the largest unknown costs.
- Engine-neutral extraction from `tree_link.rs` / `tree_mirror.rs` may be larger than assumed [unverified].
- I did not verify the exact tree op set for writing window geometry; the Lua shell proves it exists, but the op names should be read from `gnoblin-proto` before the slice is sized.
