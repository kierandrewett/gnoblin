# Overrides

An override puts a value over the committed configuration, or over the window rules of one window, until your code
takes it back. Use it for a change that belongs to the running session, such as a mode that the user switches on, and
not for a setting that belongs in the configuration file. An override never writes the configuration.

Two calls make an override. Both return an `Override` object.

```lua
-- A setting, for the whole session.
local bell = gnoblin.settings.override("compositor.visual_bell", true)

-- An effect, for one window.
local blurred = window:set_effect("blur", 20)

-- Later:
bell:revoke()
blurred:revoke()
```

Call them while you handle an event, as with every operation. At the top level of a configuration file no operation
can start yet.

## Settings

`gnoblin.settings.override(path, value)` layers `value` over one committed setting. `path` is the section and the
key, with the public names of the configuration: `"window_management.focus_mode"`, `"compositor.blur"`.

Only a setting that can change while the session runs is accepted:

| Path                                 | Value                                             |
| ------------------------------------ | ------------------------------------------------- |
| `window_management.*`                | Any key of [window management][window-management] |
| `compositor.*`                       | Any key of the [compositor][compositor] section   |
| `cursor.theme`, `cursor.size`        | A theme name, a size from 1 to 256                |
| `layer_shell.preserve_active_window` | `true` or `false`                                 |
| `monitors.privacy_screen`            | `true` or `false`                                 |
| `input.orientation_lock`             | `true` or `false`                                 |

[window-management]: /config/configure/window_management
[compositor]: /config/configure/compositor

The call raises an error for any other path, and the error says why. For example, `xwayland.*` needs an Xwayland
restart, `shortcuts.*` are registered when the configuration loads, and `window_rules` are matched when a window
appears. The call raises an error too for a value that the configuration would not accept, with the message of the
configuration check.

`monitors.privacy_screen` needs an output that has a privacy screen. The orientation lock and the privacy screen take
`true` or `false`; to follow the configuration or the system setting again, revoke the override.

While an override is active, the compositor reads its value in place of the committed one. The override is not part
of the configuration, so these calls keep showing the committed values:

- `gnoblin.settings`
- `gnoblinctl config show`
- `gnoblin.focus.policy()`

Read the effect where it happens. For example, the `source` of `gnoblin.input.orientation_lock()` is `"runtime"`
while an override is active.

## Window effects

`window:set_effect(name, value)` layers one effect over one window. The effects are those that
[window rules](/config/window_rule#rule-fields) support:

| Effect                | Value                                                                 |
| --------------------- | --------------------------------------------------------------------- |
| `opacity`             | A number from 0 to 1.                                                 |
| `blur`                | A number from 0 to 100; 0 turns the blur off.                         |
| `blur_ignore_shadows` | `true` or `false`.                                                    |
| `shader`              | A GLSL file path, or `""` to clear the shader.                        |
| `shader_uniforms`     | A table of at most 64 numbers.                                        |
| `corners`             | The [corner fields](/config/window_rule#corners), including `shadow`. |
| `frame`               | The [frame fields](/config/window_rule#frame-fields).                 |

The value goes over every rule that matches the window, as one more rule after them. It changes only what it sets: a
`corners` table with only `radius` leaves the other corner fields as the rules give them. A name that is not in the
table raises an error that lists the effects.

`opacity` is the rule opacity of the window content. It is not `window:set_opacity()`, which changes how the whole
window is drawn; the two multiply.

The effect ends when you revoke it, when the window closes and when a configuration reload commits. The window then
shows what its rules give.

## The Override object

| Field    | Value                                                   |
| -------- | ------------------------------------------------------- |
| `id`     | `"override-"` and a number.                             |
| `alive`  | `true` until the override has ended.                    |
| `scope`  | `"settings"` or `"window"`.                             |
| `name`   | The setting path or the effect name.                    |
| `window` | The ID of the window of an effect. `nil` for a setting. |
| `value`  | The value that it layers.                               |

`override:revoke()` returns an `Operation`. The setting or effect returns to the next older layer, which is the
committed value when there is none, and the object ends. The operation fails with `not_found` for an override that has
already ended.

`override:on("closed", fn)` calls `fn(override, reason)` once, when the override ends. Listening ends after that.
`reason` is one of these:

| Reason              | Meaning                                                    |
| ------------------- | ---------------------------------------------------------- |
| `"revoked"`         | You called `revoke()`.                                     |
| `"replaced"`        | A newer override of the same target took over.             |
| `"window_closed"`   | The window of an effect closed.                            |
| `"config_reloaded"` | A configuration reload committed.                          |
| A message           | The compositor refused the override. The message says why. |

The object exists when you make it. If the compositor refuses the override, the object ends and `"closed"` gets the
message.

## Which override wins

When two overrides name one target, the second wins. A target is one setting, or one effect of one window. When you make a second
override of the same target, the first ends at once with the reason `"replaced"`, and its `"closed"` listeners run.
Revoking the second does not bring the first back.

Overrides of different targets do not affect one another. Two effects of one window, such as `blur` and `corners`,
apply together.

```lua
local a = gnoblin.settings.override("compositor.visual_bell", true)
local b = gnoblin.settings.override("compositor.visual_bell", false)
-- a.alive is false, and its "closed" listeners got "replaced".
b:revoke() -- the committed value shows again.
```

Socket clients that set the orientation lock or the privacy screen with `gnoblinctl input orientation-lock` or the
`monitors.set_privacy_screen` call take part in the same rule. Their value is an override that no Lua object holds.

## When overrides end

An override is owned by the runtime that made it. These end it:

- `revoke()`.
- A newer override of the same target.
- The close of the window, for an effect.
- A configuration reload that commits. The old runtime tells its objects before it goes, so a `"closed"` listener runs
  with `"config_reloaded"`. That listener can read, but it cannot start an operation. The new configuration makes its
  own overrides. A reload that fails keeps them.
- A restart of the Lua worker, and the stop of the compositor.

A reload also ends the orientation lock and privacy screen values that a socket client set.

## Overrides in `gnoblinctl lua`

A `gnoblinctl lua` script ends before anything could revoke an override, so these two calls raise an error there:

- `gnoblin.settings.override`
- `window:set_effect`

Make the override in a configuration. To set the orientation lock from a terminal, use
`gnoblinctl input orientation-lock on`, `off` or `inherit`.
