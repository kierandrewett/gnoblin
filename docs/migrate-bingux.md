# Migrate Bingux to standalone Gnoblin

Bingux remains a Quickshell application. The standalone Gnoblin session does
not start GNOME Shell or its GJS bridge, so Bingux must use Gnoblin's public
compositor socket, `gnoblinctl`, Wayland protocols, and desktop portals.

Keep panel, dock, launcher, notification, and popup UI in Bingux. Use Lua for
Gnoblin policy and runtime behavior, not for drawing shell surfaces.

## Readiness

Bingux has not completed the standalone client migration. Gnoblin provides
Lua-backed APIs for workspaces, window snapshots and actions, connection-bound
shortcuts, focus, thumbnails, privacy, snapping, text insertion, and OSD
requests. Bingux still uses compatibility socket operations for several of
these paths, so API availability does not prove that the standalone client
uses them. Screen capture remains on the ScreenCast portal.

A live devkit check confirmed that a modal held shortcut registers through
`ShortcutSession.qml`. Native API smoke checks exercised privacy calls,
thumbnail completion handling, and snap subscriptions. A full real-window
preview and actual pointer or keyboard snap are not yet verified end to end.

Application-launch focus and OSD already have standalone Gnoblin interfaces.
Bingux still needs to connect its client paths to them. A fresh standalone
session check is also needed for emoji insertion, full window previews, and
pointer and keyboard snapping.

The calendar helper also calls GNOME Shell's private
`org.gnome.Shell.CalendarServer` service. That service is absent when GNOME
Shell is not running. Replace it with a Bingux-owned Evolution Data Server
provider, or make calendar events an optional integration with a clear
unavailable state. Keep this out of Gnoblin's compositor API.

In `ShortcutSession.qml`, the private `bingux.input-anchor` and
`bingux.type-text` calls are still used by the emoji path. Replace them with
Gnoblin's text-target API for the standalone session. It needs a fresh
Super+Period press after the picker closes and a focused Wayland client with an
active text-input-v3 session. X11 and clipboard fallback are unsupported.

`osd-bridge.js` patches GNOME Shell's OSD manager and calls
`org.gnoblin.Shell`. Remove it from the standalone launch path and render OSDs
in Bingux from `gnoblin.osd.requested`.

The standalone session has no `org.gnome.Shell` service or GJS bridge. Keep any
GNOME-session compatibility path separate from the standalone path.

## Use the standalone APIs

The GNOME compatibility bridge accepts operations that the standalone Gnoblin
socket does not. Keep those calls out of Bingux's standalone launch path.

The remaining compatibility calls are concentrated in
`shell/bingux/ShortcutSession.qml`, with UI consumers in the switcher, privacy
indicator, snap assist, and emoji picker. `shell/bingux/WorkspaceState.qml`
also still uses the legacy workspace commands in the primary Bingux checkout.
Treat each UI as migrated only after its transport uses the standalone API and
the interaction works in a fresh Gnoblin session.

- **Shortcuts:** Migrate `ShortcutSession.qml` from its compatibility command
  stream (`clear`, `bind`, and `status`) to version-negotiated
  `shortcut.bind`, `shortcut.unbind`, and event subscriptions. Bindings belong
  to the connection; register them again after reconnecting. Keep the
  compatibility path for GNOME sessions separate.
- **Window state:** `ShortcutSession.qml` requests the legacy `windows` stream
  and emits snapshots to `WindowSwitcher.qml`. Replace it with a
  `windows.list` snapshot and subscriptions to window and workspace events.
  Use `window.list` for filtered socket reads where needed.
- **Focus:** `ShortcutSession.qml` sends the legacy `activate` operation for
  `WindowSwitcher.qml` and the application launcher. Replace it with
  `window.focus`, passing the one-use `focus_context` from shortcut
  activation. For pointer-driven focus, use an XDG Activation token created
  from user input on the same socket connection. A window ID alone cannot take
  focus. Shortcut focus contexts require API 1.11; XDG Activation focus
  requires API 1.32.
- **Previews:** `ShortcutSession.qml` sends the legacy `preview` operation for
  `WindowSwitcher.qml`. Replace it with the asynchronous `window.thumbnail`
  API and route the completion event back to the switcher. Requests fail while
  the session is locked, and a closed or non-drawable window can return an
  error.
- **Privacy:** `PrivacyState.qml` still sends `privacy`, `stop-sharing`, and
  `stop-recording` compatibility operations. Use `privacy.state`, the
  `gnoblin.privacy.changed` event, `privacy.stop_sharing`, and
  `privacy.stop_recording`. Stop methods require API 1.31. The native API does
  not report camera or location activity.
- **Snapping:** `ShortcutSession.qml` and `SnapAssist.qml` still use
  compatibility messages for drag tracking, snap offers, and keyboard snaps.
  Replace them with pointer-drag events and work-area-bounded
  `window.snap.offer` requests on the connection that received the drag token.
  Use `window.snap_context` and `window.snap` for keyboard snapping with the
  shortcut's one-use focus context.
- **Text insertion:** The active emoji path still requests caret data through
  the private `bingux.input-anchor` bridge. Replace `bingux.input-anchor` and
  `bingux.type-text` with the standalone text-target methods. Pass the one-use
  focus context from a shortcut activation to `input.text_target`; send the
  returned target and text to `input.insert_text` on the same connection.

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

## Finish the remaining migration

1. Replace switcher, dock, notification, and app-launch focus calls with
   shortcut focus contexts or XDG Activation tokens created from the relevant
   user action. Keep each token and request on the same connection.
2. Remove the OSD bridge shim from standalone startup. Subscribe to
   `gnoblin.osd.requested` and draw the OSD in Bingux. Keep LaunchFeedback on
   its documented D-Bus API.
3. Keep GNOME Shell D-Bus calls and compatibility-only socket operations out
   of standalone startup. Preserve them in the separate GNOME compatibility
   path if Bingux continues to support GNOME sessions.
4. Replace the calendar helper's GNOME Shell CalendarServer dependency with a
   Bingux-owned provider, or disable event loading cleanly when that optional
   provider is unavailable.
5. Replace workspace polling with workspace snapshots and lifecycle events,
   then verify the standalone path in a fresh session, including shortcut
   reconnects, focus, previews, snapping, emoji insertion, OSD, launch
   feedback, calendar availability, and portal capture.

## Check the migration

Run Bingux's QML checks and tests, then verify it in a fresh standalone Gnoblin
session. Check shortcut registration after startup and reconnect, workspace
switching, and window events. Verify focus transfer, thumbnails, privacy
changes, snapping, text insertion, launch feedback, OSD, and portal capture.

If Bingux still supports GNOME sessions, confirm that path separately.

The [shell integration guide](shell-integration.md) and
[bridge examples](bridge-examples.md) describe the public integration
interfaces.
