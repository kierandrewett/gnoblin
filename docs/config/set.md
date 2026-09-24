# gnoblin.set

Merge a settings table into the current config document, merging maps recursively and replacing supplied lists. `gnoblin.configure` additionally converts public `snake_case` setting names to internal names; `gnoblin.set` accepts internal setting names.

```lua
gnoblin.set {shell = {minimize_duration = 150}}
```

For ordinary config files, use [`gnoblin.configure`](/config/configure), which accepts public `snake_case` names.
