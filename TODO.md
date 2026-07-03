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
  gschema key.
- **Wayland soft-reload** — reloads theme + extensions in-process so windows
  survive; over D-Bus (`Reload`) and via `Alt+F2` `r`.
- **GNOME Shell patch set** — relaxed extension loading, unsafe-mode, portal
  auto-grant, `--disable-extensions` flag, hidden native top bar, branding,
  feature schema.
- **Mutter protocol overlays** — layer-shell v5 + the rest, each gated via
  `gnoblin.conf` `[protocols]`.
- **Retirement cleanup** — deleted `src/compositor/`, `src/clients/`,
  `subprojects/slint`, `dist/`, devkit scripts + tests.

## Next

- More feature toggles beyond `osd`/`screenshot` — notifications, polkit.
- Extension sideload + hot-reload workflow (relaxed loading is in; wire the
  ergonomics).
- Unattended screensharing — portal auto-grant exists; add persist / restore via
  `restore_token`.
- A scripting layer for automating the desktop.
- `gnome-control-center` fork for gnoblin settings.
- `kobel` — Kieran's personal Quickshell chrome config, in a separate repo.
