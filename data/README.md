# Runtime data

`data/` contains files that `cargo xtask dist install` copies into the install prefix (see `xtask/src/dist.rs` for the
table and `docs/install-layout.md` for the result).

- `js/` is the TypeScript library, `gnoblin-core` and `gnoblin-std`, installed as `share/gnoblin/js/`.
- `gnoblin-portals.conf` selects the Gnoblin backend when installed and uses another available backend as fallback. It is
  installed as the desktop-specific portal default.
- `session/gnoblin.desktop` registers Gnoblin as a Wayland session. The installer writes the selected prefix into its `Exec`
  entry.
- `session/systemd-user/gnoblin-session.target` groups Gnoblin's user services.
- `pam.d/gnoblin-lock` is the PAM service the session lock checks passwords through. It includes the system `login` stack
  for `auth` and `account`, and the install puts it at `lib/pam.d/gnoblin-lock`.
