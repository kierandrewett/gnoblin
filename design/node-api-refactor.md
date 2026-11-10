# Node API: one tree for every shell

Status: proposal, second pass (syntax); nothing here is built. Pass 1 claims about current code cite `file:line` at
`a1d6d13aa`; claims added in pass 2 say `c28c0c671` (both `origin/rust-compositor`). What I could not check is listed
in section 10.

All syntax is plain Lua 5.5. There is no dialect, preprocessor, source transform or new token. Every `lua` block in
this file parses with the Lua 5.5.1 that Gnoblin vendors (`Cargo.toml:232`: `mlua` with `lua55` and `vendored`;
D10), and the class system, the stock library and the ten scripts ran on it against a fake core (section 10).

## What changed in pass 2

- `gnoblin.new("Type", { props })` and `parent:add(child)` become `Type{ props, child, child }`. Classes are
  globals; no string names a type.
- A class system with single inheritance from `Node`: `Node:extend "Panel" { height = 32 }`. Native and Lua
  classes are the same kind of thing, and `n:is(Window)` tests either.
- `n:bind("prop", fn)` becomes "a function value is a binding". Dependencies are tracked, so the `changed` signal
  goes, and a layout's `arrange` re-runs by itself.
- `n:on("signal", fn)` becomes `on_signal = fn`, `function Class:on_signal()` for every instance,
  `n.signal:connect(fn)` and `await(n.signal)`. Every handler is a coroutine, so `gnoblin.spawn` goes.
- `Tween` and `Effect` stop being nodes: `n:to(0.2, { x = 0 }, ease.out)` is an animated write, and
  `effects = { { blur = 30 } }` is data like `draw`. Ten native classes remain, down from twelve.
- `find`, `children(type)`, tag groups and window rules become CSS-like selectors: `ws:all"> Window"`,
  `n"#clock"`, `w:is"[app_id=foot]"`.
- Layouts and placement modes are classes (`Tiling`, `Strip`, `Floating`, `Cascade`), and arrivals are `ready`
  handlers on classes (`function Window:on_ready()`), which delete five global signals.
- All of it is Lua: `node.lua` (370 non-blank lines, section 5a) over ten native classes, nine `core` calls, the
  frame clock and four `sys` calls (5a.4).
- Sections 5, 6, 7, 8 and 10 are rewritten; section 5a is new; sections 2, 3 and 9 have small edits.

The core idea in five sentences:

1. Gnoblin already has the structure it needs: the pure scene tree in `gnoblin-render` holds parents, children,
   paint order, 3D transforms, cameras, effects and a hit test, but it is private and fed one way.
2. Make that tree the public object model: ten native classes rooted at `Node`, one transform, one order (tree
   order), one input pick, and every client request delivered as a signal on the node it concerns.
3. Gnoblin keeps mechanism only: protocol semantics, drawing, picking, the frame clock, motion, capture and a
   fail-closed lock. Classes, bindings, placement, stacking, focus, layouts and widgets are a shipped Lua shell that
   users copy and edit.
4. Scripts change the tree in transactions applied at frame boundaries, so the same tree and inputs give the same
   frame, and a layout is a class with an `arrange` method, with no kinds, extents, declines or fallback.
5. The shell is trusted, so roles, capabilities, grants, bands, powers and tiers go; the one edge is the socket,
   and XR becomes a program that turns View nodes into OpenXR layers.

Codes used in the tables of sections 1 and 2:

| Axis code | Presentation axis | Axis code | Presentation axis    | Caller code | Caller                         |
| --------- | ----------------- | --------- | -------------------- | ----------- | ------------------------------ |
| G         | geometry          | Dc        | decoration           | S           | shell script (runtime role)    |
| T         | transform         | E         | effects              | C           | Wayland client                 |
| Z         | order and z       | I         | input routing        | X           | extension                      |
| D         | depth             | F         | focus                | K           | the compositor itself          |
| V         | visibility        | Ou        | output assignment    | P           | IPC peer, such as `gnoblinctl` |
| O         | opacity           | W         | workspace membership |             |                                |

## 1. Inventory

### Window management and layout

| Object or API                        | Where                                               | Created and ended by | Callers    | Axes            |
| ------------------------------------ | --------------------------------------------------- | -------------------- | ---------- | --------------- |
| `Wm` state machine                   | `crates/gnoblin-wm/src/wm.rs:110-147`               | K at start           | K          | G Z V F Ou W    |
| `Window` record                      | `crates/gnoblin-wm/src/window.rs:64-109`            | K on map and unmap   | K          | G V Ou W O      |
| `Stack`, `StackLayer`                | `crates/gnoblin-wm/src/stack.rs:23-63`              | K                    | K          | Z               |
| Placement modes                      | `crates/gnoblin-wm/src/placement.rs:18-59`          | config               | K          | G               |
| Window rules                         | `crates/gnoblin-wm/src/rules.rs:78-180`             | config at load       | S (config) | G W O E         |
| Pointer focus, auto-raise            | `crates/gnoblin-wm/src/pointer_focus.rs:24`         | config               | K          | F Z             |
| Snapping, drag constraint            | `snapping.rs:14`, `drag.rs:22`                      | config               | K          | G               |
| Workspaces, workspace sets           | `workspace.rs:1-6`, `workspace_set.rs:16-38`        | config               | S P        | W V             |
| Layout contract and `Slot`           | `crates/gnoblin-wm/src/layout/engine.rs:118-239`    | K per workspace      | K          | G T V O F       |
| `Placement`, `Camera`, `Reply`       | `crates/gnoblin-wm/src/layout/placement.rs:126-221` | layout run           | layout     | G T V O Dc D    |
| Lua layout host (in compositor)      | `crates/gnoblin-lua/src/layout/host.rs:28-55`       | K                    | K          | as above        |
| Shipped layout modules (8)           | `data/lua/gnoblin/layouts/*.lua`                    | `require`            | S          | G T V O         |
| `FloatingEngine`                     | `crates/gnoblin-compositor/src/layout.rs:401-452`   | K                    | K          | none (declines) |
| `workspace.layout`, `layout_command` | `crates/gnoblin-schema/schema/workspace.toml`       | IPC                  | S P        | G               |

### Scene, renderer and frames

| Object or API                    | Where                                                                          | Created and ended by | Callers            | Axes      |
| -------------------------------- | ------------------------------------------------------------------------------ | -------------------- | ------------------ | --------- |
| `Scene` tree, `NodeKind::Space`  | `crates/gnoblin-render/src/scene.rs:63-125`                                    | composer             | K                  | T Z D E O |
| `Scene::hit_test`                | `crates/gnoblin-render/src/scene.rs:645-654`                                   | n/a                  | no caller in input | I         |
| `TweenPlayer`                    | `crates/gnoblin-render/src/tween.rs:1-35`                                      | composer             | K                  | T O       |
| `Composer`                       | `crates/gnoblin-backend/src/render/compose.rs:213-260`                         | K                    | K                  | Z T O V E |
| `OutputFrame` layer list, cursor | `crates/gnoblin-render-frame/src/frame.rs:385-395` (per `layers-status.md:27`) | K                    | K                  | Z         |
| Effects (`MAX_PASSES` 32)        | `crates/gnoblin-render/src/effect.rs:28-31`                                    | `window.add_effect`  | S P                | E         |
| `effect` object                  | `crates/gnoblin-schema/schema/effect.toml`                                     | IPC                  | S P                | E         |
| Shell visuals                    | `window.toml:209-251`, `compose.rs:764-823`                                    | IPC                  | S P                | O T       |
| GLES, Vulkan, software renderers | `crates/gnoblin-render-{gles,vulkan,software}`                                 | K                    | K                  | draw only |

### Layers, decorations and protocol surfaces

| Object or API                        | Where                                                                       | Created and ended by | Callers      | Axes                   |
| ------------------------------------ | --------------------------------------------------------------------------- | -------------------- | ------------ | ---------------------- |
| `LayerRegistry`, `Band`, `LayerSpec` | `crates/gnoblin-layers/src/band.rs:18-58`, `scope.rs:17-112`                | K, IPC               | K P          | Z I F V E Ou           |
| `layers.register`                    | `crates/gnoblin-schema/schema/layers.toml`                                  | IPC                  | P with grant | Z I                    |
| `gnoblin_window_frame_v1`            | `crates/gnoblin-wayland/src/protocols/gnoblin_window_frame_v1.rs:1-48`      | K on map             | C (renderer) | Dc I                   |
| `zxdg_decoration_v1`                 | `zxdg_decoration_v1.rs:6-9`                                                 | C                    | C            | Dc (always CSD)        |
| wlr layer shell                      | `zwlr_layer_shell_v1.rs` (records at `:452-478`, per `layers-status.md:41`) | C                    | C            | G Z I F Ou             |
| xdg-shell, popups                    | `protocols/xdg_shell.rs`                                                    | C                    | C            | G                      |
| Pointer target `surface_under`       | `crates/gnoblin-wayland/src/state.rs:3013-3038`                             | n/a                  | K            | I                      |
| `Modal` overlay                      | `crates/gnoblin-wayland/src/modal.rs:14-24` (per `layers.md:98`)            | portal               | K            | I Z                    |
| foreign toplevel (wlr and ext)       | `zwlr_foreign_toplevel_management_v1.rs`, `ext_foreign_toplevel_list_v1.rs` | K                    | C            | none (reads, requests) |

### Input, lock, capture and outputs

| Object or API                      | Where                                                                  | Created and ended by   | Callers | Axes         |
| ---------------------------------- | ---------------------------------------------------------------------- | ---------------------- | ------- | ------------ |
| Chords, bindings, grabs (`Router`) | `crates/gnoblin-input/src/router.rs:48-224`                            | config                 | K       | I            |
| Gestures and action names          | `gesture_router.rs:56-79`, `gnoblin-config/schema/config.toml:298-304` | config                 | K       | I            |
| Script input policy queue          | `crates/gnoblin-input/src/policy.rs:40-127`                            | K                      | S       | I            |
| EIS endpoint and gate              | `crates/gnoblin-compositor/src/eis.rs:1-49`                            | K                      | C       | I            |
| Session lock                       | `crates/gnoblin-lock/src/lib.rs:1-30`                                  | C (`ext-session-lock`) | C       | V I          |
| Capture sessions                   | `crates/gnoblin-capture/src/lib.rs:1-66`                               | C, portal              | C P     | reads pixels |
| Outputs                            | `crates/gnoblin-output/src/lib.rs:1-21`, `output.toml`                 | backend                | S P C   | Ou G         |
| Accessibility magnifier            | `crates/gnoblin-a11y/src/lib.rs:1-8`                                   | config                 | K       | T            |

### Object system, trust and extensions

| Object or API                        | Where                                                           | Created and ended by | Callers | Axes              |
| ------------------------------------ | --------------------------------------------------------------- | -------------------- | ------- | ----------------- |
| Schema objects (15 files)            | `crates/gnoblin-schema/schema/*.toml`                           | K                    | S P     | via members       |
| Generated Lua bindings and stub      | `crates/gnoblin-lua/build.rs:1-19`, `docs/public/gnoblin.d.lua` | build                | S       | n/a               |
| Hand-written host members            | `crates/gnoblin-lua/src/bindings.rs:368-371`                    | build                | S       | n/a               |
| `gnoblin-model::Graph` mirror        | `crates/gnoblin-model/src/lib.rs:1-30`                          | K, runtime           | S P     | n/a               |
| IPC capabilities (11)                | `crates/gnoblin-proto/src/capability.rs:27-92`                  | handshake            | P       | all, by grant     |
| Privileged roles                     | `crates/gnoblin-compositor/src/ipc/privileged.rs:77-116`        | guardian             | P       | all, by grant     |
| Wayland `ClientRole` (6)             | `crates/gnoblin-wayland/src/access.rs:195-216`                  | K on connect         | C       | global visibility |
| Extension manifest, powers, tiers    | `design/rust-compositor.md:2744-2909` (D22)                     | host config          | X       | all, by grant     |
| WASM module host (deleted, step 10A) | `crates/gnoblin-wasm/src/lib.rs:1-11`                           | host config          | X       | all, by grant     |
| Request broker and slots             | `crates/gnoblin-model/src/slot.rs:1-20`                         | services role        | P       | prompts           |

## 2. Rule census with verdicts

Default verdict is delete. A rule stays only when a script in section 3 cannot work without it. "Know" asks whether
an ordinary shell developer must learn the rule to do ordinary work.

### Layout

| Rule                                     | Where                                                                                                  | Added to solve                  | Know | Verdict | Why                                           |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------- | ---- | ------- | --------------------------------------------- |
| `LayoutKind` floating, tiling, kiosk     | `layout/kind.rs:11-66`                                                                                 | who owns geometry beside the wm | yes  | delete  | no wm policy left to share with               |
| `LayoutExtent` output, unbounded         | `layout/kind.rs:71-78`, `settle.rs:139-156`                                                            | keep tiles inside the output    | yes  | delete  | `clip` and camera do it                       |
| `WindowPlacement` free, managed          | `layout/kind.rs:103-108`, `window.rs:89`                                                               | which windows a layout sees     | yes  | delete  | layouts see their children                    |
| Float toggle refusals                    | `wm.rs:2066-2068`, `:2084-2085`                                                                        | kind and transient checks       | yes  | delete  | no float flag                                 |
| `Reply::Declined`, `Collected::Declined` | `placement.rs:216-221`, `engine.rs:148`, `:230`                                                        | let wm logic run                | yes  | delete  | no second placement logic                     |
| Centre-and-cascade fallback              | `layout/fallback.rs:15-34`                                                                             | a declined reply                | no   | delete  | nothing declines                              |
| Native `FloatingEngine`                  | `gnoblin-compositor/src/layout.rs:401-452`                                                             | skip the Lua VM on default path | no   | delete  | floating is a stock script                    |
| Requests `Place`, `Move`, `Leave`        | `placement.rs:225-243`                                                                                 | ask before changing geometry    | yes  | delete  | signals on nodes                              |
| `LayoutEffect` float and focus           | `placement.rs:178-190`                                                                                 | layout asks wm for state        | yes  | delete  | script sets `seat.focus`                      |
| `Placement.decorated`                    | `placement.rs:139-140`                                                                                 | kiosk without frames            | yes  | delete  | never read anywhere (grep `.decorated`)       |
| Reply `camera`                           | `placement.rs:160-174`, `settle.rs:99`                                                                 | 3D layouts                      | yes  | fold    | validated, then dropped: no consumer          |
| Four transform types                     | `layout/transform.rs:26`, `gnoblin-types/src/transform.rs:98`, `gnoblin-animation/src/transform.rs:25` | each layer had its own          | yes  | fold    | one node transform                            |
| Whole-reply validation `settle`          | `layout/settle.rs:37-112`                                                                              | atomic placement sets           | yes  | fold    | per-setter checks; transactions are atomic    |
| Order sorted by window id                | `settle.rs:101-104`                                                                                    | determinism                     | no   | delete  | it throws away the layout's order             |
| `BOUND` 2^20                             | `layout/transform.rs:11`                                                                               | renderer overflow               | no   | keep    | one setter limit for every coordinate         |
| Generations, tickets, `Slot`             | `layout/engine.rs:26-239`                                                                              | stale async replies             | no   | delete  | transactions carry a sequence                 |
| Frame budget 4 ms                        | `gnoblin-wayland/src/state.rs:588`                                                                     | never wait on a script          | no   | keep    | the one script budget                         |
| Layout command wait 2 s                  | `state.rs:591`, `:1987`; `arrange.rs:392-393`                                                          | synchronous commands            | no   | delete  | blocks the event loop: a bug                  |
| Worker hard limit 100 ms, leak cap 3     | `gnoblin-lua/src/layout/host.rs:50-51`                                                                 | wedged in-process VM            | no   | delete  | if decision 1 picks a process                 |
| Three failures switch to floating        | `design/rust-compositor.md:2291-2294`                                                                  | bad layouts                     | yes  | delete  | errors are signals; tree stays                |
| Layout reference with hash               | `rust-compositor.md:2071-2075`, `gnoblin.d.lua:758`                                                    | pin modules across processes    | yes  | delete  | ordinary `require`                            |
| `layout_command` string table            | `workspace.toml`, `strip.lua:374-383`                                                                  | call into the layout VM         | yes  | delete  | layouts are plain Lua objects                 |
| Layout stdlib subset                     | `rust-compositor.md:2246-2254`                                                                         | bound the in-process VM         | no   | fold    | one sandbox profile for the shell VM          |
| `layout` config section                  | `rust-compositor.md:2322-2326`                                                                         | choose a layout per workspace   | yes  | delete  | the shell assigns; `outputs` is ignored today |

### Window management policy

