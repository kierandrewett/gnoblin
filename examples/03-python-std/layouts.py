"""The built-in layouts of ``gnoblin.std`` from Python: one workspace and a chord for each tiling layout the library runs.

    gnoblin ./layouts.py

``Super+1`` is a default master and stack, ``Super+2`` a wider master area of two windows on the right with its own gap, ``Super+3`` an
automatic grid, ``Super+4`` a grid of three columns filled by column, ``Super+5`` a monocle and ``Super+6`` a master and stack with
the master on top. ``Super+Button1`` swaps a tiled window with the tile it is dropped on.

Every class here is generated from the library's table of layout kinds (``gnoblin-std-abi``'s ``LAYOUT_KINDS``): the options, their
defaults and their enumerations are the library's, and so is the algorithm. The shell only names a kind and its options.
"""

from gnoblin import gnoblin as session
from gnoblin import Gnoblin
from gnoblin.std import (
    Gaps,
    GridFill,
    GridLayout,
    MasterStackLayout,
    MasterStackOrientation,
    MonocleLayout,
    SwapBinding,
    Workspace,
    Workspaces,
)

main = Workspace("main", gaps=Gaps(inner=6.0, outer=12.0), layout=MasterStackLayout())
Workspaces.get().add(main)

LAYOUTS = {
    "Super+1": lambda: MasterStackLayout(),
    "Super+2": lambda: MasterStackLayout(ratio=0.6, master_count=2, gap=4.0, orientation=MasterStackOrientation.RIGHT),
    "Super+3": lambda: GridLayout(),
    "Super+4": lambda: GridLayout(columns=3, fill=GridFill.COLUMN),
    "Super+5": lambda: MonocleLayout(),
    "Super+6": lambda: MasterStackLayout(orientation=MasterStackOrientation.TOP),
}


def switch(make):
    def run(event):
        main.layout = make()
        return True

    return run


context = Gnoblin.get().input.primary
for chord, make in LAYOUTS.items():
    context.bind(chord, switch(make))

SwapBinding().install()
session.log("layouts shell is running")
