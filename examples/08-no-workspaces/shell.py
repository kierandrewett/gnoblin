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

from gnoblin import gnoblin
from tiles import tiles


def work_area():
    """The primary output's rectangle: the first output of the compositor, or a default when it has none yet."""
    output = gnoblin.outputs.at(0)
    if output is not None:
        area = output.bounds
        if area.width > 0 and area.height > 0:
            return (area.x, area.y, area.width, area.height)
    return (0.0, 0.0, 1920.0, 1080.0)


# The window the user chose as the master, if one; a closed one is simply not found and the oldest leads again.
master = None


def retile():
    windows = list(gnoblin.windows)
    if master is not None and master in windows:
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
