"""Example 34 in Python: workspaces and what lays their windows out.

    gnoblin ./shell.py

Makes two workspaces, ``main`` and ``side``, and adds both to the list; the windows that open join ``main``. The chords change the layout of
``main`` and show a workspace:

- ``Super+1`` to ``Super+7``: the library's own layouts, master and stack, grid, monocle, dwindle, strip, arc and carousel;
- ``Super+8``: a layout this shell implements itself (equal columns);
- ``Super+9`` and ``Super+0``: show ``side``, show ``main``;
- ``Super+MouseLeft+Drag``: a drag that moves the divider between two tiles. The press names a window, the workspace names the divider
  that window borders at that point (``find_split``), and the pointer's travel sets its ratio (``set_split``).

Every line starts with ``workspaces:``, as the Rust shell of the same example spells it.
"""

from gnoblin import gnoblin as session
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import (
    ArcLayout,
    CarouselLayout,
    DwindleLayout,
    Gaps,
    GridLayout,
    Layout,
    LayoutPlacement,
    LayoutRect,
    MasterStackLayout,
    MonocleLayout,
    StripLayout,
    Workspace,
    Workspaces,
)


class Columns(Layout):
    """Equal columns: the layout this shell implements."""

    def compute(self, context):
        area = context.area
        gap = context.outer_gap
        inner = (area.x + gap, area.y + gap, area.width - 2 * gap, area.height - 2 * gap)
        width = float(int(inner[2] / max(len(context.children), 1)))
        return [
            LayoutPlacement(
                node=node,
                bounds=LayoutRect(x=inner[0] + width * at, y=inner[1], width=width, height=inner[3]),
                tiled=True,
            )
            for at, node in enumerate(context.children)
        ]


# In the order of Super+1 to Super+8.
LAYOUTS = [
    ("master_stack", MasterStackLayout),
    ("grid", GridLayout),
    ("monocle", MonocleLayout),
    ("dwindle", DwindleLayout),
    ("strip", StripLayout),
    ("arc", ArcLayout),
    ("carousel", CarouselLayout),
    ("columns", Columns),
]


def say(text):
    session.log(f"workspaces: {text}")


main = Workspace("main")
main.name = "Main"
main.gaps = Gaps(inner=6.0, outer=12.0)
main.layout = MasterStackLayout()
side = Workspace("side")
side.name = "Side"
side.layout = MasterStackLayout()
workspaces = Workspaces.get()
workspaces.add(main)
workspaces.add(side)
say(f"the list holds {workspaces.length} workspaces: {main.id} at {main.index}, {side.id} at {side.index}")

clients = main.clients


def joined(node):
    say(f"client {node} joined {main.id}, {clients.length} windows")


clients.on("added", joined)

context = Gnoblin.get().input.primary


def choose(at):
    name, make = LAYOUTS[at]

    def run(event):
        main.layout = make()
        say(f"{main.id} has the {name} layout: true")
        return True

    return run


def show(workspace):
    def run(event):
        workspace.show()
        say(f"show {workspace.id}: true, active {str(workspace.active).lower()}")
        return True

    return run


for at in range(len(LAYOUTS)):
    context.bind(f"Super+{at + 1}", choose(at))
context.bind("Super+9", show(side))
context.bind("Super+0", show(main))

# The divider a drag moves, and where the press was.
held = {}


def drag(event):
    phase = event.phase.name.lower() if event.phase is not None else "none"
    x, y = event.position.x, event.position.y
    if phase == "start":
        hit = context.hit_test(event.position)
        # The window under the press; a layout whose divider does not depend on the point is asked about the first window.
        members = clients.members
        client = hit.client.id if hit is not None else (members[0] if members else None)
        split = main.find_split(client, x, y) if client is not None else None
        if split is None:
            say("the press is on no divider")
            return False
        say(f"divider {split.token} at {split.ratio}")
        held.update(split=split, ratio=split.ratio, cross=split.cross_ratio)
    elif phase == "update" and held:
        split = held["split"]
        # The drag's distance from the press, along the axis the divider moves on.
        travel = (event.dx or 0.0) if split.axis.name.lower() == "horizontal" else (event.dy or 0.0)
        ratio = min(max(held["ratio"] + split.direction * travel / split.span, split.min), split.max)
        cross = None
        if split.cross_axis is not None:
            across = (event.dx or 0.0) if split.cross_axis.name.lower() == "horizontal" else (event.dy or 0.0)
            cross = min(
                max(held["cross"] + split.cross_direction * across / split.cross_span, split.cross_min),
                split.cross_max,
            )
        main.set_split(split.token, ratio, cross)
    elif phase == "end" and held:
        say(f"divider {held.pop('split').token} dropped")
    elif phase == "cancel" and held:
        split = held.pop("split")
        main.reset_split(split.token)
        say(f"divider {split.token} put back")
    return True


context.bind("Super+MouseLeft+Drag", drag)
say("running")
