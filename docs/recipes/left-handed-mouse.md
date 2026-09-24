# Set up a left-handed mouse

Swap the primary mouse buttons and slow the pointer slightly:

```lua
gnoblin.configure {
    input = {
        mouse = {
            left_handed = true,
            speed = -0.2,
        },
    },
}
```

Mouse speed ranges from `-1` to `1`; `0` uses the system default. Add
`natural_scroll = true` if you also want to reverse the scroll direction. See
the [mouse reference](/config/configure/input/mouse).
