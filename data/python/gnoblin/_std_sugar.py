"""The hand-written spellings over the generated ``gnoblin.std``: ``len(collection)``, ``client.window``, ``clients()``.

``std.py`` is the table, typed. What a Python reader expects of a collection or a client that the table has no name for is here,
and nothing else: no behaviour of the desktop lives in this file.
"""

from __future__ import annotations

import math
from typing import Any, Iterator

from . import _std_base as _b
from ._base import Node

# The error a floor of float arithmetic is allowed: the right edge of 619.2 + 404.8 is 1023.9999999999999.
EDGE_EPSILON = 1e-6


def install(namespace: dict[str, Any]) -> None:
    """Adds the spellings to the classes of ``gnoblin.std``, and the functions that name its roots."""
    Collection = namespace["Collection"]
    Client = namespace["Client"]
    Output = namespace["Output"]
    Workspace = namespace["Workspace"]
    CollectionKind = namespace["CollectionKind"]

    def members(self: Any) -> Iterator[Any]:
        """The members as the class of their kind: clients, outputs or workspaces."""
        kind = self.kind()
        for member in self.members():
            if kind == CollectionKind.CLIENTS:
                yield Client(member)
            elif kind == CollectionKind.OUTPUTS:
                yield Output(member)
            else:
                yield Workspace(member)

    Collection.__len__ = lambda self: self.len()
    Collection.__iter__ = members
    Collection.__contains__ = lambda self, member: self.contains(member)
    Collection.items = members

    def window(self: Any) -> Node:
        """The window of the tree this client is."""
        return Node._of(self.node)

    Client.window = property(window)

    def clients() -> Any:
        """Every client of the compositor, in the order they appeared."""
        return Collection.root(CollectionKind.CLIENTS)

    def outputs() -> Any:
        """Every connected output."""
        return Collection.root(CollectionKind.OUTPUTS)

    def workspaces() -> Any:
        """The workspaces the shell added, in order."""
        return Collection.root(CollectionKind.WORKSPACES)

    Placement = namespace["Placement"]
    PlacementFlags = namespace["PlacementFlags"]
    Rect = namespace["Rect"]
    LayoutContext = namespace["LayoutContext"]
    LayoutSpec = namespace["LayoutSpec"]
    WorkspaceSpec = namespace["WorkspaceSpec"]
    LayoutKind = namespace["LayoutKind"]

    def at(self: Any, index: int) -> Any:
        """The member at ``index``, or ``None``."""
        found = self.members()
        return found[index] if -len(found) <= index < len(found) else None

    def on(self: Any, event: Any, callback: Any, replay: bool = False) -> Any:
        """Calls ``callback`` for ``event`` of the collection; ``replay`` tells it of every member that is there already."""
        return self.listen(event, 1 if replay else 0, callback)

    Collection.at = at
    Collection.on = on

    def tile(node: Any, x: float, y: float, width: float, height: float) -> Any:
        """A tiled placement of ``node`` at whole pixels.

        Each of the four edges is floored on its own, and the size is the distance between two floored edges, so a master of 607.2
        and a stack of 404.8 that starts at 619.2 are 607 and 404 wide, and the stack still ends at 1023 in an area that ends at 1024.
        This is the rule of the Rust SDK's ``Placement::tile`` and of the TypeScript library's tiling layouts.
        """
        left = math.floor(x + EDGE_EPSILON)
        top = math.floor(y + EDGE_EPSILON)
        return Placement(
            flags=PlacementFlags.BOUNDS | PlacementFlags.TILED,
            node=node,
            bounds=Rect(
                float(left),
                float(top),
                float(math.floor(x + width + EDGE_EPSILON) - left),
                float(math.floor(y + height + EDGE_EPSILON) - top),
            ),
        )

    Placement.tile = staticmethod(tile)

    def inset_area(self: Any) -> Any:
        """The workspace's area less the outer gap on every side: where a tiling layout works."""
        gap = self.gap_outer
        area = self.area
        return Rect(area.x + gap, area.y + gap, area.width - 2 * gap, area.height - 2 * gap)

    LayoutContext.inset_area = inset_area

    def create(
        cls: Any,
        id: str,
        *,
        name: str = "",
        layout: Any = None,
        gaps: tuple[float, float] = (0.0, 0.0),
    ) -> Any:
        """Makes a workspace, gives it the layout (an object with ``compute``, or ``None`` for floating) and adds it to the list."""
        spec = WorkspaceSpec(
            id=id,
            name=name,
            gap_inner=gaps[0],
            gap_outer=gaps[1],
            layout=None if layout is None else LayoutSpec(kind=LayoutKind.FOREIGN, vtable=layout),
        )
        workspace = cls.new(spec)
        workspace.add()
        return workspace

    Workspace.create = classmethod(create)

    _b.BuiltinBinding.registration_class = namespace["Registration"]

    def use(self: Any, layout: Any) -> Any:
        """Gives the workspace a built-in layout (``MasterStackLayout(ratio=0.6)``) and returns it, bound to this workspace."""
        return layout.attach(self)

    Workspace.use = use

    namespace["clients"] = clients
    namespace["outputs"] = outputs
    namespace["workspaces"] = workspaces
