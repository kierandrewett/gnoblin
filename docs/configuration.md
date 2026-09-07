# Configuration

Gnoblin reads `gnoblin.conf` at startup and watches it for live shell changes.
The `[shell]` section controls native window behaviour. The `[protocols]`
section controls Wayland protocol registration and still requires a new session.
Existing subsystem toggles use `org.gnoblin.shell` GSettings through `gnoblinctl`.

## `gnoblin.conf`

Location: `$GNOBLIN_CONFIG`, else `$XDG_CONFIG_HOME/gnoblin/gnoblin.conf`
(`~/.config/gnoblin/gnoblin.conf`). Gnoblin registers each implemented
protocol by default; add an entry only when you need to turn one off. Stock
session modes do not register these globals, regardless of this file.

```ini
[protocols]
# Each key gates one implemented protocol overlay in the Gnoblin session.
# true (default) = advertised; false = not registered, so clients cannot bind it.
wlr-layer-shell                 = true   # layer-shell chrome (bars/docks/etc.); the
                                         #   whole point — leave on unless debugging
wlr-screencopy                  = true   # grim-style screen capture
ext-idle-notify                 = true   # idle notifications (swayidle etc.)
ext-foreign-toplevel-list       = true   # window list for taskbars
wlr-foreign-toplevel-management = true   # window control for taskbars
wlr-gamma-control               = true   # night-light via wlsunset
wlr-output-power-management     = true   # DPMS via wlr clients
ext-data-control                = true   # clipboard managers (cliphist)
```

(Full reference copy: `src/data/gnoblin.conf.example`.)

### Live shell settings

Add this section to the same file:

```ini
[shell]
window-switcher = false
minimize-animation = fade
minimize-duration = 200
```

These are the defaults, including when the file does not exist.

- `window-switcher`: enable the native GNOME application, window, and group
  switchers and cyclers. Defaults to `false`. This suppresses their UI and
  switching action; it does not install an external switcher or free their
  existing keybindings. The display-mode switcher and accessibility switcher
  are separate and stay available.
- `minimize-animation`: `fade`, `none`, or `gnome`. `fade` keeps the window in
  place during minimise and restore. `gnome` retains the native icon-target
  animation, including its top-corner fallback when no icon position exists.
- `minimize-duration`: 0 to 5000 milliseconds, default 200. Reduced-motion
  settings still take precedence.

Saving applies changes after a 150 ms debounce, including editor saves that
replace the file with a rename. Invalid shell values retain the complete last
valid configuration and log a warning. Removing a key or deleting the file
restores defaults. Unknown keys in `[shell]` are errors. Other sections are
reserved for their existing readers and are not applied by the shell watcher.

Run `gnoblinctl reload-config` to read the file immediately. It returns a
failure if the file is invalid. `gnoblinctl reload` and `Alt+F2`, `r` also read
it. Neither operation re-registers Wayland protocols.

The updated shell must be installed and loaded by a new login once. Subsequent
configuration edits do not require logout. Stock GNOME sessions keep their
native switcher and animations.

Verify with `gjs -m tests/shell-config-test.js`. To test the built shell in an
isolated session:

```sh
GNOBLIN_CONFIG='' GNOBLIN_PREFIX="$PWD/install" \
  GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-live-shell-config.py" \
  ./scripts/run-gnome-shell.sh
```

### Grammar

The parser (`src/config/gnoblin-config.c`) is intentionally small:

- Lines are trimmed of leading/trailing space, tab, CR, LF.
- Empty lines and lines starting with `#` or `;` (after trimming) are
  comments.
- `[section]` lines start a new section; a missing closing `]` drops the
  line.
- `key = value` lines use the first `=`; empty keys are ignored. Repeated
  keys are kept in file order — scalar lookups return the last value.
- A value starting with a quote (`'` or `"`) runs to the matching quote;
  everything after is dropped, no escape processing.
- An unquoted value strips a `#` inline comment, but only when the `#` is
  introduced by whitespace and outside a quoted span (`;` is always data,
  since `spawn`/`bind`-style values can legitimately contain it).
- A missing file, or a key that isn't set, falls back to the caller's
  default — there's no separate "defaults" file layered underneath.

Verify a change with `just test-config`, or by hand:

```sh
gcc -fsyntax-only src/config/gnoblin-config.c -I src/config $(pkg-config --cflags glib-2.0)
cc tests/config-test.c src/config/gnoblin-config.c -I src/config \
    $(pkg-config --cflags --libs glib-2.0) \
    -o /tmp/gnoblin-config-test
/tmp/gnoblin-config-test
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

gnoblinctl reload-config            read gnoblin.conf immediately

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
patch and rebuilding, not adding a `gnoblin.conf` key.

The session configures Mutter's `overlay-key` as `Super`. Mutter emits its
release event only when no other input is used. `gnoblinControl` forwards that
event to external chrome, while `hasOverview: false` keeps the native overview
disabled.