| Rule                                            | Where                                                     | Added to solve             | Know | Verdict | Why                                    |
| ----------------------------------------------- | --------------------------------------------------------- | -------------------------- | ---- | ------- | -------------------------------------- |
| `PlacementMode` and its key                     | `placement.rs:18-59`, `config.toml:1883-1890`             | where new windows open     | yes  | delete  | `Floating` subclasses (section 8)      |
| Dialog centred on parent                        | `wm.rs:1297-1317`                                         | transients                 | no   | delete  | stock script                           |
| Default size 800 by 600                         | `wm.rs:92`                                                | clients with no size       | no   | delete  | stock script                           |
| `WindowKind` normal, dialog, modal              | `window.rs:22-38`                                         | transient handling         | yes  | fold    | read-only `parent_window`, `modal`     |
| `WindowState` flags and precedence              | `window.rs:47-56`, `rust-compositor.md:2197-2210`         | maximise, fullscreen, snap | yes  | delete  | shell sets `states` sent to the client |
| `StackLayer` normal, fullscreen                 | `stack.rs:23-28`                                          | fullscreen above windows   | yes  | delete  | tree order                             |
| `ClaimOrigin` Gnoblin above extension           | `stack.rs:33-36`, `:86-103`                               | trust order on a target    | no   | delete  | tree order                             |
| Six focus and raise options                     | `wm.rs:110-129`, `config.toml:1797-1855`                  | focus policy               | yes  | delete  | `Window` handlers in the stock shell   |
| Edge tiling, `SnapZone`, refusal                | `wm.rs:1994-1996`, `snapping.rs:14`                       | snapping                   | yes  | delete  | stock script                           |
| `DragConstraint`                                | `drag.rs:22-25`                                           | keep drags on screen       | no   | delete  | stock drag helper                      |
| Three window-management keys                    | `config.toml:1816`, `:1868`, `:1893`                      | Mutter parity              | yes  | delete  | no Rust code reads them                |
| Rule engine, `MAX_RULES`, apply once            | `rules.rs:78-180`                                         | declarative rules          | yes  | delete  | an `if` in `Window:on_ready`           |
| Rule `layer` match                              | `rules.rs:87-90`, `:123-125`                              | one vocabulary             | yes  | delete  | it matches nothing, by design          |
| `MAX_WINDOWS` 256 and record limit              | `wm.rs:106`, `:374`                                       | bound the registry         | no   | delete  | no script needs it                     |
| Workspaces and sets                             | `workspace.rs`, `workspace_set.rs:16-38`                  | named desktops             | yes  | fold    | `Workspace` node; sets are a Lua table |
| Output removal moves windows                    | `wm.rs:1166-1190`                                         | hotplug                    | no   | delete  | stock `Screen:on_destroyed`            |
| Workspace slide, close fade, bell flash, 200 ms | `compose.rs:81`, `:398-412`, `:660-682`, `:922`           | built-in animation         | yes  | delete  | `to` in the stock shell                |
| Reduced motion in the composer                  | `compose.rs:913-915`                                      | accessibility              | no   | fold    | every motion is instant when set       |
| Four opacity writers                            | `compose.rs:705`, `:725`, `:746`, `:764`; `state.rs:2212` | layout, rule, shell, alpha | yes  | fold    | one `opacity`                          |
| Shell scale and offset, limits                  | `window.toml:221-251`, `compose.rs:780-823`               | shell visuals              | yes  | fold    | node transform; `BOUND` covers range   |
| Rotated window refused                          | `compose.rs:47-49`, `state.rs:2179-2195`                  | no 3D draw path            | no   | delete  | draw any invertible transform          |

### Layers and decorations

| Rule                                            | Where                                                  | Added to solve                  | Know | Verdict | Why                                     |
| ----------------------------------------------- | ------------------------------------------------------ | ------------------------------- | ---- | ------- | --------------------------------------- |
| Eight bands, compositor-only bands              | `gnoblin-layers/src/band.rs:18-58`                     | total order and a trust ceiling | yes  | delete  | tree order; `Lock` root                 |
| `LayerScope` output, window                     | `scope.rs:17-25`                                       | follow a target                 | yes  | delete  | make it a child                         |
| `Order` above, below, above target              | `scope.rs:33-43`                                       | order within a band             | yes  | delete  | sibling index                           |
| `InputPolicy` four values                       | `scope.rs:50-60`                                       | modal dialogs                   | yes  | fold    | `input` plus `seat:grab`                |
| `Focus` takes, none                             | `scope.rs:67-73`                                       | focus stealing                  | yes  | fold    | shell sets `seat.focus`                 |
| `LockBehaviour` hide, keep                      | `scope.rs:80-86`                                       | lock                            | no   | delete  | `Lock` root                             |
| `EffectsPolicy` inherit, none                   | `scope.rs:93-99`                                       | effects reaching layers         | no   | delete  | `effects` belong to their node          |
| `ContentKind` surface, painter                  | `scope.rs:106-112`                                     | compositor-drawn layers         | no   | fold    | `Surface` versus `Canvas`               |
| Provider namespaces, reserved id, name lengths  | `name.rs:25-32`, `registry.rs:66-102`                  | attribution                     | yes  | delete  | nodes have no owners                    |
| 32 layers per provider                          | `registry.rs:47`                                       | a greedy extension              | no   | delete  | trusted shell                           |
| `secure`                                        | `registry.rs:143`, `rules.rs:162-164`                  | keep secrets out of captures    | no   | keep    | one node property, one rule             |
| `capture.secure-treatment` mask, omit           | `rust-compositor.md:3139`                              | style of masking                | no   | delete  | always mask                             |
| `Modal` and the overlay frame variant           | `layers-status.md:27`                                  | portal dialogs                  | no   | delete  | stock shell node with `input`           |
| `gnoblin_window_frame_v1`, `FrameRenderer` role | `gnoblin_window_frame_v1.rs:1-48`, `access.rs:205-209` | decorations by a client         | yes  | delete  | decision 2: `Canvas`                    |
| Decoration mode always client-side              | `zxdg_decoration_v1.rs:6-9`                            | no frame drawer                 | no   | fold    | `Window.decorated` answers the protocol |

### Input

| Rule                              | Where                                            | Added to solve                      | Know | Verdict | Why                                |
| --------------------------------- | ------------------------------------------------ | ----------------------------------- | ---- | ------- | ---------------------------------- |
| Chord grammar and conflicts       | `gnoblin-input/src/lib.rs:37-44`                 | one meaning per chord               | no   | keep    | mechanism behind `seat:bind`       |
| Gesture recogniser thresholds     | `gesture.rs:337-399`                             | touchpad gestures                   | no   | keep    | mechanism behind `Seat:bind`       |
| Gesture action names (five)       | `config.toml:298-304`, `gesture_router.rs:56-62` | gestures without Lua                | yes  | delete  | handlers are functions             |
| Script policy queue with deadline | `policy.rs:40-127`                               | script veto on raw input            | yes  | delete  | consumption is decided by the pick |
| Grab scopes                       | `router.rs:48-89`                                | grabs                               | no   | keep    | `seat:grab`                        |
| Shortcut inhibit                  | `router.rs:153`                                  | keyboard-shortcuts-inhibit protocol | no   | keep    | protocol semantics                 |
| Lock input gate                   | `gnoblin-lock/src/lib.rs:20-23`                  | fail-closed lock                    | no   | keep    | `Lock` rule                        |
| EIS default deny, no grant path   | `gnoblin-compositor/src/eis.rs:3-6`              | remote input safety                 | no   | fold    | `remote` signal on `Seat`          |
| Rect-only pointer target          | `state.rs:3020-3035`                             | first slice                         | no   | delete  | ray pick on the tree               |

### Trust and extensions

| Rule                                          | Where                                   | Added to solve                    | Know | Verdict | Why                                                 |
| --------------------------------------------- | --------------------------------------- | --------------------------------- | ---- | ------- | --------------------------------------------------- |
| Eleven IPC capabilities                       | `gnoblin-proto/src/capability.rs:27-92` | least privilege per process       | yes  | delete  | owner socket is trusted                             |
| Roles runtime, services, portal, input        | `ipc/privileged.rs:77-98`               | the same                          | yes  | delete  | one owner connection                                |
| Owner grants on the ordinary socket           | `ipc/privileged.rs:108-116`             | the same                          | yes  | delete  | the same                                            |
| `ClientRole` six values                       | `access.rs:195-216`                     | Wayland global visibility         | no   | fold    | two: sandboxed or not; `SessionHost` stays          |
| Privileged globals list                       | `access.rs:114-135`                     | hide globals from sandboxes       | no   | keep    | the one edge                                        |
| D22 packages: manifest, powers, tiers, hashes | `rust-compositor.md:2744-2909`          | third-party code                  | yes  | delete  | extensions receive node references                  |
| WASM tier (deleted, step 10A)                 | `gnoblin-wasm/src/lib.rs:1-11`          | native plugins (D6)               | yes  | delete  | no script needs it                                  |
| Dynamic Wayland globals                       | `rust-compositor.md:2848-2862`          | extension protocols               | yes  | delete  | not built; no script needs it                       |
| Per-extension Lua environments                | `gnoblin-lua/src/bindings.rs:391`       | veto power isolation              | no   | fold    | restricted environment without roots                |
| IPC extension namespaces                      | `request-intercept.md:41`               | custom APIs                       | yes  | fold    | nodes with script-declared members                  |
| Request slots and providers                   | `gnoblin-model/src/slot.rs:1-20`        | who draws prompts                 | yes  | fold    | signals on `session`; no handler refuses            |
| Portal provider negotiation                   | `rust-compositor.md:2459-2482`          | who draws portal UI               | yes  | fold    | one signal; the stock handler uses the egui painter |
| `capture-window` capability                   | `capture-api.md:27-37`                  | shell captures                    | yes  | delete  | not built; shell captures any `View`                |
| `CaptureScreen` capability                    | `capability.rs:86-91`                   | portal screenshots                | no   | delete  | portal path asks the shell's handler                |
| Privacy indicator                             | `capture-api.md:79-84`                  | visible capture                   | no   | keep    | mechanism on every active capture                   |
| Lock capture embargo                          | `gnoblin-capture/src/lib.rs:28-31`      | no capture while locked           | no   | keep    | `Lock` rule                                         |
| Five hackability tiers                        | `rust-compositor.md:67-73`              | layering of power                 | yes  | delete  | one API, two transports                             |
| Two Lua window collections                    | `gnoblin.d.lua:129`, `:747`             | generated and hand-written halves | yes  | delete  | one tree                                            |

### Effects, outputs and XR

| Rule                                                          | Where                                               | Added to solve              | Know | Verdict | Why                             |
| ------------------------------------------------------------- | --------------------------------------------------- | --------------------------- | ---- | ------- | ------------------------------- |
| Effect kinds and pass limits                                  | `render/src/effect.rs:28-31`, `window.toml:293-301` | blur, shadow, shader        | no   | fold    | `effects` list; limits stay     |
| Effect skipped on 3D, `active`, `reason`                      | `rust-compositor.md:3038`                           | renderer gaps               | no   | keep    | read-only `skipped` on the node |
| Layout per output                                             | `output.toml` `layout`, `set_layout`                | default layout              | yes  | delete  | shell assigns                   |
| XR panel kinds, rules 1 to 3                                  | `xr-extension.md:121-125`                           | flatten or split per output | yes  | delete  | the shell builds `View` nodes   |
| XR layer budget fallback                                      | `xr-extension.md:156-162`                           | runtime layer limits        | no   | delete  | shell reads the budget          |
| XR `attach = viewer`, `windows = false`, rule `output` action | `xr-extension.md:263-275`                           | head-locked surfaces        | yes  | delete  | child of the head `Camera`      |
| XR layout `depth` key                                         | `xr-extension.md:451-457`                           | intersecting windows        | yes  | fold    | `Camera.depth_test`             |
| Virtual output `on_remove`                                    | `xr-extension.md:389-395`                           | headset session end         | yes  | delete  | `destroyed` on the `xr` seat    |
| Session lock states and cover                                 | `gnoblin-lock/src/lib.rs:11-30`                     | fail-closed lock            | no   | keep    | `Lock` node                     |

## 3. The scripts, marked up

These are the scripts a shell developer wants to write, in the API they would reach for. Each marked line says what
it needs today.

They keep pass 1's syntax on purpose. The marks record what today's code lacks, which no syntax changes; section 6
has the same ten scripts in the pass 2 syntax.

| Mark   | Meaning                                         |
| ------ | ----------------------------------------------- |
| `[SC]` | needs a special case in Gnoblin                 |
| `[RO]` | needs a role, grant or capability               |
| `[PR]` | needs a separate protocol or process            |
| `[2S]` | goes through a second system for the same thing |
| `[NO]` | cannot be done today                            |

### 3.1 Tiling: master and stack

```lua
-- master_stack.lua: the first window on the left, the rest stacked on the right
local ws, seat = gnoblin.workspaces.by_id(1), gnoblin.seat
ws:set_layout(require("layouts.master_stack"))   -- [SC] needs M.kind and a hashed reference: kind.rs:11-20, gnoblin.d.lua:758
                                                  -- [2S] runs in a second Lua VM in the compositor: gnoblin-compositor/Cargo.toml:17-18
local function arrange()
  local wins = ws:windows()                       -- [NO] stack order is not visible, only creation order: arrange.rs:179-189
  local W, H, n = ws.width, ws.height, #wins
  for i, w in ipairs(wins) do
    local x, y, ww, hh = 0, 0, W // 2, H
    if i > 1 then x, y, ww, hh = W // 2, (i - 2) * (H // (n - 1)), W // 2, H // (n - 1) end
    w.width, w.height = ww, hh                    -- [SC] every tiled window in one reply, or the reply is refused: settle.rs:93-97
    w:tween({ x = x, y = y }, 0.15)               -- [NO] duration and easing are fixed in the composer: compose.rs:81
  end
end
ws:on("window_added", arrange)                    -- [SC] dialogs never reach a layout: kind.rs:60-66
ws:on("window_removed", arrange)
seat:bind("Super+Return", function()              -- [SC] only through layout_command string params: state.rs:1982-1999
  seat.focus.index = 1                            -- [NO] the reply's order is sorted away: settle.rs:101-104
  arrange()                                       -- [SC] a command blocks the event loop up to 2 s: state.rs:591
end)
```

### 3.2 Floating with cascade and free-space placement

```lua
-- floating.lua: cascade new windows, centre dialogs, raise on click
local ws, seat = gnoblin.workspaces.by_id(1), gnoblin.seat
ws:on("window_added", function(w)                 -- [SC] floating is a native engine that declines: layout.rs:401-452
  local p = w.parent_window
  if p then                                       -- [SC] dialog centring is a wm rule: wm.rs:1297-1317
    w.x, w.y = p.x + (p.width - w.width) // 2, p.y + (p.height - w.height) // 2
  else
    local step = 32 * (#ws:windows() - 1)
    w.x, w.y = step, step                         -- [NO] the wm has placed it before any script runs: wm.rs:1318-1322
  end                                             -- [SC] the only choice is a config key: config.toml:1883-1890
end)
seat:on("button", function(e)
  if e.pressed and e.window then e.window:raise() end  -- [SC] raise_on_click is a wm option: wm.rs:1827-1834
end)                                              -- [NO] a script cannot reorder windows (D33)
```

### 3.3 Scrolling strip with focus walking

```lua
-- strip.lua: columns in a row; the view follows the focused column
local ws, seat = gnoblin.workspaces.by_id(1), gnoblin.seat
local strip = gnoblin.group()                     -- [NO] no group to scroll: the scene is private, scene.rs:1-7
ws:add(strip)
ws.clip = true                                    -- [NO] no clip property: scene.rs:73-86
local function reveal(w)                          -- [SC] the offset is ctx.state.view inside the layout: strip.lua:149-171
  if w.x + strip.x < 0 then strip:tween({ x = -w.x }, 0.2) end
  if w.x + w.width + strip.x > ws.width then strip:tween({ x = ws.width - w.x - w.width }, 0.2) end
end
ws:on("window_added", function(w)
  local cols = strip:windows()
  local last = cols[#cols]
  w.x, w.y, w.width, w.height = last and last.x + last.width + 8 or 0, 0, ws.width // 2, ws.height
  strip:add(w)                                    -- [SC] needs kind tiling and extent unbounded: kind.rs:53-55
end)
seat:bind("Super+Right", function()
  local nxt = strip:windows()[seat.focus.index + 1]
  if nxt then seat.focus = nxt; reveal(nxt) end   -- [SC] focus only through LayoutEffect::Focus: placement.rs:184-189
end)                                              -- [SC] scrolling only through the scroll_to command: strip.lua:374-383
```

### 3.4 Custom titlebar with animated close

```lua
-- titlebar.lua: a drawn titlebar, a shadow, and a fade on close
gnoblin.on("window_added", function(w)
  w.decorated = true                              -- [NO] the answer is always client-side: zxdg_decoration_v1.rs:6-9
  local bar = gnoblin.canvas({ y = -28, width = w.width, height = 28 })
  bar.draw = { { rect = { 0, 0, w.width, 28 }, fill = "#303446" },
               { text = w.title, x = 12, y = 6 } }   -- [NO] no compositor painter for shapes or text
  w:add(bar)                                      -- [PR] frames come from a renderer client: gnoblin_window_frame_v1.rs:3-10
                                                  -- [RO] that client needs the FrameRenderer role: access.rs:205-209
                                                  -- [NO] frames are never composed: layers-status.md:11
  w:add_effect("shadow", { radius = 24 })         -- [2S] effects are a separate object list: window.toml:293-301
  bar:on("pressed", function() w:close() end)
  w:on("closed", function()
    w:tween({ opacity = 0 }, 0.18):on("finished", function() w:destroy() end)
  end)                                            -- [SC] the composer runs its own fade: compose.rs:398-412
end)                                              -- [NO] a script cannot keep a closed window on screen
```

