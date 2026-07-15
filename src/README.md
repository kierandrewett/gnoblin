# Source Map

`src/` is split by ownership boundary. gnoblin is patched GNOME (Mutter +
GNOME Shell); everything here is either an overlay copied into a submodule at
build time, or runtime data installed into the prefix. Start here to find where
a feature lives.

## Top-Level Areas

- `protocols/` — Mutter overlay sources for the implemented Wayland protocols:
  layer-shell, screencopy, idle-notify, data/gamma/output-power control, and
  foreign-toplevel list plus management. `session-lock/` and
  `output-management/` are deferred plans and vendored XML, not runtime
  implementations. `scripts/copy-overlay.sh` copies owned files into Mutter;
  the build and registration wiring lives under `patches/mutter/`.
- `config/` — the shared C `gnoblin.conf` parser. It is compiled into the Mutter
  protocol overlays (e.g. to gate protocols), so keep it C-compatible.
- `gnome-shell-overlay/` — the `gnoblinControl.js` session component that hosts
  the `org.gnoblin.Shell` control protocol (Ping/GetVersion/Reload + feature
  toggles). Copied into the GNOME Shell submodule via its `manifest` and
  registered by `patches/gnome-shell/30-gnoblin-control`.
- `data/` — installed runtime data: the `gnoblin` session mode + gnome-session +
  `.desktop` + gnoblin-specific systemd --user units (`session/systemd-user/`,
  `org.gnoblin.Shell.target`/`@wayland.service` — kept separate from the
  shared `org.gnome.Shell` unit so registering them never shadows a system
  GNOME Shell install), the `org.gnoblin.shell` gschema, and
  `gnoblin.conf.example`.
- `tools/` — runtime helpers installed by `scripts/install-session.sh`:
  `gnoblinctl` (the `org.gnoblin.Shell` control front-end), `gnoblin-env.sh`
  (the shared prefix and library lookup contract), `gnoblin-session` (the login
  entry's `Exec=` target), and `gnoblin-shell-service` (the systemd user-unit
  wrapper).
- `control-center/` — the `gnoblin` panel for the forked `gnome-control-center`
  (feature toggles, typed portal grants, and a reload button). Copied into the
  submodule via its `manifest` and registered by
  `patches/gnome-control-center/10-gnoblin-panel`. Built with `just dev-settings`
  (not part of `just dev`).

## Common Tasks

- Adding a Wayland protocol: create a directory under `protocols/`, add a
  `manifest`, wire it in `protocols/aggregator/`, then regenerate the Mutter
  protocol patch with `scripts/gen-gnoblin-protocols-patch.sh` and gate it with a
  `[protocols]` key in `gnoblin.conf`.
- Changing what the `org.gnoblin.Shell` protocol exposes: edit
  `gnome-shell-overlay/js/ui/components/gnoblinControl.js`.
- Changing how GNOME Shell's stock UI is stripped, or which background
  components load: edit `data/session/modes/gnoblin.json`.
- Changing the config parser or grammar: start in `config/`; keep the C parser
  and the parser test in `tests/config-test.c` in lockstep.
- Changing the documented protocol gates: update `data/gnoblin.conf.example`.
- Changing the login-manager/systemd wiring: `data/session/gnoblin.desktop`
  (Exec= gets rewritten at install time), `data/session/systemd-user/`, and
  `tools/gnoblin-session` / `tools/gnoblin-shell-service`. See
  `scripts/install-session.sh` and `scripts/register-session.sh`.
