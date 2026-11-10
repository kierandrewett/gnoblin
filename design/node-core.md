# Node core: protocol and schema

Status: proposal for D35 step 3 (`node-api-refactor.md` 7.2). Nothing here is built. Code claims cite `file:line` at
`origin/rust-compositor` (`d6a9d2032`). Anything not read is marked (unverified).

Decision 1a (`node-api-refactor.md` 7.1): the shell's Lua runs in the runtime process. The compositor owns the tree.
The shell keeps a mirror and sends transactions over the owner socket. A shell crash restarts the shell; windows stay
put.

Terms: **transaction**: one batch of shell writes, applied whole at a frame boundary. **Change batch**: the
compositor's report of what changed, sent once per changed frame. **Mirror**: the shell's copy of the tree.

## 1. What exists today

| Piece                | Where                                                                             | Role for node core                                                                                              |
| -------------------- | --------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Schema source        | `crates/gnoblin-schema/schema/*.toml` (15 files)                                  | One TOML file per namespace; generates bindings (`crates/gnoblin-schema/src/lib.rs:1-25`)                       |
| Schema parser        | `crates/gnoblin-schema/src/parse.rs:48-102`                                       | `RawObject`, `RawProperty`, `RawMethod`, `RawSignal` (`deny_unknown_fields` on each)                            |
| Duplicate members    | `crates/gnoblin-schema/src/parse.rs:250,283,315`                                  | Refused per object                                                                                              |
| Schema types         | `crates/gnoblin-schema/src/model.rs` (unverified line numbers)                    | `bool bytes double int string uint object:<name> list<T>`; no union, struct or map                              |
| Proto generator      | `crates/gnoblin-schema/src/proto.rs:41,65,77,155`                                 | `proto_files`, `member_path`, `sensitive_table`, `render`                                                       |
| Envelope             | `crates/gnoblin-proto/proto/envelope.proto:18,33-35,64`                           | `namespace`, `member`, `request_id`, `flags`; `oneof` request/reply/event/error; `Error` at 64                  |
| Frames               | `crates/gnoblin-proto/src/frame.rs:22,25,31`                                      | 4-byte prefix; 16 MiB cap, refused above it                                                                     |
| Handshake            | `crates/gnoblin-proto/src/handshake.rs:27,31,69,148`                              | `GNBL` magic; 20-byte frames; version `0.2.0`                                                                   |
| Owner dispatcher     | `crates/gnoblin-compositor/src/ipc/server.rs:1-70,769,1196,1472,1501`             | One thread owns the graph; the module doc lists eight failure modes (30-57)                                     |
| Privileged grants    | `crates/gnoblin-compositor/src/ipc/privileged.rs:102-110`                         | The owner's own connections see and call every member                                                           |
| Scene tree           | `crates/gnoblin-render/src/scene.rs:94,104,409-444,501,540,654,676,691,765,928`   | `NodeKind` (Visual or Space), `NodeSpec`, `insert`, `remove` (returns subtree), `reparent`, setters, `hit_test` |
| Scene bound          | `crates/gnoblin-render/src/scene.rs:279,297,337`                                  | `SceneError` (`NonFinite`, `OutOfBounds`, `DuplicateNode`, `Cycle`); `BOUND = 2^20`                             |
| Scene dirty set      | `crates/gnoblin-render/src/scene.rs:410,413,883,940`                              | `BTreeMap` of nodes; `BTreeSet` of dirty ids; `take_dirty`; `take_damage`                                       |
| Tween player         | `crates/gnoblin-render/src/tween.rs:48,58,223,250,323`                            | `TrackStart`, `TrackSpec`, `TweenPlayer::advance(scene, ms)`, `retarget`                                        |
| Easing curves        | `crates/gnoblin-animation/src/easing.rs:24,79`                                    | Named curves in Rust; move to Lua in step 2                                                                     |
| Frame budget         | `crates/gnoblin-wayland/src/state.rs:588,2162`                                    | `LAYOUT_FRAME_BUDGET = 4 ms`; used by the layout VM                                                             |
| Runtime role         | `crates/gnoblin-lua/src/role.rs:1-12`                                             | The compositor runs no Lua; a script fault restarts the runtime (D1)                                            |
| Node library (draft) | `data/lua/gnoblin/node.lua:1-651`, `:304` (`core.new`)                            | Class system and calls; no transactions yet                                                                     |
| Model values         | `crates/gnoblin-model/src/value.rs:19`, `error.rs:16`                             | `Value` (bool, int, uint, double, text, bytes, object, list); `ModelError`                                      |
| Old window policy    | `crates/gnoblin-wm/src/lib.rs:85`, `placement.rs`, `stack.rs`, `pointer_focus.rs` | Placement, stacking and focus in Rust; removed in steps 5 to 7                                                  |
| Layout VM            | `crates/gnoblin-lua/src/layout/`, `data/lua/gnoblin/layouts/*.lua`                | In-compositor layout engine; removed in step 7                                                                  |

The compositor places windows today (`crates/gnoblin-wm`). Lua supplies configuration and layouts only, through the
runtime role (`role.rs:1-12`).

## 2. The native classes as schema

### 2.1 The format today

A schema file holds one namespace and its objects. Each object has properties, methods and signals; each member has
`since`. From `crates/gnoblin-schema/schema/window.toml:1-14,78-95`:

```toml
namespace = "gnoblin"              # window.toml:3
version = "0.2.0"                  # window.toml:4

[[object]]                         # window.toml:6
name = "window"
since = "0.2.0"

[[object.property]]                # window.toml:11
name = "id"
type = "uint"
since = "0.2.0"

[[object.method]]                  # window.toml:78
name = "focus"
since = "0.2.0"

[[object.signal]]                  # window.toml:89
name = "closed"
since = "0.2.0"
```

Accepted keys (`parse.rs:48-102`, all `deny_unknown_fields`):

| Level    | Accepted today                                            | Missing for node core                               |
| -------- | --------------------------------------------------------- | --------------------------------------------------- |
| object   | `name`, `since`, `secret`, `property`, `method`, `signal` | `base`                                              |
| property | `name`, `type`, `since`, `secret`                         | `default`, `readonly`, `animated`                   |
| method   | `name`, `since`, `secret`, `function`, `params`           | none; async answers use `tree.answer` (section 4.1) |
| signal   | `name`, `since`, `secret`, `field`                        | none                                                |

A list of objects already works: `window.toml:276-279` declares `effects` as `list<object:effect>`, read-only, with
each element an `effect` object (`effect.toml`). Richer shapes are flat objects with a `kind` text (the same pattern
`effect.toml` uses).

### 2.2 The additions

Four keys go into `RawObject` and `RawProperty` (`parse.rs:48-102`):

1. `base = "<class>"` on an object: the class it extends (`Node` for every tree class). The generator emits the
   inherited property list into each class; there is no runtime inheritance.
