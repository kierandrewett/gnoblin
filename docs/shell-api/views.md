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

## Cameras

A View draws its source through the `Camera` its `camera` names. The camera's own position and rotation are its pose, and
it looks down negative `z`. Scene `y` grows down.

| `projection`   | The picture                                                                                                           |
| -------------- | --------------------------------------------------------------------------------------------------------------------- |
| `orthographic` | Flat. This is the default, and the View's drawing without a camera. Any other text also draws flat.                   |
| `perspective`  | Perspective with the vertical field `fov` in degrees (60 by default), or an off-axis frustum from the `fov_*` angles. |
| `stereo`       | Two perspective eyes side by side, the left eye in the left half, `ipd` metres apart (see `metres` below).            |

- `near` and `far` are the nearest and farthest depths drawn, in the camera's units.
- `fov_left`, `fov_right`, `fov_up` and `fov_down` are four angles in degrees for an off-axis frustum, as OpenXR states a
  field of view: `fov_left` and `fov_down` are negative for a symmetric view. All four zero, the default, means the
  symmetric frustum from `fov`. For `stereo` they describe the left eye, and the right eye uses the mirror image.
- `ipd` is the distance between a stereo camera's eyes in metres, 0.063 by default. `metres` is the camera's unit in
  metres, 0.001 by default, so the eyes are `ipd / metres` units apart.
- A frustum that is not one (`fov_left` not below `fov_right`, `fov_down` not below `fov_up`, an angle of 90 degrees or
  more), an `ipd` below zero, or a `metres` of zero or less draws a blank picture of the View's size. Nothing is
  refused: each property takes any number, and the picture is blank when the combination cannot be drawn.

A stereo picture is drawn like any other, one eye after the other, each into half the width: on the CPU on the software
renderer, and on the GPU as one texture with two viewports on GLES.

Redraw time on the software renderer of one full-size window seen from a window's height away, with a headset's field of
view (median of 15, release build, the same shared virtual machine as the tables below):

- 320x200: 0.9 ms. 640x360: 4.3 ms. 1280x720: 17 ms.
- 1920x1080: 65 ms. 3840x2160: 141 ms.

A stereo View of headset size needs an `fps` cap on the software renderer, as a large flat one does.

