# FIXTURE, not an example: example 3 as it was before it moved to the generated object API (`examples/03-python-std`). It is kept
# because `gnoblin.std`, the typed-table binding over `gnoblin._std_base`, still ships in the Python runtime, and this shell is what
# proves it there: the same rectangles as the native Rust shell. Delete it with `gnoblin.std`.

"""Example 3: a Python shell on ``gnoblin.std``.

    gnoblin ./shell.py

The same shell as example 6 (a tiling layout in Rust), written in Python over the generated bindings of ``libgnoblin-std``:

- ``Columns`` is a ``Layout``: given the clients of a workspace and its area, it answers with where each goes, one column each, the
  newest on the right. The library calls it whenever the workspace changes (a window joins or leaves, two swap places, the area changes)
  and writes what it answers, so the shell never positions a window itself;
- ``Workspace.create`` makes the one workspace, with a gap of 10 between tiles and 20 to the screen's edge;
- a listener on the collection of clients says where each window opens;
- ``Super+Tab`` swaps the window that has the keyboard with the first, which moves both columns without the layout being told how.

The layout is an ordinary object: it may keep state between calls, and the library runs it outside every lock of its own.
"""

from gnoblin import gnoblin
from gnoblin.std import EventKind, Layout, LayoutContext, Placement, Workspace, clients


class Columns(Layout):
    """Equal-width columns across the workspace, separated by the workspace's inner gap."""

    def compute(self, cx: LayoutContext) -> list[Placement]:
        count = len(cx.children)
        if count == 0:
            return []
        area = cx.inset_area()
        width = max((area.width - cx.gap_inner * (count - 1)) / count, 0.0)
        return [
            Placement.tile(client, area.x + index * (width + cx.gap_inner), area.y, width, area.height)
            for index, client in enumerate(cx.children)
        ]


main = Workspace.create("main", name="Main", layout=Columns(), gaps=(10.0, 20.0))


def opened(event) -> None:
    gnoblin.log(f"client {event.member} opened on {main.info().id}")


clients().on(EventKind.ADDED, opened)


@gnoblin.bind("Super+Tab")
def swap_with_first() -> None:
    focused = next((client for client in clients() if client.window.focused), None)
    first = main.clients().at(0)
    if focused is not None and first is not None:
        main.clients().swap(focused.node, first)


gnoblin.log("python std shell is running")
