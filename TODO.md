# gnoblin — TODO

Working tracker for the **patched-GNOME** direction (pinned Mutter 49.5 +
GNOME Shell 49.6). Kept in the repo, not Claude's task tool. Newest asks bubble
to the top of **Next**.

> The from-scratch C++ compositor + Rust/Slint clients were retired. Recover
> them from the `archive/cpp-compositor` tag.

## Done

- **Build revival** — `just dev` builds patched mutter + gnome-shell + session
  data into `./install`. `just gnome-verify` (boots gnome-shell in gnoblin mode,
  checks `zwlr_layer_shell_v1`), `just gnome-dbus-verify` (org.gnoblin.*
  round-trip), `just test-config`, `just test-mutter`.
- **Session-mode UI strip** — `src/data/session/modes/gnoblin.json`
  (parentMode `user`, empty panel, `hasOverview:false`, minimal components).
  Stock chrome gone without heavy JS patching.
- **`org.gnoblin.Shell` control component** — `gnoblinControl.js` session
  component: Ping / GetVersion / Reload + ListFeatures/GetFeature/SetFeature/
  FeatureChanged, persisted in the `org.gnoblin.shell` `disabled-features`
  gschema key. Feature toggles now cover `osd` (+ per-type), `screenshot`,
  and `notifications`.
- **Wayland soft-reload** — reloads theme + extensions in-process so windows
  survive; over D-Bus (`Reload`) and via `Alt+F2` `r`.
- **Extension hot-reload + GJS scripting layer** — `ReloadExtension` cache-busts
  an extension's import; `ScriptHost` loads/hot-reloads
  `~/.config/gnoblin/scripts/*.js` against a small event bus. Both driven over
  `org.gnoblin.Shell` / `gnoblinctl`, both covered by `gnome-hot-reload-verify`
  and `gnome-scripting-verify`.
- **GNOME Shell patch set** — relaxed extension loading, unsafe-mode, portal
  auto-grant, `--disable-extensions` flag, hidden native top bar, branding,
  feature schema.
- **Mutter protocol overlays** — layer-shell v5 + the rest, each gated via
  `gnoblin.conf` `[protocols]`.
- **Unattended screensharing** — `xdg-desktop-portal-gnome` patch adds
  macOS-style per-app persistent screencast/RemoteDesktop grants (tick "always
  allow" once, never re-prompted), stored under
  `~/.config/gnoblin/portal-grants/`, managed via `gnoblinctl screen-grants` /
  `revoke-grant`.
- **`gnome-control-center` fork for gnoblin settings** — a `gnoblin` panel
  driving feature toggles, screencast grants, and a reload button; hides the
  Multitasking panel. `just dev-settings` (not part of `just dev`).
- **Devkit** — `just gnome-devkit` boots a visible nested gnoblin session +
  wired terminal for iterating on your own chrome without logging out;
  `just gnome-devkit-verify` regression-tests the env plumbing.
- **Retirement cleanup** — deleted `src/compositor/`, `src/clients/`,
  `subprojects/slint`, `dist/`, and the old C++-compositor-era devkit
  scripts + tests (the current `Devkit` entry above is a from-scratch
  replacement for the patched-GNOME stack, not a survivor of this cleanup).
- **Docs + dead-code sweep** — user guides under `docs/` (installation, devkit,
  testing, configuration), root README now links into them. Removed docs/
  scripts/schema left over from the pre-pivot layout (stale gschema keys with
  no reader, dangling references to deleted scripts, a missing
  `scripts/devkit-document-portal-stub.py` the devkit's isolated D-Bus config
  depended on but never shipped).
- **Real-login session registration** — `just dev-session` now also installs
  gnoblin-specific systemd --user units (`org.gnoblin.Shell.target` /
  `@wayland.service`, pointed at the patched gnome-shell via a wrapper that
  prepends this prefix's lookup paths); `just dev-session-register` links
  them into your live systemd --user instance and prints the one remaining
  (root) command to add "Gnoblin" to your login manager's picker. Previously
  a prefix-only `just dev-session` install had no working path to a real
  login at all: gnome-session's `RequiredComponents=org.gnome.Shell` would
  have resolved to a system GNOME Shell install's own systemd unit, not the
  patched one, silently launching the wrong shell. Gnoblin-specific unit
  names sidestep that instead of shadowing the shared `org.gnome.Shell`
  unit, so a system GNOME session (if present) is unaffected either way.
  `systemctl --user link` + `daemon-reload` was verified on a real machine
  with an existing system GNOME install: the units link cleanly, and
  `org.gnome.Shell@wayland.service` stays resolved to the system shell
  throughout (no shadowing). Actually selecting "Gnoblin" at GDM and
  logging in is NOT yet verified — that needs the manual root step in
  docs/installation.md, then a real logout/login, which wasn't done as part
  of this pass.

## Next

- Remaining feature toggle: `polkit`.
- Extension sideload ergonomics — hot-reload itself works
  (`gnome-hot-reload-verify`); sideloading is still "drop it in
  `~/.local/share/gnome-shell/extensions/<uuid>/` by hand".
- `kobel` — Kieran's personal Quickshell chrome config, in a separate repo.
