# Migrate Bingux to standalone Gnoblin

Bingux remains a Quickshell application. The standalone Gnoblin session does
not start GNOME Shell or its GJS bridge, so Bingux must use Gnoblin's public
compositor socket, `gnoblinctl`, Wayland protocols, and desktop portals.

Keep panel, dock, launcher, notification, and popup UI in Bingux. Use Lua for
Gnoblin policy and runtime behavior, not for drawing shell surfaces.

## Readiness

Bingux's standalone transport now uses Gnoblin's negotiated Lua-backed API for
workspace and window state, shortcuts, focus, thumbnails, privacy, snapping,
and emoji text insertion. It keeps the older compositor messages as a
compatibility path for sessions that do not advertise the Gnoblin API. Capture
window enumeration also uses `window.list`; screen capture and recording still
use the ScreenCast portal.

Source coverage does not prove each interaction works in a running session.
Verify shortcut reconnects, window thumbnails, focus handoff, pointer and
keyboard snapping, privacy controls, emoji insertion, and capture in a fresh
standalone session.

`ApplicationLauncher.qml` still attempts to focus a newly launched window
without an activation context. Gnoblin rejects focus requests without a fresh
user or launch context. The launch path needs an XDG Activation token created
from the initiating user action and sent over the same socket connection, or
it must leave focus to the application's normal activation request.

The calendar helper also calls GNOME Shell's private
`org.gnome.Shell.CalendarServer` service. That service is absent when GNOME
Shell is not running. Replace it with a Bingux-owned Evolution Data Server
provider, or make calendar events an optional integration with a clear
unavailable state. Keep this out of Gnoblin's compositor API.

The standalone emoji path uses `input.text_target` and `input.insert_text`.
Text insertion needs a fresh Super+Period press after the picker closes and a
focused Wayland client with an active text-input-v3 session. X11 and clipboard
fallback are unsupported. The older input-anchor messages remain only for the
compatibility path.

`osd-bridge.js` patches GNOME Shell's OSD manager and calls
`org.gnoblin.Shell`. Keep it inside the GNOME compatibility session. The
standalone path should render OSDs in Bingux from `gnoblin.osd.requested`.

The standalone session has no `org.gnome.Shell` service or GJS bridge. Keep any
GNOME-session compatibility path separate from the standalone path.

## Use the standalone APIs

The compatibility bridge accepts operations that the standalone Gnoblin
socket does not. Keep those operations in the fallback path only. In the
standalone path, `ShortcutSession.qml` negotiates methods and events, owns
shortcut bindings for one connection, and re-registers them after reconnecting.

Workspace state subscribes to workspace events. It polls only when an older
server does not provide those events.

The remaining API constraints are:

- Window focus and snapping require short-lived user contexts.
- Thumbnail requests complete asynchronously and fail while the session is
  locked.
- Privacy stop methods require API 1.31. The native privacy snapshot does not
  report camera or location activity.
- Text insertion requires API 1.28 and an active Wayland text-input-v3 session.
  X11 and clipboard fallback are unsupported.

The standalone socket accepts these top-level operations:

- `api` for a versioned method call;
- `events` for event subscriptions;
- `windows` and `monitors` for snapshots and change events;
- `ping` to check the connection.

Negotiate the API version from the initial `hello` event. The minimum
versions used by this migration are:

- dynamic shortcut binding: 1.11; held or modal bindings: 1.22;
- shortcut focus contexts: 1.11; XDG Activation focus: 1.32;
- pointer drag snapping: 1.26; keyboard snapping: 1.28;
- window thumbnails: 1.23; text targets: 1.28;
- privacy state: 1.17; privacy stop actions: 1.31;
- OSD requests: 1.27.

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

## Finish the remaining work

1. Fix app-launch focus. Create an XDG Activation token from the initiating
   user action and use it on the same socket connection, or leave focus to the
   application's activation request.
2. Subscribe to `gnoblin.osd.requested` and draw OSDs in Bingux. Keep the
   GNOME Shell OSD shim in the compatibility session and keep LaunchFeedback
   on its documented D-Bus API.
3. Replace the calendar helper's GNOME Shell CalendarServer dependency with a
   Bingux-owned provider, or disable event loading cleanly when that optional
   provider is unavailable.
4. Verify the standalone path in a fresh session, including shortcut
   reconnects, workspace and window events, focus, previews, snapping, emoji
   insertion, OSD, launch feedback, calendar availability, and portal capture.
   Check the GNOME compatibility path separately if Bingux continues to
   support it.

## Check the migration

Run Bingux's QML checks and tests, then verify it in a fresh standalone Gnoblin
session. Check shortcut registration after startup and reconnect, workspace
switching, and window events. Verify focus transfer, thumbnails, privacy
changes, snapping, text insertion, launch feedback, OSD, and portal capture.

If Bingux still supports GNOME sessions, confirm that path separately.

The [shell integration guide](shell-integration.md) and
[bridge examples](bridge-examples.md) describe the public integration
interfaces.
