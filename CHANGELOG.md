# Changelog

## Unreleased

### Added

- The session lock is a compositor-owned `lock` node in the tree (`class lock`, prop `state`: `not-locked`, `pending`, `locked`). It is created at startup and follows the session-lock state, so a shell that attaches while the lock is engaged reads `locked` in its snapshot, and an unlock with no shell attached returns the node to `not-locked`. Tests `the_lock_node_is_a_compositor_node_that_publishes_only_a_state_change` and `the_lock_node_returns_to_not_locked_when_the_session_unlocks_with_no_shell` in `gnoblin-backend`, `a_shell_that_attaches_while_the_lock_is_engaged_reads_the_lock_state` in `tests/tree_live.rs`, and `unlock_returns_the_pointer_to_the_covered_client` in `tests/session_lock.rs` (gnoblin-wayland).
- Lua `seat:claim(chord, grab)` returns a per-VM integer claim id (1, 2, ... never reused) and sends a `claim` call carrying the chord as the op `text` and the grab flag; `seat:unclaim(id)` sends `unclaim`. `Seat:bind{[chord]=fn}` registers each fn under its claim id, `Seat:unbind` unclaims, and a `chord` seat event `(id, chord, state)` runs the fn once on `press`. Tests `seat claim returns increasing ids and queues a claim call carrying the chord` and `a chord event runs the bound fn once on press and unbind stops later events` in `crates/gnoblin-lua/tests/lua/shell_test.lua`.
- `gnoblin-xr` drives the owner's pointer from the OpenXR ray. `Pointer` (`extensions/gnoblin-xr/crates/gnoblin-xr/src/pointer.rs`) casts the ray with `xr_scene::pick` and sends owner `input` ops: `motion` on change, `leave` on a miss or a new window. `FrameLoop::locate_ray` exposes the runtime's ray without changing tick pacing. Buttons and scroll are not wired: the runtime has no button source. Test `pointer_sends_motion_then_leave_over_the_window_under_the_ray` in `bridge.rs`.
- Screen nodes: the composer creates a `screen` node per declared output, publishes its geometry, and destroys it with the output.
- Tests for seat `focus` writes from the shell: window accepted, `nil` clears, non-window and unknown targets refused `bad_value`, non-node value refused `wrong_type`.
- A shell write of the main seat node's `focus` moves the keyboard to that window, through the same path as a compositor focus change; a `nil` write clears it. The compositor's own focus changes are published and never read back. Test `a_shell_write_of_the_seat_focus_moves_the_keyboard_to_that_window`.
- A shell node that takes input (`class` other than `window`, `input` set) holds the keyboard when the shell writes the main seat's `focus` to it. The client gets a `wl_keyboard` leave, and each key the router does not consume is sent to the shell as `tree.event` `kind = "key"` with `pressed` and `keysym`. A window focus takes the hold back. Under the session lock no key reaches the shell (the route is gated on `Unlocked`; no end-to-end test yet). Tests `key_edges_reach_the_shell_in_order_and_do_not_advance_the_serial` in `tests/tree_frame.rs`, `a_seat_focus_on_a_shell_node_with_input_on_is_accepted_and_keeps_the_node` and `a_shell_node_that_holds_the_seat_focus_is_the_key_target_until_it_is_released` in `tests/tree_live.rs` (gnoblin-wayland).

### Fixed

- Nodes the attach snapshot installs get `ready` once each, parents first, on the first VM frame after the shell library loads, so `Screen:on_ready` runs.

- The compositor's own changes (the main seat and screen nodes created before an attach) are published as their own batch before the attach snapshot is read. The snapshot carries them, and an attaching shell no longer receives them a second time as `changes` events after the snapshot. Fixes the `tree_ipc` failures introduced by the seat and screen node commits.
- Seat and node `claim` and `unclaim` calls from the shell are captured in op order, not refused as unknown methods. `has_method` accepts `claim` and `unclaim` on `seat` and `node`; test `seat_claim_calls_are_captured_in_op_order_and_others_are_not` in `tests/tree_frame.rs` (gnoblin-wayland).
- A claimed chord reports both of its edges to the shell that claimed it: a `chord` press when the key goes down and a `chord` release when it comes up, each with the claim id. The client sees neither edge while the claim holds. A press the lock held back goes to the client, and so does its release. A destroyed seat releases its claims, so their chords reach the client again. Tests `a_claimed_chord_reaches_the_shell_and_never_the_client`, `an_unclaimed_chord_reaches_the_client_again`, `a_locked_session_never_fires_a_claimed_chord` and `a_destroyed_seat_takes_its_claims_with_it` in `tests/protocols.rs` (gnoblin-wayland).

### Changed

- Attaching the tree shell from a connection without the owner grant is refused `not-owner` (`failed_op` -1), not `not-privileged`. Test `attach_refuses_a_non_owner_peer` replaces `attach_refuses_an_unprivileged_peer`. A transaction from a connection that does not hold the tree is still refused `not-privileged`.

- The `gnoblin-xr` pointer's seat is named `xr`, not `gnoblin-xr-pointer`. The bridge reuses a seat named `xr` already in the owner snapshot and creates one only when none exists. Tests `pointer_creates_the_xr_seat_when_the_snapshot_has_none` and `pointer_reuses_an_existing_xr_seat` in `bridge.rs`.

- Frame callbacks reach a window that only a visible `View` shows, not only one on a monitor. A View shows the subtree under its `source` when the View and its ancestors are visible with non-zero opacity. A mapped window on neither a monitor nor such a View gets no frame callback; before, output-less mapped windows got them. Unit tests `a_visible_view_shows_its_source_subtree_and_a_hidden_one_shows_nothing` and the other `view` tests in `gnoblin-render` cover the visibility rules and a View inside its own source. Compositor-level callback tests are not added: there is no harness for the shell tree.

### Added

- The owner-only global `gnoblin_view_source_manager_v1` (protocol `protocols/gnoblin-view-source-v1.xml`) is offered to the owner's privileged connection and to nothing else: an application, a sandbox and a trusted service do not see it. Its `create_source(node)` returns an `ext_image_capture_source_v1` for a View node, the source `ext-image-copy-capture` reads a View's picture from. A node that is not a View of the scene is a protocol error `invalid_node`. The frame loop publishes the set of View nodes into the capture tables each frame, so a View created and named in the same loop iteration is refused until the next. Tests in `crates/gnoblin-wayland/tests/view_source.rs` (`the_owner_sees_the_view_source_global`, `an_ordinary_application_does_not_see_the_view_source_global`, `a_trusted_service_does_not_see_the_view_source_global`, `a_source_for_a_node_that_is_not_a_view_is_a_protocol_error`).

- A capture of a View copies the View's picture at the View's size into the client's buffer, on the same `ext-image-copy-capture` path as an output. The picture is redelivered when the View's rect changes size, and damage on one View does not reach another capture target. Tests `a_view_frame_follows_the_view_rect_after_a_resize` and `view_damage_does_not_reach_other_targets` in `gnoblin-capture`. Colour reaches the client buffer as the same pixel in `a_view_picture_pixel_reaches_the_client_as_the_same_colour` (`gnoblin-render-frame`). The live shm write is not reachable in the harness, so no client round-trip test covers it.

- The session lock embargoes a View capture exactly as it embargoes an output or window capture: a granted View stream is refused while the lock is up, and a View stream already running is stopped by the lock and never resumes. Tests `a_granted_view_stream_is_refused_under_the_lock` and `a_view_stream_is_stopped_by_the_lock_and_never_resumes` in `gnoblin-capture`.

- Window nodes publish their `x`, `y`, `width` and `height` as `set` changes when they change: the size is the content size, the position is the placed frame origin.
- A window node announces `ready` to the shell once, in the batch after it is created with content: a `tree.event` with kind `ready` and the node id. Test `a_window_is_announced_ready_once_when_it_first_has_a_node` and `a_window_node_is_announced_ready_once_in_the_batch_that_follows_its_mapping`. Node-core step 2.
- Window nodes publish `focused`, `fullscreen`, `maximised` and `minimised` as `set` changes: `focused` comes from the keyboard focus, the others from the window manager state. All four are written together, and only when one of them changes. Test `a_window_state_is_published_only_when_it_changes`. Node-core step 3.
- A window node is announced `ready` each time its node is created. A window that is recreated gets a new node and is announced `ready` again for that node. Node-core step 2.
- A `tree.snapshot` node carries `props`: its non-default property values as `change` records with kind `set`, so a shell that attaches to a tree already holding values sees them at once. The scene keeps a value for every accepted write, the snapshot is built from those values, and the Lua mirror seeds each node's values from `props`. Node-core section 4.6.
- The compositor owns a main seat node (`seat`, named `seat0`) under the root. It publishes `pointer_x` and `pointer_y` when the pointer moves, and its `focus` reference names the window node that holds keyboard focus, published only when it changes; the shell cannot move or destroy the node, and a focus write to a non-window node is refused. Tests `the_main_seat_is_a_compositor_node_under_the_root`, `the_main_seat_publishes_the_pointer_only_when_it_moves`, `the_main_seat_publishes_the_focused_window_node`. Node-core step 4 (part).

