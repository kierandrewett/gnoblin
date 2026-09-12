# Configuration

See [Window effects](window-effects.md) for blur rules, custom GLSL shaders,
shader uniforms and file hot reload, including a Bingux configuration example.

Gnoblin reads `~/.config/gnoblin/gnoblin.toml` and watches it for live changes.
`$XDG_CONFIG_HOME` changes the base directory. `$GNOBLIN_CONFIG` selects an
explicit file. A `.conf` override uses the legacy INI reader; other filenames
use TOML. Without an override, TOML takes priority over `gnoblin.conf`.
The watcher detects creation and atomic replacement of either default file.

## Configuration fragments

TOML configurations can load package or user fragments without copying them
into the main file:

```toml
include = ["/usr/share/bingux/gnoblin.toml"]
```

`source` is accepted as an alias for `include`, matching the spelling used by
Hyprland. Paths may be absolute, relative to the file containing the directive,
or begin with `~/`. A fragment can include more fragments, but cycles and
missing files are rejected. Fragments are merged in declaration order, then
the containing file is applied last: tables merge recursively, while
`window-rules`, `shortcuts`, `autostart`, and permission `rules` append. Scalar
settings and ordinary arrays in the containing file override included values.

The main file and every loaded fragment are watched. Saving any of them runs the
same parse-and-validate transaction as `gnoblinctl reload-config`; invalid edits
keep the last valid configuration and report the file and expected value.
`include` and `source` cannot be used together in one file.

Protocol advertisement remains a compositor-startup decision. An included
`[protocols]` change is validated immediately but needs a new session before
the Wayland global changes.

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

### Layer-shell keyboard focus

```toml
[layer-shell]
preserve-active-window = true
```

`preserve-active-window` is a boolean, defaults to `true`, and applies to every
layer-shell client regardless of its namespace or toolkit. Omit it to use the
default. Use literal `true` or `false`, not quoted strings; unknown keys in this
section and values of another type are rejected.

Layer-shell surfaces include desktop launchers, panels, and shell context menus.
Keyboard input and application activation are separate: a search box needs your
typing, but need not make the application underneath look inactive or change
which application is selected.

With `true`, a surface requesting **exclusive** keyboard input receives typing
while the existing application remains active. The application does not receive
a second copy of those keys. This is the default for Search, shell menus, and
third-party launchers alike; no special surface name is required. When the layer
releases its keyboard request, unmaps, or disconnects, its input handler is
removed so normal keyboard routing can resume.

With `false`, exclusive layers use normal layer-window activation instead:
opening one can deactivate the application beneath it. Choose this for a client
that depends on that activation behavior, or to restore the previous behavior
of ordinary exclusive layers. This also disables the old popup-specific
exception; the policy is uniform across clients.

The client's keyboard request still matters:

| Request | Effect of this setting |
| --- | --- |
| None | No keyboard input; the surface cannot take keyboard focus. |
| Exclusive | `true` preserves the active application; `false` activates the layer window. |
| On demand | Unchanged: normal user-directed focus, such as click-to-focus. |

This setting does not make passive bars interactive, prevent an on-demand panel
from taking focus when clicked, or change native application menus that use
`xdg-popup` rather than layer-shell. Session locking uses separate mechanisms.
Multiple keyboard-interactive layers still need compositor arbitration; this
option does not broadcast input to all of them.

**Requires a new compositor session:** save the file, then log out and back in.
The value is read when layer-shell starts. `gnoblinctl reload-config` and closing
and reopening a menu do not change it for the running session.

### Window drag boundary

```toml
[window-management]
constrain-drag-to-work-area = true
```

When enabled, the compositor keeps a dragged window's frame below the current
monitor work-area top. Layer-shell panels contribute their exclusive zones to
that work area, so the setting applies to any Wayland panel and does not name
or depend on Bingux. The default is `true`; set it to `false` when windows are
intentionally allowed to overlap reserved panel space. The value is re-read at
the start of each drag.

`input-source-switcher` controls GNOME's native keyboard-layout popup. It is
off by default in Gnoblin so external chrome can present the active layout and
its own selector; it does not disable input sources or programmatic layout
selection. Set it to `true` to restore GNOME's modifier-based switcher.

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

