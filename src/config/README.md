# Configuration reader

Gnoblin evaluates one Lua root file in a fresh restricted Lua state. The
default root is `$XDG_CONFIG_HOME/gnoblin/init.lua`. `GNOBLIN_CONFIG` selects
an explicit root and does not require a `.lua` suffix.

Use `require('gnoblin')` to get `g`. `g.config` is the one live settings table.
Use `g.set { ... }` to merge settings. Use `g.load('conf.d/**/*.lua')` to load
sorted Lua fragments in the same state. `require()` loads a local Lua module
once and returns its original cached result.

The reader records loaded files and glob directories. A missing root gives an
empty document. A missing file named by `g.load()` is an error.

There is no TOML or INI configuration path.

Run `./tests/test-config.sh` for the native Lua and glob tests.
