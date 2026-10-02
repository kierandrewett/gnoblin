# Migrate Bingux to standalone Gnoblin

Bingux is a Quickshell shell. In a standalone Gnoblin session, it owns the
panels, dock, launcher, notifications, popups, and other visible shell UI.
Gnoblin owns the compositor and session services. Connect the two through the
Gnoblin compositor socket, standard Wayland protocols, and desktop portals.

This guide covers Bingux in a standalone Gnoblin session. Gnoblin does not
start GNOME Shell or provide GNOME Shell's private D-Bus services. Any
GNOME-specific integration belongs to Bingux and is separate from this path.

## What already uses Gnoblin

Bingux's standalone path already uses the compositor socket for window and
workspace state, dynamic shortcuts, focus, previews, snapping, privacy state,
and emoji insertion. Capture and recording use the ScreenCast portal. Launch
feedback uses Gnoblin's D-Bus service. Keep these responsibilities in place:

| Bingux behavior                 | Standalone interface                                                      | Limits                                                                                                          |
| ------------------------------- | ------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Window and workspace state      | Compositor socket snapshots and events                                    | Use stable Gnoblin IDs and refresh after reconnecting.                                                          |
| Global shortcuts                | Connection-owned shortcut bindings and events                             | Held shortcuts need the advertised API methods. End a held session without dropping the binding when supported. |
| Focus transfer                  | `window.focus` with a recent activation context, or normal XDG Activation | A window ID by itself does not authorize focus.                                                                 |
| Lua runtime health              | `runtime.status` from native-control API 1.67+                            | Poll `state` during worker recovery; `generation` changes when Mutter accepts a new config.                     |
| Window previews                 | Asynchronous window thumbnails                                            | Requests can fail while the session is locked or the window is gone.                                            |
| Pointer and keyboard snapping   | Gnoblin snap operations                                                   | Requests need a valid shortcut context; keyboard snapping also needs its API method.                            |
| Privacy indicators and controls | Privacy snapshot and stop operations                                      | Show only the activity sources returned by the compositor.                                                      |
| Emoji insertion                 | Text-target and text-insertion operations                                 | Requires a fresh user action and an active Wayland text-input-v3 session. X11 clients are unsupported.          |
| Screen capture and recording    | XDG ScreenCast portal                                                     | Keep selection and recording UI in Bingux; the portal handles the session.                                      |
| App launch feedback             | `org.gnoblin.LaunchFeedback`                                              | This reports launch activity; Bingux draws the indicator.                                                       |

Do not infer support from a version number alone. Read the socket's initial
`hello` record and check its advertised API methods, events, and capabilities.
Disable an unavailable action cleanly or provide a documented fallback. The
[compositor bridge reference](/compositor-bridge) defines request shapes,
versions, and errors.

## Remove the remaining GNOME Shell dependencies

### Calendar events

`shell/bingux/calendar-events.py` calls the private
`org.gnome.Shell.CalendarServer` service. That service is not present in a
standalone Gnoblin session, so calendar events will be unavailable there.
Replace this provider with a Bingux-owned Evolution Data Server (EDS) client,
or make event loading optional and show a clear unavailable state. Keep
recurrence and calendar data in Bingux; they do not belong in the compositor
API.

### OSDs

The standalone shell receives `gnoblin.osd.requested` on the compositor socket
and renders the request in Bingux. This event is available from native-control
API 1.27.

It includes a stable `monitor_id`. Current builds add `output_names`, the
sorted, unique names of active physical connectors for that logical monitor.
Use it to route OSD surfaces on cloned displays. Older API 1.27 builds may omit
it.

Mutter may also supply `icon` and `label`, but provides no OSD level or maximum.
Gnoblin sends this event when it delegates OSD presentation to the shell. See
the
[compositor bridge reference](/compositor-bridge#api-127-shell-presentation-requests)
for the full contract.

Keep `shell/bingux/osd-bridge.js`, which patches GNOME Shell's private OSD
manager, inside the GNOME compatibility path. It is only a fallback for older
Gnoblin builds that suppress native OSDs without emitting the socket event; do
not start it in a standalone session.

### Compatibility GJS modules

Keep any GNOME-specific adapters in Bingux's own GNOME integration. The
standalone path uses Gnoblin's native text-input API and must not import
`Main`, `Meta`, or `global` from GJS. Do not start GNOME adapters because a
Gnoblin process or configuration exists; the standalone session has no GNOME
Shell runtime.

## Keep shell UI and app dependencies in Bingux

Gnoblin does not draw Bingux panels, menus, launchers, notifications, or OSDs.
Use Quickshell layer surfaces for those interfaces. Use standard Wayland
protocols for portable window listing and control, the Gnoblin socket for
Gnoblin-specific compositor behavior, and portals for capture and other
application requests.

Users may run Gnoblin without GNOME applications or services installed. Bingux
must handle optional applications and providers being absent: for example,
opening GNOME Calendar should offer an unavailable or install action instead
of assuming the application exists. Document runtime packages such as
PipeWire/WirePlumber and portal implementations with Bingux's install
instructions, not as compositor socket features.

## Migration steps

1. Run Bingux in a standalone Gnoblin login and confirm it connects to the
   compositor socket. Check the `hello` record before enabling each feature.
2. Keep socket subscriptions on a persistent connection. On disconnect, clear
   connection-owned shortcut and focus contexts, reconnect, request fresh
   snapshots, and register bindings again.
3. Render `gnoblin.osd.requested` events in Bingux's own OSD surfaces. Keep the
   GJS OSD adapter out of the standalone startup path.
4. Replace the calendar helper's private `CalendarServer` client or expose an
   explicit no-provider state.
5. Keep GNOME-specific adapters outside the standalone Gnoblin startup path.
6. Treat desktop applications, PipeWire, WirePlumber, and portal backends as
   optional runtime dependencies. Handle missing services without crashing or
   blocking the shell.

## Verify the standalone path

Test in a fresh standalone session, not only inside GNOME Shell. Verify
window/workspace updates, shortcut registration after reconnect, focus handoff,
previews, both snapping modes, privacy controls, emoji insertion, OSDs, launch
feedback, calendar availability, and portal capture. Repeat relevant checks in
a GNOME login if Bingux still supports that compatibility path.

The [shell integration guide](/shell-integration) describes the public shell
interfaces. The [Gnoblin Apps guide](/gnome-apps) explains which GNOME
applications and services are optional.
