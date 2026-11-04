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
[compositor bridge](/bridge/).

## Animation controls

The Lua runtime exposes animation controls through `gnoblin.animations`. Keep
`gnoblin.animation { ... }` for declaring or updating a named animation in
configuration.

| Method                                              | Arguments                  | Successful result                  |
| --------------------------------------------------- | -------------------------- | ---------------------------------- |
| `gnoblin.animations.list()`                         | None                       | Read-only `AnimationInfo[]`        |
| `gnoblin.animations.get(name)`                      | Animation name             | Read-only `AnimationInfo` or `nil` |
| `target:inspect_animation(name, event?)`            | Name; optional event       | `Operation<AnimationInspection>`   |
| `target:preview_animation(name, event?, autoplay?)` | Name; optional event; flag | `Operation<AnimationPreview>`      |

`target` is a `Window` or a `LayerSurface`. Find one with `gnoblin.windows.active()`,
`gnoblin.find("Window", filter)` or `gnoblin.find("LayerSurface", filter)`. The calls take no
window ID, layer ID or target type: the object you call is the target.

`gnoblinctl lua` supports the same preview record methods. The CLI waits for each
operation and returns the updated read-only preview, or the stop result.

| Argument   | Accepted value                           | Meaning                                       |
| ---------- | ---------------------------------------- | --------------------------------------------- |
| `name`     | 1–80 ASCII letters, digits, `_` or `-`   | Selects an animation.                         |
| `event`    | Optional lowercase kebab-case event name | Selects an event supported by that animation. |
| `autoplay` | Boolean; default `false`                 | Starts a preview immediately when `true`.     |

See the [animation guide](/guides/animations) for supported animation events.
In Lua, `window:preview_animation(name, event?, autoplay?)` returns an `Operation`. Its
successful value is an `AnimationPreview`, a live object like a
[window](/shell-api/windows#live-window-objects): one handle for one `id`, which each call
on it updates. The compositor names a preview by its `id`; the record has no other name for it.

- `preview.alive` is `false` once `stop()` has succeeded, a call on the preview has failed
  because it is gone, or a config reload has ended the handle.
- `preview:on(signal, fn)` takes `"changed"`, with the list of changed fields, or `"closed"`,
  with the reason when there is one. It returns a subscription.
- `preview:watch(field, fn)` calls `fn(preview, value, old)` when `event`, `progress`,
  `playing`, `paused` or `spec` changes.

Control a preview with the methods of its record. They return new operations:

- `preview:seek(progress)` takes a value from 0 to 1.
- `preview:step(milliseconds)` takes 1 to 60000.
- `preview:play()`, `preview:pause()` and `preview:stop()` take no argument.

The stop result contains an `ok` flag and the `session` ID of the stopped preview, which is its `id`.

Configured `open` and `close` animations run for normal windows during their
map and destroy lifecycles. The compositor starts built-in or registered
animations whose event matches the selected rule. The native methods below
control explicit preview sessions.

For example, inspect a new window's `open` animation, seek to its midpoint,
then play from there:

```lua
gnoblin.on("gnoblin.window.created", function(event)
    local request = event.window:preview_animation("gnome-open", "open")

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