### 3.5 Panel, launcher and notification overlay

```lua
-- panel.lua, launcher.lua and toasts.lua
local screen = gnoblin.outputs.primary()
local bar = gnoblin.canvas({ width = screen.width, height = 32 })  -- [PR] a panel must be a layer-shell client
                                                  -- [RO] layer shell is a privileged global: access.rs:116
                                                  -- [NO] Lua cannot place compositor content: layers-status.md:34
bar.draw = { { rect = { 0, 0, screen.width, 32 }, fill = "#1e1e2e" } }
gnoblin.root:add(bar)
gnoblin.workspaces.by_id(1).y = 32                -- [PR] reserving space needs an exclusive zone
gnoblin.seat:bind("Super+Space", function()
  local box, query = gnoblin.canvas({ width = 600, height = 48 }), ""
  gnoblin.root:add(box)
  gnoblin.seat.focus = box                        -- [SC] keyboard focus follows layer-shell interactivity rules
  box:on("key", function(e)
    if e.key == "Return" then gnoblin.process.spawn({ "sh", "-c", query }); box:destroy()
    elseif e.text then query = query .. e.text end
  end)
end)
gnoblin.notifications:on("notify", function(n)    -- [NO] no notification server in the tree
  local toast = gnoblin.canvas({ width = 320, height = 64, draw = { { text = n.summary, x = 12, y = 20 } } })
  gnoblin.root:add(toast)                         -- [SC] overlay band and 32 layers per provider: band.rs:27-28, registry.rs:47
  toast:tween({ opacity = 0 }, 4):on("finished", function() toast:destroy() end)
end)
```

### 3.6 Overview with live thumbnails

```lua
-- overview.lua: a thumbnail of every window; click one to focus it
gnoblin.seat:bind("Super+Tab", function()
  local layer = gnoblin.group()
  layer:add_effect("blur", 30)                    -- [RO] an overlay needs RegisterLayer: capability.rs:53-58
  for i, w in ipairs(gnoblin.windows.list()) do
    local thumb = gnoblin.view({ source = w, width = 320, height = 200 })  -- [NO] no subtree-to-texture: compose.rs:1114
                                                  -- [RO] window capture is runtime-only: capture-api.md:27-37
    thumb.x, thumb.y = (i - 1) % 4 * 340, (i - 1) // 4 * 220
    thumb:on("pressed", function() w:focus(); layer:destroy() end)  -- [NO] no input to compositor-drawn content
    layer:add(thumb)
  end
  gnoblin.root:add(layer)
end)
-- The only route today moves the windows themselves:
-- w:set_scale(0.3); w:set_offset(x, y)           -- [2S] shell visuals beside layout placement: window.toml:221-251
```

### 3.7 Workspace switcher with animated transitions

```lua
-- workspaces.lua: Super+N slides to workspace N
local current = 1
local function switch(i)
  local from, to = gnoblin.workspaces.by_id(current), gnoblin.workspaces.by_id(i)
  if not to or from == to then return end
  local dir = i > current and 1 or -1
  to.x = dir * to.width                           -- [NO] a workspace is not a drawable group
  from:tween({ x = -dir * from.width }, 0.25)     -- [SC] the slide is built into the composer: compose.rs:660-682
  to:tween({ x = 0 }, 0.25)                       -- [NO] its duration is fixed: compose.rs:81
  current = i
end
for i = 1, 9 do
  gnoblin.seat:bind("Super+" .. i, function() switch(i) end)
  gnoblin.workspaces.by_id(i):on("activate_requested", function() switch(i) end)  -- [NO] ext-workspace is not served
end
gnoblin.seat:gesture({ kind = "swipe", fingers = 3 }, function(g)  -- [2S] gestures take action names: config.toml:298-304
  if g.phase == "end" then switch(current + (g.dx < 0 and 1 or -1)) end
end)
gnoblin.workspaces.activate_set("work")           -- [SC] workspace sets are their own concept: workspace_set.rs:16-38
```

### 3.8 Screen recorder and remote view

```lua
-- recorder.lua and remote.lua
gnoblin.seat:bind("Super+Shift+R", function()
  local stream = gnoblin.outputs.primary():capture({ stream = true })  -- [NO] no PipeWire producer: gnoblin-capture/src/lib.rs:46-66
                                                  -- [RO] CaptureScreen is portal-only: capability.rs:86-91
  gnoblin.process.spawn({ "gst-launch-1.0", "pipewiresrc", "path=" .. stream.node, "!", "filesink",
                          "location=/tmp/rec.raw" })
end)
gnoblin.seat:on("remote_requested", function(r)
  r:accept()                                      -- [NO] the EIS gate has no grant path: eis.rs:3-6
end)
```

### 3.9 Lock screen

```lua
-- lock.lua
gnoblin.seat:bind("Super+L", function()
  gnoblin.lock:engage()                           -- [PR] only an ext-session-lock client can lock: gnoblin-lock/src/lib.rs:1-10
  local field, typed = gnoblin.canvas({ width = 320, height = 44 }), ""
  gnoblin.lock:add(field)                         -- [SC] the lock band takes only the compositor: band.rs:56-58
  gnoblin.seat.focus = field
  field:on("key", function(e)
    if e.key ~= "Return" then typed = typed .. (e.text or ""); return end
    if gnoblin.lock:authenticate(typed) then      -- [NO] no PAM conversation in the tree
      gnoblin.lock:release()
    end
    typed = ""
  end)
end)
```

### 3.10 VR shell

```lua
-- vr.lua: gnoblin-xr runs as an extension process -- [RO] grants capture, input.inject, create_virtual: xr-extension.md:510-512
local room = gnoblin.group({ name = "xr" })
local head = gnoblin.camera({ projection = "stereo" })  -- [NO] no camera object; the reply camera is dropped after settle.rs:99
room:add(head)
for i, screen in ipairs(gnoblin.outputs.list()) do
  room:add(gnoblin.view({ source = screen, x = (i - 1) * 2100, z = -2000 }))
                                                  -- [SC] panel kinds chosen by rules 1 to 3: xr-extension.md:121-125
                                                  -- [PR] needs gnoblin_view_v1, not built: xr-extension.md:314-333
end
for i, w in ipairs(gnoblin.windows.list()) do
  local a = i * 0.6
  w:tween({ x = math.sin(a) * 1800, z = -math.cos(a) * 1800, rotation = { 0, -math.deg(a), 0 } }, 0.3)
end                                               -- [NO] a rotated window is refused: compose.rs:47-49
head:add(gnoblin.view({ source = gnoblin.canvas({ width = 800, height = 80 }), z = -1000 }))
                                                  -- [SC] needs an output with attach = viewer: xr-extension.md:263-275
local xr_seat = gnoblin.seat_named("xr")          -- [NO] one seat only: multi-seat.md:23-26
-- xr_seat's ray must click the window it hits    -- [NO] the pointer ignores transforms: state.rs:3020-3035
xr_seat:bind("BTN_EXTRA", function() gnoblin.outputs.list()[2]:focus() end)  -- bindings work today
```

## 4. The missing structure, with code evidence

The hypothesis holds, and the graph already exists. `gnoblin-render::Scene` is a pure tree with parents, sibling
order, a 3D transform per node, effects, camera spaces, damage and an inverse-transform hit test (`scene.rs:1-42`).
It is a hub crate the mechanism may name (`xtask/src/deps.rs:87`). Nothing new is needed: make it the authority
and give it to scripts.

Today five owners split presentation between them:

| Owner              | Decides                                    | Evidence                                                         |
| ------------------ | ------------------------------------------ | ---------------------------------------------------------------- |
| `gnoblin-wm`       | geometry, stacking, focus, workspaces      | `wm.rs:1318-1353`, `stack.rs:144-195`                            |
| Layout engine      | tiled geometry, transform, opacity, camera | `placement.rs:126-141`                                           |
| Layer registry     | band order, input policy, lock, effects    | `scope.rs:17-112`, `band.rs:18-35`                               |
| Composer and scene | paint order, transforms, opacity, motion   | `compose.rs:20-24`, `:1086`, `scene.rs:73-86`                    |
| Protocol modules   | frames, modal overlay, layer-shell records | `gnoblin_window_frame_v1.rs:640-649` (per `layers-status.md:10`) |

What the split costs:

- **Three orders.** The wm stack (`stack.rs:58-63`), registry entries interleaved into it (`stack.rs:125-137`) and
  the composer's own order (`compose.rs:216`, `:1086`).
- **Four opacity writers.** Layout placement (`state.rs:2212`), rules (`compose.rs:746`), shell visuals
  (`compose.rs:764`) and surface alpha (`compose.rs:725`).
- **Dropped intent.** The layout's camera is validated (`settle.rs:99`) and never applied. `decorated` is never
  read. The layout's order is sorted away (`settle.rs:101-104`).
- **Input that does not see the picture.** `surface_under` tests plain rectangles in wm stack order
  (`state.rs:3020-3035`). `Scene::hit_test` inverts the real transforms (`scene.rs:645-654`) and has no caller.
- **Frames hit-tested but never drawn** (`layers-status.md:11`, `:47`).
- **Layouts that see a subset.** Only managed, visible, non-fullscreen windows reach a layout
  (`arrange.rs:89-92`, `:179-189`). Every shipped layout except `kiosk` declines every request
  (`floating.lua:23-25`, `master_stack.lua:144-146`, `strip.lua:228-230`).

What was built wrongly:

- **Policy in the core.** The plan says "the Gnoblin core decides policy (stacking, focus, placement, lock)"
  (`design/rust-compositor.md:87-91`). That contradicts the product. Layouts were bolted on top, and kinds, extents,
  declines and the fallback exist only so two placement authorities can share one window.
- **A permission system for a trusted shell:** capabilities, roles, grants, powers and tiers.
- **A second Lua VM in the compositor** (`gnoblin-compositor/Cargo.toml:17-18`), while `gnoblin-lua/src/lib.rs:7`
  still says the compositor runs no Lua.
- **A blocking wait.** Layout commands wait up to 2 s on the event loop (`state.rs:591`, `arrange.rs:392-393`).

## 5. The target model, on one page

A panel, a layout and a VR HUD, each complete:

```lua
-- A panel: a bar on every desk with the workspace name and a clock.
function Desk:on_ready()
  self:add(Bar{ Text{ text = function() return self.active.name end }, Box{ grow = 1 }, Clock{} })
end

-- A layout: windows side by side in equal columns. Make it the default for new desks.
Workspace:extend "Columns" { gap = 8 }
function Columns:arrange()
  local wins = self:all"> Window"
  local w = (self.width - self.gap * (#wins - 1)) // math.max(#wins, 1)
  for i, win in ipairs(wins) do
    win.width, win.height = w, self.height
    win:to(0.2, { x = (i - 1) * (w + self.gap), y = 0 }, ease.out)
  end
end
function Desk:workspace(name) return Columns{ name = name } end

-- A VR HUD: a box fixed to the head camera, one metre ahead, built when the headset's seat arrives.
function Seat:on_ready()
  if self.name ~= "xr" then return end
  root:add(Node{ name = "room",
    Camera{ name = "head", projection = "stereo", metres = 0.001,
      View{ z = -1000, width = 600, height = 60,
        source = Box{ width = 600, height = 60, radius = 12, padding = 12, direction = "row", fill = "#1e1e2ecc",
          Clock{ size = 24 }, Box{ grow = 1 }, Text{ size = 24, text = function() return desk().active.name end } } } },
  })
end
```

**The tree.** `root` holds everything. Gnoblin puts each node it makes under `root`, and the shell moves it. The
order is screens, then seats, the lock, then each window or surface as it maps.

**Drawing.** A `Screen` draws its `source`; while the lock is engaged, every screen draws `lock` instead. Children
draw after their parent, in order. Whoever parents a node destroys it.

**Making nodes.** `Class{ name = value, child, child }`: named fields set properties or add handlers, listed values
are children, and a string child is a `Text`. `parent:add(child, index)` moves a live node and returns it;
`n:destroy()` ends it and its subtree.

**Classes.** `Base:extend "Name" { field = default }` declares the global `Name`:

| In the class table | Declares                                                     |
| ------------------ | ------------------------------------------------------------ |
| `count = 0`        | a property; the default fixes its type (number, string, ...) |
| `target = Window`  | a property holding a `Window` or nil                         |
| `width = function` | a bound default (see below)                                  |
| `clicked = signal` | a signal: `self.clicked:emit(...)`                           |

| Written as                               | Means                                                                |
| ---------------------------------------- | -------------------------------------------------------------------- |
| `n.x = 5`                                | set; ends any binding on `x`                                         |
| `n.x = function(n) ... end`              | bind: re-runs whenever a property it read changes                    |
| `on_closed = fn`, `n.closed:connect(fn)` | add a handler on that node                                           |
| `function Window:on_closed()`            | add a handler for every `Window`; base class handlers run first      |
| `function Class:name()`                  | a method; a subclass replaces it; call the base as `Base.name(self)` |
| `build(self)`                            | returns children; runs once, after the fields                        |
| `arrange(self)`                          | runs like a binding: whenever what it read changes                   |
| `n:is(Window)`, `n:is".dock"`            | class or selector test                                               |

**Time.** Each handler runs in its own coroutine, so it can wait:

- `await(seconds)` waits that long.
- `await(n.closed)` waits for the signal and returns its arguments.
- `await(n:to(...))` waits for a motion to end.
- `go(fn)` starts a coroutine; `every(seconds, fn)` repeats one. Both stop when the node whose handler started them
  is destroyed.
- Bindings must not wait.

**Motion.** `n:to(seconds, { x = 0, opacity = 1 }, ease.out)` animates native properties inside Gnoblin; reads
return the end value at once. Curves: `ease.linear`, `out`, `out_cubic`, `in_cubic`, `in_out`, or any Lua function
from 0..1. Reduced motion makes every motion instant.

**Selectors.** Parts compound without spaces, as in `Window.dock[app_id=foot]`:

| Selector                    | Matches                                           |
| --------------------------- | ------------------------------------------------- |
| `Window`                    | a node of that class or a subclass                |
| `.dock`                     | a node whose `tags` contain `dock`                |
| `#clock`                    | a node named `clock`                              |
| `[app_id=foot]`, `[secure]` | a property whose text equals the value, or is set |
| `A B`, `A > B`              | `B` anywhere under `A`, or as a direct child      |

`n:all(sel)` returns matches in tree order, `n:find(sel)` or `n"sel"` the first, and `n:closest(sel)` the nearest of
`n` and its ancestors.

**Every node.**

| Members                                       | Meaning                                                                                                                         |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `x y z`, `rotation scale`, `width height`     | local transform and rectangle; `rotation`, `scale` are `{x, y, z}` or a number                                                  |
| `opacity visible clip secure`                 | drawn or not; clip children; never captured                                                                                     |
| `input`                                       | `false`, `true`, or `"source"` on a `View`                                                                                      |
| `effects`, `skipped`                          | passes in order: `{ blur = 30 }`, `{ shadow = 24 }`, `{ tint = c }`, `{ corners = 8 }`, `{ shader = src }`; read-only `skipped` |
| `name tags data`                              | name for `#`; set of tags for `.`; a free table                                                                                 |
| `id parent children index`                    | read-only                                                                                                                       |
| `add destroy raise to all find closest is`    | methods                                                                                                                         |
| `ready child_added destroyed`                 | signals; `ready` fires once, when the node first gets a parent                                                                  |
| `press release motion scroll enter leave key` | input signals carrying an `Event`                                                                                               |

