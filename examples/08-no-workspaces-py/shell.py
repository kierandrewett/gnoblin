"""Example 8 in Python: a shell with no workspaces and no scene graph.

    gnoblin ./shell.py

The windows of the tree are the whole model. The shell tiles them master-and-stack on the primary screen by writing each window's
rectangle, the window the user chose as the master first and the others in the order they opened, and it makes no node of its own:
no canvas, no camera, no view. The shell does not reorder the tree: the order of the windows is its own. There is no workspace because
there is nothing a workspace would hold that the tree does not.

- one window fills the screen; with more, the master takes the left half and the others split the right half;
- a window opening or closing tiles the rest again, in the same transaction;
- <Super>m makes the window that has the keyboard the master;
- <Super>Return starts a terminal.
"""

from gnoblin import Screen, gnoblin
from tiles import tiles


def work_area():
    """The primary screen's rectangle: the first screen of the tree, or a default when the compositor has none yet.

    The compositor publishes a screen's place and size as the node's own ``x``, ``y``, ``width`` and ``height``; a screen that leaves
    them at zero is read by its ``screen_*`` properties instead (the same rule as the Rust example)."""
    for node in gnoblin.root.children():
        if isinstance(node, Screen):
            if node.width > 0 and node.height > 0:
                return (node.x, node.y, node.width, node.height)
            return (node.screen_x, node.screen_y, node.screen_width, node.screen_height)
    return (0.0, 0.0, 1920.0, 1080.0)


# The window the user chose as the master, if one; a closed one is simply not found and the oldest leads again.
master = None


def retile():
    windows = list(gnoblin.windows)
    if master in windows:
        windows.remove(master)
        windows.insert(0, master)
    for window, (x, y, width, height) in zip(windows, tiles(work_area(), len(windows))):
        window.x, window.y, window.width, window.height = x, y, width, height


@gnoblin.on_client_created
@gnoblin.on_client_destroyed
def _changed(_window):
    retile()


@gnoblin.bind("<Super>m")
def promote():
    global master
    focused = next((window for window in gnoblin.windows if window.focused), None)
    if focused is not None:
        master = focused
        retile()


@gnoblin.bind("<Super>Return")
def terminal():
    gnoblin.spawn("foot")


retile()
