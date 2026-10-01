# Migrate Bingux to standalone Gnoblin

Bingux remains a Quickshell application. The standalone Gnoblin session does
not start GNOME Shell or its GJS bridge, so Bingux must use Gnoblin's public
compositor socket, `gnoblinctl`, Wayland protocols, and desktop portals.

This guide describes the changes needed by the current Bingux integration. It
does not claim that Bingux has already completed them. Keep panel, dock,
launcher, notification, and popup UI in Bingux. Use Lua for Gnoblin policy and
runtime behavior, not for drawing shell surfaces.

## Replace the legacy bridge calls

The compositor socket protocol used by the GNOME Shell compatibility bridge
accepts several short operations that the standalone Gnoblin socket does not.
Migrate those calls before running Bingux in a standalone session.

Replace these compatibility-bridge calls:

- **Shortcuts:** Replace `bind`, `clear`, and `status` with `shortcut.bind`,
  `shortcut.unbind`, and the `gnoblin.shortcut.binding-activated` event. Negotiate
  the API version from `hello`. Bindings belong to the connection; register them
  again after reconnecting.
- **Workspaces:** Replace `workspace-list` and `workspace-switch` with
  `workspace.list` and `workspace.switch`. Subscribe to workspace events or
  refresh the snapshot after a change.
- **Windows:** Subscribe with `op: "windows"` for the initial snapshot and live
  window and workspace events. Use `windows.list` for filtered reads.
- **Focus:** Replace `activate` with `window.focus` using an XDG Activation token
  created from user input on the same socket connection. A window ID alone
  cannot take focus.
- **Previews:** Replace `preview` with the asynchronous `window.thumbnail` API.
- **Privacy:** Replace `privacy` with the `privacy.state` read and the
  `gnoblin.privacy.changed` event.
- **Snapping:** Replace `window-drag` and snap events with pointer-drag event
  subscriptions and `window.snap.offer` on the connection that received the
  drag token.
- **Text insertion:** Replace `bingux.input-anchor` and `bingux.type-text` with
  `input.text_target` and `input.insert_text`, using the one-use focus context
  from a shortcut activation. These methods require the active text-input-v3
  client.
- **Capture:** Replace `capture-windows` with `windows.list` for enumeration and
  `window.thumbnail` for bounded previews. Continue using the ScreenCast portal
  for screen capture and recording.

The standalone socket accepts these top-level operations:

- `api` for a versioned method call;
- `events` for event subscriptions;
- `windows` and `monitors` for snapshots and change events;
- `ping` to check the connection.

The [compositor bridge reference](compositor-bridge.md) documents request
envelopes, API versions, event payloads, ownership, and limits. The standalone
socket does not accept the compatibility bridge's short command names.

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
2. Replace the compatibility operations with the API calls in the table. Handle
   asynchronous operation completion and errors instead of assuming a command
   reply means a compositor action succeeded.
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
