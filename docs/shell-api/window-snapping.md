# Window Snapping

Build and commit a window frame using a trusted focus context.

Window snapping is available from a trusted event callback in a Gnoblin session.

## Pointer and keyboard snapping

Lua pointer callbacks receive a read-only `event.drag` record on the started
and updated events. It contains window and drag IDs, settings revision, pointer
coordinates, modifiers, monitor and work-area rectangles, the window frame,
and maximized state.

Call `event.drag:offer_targets(targets)` during the callback. An offer can
contain 1 to 128 targets. Each target has:

- a unique `id`;
- pointer `hit` and destination `frame` rectangles;
- optional `maximize`, `required_modifiers`, and `forbidden_modifiers` fields.

Rectangles use integer logical coordinates and `{x, y, width, height}` fields.
Both must fit in the current work area. Modifier arrays accept only `control`,
which cannot be both required and forbidden.

On release, Mutter checks the latest accepted offer against the actual pointer
and modifiers. A match applies its frame; otherwise Mutter completes the
normal move or tile-preview behavior. The shell owns snap guides and other
presentation.

Drag records expire on release, lock, config reload, window loss, or compositor
teardown. An old runtime cannot replace an offer after reload.

This callback offers a right-half target while a window is being dragged. Put it
in a loaded Lua config file; the shell can draw a matching guide while the drag
is active:

```lua
gnoblin.on("gnoblin.window.drag.started", function(event)
    local area = event.drag.work_area
    local half = math.floor(area.width / 2)

    event.drag:offer_targets({
        {
            id = "right-half",
            hit = {x = area.x + area.width - 32, y = area.y, width = 32, height = area.height},
            frame = {x = area.x + half, y = area.y, width = area.width - half, height = area.height},
        },
    })
end)
```

Socket clients can offer pointer snap targets through the compositor bridge.
Lua callbacks use `event.drag:offer_targets()` as described above.

### Keyboard snapping

For keyboard-selected layouts, call `gnoblin.windows.snap_context(context)` in
a trusted shortcut callback. It consumes the one-use focus context and returns
an `Operation<SnapContext>` for the focused window; callers cannot provide a
window ID.

Commit once with a monitor ID and frame rectangle. Gnoblin checks the window,
monitor, lock state, runtime generation, and work-area bounds again. The
operation resolves to `{window_id, monitor_id, committed = true}`. The context
expires with its source focus context and is revoked by lock or config reload.

The context is available only in a trusted shortcut callback. It expires
after five seconds and is revoked by session lock or config reload. See the
[compositor bridge](/bridge/) for the socket contract.
