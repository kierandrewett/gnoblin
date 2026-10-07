# gnoblin.config

The mutable compatibility table containing the configuration assembled so
far. Calls to [`gnoblin.configure`](/config/configure), [`gnoblin.load`](/config/load),
and declaration functions add to this table while Gnoblin evaluates config
files.

Use `gnoblin.configure` to write settings with public `snake_case` keys. Direct
table access uses normalized section and field names. For example, configure
pointer location and inspect the compatibility table:

```lua
gnoblin.configure {compositor = {locate_pointer = false}}

local locate_pointer = gnoblin.config.compositor["locate-pointer"]
gnoblin.config.compositor["locate-pointer"] = true
```

For reads, prefer the detached, read-only [`gnoblin.settings`](/config/runtime-api)
snapshot. `gnoblin.configure` is the supported way to declare settings;
direct edits through `gnoblin.config` remain for compatibility and are
validated with the rest of the config when the file finishes loading.

## Type definition

`gnoblin.config` is the mutable compatibility map. Its fields use the
normalized, hyphenated names; direct writes are checked when the config file
finishes loading. Prefer `gnoblin.settings` for reads and `gnoblin.configure`
for writes.

```lua
local settings = gnoblin.config -- table: string keys, values of any config type
```
