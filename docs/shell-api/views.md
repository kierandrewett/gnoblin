# Views

A `View` draws another node's subtree as a picture.

- `source` is the node to draw.
- `camera` is the camera node to draw it through. Without one, the View is orthographic.
- `width` and `height` are the picture's size.

The compositor draws the picture where the View sits. A `view`-role client reads the same picture over
`gnoblin_view_source_v1`, `capture` writes it to a file, and the portal's ScreenCast can stream it over PipeWire
(see [Casting a View](#casting-a-view)).

```lua
local preview = View({ width = 320, height = 200, source = window, fps = 10 })
root:add(preview)
```

## `fps`

`fps` caps how often the compositor redraws the View's picture. It is a cap, not a rate. A View whose source does not
change is never redrawn, whatever its `fps`.

| `fps` | The View is redrawn                                                                                           |
| ----- | ------------------------------------------------------------------------------------------------------------- |
| `0`   | on every change of its source, its camera or itself. This is the default, which the schema calls "on demand". |
| `n`   | at most every `ceil(1000 / n)` milliseconds. At `10` that is every 100 ms; at `60`, every 17 ms.              |

Changes inside a slot are coalesced. The View keeps its picture, and when the slot ends it is drawn once, from the
state the source is in then.

Two cases skip the wait. A View with no picture yet, or whose size changed, is drawn at once, so a picture of the wrong
size is never kept. Lowering `fps` to `0` draws any change that was waiting on the next frame.

The slots are measured on the session's clock, which the loop advances once per turn. A redraw happens on the first
turn at or after the end of the slot.

## Casting a View

A View is a screen cast source, beside the outputs. The stream carries the View's capture picture, the same one
`capture` writes and `gnoblin_view_source_v1` reads, so a secure node is masked in it (see
[Secure nodes](#secure-nodes)). A View has no cursor, so `cursor_mode` has no effect on it.

- **Size.** The stream is the size the View is drawn at. When the shell resizes the View, the stream is negotiated
  again at the new size, and a consumer receives whole pictures of it.
- **Rate.** A new picture arrives no faster than `fps` allows, because the stream carries the picture the View drew
  last. While the View is idle the stream repeats its picture, at up to 30 frames a second, so a consumer that attaches
  late has one at once.
- **Format.** `BGRA` in memory (the stream's `argb8888`), one memfd plane per buffer. There is no DMA-BUF.

### Asking for one

The portal API cannot name a node, so Gnoblin uses its own key. An app calls `ScreenCast.SelectSources` with `types`
set to the virtual source bit (4) and a `view` option, a `t` holding the View's node id. The shell that made the View
knows the id. The frontend forwards options it does not know.

```text
SelectSources(session, { "types": <u 4>, "view": <t 4294967554> })
Start(session, parent_window, {})       -> the user is asked, then the node is answered
```

- Only a caller the frontend says is not sandboxed may share a View. A sandboxed or unidentified caller's `types` is
  narrowed to monitors, and it is given the first output, the same narrowing a window share gets.
- A virtual source with no `view` is refused at `SelectSources` (`InvalidArgs`), before the user is asked.
- The user is asked as for any share. A refusal is the response code `1`, and no stream is made.
- A View that is not in the scene when the stream starts is refused after the user's yes: `NotSupported`, and the
  compositor's reason is `no-view`.

The portal sends the compositor `cast.grant {session, source}` with the View as the source, then `cast.start`. The
compositor starts no stream for a session it was not told the user approved (`no-consent`).

### Privacy, lock and lifetime

- A stream is a session in the capture table, so the count the privacy indicator reads is one higher from the moment
  `cast.start` answers until the stream ends.
- Starting and ending a stream, and every refusal, leave an audit record in the log (`area = "capture"`).
- Nothing is started while the session is locked (`locked`). A stream that is running when the lock goes up ends at
  once, whether or not anything reads it, and does not come back when the lock is released: the portal makes a new
  session and the user is asked again. An output stream may keep showing the lock scene; a View stream may not.
- Destroying the View ends its stream, and the node leaves PipeWire. Closing the portal session, or the portal's
  connection going away, ends it too.

## `capture(path)`

`view:capture(path)` writes a PNG of what the View shows to `path`. It returns a signal that fires with the path
written, or with an error code if the compositor refused.

```lua
go(function()
    local written, err = await(preview:capture(os.getenv("XDG_RUNTIME_DIR") .. "/preview.png"))
    if err ~= "" then
        report("capture failed: " .. err)
    end
end)
```

### Paths

The path must meet all of these rules:

- It is absolute and has no `.` or `..` parts.
- It ends in `.png`.
- Its directory already exists inside the session's runtime directory, `$XDG_RUNTIME_DIR`, which is the directory of the
  Wayland socket.
- No link at the target or in the directory chain leaves the runtime directory.

The compositor writes the file under a name of its own and renames it over the target, with mode `0600`. A reader
never sees a half-written picture.

### Errors

| Error         | Meaning                                                                            |
| ------------- | ---------------------------------------------------------------------------------- |
| `locked`      | The session is locked, or the lock embargoes capture. Nothing was read or written. |
| `unavailable` | The View has no picture, or the session has no runtime directory.                  |
| `bad-value`   | The path is not one the compositor will write.                                     |
| `too-large`   | The picture does not fit the encoder's limit.                                      |
| `io-error`    | The file could not be written.                                                     |

A View has no picture when it has no `source`, or its source cannot be drawn.

### Privacy

A capture is visible. While it runs it is a session in the compositor's capture table, so the count of open capture
sessions that the privacy indicator reads is one higher. The count rises before the picture is read and falls after
the file is written.

Each capture leaves an audit record in the log, with the View, the path and the outcome.

### Secure nodes

A node whose `secure` is true is never captured. The View still shows it on screen.

- The capture picture (the PNG `capture` writes, and every frame of a `gnoblin_view_source_v1` session) holds a neutral
  grey rectangle in the node's place, the same thing a capture of an output draws over a secure window.
- Everything below a secure node is secure with it, so a window under a secure parent is masked though the window is
  not marked.
- With `capture.secure-treatment = "omit"` the node is left out of the picture instead, and what lies beneath it in the
  View's source shows.
- The mark is read every frame: a node that becomes secure is masked in the next picture, whatever the View's `fps`.
- A capture of an output that shows a View with a secure node in its source replaces the whole View, so the portal's
  screenshot and screen cast never carry the node's pixels through a View either.
- A View of a View draws no picture of the inner View, so no capture of it can hold what the inner View's source holds.

### Not covered

A View has no cursor, so `capture` takes no cursor option. A client that wants frames continuously uses
`gnoblin_view_source_v1` or the portal, not this call.
