# Use a visual bell

Replace the audible bell with a short flash on the focused frame:

```lua
gnoblin.configure {
    compositor = {
        visual_bell = true,
        audible_bell = false,
        visual_bell_type = "frame-flash",
    },
}
```

The visual bell type can also be `"fullscreen-flash"`. These settings apply
when a client requests a bell; they do not replace an application's own
notifications. See the [compositor reference](/config/configure/compositor).