The XR extension shows a View with a stereo camera as an OpenXR projection layer; see
[XR mode](../xr.md#stereo-views).

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

`fps` is an unsigned whole number. A negative or fractional value is refused and the View keeps the value it had. On
the wire the code is `wrong-type`; in Lua it is a type error.

The clock has a resolution of one millisecond, so every `fps` of 1000 or more is the same slot. A View draws at most
once per millisecond, whether `fps` is 1000 or the largest number the wire carries. No value makes the compositor draw
more often than `0` does, or never.

## Where a View is drawn, and what a redraw costs

Where the picture of a View is made depends on the renderer the session runs.

| Renderer                                                | The View's picture                                                                           | A redraw costs the compositor's thread  |
| ------------------------------------------------------- | -------------------------------------------------------------------------------------------- | --------------------------------------- |
| GLES (the DRM backend, or `compositor.renderer = gles`) | A texture the renderer draws from a description of the source and camera. Nothing is copied. | Building the description: microseconds  |
| Software (and Vulkan)                                   | An image the compositor rasterises on the CPU, per pixel, on its own thread.                 | Time that grows with the pixels covered |

On the GLES path the compositor describes the source subtree through the camera, for a stereo camera one description
per eye, and the renderer draws it into a texture the size of the View. The View in the scene samples that texture.

The renderer keeps the texture until the View changes, is resized or is destroyed. A View that is destroyed gives its
texture back on the next frame, and a View that is shown on no output keeps its texture while something reads it.

The picture reaches the CPU only when something reads it: a capture session (`gnoblin_view_source_v1`, and so the XR
extension), a PipeWire stream or a `capture` still.

- The compositor reads the texture back once per redraw, only for a View that is being read.
- A request that arrives before the first read is answered in the same loop turn.
- That readback is the only CPU copy of a GPU View. A secure node is never in it: see [Secure nodes](#secure-nodes).

Two consequences:

- A View of a window that shows a GPU buffer (a DMA-BUF) shows the window's real pixels on the GLES path. The CPU path
  cannot read such a buffer and masks the window grey.
- A View whose picture cannot be drawn (the driver refuses its size, or the GL context is lost) shows nothing until its
  next change, and the rest of the frame is drawn as usual. It is asked again at its next redraw.

A View has a size limit on both paths: at most 8192 pixels on a side and 32 Mpx (a 7680x4320 picture fits). A View
outside it, or with a side of zero, has no picture, and nothing is drawn or captured for it.

### Measurements

Median of 15 redraws, release build, on a shared 4-vCPU virtual machine, one window covering the whole View.

The GLES numbers were taken on **llvmpipe**, which is a software GL driver. Its absolute times are CPU time spent
emulating a GPU and are **not** GPU numbers.

What the table shows is the split. The compositor's own thread does the first column's work on the CPU path and almost
none of it on the GLES path, and the readback is the only copy.

| View size | Camera | CPU path, on the compositor thread | GLES path, on the compositor thread | GLES draw (llvmpipe, not a GPU) | Readback, only when read |
| --------- | ------ | ---------------------------------- | ----------------------------------- | ------------------------------- | ------------------------ |
| 320x200   | flat   | 2.5 ms                             | 0.01 ms                             | 1.9 ms                          | 0.3 ms                   |
| 640x360   | flat   | 9.0 ms                             | 0.01 ms                             | 1.9 ms                          | 2.7 ms                   |
| 1280x720  | flat   | 36 ms                              | 0.02 ms                             | 6.1 ms                          | 4.0 ms                   |
| 1920x1080 | flat   | 82 ms                              | 0.02 ms                             | 14 ms                           | 5.0 ms                   |
| 3840x2160 | flat   | 353 ms                             | 0.02 ms                             | 60 ms                           | 8.7 ms                   |
| 1280x720  | stereo | 16 ms                              | 0.02 ms                             | 5.2 ms                          | 5.4 ms                   |
| 1920x1080 | stereo | 36 ms                              | 0.02 ms                             | 12 ms                           | 8.0 ms                   |
| 3840x2160 | stereo | 165 ms                             | 0.02 ms                             | 58 ms                           | 22 ms                    |

The software renderer's table, from `view_cost.rs` (about 40 to 60 nanoseconds a pixel, ten windows at 90% opacity
costing about two and a half times one window) is unchanged and still holds for a session on the software renderer.

A 60 Hz frame is 16.7 ms. On the software renderer a View of up to about 640x360 redraws inside one frame, and a larger
View that is redrawn at every change of a live source makes the frames that carry the redraw late. To keep a large View
cheap there:

- Cap it with `fps`. A 720p View of a window-sized source takes about 40 ms, so `5` spends a fifth of the thread.
- Keep the View small.
- Write a still with `capture` instead of keeping a live View.

On GLES a large live View does not stall the compositor thread. A View that is also streamed or captured at every
change pays the readback each time, so cap that one with `fps` too. The measurement is `cargo test --release -p
gnoblin-backend --lib the_cost_of_one_view_redraw -- --ignored --nocapture` for both paths, and `cargo test --release -p
gnoblin-render-software --test view_cost -- --ignored --nocapture` for the CPU rasteriser alone.

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
- Destroying the View's `source` does not end the stream, because the View is still there and the shell may give it
  another source. From the next frame the View has no picture, so the stream carries none of the destroyed source's
  pixels (its buffers arrive empty, with a zero-length chunk, and nothing stale is repeated), and the node and the
  privacy indicator stay up. A new `source` makes the stream carry that source's pixels again, on the same session.
- A consumer that stops reading does not slow the compositor. The compositor drives the stream with a turn of its
  PipeWire connection that never waits, and a frame it cannot place because the consumer holds every buffer is skipped.
  A stalled consumer keeps its node and its session until it reads again, when frames resume. The View's own redraws
  and the output's frames do not wait for it.

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
