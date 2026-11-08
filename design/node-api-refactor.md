# Node API: one tree for every shell

Status: proposal; nothing here is built. Every claim about current code cites `file:line` at `a1d6d13aa`
(`origin/rust-compositor`). What I could not check is listed in section 10.

The core idea in five sentences:

1. Gnoblin already has the structure it needs: the pure scene tree in `gnoblin-render` holds parents, children,
   paint order, 3D transforms, cameras, effects and a hit test, but it is private and fed one way.
2. Make that tree the public object model: twelve node types, one transform, one order (tree order), one input
   pick, and every client request delivered as a signal on the node it concerns.
3. Gnoblin keeps mechanism only: protocol semantics, drawing every node including decorations, picking, the frame
   clock, tweens, capture and a fail-closed lock, while placement, stacking, focus and animation policy move into a
   shipped Lua shell that users copy and edit.
4. Scripts change the tree in transactions applied at frame boundaries, so the same tree and inputs give the same
   frame, and a layout is a script on a workspace node with no kinds, extents, declines or fallback.
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

| Object or API                     | Where                                                           | Created and ended by | Callers | Axes              |
| --------------------------------- | --------------------------------------------------------------- | -------------------- | ------- | ----------------- |
| Schema objects (15 files)         | `crates/gnoblin-schema/schema/*.toml`                           | K                    | S P     | via members       |
| Generated Lua bindings and stub   | `crates/gnoblin-lua/build.rs:1-19`, `docs/public/gnoblin.d.lua` | build                | S       | n/a               |
| Hand-written host members         | `crates/gnoblin-lua/src/bindings.rs:368-371`                    | build                | S       | n/a               |
| `gnoblin-model::Graph` mirror     | `crates/gnoblin-model/src/lib.rs:1-30`                          | K, runtime           | S P     | n/a               |
| IPC capabilities (11)             | `crates/gnoblin-proto/src/capability.rs:27-92`                  | handshake            | P       | all, by grant     |
| Privileged roles                  | `crates/gnoblin-compositor/src/ipc/privileged.rs:77-116`        | guardian             | P       | all, by grant     |
| Wayland `ClientRole` (6)          | `crates/gnoblin-wayland/src/access.rs:195-216`                  | K on connect         | C       | global visibility |
| Extension manifest, powers, tiers | `design/rust-compositor.md:2744-2909` (D22)                     | host config          | X       | all, by grant     |
| WASM module host                  | `crates/gnoblin-wasm/src/lib.rs:1-11`                           | host config          | X       | all, by grant     |
| Request broker and slots          | `crates/gnoblin-model/src/slot.rs:1-20`                         | services role        | P       | prompts           |

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
| `PlacementMode` and its key                     | `placement.rs:18-59`, `config.toml:1883-1890`             | where new windows open     | yes  | delete  | stock `floating.lua` (section 8)       |
| Dialog centred on parent                        | `wm.rs:1297-1317`                                         | transients                 | no   | delete  | stock script                           |
| Default size 800 by 600                         | `wm.rs:92`                                                | clients with no size       | no   | delete  | stock script                           |
| `WindowKind` normal, dialog, modal              | `window.rs:22-38`                                         | transient handling         | yes  | fold    | read-only `parent_window`, `modal`     |
| `WindowState` flags and precedence              | `window.rs:47-56`, `rust-compositor.md:2197-2210`         | maximise, fullscreen, snap | yes  | delete  | shell sets `states` sent to the client |
| `StackLayer` normal, fullscreen                 | `stack.rs:23-28`                                          | fullscreen above windows   | yes  | delete  | tree order                             |
| `ClaimOrigin` Gnoblin above extension           | `stack.rs:33-36`, `:86-103`                               | trust order on a target    | no   | delete  | tree order                             |
| Six focus and raise options                     | `wm.rs:110-129`, `config.toml:1797-1855`                  | focus policy               | yes  | delete  | stock `focus.lua`                      |
| Edge tiling, `SnapZone`, refusal                | `wm.rs:1994-1996`, `snapping.rs:14`                       | snapping                   | yes  | delete  | stock script                           |
| `DragConstraint`                                | `drag.rs:22-25`                                           | keep drags on screen       | no   | delete  | stock drag helper                      |
| Three window-management keys                    | `config.toml:1816`, `:1868`, `:1893`                      | Mutter parity              | yes  | delete  | no Rust code reads them                |
| Rule engine, `MAX_RULES`, apply once            | `rules.rs:78-180`                                         | declarative rules          | yes  | delete  | stock `rules.lua` in plain Lua         |
| Rule `layer` match                              | `rules.rs:87-90`, `:123-125`                              | one vocabulary             | yes  | delete  | it matches nothing, by design          |
| `MAX_WINDOWS` 256 and record limit              | `wm.rs:106`, `:374`                                       | bound the registry         | no   | delete  | no script needs it                     |
| Workspaces and sets                             | `workspace.rs`, `workspace_set.rs:16-38`                  | named desktops             | yes  | fold    | `Workspace` node; sets are a Lua table |
| Output removal moves windows                    | `wm.rs:1166-1190`                                         | hotplug                    | no   | delete  | `screen_removed` signal; stock script  |
| Workspace slide, close fade, bell flash, 200 ms | `compose.rs:81`, `:398-412`, `:660-682`, `:922`           | built-in animation         | yes  | delete  | tweens in the stock shell              |
| Reduced motion in the composer                  | `compose.rs:913-915`                                      | accessibility              | no   | fold    | every `Tween` is instant when set      |
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
| `EffectsPolicy` inherit, none                   | `scope.rs:93-99`                                       | effects reaching layers         | no   | delete  | effects are children                    |
| `ContentKind` surface, painter                  | `scope.rs:106-112`                                     | compositor-drawn layers         | no   | fold    | `Surface` versus `Canvas`               |
| Provider namespaces, reserved id, name lengths  | `name.rs:25-32`, `registry.rs:66-102`                  | attribution                     | yes  | delete  | nodes have no owners                    |
| 32 layers per provider                          | `registry.rs:47`                                       | a greedy extension              | no   | delete  | trusted shell                           |
| `secure`                                        | `registry.rs:143`, `rules.rs:162-164`                  | keep secrets out of captures    | no   | keep    | one node property, one rule             |
| `capture.secure-treatment` mask, omit           | `rust-compositor.md:3139`                              | style of masking                | no   | delete  | always mask                             |
| `Modal` and the overlay frame variant           | `layers-status.md:27`                                  | portal dialogs                  | no   | delete  | stock shell node with `input`           |
| `gnoblin_window_frame_v1`, `FrameRenderer` role | `gnoblin_window_frame_v1.rs:1-48`, `access.rs:205-209` | decorations by a client         | yes  | delete  | decision 2: `Canvas`                    |
| Decoration mode always client-side              | `zxdg_decoration_v1.rs:6-9`                            | no frame drawer                 | no   | fold    | `Window.decorated` answers the protocol |

