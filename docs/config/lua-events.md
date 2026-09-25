# Lua events

Register Lua callbacks with `gnoblin.on(name, callback)`. Event names identify
their source: `gnome.shell.*` and `gnome.interface.*` come from GNOME,
`mutter.*` comes from Mutter, and `gnoblin.*` comes from Gnoblin. The Lua
runtime stays alive for the session. Reloading the config replaces it and
registers its callbacks again.

For the current GNOME light or dark appearance and a live border example, see
[Light and dark appearance](/guides/theming).

```lua
gnoblin.on("mutter.wayland.pointer-window-changed", function(event)
    local speed = event.app_id == "org.chromium.Chromium" and 0.3 or 1.0
    gnoblin.configure {input = {touchpad = {scroll_speed = speed}}}
end)
```

The Mutter pointer-window event changes when the pointer enters a different
Wayland surface, before later scroll events reach that surface. It does not
depend on keyboard focus or clicking the window. The event also fires when the
pointer leaves a client surface; then the window fields are empty strings.

## Event sources

### GNOME Shell

GNOME Shell events use the `gnome.shell.*` prefix.

| Event name                                                     | Fields                          | Dispatched when                                                               |
| -------------------------------------------------------------- | ------------------------------- | ----------------------------------------------------------------------------- |
| `gnome.shell.focus.changed`                                    | `app_id`, `wm_class`, `title`   | The keyboard-focused window changes. This can differ from the pointer window. |
| `gnome.shell.window.created`                                   | `app_id`, `wm_class`, `title`   | A window is created.                                                          |
| `gnome.shell.window.unmanaged`                                 | `app_id`, `wm_class`, `title`   | A window is removed.                                                          |
| `gnome.shell.input.<type>`                                     | Fields depend on the input type | A Clutter input event reaches the shell's captured-event handler.             |
| `gnome.shell.overview.showing`, `.shown`, `.hiding`, `.hidden` | No additional fields            | The overview changes visibility state.                                        |

Common input event types are:

- `motion`, with pointer coordinates `x` and `y`.
- `button_press` and `button_release`, with `button` and pointer coordinates.
- `scroll`, with pointer coordinates, `scroll_x`, `scroll_y`, and
  `scroll_direction`.
- `key_press` and `key_release`, with `key_symbol`.

Each input event also includes `type` and `time`. Events consumed before
reaching the captured-event handler are not included.

For compatibility, the earlier unqualified names remain available:
`pointer_window_changed`, `focus_changed`, `window_created`,
`window_unmanaged`, and `input.<type>`.
`pointer_window_changed` is emitted by Mutter's Wayland pointer tracking;
the other listed compatibility events come from the shell integration.

Gnoblin also reports the desktop's light or dark preference:

```lua
gnoblin.on("gnome.interface.color-scheme-changed", function(event)
    print(event.color_scheme) -- "default", "prefer-dark", or "prefer-light"
end)
```

The values are `default`, `prefer-dark`, and `prefer-light`. Gnoblin sends the
current value after config load and whenever it changes. GTK apps consume the
desktop preference automatically; some non-GTK apps opt into it. See the
[theming guide](/guides/theming) for GTK behavior and a live border example.

### Mutter

Mutter GObject signals use the object's source prefix followed by the signal
name. Gnoblin watches the current display, windows and workspaces, workspace
manager, backend, monitor manager, and cursor tracker. Examples include:

```lua
gnoblin.on("mutter.display.restacked", function(event)
    print(event.name)
end)

gnoblin.on("mutter.window.position-changed", function(event)
    print(event.window_title)
end)
```

The source prefixes are `mutter.display`, `mutter.window`,
`mutter.workspace-manager`, `mutter.workspace`, `mutter.backend`,
`mutter.monitor-manager`, and `mutter.cursor-tracker`. Gnoblin forwards the
signals exposed by the running Mutter build. Newly created windows and
workspaces are watched as they appear.

Gnoblin also exposes the Wayland pointer-window transition as
`mutter.wayland.pointer-window-changed`, with `app_id`, `wm_class`, and
`title`. The older `pointer_window_changed` name remains available.

