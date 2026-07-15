# Runtime Data

`src/data/` owns installed configuration and session data. These files are read
at runtime, installed into the prefix, or copied into a submodule; avoid
treating them as passive examples.

## Files

- `session/modes/gnoblin.json` is the GNOME Shell **session mode** that strips
  the stock UI (empty panel, `hasOverview: false`, minimal components +
  `gnoblinControl`). This is the low-patch way gnoblin removes GNOME's chrome.
- `session/gnome-session/gnoblin.session` + `session/gnoblin.desktop` register
  the session at the login manager; installed by `scripts/install-session.sh`.
- `org.gnoblin.shell.gschema.xml` is the GSettings schema for the
  `org.gnoblin.Shell` control protocol: `disabled-features`, the runtime
  feature toggles above. Its `manifest` copies it into the patched GNOME
  Shell tree.
- `gnoblin.conf.example` is the user-facing reference for implemented Mutter
  protocol gates. The overlays read `$GNOBLIN_CONFIG` or
  `$XDG_CONFIG_HOME/gnoblin/gnoblin.conf`. Within a Gnoblin session, an unset
  key uses the caller's enabled default; stock session modes register none of
  these globals.

## Verification

```sh
glib-compile-schemas --strict --dry-run src/data
```

Schema changes: after `just dev`, `just gnome-dbus-verify` exercises the
`org.gnoblin.Shell` protocol that reads the `disabled-features` key.
