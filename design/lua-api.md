# Gnoblin Lua API

**Status:** internal target design. This file inventories the Lua API present
in the checkout and proposes the standalone interface. Proposed signatures
are not implemented unless marked current, and this is not a frozen contract.
The public configuration and runtime references remain the source of truth
for shipped behavior.

## Goals and boundary

Gnoblin's Lua API is the control and policy interface for Gnoblin sessions.
The standalone window-management and session core consists of Gnoblin and
Mutter. There is no GNOME Shell process, GJS runtime, or GNOME Shell
compatibility layer. Shell projects such as Bingux build their interface as
independent Wayland clients and use Gnoblin's public control API.

This design catalogs the current Lua globals and registered runtime methods,
then proposes the standalone API for windows, focus, workspaces, monitors,
layer surfaces, input, shortcuts, animation, permissions, portals, and session
lifecycle. It is a target contract, not an implementation claim. GNOME Shell
UI, script and feature registries, widget creation, and output mode/profile
configuration are outside this API. Output mode changes remain with the
session display-configuration interface, which owns validation and rollback.

The API owns:

- Window, focus, workspace, monitor, input, shortcut, permission, and session state.
- Policy and declarative configuration.
- Typed operations that ask the compositor or session supervisor to perform
  work.
- Stable events that shell clients and Lua configuration can observe.

The API does not own shell presentation. Panels, docks, launchers, notifications,
OSDs, overview surfaces, and other UI are ordinary client windows, usually
Wayland layer-shell surfaces. Lua does not create widgets, draw surfaces, or
replace Wayland protocols.

Mutter owns windows, input devices, focus, geometry, rendering, and compositor
protocols. Gnoblin owns policy and the public control contract. Compositor
operations cross a narrow versioned interface; the Lua API does not expose
Mutter objects or private C types.

### Shell-client coverage

The target covers the window-manager calls used by Bingux's current dock,
launcher, switcher, window menu, and workspace selector:

| Shell need                                                                                        | Gnoblin API                                                                  |
| ------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| List and group windows; track creation, closure, focus, attention, workspace, and monitor changes | `gnoblin.windows.list()`, `Window` properties, and `gnoblin.window.*` events |
| Restore a minimized window or activate a user-selected window                                     | `window:restore()` and `window:focus(context)`                               |
| Close, minimize, maximize, move, resize, pin above, or keep a window on all workspaces            | Typed `Window` methods and setters                                           |
| Show and switch workspaces; move a window to another workspace                                    | `gnoblin.workspaces.*`, `Workspace` methods, and workspace events            |
| Inspect outputs and shell-owned layer surfaces                                                    | `gnoblin.monitors.list()` and `gnoblin.layers.list()`                        |

This is a control surface for shell clients. It does not replace the Wayland
protocols through which those clients create and operate their own UI windows.

## Design rules

1. Keep one public namespace: `gnoblin`. Config declarations, runtime state,
   operations, and events use the same vocabulary.
2. Use nouns for state collections and objects: `gnoblin.windows`,
   `gnoblin.workspaces`, `Window`, `Workspace`.
3. Use verbs for changes: `window:focus(context)`, `window:set_above(true)`,
   `workspace:rename("Web")`.
4. Read state from named properties on returned records. Do not require a
   `get_config()`, `get_window()`, or generic `action()` dispatcher.
5. Use stable IDs for identity. A workspace number is its current position,
   not its identity.
6. Make operations typed, validate their arguments, and report asynchronous
   completion and errors.
7. Keep Gnoblin events stable and typed. Raw Mutter signals remain an explicitly
   unstable, compositor-specific escape hatch.
8. Use snake_case for Lua properties and method names. Event names use
   lowercase dotted names with hyphens only where an existing event already
   establishes that spelling.

## Runtime and operation model

The compositor and the Lua runtime may run in separate processes. Lua code
must not depend on that process split. The same operation names and schemas are
used by Lua calls and the local shell-client control interface; remote clients
cannot execute arbitrary Lua.

State reads return immutable snapshots from Gnoblin's latest compositor state.
Each record includes a monotonically increasing `revision`. A snapshot does
not change after it is returned; subscribe to events or read a new snapshot to
observe later state.

Every mutating call returns an `Operation` handle. It has:

| Property or method       | Type                                             | Meaning                                                                                                                                                  |
| ------------------------ | ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`                     | integer                                          | Session-unique request identifier.                                                                                                                       |
| `method`                 | string                                           | Canonical operation name, such as `window.set_above`.                                                                                                    |
| `status`                 | One of `"pending"`, `"succeeded"`, or `"failed"` | Current operation state.                                                                                                                                 |
| `value`                  | any or `nil`                                     | Result after success.                                                                                                                                    |
| `error`                  | `Error` or `nil`                                 | Failure after rejection.                                                                                                                                 |
| `:on_complete(callback)` | `Subscription`                                   | Call callback once with `(value, error)`; if already complete, call it on the next main-loop turn. Unsubscribe before that turn to prevent the callback. |

Operations are not cancellable once dispatched. A failed request does not
change state. A successful request means the compositor accepted and applied
the requested change; a later user or policy action may change it again.
Completion is also emitted as `gnoblin.operation.completed` for clients that
use events instead of retaining an `Operation` handle.

Queries are local snapshot reads and return immediately. Mutations are queued
for the compositor or supervisor and complete asynchronously. Lua callbacks
run on the Gnoblin event loop and must not block it. An operation requested
from a callback is dispatched only after that callback returns.

Argument tables reject unknown fields. Optional fields are omitted rather than
set to `nil`. Errors have a stable `code`, human-readable `message`, and
optional `details` table:

Each method called on a record implicitly uses that record's stable `id`.
The wire request includes the ID explicitly, for example
`window:set_above(true)` maps to `window.set_above({id = window.id, enabled = true})`.
For `window:focus(context)`, `window:begin_move(context)`, and
`window:begin_resize(edge, context)`, the operation arguments contain the
window ID and any method-specific fields. The opaque context travels in the
authenticated request envelope; Lua callers do not serialize it themselves.
If the record's object has closed or been removed, the operation fails with
`not_found`.

| Error code         | Meaning                                                                      |
| ------------------ | ---------------------------------------------------------------------------- |
| `invalid_argument` | A field is missing, has the wrong type, or is out of range.                  |
| `not_found`        | The requested window, workspace, monitor, device, or grant no longer exists. |
| `unsupported`      | The current compositor or session cannot perform this operation.             |
| `denied`           | Session policy or security state rejects the operation.                      |
| `busy`             | A mutually exclusive operation, such as shortcut capture, is active.         |
| `cancelled`        | The user or session lifecycle cancelled the operation.                       |
| `timed_out`        | The operation reached its deadline.                                          |
| `unavailable`      | The compositor or session service is not ready.                              |
| `internal`         | An unexpected implementation error occurred.                                 |

`Error` fields are `code`, `message`, and optional structured
`details`. Details are method-specific and contain only values safe for the
caller to inspect.

The API contract requires the implementation to authorize state-changing
operations. The logged-in user's processes share an operating-system identity;
the session socket is therefore not a security boundary between mutually
untrusted processes running as that user. The intended external callers are
shell clients and local session tools. The API does not promise to isolate
applications running under the same user. Window activation still follows the
focus rules below: a client name or process ID alone must never bypass Mutter's
activation checks. The wire protocol must carry verifiable user-interaction
context for shell requests to activate a selected window; its encoding is a
transport detail and is not exposed as an input serial or raw activation token
to Lua.

## Configuration API

Configuration is declarative and merges into the current document while files
load. `gnoblin.settings` is the read-only view of the committed settings;
`gnoblin.configure` writes validated changes for this session. Its detached
snapshot uses public snake_case names and includes a `revision` that advances
when committed setting values change. No-op events and identical reloads keep
the same revision. The revision exists only on the snapshot and is never merged
into the configuration document. Neither runtime changes nor reloads rewrite
the user's files.
`gnoblin.config` remains a compatibility view for existing configs while
callable runtime methods use the separate `gnoblin.runtime` namespace.

### Current Lua globals

| Name                                         | Signature or value                                          | Current behavior and target                                                                                                                                                                       |
| -------------------------------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `gnoblin.configure`                          | callable table: `gnoblin.configure(settings)`               | **Current and retained.** Merge public snake_case settings into the config document.                                                                                                              |
| `gnoblin.configure.shortcuts`                | named entry view                                            | **Current and retained.** Read and edit named shortcut entries.                                                                                                                                   |
| `gnoblin.configure.autostart`                | named entry view                                            | **Current and retained.** Read and edit named autostart entries.                                                                                                                                  |
| `gnoblin.config`                             | mutable config table                                        | **Current; compatibility view.** Keys use normalized internal hyphenated names. Prefer `gnoblin.configure`.                                                                                       |
| `gnoblin.settings`                           | read-only property                                          | **Current.** Detached snapshot of committed settings with public snake_case names and a non-persistent `revision`; available in native and Shell-backed Lua runtimes after initial config commit. |
| `gnoblin.focus.policy`                       | read-only property                                          | **Current.** Immutable focus-preference snapshot with the committed settings revision in native and Shell-backed Lua runtimes.                                                                    |
| `gnoblin.snapshot()`                         | `() -> Settings`                                            | **Current; compatibility only.** Returns a copy of the mutable config view. Prefer `gnoblin.settings` for reads.                                                                                  |
| `gnoblin.load(path)`                         | `(string) -> true`                                          | **Current; retained.** Load a relative file or glob in the current config context.                                                                                                                |
| `gnoblin.array(values)`                      | `(table) -> table`                                          | **Current; retained.** Mark a Lua table as an array where empty-table shape would otherwise be ambiguous.                                                                                         |
| `gnoblin.on(name, callback)`                 | `(string, function) -> Subscription`                        | **Current compatibility alias.** Prefer `gnoblin.events.on`.                                                                                                                                      |
| `gnoblin.events.on(name, callback)`          | `(string, function) -> Subscription`                        | **Current.** Register an event callback and return an unsubscribe handle.                                                                                                                         |
| `gnoblin.events.once(name, callback)`        | `(string, function) -> Subscription`                        | **Current.** Remove the callback before its first invocation.                                                                                                                                     |
| `gnoblin.events.mutter.on(name, callback)`   | `(MutterEventName, function) -> Subscription`               | **Current.** Subscribe to an unstable Mutter event; the name must start with `mutter.`.                                                                                                           |
| `gnoblin.events.mutter.once(name, callback)` | `(MutterEventName, function) -> Subscription`               | **Current.** Subscribe to one unstable Mutter event; the name must start with `mutter.`.                                                                                                          |
| `gnoblin.shortcuts.actions(group?)`          | `(group?: "wm"                                              | "mutter"                                                                                                                                                                                          | "wayland") -> ShortcutAction[]` | **Current.** Read available built-in keybinding actions and schema defaults. |
| `gnoblin.shortcuts.list()`                   | `() -> ShortcutState[]`                                     | **Current; native runtime.** Read the configured shortcuts registered by the compositor.                                                                                                          |
| `gnoblin.shortcuts.capture(options?)`        | `({timeout?: integer 1–60}) -> Operation<CapturedShortcut>` | **Current; native runtime only.** Capture one normalized accelerator; default timeout is 30 seconds and Escape cancels.                                                                           |

`CapturedShortcut` contains the normalized GTK accelerator string in
`accelerator`. Bare Super is returned as `"Super"`; Escape cancels. Capture is
rejected while the session is locked, another capture is active, Mutter has an
input-capture session, or a compositor stage grab is active. Key events are
consumed by Mutter during capture and are not sent to Lua.
| `gnoblin.windows` | `list(filter?)`, `focused()`, `by_id(id)` | **Current; native runtime only.** Read-only revisioned window snapshot records. |
| `gnoblin.workspaces` | `list()`, `active()`, `by_id(id)` | **Current; native runtime only.** Read-only revisioned workspace snapshot records. |
| `gnoblin.monitors` | `list()`, `primary()` | **Current; native runtime only.** Read-only revisioned monitor snapshot records. |
| `gnoblin.layers` | `list(filter?)` | **Current subset; native runtime only.** Read-only revisioned layer-surface records; filters match `monitor_id`, `namespace`, and `layer` exactly. |
| `gnoblin.input` | `devices()`, `sources()`, `current_source()`, `select_source(selector)` | **Current subset; native runtime only.** Read-only device/source snapshots and XKB source selection. |
| `gnoblin.listeners` | map of event names to callback arrays | **Current; inspect only.** Do not edit this table directly. |
| `gnoblin.window_rule(rule)` | `(WindowRule) -> nil` | **Current and retained.** Append a window or layer matching rule. |
| `gnoblin.permission_rule(rule)` | `(PermissionRule) -> nil` | **Current and retained.** Append a portal permission rule. |
| `gnoblin.shortcut(entry)` | `(Shortcut) -> nil` | **Current compatibility helper.** Prefer `gnoblin.configure {shortcuts = {...}}`. |
| `gnoblin.animation(entry)` | `(Animation) -> nil` | **Current and retained.** Declare a named compositor animation. |
| `gnoblin.autostart(entry)` | `(Autostart) -> nil` | **Current compatibility helper.** Prefer `gnoblin.configure {autostart = {...}}`. |
| `gnoblin.remove_shortcut(name)` | `(string) -> nil` | **Current compatibility helper.** Prefer an entry with `enable = false`. |
| `gnoblin.remove_autostart(name)` | `(string) -> nil` | **Current compatibility helper.** Prefer an entry with `enable = false`. |
| global `require(name)` | `(string) -> any` | **Current custom loader.** Loads a local module beside the calling file or under its `lua/` directory; it is not Lua's installed-module search path. |

Configuration loading removes the standard Lua `os`, `io`, `debug`,
`package`, `dofile`, and `loadfile` globals. Do not use these as public
Gnoblin APIs.

### Current configuration document properties

The current `gnoblin.configure` document accepts these top-level properties.
Each is optional; omitted sections keep earlier values. The table records the
current hybrid implementation so the migration can account for every field.
Shell-backed settings are removed from the standalone target; they do not
imply that a Shell compatibility layer will ship.

| Property            | Shape                                                    | Public schema                                                      | Standalone target                                                                          |
| ------------------- | -------------------------------------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------ |
| `shell`             | GNOME Shell feature and animation preferences            | [shell settings](../docs/config/configure/shell.md)                | Remove; shell projects own UI preferences. Gnoblin owns compositor animation declarations. |
| `keybindings`       | Existing desktop keybinding overrides                    | [keybindings](../docs/config/configure/keybindings.md)             | Remove GNOME Shell actions; retain supported Mutter and Wayland actions as typed bindings. |
| `window_management` | Focus, placement, titlebar, and window-rule settings     | [window management](../docs/config/configure/window_management.md) | Retain Gnoblin-owned focus, window, and workspace policy.                                  |
| `compositor`        | Rendering and compositor behavior                        | [compositor settings](../docs/config/configure/compositor.md)      | Retain.                                                                                    |
| `input`             | Keyboard, pointer, touchpad, tablet, and stylus settings | [input settings](../docs/config/configure/input.md)                | Retain.                                                                                    |
| `input_sources`     | Input source definitions and selection policy            | [input sources](../docs/config/configure/input_sources.md)         | Retain.                                                                                    |
| `touchpad_gestures` | Gesture matching and actions                             | [touchpad gestures](../docs/config/configure/touchpad_gestures.md) | Retain Gnoblin-owned gesture behavior.                                                     |
| `permissions`       | Portal policy and permission rules                       | [permissions](../docs/config/configure/permissions.md)             | Retain.                                                                                    |
| `workspaces`        | Ordered persistent workspace declarations                | [window management](../docs/config/configure/window_management.md) | Retain.                                                                                    |
| `layer_shell`       | Layer-shell policy and supported behavior                | [layer shell](../docs/config/configure/layer_shell.md)             | Retain compositor policy; shell surfaces remain client-owned.                              |
| `protocols`         | Optional Wayland protocol implementations                | [protocols](../docs/config/configure/protocols.md)                 | Retain.                                                                                    |
| `frame_renderers`   | Window and layer frame effects                           | [frame renderers](../docs/config/configure/frame_renderers.md)     | Retain compositor frame rendering; UI stays in clients.                                    |
| `cursor`            | Cursor appearance and behavior                           | [cursor](../docs/config/configure/cursor.md)                       | Retain.                                                                                    |
| `shortcuts`         | Named command or compositor-action bindings              | [shortcuts](../docs/config/configure/shortcuts.md)                 | Retain Gnoblin shortcuts; remove Shell-specific actions.                                   |
| `autostart`         | Named session commands                                   | [autostart](../docs/config/configure/autostart.md)                 | Retain under the Gnoblin session supervisor.                                               |

The linked pages give the full current nested schema. The target keeps
Gnoblin-owned policy and removes GNOME Shell preferences and actions. Gnoblin's
Lua runtime owns Gnoblin session behavior; external shell projects own their
UI and its configuration.

### Declaration types

| Record           | Required fields                                             | Optional fields                                                                                                                  |
| ---------------- | ----------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `WindowRule`     | `match`                                                     | Existing compositor-owned fields in the [window-rule schema](../docs/config/window_rule.md), without shell-only animation events |
| `PermissionRule` | `name`, `match`, nonempty `capabilities`, `level`           | `monitors` for `screen-cast` or `remote-desktop`; `devices` and `clipboard` for `remote-desktop`                                 |
| `Shortcut`       | `name`, `binding`, and exactly one of `command` or `action` | `trigger`, `capture_input`, `enable`                                                                                             |
| `Animation`      | `name`, `event`, and `from`/`to` or `keyframes`             | `enable`, `duration`, `ease`, `origin`, `target`                                                                                 |
| `Autostart`      | `name`, nonempty `command` array                            | `when = "on_login"`, `enable`                                                                                                    |

The precise fields, enum members, and defaults are validated by the current
configuration schema. The target API will reuse those schema types rather
than create a second, incompatible spelling. In particular,
`PermissionRule` and `WindowRule` keep their existing match and scope
semantics; runtime operations act on resolved object IDs rather than
re-evaluating configuration rules.

## Target Lua API

All names in this section are **proposed** unless they are also listed in the
current inventory below. A returned record is a read-only snapshot. Its
methods are convenience wrappers over the canonical typed operation names
listed in the method tables.

The target has no GNOME Shell feature registry, script manager, event source,
or adapter. The current Shell-backed APIs and events documented later are
migration input only and are removed as their behavior moves into Gnoblin or
independent shell clients.

### Root and events

| Member                                       | Signature                     | Result                                                                                               |
| -------------------------------------------- | ----------------------------- | ---------------------------------------------------------------------------------------------------- |
| `gnoblin.settings`                           | read-only property            | `Settings` snapshot with a `revision`; available after the first config commit in both runtime paths |
| `gnoblin.version()`                          | `()`                          | `Version`                                                                                            |
| `gnoblin.capabilities.list()`                | `()`                          | `Capability[]`                                                                                       |
| `gnoblin.events.on(name, callback)`          | `(string, function)`          | `Subscription`                                                                                       |
| `gnoblin.events.once(name, callback)`        | `(string, function)`          | `Subscription`                                                                                       |
| `gnoblin.events.mutter.on(name, callback)`   | `(MutterEventName, function)` | `Subscription`; unstable Mutter events                                                               |
| `gnoblin.events.mutter.once(name, callback)` | `(MutterEventName, function)` | `Subscription`; one unstable Mutter event                                                            |
| `subscription:unsubscribe()`                 | `()`                          | `nil`; safe to call more than once                                                                   |

`gnoblin.on` is a temporary alias for `gnoblin.events.on` during API
migration and is removed after clients migrate.
Callbacks receive one event record with `name`, `sequence`, `time`, and
the event-specific fields. A callback error is logged and does not prevent
other listeners from running.

### Focus and activation

| Lua call or property             | Arguments                                   | Result                 | Canonical operation |
| -------------------------------- | ------------------------------------------- | ---------------------- | ------------------- |
| `gnoblin.focus.policy`           | read-only property                          | `FocusPolicy` snapshot | state read          |
| `gnoblin.focus.history(filter?)` | `workspace_id?`, `monitor_id?`, `limit?`    | `Window[]`             | native state read   |
| `window:focus(context)`          | `FocusContext` from a user-originated event | `Operation<Window>`    | `window.focus`      |

The focus policy uses the existing `window_management` settings.
The read-only `gnoblin.focus.policy` property returns a `FocusPolicy` with
these fields:

| Field                          | Type and accepted values             | Current default; standalone proposal and effect                                                                                     |
| ------------------------------ | ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| `focus_mode`                   | `"click"`, `"sloppy"`, or `"mouse"`  | `"click"`; controls whether pointer entry changes focus.                                                                            |
| `focus_new_windows`            | `"smart"` or `"strict"`              | Current: `"smart"`. Proposed standalone default: `"strict"` to prevent unsupported app requests from interrupting the current task. |
| `raise_on_click`               | boolean                              | `true`; raise a window when clicked.                                                                                                |
| `auto_raise`                   | boolean                              | `false`; raise the focused window automatically.                                                                                    |
| `focus_change_on_pointer_rest` | boolean                              | `false`; delay pointer-follow focus until the pointer rests.                                                                        |
| `auto_raise_delay`             | integer from 0 to 10000 milliseconds | `500`; delay before automatic raise.                                                                                                |
| `revision`                     | integer                              | Revision of this policy snapshot.                                                                                                   |

Change the effective policy through the same declarative API used by
configuration:

```lua
gnoblin.configure {
    window_management = {focus_new_windows = "strict"},
}
```

`"click"` focuses a window when clicked; pointer movement alone does
not change focus. `"sloppy"` focuses a window when the pointer enters it and
returns to the most recent eligible window when the pointer leaves all
windows. `"mouse"` focuses on pointer entry and clears focus when the pointer
leaves all windows.

`"smart"` usually focuses a new window even when its activation context is
weak. This is convenient for apps that do not provide activation metadata, but
can let an app interrupt the current task. `"strict"` uses Mutter's activation
and transient-parent checks for application-originated requests. Mutter honors
a request when its recent user or launch context is valid; an unapproved
request remains unfocused and may mark the window as demanding attention.
Strict mode can expose missing activation support in apps and toolkits, so
`"smart"` remains an opt-in compatibility choice. A valid `FocusContext` for
an explicit shell selection follows the user-activation path in either mode.
The current checkout defaults to `"smart"`; this proposal recommends
`"strict"` for the standalone session.

`gnoblin.focus.history()` is available in the native runtime. It returns live
window snapshots, including minimized windows, ordered by most-recently
focused events observed by this runtime. The current focused window seeds the
order when a snapshot is installed. Other windows already open at startup, or
new windows not yet focused, follow snapshot order until a focus event places
them in the MRU order. Closed windows are removed. Shell-backed sessions do
not provide this read. Filters match workspace and monitor IDs; `limit` is
1–256 and defaults to 50. A shell client can display this list and call
`window:focus(context)` when the user selects an entry. `FocusContext` is an
opaque, single-use value issued for a real user action. The compositor
validates it and its lifetime;
Lua cannot construct or inspect one. Missing, expired, already-used, or
mismatched context fails with `denied` and does not change keyboard focus. A
Lua callback that handles a user-originated Gnoblin event receives the context
on the event record. A shell client handling its own pointer or keyboard event
supplies standard Wayland activation evidence through its local client
binding, which converts it to the same opaque context. Calls from timers,
startup hooks, or application callbacks have no context and cannot force
focus. The context is not a client-chosen string.

Application processes do not call this API to claim focus. Wayland
applications request activation with XDG Activation tokens, and Mutter decides
whether to honor each request. The protocol permits the compositor to ignore
invalid or unwanted tokens. Mutter validates activation context before
Gnoblin reports the resulting focus and attention state; raw activation tokens
and raw input serials are not exposed to Lua. Lua callbacks do not
synchronously approve or veto application requests. A shell client may call
`window:focus(context)` in response to an explicit user selection; the request
uses Mutter's normal activation path and cannot mint its own context.

See the [XDG Activation protocol](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/xdg-activation/xdg-activation-v1.xml),
the [Mutter window API](https://gnome.pages.gitlab.gnome.org/mutter/meta/class.Window.html),
and the [current window management settings](../docs/config/configure/window_management.md).

### Windows

#### Collection functions

| Lua call                        | Arguments                                                       | Result          | Canonical operation |
| ------------------------------- | --------------------------------------------------------------- | --------------- | ------------------- |
| `gnoblin.windows.list(filter?)` | `app_id?`, `title?`, `focused?`, `workspace_id?`, `monitor_id?` | `Window[]`      | `window.list`       |
| `gnoblin.windows.focused()`     | none                                                            | `Window or nil` | state read          |
| `gnoblin.windows.by_id(id)`     | stable window ID                                                | `Window or nil` | state read          |

`title` is a case-insensitive substring filter. Other supplied filters are
exact matches. Empty filter matches all managed application windows. Layers
are returned separately by `gnoblin.layers.list`.

#### Window properties

Every property below is present on a `Window` snapshot; a value may be
`nil` when the protocol or client does not supply it.

| Property            | Type     | Meaning                                                                                                           |
| ------------------- | -------- | ----------------------------------------------------------------------------------------------------------------- |
| `id`                | string   | Stable identity for the lifetime of this managed window.                                                          |
| `revision`          | integer  | State revision when the snapshot was created.                                                                     |
| `title`             | string   | Current window title.                                                                                             |
| `app_id`            | string?  | Desktop application ID.                                                                                           |
| `gtk_app_id`        | string?  | GTK application ID, when supplied.                                                                                |
| `rule_app_id`       | string?  | Normalized identity used for Gnoblin window rules.                                                                |
| `wm_class`          | string?  | X11 compatibility class, when supplied.                                                                           |
| `role`              | string?  | Window role, when supplied.                                                                                       |
| `type`              | integer  | Mutter `MetaWindowType`; see its value mapping below.                                                             |
| `workspace_id`      | string?  | Stable ID of the current workspace.                                                                               |
| `workspace_number`  | integer? | Current one-based workspace position.                                                                             |
| `monitor_id`        | string?  | Stable ID of the current monitor.                                                                                 |
| `frame`             | `Rect`   | Current logical-pixel frame geometry.                                                                             |
| `focused`           | boolean  | Whether this is the keyboard-focused window.                                                                      |
| `demands_attention` | boolean  | Whether Mutter marks this window as requesting user attention. This does not identify why it requested attention. |
| `minimized`         | boolean  | Whether the window is minimized.                                                                                  |
| `maximized`         | boolean  | Whether the window is maximized.                                                                                  |
| `fullscreen`        | boolean  | Whether the window is fullscreen.                                                                                 |
| `above`             | boolean  | Whether the window is kept above ordinary windows.                                                                |
| `sticky`            | boolean  | Whether the window is visible on all workspaces.                                                                  |
| `modal`             | boolean  | Whether the window is modal.                                                                                      |
| `resizable`         | boolean  | Whether the client permits user resizing.                                                                         |
| `closable`          | boolean  | Whether the compositor permits closing the window.                                                                |
| `movable`           | boolean  | Whether the compositor permits moving the window.                                                                 |
| `minimizable`       | boolean  | Whether the compositor permits minimizing it.                                                                     |
| `maximizable`       | boolean  | Whether the compositor permits maximizing it.                                                                     |

The native preview returns Mutter's `MetaWindowType` integer. Values follow
the pinned Mutter enum: `0` normal, `1` desktop, `2` dock, `3` dialog,
`4` modal dialog, `5` toolbar, `6` menu, `7` utility, `8` splash screen,
`9` dropdown menu, `10` popup menu, `11` tooltip, `12` notification,
`13` combo box, `14` drag-and-drop, and `15` other override-redirect window.
This describes the client surface type; it does not prescribe how a shell
presents or groups the window.

Window properties are read-only values. Mutations use named methods so they
can return an `Operation` and report errors. For example,
`window:set_above(true)` writes the property whose current value is
`window.above`; Lua assignment such as `window.above = true` is not used for
compositor changes.

#### Window methods

| Lua method                             | Arguments                      | Canonical operation        | Effect                                                                                                            |
| -------------------------------------- | ------------------------------ | -------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `:focus(context)`                      | `FocusContext`                 | `window.focus`             | Restore and raise the window, activate its workspace, and request keyboard focus for an explicit shell selection. |
| `:close()`                             | none                           | `window.close`             | Ask the client to close using the normal protocol.                                                                |
| `:minimize()`                          | none                           | `window.minimize`          | Minimize if supported.                                                                                            |
| `:toggle_minimize()`                   | none                           | `window.toggle_minimize`   | Minimize a normal window or restore a minimized one.                                                              |
| `:restore()`                           | none                           | `window.restore`           | Unminimize and unmaximize as needed to show the window.                                                           |
| `:set_above(enabled)`                  | boolean                        | `window.set_above`         | Set the above state idempotently.                                                                                 |
| `:set_sticky(enabled)`                 | boolean                        | `window.set_sticky`        | Set visibility across workspaces idempotently.                                                                    |
| `:set_maximized(enabled)`              | boolean                        | `window.set_maximized`     | Set the maximized state idempotently.                                                                             |
| `:set_fullscreen(enabled)`             | boolean                        | `window.set_fullscreen`    | Set the fullscreen state idempotently.                                                                            |
| `:move(position)`                      | `{x, y}`                       | `window.move`              | Move the frame in logical compositor coordinates.                                                                 |
| `:resize(size)`                        | `{width, height}`              | `window.resize`            | Request a size in logical pixels, respecting client constraints.                                                  |
| `:move_to_workspace(target, options?)` | `WorkspaceSelector`; `follow?` | `window.move_to_workspace` | Move this window; optionally activate the destination.                                                            |
| `:move_to_monitor(target)`             | `MonitorSelector`              | `window.move_to_monitor`   | Move this window to a monitor.                                                                                    |
| `:begin_move(context)`                 | `FocusContext`                 | `window.begin_move`        | Start Mutter's keyboard move grab with a trusted shortcut context.                                                |
| `:begin_resize(edge, context)`         | `ResizeEdge`, `FocusContext`   | `window.begin_resize`      | Start Mutter's keyboard resize grab at the selected edge.                                                         |

`move` and `resize` are programmatic placement requests. Interactive
movement is separate and lets Mutter apply pointer constraints, grabs, and
client decorations correctly. The API does not provide fake UI drag events.

`move` accepts integer coordinates from −100000 to 100000. `resize`
accepts integer width and height from 1 to 32768; the compositor may clamp
them to the client's minimum and maximum sizes.

```lua
local window = gnoblin.windows.focused()
if window and not window.above then
    window:set_above(true):on_complete(function(_, err)
        if err then
            print("Could not keep the window above others: " .. err.message)
        end
    end)
