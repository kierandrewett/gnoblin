"""The built-in layouts of ``gnoblin.std`` from Python: one workspace and a chord for each tiling layout the library runs.

    gnoblin ./layouts.py

``Super+1`` is a default master and stack, ``Super+2`` a wider master area of two windows on the right with its own gap, ``Super+3`` an
automatic grid, ``Super+4`` a grid of three columns filled by column, ``Super+5`` a monocle and ``Super+6`` a master and stack with
the master on top. ``Super+Button1`` swaps a tiled window with the tile it is dropped on.

Every class here is generated from the library's table of layout kinds (``gnoblin-std-abi``'s ``LAYOUT_KINDS``): the options, their
defaults and their enumerations are the library's, and so is the algorithm. The shell only names a kind and its options.
"""

from gnoblin import gnoblin
from gnoblin.std import (
    GridFill,
    GridLayout,
    MasterStackLayout,
    MasterStackOrientation,
    MonocleLayout,
    SwapBinding,
    Workspace,
)

main = Workspace.create("main", gaps=(6.0, 12.0))
main.use(MasterStackLayout())

LAYOUTS = {
    "Super+1": lambda: MasterStackLayout(),
    "Super+2": lambda: MasterStackLayout(ratio=0.6, master_count=2, gap=4.0, orientation=MasterStackOrientation.RIGHT),
    "Super+3": lambda: GridLayout(),
    "Super+4": lambda: GridLayout(columns=3, fill=GridFill.COLUMN),
    "Super+5": lambda: MonocleLayout(),
    "Super+6": lambda: MasterStackLayout(orientation=MasterStackOrientation.TOP),
}

for chord, make in LAYOUTS.items():
    gnoblin.bind(chord)(lambda make=make: main.use(make()))

swap = SwapBinding().start()
gnoblin.log("layouts shell is running")