## Keyboard shortcuts

Custom commands and built-in bindings can be configured in TOML, without using
GNOME Settings. Once the updated shell is installed, changes reload on save.

```toml
[[shortcuts]]
name = "capture"
binding = "<Alt>s"
command = ["qs", "ipc", "--any-display", "-c", "bingux", "call", "capture", "open"]

[[shortcuts]]
name = "terminal"
binding = "<Super>Return"
command = ["ptyxis", "--new-window"]

[keybindings.wm]
close = ["<Super>q"]
```

Each command shortcut needs a unique name (letters, numbers, `_`, `-`), a GTK-style
accelerator, and a nonempty argument array. Arguments retain spaces, quotes and
literal `$HOME`; no implicit shell expansion occurs. Use an explicit shell for
pipes or substitutions. Adding, changing or removing an entry updates the
shortcut registration; removing it does not stop a previously launched process.
Identical saves do not re-register shortcuts. There is a limit of 256 commands.

Built-in groups are `shell`, `wm`, `mutter`, `wayland`, and `media`, mapping to
the corresponding GNOME keybinding schemas. Values are accelerator arrays;
`[]` disables that action's binding. Unknown actions, invalid keys, malformed
commands and duplicate configured accelerators reject the edit before any
shortcut settings change. Policy-locked settings produce an error. Overrides
are persistent, like explicit `[shell]` feature settings: omitting a built-in
override stops managing it but does not restore its former value.

Use `binding = "Super"` for a bare Super press and release. This uses Mutter's
modifier-only release event, so Super shortcuts and Super-drag do not trigger it.
For example:

```toml
[[shortcuts]]
name = "search"
binding = "Super"
capture-input = true
command = ["binguxctl", "search", "toggle"]
```

`capture-input = true` buffers keyboard events in the compositor before the
command starts. The popup sends `shortcut-input` messages over the compositor
bridge: `prepared` releases the temporary grab once its surface is mapped, `ready`
replays buffered events after its text input has keyboard focus, and `closed`
clears the handoff. Events retain their native keycodes and modifiers. A failed
handoff releases the grab after three seconds and discards queued input.
Bare Super still activates on release so held Super shortcuts continue to work.

Gnoblin registers command accelerators directly with Mutter and ignores keyboard
auto-repeat: holding a shortcut does not repeatedly launch or toggle its command.
Commands are enabled in the normal desktop, overview, shell menus and modal
dialogs, not the lock/login screen.
Removing a command releases its grab; invalid/conflicting edits retain the
previous working command registrations. Old config-owned media-key entries are
removed during migration; unrelated user shortcuts remain untouched. An existing shortcut outside the file
can still claim an accelerator: disable/rebind that action explicitly rather than
silently stealing it. Registration failures appear in the Gnoblin config log.

Do not define the same command shortcut in both TOML and another settings manager.
Test configuration ownership without touching real shortcuts with
`GSETTINGS_BACKEND=memory gjs -m tests/shortcuts-test.js`.

## Autostart commands

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

The `[shell]` table accepts booleans for `notifications` and
`input-source-switcher`:

```toml
[shell]
notifications = true
input-source-switcher = false
```

Explicit entries update the existing persistent GSettings state. Omitted
entries retain their current state. Removing an entry stops managing it from
the file; it does not reset its persistent value. CLI changes remain possible,
but the next reload reapplies explicit file entries. Validation completes
before feature settings or autostart commands are applied.

Gnoblin never shows GNOME OSD popups or the GNOME screenshot UI. Legacy
`osd`, `osd-*`, and `screenshot` entries with `false` remain accepted for
configuration compatibility. A `true` value is stale configuration and is
ignored. `gnoblinctl enable` rejects these removed features.

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
immediately and reports validation errors. `gnoblinctl reload` also rereads it,
alongside the theme and user-script reload.

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

