# Packaging

The files a distribution package of Gnoblin puts into the system, and the list of distributions Gnoblin targets.
`cargo xtask dist install` copies the installed files into the install prefix (see `xtask/src/dist.rs` for the table and
`docs/install-layout.md` for the result).

- `session/gnoblin.desktop` registers Gnoblin as a Wayland session. The installer writes the selected prefix into its `Exec`
  entry.
- `session/systemd-user/gnoblin-session.target` groups Gnoblin's user services.
- `pam.d/gnoblin-lock` is the PAM service the session lock checks passwords through. It includes the system `login` stack
  for `auth` and `account`, and the install puts it at `lib/pam.d/gnoblin-lock`.
- `gnoblin-portals.conf` selects the Gnoblin backend when installed and uses another available backend as fallback. It is
  installed as the desktop-specific portal default.
- `geoclue/50-gnoblin.conf` is a GeoClue drop-in that adds Gnoblin's location agent to GeoClue's whitelist. It is not
  installed: copy it to `/etc/geoclue/conf.d/` (see `docs/config/configure/location.md`).
- `targets.json` is the list of distributions and releases Gnoblin targets, with the evidence that exists for each.
  `scripts/check-packaging-targets.py` validates it, and `cargo xtask native-probe` reads it.

The package repositories are built by `scripts/pkgrepo` (see `scripts/pkgrepo/DESIGN.md`).
