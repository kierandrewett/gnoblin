# Capture API

Status: design. Phase 1 (still images) is the only phase to build. Phase 2 (streams) is design only and is not enabled for any role.

## Purpose

Gnoblin lets trusted parts of the session take still pictures of windows, outputs and regions.

Today the portal role can take an output picture through `capture.screen`, which uses the `CaptureScreen` capability. That capability was granted for portal screenshots after user consent. This document adds a separate path for trusted callers. It sets who may capture, what they may capture, and which checks run first.

Out of scope: captures by extensions, captures by ordinary clients, streams, and any change to portal screen sharing.

## Targets

Each `capture.still` request names exactly one target.

- Window: the pixels in the window's own buffer, cropped to its content rectangle. Parts covered by other windows are included, because the client drew them. Popups are excluded. A window with no drawn buffer (unmapped, or minimised) is refused as `window-not-drawn`.
- Output: the composed picture of one output, as the backend screenshot returns it.
- Region: a rectangle of one output, cropped from the output picture.

The compositor never writes files. Phase 1 returns the PNG bytes in the reply. The client decides where the file goes.

## Authorization and consent contract

### Capability

- New capability `capture-window` (`Capability::CaptureWindow`). It is the handshake name.
- Granted to the `runtime` role only (`grants("runtime")`).
- Not granted to: `services`, `portal`, `input`, any extension, or any ordinary client.
- Not granted to the session owner's ordinary-socket connections. `owner_grants` does not add it. A same-uid program that reaches the ordinary socket never gets `capture-window`.
- The portal keeps `CaptureScreen`. `CaptureScreen` does not authorize `capture.still`. `capture-window` does not authorize `capture.screen`. No window target reuses `CaptureScreen`.

### Who may call `capture.still`

A connection may call `capture.still` only if it holds `capture-window`. Any other connection is refused `denied` before the graph sees the call.

A user shell that needs a capture asks the runtime. It does not hold the capability itself.

### What may be captured

- Windows of other clients: only through a `capture-window` connection.
- Outputs and regions: only through a `capture-window` connection.
- No request may name "all windows" or "all outputs". Each request names one target.

### Consent

| Target kind                                       | User prompt                                                             | Audit record            | Privacy indicator                                               |
| ------------------------------------------------- | ----------------------------------------------------------------------- | ----------------------- | --------------------------------------------------------------- |
| Window, output, region (still), caller is runtime | Not required                                                            | Required, every capture | Required, signal before pixels are read (see Privacy indicator) |
| Any still, caller is not runtime                  | Not applicable: no such caller exists. Any future caller needs a prompt | Required                | Required                                                        |
| Stream (phase 2)                                  | Required, each stream                                                   | Required                | Required for the stream's lifetime                              |

Why no prompt for the runtime: the runtime is the trusted process that evaluates the user's configuration. Nothing outside the session can reach its connection. The audit record and the indicator keep the capture visible.

### Audit record

- Phase 1: the compositor writes one structured log line for each `capture.still` call. The line holds the caller role, the target kind, the target id, the size, and the outcome (`ok` or the refusal code).
- The full audit belongs to the request interceptor. Unverified: the request-intercept design is not in this tree yet, so the log line is the only record until it lands.

## Locked session refuses

- While the session lock is up, or the lock embargoes capture, `capture.still` refuses every target kind with `locked`.
- The check runs before any pixel is read. It uses the same observation snapshot as `capture.screen` (`still::choose_output`).
- A window target never returns lock-cover pixels. While locked, no window target is readable.
- A View capture (`gnoblin_view_source_manager_v1`, ext-image-copy-capture) is embargoed by the same lock: a granted View stream is refused while the lock is up, and a running one stops and does not resume.

## Secure treatment

- The `capture.screen` handler comment says the backend screenshot applies the configured secure treatment to secure layers. Unverified: the backend code that does this was not read in this design pass.
- No per-window secure flag was found in the code. The layer design (`design/layers.md`) has a `secure` field, but it applies to layers, not to windows.
- Window stills read the client buffer directly. The output masking path does not run for them. So a window still of secure content is not masked today.
- Required rule: secure content never reaches a `capture.still` reply. Phase 1 must not ship window stills until the Needs Kieran question on secure windows is answered.

### Magnifier

- The magnifier lives in `crates/gnoblin-a11y`. Its zoom is a view effect.
- Captures show the unzoomed desktop. Window stills read client buffers, so the zoom does not touch them.
- Unverified: whether the output screenshot path includes the magnifier zoom. If it does, the output path needs a fix before phase 1 ships.

## Privacy indicator

