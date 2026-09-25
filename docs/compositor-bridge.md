# Compositor bridge

Use this socket API to build a dock, launcher or window switcher. It lets your
shell read windows, request window actions and take temporary input grabs for
interactive UI. Put persistent shortcuts in the [Lua config](/config/configure/shortcuts).
Your shell still draws the UI and decides how to group and order windows.

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

![Bingux panel and dock with Files, Firefox and Foot in a Gnoblin session](images/gnoblin-bingux-firefox.png)

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

Read the socket protocol version from `hello.version`. Check `features` before
using an optional capability:

- Bare Super requires `overlay-shortcut`.
- `blur-region` requires `blur-regions`.
- `layer-animation-policy` requires the same-named feature.

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
| `activate`                       | `window`; optional `session` for the built-in switcher fallback          | Focus a window; no success reply                    |
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

When a client claims a built-in switcher shortcut, pass the matching `session`
from its `activated` event with `activate`. Gnoblin ignores an activation from
an older switcher session.

`command` supports these operations:

- Window records: `windows`, `capture-windows`.
- Workspaces: `workspaces`, `workspace-list`, `workspace-switch`,
  `workspace-next`, `workspace-previous`, `workspace-move-active`.
- `monitors` lists monitors; `layers` returns layer-shell surfaces.
- `window` takes an `action` and a stable window ID or `"active"`.
- `animation` lists, inspects and previews registered animations. See the
  [animation CLI guide](gnoblinctl.md#animations).

Successful `command` requests return an `event: "reply"` with the matching ID
and a `result` object. Read-only results use these shapes:

| Command           | Result field | Contents                                                                                                                 |
| ----------------- | ------------ | ------------------------------------------------------------------------------------------------------------------------ |
| `windows`         | `windows[]`  | The same fields as [`gnoblinctl window list --json`](gnoblinctl.md#output-for-scripts).                                  |
| `capture-windows` | `windows[]`  | `id`, `title`, `appId`, `appName`, frame `x`, `y`, `width`, `height`, and `bufferWidth`, `bufferHeight`.                 |
| `monitors`        | `monitors[]` | `id`, `x`, `y`, `width`, `height`, `primary`, and `scale`; see [monitor records](gnoblinctl.md#workspaces-and-monitors). |
| `layers`          | `surfaces[]` | Layer-shell `id`, `namespace`, and `title`; see [layer surfaces](gnoblinctl.md#layer-surfaces).                          |

The [CLI reference](gnoblinctl.md) lists window actions and arguments.

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

The request `id` correlates its reply. A configured workspace ID is assigned
from its initial position and stays with that `MetaWorkspace` if workspaces are
reordered during the session. An unconfigured workspace gets an ID such as
`@session-1`, which lasts only for that session.

Each item from `workspaces` has these fields:

| Field     | Meaning                                                            |
| --------- | ------------------------------------------------------------------ |
| `id`      | Configured ID or a generated session-only ID such as `@session-1`. |
| `number`  | Current one-based position.                                        |
| `name`    | Display label.                                                     |
| `active`  | Whether this workspace is selected.                                |
| `windows` | Number of eligible windows on the workspace.                       |

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

`capture-windows` returns visible, non-minimised windows in stacking order.
`bufferWidth` and `bufferHeight` follow the compositor paint box and can include
decoration shadows outside the frame dimensions.

Capture IDs come from Mutter; `windows` snapshots use a stable sequence
string. Get an action ID from `windows` or `gnoblinctl window list` before
sending a `window` action.

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

Put persistent command shortcuts, including media keys, in the [Lua config](/config/configure/shortcuts). A shell can use `bind` while it runs an interactive UI such as a switcher. Bridge bindings belong to that connection and disappear when it disconnects.

- `trigger` is `"press"` by default or `"release"` to wait until the accelerator's main key is released.
- Accelerator chords can include Alt, Control, Shift or Super. A held session supports Alt, Control or Super; Shift is not a valid `hold` value.
- Bare `"Super"` activates on release because Mutter first checks whether another key joins the chord.

```json
{ "op": "bind", "id": "example", "accelerator": "<Alt>F8", "hold": 8, "trigger": "release" }
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

The optional `modal` field defaults to `true`:

- Modal holds capture keyboard and pointer input for a popup. An Alt-held
  switcher can continue until Alt is released.
- `modal: false` leaves focus and input with the current app, while reporting
  modifier release. Use it to change shell state without opening an input UI.

This binding reports the `<Alt>F8` activation and later Alt release while the
focused application keeps receiving input:

```json
{ "op": "bind", "id": "cycle-mode", "accelerator": "<Alt>F8", "hold": 8, "modal": false }
```

Events sent during a binding session are:

| Event                   | Fields                                        | Meaning                                                                      |
| ----------------------- | --------------------------------------------- | ---------------------------------------------------------------------------- |
| `activated`             | `id`, `first`, `session`, `modifiers`, `time` | Accelerator fired.                                                           |
| `key`                   | `key`, `modifiers`                            | Key symbol and modifier mask for a captured key press.                       |
| `pointer`               | `x`, `y`, `button`                            | Captured button press at global logical-pixel coordinates; button 1 is left. |
| `released`, `cancelled` | `session`                                     | The modifier was released or the session ended early.                        |

- `first` is true when no binding session was already active.
- `session` is `0` for ordinary bindings and nonzero for the built-in switcher fallback.
- `modifiers` is a Clutter modifier mask; `time` is the event timestamp in milliseconds.
- `key` is the Clutter key symbol.

Hide the UI when `released` or `cancelled` arrives. Sending `{"op":"end"}`,
locking, disconnecting or reaching the ten-second timeout cancels the session.

## Bare Super and buffered typing

```json
{ "op": "bind", "id": "search", "accelerator": "Super", "hold": 0, "captureInput": true }
```

Requires the advertised `overlay-shortcut` feature. Only one bridge client can
own bare Super; remove a duplicate Lua command binding first.

Super activates on release, excluding chords. With capture enabled, send:

```json
{ "op": "shortcut-input", "name": "search", "state": "prepared" }
{ "op": "shortcut-input", "name": "search", "state": "ready" }
```

Use these states in order for an overlay that accepts typing:

| State      | Send it when                                                 | Effect                                                |
| ---------- | ------------------------------------------------------------ | ----------------------------------------------------- |
| `prepared` | The overlay exists and its text field has focus.             | Releases the compositor grab; input stays buffered.   |
| `ready`    | Its Wayland window is active and can receive keyboard input. | Replays buffered keys to the focused client in order. |
| `closed`   | The overlay closes or stops accepting input.                 | Cancels pending handoff and clears the ready state.   |

The buffer expires after three seconds if the handoff never becomes ready.
Do not send these states while the session is locked. Bingux uses this
handoff for its search overlay; another shell can send the same operations.
The binding ID is the handoff name.

## Windows and controls

| Request                            | Result                                    |
| ---------------------------------- | ----------------------------------------- |
| `{"op":"windows"}`                 | Subscribe to complete snapshots           |
| `{"op":"activate","window":"123"}` | End this client's grab and focus a window |
| `{"op":"status"}`                  | Registered IDs and active session         |
| `{"op":"end"}`                     | Cancel this client's input session        |
| `{"op":"clear"}`                   | Remove this client's bindings and session |

`windows` records include the CLI fields plus `workspace`, `workspaceId`,
`workspaceNumber`, `monitorIndex`, `maximized`, `fullscreen`, and `geometry`.
Their IDs are stable strings for the window's session lifetime.

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

`ui-session` shares advisory state and owner-directed commands between UI
processes. Names match `^[a-z][a-z0-9-]{0,63}$`; one client owns each name.

| Action    | Fields                 | Event                                       |
| --------- | ---------------------- | ------------------------------------------- |
| `watch`   | None                   | `ui-state` for each owner and later changes |
| `state`   | `name`, `state` object | Publishes `ui-state`                        |
| `command` | `name`, `command`      | Sends `ui-command` to the owner             |

`watch` immediately sends each current owner state, then sends updates. A
`state` request claims or updates a name and broadcasts its object to watchers.
A second client cannot claim that name. An owner disconnect removes it and
broadcasts `state: null`; commands are not queued for a later owner.

One client watches; another owns `search` and publishes its state. A watcher
can then send a command to that owner. Each request goes on the named client's
own bridge connection:

| Client  | Request                                                                               |
| ------- | ------------------------------------------------------------------------------------- |
| Watcher | `{"op":"ui-session","action":"watch"}`                                                |
| Owner   | `{"op":"ui-session","action":"state","name":"search","state":{"visible":true}}`       |
| Watcher | `{"op":"ui-session","action":"command","name":"search","command":{"action":"close"}}` |

The owner defines the command object's schema. Gnoblin forwards it to the
current owner as `ui-command`; if there is no owner, the command is ignored.
State is also owner-defined except for these optional stacking fields:

| Field              | Effect                                                                                  |
| ------------------ | --------------------------------------------------------------------------------------- |
| `visible`          | `true` requests the `surface` and its companions.                                       |
| `surface`          | Layer-shell namespace of the owning surface.                                            |
| `companions`       | Companion layer-shell namespaces; Gnoblin considers at most the first 16 string values. |
| `revealCompanions` | `true` requests companions even when `visible` is false.                                |
| `companionsAbove`  | `true` places companions above the owning surface; otherwise they go below it.          |

The owner must provide a string `surface` and an array `companions` for the
stacking request to apply. Bingux uses this to coordinate separate panels;
other shells can choose their own state and command objects.

| Operation                | Fields                                                                                    | Limit or effect                                                                                                                            |
| ------------------------ | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `blur-region`            | `namespace`, monitor origin `screen: [x,y]`, local `region: [x,y,width,height]` or `null` | Up to 64 per client; requires a matching blur window rule; cleared on disconnect. See [effect rendering](effects-rendering.md#blur-cache). |
| `layer-animation-policy` | `namespace`                                                                               | Returns `enter`, `exit`, `windowShadow`; namespace at most 128 characters                                                                  |

| Field    | Shape and accepted values                                                         | Meaning                                    |
| -------- | --------------------------------------------------------------------------------- | ------------------------------------------ |
| `screen` | `[x, y]`; finite coordinates from −65,536 to 65,536                               | Monitor origin in desktop coordinates.     |
| `region` | `null` or `[x, y, width, height]`; finite values from −65,536 to 65,536; size ≥ 0 | Local rectangle; `null` clears the region. |

Gnoblin matches the namespace and monitor origin for layer surfaces from the
requesting process. A matching blur window rule is still required.

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
