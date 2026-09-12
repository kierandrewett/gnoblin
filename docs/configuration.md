# Configuration

See [Window effects](window-effects.md) for blur rules, custom GLSL shaders,
shader uniforms and file hot reload, including a Bingux configuration example.

Gnoblin reads `~/.config/gnoblin/init.lua`. `$XDG_CONFIG_HOME` changes the base
directory. `$GNOBLIN_CONFIG` selects one explicit Lua file. Gnoblin has one
configuration format: Lua.

## Lua configuration

Start with a small init file:

```lua
local g = require("gnoblin")

-- A component owns its defaults. Load it before your overrides.
g.load("/usr/share/gnoblin/conf.d/*.lua")
g.load("conf.d/**/*.lua")

g.set({
    shell = {
        ["minimize-animation"] = "zoom",
        ["minimize-duration"] = 150,
    },
    shortcuts = {
        {name = "terminal", binding = "<Super>Return", command = {"ptyxis"}},
    },
})
```

The model is a user init file and ordinary Lua modules. Variables, functions,
conditions, and loops can build the settings. There is no package manager or
second settings schema. Lua uses `{...}` for tables and arrays, and `["hyphenated-key"]`
for keys that contain a hyphen.

The configuration API is small:

| API | Behaviour |
| --- | --- |
| `g.set(table)` | Merge a table into the configuration. Later scalar values win. |
| `g.config` | Read or change the current configuration table directly. |
| `g.load(path)` | Load a Lua file, or matching Lua files from a glob, at this point. |
| `require("module")` | Run a Lua module once per reload and return its result. |

`g.set` and `g.load` merge tables recursively. Rule arrays (`window-rules`,
`shortcuts`, `autostart`, and permission `rules`) append. Ordinary arrays replace
the previous value. Direct assignment replaces a value and can remove imported
rules, for example `g.config.shortcuts = {}`. Load order is explicit: a package
loaded after an override can replace that override.

A component file can return a plain table:

```lua
-- ~/.config/gnoblin/appearance.lua
return {
    shell = {["minimize-duration"] = 120},
    ["window-rules"] = {
        {match = {type = "layer"}, blur = 24},
    },
}
```

Load that file with `g.load("appearance.lua")`. Relative paths resolve from the
file that calls `g.load`. Absolute paths and `~/` paths also work. Use
`require("appearance")` for a module that calls `g.set` itself, or use
`g.set(require("appearance"))` for a module that returns a settings table.
`require` caches its result for the current reload. A new reload starts with a
fresh Lua state, so previous rules do not accumulate.

Use one main file to include the settings you need, as with nginx:

```lua
local g = require("gnoblin")
g.load("/usr/share/gnoblin/conf.d/*.lua") -- installed component settings
g.load("conf.d/**/*.lua")              -- your settings, including subdirectories
```

A glob loads matching files in bytewise path order. Prefix names with numbers
such as `10-appearance.lua` and `90-local.lua` when order matters. `*`, `?`, and
`[abc]` match within a path segment; `**` includes subdirectories. Hidden files
and hidden directories require an explicit dot in the pattern. Recursive `**`
does not follow directory symlinks. An unmatched glob is allowed. A missing
exact file is an error. All searched directories are watched, so adding or
removing a matching file also reloads the configuration.

Bingux installs `share/gnoblin/conf.d/bingux.lua` under its installation prefix.
Its installer prints the corresponding include pattern. The main init file owns
the include; each component owns its file. The CLI stays small:

```sh
gnoblinctl config path
gnoblinctl config reload
```

All loaded files are watched. Syntax errors, missing modules, and invalid
settings retain the last working configuration. The error reports through the
shell log and `gnoblinctl config reload`. Correcting the file retries the load.
Lua computes configuration data. The base, table, string, math, and UTF-8
functions are available. External I/O, process execution, and native Lua modules
are not exposed. Start processes through named `autostart` or `shortcuts` entries
so repeated reloads retain their existing lifecycle rules. Evaluation has limits
of 8 MiB of Lua memory, one million Lua instructions, and 32 nested files.

Protocol advertisement remains a compositor-startup operation. The updated
build requires installation and one new login. Supported settings reload on
save after that login; protocol changes still require a new session.

## Window behaviour

```lua
g.set({shell = {
    ["window-switcher"] = false,
    ["minimize-animation"] = "zoom",
    ["minimize-duration"] = 200,
    -- Optional fallback, in logical desktop coordinates:
    -- ["minimize-target"] = {960, 1040},
}})
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

```lua
g.set({["layer-shell"] = { ["preserve-active-window"] = true }})
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
The value is read when layer-shell starts. `gnoblinctl config reload` and closing
and reopening a menu do not change it for the running session.

### Window drag boundary

```lua
g.set({["window-management"] = { ["constrain-drag-to-work-area"] = true }})
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

Custom commands and built-in bindings can be configured in the file, without using
GNOME Settings. Once the updated shell is installed, changes reload on save.

```lua
g.set({
shortcuts = {
    {name = "capture", binding = "<Alt>s", command = {"qs", "ipc", "--any-display", "-c", "bingux", "call", "capture", "open"}},
    {name = "terminal", binding = "<Super>Return", command = {"ptyxis", "--new-window"}},
},
keybindings = {wm = {close = {"<Super>q"}}},
})
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
are persistent, like explicit `shell` feature settings: omitting a built-in
override stops managing it but does not restore its former value.

