# Compositor bridge

`src/scripts/compositor-bridge.js` exposes reusable shortcut sessions and window
control to desktop clients. It contains no Alt+Tab chooser, window ordering, or
switcher settings. Bingux owns that feature in Quickshell.

The local session installer and Nix package install the script at
`share/gnoblin/scripts/compositor-bridge.js`. Link that file into
`~/.config/gnoblin/scripts/`, then run `gnoblinctl reload-scripts`. Bingux's Nix
module installs the link. No compositor restart is required.

## Protocol

Connect to `$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock`. The server sends
`{"event":"hello","version":1}`. Each request and response is one UTF-8 JSON
record followed by a newline. Keep the connection open. Test sessions can set
`GNOBLIN_COMPOSITOR_SOCKET` to a private path on both server and client.

| Request | Behaviour |
| --- | --- |
| `{"op":"bind","id":"example","accelerator":"<Alt>F8","hold":8}` | Register a shortcut and acknowledge with `bound`. |
| `{"op":"clear"}` | Remove this client's bindings and cancel its input session. |
| `{"op":"end"}` | Cancel this client's input session. |
| `{"op":"windows"}` | Subscribe to complete `windows` snapshots. |
| `{"op":"activate","window":"123"}` | Release this client's input session and activate that window. |
| `{"op":"preview","window":"123","width":224,"height":126}` | Request a window thumbnail without raising or focusing it. |
| `{"op":"status"}` | Return registered IDs and the active input session's ID. |

`hold` is zero for a single activation, 8 for Alt, 4 for Control, or 67108864
for Super. A held shortcut starts a temporary input grab before a client surface
maps. It sends `activated` with `id`, `first`, `modifiers`, and `time`.
Further registered shortcuts send another activation. Other key presses send
`key` with a keysym and modifier mask. Button presses send `pointer` with global
logical `x`, `y`, and a Clutter button number (1 is left).

Modifier release sends `released`. An explicit end, session lock, disconnection,
script reload, or ten-second timeout ends the grab. Connected clients receive
`cancelled` when the session is cancelled. Clients must hide their UI on either
event and choose their own action on release.

Window records contain `id`, `title`, `appId`, `focused`, `minimized`,
`lastUserTime`, `parent`, and `monitor` (logical origin or null). IDs are strings
and remain stable for the lifetime of the window in this compositor session.
The bridge excludes skip-taskbar and override-redirect windows. Clients decide
how to order windows and group transients. Activation of a stale ID returns
`error` with a message.

The socket directory is private to the user. The bridge accepts eight clients,
32 bindings per client, and bounded input and output buffers. Invalid requests
return `error`; malformed JSON or excessive buffered input disconnects the
client. Disconnection releases all resources owned by that client.

Preview responses contain `event: "preview"`, the window ID, dimensions, and a
PNG data URI in `source`. A failed capture or released image buffer returns an
empty source and a message. Clients should retain their previous valid image.
Requests accept dimensions up to 480 by 320 pixels and preserve aspect ratio.
Only one capture may be pending per client. Readback runs at low priority after
a short delay so queued keyboard input runs first. The client controls update
frequency; the bridge does not start a continuous capture stream. Capture is
unavailable while the session is locked. The bridge scales the window texture
on the GPU before reading thumbnail pixels back. PNG encoding uses the shell's
asynchronous worker and an in-memory stream; no temporary image files are
written. Minimized windows use their retained backing buffer. Output is limited
to 4 MiB per connection, including preview data.