- The XR extension's session crate (`extensions/gnoblin-xr/crates/xr-session`). `FrameLoop` drives a `Runtime` through the OpenXR session states: it begins the session once on `Ready`, ends it once on `Stopping`, runs one paired `wait_frame`, `begin_frame`, `locate_head` and `end_frame` per frame while the session is running, and stops cleanly on `Exiting`. A runtime error propagates to the caller. `openxr_runtime` is the OpenXR-backed `Runtime`: it loads the loader and creates a headless (`XR_MND_headless`) session. It is the only `unsafe` code in the extension. Nothing creates swapchains, submits layers or connects to the owner socket yet, and the loop passes empty layer lists to `end_frame`. Node-api-refactor section 9 (D34); the frame loop is tested with a fake runtime, and the OpenXR runtime's test runs only with `GNOBLIN_XR_MONADO=1`.
- The owner's transaction can send an `input` op (`messages.toml`, `tree.op`): `node` is a window node, `seat` a seat node, `kind` one of `motion`, `button`, `scroll` or `leave`, with node-local `x` and `y`, `button` and `state` (0 release, 1 press) and `dx` and `dy`. The compositor delivers the wl_pointer events (enter, motion, button, axis, frame, leave) to the window's client on that seat, independent of the main seat's focus. Moving to another window sends leave on the first and enter on the second. A node that is not a window, or an unknown seat, is refused `bad-value`; a connection that is not the owner is refused `not-privileged`. Node-core step 8 (part), XR seat input.

- `gnoblin_render_software::render_view` draws a scene subtree through a `ViewCamera` into an image of the View's size. The camera has a pose (`position` and `rotation_degrees`) and a `ViewProjection`: orthographic, perspective with a vertical field of view, or an asymmetric frustum from four edge angles, which gives a stereo eye. Each node draws its buffer on its own plane, mapped to pixels through the camera, so a rotated node lands on its projected corners. Siblings paint in tree order, and invisible subtrees, zero-opacity subtrees and nodes behind the camera draw nothing. Opacity multiplies down the tree and blends straight alpha. Clipping ancestors cut their children to their bounds on their own plane. A non-finite transform or a camera that cannot project removes only the affected nodes, never panics, and the same scene always gives the same bytes. Node-api-refactor section 5 (render targets).

- The owner can create a `seat` node named by a readonly `name` in the same transaction as its `new`; the compositor exposes an extra `wl_seat` global with that name (pointer capability) and removes it when the node is destroyed. A `name` must be non-empty and unique, else the transaction is refused `bad-value`. A later `set` of `name` is refused `read-only`. Node-core section 2 (readonly construct-time rule).

- A `View` node draws a scene subtree. Its `source` and `camera` are object-valued node references, set through the tree schema (`view.toml`) in a `tree.transact` and reaching the scene's `set_source` and `set_camera`. The composer renders the source through the camera's projection, pose and `fov`/`near`/`far` (orthographic when `camera` is unset) into a cached snapshot, and draws it at the View's rect as a surface. The cache re-renders only when the source subtree or the camera changes, by generation counter. A source that contains its own View, or a missing node, draws nothing. The `Screen` class extends `View`, so its wire field numbers from `make` onward shift by one (`camera` is 19); the tree schema is 0.2.0 and unreleased.

- A client's window request (move, resize, maximize, unmaximize, fullscreen, unfullscreen, minimize, window menu) reaches a shell holding the tree as a `tree.event` with `kind = "request"` and `request` naming it, on the window's node, with a per-tree `seq` and the frame that delivered it. While the shell holds the tree and the session is not locked the window manager does not act on the request; with no shell the window manager acts as before. Node-core step 8, section 5.2 (part). The lock's F21 and F22 tests and the unknown-kind log are not yet done.

- Pointer edges over a shell node reach the shell as `tree.event` with `kind` `enter`, `motion`, `leave`, `press`, `release` or `scroll`, node-local `x` and `y`, `button` on press and release, and `dx` and `dy` on scroll. The pick is the topmost shell node under the pointer, taken only while the session is unlocked and the shell holds the tree; a window topmost at the point takes the pointer, and a locked session routes the pointer to the lock surface. Key events are not routed to the shell, since the design sends keys to `seat.focus`. F30 is not enforced: an event for a node destroyed in the same frame is still sent. Node-core step 8, section 5.1 (part).

- The XR extension's pure planner (`extensions/gnoblin-xr/crates/xr-scene`). `plan` turns the `room` subtree of a mirrored tree into OpenXR layer placements in tree order: a visible View becomes a world-space `Quad` at its global transform in metres, a View under the head camera becomes a head-locked `Quad`, a View under a stereo camera becomes a `Projection`, and invisible subtrees and opacity products follow the node tree. Invalid transforms are skipped with a named reason. `pick` tests a ray against the quads in reverse submission order and returns the View's pixel coordinates; layers with `input` false are skipped. It has no OpenXR or socket dependency, and it is deterministic. Node-api-refactor section 9 (D34).
- The XR extension's owner-socket client (`extensions/gnoblin-xr/crates/xr-tree`). `TreeClient::connect` handshakes with the owner socket, attaches to the `tree` namespace at version 1, reads the snapshot into an `xr_scene::TreeView`, and `poll` applies each `changes` batch as it arrives. `gnoblin-xr` runs the bridge: it connects, opens the headless OpenXR session, and plans the `room` subtree on every frame against the located head pose, logging the layer and skipped counts when they change. It submits no layers yet, so the session runs with empty layer lists. `--help` and `--version` exit without connecting.

- A mapped toplevel is a `window` node in the tree (node-core step 3, section 9). The composer creates it on the first frame after the map, unparented, with an id from `NodeIdAllocator`; it carries the client's title and app id read-only, its `created` change is published in the next batch, its title and app-id changes as `set`, and its `destroyed` change when it unmaps. A shell holding the tree places windows itself, so the window manager's placement does not move them until the shell detaches. Node-core step 3, slice 5 and 6 (part).
- The stock tiled layouts (`data/lua/gnoblin/shell/tiling.lua`): `Tiling` (master and stack), `Monocle`, `GridLayout` (a near-square grid), `Dwindle` and `Kiosk`. Each is a `Workspace` class whose `arrange` is a binding over its window children, with its options as class fields (`ratio`, `gap`, `master_count`, `orientation`, `fill`, `split`). They are ported from `data/lua/gnoblin/layouts`, keeping its geometry and defaults; the old `M.arrange`, context and effects API is dropped. `GridLayout` is named so because `Grid` is the shell's box class. The stock library is not loaded by the compositor yet.
- The stock shell's focus, edge tiling and window rules (`data/lua/gnoblin/shell/focus.lua`, `snap.lua`, `rules.lua`, wired into `shell/init.lua`). `focus.lua` validates the window_management focus keys (`focus_mode`, `focus_new_windows`, `raise_on_click`, `auto_raise`, `auto_raise_delay`, `focus_change_on_pointer_rest`) and an unknown key raises. A window started from a launcher takes focus only under `focus_new_windows = "allow"`; `raise_on_click` raises a pressed window; `hover` and `hover-strict` focus the window the pointer enters; `auto_raise` raises the focused window after its delay. `snap.lua` tiles a window dropped on the left, right or top edge when `edge_tiling` is true (default false). `rules.lua` compiles window rules at load: patterns are validated, unknown fields and wrong types raise, and the last matching rule wins per field (`workspace`, `fullscreen`, `secure`). `corners` is validated but not applied to the window, because a window has no corners property: it is an effect. Not yet: `focus_change_on_pointer_rest` is validated but not wired; there is no leave event, so `auto_raise` acts only on the focused window; rules are re-evaluated on ready and pointer enter, not on title change. The compositor does not load the stock library yet, so none of this runs in a session. Covered by `focus_test.lua`, `snap_test.lua` and `rules_test.lua`, run from `node_class.rs` on Lua 5.5.

- `data/lua/gnoblin/shell/strip.lua` adds two workspace layouts, `Strip` (a row of columns, one window per column, scrolled to keep the focused column in view; `focus_left`, `focus_right`, `scroll_to`, and a closed column hands focus to its neighbour) and `Carousel3d` (a ring of windows in perspective, the focused one at the front; `rotate`). `Super+Left` and `Super+Right` move focus between Strip columns. The stock library is not loaded by the compositor yet, so these are inert until it is wired in. Node-core step 3, the stock shell.
- The stock shell gains a lock screen, an overview and a panel module (`data/lua/gnoblin/shell/lock.lua`, `overview.lua`, `panel.lua`). `Lock` engages on `requested`, and `Lock:unlock(password)` awaits the native core's `authenticate` answer and disengages on `ok`. `Desk:toggle_overview()` dims every workspace but the active one while it is off. `toast` moves from `init.lua` to `panel.lua`. The cases are in `tests/lua/ui_test.lua`. The compositor does not load the stock library yet.

- The stock shell gains a titlebar on every window (`data/lua/gnoblin/shell/decorations.lua`: title, a close icon that calls `Window:close()`, and drag on press), a fade before a window is destroyed (`Window:vanish`), a launcher on `Super+Space` (`shell/launcher.lua`: Return spawns the typed command, Escape closes it) and `Super+1` to `Super+9` to slide to a workspace (`shell/workspaces.lua`). The cases are in `tests/lua/ui_test.lua`. The compositor does not load the stock library yet.

- Node properties `tags` (`list<string>`) and `effects` (`list<object:effect>`) carry their values on the tree wire and into the render transaction. A `list<object:effect>` set takes at most 16 blur or shadow passes; a shader pass is refused `bad-value` because the compositor does not run shader passes from the tree yet. The Lua side encodes `tags` as text and `effects` as flat `effect` objects. The stock Lua library does not set these properties yet. Node-core section 4.4a.

