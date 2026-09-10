# Runtime Data

`src/data/` owns installed configuration and session data. These files are read
at runtime, installed into the prefix, or copied into a submodule; avoid
treating them as passive examples.

## Files

- `session/modes/gnoblin.json` defines the GNOME Shell session mode.
  It disables the stock panel, overview, notification banners and welcome dialog.
  Password, permission, network and removable-drive agents remain enabled.
  `gnoblinControl` provides the control API. These agents can display prompts;
  they are not all background-only services.
- `session/gnome-session/gnoblin.session` + `session/gnoblin.desktop` register
  the session at the login manager; installed by `scripts/install-session.sh`.
- `org.gnoblin.shell.gschema.xml` is the GSettings schema for the
  `org.gnoblin.Shell` control protocol: `disabled-features`, the runtime
  feature toggles above. Its `manifest` copies it into the patched GNOME
  Shell tree.
- `gnoblin.toml.example` is the user-facing reference for implemented Mutter
  protocol gates. The overlays read `$GNOBLIN_CONFIG` or
  `$XDG_CONFIG_HOME/gnoblin/gnoblin.toml`. Within a Gnoblin session, an unset
  key uses the caller's enabled default; stock session modes register none of
  these globals.

## Verification

```sh
glib-compile-schemas --strict --dry-run src/data
```

Schema changes: after `just dev`, `just gnome-dbus-verify` exercises the
`org.gnoblin.Shell` protocol that reads the `disabled-features` key.
