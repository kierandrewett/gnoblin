# gnoblin.snapshot

Return a copy of the configuration assembled so far. Changing the copy does not change Gnoblin's config. Snapshot keys use the internal hyphenated names, and named shortcuts and autostart entries are lists.

```lua
local settings = gnoblin.snapshot()
print(settings.shell["minimize-duration"])
```

Use this in files loaded after a shell's config to inspect earlier settings. For named entries you want to edit or loop over, use [`gnoblin.configure.shortcuts`](/config/configure/shortcuts) or [`gnoblin.configure.autostart`](/config/configure/autostart). See [file loading](/guides/files_and_load_order) for evaluation order.

The named views expose shortcuts by name, so a later file can inspect or
change one without copying the whole list:

```lua
for name, shortcut in pairs(gnoblin.configure.shortcuts) do
    print(name, shortcut.binding or "built-in action")
end
```
