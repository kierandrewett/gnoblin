# gnoblin.load

Evaluate another Lua config file in the current config state. Relative paths resolve from the file making the call. A glob is sorted before evaluation; a pattern with no matches is ignored. See the [file loading guide](/guides/files_and_load_order).

```lua
gnoblin.load("conf.d/**/*.lua")
```

A missing explicit file is an error. Config loading is limited to 32 active files at once, including the root config.

## Lua `require`

`require` is a global function, not a member of `gnoblin`. It loads a local module from beside the calling file or its `lua/` directory, caches the result for the current reload, and returns it. It does not support Lua's normal installed module search paths.

```lua
local appearance = require("appearance")
gnoblin.configure(appearance)
```

Module names cannot contain `..`; use `gnoblin.load("parts/motion.lua")` for explicit subdirectories.