An `Event` has `target`, `x`, `y` (in the target's units), `seat`, `button`, `key`, `text` and `stop()`. It goes to
the picked node, then up its ancestors until `stop()`.

**Native classes.**

| Class              | Is                                  | Adds                                                                                                                                       |
| ------------------ | ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `Surface`          | client content                      | read-only `role`: `popup`, `layer`, `lock`, `cursor`, `drag`, `x11`                                                                        |
| `Window : Surface` | an app window                       | see below                                                                                                                                  |
| `Canvas`           | content Gnoblin paints              | `draw`: `{ rect = {x, y, w, h}, radius, fill }`, `{ text, size, color, font }`, `{ icon, size }`, `{ image }`; `measure(text, size, font)` |
| `View`             | a subtree drawn through a camera    | `source` (kept alive), `camera`, `capture{ cursor }`                                                                                       |
| `Screen : View`    | a monitor                           | `connector`, `modes`, `mode`, `scale`, `enabled`                                                                                           |
| `Camera`           | a projection posed by its transform | `projection` (`orthographic`, `perspective`, `stereo`), `fov`, `near`, `far`, `depth_test`, `metres`                                       |
| `Seat`             | one user's input                    | `focus`, `screen`, `x`, `y` (the pointer); `claim(chord)`, `grab(node)`; signals `chord`, `remote`                                         |
| `Workspace`        | a group published to taskbars       | `active`; signal `activate`                                                                                                                |
| `Lock`             | the session lock root               | `engaged`; `authenticate(secret)`, awaited for true or false; signal `requested`                                                           |

`Window` members:

| Kind            | Members                                                                                                |
| --------------- | ------------------------------------------------------------------------------------------------------ |
| read-only       | `title app_id pid parent_window committed_width committed_height placed`, size limits                  |
| written         | `width height` (the configure size), `maximized fullscreen activated tiled decorated`                  |
| method          | `close()`                                                                                              |
| request signals | `move resize maximize fullscreen minimize menu activate closed`; nothing happens unless a handler acts |

**Stock library** (Lua in `data/lua/gnoblin/shell`, copied and edited by users):

- Widgets: `Box`, `Text`, `Icon`, `Grid`, `Bar`, `Clock`, `Entry`. `Box` has `fill`, `radius`, `gap`, `padding`,
  `grow` and `direction` (`row`, `column` or `none`).
- `Desk`, one per screen, with `show(ws)`, `step(n)` and `workspace(name)`; override `workspace` to choose a layout.
- Layouts: `Floating`, `Cascade`, `Centred`, `Tiling`, `Strip`.
- Globals for the tree: `root`, `seat`, `lock`, `desk()`.
- Globals for work: `spawn"cmd"` or `spawn{argv}`, `date(fmt)`, `toast(text)`, `drag`.
- Globals for time: `ease`, `signal`, `await`, `go`, `every`.
- Bindings: `seat:bind{ ["Super+Return"] = fn, Swipe3 = fn }` claims each chord, so no client sees it.

**Frames.** Events of one frame run in arrival order. Their writes form one transaction, applied whole at the next
frame boundary if it lands in the 4 ms budget, else the frame after; never dropped or reordered. Bindings settle in
creation order before it is sent.

**Input.** A pointer is a ray. The order is lock, grab, claimed chords, then the pick: the last-drawn node under
the ray with `input` set. A hit client surface gets protocol events; a `View` with `input = "source"` passes the ray
into its source; any other node gets an `Event`. Keys go to `seat.focus`.

**Lock and trust.** While engaged, only `lock` is drawn and takes input; a shell crash keeps it engaged. The owner's
socket sees the whole tree; a sandboxed client sees standard protocols only.

**Wire, not API.** These protocols carry the nodes and never appear in scripts:

| Class          | Wire                                            |
| -------------- | ----------------------------------------------- |
| `Window`       | xdg-shell, X11, foreign toplevel                |
| `Surface`      | popups, layer shell, session lock, cursor, drag |
| `Screen`       | `wl_output`, output management                  |
| `View:capture` | ext-image-copy-capture, PipeWire                |
| `Workspace`    | ext-workspace                                   |
| `Seat`         | `wl_seat`, libei                                |

## 5a. Syntax: the sugar and how it is built

### 5a.1 What pass 1 got wrong

| Pass 1 construct                                    | What was wrong                                              | Pass 2                                             |
| --------------------------------------------------- | ----------------------------------------------------------- | -------------------------------------------------- |
| `gnoblin.new("Window", { ... })`                    | type named by a string; prefix; no completion               | `Window{ ... }`                                    |
| `new` then `parent:add(child)`                      | the tree's shape is invisible in the code                   | children listed in the constructor                 |
| `n:on("child_added", fn)`                           | signal named by a string; a typo is silent                  | `on_child_added = fn`, `n.child_added:connect(fn)` |
| `n:bind("width", fn)`                               | a second way to set a property, named by a string           | `width = function() ... end`                       |
| `gnoblin.spawn(function() ... :await() end)`        | ceremony around every animation                             | every handler is a coroutine                       |
| `n:tween(props, s, "ease_out")` and a `Tween` child | a node for what is a property write; easing by name         | `n:to(s, props, ease.out)`                         |
| `gnoblin.new("Effect", { kind = "shadow" })`        | a node with a meaningless transform; layouts had to skip it | `effects = { { shadow = 24 } }`                    |
| `win.meta.panel`, `xr.meta.metres_per_unit`         | an untyped table doing a class's job                        | class fields; `Camera.metres`                      |
| `tags.dock = true`, `find("#dock")`                 | `#` meant tag; one kind of lookup                           | `.dock`, `#name`, types, attributes                |
| `gnoblin.on("window_added", fn)` and four more      | five global signals for one fact: a node arrived            | `function Window:on_ready()`                       |
| `M.attach(ws, opts)` returning closures             | a hand-made object per layout; options as a loose table     | a class with properties                            |
| `seat:bind(chord, fn)`, `seat:gesture(spec, fn)`    | two calls for one idea                                      | `seat:bind{ chord = fn, ... }`                     |
| `ws:children("Window")`, `w.type ~= "Window"`       | types as strings                                            | `ws:all(Window)`, `w:is(Window)`                   |
| `{ placement = "cascade" }`                         | a mode as a string enum                                     | the subclass `Cascade`                             |
| `lock:engage()`, `lock:release()`                   | two methods for one property                                | `lock.engaged = true`                              |
| `shell.screens[screen].ws[i]`                       | bookkeeping tables beside the tree                          | `desk().space.children[i]`                         |

### 5a.2 What comes from where

| Source           | Taken                                                                                                                                                                                       | Not taken, and why                                                                                                                |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Godot            | `class_name` (a global class), `extends`, typed defaults like `@export`, signals as values, `await signal`, `_ready`, groups as tags, scenes as classes with `build`, autoloads as globals  | `_process`: motion is native and the shell wakes only for input, timers and changes; `_input` apart from signals; `Tween` objects |
| DOM and HTML     | nesting, text children, `onclick` beside `addEventListener`, `querySelector`, `closest`, `classList`, `dataset`, a path fixed before dispatch, bubbling, `stopPropagation`, custom elements | the capture phase: every client event would wait on the shell VM; an overlay with `input = true` does the same job                |
| QML, Quickshell  | declarative trees, bindings with automatic dependencies, `Component` as a class, the default property as listed children                                                                    | `Variants`: `arrange` reconciles children (the VR ring); a second language                                                        |
| Solid, Vue       | effects that track what they read, batched, in creation order                                                                                                                               | separate signal and memo types: properties already are the signals                                                                |
| Flutter, SwiftUI | `build` returning children                                                                                                                                                                  | rebuilding a subtree on change: windows have identity and cannot be rebuilt                                                       |
| Textual          | CSS selectors over a widget tree                                                                                                                                                            | stylesheets: properties are set in code                                                                                           |
| Lua 5.5          | `__call`, `__index`, `__newindex`, `__tostring`, call sugar `f"s"` and `f{}`, `:` methods, coroutines, weak tables                                                                          | `__close`, operator paths (`root / "a"`), `__concat` children, `global` declarations: obscure or unneeded                         |

### 5a.3 Each sugar and the plain Lua it is

| Written                              | Plain Lua it runs as                                                           |
| ------------------------------------ | ------------------------------------------------------------------------------ |
| `Node:extend "Panel" { h = 32 }`     | `Node.extend(Node, "Panel")({ h = 32 })`, which also sets `_G.Panel`           |
| `Panel{ x = 1, child }`              | `getmetatable(Panel).__call(Panel, { x = 1, child })`                          |
| `n.width = function() ... end`       | `__newindex(n, "width", fn)`: a binding                                        |
| `on_closed = fn`, `n.on_closed = fn` | `n.closed:connect(fn)`                                                         |
| `function Window:on_closed() end`    | `Window.on_closed = fn`; `__newindex` on the class adds a class handler        |
| `n.closed`                           | `__index`: a `Signal` value holding the node and the name                      |
| `await(n.closed)`                    | register the running coroutine, then `coroutine.yield()`                       |
| `n"#clock"`                          | `n:find("#clock")`, through `__call` on instances                              |
| `ws:all"> Window"`                   | `ws.all(ws, "> Window")`                                                       |
| `seat:bind{ ["Super+1"] = fn }`      | `seat.bind(seat, { ["Super+1"] = fn })`                                        |
| `n:to(0.2, { x = 0 }, ease.out)`     | `core.set(id, "x", 0, 0.2, samples)` per key, plus a timer that emits a signal |
| a string child `"Hi"`                | `Text{ text = "Hi" }`                                                          |
| `tostring(Window)`                   | `"Window"`: why a class works as a selector                                    |

### 5a.4 The native core

Everything in sections 5 and 6 is Lua except this list:

1. Ten classes from the schema, each with a `base`, its properties (type, default), signals and methods: `Node`,
   `Surface`, `Window`, `Canvas`, `View`, `Screen`, `Camera`, `Seat`, `Workspace`, `Lock`.
2. Nine tree calls:

    | Call                                         | Does                                                              |
    | -------------------------------------------- | ----------------------------------------------------------------- |
    | `core.classes()`                             | the schema, parent classes first                                  |
    | `core.root()`                                | the root's id                                                     |
    | `core.new(class)`                            | a new, unparented node; returns its id                            |
    | `core.get(id, key)`                          | a value from the mirror; node values are ids                      |
    | `core.set(id, key, value, seconds, samples)` | a write; animated when `seconds` is given                         |
    | `core.insert(parent, id, index)`             | parent or move a node                                             |
    | `core.destroy(id)`                           | end a node; returns the references that became nil                |
    | `core.call(id, method, ...)`                 | a native method; a slow one returns a token, answered as an event |
    | `core.report(message)`                       | a script error, to the log                                        |

3. The frame clock: the host calls `tick(now, changes, events)` (the function `node.lua` returns) once per frame
   that has input, or at the time `tick` last returned. It sends the writes as one transaction when `tick` returns.
4. `sys`: `spawn(command or argv)` returns a process with `kill()`; `read(path)`, `write(path, data)`;
   `date(format)`, recorded as input; `connect(socket path)`.

### 5a.5 The class system

`data/lua/gnoblin/node.lua`, complete. It is plain Lua 5.5 and ran unchanged in the scratch tests of section 10.

```lua
-- data/lua/gnoblin/node.lua: classes, properties, bindings, signals, motion, selectors, scheduler.
-- Plain Lua 5.5 over the native core. The host runs it once and calls the function it returns.
local core = ...
local CLASS, ID, VALS, BOUND, HANDLERS = {}, {}, {}, {}, {}  -- private keys: no name can collide
local nodes, classes, timers = {}, {}, {}                    -- id -> node, name -> class
local now, seq, current, queue = 0, 0, nil, {}
local Class, Instance, Signal = {}, {}, {}                   -- the three metatables
signal = setmetatable({}, { __name = "signal" })             -- marker: `pressed = signal`
local function meta(c) return rawget(c, CLASS) end

-- Reactivity. A binding re-runs when a property it read changes, never because of its own write.
local readers = setmetatable({}, { __mode = "k" })            -- readers[node][key] = { [binding] = true }
local function track(n, k)
  if not current then return end
  local by = readers[n] or {}; readers[n] = by
  local set = by[k] or {}; by[k] = set
  set[current] = true; current.sources[#current.sources + 1] = set
end
local function touch(n, k)
  for b in pairs(readers[n] and readers[n][k] or {}) do
    if b ~= current and not b.queued then b.queued = true; queue[#queue + 1] = b end
  end
end
local function run(b)
  for _, set in ipairs(b.sources) do set[b] = nil end
  b.sources, b.queued = {}, false
  local outer = current; current = b
  local ok, err = pcall(b.fn)
  current = outer
  if not ok then core.report(err) end
end
local function bind(fn)
  seq = seq + 1
  local b = { fn = fn, seq = seq, sources = {} }
  run(b); return b
end
local function flush()
  for _ = 1, 100 do
    if #queue == 0 then return end
    local batch = queue; queue = {}
    table.sort(batch, function(a, b) return a.seq < b.seq end)  -- creation order: deterministic
    for _, b in ipairs(batch) do if not b.dead then run(b) end end
  end
  core.report("bindings did not settle in 100 passes: two bindings feed each other")
end

-- Scheduler. Every handler runs in its own coroutine, so `await` works anywhere but in a binding.
-- A coroutine belongs to the node whose handler started it and stops when that node is destroyed.
local owners = setmetatable({}, { __mode = "k" })
local function resume(co, ...)
  local o = owners[co]
  if o and not nodes[rawget(o, ID)] then return end
  local ok, err = coroutine.resume(co, ...)
  if not ok then core.report(err) end
end
local function start(o, fn, ...)
  local co = coroutine.create(fn)
  owners[co] = o; resume(co, ...); return co
end
function go(fn, ...) return start(owners[coroutine.running()], fn, ...) end
local function emit(n, name, ...)
  local chain, c = {}, rawget(n, CLASS)
  while c do table.insert(chain, 1, c); c = meta(c).base end   -- base class handlers first
  for _, k in ipairs(chain) do
    for _, fn in ipairs(meta(k).handlers[name] or {}) do start(n, fn, n, ...) end
  end
  local l = rawget(n, HANDLERS)
  local list, keep = l[name] or {}, {}
  l[name] = nil
  for _, h in ipairs(list) do
    if h.fn then keep[#keep + 1] = h; start(n, h.fn, n, ...) else resume(h.co, ...) end
  end
  for _, h in ipairs(l[name] or {}) do keep[#keep + 1] = h end   -- handlers added meanwhile
  l[name] = #keep > 0 and keep or nil
end
local function listen(n, name, h)
  local l = rawget(n, HANDLERS); l[name] = l[name] or {}
  table.insert(l[name], h)
end
Signal.__index = Signal
function Signal:connect(fn) listen(self.node, self.name, { fn = fn }) end
function Signal:emit(...) emit(self.node, self.name, ...) end
function await(x)
  if current then error("await inside a binding", 2) end
  local co, main = coroutine.running()
  if main then error("await outside a handler", 2) end
  if type(x) == "number" then
    seq = seq + 1; timers[#timers + 1] = { at = now + x, seq = seq, co = co }
  else
    listen(x.node, x.name, { co = co })
  end
  return coroutine.yield()
end
function every(seconds, fn) return go(function() while true do fn(); await(seconds) end end) end

-- Nodes. Values of native properties live in the native tree; Lua properties live in VALS.
local make
local function wrap(id)                                      -- a node Gnoblin made gets its Lua face
  if id == nil or nodes[id] then return nodes[id] end
  return make(classes[core.get(id, "class")], id)
end
local function read(n, k, p)
  local b = rawget(n, BOUND)[k]
  if b and b.queued then run(b) end                          -- a stale binding settles before a read
  track(n, k)
  if not p.native then return rawget(n, VALS)[k] end
  local v = core.get(rawget(n, ID), k)
  if p.type == "node" then return wrap(v) end
  if p.type == "nodes" then local out = {}; for i, id in ipairs(v) do out[i] = wrap(id) end; return out end
  return v
end
local function write(n, k, p, v, seconds, curve)
  if p.native then
    core.set(rawget(n, ID), k, p.type == "node" and v and rawget(v, ID) or v, seconds, curve)
  else
    local t = p.type
    if v ~= nil and t ~= "any" and (type(t) == "string" and type(v) ~= t or type(t) == "table" and not v:is(t)) then
      error(("%s.%s takes %s, not %s"):format(meta(rawget(n, CLASS)).name, k, tostring(t), tostring(v)), 3)
    end
    if rawget(n, VALS)[k] == v then return end
    rawget(n, VALS)[k] = v
  end
  touch(n, k)
end
Instance.__index = function(n, k)
  local m = meta(rawget(n, CLASS))
  local p = m.props[k]
  if p then return read(n, k, p) end
  if m.signals[k] then return setmetatable({ node = n, name = k }, Signal) end
  return rawget(n, CLASS)[k]                                  -- methods, through the class chain
end
Instance.__newindex = function(n, k, v)
  local m = meta(rawget(n, CLASS))
  local name = type(k) == "string" and k:match("^on_(.+)$")
  if name and m.signals[name] then return listen(n, name, { fn = v }) end
  local p = m.props[k] or error(("%s has no property '%s'"):format(m.name, tostring(k)), 2)
  local old = rawget(n, BOUND)[k]
  if old then old.dead = true; rawget(n, BOUND)[k] = nil end
  if type(v) == "function" then                              -- a function value is a binding
    rawget(n, BOUND)[k] = bind(function() write(n, k, p, v(n)) end)
  else
    write(n, k, p, v)
  end
end
Instance.__call = function(n, selector) return n:find(selector) end
Instance.__tostring = function(n) return meta(rawget(n, CLASS)).name .. "#" .. tostring(n.name) end

-- Classes. `Panel{ ... }` constructs; `function Panel:on_x()` adds a handler for every Panel.
function make(cls, id, spec)                                 -- spec is nil for a node Gnoblin made
  local m = meta(cls)
  local n = setmetatable({ [CLASS] = cls, [ID] = id, [VALS] = {}, [BOUND] = {}, [HANDLERS] = {} }, Instance)
  nodes[id] = n
  local keys = {}
  for k in pairs(spec or {}) do if type(k) == "string" then keys[#keys + 1] = k end end
  table.sort(keys)
  for pass = 1, 3 do                                         -- plain defaults, the call's fields, bound defaults
    if pass == 2 then for _, k in ipairs(keys) do n[k] = spec[k] end end
    for _, k in ipairs(pass ~= 2 and m.order or {}) do
      local p, d = m.props[k], m.props[k].default
      local skip = spec and spec[k] ~= nil or not spec and p.native or (type(d) == "function") ~= (pass == 3)
      if type(d) == "table" and getmetatable(d) == nil then local t = {}; for i, v in pairs(d) do t[i] = v end; d = t end
      if not skip then n[k] = d end
    end
  end
  for _, c in ipairs(n:build() or {}) do n:add(c) end
  for _, c in ipairs(spec or {}) do n:add(c) end
  if n.arrange ~= classes.Node.arrange then rawget(n, BOUND)[CLASS] = bind(function() n:arrange() end) end
  return n
end
local function construct(cls, spec) return make(cls, core.new(meta(cls).native), spec or {}) end
Class.__index = function(c, k) local base = meta(c).base; return base and base[k] end
Class.__call = construct
Class.__tostring = function(c) return meta(c).name end
Class.__newindex = function(c, k, v)                        -- `on_` names add; every other name replaces
  local name = type(k) == "string" and k:match("^on_(.+)$")
  if not (name and meta(c).signals[name]) then return rawset(c, k, v) end
  local h = meta(c).handlers
  h[name] = h[name] or {}; table.insert(h[name], v)
end
local function declare(base, name, native)
  local bm = base and meta(base)
  local m = { name = name, base = base, native = native or bm.native, handlers = {}, order = {},
              props = setmetatable({}, { __index = bm and bm.props }),
              signals = setmetatable({}, { __index = bm and bm.signals }) }
  local c = setmetatable({ [CLASS] = m }, Class)
  classes[name] = c; _G[name] = c                            -- usable everywhere by its name
  return c, m
end
local function settle(m)                                     -- every inherited default, sorted by name
  local seen, t = {}, m.props
  m.order = {}
  while t do
    for k, p in pairs(t) do
      if not seen[k] then seen[k] = true; if m.props[k].default ~= nil then m.order[#m.order + 1] = k end end
    end
    t = getmetatable(t).__index
  end
  table.sort(m.order)
end
local function extend(base, name)
  if classes[name] then error("class " .. name .. " is already declared", 2) end
  return function(spec)
    local c, m = declare(base, name)
    for k, v in pairs(spec) do
      local p = m.props[k]
      if v == signal then m.signals[k] = true
      elseif p then m.props[k] = { native = p.native, type = p.type, default = v }   -- a new default
      elseif getmetatable(v) == Class then m.props[k] = { type = v }                 -- a node of that class
      else m.props[k] = { type = type(v) == "function" and "any" or type(v), default = v } end
    end
    settle(m)
    return c
  end
end

-- The native classes, parent first, from the schema. A method that takes time returns a signal.
for _, s in ipairs(core.classes()) do
  local c, m = declare(s.base and classes[s.base], s.name, s.name)
  for k, p in pairs(s.props) do m.props[k] = { native = true, type = p.type, default = p.default } end
  for _, sig in ipairs(s.signals) do m.signals[sig] = true end
  for _, f in ipairs(s.methods) do
    rawset(c, f, function(n, ...)
      local v = core.call(rawget(n, ID), f, ...)
      if s.slow and s.slow[f] then return setmetatable({ node = n, name = v }, Signal) end
      return v
    end)
  end
  settle(m)
end
local Node = classes.Node
meta(Node).props.tags = { type = "table", default = {} }
meta(Node).props.data = { type = "table", default = {} }
for _, c in pairs(classes) do settle(meta(c)) end
rawset(Node, "extend", extend)
function Node:build() end
function Node:arrange() end
root = wrap(core.root())
local compound
function Node:is(sel)                                        -- a class, or any selector without spaces
  if getmetatable(sel) ~= Class then return compound(sel)(self) end
  local k = rawget(self, CLASS)
  while k and k ~= sel do k = meta(k).base end
  return k ~= nil
end
function Node:add(child, index)
  if type(child) == "string" then child = classes.Text{ text = child } end
  local first = core.get(rawget(child, ID), "parent") == nil
  core.insert(rawget(self, ID), rawget(child, ID), index)
  touch(self, "children"); touch(child, "parent")
  if first then emit(child, "ready") end
  emit(self, "child_added", child)
  return child
end
function Node:destroy()
  for _, c in ipairs(self.children) do c:destroy() end
  for _, b in pairs(rawget(self, BOUND)) do b.dead = true end
  local p = self.parent
  local cleared = core.destroy(rawget(self, ID))              -- references to it become nil
  nodes[rawget(self, ID)] = nil
  if p then touch(p, "children") end
  for _, c in ipairs(cleared) do touch(wrap(c[1]), c[2]) end
end
function Node:raise() self.parent:add(self) end             -- the last child draws on top

-- Motion. `node:to(0.2, { x = 0 }, ease.out)` hands Gnoblin a sampled curve and returns a signal.
ease = {
  linear = function(t) return t end,
  out = function(t) return 1 - (1 - t) ^ 2 end,
  out_cubic = function(t) return 1 - (1 - t) ^ 3 end,
  in_cubic = function(t) return t ^ 3 end,
  in_out = function(t) return t < 0.5 and 2 * t * t or 1 - (2 - 2 * t) ^ 2 / 2 end,
}
function Node:to(seconds, props, curve_fn)
  local curve = {}
  for i = 0, 32 do curve[i + 1] = (curve_fn or ease.out)(i / 32) end
  local keys = {}
  for k in pairs(props) do keys[#keys + 1] = k end
  table.sort(keys)
  for _, k in ipairs(keys) do
    write(self, k, meta(rawget(self, CLASS)).props[k] or error("no property " .. k, 2), props[k], seconds, curve)
  end
  local done = setmetatable({ node = self, name = {} }, Signal)   -- a fresh name per motion
  seq = seq + 1; timers[#timers + 1] = { at = now + seconds, seq = seq, signal = done }
  return done
end

-- Selectors: `Type.tag#name[prop=value]`, joined by a space (descendant) or `>` (child).
local compiled = {}
function compound(s)
  local tests, t = {}, s:match("^[%a_][%w_]*")
  if t then local c = classes[t] or error("no class " .. t, 4); tests[1] = function(n) return n:is(c) end end
  for kind, word in s:gmatch("([.#])([%w_-]+)") do
    tests[#tests + 1] = kind == "." and function(n) return n.tags[word] end or function(n) return n.name == word end
  end
  for key, val in s:gmatch("%[([%w_]+)=?([^%]]*)%]") do
    tests[#tests + 1] = function(n)
      local v = n[key]
      if val == "" then return v ~= nil and v ~= false end
      return tostring(v) == val
    end
  end
  return function(n) for _, f in ipairs(tests) do if not f(n) then return false end end; return true end
end
local function compile(sel)
  local s = tostring(sel)
  if not compiled[s] then
    local steps = {}
    for comb, part in (" " .. s):gmatch("%s*(>?)%s*([^%s>]+)") do
      steps[#steps + 1] = { child = comb == ">", test = compound(part) }
    end
    compiled[s] = steps
  end
  return compiled[s]
end
local function matches(n, steps, i, scope)
  if not steps[i].test(n) then return false end
  local p = n.parent
  if i == 1 then return not steps[1].child or p == scope end
  if steps[i].child then return p ~= scope and matches(p, steps, i - 1, scope) end
  while p and p ~= scope do
    if matches(p, steps, i - 1, scope) then return true end
    p = p.parent
  end
  return false
end
function Node:all(sel)
  local steps, out = compile(sel), {}
  local shallow = #steps == 1 and steps[1].child
  local function walk(n)
    for _, c in ipairs(n.children) do
      if matches(c, steps, #steps, self) then out[#out + 1] = c end
      if not shallow then walk(c) end
    end
  end
  walk(self); return out
end
function Node:find(sel) return self:all(sel)[1] end
function Node:closest(sel)
  local test, n = compound(tostring(sel)), self
  while n and not test(n) do n = n.parent end
  return n
end
function Node.describe(c)                                    -- for the stub and docs generator
  local m, props, signals = meta(c), {}, {}
  for k, p in pairs(m.props) do props[k] = { type = tostring(p.type), default = p.default, native = p.native } end
  for k in pairs(m.signals) do signals[#signals + 1] = k end
  table.sort(signals)
  return { name = m.name, base = m.base and meta(m.base).name, props = props, signals = signals }
end

-- The one entry the host calls each frame: the frame clock, changed native properties, events.
return function(t, changes, events)
  now = t
  for _, c in ipairs(changes) do touch(wrap(c[1]), c[2]) end
  for _, e in ipairs(events) do
    if e.bubbles then                                         -- input: the path is fixed first, as in the DOM
      e.target = wrap(e.target)
      e.stop = function(self) self.stopped = true end
      local path, n = {}, e.target
      while n do path[#path + 1] = n; n = n.parent end
      for _, at in ipairs(path) do
        if e.stopped then break end
        if nodes[rawget(at, ID)] then emit(at, e.type, e) end
      end
    else
      local n = wrap(e.target)
      emit(n, e.type, table.unpack(e.args or {}))
      if e.type == "destroyed" then nodes[e.target] = nil end  -- Gnoblin ended it; handlers have run
    end
  end
  table.sort(timers, function(a, b) return a.at < b.at or a.at == b.at and a.seq < b.seq end)
  while timers[1] and timers[1].at <= now do
    local due = table.remove(timers, 1)
    if due.signal then due.signal:emit() else resume(due.co) end
  end
  flush()
  return timers[1] and timers[1].at                           -- when to call again without input
end
```

Limits of this code:

- `to` on a Lua property sets it at once. Only native properties are drawn, so only they move.
- Mutating `n.tags` in place does not notify bindings; assign a new table.
- `e.x` and `e.y` arrive from the host in the target's units; the code does not recompute them per ancestor.

### 5a.6 What cannot be Lua

| Stays Rust                                             | Why                                                                                      |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------- |
| Motion interpolation                                   | it runs every frame on the compositor's clock; a busy shell must not stall animation     |
| The pick and delivery to clients                       | pointer motion reaches a client without waiting on the shell; `input` decides in advance |
| Chord canonicalisation and claims, grabs               | a key is consumed or passed before any Lua runs                                          |
| `Canvas` text shaping and painting                     | a hot path with a font stack                                                             |
| The lock rule                                          | it must hold after the shell crashes                                                     |
| Protocols, capture, the privacy indicator, the sandbox | they are the wire and the edge                                                           |

### 5a.7 Generalisations: kept and rejected

Each kept idea removes a concept or a line of ceremony.

| Idea                                          | Deletes                                                                           |
| --------------------------------------------- | --------------------------------------------------------------------------------- |
| A function value is a binding                 | `bind`, the `changed` signal, `child_added` and `child_removed` wiring in layouts |
| `arrange` is a binding                        | `attach(ws, opts)`, per-layout event wiring                                       |
| Arrival is `ready` on a class                 | `window_added`, `surface_added`, `screen_added`, `screen_removed`, `seat_added`   |
| Selectors                                     | `children(type)`, `find(name)`, tag groups, the rule engine's match keys          |
| Placement modes are subclasses                | the `placement` enum and its validation                                           |
| A motion is an animated write                 | the `Tween` node and its `finished`, `stop` and `await`                           |
| Effects are data                              | the `Effect` node and the layout rule that skipped it                             |
| Every handler is a coroutine                  | `gnoblin.spawn`, `tween:await`                                                    |
| Window rules are an `if` in `Window:on_ready` | `rules.rs`, `MAX_RULES`, apply-once                                               |
| A dialog is its parent's child                | dialog centring as a rule, transient stacking                                     |
| One `Event` type for all input                | per-signal argument shapes                                                        |
| No `remove()`: a node is moved or destroyed   | orphans that nobody owns                                                          |

Rejected:

- **Signals and properties as one mechanism.** A property has a value, a signal an occurrence. Merging them needs
  an event on every write or a value on every signal. Tracked bindings already delete the overlap that mattered,
  the `changed` signal.
- **`Workspace` as a Lua class only.** ext-workspace must know which nodes to publish. Lua-only would need a
  "publish me" flag on `Node`: a special case. A ten-line native class is smaller.
- **`Lock` as a stock class.** Fail-closed must hold with no shell.
- **Seat bindings as selectors over events.** Chords are claimed before dispatch; a table of chords is that claim.
- **DOM capture.** Covered in 5a.2.
- **Operator sugar** (`root / "bar" / "clock"`, `a .. b` for children). Obscure, and a second way to do what
  selectors and constructor lists do.
- **`Text"hi"`.** It needs a per-class default property. A string child already makes a `Text`.

## 6. Scripts on the target model

Every script here is a whole file in the user's config directory, run after the stock library. Ready signals for the
nodes already under `root` are delivered after all scripts have loaded, so a script declares and Gnoblin's nodes
arrive afterwards.

### 6.0 The stock library

The widgets, in `data/lua/gnoblin/shell/ui.lua`:

```lua
-- data/lua/gnoblin/shell/ui.lua: the stock widgets. Each is a Box, as each HTML element is a box.
Canvas:extend "Box" {
  fill = "#00000000", radius = 0, direction = "none", gap = 0, padding = 0, grow = 0,
  draw = function(b) return { { rect = { 0, 0, b.width, b.height }, radius = b.radius, fill = b.fill } } end,
}
function Box:arrange()                     -- "row" or "column": a small flexbox; "none": children stay put
  if self.direction == "none" then return end
  local main, size, cross, across = "x", "width", "y", "height"
  if self.direction == "column" then main, size, cross, across = "y", "height", "x", "width" end
  local kids, used, grow, pad = self:all("> Box"), 0, 0, self.padding
  for _, c in ipairs(kids) do
    if c.grow > 0 then grow = grow + c.grow else used = used + c[size] end
  end
  local free = self[size] - 2 * pad - self.gap * math.max(#kids - 1, 0) - used
  local at = pad
  for _, c in ipairs(kids) do
    if c.grow > 0 then c[size] = math.max(0, free * c.grow // grow) end
    c[main], c[cross], c[across] = at, pad, self[across] - 2 * pad
    at = at + c[size] + self.gap
  end
end

Box:extend "Text" {
  text = "", size = 14, color = "#cdd6f4", font = "sans",
  width = function(t) return (t:measure(t.text, t.size, t.font)) end,
  height = function(t) return t.size * 3 // 2 end,
  draw = function(t) return { { text = t.text, size = t.size, color = t.color, font = t.font } } end,
}

Box:extend "Icon" {
  icon = "", size = 16,
  width = function(i) return i.size end,
  height = function(i) return i.size end,
  draw = function(i) return { { icon = i.icon, size = i.size } } end,
}

Box:extend "Grid" {}
function Grid:arrange()                    -- children in a near-square grid
  local kids, pad, gap = self.children, self.padding, self.gap
  local cols = math.max(1, math.ceil(math.sqrt(#kids)))
  local rows = math.max(1, math.ceil(#kids / cols))
  local w = (self.width - 2 * pad - (cols - 1) * gap) // cols
  local h = (self.height - 2 * pad - (rows - 1) * gap) // rows
  for i, c in ipairs(kids) do
    c.x, c.y = pad + (i - 1) % cols * (w + gap), pad + (i - 1) // cols * (h + gap)
    c.width, c.height = w, h
  end
end

Box:extend "Bar" { edge = "top", height = 32, fill = "#1e1e2e", padding = 6, gap = 12, direction = "row" }

Text:extend "Clock" { format = "%H:%M" }
function Clock:on_ready() every(1, function() self.text = date(self.format) end) end

Box:extend "Entry" { text = "", secret = false, size = 14, padding = 12, direction = "row", input = true,
                     submit = signal }
function Entry:build()
  return { Text{ grow = 1, size = self.size, text = function()
    return self.secret and ("*"):rep(utf8.len(self.text)) or self.text
  end } }
end
function Entry:on_key(e)
  if e.key == "Return" then self.submit:emit(self.text)
  elseif e.key == "BackSpace" then self.text = self.text:sub(1, (utf8.offset(self.text, -1) or 1) - 1)
  elseif e.text then self.text = self.text .. e.text end
end
```

The shell, in `data/lua/gnoblin/shell/init.lua`. It is the whole of today's window-management policy that the
scripts below rely on:

```lua
-- data/lua/gnoblin/shell/init.lua: the stock shell. Copy it and edit it.
seat, lock = root:find(Seat), root:find(Lock)
spawn, date = sys.spawn, sys.date
function desk() return seat.screen.source end                 -- the Desk under the pointer

-- Key and gesture bindings: Gnoblin claims each chord, so no client sees it.
function Seat:bind(map)
  local keys = self.data.keys or {}
  local chords = {}
  for chord in pairs(map) do chords[#chords + 1] = chord end
  table.sort(chords)
  for _, chord in ipairs(chords) do keys[self:claim(chord, map[chord] ~= false)] = map[chord] or nil end
  self.data.keys = keys
end
function Seat:on_chord(e) local fn = self.data.keys[e.chord]; if fn then fn(e) end end

-- One Desk per Screen: bars on its edges, workspaces in the space between.
Node:extend "Desk" {
  screen = Screen, active = Workspace, space = Node,
  width = function(d) return d.screen.width end,
  height = function(d) return d.screen.height end,
}
function Desk:workspace(name) return Floating{ name = name } end   -- override to choose a layout
function Desk:build()
  self.space = Node{ clip = true }
  for i = 1, 9 do self.space:add(self:workspace(tostring(i))) end
  for i, ws in ipairs(self.space.children) do ws.visible, ws.active = i == 1, i == 1 end
  self.active = self.space.children[1]
  return { self.space }
end
function Desk:arrange()
  local top, bottom = 0, 0
  for _, bar in ipairs(self:all("> Bar")) do
    bar.width = self.width
    if bar.edge == "top" then bar.y, top = top, top + bar.height
    else bottom = bottom + bar.height; bar.y = self.height - bottom end
  end
  local s = self.space
  s.y, s.width, s.height = top, self.width, self.height - top - bottom
  for _, ws in ipairs(s.children) do ws.width, ws.height = s.width, s.height end
end
function Desk:show(ws)
  local from = self.active
  if not ws or ws == from then return end
  local dir, W = ws.index > from.index and 1 or -1, self.width
  ws.x, ws.visible, ws.active, from.active, self.active = dir * W, true, true, false, ws
  from:to(0.25, { x = -dir * W }, ease.out_cubic)
  await(ws:to(0.25, { x = 0 }, ease.out_cubic))
  if self.active ~= from then from.visible, from.x = false, 0 end
end
function Desk:step(n) self:show(self.space.children[self.active.index + n]) end
function Workspace:on_activate() self:closest(Desk):show(self) end
function Workspace:walk(step)                                   -- focus the next window in tree order
  local wins, f = self:all(Window), seat.focus
  for i, w in ipairs(wins) do
    if w == f then seat.focus = wins[i + step] or f; return end
  end
end
function Screen:on_ready() self.source = root:add(Desk{ screen = self }) end
function Screen:on_destroyed()                                 -- unplugged: its windows move to another desk
  local to = root:find(Desk)
  if to == self.source then to = root:all(Desk)[2] end
  for _, w in ipairs(self.source:all("Workspace > Window")) do if to then to.active:add(w) end end
  self.source:destroy()
end
function Window:on_ready()
  local p = self.parent_window
  if p then
    p:add(self)                                                 -- a dialog moves with its parent
    self.x, self.y = (p.width - self.committed_width) // 2, (p.height - self.committed_height) // 2
  else
    desk().active:add(self)
  end
  self.activated = function(w) return seat.focus == w end
  seat.focus = self
end
function Window:on_press(e) seat.focus = self; self:raise(); e:stop() end
function Window:on_move(e) drag(e.seat, self) end
function Window:on_closed() self:vanish() end
function Window:vanish() self:destroy() end                   -- a method: replace it to animate

function drag(s, w)                                             -- follow the pointer until release
  local dx, dy = w.x - s.x, w.y - s.y
  w.x = function() return s.x + dx end
  w.y = function() return s.y + dy end
  s:grab(w); await(w.release); s:grab(nil)
  w.x, w.y = w.x, w.y                                           -- a plain write ends the bindings
end

function toast(text)
  go(function()
    local d = desk()
    local t = d:add(Box{ x = d.width - 340, y = 44, width = 320, height = 64, radius = 10, padding = 14,
                         direction = "row", fill = "#45475a", opacity = 0, text })
    await(t:to(0.12, { opacity = 1 })); await(4); await(t:to(0.3, { opacity = 0 }))
    t:destroy()
  end)
end
```

`drag` shows what bindings buy: the window's position is bound to the pointer while the button is down, and one
plain write ends it.

### 6.1 Tiling: master and stack

```lua
-- data/lua/gnoblin/shell/tiling.lua: the first window on the left, the rest stacked on the right.
Workspace:extend "Tiling" { ratio = 0.55, gap = 8 }

function Tiling:arrange()
  local wins = self:all("> Window")
  local n, W, H, gap = #wins, self.width, self.height, self.gap
  local main = n == 1 and W or math.floor((W - gap) * self.ratio)
  for i, w in ipairs(wins) do
    if i == 1 then
      w.width, w.height = main, H
      w:to(0.15, { x = 0, y = 0 })
    else
      local h = (H - (n - 2) * gap) // (n - 1)
      w.width, w.height = W - main - gap, h
      w:to(0.15, { x = main + gap, y = (i - 2) * (h + gap) })
    end
  end
end
```

```lua
-- init.lua: every workspace tiles; Super+Return makes the focused window the main one
function Desk:workspace(name) return Tiling{ name = name, ratio = 0.6 } end

seat:bind{
  ["Super+Return"] = function() local w = seat.focus; if w then w.parent:add(w, 1) end end,
}
```

`arrange` reads the child list, the size and `ratio`, so adding, removing, reordering or resizing re-tiles with no
handler.

### 6.2 Floating with cascade and free-space placement

The `Floating`, `Cascade` and `Centred` classes are the proof case in section 8.4. Click-to-focus and raise are the
stock `Window:on_press`.

```lua
-- init.lua: cascade new windows, focus follows the pointer, raise on click (stock)
function Desk:workspace(name) return Cascade{ name = name } end

function Window:on_enter() seat.focus = self end
```

### 6.3 Scrolling strip with focus walking

```lua
-- data/lua/gnoblin/shell/strip.lua: columns in a row; the view follows the focused column.
Workspace:extend "Strip" { column = 0.5, gap = 8, clip = true, row = Node }

function Strip:build()
  self.row = Node{}
  return { self.row }
end

function Strip:on_child_added(w)           -- a window dropped on the strip goes after the focused one
  if not w:is(Window) then return end
  local f = seat.focus
  self.row:add(w, f and f.parent == self.row and f.index + 1 or nil)
end

function Strip:arrange()
  local x, row, f = 0, self.row, seat.focus
  for _, w in ipairs(row.children) do
    w.x, w.y, w.width, w.height = x, 0, math.floor(self.width * self.column), self.height
    x = x + w.width + self.gap
  end
  if f and f.parent == row then
    local left = -row.x
    if f.x < left then
      row:to(0.2, { x = -f.x })
    elseif f.x + f.width > left + self.width then
      row:to(0.2, { x = self.width - f.x - f.width })
    end
  end
end
```

```lua
-- init.lua: scrolling columns; Super+Left and Super+Right walk the focus and the view follows
function Desk:workspace(name) return Strip{ name = name, column = 0.5 } end

seat:bind{
  ["Super+Right"] = function() desk().active:walk(1) end,
  ["Super+Left"] = function() desk().active:walk(-1) end,
}
```

`arrange` reads `seat.focus`, so moving the focus by any means (a key, a click, a new window) scrolls the strip.

### 6.4 Custom titlebar with animated close

```lua
-- decorations.lua: a drawn titlebar, a shadow, and a fade on close
Box:extend "Titlebar" {
  window = Window, y = -28, height = 28, padding = 6, gap = 8, radius = 8, direction = "row", input = true,
  width = function(t) return t.window.width end,
  fill = function(t) return t.window.activated and "#303446" or "#232634" end,
}
function Titlebar:build()
  local w = self.window
  return {
    Text{ grow = 1, size = 13, color = "#c6d0f5", text = function() return w.title end },
    Icon{ icon = "window-close-symbolic", on_press = function(_, e) e:stop(); w:close() end },
  }
end
function Titlebar:on_press(e) drag(e.seat, self.window) end

function Window:on_ready()
  self.decorated = true
  self.effects = { { shadow = 24, opacity = 0.4 } }
  self:add(Titlebar{ window = self })
end

function Window:vanish()                    -- replaces the stock method: fade, then go
  await(self:to(0.18, { opacity = 0, scale = 0.92 }, ease.in_cubic))
  self:destroy()
end
```

The new `Window:on_ready` runs after the stock one, so the window is already placed when it is decorated.

### 6.5 Panel, launcher and notification overlay

```lua
-- panel.lua: a bar on every desk
function Desk:on_ready()
  self:add(Bar{
    Text{ text = function() return "workspace " .. self.active.name end },
    Box{ grow = 1 },
    Clock{},
  })
end

-- launcher.lua: type a command, Return runs it
seat:bind{
  ["Super+Space"] = function()
    local d = desk()
    seat.focus = d:add(Entry{
      x = d.width // 2 - 300, y = 200, width = 600, height = 48, radius = 12, fill = "#313244", size = 18,
      on_submit = function(box, command) spawn(command); box:destroy() end,
      on_key = function(box, e) if e.key == "Escape" then box:destroy() end end,
    })
  end,
}

-- notifications: any handler calls toast(text); the stock shell draws it
function Window:on_closed() toast(self.title .. " closed") end
```

The stock `Desk:arrange` docks every `Bar` and shrinks the workspaces, which replaces exclusive zones for shell UI.
A layer-shell client still works: it arrives as a `Surface` with `role = "layer"`, and a stock `Surface:on_ready`
places it by the wlr rules (not written in this pass). A notification D-Bus server and a desktop-entry index are
services outside the node set, and neither exists today.

### 6.6 Overview with live thumbnails

```lua
-- overview.lua: a live thumbnail of every window; click one to focus it
Grid:extend "Overview" { input = true, opacity = 0, gap = 24, padding = 48, effects = { { blur = 30 } } }

function Overview:build()
  local d = desk()
  self.width, self.height = d.width, d.height
  local thumbs = {}
  for _, w in ipairs(d.active:all("> Window")) do
    thumbs[#thumbs + 1] = View{ source = w, input = true, on_press = function() self:pick(w) end }
  end
  return thumbs
end
function Overview:on_ready() self:to(0.15, { opacity = 1 }) end
function Overview:on_press(e) if e.target == self then self:destroy() end end
function Overview:pick(w) seat.focus = w; w:raise(); self:destroy() end

seat:bind{ ["Super+Tab"] = function() desk():add(Overview{}) end }
```

A blur pass samples the backdrop behind its node (`gnoblin-render/src/effect.rs:277-299` at `c28c0c671`), so the
overview blurs the desk under it, not its own thumbnails.

### 6.7 Workspace switcher with animated transitions

```lua
-- workspaces.lua: Super+N slides to workspace N; a three-finger swipe steps
for i = 1, 9 do
  seat:bind{ ["Super+" .. i] = function() desk():show(desk().space.children[i]) end }
end
seat:bind{
  Swipe3 = function(e) if e.phase == "end" then desk():step(e.dx < 0 and 1 or -1) end end,
}
```

The slide is `Desk:show` in 6.0. A taskbar's ext-workspace request arrives as `Workspace:on_activate`. A named
workspace set is a Lua table of workspaces to show.

### 6.8 Screen recorder and remote view

```lua
-- recorder.lua: Super+Shift+R starts and stops a recording of the screen under the pointer
local recording
seat:bind{
  ["Super+Shift+R"] = function()
    if recording then recording.stream:stop(); recording.process:kill(); recording = nil; return end
    local stream = seat.screen:capture{ cursor = true }
    recording = { stream = stream, process = spawn{ "gst-launch-1.0", "pipewiresrc", "path=" .. stream.node,
      "!", "videoconvert", "!", "x264enc", "!", "mp4mux", "!", "filesink", "location=/tmp/recording.mp4" } }
  end,
}

-- remote.lua: a remote viewer sees one screen and may drive this seat
function Seat:on_remote(r)
  toast(r.client .. " is controlling this seat")
  r:accept{ screen = self.screen }
end
```

The privacy indicator and the lock embargo stay: they are mechanism.

### 6.9 Lock screen

```lua
-- lock.lua: Super+L locks; the password field sits on the first screen
Box:extend "LockScreen" { screen = Screen, prompt = false, fill = "#11111b" }

function LockScreen:build()
  local s = self.screen
  self.x, self.y, self.width, self.height = s.x, s.y, s.width, s.height
  if not self.prompt then return end
  return { Entry{ secret = true, width = 320, height = 44, radius = 22, fill = "#313244",
    x = function(e) return (self.width - e.width) // 2 end, y = function(e) return self.height // 2 end,
    on_submit = function(entry, secret)
      entry.text = ""
      if await(lock:authenticate(secret)) then
        for _, c in ipairs(lock.children) do c:destroy() end
        lock.engaged = false
      else
        entry.fill = "#f38ba8"
      end
    end } }
end

function lock_screen()
  if lock.engaged then return end
  lock.engaged = true
  for i, s in ipairs(root:all(Screen)) do lock:add(LockScreen{ screen = s, prompt = i == 1 }) end
  seat.focus = lock:find(Entry)
end

seat:bind{ ["Super+L"] = lock_screen }
function Lock:on_requested() self.engaged = true end   -- an ext-session-lock client draws its own
```

An ext-session-lock client still works: its surfaces appear under `lock` as `Surface` nodes with `role = "lock"`,
sized to their screen.

### 6.10 VR shell

```lua
-- vr.lua: gnoblin-xr shows the node named "room"; the head camera carries the HUD
Box:extend "Hud" { width = 800, height = 80, radius = 16, fill = "#1e1e2ecc", padding = 24, gap = 24,
                   direction = "row" }
function Hud:build()
  return { Clock{ size = 28 }, Text{ size = 28, text = function() return "workspace " .. desk().active.name end } }
end

Node:extend "Ring" { radius = 1800 }        -- the active workspace's windows on a circle round the viewer
function Ring:arrange()
  local wins, views = desk().active:all(Window), {}
  for _, v in ipairs(self.children) do if v.source then views[v.source] = v else v:destroy() end end
  for i, w in ipairs(wins) do
    local v = views[w] or self:add(View{ source = w, input = "source" })
    views[w] = nil
    local a = (i - 1) * 2 * math.pi / #wins
    v.width, v.height = w.width, w.height
    v:to(0.3, { x = math.sin(a) * self.radius, z = -math.cos(a) * self.radius, rotation = { 0, -math.deg(a), 0 } })
  end
  for _, v in ipairs(self.children) do if views[v.source] then v:destroy() end end
end

function Seat:on_ready()                      -- the headset's seat arrives when an XR session starts
  if self.name ~= "xr" then return end
  local room = root:add(Node{ name = "room",
    Camera{ name = "head", projection = "stereo", metres = 0.001,
      View{ source = Hud{}, y = 350, z = -1000, width = 800, height = 80 } },
    Ring{},
  })
  for i, s in ipairs(root:all(Screen)) do     -- each monitor as a panel two metres ahead
    room:add(View{ source = s, input = "source", width = s.width, height = s.height,
                   x = (i - 1) * 2100 - 1000, y = -600, z = -2000 })
  end
  self:bind{ BTN_SIDE = function() desk():step(1) end }
  self.destroyed:connect(function() room:destroy() end)
end
```

The controller ray clicks through a panel into its window because the panel's `input` is `"source"`. `Ring` is
QML's `Variants` done by hand: `arrange` keeps one view per window and drops the rest.

### 6.11 What each script uses and replaces

| Script            | Classes used                                                   | Replaces                                                                            |
| ----------------- | -------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Tiling            | `Workspace`, `Window`, `Seat`                                  | kinds, managed placement, `Reply`, `settle`, layout commands, fixed motion          |
| Floating          | `Workspace`, `Window`, `Seat`                                  | `placement.rs`, dialog rule, `FloatingEngine`, fallback, raise option, `StackLayer` |
| Strip             | `Workspace`, `Node`, `Window`, `Seat`                          | unbounded extent, `ctx.state.view`, `scroll_to`, focus effect                       |
| Titlebar          | `Window`, `Canvas` (`Box`, `Text`, `Icon`)                     | frame protocol and role, always-CSD answer, composer fade, `add_effect`             |
| Panel and friends | `Canvas` (`Bar`, `Clock`, `Entry`), `Seat`                     | layer-shell for shell UI, exclusive zones, overlay band, provider limits            |
| Overview          | `View`, `Canvas` (`Grid`), `Window`, `Seat`                    | shell visuals, `capture-window`, `RegisterLayer`                                    |
| Workspaces        | `Workspace`, `Seat`                                            | `Workspaces`, workspace sets, composer slide, gesture actions                       |
| Recorder, remote  | `Screen`, `Seat`                                               | `CaptureScreen`, `capture-window`, the closed EIS gate                              |
| Lock              | `Lock`, `Canvas` (`Entry`), `Seat`                             | the `lock` band and its compositor-only rule                                        |
| VR                | `Node`, `Camera`, `View`, `Screen`, `Window`, `Canvas`, `Seat` | panel planner, `gnoblin_view_v1`, room, viewer outputs, `depth` key, grants         |

### 6.12 Before and after

Non-blank lines of script. The stock library is counted once, separately: `shell/ui.lua` is 60 lines and
`shell/init.lua` 94.

| Script                         | Pass 1 | Pass 2                                  |
| ------------------------------ | ------ | --------------------------------------- |
| Tiling (layout and `init.lua`) | 43     | 22 (17 for `Tiling`, 5 for `init.lua`)  |
| Titlebar with animated close   | 30     | 24                                      |
| Panel, launcher and toasts     | 42     | 21                                      |
| Workspace switcher             | 25     | 7, with `Desk:show` and `step` in stock |

Tiling, pass 1:

```lua
-- layouts/master_stack.lua
local M = {}
function M.attach(ws, opts)
  opts = opts or {}
  local ratio, gap = opts.ratio or 0.55, opts.gap or 8
  local function arrange()
    local wins = ws:children("Window")
    local n = #wins
    if n == 0 then return end
    local W, H = ws.width, ws.height
    local mw = n == 1 and W or math.floor((W - gap) * ratio)
    for i, w in ipairs(wins) do
      if i == 1 then
        w.width, w.height = mw, H
        w:tween({ x = 0, y = 0 }, 0.15, "ease_out")
      else
        local h = (H - (n - 2) * gap) // (n - 1)
        w.width, w.height = W - mw - gap, h
        w:tween({ x = mw + gap, y = (i - 2) * (h + gap) }, 0.15, "ease_out")
      end
    end
  end
  ws:on("child_added", arrange)
  ws:on("child_removed", arrange)
  ws:on("changed", function(prop) if prop == "width" or prop == "height" then arrange() end end)
  arrange()
  return {
    arrange = arrange,
    promote = function(w) w.index = 1; arrange() end,
    set_ratio = function(r) ratio = r; arrange() end,
  }
end
return M
```

```lua
-- in the user's init.lua
local shell = require("gnoblin.shell")
local tiles = {}
for _, s in pairs(shell.screens) do
  for _, ws in ipairs(s.ws) do tiles[ws] = require("layouts.master_stack").attach(ws, { ratio = 0.6 }) end
end
gnoblin.seat:bind("Super+Return", function()
  local w = gnoblin.seat.focus
  if w and tiles[w.parent] then tiles[w.parent].promote(w) end
end)
```

Pass 2 is 6.1. The three `ws:on` lines, the explicit first `arrange()`, the returned table of closures and the
`tiles` lookup all go: `arrange` is a binding and the layout is a class.

Titlebar, pass 1:

```lua
-- shell/decorations.lua
local M = {}
function M.decorate(win)
  win.decorated = true
  local bar = gnoblin.new("Canvas", { y = -28, height = 28, input = true })
  bar:bind("width", function() return win.width end)
  bar:bind("draw", function()
    local active = win.states.activated
    return {
      { rect = { 0, 0, win.width, 28 }, radius = { 8, 8, 0, 0 }, fill = active and "#303446" or "#232634" },
      { text = win.title, x = 12, y = 6, size = 13, color = "#c6d0f5" },
      { icon = "window-close-symbolic", x = win.width - 24, y = 6, size = 16 },
    }
  end)
  win:add(bar)
  win:add(gnoblin.new("Effect", { kind = "shadow", radius = 24, opacity = 0.4 }))
  bar:on("button", function(e)
    if not e.pressed then return end
    if e.x > bar.width - 32 then win:close() else require("gnoblin.shell.drag").move(e.seat, win) end
  end)
  win:on("closed", function()
    gnoblin.spawn(function()
      win:tween({ opacity = 0, scale = { 0.92, 0.92, 1 } }, 0.18, "ease_in"):await()
      win:destroy()
    end)
  end)
end
return M
```

```lua
-- in the user's init.lua
gnoblin.on("window_added", function(w) require("gnoblin.shell.decorations").decorate(w) end)
```

Pass 2 is 6.4. The hit test by pixel (`e.x > bar.width - 32`) becomes a close `Icon` with its own handler, and the
layout of title and icon is the `Box` row instead of hand-placed coordinates.

Panel, launcher and toasts, pass 1:

```lua
-- shell/panel.lua
for screen, s in pairs(shell.screens) do
  local bar = gnoblin.new("Canvas", { name = "panel", height = 32, input = true })
  bar:bind("width", function() return screen.width end)
  bar:bind("draw", function()
    return {
      { rect = { 0, 0, bar.width, 32 }, fill = "#1e1e2e" },
      { text = gnoblin.date("%H:%M"), x = bar.width // 2 - 20, y = 8, size = 14, color = "#cdd6f4" },
    }
  end)
  s.panels:add(bar)
  s.workspaces.y = 32
  for _, ws in ipairs(s.ws) do ws.height = screen.height - 32 end
end

-- shell/launcher.lua
local function open(s, seat)
  local box = gnoblin.new("Canvas", { x = s.root.width // 2 - 300, y = 200, width = 600, height = 48, input = true })
  local query = ""
  local function draw()
    box.draw = { { rect = { 0, 0, 600, 48 }, radius = 12, fill = "#313244" },
                 { text = query, x = 16, y = 12, size = 18, color = "#cdd6f4" } }
  end
  draw(); s.overlay:add(box); seat.focus = box
  box:on("key", function(e)
    if e.key == "Escape" then box:destroy()
    elseif e.key == "Return" then gnoblin.process.spawn({ "sh", "-c", query }); box:destroy()
    elseif e.key == "BackSpace" then query = query:sub(1, -2); draw()
    elseif e.text then query = query .. e.text; draw() end
  end)
end
gnoblin.seat:bind("Super+Space", function() open(shell.current(), gnoblin.seat) end)

-- shell/toast.lua: any script calls shell.toast(text)
function shell.toast(text)
  local s = shell.current()
  local t = gnoblin.new("Canvas", { x = s.root.width - 340, y = 44, width = 320, height = 64, opacity = 0,
    draw = { { rect = { 0, 0, 320, 64 }, radius = 10, fill = "#45475a" },
             { text = text, x = 14, y = 22, size = 14, color = "#cdd6f4" } } })
  s.overlay:add(t)
  gnoblin.spawn(function()
    t:tween({ opacity = 1 }, 0.12):await(); gnoblin.wait(4); t:tween({ opacity = 0 }, 0.3):await(); t:destroy()
  end)
end
```

Pass 2 is 6.5. Pass 1's clock never updated: its `draw` binding read no property that changed each minute. Pass 2's
`Clock` sets its text every second, and the `Entry` widget absorbs the hand-written key handling.

## 7. Refactor plan

### 7.1 Decisions for the owner

Decisions 1 and 2 stand from pass 1. Decisions 3 and 4 are new in pass 2.

**Decision 1: where the shell VM runs.**

- **(a) In the runtime process, as D1 chose (recommended).** The tree lives in the compositor; the shell keeps a
  mirror and sends transactions over the local socket. A shell crash restarts the shell and the windows stay put.
  The in-compositor layout VM, its worker, abandon-and-replace and leak caps are deleted. Cost: one serialised
  transaction per frame and a mirror's memory (round-trip cost unverified).
- **(b) On a thread inside the compositor.** No serialisation. A fault in the Lua C code ends the whole session,
  and the wedged-worker machinery of D17 stays.

**Decision 2: decorations.**

- **(a) Gnoblin draws them; delete `gnoblin_window_frame_v1` (recommended).** Decorations are `Canvas` children of
  a `Window` (`Titlebar` in 6.4) and entries in its `effects`. Bingux's `bingux-frame` stops working until its
  titlebar is rewritten as Lua, and Gnoblin must gain text and icon painting.
- **(b) Keep the protocol as a second source of decoration surfaces.** `bingux-frame` keeps working, but two
  decoration paths, the `FrameRenderer` role and the region table stay, and frames still need wiring into the tree.

**Decision 3: classes are global names.**

- **(a) `Node:extend "Panel" {}` makes a global `Panel` (recommended).** This is Godot's `class_name`: any script
  can say `Panel{}` and any selector can say `Panel` without a `require`. A second declaration of the same name is
  an error at the second declaration's line. Cost: one shared namespace for the whole shell.
- **(b) Classes are locals that a module returns.** No shared namespace, but every script starts with `require`
  lines, and a selector needs the class object, not a name.

**Decision 4: handlers on a class add up.**

- **(a) `function Window:on_ready()` adds a handler; every other `function Class:name()` replaces (recommended).**
  Decorations, rules and placement each hook a new window without knowing about each other. To change a stock
  behaviour, replace the method its handler calls (`Window:vanish` in 6.4) or edit the copied stock file. Cost: one
  rule that differs from plain Lua assignment, enforced by `__newindex` on classes.
- **(b) Plain Lua: the last definition wins.** No special rule, but two scripts that both hook window arrival
  silently break each other, and every hook needs an explicit `connect` instead of a method.

The steps below assume (a) for all four.

### 7.2 Ordered migration

Each step writes its failure-mode tests first, cuts one area over completely, and deletes what it replaces in the
same change. Builds and tests run on Agent37 only. Pass 2 moves the Lua half earlier: the class system and the
stock library land in step 4 with tests against a fake core, before any policy is cut over.

| Step | What                                   | Crates and files                                                                                                              | Green at the end                                  |
| ---- | -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| 1    | Tree contract tests                    | `gnoblin-render/tests/tree.rs`, `gnoblin-model/tests/model.rs`                                                                | all existing suites                               |
| 2    | Scene becomes the one authority inside | `compose.rs` (drop `order`, opacity writers), `state.rs:3013` (ray pick on `Scene`); `TweenPlayer` takes sampled curves       | parity suite; new rotated-window click test       |
| 3    | Ten native classes and transactions    | one schema file per class with `base`; `gnoblin-proto`, `gnoblin-model`; the nine `core` calls of 5a.4                        | generated bindings; old objects untouched         |
| 4    | Class system, stock library, shell VM  | `data/lua/gnoblin/node.lua`, `shell/init.lua`, `shell/ui.lua`; `tests/shell/` on a fake core; `gnoblin-lua` host calls `tick` | fake-core suites, replay-twice; wm still places   |
| 5    | Placement cutover (section 8)          | delete `placement.rs`, `FloatingEngine`, the D17 floating module; add `shell/floating.lua`                                    | placement goldens, parity placement scenarios     |
| 6    | Stacking and focus cutover             | delete `stack.rs`, `pointer_focus.rs`, focus keys; the stock `Window:on_press` and `raise`                                    | parity focus and stacking scenarios               |
| 7    | States, workspaces, rules, layouts     | delete `gnoblin-wm`, `gnoblin-lua/src/layout`, `data/lua/gnoblin/layouts`; add `shell/tiling.lua`, `shell/strip.lua`          | wm failure modes ported to the stock shell        |
| 8    | Layers and decorations                 | delete `gnoblin-layers`, `modal.rs`, `layers.toml`, `gnoblin_window_frame_v1.rs`; add the `Canvas` painter and `measure`      | portal, alert and menu tests as stock shell nodes |
| 9    | Views, capture, effects                | GLES subtree-to-texture, `View:capture`, PipeWire producer; the `effects` list replaces `effect.toml`                         | capture, secure and effect goldens                |
| 10   | Trust edge                             | delete `capability.rs` grants, `privileged.rs` roles, D22 code, `gnoblin-wasm`, slots                                         | IPC tests on one owner connection                 |
| 11   | Seats                                  | `Seat` nodes, `claim`, device identity (multi-seat phase 1); delete `policy.rs`, gesture action names; gesture chords         | input failure modes                               |
| 12   | XR                                     | stereo and off-axis `Camera`, ray input, linux-dmabuf; `gnoblin-xr` per section 9                                             | translational test on recorded trees              |

What stays Rust is the list in 5a.6. The named easing curves in `gnoblin-animation/src/easing.rs:24-60` (at
`c28c0c671`) become Lua functions in step 2; the Rust side keeps one sampled-curve variant.

### 7.3 Lua API, stub and docs

- **Native classes** stay generated from the schema (`gnoblin-lua/build.rs:1-19`): each schema file gains a `base`
  so the stub writes `---@class Window: Surface`. The stub changes in step 3 and shrinks as old objects go.
  `host_members.toml` folds into the schema.
- **Lua classes** are not in the schema. The registry is their source: `Node.describe(Class)` (in 5a.5) returns the
  name, base, typed properties and signals.
- **The generator** loads `node.lua` and the stock library against a schema-only core, walks the registry and
  appends `---@class Tiling: Workspace` and one `---@field` per property and signal to `gnoblin.d.lua`.
- **Methods and doc text** come from the source: LuaLS reads `function Tiling:arrange()` once the class is declared
  (unverified, section 10), and the generator copies the `---` comment above each `extend` and method.
- **A user's own classes** get completion the same way: `gnoblin-lua stub` runs on the user's config directory and
  writes the stub next to it.
- **Docs-parity** keeps its rule (`xtask/src/docs_parity.rs:1-7`) with a new name grammar: `Class.property`,
  `Class:method`, `Class.signal`. The implemented set is the generated stub, native and Lua halves together, so a
  documented stock class member counts as implemented when the registry has it. Each member documented before it
  is wired gets an `allow.toml` entry naming its step.
- **Pages:** `docs/shell-api/nodes.md` holds section 5 and grows into one page per class, generated. Rewritten by
  hand: `windows.md`, `workspaces.md`, `layer-surfaces.md`, `window-snapping.md`, `focus-activation.md`,
  `overrides.md`, `tweens.md` (renamed `motion.md`), `input-grab.md`. `grants.md` is deleted.

### 7.4 Configuration compatibility

Clean cutover, no shims. Each removed key is refused with its replacement, by the existing hook
(`gnoblin-config/src/hooks.rs:86-116`).

| Removed                                                | Replacement                                                                                  |
| ------------------------------------------------------ | -------------------------------------------------------------------------------------------- |
| `window-management.new-window-placement`               | `function Desk:workspace(name) return Cascade{ name = name } end` (or `Floating`, `Centred`) |
| The six focus and raise keys (`config.toml:1797-1855`) | handlers: `function Window:on_enter() seat.focus = self end`; click-to-raise is stock        |
| `window-management.edge-tiling`                        | `require "gnoblin.shell.snap"`, which replaces the global `drag` with a snapping one         |
| Three keys with no consumer (section 2)                | none: they never had an effect                                                               |
| `layout` section, `workspace:set_layout`               | `function Desk:workspace(name) return Tiling{ name = name } end`                             |
| `gnoblin.window_rule{...}`                             | `function Window:on_ready() if self:is"[app_id=foot]" then ... end end`                      |
| `touchpad-gestures[].action`                           | `seat:bind{ Swipe3 = function(e) ... end }`                                                  |
| `extensions`, `extension_dirs`                         | `require "name"` from the module roots                                                       |

## 8. Proof case: floating with cascade placement

### 8.1 Prerequisites

Steps 1 to 4 must have landed:

- `Workspace` and `Window` nodes with position, size, `committed_width`, `committed_height`, `parent_window`,
  `visible` and `placed`.
- Transactions, and `node.lua` (section 5a) with its fake-core tests.
- The stock shell skeleton of section 6.0: `Desk`, and `Window:on_ready` putting a new window into `desk().active`.

### 8.2 Failure modes, written before the code

1. A window maps before the shell is up. It waits under `root`, undrawn; when the shell starts, it gets `ready` like
   any new node and is placed then.
2. The shell restarts mid-session. Gnoblin drops the nodes the old shell made and returns windows to `root` with
   their geometry. The new shell places them again: placement is not persisted, so `placed` is per runtime and
   `Floating` re-places each window.
3. The transaction misses the frame budget. The window is drawn one frame later, never at the origin first.
4. The work area is full. The window still gets a rectangle inside it, by bounded loops.
5. The window is larger than the work area. It is aligned to the area's origin.
6. The client commits an empty size. The size is floored at 1 by 1.
7. A dialog's parent is closed. The dialog has no `parent_window` and is placed as a normal window. A dialog whose
   parent is on another workspace goes with its parent: it is the parent's child.
8. Two windows map in one batch. The second avoids the first, because reads in a transaction see its own writes.
9. A panel shrinks the workspace. Existing floating windows do not move: `Floating` places in `on_child_added`,
   never in `arrange`.
10. The client resizes after placement. It is not placed again.
11. The same map sequence replayed twice gives the same rectangles.
12. Every coordinate is an integer; no division can make a NaN.

### 8.3 Tests before and after

| Test                                                                                          | Before                                    | After                                                                                            |
| --------------------------------------------------------------------------------------------- | ----------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Eleven unit tests in `placement.rs:123-222`                                                   | pass on Rust `place`                      | moved to `tests/shell/placement.lua` cases                                                       |
| New golden table `tests/shell/placement_cases.toml`: those eleven cases plus 200 seeded cases | generated from Rust `place` and committed | `stock_floating_matches_placement_goldens` on `Floating.place`, `Cascade.place`, `Centred.place` |
| `failure_modes.rs` placement and transient scenarios                                          | pass on `Wm`                              | replayed through the headless backend and stock shell                                            |
| Parity placement scenarios                                                                    | pass against Mutter                       | pass against Mutter with the stock shell                                                         |
| `native_floating_matches_the_shipped_floating_module` (`layout.rs:507`)                       | passes                                    | deleted with its subject                                                                         |
| Failure modes 1 to 12 above                                                                   | written, failing                          | pass; 1, 2, 7 and 8 also run against the fake core in `tests/shell/`                             |

The placement functions ignore `self`, so the golden test calls them as plain functions:
`Cascade.place(nil, w, h, area, occupied)`.

### 8.4 The code change

New file `data/lua/gnoblin/shell/floating.lua`. Placement modes are subclasses, so the mode enum and its string
go:

```lua
-- data/lua/gnoblin/shell/floating.lua: free-space, cascade and centred placement.
-- Replaces crates/gnoblin-wm/src/placement.rs. The pure functions are what the goldens test.
local SCAN_STEP, CASCADE_STEP = 16, 32
local MAX_SCAN_STEPS, MAX_CASCADE_STEPS = 20000, 32
local DEFAULT_W, DEFAULT_H = 800, 600

local function intersects(a, b)
  return a.w > 0 and a.h > 0 and b.w > 0 and b.h > 0
    and a.x < b.x + b.w and b.x < a.x + a.w and a.y < b.y + b.h and b.y < a.y + a.h
end

local function hits(r, occupied)
  for _, o in ipairs(occupied) do
    if intersects(r, o) then return true end
  end
  return false
end

local function clamp_inside(r, area)
  local x = r.w >= area.w and area.x or math.min(math.max(r.x, area.x), area.x + area.w - r.w)
  local y = r.h >= area.h and area.y or math.min(math.max(r.y, area.y), area.y + area.h - r.h)
  return { x = x, y = y, w = r.w, h = r.h }
end

local function cascade(origin, area, occupied)
  local c = clamp_inside(origin, area)
  for step = 1, MAX_CASCADE_STEPS do
    if not hits(c, occupied) then break end
    local d = CASCADE_STEP * step
    c = clamp_inside({ x = origin.x + d, y = origin.y + d, w = origin.w, h = origin.h }, area)
  end
  return c
end

local function free_space(w, h, area, occupied)
  local steps, y = 0, area.y
  while y + h <= area.y + area.h and steps < MAX_SCAN_STEPS do
    local x = area.x
    while x + w <= area.x + area.w and steps < MAX_SCAN_STEPS do
      local c = { x = x, y = y, w = w, h = h }
      if not hits(c, occupied) then return c end
      x, steps = x + SCAN_STEP, steps + 1
    end
    y = y + SCAN_STEP
  end
  return cascade({ x = area.x, y = area.y, w = w, h = h }, area, occupied)
end

local function centred(w, h, area, occupied)
  local c = clamp_inside({ x = area.x + math.max(0, area.w - w) // 2,
                           y = area.y + math.max(0, area.h - h) // 2, w = w, h = h }, area)
  for _, o in ipairs(occupied) do
    if o.x == c.x and o.y == c.y and o.w == c.w and o.h == c.h then return cascade(c, area, occupied) end
  end
  return c
end

-- A placement is a method, so a mode is a subclass: Floating, Cascade, Centred.
Workspace:extend "Floating" {}
function Floating:place(w, h, area, occupied) return free_space(w, h, area, occupied) end

Floating:extend "Cascade" {}
function Cascade:place(w, h, area, occupied)
  return cascade({ x = area.x, y = area.y, w = w, h = h }, area, occupied)
end

Floating:extend "Centred" {}
function Centred:place(w, h, area, occupied) return centred(w, h, area, occupied) end

function Floating:on_child_added(win)
  if not win:is(Window) or win.placed then return end      -- placed once, never again
  local w = math.max(1, win.committed_width > 0 and win.committed_width or DEFAULT_W)
  local h = math.max(1, win.committed_height > 0 and win.committed_height or DEFAULT_H)
  local occupied = {}
  for _, other in ipairs(self:all("> Window")) do
    if other ~= win and other.visible then
      occupied[#occupied + 1] = { x = other.x, y = other.y, w = other.width, h = other.height }
    end
  end
  local r = self:place(w, h, { x = 0, y = 0, w = self.width, h = self.height }, occupied)
  win.x, win.y, win.width, win.height = r.x, r.y, r.w, r.h
end
```

Against pass 1, the pure functions are unchanged except that `center` is spelt `centred`, and the 1 by 1 floor moves
into `on_child_added`.

Gone: `M.attach`, the weak `placed` table, the mode table and its error, and the dialog branch. Dialog centring moves
to the stock `Window:on_ready`, because a dialog is now its parent's child (section 6.0).

Rust and config deletions in the same change:

| Delete                                                                | Where                                                  |
| --------------------------------------------------------------------- | ------------------------------------------------------ |
| `placement.rs` and its tests                                          | `crates/gnoblin-wm/src/placement.rs`                   |
| `PlacementMode` export                                                | `crates/gnoblin-wm/src/lib.rs:118`                     |
| `WmOptions.placement` and its default                                 | `crates/gnoblin-wm/src/wm.rs:111-112`, `:136`          |
| Geometry on map and dialog centring; geometry now comes from the node | `crates/gnoblin-wm/src/wm.rs:1297-1322`                |
| `FloatingEngine` and its parity test                                  | `gnoblin-compositor/src/layout.rs:401-452`, `:503-530` |
| The D17 floating layout module                                        | `data/lua/gnoblin/layouts/floating.lua`                |
| `new-window-placement` key                                            | `gnoblin-config/schema/config.toml:1883-1890`          |

Add to the window-management hook (`hooks.rs:91`): `new-window-placement` refused with "choose a class in
`Desk:workspace`, for example `function Desk:workspace(name) return Cascade{ name = name } end`". Update
`CHANGELOG.md` and `docs/shell-api/windows.md` in the same change.

## 9. What happens to D17, D22, D25, D33 and D34

### D17: layout engines

Retracted. A layout is a Lua class that extends `Workspace` and arranges its windows in `arrange`.

- Kept: 3D-first placement (now the node transform), finite and invertible checks (now setter checks), output in
  input order and no `math.random` (now the shell VM's rules).
- Deleted: `Request`, `Reply`, `Declined`, the message table, `FloatingEngine`, layout references with a hash, layout
  commands, the failure counter, the `layout` config section, and the in-compositor VM with its worker,
  abandon-and-replace and leak cap (under decision 1a).

### D22: general extension system

Retracted. An extension is a Lua file the shell `require`s, declaring classes and handlers like any other script, or
a program on the owner socket.

- Kept: the Lua sandbox limits, the module resolver, guardian supervision of programs.
- Deleted: manifests, powers, capabilities, grants, the never-grantable list, hashes, isolation tiers, the WASM
  tier, dynamic Wayland globals.

### D25: layout kinds

Retracted. Kinds, extents and `managed` or `free` placement are deleted. Its one surviving sentence, "a window has no
floating flag", holds because Gnoblin keeps no window policy state at all; `placed` only records that a shell has
positioned the window.

### D33: layout owns placement

Retracted, because the tree achieves its aim without its parts. One authority places, orders and stacks windows:
the stock shell script. There is no decline. D33 kept layer bands for fullscreen and panels. Here that is shell
policy: the stock shell puts a fullscreen window above its screen's panels by tree order.

### D34: XR extension

Folded. `gnoblin-xr` stays an out-of-process program on the owner socket. It is purely translational:

- It reads the `View` nodes under the node named `room`, in tree order, and submits one OpenXR layer per view. The
  stock shell builds `room` when the seat named `xr` arrives and destroys it when that seat goes (section 6.10).
- A flat `View` becomes a quad layer at its global pose, converted with the head camera's `metres` (metres per unit).
- A `View` under the `stereo` camera named `head` uses the `VIEW` space, so the runtime head-locks it.
- A `View` with a `stereo` camera becomes a projection layer. Gnoblin draws both eyes from eye poses the program
  sends, as D34 already planned.
- It sends controller rays, buttons and the head pose as input on the seat named `xr`, and writes the runtime's
  layer limit to the head camera's read-only `layer_budget`. It writes nothing else.

| D34 item                            | Becomes                                                       |
| ----------------------------------- | ------------------------------------------------------------- |
| G1 hit test through the scene       | core: the pick (section 5)                                    |
| G2 ray pointer                      | core: every pointer is a ray                                  |
| G3 `gnoblin_view_v1`                | deleted: node API plus a capture source per `View`            |
| G4 DMA-BUF                          | kept: core protocol work                                      |
| G5 room                             | a `Node` named `room`; `Camera.metres`                        |
| G6 virtual outputs, `on_remove`     | `View` nodes; removal is shell policy                         |
| G7 extension process and grants     | a program on the owner socket                                 |
| G8 off-axis projection              | `Camera.fov` as four angles                                   |
| G9 frame callbacks off-screen       | core rule: a surface drawn in any `View` gets callbacks       |
| G10 DRM lease                       | kept: core protocol work                                      |
| G11 subtree to a buffer             | core: `View`                                                  |
| G12 viewer outputs, rule action     | deleted: children of the `head` camera                        |
| G13 layout `depth`                  | `Camera.depth_test`                                           |
| Panel rules 1 to 3, budget fallback | deleted: the shell builds views and reads `head.layer_budget` |

## 10. Risks and unverified items

### Risks

| Risk                                                                   | Answer                                                                                                             |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| A transaction per frame over a socket is too slow                      | measure on Agent37 in step 4; decision 1b is the fallback                                                          |
| One frame of latency for interactive moves                             | the 4 ms budget usually lands the same frame; `drag` binds to `seat.x`, so measure it in step 4                    |
| Parity drift when policy moves to Lua                                  | parity scenarios run unchanged against the stock shell (steps 5 to 7)                                              |
| Binding cycles                                                         | 100 passes, then a reported error naming the cycle; the tree keeps the last values                                 |
| A Lua `__index` function on every property read is slow                | measure `arrange` over 200 windows in step 4; a layout reads each window a few times per change                    |
| A binding re-runs more often than needed (it reads a whole child list) | correct but wasteful; `all("> Window")` reads only the direct child list                                           |
| Class handlers add, unlike plain Lua assignment                        | documented on the one page; to change stock behaviour, replace the method the stock handler calls (`vanish`)       |
| Two scripts declare the same class name                                | the second `extend` is an error at its line (decision 3); a shell reload starts a fresh VM                         |
| `to` on a Lua property jumps instead of moving                         | only native properties are drawn; a layout animates the windows it moves, not its own `ratio`                      |
| `tags` are not reactive when mutated in place                          | assign a new table (`n.tags = { dock = true }`) to notify; say so in `nodes.md`                                    |
| A shell that ignores `closed` leaves dead windows on screen            | the stock `Window:on_closed` calls `vanish`; documented as the ownership rule                                      |
| A shell restart loses workspace membership                             | windows return to `root` with their geometry; the new shell re-places them, because placement is not persisted yet |
| Text painting cost in the compositor                                   | `Canvas` is rasterised once per change, not per frame                                                              |
| GPU renderers are not bit-exact                                        | determinism is proven on the software renderer; GLES uses tolerances                                               |
| Large deletion across many crates                                      | twelve steps, each green on its own                                                                                |

### Determinism: what is reproducible and how it is tested

- Reproducible: the tree after every frame, given the recorded inputs, client commits, transactions and frame
  times; and the pixels of the software renderer for that tree and those buffers.
- Not reproducible bit for bit: GLES and Vulkan pixels; wall-clock reads (`date`), which are recorded as inputs.
- The class system adds no order of its own that a run can change. Constructor fields apply in sorted key order;
  bindings re-run in creation order; class handlers run base class first, then in definition order; instance
  handlers in connection order; timers by time then creation; selectors in tree order; chords are claimed in sorted
  order. Every `pairs` loop in `node.lua` is either sorted afterwards or builds a set whose order never reaches the
  tree.
- Test: a replay harness records a headless session, replays it twice, and compares a tree hash per frame and the
  software renderer's frames. It extends the existing replay-twice model tests (`gnoblin-wm/src/lib.rs:85-91`,
  `gnoblin-input/src/lib.rs:78-83`).
- The Lua half is tested without a compositor: `tests/shell/*.lua` load `node.lua`, the stock library and a script
  against a fake core (a plain Lua tree with the ten classes) in the vendored Lua 5.5 VM, drive events and assert on
  the tree, then replay the same session twice and compare dumps.

Evidence for this pass (scratch, not committed):

- I built the vendored Lua 5.5.1 from `lua-src` 551.0.2 with `cc` (no cargo) and parsed every Lua block in this
  file with it.
- On it, I ran section 5a's `node.lua`, the stock library of 6.0, the floating module of 8.4, the ten scripts of
  section 6 and the one-page example against a fake core: five scenario files, 48 assertions, all passing.
- One replay-twice comparison of a 600-line tree dump came out identical.
- The fake core is not Gnoblin. It proves the Lua runs and does what the text says, not that the Rust side exists.

### Unverified

- Round-trip cost of a local socket transaction per frame.
- Whether the egui CPU painter (`gnoblin-portal-ui/Cargo.toml:3`) is fast enough for `Canvas` text.
- OpenXR: quad and projection layers are core, cylinder is an extension, layers draw in submission order, and the
  `VIEW` space head-locks. Taken from D34, which checked them; not re-read here.
- `OutputFrame` and `modal.rs` lines are cited through `layers-status.md` and `layers.md`, not re-read.
- Whether 33 samples of an easing curve, interpolated linearly, look smooth for a spring with overshoot.
- Whether LuaLS merges a generated `---@class Tiling: Workspace` with methods it reads from `layouts/tiling.lua`.
- The gesture chord spelling `Swipe3` is new; the chord grammar (`gnoblin-input/src/chord.rs:254`) does not parse it
  today.
- Which sandbox profile the shell VM runs under. The configuration profile clears `require`
  (`gnoblin-lua-sandbox/src/lib.rs:311`), so the shell needs the existing module loader; the layout profile has no
  `string.rep` (`lib.rs:24-27`), which `Entry` uses.

Lua behaviour the design relies on. Each was exercised by the scratch run above in Lua 5.5.1; none depends on a
dialect:

- `__index` and `__newindex` given as functions fire only for keys absent from the raw table. Instances hold only
  private table keys, so every named read and write goes through them.
- `__call` on a class table (`Panel{}`) and on an instance (`n"#clock"`); call sugar `f"s"` and `f{}`; a method call
  with a string argument (`ws:all"> Window"`); chained calls (`Node:extend "Panel" {}`).
- `__tostring` on a class gives its name, which is how a class works as a selector.
- `coroutine.running()` returns the coroutine and a main-thread flag; a coroutine can yield across `pcall` (5.4 and
  later). `await` refuses to run inside a binding before it would yield.
- Weak-keyed tables (`__mode = "k"`) for readers and coroutine owners; correctness does not depend on collection.
- The sandbox wraps `setmetatable` only to refuse `__gc` in the layout profile
  (`gnoblin-lua-sandbox/src/lib.rs:406-421`); `node.lua` uses no `__gc`. The instruction hook is global, so it bounds
  every handler coroutine (`lib.rs:592-593`).

### Absent from the tree (checked by search)

- GLES subtree-to-texture: the composer composes per output only (`compose.rs:1114`).
- ext-workspace, linux-dmabuf and DRM lease: no module in `gnoblin-wayland/src/protocols`.
- A PAM conversation for `Lock:authenticate`.
- A notification server and a desktop-entry index.
- Any Lua class system, binding engine or selector engine: the only shipped Lua is the eight layouts in
  `data/lua/gnoblin/layouts` (at `c28c0c671`).

### Checked outside the tree

- Quickshell has `PanelWindow`, `PopupWindow`, `ScreencopyView`, `WlSessionLock`, `ToplevelManager` and a PAM context
  ([Quickshell type reference](https://quickshell.org/docs/v0.3.1/types/Quickshell/),
  [Quickshell.Wayland types](https://quickshell.org/docs/v0.3.2/types/Quickshell.Wayland/)). `Variants` creates and
  destroys instances of a component as a data model changes
  ([Variants](https://quickshell.org/docs/v0.3.0/types/Quickshell/Variants)).
- In Godot 4 a `Tween` is a `RefCounted` bound to a node, not a node, and it ends when its node leaves the tree
  ([Godot Tween reference](https://docs.godotengine.org/en/4.2/classes/class_tween.html)). Pass 2 goes one step
  further: a motion is a property write with a duration.
- Godot signals are first-class `Signal` values that can be connected and awaited
  ([Using signals](https://docs.godotengine.org/en/stable/getting_started/step_by_step/signals.html),
  [GDScript `await`](https://docs.godotengine.org/en/stable/tutorials/scripting/gdscript/gdscript_basics.html)).
  A node's `_ready` runs after its children have entered the tree
  ([Node](https://docs.godotengine.org/en/stable/classes/class_node.html)).
- Solid's `createEffect` tracks the reactive values a function reads and re-runs it when they change
  ([createEffect](https://docs.solidjs.com/reference/basic-reactivity/create-effect)).
- From memory, not re-checked: the DOM fixes an event's propagation path before dispatch and `stopPropagation`
  lets the current node's other listeners finish; `Element.closest` tests the element itself first; Textual styles
  widgets with CSS selectors over a widget tree; LuaLS reads `---@class Name: Base` annotations.
