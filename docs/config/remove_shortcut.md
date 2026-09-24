# gnoblin.remove_shortcut

This older helper removes a named shortcut declared earlier during this config load. Prefer setting `enable = false` on its `gnoblin.configure.shortcuts.NAME` entry. A later `gnoblin.shortcut` call with the same name can add it again.

```lua
gnoblin.remove_shortcut("terminal")
```

See [`gnoblin.shortcut`](/config/shortcut) to define or update a shortcut.
