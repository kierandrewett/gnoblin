# Runtime data

`data/` contains files installed with the standalone Gnoblin session.

- `gnoblin-portals.conf` selects the Gnoblin backend when installed and uses
  another available backend as fallback. The core package and local session
  registration install it as the desktop-specific portal default.
- `session/gnoblin.desktop` registers Gnoblin as a Wayland session. The
  installer writes the selected prefix into its `Exec` entry.
- `session/systemd-user/gnoblin-session.target` groups Gnoblin's user services.
- `session/systemd-user/gnoblin-idle.service.in` starts the standalone idle
  service. The installer fills in its prefix.
- `session/schemas/00_org.gnoblin.mutter.gschema.override` supplies Gnoblin's
  Mutter defaults and is compiled with the other installed schemas.
- `pam.d/gnoblin-lock` is the PAM service the session lock checks passwords
  through. It includes the system `login` stack for `auth` and `account`, and
  `xtask dist` installs it as `lib/pam.d/gnoblin-lock`.

The session data is installed by `cmake/install-session.cmake`.
