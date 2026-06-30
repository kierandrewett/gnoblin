# Runtime Data

`src/data/` owns installed configuration and plugin data. These files are read
at runtime or embedded into binaries; avoid treating them as passive examples.

## Files

- `gnoblin.conf.example` is the user-facing reference config installed under
  `share/gnoblin/`.
- `gnoblin.defaults.conf` is the shipped base layer. The compositor embeds it
  when `GNOBLIN_EMBED_DEFAULTS` is set, and Rust shell code embeds the same file
  for quick-settings plugin defaults.
- `org.gnoblin.shell.gschema.xml` is the GSettings schema consumed by GNOME
  Shell/control-center integration for shell placement toggles.
- `plugins/gnoblin-qs-*` are executable quick-settings plugin commands.
- `manifest` copies the schema into the patched GNOME Shell tree.

## Config Layers

`gnoblin.defaults.conf` is parsed first, then the user's
`$XDG_CONFIG_HOME/gnoblin/gnoblin.conf` is parsed on top. Repeated keys stay in
file order, and scalar lookups use the last value, so user keys override the
shipped base layer.

Keep config keys, values, and parser grammar stable unless all consumers are
updated together. In particular, quick-settings plugin IDs and command names are
referenced by config and by installed script names.

## Quick Settings Plugins

The built-in quick-settings tiles are command plugins declared in
`gnoblin.defaults.conf` as `[qs-plugin.NAME]` sections. The `command` value must
match an installed script name in `plugins/`.

Each script emits one JSON object on stdout describing a tile and optional menu.
Oneshot scripts receive interaction payloads in `GNOBLIN_QS_EVENT`; persistent
scripts receive ndjson events on stdin.

## Verification

Useful checks after editing this folder:

```sh
bash -n src/data/plugins/gnoblin-qs-*
glib-compile-schemas --strict --dry-run src/data
cc tests/config-example-test.c src/config/gnoblin-config.c -I src/config $(pkg-config --cflags --libs glib-2.0) -o /tmp/gnoblin-config-example-test
/tmp/gnoblin-config-example-test src/data/gnoblin.conf.example
```
