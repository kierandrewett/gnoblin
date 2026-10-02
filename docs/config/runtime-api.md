# Lua runtime API

Use `gnoblin` runtime methods from an event callback to inspect or change the
running session. Lua and `gnoblinctl` share method names and argument fields
where a method is exposed over the control socket. Some trusted interaction
methods are Lua-runtime-only. Operation-based methods return an `Operation`;
`gnoblinctl` waits for the matching reply and prints its result.

Lua callbacks run on the `gnoblin` supervisor's event loop, separate
from Mutter's compositor main thread. Keep callbacks nonblocking; operations
complete asynchronously.

`gnoblin.settings` returns an immutable snapshot of the active configuration
using public snake_case setting names. Read it as a property, for example
`gnoblin.settings.window_management.focus_mode`. It is available after the
initial configuration is committed.

The snapshot includes a `revision` that advances when committed configuration
values change. A no-op event or identical reload keeps the same revision. This
field belongs to the detached snapshot and is never written into the config.
`gnoblin.snapshot()` returns a detached copy of the config view; prefer
`gnoblin.settings` for immutable runtime reads.

`gnoblin.version()` returns an immutable `Version` record. It reads the installed
`share/gnoblin/version.ini` metadata file. Set `GNOBLIN_VERSION_METADATA_FILE`
to use another file. If that path is unset or unreadable, Gnoblin checks the
executable's installation prefix and then the system XDG data directories. A
missing or invalid file does not raise an error; fields without a source value
are returned as `"unknown"`.

The `api` field comes from the native-control API constants when available at
build time. The `lua` field reports the linked Lua runtime version. Remote URLs
have userinfo, query, and fragment parts removed before returning.

The record contains string fields `gnoblin`, `gnome`, `mutter`, `lua`, `api`,
`git_remote`, `git_sha`, and `build_id`. The generated `build_id` combines the
Gnoblin release version with the source commit; builds from modified source
trees add a `modified` suffix.

Native-control API 1.19 exposes these reads to local clients:

- `version` returns the version record.
- `capabilities.list` and `focus.history` return arrays.
- `settings` and `focus.policy` return committed settings snapshots.

Native-control API 1.37 adds four snapshot collection reads. They return JSON
arrays, including when a collection is empty. `layers.list` accepts the same
optional filters as `gnoblin.layers.list()`.

