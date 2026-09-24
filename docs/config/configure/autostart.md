# gnoblin.configure.autostart

Define commands to run once per login. Entries merge with the same name; omitted fields keep their earlier values. Set `enable = false` to disable an imported entry. Disabling an entry does not stop a process that is already running.

```lua
gnoblin.configure {
    autostart = {
        panel = {command = {"waybar"}},
    },
}
```

See the [autostart guide](/guides/autostart) for launch timing and examples. The older [`gnoblin.autostart`](/config/autostart) declaration form is also available.
