# Configuration

See [Window effects](window-effects.md) for blur rules, custom GLSL shaders,
shader uniforms and file hot reload, including a Bingux configuration example.

Gnoblin reads `~/.config/gnoblin/gnoblin.toml` and watches it for live changes.
`$XDG_CONFIG_HOME` changes the base directory. `$GNOBLIN_CONFIG` selects an
explicit file. A `.conf` override uses the legacy INI reader; other filenames
use TOML. Without an override, TOML takes priority over `gnoblin.conf`.
The watcher detects creation and atomic replacement of either default file.

Mutter and GNOME Shell share the native TOML parser. Duplicate keys, invalid
values and invalid protocol types are rejected. A bad live edit retains the
last valid configuration. The updated build needs installation and one new
login; later supported edits apply without logout.

## Window behaviour

```toml
[shell]
window-switcher = false
minimize-animation = "zoom"
minimize-duration = 200
# Optional fallback, in logical desktop coordinates:
# minimize-target = [960, 1040]
```

`zoom` is the default. Minimise and restore use, in order:

1. The window's dock icon rectangle, supplied by the dock.
2. `minimize-target = [x, y]`, if configured.
3. Bottom-centre of the window's monitor.

Coordinates use the logical desktop space, including monitor offsets. They
are not physical pixels. Each window can have a different dock icon target.
The dock hint moves with its surface, and is cleared when the surface or its
handle disappears. A zero-size rectangle clears the hint.

