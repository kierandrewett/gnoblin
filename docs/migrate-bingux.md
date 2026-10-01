# Migrate Bingux to standalone Gnoblin

Bingux remains a Quickshell application. The standalone Gnoblin session does
not start GNOME Shell or its GJS bridge, so Bingux must use Gnoblin's public
compositor socket, `gnoblinctl`, Wayland protocols, and desktop portals.

Keep panel, dock, launcher, notification, and popup UI in Bingux. Use Lua for
Gnoblin policy and runtime behavior, not for drawing shell surfaces.

## Readiness

Bingux is partially compatible with the standalone compositor socket.
Workspace navigation, capture window enumeration, and shortcut registration
now use the versioned API. Window-switcher previews now use the native
thumbnail API.

A live devkit check confirmed that a modal held shortcut registers through
`ShortcutSession.qml`. Focus, snapping, privacy, text insertion, and OSD still
need migration before the whole shell works in a standalone session.

- `shell/bingux/ShortcutSession.qml` uses `shortcut.bind`, `shortcut.unbind`,
  `ping`, and versioned shortcut event subscriptions when connected to
  standalone Gnoblin. It retains the compatibility protocol for GNOME sessions.
  Its window activation, drag, privacy, text input, and private
  `bingux.*` operations still need standalone API replacements.
- `shell/bingux/WorkspaceState.qml` now uses the versioned `workspace.list` and
  `workspace.switch` API methods. It subscribes to workspace lifecycle events
  on `op: "windows"` and polls only when connected to an older API version.
- `shell/bingux/capture_backend.py` uses `window.list` for window metadata and
  keeps image capture on the ScreenCast portal.
- `shell/gnoblin/bingux-text-input.js` and `shell/bingux/EmojiPicker.qml` use
  GNOME Shell text-input state, clipboard fallback, and caret placement. Use
  the standalone text-target methods for focused Wayland text-input-v3 clients.
- `shell/bingux/osd-bridge.js` patches GNOME Shell's OSD manager and calls
  `org.gnoblin.Shell`. Subscribe to `gnoblin.osd.requested` and draw the OSD in
  Bingux.

The standalone session has no `org.gnome.Shell` service or GJS bridge. Keep any
GNOME-session compatibility path separate from the standalone path.

## Replace the legacy bridge calls

The compositor socket protocol used by the GNOME Shell compatibility bridge
accepts several short operations that the standalone Gnoblin socket does not.
Migrate those calls before running Bingux in a standalone session.

Replace these compatibility-bridge calls:

- **Shortcuts:** `ShortcutSession.qml` now negotiates the API from `hello`,
  registers connection-owned bindings with `shortcut.bind`, removes stale
  bindings with `shortcut.unbind`, checks liveness with `ping`, and subscribes
  to shortcut activation and held-session events. Bindings belong to the
  connection; register them again after reconnecting. Keep the compatibility
  path for GNOME sessions until the standalone path is fully migrated.
- **Windows:** Subscribe with `op: "windows"` for the initial snapshot and live
  window and workspace events. Use `window.list` for filtered socket reads or
  `windows.list` when using the newer Lua-backed snapshot method.
- **Focus:** Replace `activate` with `window.focus` using an XDG Activation token
  created from user input on the same socket connection. A window ID alone
  cannot take focus. A Quickshell surface can create the XDG token after the
  click and pass it to the compositor socket owned by that same process.
  A keyboard switcher can instead use the one-use `focus_context` from
  `gnoblin.shortcut.activated`. The token belongs to its receiving connection
  and authorizes only one request.
- **Previews:** Replace `preview` with the asynchronous `window.thumbnail` API.
  `ShortcutSession.qml` now requests thumbnails and routes the completion event
  back to the switcher. Requests fail while the session is locked, and a closed
  or non-drawable window can return an error.
- **Privacy:** Replace `privacy` with the `privacy.state` read and the
  `gnoblin.privacy.changed` event.
