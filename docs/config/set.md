# gnoblin.set

Compatibility function for configs that use Gnoblin's internal setting names. It merges maps and replaces supplied lists. Use [`gnoblin.configure`](/config/configure) for new config files; that function accepts public `snake_case` names.

```lua
gnoblin.set {shell = {["minimize-duration"] = 150}}
```

Existing files can also use `require("gnoblin")` to get the same API table and edit its raw `config` table. Raw keys use internal names.
