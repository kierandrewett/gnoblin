"""Example 3: a Python shell on the classes generated from the catalog of ``libgnoblin-std``.

    gnoblin ./shell.py

The same shell as example 6 (a tiling layout in Rust), written in Python over the generated object API:

- ``Columns`` is a ``Layout``: given the clients of a workspace and its area, it answers with where each goes, one column each, the
  newest on the right, each edge floored to a whole pixel on its own. The library calls it whenever the workspace changes (a window
  joins or leaves, two swap places, the area changes) and writes what it answers, so the shell never positions a window itself;
- ``Workspace`` makes the one workspace, with a gap of 10 between tiles and 20 to the screen's edge, and ``Workspaces`` lists it;
- a listener on the collection of clients says where each window opens;
- ``Super+Tab`` swaps the window that has the keyboard with the first, which moves both columns without the layout being told how.

The layout is an ordinary object: it may keep state between calls, and the library runs it outside every lock of its own.
"""

import math

from gnoblin import gnoblin as session
from gnoblin import Gnoblin
from gnoblin.std import Gaps, Layout, LayoutPlacement, LayoutRect, Workspace, Workspaces


class Columns(Layout):
    """Equal-width columns across the workspace, separated by the workspace's inner gap."""

    def compute(self, context):
        count = len(context.children)
        if count == 0:
            return []
        area = context.area
        gap = context.inner_gap
        left = area.x + context.outer_gap
        top = math.floor(area.y + context.outer_gap)
        bottom = math.floor(area.y + area.height - context.outer_gap)
        width = max((area.width - 2 * context.outer_gap - gap * (count - 1)) / count, 0.0)
        placements = []
        for index, client in enumerate(context.children):
            start = math.floor(left + index * (width + gap))
            end = math.floor(left + index * (width + gap) + width)
            bounds = LayoutRect(x=float(start), y=float(top), width=float(end - start), height=float(bottom - top))
            placements.append(LayoutPlacement(node=client, bounds=bounds, tiled=True))
        return placements


gnoblin = Gnoblin.get()
main = Workspace("main", name="Main", layout=Columns(), gaps=Gaps(inner=10.0, outer=20.0))
Workspaces.get().add(main)


@gnoblin.clients.on("added")
def opened(client, collection):
    session.log(f"client {client.id} opened on {main.id}")


def swap_with_first(event):
    focused = next((client for client in gnoblin.clients.to_list() if client.focused), None)
    clients = main.clients
    if focused is None or clients is None:
        return False
    members = clients.members
    if not members:
        return False
    clients.swap(focused, members[0])
    return True


gnoblin.input.primary.bind("Super+Tab", swap_with_first)
session.log("python std shell is running")