- **Snapping:** Replace `window-drag` and snap events with pointer-drag event
  subscriptions and `window.snap.offer` on the connection that received the
  drag token.
- **Text insertion:** Replace `bingux.input-anchor` and `bingux.type-text` with
  the standalone text-target methods. Pass the one-use focus context from a
  shortcut activation to `input.text_target`; it returns an opaque target.
  Send that target and the text to `input.insert_text` on the same connection.

    The focused Wayland surface must keep an active text-input-v3 session. X11
    and the old clipboard fallback are unsupported. Keep Emoji insertion
    unavailable or explain the failure when the compositor rejects a request.

- **Capture:** Window enumeration now uses `window.list`. Continue using the
  ScreenCast portal for screen capture and recording.

The standalone socket accepts these top-level operations:

- `api` for a versioned method call;
- `events` for event subscriptions;
- `windows` and `monitors` for snapshots and change events;
- `ping` to check the connection.

Negotiate the API version from the initial `hello` event and use the minimum
version documented for each method. Basic dynamic shortcut binding requires
API 1.11; held or modal bindings require API 1.22. Thumbnails require 1.23;
text targets and keyboard snapping require
1.28; privacy state requires 1.17; and OSD requests require 1.27.

`workspace.list` and `window.list` are Lua-backed from API 1.52 and 1.53. The
[compositor bridge reference](compositor-bridge.md) lists every version,
argument, response, event, and capability.

For one-off reads and actions, prefer `gnoblinctl`; it handles API negotiation
and socket requests. A long-running shell component should keep one connection
open for its subscriptions and connection-owned capabilities.

## Keep shell presentation in Bingux

The compatibility bridge also exposes `ui-state` and `ui-command` callbacks.
Those are shell-specific state and presentation, so Bingux should own them in
its QML state rather than replace them with new Gnoblin APIs.

The `org.gnoblin.LaunchFeedback` D-Bus service is available in the standalone
session. Bingux can keep using it to show the busy cursor while an app starts.
For OSDs, subscribe to `gnoblin.osd.requested` and draw the OSD in Bingux. The
event reports compositor state; it does not provide a popup or prescribe its
appearance. Keep OSD rendering independent of GNOME Shell monkey-patches and
`org.gnoblin.Shell`.

Use standard Wayland protocols for Bingux's own windows and the portal
interfaces for screen sharing, remote desktop, and other application requests.
Gnoblin's Lua API controls compositor and session behavior; it does not create
or render Bingux UI.

## Migration order

1. Keep a single persistent socket client and reconnect with backoff. After a
   reconnect, read fresh snapshots, restore subscriptions, and re-register
   every shortcut owned by that connection.
2. Replace the compatibility operations with the calls listed above. Match
   replies by request ID. For asynchronous methods, also handle
   `gnoblin.operation.completed`; report failures instead of treating an
   accepted request as a completed compositor action. Use a fresh XDG
   Activation token for pointer-driven focus or the one-use shortcut context
   for keyboard-driven focus. A window ID by itself cannot authorize focus.
3. Remove the OSD bridge shim and subscribe to `gnoblin.osd.requested`. Keep
   LaunchFeedback on its documented D-Bus API.
4. Remove GNOME Shell D-Bus calls and compatibility-only socket commands from
   the standalone launch path. Do not remove GNOME-session support from a
   separate compatibility package as part of this change.

## Check the migration

Run Bingux's QML checks and tests, then verify it in a fresh standalone Gnoblin
session. Check shortcut registration after startup and reconnect, workspace
switching, and window events. Verify focus transfer after user input, thumbnails,
privacy changes, snapping, and text insertion. Check launch feedback, OSD
requests, and screen capture through the portal.

If Bingux still supports GNOME sessions, confirm that path separately.

The [shell integration guide](shell-integration.md) and
[bridge examples](bridge-examples.md) describe the public integration
interfaces.
