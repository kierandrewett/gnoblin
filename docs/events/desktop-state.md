# Window, Workspace, and Monitor Events

Track window, workspace, and monitor records as compositor state changes.

Lifecycle events describe active state in the current compositor session.

### Monitor and workspace records

Monitor records describe active logical monitors. The `id` is the canonical
connector name. A monitor event's `changed` field lists updated properties:

- Position: `index`, `x`, `y`, `width`, `height`, `work_area`.
- Display: `primary`, `scale`, `enabled`, `refresh_rate`, `transform`.
- Identity: `id`, `name`, `make`, `model`, `serial`.

Create, rename, and activation events include a `workspace` record. Workspace
change events include a `changed` array listing `number`, `window_count`, or
`persistent`. The `window_count` value matches immediate workspace reads.

A removal carries `workspace_id` and `last`, the final record before removal.
The moved event reports the stable window ID and source and destination
workspace IDs. It has no workspace record.

The attention event also carries `demands_attention` at the top level. It can
follow a focus request that policy did not activate; it reports Mutter's
attention state and does not identify why the window requested attention.

Strict focus policy also emits `gnoblin.window.activation-denied` when Mutter
rejects an application's activation request. The event's `reason` is one of:

- `missing_context`: the request has no activation context.
- `invalid_context`: its token is not valid.
- `stale_context`: its startup context is too old.

The event contains no activation token or input serial. Socket subscription is
documented in the [compositor bridge](/bridge/).

### Window records

Window records contain the fields available from the compositor:

- Identity: `id`, `title`, `app_id`, `gtk_app_id`, `wm_class`, `rule_app_id`, and optional `role`.
- Location: `workspace_id`, `workspace_number`, `monitor_id`, `monitor_index`, `parent`.
- State: `maximized`, `fullscreen`, `minimized`, `focused`, `above`, `sticky`, `modal`, and `demands_attention`.
- Capabilities: `closable`, `minimizable`, `maximizable`, `movable`, and `resizable`.
- Type: `type`, an integer `MetaWindowType` value.
- Geometry: `frame` and the optional `monitor` origin.
- Interaction: `last_user_time`.
- Record metadata: `revision`, the state revision for this record.

A field is omitted when the compositor does not provide it. `monitor_id` is
the active connector name for the current logical monitor. Cloned outputs use
the lexicographically first active connector. `monitor_index` is the current
Mutter order and can change when outputs change.

The `type` value follows Mutter's `MetaWindowType` enum. It describes the
client surface rather than shell presentation.

`modal` is true for type value `4`; `changed` includes `modal` when it changes.

| Value | Type           | Meaning                               |
| ----- | -------------- | ------------------------------------- |
| 0     | normal         | Standard application window.          |
| 1     | desktop        | Desktop background surface.           |
| 2     | dock           | Dock or panel surface.                |
| 3     | dialog         | Dialog window.                        |
| 4     | modal dialog   | Dialog that blocks its parent window. |
| 5     | toolbar        | Toolbar surface.                      |
| 6     | menu           | Menu surface.                         |
| 7     | utility        | Utility window.                       |
| 8     | splash screen  | Application startup surface.          |
| 9     | dropdown menu  | Dropdown menu surface.                |
| 10    | popup menu     | Popup menu surface.                   |
| 11    | tooltip        | Tooltip surface.                      |
| 12    | notification   | Notification surface.                 |
| 13    | combo box      | Combo-box popup surface.              |
| 14    | drag-and-drop  | Drag-and-drop surface.                |
| 15    | override other | Other override-redirect surface.      |

The `changed` array lists mapped properties that changed. Focus transitions and
attention changes use separate events; `demands_attention` is omitted from the
`changed` array.

When focus moves between windows that remain managed, the native runtime
dispatches `unfocused` before `focused` within that state revision.

A closed event's `last` field contains the final available window record.

Layer lifecycle events follow the native layer snapshot. Each carries
the shared state `revision`, `sequence`, and monotonic `time`. The stable ID is
in `layer.id` on `created`; `changed` and `removed` also carry `layer_id`.

- `created` includes the record when its layer role first appears in the
  snapshot. It can initially be unmapped.
- `changed` includes the updated `layer` and a `changed` array. The array names
  fields that changed, except for per-record `revision`. Mapped transitions are
  changes.
- `removed` includes the final record as `last` and is sent when the layer
  disappears from the snapshot.

`removed` reports the workspace's last record before removal. Its `number` is
the position immediately before removal.

When `workspace.create` uses `activate = true`, Gnoblin emits `created` first
with `active = false`, then emits `activated` after switching workspaces.

A Lua API call returns an `Operation` handle with its current status, result,
failure reason, and a completion callback. See the [Lua runtime API](/shell-api/)
for the handle fields and callback behavior.

## Type definition

Event names are open-ended: Gnoblin accepts a nonempty UTF-8 string of up to
128 bytes with no NUL byte, but only names dispatched by Gnoblin or Mutter
produce callbacks. The event fields depend on the event name; all
callback values are Lua tables.

```lua
gnoblin.on(event_name, function(event)
    -- event_name: string | "*"
    -- event = {
    --     name = string,
    --     source = string?, signal = string?,
    --     app_id = string?, wm_class = string?, title = string?,
    --     type = string?, time = number?, x = number?, y = number?,
    --     button = integer?, key_symbol = string?,
    --     scroll_x = number?, scroll_y = number?, scroll_direction = string?,
    --     ... -- additional fields depend on the event source
    -- }
end)
```
