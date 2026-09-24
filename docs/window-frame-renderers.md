# Frame renderer architecture

Gnoblin owns decoration policy, geometry and input. An external process draws
the frame. The private v1 protocol is experimental.

Register a renderer using an absolute executable path or a command name on the
compositor's `PATH`; see the [configuration reference](/config/configure#frames).

For configuration, see [titlebars](/guides/window_frames).
For implementation steps, see [write a renderer](frame-renderer-api.md).

## Ownership

| Gnoblin                          | Renderer                 |
| -------------------------------- | ------------------------ |
| Decoration negotiation           | Frame appearance         |
| Crop and committed extents       | Buffers and damage       |
| Move/resize grabs and actions    | Button hit regions       |
| Content exclusion and final mask | Hover/pressed appearance |
| Native fallback                  | Theme interpretation     |

One process can render many frames. Gnoblin passes it a private Wayland
connection; only that connection sees the frame global. This limits protocol
capabilities, not the executable's OS access.

## Commit sequence

1. Gnoblin creates a frame handle with identity, state, actions and geometry.
2. The renderer attaches a surface with no existing role.
3. It acknowledges configure, declares regions and commits matching pixels.
4. Gnoblin applies the acknowledged state atomically.

Regions are double-buffered, limited to 64 and last-defined-wins.
A toolkit xdg-toplevel cannot take a second role.

The compositor excludes the app body from painting and input.
Native resize regions take priority. The renderer never takes keyboard focus
or requests arbitrary commands.

See the [protocol XML](https://github.com/kierandrewett/gnoblin/blob/main/src/protocols/window-frame/gnoblin-window-frame-v1.xml).

## Slow or failed renderers

App commits never wait for a renderer.
Geometry changes use native fallback until a matching buffer arrives;
stale generations cannot replace newer layout.

Invalid dimensions or serials disconnect the renderer.
Process failure triggers bounded backoff, with at most four launch attempts
per session. Shutdown terminates only Gnoblin's renderer children.

A stalled renderer may keep valid current pixels; native controls remain usable.
New geometry selects fallback. There is no continuous heartbeat.

The helper permits two outstanding shared-memory buffers per frame.
Its full-window canvases need further high-DPI and many-window cost measurements.

## Reference adapters

```sh
scripts/build-frame-renderers.sh
```

Outputs:

- `build/frame-renderers/gnoblin-frame-cairo`
- `build/frame-renderers/gnoblin-frame-qt`

Both accept `--theme-file=/absolute/path` containing a six-digit hex background.
Valid edits repaint; invalid edits retain the previous colour.

Their button layout is fixed. Native fallback separately supports Lua's
`button_layout`. Other toolkits need an adapter; ordinary toolkit windows
cannot be attached directly.

## Reload and tests

`gnoblinctl config reload` restarts configured services, even when an
executable's path is unchanged. Compositor upgrades require a new session.

Private tests cover pixels, crop, controls, fullscreen, theme reload and fallback.
Start with `tests/test-window-frames.py`; use `GNOBLIN_SSD_RENDERER=cairo`
or `qt` for adapters.

Mixed-scale transitions, popup-heavy cropped apps, adversarial fuzzing and
GPU-buffer adapters need further coverage.
