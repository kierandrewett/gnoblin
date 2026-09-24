# gnoblin.configure.autostart

Define commands to run once per login. Entries merge with the same name; omitted fields keep their earlier values. `when` currently accepts only `"on_login"` and defaults to that value. Set `enable = false` to disable an imported entry. Disabling an entry does not stop a process that is already running.

```lua
gnoblin.configure {
    autostart = {
        panel = {command = {"waybar"}, when = "on_login"},
    },
}
```

See the [autostart guide](/guides/autostart) for launch timing and examples.