- The design names `org.gnoblin.Privacy` as the privacy surface (`design/e2e-privacy.md`). Its payload names no app (`design/e2e-privacy.md`, the note on indicator payloads).
- Required rule: while a capture runs, the indicator shows active capture.
- Unproven: a still is one instant, so the signal must come before the pixels are read. Unverified: whether `org.gnoblin.Privacy` reports still captures at all. Today its monitor covers PipeWire nodes.
- Phase 1 cannot claim the indicator works until the active-still signal is shown in a test.

## Streams (phase 2, design only)

Streams are not enabled for any role. Nothing in this section is built.

Before a stream may start, all of these must be designed and agreed:

- Consent: a user prompt for each stream, because a stream runs over time.
- Capability: either `capture-window` with a stream member, or a new capability. The portal ScreenCast path (`crates/gnoblin-pipewire-export`) uses the consent grant of that path. It must not be reused for `capture-window`.
- Indicator: `org.gnoblin.Privacy` stays active for the stream's whole lifetime.
- Lock: the lock stops every stream, as the export already does for ScreenCast sessions.
- Node ids: each node id belongs to the connection that opened it.

## Naming a View as a capture source: the options (findings, nothing implemented)

`gnoblin_view_source_manager_v1` is 97 lines of server code and one XML. Its one request, `create_source(node)`, makes an
`ext_image_capture_source_v1` for a View node, and everything after that (the lock embargo, damage, the privacy
indicator, the secure-content mask, the size follow) is `capture_shared` working on `Target::View`. The question is only
how a client names the View. Four ways to keep it standard, read from the code at `c32194972`:

- **A. Publish each View as an `ext_foreign_toplevel_handle_v1`.** The source comes from the standard
  `ext_foreign_toplevel_image_capture_source_manager_v1`, and the handle's identifier names the View.
- **B. Keep a source manager** whose `create_source` takes a node id, as now. `ext-image-capture-source-v1` says it "may be
  extended to support more image capture sources", and a source manager that creates `ext_image_capture_source_v1` is that
  extension point, the same one the output and toplevel managers use.
- **D. Expose each View as a virtual `wl_output`** and capture it with `ext_output_image_capture_source_manager_v1`.
- **E. Keep standard capture and change only how the XR extension gets its source:** the screen-cast members
  (`cast.grant` and `cast.start` with a View) and a PipeWire stream, with no Wayland capture protocol in the extension.

What A costs, each checked against the code:

1. **The list cannot be filtered per client inside one global.** Smithay's `ForeignToplevelListState` announces every
   handle to every list instance and replays every live handle to a new one (`new_toplevel_with_identifier`, `bind`); its
   only filter is whether a client may bind the global. The protocol text says the toplevel event is "emitted for all
   toplevels". So "Views only to the `view` role, windows only to everyone else" is two globals with one interface name
   (two `ForeignToplevelListState`s with opposite `can_view` filters), not a filter on one list. That is a role-specific
   audience for a standard interface: the same kind of per-client special case the nested-session rule removes, and a
   registry that shows two `ext_foreign_toplevel_list_v1` entries to a debugging tool.
2. **The role gate is by interface name and cannot tell the two lists apart.** `VIEW_INTERFACES` and `sees_interface` work
   on interface names. The `view` role would need `ext_foreign_toplevel_list_v1` and
   `ext_foreign_toplevel_image_capture_source_manager_v1` in the list, and the name does not say which of the two lists it
   may bind; only each global's own `can_view` does. A new `Global` variant for the Views list replaces `Global::ViewSource`,
   so the deletion removes no role or global from `access.rs`, it renames one.
3. **The identifier becomes a private contract.** XR must build or parse an identifier such as `gnoblin-view-<node>`
   to ask for the View whose node id it already holds from the tree. The alternative is to match `title` or `app_id`, which are
   not unique. Either way a convention outside the standard says which handle is which node, which is what `create_source(node)`
   states in one typed argument.
4. **Identifier uniqueness is stricter than a node id.** The list says an identifier "must not be reused" while the toplevel
   is unmapped. Node ids are chosen by the tree's writer and `Scene::insert` refuses only a live duplicate, so an id can come
   back after a node is destroyed [INFERENCE: not shown to happen in practice]. A window identifier is derived from a window
   id (`gnoblin-<window id>`, `ext_foreign_toplevel_list_v1.rs`), and the session counts window ids up from one (`next_window`
   in `State`) [INFERENCE: nothing else mints one]. A View identifier would need a
   publication counter as well, and `gnoblin-view-` plus 16 hex digits plus a counter does not fit the 32 characters.
5. **A new publish and retire path.** Windows are published from the window record on every `publish`
   (`sync_foreign_toplevels`). A View appears and goes as its node, its source and its size change (`views()` reports each View
   with a size), so it needs its own diff, `closed` on removal, and `title` and `app_id` that a View node may not have. On
   destruction the handle closes and the source stops, which `capture_shared` already does for `Target::View` (`target_closed`).
