# Configuration files

Use the Lua runtime API to inspect the active configuration path, print the
bundled default Lua source, or restore the standard configuration directory.

```lua
print(gnoblin.config.path())
print(gnoblin.config.default())
```

`path()` follows `GNOBLIN_CONFIG` when set. Otherwise it uses
`$XDG_CONFIG_HOME/gnoblin/init.lua`, or `~/.config/gnoblin/init.lua` when
`XDG_CONFIG_HOME` is unset. It also recognizes the existing `gnoblin.toml` and
`gnoblin.conf` files while selecting the active path.

`default()` returns the bundled `init.lua` source as a string. To install all
bundled defaults, call `gnoblin.config.restore_default()`. It returns an
`Operation`, saves the existing standard config directory as a recovery backup,
installs the embedded files, and requests a live reload. If `GNOBLIN_CONFIG`
selects a single file outside the standard `init.lua` path, restoration fails
without replacing its parent directory.

`gnoblinctl init` creates a missing standard config directory. It is a
command-line setup action and has no Lua API equivalent.