2. `default = <toml value>` on a property. Every tree property has one, so the mirror never sees a missing value.
   The default must match the type.
3. `readonly = true`: `set` is refused `read-only`, except in the transaction that creates the node: a `new` followed by `set` of a readonly property in the same transaction is accepted (construct-time only). A later `set` of a readonly property is refused.
4. `animated = true`: `set` with `seconds` is allowed; without it, `not-animated`.

Validation follows the existing duplicate checks (`parse.rs:250`). `base` must name a class in the same namespace
(unverified: the namespace check is not read).

### 2.3 Where the files live

The new classes go in `crates/gnoblin-schema/schema/tree/`, one file each, namespace `tree`. Class names are
capitalised (`Node`, `Window`) so they do not collide with the old `window` object. The old objects stay until their
step (section 9). `tree` also holds the transaction messages (section 4) and events (section 5).

### 2.4 The ten classes

Every tree class has `Node`'s properties. A node's children are not a property: the mirror derives them from the
tree. Animatable properties are marked **A**.

**Node** (base). Every node, visual or not.

| Property          | Type                  | Default   | Flags    | Notes                                                                |
| ----------------- | --------------------- | --------- | -------- | -------------------------------------------------------------------- |
| `id`              | `uint`                | none      | readonly | the node's id (section 3.1)                                          |
| `parent`          | `object:Node`         | nil       | readonly | nil only for `root`; changed by `insert`                             |
| `name`            | `string`              | `""`      |          | the `#name` selector key                                             |
| `x`, `y`, `z`     | `double`              | `0.0`     | A        | local translation                                                    |
| `rotation`        | `list<double>`        | `[0,0,0]` | A        | three components                                                     |
| `scale`           | `list<double>`        | `[1,1,1]` | A        | three components                                                     |
| `width`, `height` | `double`              | `0.0`     | A        | local rectangle                                                      |
| `opacity`         | `double`              | `1.0`     | A        | clamped to `[0,1]` (`scene.rs:383` `clamp_opacity`, unverified line) |
| `visible`         | `bool`                | `true`    |          | a hidden node and its subtree are neither drawn nor hit              |
| `clip`            | `bool`                | `false`   |          | children clipped to the rectangle                                    |
| `input`           | `bool`                | `true`    |          | the node is skipped by the pick; its children are still tested       |
| `secure`          | `bool`                | `false`   |          | never captured by a `View`                                           |
| `effects`         | `list<object:effect>` | `[]`      |          | paint order, first to last                                           |
| `tags`            | `list<string>`        | `[]`      |          | the `.tag` selector key                                              |

Signals: `destroyed()`. Input signals (`press`, `release`, `motion`, `scroll`, `enter`, `leave`, `key`) are section 5.

**Surface** (base `Node`). A client surface that is not a toplevel: popup, layer shell, session lock, cursor, drag icon.

| Property | Type     | Default | Flags                                                 |
| -------- | -------- | ------- | ----------------------------------------------------- |
| `role`   | `string` | none    | readonly (`popup`, `layer`, `lock`, `cursor`, `drag`) |
| `mapped` | `bool`   | `false` | readonly                                              |
| `app_id` | `string` | `""`    | readonly                                              |

**Window** (base `Node`). A toplevel: xdg-shell, X11, foreign toplevel.

| Property     | Type               | Default | Flags    | Notes                                                |
| ------------ | ------------------ | ------- | -------- | ---------------------------------------------------- |
| `title`      | `string`           | `""`    |          | set by the client; not trusted for attribution       |
| `app_id`     | `string`           | `""`    | readonly | set by the client                                    |
| `focused`    | `bool`             | `false` | readonly | written by the shell's focus policy                  |
| `fullscreen` | `bool`             | `false` |          | a client request; the shell may refuse (section 5.2) |
| `maximised`  | `bool`             | `false` |          |                                                      |
| `minimised`  | `bool`             | `false` |          |                                                      |
| `parent`     | `object:Window`    | nil     | readonly | the dialog's parent (unverified: plan 8.2)           |
| `workspace`  | `object:Workspace` | nil     |          | the workspace the window belongs to                  |

Methods: `focus()`, `close()`. `present` (`window.toml:84-88` today) moves to the shell in step 6.

**Canvas** (base `Node`). A visual that paints a paint list built by the shell (text, rectangle, image). The paint list
is rasterised when it changes, not every frame (unverified: plan risk table, section 10).

| Property  | Type    | Default | Flags | Notes                                                         |
| --------- | ------- | ------- | ----- | ------------------------------------------------------------- |
| `content` | `bytes` | `[]`    |       | encoded paint list; refused `bad-paint` if it does not decode |

Method: `measure(text, size)`. It runs on the compositor and answers through `tree.answer` like any call. No font
measurement API exists in the tree today (unverified).

**View** (base `Node`). Draws another node's subtree as a texture. Replaces `capture.toml`.

| Property | Type          | Default  | Flags | Notes                                                |
| -------- | ------------- | -------- | ----- | ---------------------------------------------------- |
| `source` | `object:Node` | nil      |       | the captured subtree                                 |
| `input`  | `string`      | `"none"` |       | `none`, or `source` (the ray passes into the source) |
| `fps`    | `uint`        | `0`      |       | `0` is on demand                                     |

Method: `capture()` returns an operation id (`operation.toml`).

**Screen** (base `Node`). An output. Replaces `output.toml`. Created and destroyed by the compositor.

| Property          | Type     | Default    | Flags               |
| ----------------- | -------- | ---------- | ------------------- |
| `name`            | `string` | none       | readonly            |
| `make`, `model`   | `string` | `""`       | readonly            |
| `x`, `y`          | `double` | `0.0`      | readonly            |
| `width`, `height` | `double` | none       | readonly            |
| `scale`           | `double` | `1.0`      |                     |
| `transform`       | `string` | `"normal"` |                     |
| `primary`         | `bool`   | `false`    |                     |
| `enabled`         | `bool`   | `true`     |                     |
| `mode`            | `string` | none       | readonly (`WxH@Hz`) |

Method: `apply(...)` takes an output configuration (`output.toml`, unverified line). Signal: `mode_changed()`.

**Camera** (base `Node`). A space: children are projected through it (`scene.rs:94-98`, `NodeKind::Space`).

| Property      | Type     | Default          | Flags |
| ------------- | -------- | ---------------- | ----- |
| `projection`  | `string` | `"orthographic"` |       | `orthographic`, `perspective`, `stereo` (stereo in step 12) |
| `metres`      | `double` | `0.001`          |       |
| `near`, `far` | `double` | `0.1`, `1000.0`  |       |
| `fov`         | `double` | `60.0`           | A     |

**Seat** (base `Node`). An input seat (`wl_seat`, libei). Draws nothing.