6. **What other list clients see.** With one global, every taskbar, switcher, portal picker and accessibility tool that reads
   the standard list would show Views as windows, and the wlr list (fed from the same record) would have to leave them out
   again. With two globals they see nothing, which is item 1.

What is the same either way: `capture_shared` resolves a source to `Target::View(node)` in one line (a second branch
beside `window_of_identifier` for A, the existing `create_source` for B), and the lock embargo, damage, privacy indicator and
secure-content mask apply to the target and not to how it was named. A changes none of them, so it does not add safety.

## D and E, judged the same way

**D, a View as a virtual output.** It fails on the first check. Outputs come from the backend (`BackendEvent::OutputConnected`,
`gnoblin-backend/src/lib.rs:721`), and only the headless backend can add one at run time, through a handle meant for tests
(`headless.rs:435`, `connect_output`); the nested and DRM backends cannot. The compositor's own "outputs with no place of
their own" show another output's picture, not a node (`gnoblin-wayland/src/output_policy.rs:83-89`), so a View output is a new
kind of output rendered from a scene node. Then every consumer of outputs has to learn it or exclude it: every output must
present a frame for a lock to be proven (`report_outputs`, `ext_session_lock_v1.rs:563`, `output_ids`, `state.rs:1072`);
`wl_output` is not privileged, so every application would list an extra monitor and the shell would lay windows out on it;
`zwlr_output_management_v1` (`Global::OutputManagement`, `zwlr_output_management_v1.rs:320`) would list it and let a user
disable or move it. Hiding it from them is per-client output visibility, the special case the nested-session rule forbids, and
the `view` role would need `wl_output`, which `VIEW_INTERFACES` hides today. A View also has a camera, a stereo rig and a size
with no mode or scale, so the mapping is by analogy only. D is worse than A on every point above and removes nothing.

**E, the screen-cast path.** Real, and narrower than it sounds. What works today, by consumer:

| Consumer of a View's pixels                              | State                                                                                                                                                                                                                                                                                                                                        |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The shell's `View:capture(path)`                         | Works; reads the capture picture, tested in `view_capture_e2e.rs`.                                                                                                                                                                                                                                                                           |
| The XR extension, through `gnoblin_view_source_v1`       | Works, with damage and the shared-memory and GPU buffer paths; tested in `view_capture_e2e.rs` and in `xr-capture`. This is what A or B would keep.                                                                                                                                                                                          |
| A PipeWire stream of a View (`cast.grant`, `cast.start`) | The compositor half works: `ipc/consent.rs:13-14,223-330`, tested with a fake portal in `crates/gnoblin-compositor/tests/cast.rs:165-250` (`no-view`, no consent, the lock) and with a private PipeWire daemon in `view_capture_e2e.rs`. The stream is memfd only; a DMA-BUF stream buffer is "not done" (`design/cutover-survey.md`, 16.7). |
| A real portal frontend offering a View to an app         | Planned, not wired: the same survey calls the portal ScreenCast path "not wired to the portal (16.2)".                                                                                                                                                                                                                                       |

The extension would need to link PipeWire, which `design/xr-extension.md:45` says it does not have and `:170` says is for audio.
Its connection is already an owner connection (`gnoblin-xr/src/main.rs:77`; every same-user helper is the owner,
`ipc/privileged.rs:10-20`), so it could call `cast.grant` for itself. Then the consent step records the owner's own consent
to itself, and a real portal prompt per View is not workable for a headset desktop. It also trades the Wayland copy's
damage-driven frames and buffer-constraint negotiation for a stream paced by PipeWire, from a path that has no GPU buffer yet. E
removes the private Wayland protocol and does nothing about naming, because the View is still named by node id, now over the
IPC; it deletes nothing in `capture_shared`. It is a possible end state once the stream has DMA-BUF buffers and the extension
accepts PipeWire, not a way to retire the protocol now.

Findings: A is expressible and nothing in the capture path blocks it, but it trades one small, typed, owner-visible source
manager for a second list global, a naming convention, a lifetime coupling to the list's no-reuse rule and a second
publishing path. D is worse. E works on the compositor side but needs the extension to take PipeWire and gives up damage and
GPU buffers for now. B keeps the one private XML, and nothing else private. **B is the less hacky choice**, and it is the
extension the capture-source protocol is built for. The refactor that remains under B is to narrow the manager, not to remove
it: keep `create_source(node)`, keep it in `VIEW_INTERFACES`, keep the view role, and move its XML under the protocol crate
layout. This is waiting on the owner's choice; no Part B code, test or XR change was made.

## XR in any shell: standard window capture (the owner's option E; findings, nothing implemented)

The screen-cast variant judged above is a different route that shares the letter; this one is the standard toplevel path.