Every signal event includes `source` and `signal`.

- Scalar signal arguments appear as `arg0`, `arg1`, and so on. Their GObject
  types appear in `arg0_type`, `arg1_type`, and so on.
- Object arguments appear as type names. Window arguments also include
  `argN_app_id`, `argN_wm_class`, and `argN_title`.
- `mutter.window.*` events include `window_app_id`, `window_wm_class`, and
  `window_title`.

Values that cannot be represented as simple Lua event fields are omitted or
reduced to a type or name string.

### Gnoblin

Gnoblin events use the `gnoblin.*` prefix.

| Event name                        | Fields                                            | Dispatched when                                                                                      |
| --------------------------------- | ------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `gnoblin.config.reloaded`         | `path`                                            | Config and its Lua modules load and apply successfully.                                              |
| `gnoblin.config.reload-failed`    | `path`, `error`                                   | A config reload fails while a previously loaded event subscription is still active.                  |
| `gnoblin.workspace.created`       | Workspace record                                  | A runtime workspace is created.                                                                      |
| `gnoblin.workspace.renamed`       | Workspace record                                  | A workspace display name changes.                                                                    |
| `gnoblin.workspace.removed`       | Workspace record                                  | A temporary workspace is removed.                                                                    |
| `gnoblin.workspace.activated`     | Workspace record                                  | The active workspace changes.                                                                        |
| `gnoblin.api.operation-completed` | `request_id`, `method`, `ok`, `result` or `error` | A queued Lua runtime API operation completes.                                                        |
| `gnoblin.feature.changed`         | `feature`, `enabled`                              | A Gnoblin feature changes after initial state setup.                                                 |
| `gnoblin.scripts.loaded`          | `scripts`                                         | The user script load pass completes; `scripts` is a comma-separated list of loaded script filenames. |
| `gnoblin.scripts.load_failed`     | `script`, `error`                                 | A user script cannot be imported or throws while loading.                                            |

```lua
gnoblin.on("gnoblin.feature.changed", function(event)
    print(event.feature .. " enabled: " .. tostring(event.enabled))
end)
```

Workspace events include the record's `id`, current `number`, `name`, and
`active` status. Records also include the `windows` count and `persistent` flag.

`removed` reports the workspace's last record before removal. Its `number` is
the position immediately before removal.

When `workspace.create` uses `activate = true`, Gnoblin emits `created` first
with `active = false`, then emits `activated` after switching workspaces.

Lua runtime API calls return a request ID. Match that ID in
`gnoblin.api.operation-completed`; successful events include `result`, while
failed events include `error`. Runtime actions requested from an event callback
run after the callback returns. See the [Lua runtime API](/config/runtime-api)
for method arguments and results.

## Listen to every forwarded event

Pass `"*"` to receive every event source currently forwarded by Gnoblin:

```lua
gnoblin.on("*", function(event)
    print(event.name)
end)
```

Every callback receives one event table. `event.name` contains the dispatched
name. Registering a name does not create an event source: Mutter, GNOME Shell,
or Gnoblin must dispatch it. Signal availability follows the Mutter and GNOME
Shell versions used to build Gnoblin.

`gnoblin.listeners` maps each registered event name to its callback list.
Inspect it to see which handlers earlier config files have added:

```lua
for name, callbacks in pairs(gnoblin.listeners) do
    print(name, #callbacks)
end
```

Register callbacks with `gnoblin.on`; the listener table is for inspection.

Callbacks run synchronously in the compositor's main thread. Keep them short,
especially handlers for high-frequency `*.input.*` events.

`gnoblin.configure` changes follow the usual live-setting rules. Startup-only
settings still need a new session. If a callback fails, Gnoblin logs the error
and keeps the settings from before that event. For settings applied by the
shell, Gnoblin applies the updated document after the callback returns.

## Type definition

Event names are open-ended: Gnoblin accepts a nonempty UTF-8 string of up to
128 bytes with no NUL byte, but only names dispatched by Gnoblin, Mutter or
the shell produce callbacks. The event fields depend on the event name; all
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
