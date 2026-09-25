# gnoblin.config

The mutable table containing the configuration assembled so far. Calls to
[`gnoblin.configure`](/config/configure), [`gnoblin.load`](/config/load), and
declaration functions add to this table while Gnoblin evaluates the config
files.

Use `gnoblin.configure` to write settings with public `snake_case` keys. Direct
table access uses the normalized config document, whose setting keys use
hyphens:

```lua
gnoblin.configure {shell = {minimize_duration = 200}}

local duration = gnoblin.config.shell["minimize-duration"]
gnoblin.config.shell["minimize-duration"] = 150
```

For most config changes, `gnoblin.configure` is clearer and performs public
key conversion. Direct edits use internal key names and are validated with the
rest of the config when the file finishes loading.

## Type definition

`gnoblin.config` is the mutable configuration map. Its keys use the
normalized, hyphenated names; direct writes are checked when the config file
finishes loading.

```lua
local settings = gnoblin.config -- table: string keys, values of any config type
```
