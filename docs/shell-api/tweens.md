# Tweens and easing

A tween changes window properties over time. You build a timeline of steps in
Lua, and the compositor plays it in its render loop, so each frame follows the
display and does not wait for Lua.

```lua
gnoblin.on("gnoblin.window.created", function(event)
    local window = event.window
    window:set_opacity(0)
    gnoblin.tween()
        :property(window, "opacity", 1, 0.25, "cubic", "out")
        :parallel():property(window, "scale", 1, 0.25, "back", "out")
        :play()
end)
```

This fades and scales each new window in over a quarter of a second.

## Build a timeline

`gnoblin.tween()` returns an empty `Tween`. Each method adds a step and returns
the tween, so calls chain. Steps run one after another.

| Method                                                 | Step                                                    |
| ------------------------------------------------------ | ------------------------------------------------------- |
| `property(window, property, to, seconds, ease?, dir?)` | Change one window property to `to` over `seconds`.      |
| `interval(seconds)`                                    | Wait.                                                   |
| `callback(fn)`                                         | Call `fn(tween)` when the timeline reaches this step.   |
| `parallel()`                                           | Run the next step at the same time as the previous one. |

A property step starts from the value the window has when the step begins.
`seconds` is from 0 to 600. A callback does not hold the timeline back.

What a tween can drive is a property that a live object declares animatable. Only a
`Window` declares any today. The animatable window properties are:

- `opacity`, from 0 to 1.
- `scale`, from 0.01 to 10, about the window's centre.
- `rotation`, in degrees from -3600 to 3600.
- `offset`, a table `{x = number, y = number}` in logical pixels.

They are the same values as the [window draw methods](/shell-api/windows#change-how-a-window-is-drawn),
and the window keeps the values a tween ends with. A target or a property that no
kind declares raises an error that lists the kinds and their properties.

## Play and control a tween

| Method          | Effect                                                  |
| --------------- | ------------------------------------------------------- |
| `play()`        | Start the tween, or resume it after `pause()`.          |
| `pause()`       | Stop where it is.                                       |
| `seek(seconds)` | Jump to a time from the start. Callbacks before it run. |
| `kill()`        | Stop. Windows keep the values they have.                |

`play()` returns an `Operation` that succeeds with `{tween_id, outcome}` when
the tween ends. `tween.status` is `"idle"`, `"playing"`, `"paused"` or the
outcome. The outcome is one of these values:

- `"finished"`: the timeline reached its end, or every target window closed.
- `"killed"`: `kill()` stopped it.
- `"cancelled"`: a stopped Lua worker, a session lock or a config reload
  stopped it.

You cannot add steps after `play()`. A tween cannot start while the session is
locked.

## The Tween object

A `Tween` is a live object, like a [window](/shell-api/windows#live-window-objects).
It has an `id` from the moment `gnoblin.tween()` returns it, and these fields:

| Field      | Value                                             |
| ---------- | ------------------------------------------------- |
| `id`       | Identifier of the tween in the session.           |
| `alive`    | `false` once the tween has ended.                 |
| `status`   | `"idle"`, `"playing"`, `"paused"` or the outcome. |
| `revision` | Counts the changes of `status`; it starts at 1.   |

`tween:on(signal, fn)` takes one of these signals and returns a subscription:

- `"finished"` calls `fn(tween)` when the timeline reaches its end. A killed or cancelled tween
  does not raise it.
- `"closed"` calls `fn(tween, outcome)` when the tween has ended, however it ended. The outcome is
  `"finished"`, `"killed"`, `"cancelled"` or `"config_reloaded"`. Listening ends after `"closed"`.
- `"changed"` calls `fn(tween, changed)` with the list of changed fields.

`tween:watch("status", fn)` calls `fn(tween, value, old)` when the status changes.

```lua
local fade = gnoblin.tween():property(window, "opacity", 0.3, 0.4)
fade:on("finished", function() window:close() end)
fade:play()
```

A config reload waits for the other pending operations, but not for a playing tween. The tween keeps
playing until the new configuration takes effect.

Then the compositor stops it. The `Operation` of `play()` succeeds with the outcome `"cancelled"`, and the
`"closed"` signal of the tween runs with the reason `"config_reloaded"`. A tween that has not played when a
reload commits ends with `"config_reloaded"` too. A reload that fails leaves a playing tween as it is.

## Easing

`ease` names the curve. `dir` is `"in"` (slow start), `"out"` (slow end) or
`"in_out"` (both); it defaults to `"in_out"`. The curves are `linear`,
`sine`, `quad`, `cubic`, `quart`, `quint`, `expo`, `circ`, `back`, `elastic`
and `bounce`. `linear` is the default.

The same math is available in Lua:

| Function                      | Result                              |
| ----------------------------- | ----------------------------------- |
| `gnoblin.ease(name, t, dir?)` | Eased progress for `t` from 0 to 1. |
| `gnoblin.lerp(a, b, t)`       | `a + (b - a) * t`.                  |

```lua
print(gnoblin.ease("cubic", 0.5, "out")) -- 0.875
```
