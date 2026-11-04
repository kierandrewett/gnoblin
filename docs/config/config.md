# gnoblin.config

The `gnoblin.config` table holds three functions that work with the config files on disk. It does not hold your
settings. Reading any other key of `gnoblin.config`, or assigning to it, raises an error that points to
[`gnoblin.configure`](/config/configure) and `gnoblin.settings.override`.

Use `gnoblin.configure` to declare settings with public `snake_case` keys. Use
[`gnoblin.settings`](/shell-api/) to read the committed settings. While a config loads,
[`gnoblin.snapshot`](/config/snapshot) returns a copy of what the files built so far.

```lua
gnoblin.configure {compositor = {locate_pointer = false}}

gnoblin.config.compositor["locate-pointer"] = true -- error: use gnoblin.configure
```

## Functions

| Function                           | Result                                                                                        |
| ---------------------------------- | --------------------------------------------------------------------------------------------- |
| `gnoblin.config.path()`            | The active config file path. It uses `GNOBLIN_CONFIG` and the XDG defaults.                   |
| `gnoblin.config.default()`         | The bundled default Lua config source, as a string.                                           |
| `gnoblin.config.restore_default()` | An `Operation`. It saves the config directory as a backup, restores the defaults and reloads. |

## Type definition

```lua
local path = gnoblin.config.path() -- string
local source = gnoblin.config.default() -- string
local operation = gnoblin.config.restore_default() -- Operation<AcceptedResult>
```
