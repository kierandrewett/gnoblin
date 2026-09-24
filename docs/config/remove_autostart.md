# gnoblin.remove_autostart

This older helper removes a named autostart entry from the config. Prefer setting `enable = false` on its `gnoblin.configure.autostart.NAME` entry. It does not stop a process that is already running.

```lua
gnoblin.remove_autostart("panel")
```

See [`gnoblin.autostart`](/config/autostart) to define or update an entry.
