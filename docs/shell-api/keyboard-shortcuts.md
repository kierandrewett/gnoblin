# Keyboard shortcuts

`gnoblin.shortcuts.list()` returns native command and action shortcuts. It
omits shortcuts owned by shell clients. A configured entry with
`capture_input = true` starts a shortcut session and sends events to Lua
listeners. It has no `ShortcutState` record. Socket clients receive session
events for dynamic bindings they register and subscribe to. Each read-only
record contains:

- `name`, `binding`, `enabled`, `trigger`, and `revision`.
- Either `command` (an argument array) or `action` (a `group.key` identifier).
- `binding` as a string for one accelerator or an array for multiple bindings.

`enabled` is false for a configured built-in action with no bindings. Disabled
shortcut declarations are omitted.

```lua
for _, shortcut in ipairs(gnoblin.shortcuts.list()) do
    print(shortcut.name, shortcut.binding)
end
```

Use `gnoblin.shortcuts.list()` from Lua; see the [compositor bridge](/compositor-bridge) for socket request details.


`gnoblin.shortcuts.bind` accepts
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

`gnoblin.shortcuts.end_session` accepts the binding `id` and the
`session_id` from that binding's `gnoblin.shortcut.session.activated` event.
Ending a session releases its keyboard capture while keeping the binding
registered. A stale session ID or a binding owned by another connection is
rejected. The matching ended event uses reason `cancelled`.

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

Dynamic registrations belong to the client that created them. Active sessions
end on modifier
release, unbind, config reload, session lock, capture preemption, owner
disconnect, or after ten seconds.

Lua listeners can subscribe to these shortcut session events:

- `gnoblin.shortcut.session.activated` when a held binding starts.
- `gnoblin.shortcut.session.key` for captured keyboard input. Fields include
  `keyval`, `keycode`, `modifiers`, `phase` (`press` or `release`), and `time`.
  Real key presses and releases can carry a one-use `focus_context`; repeated
  events do not carry one.
- `gnoblin.shortcut.session.ended` when a session ends. Reasons are `released`,
  `unbound`, `owner_disconnected`, `config_changed`, `locked`, `preempted`,
  `timed_out`, `cancelled`, `compositor_stopped`, and `runtime_stopped`. The
  `cancelled` reason is used by `shortcut.session.end`. `runtime_stopped` is
  sent to a socket client when the Lua runtime stops while that client owns the
  active session.

Lua listeners receive the same authority as an opaque `event.focus_context`
userdata on real, non-repeated `session.key` events. Synthetic and input-method
events are excluded. The context expires after five seconds and authorizes one
focus-sensitive compositor operation. Gnoblin still delivers the key event if
it cannot issue a context.

`gnoblin.shortcut.binding-activated` can carry a one-use `focus_context` on
trusted activation. `gnoblin.shortcut.binding-deactivated` is emitted for
press-triggered bindings. The event contains:

- `id` and `accelerator` to identify the binding.
- `input_time`, Mutter's timestamp for the key release.

Release-triggered bindings activate on release and do not emit a second
deactivation event.

Modal sessions capture keyboard events only. Pointer input remains available to
the shell's layer-shell surfaces and client windows.

`gnoblin.shortcuts.actions(group?)` reads Gnoblin's packaged keybinding
catalogue. The build generates it from executable handlers in the pinned
Mutter source and the matching pinned keybinding schema sources. Gnoblin ships
the descriptions and default bindings with the runtime, so listing actions and
validating configured actions do not need installed GSettings schemas.
Omit `group` to list actions from all three groups.

| Group     | Contains                                                           |
| --------- | ------------------------------------------------------------------ |
| `wm`      | Window-manager actions, such as closing or maximizing a window.    |
| `mutter`  | Compositor actions defined by Mutter.                              |
| `wayland` | Actions defined by Mutter specifically for its Wayland compositor. |

An unknown group raises an error. The catalogue includes only actions with an
executable handler in Gnoblin's pinned Mutter build. The `gnome:shell` group is
not included. This read API is separate from `gnoblin.configure.shortcuts`,
which declares shortcut configuration.

To assign a built-in action, set `action = action.id` on a named
`gnoblin.configure.shortcuts` entry. See the
[shortcut configuration reference](/config/configure/shortcuts) for the
declarative form. `gnoblin.shortcuts.bind()` registers a Gnoblin shortcut event;
it does not invoke a built-in action.

Each action record contains:

- `id`, `group`, and `key`.
- Optional `description`, when the pinned schema provides one.
- `default_bindings`, the pinned schema's exact accelerator string array.

These are packaged schema defaults, not the current user override. An empty
array means that the action has no default accelerator. Actions are returned
in `wm`, `mutter`, `wayland` order, with keys sorted alphabetically within each
group.

```lua
for _, action in ipairs(gnoblin.shortcuts.actions("wm")) do
    if action.id == "wm.close" then
        for _, binding in ipairs(action.default_bindings) do
            print(binding)
        end
    end
end
```

## Capture a shortcut

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

The socket capability and request contract are documented in the
[compositor bridge](/compositor-bridge).

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
