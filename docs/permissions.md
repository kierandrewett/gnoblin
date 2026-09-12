# Portal permissions

Gnoblin has one permission policy in `init.lua`. The shell validates and
reloads it. Portal handlers ask the existing `org.gnoblin.Shell` control service
for a decision before they restore a session or show a consent dialog.

## Levels

| Level | Behaviour |
| --- | --- |
| `default` | Use the normal portal flow, including standard restore tokens. This is the initial fallback. |
| `ask` | Require the portal's consent or selection dialog. Capture restore tokens cannot skip consent. |
| `allow` | Approve the matching request within the rule's configured capabilities. |
| `deny` | Reject the request without a dialog, including requests with restore tokens. |

The global fallback accepts `default`, `ask`, or `deny`. Automatic approval
requires an explicit app rule. A matching `deny` rule always wins. Otherwise,
the last matching rule wins as a whole. Rules do not combine device or monitor
selections. A fallback of `deny` with explicit `allow` rules forms a whitelist.
A fallback of `default` with `deny` rules forms a blocklist.

## Example: RustDesk

```lua
g.set({permissions = {
    default = "default",
    rules = {
        {name = "rustdesk", match = "^host-exe:/usr/bin/rustdesk$",
         capabilities = {"screen-cast", "remote-desktop"}, level = "allow",
         monitors = {"primary"}, devices = {"keyboard", "pointer"}, clipboard = true},
        {name = "blocked-capture", match = "^app-id:com\\.example\\.Untrusted$",
         capabilities = {"screen-cast", "remote-desktop", "screenshot"}, level = "deny"},
    },
}})
```

Set the identity to the one used by your installation. A Flatpak installation
can use `^app-id:com\.rustdesk\.RustDesk$`. A native portal client may also have
an app ID supplied by the portal frontend. An AppImage can have a changing
executable path; avoid granting an entire temporary directory.

RustDesk uses the ScreenCast portal for Wayland capture. Some RustDesk input
paths use `uinput`, which is outside this policy. See the
[RustDesk Linux documentation](https://rustdesk.com/docs/en/client/linux/).

## Identity matching

`match` is a case-sensitive JavaScript regular expression against one complete,
namespaced identity:

- `app-id:<id>`: the app ID supplied by the trusted desktop portal frontend.
- `host-exe:<absolute-path>`: the executable of the original D-Bus caller when
  the frontend has no app ID.

Use `^` and `$` for an exact match. Window titles and client-provided Wayland
`app_id` strings are not permission identities. The backend checks that the
caller owns `org.freedesktop.portal.Desktop`, resolves the original caller from
the portal request handle, and checks its Unix user before using an identity.
An unverified identity cannot receive automatic approval. It is denied when
the fallback is `deny`; otherwise it must use a dialog.

These are user-session preferences, not isolation between hostile processes
running as the same Unix user. Host applications can have frontend-assigned app
IDs. Use executable matching where the frontend provides an empty app ID.

## Supported capabilities

| Capability | Automatic approval |
| --- | --- |
| `screen-cast` | Capture the configured monitors. |
| `remote-desktop` | Grant only the configured input devices, clipboard access and monitors. |
| `input-capture` | Approve the requested supported input-capture capabilities. |
| `screenshot` | Skip screenshot consent; an explicitly requested interactive capture still needs a selection. |
| `access` | Approve a generic Access dialog; dialogs with choices still need user input. |

`monitors` contains `primary` or exact connector names such as `DP-1`. Multiple
entries require a client that requests multiple streams. A missing monitor,
missing selection, or incompatible source type causes automatic capture to
fail. Rules do not guess a window from its title or silently choose another
monitor. `allow` for screen capture therefore needs `monitors` for unattended use.

`devices` contains `keyboard`, `pointer`, and/or `touchscreen`. It defaults to
an empty list. `clipboard` defaults to `false`. A remote request that exceeds
these limits fails. A ScreenCast deny also blocks video requested through
RemoteDesktop. A ScreenCast `ask` requires the combined remote session to show
its dialog. A separate ScreenCast `allow` supplies the monitor selection for
automatically approved combined sessions.

The Access backend does not receive a reliable capability identifier for
separate camera, microphone, or location requests. The `access` rule is therefore
a generic gate, not three separate permissions. Frontend-cached permissions
that do not call a backend cannot be overridden here. File chooser, USB,
background, global shortcuts, direct Mutter D-Bus calls, raw Wayland protocols,
and kernel device access are outside these five gates. Startup `protocols`
switches continue to control protocol availability.

## Control commands

```sh
gnoblinctl permissions list
gnoblinctl permissions check screen-cast host-exe:/usr/bin/rustdesk
```

Edit Lua directly, then run `gnoblinctl config reload`. `check` reports the
level, matching rule and selection limits. It evaluates a supplied identity;
it does not authenticate a process or check current monitor availability.
`list` includes supported capabilities, levels and the active configuration path.

Invalid configuration edits retain the last valid policy. An invalid
configuration at initial startup, or an unavailable policy service in a Gnoblin
session, causes backend requests to be denied.

Changes apply to new permission requests. They do not disconnect an active
remote session. A dialog that is already open belongs to its original request.
## Migration and verification

The old custom `portal-grants` files no longer grant access, and the additional
"remember forever" checkboxes are removed. Standard portal restore tokens remain
available under `default`. Existing custom files are retained for inspection
and removal with `gnoblinctl grant list` and `gnoblinctl grant revoke`.
There is no automatic conversion into broader regex rules.

Both the shell overlay and patched portal backend must be built and installed.
The portal checks policy only in the Gnoblin session; stock GNOME keeps its
normal portal behaviour. For a private test against the checkout:

```sh
node tests/permissions.test.mjs
meson compile -C build/xdg-desktop-portal-gnome
GNOBLIN_PREFIX="$PWD/install" \
G_RESOURCE_OVERLAYS="/org/gnome/shell=$PWD/src/gnome-shell-overlay/js" \
GNOBLIN_TEST_DISABLE_NOTIFICATIONS=1 \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-permissions-live.py" \
bash scripts/run-gnome-shell.sh
```

The live test uses real backend calls on an isolated bus. It checks unattended
capture and input, restore-token denial, forced consent and cancellation,
identity verification, rule precedence, configuration reloads.
