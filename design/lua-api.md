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
then proposes the remaining standalone API for windows, focus, workspaces,
monitors, layer surfaces, input, shortcuts, animation, permissions, portals,
and session lifecycle. Implemented rows are marked current; other rows are
proposals, not implementation claims. GNOME Shell
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

The [spatial desktop plan](spatial-desktop.md) extends these ownership rules
to headset presentations. Gnoblin supplies surface state, appearance,
authorized input, and temporary-state recovery. Shell developers own spatial
scenes, placement, and interaction; the headset renders the 3D scene. The
plan does not add implemented Lua methods to this reference.

Mutter owns windows, input devices, focus, geometry, rendering, and compositor
protocols. Gnoblin owns policy and the public control contract. Compositor
operations cross a narrow versioned interface; the Lua API does not expose
Mutter objects or private C types.

### Shell-client coverage

The target covers the window-manager calls used by Bingux's current dock,
launcher, switcher, window menu, and workspace selector:

| Shell need                                                                                        | Gnoblin API                                                                     |
| ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| List and group windows; track creation, closure, focus, attention, workspace, and monitor changes | `gnoblin.windows.list()`, `Window` properties, and `gnoblin.window.*` events    |
| Restore a minimized window or activate a user-selected window                                     | `window:restore()`; the shell sends `window.focus` with an XDG Activation token |
| Close, minimize, maximize, move, resize, pin above, or keep a window on all workspaces            | Typed `Window` methods and setters                                              |
| Show and switch workspaces; move a window to another workspace                                    | `gnoblin.workspaces.*`, `Workspace` methods, and workspace events               |
| Inspect outputs and shell-owned layer surfaces                                                    | `gnoblin.monitors.list()` and `gnoblin.layers.list()`                           |

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

The standalone session runs Lua configuration in a private worker launched by
the stable `gnoblin` session host, which starts Mutter as its compositor child.
The worker and compositor communicate through a private typed channel. Keep
the API independent of that process split. Lua calls and the local
shell-client control interface share operation names and schemas only where a
method is exposed on the control socket. Trusted focus, text-target,
pointer-drag, and snap-context methods are available only to the supervised
Lua runtime; remote clients cannot execute arbitrary Lua.

State reads return immutable snapshots from Gnoblin's latest compositor state.
Each record includes a monotonically increasing `revision`. A snapshot does
not change after it is returned; subscribe to events or read a new snapshot to
observe later state.

Every mutating call returns an `Operation` handle. It has:

| Property or method       | Type                                             | Meaning                                                                                                                                                                      |
| ------------------------ | ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`                     | integer                                          | Session-unique request identifier.                                                                                                                                           |
| `method`                 | string                                           | Canonical operation name, such as `window.set_above`.                                                                                                                        |
| `status`                 | One of `"pending"`, `"succeeded"`, or `"failed"` | Current operation state.                                                                                                                                                     |
| `value`                  | any or `nil`                                     | Result after success.                                                                                                                                                        |
| `error`                  | `Error` or `nil`                                 | Failure after rejection.                                                                                                                                                     |
| `:on_complete(callback)` | `Subscription`                                   | Call callback once with `(value, error)`; if already complete, call it on the next supervised runtime event-loop turn. Unsubscribe before that turn to prevent the callback. |

Operations are not cancellable once dispatched. A failed request does not
change state. A successful request means the compositor accepted and applied
the requested change; a later user or policy action may change it again.
Completion is also emitted as `gnoblin.operation.completed` for clients that
use events instead of retaining an `Operation` handle.

Queries are local snapshot reads and return immediately. Mutations are queued
for the compositor or supervisor and complete asynchronously. Lua callbacks
run on the supervised runtime's event loop, not Mutter's compositor main
thread, and must not block that loop. An operation requested
from a callback is dispatched only after that callback returns. Runtime
mutations are available only while a supervised event callback is running;
calling one during top-level config evaluation or ordinary module loading fails
before an operation is queued. The control socket has its own validated call
path for methods it exposes.

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
runtime methods live in their domain namespaces. `gnoblin.runtime` contains
supervisor operations such as configuration reload.

### Current Lua globals

| Name                                            | Signature or value                                          | Current behavior and target                                                                                                                                                                 |
| ----------------------------------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `gnoblin.configure`                             | callable table: `gnoblin.configure(settings)`               | **Current and retained.** Merge public snake_case settings into the config document.                                                                                                        |
| `gnoblin.configure.shortcuts`                   | named entry view                                            | **Current and retained.** Read and edit named shortcut entries.                                                                                                                             |
| `gnoblin.configure.autostart`                   | named entry view                                            | **Current and retained.** Read and edit named autostart entries.                                                                                                                            |
| `gnoblin.config`                                | mutable config table                                        | **Current; compatibility view.** Keys use normalized internal hyphenated names. Prefer `gnoblin.configure`.                                                                                 |
| `gnoblin.settings`                              | read-only property                                          | **Current.** Detached snapshot of committed settings with public snake_case names and a non-persistent `revision`; available after the initial config commit in the standalone Lua runtime. |
| `gnoblin.focus.policy`                          | read-only property                                          | **Current.** Immutable focus-preference snapshot with the committed settings revision in the standalone Lua runtime.                                                                        |
| `gnoblin.focus.history(filter?)`                | `(filter?: FocusFilter) -> Window[]`                        | **Current; native runtime.** Read windows in most-recently-focused order, with optional workspace, monitor, and limit filters.                                                              |
| `gnoblin.version()`                             | `() -> Version`                                             | **Current.** Read Gnoblin, GNOME, Mutter, Lua, API, Git remote, Git SHA, and build ID.                                                                                                      |
| `gnoblin.capabilities.list()`                   | `() -> Capability[]`                                        | **Current; native runtime.** Read supported compositor and protocol capabilities.                                                                                                           |
| `gnoblin.snapshot()`                            | `() -> Settings`                                            | **Current; compatibility only.** Returns a copy of the mutable config view. Prefer `gnoblin.settings` for reads.                                                                            |
| `gnoblin.load(path)`                            | `(string) -> true`                                          | **Current; retained.** Load a relative file or glob in the current config context.                                                                                                          |
| `gnoblin.array(values)`                         | `(table) -> table`                                          | **Current; retained.** Mark a Lua table as an array where empty-table shape would otherwise be ambiguous.                                                                                   |
| `gnoblin.on(name, callback)`                    | `(string, function) -> Subscription`                        | **Current compatibility alias.** Prefer `gnoblin.events.on`.                                                                                                                                |
| `gnoblin.events.on(name, callback)`             | `(string, function) -> Subscription`                        | **Current.** Register an event callback and return an unsubscribe handle.                                                                                                                   |
| `gnoblin.events.once(name, callback)`           | `(string, function) -> Subscription`                        | **Current.** Remove the callback before its first invocation.                                                                                                                               |
| `gnoblin.events.mutter.on(name, callback)`      | `(MutterEventName, function) -> Subscription`               | **Current.** Subscribe to an unstable Mutter event; the name must start with `mutter.`.                                                                                                     |
| `gnoblin.events.mutter.once(name, callback)`    | `(MutterEventName, function) -> Subscription`               | **Current.** Subscribe to one unstable Mutter event; the name must start with `mutter.`.                                                                                                    |
| `gnoblin.shortcuts.actions(group?)`             | `(group?: string) -> ShortcutAction[]`                      | **Current.** Read available built-in keybinding actions; accepted groups are `wm`, `mutter`, and `wayland`.                                                                                 |
| `gnoblin.shortcuts.list()`                      | `() -> ShortcutState[]`                                     | **Current; native runtime.** Read the configured shortcuts registered by the compositor.                                                                                                    |
| `gnoblin.shortcuts.capture(options?)`           | `({timeout?: integer 1–60}) -> Operation<CapturedShortcut>` | **Current; native runtime only.** Capture one normalized accelerator; default timeout is 30 seconds and Escape cancels.                                                                     |
| `gnoblin.shortcuts.bind(args)` / `unbind(args)` | `(table) -> Operation<Result>`                              | **Current; native runtime.** Bind or remove a Gnoblin shortcut.                                                                                                                             |

| `gnoblin.windows` | `list(filter?)`, `focused()`, `by_id(id)`, `snap_context(context)` | **Current; native runtime only.** Read-only revisioned window snapshots and a one-use context for keyboard snapping. |
| `gnoblin.workspaces` | `list()`, `active()`, `by_id(id)`, workspace mutations | **Current; native runtime only.** Read-only revisioned workspace snapshots and typed workspace operations. |
| `gnoblin.monitors` | `list()`, `primary()` | **Current; native runtime only.** Read-only revisioned monitor snapshot records. |
| `gnoblin.layers` | `list(filter?)`, `animation_policy(namespace)` | **Current; native runtime only.** Read-only revisioned layer-surface records and effective animation/shadow policy. |
| `gnoblin.input` | `devices()`, `list()`, `current()`, `sources()`, `current_source()`, `select_source(selector)`, `text_target(context)` | **Current; native runtime only.** Read-only device/source snapshots, XKB source selection, and trusted text insertion targets. |
| `gnoblin.animations` | `list()`, `get(name)`, `surfaces()`, `inspect(args)`, `preview(args)`, `seek(args)`, `step(args)`, `play(args)`, `pause(args)`, `stop(args)` | **Current; native runtime only.** Read and control declared compositor animation previews. |
| `gnoblin.launches` | `list()`, `begin(args)`, `end(args)` | **Current; native runtime only.** Read and report tracked application launches. |
| `gnoblin.portals.grants(filter?)` | `(filter?: {kind?: string}) -> PortalGrant[]` | **Current; native runtime only.** Read active portal grants, optionally by kind. |
| `gnoblin.privacy.state()` | `() -> PrivacyState` | **Current; native runtime only.** Read screen-sharing and recording state. |
| `gnoblin.permissions` / `gnoblin.grant` | `permissions.list()`, `policy()`, `check(args)`, `grant.list()`, `revoke(args)` | **Current; native runtime only.** Inspect permission policy, check requests, list grants, and revoke grants. |
| `gnoblin.session` | `lock()`, `activity()`, `status()`, `logout()` | **Current; native runtime only.** Control or read the supervised session. |
| `gnoblin.runtime.reload_config()` | `() -> Operation<Result>` | **Current; native runtime only.** Reload the active configuration. |
| `gnoblin.listeners` | map of event names to callback arrays | **Current; inspect only.** Do not edit this table directly. |
| `gnoblin.window_rule(rule)` | `(WindowRule) -> nil` | **Current and retained.** Append a window or layer matching rule. |
| `gnoblin.permission_rule(rule)` | `(PermissionRule) -> nil` | **Current and retained.** Append a portal permission rule. |
| `gnoblin.shortcut(entry)` | `(Shortcut) -> nil` | **Current compatibility helper.** Prefer `gnoblin.configure {shortcuts = {...}}`. |
| `gnoblin.animation(entry)` | `(Animation) -> nil` | **Current and retained.** Declare a named compositor animation; runtime controls are under `gnoblin.animations`. |
| `gnoblin.autostart(entry)` | `(Autostart) -> nil` | **Current compatibility helper.** Prefer `gnoblin.configure {autostart = {...}}`. |
| `gnoblin.remove_shortcut(name)` | `(string) -> nil` | **Current compatibility helper.** Prefer an entry with `enable = false`. |
| `gnoblin.remove_autostart(name)` | `(string) -> nil` | **Current compatibility helper.** Prefer an entry with `enable = false`. |
| global `require(name)` | `(string) -> any` | **Current custom loader.** Loads a local module beside the calling file or under its `lua/` directory; it is not Lua's installed-module search path. |

`CapturedShortcut` contains the normalized GTK accelerator string in
`accelerator`. Bare Super is returned as `"Super"`; Escape cancels. Capture is
rejected while the session is locked, another capture is active, Mutter has an
input-capture session, or a compositor stage grab is active. Key events are
consumed by Mutter during capture and are not sent to Lua.

Configuration loading removes the standard Lua `os`, `io`, `debug`,
`package`, `dofile`, and `loadfile` globals. Do not use these as public
Gnoblin APIs.

### Current configuration document properties

The current `gnoblin.configure` document accepts these top-level properties.
Each is optional; omitted sections keep earlier values. The table records the
legacy config view and the standalone disposition of each property. Former
Shell-backed keys are not part of a second runtime or compatibility session.

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

The linked pages give the full current nested schema. The standalone runtime
keeps Gnoblin-owned policy and omits GNOME Shell preferences and actions.
External shell projects own their UI and its configuration.

### Declaration types

| Record           | Required fields                                             | Optional fields                                                                                                                  |
| ---------------- | ----------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `WindowRule`     | `match`                                                     | Existing compositor-owned fields in the [window-rule schema](../docs/config/window_rule.md), without shell-only animation events |
| `PermissionRule` | `name`, `match`, nonempty `capabilities`, `level`           | `monitors` for `screen-cast` or `remote-desktop`; `devices` and `clipboard` for `remote-desktop`                                 |
| `Shortcut`       | `name`, `binding`, and exactly one of `command` or `action` | `trigger`, `capture_input`, `enable`                                                                                             |
| `Animation`      | `name`, `event`, and `from`/`to` or `keyframes`             | `enable`, `duration`, `ease`, `origin`, `target`                                                                                 |
| `Autostart`      | `name`, nonempty `command` array                            | `when = "on_login"`, `enable`                                                                                                    |

The precise fields, enum members, and defaults are validated by the current
configuration schema. The target API reuses those schema types rather than
create a second spelling. `PermissionRule.match` and the string fields in
`WindowRule.match` share Lua 5.4 pattern syntax, but match different values:
verified portal identities for permission rules and window properties for
window rules. Runtime operations act on resolved object IDs rather than
re-evaluating configuration rules.

`WindowRule.match.app_id`, `title`, and `layer` use Lua 5.4
`string.find` pattern semantics. Patterns are byte-oriented and search anywhere
unless anchored with `^` or `$`. Lua classes and ranges, quantifiers, captures
and backreferences, `%f[set]` frontiers, and `%bxy` balanced pairs are
supported; JavaScript and PCRE regular-expression syntax is not. The pure-C
matcher is shared by config validation and compositor matching. It limits
patterns to 4,096 bytes, subjects to 16,384 bytes, matching work to 1,000,000
steps, and recursion to 128 levels. An ordinary non-match has no error;
malformed patterns and limit exhaustion are returned as distinct errors. The
loader rejects invalid rule patterns before committing a candidate config, so
a failed validation leaves the last active config in place. The standalone
Mutter open-animation path uses this matcher and keeps malformed-pattern and
limit errors distinct from an ordinary non-match.

## Target Lua API

All names in this section are **proposed** unless they are also listed in the
current inventory below. A returned record is a read-only snapshot. Its
methods are convenience wrappers over the canonical typed operation names
listed in the method tables.

The standalone runtime has no GNOME Shell feature registry, script manager,
event source, or adapter. Rows explicitly marked as legacy below describe
pre-cutover interfaces and are not registered by the standalone runtime.

### Root and events

| Member                                       | Signature                     | Result                                                                                                   |
| -------------------------------------------- | ----------------------------- | -------------------------------------------------------------------------------------------------------- |
| `gnoblin.settings`                           | read-only property            | `Settings` snapshot with a `revision`; available after the first config commit in the standalone runtime |
| `gnoblin.version()`                          | `()`                          | `Version`                                                                                                |
| `gnoblin.capabilities.list()`                | `()`                          | `Capability[]`                                                                                           |
| `gnoblin.events.on(name, callback)`          | `(string, function)`          | `Subscription`                                                                                           |
| `gnoblin.events.once(name, callback)`        | `(string, function)`          | `Subscription`                                                                                           |
| `gnoblin.events.mutter.on(name, callback)`   | `(MutterEventName, function)` | `Subscription`; unstable Mutter events                                                                   |
| `gnoblin.events.mutter.once(name, callback)` | `(MutterEventName, function)` | `Subscription`; one unstable Mutter event                                                                |
| `subscription:unsubscribe()`                 | `()`                          | `nil`; safe to call more than once                                                                       |

`gnoblin.on` is a current compatibility alias for `gnoblin.events.on`.
Callbacks receive one event record with `name`, `sequence`, `time`, and
the event-specific fields. A callback error is logged and does not prevent
other listeners from running.

### Focus and activation

| Lua call or property             | Arguments                                   | Result                 | Canonical operation |
| -------------------------------- | ------------------------------------------- | ---------------------- | ------------------- |
| `gnoblin.focus.policy`           | read-only property                          | `FocusPolicy` snapshot | state read          |
| `gnoblin.focus.history(filter?)` | `workspace_id?`, `monitor_id?`, `limit?`    | `Window[]`             | native state read   |
| `window:focus(context)`          | `FocusContext` from a user-originated event | `Operation<{id}>`      | `window.focus`      |

The focus policy uses the existing `window_management` settings.
The read-only `gnoblin.focus.policy` property returns a `FocusPolicy` with
these fields:

| Field                          | Type and accepted values             | Current default; standalone proposal and effect                                                                   |
| ------------------------------ | ------------------------------------ | ----------------------------------------------------------------------------------------------------------------- |
| `focus_mode`                   | `"click"`, `"sloppy"`, or `"mouse"`  | `"click"`; controls whether pointer entry changes focus.                                                          |
| `focus_new_windows`            | `"strict"` or `"smart"`              | `"strict"`; prevents activation requests without valid launch or user context from interrupting the current task. |
| `raise_on_click`               | boolean                              | `true`; raise a window when clicked.                                                                              |
| `auto_raise`                   | boolean                              | `false`; raise the focused window automatically.                                                                  |
| `focus_change_on_pointer_rest` | boolean                              | `false`; delay pointer-follow focus until the pointer rests.                                                      |
| `auto_raise_delay`             | integer from 0 to 10000 milliseconds | `500`; delay before automatic raise.                                                                              |
| `revision`                     | integer                              | Revision of this policy snapshot.                                                                                 |

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

`"strict"` uses Mutter's activation and transient-parent checks for
application-originated requests. Mutter honors a request when its recent user
or launch context is valid; an unapproved request remains unfocused and may
mark the window as demanding attention. `"smart"` focuses a new window even
when its activation context is weak. This is convenient for apps that do not
provide activation metadata, but can let an app interrupt the current task, so
it remains an opt-in compatibility choice. Supervised Lua callbacks use their
one-use `FocusContext` for an explicit selection. An independent shell client
uses the socket `window.focus` operation with an XDG Activation token instead.

`gnoblin.focus.history()` is available in the native runtime. It returns live
window snapshots, including minimized windows, ordered by most-recently
focused events observed by this runtime. The current focused window seeds the
order when a snapshot is installed. Other windows already open at startup, or
new windows not yet focused, follow snapshot order until a focus event places
them in the MRU order. Closed windows are removed. Filters match workspace and
monitor IDs; `limit` is 1–256 and defaults to 50. A shell client can display
this list and activate a selected window through the local `window.focus`
operation with an XDG Activation token obtained after its own user input.
Gnoblin requires the process that created the token to own the socket
connection, and Mutter validates the token's source surface and input serial.
The token is single-use and is not a Lua value.

The Lua `window:focus(context)` method instead requires a `FocusContext`: an
opaque, single-use value issued for a real user action. A Lua callback that
handles a user-originated Gnoblin event receives the context on the event
record. The compositor validates its lifetime; Lua cannot construct or
inspect it. Missing, expired, already-used, or mismatched context fails with
`denied` and does not change keyboard focus. Calls from timers, startup hooks,
or application callbacks have no context and cannot force focus. The context
is not a client-chosen string.

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

| Lua method                             | Arguments                      | Canonical operation          | Effect                                                                                                            |
| -------------------------------------- | ------------------------------ | ---------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `:focus(context)`                      | `FocusContext`                 | `window.focus`               | Restore and raise the window, activate its workspace, and request keyboard focus for an explicit shell selection. |
| `:close()`                             | none                           | `window.close`               | Ask the client to close using the normal protocol.                                                                |
| `:minimize()`                          | none                           | `window.minimize`            | Minimize if supported.                                                                                            |
| `:toggle_minimize()`                   | none                           | `window.toggle_minimize`     | Minimize a normal window or restore a minimized one.                                                              |
| `:restore()`                           | none                           | `window.restore`             | Unminimize and unmaximize as needed to show the window.                                                           |
| `:restore_or_minimize()`               | none                           | `window.restore_or_minimize` | Unmaximize, restore the saved pre-snap frame, or minimize when no frame is saved.                                 |
| `:set_above(enabled)`                  | boolean                        | `window.set_above`           | Set the above state idempotently.                                                                                 |
| `:set_sticky(enabled)`                 | boolean                        | `window.set_sticky`          | Set visibility across workspaces idempotently.                                                                    |
| `:set_maximized(enabled)`              | boolean                        | `window.set_maximized`       | Set the maximized state idempotently.                                                                             |
| `:set_fullscreen(enabled)`             | boolean                        | `window.set_fullscreen`      | Set the fullscreen state idempotently.                                                                            |
| `:move(position)`                      | `{x, y}`                       | `window.move`                | Move the frame in logical compositor coordinates.                                                                 |
| `:resize(size)`                        | `{width, height}`              | `window.resize`              | Request a size in logical pixels, respecting client constraints.                                                  |
| `:move_to_workspace(target, options?)` | `WorkspaceSelector`; `follow?` | `window.move_to_workspace`   | Move this window; optionally activate the destination.                                                            |
| `:move_to_monitor(target)`             | `MonitorSelector`              | `window.move_to_monitor`     | Move this window to a monitor.                                                                                    |
| `:begin_move(context)`                 | `FocusContext`                 | `window.begin_move`          | Start Mutter's keyboard move grab with a trusted shortcut context.                                                |
| `:begin_resize(edge, context)`         | `ResizeEdge`, `FocusContext`   | `window.begin_resize`        | Start Mutter's keyboard resize grab at the selected edge.                                                         |

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

The standalone Lua runtime does not expose the generic `window.action(args)`
dispatcher. Each compositor action is a typed `Window` method. The raw
compositor socket retains `window.action` for compatibility with existing
socket clients; Lua callers use the snapshot methods above. The old `menu`
action has no compositor equivalent: an external shell client owns its menu UI
and can draw it on a layer-shell surface from the window snapshot.

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

| Lua call                                  | Arguments                                             | Result                     | Canonical operation     |
| ----------------------------------------- | ----------------------------------------------------- | -------------------------- | ----------------------- |
| `gnoblin.workspaces.list()`               | none                                                  | `Workspace[]`              | `workspace.list`        |
| `gnoblin.workspaces.active()`             | none                                                  | `Workspace or nil`         | state read              |
| `gnoblin.workspaces.by_id(id)`            | stable workspace ID                                   | `Workspace or nil`         | state read              |
| `gnoblin.workspaces.create(options)`      | `name`, optional `id`, `activate`                     | `Operation<Workspace>`     | `workspace.create`      |
| `gnoblin.workspaces.rename(options)`      | workspace selector and `name`                         | `Operation<Workspace>`     | `workspace.rename`      |
| `gnoblin.workspaces.remove(options)`      | workspace selector                                    | `Operation<Workspace>`     | `workspace.remove`      |
| `gnoblin.workspaces.activate(options)`    | workspace selector                                    | `Operation<Workspace>`     | `workspace.switch`      |
| `gnoblin.workspaces.next()`               | none                                                  | `Operation<Workspace>`     | `workspace.next`        |
| `gnoblin.workspaces.previous()`           | none                                                  | `Operation<Workspace>`     | `workspace.previous`    |
| `gnoblin.workspaces.move_active(options)` | workspace selector and optional `follow`              | `Operation<WorkspaceMove>` | `workspace.move_active` |
| `gnoblin.workspaces.move_window(options)` | window and workspace selectors, optional `follow`     | `Operation<WorkspaceMove>` | `workspace.move_window` |
| `workspace:activate()`                    | none                                                  | `Operation<Workspace>`     | `workspace.switch`      |
| `workspace:rename(name)`                  | nonempty name, at most 80 characters                  | `Operation<Workspace>`     | `workspace.rename`      |
| `workspace:remove()`                      | none                                                  | `Operation<Workspace>`     | `workspace.remove`      |
| `workspace:move_here(window, options?)`   | `Window`, window ID, or `"active"`; optional `follow` | `Operation<WorkspaceMove>` | `workspace.move_window` |

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

| Lua call                                | Arguments                                          | Result                        | Canonical operation                                                 |
| --------------------------------------- | -------------------------------------------------- | ----------------------------- | ------------------------------------------------------------------- |
| `gnoblin.input.devices()`               | none                                               | `InputDevice[]`               | `input.devices`                                                     |
| `gnoblin.input.sources()`               | none                                               | `InputSource[]`               | `input.sources`                                                     |
| `gnoblin.input.current_source()`        | none                                               | `InputSource or nil`          | state read                                                          |
| `gnoblin.input.select_source(selector)` | `{type, id}` source selector                       | `Operation<InputSource>`      | `input.select`                                                      |
| `gnoblin.input.text_target(context)`    | live `FocusContext` from shortcut event            | `Operation<TextTarget>`       | Lua wrapper for `input.text_target`; socket counterpart is API 1.28 |
| `target:insert_text(text)`              | UTF-8 text from 1 to 256 bytes, without controls   | `Operation<{inserted}>`       | Lua wrapper for `input.insert_text`; socket counterpart is API 1.28 |
| `gnoblin.shortcuts.list()`              | none                                               | `ShortcutState[]`             | `shortcut.list`                                                     |
| `gnoblin.shortcuts.actions(group?)`     | optional group: `"wm"`, `"mutter"`, or `"wayland"` | `ShortcutAction[]`            | socket read `shortcuts.actions` (API 1.41)                          |
| `gnoblin.shortcuts.capture(options?)`   | `timeout?` seconds                                 | `Operation<CapturedShortcut>` | `shortcut.capture`                                                  |

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
selection because the native runtime has no IBus engine client. The standalone
runtime supports the documented XKB source-selection path only.

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
standalone target does not register the `gnome:shell` group. Declare a binding
for an action with `action = action.id` in `gnoblin.configure.shortcuts`.
`gnoblin.shortcuts.bind()` registers a Gnoblin shortcut event and does not
invoke a schema action.

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
| `gnoblin.animations.get(name)`     | animation name                                                | `AnimationInfo or nil`        | `animation.get`     |
| `gnoblin.animations.preview(spec)` | `name`, `target`, optional `event`, `target_type`, `autoplay` | `Operation<AnimationPreview>` | `animation.preview` |
| `preview:seek(progress)`           | number from 0 to 1                                            | `Operation<AnimationPreview>` | `animation.seek`    |
| `preview:step(milliseconds)`       | integer from 1 to 60000                                       | `Operation<AnimationPreview>` | `animation.step`    |
| `preview:play()`                   | none                                                          | `Operation<AnimationPreview>` | `animation.play`    |
| `preview:pause()`                  | none                                                          | `Operation<AnimationPreview>` | `animation.pause`   |
| `preview:stop()`                   | none                                                          | `Operation<{ok, session}>`    | `animation.stop`    |

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
and integer `revision`. When an operation completes with an
`AnimationPreview` value, that immutable record also exposes `seek`, `step`,
`play`, `pause`, and `stop` methods. Each method returns a new `Operation`;
the completed preview value from `seek`, `step`, `play`, and `pause` has the
same methods. `stop` returns `{ok = true, session = string}`.
`Capability` fields: string `id` and `description`; boolean `available`;
optional string `reason`; and integer `revision`. Capabilities report which
APIs or protocols are supported; version negotiation reports methods and
events. These are compositor and protocol capabilities, not a Shell feature
registry.

The standalone runtime advertises `window-thumbnails` for bounded window
previews, `session-activity` for idle-monitor state, and `microphone-monitor`
for PipeWire microphone activity monitoring. The microphone capability is
available only when the Mutter build includes remote-desktop support and
PipeWire is connected; its unavailable record includes `remote_desktop_disabled` or
`pipewire_unavailable` as its reason. Native API 1.33 emits
`gnoblin.capability.changed` after updating the capability snapshot when this
availability changes. Check the snapshot before using an optional feature.

The API controls compositor animation specifications and reports compositor
capabilities. It does not expose draw calls, shaders, arbitrary Mutter
objects, or shell widgets.

### Owner-scoped shortcut sessions

`gnoblin.shortcuts.bind(options)` registers an in-memory global shortcut and
returns an `Operation` whose result contains the binding ID and normalized
options. `gnoblin.shortcuts.unbind {id = ...}` removes that registration.
Options are `id`, `accelerator`, `hold`, `trigger`, `mode`, and
`capture_input`. `id` is 1–64 ASCII letters, digits, underscores, or hyphens.
`accelerator` is a nonempty GTK accelerator string of at most 128 bytes.
`hold` is `"none"`, `"super"`, `"control"`, or `"alt"`; it defaults to
`"none"`. `trigger` is `"press"` or `"release"` and defaults to `"press"`.
`mode` is `"passive"` or `"modal"` and defaults to `"passive"`. Modal mode
requires a held modifier. `capture_input` defaults to `false`; setting it to
`true` is supported only for a bare `"Super"` binding. A bare `"Super"`
binding requires `capture_input = true`, `trigger = "release"`, and
`hold = "none"`. Registration fails with `unsupported` when the Mutter early
modifier hook is unavailable.

Passive bindings leave keyboard input with applications. Modal bindings use
Mutter's native keyboard capture while the held modifier remains down. The
session ends when that modifier is released, the binding is unbound, its owner
is disconnected, the config reloads, the session locks, another capture takes
over, or ten seconds elapse. A session cannot outlive its Lua runtime
generation or socket connection. The shell owns pointer input and UI; Gnoblin
does not capture pointer events for shortcut sessions.

Lua calls and native-control socket calls use the same `shortcut.bind` and
`shortcut.unbind` operation schemas. Lua registrations belong to the active
runtime generation. Socket registrations belong to the authenticated client
connection. Both are removed with their owner. Socket clients need API 1.22 to
request held or modal options and to subscribe to the session events below.

| Event                                  | Fields                                                                                                                       | Meaning                                                                                                                                                                                                                                                                                                               |
| -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `gnoblin.shortcut.binding-activated`   | `id`, `accelerator`, `trigger`, `first`, `modifiers`, `time`, `input_time`, `session_id` when held, optional `focus_context` | A compositor-verified first activation. Only this first activation can carry focus authority.                                                                                                                                                                                                                         |
| `gnoblin.shortcut.binding-deactivated` | `id`, `accelerator`, `input_time`                                                                                            | The physical accelerator was released after a press-triggered activation. Socket clients need native-control API 1.36.                                                                                                                                                                                                |
| `gnoblin.shortcut.session.activated`   | `id`, `session_id`, `first`, `trigger`, `modifiers`, `time`                                                                  | A held binding activated. `first` is false for repeated accelerator activations in the same session.                                                                                                                                                                                                                  |
| `gnoblin.shortcut.session.key`         | `id`, `session_id`, `keyval`, `keycode`, `modifiers`, `phase`, `time`                                                        | A modal keyboard event. `phase` is `"press"` or `"release"`.                                                                                                                                                                                                                                                          |
| `gnoblin.shortcut.session.ended`       | `id`, `session_id`, `reason`, `time`                                                                                         | The session ended. `reason` is `"released"`, `"unbound"`, `"owner_disconnected"`, `"config_changed"`, `"locked"`, `"preempted"`, `"timed_out"`, `"compositor_stopped"`, or `"runtime_stopped"`. `runtime_stopped` is socket-only and occurs when the Lua runtime stops while a socket client owns the active session. |

`binding-deactivated` pairs with `binding-activated` for press-triggered
socket bindings. Release-triggered bindings activate on release and do not emit
a second deactivation event. `input_time` is Mutter's event timestamp in
milliseconds and uses the same clock as the activation event's `input_time`.

Only the owner receives session events. Lua receives an opaque `FocusContext`
userdata on the first trusted `binding-activated` event. Socket clients receive
a connection-bound one-use `focus_context` token on that event. Native handles
and generations are private compositor metadata; they are never included in
Lua tables or public socket events. Repeats, key events, and session-ended
events do not carry focus authority. A binding alone never authorizes a focus
change.

```lua
local binding