| Property                 | Type          | Default | Flags    | Notes                                |
| ------------------------ | ------------- | ------- | -------- | ------------------------------------ |
| `name`                   | `string`      | none    | readonly |                                      |
| `focus`                  | `object:Node` | nil     |          | keyboard focus (plan 5, input order) |
| `pointer_x`, `pointer_y` | `double`      | `0.0`   | readonly |                                      |
| `device`                 | `string`      | `""`    | readonly | device identity for multi-seat       |

Method: `warp(x, y)`. Chords are a `call` to the seat (section 5.1).

**Workspace** (base `Node`). A named set of windows on one screen. Replaces `workspace.toml`. Created by the compositor.

| Property | Type            | Default | Flags | Notes                          |
| -------- | --------------- | ------- | ----- | ------------------------------ |
| `index`  | `uint`          | none    |       | position in the workspace list |
| `active` | `bool`          | `false` |       | one active per screen          |
| `output` | `object:Screen` | nil     |       |                                |

Method: `activate()`.

**Lock** (base `Node`). The session lock. While `engaged`, the compositor draws only `Lock` and routes input only to it.

| Property  | Type     | Default        | Flags    | Notes                                         |
| --------- | -------- | -------------- | -------- | --------------------------------------------- |
| `state`   | `string` | `"not-locked"` | readonly | `not-locked`, `pending`, `locked`             |
| `engaged` | `bool`   | `false`        | readonly | true from the moment the lock client takes it |

Methods: `engage()`, `release()`. Both are privileged and the owner grant decides (`privileged.rs:102-110`).

### 2.5 Old objects and the new classes

| Old object (`schema/*.toml`)                                    | New class                        | Kept or replaced                         | Step                |
| --------------------------------------------------------------- | -------------------------------- | ---------------------------------------- | ------------------- |
| `window`                                                        | `Window`                         | replaced; `focus` moves, `present` moves | 3 schema, 6 methods |
| `output`                                                        | `Screen`                         | replaced                                 | 3 schema, 5 apply   |
| `workspace`                                                     | `Workspace`                      | replaced                                 | 7                   |
| `layers`                                                        | `Surface` (role layer)           | replaced                                 | 8                   |
| `lock`                                                          | `Lock`                           | replaced                                 | 8                   |
| `portal_ui`                                                     | `Canvas`, `Surface`              | replaced                                 | 8                   |
| `capture`                                                       | `View`                           | replaced                                 | 9                   |
| `effect`                                                        | value type behind `Node.effects` | kept as an element type                  | 9                   |
| `eis`                                                           | `Seat`                           | replaced                                 | 11                  |
| `session`, `idle`, `settings`, `config`, `request`, `operation` | none                             | kept: not tree classes                   | -                   |

## 3. The tree contract

### 3.1 Ids

| Range                   | Allocated by                             | Lifetime                               |
| ----------------------- | ---------------------------------------- | -------------------------------------- |
| bit 63 set (`>= 2^63`)  | the compositor, from a counter           | until the node is destroyed            |
| bit 63 clear (`< 2^63`) | the shell, from its per-connection range | until destroyed or the connection ends |

- The shell's id is `(epoch << 32) | counter`. The epoch is granted by `tree.attach` and changes on each attach. The
  counter never repeats within an epoch.
- An id is never reused in one compositor run. A destroyed id stays unknown.
- `core.new(class)` returns an id before the transaction applies (`node.lua:304`), so the shell chooses the id. The
  epoch stops an old shell's id from naming a node the new shell made.

### 3.2 Ownership

Ruling (owner of the design, overrides the table below where they disagree): windows and layer surfaces arrive
unparented and the shell places them. The shell may insert or move any node that is unparented, or that the shell
itself parented, and may destroy nodes it created. It may not move a node the compositor parented (popup, cursor, lock
surface), nor destroy a compositor-owned node; both are refused `not-owner`. Moving or destroying the root is refused
`root-immutable`. Ownership governs lifecycle and parentage only. Property writes are governed only by the schema's
`readonly` flag, whoever owns the node: the shell may write `x`, `y`, `width`, `height`, `visible`, opacity and the
other placement properties of a compositor-owned window, and may not write a read-only property (`title`, `app_id`,
...), which is refused `read-only`.

| Node                                                       | Created by         | Destroyed by                                        | Shell may `insert` it? |
| ---------------------------------------------------------- | ------------------ | --------------------------------------------------- | ---------------------- |
| `root`                                                     | compositor         | never                                               | no                     |
| `Screen`, `Seat`, `Lock`, `Surface`, `Window`, `Workspace` | compositor         | compositor, when the protocol object or output ends | yes                    |
| any other class                                            | shell (`core.new`) | the shell (`destroy`), or an ancestor's destroy     | yes                    |

The shell destroys only the nodes it created. Destroying a compositor node is refused `not-owner`. Destroying a node
ends its subtree; the change batch names every id removed (`scene.rs:501` `remove` returns the subtree).

### 3.3 Reparent

`insert(parent, node, index)` moves a live node in one op:

1. `parent` and `node` exist (`unknown-node`); `node` is not `root` (`root-immutable`).
2. `parent` is not inside `node`'s subtree (`cycle`, `scene.rs:550`).
3. The node leaves its old parent and joins `parent` at `index`, clamped to the child count (no error).
4. A `Window` keeps its id, geometry and `placed` mark through a move. The batch carries one `parented` entry, not a
   destroy and a create.

### 3.4 Destroy, and what a destroyed id returns

- A change batch with `destroyed` removes the id and its subtree from the mirror. `core.get(id, key)` on a removed id
  returns `nil`.
- A transaction op on a removed id refuses the whole transaction `unknown-node` (section 4.3).
- A stored node reference (`parent`, `focus`, `source`, `workspace`) becomes `nil` when its target goes. The holder's
  change is in the same batch.

### 3.5 Bounds

- Coordinates and rectangle edges: `|v| <= 2^20` (`scene.rs:337`). Beyond it: `out-of-bounds` (refused, not clamped).
- Non-finite values: `not-finite` (`scene.rs:279`, `NonFinite`).
- Node count: 65536 (new; no cap exists today). Tree depth: 256 (new).

## 4. The transaction

### 4.1 Messages

Members of namespace `tree`, carried in `Envelope` frames (`envelope.proto:18`):

