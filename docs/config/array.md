# gnoblin.array

Mark a Lua table as a list. This is useful for an empty list, because `{}` otherwise represents an empty settings map.

```lua
gnoblin.configure {
    window_rules = gnoblin.array {},
}
```

The marker is retained while Gnoblin converts the Lua document. For nonempty dense lists, ordinary Lua tables work.

## Type definition

Pass a Lua list table to `gnoblin.array`; the result is the same list marked
for config conversion.

```lua
local marked_list = gnoblin.array {value, ...}
```