One key: `disabled-features` (`as`). Its default disables `notifications`,
`input-source-switcher`, and the removed `osd*` and `screenshot` features.
The two remaining controls are enabled when their ids are absent from this
list. Removed UI stays unavailable regardless of stored preferences.
Read/write it directly with `gsettings`, or
— the normal path — through `org.gnoblin.Shell`'s
`ListFeatures`/`GetFeature`/`SetFeature` (which also emits
`FeatureChanged`), via `gnoblinctl`.

### Feature ids

| id | Gates |
|---|---|
| `notifications` | Own `org.freedesktop.Notifications`; disabled by default so an external daemon can own it |
| `input-source-switcher` | GNOME's native keyboard-layout popup; source state and switching remain available when disabled |

Source of truth: the `FEATURES` constant in
`src/gnome-shell-overlay/js/ui/components/gnoblinControl.js`.

## `gnoblinctl`

A thin `gdbus` wrapper over `org.gnoblin.Shell`, installed to
`$PREFIX/bin/gnoblinctl` by `just dev-session`. Source:
`src/tools/gnoblinctl`.

```
gnoblinctl ping                     health check (-> pong)
gnoblinctl version                  shell + protocol version
gnoblinctl reload                   Wayland soft-reload (config + theme + user scripts)

gnoblinctl reload-config            read gnoblin.toml immediately
gnoblinctl load-config /path/file    add an idempotent include and reload it
gnoblinctl unload-config /path/file  remove an include and reload it

gnoblinctl features                 list feature toggles + state
gnoblinctl feature <id>             show one feature's state
gnoblinctl enable  <id>             turn a subsystem ON  (SetFeature true)
gnoblinctl disable <id>             turn a subsystem OFF (SetFeature false)

gnoblinctl scripts                  list loaded user scripts
gnoblinctl reload-scripts           reload ~/.config/gnoblin/scripts/*.js

gnoblinctl portal-grants            list persistent Screen Cast and Remote Desktop grants
gnoblinctl revoke-grant <kind> <id> revoke one portal-scoped grant
```

`gnoblinctl reload` re-applies the shell theme/CSS and reloads user scripts in-process, without
tearing down Mutter — your windows and your chrome survive.

## Portal permissions

Use `[permissions]` and `[[permissions.rules]]` in `gnoblin.toml` to set
`default`, `ask`, `allow`, or `deny` decisions for apps matched by an identity
regex. `gnoblinctl permissions` inspects and edits the same policy.
See [Portal permissions](permissions.md) for the RustDesk example, supported
capabilities, rule precedence and migration from custom grant files.

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

Layer surfaces keep their assigned display while an entrance or exit animation
crosses another display's stage view. The compositor reports that assigned
output to Wayland clients throughout the animation. This prevents a panel's
reported screen from changing briefly and moving related docks or popups to a
neighbouring display.

The Bingux companion repository supplies the native two-display editor check.
It uses horizontal and vertical arrangements, different scale factors, and both
primary and secondary outputs. Run it from the Gnoblin checkout:

```sh
GNOBLIN_PREFIX="$PWD/install" MONITOR=1920x1200 EXTRA_MONITOR=1920x1200 \
GNOBLIN_TEST_DISABLE_NOTIFICATIONS=1 \
GNOBLIN_TEST_GSETTINGS_BACKEND=keyfile \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/../bingux/tests/customise-monitors-live.py" \
bash scripts/run-gnome-shell.sh
```

Set `QS_TEST_BIN` if Quickshell is not available as `qs` on `PATH`. Set
`BINGUX_REDUCED_MOTION=1` to repeat the editor checks without client animations.
The display configuration and editor changes stay inside the private session.

Retained `gnoblin-shell-popup` surfaces rise above existing overlay panels when
they request menu keyboard input again. This restores their visible and clickable
area after the sidebar has been raised, without changing the active application.
The Bingux `tests/desktop-layout-live.py --case control-connectivity` check covers
this path with native clicks after moving Bluetooth into the dock, bar and sidebar.

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

Layer-shell resize requests keep the current buffer anchored until replacement
content arrives. This prevents tooltip jumps while a client changes its size
and centring margins. `just gnome-layer-animation-verify` includes pixel checks
for bottom, right, and top-anchored resize handshakes.
