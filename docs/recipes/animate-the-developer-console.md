# Animate the developer console

Give the built-in console a short fade and slide. Add these declarations after
your `gnoblin.load(...)` lines in `~/.config/gnoblin/init.lua`:

```lua
gnoblin.animation {
    name = "console-arrive",
    event = "console-open",
    duration = 190,
    ease = "ease-out-cubic",
    from = {y = -18, opacity = 0},
    to = {y = 0, opacity = 1},
}

gnoblin.animation {
    name = "console-leave",
    event = "console-close",
    duration = 130,
    ease = "ease-in-quad",
    from = {y = 0, opacity = 1},
    to = {y = -12, opacity = 0},
}
```

The console uses the registered animation for each event automatically. Open
it with **Alt+F2** to see the entrance, then press **Escape** to close it.
Set either duration to `0` to make that transition immediate.

`y` is a translation in logical pixels. `opacity` goes from `0` (transparent)
to `1` (opaque). See [animation events and properties](/guides/animations#properties-and-keyframes)
for the other supported values.