| Member          | Frame kind | Direction           | Payload                                                                                        |
| --------------- | ---------- | ------------------- | ---------------------------------------------------------------------------------------------- |
| `tree.attach`   | Request    | shell to compositor | `version`; reply: `epoch`, `root`, `serial`                                                    |
| `tree.snapshot` | Event      | compositor to shell | `serial`, `chunk`, `last bool`, `nodes list<object:Node>` (section 4.6)                        |
| `tree.transact` | Request    | shell to compositor | `base_serial uint`, `serial uint`, `ops list<object:op>`                                       |
| `tree.reply`    | Reply      | compositor to shell | `serial`, `applied bool`, `failed_op int` (-1 if applied), `code string`, `removed list<uint>` |
| `tree.changes`  | Event      | compositor to shell | `serial uint`, `changes list<object:change>`                                                   |
| `tree.event`    | Event      | compositor to shell | `frame uint`, `seq uint`, `node uint`, `kind string`, `event object:event`                     |
| `tree.call`     | Request    | shell to compositor | `node uint`, `method string`, `args bytes`, `token uint`                                       |
| `tree.answer`   | Event      | compositor to shell | `token uint`, `result bytes`, or `error string`                                                |

`op` is one object with `kind` and flat optional fields, the `effect.toml` pattern:

| Field                                                                 | Used by                    | Type                                                                                                 |
| --------------------------------------------------------------------- | -------------------------- | ---------------------------------------------------------------------------------------------------- |
| `kind`                                                                | all                        | `string`: `set`, `insert`, `destroy`, `call`, `new`                                                  |
| `node`                                                                | set, insert, destroy, call | `uint`                                                                                               |
| `class`                                                               | new                        | `string`                                                                                             |
| `id`                                                                  | new                        | `uint`, the shell's pre-allocated id                                                                 |
| `key`                                                                 | set                        | `string`                                                                                             |
| `value_kind`                                                          | set                        | `string`: `bool`, `int`, `uint`, `double`, `text`, `node`, `list_double`, `list_text`, `list_effect` |
| `value_bool`, `value_int`, `value_uint`, `value_double`, `value_text` | set                        | matching type; only the one named by `value_kind` is read                                            |
| `value_node`                                                          | set                        | `uint`; `0` is nil                                                                                   |
| `value_list`                                                          | set                        | `list<double>`                                                                                       |
| `value_texts`                                                         | set                        | `list<string>`, read when `value_kind` is `list_text`                                                |
| `value_effects`                                                       | set                        | `list<effect>`, read when `value_kind` is `list_effect`; blur and shadow only, at most 16 per node   |
| `seconds`                                                             | set                        | `double`; absent means an instant set                                                                |
| `samples`                                                             | set                        | `list<double>` (section 6.1)                                                                         |
| `parent`, `index`                                                     | insert                     | `uint`, `int`                                                                                        |
| `method`, `args`                                                      | call                       | `string`, `bytes`                                                                                    |

The typed value fields are the cost of having no union in the schema format. The alternative, a `bytes` value the shell
encodes itself, moves the type check out of the schema and into code the schema cannot check.

`base_serial` is the last change batch the shell applied. `serial` is the shell's transaction counter, monotone per
connection.

### 4.2 Applying at the frame boundary

Each frame, the compositor:

1. Drains the transactions queued by the socket, in arrival order. `TreeFrame::drain` (`crates/gnoblin-wayland/src/tree_frame.rs`) runs first, before `Composer::begin_frame`, against the scene the composer draws.
2. For each transaction in order: validates every op against the tree plus the earlier ops of the same transaction
   (so an op sees an earlier `new`). If every op validates, applies them all; otherwise applies nothing.
3. If any transaction applied, publishes one change batch at the next serial (`TreeFrame::take_batches`, `State::take_tree_batches`).
4. Runs the normal frame: `take_dirty` (`scene.rs:883`), compose, `take_damage` (`scene.rs:940`).

A transaction that arrives after step 1 waits for the next frame. The frame never waits for the shell. A late
transaction draws one frame later, never at its origin first (plan 8.2 item 3, unverified line).

The 4 ms budget (`TREE_FRAME_BUDGET`, `tree_frame.rs`) bounds tree work. A transaction that starts after the budget is spent waits, whole, at the front of the queue for the next frame, so order is kept; the first transaction of a frame always runs, so one longer than the budget still lands. Each transaction is capped at 4096 ops; validation is linear in the ops. The per-op cost is unmeasured until step 4.

### 4.3 Atomicity: whole transaction

A transaction applies whole or not at all. A refusal names the first failing op (`failed_op`).

- The mirror stays honest only if it has every write or none. A partial apply leaves a binding that reads its own
  writes seeing a tree no script wrote (plan 8.2 item 8, unverified line).
- Per-op errors make every script handle partial state. A script runs as one unit per event, and a whole refusal fits.
- The cost: one bad op drops the frame's other writes. A shell that wants independent writes sends independent
  transactions.

### 4.4 Refusals

Each refusal is one `Error.code` (`envelope.proto:64`, kebab case):

| Code                     | Cause                                              | Source of the rule             |
| ------------------------ | -------------------------------------------------- | ------------------------------ |
| `unknown-node`           | id not in the tree, or from an older epoch         | section 3                      |
| `not-owner`              | shell destroys or moves a compositor node          | section 3.2                    |
| `root-immutable`         | move or destroy of `root`                          | section 3.2                    |
| `cycle`                  | reparent into its own subtree                      | `scene.rs:550` `Cycle`         |
| `unknown-class`          | `new` names no class                               | section 2                      |
| `unknown-member`         | `set` or `call` names no property or method        | section 2                      |
| `wrong-type`             | value kind differs from the declared type          | `ModelError` (`error.rs:16`)   |
| `read-only`              | `set` on a readonly property                       | section 2.2                    |
| `not-animated`           | `seconds` on a property without `animated`         | section 2.2                    |
| `not-finite`             | NaN or infinite                                    | `scene.rs:279` `NonFinite`     |
| `out-of-bounds`          | coordinate beyond `2^20`                           | `scene.rs:297` `OutOfBounds`   |
| `duplicate-node`         | `new` with a live id                               | `scene.rs:446` `DuplicateNode` |
| `bad-curve`              | empty, unsorted or out-of-range `samples`          | section 6.1                    |
| `out-of-range`           | `seconds` negative or above 3600                   | section 6.1                    |
| `too-large`              | more than 4096 ops, or a node over 1 MiB           | section 4.2, 4.6               |
| `too-many-nodes`         | `new` past 65536                                   | section 3.5                    |
| `too-deep`               | `insert` past depth 256                            | section 3.5                    |
| `stale-base`             | conflicting writes after `base_serial`             | section 4.5                    |
| `invalid-serial`         | `base_serial` above the compositor's serial        | section 4.5                    |
| `not-privileged`         | privileged member from a peer without the grant    | `ipc/server.rs:52-57` (mode 7) |
| `shell-already-attached` | a second `tree.attach` while one is live           | section 8, F2                  |
| `bad-paint`              | `Canvas.content` does not decode                   | section 2.4                    |
| `bad-value`              | `value_kind` does not match the value field's type | section 4.1                    |

### 4.4a Value kinds and caps

