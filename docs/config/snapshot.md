# gnoblin.snapshot

Return a copy of the configuration assembled so far. Changing the copy does
not change Gnoblin's config. Snapshot section and field names use the internal
hyphenated form, and named shortcuts and autostart entries are lists. For
example, inspect the pointer-location setting:

```lua
local settings = gnoblin.snapshot()
print(settings.compositor["locate-pointer"])
```

Use this only while a config loads, to inspect earlier settings. At run time
`gnoblin.snapshot()` raises an error. Read the committed settings from the
[`gnoblin.settings`](/shell-api/) snapshot instead.
For named entries you want to edit or loop over, use
[`gnoblin.configure.shortcuts`](/config/configure/shortcuts) or
[`gnoblin.configure.autostart`](/config/configure/autostart). See
[file loading](/guides/files_and_load_order) for evaluation order.

## Type definition

```lua
local settings = gnoblin.snapshot() -- returns a detached config table; load time only
```
