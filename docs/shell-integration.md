# Shell integration notes

These APIs are for shell authors. Desktop configuration starts with
[the configuration guide](configuration.md).

## Dock animation targets

A dock can tell Gnoblin where each window's icon appears.
Minimise and restore animations then use that rectangle.

With Quickshell, call
[Toplevel.setRectangle](https://quickshell.org/docs/types/Quickshell.Wayland/Toplevel/)
using coordinates relative to the dock's PanelWindow:

```qml
function updateTarget(toplevel) {
    const point = icon.mapToItem(dock.contentItem, 0, 0);
    toplevel.setRectangle(dock,
        Qt.rect(point.x, point.y, icon.width, icon.height));
}
```

Update after layout changes and when a window joins a group.
Give each grouped window the same icon rectangle before minimising it.

A zero-size rectangle clears the hint. The hint also clears when its surface
or window handle disappears.

## Layer placement and animation

Gnoblin animates the compositor actor without resizing the client or changing
its exclusive zone. The assigned output stays fixed through entry and exit.

Layer resize handshakes keep the old buffer anchored until replacement content
arrives. Clients still need correct committed anchors and margins.

Use a namespace rule with `animation = "none"` when the client owns its
whole-surface transition. See [animations](animations.md).

## Input and window control

Use the [compositor bridge](compositor-bridge.md) for shortcuts and window state.
Use the [window-menu contract](window-menu.md#write-a-handler) for titlebar menus
and [snapping contract](window-snapping.md#shell-integration) for layout pickers.
