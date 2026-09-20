# Compositor bridge

A user-local JSON socket for shortcuts, window control, previews and activity
state. The client owns its UI and window ordering.

## Enable it

Install or link `share/gnoblin/scripts/compositor-bridge.js` into
`~/.config/gnoblin/scripts/`, then run `gnoblinctl reload`.
Custom shells can use this directly; Bingux provides its own wiring.

## Connect

Socket: `$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock`.
Use `GNOBLIN_COMPOSITOR_SOCKET` on both ends for a private test path.

The server sends a greeting, including available features:

```json
{ "event": "hello", "version": 1, "features": ["ui-sessions", "switcher-fallback", "overlay-shortcut"] }
```

Additional features depend on the running build. Send one UTF-8 JSON object
per line, followed by a newline. Keep the connection open.

## Register a shortcut

```json
{ "op": "bind", "id": "example", "accelerator": "<Alt>F8", "hold": 8 }
```

The server acknowledges:

```json
{ "event": "bound", "id": "example" }
```

| Hold value | Behavior       |
| ---------- | -------------- |
| `0`        | One activation |
| `8`        | Hold Alt       |
| `4`        | Hold Control   |
| `67108864` | Hold Super     |

Held sessions begin an input grab before the client maps its surface.
Events include `activated`, `key`, `pointer`, `released` and `cancelled`.

`activated` includes id, first, modifiers and time.
Pointer coordinates are global logical pixels; button 1 is left.

Hide the UI on release or cancellation. Explicit end, lock, disconnect,
script reload and a ten-second timeout end the grab.

## Bare Super and buffered typing

```json
{ "op": "bind", "id": "search", "accelerator": "Super", "hold": 0, "captureInput": true }
```

Requires the advertised `overlay-shortcut` feature. Only one bridge client can
own bare Super; remove a duplicate Lua command binding first.

Super activates on release, excluding chords. With capture enabled, send:

```json
{ "op": "shortcut-input", "name": "search", "state": "ready" }
```

Send ready only after the layer and text field have keyboard focus.
Send `state: "closed"` when dismissed. The binding ID is the handoff name.

## Windows and controls

| Request                            | Result                                    |
| ---------------------------------- | ----------------------------------------- |
| `{"op":"windows"}`                 | Subscribe to complete snapshots           |
| `{"op":"activate","window":"123"}` | End this client's grab and focus a window |
| `{"op":"status"}`                  | Registered IDs and active session         |
| `{"op":"end"}`                     | Cancel this client's input session        |
| `{"op":"clear"}`                   | Remove this client's bindings and session |

Window records contain id, title, appId, focused, minimized, lastUserTime,
parent and monitor. IDs are strings, stable for the window's session lifetime.

Skip-taskbar and override-redirect windows are excluded.
The client chooses grouping and ordering. Stale IDs return an error.

A subscription with no open windows produces:

```json
{ "event": "windows", "windows": [] }
```

When windows exist, the array uses the [window record example](gnoblinctl.md#output-for-scripts).
See [CLI development](cli-development.md#change-a-window) for a complete
request, successful reply and error reply.

## Previews

```json
{ "op": "preview", "window": "123", "width": 224, "height": 126 }
```

The response contains a PNG data URI in `source`, dimensions and window ID.
On failure it returns an empty source and a message; retain the last valid image.

Maximum size: 480 × 320, preserving aspect ratio. One capture may be pending
per client. No continuous stream is started; clients choose update frequency.

Capture is unavailable while locked. Readback runs at low priority, PNG encoding
uses a worker, and minimised windows use their retained buffer.

## Text input

`{"op":"input-anchor"}` returns x, y, pid, window, frame, buffer and caret.
Coordinates are logical desktop pixels. Caret may be null; it clears on focus
changes and lock, and adjusts when the window moves.

`{"op":"type-text","window":"123","text":"…"}` accepts up to 64 UTF-16 units,
without control characters. The target must be focused and unlocked, with
Control, Alt and Super released.

Hide the picker and restore the target before sending. A `typed` reply
acknowledges delivery, not proof that the app consumed the text.

For XWayland, a GTK helper temporarily offers the text and sends Ctrl+V.
It preserves clipboard formats and restores them after 600 ms unless a new copy
takes priority. Clipboard managers may record the temporary text.

Clipboard preparation is limited to 64 formats, 64 MiB and three seconds.
Failed preparation leaves it unchanged. Native Wayland text input does not
use the clipboard.

The fallback needs Python, PyGObject, GTK 3 and both installed `scripts/lib/`
helpers beside the bridge.

## Recording and camera activity

`{"op":"privacy"}` subscribes to screenSharing, recording, recordingCount,
recordingElapsed, cameraInUse and locationCaptures.

Location captures list authorised GeoClue desktop IDs. Elapsed time is whole
seconds and survives script reload; clients can advance it between updates.

`stop-sharing` stops non-recording remote sessions.
`stop-recording` stops recording sessions. An encoder owner should use its
normal finalisation path to save output. These requests do not grant access.

## Limits and disconnects

The socket directory is private to the user.
Limits: eight clients, 32 bindings per client, 4 MiB output per connection.

Invalid requests return errors. Malformed JSON or excessive input disconnects
the client. Disconnect releases its bindings and other resources.