### Input

| Rule                              | Where                                            | Added to solve                      | Know | Verdict | Why                                 |
| --------------------------------- | ------------------------------------------------ | ----------------------------------- | ---- | ------- | ----------------------------------- |
| Chord grammar and conflicts       | `gnoblin-input/src/lib.rs:37-44`                 | one meaning per chord               | no   | keep    | mechanism behind `seat:bind`        |
| Gesture recogniser thresholds     | `gesture.rs:337-399`                             | touchpad gestures                   | no   | keep    | mechanism behind `seat:gesture`     |
| Gesture action names (five)       | `config.toml:298-304`, `gesture_router.rs:56-62` | gestures without Lua                | yes  | delete  | handlers are functions              |
| Script policy queue with deadline | `policy.rs:40-127`                               | script veto on raw input            | yes  | delete  | consumption is decided by the pick  |
| Grab scopes                       | `router.rs:48-89`                                | grabs                               | no   | keep    | `seat:grab`                         |
| Shortcut inhibit                  | `router.rs:153`                                  | keyboard-shortcuts-inhibit protocol | no   | keep    | protocol semantics                  |
| Lock input gate                   | `gnoblin-lock/src/lib.rs:20-23`                  | fail-closed lock                    | no   | keep    | `Lock` rule                         |
| EIS default deny, no grant path   | `gnoblin-compositor/src/eis.rs:3-6`              | remote input safety                 | no   | fold    | `remote_requested` signal on `Seat` |
| Rect-only pointer target          | `state.rs:3020-3035`                             | first slice                         | no   | delete  | ray pick on the tree                |

### Trust and extensions

| Rule                                          | Where                                   | Added to solve                    | Know | Verdict | Why                                                 |
| --------------------------------------------- | --------------------------------------- | --------------------------------- | ---- | ------- | --------------------------------------------------- |
| Eleven IPC capabilities                       | `gnoblin-proto/src/capability.rs:27-92` | least privilege per process       | yes  | delete  | owner socket is trusted                             |
| Roles runtime, services, portal, input        | `ipc/privileged.rs:77-98`               | the same                          | yes  | delete  | one owner connection                                |
| Owner grants on the ordinary socket           | `ipc/privileged.rs:108-116`             | the same                          | yes  | delete  | the same                                            |
| `ClientRole` six values                       | `access.rs:195-216`                     | Wayland global visibility         | no   | fold    | two: sandboxed or not; `SessionHost` stays          |
| Privileged globals list                       | `access.rs:114-135`                     | hide globals from sandboxes       | no   | keep    | the one edge                                        |
| D22 packages: manifest, powers, tiers, hashes | `rust-compositor.md:2744-2909`          | third-party code                  | yes  | delete  | extensions receive node references                  |
| WASM tier                                     | `gnoblin-wasm/src/lib.rs:1-11`          | native plugins (D6)               | yes  | delete  | no script needs it                                  |
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

| Rule                                                          | Where                                               | Added to solve              | Know | Verdict | Why                              |
| ------------------------------------------------------------- | --------------------------------------------------- | --------------------------- | ---- | ------- | -------------------------------- |
| Effect kinds and pass limits                                  | `render/src/effect.rs:28-31`, `window.toml:293-301` | blur, shadow, shader        | no   | fold    | `Effect` child node; limits stay |
| Effect skipped on 3D, `active`, `reason`                      | `rust-compositor.md:3038`                           | renderer gaps               | no   | keep    | read-only on `Effect`            |
| Layout per output                                             | `output.toml` `layout`, `set_layout`                | default layout              | yes  | delete  | shell assigns                    |
| XR panel kinds, rules 1 to 3                                  | `xr-extension.md:121-125`                           | flatten or split per output | yes  | delete  | the shell builds `View` nodes    |
| XR layer budget fallback                                      | `xr-extension.md:156-162`                           | runtime layer limits        | no   | delete  | shell reads the budget           |
| XR `attach = viewer`, `windows = false`, rule `output` action | `xr-extension.md:263-275`                           | head-locked surfaces        | yes  | delete  | child of the head `Camera`       |
| XR layout `depth` key                                         | `xr-extension.md:451-457`                           | intersecting windows        | yes  | fold    | `Camera.depth_test`              |
| Virtual output `on_remove`                                    | `xr-extension.md:389-395`                           | headset session end         | yes  | delete  | shell handles `removed`          |
| Session lock states and cover                                 | `gnoblin-lock/src/lib.rs:11-30`                     | fail-closed lock            | no   | keep    | `Lock` node                      |

## 3. The scripts, marked up

These are the scripts a shell developer wants to write, in the API they would reach for. Each marked line says what
it needs today.

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

**The tree.** `gnoblin.root` is the desktop. A node is drawn when it is reachable from a `Screen`'s `source`, or
from `gnoblin.lock` while the lock is engaged. Children draw after their parent, in sibling order.

**Ownership.** Whoever parents a node destroys it. Gnoblin parents popups, cursors and lock surfaces. Windows and
layer surfaces arrive unparented, and the shell places them. `add` reparents a live node and keeps its state.

**Instancing and groups.** A reusable piece is a Lua function that builds and returns a subtree. A group is a tag:
`node.tags.dock = true`, then `gnoblin.root:find("#dock")`.

**Globals.**

| Name                                                                     | Meaning                                                                         |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------- |
| `gnoblin.root`, `gnoblin.lock`                                           | the desktop root; the lock root                                                 |
| `gnoblin.screens()`, `gnoblin.seat`, `gnoblin.seat_named(name)`          | live screens; the default seat; a seat by name                                  |
| `gnoblin.new(type, props)`                                               | a new, unparented node of one of the twelve types                               |
| `gnoblin.on(event, fn)`                                                  | `window_added`, `surface_added`, `screen_added`, `screen_removed`, `seat_added` |
| `gnoblin.spawn(fn)`, `gnoblin.wait(s)`                                   | start a coroutine; wait inside one                                              |
| `gnoblin.now()`, `gnoblin.date(format)`                                  | frame clock in seconds; wall clock, recorded as input                           |
| `gnoblin.process`, `gnoblin.idle`, `gnoblin.settings`, `gnoblin.session` | services, unchanged by this design                                              |