Native-control API 1.38 adds `window.restore_or_minimize`. It unmaximizes a
maximized window, restores its saved pre-snap frame, or minimizes it. Its result
contains the stable `id` and the action performed. The [bridge
reference](/compositor-bridge#api-138-restore-a-snapped-window) lists the
possible actions. Every supported socket client uses the shared Lua operation,
which requires the Lua supervisor.

Native-control API 1.39 adds `launches.snapshot`, which returns the launch
records and their collection revision together. It requires a connected Lua
supervisor and keeps the collection revision available when there are no
launch records.

Native-control API 1.40 adds `shortcuts.list`, a shared read backed by
`gnoblin.shortcuts.list()`. It returns the same shortcut records as a JSON
array and requires the Lua supervisor.

Native-control API 1.41 adds `shortcuts.actions`, a shared read backed by
`gnoblin.shortcuts.actions(group?)`. It returns built-in action metadata from
the installed GSettings schemas and requires the Lua supervisor.

Native-control API 1.42 adds the `permissions.list` Lua read. Every supported
socket client uses `gnoblin.permissions.list()`. It returns the committed
permission policy, capabilities, levels, and configuration path, and requires
the Lua supervisor.

Native-control API 1.43 adds the `permissions.check` Lua read. Every supported
socket client uses `gnoblin.permissions.check(args)`. It returns the decision
record and requires the Lua supervisor.

Native-control API 1.44 adds the `permissions.policy` Lua read. Every supported
socket client uses `gnoblin.permissions.policy()`. It returns the policy and
revision and requires the Lua supervisor.

Native-control API 1.23 adds `window.thumbnail`. API 1.24 adds
`session.activity` and its change event. API 1.26 adds pointer-drag lifecycle
events and capability-bound `window.snap.offer`.

API 1.27 adds two events for shell-owned presentation of compositor requests:
`gnoblin.window.menu-requested` and `gnoblin.osd.requested`.

Native-control API 1.28 adds direct socket methods for text insertion and
keyboard-selected window snapping. See the [compositor bridge](/compositor-bridge#api-128-text-insertion-and-keyboard-snapping)
for their connection-owned token contract.

Native-control API 1.29 adds `session.status` as a read available to Lua and
local socket clients.

Native-control API 1.30 adds one-use capabilities for actions from Mutter WM
menus.

Native-control API 1.32 adds `session.logout()` and lets a socket client focus
a window with a compositor-verified XDG Activation token. Lua focus operations
continue to require a trusted `FocusContext` from a shortcut event.

Native-control API 1.33 adds the `microphone-monitor` capability and the
`gnoblin.capability.changed` event for PipeWire availability transitions.

`gnoblin.session.status()` returns a record with these fields:

- `state` is `"running"` while the compositor answers the request.
- `lock_available` says whether Mutter can report its lock state.
- When `lock_available` is true, `lock_state` is `unlocked`, `covering`,
  `locked`, or `failsafe`.

When lock state is unavailable, the record omits `lock_state`; unavailable does
not mean unlocked.

The read cannot report why a stopped session exited. The socket is unavailable
after the compositor stops. Socket clients of every supported API version get
this record through the Lua supervisor; it reports compositor state, not
supervisor health. Subscribe to
`gnoblin.session.lock-state-changed` for lock transitions.

`gnoblin.session.logout()` takes no arguments and returns an operation whose
successful value is `{accepted = true}`. After the compositor confirms that
operation, the supervisor exits successfully and stops the compositor. The
session wrapper then stops Gnoblin's user services and returns to the login
manager. A socket caller may receive EOF while the session is closing.

`gnoblinctl lua` exposes this operation and `gnoblin.session.lock()` too. The
lock result reports that a subscribed shell client received the request; check
`session.status()` to confirm the compositor entered a locked state.

Send `{}` as `arguments` for reads without filters. Responses contain the
snapshot directly, not an operation handle. See the
[compositor bridge](/compositor-bridge#connect) for the wire format and version
requirements.

API 1.20 adds `runtime.reload_config()` with no arguments. In a standalone
native session, reload applies changes to these settings:

- `animations`
- `input`
- `permissions`
- `touchpad-gestures`
- `window-rules`
- `workspaces`

Other setting changes are rejected and leave the active runtime unchanged.
Start a new session to apply them.

Call `gnoblin.runtime.status()` to read worker health:

```lua
local status = gnoblin.runtime.status()
print(status.state, status.generation)
```

The method uses native-control API 1.67. It returns a read-only record with
`state` and `generation`. The state values are:

- `starting` before the worker connects;
- `running` while it serves requests;
- `restarting` while Mutter suspends it for replacement;
- `unavailable` when the supervisor is disconnected or stopping.

The compositor answers the read while the Lua worker is restarting, so shell
clients can use `gnoblinctl lua` to poll recovery. A call from the live Lua
runtime reports `running` and that worker's active generation.

`generation` identifies the runtime configuration accepted by Mutter. It
stays unchanged across worker recovery and changes when a different
configuration is accepted.

The native open-animation matcher uses updated rules for windows mapped after
reload. Reload does not replay open animations for windows already mapped.

The Lua `Operation` completes after the candidate config has loaded, validated,
and been staged. Its `runtime_generation` is the generation assigned if the
candidate commits; it does not confirm commit. Mutter waits for active-runtime
operations and deferred callbacks before swapping runtimes.

`gnoblinctl lua` also exposes `gnoblin.runtime.reload_config()`. It waits for
the compositor operation and returns its result as a deeply read-only value.

`gnoblin.config.reloaded` signals that the candidate became active. If the
session stops first, the candidate is discarded and that event is not sent, even
if the Lua operation already reported successful staging.

After the commit, the runtime also emits `gnoblin.focus.policy-changed` and
`gnoblin.permission.changed` when those effective policies changed. These
events carry the committed policy snapshot and configuration revision. A
failed reload emits `gnoblin.config.reload-failed` while the previous runtime
remains active.

`gnoblin.session.lock()` asks a subscribed external shell client to show its
lock UI. The client owns that UI and must use Mutter's lock protocol; this Lua
method does not lock the session by itself. It takes no arguments and returns
an `Operation<LockRequest>`.

On success, `dispatched` is `true`. `subscribers` counts connected clients
subscribed when Gnoblin targets the request. A slow connection may close before
it handles the event, so these fields do not confirm that the client showed its
UI or that the session locked. The operation fails when compositor locking is
unavailable or no client is subscribed. No Lua unlock method is provided.

The lock state is `unlocked`, `covering`, `locked`, or `failsafe`. Only
`locked` confirms the compositor's lock transition. See the
[compositor bridge](/compositor-bridge#api-version-121-session-locking)
for socket subscription and request details.

`gnoblin.session.activity()` returns the latest native idle-monitor sample.
Its fields are `available`, `idle`, `threshold_ms`, `idle_for_ms`, and
`revision`. Gnoblin uses a fixed threshold of 120 seconds. Idle inhibitors and
desktop idle-timeout preferences do not change it.

When the sample is available and idle, `idle_for_ms` advances from the last
sample using the supervisor's monotonic clock. If monitoring is unavailable,
`idle` is false and `idle_for_ms` is zero.

`gnoblin.session.activity-changed` is emitted when availability or idle state
changes. Its `idle_for_ms` is sampled at that transition and does not update
continuously. The event also carries `threshold_ms`, `revision`, `sequence`,
and monotonic-clock `time`.

Native sessions provide immediate reads through `gnoblin.windows`,
`gnoblin.workspaces`, `gnoblin.monitors`, `gnoblin.layers`,
`gnoblin.input.devices()`, and `gnoblin.capabilities`. See
[Lua events](/config/lua-events) for event names and callback behavior.

`gnoblin.focus.policy` returns a read-only snapshot of the supported focus
settings. Its `revision` identifies the committed settings snapshot.

The snapshot includes:

- `focus_mode` and `focus_new_windows`.
- `raise_on_click`, `auto_raise`, and `focus_change_on_pointer_rest`.
- `auto_raise_delay` and `revision`.

`gnoblin.focus.policy-changed` runs after a successful config commit only when
one of these effective focus settings changes. Its `policy` field contains the
committed snapshot, and the event's `revision` matches `policy.revision`.
Rejected or failed config changes do not emit the event. Native-control API
1.13 socket clients can subscribe to the same event.

Use `gnoblin.focus.history(filter?)` to read native window snapshots in recent
focus order. Filters accept `workspace_id`, `monitor_id`, and `limit`. Workspace
and monitor IDs match exactly. The limit is 1–256 and defaults to 50; filters
may return fewer records.

History includes minimized windows and removes windows after they close. At
startup, only the focused window seeds the recent order. Other open windows
stay in snapshot order until the runtime observes their focus.

The settings property becomes available after the initial config load commits.
Native compositor snapshots are seeded after the initial Lua config
evaluation. They are unavailable while Gnoblin first loads the config file, so
call these read methods from a runtime event callback after native startup.
Calling one before the snapshot is ready raises a Lua error.

```lua
gnoblin.events.on("gnoblin.window.focused", function(event)
    local window = gnoblin.windows.by_id(event.window_id)
    if window then
        print("Focused window: " .. window.title)
    end
end)
```

Operation-based Lua API calls queue one operation, including legacy list
queries. The handle exposes the completion state and result:

| Member                  | Type                                      | Meaning                                                       |
| ----------------------- | ----------------------------------------- | ------------------------------------------------------------- |
| `id`                    | Positive integer                          | Request ID for this session.                                  |
| `method`                | String                                    | Canonical API method name.                                    |
| `status`                | `"pending"`, `"succeeded"`, or `"failed"` | Current operation state.                                      |
| `value`                 | Result or `nil`                           | Completed value on success.                                   |
| `error`                 | `Error` or `nil`                          | Failure detail with a stable code and human-readable message. |
| `on_complete(callback)` | Function to `Subscription`                | Call once with `(value, error)`.                              |

If the operation has already completed, Gnoblin schedules the callback for the
next main-loop turn. You can unsubscribe before it runs.

Callbacks registered while an operation is pending run when its completion is
dispatched. A callback can queue more operations; Gnoblin applies them after it
returns through the normal configuration and operation path. Gnoblin logs
callback errors and continues with other completion callbacks.

Native API 1.11 adds `gnoblin.operation.completed` for completed operations.
Its payload contains an operation ID, method, and success flag. Success has a
`value`; failure has an `Error` record with a stable code and human-readable
message.

Native `Error.code` values are:

| Code               | Meaning                                                          |
| ------------------ | ---------------------------------------------------------------- |
| `invalid_argument` | An operation argument or event-produced config value is invalid. |
| `not_found`        | The requested object or resource no longer exists.               |
| `unsupported`      | The current compositor or session cannot perform the operation.  |
| `denied`           | Session policy or security state rejects the operation.          |
| `busy`             | An exclusive input operation is already active.                  |
| `cancelled`        | The user cancelled the operation.                                |
| `timed_out`        | The operation reached its deadline.                              |
| `unavailable`      | The compositor or session service is not ready.                  |
| `internal`         | An unexpected implementation error occurred.                     |

Invalid arguments and calls made outside a runtime event callback fail before
an operation is queued. A queued request can still fail when Mutter or the
session handler applies it. If the config produced during that event fails
validation, Gnoblin rejects the event's queued operations and completes their
handles with the validation error.

## Workspaces

Use `{id = "code"}` to select a stable ID or `{number = 2}` to select the
current one-based position from 1 to 1024. A selector must contain exactly one
of these fields.

IDs declared in `gnoblin.configure.workspaces` persist across sessions. IDs
supplied to runtime `create` are temporary and last only for the session.
Gnoblin-generated `@session-N` IDs can be selected during that session but
should not be saved in configuration.
See the [workspace configuration reference](/config/configure/window_management)
and the [writing workspace recipe](/recipes/writing-workspace).

| Method                         | Arguments                                                          | Successful result             |
| ------------------------------ | ------------------------------------------------------------------ | ----------------------------- |
| `workspaces.create(args)`      | `name` required; optional `id`, `activate`                         | New `Workspace` record        |
| `workspaces.rename(args)`      | Exactly one of `id` or `number`, plus `name`                       | Updated `Workspace` record    |
| `workspaces.remove(args)`      | Exactly one of `id` or `number`                                    | Removed workspace record      |
| `workspaces.activate(args)`    | Exactly one of `id` or `number`                                    | Activated `Workspace` record  |
| `workspaces.next()`            | None                                                               | Activated `Workspace` record  |
| `workspaces.previous()`        | None                                                               | Activated `Workspace` record  |
| `workspaces.move_active(args)` | `workspace` selector; optional `follow`                            | `{workspace, window, follow}` |
| `workspaces.move_window(args)` | `window` ID or `"active"`; `workspace` selector; optional `follow` | `{workspace, window, follow}` |

`workspaces.list()` returns the current workspaces as an immediate,
immutable snapshot. Mutations return operation handles and complete
asynchronously. Workspace reads and mutations all use the `workspaces`
namespace in Lua configuration. The singular `workspace.*` names are reserved
for compositor-socket operation identifiers.

`create` requires a nonempty name of up to 80 characters. Its optional ID must
match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`; if omitted, Gnoblin generates a
session-only ID. `activate` defaults to `false`. Rename also requires a
nonempty name of up to 80 characters. `follow` defaults to `false`; when true,
moving a window also switches to the destination workspace.

A `Workspace` record has `id`, `number`, `name`, `active`, `windows`, and
`persistent` fields. Gnoblin does not remove a declared workspace, the active
workspace, or a workspace that still contains windows.

## Windows, layers, and monitors

### Immediate window snapshots

The native Mutter runtime exposes the latest window state directly:

| Lua call                        | Arguments                                                                   | Result                                |
| ------------------------------- | --------------------------------------------------------------------------- | ------------------------------------- |
| `gnoblin.windows.list(filter?)` | Optional `app_id`, `title`, `focused`, `workspace_id`, `monitor_id` filters | Read-only window snapshots            |
| `gnoblin.windows.focused()`     | None                                                                        | Read-only window snapshot or `nil`    |
| `gnoblin.windows.by_id(id)`     | Stable string window ID                                                     | Read-only window snapshot or `nil`    |
| `gnoblin.workspaces.list()`     | None                                                                        | Read-only workspace snapshots         |
| `gnoblin.workspaces.active()`   | None                                                                        | Read-only workspace snapshot or `nil` |
| `gnoblin.workspaces.by_id(id)`  | Stable string workspace ID                                                  | Read-only workspace snapshot or `nil` |

Window filter values:

- `title` takes a string and matches a case-insensitive substring.
- `focused` takes a boolean.
- `app_id`, `workspace_id`, and `monitor_id` take strings and match exactly.

Unknown filter names or values of the wrong type raise a Lua error. These reads
use a cached native snapshot and return immediately; they do not create an
`Operation`. Each record includes a state revision. Read the collection again
to get newer state.

Gnoblin refreshes both collections in one coalesced main-loop publication. A
read made before that publication completes returns the most recently published
state. Lifecycle callbacks run after the corresponding snapshots are refreshed,
so handlers can read the new state.

Subscribe to these window events to track changes:

- `gnoblin.window.created` and `gnoblin.window.closed`
- `gnoblin.window.changed` and `gnoblin.window.attention-changed`
- `gnoblin.window.focused` and `gnoblin.window.unfocused`

These reads are available in Gnoblin's standalone Mutter runtime. They raise a
Lua error when the snapshot is unavailable.

The native runtime exposes `gnoblin.monitors.list()` and
`gnoblin.monitors.primary()` as cached read-only snapshot methods. The primary
method returns a record or `nil`. Snapshots include active logical monitors;
cloned outputs use the first active connector alphabetically as the canonical
ID. Each record includes its state revision. Snapshots refresh before monitor
lifecycle callbacks run.

Subscribe to `gnoblin.monitor.added`, `gnoblin.monitor.changed`, and
`gnoblin.monitor.removed` to track output changes. The `changed` event's
`changed` array lists changed record properties: `id`, `index`, `x`, `y`,
`width`, `height`, `primary`, `scale`, `enabled`, `name`, `make`, `model`,
`serial`, `refresh_rate`, or `transform`. These immediate reads and events are
available only in the native Mutter runtime.

`gnoblin.layers.list(filter?)` returns read-only records from the latest native
layer-surface snapshot. Each record includes its state revision. Layer records
have no mutating methods; shell clients continue to own their surfaces.

The optional filter accepts exact string matches for `monitor_id`, `namespace`,
and `layer`. Unknown fields and non-string values raise a Lua error. The read
is available only in the native Mutter runtime and raises a Lua error while its
snapshot is unavailable.

The legacy `layer.list` socket method remains available. Every client version
serves it from this Lua snapshot and preserves its `{surfaces: [...]}` response. See the
[compositor bridge](/compositor-bridge#api-version-12-layer-surfaces) for the
socket request format.

Layer records expose these fields:

| Field                              | Type and values                                                                |
| ---------------------------------- | ------------------------------------------------------------------------------ |
| `id`                               | Stable string ID.                                                              |
| `title`, `namespace`, `monitor_id` | Optional strings.                                                              |
| `layer`                            | `"background"`, `"bottom"`, `"top"`, or `"overlay"`.                           |
| `keyboard_interactive`             | `"none"`, `"on_demand"`, or `"exclusive"`.                                     |
| `exclusive_zone`                   | Integer.                                                                       |
| `anchor`                           | Array containing zero or more of `"top"`, `"bottom"`, `"left"`, and `"right"`. |
| `geometry`                         | Rectangle.                                                                     |
| `mapped`                           | Boolean.                                                                       |
| `revision`                         | Integer state revision.                                                        |

Mutter currently omits `app_id` because layer-shell does not provide one.

`gnoblin.layers.animation_policy(namespace)` returns the effective enter and
exit phases and `window_shadow` for a layer namespace. The namespace must be 1
to 128 UTF-8 bytes. Without an animation override, both phases use `slide`.

`window_shadow` defaults to `false` unless a matching default-window rule
supplies a shadow value. The method raises a Lua error if the committed policy
cannot be read. Native-control API 1.31 also exposes this read to socket
clients as `layer.animation_policy` with a `namespace` argument.

`gnoblin.capabilities.list()` returns read-only native compositor and protocol
capability records. It takes no arguments and has no filters or mutators. The
read raises a Lua error if its snapshot is unavailable.

Capability records expose these fields:

- `id`: the capability name.
- `description`: what the capability provides.
- `available`: whether its current requirements are met.
- `revision`: the compositor state revision.
- `reason`: the cause when the capability is unavailable; omitted otherwise.

The standalone runtime advertises these optional capabilities:

- `window-thumbnails`: bounded window previews.
- `session-activity`: native idle-monitor state.
- `microphone-monitor`: PipeWire microphone activity monitoring.
- `camera-monitor`: PipeWire camera activity monitoring.

`microphone-monitor` and `camera-monitor` are available only when this Mutter
build includes remote-desktop support and the PipeWire monitor is connected.
When unavailable, `reason` is one of:

- `remote_desktop_disabled`: the Mutter build lacks remote-desktop support.
- `pipewire_unavailable`: the monitor cannot connect.

Camera activity follows running PipeWire nodes whose media role is `Camera`.
The activity remains available for 500 ms after the last camera node stops to
avoid flickering.

Subscribe to `gnoblin.capability.changed` through the Lua event API or API 1.33
on the compositor socket to receive the updated record when availability
changes. Check the capability snapshot before using an optional capability.

`gnoblin.input.devices()` returns read-only records from the latest native
input-device snapshot. It takes no arguments and is available in runtime event
callbacks after startup. The read raises a Lua error if its snapshot is
unavailable.

Input-device records expose these fields:

| Field                       | Type and values                                                 |
| --------------------------- | --------------------------------------------------------------- |
| `id`, `name`, `device_type` | Strings. The `input:N` ID remains stable for this session only. |
| `seat`                      | Optional string; absent when Mutter provides no seat name.      |
| `vendor_id`, `product_id`   | Optional integers.                                              |
| `capabilities`              | Array of capability names.                                      |
| `revision`                  | Integer state revision.                                         |

Subscribe to these events to track device changes in the native runtime:

- `gnoblin.input.device-added` contains `device`.
- `gnoblin.input.device-removed` contains `device_id` and the last device
  record in `last`.

`gnoblin.input.devices()` refreshes before either callback runs. Device IDs
last only for the current compositor session. Records do not expose device
paths or `enabled`.

The current Mutter backend does not expose a safe enabled-state getter, so
records omit `enabled`.

`gnoblin.input.sources()` returns configured XKB layouts and variants and
configured IBus engine IDs. XKB entries are checked against the installed XKB
registry. IBus records use the engine ID for both `name` and `short_name`.
Gnoblin does not need an IBus library to list or select them. The IBus service
must be running for selection; otherwise the operation fails with
`unavailable`.

The list comes from `input-sources.sources` when configured. Otherwise it uses
the desktop input-source setting. Mutter loads at most four layouts into one
keymap at a time. Gnoblin switches the active group of four when you select a
listed source outside that group.

`gnoblin.input.current_source()` returns the current configured XKB source or
the current configured IBus engine, or nil when the active source is unknown
or is not configured in Gnoblin.

`gnoblin.input.select_source({type = "xkb", id = "us"})` requests a listed XKB
source and returns an operation handle. The operation completes after Mutter
confirms the keymap change. Use `{type = "ibus", id = "engine-id"}` to select a
listed IBus source. Gnoblin calls `org.freedesktop.IBus.SetGlobalEngine` over
the session bus and completes when IBus accepts the request. XKB selection
changes the seat's keyboard layout; IBus selection changes the session's
global input method. Per-window source restoration is not provided.

Subscribe to `gnoblin.input.source-changed` for confirmed current-source
changes and `gnoblin.input.sources-changed` when the configured source list
changes. The current-source event has `available = false` and no `source` when
Mutter's current keymap is external or unknown.

`gnoblin.input.orientation_lock()` returns an immutable `OrientationLock`
record with boolean `available` and `locked`, string `orientation`, `source`,
and integer `revision`. `source` is `system` when the system setting applies,
`config` when `input.orientation_lock` supplies a boolean, and `runtime` while
a runtime override is active.

The `orientation` field is `normal`, `bottom-up`, `left-up`, `right-up`, or
`undefined`. The last value means no orientation is available.

Call one of these to request a change:

- `gnoblin.input.set_orientation_lock(true)` locks the current orientation.
- `gnoblin.input.set_orientation_lock(false)` unlocks orientation.
- `gnoblin.input.set_orientation_lock("inherit")` clears the runtime override.

Each call returns an asynchronous `Operation<OrientationLock>` and does not
write the config file. After `inherit`, Gnoblin restores the configured boolean
if one is set or follows the system setting otherwise.

On config reload, Gnoblin applies a boolean `input.orientation_lock` value.
When omitted or set to `"inherit"`, it clears the override and follows the
system setting. Subscribe to `gnoblin.input.orientation-lock-changed` to
receive the updated record, plus these event fields:

- `name` identifies the event.
- `sequence` orders events.
- `time` is monotonic.

Socket clients can use `input.orientation_lock` and
`input.set_orientation_lock` through native-control API 1.66; see the
[compositor bridge](/compositor-bridge#api-version-166-orientation-lock).

`device_type` is `"pointer"`, `"keyboard"`, `"extension"`, `"joystick"`,
`"tablet"`, `"touchpad"`, `"touchscreen"`, `"pen"`, `"eraser"`,
`"cursor"`, `"pad"`, or `"unknown"`. Capabilities are zero or more of
`"pointer"`, `"keyboard"`, `"touchpad"`, `"touch"`, `"tablet_tool"`,
`"tablet_pad"`, `"trackball"`, and `"trackpoint"`.

### Insert text into the focused Wayland client

Use `gnoblin.input.text_target(context)` from a shortcut event callback to
request a one-use text target. It returns an operation; on success, its value
is an opaque `TextTarget`:

```lua
gnoblin.events.on("gnoblin.shortcut.activated", function(event)
    if not event.focus_context then
        return
    end

    gnoblin.input.text_target(event.focus_context):on_complete(function(target, err)
        if err then
            return
        end
        target:insert_text("😀")
    end)
end)
```

The compositor consumes the `FocusContext` when it handles the request. It
creates a target only when the same Wayland surface and client still have
keyboard focus and have an active text-input-v3 session. The target expires
with its five-second `FocusContext` deadline. Locking the session, reloading
the runtime, losing focus, or closing the runtime revokes it. X11 clients are
not supported.

`TextTarget.window_id` is the focused window's stable ID when the surface
belongs to a window. `TextTarget.caret` is an optional rectangle in
compositor logical coordinates. These fields are for shell presentation; only
the opaque target authorizes insertion.

Call `target:insert_text(text)` once. It returns an operation and consumes the
target on the first attempt. Text must contain 1 to 256 bytes of valid UTF-8
without NUL or control characters.

Insertion succeeds only while the same surface, client, focus epoch, and active
text input remain current. The session must be unlocked. Mutter commits the
text through its focused input-method path.

Modifiers held when the shortcut activated may remain held. Adding another
modifier invalidates the target. Ctrl, Alt, Shift, Lock, Meta, and Hyper state
prevents target creation. Gnoblin keeps at most 128 active text targets per
compositor; creation fails while the limit is full.

### Snapshot record methods

Window and workspace snapshots expose methods that queue the corresponding
typed operation. Call them with colon syntax from a runtime event callback.
Each method returns an `Operation` handle.

| Window method                                  | Arguments                                         | Effect                                                    |
| ---------------------------------------------- | ------------------------------------------------- | --------------------------------------------------------- |
| `window:close()`                               | None                                              | Ask the application to close.                             |
| `window:minimize()`                            | None                                              | Minimize the window.                                      |
| `window:toggle_minimize()`                     | None                                              | Minimize or restore the window.                           |
| `window:unminimize()`                          | None                                              | Remove minimization without changing maximization.        |
| `window:restore()`                             | None                                              | Remove minimization and maximization.                     |
| `window:set_maximized(enabled)`                | Boolean                                           | Set maximization.                                         |
| `window:set_fullscreen(enabled)`               | Boolean                                           | Set fullscreen.                                           |
| `window:set_above(enabled)`                    | Boolean                                           | Set the above state.                                      |
| `window:set_sticky(enabled)`                   | Boolean                                           | Set visibility across workspaces.                         |
| `window:move(position)`                        | `{x, y}` integer coordinates                      | Move in logical desktop pixels.                           |
| `window:resize(size)`                          | `{width, height}` integer dimensions              | Resize in logical pixels.                                 |
| `window:move_to_workspace(selector, options?)` | Workspace selector; optional `follow` boolean     | Move this window and optionally activate the destination. |
| `window:move_to_monitor(target)`               | Monitor connector ID or `{id = ID}`               | Move this window to an active monitor.                    |
| `window:thumbnail(size)`                       | `{width = integer 1–480, height = integer 1–320}` | Capture a bounded compositor-rendered PNG preview.        |

| Workspace method                        | Arguments                                                                   | Effect                              |
| --------------------------------------- | --------------------------------------------------------------------------- | ----------------------------------- |
| `workspace:activate()`                  | None                                                                        | Activate this workspace.            |
| `workspace:rename(name)`                | Nonempty name up to 80 characters                                           | Rename this workspace.              |
| `workspace:remove()`                    | None                                                                        | Remove this workspace when allowed. |
| `workspace:move_here(window, options?)` | Window snapshot, stable window ID, or `"active"`; optional `follow` boolean | Move the window to this workspace.  |

Record properties remain read-only. In the native runtime,
`Window:focus(context)` requests focus using the context from
`gnoblin.shortcut.activated`. The callback provides that value only for a
trusted shortcut key press. It expires after five seconds.
A successful focus operation completes with `{id = window.id}`.

The same context can authorize `begin_move` or `begin_resize`. It works once
across all three operations. A config reload invalidates contexts from the old
Lua runtime; a session lock also revokes them.

Mutter still applies its normal window activation policy.

Contexts are not provided for clicks, external-client shortcuts, release
bindings, synthetic input, input-method events, or repeated key presses. Direct
`Window:focus`, `Window:begin_move`, and `Window:begin_resize` calls without a
context remain denied.

A separate shell process can subscribe through native-control API 1.10 and use
its connection-bound `focus_context` token; see the
[compositor bridge](/compositor-bridge#api-version-110-shortcut-focus-grants).

### Pointer and keyboard snapping

Lua pointer callbacks receive a read-only `event.drag` record on the started
and updated events. It contains window and drag IDs, settings revision, pointer
coordinates, modifiers, monitor and work-area rectangles, the window frame,
and maximized state.

Call `event.drag:offer_targets(targets)` during the callback. An offer can
contain 1 to 128 targets. Each target has:

- a unique `id`;
- pointer `hit` and destination `frame` rectangles;
- optional `maximize`, `required_modifiers`, and `forbidden_modifiers` fields.

Rectangles use integer logical coordinates and `{x, y, width, height}` fields.
Both must fit in the current work area. Modifier arrays accept only `control`,
which cannot be both required and forbidden.

On release, Mutter checks the latest accepted offer against the actual pointer
and modifiers. A match applies its frame; otherwise Mutter completes the
normal move or tile-preview behavior. The shell owns snap guides and other
presentation.

Drag records expire on release, lock, config reload, window loss, or compositor
teardown. An old runtime cannot replace an offer after reload.

Layer-shell clients can use the native-control socket API 1.26. Subscribe to
the drag lifecycle events before a drag begins. Started and updated events
include a private token for that connection. Submit an offer on that same
connection:

```json
{
    "op": "api",
    "api_version": { "major": 1, "minor": 26 },
    "id": "offer-1",
    "method": "window.snap.offer",
    "arguments": {
        "drag_id": 42,
        "drag_token": "<token from the event>",
        "targets": [
            {
                "id": "left",
                "hit": { "x": 0, "y": 0, "width": 700, "height": 900 },
                "frame": { "x": 0, "y": 0, "width": 700, "height": 900 }
            }
        ]
    }
}
```

The token belongs to one live drag and config generation. The first accepted
Lua or socket offer owns that drag; only that owner can replace its targets.
Disconnect or replacing the event subscription revokes a socket token and
clears its offer.

The compositor checks the actual release pointer, modifiers, monitor, and work
area before applying a match. It never waits for the shell or Lua runtime on
release.

### Compositor requests

Subscribe to `gnoblin.window.menu-requested` to show a shell-owned window menu,
or `gnoblin.osd.requested` to show an OSD requested by Mutter. Both events
require native-control API 1.27. They are also available to Lua callbacks as
events with the same names.

`gnoblin.window.menu-requested` includes:

- `window_id`: Gnoblin's stable window ID;
- `menu_type`: `wm` for a window manager menu or `app` for an application menu;
- `x` and `y`: global logical coordinates supplied by Mutter.

For `menu_type == "wm"`, Lua receives a non-serializable
`event.menu_context` userdata. In that callback, call
`event.menu_context:begin_move()` or
`event.menu_context:begin_resize(edge)` to start Mutter's keyboard grab on the
exact window that raised the menu. The context works once, expires after five
seconds, and is available only during that event callback. It is revoked on
session lock or runtime/config teardown. App-menu requests have no context.

Socket API 1.30 clients receive a per-connection `menu_context` token on WM
menu events. Pass it to `window.begin_move` or `window.begin_resize` without an
`id`; Gnoblin binds it to the client and original window. Application-menu
events remain informational.

The token expires after five seconds and is consumed on every matching attempt,
including malformed arguments. Disconnect and event-subscription replacement
revoke it. Both Lua and socket paths recheck the window and lock state before
using a fresh Mutter timestamp.

`gnoblin.osd.requested` includes:

- `monitor_id`: the stable ID of the logical monitor;
- `output_names`: on current builds, a sorted, unique list of active physical
  connector names for that logical monitor. Older API 1.27 builds may omit it;
- `icon` and `label`: optional fields supplied by Mutter.

Mutter provides no OSD level or maximum. Gnoblin does not create the OSD; the
subscribed shell decides how to present the request.

For keyboard-selected layouts, call `gnoblin.windows.snap_context(context)` in
a trusted shortcut callback. It consumes the one-use focus context and returns
an `Operation<SnapContext>` for the focused window; callers cannot provide a
window ID.

Commit once with a monitor ID and frame rectangle. Gnoblin checks the window,
monitor, lock state, runtime generation, and work-area bounds again. The
operation resolves to `{window_id, monitor_id, committed = true}`. The context
expires with its source focus context and is revoked by lock or config reload.

Keyboard `SnapContext` is available to Lua and native-control API 1.28 clients.
Socket clients use `window.snap_context` and `window.snap`; they pass the focus
token from a shortcut event and cannot choose the target window.

Socket focus tokens expire after five seconds and work only on the connection
that received them. Disconnect, event-filter replacement, config reload, or
session lock revokes them. A failed focus request consumes its context.
Gnoblin keeps at most 128 active snap contexts per compositor. If the limit is
full, context creation fails until an existing context expires or is revoked.

Any same-user process can connect to the mode-0600 socket, so same-user clients
are inside the trust boundary.

Lua configuration reads windows and workspaces through the immediate,
immutable `gnoblin.windows.list()` and `gnoblin.workspaces.list()` snapshots.
The compositor socket keeps `window.list` and `workspace.list` for compatibility
clients. Every client version reads both methods from the Lua snapshots; the
socket adapters preserve their existing response shapes and field names.

The newer `windows.list` method returns the snake_case fields described in
[Lua events](/config/lua-events) and adds a per-record revision.

Native workspace records rename the `windows` count to `window_count` and add
a revision. Window, workspace, monitor, layer, input-device, and capability
snapshots share a revision that advances when any of those states changes.

Lua configuration changes windows through typed methods on `Window` snapshots.

The raw compositor socket exposes the operations below to clients such as
`gnoblinctl`. The legacy `window.action` method accepts basic compositor
actions, resize from API 1.61, move from API 1.62, and workspace and monitor
moves from API 1.63. Lua configuration uses typed window methods instead.

| Method                   | Arguments                                                         | Successful result                                            |
| ------------------------ | ----------------------------------------------------------------- | ------------------------------------------------------------ |
| `window.list(args)`      | Optional `app_id`, `title`, `focused` filters                     | `{windows = {Window, ...}}`                                  |
| `window.match(args)`     | Optional string `window` ID; defaults to `"active"`               | Window identity and a `match` rule table                     |
| `window.action(args)`    | `action`; optional `window`; action-specific fields below         | `{ok, pending, window, action}`                              |
| `window.thumbnail(args)` | Stable window `id`; integer `width` and `height` up to 480 by 320 | Asynchronous operation with actual dimensions and base64 PNG |
| `layer.list()`           | None                                                              | `{surfaces = {Surface, ...}}`                                |
| `monitor.list()`         | None                                                              | `{monitors = {Monitor, ...}}`                                |

`layer.list` and `monitor.list` use the Lua snapshots for every client
version. Their socket adapters preserve the existing response wrappers.

`window.thumbnail` requires API 1.23. It accepts a stable window `id` and
integer `width` and `height` dimensions up to 480 by 320. The compositor scales
down to fit while preserving aspect ratio.

Every supported socket client routes basic `window.action` requests through
the Lua runtime. API 1.61 adds resize through `window.resize`; API 1.62 adds
move through `window.move`. API 1.63 adds workspace and monitor moves through
`window.move_to_workspace` and `window.move_to_monitor`.

Resize accepts integer dimensions from 1 to 32768 logical pixels. Move accepts
integer coordinates from −100000 to 100000 logical pixels.

The workspace action accepts a `workspace` selector with exactly one `id` or
one-based `number` field. The monitor action accepts the current zero-based
monitor `index`; Gnoblin resolves it to the stable connector ID before calling
the typed operation. Both actions require API 1.63.

The response acknowledges that Lua queued the typed operation. The compositor
applies it asynchronously. Action availability is:

- `resize`: API 1.61.
- `move`: API 1.62.
- `workspace` and `monitor`: API 1.63.

Basic actions remain available to older clients.

The operation returns the stable `window_id`, actual dimensions, and `data` as
a base64-encoded PNG. Encoded PNG output is limited to 512 KiB.

- Requests fail while the session is locked.
- Each socket client can have one active capture; the session allows four.
- Gnoblin drops results after a disconnect or window destruction.

Thumbnails are compositor-rendered previews. Mutter 51 provides no
protected-content metadata or DRM-redaction guarantee, and Gnoblin does not
promise that protected content is hidden. The local socket is mode 0600; as
with other native-control methods, same-user processes are inside the session
trust boundary.

In the standalone runtime, each monitor has these fields:

| Field                       | Meaning                                                                                                            |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `id`                        | Canonical active connector name. Cloned outputs use the first connector alphabetically.                            |
| `index`                     | Current zero-based Mutter order. It can change when outputs change.                                                |
| `x`, `y`, `width`, `height` | Logical-pixel monitor geometry.                                                                                    |
| `primary`                   | Whether this is the primary logical monitor.                                                                       |
| `scale`                     | Current monitor scale factor.                                                                                      |
| `enabled`                   | Always `true`; this list contains active logical monitors only.                                                    |
| `transform`                 | Logical monitor transform: `normal`, `90`, `180`, `270`, `flipped`, `flipped-90`, `flipped-180`, or `flipped-270`. |
| `name`                      | Optional display name supplied by Mutter.                                                                          |
| `make`, `model`, `serial`   | Optional physical display details supplied by Mutter.                                                              |
| `refresh_rate`              | Optional current refresh rate in Hz for the connector used as `id`.                                                |

Use the connector `id` for typed monitor moves.
When outputs are cloned, the refresh rate comes from the lexicographically
first active connector used as the monitor ID. The native result does not
include inactive physical outputs.

| Filter or field | Accepted value                 | Meaning                                             |
| --------------- | ------------------------------ | --------------------------------------------------- |
| `app_id`        | Application ID string          | Exact match in `window.list`.                       |
| `title`         | Text string                    | Case-insensitive substring filter in `window.list`. |
| `focused`       | Boolean; set `true` to filter  | Lists only the focused window when true.            |
| `window`        | `"active"` or stable window ID | Selects the `window.match` or action target.        |

`window.match` returns the stable window ID, desktop and GTK application IDs,
WM class, and a rule `match` table containing type, title, focus state, and the
rule app ID when available.
All socket clients get this record from the shared Lua snapshot. Older clients
keep the same response fields.

`window.action` accepts these actions:

```lua
WindowAction = "above" | "unabove" | "stick" | "unstick" | "close"
             | "minimize" | "restore" | "maximize" | "unmaximize"
             | "fullscreen" | "unfullscreen" | "resize" | "move"
             | "workspace" | "monitor"
```

The optional `window` is a stable ID from `window.list`. When omitted, the
focused window is used.

Action availability:

- `resize` requires API 1.61.
- `move` requires API 1.62.
- `workspace` and `monitor` require API 1.63.

This legacy method does not accept focus, menus, or interactive moves and
resizes. Use the typed methods below for those operations. Focus and
interactive operations require a trusted context.

The result reports that the compositor accepted the action for the selected
window. The change may complete asynchronously. Requests fail while the session
is locked or when the target window cannot perform the action in its current
state.

### Typed window operations

Typed operations take the stable string `id` returned by `window.list`. They
reject unknown argument fields. Most successful operations return `{id}` as
their completed value. `window.restore_or_minimize` also returns the selected
`action`. Window operations fail while the session is locked.

| Method                             | Arguments                                                                                                                                    | Effect                                                                     |
| ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `window.close(args)`               | `id`                                                                                                                                         | Requests a normal close; the application can show an unsaved-work prompt.  |
| `window.minimize(args)`            | `id`                                                                                                                                         | Minimizes the window.                                                      |
| `window.toggle_minimize(args)`     | `id`                                                                                                                                         | Restores a minimized window; otherwise minimizes it.                       |
| `window.restore(args)`             | `id`                                                                                                                                         | Removes minimization and maximization. It does not leave fullscreen.       |
| `window.restore_or_minimize(args)` | `id`                                                                                                                                         | Unmaximizes, restores the saved pre-snap frame, or minimizes the window.   |
| `window.set_maximized(args)`       | `id`, `enabled` boolean                                                                                                                      | Sets maximization on or off.                                               |
| `window.set_fullscreen(args)`      | `id`, `enabled` boolean                                                                                                                      | Sets fullscreen on or off.                                                 |
| `window.set_above(args)`           | `id`, `enabled` boolean                                                                                                                      | Sets always-on-top on or off.                                              |
| `window.set_sticky(args)`          | `id`, `enabled` boolean                                                                                                                      | Shows the window on all workspaces or only its own.                        |
| `window.move(args)`                | `id`, integer `x`, integer `y` from −100000 to 100000                                                                                        | Sets the frame position in logical desktop pixels.                         |
| `window.resize(args)`              | `id`, integer `width`, integer `height` from 1 to 32768                                                                                      | Sets the outer frame size in logical pixels.                               |
| `window.move_to_workspace(args)`   | `id`, `workspace` selector; optional `follow` boolean                                                                                        | Moves the window; `follow = true` also activates that workspace.           |
| `window.move_to_monitor(args)`     | `id`, `monitor` connector ID or `{id = connector ID}`                                                                                        | Moves the window to that active monitor.                                   |
| `window.focus(args)`               | Lua: `Window:focus(FocusContext)`; socket 1.10: `id` + `focus_context`; socket 1.32: `id` + `activation_token`                               | Focuses a listed window with one-use focus proof.                          |
| `window.begin_move(args)`          | Native Lua: `Window:begin_move(FocusContext)`; socket API 1.12: `id`, `focus_context`; menu API 1.30: `menu_context`                         | Starts a keyboard move grab with one-use shortcut or WM-menu authority.    |
| `window.begin_resize(args)`        | Native Lua: `Window:begin_resize(edge, FocusContext)`; socket API 1.12: `id`, `edge`, `focus_context`; menu API 1.30: `menu_context`, `edge` | Starts a keyboard resize grab at the selected edge with one-use authority. |

Resize edges are:

- `north`, `south`, `east`, or `west` to resize from a side.
- `north_east`, `north_west`, `south_east`, or `south_west` to resize from a corner.

Each interactive method needs the same one-use trusted context used by the
Lua focus method. The context authorizes one operation. Mutter starts its
keyboard grab with the shortcut timestamp and current pointer sprite. Calls
without a trusted context fail closed.

Socket clients can use `window.focus` with an XDG Activation token at API
1.32. The shell obtains the token through XDG Activation after user input on
one of its Wayland surfaces, then sends it with the target window's stable
`id`.

Gnoblin checks that the same process created the token and owns the socket
connection. Mutter validates the seat, serial, and source surface. Gnoblin
checks that the session is unlocked and consumes the token before focusing.

The token is single-use. Gnoblin does not expose it to Lua or accept
caller-supplied timestamps or input serials. See the
[compositor bridge](../compositor-bridge.md#api-version-132-xdg-activation-focus-and-session-logout)
for the wire format.

The native `monitor.list()` result uses active connector names such as `DP-1`
or `eDP-1` as string IDs. For cloned outputs, it selects the first active
connector alphabetically.

A typed move fails with `not_found` if its connector ID is no longer listed.
Native `window.action` with `action: "focus"` is denied; use `window.focus`
with a verified context instead.

## Animations

The Lua runtime exposes animation controls through `gnoblin.animations`. Keep
`gnoblin.animation { ... }` for declaring or updating a named animation in
configuration.

| Method                             | Arguments                                         | Successful result                            |
| ---------------------------------- | ------------------------------------------------- | -------------------------------------------- |
| `gnoblin.animations.list()`        | None                                              | Read-only `AnimationInfo[]`                  |
| `gnoblin.animations.get(name)`     | Animation name                                    | Read-only `AnimationInfo` or `nil`           |
| `gnoblin.animations.surfaces()`    | None                                              | `{surfaces = {Surface, ...}}`                |
| `gnoblin.animations.inspect(args)` | `name`, `target`; optional `event`, `target_type` | Animation details and resolved specification |
| `gnoblin.animations.preview(args)` | Same as inspect; optional `autoplay`              | `AnimationPreview` record                    |
| `gnoblin.animations.seek(args)`    | `session`; `progress` from 0 to 1                 | Updated `AnimationPreview` record            |
| `gnoblin.animations.step(args)`    | `session`; `milliseconds` from 1 to 60000         | Updated `AnimationPreview` record            |
| `gnoblin.animations.play(args)`    | `session`                                         | Updated `AnimationPreview` record            |
| `gnoblin.animations.pause(args)`   | `session`                                         | Updated `AnimationPreview` record            |
| `gnoblin.animations.stop(args)`    | `session`                                         | `{ok = true, session = string}`              |

`gnoblinctl lua` supports the five controls that take an explicit session
table. Use the `id` from an `AnimationPreview` as the `session` value. The CLI
waits for each operation and returns the updated read-only preview, or the
stop result.

| Field         | Accepted value                                  | Meaning                                       |
| ------------- | ----------------------------------------------- | --------------------------------------------- |
| `target_type` | `window` (default), `layer`, or `namespace`     | Selects how to resolve `target`.              |
| `target`      | `"active"` or window ID; layer ID; or namespace | Identifies the preview target.                |
| `name`        | 1–80 ASCII letters, digits, `_` or `-`          | Selects an animation.                         |
| `event`       | Optional lowercase kebab-case event name        | Selects an event supported by that animation. |
| `autoplay`    | Boolean; default `false`                        | Starts a preview immediately when `true`.     |

See the [animation guide](/guides/animations) for supported animation events.
In Lua, `gnoblin.animations.preview(spec)` returns an `Operation`. Its
successful value is an immutable preview record.

The record methods are `seek`, `step`, `play`, `pause`, and `stop`. They return
new operations. The stop result contains an `ok` flag and a `session` ID.

Configured `open` and `close` animations run for normal windows during their
map and destroy lifecycles. The compositor starts built-in or registered
animations whose event matches the selected rule. The native methods below
control explicit preview sessions.

For example, inspect a new window's `open` animation, seek to its midpoint,
then play from there:

```lua
gnoblin.events.on("gnoblin.window.created", function(event)
    local request = gnoblin.animations.preview {
        name = "gnome-open",
        event = "open",
        target = event.window.id,
        target_type = "window",
    }

    request:on_complete(function(preview, error)
        if error then
            print(error.message)
            return
        end

        preview:seek(0.5):on_complete(function(seeked, seek_error)
            if seek_error then
                print(seek_error.message)
                return
            end
            seeked:play()
        end)
    end)
end)
```

## Input sources

| Lua method                                  | Arguments                       | Successful result                       |
| ------------------------------------------- | ------------------------------- | --------------------------------------- |
| `gnoblin.input.devices()`                   | None                            | Physical input-device records           |
| `gnoblin.input.sources()`                   | None                            | Configured XKB input-source records     |
| `gnoblin.input.current_source()`            | None                            | Current XKB source, or `nil`            |
| `gnoblin.input.select_source({type, id})`   | Source selector                 | Operation returning the selected source |
| `gnoblin.input.orientation_lock()`          | None                            | Immutable `OrientationLock` record      |
| `gnoblin.input.set_orientation_lock(value)` | `true`, `false`, or `"inherit"` | `Operation<OrientationLock>`            |

Pass a source's `type` and `id` from `gnoblin.input.sources()` to
`gnoblin.input.select_source()`.

Lua also retains two read aliases: `gnoblin.input.list()` matches
`gnoblin.input.sources()`, and `gnoblin.input.current()` matches
`gnoblin.input.current_source()`. The socket operation `input.select` is
exposed in Lua as `gnoblin.input.select_source()`.

These aliases are available in supervised configuration and `gnoblinctl lua`.
The CLI maps them to the canonical socket reads. Socket clients call the
canonical method names directly.

## Privacy and permissions

| Method                             | Arguments                | Successful result                                        |
| ---------------------------------- | ------------------------ | -------------------------------------------------------- |
| `gnoblin.privacy.state()`          | None                     | Read-only `PrivacyState` snapshot                        |
| `gnoblin.privacy.stop_sharing()`   | None                     | `Operation<{requested: integer}>`                        |
| `gnoblin.privacy.stop_recording()` | None                     | `Operation<{requested: integer}>`                        |
| `permissions.list()`               | None                     | Read-only policy, capabilities, levels, and config path  |
| `permissions.policy()`             | None                     | Immutable policy with `default`, `rules`, and `revision` |
| `permissions.check(args)`          | `capability`, `identity` | Permission decision with scope details                   |
| `grant.list()`                     | None                     | `{grants = {Grant, ...}}`                                |
| `grant.revoke(args)`               | `kind`, `id`             | `{ok, id}`                                               |

Permission capabilities, identities, grant kinds, and scope fields use the
same values as [session permissions](/config/configure/permissions).
`gnoblinctl lua` exposes `gnoblin.permissions.list()` as a deeply read-only
snapshot through native-control API 1.42.

`permissions.check` returns an immutable decision with `level`, `rule`,
`monitors` (a string array), `devices` (an array containing any of
`"keyboard"`, `"pointer"`, and `"touchscreen"`), `clipboard`, and `revision`.
The revision identifies the committed permission-policy snapshot.

`gnoblin.privacy.state()` returns an immutable `PrivacyState` record with an
`available` field and a `revision`.

Socket clients of every supported API version read this state through the
shared Lua runtime.

The `available` record uses `screen_sharing`, `recording`,
`microphone_in_use`, `camera_in_use`, and `location_in_use`. Gnoblin omits an
activity field when its source is unavailable.

The native runtime reports screen-sharing and recording activity from Mutter's
tracked remote-access handles. Microphone and camera monitoring are available
when Mutter is built with remote-desktop support and can connect to PipeWire.

`gnoblinctl lua` also exposes `gnoblin.privacy.stop_sharing()` and
`gnoblin.privacy.stop_recording()`. The console waits for compositor completion
and returns a deeply read-only `{requested = integer}` result. A positive count
means Mutter was asked to stop those handles; it does not confirm that they
have closed.

It reports running audio-capture streams, including meter streams opened by
volume-control applications. A stream's self-reported application ID is not
trusted to suppress microphone activity. The monitor reports an active capture
stream; it does not inspect whether the stream is carrying audible samples.

Camera monitoring follows running PipeWire nodes whose media role is `Camera`.
It keeps the activity state for 500 ms after the last node stops to avoid
flickering. Location availability and activity come from Gnoblin's GeoClue
agent. A source is unavailable when its service or monitor cannot be reached;
unavailable does not mean inactive.

GeoClue must allow the `gnoblin` agent ID in its agent whitelist. Append
`gnoblin` to the existing `[agent]` `whitelist` in the system's GeoClue
configuration, preserving the other IDs. If the service rejects Gnoblin as an
agent, location is reported as unavailable and requests are not delivered.

Gnoblin publishes `gnoblin.location.authorization-requested` when GeoClue asks
whether an application may use location. A shell or Lua handler can answer
with `gnoblin.location.authorize_app`:

```lua
gnoblin.on("gnoblin.location.authorization-requested", function(event)
  gnoblin.location.authorize_app {
    request_id = event.request_id,
    allow = false,
    accuracy = 0,
  }
end)
```

This handler denies every request. A shell that asks the user for consent can
return `allow = true` with the selected accuracy after the user approves.
Socket clients need native-control API 1.65 to receive and answer these
requests. GeoClue configuration that does not allow the agent must be fixed by
the system administrator before the capability becomes available.

GeoClue supplies `app_id` as the application's desktop ID. Treat it as a
request attribute, not an authenticated identity.

Accuracy levels are:

- `0`: none. Use this when denying a request.
- `1`: country.
- `4`: city.
- `5`: neighborhood.
- `6`: street.
- `8`: exact.

An allowed answer must choose a nonzero level no more precise than the
request. Gnoblin also clamps approval to the system location setting's enabled
state and maximum accuracy.

The event fields are `request_id`, `app_id`, `requested_accuracy`, and
`expires_at_us`, plus the standard event metadata.

Requests expire after 25 seconds. Gnoblin denies unanswered requests and
requests still pending when the runtime stops or the GeoClue agent becomes
unavailable. A socket client can answer only if it received the request event.
The answer is a one-use operation.

`gnoblin.privacy.stop_sharing()` requests closure of tracked non-recording
handles. `gnoblin.privacy.stop_recording()` requests closure of tracked
recording handles. Each operation result contains integer `requested`, the
number of handles passed to `meta_remote_access_handle_stop()`. A positive
count confirms that the calls were issued, not that a session has already
closed. Mutter reports closure later through `gnoblin.privacy.changed`; these
methods do not revoke a saved portal grant.

`permissions.policy()` returns the committed Gnoblin policy. The record has a
default level, ordered rules, and revision. Native-control API 1.16 adds the
matching socket method and `gnoblin.permission.changed` event.
API 1.44 routes that socket method through the Lua runtime.

`permissions.list()` returns that policy with capability names, supported
levels, and the configuration path. Its socket method uses the Lua runtime
starting at API 1.42.

Use `gnoblin.grant.revoke {kind, id}` with the `kind` and `id` from a listed
portal grant. The optional `created_at` timestamp rejects a stale record if a
new grant reuses its ID. `gnoblinctl lua` exposes this operation and returns a
deeply read-only result after the compositor confirms completion.

Each grant record contains:

| Field           | Type and values                                                  |
| --------------- | ---------------------------------------------------------------- |
| `id`            | Opaque string returned by `grant.list()`.                        |
| `kind`          | `"screen-cast"` or `"remote-desktop"`.                           |
| `requester`     | Verified portal identity, such as `app-id:org.example.Recorder`. |
| `devices`       | Integer bitmask: keyboard `1`, pointer `2`, touchscreen `4`.     |
| `clipboard`     | Boolean.                                                         |
| `screenStreams` | Boolean indicating whether a screen-stream selection is stored.  |

Both methods return `Operation` handles. Grant listing and revocation run in
the portal backend, which validates the stored record before returning or
removing it. A missing grant, invalid record, or unavailable backend fails the
operation.

The native Lua runtime also provides `gnoblin.portals.grants(filter?)` for an
immediate snapshot. Pass `{kind = "screen-cast"}` or
`{kind = "remote-desktop"}` to filter the records. Each immutable
`PortalGrant` record contains:

| Field                | Type and values                                                 |
| -------------------- | --------------------------------------------------------------- |
| `id`                 | Opaque portal grant ID.                                         |
| `kind`               | `"screen-cast"` or `"remote-desktop"`.                          |
| `requester`          | Verified portal identity.                                       |
| `devices`            | Array of `"keyboard"`, `"pointer"`, or `"touchscreen"`.         |
| `clipboard`          | Boolean.                                                        |
| `has_screen_streams` | Boolean indicating whether a screen-stream selection is stored. |
| `created_at`         | Unix timestamp in milliseconds.                                 |
| `revision`           | Revision of the current native grant snapshot.                  |

Call `grant:revoke()` on a record from this snapshot. It returns an
`Operation` whose successful value contains `ok` and `id`. The portal backend
checks that the record still has the same creation time before revoking it.
Stale records fail.

New grants store their creation time. Older grants use file modification time
at whole-second precision; this is inferred metadata, not a verified consent
time. The collection is available after the native portal grant snapshot has
loaded.

Native-control API 1.15 adds the socket method `portals.grants`, which returns
the same records as a JSON array and accepts the same optional `kind` filter.
Every supported client version reads it through `gnoblin.portals.grants()`.
API 1.15 clients can subscribe to `gnoblin.portal.grant-added` and
`gnoblin.portal.grant-removed`; see the [compositor bridge](/compositor-bridge#api-version-115-portal-grant-snapshots-and-events).

## Launch feedback and configuration

Use `gnoblin.launches` to read launch feedback and track a launch request.
`list()` returns the latest cached native snapshot as immutable `Launch`
records. `snapshot()` returns those records together with the collection
revision, including when the collection is empty.

The native controller seeds an empty snapshot during startup and refreshes it
before dispatching launch-change events. Both methods are unavailable before
native startup completes. `gnoblinctl lua` also exposes both reads through
native-control API 1.39, with deeply read-only launch records and snapshots.

| Method                            | Arguments                                     | Result                                          |
| --------------------------------- | --------------------------------------------- | ----------------------------------------------- |
| `gnoblin.launches.list()`         | None                                          | `Launch[]` snapshot                             |
| `gnoblin.launches.snapshot()`     | None                                          | `{launches, revision}` snapshot                 |
| `gnoblin.launches.begin(options)` | `token`, `application`; optional `timeout_ms` | An `Operation` whose value is a `Launch` record |
| `gnoblin.launches.finish(token)`  | Launch token string                           | An `Operation` whose value is `{ok, token}`     |

`begin` uses a token and application to identify the launch attempt.
`timeout_ms` defaults to 3000, accepts 100 through 10000, and maps to the
native launch operation's `milliseconds` field.

Call `finish(token)` with the same token when the launch is cancelled or the
application has started. The bracket form `gnoblin.launches["end"](token)`
remains as a compatibility alias because `end` is a Lua keyword. The returned
`Operation` completes asynchronously, as described near the start of this
page.

The operation-based `launch.status()`, `launch.begin(args)`, and
`launch.end(args)` methods remain available for generic API and bridge callers.
They use the same native controller as `gnoblin.launches` while keeping their
established field names and argument-table shape.

| Method                    | Arguments                                       | Successful result                                |
| ------------------------- | ----------------------------------------------- | ------------------------------------------------ |
| `launch.status()`         | None                                            | `{launches, revision}`                           |
| `launch.begin(args)`      | `token`, `application`; optional `milliseconds` | `Launch` record                                  |
| `launch.end(args)`        | `token`                                         | `{ok, token}`                                    |
| `runtime.reload_config()` | None                                            | `{ok, action}`, plus native `runtime_generation` |

`launch.begin` accepts a token up to 128 characters and an application hint up
to 512 characters. `milliseconds` defaults to 3000 and is clamped to 100–10000
ms.

The legacy socket `launch.status` method reads this snapshot through Lua for
every supported client version and enables launch-change events on that
connection.

The native runtime retains at most 64 records. It evicts the oldest completed
record when needed and rejects a new launch if all retained records are pending.

Each immutable `Launch` record contains:

- `token` and `application`.
- `started_at`, as Unix time in milliseconds.
- `timeout_ms`, `state`, and `revision`.

The state begins as `pending`. Mutter reports `started` when a matching mapped
window appears or becomes focused. `launch.end` changes a pending record to
`ended`; the deadline changes it to `timed_out`.

Matching uses GTK application ID, WM class, or WM class instance. Values are
lowercased and a `.desktop` suffix is removed. The native controller cannot report failures from an external process
launcher.

Subscribe to `gnoblin.launch.changed` for updates. End a request with the same
token when it is cancelled or the application has started.

The native socket response waits until the candidate commits. If shutdown
aborts an outstanding reload, the server queues an error reply, but may close
the connection before that reply flushes; the client can receive EOF instead.

## Shortcut state and capture

`gnoblin.shortcuts.list()` returns named shortcuts configured for the native
compositor. Records do not include shortcuts owned by external shell clients.
Each read-only record contains:

- `name`, `binding`, `enabled`, `trigger`, and `revision`.
- Either `command` (an argument array) or `action` (a `group.key` identifier).
- `binding` as a string for one accelerator or an array for multiple bindings.

`enabled` is false for a configured built-in action with no bindings. Disabled
shortcut declarations and shortcuts that need shell input capture are not
registered by the native compositor and are not listed.

```lua
for _, shortcut in ipairs(gnoblin.shortcuts.list()) do
    print(shortcut.name, shortcut.binding)
end
```

Native-control API version 1.9 adds the read-only `shortcut.list` method. It
accepts no arguments and returns the same records as a JSON array. API version
1.10 adds the socket event `gnoblin.shortcut.activated` and the `window.focus`
method, which requires a one-use context from that event.

`gnoblinctl shortcut list` uses the API 1.40 `shortcuts.list` read. The older
`shortcut.list` socket method remains available to existing clients. API 1.9
and newer clients get the original JSON array from Lua.

The legacy `shortcut.actions` socket method also remains available. API 1.5
and newer clients get its results from the shared Lua read.

API 1.12 adds `window.begin_move` and `window.begin_resize`; both consume the
same connection-bound token. See the
[compositor bridge](/compositor-bridge#api-version-110-shortcut-focus-grants)
for the socket request format.

API 1.22 adds held and modal dynamic shortcut bindings. `shortcut.bind` accepts
`id` and `accelerator`, with optional `hold`, `trigger`, `mode`, and
`capture_input` fields:

- `hold` accepts `none`, `super`, `control`, or `alt`. It defaults to `none`.
- `trigger` accepts `press` or `release`. It defaults to `press`.
- `mode` accepts `passive` or `modal`. It defaults to `passive`. Modal mode
  requires a held modifier and captures keyboard events while it is held.
- `capture_input` defaults to `false`. Set it to `true` for a bare `Super`
  binding; this flag is rejected for other accelerators. A bare `Super`
  binding also requires `trigger: "release"`, `hold: "none"`, and an available
  compositor early modifier hook.

`shortcut.unbind` accepts only the binding `id`.

API 1.64 adds `shortcut.session.end`. It accepts the binding `id` and the
`session_id` from that binding's `gnoblin.shortcut.session.activated` event.
Ending a session releases its keyboard capture while keeping the binding
registered. A stale session ID or a binding owned by another connection is
rejected. The matching ended event uses reason `cancelled`.

```json
{
    "op": "api",
    "id": "bind-search",
    "api_version": { "major": 1, "minor": 22 },
    "method": "shortcut.bind",
    "arguments": { "id": "search", "accelerator": "Super", "trigger": "release", "capture_input": true }
}
```

Lua uses the same operation as `gnoblin.shortcuts.end_session`:

```lua
gnoblin.shortcuts.bind {
    id = "switcher",
    accelerator = "<Alt>Tab",
    hold = "alt",
    mode = "modal",
}

gnoblin.events.on("gnoblin.shortcut.session.activated", function(event)
    if event.id == "switcher" then
        gnoblin.shortcuts.end_session {
            id = event.id,
            session_id = event.session_id,
        }
    end
end)
```

Registrations belong to the socket connection that created them. Disconnecting
that client removes its bindings. Active sessions also end on modifier
release, unbind, config reload, session lock, capture preemption, owner
disconnect, or after ten seconds.

Socket clients can subscribe to the following API 1.22 events:

- `gnoblin.shortcut.session.activated` when a held binding starts.
- `gnoblin.shortcut.session.key` for captured keyboard input. Fields include
  `keyval`, `keycode`, `modifiers`, `phase` (`press` or `release`), and `time`.
- `gnoblin.shortcut.session.ended` when a session ends. Reasons are `released`,
  `unbound`, `owner_disconnected`, `config_changed`, `locked`, `preempted`,
  `timed_out`, `cancelled`, `compositor_stopped`, and `runtime_stopped`. The
  `cancelled` reason is used by `shortcut.session.end`. `runtime_stopped` is
  sent to a socket client when the Lua runtime stops while that client owns the
  active session.

`gnoblin.shortcut.binding-activated` remains available from API 1.11. Its first
trusted activation can carry a connection-bound, one-use `focus_context` token.
API 1.36 adds `gnoblin.shortcut.binding-deactivated` for press-triggered
bindings. The event contains:

- `id` and `accelerator` to identify the binding.
- `input_time`, Mutter's timestamp for the key release.

Release-triggered bindings activate on release and do not emit a second
deactivation event.

Modal sessions capture keyboard events only. Pointer input remains available to
the shell's layer-shell surfaces and client windows.

`gnoblin.shortcuts.actions(group?)` reads built-in actions from installed
GSettings schemas. Omit `group` to list all installed groups.

| Group     | Schema                                 |
| --------- | -------------------------------------- |
| `wm`      | `org.gnome.desktop.wm.keybindings`     |
| `mutter`  | `org.gnome.mutter.keybindings`         |
| `wayland` | `org.gnome.mutter.wayland.keybindings` |

An unknown group raises an error. Requesting a supported group whose schema is
not installed also raises an error. The `shell` group is not included. This
read API is separate from `gnoblin.configure.shortcuts`, which declares
shortcut configuration.

To assign a built-in action, set `action = action.id` on a named
`gnoblin.configure.shortcuts` entry. See the
[shortcut configuration reference](./configure/shortcuts.md) for the
declarative form. `gnoblin.shortcuts.bind()` registers a Gnoblin shortcut event;
it does not invoke a built-in action.

Each action record contains:

- `id`, `group`, and `key`.
- Optional `description`, when the schema provides one.
- `default_bindings`, the schema's exact accelerator string array.

These are schema defaults, not the current user override. An empty array means
that the action has no default accelerator. Actions are returned in `wm`,
`mutter`, `wayland` order, with keys sorted alphabetically within each group.

```lua
for _, action in ipairs(gnoblin.shortcuts.actions("wm")) do
    if action.id == "wm.close" then
        for _, binding in ipairs(action.default_bindings) do
            print(binding)
        end
    end
end
```

### Capture a shortcut

| Method                   | Arguments                                               | Successful result                                     |
| ------------------------ | ------------------------------------------------------- | ----------------------------------------------------- |
| `shortcut.capture(args)` | Optional `timeout` in seconds, from 1 to 60; default 30 | Pending operation; completion returns `{accelerator}` |

Call `gnoblin.shortcuts.capture()` from a runtime event callback. On success,
`value.accelerator` is the normalized accelerator string.

Press Escape to cancel. Capture fails if another capture is active, the session
is locked, Mutter has an active input-capture session, or a compositor stage
grab is active. If an input-capture session or stage grab starts during
capture, Gnoblin cancels the operation before forwarding keys to that owner.
The native runtime does not expose key events to Lua.

This method requires native-control API version 1.8 and advertises the
`shortcut-capture` capability. Check the socket handshake before calling it.

```lua
gnoblin.events.once("gnoblin.window.focused", function()
    local operation = gnoblin.shortcuts.capture({timeout = 10})
    operation:on_complete(function(value, err)
        if err then
            print("Shortcut capture failed: " .. err)
        else
            print(value.accelerator)
        end
    end)
end)
```

See [`gnoblinctl`](/gnoblinctl) for command-line forms of these operations.
