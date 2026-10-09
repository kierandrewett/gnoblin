# Shell runtime API

Use the Lua runtime API to read compositor state, handle events, and request
session actions from a shell integration. Methods return immutable snapshots
for reads and `Operation` handles for asynchronous actions.

Callbacks run on Gnoblin's Lua supervisor, separate from Mutter's compositor
thread, so keep them short and nonblocking.

The Shell development navigation groups these runtime topics. The Config API
navigation covers Lua configuration declarations.

Start with the task you need to support:

- [Inspect and control windows](../shell-api/windows): list windows, read their
  state, and request window operations.
- [Handle focus and activation](../shell-api/focus-activation): read focus
  policy and history, and use trusted input contexts for focus-sensitive work.
- [Manage workspaces and monitors](../shell-api/workspaces-monitors): read and
  change workspaces, inspect displays, and control privacy screens.
- [Read layer surfaces](../shell-api/layer-surfaces): inspect shell surfaces
  and their effective animation policy.
- [Handle input](../shell-api/input): inspect devices and sources, insert text,
  and coordinate pointer or keyboard interactions.
- [Register keyboard shortcuts](../shell-api/keyboard-shortcuts): inspect
  packaged actions, capture accelerators, and manage shortcut sessions.
- [Track launch feedback](../shell-api/launch-feedback): record launches and
  observe their state.
- [Read permissions and privacy state](../shell-api/permissions-privacy):
  inspect policy, portal grants, and privacy activity.
- [Answer authentication prompts](../shell-api/authentication): show polkit
  password prompts and return the answer.
- [Answer keyring and GPG prompts](../shell-api/passphrase-prompts): show
  passphrase prompts from `gnome-keyring` and GPG and return the answer.
- [Read appearance and control animations](../shell-api/appearance-animations):
  query color preference and preview animations.
- [Monitor session and runtime health](../shell-api/session-runtime): read lock,
  idle, and worker state or request session actions.
- [Subscribe to events](./lua-events): handle window, monitor, input, session,
  shortcut, and configuration changes.

## Read committed settings

`gnoblin.settings` is an immutable snapshot of active configuration values.
Setting names use public snake_case, and `revision` identifies the committed
snapshot. It becomes available after the initial configuration commits.

```lua
local focus_mode = gnoblin.settings.window_management.focus_mode
print(focus_mode, gnoblin.settings.revision)
```

`gnoblin.snapshot()` returns a detached copy of the Lua configuration view.
Use `gnoblin.settings` for immutable reads of committed settings.

## Check the running build

`gnoblin.version()` returns an immutable record with string fields `gnoblin`,
`gnome`, `mutter`, `lua`, `api`, `git_remote`, `git_sha`, and `build_id`.
Missing values are returned as `"unknown"`. User information, query strings,
and fragments are removed from remote URLs.

Gnoblin reads `share/gnoblin/version.ini`. Set
`GNOBLIN_VERSION_METADATA_FILE` to select another file. If it is unset or
unreadable, Gnoblin checks the executable's installation prefix and then the
system XDG data directories.

## Shared behavior

Read-only snapshots are detached from compositor state. Use event callbacks to
read current snapshots after startup; some snapshots are not available while
the initial configuration is loading. Runtime actions return an `Operation`.
Its `status`, `value`, and `error` fields report completion; use
`operation:on_complete(callback)` to handle the result asynchronously.

The Lua API and local control socket share methods where the compositor exposes
them. The socket's request format, API version negotiation, and connection-bound
capabilities are documented in the [compositor bridge](/compositor-bridge).
Use [`gnoblinctl lua`](/gnoblinctl#lua-console) to inspect the live session or
run a local Lua file. The CLI runs Lua locally and never sends Lua source to the
compositor.


## Operations

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

`gnoblin.operation.completed` reports an operation ID, method, and success
flag. Success has a `value`; failure has an `Error` record with a stable code
and human-readable message. See [Lua events](/config/lua-events) for callback
subscriptions.

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

For public Lua configuration declarations, see the [configuration API](/config).

## Earlier reference sections

These section fragments remain available for bookmarks and in-site links. Each
points to its current topic page.

### Appearance

See [Appearance and animations](/shell-api/appearance-animations).

### Workspaces

See [Workspaces and monitors](/shell-api/workspaces-monitors).

### Windows, layers, and monitors

See [Windows](/shell-api/windows), [layer surfaces](/shell-api/layer-surfaces), and [workspaces and monitors](/shell-api/workspaces-monitors).

### Immediate window snapshots

See [Windows](/shell-api/windows).

### Monitor privacy screens

See [Workspaces and monitors](/shell-api/workspaces-monitors).

### Insert text into the focused Wayland client

See [Input](/shell-api/input).

### Snapshot record methods

See [Windows](/shell-api/windows) and [workspaces and monitors](/shell-api/workspaces-monitors).

### Pointer and keyboard snapping

See [Input](/shell-api/input).

### Compositor requests

See [Windows](/shell-api/windows).

### Tablet-pad help

See [Input](/shell-api/input).

### Typed window operations

See [Windows](/shell-api/windows).

### Animations

See [Appearance and animations](/shell-api/appearance-animations).

### Input sources

See [Input](/shell-api/input).

### Privacy and permissions

See [Permissions and privacy](/shell-api/permissions-privacy).

### Permission policy

See [Permissions and privacy](/shell-api/permissions-privacy).

### Privacy state

See [Permissions and privacy](/shell-api/permissions-privacy).

### Privacy controls

See [Permissions and privacy](/shell-api/permissions-privacy).

### Location requests

See [Permissions and privacy](/shell-api/permissions-privacy).

### Permission snapshots

See [Permissions and privacy](/shell-api/permissions-privacy).

### Portal grants

See [Permissions and privacy](/shell-api/permissions-privacy).

### Launch feedback and configuration

See [Launch feedback](/shell-api/launch-feedback) and [session and runtime health](/shell-api/session-runtime).

### Shortcut state and capture

See [Keyboard shortcuts](/shell-api/keyboard-shortcuts).

### Capture a shortcut

See [Keyboard shortcuts](/shell-api/keyboard-shortcuts).