end
```

The generic `window.action(args)` dispatcher is **not** part of the target
API. Each compositor action becomes a typed method or property setter. The
current `menu` action has no compositor equivalent: an external shell client
owns its menu UI and can draw it on a layer-shell surface from the window
snapshot. This preserves discoverable property names such as `above` while
making writes explicit and acknowledgeable.

#### Geometry and selectors

| Type                | Fields                                                                                                   |
| ------------------- | -------------------------------------------------------------------------------------------------------- |
| `Rect`              | `x`, `y`, `width`, `height`: finite logical-pixel numbers                                                |
| `Point`             | `x`, `y`: finite logical-pixel numbers                                                                   |
| `Size`              | `width`, `height`: positive finite logical-pixel numbers                                                 |
| `WorkspaceSelector` | Exactly one of `{id = string}` or `{number = integer}`                                                   |
| `MonitorSelector`   | Stable monitor ID string or `{id = string}`                                                              |
| `ResizeEdge`        | `"north"`, `"south"`, `"east"`, `"west"`, `"north_east"`, `"north_west"`, `"south_east"`, `"south_west"` |

Coordinates use the compositor's logical layout. Monitor transforms and scale
are applied by the compositor; clients must not multiply geometry by scale.

### Workspaces

| Lua call                                | Arguments                                             | Result                     | Canonical operation     |
| --------------------------------------- | ----------------------------------------------------- | -------------------------- | ----------------------- |
| `gnoblin.workspaces.list()`             | none                                                  | `Workspace[]`              | `workspace.list`        |
| `gnoblin.workspaces.active()`           | none                                                  | `Workspace or nil`         | state read              |
| `gnoblin.workspaces.by_id(id)`          | stable workspace ID                                   | `Workspace or nil`         | state read              |
| `gnoblin.workspaces.create(options)`    | `name`, optional `id`, `activate`                     | `Operation<Workspace>`     | `workspace.create`      |
| `gnoblin.workspaces.next()`             | none                                                  | `Operation<Workspace>`     | `workspace.next`        |
| `gnoblin.workspaces.previous()`         | none                                                  | `Operation<Workspace>`     | `workspace.previous`    |
| `workspace:activate()`                  | none                                                  | `Operation<Workspace>`     | `workspace.activate`    |
| `workspace:rename(name)`                | nonempty name, at most 80 characters                  | `Operation<Workspace>`     | `workspace.rename`      |
| `workspace:remove()`                    | none                                                  | `Operation<Workspace>`     | `workspace.remove`      |
| `workspace:move_here(window, options?)` | `Window`, window ID, or `"active"`; optional `follow` | `Operation<WorkspaceMove>` | `workspace.move_window` |

A workspace record has:

| Property       | Type    | Meaning                                                                                |
| -------------- | ------- | -------------------------------------------------------------------------------------- |
| `id`           | string  | Stable session identity. Configured IDs persist across sessions; generated IDs do not. |
| `number`       | integer | Current one-based position, which can change after removal or reordering.              |
| `name`         | string  | Display name.                                                                          |
| `active`       | boolean | Whether it is the active workspace.                                                    |
| `window_count` | integer | Number of managed application windows on it.                                           |
| `persistent`   | boolean | Whether configuration declared it.                                                     |
| `revision`     | integer | Snapshot revision.                                                                     |

Configured IDs match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`. Runtime-created
IDs are session-only and use the `@session-N` form. The active workspace,
configured workspaces, and nonempty workspaces cannot be removed. A move uses
a selector with exactly one of `id` or `number`; `follow` defaults to
`false`.

