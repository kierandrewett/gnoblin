"""Example 8 in Python: a shell with no workspaces and no scene graph.

    gnoblin ./shell.py

The windows of the tree are the whole model. The shell tiles them master-and-stack on the primary screen by writing each window's
rectangle, in the order the compositor stacks them (the first window of the root's children is the master), and it makes no node of
its own: no canvas, no camera, no view. There is no workspace because there is nothing a workspace would hold that the tree does not.

- one window fills the screen; with more, the master takes the left half and the others split the right half;
- a window opening or closing tiles the rest again, in the same transaction;
- Super+m makes the window that has the keyboard the master;
- Super+Return starts a terminal.
"""

from gnoblin import Screen, gnoblin
from tiles import tiles


def work_area():
    """The primary screen's rectangle: the first screen of the tree, or a default when the compositor has none yet."""
    for node in gnoblin.root.children():
        if isinstance(node, Screen):
            return (node.screen_x, node.screen_y, node.screen_width, node.screen_height)
    return (0.0, 0.0, 1920.0, 1080.0)


def retile():
    windows = gnoblin.windows
    for window, (x, y, width, height) in zip(windows, tiles(work_area(), len(windows))):
        window.x, window.y, window.width, window.height = x, y, width, height


@gnoblin.on_client_created
@gnoblin.on_client_destroyed
def _changed(_window):
    retile()


@gnoblin.bind("Super+m")
def promote():
    # The master is the first of the root's children: lowering a window to the bottom of the stack makes it the first.
    focused = next((window for window in gnoblin.windows if window.focused), None)
    if focused is not None:
        focused.insert_into(gnoblin.root, 0)
        retile()


@gnoblin.bind("Super+Return")
def terminal():
    gnoblin.spawn("foot")


retile()
