"""Example 7 in Python: a custom input binding written entirely in Python.

    gnoblin ./shell.py

A binding is a function: the chord it listens for and what a press does. This one nudges the focused window (or the first one when
none has focus) by a fixed step. There is no Rust or C in this example; the shell is this file.
"""

from gnoblin import gnoblin


def nudge(dx, dy):
    def press():
        target = next((window for window in gnoblin.windows if window.focused), None)
        if target is None and gnoblin.windows:
            target = gnoblin.windows[0]
        if target is not None:
            target.x, target.y = target.x + dx, target.y + dy

    return press


gnoblin.bind("Super+Right", nudge(100, 0))
gnoblin.bind("Super+Down", nudge(0, 100))