**Every node** has these members. All other types add to them.

| Member                                                                | Kind     | Meaning                                                            |
| --------------------------------------------------------------------- | -------- | ------------------------------------------------------------------ |
| `type`                                                                | property | read-only type name, such as `"Window"`                            |
| `x`, `y`, `z`, `rotation`, `scale`                                    | property | local transform; `rotation` and `scale` are `{x, y, z}`            |
| `matrix`                                                              | property | 4x4 alternative to the five above; must be invertible              |
| `width`, `height`                                                     | property | the node's own rectangle in local units                            |
| `opacity`, `visible`, `clip`                                          | property | group opacity 0 to 1; drawn or not; clip children to the rectangle |
| `input`                                                               | property | `false`, `true`, or `"source"` on a `View` (see Input)             |
| `secure`                                                              | property | never drawn into a captured or texture `View`                      |
| `name`, `tags`, `meta`                                                | property | lookup name; tag set; free table for scripts and extensions        |
| `parent`, `index`                                                     | property | read-only parent; writable sibling position                        |
| `add(child, index)`, `remove()`, `destroy()`                          | method   | reparent; unparent; end                                            |
| `raise()`, `lower()`, `children(type)`, `find(name)`                  | method   | order; list; lookup                                                |
| `tween(props, seconds, ease)`                                         | method   | returns a `Tween` child                                            |
| `on(signal, fn)`, `bind(prop, fn)`                                    | method   | subscribe; reactive binding                                        |
| `child_added`, `child_removed`, `changed`, `destroyed`                | signal   | tree changes; `changed` names the property                         |
| `pointer_enter`, `pointer_leave`, `motion`, `button`, `scroll`, `key` | signal   | input on this node or a descendant                                 |

**The twelve types.**

| Type        | Is                                      | Adds                                                                                       |
| ----------- | --------------------------------------- | ------------------------------------------------------------------------------------------ |
| `Node`      | a group                                 | nothing                                                                                    |
| `Window`    | an app window, Wayland or X11           | identity, size hints, `states`, `decorated`, `close()`, request signals (below)            |
| `Surface`   | other client content                    | `role` (`popup`, `layer`, `lock`, `cursor`, `drag`, `x11`), protocol fields read-only      |
| `Canvas`    | content Gnoblin paints                  | `draw`: a list of rect, text, image and icon items                                         |
| `View`      | a subtree drawn through a camera        | `source`, `camera`, `capture(opts)`; drawn where the node sits                             |
| `Screen`    | a `View` shown on a monitor             | `connector`, `modes`, `mode`, `scale`, `enabled`; Gnoblin creates it on hotplug            |
| `Camera`    | a projection; its transform is its pose | `projection` (`orthographic`, `perspective`, `stereo`), `fov`, `near`, `far`, `depth_test` |
| `Effect`    | one pass on its parent, in child order  | `kind` (`blur`, `shadow`, `shader`, `tint`, `corners`), parameters, `active`, `reason`     |
| `Tween`     | animates its parent                     | `duration`, `ease`, `delay`, `stop()`, `await()`, `finished`                               |
| `Seat`      | one user's input                        | `focus`, `cursor`, `pointer`, `bind()`, `gesture()`, `grab()`, `inject()`                  |
| `Workspace` | a group published to taskbars           | `active`, `activate_requested`                                                             |
| `Lock`      | the session lock root                   | `engaged`, `engage()`, `authenticate()`, `release()`, `lock_requested`                     |

`Window` details:

- Read-only: `app_id`, `title`, `pid`, `parent_window`, `modal`, `min_width`, `min_height`, `max_width`,
  `max_height`, `committed_width`, `committed_height`.
- Written by the shell: `width` and `height` (the configure size), `states` (`maximized`, `fullscreen`,
  `activated`, `tiled`, `suspended`), `decorated`.
- Request signals: `move_requested`, `resize_requested`, `state_requested`, `menu_requested`,
  `activation_requested`, `closed`. Nothing happens unless a script acts.

**Transforms.** A node's global matrix is its parent's times its own. 2D is `z = 0` with rotation about z only.
Coordinates are logical pixels, bounded by 2^20.

**Render targets.** A `View` draws `source` through `camera` into its own pixels. With no camera it shows the
source's rectangle scaled to fit. A `View` whose source is another `View` shows that view's pixels. A thumbnail, a
capture, a headset eye and a monitor are all `View`s.

**Input.** A pointer is a ray; a 2D pointer is the ray through its `Screen`'s camera. Order: lock, then seat grab,
then bindings, then the pick. Keys go to `seat.focus`.

- The pick is the last-drawn node under the ray with `input = true`.
- A hit client surface gets protocol events.
- A hit `View` with `input = "source"` passes the ray into its source.
- Any other hit node gets signals. They bubble to ancestors for observation and cannot be vetoed later.

**Scripts.** One shell VM. A script is a module whose `attach(node, ...)` connects handlers; every connection ends
when its node is destroyed. Events of one frame run their handlers in arrival order and produce one transaction.
Reads inside a transaction see its own writes.

The transaction applies whole at the next frame boundary if it arrives within the 4 ms budget, otherwise at the
frame after. It is never dropped or reordered. `bind` re-evaluates after each transaction, in creation order.
Coroutines: `gnoblin.spawn(fn)`, `gnoblin.wait(seconds)`, `tween:await()`.

**Time.** `gnoblin.now()` is the frame clock. Tweens advance on it inside Gnoblin, so animation never waits on a
script. Each `Screen` keeps its own vsync and emits `presented(sequence, time)` from presentation feedback. Reduced
motion makes every tween instant.

**Lock.** While engaged, every `Screen` draws only the `Lock` subtree and only it takes input. A shell crash keeps
it engaged.

**Trust.** A connection on the session owner's socket gets the whole tree. A sandboxed client gets standard
protocols only. A Lua module the shell `require`s sees only the nodes it is handed.

**Wire, not API.** These protocols carry the nodes and never appear in scripts:

| Node           | Wire                                            |
| -------------- | ----------------------------------------------- |
| `Window`       | xdg-shell, X11, foreign toplevel                |
| `Surface`      | popups, layer shell, session lock, cursor, drag |
| `Screen`       | `wl_output`, output management                  |
| `View:capture` | ext-image-copy-capture, PipeWire                |
| `Workspace`    | ext-workspace                                   |
| `Seat`         | `wl_seat`, libei                                |

## 6. Scripts on the target model

Every script below plugs into this skeleton, the start of the shipped stock shell. Each script begins with
`local shell = require("gnoblin.shell")`.

```lua
-- data/lua/gnoblin/shell/init.lua
local shell = { screens = {} }

local function setup(screen)
  local s = {}
  s.root = gnoblin.new("Node", { name = screen.connector, x = screen.x, y = screen.y,
                                 width = screen.width, height = screen.height, clip = true })
  s.workspaces = gnoblin.new("Node", { width = screen.width, height = screen.height })
  s.panels = gnoblin.new("Node")
  s.overlay = gnoblin.new("Node")
  s.root:add(s.workspaces); s.root:add(s.panels); s.root:add(s.overlay)
  s.ws = {}
  for i = 1, 9 do
    s.ws[i] = gnoblin.new("Workspace", { name = tostring(i), width = screen.width,
                                         height = screen.height, visible = i == 1, active = i == 1 })
    s.workspaces:add(s.ws[i])
  end
  s.active = s.ws[1]
  gnoblin.root:add(s.root)
  shell.screens[screen] = s
end

function shell.current()
  local s = shell.screens[gnoblin.seat.pointer.screen]
  if s then return s end
  local _, first = next(shell.screens)
  return first
end

for _, screen in ipairs(gnoblin.screens()) do setup(screen) end
gnoblin.on("screen_added", setup)
gnoblin.on("window_added", function(w) shell.current().active:add(w) end)
return shell
```

### 6.1 Tiling: master and stack

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

### 6.2 Floating with cascade and free-space placement

The module is the proof case in section 8. Its use:

```lua
local floating = require("gnoblin.shell.floating")
for _, s in pairs(shell.screens) do
  for _, ws in ipairs(s.ws) do floating.attach(ws, { placement = "cascade" }) end
end
require("gnoblin.shell.focus").attach(gnoblin.seat, { mode = "click", raise = true })
```

### 6.3 Scrolling strip with focus walking

```lua
-- layouts/strip.lua
local M = {}
function M.attach(ws, seat, opts)
  opts = opts or {}
  local col, gap = opts.column_width or 0.5, opts.gap or 8
  ws.clip = true
  local strip = gnoblin.new("Node", { name = "strip" })
  ws:add(strip)
  local function arrange()
    local x = 0
    for _, w in ipairs(strip:children("Window")) do
      w.width, w.height = math.floor(ws.width * col), ws.height
      w.x, w.y = x, 0
      x = x + w.width + gap
    end
  end
  local function reveal(w)
    local left = -strip.x
    if w.x < left then
      strip:tween({ x = -w.x }, 0.2, "ease_out")
    elseif w.x + w.width > left + ws.width then
      strip:tween({ x = ws.width - w.x - w.width }, 0.2, "ease_out")
    end
  end
  ws:on("child_added", function(w)
    if w.type ~= "Window" then return end
    local after = seat.focus and seat.focus.parent == strip and seat.focus.index + 1
    strip:add(w, after or nil)
    arrange(); seat.focus = w; reveal(w)
  end)
  strip:on("child_removed", arrange)
  local function walk(step)
    local f = seat.focus
    if not f or f.parent ~= strip then return end
    local nxt = strip:children("Window")[f.index + step]
    if nxt then seat.focus = nxt; reveal(nxt) end
  end
  return { walk = walk }
end
return M

-- in the user's init.lua
local strips = {}
for _, s in pairs(require("gnoblin.shell").screens) do
  for _, ws in ipairs(s.ws) do strips[#strips + 1] = require("layouts.strip").attach(ws, gnoblin.seat) end
end
gnoblin.seat:bind("Super+Right", function() for _, st in ipairs(strips) do st.walk(1) end end)
gnoblin.seat:bind("Super+Left", function() for _, st in ipairs(strips) do st.walk(-1) end end)
```

### 6.4 Custom titlebar with animated close

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