gnoblin.shortcuts.bind {
    id = "window-switcher",
    accelerator = "<Super>space",
    hold = "super",
    trigger = "press",
    mode = "modal",
}:on_complete(function(value, err)
    if not err then binding = value end
end)

gnoblin.events.on("gnoblin.shortcut.session.activated", function(event)
    if event.id == "window-switcher" then
        switcher:step(event.first)
    end
end)

gnoblin.events.on("gnoblin.shortcut.session.key", function(event)
    if event.id == "window-switcher" and event.phase == "press" then
        switcher:handle_key(event.keyval, event.modifiers)
    end
end)
```

Bare Super can capture type-ahead input only when the Mutter early modifier
hook is available. The hook arms for an explicitly registered bare-Super
binding and leaves other Super combinations on Mutter's normal path.

```lua
gnoblin.shortcuts.bind {
    id = "search",
    accelerator = "Super",
    trigger = "release",
    capture_input = true,
}
```

#### Shell presentation requests

Mutter window-menu requests are exposed as `gnoblin.window.menu-requested` with
`window_id`, `menu_type` (`wm` or `app`), and global logical `x`/`y`. Mutter OSD
requests are exposed as `gnoblin.osd.requested` with stable `monitor_id` and the
optional `icon` and `label` Mutter supplied. The request contains no level,
maximum, or output list. These API 1.27 events let the shell own all UI.

API 1.30 adds a non-serializable `MenuContext` userdata to Lua WM-menu event
callbacks. `event.menu_context:begin_move()` and
`event.menu_context:begin_resize(edge)` authorize one keyboard grab on the
exact live window that raised that WM menu. The context is valid only in its
callback, expires after five seconds, and is revoked by lock or runtime/config
teardown. App-menu events carry no authority. Socket clients receive a
per-connection opaque `menu_context` token and use the same operations without
passing a window ID; see the runtime API and compositor bridge references.

#### Pointer and keyboard snapping

The supervised Lua runtime exposes Mutter's pointer move lifecycle through
`gnoblin.window.drag.started`, `gnoblin.window.drag.updated`, and
`gnoblin.window.drag.ended`. Started and updated events include an opaque
`event.drag` record with read-only `id`, `window_id`, `settings_revision`,
`pointer`, `modifiers`, `monitor_id`, `monitor`, `work_area`, `frame`, and
`maximized` properties. It applies only to mouse move grabs. Shells own guides
and picker presentation; Mutter retains pointer ownership and applies a frame
only after a matching release-time offer. External layer-shell shells can use
the public control socket API 1.26: subscribe to the three drag events before
the drag starts, then submit `window.snap.offer` with the `drag_id`, the
connection's `drag_token`, and the target list. Each subscribed connection
gets a different unpredictable token for the live drag. The first accepted
offer, from Lua or a socket client, owns the target list; only that owner can
replace it. A socket owner's disconnect or event-subscription replacement
clears its offer. The compositor validates the actual release state before
applying a target. Keyboard `SnapContext` remains Lua-only.

`event.drag:offer_targets(targets)` accepts between 1 and 128 targets. Each
target has a unique `id`, `hit` and `frame` rectangles, and optional `maximize`,
`required_modifiers`, and `forbidden_modifiers`. Rectangles use integer logical
coordinates and `{x, y, width, height}` fields. Both rectangles must fit in the
current work area. Modifier arrays currently accept only `"control"`, which
must not appear in both arrays. The compositor checks its observed pointer and
modifier state on release; no synchronous Lua request occurs in that path. If
no offered region matches, normal Mutter move and tile-preview behavior
continues. Offers and contexts are invalidated on release, cancellation,
lock, config reload, owner/runtime loss, or window loss.

Trusted keyboard layout selection uses a separate `SnapContext`:

| Lua call                                | Arguments             | Result                                          |
| --------------------------------------- | --------------------- | ----------------------------------------------- |
| `gnoblin.windows.snap_context(context)` | Live `FocusContext`   | `Operation<SnapContext>`                        |
| `snap_context:commit(target)`           | `monitor_id`, `frame` | `Operation<{window_id, monitor_id, committed}>` |

The compositor chooses the currently focused window; the caller cannot pass a
window ID. The returned context exposes `window_id`, `monitor_id`, `monitor`,
`work_area`, and `expires_at_us`. Commit is one-use and rechecks the window,
monitor, lock state, runtime generation, and work-area bounds. The parent Lua
runtime exposes these operations as `gnoblin.windows.snap_context` and
`snap_context:commit()`. Native API 1.28 also exposes `window.snap_context` and
`window.snap` to socket clients, using connection-bound tokens; see the
[compositor bridge](/compositor-bridge#api-128-text-insertion-and-keyboard-snapping).
Pointer snap offers are also available through the capability-bound socket API
above.

```json
{"op":"events","api_version":{"major":1,"minor":26},"events":["gnoblin.window.drag.started","gnoblin.window.drag.updated","gnoblin.window.drag.ended"]}
{"op":"api","api_version":{"major":1,"minor":26},"id":"snap-1","method":"window.snap.offer","arguments":{"drag_id":42,"drag_token":"<token from the event>","targets":[{"id":"left","hit":{"x":0,"y":0,"width":700,"height":900},"frame":{"x":0,"y":0,"width":700,"height":900}}]}}
```

The token is bound to the receiving connection, live drag, and runtime
generation. A stale token, a token from another connection, or an offer after
release, lock, reload, or owner loss is rejected. Offer geometry remains
bounded by the compositor's current work area.

```lua
gnoblin.events.on("gnoblin.window.drag.updated", function(event)
    event.drag:offer_targets({
        {
            id = "left-edge",
            hit = left_hit,
            frame = left_frame,
            forbidden_modifiers = {"control"},
        },
        {
            id = "control-layout",
            hit = layout_hit,
            frame = layout_frame,
            required_modifiers = {"control"},
        },
    })
end)