`workspaces.create({name, id?, activate?})` requires a nonempty name up to
80 characters. If `id` is omitted, Gnoblin creates a session-only ID.
`activate` defaults to `false`. Rename requires a nonempty name up to 80
characters. `WorkspaceMove` contains `window_id`, `workspace_id`,
`follow`, and the resulting `revision`.

### Monitors and layer surfaces

| Lua call                       | Arguments                                                        | Result                              | Canonical operation                                |
| ------------------------------ | ---------------------------------------------------------------- | ----------------------------------- | -------------------------------------------------- |
| `gnoblin.monitors.list()`      | none                                                             | `Monitor[]`                         | cached native monitor snapshot                     |
| `gnoblin.monitors.primary()`   | none                                                             | `Monitor or nil`                    | cached native monitor snapshot                     |
| `gnoblin.layers.list(filter?)` | optional `monitor_id`, `namespace`, `layer` exact string matches | read-only `LayerSurface[]` snapshot | native cached snapshot refreshed from `layer.list` |

`Monitor` fields: string `id` (the canonical active connector name); optional strings `name`, `make`,
`model`, `serial`; booleans `primary` and `enabled`; logical-pixel
numbers `x`, `y`, `width`, and `height`; positive `scale`;
`refresh_rate` in Hz; and integer `revision`. `transform` is one of
`"normal"`, `"90"`, `"180"`, `"270"`, `"flipped"`,
`"flipped-90"`, `"flipped-180"`, or `"flipped-270"`. Identity strings
may be absent when the output does not supply them.

In the current native preview, `monitor.list()` also returns the current
zero-based `index` for compatibility actions. The canonical `id` is the
lexicographically first active connector when a logical monitor combines
cloned outputs. The target API uses `id` for monitor selection because the
index can change when outputs are added or removed. Connector names identify
the active output configuration; changing the connected port or monitor may
change the name used as its ID.

The native preview also returns `enabled = true` and the logical monitor's
`transform`. Mutter-supplied `name`, `make`, `model`, and `serial` fields are
optional. `refresh_rate` is the current mode rate for the canonical connector
and is omitted when that connector has no current mode. Only active logical
monitors are listed, so inactive physical outputs are not available. Each
cached record includes the state `revision`. `gnoblin.monitors.primary()`
returns the primary record or `nil` when none is marked primary. Native monitor
snapshots refresh before lifecycle events are dispatched. The legacy
`monitor.list()` operation remains available for compatibility.

`LayerSurface` fields: string `id`; optional strings `title`, `namespace`,
and `monitor_id`; `layer` is `"background"`,
`"bottom"`, `"top"`, or `"overlay"`; `keyboard_interactive` is
`"none"`, `"on_demand"`, or `"exclusive"`; integer `exclusive_zone`;
`anchor` is an array containing zero or more of `"top"`, `"bottom"`,
`"left"`, and `"right"`; `geometry` is a `Rect`; `mapped` is boolean;
and `revision` is an integer. Mutter currently omits `app_id` because
layer-shell does not provide one. The API inspects client surfaces; it does
not create or draw them.

Monitor mode-setting is intentionally absent from the first target. Output
configuration belongs to the session's display-configuration interface and
must retain its validation and rollback semantics.

### Input and shortcuts

| Lua call                                | Arguments                                          | Result                        | Canonical operation   |
| --------------------------------------- | -------------------------------------------------- | ----------------------------- | --------------------- |
| `gnoblin.input.devices()`               | none                                               | `InputDevice[]`               | `input.devices`       |
| `gnoblin.input.sources()`               | none                                               | `InputSource[]`               | `input.sources`       |
| `gnoblin.input.current_source()`        | none                                               | `InputSource or nil`          | state read            |
| `gnoblin.input.select_source(selector)` | `{type, id}` source selector                       | `Operation<InputSource>`      | `input.select_source` |
| `gnoblin.shortcuts.list()`              | none                                               | `ShortcutState[]`             | `shortcut.list`       |
| `gnoblin.shortcuts.actions(group?)`     | optional group: `"wm"`, `"mutter"`, or `"wayland"` | `ShortcutAction[]`            | state read            |
| `gnoblin.shortcuts.capture(options?)`   | `timeout?` seconds                                 | `Operation<CapturedShortcut>` | `shortcut.capture`    |

`InputDevice` fields: string `id`, `name`, and `device_type`; optional string
`seat` when Mutter provides a seat name;
optional integer `vendor_id` and `product_id`; optional boolean `enabled`;
string-array `capabilities`; and integer `revision`. Current records use a
session-only `input:N` ID and omit `enabled` because Mutter 51 has no safe
enabled-state getter. `device_type` is one of `"pointer"`,
`"keyboard"`, `"extension"`, `"joystick"`, `"tablet"`, `"touchpad"`,
`"touchscreen"`, `"pen"`, `"eraser"`, `"cursor"`, `"pad"`, or
`"unknown"`. `capabilities` contains zero or more of `"pointer"`,
`"keyboard"`, `"touchpad"`, `"touch"`, `"tablet_tool"`, `"tablet_pad"`,
`"trackball"`, and `"trackpoint"`. These Gnoblin spellings normalize the
device types and capabilities Mutter reports; a platform may not provide every
type.
`InputSource` fields: string `id`, `type`, `short_name`, and `name`;
boolean `current`; and integer `revision`. A source selector contains both
`type` and `id`; copy the pair from a listed source record. Native mode lists
and selects configured XKB layouts or variants only. It rejects IBus
selection because the native runtime has no IBus engine client. Shell-backed
compatibility sessions retain the existing XKB and IBus behavior.

`ShortcutState` fields: string `name`; `binding` as one accelerator string
or an array of strings; boolean `enabled`; `trigger` as `"press"` or
`"release"`; exactly one of `command` (a string array) or `action` (a
`ShortcutAction.id`); and integer `revision`. Commands are not exposed with
secret environment values. A configured shortcut emits a typed activation
event; an external shell client decides what UI, if any, to show.

