# Compositor bridge

Use this socket API to build a dock, launcher or window switcher. It lets your
shell read windows, register shortcuts and request window actions. Your shell
still draws the UI and decides how to group and order windows.

For terminal commands and scripts, [gnoblinctl](gnoblinctl.md) handles the
connection for you.

## Availability

The bridge is a built-in Gnoblin compositor service. It starts with the
Gnoblin Shell component, stays available across `gnoblinctl reload`, and does
not need to be installed under `~/.config/gnoblin/scripts/`.
`gnoblinctl window list` uses the same socket. Check `gnoblinctl status` before
debugging a client connection. Older installed builds may not include the
built-in service yet; [check the running build](source-development.md#verify)
when its behaviour differs from this reference.

Bingux is a separate shell project that uses this interface. A custom shell can
connect to it without installing Bingux.

## Connect

Socket: `$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock`.
Use `GNOBLIN_COMPOSITOR_SOCKET` on both ends for a private test path.

The server sends a greeting, including available features:

```json
{ "event": "hello", "version": 1, "features": ["ui-sessions", "switcher-fallback", "overlay-shortcut"] }
```

Additional features depend on the running build. Send one UTF-8 JSON object
per line, followed by a newline. Keep the connection open.

The `hello.version` field is the socket protocol version. Check `features`
before using optional operations such as `ui-session`, bare Super, blur regions
or layer animation policy. A validation error is
`{"event":"error","message":"..."}`; if a valid `command` request fails,
the error also carries its request `id`. Malformed JSON or excessive input
closes the socket; ordinary validation errors leave it open.

## Operation index

| `op`                             | Required fields                          | Reply or stream                                     |
| -------------------------------- | ---------------------------------------- | --------------------------------------------------- |
| `command`                        | `id`, `command`; command-specific fields | One `reply` with matching `id`, or `error`          |
| `windows`                        | None                                     | Current `windows` snapshot, then changes            |
| `privacy`                        | None                                     | Current `privacy` state, then changes               |
| `status`                         | None                                     | One `status` with binding IDs and active session ID |
| `bind`                           | `id`, `accelerator`, `hold`              | `bound`, then activation and input events           |
| `activate`                       | `window`                                 | Focus a window; no success reply                    |
| `preview`                        | `window`, `width`, `height`              | One `preview` event                                 |
| `shortcut-input`                 | `name`, `state`                          | Input handoff; no success reply                     |
| `ui-session`                     | `action`; other fields depend on action  | `ui-state` or `ui-command` events                   |
| `layer-animation-policy`         | `namespace`                              | One policy event for that layer namespace           |
| `blur-region`                    | `namespace`, `screen`, `region`          | No success reply                                    |
| `window-drag`                    | None                                     | Current `window-drag` state, then changes           |
| `snap-offer`                     | `serial`, `regions`                      | No success reply; may later get `snap-completed`    |
| `snap-context`                   | None                                     | One `snap-context` event                            |
| `snap-window`                    | `window`, `monitor`, `target`            | Applies a region; no success reply                  |
| `stop-sharing`, `stop-recording` | None                                     | Requests stop; no success reply                     |
| `end`                            | Optional `session` for fallback switcher | Ends this client's input session                    |
| `clear`                          | None                                     | Removes this client's bindings and session          |

`command` accepts `windows`, `capture-windows`, `workspaces`, `monitors`,
`workspace-switch` and `window`. `workspace-switch` needs a one-based
`workspace`; `window` needs an `action` and a stable window ID or `"active"`.
The [CLI reference](gnoblinctl.md) lists window actions and arguments. Only
`command` supplies a correlation ID: match `reply` or `error` by that ID
because other events can arrive first. A reply with `pending: true` means the
action was accepted; observe later state to confirm completion.

`capture-windows` returns visible, non-minimised windows in stacking order
with title, app name, frame position and size, and `bufferWidth` and
`bufferHeight` for capture. These IDs come from Mutter's window ID, whereas
`windows` snapshots use a stable sequence string. Obtain an action ID from
`windows` or `gnoblinctl window list` before sending a `window` action.

## Example: watch the window list

Run this Python program inside your Gnoblin session:

```python
import json
import os
import socket

path = os.environ.get("GNOBLIN_COMPOSITOR_SOCKET") or os.path.join(
    os.environ["XDG_RUNTIME_DIR"], "gnoblin/compositor-v1.sock"
)
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
    connection.connect(path)
    with connection.makefile("r", encoding="utf-8") as messages:
        print("Greeting:", json.loads(messages.readline()))
        connection.sendall(b'{"op":"windows"}\n')
        for line in messages:
            event = json.loads(line)
            if event.get("event") == "windows":
                print(event["windows"])
```

It prints the current list and subsequent snapshots as windows change. Stop it
with Ctrl+C. Each snapshot replaces the previous list; it is not a list of changes.

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

For a held shortcut, Gnoblin captures key and pointer events before your popup
becomes visible. For example, an Alt-held switcher can finish when Alt is released.
Events include `activated`, `key`, `pointer`, `released` and `cancelled`.

`activated` includes id, first, modifiers and time.
Pointer coordinates are global logical pixels; button 1 is left.

Hide the UI on release or cancellation. Sending `{"op":"end"}`, locking,
disconnecting or reaching the ten-second timeout also releases the captured
input.

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
The client chooses grouping and ordering. Stale IDs return an error. `parent`
links a transient dialog to its parent; `lastUserTime` is a recency hint.

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

## Bingux text-entry integration

Text entry is provided by a Bingux-owned integration. Bingux installs a Gnoblin
script that registers the namespaced
`bingux.input-anchor` and `bingux.type-text` operations. Other shells can omit
the integration script and implement their own text-entry behavior.

`{"op":"bingux.input-anchor"}` returns x, y, pid, window, frame and caret.
Coordinates are logical desktop pixels. Caret may be null; it clears on focus
changes and lock, and adjusts when the window moves.

`{"op":"bingux.type-text","window":"123","text":"…"}` accepts up to 64 UTF-16
units, without control characters. The target must be focused and unlocked,
with Control, Alt and Super released.

Hide the picker and restore the target before sending. A `typed` reply
acknowledges delivery, not proof that the app consumed the text.

For XWayland, a GTK helper temporarily offers the text and sends Ctrl+V.
It preserves clipboard formats and restores them after 600 ms unless a new copy
takes priority. Clipboard managers may record the temporary text.

Clipboard preparation is limited to 64 formats, 64 MiB and three seconds.
Failed preparation leaves it unchanged. Native Wayland text input does not
use the clipboard.

On XWayland, Bingux uses its `bingux-clipboard-paste` helper, which needs
Python, PyGObject and GTK 3. Native Wayland text input does not use the helper.
The helper and GJS integration are shipped by Bingux. The integration script
is loaded from the XDG script search path; see [writing scripts](user-scripts.md).

## Recording and camera activity

`{"op":"privacy"}` subscribes to screenSharing, recording, recordingCount,
recordingElapsed, cameraInUse and locationCaptures.

Location captures list authorised GeoClue desktop IDs. Elapsed time is whole
seconds; clients can advance it between updates.

`stop-sharing` stops non-recording remote sessions.
`stop-recording` stops recording sessions. An encoder owner should use its
normal finalisation path to save output. These requests do not grant access.

## Coordinate shell processes

`ui-session` shares named state between UI processes. Names match
`^[a-z][a-z0-9-]{0,63}$`; one client owns each name.

| Action    | Fields                 | Event                                       |
| --------- | ---------------------- | ------------------------------------------- |
| `watch`   | None                   | `ui-state` for each owner and later changes |
| `state`   | `name`, `state` object | Publishes `ui-state`                        |
| `command` | `name`, `command`      | Sends `ui-command` to the owner             |

Use the envelope `{"op":"ui-session","action":"watch"}`. Owner disconnect
publishes `state: null`. A visible layer can include `surface` (its namespace),
`companions` (up to 16 namespaces), `revealCompanions: true`, and
`companionsAbove: true` in its state to coordinate panel stacking.

| Operation                | Fields                                                                                    | Limit or effect                                                                                                                            |
| ------------------------ | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `blur-region`            | `namespace`, monitor origin `screen: [x,y]`, local `region: [x,y,width,height]` or `null` | Up to 64 per client; requires a matching blur window rule; cleared on disconnect. See [effect rendering](effects-rendering.md#blur-cache). |
| `layer-animation-policy` | `namespace`                                                                               | Returns `enter`, `exit`, `windowShadow`; namespace at most 128 characters                                                                  |

For drag layouts, subscribe with `window-drag`, offer hit and target rectangles
using `snap-offer`, and apply a keyboard-chosen rectangle with `snap-window`.
The [snapping guide](/guides/window_snapping#shell-integration) gives the request
shapes and work-area checks.

## Limits and disconnects

The socket directory is private to the user.
Limits: 32 clients, 32 bindings per client, 64 queued records and 4 MiB queued
output per connection. An input buffer over 16 KiB or malformed JSON closes the
connection. A slow reader can also be disconnected when its output queue fills.

Invalid requests return errors. Malformed JSON or excessive input disconnects
the client. Disconnect releases its bindings and other resources.
