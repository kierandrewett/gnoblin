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
| `on_finished(fn)`                                      | Call `fn(tween, outcome)` when the tween ends.          |

A property step starts from the value the window has when the step begins.
`seconds` is from 0 to 600. A callback does not hold the timeline back.

The animatable window properties are:

- `opacity`, from 0 to 1.
- `scale`, from 0.01 to 10, about the window's centre.
- `rotation`, in degrees from -3600 to 3600.
- `offset`, a table `{x = number, y = number}` in logical pixels.

They are the same values as the [window draw methods](/shell-api/windows#change-how-a-window-is-drawn),
and the window keeps the values a tween ends with. Any other property raises an
error.

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
- `"cancelled"`: a config reload, a stopped Lua worker or a session lock
  stopped it.

You cannot add steps after `play()`. A tween cannot start while the session is
locked.

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