- A queued tree transaction whose holder detached before its frame applies is refused `not-privileged` (`failed_op` -1) and changes nothing. The frame asks the submitter whether it still holds the tree when the transaction applies, not when it was queued (`TreeQueue::submit` takes that check; `tree_host` passes the session's holder test). Node-core step 3, slice 5 (part).

- The shell's owner-socket link (`gnoblin-lua`, `tree_link.rs`). `SocketTreeLink` implements `TreeLink` over the client end of a handshaken session socket, in the `tree` namespace of `gnoblin_proto` envelopes. `attach` and `transact` send their request and return; `poll` reads whatever has arrived without blocking and returns the attached reply, the snapshot chunks, the change batches and the transaction replies in arrival order. A refused attach or transaction, a closed socket, a frame cut short, an undecodable frame or payload, and a reply to an unknown request are link errors that name the cause; none of them panics. Node-core step 3, slice 10 (part). The compositor does not start it yet.

- The shell VM (`gnoblin-lua`, `shell_vm.rs`). `ShellVm` attaches to the tree owner socket, boots its mirror from the snapshot and loads `node.lua` against the Lua `core` table. Each frame it applies the serial-stamped change batches, runs the bindings and handlers the changes wake, and sends the queued transaction as one `tree.transact`. A refused transaction leaves the mirror unchanged, and `run` reports a Lua error or a refusal by name. Node-core step 3, slice 10 (part). The compositor does not start it yet.

- The shell's tree mirror (`gnoblin-lua`, `tree_mirror.rs`). `LuaMirror` holds the tree the shell sees: nodes by id with their class, parent, ordered children and typed properties. It rebuilds from a snapshot delivered in chunks and applies serial-stamped change batches, and it rejects a gap or a stale batch so the shell reattaches for a fresh snapshot. `LuaTxBuilder` records a shell transaction; it owns only its pending ops and overlay, and each read takes the mirror it started from, so its reads see its own writes and a refused transaction leaves the mirror unchanged. No I/O in this module: the socket link is `tree_link.rs`, and the runtime does not load the shell VM. Node-core step 3, slice 10 (part).

- The Lua `core` table (`gnoblin-lua`, `tree_core.rs`) binds the shell's tree to `node.lua`'s calls: `new`, `get`, `set`, `insert`, `destroy`, `call`, `root`, `classes` and `report`. `ValueKind` is named `TreeValueKind`, so it does not clash with the animation crate's type. Not yet: `node.lua`'s varargs `core.call` and return token, and the class system's F40 and F44 checks. Node-core step 3, slice 10 (part).

- The ten node-core tree classes are Lua classes (`gnoblin-lua`, `tree_classes.rs`). `build.rs` generates `tree_classes()` from the `tree` namespace of the schema, so the class table cannot drift from the schema. Each class lists its properties with their types, defaults and `readonly` flags, and its base class; a subclass inherits its base's properties, methods and signals, and `Window` reaches every `Node` property. The schema's records (`op`, `change` and `snapshot_node`) are values, not identities, so they are not classes. The `tags` (`list<string>`) and `effects` (`list<object:effect>`) lists are generated too: `TreeValueKind` has the `ListText` and `ListEffect` kinds. Node-core step 3, slice 2 (part).

- The Lua `core.insert` binding takes `(parent, node, index)`, the order `node.lua` calls it in. The binding took `(parent, index, node)` before this slice, so any caller written against it must change.
- The runtime role's shell VM slot (`gnoblin-lua`, `shell_role.rs`, `role.rs`). Opt-in and experimental. `GNOBLIN_SHELL_VM` names the directory of the shell's Lua. Unset, the role runs exactly as before: no shell VM is started and nothing is connected. Set, the role tries to start the shell VM and, on any failure, logs the cause by name and keeps serving with the slot empty. The VM opens its own session on the owner socket, found by the same rule as every other role (`GNOBLIN_IPC_SOCKET`, else `$XDG_RUNTIME_DIR/gnoblin/ipc.sock`), handshakes with it and attaches the tree on that connection; a refused or missing socket is named at connect time and the VM does not run. The role calls the VM's `tick` on every loop turn, not only when a change batch arrives, because the link is polled on each frame. Node-core step 3, slice 10 (part).
- The shell VM waits for its mirror. `boot_mirror` polls the link until the attach reply and the whole snapshot have arrived, so a shell that attaches before the compositor has sent its snapshot no longer boots with an empty tree. The loop is covered over the real socket wire (`tests/shell_loop.rs`): a compositor change batch sends no transaction, and a frame in which the VM wrote one sends exactly one `tree.transact`. Node-core step 3, slice 10 (part).

- The tree's frame boundary (`gnoblin-wayland`, `tree_frame.rs`). Each frame drains the transactions the shell queued through `TreeQueue`, in arrival order, before the composer draws, and applies each whole or not at all through `validate_and_apply` against the scene the composer draws from. A transaction that arrives after the drain waits for the next frame, so a late transaction draws one frame later and is never half-drawn. Frames that change the tree publish one `TreeBatch` (`State::take_tree_batches`). Tree work shares the 4 ms budget (`TREE_FRAME_BUDGET`): a transaction that starts after the budget is spent waits, whole, in order, for the next frame; the first transaction of a frame always runs. A frame draws with no shell attached. A transaction is checked against the batches published since its base serial when it applies, not when it was queued: a deferred transaction whose writes touch a node that a later batch changed is refused `stale-base` (`failed_op` names the first conflicting op), changes nothing and publishes nothing, while one that touches only other nodes applies in order. The rule is `gnoblin_render::tree_revision`, the same function `TreeSession::transact` calls, so the session and the frame cannot disagree. Node-core step 3, slice 6; section 4.5 (F6).
- The owner socket serves the tree (`gnoblin-compositor`, `ipc/tree_host.rs`, `ipc/tree.rs`). An owner connection attaches with `tree.attach` (version 1), is answered with its root and a snapshot of the scene, and transacts with `tree.transact`. One shell holds the tree at a time: a second attach while the first holds it is refused, and the hold is released when that connection closes. A transaction is applied whole or not at all; an applied transaction is answered on the reply and its changes are published as a `tree.changes` event at the new serial. A transaction before attach is refused. The owner is the connection with the `ReadConfig` capability. Calls and animations from an applied transaction are not dispatched yet, and the scene behind the host is a test implementation until the frame boundary (slice 6) supplies the real one. Node-core step 3, slice 5.

- The tree's frame boundary (`gnoblin-wayland`, `tree_frame.rs`). Each frame drains the transactions the shell queued through `TreeQueue`, in arrival order, before the composer draws, and applies each whole or not at all through `validate_and_apply` against the scene the composer draws from. A transaction that arrives after the drain waits for the next frame, so a late transaction draws one frame later and is never half-drawn. Frames that change the tree publish one `TreeBatch` (`State::take_tree_batches`). Tree work shares the 4 ms budget (`TREE_FRAME_BUDGET`): a transaction that starts after the budget is spent waits, whole, in order, for the next frame; the first transaction of a frame always runs. A frame draws with no shell attached. A transaction whose base serial is not the frame's starting serial is refused `stale-base` and changes nothing; this is the exact-serial rule `TreeSession::transact` uses. The section 4.5 per-id conflict rule (F6) is not implemented yet. Node-core step 3, slice 6.
- The `extensions/` directory holds out-of-process extension packages, each its own Cargo workspace excluded from the root workspace (`extensions/README.md`). `extensions/gnoblin-xr` is the scaffold for the XR extension (design/xr-extension.md, D34): the `xr-runtime` library finds the active OpenXR runtime by the loader's discovery rules (`XR_RUNTIME_JSON`, the XDG config dirs, `/etc/xdg`), detects `monado-service` and `wivrn-server` on `PATH`, and supervises the runtime service child with an on-failure restart policy (at most 3 restarts) and a clean stop on drop. No rendering and no layer submission yet.
- The replay harness (`crates/gnoblin-render/tests/replay.rs`) records a session of timed inputs (node transactions, frame ticks with `now`, scripted pointer events) and applies it through `validate_and_apply`. The tree hash is taken after every frame, and a session replays twice to the same per-frame hashes (F34). The session has a text format, documented in the module docs, that round-trips. Node-core step 3, slices 9 and 11 (part; the Lua parts are slice 10).
- Touchpad gestures reach the compositor. libinput swipe and pinch begin, update, end and cancel events keep their finger count, deltas and scale, and `Drm::dispatch` now delivers all libinput input to the host (previously it was pumped and never drained). A new `gnoblin-input` touchpad adapter turns samples into gesture events; `gnoblin-wayland` routes them to the `touchpad_gestures` bindings configured in Lua. Cancelled swipes, gestures on the lock screen, and gestures with no matching binding fire nothing; holds are dropped. Built-in `action` names are validated but have no executor yet, so only `command` bindings run. Covered by unit tests with synthetic samples; not yet verified on hardware.

- The tree transaction checks every property value against its schema type and refuses a wrong kind (`wrong-type`), an oversized text, byte string or list (`too-large`), and a non-finite number (`not-finite`). Node references name a live node of the property's class or a subclass. The catalog maps every tree property to a value kind and refuses to build for a schema type it cannot hold (node-core section 4.4a).
- The scene tree enforces ownership, id ranges and caps. Every node records its `owner` (the actor that created it) and `parented_by` (the actor that last parented it). `insert_as`, `remove_as` and `reparent_as` take the acting `Actor`: the shell may insert only ids below `COMPOSITOR_ID_BASE`, destroy only nodes it created (a subtree holding a compositor node is refused `NotOwner`), and move only unparented or shell-parented nodes. The node cap is `MAX_NODES` (65536), refused `TooManyNodes`; the depth cap is `MAX_DEPTH` (256), refused `TooDeep` on insert and on move. `remove` returns the removed subtree ids. Property writes are not governed by ownership. Node-api step 3, slice 3.
- Sampled curves: `SampledCurve` (`gnoblin-animation`) is a list of up to 1024 values evenly spaced over progress `[0, 1]`, interpolated linearly, validated when built (non-empty, first sample 0, last sample 1, finite, magnitude at most 1e6; overshoot between 0 and 1 is allowed). `TweenPlayer::start_sampled` and `retarget_sampled` play one, and `retarget` starts from the value on the node. The named easings are unchanged. Node-core step 3, slice 7.
- The node transaction (`gnoblin_render::transaction`): a batch of up to `MAX_OPS` (4096) ops `set`, `insert`, `destroy`, `call` and `new` is validated and applied as a whole. `validate_and_apply` works on a copy of the scene and swaps it in only when every op passes, so a refusal changes nothing; `Refusal` names the failed op index (`failed_op`, -1 when the batch is too large) and a kebab-case code from table 4.4. This slice emits `too-large`, `unknown-node`, `not-owner`, `root-immutable`, `cycle`, `unknown-class`, `unknown-member`, `wrong-type`, `read-only`, `not-animated`, `not-finite`, `out-of-bounds`, `out-of-range`, `bad-curve`, `bad-value`, `duplicate-node`, `too-many-nodes` and `too-deep`; the serial, IPC and paint codes (`stale-base`, `invalid-serial`, `shell-already-attached`, `not-privileged`, `bad-paint`) belong to later slices. Property names are checked against the `ClassInfo` trait, which a later slice implements from the schema catalog; the crate does not depend on `gnoblin-schema`. An animated `set` is validated and returned in `Applied` for the animation system; the scene is not written for it. Node-api step 3, slice 4.
- The schema format accepts `base` on an object (a class in the same namespace it inherits from; its properties, methods and signals come first in the derived class, and a property may not change type), `default` on a property (checked against its type), and `readonly` and `animated` booleans on a property (default false). Every existing schema parses unchanged. Node-core step 3, slice 1; the tree classes come in slice 2.
- The scene's paint and hit order is the sibling order: a node's position among its siblings is its paint order, last on top. `z_order` is removed from `NodeSpec`, and `set_z_order` and `set_z_orders` are replaced by `raise`, `lower`, `move_to`, `insert_at`, `reparent_at` and `set_order(parent, &[NodeId])`, which refuses a list that is not a permutation of the siblings. The composer no longer keeps a second stacking order. Node-api step 2 (D35).
- The schema holds the ten node-core tree classes (`Node`, `Surface`, `Window`, `Canvas`, `View`, `Screen`, `Camera`, `Seat`, `Workspace`, `Lock`) and the `tree` owner-socket messages, in namespace `tree`. The owner socket is the only transport, so the Lua stub, bindings and stock files leave the tree out. Node-core step 3, slice 2; the compositor does not serve these messages yet.
- The Lua module `gnoblin.shell.floating` (`data/lua/gnoblin/shell/floating.lua`) places floating windows in free space, cascade and centre, as the `Floating`, `Cascade` and `Centred` classes on the node class system. Its placement matches the 212 committed placement goldens. The compositor still places floating windows natively; the module is not loaded yet.
- The shipped `gnoblin.layouts.strip` layout places windows as columns in a strip wider than the screen, with a view
  offset, stacked or tabbed columns, and the `width`, `consume`, `expel`, `move_column`, `fit_all`, `toggle_tabbed`,
  `focus` and `scroll_to` commands. It replaces the scrolling layout, which is removed.
- The window manager accepts up to `MAX_WINDOWS` (256) windows. A window past the cap is refused with
  `TooManyWindows` before any state changes. Each provider in the layer registry has its own limit; extension
  providers keep the default of 32.
- Named workspace sets: the `workspace_sets` field of `gnoblin.configure` declares sets that name the workspace each
  output shows, checked as a whole list when the session starts. `gnoblin.workspaces.activate_set(name)` and
  `gnoblinctl workspace activate-set NAME` make a set live; an unknown name is refused with `unknown-set`. A named
  output that is not connected falls back to the primary output with a logged warning.
- `Window.client_type` (`"wayland"` or `"x11"`, bridge API 1.93) in window records, `gnoblin.find("Window", {client_type = ...})`,
  the window rule `match.client_type` key and `gnoblinctl window list` and `window match`.
- The services role starts the GeoClue agent next to the polkit agent, with the policy of the `location` table
  in the configuration. Until now the role claimed the `location.agent` slot but never served it. With no system
  bus the agent is skipped and the role carries on.
- A Vulkan renderer for the Rust compositor (`gnoblin-backend`, feature `vulkan`), drawn with `ash` and the shared WGSL
  passes compiled by `naga`: solid and `wl_shm` surface layers, opacity, tint, corner radius, the portal overlay and
  the cursor, readable back as an image and compared pixel for pixel with the software renderer. It runs on a GPU or
  on a software driver such as lavapipe. Presentation, dmabuf import and effects are not done yet. The new
  `compositor.renderer` setting (`auto`, `vulkan`, `gles` or `software`) chooses the renderer; `auto` keeps today's
  choice, and a named renderer the session cannot create is refused with the setting named.
- Text a child prints without the `gnoblin-log` facade (a panic, a Lua `print`, output from a C library or Xwayland)
  is logged as component `raw`, tagged with the child's role and stream. Only the compositor, runtime and portal roles
  are captured by default, and the services role only with `GNOBLIN_LOG_RAW_SERVICES=1`; other children are read and
  discarded. Each line is scrubbed of secret-looking text on a best-effort basis that cannot recognise every secret, see
  `docs/guides/logging.md`.
- The screensaver service watches logind. A `PrepareForSleep(true)` (suspend or hibernate) locks the session while the
  sleep inhibitor is still held, then releases the inhibitor. A `Lock` signal on the session's own logind object locks
  the session. The service takes the inhibitor again on resume. A logind lock on another session is ignored. Without a
  system bus the watch is skipped.
- Window rules accept a workspace name declared in `workspaces`, in the `workspace = {id = ...}` target and the `workspace-id` match. A declared name resolves to its position. An undeclared name that is not a number is refused when the configuration is committed, with the field named. The declared list also sets the persistent workspace count.

### Fixed

- The Lua `core.insert` binding takes `(parent, node, index)`, the order `node.lua` calls it in. It had taken `(node, parent, index)`, so each child insert from `node.lua` swapped the parent and the child.

- The `gnoblinctl` and `gnoblin-wasm` catalogs leave out the owner-only `tree` namespace (`Catalog::load_dir_except(dir, &["tree"])`), as `gnoblinctl/build.rs` already did. Its `lock` and `destroyed` classes collided with the public ones, so `the_command_tree_matches_the_schema` and `the_generated_wit_matches_the_schema` failed. `gnoblin-schema/tests/owner_only_tree.rs` fails when any other crate loads the full catalog.

- Window records past the 32nd window do not trip a debug assertion, because the window manager's layer provider has
  its own limit.
- The `layout_failed` signal's documented `reason` values now match the ones it emits.
- Pointer picking goes through the scene: a press or pointer motion over a window that a layout turned or moved
  reaches the window where it is drawn, not the unrotated rectangle it was placed in. Stacking ties still go to the
  last-drawn window.
- The composer's window nodes take their ids from `NodeIdAllocator` (`gnoblin-render`, `scene_ids.rs`), which starts at the compositor id base, never reuses an id and refuses exhaustion. Before, they started at 1, inside the shell's id range, so a shell node could collide with a window node (`duplicate-node`). A window whose node cannot be created is logged and not drawn, rather than dropped silently.

### Changed

- A window's `wp_alpha_modifier_v1` factor and the opacity a window rule sets fold into the window's scene node, as
  placement and shell opacity already do, so the composer draws the node's opacity alone. A closing window keeps the
  factors it had when its close fade began, as it already kept its placement and shell visuals: setting one during the
  close fade no longer changes the fade.
- Log records are written to `gnoblin.jsonl` in the state directory, one JSON object per line. `gnoblinctl log` and
  `gnoblinctl log --follow` read that file. The `gnoblin-store` crate, the SQLite log store, `gnoblinctl store`, the
  store IPC verbs and the `--store` flag are removed. `gnoblin.jsonl` and the legacy `store` files are not deleted.
- Log messages no longer start with a `[tag]` prefix. A crate's logger names its component, so a tag equal to
  that component is dropped, and any other tag becomes an `area = "tag"` field on the event. `xtask logging` fails a
  level macro whose message still starts with `[tag] `.
- Layouts with `extent = "unbounded"` must declare `kind = "tiling"`; a floating or kiosk layout that asks for it is
  refused when it loads. The wm now enforces `extent` when it settles a reply: a placement whose rectangle (before its
  transform) lies outside the output, or past the coordinate bound when unbounded, refuses the whole reply. The
  `carousel3d` layout declares `extent = "unbounded"`.

- Xwayland windows get Gnoblin's own frame, rules and effects like Wayland windows. Mutter no longer launches
  `mutter-x11-frames` or reparents X11 clients into frame windows. An X11 window that does not turn decorations off is
  framed under `frame.mode = "auto"` (the default for X11 when no rule sets a mode), `_GTK_FRAME_EXTENTS` feeds the
  visible bounds, `_NET_FRAME_EXTENTS` and `_NET_REQUEST_FRAME_EXTENTS` report Gnoblin's border, and Mutter's stock
  shadow no longer draws for managed X11 windows. A border change keeps the client where it is.
  `mutter-x11-frames` is no longer built or installed. A request for frame extents before the window exists is answered
  from the rules that match only the window type and `client_type`. The Mutter code for a frame window (`MetaFrame`,
  its sync counter and borders, the `_MUTTER_FRAME_FOR`, `_MUTTER_FRAME_EXTENTS` and `_MUTTER_NEEDS_FRAME` properties,
  and the frame mask in the X11 window actor) is removed in patch `11-window-frames/0011`.

### Changed

- `patches/` is organised into topic directories (`patches/mutter/<NN>-<topic>/<NNNN>-<slug>.patch`) with dense numbers,
  and each patch project has a `series` file that is the only source of apply order. The patch bodies, their order and
  the patched trees are unchanged (`design/patches-layout-mapping.tsv` maps old paths to new). CMake, the Nix package,
  `scripts/check-build-deps.py`, `scripts/list-tarball-sources.sh` and the tests read the series instead of sorting a
  glob. `scripts/manage-patches.py check` now fails when a patch file is missing from its series or the series names
  a missing file, and `renumber` and `path` are new. `patches/README.md` describes the layout.
- One table of wire operations (`src/control/gnoblin-api-operations.h`, round 2; no wire change, API stays 1.92).
  The version gates and their errors, the hello method list, the `gnoblinctl` minors, the supervisor packet flags and
  the compositor dispatch read it instead of repeating names in chains. Record methods of `lua_api[]` declare their
  positional arguments, and the runtime packs them from that schema with the same errors as before.
- `window:set_effect(name, value)` declares its arguments in `lua_api[]` like the other record methods, so a wrong
  call fails with `window:set_effect takes a name and a value`. `scripts/audit-lua-annotations.py` now also reads
  the schemas that `clang-format` wraps onto a second line, which it skipped before.
- `gnoblinctl lua` has the names and call shapes of a Lua configuration (round 2; no wire change).
    - The console reads `lua_api[]`, the table the runtime installs, instead of a prefix chain of its own. A name
      that the table does not hold fails with `gnoblin.NAME is not part of the Lua API`; the table of removed
      names (`removed[][]`) is gone. A name that needs a runtime (`gnoblin.timers`, `gnoblin.process`,
      `gnoblin.tween`, `gnoblin.idle.watch`, `gnoblin.input.grab`, `gnoblin.input.on`,
      `gnoblin.settings.override`, `gnoblin.load`, `gnoblin.array`, `gnoblin.snapshot`, `gnoblin.configure`,
      `gnoblin.window_rule`, `gnoblin.animation`, `Window:set_effect`, `Window:present` and the `on` and
      `watch` methods of records) raises one error: `NAME is not available in gnoblinctl lua because the
console has no runtime`. The generic call that accepted one table for any unknown name is gone, so a call
      is always positional. `scripts/audit-lua-annotations.py` checks that every console name is in
      `lua_api[]` and that every wire operation of a console record has a console method.
    - `gnoblin.find`, `gnoblin.ease` and `gnoblin.lerp` work in the console. `gnoblin.windows.list` and
      `gnoblin.layers.list` take no filter, as in a configuration; `gnoblin.config` holds only `path()`,
      `default()` and `restore_default()` and cannot be assigned to.
    - `gnoblin.animations.preview`, `inspect` and `surfaces` are gone from the console. `window:inspect_animation`
      and `window:preview_animation` (and the same on a `LayerSurface`) replace them. `SnapContext:commit`
      takes `(monitor, x, y, width, height)`, `Window:move_to_workspace` takes a `Workspace` and
      `Window:move_to_monitor` takes an `Output`, as in a configuration.
    - `gnoblin.location.authorize_app {request_id, allow, accuracy}` is gone. The `gnoblin.location.authorization-requested`
      event carries `event.request`, a `Request` of kind `location` with `answer(allow, accuracy?)` and `cancel()`.
- `gnoblin.find`, a read-only `gnoblin.config` and handle arguments (round 2; no wire change).
    - `gnoblin.find(kind, filter?)` lists the live objects of `Window`, `Workspace`, `LayerSurface`, `Output`,
      `Shortcut`, `Launch`, `InputSource` or `Process`. A table filter matches declared scalar fields
      (`gnoblin.find("Window", {app_id = "notes", focused = true})`); a function filter is a predicate. An unknown
      kind, an unknown field or a field that is not a scalar is an error that names what works.
      `gnoblin.windows.list` and `gnoblin.layers.list` take no argument in Lua now. `gnoblin.focus.history`
      keeps its filter, because its result has an order and a limit. Socket and `gnoblinctl` reads keep theirs.
      See `docs/shell-api/find.md`.
    - `gnoblin.config` holds only `path()`, `default()` and `restore_default()`. Assigning to it, or reading any
      other key, raises an error that points to `gnoblin.configure` and `gnoblin.settings.override`, while the
      config loads as well as at run time. `gnoblin.snapshot()` works only while the config loads and raises an
      error at run time; `gnoblin.settings` reads the committed values. Assigning to a named shortcut or
      autostart entry at run time is an error. At run time `gnoblin.configure`, `gnoblin.window_rule` and
      `gnoblin.animation` return an `Operation` that succeeds when the runtime commits the change, and fails with
      `invalid_argument` when the change is rejected.
    - `Window:move_to_workspace` takes a `Workspace`, `Window:move_to_monitor` takes an `Output`, and
      `gnoblin.layers.animation_policy` takes a `LayerSurface`; IDs and numbers still work.
      `gnoblin.animations.surfaces`, `inspect` and `preview` and the `AnimationSurface` record are gone: use
      `window:inspect_animation(name, event?)` and `window:preview_animation(name, event?, autoplay?)`, or the
      same methods on a `LayerSurface`. The wire methods `animation.surfaces`, `animation.inspect` and
      `animation.preview` stay.
    - `SnapContext:commit(monitor, x, y, width, height)` replaces `commit({monitor_id = ..., frame = {...}})`.
      `monitor` is an `Output` or a monitor ID.
    - `Window.monitor` is annotated as what it holds: the origin `{x, y}` of the monitor that holds the window, not a
      monitor index (`monitor_index` and `monitor_id` name the monitor). The name stays, because the socket
      `windows.list` returns the same record as Lua and `window.list` keeps `monitor`. `Window.offset` is `{x, y}` in
      the Lua record.
    - The layer animation rule loses the `in`, `out` and `ease` aliases. Use `layer_open`, `layer_close` and
      `easing`.
    - The generated stub is corrected: `Workspace.id` is required, `create` accepts an ID, and the window count of a
      `Workspace` is `window_count`, the key the record always had. The stub said `windows`, and
      `workspace:watch("windows", fn)` never fired.
- `Tween`, `Process` and `AnimationPreview` are live objects, and every subscription has one shape (no wire change;
  the tween track gains an optional `target_kind`).
    - Each has `id`, `alive`, `revision`, `on` and `watch`, and one handle per id. `tween:on("finished")` runs when
      the timeline ends; `on("closed")` runs for every end, with the outcome (`"finished"`, `"killed"`,
      `"cancelled"`) as its reason. `Tween:on_finished` is gone. `Tween:property` reads the properties it may drive
      from the `animatable` list of each kind, not from the word "Window".
    - `gnoblin.process.list()` and `gnoblin.process.by_id(id)` find a process again;
      `gnoblin.process.by_id(window.process)` resolves a window's `process`. `process:watch` follows `pid`,
      `status`, `exit_code` and `signal`. The process `closed` signal still runs once at the end. An
      `AnimationPreview` has no `session` field (use `id`) and ends when `stop()` succeeds.
    - `gnoblin.input.on`, `Request:on` and every object's `on` and `watch` return a `Subscription`; call
      `sub:unsubscribe()`. `gnoblin.input.on` returned a handler ID string before and could not be undone.
    - `watch` takes only fields that change while the object is alive (`@observable` in the annotations).
      `id`, `alive`, `revision` and fixed fields raise an error that lists the fields that work. `Workspace` has
      `revision`.
    - A config reload ends every object the runtime owns, not only `Override` and `Grab`: `Shortcut`,
      `IdleWatch`, `Launch`, `Process`, `Tween` and `AnimationPreview` run their `"closed"` listeners with the
      reason `"config_reloaded"` (`"runtime_stopped"` when the worker stops), and a pending `Request` takes
      `on("closed")` with the same reasons. See `docs/shell-api/types.md`.

- `FocusContext` and `MenuContext` are one scoped `Grant` (bridge API 1.92). A shortcut, pointer binding,
  shortcut session key, input event or window menu request carries `event.grant`. `grant:narrow(scope, window?)`
  returns a grant for fewer operations (`focus`, `begin_move`, `begin_resize`, `snap_context`, `text_target`)
  and one window; nothing widens a grant. Each operation of a gesture can run once, so one key press can focus a
  window and then begin a move; before, the first operation used up the whole gesture. Expiry (five seconds),
  reload and lock rules stay, and timers and other events have no grant. `Window:focus`, `begin_move`,
  `begin_resize`, `gnoblin.windows.snap_context` and `gnoblin.input.text_target` take a `Grant`;
  `event.focus_context` and `event.menu_context` are gone from Lua and from `gnoblinctl lua`. The default
  pointer bindings use `event.grant`. Socket clients keep `focus_context` and the method names; their token now
  lasts for the gesture and each operation uses it once. A Lua window menu move or resize failed before
  because the session demanded a gesture context for it; it now works. See `docs/shell-api/grants.md`.

- One chord grammar for every shortcut mechanism (no wire change). A key combination is zero or
  more modifiers in angle brackets, then one key, such as `<Super><Alt>F9` or `XF86AudioMute`.
  One parser (`src/control/gnoblin-chord.h`) reads it in the configuration validators, in the
  Lua runtime (`gnoblin.shortcuts.bind`, `gnoblin.input.on`, `keybindings.keyboard`,
  `keybindings.pointer`, `keybindings.wm`, `mutter` and `wayland`), in `configure.shortcuts`, in
  the stylus keybindings and in the compositor. Before, each place checked a chord on its own:
  dynamic binds only checked the length, the compositor said "already claimed" for a mistake, and
  Mutter skipped an unknown `<Foo>` and bound the bare key.
    - A chord in another form, such as `Super+Alt+F9` or `ctrl-q`, is an error that names the
      form to write, for example `Write <Alt><Super>F9`. So are `<Windows>q`, a second key, a
      repeated modifier and `<Mod2>` to `<Mod5>`. Mutter's spellings `<Primary>`, `<Ctrl>`,
      `<Ctl>`, `<Shft>` and `<Mod1>` still work and read as `<Control>`, `<Shift>` and `<Alt>`.
    - Gnoblin gives a chord back with the modifiers in the order
      `<Shift><Control><Alt><Meta><Super><Hyper>`: `Shortcut.accelerator`, the `shortcut.bind`
      reply and the events use that form.
    - A pointer chord needs at least one modifier and ends with `Button1` to `Button3`.
    - An empty stylus keybinding still means none.
    - The four mechanisms stay, because they do different jobs. See
      [Which mechanism to use](docs/guides/shortcuts.md#which-mechanism-to-use).
- A Lua handler that throws now keeps running on later events. Before, Gnoblin disabled a `gnoblin.on`
  listener or an input handler after its first error, and an input handler that threw stopped the
  handlers after it. Now Gnoblin logs the error with the event name and the Lua stack, and every
  other handler of the event still runs. This applies to `gnoblin.on`, the `on` and `watch`
  callbacks of objects, `Process` and `Request` signals, `Operation` callbacks, tween callbacks and
  input handlers.
    - Each handler call has its own instruction budget, so a loop that never ends in one handler no
      longer makes the handlers after it fail.
    - An input handler that throws or returns an invalid value decides nothing, and only its own
      queued operations are dropped.
    - A handler that keeps failing is logged for its first three failures in a row and then once for
      every 1000. The user notice appears once for each run of failures.
    - See [Handler errors](docs/shell-api/errors.md#handler-errors).
- Shortcuts and launches are live objects in Lua; the functions that took IDs or tokens are gone
  (hard cutover; the wire names `shortcut.bind`, `shortcut.unbind`, `shortcut.session.end`,
  `launch.begin` and `launch.end` stay for socket clients, and no wire change was needed).
    - `gnoblin.shortcuts.bind(id, accelerator, options?)` returns a `Shortcut`: interned by ID,
      with `alive`, `on` and the signals `activated`, `deactivated`, `session_activated`,
      `session_key`, `session_ended` and `closed`. A refused binding ends at once and `closed`
      gets the reason.
    - `shortcut:unbind()` replaces `shortcuts.unbind(id)`, and `shortcut:end_session(session_id)`
      replaces `shortcuts.end_session(id, session_id)`: the shortcut owns the session.
    - `shortcuts.list()` now returns the shortcuts this runtime bound, as the same objects. The
      configured shortcut records move to `shortcuts.configured()`; the socket read
      `shortcuts.list` is unchanged.
    - `gnoblin.launches.begin(token, application, options?)` returns a `Launch` with `alive`,
      `on`, `watch` and the signals `changed`, `started`, `timed_out` and `closed`.
      `launch:finish()` replaces `launches.finish(token)`. `launches.list()` returns the same
      objects. A `Launch` has no link to a window or `Process`: the compositor does not record
      which window answered a launch.
    - `gnoblinctl lua` returns `Shortcut` and `Launch` records with the same methods.
- `gnoblin.input.select_source(type, id)` is gone from Lua: `InputSource:select()` replaces it.
  `input.sources()` and `input.current_source()` return `InputSource` objects, one for each type
  and ID, with `alive`, `on` and `watch` and the signals `changed`, `selected` and `closed`. The
  events `gnoblin.input.source-changed` and `gnoblin.input.sources-changed` move them. The wire
  method `input.select` stays; no API bump.
- A `Process` has `alive` and `on("closed", fn)`. The end signal is `closed`, as on Window,
  Workspace and Output, so code can treat every object alike; `status`, `exit_code` and `signal`
  say how it ended, and `wait()` and `kill()` stay.
- `gnoblin.session.suspend()`, `hibernate()`, `reboot()` and `poweroff()` finish with logind's real
  answer (bridge API 1.89). Gnoblin reads the capability (`CanSuspend` and its siblings), then asks
  logind for the action, and the Operation ends with `{accepted = true}` or with an error. See
  [session and runtime health](docs/shell-api/session-runtime.md).
    - `denied`: the capability answer is `no`, or logind or polkit refuses the action.
    - `unsupported`, `busy`, `unavailable`, `timed_out` and `internal` name the other failures.
    - The Lua runtime keeps running after the call. Before, the call stopped the runtime and the
      supervisor asked logind afterwards, so the Operation could not report a refusal.

- The orientation lock and the privacy screen use the new override call (bridge API 1.90). The
  Lua functions `gnoblin.input.set_orientation_lock(value)` and
  `gnoblin.outputs.set_privacy_screen(value)` are gone from Lua and from `gnoblinctl lua`. Use
  `gnoblin.settings.override("input.orientation_lock", value)` and
  `gnoblin.settings.override("monitors.privacy_screen", value)`, and `override:revoke()` in place
  of `"inherit"`. The wire methods `input.set_orientation_lock` and `monitors.set_privacy_screen`
  stay for socket clients, with two changes: a configuration reload that commits ends the value
  that a socket client set (the orientation lock already did), and `monitors.set_privacy_screen`
  fails with `true` or `false` when no output has a privacy screen. A newer value replaces an older
  one, whoever set it.

### Removed

- The external prompter for keyring unlock passwords is removed. The built-in keyring asks for its unlock password
  through the `prompt.secret` slot, like any passphrase request, and fails closed when no shell claims the slot. See
  `docs/shell-api/passphrase-prompts.md`.
- The `prompts` configuration section and its `prompts.enabled` key are removed. A configuration that sets them is
  refused when it loads. `keyring.enabled` still turns the built-in keyring on and off.
- The `when` key of `touchpad_gestures` entries is removed, with its `"unlocked"`, `"locked"` and `"any"` values and the
  legacy `"normal"` and `"unlock-screen"` spellings. A configuration that sets `when` is refused when it loads. Gesture
  bindings run only while the session is unlocked and never reserve a gesture on the lock screen, so a binding meant for
  the lock screen can no longer be written. Gestures there are left to the lock screen client.

### Fixed

- A `config.commit` that removes window rules is answered only after the compositor loop has applied the removal. Before,
  a window the client mapped right after the answer could still be placed under the removed rule, for example maximised
  at the output size, depending on the order the loop drained its queues.

### Added

- `gnoblin.input.grab(options?)` takes the keyboard and returns a live `Grab` (bridge API 1.91). See
  [keyboard grabs](docs/shell-api/input-grab.md). Before, the only keyboard capture was the
  shortcut session of a `modal` or `capture_input` shortcut: one slot, a fixed 10 second timer,
  and key events without text.
    - The grab is a lease. It expires after `timeout` idle seconds (1 to 60, default 10) unless
      `grab:renew()` runs or a key event arrives; each key event starts the timeout again, because
      someone who types still needs the grab.
    - `grab:on("key", fn)` gets `{key, keysym, keycode, modifiers, pressed, repeated, text,
composing, time}`. `text` is the character a press types. Gnoblin feeds each press to the
      compose table of the locale (xkbcommon), so a dead key or a Compose sequence gives one
      composed character on the key that completes it, and `composing` is true inside the sequence;
      a key that breaks the sequence has its own text.
    - `grab:on("closed", fn)` gets the reason: `released`, `expired`, `replaced` (a second grab
      takes the one slot), `locked`, `preempted` (Mutter gave the keyboard to a popup or other
      grab), `config_reloaded`, `runtime_stopped`, or the message of the compositor when it refused.
    - The grab refuses to start while the session is locked and ends when it locks. It never takes
      the keyboard from a shortcut session or `shortcuts.capture`, and they cannot start while it
      holds the keyboard. Its events carry no `focus_context`.
    - The grab refuses to start while any key is down. Mutter gives the release of a held key to
      the grab, so the focused application, which saw the press, would never see the key lift.
      The refusal names the held key. Start a grab from a key release or a later event.
    - Shortcut sessions keep their fixed 10 second timer. It guards a user gesture against a lost
      key release and nothing owns it, so nothing can renew it; a script-renewed lease would let
      a script hold a session open, and a grab that carries a `focus_context` would let a script
      move windows without a gesture.
    - `gnoblin.input.grab` raises an error in `gnoblinctl lua`: a console script has no runtime
      to receive keys.
- `gnoblin.settings.override(path, value)` and `window:set_effect(name, value)` return a live
  `Override` that layers a value over the committed configuration, or an effect over the window
  rules of one window, until it is taken back (bridge API 1.90). See
  [overrides](docs/shell-api/overrides.md).
    - Settings that can change at runtime are `window_management.*`, `compositor.*`,
      `cursor.theme`, `cursor.size`, `layer_shell.preserve_active_window`,
      `monitors.privacy_screen` and `input.orientation_lock`. Any other path raises an error that
      says why it cannot change. The value is checked as the configuration checks it.
    - The effects are those of window rules: `opacity`, `blur`, `blur_ignore_shadows`, `shader`,
      `shader_uniforms`, `frame` and `corners`. The effect goes over the matching rules as one more
      rule and changes only what it sets.
    - `override:revoke()` returns the target to the next older layer, and
      `override:on("closed", fn)` gets the reason: `revoked`, `replaced`, `window_closed`,
      `config_reloaded` or the compositor's message.
    - The latest override of one setting, or of one effect of one window, wins; the older one ends
      with `replaced`. A reload, a Lua worker restart and the close of the window end overrides.
    - `gnoblin.settings` keeps showing the committed values.
- Outputs are live objects (bridge API 1.88). See [outputs](docs/shell-api/outputs.md).
    - Every read of one display returns the same `Output`, with `alive`, `on` and `watch`.
    - An output also has its logical monitor's index, size and work area, and its privacy
      screen state.
    - The privacy screen request covers the whole session, because the compositor keeps one request and applies it
      to every output that has a privacy screen. An `Output` has no method for it.
      `output.privacy_screen` and `output:watch("privacy_screen", fn)` read the state. Bridge API 1.90 replaced
      the first write call, `gnoblin.outputs.set_privacy_screen(value)`, with
      `gnoblin.settings.override("monitors.privacy_screen", value)`. The wire call
      `monitors.set_privacy_screen` is unchanged.
    - New events tell Lua and shell processes when a display is added, changes or is removed.
    - Monitor events and monitor reads stay for shell processes only.
- `gnoblin.idle.watch(seconds)` returns a live `IdleWatch` for any idle time from above 0 up to
  86400 seconds, with `alive`, `idle`, `seconds`, `on("idle" | "active" | "closed")`, `watch` and
  `cancel()`. Each runtime can have up to 256 watches. A reload ends every watch. See
  [idle watches](docs/shell-api/idle.md).
- The `gnoblin.location.authorization-finished` event (bridge API 1.87) reports how each location
  request ended: `allowed`, `denied` or `timeout`. A Lua location `Request` now finishes from it,
  so `finished()` reports the real outcome when a shell process answered the request instead of
  `timeout`.
- Lua timers and Operation methods. `gnoblin.timers.after(seconds)` returns an Operation that
  succeeds after the given time, and `gnoblin.timers.now()` reads a monotonic clock. Every
  Operation now has `map(fn)`, `timeout(seconds)` and `cancel()` beside `on_complete`. Pending
  timers end when the config reloads and do not delay the reload. See
  [operations, errors and timers](docs/shell-api/errors.md).
- `gnoblin.process.spawn(argv, options?)` starts a program and returns a `Process` with its
  `pid` and `status`, `wait()` for its exit and `kill(signal?)`. Options set the working
  directory, environment, standard input, where each output stream goes and how much of it to
  capture, and a timeout. Output is returned as raw bytes. A process keeps running across a
  config reload. See [processes](docs/shell-api/processes.md).
- Window records carry `pid`, the client's process ID from the display server, and `process`,
  the `Process.id` when `gnoblin.process.spawn` started the program. `window:present()` focuses
  such a window without a key press, after the user has not typed or clicked for 250 ms. A
  shell can now open a passphrase prompt that takes typing straight away. See
  [focus and activation](docs/shell-api/focus-activation.md).
- Keyring and GPG prompts, polkit authentication, location authorization and session-lock
  requests reach Lua as one `Request` object, `event.request`. It has `id`, `kind`, `status`,
  the request's detail fields and `finished()`, an Operation that succeeds once with the
  outcome. `answer()`, `cancel()`, `claim()` and `on()` exist only on the kinds that use them.
  See [requests](docs/shell-api/requests.md).
- The Lua configuration receives `gnoblin.session.lock-requested` when it listens for it, so
  a config can start a locker without a separate shell process. See
  [session lock](docs/session-lock.md).
- Windows, workspaces and layer surfaces are live objects in Lua. Every read of one returns
  the same object, updated with each change, with `alive`, `on(signal, fn)` for its
  `changed`, `closed` and other signals, and `watch(field, fn)` for one declared field. Before,
  each read returned a detached copy. See [live window objects](docs/shell-api/windows.md#live-window-objects).
- `window:set_opacity`, `set_scale`, `set_rotation`, `set_offset` and `reset_visuals` change
  how the compositor draws a window without moving its frame. Clicks follow the drawn
  position, a config reload resets them, and they fail while the session is locked. Window
  objects show the values in `opacity`, `scale`, `rotation` and `offset`, and `gnoblinctl lua`
  has the same methods. See
  [change how a window is drawn](docs/shell-api/windows.md#change-how-a-window-is-drawn).
- `gnoblin.tween()` builds a timeline of window opacity, scale, rotation and offset changes,
  intervals and callbacks, and the compositor plays it in its render loop. `play`, `pause`,
  `seek` and `kill` control it; a reload or a session lock cancels it. `gnoblin.ease` and
  `gnoblin.lerp` give the same easing math in Lua. See [tweens and easing](docs/shell-api/tweens.md).
- Packages for Debian testing (`.deb`) and Alpine edge (`.apk`, release assets only), and arm64
  builds of every package. A single build
  now writes the rpm, deb, Arch and tar packages: `scripts/build-prefix.sh` builds Gnoblin
  into a prefix and pkgsmith writes the packages from it with the definitions in `pkgsmith/`.
  See [packaging and releases](docs/distribution.md).
- Signed package repositories for Fedora, openSUSE Tumbleweed, Debian testing and Arch, each with
  a `stable` and a `nightly` channel. A release tag publishes the stable channel.
  A daily build of the main branch publishes the nightly channel when main has
  moved. `scripts/pkgrepo` builds, signs and uploads the repositories, and
  `design/package-repositories.md` describes the layout. See
  [package repositories](docs/package-repositories.md).
- A small notice at the top centre of the screen when a config reload finishes (`Config reloaded`,
  gone after about 2 seconds), when a reload is rejected (the first line of the error, until it is
  clicked or a reload succeeds), and when a shortcut or command does not start
  (`<name> could not start: <reason>`). See
  [reload and error notices](docs/guides/files_and_load_order.md#reload-and-error-notices).

- System drop-ins load on their own. Lua files in `gnoblin/conf.d` under each `XDG_DATA_DIRS` entry
  (such as `/usr/share/gnoblin/conf.d`) and in `/etc/gnoblin/conf.d` run before your `init.lua`, or
  after the built-in defaults when you have no `init.lua`, so a package no longer needs a `gnoblin.load`
  line in your config. A drop-in that fails is skipped and shown in the recovery notice. See
  [system drop-ins](docs/guides/files_and_load_order.md#system-drop-ins).

### Changed

- The RPM specs, the PKGBUILDs, the openSUSE chain and `packaging/` are gone. Package
  names, the Fedora epoch and the `Provides` and `Obsoletes` entries are unchanged, so
  existing installs upgrade. The portal backend is now version `51.0.0` with a release
  number that follows the Gnoblin release. `gnoblin-ananicy-rules` and
  `gnoblin-geoclue-integration` are built for every format and take the Gnoblin version.
- COPR no longer receives releases, because pkgsmith writes binary packages and COPR builds
  source RPMs. The release no longer carries source RPMs, PKGBUILDs or `gnoblin.db`. Use
  the package repositories or the release packages. `scripts/install-system.sh`, which
  installed from COPR, is removed. The `gnoblin-ananicy-v*` releases are replaced by the
  package of the same name in every Gnoblin release.
- `scripts/prepare-release.sh` sets the version in `src/gnoblin-version.json` only.
- The Nix package and module moved from `packaging/nix/` to `nix/`. The flake's outputs are unchanged.
- Packages accept a pre-release version. The source bundle keeps the SemVer
  spelling (`0.1.13-nightly.…`) and pkgsmith spells the package version for the
  package manager (`0.1.13~nightly.…` for RPM). Release versions are unchanged.
- `design/package-targets.json` (formerly `packaging/targets.json`) and the platform support page list Arch, Fedora 45
  and openSUSE Tumbleweed as candidates. Release run 39855522942 passed their
  package build, install, stock GNOME coexistence and removal gates. Graphical
  session support is not claimed.

### Fixed

- `gnoblin.input.grab` was refused for ever, naming a key that was up, after the release of that key was swallowed.
  The compositor refuses a grab while a key is held and tracked the held keys behind the diagnostic console, which
  takes every key while it is open: a key pressed before the console opened and released under it stayed held until
  the user tapped it again. Held keys are now tracked ahead of the console (Mutter patch
  `0132-track-held-keys-before-the-console`), so a key released under the console is up, and a key pressed under it
  and still down afterwards refuses a grab until it lifts. Held keys are kept per keyboard, and a keyboard that is
  removed drops the keys it held, so the removal of a device with a key down no longer refuses grabs. The refusal
  for a key that is really down is unchanged.
- A playing tween held a configuration reload back for 15 seconds, and the reload then failed. A reload now waits
  for every other pending operation but not for a tween. The compositor stops the tween when the new configuration
  takes effect, the `Operation` of `play()` succeeds with the outcome `"cancelled"`, and the `"closed"` signal of
  the `Tween` runs with the reason `"config_reloaded"`. A reload that fails leaves the tween playing.
- `gnoblin.workspaces.create`, `next`, `previous` and `Workspace:activate` and `rename` resolved to a plain record
  with a `windows` key. They resolve to the live `Workspace` that `gnoblin.workspaces.by_id` returns, with
  `window_count`.
- A `gnoblin.configure` call that the validator rejected inside an `Operation` callback failed its `Operation` and
  also logged `deferred Lua operation callback failed`. It fails the `Operation` only.
- `gnoblinctl config reload` accepted `input.keyboard.xkb_options = {"caps:escape"}` in the first
  load but refused it on reload with `input.keyboard.xkb-options: invalid value`, because the
  second normalisation of the input document read only the unnormalised string list. The same code
  is on `main`. A reload now keeps the option.
- The portal backend now destroys and releases each ScreenCast consent dialog
  and its hidden parent window when the session ends. Before, the dialog's
  widget tree stayed alive after every screen cast.
- The Lua API reference now describes `gnoblin.windows.focused`, `windows.by_id`,
  `windows.snap_context`, `gnoblin.outputs.*`, and `gnoblin.capabilities.list`.
  These functions already worked, but editor hints and the generated
  `gnoblin.d.lua` did not list them.
- The Lua API reference no longer lists `Window:match`. The runtime never installed it; the
  window-rule fields stay available from `gnoblinctl window match`.
- A callback added with `Operation:on_complete` after the operation had finished now runs.
  Before, the runtime called the Operation table instead of the callback and logged an error.
- A shell process that called `prompt.respond`, `prompt.cancel`, `auth.*` or
  `location.authorize_app` could answer a request that it never received. The call reached
  the compositor without the client's identity, so the recipient check did not run. The
  compositor now checks it.
- The `monitor.list` bridge method failed with "Lua monitor record has no integer index". The
  reply now lists each monitor again.

### Removed

Gnoblin no longer manages app permissions or persistent portal grants. Bingux
owns permission rules and enforcement.

| Surface      | Removed                                                                                    |
| ------------ | ------------------------------------------------------------------------------------------ |
| Lua          | `gnoblin.permission_rule`, `gnoblin.configure.permissions`, `gnoblin.portals.grants`       |
| `gnoblinctl` | `permissions` and `grant` commands                                                         |
| Events       | `gnoblin.permission.changed`, `gnoblin.portal.grant-added`, `gnoblin.portal.grant-removed` |
| D-Bus        | `CheckPermission`                                                                          |

Gnoblin still routes portal interfaces to the selected backend. The default GTK
backend keeps its normal consent dialogs.

`gnoblin.commands.run` and `gnoblin.commands.capture` are removed; use `gnoblin.process.spawn`. Calling a removed
function is a Lua error, so a config that still calls one fails to load.

| Before                                             | After                                                                    |
| -------------------------------------------------- | ------------------------------------------------------------------------ |
| `gnoblin.commands.run(argv)`                       | `gnoblin.process.spawn(argv)`                                            |
| `gnoblin.commands.capture {argv = argv}`           | `gnoblin.process.spawn(argv, {stdout = "capture"}):wait()`               |
| `capture` option `stdin = text`                    | `stdin = text`, now any bytes up to 1 MiB                                |
| `capture` option `timeout = seconds`               | `timeout = seconds`; the result then has `timed_out = true`              |
| result `exit_code` below 0 after a signal          | result `signal`, such as `"term"`; `exit_code` is `nil`                  |
| failure past 64 KiB of output, or non-UTF-8 output | `output_limit` chooses the size; `stdout_truncated`; output is raw bytes |

`gnoblin.prompt.respond`, `gnoblin.prompt.cancel`, `gnoblin.auth.begin`, `gnoblin.auth.respond`,
`gnoblin.auth.cancel` and `gnoblin.location.authorize_app` are removed from Lua; answer through
`event.request`. Shell processes keep these methods on the compositor bridge.

| Before                                                                         | After                           |
| ------------------------------------------------------------------------------ | ------------------------------- |
| `gnoblin.prompt.respond {request_id = id, password = p}`                       | `event.request:answer(p)`       |
| `gnoblin.prompt.respond {request_id = id, confirmed = true}`                   | `event.request:answer(true)`    |
| `gnoblin.prompt.cancel {request_id = id}`                                      | `event.request:cancel()`        |
| `gnoblin.auth.begin {request_id = id, identity = n}`                           | `event.request:claim(n)`        |
| `gnoblin.auth.respond {request_id = id, response = p}`                         | `request:answer(p)`             |
| `gnoblin.auth.cancel {request_id = id}`                                        | `request:cancel()`              |
| `gnoblin.location.authorize_app {request_id = id, allow = true, accuracy = a}` | `event.request:answer(true, a)` |
| `gnoblin.on("gnoblin.prompt.finished", ...)`                                   | `event.request:finished()`      |

Lua functions that took one table of named fields now take positional arguments, with an
optional `options` table last. The same applies to `gnoblinctl lua`. Socket clients keep
sending named fields.

| Before                                                                | After                                                       |
| --------------------------------------------------------------------- | ----------------------------------------------------------- |
| `workspaces.create {name = n, id = i, activate = a}`                  | `workspaces.create(n, {id = i, activate = a})`              |
| `workspaces.rename {number = 2, name = n}`                            | `workspace:rename(n)` on a `Workspace` object               |
| `workspaces.activate {number = 2}` and `remove`                       | `workspace:activate()` and `workspace:remove()`             |
| `workspaces.move_active {workspace = {number = 2}, follow = f}`       | `window:move_to_workspace(2, {follow = f})`                 |
| `workspaces.move_window {window = w, workspace = s, follow = f}`      | `w:move_to_workspace(s, {follow = f})`                      |
| `animations.inspect {name = n, target = t}` and `preview`             | `animations.inspect(n, {target = t})`                       |
| `animations.seek {session = s, progress = p}`                         | `preview:seek(p)` on an `AnimationPreview`; also `step(ms)` |
| `animations.play {session = s}`, `pause`, `stop`                      | `preview:play()`, `pause()` and `stop()`                    |
| `input.select_source {type = t, id = i}`                              | `input.select_source(t, i)`                                 |
| `launches.begin {token = t, application = a, timeout_ms = m}`         | `launches.begin(t, a, {timeout_ms = m})`                    |
| `shortcuts.bind {id = i, accelerator = k, hold = h}`                  | `shortcuts.bind(i, k, {hold = h})`                          |
| `shortcuts.unbind {id = i}`, `end_session {id = i, session_id = s}`   | `shortcuts.unbind(i)`, `end_session(i, s)`                  |
| `window:move {x = x, y = y}`, `resize`, `thumbnail`                   | `window:move(x, y)`, `resize(w, h)`, `thumbnail(w, h)`      |
| `window:move_to_workspace({number = 2})`, `move_to_monitor({id = m})` | `window:move_to_workspace(2)`, `move_to_monitor(m)`         |

Duplicate Lua names are removed:

| Removed                                      | Use instead            |
| -------------------------------------------- | ---------------------- |
| `gnoblin.monitor.list`                       | `gnoblin.outputs.list` |
| `layer.list`                                 | `layers.list`          |
| `input.list`                                 | `input.sources`        |
| `input.current`                              | `input.current_source` |
| `launches.snapshot`, `gnoblin.launch.status` | `launches.list`        |
| `targetType`                                 | `target_type`          |

Shell processes keep the bridge methods `monitor.list`, `layer.list` and `launch.*`.

A second pass removes every Lua function that had another name for the same action:

| Removed                                                  | Use instead                                                     |
| -------------------------------------------------------- | --------------------------------------------------------------- |
| `gnoblin.events.on`, `gnoblin.events.once`               | `gnoblin.on`, `gnoblin.once`                                    |
| `gnoblin.events.mutter.on`, `gnoblin.events.mutter.once` | `gnoblin.on`, `gnoblin.once` with a `mutter.` event name        |
| `workspace:move_here(w)`                                 | `w:move_to_workspace(workspace)`                                |
| `window:minimize()`, `unminimize()`, `toggle_minimize()` | `window:set_minimized(enabled)`                                 |
| `window:restore()`                                       | `window:set_minimized(false)` and `window:set_maximized(false)` |
| `gnoblin.launches.snapshot()`                            | `gnoblin.launches.list()`; each `Launch` has its own `revision` |
| `gnoblin.monitors.list()`, `gnoblin.monitors.primary()`  | `gnoblin.outputs.list()`, `gnoblin.outputs.primary()`           |
| `gnoblin.monitors.privacy_screen()`                      | `output.privacy_screen`                                         |
| `gnoblin.monitors.set_privacy_screen(value)`             | `gnoblin.settings.override("monitors.privacy_screen", value)`   |
| `gnoblin.outputs.set_privacy_screen(value)`              | `gnoblin.settings.override("monitors.privacy_screen", value)`   |
| `gnoblin.input.set_orientation_lock(value)`              | `gnoblin.settings.override("input.orientation_lock", value)`    |
| `gnoblin.session.activity()`                             | `gnoblin.idle.watch(seconds)`                                   |
| Lua `gnoblin.monitor.*` events                           | `gnoblin.output.added`, `changed`, `removed`                    |

`window:restore_or_minimize()` stays: it restores the frame saved before a snap, which no other
call does. The bridge keeps every wire method name.

### Migration

1. Remove the `permissions` section, `gnoblin.permission_rule` calls, and
   `gnoblin.portals.grants` calls from your Lua config. A `permissions`
   section is ignored and the log names it as an unknown section. A call to a
   removed function is a Lua error and the config fails to load.
2. Write permission rules in the portal backend you use. Bingux reads its own
   rules.
3. Stop subscribing to the removed events and calling the removed commands.
4. Replace `gnoblin.commands.run` and `gnoblin.commands.capture` with
   `gnoblin.process.spawn` as the table above shows.
5. Answer prompts, authentication and location requests through `event.request`, as the
   table above shows.
6. Rewrite table calls to the positional forms above, and replace the removed duplicate
   names.

### Changed

`org.gnoblin.Shell.GlobalShortcutsProvider` `BindShortcuts` and
`ConfigureShortcuts`, and `org.gnoblin.Shell.GlobalShortcutsRebind`
`RebindShortcuts`, take a trailing request handle (`o`), the caller's
`org.freedesktop.portal.Request` object path. The shell uses it, with the
application ID, to identify the requester. Calls with the old argument lists
are not supported. Deploy the portal backend and the shell provider together.
