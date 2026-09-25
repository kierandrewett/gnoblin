# Runtime Data

`src/data/` owns installed configuration and session data. These files are read
at runtime, installed into the prefix, or copied into a submodule; avoid
treating them as passive examples.

## Files

- `session/modes/gnoblin.json` defines the GNOME Shell session mode. It disables
  the stock panel, overview, notification banners and welcome dialog.
  Password, permission, network and removable-drive agents remain enabled.
  `gnoblinControl` provides the control API. These agents can display prompts;
  they are not all background-only services.
- `session/gnome-session/gnoblin.session` + `session/gnoblin.desktop` register
  the session at the login manager; installed by `scripts/install-session.sh`.
- `org.gnoblin.shell.gschema.xml` defines the GSettings schema for
  `org.gnoblin.Shell`.
    - `disabled-features` is an array of feature IDs to turn off at runtime.
    - By default, it disables `osd`, `osd-volume`, `osd-microphone`,
      `osd-brightness`, `osd-keyboard-brightness`, `osd-pad`, `screenshot`,
      `notifications`, and `input-source-switcher`.
    - Any feature omitted from the array remains enabled.
    - The schema manifest copies this file into the patched GNOME Shell tree.
- `init.lua.example` is the complete user-facing configuration reference.
    - The session launcher copies it to `~/.config/gnoblin/init.lua` only when no
      user config exists.
    - Packages install it under `/usr/share/gnoblin/`.
    - The overlays read `$GNOBLIN_CONFIG` or
      `$XDG_CONFIG_HOME/gnoblin/init.lua`; the selected file is evaluated as Lua.
    - An unset key uses the caller's enabled default. Stock session modes do not
      register the Gnoblin globals.
- `gnoblin-gnome-wallpaper` is a separate GTK layer-shell client installed from
  `src/tools/`. Its autostart entry restarts it if it exits during the session.
  The client uses GNOME background settings through `GnomeBG`; Gnoblin Shell's
  desktop background actors are disabled in Gnoblin mode.

## Verification

```sh
glib-compile-schemas --strict --dry-run src/data
```

Schema changes: after a source build, `just test-control-api` exercises the
`org.gnoblin.Shell` protocol that reads the `disabled-features` key.