The requirement: XR works in any shell by copying what is already rendered, with the window's frame as drawn, and places each
copy from the tree. The route: list real windows with `ext_foreign_toplevel_list_v1`, copy each with
`ext_foreign_toplevel_image_capture_source_manager_v1` and `ext_image_copy_capture_v1`, take placement from the tree.

1. **Does a window capture include the shell-built frame? Not as built.** `Target::Window` is a crop of the output frame:
   `capture_targets` takes the window's origin and size from `geometry_of`/the managed geometry
   (`capture_shared.rs:919-949`), and `copy` reads that rectangle out of the output's composed frame (`capture_layout`,
   `:725-731`). Two consequences. Anything drawn over the window (another window, a panel, a popup of another client) is in
   the picture, so a window that is partly covered on the monitor is partly covered in the headset. And a decoration child outside
   that rectangle is not in it: `design/node-core.md:601` defines a window's whole frame as its rectangle united with its decoration
   children (a titlebar), and `:645` says the titlebar is the window node's child, so the rectangle and the frame differ.
   `node-core.md:135` already promises that a window capture includes the window's popups and subsurfaces, which a crop only does
   when nothing covers them. The smallest change that makes the standard capture right is to give `Target::Window` the path
   `Target::View` has: draw the window node's subtree on its own (`draw_view`, `compose/views.rs:376`; `window_node`,
   `compose.rs:2103`), publish that picture beside the View pictures, and read it in `bounds`, `copy` and `copy_into`
   (`capture_shared.rs:545`, `:599`). The picture is the union of the node and its decoration children, unoccluded, with the
   secure mask the View path already applies. The cost is one subtree redraw per captured window and frame; the
   View path costs 17 ms at 720p on the CPU, and a GLES readback otherwise (`design/cutover-survey.md`, 16.7).
2. **Identifier to tree node: no mapping exists.** The list publishes `gnoblin-<WindowId, 16 hex digits>`
   (`ext_foreign_toplevel_list_v1.rs`). The tree's `window` class has `title`, `app_id`, `focused` and the state flags and nothing
   that names the window (`schema/tree/window.toml`), and a window node's id is a different number from the `WindowId`
   (`window_node`). XR can only match on `title` and `app_id`, which are not unique. Smallest fix with no special case: one
   read-only `identifier` text property on the window node, set by the compositor to the string the list publishes. Any
   observer then maps a handle to its node by equality. Window node ids are never reused in a session
   (`gnoblin-render/src/scene_ids.rs`, `a_window_node_id_is_never_reused_within_a_session`), so the identifier could instead be
   derived from the node id, which also meets the list's no-reuse rule by construction.
3. **Embargo, secure content and the indicator.** Unchanged, because a window source already goes through the one session
   table: a granted window stream is fed the lock scene and stops at the lock (`gnoblin-capture/src/lib.rs:113`), a window marked
   secure is masked (`:199`, `a_window_the_shell_marks_secure_is_masked_in_its_capture`), and the privacy indicator follows the
   open session. One change in access: the `view` role cannot bind the list or the toplevel source manager (`VIEW_INTERFACES`
   in `access.rs`), so XR would need them added, and then it can read every window, which is the point.
4. **What it cannot cover.** Layer-shell surfaces, menus, panels and canvases that are shell nodes are not toplevels, so
   they are not in the list and have no source. Output capture cropped by the tree's geometry gets everything flattened
   (the wallpaper, the windows above it), so a panel under a window shows the window: occlusion is the problem, and a crop cannot
   remove it. The occlusion-free routes for a non-window node are a View of it (A or B above) or nothing.
5. **Does XR need Views today? Yes, as the vehicle for windows, and for stereo.** `docs/xr.md` says each window is a panel
   and `xr-capture` reads Views only; the stock shell makes a View per window. An observer may only create the `xr` seat, destroy
   it and send pointer input (`docs/xr.md`, "What the program gets"), so XR cannot make a View of a window itself, and in a shell
   that makes none it shows nothing. The stereo View through `head` (`design/xr-extension.md:27-37`) is a camera feature of
   a View and has no window equivalent.

Findings: E works for windows, in any shell, if the toplevel source is made to draw the window subtree and the window node
carries its `identifier`; both changes are small and neither adds a role-specific list or a private protocol. It does not
cover non-window nodes or stereo Views, so the View source manager stays until XR gives those up. The two paths then
coexist: windows by the standard route, Views by `gnoblin_view_source_v1`. Nothing was implemented.

## Needs Kieran

1. Secure windows: is a per-window secure flag wanted? Should window stills of secure content be refused, or masked?
2. Privacy indicator: should `org.gnoblin.Privacy` show still captures, or only streams?
3. Audit: may the phase 1 log line stand in for the request-intercept audit until that design lands?
4. Owner ordinary socket: confirm that same-uid programs never get `capture-window`. This document says they never do.
5. Phase 2: which party answers the stream prompt, and what does the prompt say?