A property's value kind comes from its schema type. The mapping is `value_type` in `ipc/tree.rs`; the checks are in `transaction.rs`.

| Schema type           | Value kind   | Checks                                                                                                                                               |
| --------------------- | ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `bool`                | `Bool`       | kind matches (`wrong-type`)                                                                                                                          |
| `int`                 | `Int`        | kind matches (`wrong-type`)                                                                                                                          |
| `uint`                | `Uint`       | kind matches (`wrong-type`)                                                                                                                          |
| `double`              | `Double`     | kind matches (`wrong-type`); finite (`not-finite`)                                                                                                   |
| `string`              | `Text`       | kind matches (`wrong-type`); at most 64 KiB (`too-large`)                                                                                            |
| `bytes`               | `Bytes`      | kind matches (`wrong-type`); at most 1 MiB (`too-large`)                                                                                             |
| `list<double>`        | `ListDouble` | kind matches (`wrong-type`); at most 4096 items (`too-large`); each finite (`not-finite`)                                                            |
| `list<string>`        | `ListText`   | kind matches (`wrong-type`); at most 4096 items (`too-large`); each at most 64 KiB (`too-large`)                                                     |
| `list<object:effect>` | `Effects`    | at most 16 passes (`too-large`); blur and shadow only, a shader pass is `bad-value`; every number finite (`not-finite`) or in range (`out-of-range`) |
| `object:<C>`          | `Node(C)`    | names a live node whose class is `C` or a subclass, or nil (`unknown-node`, `wrong-type`)                                                            |

`TreeClasses::new` checks every property of every `Node` subclass in the catalog and refuses to build when a schema type has no value kind (for example `list<uint>`). That is a load-time error, never a silent gap at set time. A read-only property refuses every write (`read-only`).

### 4.5 Stale base

The shell builds a transaction from its mirror at `base_serial`. The compositor may have sent newer batches since.

- `base_serial == current`: apply.
- `base_serial < current`, and no op names a node that a batch published in `(base_serial, current]` changed, or that an earlier transaction in the same frame changed: apply. Anything the shell did not write keeps the compositor's current value.
- `base_serial < current`, and an op names such a node: refuse `stale-base`, with `failed_op` at the first conflicting op. The shell applies the newer batches and decides again on its next tick.
- `base_serial > current`: refuse `invalid-serial` and close the connection.
- A `base_serial` older than the last 64 batches: refuse `stale-base` (the mirror resyncs).

Decision: the conflict check is per node. Two transactions conflict only when they write the same node, whatever the property. Nodes a transaction creates never conflict. A busy frame does not force every write to retry, because only writes to the same node retry.

### 4.6 The mirror

1. **Attach.** `tree.attach` returns the epoch, the root id and the serial `S`.
2. **Snapshot.** The compositor sends `tree.snapshot` chunks of at most 1 MiB at serial `S`; the last has `last = true`.
   Each snapshot node carries `props`: the node's property values that differ from their schema defaults, as `change` records with kind `set` (the same shape as a `set` entry in a batch). A property at its default is left out. The mirror seeds each node's values from `props`, so a shell attaching to a tree that already has values sees them without waiting for a batch.
   The frame cap is 16 MiB (`frame.rs:22`), so a chunk fits with room to spare. A node over 1 MiB is refused at creation
   (`too-large`), so a whole snapshot is bounded by 65536 nodes times that cap.
3. **Changes.** Each changed frame sends one `tree.changes` at `S+1`, `S+2`, and so on. The shell applies batches in
   serial order.
4. **Gap.** A batch whose serial is not the next one means a missed batch. The shell drops its mirror and attaches
   again on the same connection; the compositor answers with a fresh snapshot (F23).
5. **Restart.** The shell process exits; its connection closes. The compositor moves each window the old shell placed
   to `root`, keeping its geometry, and marks it `placed` so nothing re-places it. Nodes the old shell created are
   destroyed. The new shell attaches and gets a snapshot. Windows stay where they were drawn.

Change batch entries (`tree.changes`):

| `change.kind` | Fields                             | Meaning                                 |
| ------------- | ---------------------------------- | --------------------------------------- |
| `created`     | `node`, `class`, `parent`, `index` | a node exists, with its property values |
| `parented`    | `node`, `parent`, `index`          | a move                                  |
| `set`         | `node`, `key`, the value fields    | a property's new target value           |
| `destroyed`   | `node`, `removed list<uint>`       | a subtree is gone                       |

Animated values are not in batches (section 6.2): the mirror holds targets.

## 5. Events

### 5.1 Input

The pick runs in the compositor, in this order (plan 5, input order, unverified line): lock, then grab, then claimed
chords, then the pick. The pick uses `Scene::hit_test` (`scene.rs:928`), so no shell round-trip sits in it.

| Target after the pick                  | What the compositor sends                                                    |
| -------------------------------------- | ---------------------------------------------------------------------------- |
| a client surface (`Window`, `Surface`) | protocol events to the client, as today                                      |
| a `View` with `input = "source"`       | the ray passed into its source, then the same rule                           |
| any other node with `input` set        | `tree.event` with `kind`, `node`, node-local coordinates, buttons, modifiers |
| nothing                                | nothing                                                                      |

Implemented: pointer edges over a shell node (`enter`, `motion`, `leave`, `press`, `release`, `scroll`, with
node-local `x`, `y`, `button`, and `dx`, `dy` for scroll). The pick is the topmost shell node under the pointer; it is
taken only while the session is unlocked and the shell holds the tree, and a window topmost at the point takes the
pointer instead. The pick does not check the node's `input` flag yet, so every shell node is an input target. Not done:
key events. The design routes keys to `seat.focus` (`node-api-refactor.md`), so they are not sent to the shell. F30 is
not enforced: an event for a node destroyed in the same frame is still sent after its `destroyed` change.

`seq` is monotone per run. A chord claimed with `seat:bind{...}` (a `call` to the seat) is stored in the compositor's
claim table, so a claimed chord never reaches a client. The claim is removed on `unbind` or when the seat is destroyed.

### 5.2 Protocol requests

A client request that needs policy (fullscreen, maximise, move, resize, activation) becomes a `tree.event` with
`kind = "request"`. The shell answers with a transaction that sets the property. If the shell is silent, F25 applies.

### 5.3 Order within one frame

1. Client commits and protocol requests, in arrival order.
2. Input events, in `seq` order.
3. Shell transactions, in arrival order. A transaction sees the steps above; it never sees a later one.
4. The change batch for the frame, sent before any `tree.event` of the next frame.

The compositor sends no event for a node that is not yet in a batch (F29).

### 5.4 Recording

Each event records its `frame`, `seq`, `kind`, `node` and payload. Each transaction records the frame it applied in, its
serial, its base serial, its ops and its reply. Client commits record the frame they were read in. A `sys.date` result
is recorded as input (section 7).

