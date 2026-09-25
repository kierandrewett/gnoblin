# Compositor bridge

Use this socket API to build a dock, launcher or window switcher. Set persistent
shortcuts in the [Lua config](/config/configure/shortcuts); use bridge bindings
for interactive UI while your client is connected. The bridge also lets your
shell read windows and request window actions.

For terminal commands and scripts, [gnoblinctl](gnoblinctl.md) handles the
connection for you.

## Availability

The bridge is a built-in Gnoblin compositor service. It starts with the
Gnoblin Shell component, stays available across `gnoblinctl reload`, and does
not need to be installed under `~/.config/gnoblin/scripts/`.
`gnoblinctl window list` uses the same socket. Check `gnoblinctl status` before
debugging a client connection.

Bingux is a separate shell project that uses this interface. A custom shell can
connect to it without installing Bingux.

![Bingux dock showing Files, Firefox and Foot in a Gnoblin session](images/gnoblin-bingux-firefox.png)

_Bingux is one shell example built on Gnoblin's compositor interfaces._

## Connect

Socket: `$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock`.
Use `GNOBLIN_COMPOSITOR_SOCKET` on both ends for a private test path.

The server sends a greeting, including available features:

```json
{ "event": "hello", "version": 1, "features": ["ui-sessions", "switcher-fallback", "overlay-shortcut"] }
```

Additional features depend on the running build. Send one UTF-8 JSON object
per line, followed by a newline. Keep the connection open.

The `hello.version` field is the socket protocol version. This build always
advertises `ui-sessions`, `switcher-fallback` and `overlay-shortcut`. It may
also advertise `blur-regions` and `layer-animation-policy` when the required
compositor support is available.

A validation error has an `error` event. If a valid `command` request fails,
the response also carries its request `id`. Malformed JSON or excessive input
closes the socket; ordinary validation errors leave it open.

## Operation index

| `op`                             | Required fields                                                          | Reply or stream                                     |
| -------------------------------- | ------------------------------------------------------------------------ | --------------------------------------------------- |
| `command`                        | `id`, `command`; command-specific fields                                 | One `reply` with matching `id`, or `error`          |
| `windows`                        | None                                                                     | Current `windows` snapshot, then changes            |
| `privacy`                        | None                                                                     | Current `privacy` state, then changes               |
| `status`                         | None                                                                     | One `status` with binding IDs and active session ID |
| `bind`                           | `id`, `accelerator`, `hold`; optional `trigger`, `modal`, `captureInput` | `bound`, then activation and input events           |
| `activate`                       | `window`; optional `session`                                             | Focus a window; no success reply                    |
| `preview`                        | `window`, `width`, `height`                                              | One `preview` event                                 |
| `shortcut-input`                 | `name`, `state`                                                          | Input handoff; no success reply                     |
| `ui-session`                     | `action`; other fields depend on action                                  | `ui-state` or `ui-command` events                   |
| `layer-animation-policy`         | `namespace`                                                              | One policy event for that layer namespace           |
| `blur-region`                    | `namespace`, `screen`, `region`                                          | No success reply                                    |
| `window-drag`                    | None                                                                     | Current `window-drag` state, then changes           |
| `snap-offer`                     | `serial`, `regions`                                                      | No success reply; may later get `snap-completed`    |
| `snap-context`                   | None                                                                     | One `snap-context` event                            |
| `snap-window`                    | `window`, `monitor`, `target`                                            | Applies a region; no success reply                  |
| `stop-sharing`, `stop-recording` | None                                                                     | Requests stop; no success reply                     |
| `end`                            | Optional `session` for fallback switcher                                 | Ends this client's input session                    |
| `clear`                          | None                                                                     | Removes this client's bindings and session          |

`command` accepts `windows`, `capture-windows`, `workspaces`, `workspace-list`,
`workspace-switch`, `workspace-next`, `workspace-previous`,
`workspace-move-active`, `monitors`, `layers` and `window`. `layers` returns
the current layer-shell surfaces in a `surfaces` array. `window` needs an
`action` and a stable window ID or `"active"`. The [CLI reference](gnoblinctl.md)
lists window actions and arguments.

### Workspace commands

Use `workspace-list` to get the workspace ID, current one-based number, display
name, active state and eligible window count:

```json
{ "op": "command", "id": "request-1", "command": "workspace-list" }
```

The reply's `result` is shaped like this:

```json
{
    "workspaces": [
        { "id": "code", "number": 1, "name": "Code", "active": true, "windows": 2 },
        { "id": "web", "number": 2, "name": "Web", "active": false, "windows": 0 }
    ]
}
```

The request `id` correlates its reply. A configured workspace ID is assigned by
its initial position and stays with that `MetaWorkspace` if workspaces are
reordered during the session. An unconfigured workspace receives a session-only
ID such as `@session-N`.

Each item from `workspaces` has these fields:

| Field     | Meaning                                                             |
| --------- | ------------------------------------------------------------------- |
| `id`      | Configured ID, or a generated session-only ID such as `@session-1`. |
| `number`  | Current one-based position.                                         |
| `name`    | Display label.                                                      |
| `active`  | Whether this workspace is selected.                                 |
| `windows` | Number of eligible windows on the workspace.                        |

