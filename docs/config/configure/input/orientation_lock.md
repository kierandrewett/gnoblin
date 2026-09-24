# gnoblin.configure.input.orientation_lock

Override the system's screen-rotation lock with
`gnoblin.configure.input.orientation_lock`:

```lua
gnoblin.configure {
    input = {
        orientation_lock = true,
    },
}
```

Use `true` to lock the current orientation or `false` to allow automatic
rotation. Remove the field to restore GNOME's orientation-lock setting. Changes
apply on config reload.
