# gnoblin.snapshot

Return a copy of the configuration assembled so far. The returned table can be inspected or changed without mutating Gnoblin's active config document.

```lua
local settings = gnoblin.snapshot()
print(settings.shell.minimize_duration)
```

This is useful in files loaded after a shell's config when you need to inspect settings declared earlier. See the [file loading guide](/guides/files_and_load_order).
