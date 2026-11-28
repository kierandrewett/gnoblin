"""Example 27 in Python: the window frame of libgnoblin-std, through the classes generated from its catalog.

    gnoblin ./shell.py

The decorations give every window a frame and call ``frame`` to fill it. A window with a server-drawn frame gets the stock titlebar; the
window titled ``close me`` is closed the way its close button closes it. Every line starts with ``frame:``, as the Rust shell of the same
example spells it.
"""

from gnoblin import gnoblin
from gnoblin.std import Decorations, Titlebar


def fill(client, frame):
    if not frame.server:
        gnoblin.log(f"frame: {client.app_id} draws its own frame")
        return True
    Titlebar.install(frame)
    gnoblin.log(f"frame: {client.app_id} has a titlebar and its top edge takes {frame.top.thickness}")
    if client.title == "close me":
        gnoblin.log(f"frame: closing {client.app_id} as its close button does")
        frame.actions.close()
    return True


decorations = Decorations.get()
decorations.frame = fill
decorations.start()
gnoblin.log("frame: running")