Other animation values are `"fade"` (stay in place), `"none"`, and `"gnome"`
(native icon-target animation with GNOME's top-corner fallback).
`minimize-duration` accepts 0 to 5000 milliseconds. Reduced-motion settings
still take precedence. Removing window-behaviour keys restores defaults.

`window-switcher` enables GNOME's application/window/group switchers and
cyclers. It defaults off. Disabling it suppresses those actions but does not
release their existing shortcut bindings. Display-mode and accessibility
switchers are separate. Stock GNOME sessions retain their native behaviour.

### Quickshell dock integration

Quickshell's [Toplevel.setRectangle](https://quickshell.org/docs/types/Quickshell.Wayland/Toplevel/)
sends the `wlr-foreign-toplevel-management` rectangle hint. Inside a dock icon
delegate, use the icon's rectangle relative to its PanelWindow content:

```qml
function updateTarget(toplevel) {
    const point = icon.mapToItem(dock.contentItem, 0, 0);
    toplevel.setRectangle(dock,
        Qt.rect(point.x, point.y, icon.width, icon.height));
}
```

Update the hint when the icon layout changes and when a window joins the app
group. Send `toplevel.setRectangle(dock, Qt.rect(0, 0, 0, 0))` when its
representation is removed.
For grouped icons, set the same rectangle on each represented window. Send
the hint before setting `toplevel.minimized = true`.

The Bingux reference is `shell/bingux/Dock.qml` in the Bingux checkout. Its
`dockButton.modelData.windows` array provides those grouped windows. The
Bingux Nix module already manages Quickshell through systemd; do not also
start that same instance through autostart.

## Autostart

```toml
[[autostart]]
name = "bingux"
command = ["qs", "-c", "bingux"]

[[autostart]]
name = "clipboard"
command = ["wl-paste", "--watch", "cliphist", "store"]
```

Each unique name starts once per login. Adding a new name on reload starts it.
Saving again, reloading scripts, or unlocking does not launch another copy.
A process that exits is not automatically restarted. Changing the command for
an already-started name takes effect on the next login. Removing an entry
prevents future starts; it does not kill a running process. Use systemd for
process supervision and restart policies.

Commands are argument arrays, not shell expressions. `$HOME`, `~`, pipes and
redirection are not expanded. Use absolute paths, a command available through
PATH, or explicitly invoke a shell if shell syntax is required. Failed
launches are logged and can be retried on a later reload.

## Runtime feature controls

The `[shell]` table also accepts booleans for `osd`, `osd-volume`,
`osd-microphone`, `osd-brightness`, `osd-keyboard-brightness`, `osd-pad`,
`screenshot`, and `notifications`:

```toml
[shell]
osd = false
screenshot = true
notifications = false
```

Explicit entries update the existing persistent GSettings state. Omitted
entries retain their current state. Removing an entry stops managing it from
the file; it does not reset its persistent value. CLI changes remain possible,
but the next reload reapplies explicit file entries. Validation completes
before feature settings or autostart commands are applied.

## Protocol settings

```toml
[protocols]
wlr-layer-shell = true
wlr-screencopy = true
ext-idle-notify = true
ext-foreign-toplevel-list = true
wlr-foreign-toplevel-management = true
wlr-gamma-control = true
wlr-output-power-management = true
ext-data-control = true
```

These settings require a new compositor session. Reload does not change
registered Wayland globals. All implemented protocols default on in Gnoblin;
stock sessions do not expose these Gnoblin globals. See
`src/data/gnoblin.toml.example` for the reference file.

## Migration and verification

From the checkout, migrate supported legacy settings with:

```sh
python3 scripts/migrate-config.py
```

The command preserves the original `.conf` and refuses to overwrite an
existing `.toml`. It copies supported `[shell]` settings and protocol booleans,
and reports ignored legacy sections. Old `[startup]` commands were not used
by the current GNOME-based shell and are deliberately not activated during
migration. Add the desired commands as named `[[autostart]]` entries.

Saves apply after a 150 ms debounce. `gnoblinctl reload-config` reads the file
immediately and reports validation errors. `gnoblinctl reload` and `Alt+F2`,
`r` also reread it, alongside the existing theme/extensions/scripts reload.

```sh
./scripts/test-config.sh
GNOBLIN_CONFIG='' GNOBLIN_PREFIX="$PWD/install" \
  GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-live-shell-config.py" \
  ./scripts/run-gnome-shell.sh
# Requires Quickshell and GTK 4:
GNOBLIN_CONFIG='' GNOBLIN_PREFIX="$PWD/install" \
  GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-minimize-target.py" \
  ./scripts/run-gnome-shell.sh
```

## `org.gnoblin.shell` GSettings

One key: `disabled-features` (`as`, default `[]`). A feature is enabled
unless its id is in this list. Read/write it directly with `gsettings`, or
— the normal path — through `org.gnoblin.Shell`'s
`ListFeatures`/`GetFeature`/`SetFeature` (which also emits
`FeatureChanged`), via `gnoblinctl`.

### Feature ids

| id | Gates |
|---|---|
| `osd` | On-screen display popups — master switch for all OSD types below |
| `osd-volume` | Volume OSD popup |
| `osd-microphone` | Microphone OSD popup |
| `osd-brightness` | Screen-brightness OSD popup |
| `osd-keyboard-brightness` | Keyboard-brightness OSD popup |
| `screenshot` | The built-in screenshot/screencast UI |
| `notifications` | Owning `org.freedesktop.Notifications` (in Gnoblin mode, disable to let an external daemon own it; stock modes always retain the GNOME service) |

Source of truth: the `FEATURES`/`OSD_TYPES` constants in
`src/gnome-shell-overlay/js/ui/components/gnoblinControl.js`.

## `gnoblinctl`

A thin `gdbus` wrapper over `org.gnoblin.Shell`, installed to
`$PREFIX/bin/gnoblinctl` by `just dev-session`. Source:
`src/tools/gnoblinctl`.

```
gnoblinctl ping                     health check (-> pong)
gnoblinctl version                  shell + protocol version
gnoblinctl reload                   Wayland soft-reload (config + theme + extensions + scripts)

gnoblinctl reload-config            read gnoblin.toml immediately

gnoblinctl features                 list feature toggles + state
gnoblinctl feature <id>             show one feature's state
gnoblinctl enable  <id>             turn a subsystem ON  (SetFeature true)
gnoblinctl disable <id>             turn a subsystem OFF (SetFeature false)

gnoblinctl extensions               list extensions + state
gnoblinctl reload-ext <uuid>        hot-reload one extension's code

gnoblinctl scripts                  list loaded user scripts
gnoblinctl reload-scripts           reload ~/.config/gnoblin/scripts/*.js

gnoblinctl portal-grants            list persistent Screen Cast and Remote Desktop grants
gnoblinctl revoke-grant <kind> <id> revoke one portal-scoped grant
```

`reload` is also bound to `Alt+F2` `r`: a Wayland-safe soft reload that
re-applies the shell theme/CSS and re-enables extensions in-process, without
tearing down Mutter — your windows and your chrome survive.

## Portal grants

Screen Cast and Remote Desktop grants are files under
`$XDG_DATA_HOME/gnoblin/portal-grants/<kind>/`, which defaults to
`~/.local/share/gnoblin/portal-grants/<kind>/`. Each filename is an opaque
SHA-256 digest; the record contains the verified namespaced requester identity
(`app-id:<id>` for a portal app or `host-exe:<canonical-path>` for an
unsandboxed process) and the exact approved capabilities.

Use `gnoblinctl portal-grants` to obtain each record's `<kind>` and `<id>`, then
`gnoblinctl revoke-grant <kind> <id>` to remove it. The gnoblin Settings panel
uses the same typed D-Bus methods. See
[Installation: unattended screensharing](installation.md#unattended-screensharing-xdg-desktop-portal-gnome)
for how a grant is created.

## Session mode (not user-configurable)

The Gnoblin session mode at `src/data/session/modes/gnoblin.json` removes the
overview, dash, app grid and panel contents. A small GNOME Shell patch makes the
native panel non-interactive and non-strutting only when the immutable primary
session mode is `gnoblin`; stock GNOME keeps its upstream panel. This is not a
runtime setting. Changing the chrome policy means editing the session data or
patch and rebuilding, not adding a `gnoblin.toml` key.

The session configures Mutter's `overlay-key` as `Super`. Mutter emits its
release event only when no other input is used. `gnoblinControl` forwards that
event to external chrome, while `hasOverview: false` keeps the native overview
disabled.


## Layer animations and window effects

These options require the rebuilt Mutter and GNOME Shell once. After the next
login, edits reload automatically. Normal application animations are unchanged.

```toml
[shell]
layer-animation = "slide" # slide, fade, none
layer-duration = 220      # milliseconds, 0 disables animation
layer-easing = "ease-out-cubic" # also ease-out-quad, ease-in-out-cubic, linear

[[window-rules]]
match.type = "layer"
blur = 24                # radius, 0 disables blur; maximum 100
opacity = 0.96           # 0 to 1, multiplied with client opacity

[[window-rules]]
match.layer = "^gnoblin-dock-tooltip$"
animation = "fade"

[[window-rules]]
match.app-id = "^org\\.gnome\\.Ptyxis$"
match.focused = false
opacity = 0.92
```

A rule can match `app-id`, `title`, `layer`, `type`, and `focused`. String
matchers are regular expressions. `type` accepts `layer` or `window`.
All matchers in a rule must match. Later rules override each named effect;
unmentioned effects retain the earlier matching value. Removing a rule restores
the previous client opacity and removes its blur effect. Invalid rules retain
the entire last valid configuration.

Layer surfaces slide from their committed anchor edges at full opacity, like a
notification. A top-anchored surface slides down; bottom, left and right edges
use the corresponding inward direction. A corner uses both axes. Opposing edges
cancel translation on that axis. Full-screen input overlays fade. Surface
destruction uses the reverse movement. GNOME's animation-disable setting also
applies. Per-rule `animation` applies to layer surfaces only.

Gnoblin applies these transforms to the compositor actor; Quickshell and other
layer-shell clients need no animation code. Animations do not resize the client
or interpolate its exclusive zone. A surface must map to trigger entry;
changing content inside an already visible surface does not remap it. Clients
that animate individual cards in a persistent surface can opt out by namespace
with `animation = "none"`.

Background blur is rendered in the compositor and masked by client alpha. Fully
transparent parts of a layer surface remain unchanged, including the area around
floating docks and search panels. Applications must draw a translucent background
to reveal the blur. Large blur regions cost more GPU time; use namespace rules to
reduce the radius or disable it where unnecessary.

Run `just --set prefix "$PWD/install" gnome-layer-animation-verify` to check
configuration reload, all four edges, corners, fade, disabled motion, repeated
mapping, intermediate compositor frames and animation cleanup.
Run `just --set prefix "$PWD/install" gnome-window-effects-verify` for pixel checks of masked blur and live
rule removal. Both use a private headless session and require Quickshell; the
pixel test also requires `grim` and Python Pillow.