## 6. Animation

### 6.1 `set` with a duration and samples

`core.set(id, key, value, seconds, samples)` (plan 5a.4, unverified line) becomes the transaction op `set` with
`seconds` and `samples`:

- `samples` are values in `[0, 1]`, evenly spaced, from `0` to `1`. The shell samples its Lua curve at 32 points;
  `ease.linear` samples to `[0, 1]`. The compositor interpolates linearly between samples.
- The compositor builds a `TrackSpec` (`tween.rs:58`) with a sampled-curve variant. Named curves in Rust
  (`easing.rs:24`) stay until step 2 moves them to Lua.
- The track starts from `TrackStart::Current` (`tween.rs:48`): the compositor's value at the frame, not the mirror's
  target.
- A `set` on a node and key with a track in flight replaces the track from the current value (`tween.rs:323`
  `retarget`).
- A `set` without `seconds` is an instant set and cancels any track on that key.
- Reduced motion: the shell turns every `set` with `seconds` into an instant set (plan 6.0, unverified line).

### 6.2 Ownership

- The compositor owns animation state; the shell owns targets and curves.
- The mirror holds the target. A read returns the end value at once.
- The compositor advances tracks each frame on its own clock (`TweenPlayer::advance(scene, elapsed_ms)`, `tween.rs:250`).
  Intermediate values never reach the shell. A layout that reacts to an intermediate value is out of this design.

### 6.3 The frame clock: `tick(now, changes, events)`

The shell's `node.lua` returns `tick(now, changes, events)` (plan 5a.4, unverified line). The host calls it:

- once per frame that carries input or a change batch, and
- at the time `tick` returned last (the shell's next timer).

Handlers' writes are collected and sent as one transaction when `tick` returns. `now` is the compositor's monotonic
clock in milliseconds, recorded as input. Timers fire by `now`. The shell reads the wall clock only through `sys.date`,
which is recorded.

## 7. Determinism and replay

### 7.1 What is recorded

One log per run, in frame order:

| Record        | Fields                                                    |
| ------------- | --------------------------------------------------------- |
| header        | compositor version; schema version `0.2.0`; renderer name |
| frame         | `frame`, `now`, tree hash after the frame                 |
| client commit | `frame`, client id, commit bytes                          |
| input event   | `frame`, `seq`, the event                                 |
| transaction   | `frame` applied, `serial`, `base_serial`, ops, reply      |
| snapshot      | `serial`, chunk hashes                                    |
| `sys` result  | `frame`, the call, the result                             |

### 7.2 Tree hash

The hash covers a canonical encoding of the tree after each frame: nodes in id order (`BTreeMap`, `scene.rs:410`),
properties in name order, children in stored order. A 64-bit FNV-1a is enough for a per-frame check.

### 7.3 Replay harness

`crates/gnoblin-render/tests/replay.rs` (new) reads a log, drives a headless compositor on the software renderer, feeds
recorded client commits, input, transactions and `sys` results at their frames, and compares the tree hash after each
frame. It runs the log twice and requires both runs to match each other and the recording. This extends the replay
tests already in `crates/gnoblin-wm/src/lib.rs:85-91` and `crates/gnoblin-input/src/lib.rs:78-83`.

The shell's Lua is tested against a fake core (slice 9). The harness does not run Lua; it replays the transactions.

### 7.4 Not reproducible

GLES/Vulkan pixels (software renderer only), wall-clock reads other than `sys.date`, and client actions outside recorded commits.

## 8. Failure modes

Each row is written before the code. A slice's tests name the rows they check.

| #   | Condition                                           | Behaviour                                                                                                                   | Slice |
| --- | --------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- | ----- |
| F1  | No shell connected at start                         | Windows map under `root` and are not drawn. The session waits. No fallback layout. The runtime guardian restarts it (D1).   | 5     |
| F2  | Second `tree.attach` while a shell is live          | Refused `shell-already-attached`. The first shell keeps the tree.                                                           | 5     |
| F3  | Shell crashes mid-write                             | The reader gets `CodecError::Truncated` (`frame.rs:48-54`). Nothing applies.                                                | 5     |
| F4  | Shell crashes after `tick`, before the send         | The writes are lost. The next shell attaches and gets a snapshot. The tree keeps its last state.                            | 5     |
| F5  | Stale serial, conflicting writes                    | Refused `stale-base`. The shell catches up and decides on its next tick.                                                    | 3     |
| F6  | Stale serial, no conflict                           | Applied. Untouched values keep the compositor's current value.                                                              | 3     |
| F7  | `base_serial` above the compositor's serial         | Refused `invalid-serial`. The connection closes.                                                                            | 5     |
| F8  | Op on a destroyed id                                | Whole transaction refused `unknown-node`; `failed_op` set.                                                                  | 3     |
| F9  | Op on an id from an older epoch                     | Refused `unknown-node`. The old id names nothing in this epoch.                                                             | 3     |
| F10 | Reparent into its own subtree                       | Refused `cycle`. No change.                                                                                                 | 3     |
| F11 | Move or destroy of `root`                           | Refused `root-immutable`.                                                                                                   | 3     |
| F12 | Shell destroys or moves a compositor node           | Refused `not-owner`.                                                                                                        | 3     |
| F13 | Coordinate beyond `2^20`                            | Refused `out-of-bounds`. Not clamped (`scene.rs:337`).                                                                      | 3     |
| F14 | NaN or infinite value                               | Refused `not-finite`.                                                                                                       | 3     |
| F15 | `new` with a live id                                | Refused `duplicate-node`.                                                                                                   | 3     |
| F16 | Transaction over 4096 ops                           | Refused `too-large` before any apply.                                                                                       | 4     |
| F17 | Snapshot needs more than one chunk                  | Chunks at most 1 MiB; the last is flagged. A node over 1 MiB is refused at creation.                                        | 5     |
| F18 | More than 65536 nodes                               | Refused `too-many-nodes`.                                                                                                   | 3     |
| F19 | Shell answers after the frame started               | The frame draws without it. The transaction applies at the next frame start. No frame waits.                                | 4     |
| F20 | Shell never answers a `call`                        | After 5 s the token is dropped and the shell receives `tree.answer` with `error = "call-timeout"`.                          | 5     |
| F21 | Shell dies while the lock is engaged                | The lock stays engaged and drawn. A new shell gets the lock state in its snapshot.                                          | 8     |
| F22 | Lock client unlocks while the shell is gone         | The unlock goes through the session-lock protocol. The shell is not consulted.                                              | 8     |
| F23 | Mirror sees a serial gap                            | Drops its mirror and attaches again on the same connection; the compositor sends a fresh snapshot.                          | 4     |
| F24 | Old or duplicate serial delivered                   | Ignored if `<=` the applied serial.                                                                                         | 4     |
| F25 | Client request needs policy; shell silent for 5 s   | The compositor answers the client with the window's current state (a configure with no change).                             | 5     |
| F26 | Wrong value type in `set`                           | Refused `wrong-type`.                                                                                                       | 3     |
| F27 | Unknown class, property or method                   | Refused `unknown-class` or `unknown-member`.                                                                                | 3     |
| F28 | Owner-only member from a non-owner peer             | Refused `not-owner` on attach (`ipc/tree.rs`). A transaction from a non-holder stays `not-privileged` (`ipc/tree_host.rs`). | 5     |
| F29 | Input for a node not yet in a batch                 | Not sent; the pick skips nodes not yet in a batch.                                                                          | 6     |
| F30 | Input for a node destroyed in the same frame        | Dropped at send; the `destroyed` change precedes any later event.                                                           | 6     |
| F31 | `seconds` on a property without `animated`          | Refused `not-animated`.                                                                                                     | 7     |
| F32 | `samples` empty, unsorted or outside `[0,1]`        | Refused `bad-curve`.                                                                                                        | 7     |
| F33 | `seconds` negative or above 3600                    | Refused `out-of-range`.                                                                                                     | 7     |
| F34 | Replay log diverges from the recording              | The harness fails at the first frame whose hash differs.                                                                    | 9     |
| F35 | Clock goes backwards                                | The compositor's monotonic clock is the only source; the shell never sees a backwards `now`.                                | 6     |
| F36 | Seat focus names a destroyed node                   | Focus becomes nil; the `focus` change is in the same batch.                                                                 | 3     |
| F37 | Workspace removed while it has windows              | Windows keep their geometry, move to `root` with `workspace` nil, and the shell places them. Windows are not destroyed.     | 3     |
| F38 | Tree deeper than 256                                | Refused `too-deep` at `insert`.                                                                                             | 3     |
| F39 | Two transactions in one frame, the second conflicts | The second sees the first's writes; a conflict is refused `stale-base` as in F5.                                            | 4     |
| F40 | Binding cycle in the shell                          | 100 passes, then an error names the cycle. The tree keeps the last values.                                                  | 10    |
| F41 | `set` on a readonly property                        | Refused `read-only`.                                                                                                        | 3     |
| F42 | `Canvas.content` does not decode                    | Refused `bad-paint`. The canvas keeps its last content.                                                                     | 3     |
| F43 | Screen unplugged                                    | Its `Screen` node is destroyed. Windows on it keep their rectangle; the compositor does not move them.                      | 5     |
| F44 | Timer fires after its node is destroyed             | The timer stops when the node that started it is destroyed.                                                                 | 10    |

## 9. Migration: old objects onto the new schema

Each step writes its failure-mode tests first, cuts one area over completely, and deletes what it replaces (plan 7.2).

| Old object or code                                                                                   | New class or home                                  | Step | Green at the end                    |
| ---------------------------------------------------------------------------------------------------- | -------------------------------------------------- | ---- | ----------------------------------- |
| `schema/window.toml`, window methods in `ipc`                                                        | `Window`, `Node` (`schema/tree/`)                  | 3, 6 | parity focus and stacking           |
| `schema/output.toml`, output apply                                                                   | `Screen`                                           | 3, 5 | placement goldens                   |
| `crates/gnoblin-wm/src/placement.rs`, floating engine                                                | `shell/floating.lua`                               | 5    | placement goldens                   |
| `crates/gnoblin-wm/src/stack.rs`, `pointer_focus.rs`, focus keys                                     | stock `Window:on_press`, `raise`                   | 6    | parity focus and stacking           |
| `schema/workspace.toml`, `crates/gnoblin-lua/src/layout/`, `data/lua/gnoblin/layouts/`               | `Workspace`; `shell/tiling.lua`, `shell/strip.lua` | 7    | wm failure modes on the stock shell |
| `crates/gnoblin-layers/`, `schema/layers.toml`                                                       | `Surface` (role layer), `Canvas`                   | 8    | portal, alert and menu tests        |
| `schema/lock.toml`                                                                                   | `Lock`                                             | 8    | lock scenarios                      |
| `schema/portal_ui.toml`                                                                              | `Canvas`, `Surface`                                | 8    | portal goldens                      |
| `schema/capture.toml`, GLES capture                                                                  | `View`                                             | 9    | capture goldens                     |
| `schema/effect.toml`                                                                                 | element type of `Node.effects`                     | 9    | effect goldens                      |
| `schema/eis.toml`, gesture action names                                                              | `Seat`                                             | 11   | input failure modes                 |
| `schema/session.toml`, `idle.toml`, `settings.toml`, `config.toml`, `request.toml`, `operation.toml` | kept                                               | -    | unchanged                           |

Plugin and config compatibility is in section 7.4 of the plan (unverified). An old object is deleted in the step that replaces it.

## 10. Implementation slices

Each slice is for one agent. Slices 1 to 3 run in parallel; builds run on Agent37 only (plan 7.2).

### Slice 1: schema format additions

- **Crate and files:** `gnoblin-schema`: `src/parse.rs` (`RawObject`, `RawProperty`), `src/model.rs` (`base`, `default`,
  `readonly`, `animated`), new cases in `tests/`.
- **Tests first:** `base` parses; a default of the wrong type is an error; `base` naming an unknown class is an error;
  `readonly` and `animated` parse as booleans; an unknown key is still refused.
- **Stays green:** every existing schema file parses unchanged.

### Slice 2: tree class schema and generated bindings

- **Crate and files:** `gnoblin-schema`, `gnoblin-proto`: `schema/tree/*.toml` (the ten classes and the `tree` messages),
  `crates/gnoblin-proto/build.rs` regeneration.
- **Tests first:** generated `Node` carries the section 2.4 properties with defaults; `Window` inherits them; `op` has
  every section 4.1 field; the sensitive table has no `tree` member.
- **Stays green:** the old objects and their generated code.

### Slice 3: scene tree contract

- **Crate and files:** `gnoblin-render`, `gnoblin-types`: `src/scene.rs` (owner per node, id ranges, `parented`, the
  node and depth caps, subtree removal), `src/scene_ids.rs`, extends `tests/tree.rs`.
- **Tests first:** F5, F6, F8 to F15, F18, F26, F27, F36 to F38, F41 as scene-level tests; reparent keeps id and
  geometry; destroy returns the subtree.
- **Stays green:** existing scene and hit-test tests.

### Slice 4: transaction validation and apply

- **Crate and files:** `gnoblin-render`: new `src/transaction.rs`, `src/lib.rs`.
- **Tests first:** whole-transaction refusal with `failed_op` (F8, F10, F13 to F16, F26, F39); an op sees an earlier
  `new`; a failed validation changes nothing (tree hash before and after equal); F24.
- **Stays green:** slice 3's tests.

### Slice 5: owner socket, attach, snapshot, batches

- **Crate and files:** `gnoblin-compositor`: new `src/ipc/tree.rs`; dispatch in `src/ipc/server.rs`; `src/ipc/mod.rs`.
- **Tests first:** F1 to F4, F7, F17, F20, F25, F28, F43 over a socket with a fake shell client; attach returns the
  epoch; a snapshot chunks at 1 MiB.
- **Stays green:** the owner dispatcher tests (`ipc/server.rs:1-70`, modes 1 to 8).
- **Status:** implemented in `gnoblin-compositor`: `ipc/tree.rs` (`TreeSession`: attach, transact, drain, detach, the holder check), `ipc/tree_host.rs` (the `tree` namespace, `TreeHostHandle`) and `ipc/server.rs` (dispatch; one `Mutex<TreeSession>` on the server). The scene behind `TreeHostHandle` is a test implementation until slice 6's frame boundary supplies the real one. Calls and animations from an applied transaction are not dispatched yet.

### Slice 6: frame boundary wiring

- **Status:** implemented in `gnoblin-wayland`: `src/tree_frame.rs` (`TreeFrame`, `TreeQueue`, `TreeBatch`) and `state.rs` (`Host::begin_frame` drains before composing; `State::tree_queue`, `set_tree_classes`, `take_tree_batches`). `Composer::with_scene` gives the drain the composer's scene. The owner socket submits through `TreeQueue` (slice 5 wiring); `ipc/` is untouched by this slice.
- **Dependency direction:** `gnoblin-wayland` already depends on `gnoblin-backend` and `gnoblin-render`. `TreeHost` is not a trait in `ipc/tree.rs`; the host lives in `gnoblin-wayland`, so no new edge is needed.
- **Fixed:** the composer allocates window node ids from `NodeIdAllocator` (`gnoblin-render/src/scene_ids.rs`). It starts at `COMPOSITOR_ID_BASE`, never reuses an id in a session and refuses exhaustion, so a window node is never in the shell's range and a tree node cannot collide with it. Tests: `every_node_the_composer_creates_has_a_compositor_id`, `a_shell_node_on_the_composers_first_id_does_not_block_a_window` and `a_window_node_id_is_never_reused_within_a_session` (`gnoblin-backend`); `a_shell_new_at_the_compositor_base_is_refused_not_owner` and `shell_ids_below_the_base_coexist_with_every_compositor_window` (`gnoblin-render`).
- **Stale base:** implemented. `gnoblin-render/src/tree_revision.rs` (`TreeRevisions`) keeps the nodes each of the last 64 batches changed. `TreeSession::transact` and the frame drain check each transaction at the moment it applies, against the batches published since its base and the changes already applied in the same frame. A deferred transaction is checked when it applies, not when it arrived.
- **Window nodes:** implemented. The composer creates a `window` class node (unparented, `NodeIdAllocator` id) on each map, with read-only `title` and `app_id` carried on the snapshot node and in `set` changes; `created` and `destroyed` changes go out in the next batch. While a shell holds the tree the composer does not write window x/y (`Scene::shell_places`), and the window manager places windows again on detach. Tests: `a_window_gets_a_window_node_with_its_title_and_app_id`, `a_shell_holding_the_tree_keeps_the_position_it_set` and `the_window_manager_places_the_window_again_when_the_shell_lets_go` (`gnoblin-backend`); `a_mapped_window_is_a_window_node_with_its_title_and_app_id`, `a_window_mapped_after_attach_is_published_as_created_with_its_class`, `a_client_title_change_is_published_as_a_set_of_the_window_node`, `a_transaction_setting_a_window_geometry_is_applied_and_published_at_the_next_serial`, `a_refused_transaction_leaves_the_window_node_untouched` and `an_unmapped_window_is_published_as_destroyed` (`tests/tree_ipc.rs`, real clients). Not covered by a real-client test: the drawn geometry of a shell-written x/width (composer tests only).
- **Tests first:** F19 (a late transaction draws one frame later); one batch per changed frame; F29, F30, F35.
- **Stays green:** frame-timing tests on the layout path (`state.rs:588`, `2162`).

### Slice 7: animated `set`

- **Crate and files:** `gnoblin-render`: `src/tween.rs` (sampled-curve variant of `TrackSpec`); `gnoblin-animation`:
  `src/easing.rs` (named curves stay until step 2).
- **Tests first:** F31 to F33; sampled curves interpolate linearly; `retarget` starts from the current value.
- **Stays green:** easing tests and the existing tween tests.

### Slice 8: input events and the lock

- **Crate and files:** `gnoblin-render` (`src/scene.rs` pick skips nodes not yet in a batch); `gnoblin-wayland`
  (`src/state.rs` event routing).
- **Tests first:** F21, F22; a claimed chord never reaches a client; the lock draws and takes input alone.
- **Stays green:** input tests and the parity input scenarios.

### Slice 9: replay harness and fake core

- **Crate and files:** `gnoblin-render`: new `tests/replay.rs`. `tests/shell/*.lua` (new): loads `node.lua` against a fake
  core of the ten classes.
- **Tests first:** F34; the fake core enforces the mirror rules (F23, F24); a script session replays twice to the same
  dump.
- **Stays green:** the model replay tests (`gnoblin-wm/src/lib.rs:85-91`).

### Slice 10: Lua core calls and the class system

- **Crate and files:** `gnoblin-lua`: `data/lua/gnoblin/node.lua` (the nine `core` calls and the class system), host
  wiring in `src/host.rs` (calls `tick`).
- **Tests first:** F40, F44; a class declared twice is an error at its line; a subclass's override is called with
  `Base.name(self)`.
- **Stays green:** the runtime role tests (`role.rs`) and config tests.
- **Status:** partial. `src/tree_core.rs` installs the `core` table over the shell's tree (`new`, `get`, `set`, `insert`, `destroy`, `call`, `root`, `classes`, `report`) and the class schema (`ClassSpec`, `PropSpec`, `TreeValueKind`); `tests/tree_core.rs` covers it. Not done: `node.lua` (varargs `core.call`, the return token), the class system (F40, F44 not verified) and the socket `TreeLink`.

### Slice 11: determinism gate

- **Crate and files:** `gnoblin-render`: extend `tests/replay.rs`; a script under `scripts/` records one run.
- **Tests first:** a 600-frame recorded run replays twice to the same hashes; the software renderer's pixels match.
- **Stays green:** slices 3 to 9.

## 11. Open questions

1. **Class names beside old objects.** Lean: keep capitalised class names; delete each old object in its own step.
2. **Caps.** 65536 nodes and depth 256 are guesses. Lean: fix now; revise on measurement in step 4.
3. **Stale base.** Resolved: keep the conflict check, per node (section 4.5).