-- in the user's init.lua
gnoblin.on("window_added", function(w) require("gnoblin.shell.decorations").decorate(w) end)
```

### 6.5 Panel, launcher and notification overlay

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

A layer-shell client still works. It arrives as a `Surface` with `role = "layer"`, and the stock `layer_shell.lua`
places it by the wlr rules. A notification D-Bus server and a desktop-entry index are services outside the node
set, and neither exists today.

### 6.6 Overview with live thumbnails

```lua
-- shell/overview.lua
local function open(s, seat)
  local wins = s.active:children("Window")
  local layer = gnoblin.new("Node", { width = s.root.width, height = s.root.height, input = true, opacity = 0 })
  layer:add(gnoblin.new("Effect", { kind = "blur", radius = 30 }))
  local cols = math.max(1, math.ceil(math.sqrt(#wins)))
  local tw, th = s.root.width // (cols + 1), s.root.height // (cols + 1)
  for i, w in ipairs(wins) do
    local thumb = gnoblin.new("View", { source = w, width = tw, height = th, input = true,
      x = ((i - 1) % cols) * (tw + 24) + tw // 2, y = ((i - 1) // cols) * (th + 24) + th // 2 })
    thumb:on("button", function(e)
      if e.pressed then seat.focus = w; w:raise(); layer:destroy() end
    end)
    layer:add(thumb)
  end
  s.overlay:add(layer)
  layer:tween({ opacity = 1 }, 0.15)
end
gnoblin.seat:bind("Super+Tab", function() open(shell.current(), gnoblin.seat) end)
```

### 6.7 Workspace switcher with animated transitions

```lua
-- shell/workspaces.lua
function shell.switch(s, target)
  local from = s.active
  if not target or from == target then return end
  local dir = target.index > from.index and 1 or -1
  local W = s.workspaces.width
  target.x, target.visible, s.active = dir * W, true, target
  gnoblin.spawn(function()
    from:tween({ x = -dir * W }, 0.25, "ease_out_cubic")
    target:tween({ x = 0 }, 0.25, "ease_out_cubic"):await()
    from.visible, from.x, from.active, target.active = false, 0, false, true
  end)
end
function shell.next(s, step) shell.switch(s, s.ws[s.active.index + (step or 1)]) end

for _, s in pairs(shell.screens) do
  for _, ws in ipairs(s.ws) do
    ws:on("activate_requested", function() shell.switch(s, ws) end)
  end
end
for i = 1, 9 do
  gnoblin.seat:bind("Super+" .. i, function() local s = shell.current(); shell.switch(s, s.ws[i]) end)
end
gnoblin.seat:gesture({ kind = "swipe", fingers = 3 }, function(g)
  if g.phase == "end" then shell.next(shell.current(), g.dx < 0 and 1 or -1) end
end)
```

A named workspace set is a Lua table of workspaces to activate.

### 6.8 Screen recorder and remote view

```lua
-- shell/recorder.lua
local rec
gnoblin.seat:bind("Super+Shift+R", function()
  if rec then rec.stream:stop(); rec.proc:kill(); rec = nil; return end
  local screen = gnoblin.seat.pointer.screen
  local stream = screen:capture({ stream = true, cursor = true })
  local proc = gnoblin.process.spawn({ "gst-launch-1.0", "pipewiresrc", "path=" .. stream.node,
                                       "!", "videoconvert", "!", "x264enc", "!", "mp4mux", "!",
                                       "filesink", "location=/tmp/rec.mp4" })
  rec = { stream = stream, proc = proc }
end)

-- shell/remote.lua: a remote viewer sees one screen and may drive the seat
gnoblin.seat:on("remote_requested", function(r)
  shell.toast(r.client .. " wants to control this seat")
  r:accept({ screen = gnoblin.seat.pointer.screen, keyboard = true, pointer = true })
end)
```

The privacy indicator and the lock embargo stay: they are mechanism.

### 6.9 Lock screen

```lua
-- shell/lock.lua
local lock, seat = gnoblin.lock, gnoblin.seat
local function show()
  if lock.engaged then return end
  lock:engage()
  for _, screen in ipairs(gnoblin.screens()) do
    lock:add(gnoblin.new("Canvas", { x = screen.x, y = screen.y, width = screen.width, height = screen.height,
      draw = { { rect = { 0, 0, screen.width, screen.height }, fill = "#11111b" } } }))
  end
  local primary = gnoblin.screens()[1]
  local field = gnoblin.new("Canvas", { x = primary.x + primary.width // 2 - 160, y = primary.y + primary.height // 2,
                                        width = 320, height = 44, input = true })
  lock:add(field)
  local typed = ""
  local function draw(colour)
    field.draw = { { rect = { 0, 0, 320, 44 }, radius = 22, fill = colour or "#313244" },
                   { text = string.rep("*", #typed), x = 20, y = 12, size = 16, color = "#cdd6f4" } }
  end
  draw(); seat.focus = field
  field:on("key", function(e)
    if e.key == "Return" then
      local secret = typed; typed = ""
      gnoblin.spawn(function()
        if lock:authenticate(secret) then
          for _, n in ipairs(lock:children()) do n:destroy() end
          lock:release()
        else
          draw("#f38ba8")
        end
      end)
    elseif e.key == "BackSpace" then typed = typed:sub(1, -2); draw()
    elseif e.text then typed = typed .. e.text; draw() end
  end)
end
seat:bind("Super+L", show)
-- An ext-session-lock client asks to lock: engage, and its own surfaces draw the lock.
lock:on("lock_requested", function() lock:engage() end)
```

An ext-session-lock client still works: its surfaces appear under `Lock` as `Surface` nodes with `role = "lock"`,
sized to their screen.

### 6.10 VR shell

```lua
-- shell/vr.lua: the gnoblin-xr program reads the node named "xr"
local xr = gnoblin.new("Node", { name = "xr", meta = { metres_per_unit = 0.001 } })
local head = gnoblin.new("Camera", { name = "head", projection = "stereo" })
xr:add(head)
gnoblin.root:add(xr)

-- The desktop in the room: each monitor as a panel two metres ahead.
local panels = {}
for i, screen in ipairs(gnoblin.screens()) do
  local p = gnoblin.new("View", { source = screen, width = screen.width, height = screen.height,
                                  input = "source", x = (i - 1) * 2100 - 1000, y = -600, z = -2000 })
  xr:add(p); panels[i] = p
end

-- Windows in 3D: the active workspace's windows on a ring around the viewer.
local function ring(ws)
  local wins = ws:children("Window")
  for i, w in ipairs(wins) do
    local a = (i - 1) * (2 * math.pi / math.max(#wins, 1))
    local p = w.meta.panel or gnoblin.new("View", { source = w, input = "source" })
    w.meta.panel = p
    p.width, p.height = w.width, w.height
    if not p.parent then xr:add(p) end
    p:tween({ x = math.sin(a) * 1800, z = -math.cos(a) * 1800, rotation = { 0, -math.deg(a), 0 } }, 0.3)
  end
end
for _, s in pairs(shell.screens) do
  for _, ws in ipairs(s.ws) do
    ws:on("child_added", function() if ws.active then ring(ws) end end)
    ws:on("child_removed", function(w) if w.meta.panel then w.meta.panel:destroy() end; ring(ws) end)
  end
end

-- A head-locked HUD: a View under the head camera.
local hud_canvas = gnoblin.new("Canvas", { width = 800, height = 80 })
hud_canvas:bind("draw", function()
  return { { rect = { 0, 0, 800, 80 }, radius = 16, fill = "#1e1e2ecc" },
           { text = gnoblin.date("%H:%M") .. "  workspace " .. shell.current().active.name,
             x = 24, y = 24, size = 28, color = "#cdd6f4" } }
end)
head:add(gnoblin.new("View", { source = hud_canvas, width = 800, height = 80, y = 350, z = -1000 }))

-- The controller ray clicks through panels into windows (input = "source").
-- Switching monitors and workspaces from the headset:
local xrseat = gnoblin.seat_named("xr")
xrseat:bind("BTN_EXTRA", function()
  table.insert(panels, 1, table.remove(panels))
  for i, p in ipairs(panels) do p:tween({ x = (i - 1) * 2100 - 1000 }, 0.3) end
end)
xrseat:bind("BTN_SIDE", function() shell.next(shell.current(), 1) end)
```

### 6.11 What each script uses and replaces

| Script            | Nodes used                                                              | Replaces                                                                            |
| ----------------- | ----------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Tiling            | `Workspace`, `Window`, `Tween`, `Seat`                                  | kinds, managed placement, `Reply`, `settle`, layout commands, fixed motion          |
| Floating          | `Workspace`, `Window`, `Seat`                                           | `placement.rs`, dialog rule, `FloatingEngine`, fallback, raise option, `StackLayer` |
| Strip             | `Workspace`, `Node`, `Window`, `Tween`, `Seat`                          | unbounded extent, `ctx.state.view`, `scroll_to`, focus effect                       |
| Titlebar          | `Window`, `Canvas`, `Effect`, `Tween`                                   | frame protocol and role, always-CSD answer, composer fade, `add_effect`             |
| Panel and friends | `Canvas`, `Node`, `Seat`, `Tween`                                       | layer-shell for shell UI, exclusive zones, overlay band, provider limits            |
| Overview          | `Node`, `Effect`, `View`, `Window`, `Seat`, `Tween`                     | shell visuals, `capture-window`, `RegisterLayer`                                    |
| Workspaces        | `Workspace`, `Tween`, `Seat`                                            | `Workspaces`, workspace sets, composer slide, gesture actions                       |
| Recorder, remote  | `Screen`, `Seat`                                                        | `CaptureScreen`, `capture-window`, the closed EIS gate                              |
| Lock              | `Lock`, `Canvas`, `Seat`                                                | the `lock` band and its compositor-only rule                                        |
| VR                | `Node`, `Camera`, `View`, `Screen`, `Window`, `Canvas`, `Seat`, `Tween` | panel planner, `gnoblin_view_v1`, room, viewer outputs, `depth` key, grants         |

## 7. Refactor plan

### 7.1 Two decisions for the owner

**Decision 1: where the shell VM runs.**

- **(a) In the runtime process, as D1 chose (recommended).** The tree lives in the compositor; the shell keeps a
  mirror and sends transactions over the local socket. A shell crash restarts the shell and the desktop stays put.
  The in-compositor layout VM, its worker, abandon-and-replace and leak caps are deleted. Cost: one serialised
  transaction per frame and a mirror's memory (round-trip cost unverified).
- **(b) On a thread inside the compositor.** No serialisation. A fault in the Lua C code ends the whole session,
  and the wedged-worker machinery of D17 stays.

**Decision 2: decorations.**

- **(a) Gnoblin draws them; delete `gnoblin_window_frame_v1` (recommended).** Decorations are `Canvas` and `Effect`
  children of a `Window`. Bingux's `bingux-frame` stops working until its titlebar is rewritten as Lua, and Gnoblin
  must gain text and icon painting.
- **(b) Keep the protocol as a second source of decoration surfaces.** `bingux-frame` keeps working, but two
  decoration paths, the `FrameRenderer` role and the region table stay, and frames still need wiring into the tree.

The steps below assume (a) for both.

### 7.2 Ordered migration

Each step writes its failure-mode tests first, cuts one area over completely, and deletes what it replaces in the
same change. Builds and tests run on Agent37 only.

| Step | What                                   | Crates and files                                                                                       | Green at the end                                  |
| ---- | -------------------------------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------------------------- |
| 1    | Tree contract tests                    | `gnoblin-render/tests/tree.rs`, `gnoblin-model/tests/model.rs`                                         | all existing suites                               |
| 2    | Scene becomes the one authority inside | `compose.rs` (drop `order`, opacity writers), `state.rs:3013` (ray pick on `Scene`)                    | parity suite; new rotated-window click test       |
| 3    | Node schema and transactions           | one schema file per node type; `gnoblin-proto`, `gnoblin-model`                                        | generated bindings; old objects untouched         |
| 4    | Shell VM loop and skeleton             | `gnoblin-lua` runtime host, `data/lua/gnoblin/shell/init.lua`                                          | shell reads only; wm still places                 |
| 5    | Placement cutover (section 8)          | delete `placement.rs`, `FloatingEngine`, the floating module; add `shell/floating.lua`                 | placement goldens, parity placement scenarios     |
| 6    | Stacking and focus cutover             | delete `stack.rs`, `pointer_focus.rs`, focus keys; add `shell/focus.lua`                               | parity focus and stacking scenarios               |
| 7    | States, workspaces, rules, layouts     | delete `gnoblin-wm`, `gnoblin-lua/src/layout`, `data/lua/gnoblin/layouts`; add stock layouts           | wm failure modes ported to the stock shell        |
| 8    | Layers and decorations                 | delete `gnoblin-layers`, `modal.rs`, `layers.toml`, `gnoblin_window_frame_v1.rs`; add `Canvas` painter | portal, alert and menu tests as stock shell nodes |
| 9    | Views, capture, effects                | GLES subtree-to-texture, `View:capture`, PipeWire producer; `Effect` replaces `effect.toml`            | capture, secure and effect goldens                |
| 10   | Trust edge                             | delete `capability.rs` grants, `privileged.rs` roles, D22 code, `gnoblin-wasm`, slots                  | IPC tests on one owner connection                 |
| 11   | Seats                                  | `Seat` nodes, device identity (multi-seat phase 1); delete `policy.rs`, gesture action names           | input failure modes                               |
| 12   | XR                                     | stereo and off-axis `Camera`, ray input, linux-dmabuf; `gnoblin-xr` per section 9                      | translational test on recorded trees              |

### 7.3 Lua API, stub and docs

- The stub stays generated from the schema (`gnoblin-lua/build.rs:1-19`). It changes in step 3 and shrinks in each
  later step as old objects go.
- `host_members.toml` folds into the schema, so one source describes every member.
- Docs-parity keeps its rule (`xtask/src/docs_parity.rs:1-7`). Each node member documented before it is wired gets
  an `allow.toml` entry naming its step.
- Pages rewritten: `docs/shell-api/windows.md`, `workspaces.md`, `layer-surfaces.md`, `window-snapping.md`,
  `focus-activation.md`, `overrides.md`, `tweens.md`, `input-grab.md`. `grants.md` is deleted. A new `nodes.md`
  holds section 5.

### 7.4 Configuration compatibility

Clean cutover, no shims. Each removed key is refused with its replacement, by the existing hook
(`gnoblin-config/src/hooks.rs:86-116`).

| Removed                                                | Replacement                                                         |
| ------------------------------------------------------ | ------------------------------------------------------------------- |
| `window-management.new-window-placement`               | `require("gnoblin.shell.floating").attach(ws, { placement = ... })` |
| The six focus and raise keys (`config.toml:1797-1855`) | `require("gnoblin.shell.focus").attach(seat, { ... })`              |
| `window-management.edge-tiling`                        | `require("gnoblin.shell.snap").attach(seat)`                        |
| Three keys with no consumer (section 2)                | none: they never had an effect                                      |
| `layout` section, `workspace:set_layout`               | `require("layouts.master_stack").attach(ws, opts)`                  |
| `gnoblin.window_rule{...}`                             | `require("gnoblin.shell.rules").add{...}` with the same match keys  |
| `touchpad-gestures[].action`                           | `seat:gesture(spec, fn)`                                            |
| `extensions`, `extension_dirs`                         | `require("name")` from the module roots                             |

## 8. Proof case: floating with cascade placement

### 8.1 Prerequisites

Steps 1 to 4 must have landed:

- `Workspace` and `Window` nodes with position, size, `committed_width`, `committed_height`, `parent_window` and
  `visible`.
- The `child_added` signal and transactions.
- The skeleton that parents new windows into the active workspace.

### 8.2 Failure modes, written before the code

1. A window maps before the shell is up. It stays unparented and undrawn; `attach` places existing children.
2. The shell restarts mid-session. The tree keeps every position; re-attaching moves nothing.
3. The transaction misses the frame budget. The window is drawn one frame later, never at the origin first.
4. The work area is full. The window still gets a rectangle inside it, by bounded loops.
5. The window is larger than the work area. It is aligned to the area's origin.
6. The client commits an empty size. The size is floored at 1 by 1.
7. A dialog's parent is closed or on another workspace. The dialog is placed as a normal window.
8. Two windows map in one batch. The second avoids the first, because reads in a transaction see its own writes.
9. A panel shrinks the workspace. Existing floating windows do not move.
10. The client resizes after placement. It is not placed again.
11. The same map sequence replayed twice gives the same rectangles.
12. Every coordinate is an integer; no division can make a NaN.

### 8.3 Tests before and after

| Test                                                                                          | Before                                    | After                                                   |
| --------------------------------------------------------------------------------------------- | ----------------------------------------- | ------------------------------------------------------- |
| Eleven unit tests in `placement.rs:123-222`                                                   | pass on Rust `place`                      | moved to `tests/shell/placement.lua` cases              |
| New golden table `tests/shell/placement_cases.toml`: those eleven cases plus 200 seeded cases | generated from Rust `place` and committed | `stock_floating_matches_placement_goldens` on `M.place` |
| `failure_modes.rs` placement and transient scenarios                                          | pass on `Wm`                              | replayed through the headless backend and stock shell   |
| Parity placement scenarios                                                                    | pass against Mutter                       | pass against Mutter with the stock shell                |
| `native_floating_matches_the_shipped_floating_module` (`layout.rs:507`)                       | passes                                    | deleted with its subject                                |
| Failure modes 1 to 12 above                                                                   | written, failing                          | pass                                                    |

### 8.4 The code change

New file `data/lua/gnoblin/shell/floating.lua`:

```lua
-- Floating placement for one Workspace node. Replaces crates/gnoblin-wm/src/placement.rs.
local M = {}

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

local function center(w, h, area, occupied)
  local c = clamp_inside({ x = area.x + math.max(0, area.w - w) // 2,
                           y = area.y + math.max(0, area.h - h) // 2, w = w, h = h }, area)
  for _, o in ipairs(occupied) do
    if o.x == c.x and o.y == c.y and o.w == c.w and o.h == c.h then return cascade(c, area, occupied) end
  end
  return c
end

local MODES = {
  free_space = free_space,
  center = center,
  cascade = function(w, h, area, occupied)
    return cascade({ x = area.x, y = area.y, w = w, h = h }, area, occupied)
  end,
}

-- Pure placement, tested against the goldens.
function M.place(mode, w, h, area, occupied)
  return MODES[mode](math.max(1, w), math.max(1, h), area, occupied)
end

local function rect(n) return { x = n.x, y = n.y, w = n.width, h = n.height } end

function M.attach(ws, opts)
  local mode = opts and opts.placement or "free_space"
  if not MODES[mode] then error("floating: unknown placement `" .. tostring(mode) .. "`") end
  local placed = setmetatable({}, { __mode = "k" })

  local function place(win)
    if win.type ~= "Window" or placed[win] then return end
    placed[win] = true
    local w = win.committed_width > 0 and win.committed_width or DEFAULT_W
    local h = win.committed_height > 0 and win.committed_height or DEFAULT_H
    local area = { x = 0, y = 0, w = ws.width, h = ws.height }
    local parent = win.parent_window
    local r
    if parent and parent.parent == ws then
      r = clamp_inside({ x = parent.x + math.max(0, parent.width - w) // 2,
                         y = parent.y + math.max(0, parent.height - h) // 2, w = w, h = h }, area)
      win.index = parent.index + 1
    else
      local occupied = {}
      for _, other in ipairs(ws:children("Window")) do
        if other ~= win and other.visible then occupied[#occupied + 1] = rect(other) end
      end
      r = M.place(mode, w, h, area, occupied)
    end
    win.x, win.y, win.width, win.height = r.x, r.y, r.w, r.h
  end

  for _, win in ipairs(ws:children("Window")) do placed[win] = true end
  ws:on("child_added", place)
  ws:on("child_removed", function(win) placed[win] = nil end)
end

return M
```

Rust and config deletions in the same change:

| Delete                                                                | Where                                                  |
| --------------------------------------------------------------------- | ------------------------------------------------------ |
| `placement.rs` and its tests                                          | `crates/gnoblin-wm/src/placement.rs`                   |
| `PlacementMode` export                                                | `crates/gnoblin-wm/src/lib.rs:118`                     |
| `WmOptions.placement` and its default                                 | `crates/gnoblin-wm/src/wm.rs:111-112`, `:136`          |
| Geometry on map and dialog centring; geometry now comes from the node | `crates/gnoblin-wm/src/wm.rs:1297-1322`                |
| `FloatingEngine` and its parity test                                  | `gnoblin-compositor/src/layout.rs:401-452`, `:503-530` |
| `gnoblin.layouts.floating`                                            | `data/lua/gnoblin/layouts/floating.lua`                |
| `new-window-placement` key                                            | `gnoblin-config/schema/config.toml:1883-1890`          |

Add to the window-management hook (`hooks.rs:91`): `new-window-placement` refused with "use
`require('gnoblin.shell.floating').attach(ws, { placement = ... })`". Update `CHANGELOG.md` and
`docs/shell-api/windows.md` in the same change.

## 9. What happens to D17, D22, D25, D33 and D34

### D17: layout engines

Retracted. A layout is a Lua module attached to a `Workspace` node.

- Kept: 3D-first placement (now the node transform), finite and invertible checks (now setter checks), output in
  input order and no `math.random` (now the shell VM's rules).
- Deleted: `Request`, `Reply`, `Declined`, the message table, `FloatingEngine`, layout references with a hash, layout
  commands, the failure counter, the `layout` config section, and the in-compositor VM with its worker,
  abandon-and-replace and leak cap (under decision 1a).

### D22: general extension system

Retracted. An extension is a Lua module the shell `require`s and hands node references to, or a program on the
owner socket.

- Kept: the Lua sandbox limits, the module resolver, guardian supervision of programs.
- Deleted: manifests, powers, capabilities, grants, the never-grantable list, hashes, isolation tiers, the WASM
  tier, dynamic Wayland globals.

### D25: layout kinds

Retracted. Kinds, extents and `managed` or `free` placement are deleted. Its one surviving sentence, "a window has no
floating flag", holds because Gnoblin keeps no window state at all.

### D33: layout owns placement

Retracted, because the tree achieves its aim without its parts. One authority places, orders and stacks windows:
the stock shell script. There is no decline. D33 kept layer bands for fullscreen and panels. Here that is shell
policy: the stock shell puts a fullscreen window above its screen's panels by tree order.

### D34: XR extension

Folded. `gnoblin-xr` stays an out-of-process program on the owner socket. It is purely translational:

- It reads the `View` nodes under the node named `xr`, in tree order, and submits one OpenXR layer per view.
- A flat `View` becomes a quad layer at its global pose, converted with `xr.meta.metres_per_unit`.
- A `View` under the `stereo` camera named `head` uses the `VIEW` space, so the runtime head-locks it.
- A `View` with a `stereo` camera becomes a projection layer. Gnoblin draws both eyes from eye poses the program
  sends, as D34 already planned.
- It sends controller rays, buttons and the head pose as input on the seat named `xr`. It writes nothing else.

| D34 item                            | Becomes                                                          |
| ----------------------------------- | ---------------------------------------------------------------- |
| G1 hit test through the scene       | core: the pick (section 5)                                       |
| G2 ray pointer                      | core: every pointer is a ray                                     |
| G3 `gnoblin_view_v1`                | deleted: node API plus a capture source per `View`               |
| G4 DMA-BUF                          | kept: core protocol work                                         |
| G5 room                             | a `Node` with `meta.metres_per_unit`                             |
| G6 virtual outputs, `on_remove`     | `View` nodes; removal is shell policy                            |
| G7 extension process and grants     | a program on the owner socket                                    |
| G8 off-axis projection              | `Camera.fov` as four angles                                      |
| G9 frame callbacks off-screen       | core rule: a surface drawn in any `View` gets callbacks          |
| G10 DRM lease                       | kept: core protocol work                                         |
| G11 subtree to a buffer             | core: `View`                                                     |
| G12 viewer outputs, rule action     | deleted: children of the `head` camera                           |
| G13 layout `depth`                  | `Camera.depth_test`                                              |
| Panel rules 1 to 3, budget fallback | deleted: the shell builds views and reads `xr.meta.layer_budget` |

## 10. Risks and unverified items

### Risks

| Risk                                                        | Answer                                                                |
| ----------------------------------------------------------- | --------------------------------------------------------------------- |
| A transaction per frame over a socket is too slow           | measure on Agent37 in step 4; decision 1b is the fallback             |
| One frame of latency for interactive moves                  | the 4 ms budget usually lands the same frame; measure in step 4       |
| Parity drift when policy moves to Lua                       | parity scenarios run unchanged against the stock shell (steps 5 to 7) |
| Reactive binding cycles                                     | one evaluation pass per transaction; a cycle is an error              |
| A shell that ignores `closed` leaves dead windows on screen | stock shell handles it; documented as the ownership rule              |
| Text painting cost in the compositor                        | `Canvas` is rasterised once per change, not per frame                 |
| GPU renderers are not bit-exact                             | determinism is proven on the software renderer; GLES uses tolerances  |
| Large deletion across many crates                           | twelve steps, each green on its own                                   |

### Determinism: what is reproducible and how it is tested

- Reproducible: the tree after every frame, given the recorded inputs, client commits, transactions and frame
  times; and the pixels of the software renderer for that tree and those buffers.
- Not reproducible bit for bit: GLES and Vulkan pixels; wall-clock reads, which are recorded as inputs.
- Test: a replay harness records a headless session, replays it twice, and compares a tree hash per frame and the
  software renderer's frames. It extends the existing replay-twice model tests (`gnoblin-wm/src/lib.rs:85-91`,
  `gnoblin-input/src/lib.rs:78-83`).

### Unverified

- Round-trip cost of a local socket transaction per frame.
- Whether the egui CPU painter (`gnoblin-portal-ui/Cargo.toml:3`) is fast enough for `Canvas`.
- OpenXR: quad and projection layers are core, cylinder is an extension, layers draw in submission order, and the
  `VIEW` space head-locks. Taken from D34, which checked them; not re-read here.
- `OutputFrame` and `modal.rs` lines are cited through `layers-status.md` and `layers.md`, not re-read.

### Absent from the tree (checked by search)

- GLES subtree-to-texture: the composer composes per output only (`compose.rs:1114`).
- ext-workspace, linux-dmabuf and DRM lease: no module in `gnoblin-wayland/src/protocols`.
- A PAM conversation for `Lock:authenticate`.
- A notification server and a desktop-entry index.

### Checked outside the tree

- Quickshell has `PanelWindow`, `PopupWindow`, `ScreencopyView`, `WlSessionLock`, `ToplevelManager` and a PAM context
  ([Quickshell type reference](https://quickshell.org/docs/v0.3.1/types/Quickshell/),
  [Quickshell.Wayland types](https://quickshell.org/docs/v0.3.2/types/Quickshell.Wayland/)).
- In Godot 4 a `Tween` is a `RefCounted` bound to a node, not a node, and it ends when its node leaves the tree
  ([Godot Tween reference](https://docs.godotengine.org/en/4.2/classes/class_tween.html)). This design makes `Tween`
  a child node, which gives the same binding with one less concept.