`ShortcutAction` fields: `id` (the stable `group.key` identifier), `group`
(`"wm"`, `"mutter"`, or `"wayland"`), `key` (Lua snake_case key), optional
`description`, and `default_bindings` (the schema's accelerator strings).
Records have no runtime revision because they describe installed schema
metadata. `gnoblin.shortcuts.actions(group?)` reads available string-array
actions and defaults from the installed schemas, so callers do not guess keys
from another GNOME or Mutter release. An omitted group lists all installed
groups; an explicitly requested group errors if its schema is missing. The
standalone target does not register the `gnome:shell` group. These actions can
be bound as shortcuts; they are not a generic runtime action dispatcher.

Shortcut capture accepts a timeout from 1 to 60 seconds, default 30. It fails
with `busy` if another capture is active, the seat is already grabbed, or the
session is locked. Escape cancels capture. A session lock, input-capture
session, or stage grab that begins during capture cancels it before input is
forwarded to that owner.
`CapturedShortcut` contains one `accelerator` string.

### Animations and compositor capabilities

| Lua call                           | Arguments                                                     | Result                        | Canonical operation |
| ---------------------------------- | ------------------------------------------------------------- | ----------------------------- | ------------------- |
| `gnoblin.animations.list()`        | none                                                          | `AnimationInfo[]`             | `animation.list`    |
| `gnoblin.animations.get(name)`     | animation name                                                | `AnimationInfo or nil`        | state read          |
| `gnoblin.animations.preview(spec)` | `name`, `target`, optional `event`, `target_type`, `autoplay` | `Operation<AnimationPreview>` | `animation.preview` |
| `preview:seek(progress)`           | number from 0 to 1                                            | `Operation<AnimationPreview>` | `animation.seek`    |
| `preview:step(milliseconds)`       | integer from 1 to 60000                                       | `Operation<AnimationPreview>` | `animation.step`    |
| `preview:play()`                   | none                                                          | `Operation<AnimationPreview>` | `animation.play`    |
| `preview:pause()`                  | none                                                          | `Operation<AnimationPreview>` | `animation.pause`   |
| `preview:stop()`                   | none                                                          | `Operation<nil>`              | `animation.stop`    |

`AnimationInfo` fields mirror the Gnoblin compositor animation declaration:
string `name`, boolean `enable`, event-name string `event`, integer
`duration` in milliseconds, `ease` as a named curve or cubic Bézier record,
`from` and `to` as event-specific numeric property maps, `keyframes` as an
ordered array of frames, `origin` as a named pivot or normalized coordinate
pair, optional string `target`, and integer `revision`. Names are 1–80 ASCII
letters, numbers, underscores, or hyphens. Duration is an integer from 0 to
10000 milliseconds. Events are `"minimize"`, `"restore"`, `"open"`,
`"close"`, `"dialog-open"`, `"dialog-close"`, `"layer-open"`,
`"layer-close"`, `"workspace-switch"`, `"shadow-change"`, `"resize"`,
`"tile-preview-open"`, `"tile-preview-close"`, `"dialog-dim"`, and
`"dialog-undim"`. Easing curves are `"linear"`,
`"ease-in-quad"`, `"ease-out-quad"`, `"ease-in-out-cubic"`,
`"ease-in-cubic"`, `"ease-out-cubic"`, `"ease-out-expo"`, and
`"ease-out-back"`, or `{type = "cubic-bezier", x1, y1, x2, y2}`.
Pivots are `"center"`, `"top-left"`, `"top-center"`, `"top-right"`,
`"bottom-left"`, `"bottom-center"`, and `"bottom-right"`, or a normalized
coordinate pair. Property maps vary by event: window, dialog, and layer events accept
`x`, `y`, `scale`, `scale_x`, `scale_y`, `rotation`, and `opacity`;
tile-preview events accept `x`, `y`, `width`, `height`, and `opacity`;
workspace-switch, resize, shadow-change, dialog-dim, and dialog-undim accept
`progress`. Keyframes contain 2–128 ordered frames with endpoints at progress
0 and 1. The target removes `console-open`, `console-close`, and
`layer-companion-close` because those describe shell-client UI. See the
[animation guide](../docs/guides/animations.md) for current value units and
frame rules. Preview target types are `"window"`, `"layer"`, or `"namespace"`;
a preview target is `"active"`, a stable window ID, layer ID, or namespace
selected by its target type.

`AnimationPreview` fields: string `id`, `name`, `event`, `target`, and
`target_type`; normalized numeric `progress` from 0 to 1; boolean `playing`;
and integer `revision`.
`Capability` fields: string `id` and `description`; boolean `available`;
optional string `reason`; and integer `revision`. Capabilities report which
APIs or protocols are supported; version negotiation reports methods and
events. These are compositor and protocol capabilities, not a Shell feature
registry.

The API controls compositor animation specifications and reports compositor
capabilities. It does not expose draw calls, shaders, arbitrary Mutter
objects, or shell widgets.

### Permissions, privacy, and portals

| Lua call                                          | Arguments                       | Result               | Canonical operation   |
| ------------------------------------------------- | ------------------------------- | -------------------- | --------------------- |
| `gnoblin.privacy.state()`                         | none                            | `PrivacyState`       | state read            |
| `gnoblin.permissions.policy()`                    | none                            | `PermissionPolicy`   | state read            |
| `gnoblin.permissions.check(capability, identity)` | capability and identity strings | `PermissionDecision` | state read            |
| `gnoblin.portals.grants()`                        | optional `kind`                 | `PortalGrant[]`      | state read            |
| `grant:revoke()`                                  | none                            | `Operation<nil>`     | `portal.grant.revoke` |

`PrivacyState` contains an `available` record with boolean fields
`screen_sharing`, `microphone_in_use`, `camera_in_use`, and
`location_in_use`, plus a `revision`. Each matching activity field is an
optional boolean. Gnoblin omits it when its source is unavailable; consumers
must not interpret unavailable state as inactive.

`PermissionPolicy` fields are `default` (one of `"default"`, `"ask"`,
or `"deny"`), `rules` (ordered `PermissionRule[]`), and `revision`.
Global `"allow"` is invalid; allow decisions must name an explicit matching
rule. A `PermissionRule` has a unique `name` of 1–80 ASCII letters, digits,
periods, underscores, or hyphens; a `match` regular expression from 1–512
characters; a nonempty `capabilities` array; and a `level` of `"default"`,
`"ask"`, `"allow"`, or `"deny"`. Matching uses verified portal identities
such as `app-id:org.example.App` or `host-exe:/usr/bin/example`, never a window
title or Wayland `app_id` supplied by an arbitrary client.
`PermissionDecision` fields: `level`, `rule`, `monitors` (string array),
`devices` (array of `"keyboard"`, `"pointer"`, or `"touchscreen"`),
`clipboard` (boolean), and `revision`.

`PermissionLevel` is `"default"`, `"ask"`, `"allow"`, or `"deny"`.
Permission capabilities are `"screen-cast"`, `"remote-desktop"`,
`"input-capture"`, `"screenshot"`, and `"access"`. Each accepts only
its documented scope fields. `monitors` is a nonempty array of `"primary"`
or exact connector names matching `^[A-Za-z0-9_.:-]{1,80}$`; it applies to
`screen-cast` and `remote-desktop`. `devices` may contain `"keyboard"`,
`"pointer"`, or `"touchscreen"`, and `clipboard` is boolean; both apply only
to `remote-desktop`. Scope fields for other capabilities are rejected.

`PortalGrant` fields: `id`, `kind`, `requester`, `devices`,
`clipboard`, `has_screen_streams`, `created_at`, `revision`. `kind` is
`"screen-cast"` or `"remote-desktop"`; `requester` is a verified portal
identity; `devices` is an array of `"keyboard"`, `"pointer"`, and
`"touchscreen"`; `clipboard` and `has_screen_streams` are booleans;
`created_at` is a Unix timestamp in milliseconds. Revocation requires a grant
record from the current session; stale grants fail with `not_found`.

### Session, launch feedback, and reload

| Lua call                                      | Arguments                                      | Result              | Canonical operation          |
| --------------------------------------------- | ---------------------------------------------- | ------------------- | ---------------------------- |
| `gnoblin.session.status()`                    | none                                           | `SessionStatus`     | state read                   |
| `gnoblin.session.lock()`                      | none                                           | `Operation<nil>`    | `session.lock`               |
| `gnoblin.session.logout()`                    | none                                           | `Operation<nil>`    | `session.logout`             |
| `gnoblin.session.restart_compositor(reason?)` | optional reason string, at most 256 characters | `Operation<nil>`    | `session.restart_compositor` |
| `gnoblin.runtime.reload_config()`             | none                                           | `Operation<nil>`    | `runtime.reload_config`      |
| `gnoblin.launches.list()`                     | none                                           | `Launch[]`          | state read                   |
| `gnoblin.launches.begin(options)`             | `token`, `application`, optional `timeout_ms`  | `Operation<Launch>` | `launch.begin`               |
| `gnoblin.launches.end(token)`                 | launch token                                   | `Operation<nil>`    | `launch.end`                 |

`Version` fields: string `gnoblin` (Gnoblin release), `gnome` (the GNOME
upstream release line used as the source baseline, not a running Shell),
`mutter` (compositor version), `lua` (runtime version), and `api` (Gnoblin
control API version); plus sanitized string fields `git_remote`, `git_sha`,
and `build_id`. Remote credentials and URL userinfo must be removed from
`git_remote`. The implementation reads the installed
`share/gnoblin/version.ini` sidecar, `GNOBLIN_VERSION_METADATA_FILE`, the
executable prefix, or system XDG data directories. Fields absent from the
metadata are the string `"unknown"`. `api` comes from the compiled
native-control API constants when available, and `lua` comes from the linked
Lua runtime. The current identity generator does not emit a separate
`build_id`, so it is `"unknown"` unless the metadata supplies one.

`SessionStatus` fields: `state`, `locked`, `compositor_state`,
`compositor_pid`, `active_workspace_id`,
`focused_window_id`, `revision`. State values are `"starting"`,
`"running"`, `"restarting"`, `"stopping"`, and `"failed"`.
`state` is the supervisor state; `compositor_state` describes only the
compositor process. `compositor_pid` is `nil` before startup or after exit.
`locked` reports whether the session is protected by its active lock surface.

`Launch` fields: `token`, `application`, `started_at`, `timeout_ms`,
`state`, `revision`. `state` is `"pending"`, `"started"`,
`"failed"`, `"ended"`, or `"timed_out"`. Tokens are at most 128
characters; application names are at most 512. Timeout defaults to 3000 ms
and is clamped to 100–10000 ms.

`restart_compositor` is an explicit supervisor operation. It does not promise
that client windows survive a compositor restart; Wayland clients normally
lose their connection when the compositor exits. The operation reports the
new compositor lifecycle and client loss through session events. Lua config
reload should not restart the compositor unless a change is startup-only and
the caller explicitly requests it. `session.logout()` completes when the
supervisor accepts the request; the session-state event is sent before the
client connection closes when the transport permits it.

### Current API migration map

This table accounts for every method currently registered in
`src/config/gnoblin-lua.c` and documented in `docs/config/runtime-api.md`.
It records legacy method names for migration. Temporary Lua aliases may exist
while clients migrate, but none of these names creates a GNOME Shell runtime
dependency in the target contract.

| Current method                                                         | Target name or decision                                                                                                                      |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `workspace.list`                                                       | `gnoblin.workspaces.list()`                                                                                                                  |
| `workspace.create`                                                     | `gnoblin.workspaces.create(options)`                                                                                                         |
| `workspace.rename`                                                     | `workspace:rename(name)`                                                                                                                     |
| `workspace.remove`                                                     | `workspace:remove()`                                                                                                                         |
| `workspace.switch`                                                     | `workspace:activate()`                                                                                                                       |
| `workspace.next`                                                       | `gnoblin.workspaces.next()`                                                                                                                  |
| `workspace.previous`                                                   | `gnoblin.workspaces.previous()`                                                                                                              |
| `workspace.move_active`                                                | `gnoblin.workspaces.active():move_here(window, options)`                                                                                     |
| `workspace.move_window`                                                | `window:move_to_workspace(target, options)`                                                                                                  |
| `window.list`                                                          | `gnoblin.windows.list(filter)`                                                                                                               |
| `window.match`                                                         | The stable identity and rule match fields become `Window` properties; matching uses list filters.                                            |
| `window.action`                                                        | Removed from target; split into the typed `Window` methods above, including `window:focus(context)`.                                         |
| `layer.list`                                                           | `gnoblin.layers.list(filter)`                                                                                                                |
| `monitor.list`                                                         | `gnoblin.monitors.list()`                                                                                                                    |
| `animation.list`                                                       | `gnoblin.animations.list()`                                                                                                                  |
| `animation.surfaces`                                                   | Replaced by `gnoblin.layers.list()` and explicit animation target types.                                                                     |
| `animation.inspect`                                                    | `gnoblin.animations.get(name)` and preview validation.                                                                                       |
| `animation.preview`                                                    | `gnoblin.animations.preview(spec)`                                                                                                           |
| `animation.seek`                                                       | `preview:seek(progress)`                                                                                                                     |
| `animation.step`                                                       | `preview:step(milliseconds)`                                                                                                                 |
| `animation.play`                                                       | `preview:play()`                                                                                                                             |
| `animation.pause`                                                      | `preview:pause()`                                                                                                                            |
| `animation.stop`                                                       | `preview:stop()`                                                                                                                             |
| `feature.list` / `feature.show` / `feature.enable` / `feature.disable` | Removed; these toggled GNOME Shell-owned behavior and have no standalone target.                                                             |
| `script.list`                                                          | Removed. The GNOME Shell script manager does not exist in the standalone session; Lua files are loaded through `gnoblin.load` and `require`. |
| `input.list`                                                           | `gnoblin.input.sources()`; physical devices are listed separately.                                                                           |
| `input.current`                                                        | `gnoblin.input.current_source()`                                                                                                             |
| `input.select`                                                         | `gnoblin.input.select_source({type, id})`                                                                                                    |
| `privacy.get`                                                          | `gnoblin.privacy.state()`                                                                                                                    |
| `permissions.list`                                                     | `gnoblin.permissions.policy()`                                                                                                               |
| `permissions.check`                                                    | `gnoblin.permissions.check(capability, identity)`                                                                                            |
| `grant.list`                                                           | `gnoblin.portals.grants()`                                                                                                                   |
| `grant.revoke`                                                         | `grant:revoke()`                                                                                                                             |
| `launch.status`                                                        | `gnoblin.launches.list()`                                                                                                                    |
| `launch.begin`                                                         | `gnoblin.launches.begin(options)`                                                                                                            |
| `launch.end`                                                           | `gnoblin.launches.end(token)`                                                                                                                |
| `shell.ping`                                                           | Removed; transport health is not a compositor API method.                                                                                    |
| `shell.version`                                                        | `gnoblin.version()`                                                                                                                          |
| `shell.status`                                                         | `gnoblin.session.status()`                                                                                                                   |
| `shell.reload`                                                         | Removed from the core API; reload the Lua runtime or restart a selected shell client through its own lifecycle.                              |
| `runtime.reload_config`                                                | `gnoblin.runtime.reload_config()`                                                                                                            |
| `shortcut.list`                                                        | `gnoblin.shortcuts.list()`                                                                                                                   |
| `shortcut.capture`                                                     | `gnoblin.shortcuts.capture(options)`                                                                                                         |

## Event catalog

### Events available today

The current hybrid runtime forwards GNOME Shell and Mutter events as well as
Gnoblin-owned events. This catalog records the current checkout only; it is
not a promise to retain Shell event forwarding in the standalone target.
Signal coverage can change with the pinned upstream versions.

| Current event                                 | Fields                                                                                                 | Source or meaning                                            |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------ |
| `gnome.shell.focus.changed`                   | `app_id`, `wm_class`, `title`                                                                          | Keyboard focus changes.                                      |
| `gnome.shell.window.created`                  | `app_id`, `wm_class`, `title`                                                                          | Shell observes a new window.                                 |
| `gnome.shell.window.unmanaged`                | `app_id`, `wm_class`, `title`                                                                          | Shell removes a window.                                      |
| `gnome.shell.input.<type>`                    | `type`, `time`, and input-specific fields                                                              | Captured Shell input event.                                  |
| `gnome.interface.color-scheme-changed`        | `color_scheme`: `default`, `prefer-dark`, or `prefer-light`                                            | Desktop appearance preference.                               |
| `mutter.wayland.pointer-window-changed`       | `app_id`, `wm_class`, `title`; empty strings when no client surface is under pointer                   | Mutter pointer tracking.                                     |
| `mutter.touchpad.gesture`                     | `gesture`, `phase`, `fingers`, `time`, and gesture-specific deltas                                     | Mutter touchpad recognizer.                                  |
| `gnoblin.input.gesture`                       | `gesture`, `phase`, `fingers`, `sequence`, monotonic `time`, `input_time`, and gesture-specific deltas | Stable native Lua event derived from Mutter touchpad input.  |
| `mutter.<object>.<signal>`                    | `source`, `signal`, typed `argN` fields, and window identity fields when applicable                    | Forwarded Mutter GObject signal.                             |
| `gnoblin.config.reloaded`                     | `path`                                                                                                 | Config reload succeeds.                                      |
| `gnoblin.config.reload-failed`                | `path`, `error`                                                                                        | Config reload fails; the previous config remains active.     |
| `gnoblin.workspace.created`                   | `id`, `number`, `name`, `active`, `windows`, `persistent`                                              | Runtime workspace is created.                                |
| `gnoblin.workspace.renamed`                   | Same workspace fields                                                                                  | Workspace display name changes.                              |
| `gnoblin.workspace.removed`                   | Last workspace record; `number` is its former position                                                 | Temporary workspace is removed.                              |
| `gnoblin.workspace.activated`                 | Same workspace fields                                                                                  | Active workspace changes.                                    |
| Native `gnoblin.window.created`               | `window`, `name`, `revision`, `sequence`, `time`                                                       | Native runtime observes a new managed window.                |
| Native `gnoblin.window.changed`               | `window_id`, `changed`, `window`, event metadata                                                       | A mapped window property changes, excluding attention state. |
| Native `gnoblin.window.focused` / `unfocused` | `window_id`, `window`, event metadata                                                                  | Keyboard focus enters or leaves a managed window.            |
| Native `gnoblin.window.attention-changed`     | `window_id`, `window`, `demands_attention`, event metadata                                             | Mutter's attention state changes.                            |
| Native `gnoblin.window.closed`                | `window_id`, `last`, event metadata                                                                    | Native runtime removes a managed window.                     |
| `gnoblin.operation.completed`                 | `operation_id`, `method`, `ok`, then `value` or an `Error` record                                      | Native API 1.11 completion event.                            |
| `gnoblin.api.operation-completed`             | `request_id`, `method`, `ok`, then `result` or string `error`                                          | Legacy completion event retained during migration.           |
| `gnoblin.feature.changed`                     | `feature`, `enabled`                                                                                   | A feature changes after initial setup.                       |
| `gnoblin.scripts.loaded`                      | `scripts`: comma-separated loaded script filenames                                                     | Current user-script loading pass ends.                       |
| `gnoblin.scripts.load_failed`                 | `script`, `error`                                                                                      | Current user script fails to load.                           |

The `gnome.shell.*` events and current `gnoblin.feature.changed` and
`gnoblin.scripts.*` events belong to the current GNOME Shell-backed
implementation. They are removed from the standalone session. The target
folds window lifecycle into Gnoblin events, moves appearance notification to
`gnoblin.appearance.color-scheme-changed`, and uses config reload events for
Lua load success or failure. It has no Shell event source, feature registry,
or GJS script manager.

The `Native` event rows are implemented only by the native Mutter runtime.
Their window tables currently contain a subset of the target `Window` fields;
they are plain callback payloads, not read-only records with methods.

Current compatibility aliases are `pointer_window_changed`,
`focus_changed`, `window_created`, `window_unmanaged`, and
`input.<type>`. The `*` listener receives all forwarded events.
`gnoblin.on` accepts any nonempty UTF-8 event name up to 128 bytes; a
callback runs only if a source actually dispatches that name. Every current
event includes `name`. Mutter scalar arguments are named `arg0`, `arg1`,
and so on, with corresponding `argN_type` strings; non-scalar objects are
reduced to type or name fields.

Touchpad gesture `gesture` values are `"swipe"`, `"pinch"`, or
`"hold"`; phases are `"begin"`, `"update"`, or `"end"`. Swipe updates
include `dx` and `dy`; pinch updates include `scale` and
`angle_delta`. Hold events have no movement fields.
Other common Shell input event fields are: motion uses `x` and `y`;
button press/release uses `button`, `x`, and `y`; scroll uses `x`,
`y`, `scroll_x`, `scroll_y`, and `scroll_direction`; key
press/release uses `key_symbol`.

### Proposed stable events

The events below are the proposed stable `gnoblin.*` contract. Each event
record also includes `name`, monotonic `sequence`, and monotonic-clock
`time`. Events describing state changes include `revision`. Raw
`mutter.*` and `gnome.*` events are not included in this stable catalog.

| Event                                     | Additional fields                                                | Emitted when                                                                                                                     |
| ----------------------------------------- | ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `gnoblin.window.created`                  | `window: Window`                                                 | A managed application window appears.                                                                                            |
| `gnoblin.window.closed`                   | `window_id`, `last: Window`                                      | A managed window is removed.                                                                                                     |
| `gnoblin.window.focused`                  | `window_id`, `window: Window`                                    | Keyboard focus changes to a window.                                                                                              |
| `gnoblin.window.attention-changed`        | `window_id`, `window: Window`, `demands_attention`               | Mutter's attention state changes; this may follow a focus request that policy did not activate.                                  |
| `gnoblin.focus.policy-changed`            | `policy: FocusPolicy`, `revision`, `sequence`, `time`            | Effective focus policy changes after a successful config commit.                                                                 |
| `gnoblin.window.unfocused`                | `window_id`, `window: Window`                                    | A window loses keyboard focus.                                                                                                   |
| `gnoblin.window.changed`                  | `window_id`, `changed: string[]`, `window: Window`               | One or more public properties change.                                                                                            |
| `gnoblin.workspace.created`               | `workspace: Workspace`                                           | A runtime workspace appears.                                                                                                     |
| `gnoblin.workspace.renamed`               | `workspace: Workspace`                                           | Its display name changes.                                                                                                        |
| `gnoblin.workspace.changed`               | `workspace: Workspace`, `changed: string[]`                      | Its position, window count, or persistence state changes.                                                                        |
| `gnoblin.workspace.removed`               | `workspace_id`, `last: Workspace`                                | A temporary workspace is removed.                                                                                                |
| `gnoblin.workspace.activated`             | `workspace: Workspace`, `previous_id?`                           | The active workspace changes.                                                                                                    |
| `gnoblin.workspace.window-moved`          | `window_id`, `from_id`, `to_id`                                  | A window changes workspace.                                                                                                      |
| `gnoblin.monitor.added`                   | `monitor: Monitor`                                               | An output becomes available.                                                                                                     |
| `gnoblin.monitor.removed`                 | `monitor_id`, `last: Monitor`                                    | An output is removed.                                                                                                            |
| `gnoblin.monitor.changed`                 | `monitor: Monitor`, `changed: string[]`                          | Output properties change.                                                                                                        |
| `gnoblin.input.device-added`              | `device: InputDevice`                                            | A device appears in the native input-device snapshot.                                                                            |
| `gnoblin.input.device-removed`            | `device_id`, `last: InputDevice`                                 | A device disappears from the native input-device snapshot.                                                                       |
| `gnoblin.input.sources-changed`           | `sources: InputSource[]`                                         | The configured available XKB source list changes.                                                                                |
| `gnoblin.input.source-changed`            | `available`, `source?: InputSource`                              | Mutter confirms a different Gnoblin-owned keymap group, or the current source becomes unknown.                                   |
| `gnoblin.input.gesture`                   | `gesture`, `phase`, `fingers`, gesture-specific deltas           | A touchpad gesture phase arrives.                                                                                                |
| `gnoblin.shortcut.activated`              | `name`, `trigger`, `seat`, `time`, `focus_context: FocusContext` | A registered Gnoblin shortcut activates; its context can authorize one focus, interactive move, or interactive resize operation. |
| `gnoblin.animation.started`               | `animation`, `target`, `event`                                   | A configured animation starts.                                                                                                   |
| `gnoblin.animation.finished`              | `animation`, `target`, `event`, `cancelled`                      | It completes or is interrupted.                                                                                                  |
| `gnoblin.capability.changed`              | `capability: Capability`                                         | A compositor or protocol capability becomes available or unavailable.                                                            |
| `gnoblin.privacy.changed`                 | `state: PrivacyState`                                            | A monitored privacy activity changes.                                                                                            |
| `gnoblin.permission.changed`              | `policy: PermissionPolicy`, `revision`                           | A successful config commit changes the committed permission policy.                                                              |
| `gnoblin.portal.grant-added`              | `grant: PortalGrant`                                             | A portal grant becomes active.                                                                                                   |
| `gnoblin.portal.grant-removed`            | `grant_id`, `kind`                                               | A portal grant ends or is revoked.                                                                                               |
| `gnoblin.launch.changed`                  | `launch: Launch`                                                 | Launch feedback state changes.                                                                                                   |
| `gnoblin.config.reloaded`                 | `path`, `revision`                                               | Configuration loads and applies successfully.                                                                                    |
| `gnoblin.config.reload-failed`            | `path`, `error: Error`                                           | A reload fails; the last valid config remains active.                                                                            |
| `gnoblin.operation.completed`             | `operation_id`, `method`, `ok`, `value?`, `error?`               | A mutating call finishes.                                                                                                        |
| `gnoblin.session.state-changed`           | `status: SessionStatus`                                          | The supervisor or compositor changes lifecycle state.                                                                            |
| `gnoblin.appearance.color-scheme-changed` | `color_scheme`: `default`, `prefer-dark`, or `prefer-light`      | The desktop appearance preference changes.                                                                                       |

The current implementation also forwards open-ended Mutter GObject signals
and GNOME Shell events. The standalone target keeps only an explicitly
unstable Mutter signal namespace, `gnoblin.events.mutter.on(...)`; it removes
the GNOME Shell event source and Shell feature/script event stream.
The current `gnoblin.api.operation-completed` event remains a temporary
compatibility alias for `gnoblin.operation.completed` during client migration.

## Wire contract and versioning

Lua record methods are wrappers around versioned, typed operation names. The
local client interface exposes those same operations, result records, error
codes, and event payloads. It must not expose a second shell-only socket
contract.

The compositor handshake reports `api_major` and `api_minor`:

- Increment the major version when a method is removed, renamed, or changes
  meaning, or when a required field changes incompatibly.
- Increment the minor version when adding optional fields, methods, or events.
- Clients ignore unknown optional record fields and events.
- The compositor handshake advertises supported methods and event names.
- `gnoblin.capabilities.list()` reports available compositor and protocol
  capabilities.
- A method call unavailable in the negotiated version fails with
  `unsupported`; it is never silently ignored.

Stable IDs are opaque strings to clients. Clients must not infer an ID from a
title, application name, PID, list position, or workspace number.

## Current implementation inventory

The following is the registered runtime method set in this checkout. It is
documented here for migration completeness; it is not the proposed final
surface.

| Namespace     | Current methods                                                                                                                                                                                                                                                                                               |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `workspace`   | `list()`, `create(args)`, `rename(args)`, `remove(args)`, `switch(args)`, `next()`, `previous()`, `move_active(args)`, `move_window(args)`                                                                                                                                                                    |
| `window`      | `list(args)`, `match(args)`, `action(args)`, `close(args)`, `minimize(args)`, `toggle_minimize(args)`, `restore(args)`, `set_maximized(args)`, `set_fullscreen(args)`, `set_above(args)`, `set_sticky(args)`, `move(args)`, `resize(args)`, `move_to_workspace(args)`, `move_to_monitor(args)`, `focus(args)` |
| `layer`       | `list()`                                                                                                                                                                                                                                                                                                      |
| `monitor`     | `list()`                                                                                                                                                                                                                                                                                                      |
| `animation`   | `list()`, `surfaces()`, `inspect(args)`, `preview(args)`, `seek(args)`, `step(args)`, `play(args)`, `pause(args)`, `stop(args)`                                                                                                                                                                               |
| `feature`     | `list()`, `show(args)`, `enable(args)`, `disable(args)`                                                                                                                                                                                                                                                       |
| `script`      | `list()`                                                                                                                                                                                                                                                                                                      |
| `input`       | `list()`, `current()`, `select(args)`                                                                                                                                                                                                                                                                         |
| `privacy`     | `get()`                                                                                                                                                                                                                                                                                                       |
| `permissions` | `list()`, `policy()`, `check(args)`                                                                                                                                                                                                                                                                           |
| `grant`       | `list()`, `revoke(args)`                                                                                                                                                                                                                                                                                      |
| `launch`      | `status()`, `begin(args)`, `end(args)`                                                                                                                                                                                                                                                                        |
| `shell`       | `ping()`, `version()`, `status()`, `reload()`                                                                                                                                                                                                                                                                 |
| `runtime`     | `reload_config()`                                                                                                                                                                                                                                                                                             |
| `shortcut`    | `list()`, `capture(args)`                                                                                                                                                                                                                                                                                     |

Workspace mutations also have plural aliases under `gnoblin.workspaces`:
`create`, `rename`, `remove`, `activate`, `next`, `previous`, `move_active`,
and `move_window`. Native sessions additionally provide immediate
`gnoblin.workspaces.list`, `active`, and `by_id` reads and
`gnoblin.windows.list`, `focused`, and `by_id` reads. These return immutable
snapshot records. The singular `gnoblin.workspace.list()` and
`gnoblin.window.list()` methods remain operation-based compatibility calls.
Shell-backed sessions do not populate either immediate snapshot cache.

Lua event registrations use `gnoblin.events.on` and `gnoblin.events.once`; the
legacy `gnoblin.on` alias returns the same unsubscribe-able subscription.

Typed window methods take a stable `id` and return an operation whose completed
value is `{id}`. They cover close, minimize and restore, boolean state setters,
move, resize, workspace move, monitor move, and interactive move and resize.
`window.focus`, `window.begin_move`, and `window.begin_resize` require a live,
one-use trusted shortcut context; direct requests without one fail closed. A
context authorizes only one of these operations. See the
[runtime API reference](../docs/config/runtime-api.md) for arguments and
restrictions.

### Current window results and actions

The current `window.list` result has `windows`, an array of records with
these fields. They use camelCase today; the target uses the snake_case
properties defined above.

| Current property                                                 | Type                    | Meaning                                                          |
| ---------------------------------------------------------------- | ----------------------- | ---------------------------------------------------------------- |
| `id`                                                             | string                  | Stable window sequence ID.                                       |
| `title`                                                          | string                  | Current title, or an empty string.                               |
| `appId`                                                          | string                  | Desktop app ID, falling back to WM class.                        |
| `gtkAppId`                                                       | string                  | GTK app ID, or an empty string.                                  |
| `wmClass`                                                        | string                  | WM class, or an empty string.                                    |
| `ruleAppId`                                                      | string                  | GTK app ID, falling back to WM class.                            |
| `focused`                                                        | boolean                 | Keyboard focus state.                                            |
| `minimized`                                                      | boolean                 | Minimized state.                                                 |
| `workspace`                                                      | integer or `nil`        | Current one-based workspace position.                            |
| `workspaceId`                                                    | string or `nil`         | Stable workspace ID.                                             |
| `workspaceNumber`                                                | integer or `nil`        | Current one-based workspace position.                            |
| `monitorIndex`                                                   | integer                 | Current monitor index.                                           |
| `monitorId`                                                      | string or `nil`         | Canonical active connector name for the current logical monitor. |
| `above`, `sticky`, `demandsAttention`                            | boolean                 | Stacking, workspace visibility, and attention state.             |
| `closable`, `minimizable`, `maximizable`, `movable`, `resizable` | boolean                 | Current compositor capabilities.                                 |
| `role`                                                           | string or `nil`         | Window role, when supplied.                                      |
| `type`                                                           | integer                 | Mutter `MetaWindowType`; see the mapping in Window properties.   |
| `maximized`                                                      | boolean                 | Maximized in both directions.                                    |
| `fullscreen`                                                     | boolean                 | Fullscreen state.                                                |
| `geometry`                                                       | `{x, y, width, height}` | Current frame rectangle in logical pixels.                       |
| `lastUserTime`                                                   | integer                 | Last user interaction timestamp known to Mutter.                 |
| `parent`                                                         | string or `nil`         | Stable ID of the transient parent.                               |
| `monitor`                                                        | `{x, y}` or `nil`       | Monitor origin in logical coordinates.                           |

The Shell-backed result includes `id`, title and app identity, focus and
minimized state, workspace fields, `monitorIndex`, maximize and fullscreen
state, geometry, `lastUserTime`, and optional `parent` and `monitor`. The
native preview adds `monitorId`, stacking and attention state, capability
flags, optional `role`, and `type`; it also supplies `workspaceId` and
`workspaceNumber` when Mutter associates the window with a workspace.

`window.match({window?})` defaults to `"active"` and returns `id`,
`identity` with `desktop_app_id`, `gtk_app_id`, `wm_class`, and
`rule_app_id`, plus a `match` table with `type`, `title`, `focused`,
and optional `app_id`.

`window.action({action, window?, ...})` defaults its target to `"active"`.
The current action strings are `menu`, `interactive-move`,
`interactive-resize`, `above`, `unabove`, `stick`, `unstick`,
`focus`, `close`, `minimize`, `restore-or-minimize`, `restore`,
`maximize`, `unmaximize`, `fullscreen`, `unfullscreen`, `move`,
`resize`, `workspace`, and `monitor`.

Only `move` accepts `x` and `y` (integers from −100000 to 100000); only
`resize` accepts `width` and `height` (integers from 1 to 32768).
`monitor` uses a numeric index in the compatibility action. The typed
`window.move_to_monitor` method accepts the stable connector ID as a string or
`{id = string}`; cloned outputs use the lexicographically first active
connector. A disconnected or no-longer-canonical ID fails with `not_found`.
The `workspace` action
accepts `{id = string}` or `{number = integer}`. A target is a stable window
ID or `"active"`. The target API replaces these strings with typed methods
and idempotent property setters.

Current Lua runtime callback calls return an `Operation` handle. Native API
1.11 completion updates its `status`, `value`, and structured `Error` fields,
invokes callbacks registered while the operation was pending, and dispatches
`gnoblin.operation.completed` with `operation_id`, `method`, `ok`, and either
`value` or `error`. It also dispatches the legacy
`gnoblin.api.operation-completed` event with `request_id`, `result`, and a
string `error`. If `on_complete` is registered after completion, Gnoblin
queues it for the next main-loop turn. The host dispatches that callback through
the same config validation and operation drain path used for runtime events;
operations requested by the callback are applied after it returns. Queued
callbacks are discarded when their Lua runtime is replaced. The native API call
path still uses a positive request ID internally and returns a normal socket
`reply` or `error`.
See the [runtime API reference](../docs/config/runtime-api.md) and [Lua
events](../docs/config/lua-events.md) for current behavior.

## Current migration status and remaining work

The current implementation has a committed settings property, an initial typed
window-operation slice, and immediate native window/workspace reads:

- Stable-ID window mutations are registered in Lua and dispatched by the
  native Mutter boundary. Shell and `gnoblinctl` route the same method names.
- `gnoblin.events.on` and `once` return unsubscribe-able subscriptions.
- `gnoblin.settings` returns a detached immutable view of the committed
  configuration using public snake_case names. Its `revision` advances only
  when committed setting values change and is not persisted.
- `gnoblin.focus.policy` returns an immutable view of supported focus settings
  with the same committed configuration revision. It does not grant permission
  to focus a window.
- `gnoblin.focus.policy-changed` runs after a successful config commit only
  when an effective focus setting changes. Its `policy` snapshot and event
  `revision` identify the committed settings revision. Native-control API 1.13
  advertises the event to socket subscribers.
- `permissions.list()` and `permissions.check()` remain compatibility calls.
  `permissions.policy()` returns the committed policy directly with its config
  revision. The portal backend gets decisions from a compositor-owned D-Bus
  method and supplies only the verified requester identity. Invalid policies
  fail config validation; a missing native decision service denies portal
  requests in a Gnoblin session. Native-control API 1.16 adds the matching
  socket method and emits `gnoblin.permission.changed` only when a successful
  config commit changes the policy.
- `gnoblin.focus.history(filter?)` returns immutable native window snapshots in
  MRU order from confirmed focus events observed by the runtime. It seeds the
  current focused window from each installed snapshot and removes closed
  windows. Older windows with no observed focus event remain in snapshot order.
- `gnoblin.shortcuts.capture(options?)` captures one native accelerator and
  completes its `Operation` with the normalized accelerator string. Escape,
  timeout, session lock, and capture conflicts complete with an error.
- `gnoblin.shortcuts.list()` returns read-only records for configured shortcuts
  registered by the native compositor. It omits Shell-owned shortcuts and
  declarations the native runtime does not register.
- The native Lua runtime maps Mutter's raw touchpad gesture event to
  `gnoblin.input.gesture` while retaining the raw Mutter event for compatibility.
- `gnoblin.windows`, `gnoblin.workspaces`, and `gnoblin.monitors` return
  immutable native snapshots with a shared revision. The caches refresh before
  their native lifecycle events are delivered. `gnoblin.layers.list()` returns
  immutable layer-surface snapshots from the same revisioned native state.
- `gnoblin.monitors.list()` and `gnoblin.monitors.primary()` return immutable
  native monitor snapshots with per-record revisions. Active logical monitor
  additions, changes, and removals are delivered as native lifecycle events.
- `gnoblin.capabilities.list()` returns immutable native capability snapshots.
  The current snapshot lists only capabilities advertised as available, so
  every record has `available = true` and omits the optional `reason` field.
- `gnoblin.version()` returns an immutable version record in both native and
  Shell-backed Lua runtimes. Remote userinfo is removed before a remote is
  returned; absent identity fields use the string `"unknown"`.
- `gnoblin.input.devices()` returns immutable native input-device snapshots.
  The cache refreshes before `gnoblin.input.device-added` and
  `gnoblin.input.device-removed` callbacks run; device IDs are session-only.
- `gnoblin.input.sources()` and `gnoblin.input.current_source()` read immutable
  XKB source snapshots. The current source is nil when the active keymap is
  external or unknown. `gnoblin.input.select_source({type = "xkb", id = ...})`
  completes after Mutter confirms the keymap. Mutter loads at most four XKB
  layouts per keymap; selecting another listed source switches its active
  group. Native IBus selection is unsupported and fails explicitly.
- Read-only Window and Workspace snapshots expose colon methods for the
  registered close, state, geometry, workspace, and monitor operations. Methods
  queue the existing typed operations. `Window:focus(context)` accepts only a
  live context from a trusted native shortcut press.
- Native Lua and API 1.1 socket clients receive workspace create, rename,
  change, remove, activation, and window-moved events. Changes report updated
  position, window count, or persistence fields.
- Native `window.match` resolves a focused or explicitly selected window.
- Runtime calls return Lua `Operation` handles in event callbacks. The native
  host drains queued calls and reports completion without GJS.
- Native-control API 1.9 supports filtered event subscriptions and publishes
  `gnoblin.input.gesture` to subscribed socket clients. API 1.10 publishes
  `gnoblin.shortcut.activated` with a random, connection-bound focus token to
  clients subscribed at that version.
- Native-control API 1.11 adds owner-scoped, press-only `shortcut.bind` and
  `shortcut.unbind` methods, structured `gnoblin.operation.completed` events,
  and a native launch snapshot for `gnoblin.launches.list()`.
- Native-control API 1.12 adds trusted `window.begin_move` and
  `window.begin_resize` socket methods.
- Native-control API 1.13 advertises `gnoblin.focus.policy-changed`. The event
  follows a successful config commit only when the effective focus policy
  differs from the previously committed policy.
- Native-control API 1.14 adds asynchronous `grant.list` and `grant.revoke`
  calls through the portal backend that owns persisted grants. Their wire
  records retain a device bitmask and `screenStreams` for compatibility.
- Native-control API 1.15 adds the `portals.grants` snapshot, with device
  arrays, `created_at`, and per-snapshot revisions, plus
  `gnoblin.portal.grant-added` and `gnoblin.portal.grant-removed` events. Lua
  snapshot records expose `grant:revoke()`. The backend checks a record's
  creation time before removal so a stale record cannot revoke a later grant
  that reused its legacy ID. New records persist creation time; legacy records
  infer it from file modification time at one-second precision, which is not
  verified consent time.
- Native-control API 1.16 adds `permissions.policy` and the
  `gnoblin.permission.changed` event. The target `permissions.policy()` Lua
  method returns the committed policy with its settings revision.
- Typed focus and interactive grabs are available only with a one-use context issued for a real,
  non-synthetic key press matching a configured native command shortcut. Lua
  receives protected `FocusContext` userdata; each subscribed API 1.10 socket
  client receives a separate random token bound to that connection. Contexts
  expire after five seconds, are consumed by a focus or interactive-grab
  attempt, and are revoked on config reload or session lock. Shell-backed
  shortcuts do not issue these contexts.

The checkout remains short of the target contract. Privacy monitoring is still
Shell-owned: the native remote-access controller only reports new handles, and
its session manager exposes a count rather than an enumerable set of active
streams. The current native runtime cannot seed or accurately track active
screen sharing, microphone, camera, or location state. A native implementation
must report source availability and omit unknown activity values.

Direct compositor snapshots
exist for windows, workspaces, monitors, layers, capabilities, input devices,
XKB input sources, configured native shortcuts, and launch feedback. Other
input collections and operations remain unavailable or operation based. Layer, capability,
input-device, input-source, and shortcut records expose no methods. Snapshot
records are read-only values; Window and Workspace records expose methods for the
operations that are already registered, while Monitor records have no
mutating methods because display mode-setting remains outside this API.
`Window:move(position)` and `Window:resize(size)` request direct geometry
changes. `Window:begin_move(context)` and
`Window:begin_resize(edge, context)` use Mutter's keyboard grab with the
trusted event timestamp and current pointer sprite. Native API 1.12 adds these
interactive socket operations. Native API 1.11 operation errors use `Error`
records; Shell-backed compatibility completions still use strings.
Registering a completion callback after an operation completes queues it for the
next main-loop turn, where it can queue new operations or settings changes.
Event listener errors now allow remaining listeners to run; Gnoblin rolls back
config changes from the event but keeps its queued operations. If the final
config is invalid, those operations fail and notify their completion callbacks.

The native socket preview exposes the core window and layer-surface fields
available from Mutter. API 1.1 covers window, workspace, and monitor lifecycle
events with a shared state revision, per-event `sequence`, and monotonic-clock
`time`; API 1.2 adds `layer.list`; API 1.3 adds a one-time `input.devices`
snapshot; and API 1.4 adds input-device lifecycle events. API 1.5 adds
`shortcut.actions`; API 1.6 adds XKB input-source methods; API 1.7 adds launch
feedback; API 1.8 adds shortcut capture; API 1.9 adds `shortcut.list`, filtered
event subscriptions, and `gnoblin.input.gesture`; API 1.10 adds shortcut focus
grants and guarded `window.focus`; API 1.11 adds connection-owned dynamic
press-only shortcut bindings and owner-only activation events; API 1.12 adds
trusted interactive window grabs; API 1.13 adds focus-policy events; API 1.14
adds asynchronous portal-grant listing and revocation; API 1.15 adds
timestamped grant snapshots and lifecycle events; and API 1.16 adds the
committed permission-policy read and change event. Dynamic bindings do not
provide held-modifier sessions, modal input, or type-ahead handoff.
Window `changed` lists mapped public
properties; attention changes have their own event. Lua payloads use `name`;
socket payloads use `event`. Lua event records remain plain callback payloads;
they are not read-only `Window` records. Shell-backed sessions still use their
existing Shell and Mutter event sources. The standalone native path still lacks
some Shell handlers for applying input configuration, privacy state, broader
portal activity and events, animation runtime control, and session supervision.
Normal-window animation still needs native per-window selection. A Mutter map
hook must resolve the ordered matching rule for that window before it runs an
animation; choosing the first global `open` declaration would ignore rule
selection. The resolver must preserve the documented match fields, JavaScript
regular-expression behavior, and stable workspace IDs.

Continue the migration in these steps:

1. Implement input configuration, privacy state, animation runtime control,
   broader portal activity, and session operations in Gnoblin-owned handlers.
2. Migrate Bingux and other shell clients to the shared method and event
   contract; verify their UI remains client-owned.
3. Remove the GNOME Shell-specific API methods, event forwarding, configuration,
   and runtime code as their Gnoblin-owned behavior moves to Lua and Mutter.
   The standalone runtime should not require GJS.
