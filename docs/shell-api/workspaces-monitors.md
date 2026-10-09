# Workspaces and monitors

For window-level workspace moves, see [Windows](/shell-api/windows). Event
subscriptions are listed in [Lua events](/config/lua-events).

## Workspaces

Select a workspace with exactly one selector field:

| Field    | Type   | Meaning                                                      |
| -------- | ------ | ------------------------------------------------------------ |
| `id`     | String | Select a stable workspace ID, such as `"code"`.              |
| `number` | 1–1024 | Select its current one-based position in the workspace list. |

IDs declared in `gnoblin.configure.workspaces` persist across sessions. IDs
supplied to runtime `create` are temporary and last only for the session.
Gnoblin-generated `@session-N` IDs can be selected during that session but
should not be saved in configuration.
See the [workspace configuration reference](/config/configure/window_management)
and the [writing workspace recipe](/recipes/writing-workspace).

## Read workspaces

These methods return immediate, read-only snapshots:

| Method                       | Arguments          | Result                         |
| ---------------------------- | ------------------ | ------------------------------ |
| `gnoblin.workspaces.list()`  | None               | All current `Workspace` records |
| `gnoblin.workspaces.active()` | None              | Active record or `nil`         |
| `gnoblin.workspaces.by_id(id)` | String workspace ID | Matching record or `nil`       |

## Create and change workspaces

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

Mutations return operation handles and complete asynchronously. Workspace
reads and mutations use the `workspaces` namespace in Lua. For socket operation
names and fields, see the [compositor bridge](/compositor-bridge).

`create` requires a nonempty name of up to 80 characters. Its optional ID must
match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`; if omitted, Gnoblin generates a
session-only ID. `activate` defaults to `false`. Rename also requires a
nonempty name of up to 80 characters. `follow` defaults to `false`; when true,
moving a window also switches to the destination workspace.

A runtime rename changes the live workspace only. It does not edit your Lua
configuration or change workspace names saved for GNOME. A config reload restores
the declared name; update `gnoblin.configure {workspaces = {...}}` to keep a name
across reloads and sessions.

## Workspace records

A `Workspace` record has `id`, `number`, `name`, `active`, `windows`, and
`persistent` fields. Gnoblin does not remove a declared workspace, the active
workspace, or a workspace that still contains windows.


Workspace snapshots also provide these methods. Call them with colon syntax
from a runtime event callback; each returns an `Operation` handle.

| Method                                  | Arguments                                                                   | Effect                              |
| --------------------------------------- | --------------------------------------------------------------------------- | ----------------------------------- |
| `workspace:activate()`                 | None                                                                        | Activate this workspace.            |
| `workspace:rename(name)`               | Nonempty name up to 80 characters                                           | Rename this workspace.              |
| `workspace:remove()`                   | None                                                                        | Remove this workspace when allowed. |
| `workspace:move_here(window, options?)` | Window snapshot, stable window ID, or `"active"`; optional `follow` boolean | Move the window to this workspace.  |

## Monitors

`gnoblin.monitors.list()` and `gnoblin.monitors.primary()` return cached,
read-only snapshots of active logical monitors. The primary method returns a
record or `nil`. Snapshots refresh before monitor lifecycle callbacks run. Use
the stable connector `id` when moving a window. For cloned outputs, Gnoblin
uses the first active connector alphabetically as the canonical ID.

Monitor records expose these fields:

| Field                       | Meaning                                                                                                            |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `id`                        | Canonical active connector name. Cloned outputs use the first connector alphabetically.                           |
| `index`                     | Current zero-based Mutter order. It can change when outputs change.                                                |
| `x`, `y`, `width`, `height` | Logical-pixel monitor geometry.                                                                                    |
| `primary`                   | Whether this is the primary logical monitor.                                                                       |
| `scale`                     | Current monitor scale factor.                                                                                      |
| `enabled`                   | Always `true`; this list contains active logical monitors only.                                                    |
| `transform`                 | Rotation or reflection applied by Mutter.                                                                          |
| `name`                      | Optional display name supplied by Mutter.                                                                          |
| `make`, `model`, `serial`   | Optional physical display details supplied by Mutter.                                                              |
| `refresh_rate`              | Optional current refresh rate in Hz for the connector used as `id`.                                                |
| `work_area`                 | The usable area of this monitor: `x`, `y`, `width` and `height` in logical pixels, after panels reserve space with exclusive zones. It follows the active workspace.|

The `transform` values describe the output orientation:

- `normal`: no rotation or reflection.
- `90`, `180`, and `270`: rotate clockwise by that amount.
- `flipped`: mirror the output.
- `flipped-90`, `flipped-180`, and `flipped-270`: mirror the output, then apply
  the named clockwise rotation.

Use the connector `id` for typed monitor moves. When outputs are cloned, the
refresh rate comes from the lexicographically first active connector used as
the monitor ID. The native result does not include inactive physical outputs.


## Monitor privacy screens

`gnoblin.monitors.privacy_screen()` returns the cached privacy-screen
snapshot. It contains:

- `requested_enabled`: the effective requested state;
- `source`: which setting currently supplies that value;
- `revision`: the snapshot revision;
- `monitors`: active outputs, each with an `id` and `available` flag. Supported
  outputs also include `enabled` and `locked`; unsupported outputs omit them.

`gnoblin.monitors.set_privacy_screen(value)` returns an
`Operation<MonitorPrivacyScreenSnapshot>`. It accepts:

- `true`: enable privacy screens for this session;
- `false`: disable them for this session;
- `"inherit"`: clear the runtime override and use the configured value, or the
  system preference when config omits it.

The snapshot's `source` identifies the active preference:

- `runtime`: a runtime override supplies the value;
- `config`: `monitors.privacy_screen` is set in the Lua config;
- `system`: neither runtime nor config supplies a value.

Runtime overrides do not write the system preference. Use `on_complete` to
read the updated snapshot in the native runtime. `gnoblinctl lua` waits for
completion and returns the resulting read-only snapshot.

```lua
gnoblin.monitors.set_privacy_screen(true):on_complete(function(snapshot, err)
    if err then
        print(err)
        return
    end
    print(snapshot.source, snapshot.requested_enabled)
end)
```

## Monitor events

Subscribe to `gnoblin.monitor.privacy-screen-changed` to observe changes to the
requested value, its source, or per-monitor state. The event carries the same
snapshot fields plus `sequence` and `time`.

Subscribe to `gnoblin.monitor.added`, `gnoblin.monitor.changed`, and
`gnoblin.monitor.removed` to track output changes. The `changed` event's
`changed` array lists changed record properties: `id`, `index`, `x`, `y`,
`width`, `height`, `primary`, `scale`, `enabled`, `name`, `make`, `model`,
`serial`, `refresh_rate`, `transform`, or `work_area`. These immediate reads and events are
available only in the native Mutter runtime.
