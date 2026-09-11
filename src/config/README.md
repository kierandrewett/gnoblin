# Configuration readers

`gnoblin-toml.c` converts TOML into nested GVariant dictionaries using the pinned
MIT-licensed parser in `tomlc99/`. Mutter exposes this through
`Meta.gnoblin_parse_toml()` so GNOME Shell uses the same parser.

`gnoblin-config.c` reads startup protocol settings. It prefers
`$XDG_CONFIG_HOME/gnoblin/gnoblin.toml`, then the existing `gnoblin.conf`.
`$GNOBLIN_CONFIG` overrides the path. Explicit `.conf` paths use the legacy
INI parser; other paths use TOML. The legacy parser retains its repeated-key,
quote and comment behaviour for existing installations.

`gnome-shell-overlay/js/ui/components/gnoblinConfig.js` validates the shell
schema and autostart records, watches saves and retains the last valid state.
TOML files may use a root-level `include` (or `source`) string/array. Paths are
relative to the including file, included files are merged in declaration order,
and rule arrays append. Both the native reader and the shell watch included
files, so package fragments can be installed once and hot-reloaded. See
`docs/configuration.md` for user-facing semantics and migration.

Run `./scripts/test-config.sh` for native TOML and legacy parser tests. The
live shell tests require the built Mutter typelib and library from the same
prefix. No new system library or external parser process is required.
