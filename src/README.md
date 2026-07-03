# Source Map

`src/` is split by ownership boundary. gnoblin is patched GNOME (Mutter +
GNOME Shell); everything here is either an overlay copied into a submodule at
build time, or runtime data installed into the prefix. Start here to find where
a feature lives.

## Top-Level Areas

- `protocols/` — Mutter overlay sources for the Wayland protocols gnoblin adds
  to libmutter (layer-shell, screencopy, idle-notify, data/gamma/output-power
  control, foreign-toplevel list + management, KDE appmenu, session-lock and
  output-management scaffolding). These files are copied into the Mutter
  submodule by `scripts/copy-overlay.sh` and the wiring is persisted as patch
  files under `patches/mutter/`.
- `config/` — the shared C `gnoblin.conf` parser. It is compiled into the Mutter
  protocol overlays (e.g. to gate protocols), so keep it C-compatible.
- `gnome-shell-overlay/` — the `gnoblinControl.js` session component that hosts
  the `org.gnoblin.Shell` control protocol (Ping/GetVersion/Reload + feature
  toggles). Copied into the GNOME Shell submodule via its `manifest` and
  registered by `patches/gnome-shell/30-gnoblin-control`.
- `data/` — installed runtime data: the `gnoblin` session mode + gnome-session +
  `.desktop`, the `org.gnoblin.shell` gschema, `gnoblin.conf.example`, and legacy
  quick-settings plugin scripts (reference for a bring-your-own chrome).

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
- Changing a quick-settings plugin: update the matching `data/plugins/gnoblin-qs-*`
  script.
