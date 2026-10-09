# Runtime data

`src/data/` contains files installed with the standalone Gnoblin session.

- `default-config/` is the complete config tree embedded into the session
  binaries. `gnoblinctl init` writes it explicitly; recovery evaluates it
  in memory. `init.lua.example` mirrors its small entry point.
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
- `shaders/tint.frag` is an example shader available to configuration rules.

The session data is installed by `scripts/install-session.sh`.
