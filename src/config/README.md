# Configuration reader

`gnoblin-lua.c` evaluates the main `init.lua` and its Lua or TOML includes in
one fresh Lua state. Lua builds one `gnoblin.config` table. The loader converts
that table into a nested GVariant document, then Shell validates the settings.
Mutter exposes the same loader through `Meta.gnoblin_load_config()`.

`gnoblin-glob.c` expands include patterns in sorted order. The loader reports
loaded files and searched directories, including missing dependencies. The
Shell watcher uses these paths to retry errors and detect new matching files.

`gnoblin-toml.c` uses the pinned MIT-licensed parser in `tomlc99/`. It remains
available through `Meta.gnoblin_parse_toml()` for existing callers and TOML edits.
`gnoblin-config.c` selects the root file and keeps the existing native settings
accessors and legacy INI parser. A missing root uses defaults; invalid edits
retain the last working state.

`$GNOBLIN_CONFIG` selects an explicit file. Otherwise `init.lua` is the entry
point under `$XDG_CONFIG_HOME/gnoblin`. Existing `gnoblin.toml` and `gnoblin.conf`
files are fallback choices. See `docs/configuration.md` for the public API.

Build with Lua 5.4 development files (`lua-devel` on Fedora). Run
`./scripts/test-config.sh` for native readers and glob expansion. Run
`tests/lua-shell-config-test.js` with GJS and the matching Mutter typelib for
module and directory watches. `scripts/test-live-shell-config.py` verifies
these behaviours in the private headless session.
