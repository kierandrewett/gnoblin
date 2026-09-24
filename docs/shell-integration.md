# Integrate a desktop shell

These APIs are for shell authors. Desktop configuration starts with
[the configuration guide](/config).

## Dock animation targets

A dock can tell Gnoblin where each window's icon appears.
Minimise and restore animations then use that rectangle.

With Quickshell, call
[Toplevel.setRectangle](https://quickshell.org/docs/types/Quickshell.Wayland/Toplevel/)
using coordinates relative to the dock's PanelWindow:

```javascript
function updateTarget(toplevel) {
    const point = icon.mapToItem(dock.contentItem, 0, 0);
    toplevel.setRectangle(dock, Qt.rect(point.x, point.y, icon.width, icon.height));
}
```

Update after layout changes and when a window joins a group.
Give each grouped window the same icon rectangle before minimising it.

A zero-size rectangle clears the hint. The hint also clears when its surface
or window handle disappears.

## Layer placement and animation

During entry and exit animations, Gnoblin moves the displayed panel without
asking the client to resize its buffer. Space reserved for the panel (its
exclusive zone) stays unchanged, and it remains on the same monitor.

When a panel changes size, Gnoblin keeps its old buffer aligned to its chosen
edge until the client submits the new buffer. The client must still set its
Wayland anchors and margins correctly.

Use a namespace rule with `animation = "none"` when the client owns its
whole-surface transition. See [animations](/guides/animations).

## Input and window control

Use the [compositor bridge](compositor-bridge.md) for shortcuts and window state.
Use the [window-menu contract](/guides/window_menu#write-a-handler) for titlebar menus
and [snapping contract](/guides/window_snapping#shell-integration) for layout pickers.
