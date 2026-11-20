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
how a client names the View. Two ways to keep it standard, read from the code at `c32194972`:

- **A. Publish each View as an `ext_foreign_toplevel_handle_v1`.** The source comes from the standard
  `ext_foreign_toplevel_image_capture_source_manager_v1`, and the handle's identifier names the View.
- **B. Keep a source manager** whose `create_source` takes a node id, as now. `ext-image-capture-source-v1` says it "may be
  extended to support more image capture sources", and a source manager that creates `ext_image_capture_source_v1` is that
  extension point, the same one the output and toplevel managers use.

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

Findings: A is expressible and nothing in the capture path blocks it, but it trades one small, typed, owner-visible source
manager for a second list global, a naming convention, a lifetime coupling to the list's no-reuse rule and a second
publishing path. B keeps the one private XML, and nothing else private. **B is the less hacky choice**, and it is the
extension the capture-source protocol is built for. The refactor that remains under B is to narrow the manager, not to remove
it: keep `create_source(node)`, keep it in `VIEW_INTERFACES`, keep the view role, and move its XML under the protocol crate
layout. This is waiting on the owner's choice; no Part B code, test or XR change was made.

## Needs Kieran

1. Secure windows: is a per-window secure flag wanted? Should window stills of secure content be refused, or masked?
2. Privacy indicator: should `org.gnoblin.Privacy` show still captures, or only streams?
3. Audit: may the phase 1 log line stand in for the request-intercept audit until that design lands?
4. Owner ordinary socket: confirm that same-uid programs never get `capture-window`. This document says they never do.
5. Phase 2: which party answers the stream prompt, and what does the prompt say?
