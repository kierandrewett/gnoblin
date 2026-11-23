"""Example 26 in Python: workspaces and layouts of libgnoblin-std, through the classes generated from its catalog.

    gnoblin ./shell.py

Makes a workspace with the library's master and stack layout. When the third window joins, the workspace gets a layout this shell
implements (equal columns); when the fourth joins it goes back to the library's layout and the shell moves the divider as a drag would
(``find_split``, ``set_split``). Every line starts with ``objects:``, as the Rust shell of the same example spells it.
"""

from gnoblin import gnoblin
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import (
    Desks,
    Gaps,
    Layout,
    LayoutPlacement,
    LayoutRect,
    MasterStackLayout,
    Workspace,
    Workspaces,
)


class Columns(Layout):
    def compute(self, context):
        area = context.area
        gap = context.outer_gap
        width = float(int((area.width - 2 * gap) / max(len(context.children), 1)))
        return [
            LayoutPlacement(
                node=node,
                bounds=LayoutRect(
                    x=area.x + gap + width * at, y=area.y + gap, width=width, height=area.height - 2 * gap
                ),
                tiled=True,
            )
            for at, node in enumerate(context.children)
        ]


main = Workspace("main")
main.gaps = Gaps(inner=6.0, outer=12.0)
main.layout = MasterStackLayout()
Workspaces.get().add(main)
gnoblin.log(f"objects: the workspace {main.id} is in the list at index {main.index}")

phase = [0]
clients = main.clients
desks = Desks.get()


def on_added(node):
    count = clients.length
    gnoblin.log(f"objects: client {node} joined main, {count} windows")
    if count >= 3 and phase[0] == 0:
        phase[0] = 1
        main.layout = Columns()
        gnoblin.log(f"objects: a layout of the shell's own tiles {count} windows: True")
    elif count >= 4 and phase[0] == 1:
        phase[0] = 2
        main.layout = MasterStackLayout()
        gnoblin.log(f"objects: back to the library's layout with {count} windows: True")
    elif count >= 5 and phase[0] == 2:
        # The library has laid the four windows out by now, so the divider is there to be found.
        phase[0] = 3
        # The divider is asked of a window: the `Client` of the first member, which the core looks up by its id.
        area = desks.areas().area
        members = clients.members
        first = Gnoblin.get().clients.get(members[0]) if members else None
        if area is None or first is None:
            gnoblin.log("objects: cannot find the divider")
            return
        x = area.x
        while x < area.x + area.width:
            split = main.find_split(first, x, area.y + area.height / 2)
            if split is not None:
                to = min(max(0.7, split.min), split.max)
                main.set_split(split.token, to)
                gnoblin.log(f"objects: the divider is at {split.ratio} and moves to {to}: True")
                return
            x += 25
        gnoblin.log("objects: no divider found")


clients.on("added", on_added)
gnoblin.log("objects: running")