gnoblin.events.on("gnoblin.shortcut.activated", function(event)
    local operation = gnoblin.windows.snap_context(event.focus_context)
    operation:on_complete(function(context, err)
        if not err then
            context:commit({monitor_id = context.monitor_id, frame = target_frame})
        end
    end)
end)
```

#### Window thumbnails

Supported since API 1.23:

| Lua call                 | Arguments                                         | Result                 | Canonical operation |
| ------------------------ | ------------------------------------------------- | ---------------------- | ------------------- |
| `window:thumbnail(size)` | `{width = integer 1–480, height = integer 1–320}` | `Operation<Thumbnail>` | `window.thumbnail`  |

`Thumbnail` contains `window_id`, actual `width` and `height`, and `data`, a
base64-encoded PNG. The compositor scales down to fit while preserving aspect
ratio. This is an asynchronous, bounded compositor-rendered preview; it does
not expose a Mutter actor or texture. A request is rejected while the session
is locked. The compositor rechecks the stable window ID after capture, permits
one active request per socket client and four across the session, and drops the
result if the requester disconnects or the window closes. Encoded PNG output
is capped at 512 KiB. Thumbnail data is never persisted by Gnoblin.

The preview is a rendering of the window actor. Mutter 51 exposes no
protected-content metadata or capture-redaction guarantee, so Gnoblin makes no
promise that DRM or other protected content will be hidden. Use this API only
within the same-user session trust boundary described by the native-control
socket contract.

#### Trusted text target and insertion

The native Lua runtime exposes `gnoblin.input.text_target(context)` as an
asynchronous operation. Call it from a shortcut event callback with that
event's live `FocusContext`. The compositor consumes the context when it
handles the request and returns an opaque, one-use `TextTarget` only if the
same Wayland surface and client still have focus and an active text-input-v3
session. Socket clients have equivalent `input.text_target` and
`input.insert_text` methods that use connection-bound tokens; see the
[compositor bridge](/compositor-bridge#api-128-text-insertion-and-keyboard-snapping).
X11 is unsupported.

The target expires with the context's five-second deadline. Lock, focus loss,
runtime reload, or runtime disconnect revokes it. `TextTarget` exposes an
optional logical-coordinate `caret` rectangle and stable `window_id` for shell
presentation; neither field authorizes insertion.

`target:insert_text(text)` consumes the target on its first attempt. Text is
limited to 1-256 bytes of valid UTF-8 without NUL or control characters.
Insertion rechecks the same Wayland surface, client, focus epoch, and active
text-input-v3 state. It requires an unlocked session. Modifiers held when the
shortcut activated may remain held; adding another modifier invalidates the
target. Ctrl, Alt, Shift, Lock, Meta, and Hyper state prevents target creation.
Mutter then commits via its focused input-method path.

#### Layer blur regions

Blur regions are not a Lua API. The Wayland client that owns a surface sets a
surface-local `wl_region` through `ext-background-effect-v1`; Wayland resource
ownership binds the request to that exact surface. Region changes follow the
surface commit and are clipped to its size. The protocol controls shape, not
blur strength or effect policy. Gnoblin configuration can disable the global,
and window rules determine blur strength. See the
[background-effect guide](../docs/background-effects.md).

The former Shell bridge accepted screen-coordinate regions keyed by PID, layer
namespace, and screen origin. Gnoblin replaced that private channel with the
surface-owned Wayland protocol.

#### Effective layer-animation policy

Current read:

| Lua call                                     | Arguments                          | Result                 |
| -------------------------------------------- | ---------------------------------- | ---------------------- |
| `gnoblin.layers.animation_policy(namespace)` | layer namespace, 1–128 UTF-8 bytes | `LayerAnimationPolicy` |

`LayerAnimationPolicy` contains `namespace`, `enter`, `exit`, `window_shadow`,
and `revision`. Each phase is a `LayerAnimationPhase` with `animation` (the
selected built-in or registered animation name), optional `duration` in
milliseconds, and optional `easing`. `easing` uses the same named curve or
`{type = "cubic-bezier", x1, y1, x2, y2}` shape accepted by `AnimationInfo.ease`
above. The phase field names and values match the resolved policy consumed by
Bingux. A missing `duration` or `easing` means the selected preset supplies that
value. This compact effective view does not
replace the full registered declaration: `gnoblin.animations.list()` and
`gnoblin.animations.get(name)` continue to expose `AnimationInfo`, including
keyframes and event-specific properties. The query reports the effective
policy for that namespace after matching committed animation declarations and
layer window rules. With no matching animation override, both phases select
`slide`; `window_shadow` defaults to `false` unless a matching default-window
rule supplies a shadow value. It is a read-only runtime snapshot and raises a
Lua error if the committed settings cannot be read or matched.
Configuration continues to declare animations and matching rules through the
existing shared `gnoblin` config API; a policy query does not mutate config or
select a shell transition. Bingux or another shell client decides how its
surfaces respond, while Gnoblin remains authoritative for compositor animation
and shadow policy.

The query uses namespace as the existing rule-matching key, not as a unique
layer-surface identity.

### Permissions, privacy, and portals

| Lua call                                          | Arguments                       | Result                            | Canonical operation      |
| ------------------------------------------------- | ------------------------------- | --------------------------------- | ------------------------ |
| `gnoblin.privacy.state()`                         | none                            | `PrivacyState`                    | state read               |
| `gnoblin.privacy.stop_sharing()`                  | none                            | `Operation<{requested: integer}>` | `privacy.stop_sharing`   |
| `gnoblin.privacy.stop_recording()`                | none                            | `Operation<{requested: integer}>` | `privacy.stop_recording` |
| `gnoblin.permissions.policy()`                    | none                            | `PermissionPolicy`                | state read               |
| `gnoblin.permissions.check(capability, identity)` | capability and identity strings | `PermissionDecision`              | state read               |
| `gnoblin.portals.grants()`                        | optional `kind`                 | `PortalGrant[]`                   | state read               |
| `grant:revoke()`                                  | none                            | `Operation<nil>`                  | `portal.grant.revoke`    |

`PrivacyState` contains an `available` record with boolean fields
`screen_sharing`, `recording`, `microphone_in_use`, `camera_in_use`, and
`location_in_use`, plus a `revision`. Each matching activity field is an
optional boolean. Gnoblin omits it when its source is unavailable; consumers
must not interpret unavailable state as inactive.

The two stop methods request closure of every tracked Mutter remote-access
handle in the selected class: non-recording handles for `stop_sharing()` and
recording handles for `stop_recording()`. Their result contains integer
`requested`, the number of handles passed to
`meta_remote_access_handle_stop()`. This confirms the stop calls were issued;
it does not confirm that a session has closed. Observe
`gnoblin.privacy.changed` and `gnoblin.privacy.state()` for the later state
reported after Mutter signals that a handle stopped. These methods do not
revoke persistent portal grants.

`PermissionPolicy` fields are `default` (one of `"default"`, `"ask"`,
or `"deny"`), `rules` (ordered `PermissionRule[]`), and `revision`.
Global `"allow"` is invalid; allow decisions must name an explicit matching
rule. A `PermissionRule` has a unique `name` of 1–80 ASCII letters, digits,
periods, underscores, or hyphens; a Lua 5.4 pattern from 1–512 bytes,
using the same matching syntax as window-rule string fields; a nonempty
`capabilities` array; and a `level` of
`"default"`, `"ask"`, `"allow"`, or `"deny"`. The expression matches the full
verified portal identity, such as `app-id:org.example.App` or
`host-exe:/usr/bin/example`, never a window title or Wayland `app_id` supplied
by an arbitrary client.
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

| Lua call                                      | Arguments                                      | Result                    | Canonical operation          |
| --------------------------------------------- | ---------------------------------------------- | ------------------------- | ---------------------------- |
| `gnoblin.session.status()`                    | none                                           | `SessionStatus`           | state read                   |
| `gnoblin.session.activity()`                  | none                                           | `SessionActivity`         | native activity snapshot     |
| `gnoblin.session.lock()`                      | none                                           | `Operation<LockRequest>`  | `session.lock`               |
| `gnoblin.session.logout()`                    | none                                           | `Operation<{accepted}>`   | `session.logout`             |
| `gnoblin.session.restart_compositor(reason?)` | optional reason string, at most 256 characters | `Operation<nil>`          | `session.restart_compositor` |
| `gnoblin.runtime.reload_config()`             | none                                           | `Operation<ReloadResult>` | `runtime.reload_config`      |
| `gnoblin.launches.list()`                     | none                                           | `Launch[]`                | native launch snapshot       |
| `gnoblin.launches.snapshot()`                 | none                                           | `{launches, revision}`    | collection snapshot          |
| `gnoblin.launches.begin(options)`             | `token`, `application`, optional `timeout_ms`  | `Operation<Launch>`       | `launch.begin`               |
| `gnoblin.launches.end(token)`                 | launch token                                   | `Operation<{ok, token}>`  | `launch.end`                 |

`ReloadResult` contains `ok = true`, `action = "config reload"`, the committed
`settings_revision`, and the new `runtime_generation`. Reload stages the
candidate while keeping the active Lua runtime in place. The supervisor waits
asynchronously for active-runtime operations and their deferred Lua completion
callbacks to finish, including operations those callbacks enqueue. It dispatches
those callbacks on the supervisor main loop, then sends a correlated
configuration transaction to Mutter. Mutter validates
and applies the supported changes, and replies with the same transaction ID,
revision, and generation. Only an accepted reply commits the staged Lua
runtime and completes the API operation. A rejected reply discards the
candidate and returns an error; the active runtime remains in place. Events, state snapshots, and API requests received during the transaction
are queued and dispatched after the result, against whichever runtime remains
active. Settings that require a
new session remain unchanged by reload. If the session stops before the
transaction finishes, the candidate is discarded. On shutdown, the server
queues an error for outstanding requests but may close the connection before
it flushes, so clients can receive EOF.

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

`LockRequest` fields: `dispatched` (always `true` on success) and `subscribers`
(the number of connected clients subscribed to the request event when Gnoblin
targets the request). The count is not a delivery acknowledgement. The
operation fails if the compositor cannot provide session locking or no shell
client is subscribed. Completion confirms request delivery only; it does not
confirm that the session is covered or locked. Subscribe to
`gnoblin.session.lock-state-changed` for compositor state. There is no Lua
unlock method: only the active Wayland session-lock owner can unlock.

`gnoblin.session.activity()` returns the latest native idle-monitor sample.
Its `available` field is false when Mutter's idle monitor cannot be queried.
When `idle` is true, `idle_for_ms` advances from the last sample using the
supervisor's monotonic clock. Activity-change events report the duration
sampled when the state changed; treat that event value as a sample, not a live
counter. The event also carries `threshold_ms`, `revision`, `sequence`, and
monotonic-clock `time`. This state is independent of idle inhibitors and the
GNOME idle-delay setting.

`SessionActivity` contains `available`, `idle`, `threshold_ms`, `idle_for_ms`,
and `revision`. The fixed idle threshold is 120000 milliseconds. When
`available` is false, `idle` is false and `idle_for_ms` is zero.

`SessionLockState` is `"unlocked"`, `"covering"`, `"locked"`, or `"failsafe"`.
`covering` means the compositor has started its lock transition; `locked` is
the compositor-confirmed locked state; `failsafe` means the lock client failed
and the compositor retained its fail-safe state. Treat all states except
`unlocked` as unavailable for normal session actions. Only `locked` confirms a
compositor-secured session.

`SessionStatus` is available as a live API read from API 1.29. It contains
`state = "running"` and `lock_available`. When lock state is available, it
also contains `lock_state`, a `SessionLockState`. If it is unavailable,
`lock_state` is omitted; unavailable does not mean unlocked. The compositor
socket answers this read directly from Mutter and therefore remains available
if the Lua supervisor is disconnected while the compositor remains alive. It
reports compositor availability, not supervisor health. The socket cannot
report a final state after the compositor stops, so connection failure is the
only status available then.

The session host can now restart the Lua worker while keeping Mutter and its
Wayland clients alive. The worker recovery handshake restores the accepted
configuration and fresh compositor snapshots. Mutter cancels Lua-owned
shortcuts and temporary interaction state while the worker is absent. Operation
IDs remain monotonic across worker restarts: the suspension acknowledgement
passes Mutter's last accepted ID to the replacement, preventing a delayed
completion from the previous worker from matching a new operation. Mutter
discards touchpad gestures in progress when the worker stops and before the
replacement resumes. Worker recovery does not recover from a crash of the
`gnoblin` session host itself. The host owns the private compositor channel and
session lifecycle, and a replacement host cannot reconnect to the running
compositor. Preserving the compositor across host restart remains tracked in
[issue #70](https://github.com/kierandrewett/gnoblin/issues/70).

`Launch` fields: `token`, `application`, `started_at`, `timeout_ms`,
`state`, `revision`. `state` is `"pending"`, `"started"`,
`"failed"`, `"ended"`, or `"timed_out"`. Tokens are at most 128
characters; application names are at most 512. Timeout defaults to 3000 ms
and is clamped to 100–10000 ms.

`session.restart_compositor()` remains a proposal and must not promise that
client windows survive: Wayland clients normally lose their connection when
the compositor exits. Lua config reload should not restart the compositor
unless a change is startup-only and the caller explicitly requests it.
`session.logout()` is implemented at native-control API 1.32. It completes with
`{accepted = true}` after the compositor accepts the request; the supervisor
then exits successfully and the session wrapper stops Gnoblin's user services.

### Current API migration map

The Lua runtime uses plural collection reads and typed workspace operations.
The compositor socket retains its `window.list` and `workspace.list` requests
for clients such as `gnoblinctl`; those names are not registered as Lua
methods. The table shows compositor operation names whose Lua equivalents
differ or which remain socket-only.

`gnoblinctl window match` selects from `windows.list` and formats the snapshot
as the established CLI result. The raw socket `window.match` request remains
for compatibility; Lua callers inspect and filter `gnoblin.windows.list()`.
Native-control API 1.46 routes `input.devices`, `input.sources`, and
`input.current_source` through their shared Lua methods; earlier clients retain
the native compatibility route.

| Compositor operation                                                   | Lua API method or decision                                                                                                                              |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `workspace.create`                                                     | `gnoblin.workspaces.create(options)`                                                                                                                    |
| `workspace.rename`                                                     | `workspace:rename(name)`                                                                                                                                |
| `workspace.remove`                                                     | `workspace:remove()`                                                                                                                                    |
| `workspace.switch`                                                     | `workspace:activate()`                                                                                                                                  |
| `workspace.next`                                                       | `gnoblin.workspaces.next()`                                                                                                                             |
| `workspace.previous`                                                   | `gnoblin.workspaces.previous()`                                                                                                                         |
| `workspace.move_active`                                                | `gnoblin.workspaces.active():move_here(window, options)`                                                                                                |
| `workspace.move_window`                                                | `window:move_to_workspace(target, options)`                                                                                                             |
| `window.match`                                                         | CLI formats its result from `windows.list`; Lua reads identity and rule fields from `Window` snapshots. The raw socket query remains for compatibility. |
| `window.action`                                                        | Removed from the standalone Lua API; the raw compositor-socket operation remains for compatibility. Lua uses the typed `Window` methods above.          |
| `layer.list`                                                           | `gnoblin.layers.list(filter)`                                                                                                                           |
| `monitor.list`                                                         | `gnoblin.monitors.list()`                                                                                                                               |
| `animation.list`                                                       | `gnoblin.animations.list()`                                                                                                                             |
| `animation.surfaces`                                                   | `gnoblin.animations.surfaces()`                                                                                                                         |
| `animation.inspect`                                                    | `gnoblin.animations.inspect(args)`                                                                                                                      |
| `animation.preview`                                                    | `gnoblin.animations.preview(spec)`                                                                                                                      |
| `animation.seek`                                                       | `gnoblin.animations.seek(args)` or `preview:seek(progress)`                                                                                             |
| `animation.step`                                                       | `gnoblin.animations.step(args)` or `preview:step(milliseconds)`                                                                                         |
| `animation.play`                                                       | `gnoblin.animations.play(args)` or `preview:play()`                                                                                                     |
| `animation.pause`                                                      | `gnoblin.animations.pause(args)` or `preview:pause()`                                                                                                   |
| `animation.stop`                                                       | `gnoblin.animations.stop(args)` or `preview:stop()`                                                                                                     |
| `feature.list` / `feature.show` / `feature.enable` / `feature.disable` | Removed; these toggled GNOME Shell-owned behavior and have no standalone target.                                                                        |
| `script.list`                                                          | Removed. The GNOME Shell script manager does not exist in the standalone session; Lua files are loaded through `gnoblin.load` and `require`.            |
| `input.devices`                                                        | `gnoblin.input.devices()`; physical devices are listed separately.                                                                                      |
| `input.list`                                                           | `gnoblin.input.sources()`                                                                                                                               |
| `input.current`                                                        | `gnoblin.input.current_source()`                                                                                                                        |
| `input.select`                                                         | `gnoblin.input.select_source({type, id})`                                                                                                               |
| `privacy.get`                                                          | `gnoblin.privacy.state()`                                                                                                                               |
| `privacy.stop_sharing`                                                 | `gnoblin.privacy.stop_sharing()`; Native-control API 1.31                                                                                               |
| `privacy.stop_recording`                                               | `gnoblin.privacy.stop_recording()`; Native-control API 1.31                                                                                             |
| `permissions.list`                                                     | `gnoblin.permissions.list()`; shared Lua read from native-control API 1.42, with the native route retained for older clients.                           |
| `permissions.policy`                                                   | `gnoblin.permissions.policy()`; shared Lua read from native-control API 1.44, with the native route retained for older clients.                         |
| `permissions.check`                                                    | `gnoblin.permissions.check(capability, identity)`; shared Lua read from native-control API 1.43, with the native route retained for older clients.      |
| `grant.list`                                                           | `gnoblin.portals.grants()`; shared Lua read from native-control API 1.45, with the native route retained for older clients.                             |
| `grant.revoke`                                                         | `grant:revoke()`                                                                                                                                        |
| `launch.status`                                                        | CLI uses `gnoblin.launches.snapshot()` to preserve the collection revision; retain the raw socket method for compatibility.                             |
| `launch.begin`                                                         | `gnoblin.launches.begin(options)` in a standalone native session.                                                                                       |
| `launch.end`                                                           | `gnoblin.launches.end(token)` in a standalone native session.                                                                                           |
| `shell.ping`                                                           | Removed; use the unversioned socket transport operation `op = "ping"`.                                                                                  |
| `shell.version`                                                        | `gnoblin.version()`                                                                                                                                     |
| `shell.status`                                                         | `gnoblin.session.status()`                                                                                                                              |
| `shell.reload`                                                         | Removed from the core API; reload the Lua runtime or restart a selected shell client through its own lifecycle.                                         |
| `session.lock`                                                         | `gnoblin.session.lock()`; a native request to a subscribed shell client. Completion means delivery, not lock confirmation.                              |
| `runtime.reload_config`                                                | `gnoblin.runtime.reload_config()`                                                                                                                       |
| `shortcut.list`                                                        | `gnoblin.shortcuts.list()`                                                                                                                              |
| `shortcut.actions`                                                     | CLI uses the shared Lua-backed `shortcuts.actions` read (API 1.41); retain the raw method for compatibility.                                            |
| `shortcut.capture`                                                     | `gnoblin.shortcuts.capture(options)`                                                                                                                    |

## Event catalog

### Events available today

This inventory includes historical hybrid-runtime rows to explain the
standalone migration. Only Mutter and Gnoblin rows describe events registered
by the current standalone runtime; GNOME Shell, feature, and script rows are
legacy. Mutter signal coverage can change with the pinned upstream version.

| Current event                                 | Fields                                                                                                 | Source or meaning                                                                                        |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------- |
| `gnome.shell.focus.changed`                   | `app_id`, `wm_class`, `title`                                                                          | Keyboard focus changes.                                                                                  |
| `gnome.shell.window.created`                  | `app_id`, `wm_class`, `title`                                                                          | Shell observes a new window.                                                                             |
| `gnome.shell.window.unmanaged`                | `app_id`, `wm_class`, `title`                                                                          | Shell removes a window.                                                                                  |
| `gnome.shell.input.<type>`                    | `type`, `time`, and input-specific fields                                                              | Captured Shell input event.                                                                              |
| `gnome.interface.color-scheme-changed`        | `color_scheme`: `default`, `prefer-dark`, or `prefer-light`                                            | Desktop appearance preference.                                                                           |
| `mutter.wayland.pointer-window-changed`       | `app_id`, `wm_class`, `title`; empty strings when no client surface is under pointer                   | Mutter pointer tracking.                                                                                 |
| `mutter.touchpad.gesture`                     | `gesture`, `phase`, `fingers`, `time`, and gesture-specific deltas                                     | Mutter touchpad recognizer.                                                                              |
| `gnoblin.input.gesture`                       | `gesture`, `phase`, `fingers`, `sequence`, monotonic `time`, `input_time`, and gesture-specific deltas | Stable native Lua event derived from Mutter touchpad input.                                              |
| `mutter.<object>.<signal>`                    | `source`, `signal`, typed `argN` fields, and window identity fields when applicable                    | Forwarded Mutter GObject signal.                                                                         |
| `gnoblin.config.reloaded`                     | `path`; native reload also includes `revision`                                                         | Config reload succeeds.                                                                                  |
| `gnoblin.config.reload-failed`                | `path`, `error`                                                                                        | Candidate load or apply fails; rejected overlapping requests report only through their operation result. |
| `gnoblin.workspace.created`                   | `id`, `number`, `name`, `active`, `windows`, `persistent`                                              | Runtime workspace is created.                                                                            |
| `gnoblin.workspace.renamed`                   | Same workspace fields                                                                                  | Workspace display name changes.                                                                          |
| `gnoblin.workspace.removed`                   | Last workspace record; `number` is its former position                                                 | Temporary workspace is removed.                                                                          |
| `gnoblin.workspace.activated`                 | Same workspace fields                                                                                  | Active workspace changes.                                                                                |
| `gnoblin.window.created`                      | `window`, `name`, `revision`, `sequence`, `time`                                                       | Native runtime observes a new managed window.                                                            |
| Native `gnoblin.window.changed`               | `window_id`, `changed`, `window`, event metadata                                                       | A mapped window property changes, excluding attention state.                                             |
| Native `gnoblin.window.focused` / `unfocused` | `window_id`, `window`, event metadata                                                                  | Keyboard focus enters or leaves a managed window.                                                        |
| Native `gnoblin.window.attention-changed`     | `window_id`, `window`, `demands_attention`, event metadata                                             | Mutter's attention state changes.                                                                        |
| Native `gnoblin.window.closed`                | `window_id`, `last`, event metadata                                                                    | Native runtime removes a managed window.                                                                 |
| `gnoblin.operation.completed`                 | `operation_id`, `method`, `ok`, then `value` or an `Error` record                                      | Native API 1.11 completion event.                                                                        |
| `gnoblin.feature.changed`                     | `feature`, `enabled`                                                                                   | A feature changes after initial setup.                                                                   |
| `gnoblin.scripts.loaded`                      | `scripts`: comma-separated loaded script filenames                                                     | Current user-script loading pass ends.                                                                   |
| `gnoblin.scripts.load_failed`                 | `script`, `error`                                                                                      | Current user script fails to load.                                                                       |

The `gnome.shell.*`, `gnome.interface.*`, `gnoblin.feature.changed`, and
`gnoblin.scripts.*` rows describe the removed hybrid runtime. The standalone
session has no Shell event source, feature registry, or GJS script manager.
The standalone runtime maps changes to
`org.gnome.desktop.interface/color-scheme` to
`gnoblin.appearance.color-scheme-changed`; it does not emit an initial value.
Use the desktop settings service to read the preference when a shell starts.

The `Native` event rows are implemented only by the native Mutter runtime.
Their window tables contain the fields supplied by the native event. Structured
event fields are detached, read-only snapshots; window records expose the same
methods as window query results when their fields identify them as windows.

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

| Event                                     | Additional fields                                                                  | Emitted when                                                                                                                                                                    |
| ----------------------------------------- | ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `gnoblin.window.created`                  | `window: Window`                                                                   | A managed application window appears.                                                                                                                                           |
| `gnoblin.window.closed`                   | `window_id`, `last: Window`                                                        | A managed window is removed.                                                                                                                                                    |
| `gnoblin.window.focused`                  | `window_id`, `window: Window`                                                      | Keyboard focus changes to a window.                                                                                                                                             |
| `gnoblin.window.attention-changed`        | `window_id`, `window: Window`, `demands_attention`                                 | Mutter's attention state changes; this may follow a focus request that policy did not activate.                                                                                 |
| `gnoblin.focus.policy-changed`            | `policy: FocusPolicy`, `revision`, `sequence`, `time`                              | Effective focus policy changes after a successful config commit.                                                                                                                |
| `gnoblin.window.unfocused`                | `window_id`, `window: Window`                                                      | A window loses keyboard focus.                                                                                                                                                  |
| `gnoblin.window.changed`                  | `window_id`, `changed: string[]`, `window: Window`                                 | One or more public properties change.                                                                                                                                           |
| `gnoblin.window.drag.started`             | `drag: WindowDrag`                                                                 | Mutter starts a mouse move grab eligible for pointer snapping.                                                                                                                  |
| `gnoblin.window.drag.updated`             | `drag: WindowDrag`                                                                 | Coalesced move processing updates the observed drag state.                                                                                                                      |
| `gnoblin.window.drag.ended`               | `drag_id`, `window_id`, `reason`, `committed`, optional `target_id`                | A drag is released or invalidated; reasons include `committed`, `released`, `cancelled`, `preempted`, `locked`, `config_reloaded`, `runtime_stopped`, and `compositor_stopped`. |
| `gnoblin.workspace.created`               | `workspace: Workspace`                                                             | A runtime workspace appears.                                                                                                                                                    |
| `gnoblin.workspace.renamed`               | `workspace: Workspace`                                                             | Its display name changes.                                                                                                                                                       |
| `gnoblin.workspace.changed`               | `workspace: Workspace`, `changed: string[]`                                        | Its position, window count, or persistence state changes.                                                                                                                       |
| `gnoblin.workspace.removed`               | `workspace_id`, `last: Workspace`                                                  | A temporary workspace is removed.                                                                                                                                               |
| `gnoblin.workspace.activated`             | `workspace: Workspace`, `previous_id?`                                             | The active workspace changes.                                                                                                                                                   |
| `gnoblin.workspace.window-moved`          | `window_id`, `from_id`, `to_id`                                                    | A window changes workspace.                                                                                                                                                     |
| `gnoblin.monitor.added`                   | `monitor: Monitor`                                                                 | An output becomes available.                                                                                                                                                    |
| `gnoblin.monitor.removed`                 | `monitor_id`, `last: Monitor`                                                      | An output is removed.                                                                                                                                                           |
| `gnoblin.monitor.changed`                 | `monitor: Monitor`, `changed: string[]`                                            | Output properties change.                                                                                                                                                       |
| `gnoblin.input.device-added`              | `device: InputDevice`                                                              | A device appears in the native input-device snapshot.                                                                                                                           |
| `gnoblin.input.device-removed`            | `device_id`, `last: InputDevice`                                                   | A device disappears from the native input-device snapshot.                                                                                                                      |
| `gnoblin.input.sources-changed`           | `sources: InputSource[]`                                                           | The configured available XKB source list changes.                                                                                                                               |
| `gnoblin.input.source-changed`            | `available`, `source?: InputSource`                                                | Mutter confirms a different Gnoblin-owned keymap group, or the current source becomes unknown.                                                                                  |
| `gnoblin.input.gesture`                   | `gesture`, `phase`, `fingers`, gesture-specific deltas                             | A touchpad gesture phase arrives.                                                                                                                                               |
| `gnoblin.shortcut.activated`              | `shortcut`, `trigger`, `focus_context: FocusContext`                               | A registered Gnoblin shortcut activates; its context can authorize one focus, interactive move, or interactive resize operation.                                                |
| `gnoblin.window.menu-requested`           | `window_id`, `menu_type`, `x`, `y`, optional `menu_context: MenuContext`           | Mutter requests a window menu; only `wm` requests carry a one-use target-bound action capability.                                                                               |
| `gnoblin.animation.started`               | `animation`, `target`, `event`                                                     | A configured lifecycle animation or preview begins playback.                                                                                                                    |
| `gnoblin.animation.finished`              | `animation`, `target`, `event`, `cancelled`                                        | A configured lifecycle animation or preview completes or is interrupted.                                                                                                        |
| `gnoblin.capability.changed`              | `capability: Capability`                                                           | A compositor or protocol capability becomes available or unavailable.                                                                                                           |
| `gnoblin.privacy.changed`                 | `state: PrivacyState`, `revision`, `sequence`, `time`                              | A monitored privacy activity changes.                                                                                                                                           |
| `gnoblin.permission.changed`              | `policy: PermissionPolicy`, `revision`                                             | A successful config commit changes the committed permission policy.                                                                                                             |
| `gnoblin.portal.grant-added`              | `grant: PortalGrant`                                                               | A portal grant becomes active.                                                                                                                                                  |
| `gnoblin.portal.grant-removed`            | `grant_id`, `kind`                                                                 | A portal grant ends or is revoked.                                                                                                                                              |
| `gnoblin.launch.changed`                  | `launch: Launch`                                                                   | Launch feedback state changes.                                                                                                                                                  |
| `gnoblin.config.reloaded`                 | `path`, `revision`                                                                 | Configuration loads and applies successfully.                                                                                                                                   |
| `gnoblin.config.reload-failed`            | `path`, `error: string`                                                            | Candidate load or apply fails; rejected overlapping requests report only through their operation result.                                                                        |
| `gnoblin.operation.completed`             | `operation_id`, `method`, `ok`, `value?`, `error?`                                 | A mutating call finishes.                                                                                                                                                       |
| `gnoblin.session.lock-state-changed`      | `state: SessionLockState`, `sequence`, `time`                                      | Mutter reports a session-lock state transition; request delivery is not lock confirmation.                                                                                      |
| `gnoblin.session.activity-changed`        | `available`, `idle`, `threshold_ms`, `idle_for_ms`, `revision`, `sequence`, `time` | Native idle-monitor state changes; `idle_for_ms` is sampled at the transition.                                                                                                  |
| `gnoblin.appearance.color-scheme-changed` | `color_scheme`: `default`, `prefer-dark`, or `prefer-light`                        | Native API 1.34; emitted when the desktop appearance preference changes.                                                                                                        |

The standalone implementation forwards open-ended Mutter GObject signals
through the explicitly unstable `gnoblin.events.mutter.on(...)` namespace. It
does not forward GNOME Shell events or feature/script event streams. The
compositor socket retains its legacy operation event for external clients; Lua
config listeners use `gnoblin.operation.completed`.

General session lifecycle events, including `gnoblin.session.state-changed`,
remain proposed. The compositor socket closes when the compositor stops, so a
terminal lifecycle event cannot be guaranteed. Use `gnoblin.session.status()`
while connected and the dedicated lock-state event for lock transitions.

## Wire contract and versioning

Lua record methods are wrappers around versioned, typed operation names. The
local client interface exposes the same operation names, result records, error
codes, and event payloads. Authentication evidence can differ by caller: the
supervised Lua runtime uses compositor-issued contexts, while an external
shell client supplies an XDG Activation token for user-selected window focus.

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

This inventory describes the standalone Lua API and its compositor operations
in this checkout. It excludes removed GNOME Shell methods such as `feature.*`,
`script.list`, and `shell.*`; those names appear only in the migration history
below. Lua callers do not get the generic `window.action()` dispatcher. The
socket protocol's singular `window.*` and `workspace.*` names are not Lua
namespaces.

| Lua namespace            | Current methods                                                                                                                                                                                                 |
| ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `workspaces`             | Snapshots: `list()`, `active()`, `by_id(id)`. Mutations: `create`, `rename`, `remove`, `activate`, `next`, `previous`, `move_active`, `move_window`.                                                            |
| `windows`                | Snapshots: `list(filter?)`, `focused()`, `by_id(id)`, `snap_context(context)`. `Window` records expose typed close, state, geometry, focus, interactive move/resize, workspace, monitor, and thumbnail methods. |
| `layer` / `monitor`      | `layer.list()`, `monitor.list()`; snapshot aliases `layers.list(filter?)`, `monitors.list()`, `monitors.primary()`, and `layers.animation_policy(namespace)`.                                                   |
| `animations`             | `list()`, `get(name)`, `surfaces()`, `inspect(args)`, `preview(args)`, `seek(args)`, `step(args)`, `play(args)`, `pause(args)`, `stop(args)`. Configuration declarations use `gnoblin.animation(entry)`.        |
| `input`                  | `list()`, `current()`, `sources()`, `current_source()`, `text_target(context)`, `devices()`, `select_source(selector)`. `text_target()` returns a trusted target with `insert_text(text)`.                      |
| `privacy`                | `stop_sharing()`, `stop_recording()`; read-only `state()` snapshot.                                                                                                                                             |
| `permissions` / `grant`  | `permissions.list()`, `permissions.policy()`, `permissions.check(args)`, `grant.list()`, `grant.revoke(args)`, and read-only `portals.grants()`.                                                                |
| `launch` / `launches`    | `launch.status()`, `launch.begin(args)`, `launch.end(args)`; `launches.list()`, `launches.snapshot()`, `launches.begin(args)`, and `launches.end(args)`.                                                        |
| `session`                | `lock()`, `activity()`, `status()`, `logout()`.                                                                                                                                                                 |
| `runtime`                | `reload_config()`                                                                                                                                                                                               |
| `shortcut` / `shortcuts` | Registered operations: `shortcut.capture(args)`, `shortcut.bind(args)`, `shortcut.unbind(args)`. Public API: `shortcuts.actions(group?)`, `list()`, `capture(options?)`, `bind(args)`, and `unbind(args)`.      |
| `capabilities` / `focus` | `capabilities.list()`, `focus.history(filter?)`, and the read-only `focus.policy` property.                                                                                                                     |

`gnoblin.version()` reports Gnoblin, GNOME, Mutter, Lua, API, Git remote, Git
SHA, and build ID. `gnoblin.settings` and `gnoblin.focus.policy` are immutable
snapshots. `gnoblin.privacy.state()`, `gnoblin.session.activity()`,
`gnoblin.session.status()`, and `gnoblin.portals.grants()` read compositor or
supervisor state directly. `gnoblin.windows`, `workspaces`, `monitors`,
`layers`, and `animations` expose snapshot helpers in addition to the typed
operation methods listed above.

`window.action` remains a native socket protocol operation for existing socket
clients, but it is not installed as a Lua method. Pointer snapping is provided
by the compositor's `WindowDrag:offer_targets()` hook and the native
`window.snap.offer` operation; Lua keyboard snapping uses
`gnoblin.windows.snap_context(context)`.

Lua event registrations use `gnoblin.events.on` and `gnoblin.events.once`; the
legacy `gnoblin.on` alias returns the same unsubscribe-able subscription.

Typed window methods take a stable `id` and return an operation whose completed
value is `{id}`. They cover close, minimize and restore, boolean state setters,
move, resize, workspace move, monitor move, and interactive move and resize.
Lua `window.focus`, `window.begin_move`, and `window.begin_resize` require a
live, one-use trusted shortcut context; calls without one fail closed. Socket
`window.focus` also accepts an XDG Activation token at API 1.32. Mutter verifies
the token's input serial and source surface, and Gnoblin requires the socket
peer PID to match the PID that created the token. The token is consumed once
and is never exposed to Lua. See the
[runtime API reference](../docs/config/runtime-api.md) for arguments and
restrictions.

### Current window results and actions

The compositor socket `window.list` result has `windows`, an array of records
with these fields. Lua snapshots and socket API 1.35 use the same snake_case
properties. Socket API 1.x also retains camelCase aliases for compatibility;
new clients should use the canonical snake_case fields.

| Current property                                                 | Type                    | Meaning                                                          |
| ---------------------------------------------------------------- | ----------------------- | ---------------------------------------------------------------- |
| `id`                                                             | string                  | Stable window sequence ID.                                       |
| `title`                                                          | string                  | Current title, or an empty string.                               |
| `app_id`                                                         | string                  | Desktop app ID, falling back to WM class.                        |
| `gtk_app_id`                                                     | string                  | GTK app ID, or an empty string.                                  |
| `wm_class`                                                       | string                  | WM class, or an empty string.                                    |
| `rule_app_id`                                                    | string                  | GTK app ID, falling back to WM class.                            |
| `focused`                                                        | boolean                 | Keyboard focus state.                                            |
| `minimized`                                                      | boolean                 | Minimized state.                                                 |
| `workspace`                                                      | integer or `nil`        | Current one-based workspace position.                            |
| `workspace_id`                                                   | string or `nil`         | Stable workspace ID.                                             |
| `workspace_number`                                               | integer or `nil`        | Current one-based workspace position.                            |
| `monitor_index`                                                  | integer                 | Current monitor index.                                           |
| `monitor_id`                                                     | string or `nil`         | Canonical active connector name for the current logical monitor. |
| `above`, `sticky`, `demands_attention`                           | boolean                 | Stacking, workspace visibility, and attention state.             |
| `closable`, `minimizable`, `maximizable`, `movable`, `resizable` | boolean                 | Current compositor capabilities.                                 |
| `role`                                                           | string or `nil`         | Window role, when supplied.                                      |
| `type`                                                           | integer                 | Mutter `MetaWindowType`; see the mapping in Window properties.   |
| `maximized`                                                      | boolean                 | Maximized in both directions.                                    |
| `fullscreen`                                                     | boolean                 | Fullscreen state.                                                |
| `frame`                                                          | `{x, y, width, height}` | Current frame rectangle in logical pixels.                       |
| `last_user_time`                                                 | integer                 | Last user interaction timestamp known to Mutter.                 |
| `parent`                                                         | string or `nil`         | Stable ID of the transient parent.                               |
| `monitor`                                                        | `{x, y}` or `nil`       | Monitor origin in logical coordinates.                           |

The socket compatibility aliases are `appId`, `gtkAppId`, `wmClass`,
`ruleAppId`, `workspaceId`, `workspaceNumber`, `monitorIndex`, `monitorId`,
`demandsAttention`, and `lastUserTime`; `geometry` aliases `frame`. Lua records
omit those aliases. The former Shell-backed result included `id`, title and app
identity, focus and minimized state, workspace fields, monitor index, maximize
and fullscreen state, frame geometry, last user time, and optional `parent` and
`monitor`. The standalone result also provides monitor identity, stacking and
attention state, capability flags, optional `role`, and `type`.

`window.match({window?})` defaults to `"active"` and returns `id`,
`identity` with `desktop_app_id`, `gtk_app_id`, `wm_class`, and
`rule_app_id`, plus a `match` table with `type`, `title`, `focused`,
and optional `app_id`.

The raw compositor socket retains `window.action({action, window?})` for
compatibility; the standalone Lua runtime does not expose
`gnoblin.window.action`. Native socket clients can use `above`, `unabove`,
`stick`, `unstick`, `close`, `minimize`, `restore`, `maximize`, `unmaximize`,
`fullscreen`, and `unfullscreen`. The target is a stable window ID or
`"active"`; it defaults to `"active"`. Focus, menu, interactive move or
resize, geometry, workspace, monitor, and restore-or-minimize requests are not
accepted by this compatibility method. Lua callers use the typed `Window`
methods above. Focus and interactive actions require a trusted one-use
context.

The typed `window.move_to_monitor` method accepts a stable connector ID as a
string or `{id = string}`; cloned outputs use the lexicographically first
active connector. A disconnected or no-longer-canonical ID fails with
`not_found`. Typed workspace selection accepts `{id = string}` or
`{number = integer}`. The typed API uses stable IDs, explicit selectors, and
idempotent property setters.

Current Lua runtime callback calls return an `Operation` handle. Native API
1.11 completion updates its `status`, `value`, and structured `Error` fields,
invokes callbacks registered while the operation was pending, and dispatches
`gnoblin.operation.completed` with `operation_id`, `method`, `ok`, and either
`value` or `error`. If `on_complete` is registered after completion, Gnoblin
queues it for the next main-loop turn. The host dispatches that callback through
the same config validation and operation drain path used for runtime events;
operations requested by the callback are applied after it returns. Queued
callbacks are discarded when their Lua runtime is replaced. The native API call
path still uses a positive request ID internally and returns a normal socket
`reply` or `error`.
See the [runtime API reference](../docs/config/runtime-api.md) and [Lua
events](../docs/config/lua-events.md) for current behavior.

## Implementation status at this checkout

The session uses the standalone Mutter runtime. It does not start GNOME Shell,
load GJS, or include a Shell compatibility adapter. Historical Shell-backed
rows elsewhere in this file describe migration input, not a supported runtime.

Native-control API 1.17 implements `gnoblin.privacy.state()` as an immutable
snapshot with `available`, a stable `revision`, and activity fields only for
sources marked available. The current session reports screen-sharing and
recording from Mutter's tracked remote-access handles, and microphone activity
from running PipeWire audio-capture streams when Mutter has remote-desktop
support and can connect to PipeWire. Meter streams may count as active, because
the monitor does not trust an application's self-reported ID to suppress
microphone activity. Camera and location remain unavailable.
The `gnoblin.privacy.changed` event carries the updated state plus event
sequence and monotonic-time metadata. API 1.31 implements
`gnoblin.privacy.stop_sharing()`
and `gnoblin.privacy.stop_recording()` as operations that call `stop()` on
matching tracked Mutter handles. Their result's `requested` count is the number
of stop calls issued, not confirmation that sessions closed.

Native-control API 1.31 also exposes `gnoblin.layers.animation_policy()` as an
immutable read of effective layer enter/exit animation and window-shadow
policy. Without an override it returns `slide` for both phases and `false` for
`window_shadow`. Its namespace argument is limited to 1–128 UTF-8 bytes.

Native-control API 1.32 adds `gnoblin.session.logout()` and the socket
`window.focus` XDG Activation path. Logout returns `{accepted = true}` before
the supervisor exits. The socket focus path accepts a one-use token created by
the same process that owns the socket connection; Mutter verifies its source
surface and input serial, and locked sessions reject the focus request.

Native-control API 1.20 implements `runtime.reload_config`. It stages and
validates the selected Lua config, waits asynchronously for active-runtime
operations to complete, and applies supported live changes as a correlated
Mutter transaction. The API operation completes only after Mutter confirms
application. Settings that require a new session remain unchanged by reload.
Native-control API 1.21
implements `gnoblin.session.lock()` as a request to subscribed external shell
clients and reports Mutter lock-state transitions through
`gnoblin.session.lock-state-changed`; a request result does not confirm the
session is locked.

The current source also implements the immutable window, workspace, monitor,
layer, input, capability, launch, and permission snapshots described in the
current inventory above. Native API versions identify socket additions; Lua
configuration methods are not automatically available to socket clients. All
other signatures remain proposals unless marked current in the relevant table.
