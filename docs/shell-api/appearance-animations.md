# Appearance and animations

`gnoblin.appearance.color_scheme()` returns the current desktop color scheme:
`default`, `prefer-dark`, or `prefer-light`. It returns `nil` when the
`org.gnome.desktop.interface/color-scheme` schema or key is unavailable.

Use it from `gnoblinctl lua` to read the current value:

```lua
local scheme = gnoblin.appearance.color_scheme()
print(scheme or "unavailable")
```

The native runtime updates this read before delivering
`gnoblin.appearance.color-scheme-changed`. Read it from a runtime callback after
the initial compositor snapshot arrives. Calling it while the initial config
is being evaluated raises an error because that snapshot has not arrived yet.

Socket clients can read the same preference through the API described in the
[compositor bridge](/compositor-bridge).


## Animation controls

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