The older `workspaces` command keeps its legacy response: its numeric `id` is
the current one-based position, and it does not include the display name.

Switch by stable ID or current number. Send exactly one selector:

```json
{"op":"command","id":"request-2","command":"workspace-switch","workspaceId":"code"}
{"op":"command","id":"request-3","command":"workspace-switch","workspaceNumber":2}
```

`workspace-switch` also accepts a numeric `workspace` selector.
`workspace-next` and `workspace-previous` take no selector and wrap at the
ends of the current workspace list.

Switch replies contain `ok`, `pending`, and the one-based `workspace` number.
They also include the resolved workspace's `id`, `number`, `name`, `active`,
and `windows` fields.

Move the focused window with `workspace-move-active`. It accepts the same
`workspaceId` or `workspaceNumber` selector, plus an optional `follow` boolean.
The default is `false`; set it to `true` to activate the destination after
moving.

For an explicit window, use `command: "window"`, `action: "workspace"`, and
the `window` ID with `workspaceId` or `workspaceNumber`. The one-based numeric
`workspace` selector is also accepted.

Successful move replies include the resolved workspace fields, `follow`, the
stable window `window` ID, and the selected `workspace` number, `workspaceId`,
and `workspaceNumber`.

Numbers can change when dynamic workspaces are removed. Use IDs in shell
integrations that need to keep addressing a configured workspace. The
[workspace CLI examples](gnoblinctl.md#workspaces-and-monitors) show equivalent
terminal commands.

Only `command` supplies a correlation ID: match `reply` or `error` by that ID
because other events can arrive first. A reply with `pending: true` means the
action was accepted; observe later state to confirm completion.

Animations are registered in Lua with `gnoblin.animation` and are previewed
through `gnoblinctl animation`. Shell clients should continue to own their
surface content motion; use a matching `animation = "none"` layer rule to
avoid applying compositor motion twice.

The bridge's `layer-animation-policy` operation lets a shell read the
configured enter/exit policy for a namespace. See the [animation guide](/guides/animations)
for layer-shell lifecycle events and target selection.

### Capture records

`capture-windows` returns visible, non-minimised windows in stacking order.

| Field                         | Meaning                                                          |
| ----------------------------- | ---------------------------------------------------------------- |
| `id`                          | Mutter window ID, used for capture.                              |
| `title`, `appId`, `appName`   | Window title, desktop-entry ID and application name.             |
| `x`, `y`, `width`, `height`   | Frame rectangle in logical pixels.                               |
| `bufferWidth`, `bufferHeight` | Paint-box size; may include decoration shadows beyond the frame. |

### Window records

The `windows` snapshot has these fields:

| Field                                         | Meaning                                                             |
| --------------------------------------------- | ------------------------------------------------------------------- |
| `id`                                          | Stable window sequence ID for this session.                         |
| `title`, `appId`                              | Window title and desktop-entry ID (or WM class if unavailable).     |
| `gtkAppId`, `wmClass`, `ruleAppId`            | Raw GTK ID, WM class and the ID used by window rules.               |
| `focused`, `minimized`                        | Whether the window is focused or minimised.                         |
| `workspace`, `workspaceId`, `workspaceNumber` | Current workspace number, stable ID and current one-based position. |
| `monitorIndex`, `monitor`                     | Zero-based monitor index and its logical origin `{x, y}`.           |
| `maximized`, `fullscreen`                     | Current window state.                                               |
| `geometry`                                    | Frame rectangle `{x, y, width, height}` in logical pixels.          |
| `lastUserTime`                                | Mutter's timestamp for the last user interaction with the window.   |
| `parent`                                      | Stable ID of its transient parent, or `null`.                       |

Capture IDs come from Mutter; `windows` snapshots use stable sequence IDs.
Use an ID from the `windows` snapshot or `gnoblinctl window list` for a
`window` action. The CLI reference lists the supported actions and arguments.

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

## Temporary UI bindings

Use `bind` for an interactive UI such as a switcher. These bindings belong to
the client connection and disappear when it disconnects.

Optional `trigger` is `"press"` (the default) or `"release"`; it chooses whether `activated` is sent when the accelerator is pressed or its main key is released. This works with chords using Alt, Control, Shift or Super. Bare `"Super"` is release-only because Mutter first checks whether another key joins the chord.

```json
{ "op": "bind", "id": "example", "accelerator": "<Alt>F8", "hold": 8, "trigger": "release" }
```

The server acknowledges:

```json
{ "event": "bound", "id": "example" }
```

| Field          | Accepted values                                              | Default                                         |
| -------------- | ------------------------------------------------------------ | ----------------------------------------------- |
| `id`           | 1–64 letters, numbers, `_` or `-`; unique on this connection | Required                                        |
| `accelerator`  | GTK accelerator string up to 128 characters, or `"Super"`    | Required                                        |
| `hold`         | `0`, Alt `8`, Control `4`, or Super `67108864`               | Required; `0` fires once                        |
| `trigger`      | `"press"` or `"release"`                                     | `"press"`; `"Super"` uses `"release"`           |
| `modal`        | Boolean                                                      | `true`; `false` leaves app input ungrabbed      |
| `captureInput` | Boolean                                                      | `false`; enables the popup typing handoff below |

For a modal held shortcut, Gnoblin captures key and pointer events before your
popup becomes visible. An Alt-held switcher can finish when Alt is released.

Set `modal = false` for a held shortcut that should leave focus and input with
the current app. Gnoblin reports the activation and modifier release without
opening an input grab. For example, this reports Alt+F8 and the later Alt
release while the app keeps receiving input:

```json
{ "op": "bind", "id": "cycle-mode", "accelerator": "<Alt>F8", "hold": 8, "modal": false }
```

Events sent during a binding session:

| Event                   | Fields                                        | Meaning                                                      |
| ----------------------- | --------------------------------------------- | ------------------------------------------------------------ |
| `activated`             | `id`, `first`, `session`, `modifiers`, `time` | The accelerator fired.                                       |
| `key`                   | `key`, `modifiers`                            | A captured key press.                                        |
| `pointer`               | `x`, `y`, `button`                            | A captured button press in global logical pixels; 1 is left. |
| `released`, `cancelled` | `session`                                     | The modifier was released or the session ended early.        |

`first` is true when no binding session was active. `session` is zero for
ordinary bindings and nonzero for the built-in switcher fallback. `modifiers`
is a Clutter modifier mask, and `time` is the event timestamp in milliseconds.

Hide the UI on release or cancellation. Sending `{"op":"end"}`, locking,
disconnecting or reaching the ten-second timeout also releases the captured
input.

## Bare Super and buffered typing

```json
{ "op": "bind", "id": "search", "accelerator": "Super", "hold": 0, "captureInput": true }
```

Requires the advertised `overlay-shortcut` feature. Only one bridge client can
own bare Super; remove a duplicate Lua command binding first.

Super activates on release, excluding chords. With capture enabled, report the
popup's focus handoff:

```jsonl
{ "op": "shortcut-input", "name": "search", "state": "prepared" }
{ "op": "shortcut-input", "name": "search", "state": "ready" }
```

Send `prepared` once the popup is mapped. Gnoblin releases its keyboard grab
but keeps buffering keys. Send `ready` when the text field has focus; Gnoblin
replays the buffered keys in order. Send `closed` when the popup is dismissed.
The buffer expires after three seconds if the popup never becomes ready.

The binding ID is the handoff name.

## Windows and controls

| Request                            | Result                                    |
| ---------------------------------- | ----------------------------------------- |
| `{"op":"windows"}`                 | Subscribe to complete snapshots           |
| `{"op":"activate","window":"123"}` | End this client's grab and focus a window |
| `{"op":"status"}`                  | Registered IDs and active session         |
| `{"op":"end"}`                     | Cancel this client's input session        |
| `{"op":"clear"}`                   | Remove this client's bindings and session |

Snapshots exclude skip-taskbar and override-redirect windows. IDs last for the
window's session lifetime. The client chooses grouping and ordering; stale IDs
return an error. See the field list below for the complete record shape.

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

`watch` immediately sends current owners, then updates. A `state` request claims
or updates a name; another client cannot claim it. Owner disconnect removes its
state and publishes `state: null`. `command` is forwarded to the current owner
as-is; it is ignored when no owner exists and is never queued.

These requests show the message shapes; each goes over the relevant client's
own bridge connection:

```jsonl
{"op":"ui-session","action":"watch"}
{"op":"ui-session","action":"state","name":"search","state":{"visible":true}}
{"op":"ui-session","action":"command","name":"search","command":{"action":"close"}}
```

The owner defines the state and command objects. To coordinate layer stacking,
include `surface` (the owner's namespace) and `companions` (up to 16 namespace
strings) in state. Gnoblin applies the request while `visible` or
`revealCompanions` is true; set `companionsAbove: true` to place companions
above the owner. The state is advisory and does not change how a client draws.

| Operation                | Fields                                                                                    | Limit or effect                                                                                                                            |
| ------------------------ | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `blur-region`            | `namespace`, monitor origin `screen: [x,y]`, local `region: [x,y,width,height]` or `null` | Up to 64 per client; requires a matching blur window rule; cleared on disconnect. See [effect rendering](effects-rendering.md#blur-cache). |
| `layer-animation-policy` | `namespace`                                                                               | Returns `enter`, `exit`, `windowShadow`; namespace at most 128 characters                                                                  |

For `blur-region`:

- `screen`: two finite coordinates within ±65,536 for the monitor origin.
- `region`: `null` clears it. Otherwise use four finite values
  `[x, y, width, height]`; coordinates must be within ±65,536, and dimensions
  cannot be negative.
- Scope: Gnoblin applies the rectangle only to a matching layer surface owned
  by the sending process. A matching window rule must also enable blur.

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
