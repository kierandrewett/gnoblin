# Keyboard shortcuts

> **Status:** not available yet: `:end_session`, `:unbind`, `gnoblin.configure.shortcuts`, `gnoblin.shortcut.binding`, `gnoblin.shortcut.session.activated`, `gnoblin.shortcut.session.ended`, `gnoblin.shortcut.session.key`, `gnoblin.shortcuts.actions`, `gnoblin.shortcuts.bind`, `gnoblin.shortcuts.capture`, `gnoblin.shortcuts.configured`, `gnoblin.shortcuts.list`. The sections that use them describe the target, not current behaviour.

`gnoblin.shortcuts.configured()` returns the command and action shortcuts that
the configuration declares. It omits shortcuts owned by shell clients. A
configured entry with `capture_input = true` starts a shortcut session and sends
events to Lua listeners. It has no `ShortcutState` record. Socket clients
receive session events for dynamic bindings they register and subscribe to.
Each read-only record contains:

- `name`, `binding`, `enabled`, `trigger`, and `revision`.
- Either `command` (an argument array) or `action` (a `group.key` identifier).
- `binding` as a string for one accelerator or an array for multiple bindings.

`enabled` is false for a configured built-in action with no bindings. Disabled
shortcut declarations are omitted. These records are not objects: change them
with `gnoblin.configure.shortcuts`. The socket request `shortcuts.list` returns
the same records.

```lua
for _, shortcut in ipairs(gnoblin.shortcuts.configured()) do
    print(shortcut.name, shortcut.binding)
end
```

See the [compositor bridge](/bridge/) for socket request details.

`gnoblin.shortcuts.bind(id, accelerator, options?)` registers a binding and
returns a `Shortcut`.

`accelerator` is a chord in the [chord syntax](/guides/shortcuts#chord-syntax),
such as `<Super><Alt>F9`. A chord in another form, such as `Super+Alt+F9`, is
an error that names the form to write. The `Shortcut` reports the chord in its
written form.

The optional `options` table takes these fields:

- `hold` accepts `none`, `super`, `control`, or `alt`. It defaults to `none`.
- `trigger` accepts `press` or `release`. It defaults to `press`.
- `mode` accepts `passive` or `modal`. It defaults to `passive`. Modal mode
  requires a held modifier and captures keyboard events while it is held.
- `capture_input` defaults to `false`. Set it to `true` for a bare `Super`
  binding; this flag is rejected for other accelerators. A bare `Super`
  binding also requires `trigger: "release"`, `hold: "none"`, and an available
  compositor early modifier hook.

Every binding is a `Shortcut` object. It has these fields:

- `id`, `accelerator`, `hold`, `trigger`, `mode` and `capture_input`.
- `alive`, which is false once the binding has ended.

`bind` returns the object at once with the values you asked for. If the
compositor refuses the binding, for example because another client holds the
accelerator, the shortcut ends and its `closed` listeners receive the reason as
a second argument. An ID that this runtime already binds is an error: unbind it
first.

`gnoblin.shortcuts.list()` returns the shortcuts that this runtime has bound
and not unbound, in ID order. They are the same objects that `bind` returned.

A `Shortcut` raises these signals. Use `shortcut:on(signal, fn)`; `fn` receives
the shortcut and the event:

| Signal              | When                                                      | Event                                  |
| ------------------- | --------------------------------------------------------- | -------------------------------------- |
| `activated`         | The accelerator fires (on release for a release trigger). | `gnoblin.shortcut.binding-activated`   |
| `deactivated`       | A press-triggered binding is released.                    | `gnoblin.shortcut.binding-deactivated` |
| `session_activated` | A held or modal binding starts its session.               | `gnoblin.shortcut.session.activated`   |
| `session_key`       | A session receives a key.                                 | `gnoblin.shortcut.session.key`         |
| `session_ended`     | A session ends.                                           | `gnoblin.shortcut.session.ended`       |
| `closed`            | The binding has ended.                                    | None                                   |

The global events stay for code that watches every binding. The `activated`
event carries a [`grant`](/shell-api/grants) on a real activation.

`shortcut:unbind()` removes the binding and returns an `Operation`. The
`Shortcut` ends, and its `closed` listeners run, when the compositor has removed
the binding. Listening ends after `closed`, and `alive` is false. Binding the
same ID again makes a new object.

`shortcut:end_session(session_id)` takes the `session_id` from the binding's
`session_activated` signal or `gnoblin.shortcut.session.activated` event. It is
a method of the shortcut because the shortcut owns the session.

- It releases the session's keyboard capture and keeps the binding registered.
- A stale session ID, or a binding owned by another connection, is rejected.
- The matching ended event uses reason `cancelled`.

Socket clients keep the wire names `shortcut.unbind` and `shortcut.session.end`.

This binding ends each of its sessions when the session starts:

```lua
local switcher = gnoblin.shortcuts.bind("switcher", "<Alt>Tab", {hold = "alt", mode = "modal"})

switcher:on("session_activated", function(shortcut, event)
    shortcut:end_session(event.session_id)
end)

switcher:on("closed", function(shortcut, reason)
    print("switcher ended", reason)
end)
```

Dynamic registrations belong to the Lua runtime that created them. They end
when the runtime stops or reloads, and the `"closed"` listeners of a `Shortcut`
run with the reason `"config_reloaded"` or `"runtime_stopped"`. Active sessions end on modifier release,
unbind, config reload, session lock, capture preemption, owner disconnect, or
after ten seconds.

The ten seconds are a fixed guard against a lost key release, not a lease:
nothing renews it. To take the keyboard for a script, with a lease and composed
text, use a [keyboard grab](/shell-api/input-grab), which also explains why
sessions do not use it.

Lua listeners can subscribe to these shortcut session events. A `Shortcut` raises
the same events on its own object as `session_activated`, `session_key` and
`session_ended`:

- `gnoblin.shortcut.session.activated` when a held binding starts.
- `gnoblin.shortcut.session.key` for captured keyboard input. Fields include
  `keyval`, `keycode`, `modifiers`, `phase` (`press` or `release`), and `time`.
  Real key presses and releases can carry a `grant`; repeated
  events do not carry one.
- `gnoblin.shortcut.session.ended` when a session ends. Reasons are `released`,
  `unbound`, `owner_disconnected`, `config_changed`, `locked`, `preempted`,
  `timed_out`, `cancelled`, `compositor_stopped`, and `runtime_stopped`. The
  `cancelled` reason is used by `shortcut.session.end`. `runtime_stopped` is
  sent to a socket client when the Lua runtime stops while that client owns the
  active session.

Lua listeners receive the same authority as a [`Grant`](/shell-api/grants)
in `event.grant` on real, non-repeated `session.key` events. Synthetic and input-method
events are excluded. The gesture ends after five seconds, and each focus-sensitive
operation can take its scope from it once. Gnoblin still delivers the key event if
it cannot issue a grant.

`gnoblin.shortcut.binding-activated` can carry a `grant` on
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
[compositor bridge](/bridge/).

```lua
gnoblin.once("gnoblin.window.focused", function()
    local operation = gnoblin.shortcuts.capture({timeout = 10})
    operation:on_complete(function(value, err)
        if err then
            print("Shortcut capture failed: " .. err.message)
        else
            print(value.accelerator)
        end
    end)
end)
```

See [`gnoblinctl`](/cli/) for command-line forms of these operations.
