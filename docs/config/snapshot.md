# gnoblin.snapshot

Return a copy of the configuration assembled so far. Changing the copy does
not change Gnoblin's config. Snapshot section and field names use the internal
hyphenated form, and named shortcuts and autostart entries are lists. For
example, inspect whether compositor animations are enabled:

```lua
local settings = gnoblin.snapshot()
print(settings.compositor["enable-animations"])
```

Use this to inspect earlier settings while a config is loading. For runtime
reads, prefer the immutable [`gnoblin.settings`](/config/runtime-api) snapshot.
For named entries you want to edit or loop over, use
[`gnoblin.configure.shortcuts`](/config/configure/shortcuts) or
[`gnoblin.configure.autostart`](/config/configure/autostart). See
[file loading](/guides/files_and_load_order) for evaluation order.

## Type definition

```lua
local settings = gnoblin.snapshot() -- returns a detached config table
```