Use `binding = "Super"` in a shortcut table for a bare Super press and release. This uses Mutter's
modifier-only release event, so Super shortcuts and Super-drag do not trigger it.
For example:

```lua
g.set({shortcuts = {{
    name = "search", binding = "Super", ["capture-input"] = true,
    command = {"binguxctl", "search", "toggle"},
}}})
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

Do not define the same command shortcut in both Lua and another settings manager.
Test configuration ownership without touching real shortcuts with
`GSETTINGS_BACKEND=memory gjs -m tests/shortcuts-test.js`.

## Autostart commands

```lua
g.set({autostart = {
    {name = "bingux", command = {"qs", "-c", "bingux"}},
    {name = "clipboard", command = {"wl-paste", "--watch", "cliphist", "store"}},
}})
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

The `shell` table accepts booleans for `notifications` and
`input-source-switcher`:

```lua
g.set({shell = {notifications = true, ["input-source-switcher"] = false}})
```

Explicit entries update the existing persistent GSettings state. Omitted
entries retain their current state. Removing an entry stops managing it from
the file; it does not reset its persistent value. CLI changes remain possible,
but the next reload reapplies explicit file entries. Validation completes
before feature settings or autostart commands are applied.

Gnoblin never shows GNOME OSD popups or the GNOME screenshot UI. Legacy
`osd`, `osd-*`, and `screenshot` entries with `false` remain accepted for
configuration compatibility. A `true` value is stale configuration and is
ignored. `gnoblinctl feature enable` rejects these removed features.

## Protocol settings

```lua
g.set({protocols = {
    ["wlr-layer-shell"] = true, ["wlr-screencopy"] = true,
    ["ext-idle-notify"] = true, ["ext-foreign-toplevel-list"] = true,
    ["wlr-foreign-toplevel-management"] = true, ["wlr-gamma-control"] = true,
    ["wlr-output-power-management"] = true, ["ext-data-control"] = true,
}})
```

These settings require a new compositor session. Reload does not change
registered Wayland globals. All implemented protocols default on in Gnoblin;
stock sessions do not expose these Gnoblin globals. See
`src/data/init.lua.example` for the reference file.

## Verification

Saves apply after a 150 ms debounce. `gnoblinctl config reload` reads the file
immediately and reports validation errors. `gnoblinctl reload` also rereads it,
alongside the theme and user-script reload.

```sh
./tests/test-config.sh
GNOBLIN_CONFIG='' GNOBLIN_PREFIX="$PWD/install" \
  GNOBLIN_TEST_DBUS_CLIENT="$PWD/tests/test-live-shell-config.py" \
  ./scripts/run-gnome-shell.sh
# Requires Quickshell and GTK 4:
GNOBLIN_CONFIG='' GNOBLIN_PREFIX="$PWD/install" \
  GNOBLIN_TEST_DBUS_CLIENT="$PWD/tests/test-minimize-target.py" \
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

gnoblinctl config path              show the active init file
gnoblinctl config reload            reload the active configuration

gnoblinctl feature list             list feature toggles + state
gnoblinctl feature show <id>        show one feature's state
gnoblinctl feature enable <id>      turn a subsystem ON
gnoblinctl feature disable <id>     turn a subsystem OFF

gnoblinctl script list              list loaded user scripts
gnoblinctl script reload            reload ~/.config/gnoblin/scripts/*.js

gnoblinctl grant list               list persistent Screen Cast and Remote Desktop grants
gnoblinctl grant revoke <kind> <id> revoke one portal-scoped grant
```

`gnoblinctl reload` re-applies the shell theme/CSS and reloads user scripts in-process, without
tearing down Mutter — your windows and your chrome survive.

## Portal permissions

Use `permissions` and `permissions.rules` tables in `init.lua` to set
`default`, `ask`, `allow`, or `deny` decisions for apps matched by an identity
regex. Edit the `permissions` table in the Lua file, then reload. `gnoblinctl
permissions list` and `gnoblinctl permissions check` inspect the effective policy.
See [Portal permissions](permissions.md) for the RustDesk example, supported
capabilities, rule precedence and migration from custom grant files.

## Session mode (not user-configurable)

The Gnoblin session mode at `src/data/session/modes/gnoblin.json` removes the
overview, dash, app grid and panel contents. A small GNOME Shell patch makes the
native panel non-interactive and non-strutting only when the immutable primary
session mode is `gnoblin`; stock GNOME keeps its upstream panel. This is not a
runtime setting. Changing the chrome policy means editing the session data or
patch and rebuilding, not adding an `init.lua` key.

The session configures Mutter's `overlay-key` as `Super`. Mutter emits its
release event only when no other input is used. `gnoblinControl` forwards that
event to external chrome, while `hasOverview: false` keeps the native overview
disabled.


## Layer animations and window effects

These options require the rebuilt Mutter and GNOME Shell once. After the next
login, edits reload automatically. Normal application animations are unchanged.

```lua
g.set({
shell = {["layer-animation"] = "slide", ["layer-duration"] = 220,
         ["layer-easing"] = "ease-out-cubic"},
["window-rules"] = {
    {match = {type = "layer"}, blur = 24, opacity = 0.96},
    {match = {layer = "^gnoblin-dock-tooltip$"}, animation = "fade"},
    {match = {["app-id"] = "^org\\.gnome\\.Ptyxis$", focused = false}, opacity = 0.92},
},
})
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
