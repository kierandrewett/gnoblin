"""Example 30 in Python: pointer drags, clicks and touchpad swipes on the input context of libgnoblin-core.

    gnoblin ./shell.py

The classes of the ``gnoblin.model`` catalog are generated from the catalog the library serves. This shell does what the Rust shell of the
same example (``30-native-pointer``) does, and logs the same lines:

- binds ``Super+MouseLeft+Drag`` with one function for the whole drag. It is called with ``phase`` ``START`` (the pointer is where the press
  was; answer ``False`` to leave the drag to the next binding), then ``UPDATE`` for each move, then ``END`` when the button is let go, or
  ``CANCEL`` when the window under the drag closes, the binding is removed, or the shell is replaced while the drag is going;
- binds ``Super+MouseRight`` for a click;
- binds ``Swipe3Left``. A function runs once, when the swipe ends having gone far enough.
"""

from gnoblin import gnoblin as session
from gnoblin import Gnoblin

# Which generation of the shell this is. A reload test saves the file again with another number.
GENERATION = 1


def say(text):
    session.log(f"pointer: {text}")


context = Gnoblin.get().input.primary


def drag(event):
    phase = event.phase.name.lower() if event.phase is not None else "none"
    say(f"generation {GENERATION} drag {phase} dx={event.dx or 0.0:g} dy={event.dy or 0.0:g}")
    return True


def click(event):
    say(f"generation {GENERATION} click")
    return True


def swipe(event):
    say(f"generation {GENERATION} swipe left, {event.progress or 0.0:.2f} of the way")
    return True


context.bind("Super+MouseLeft+Drag", drag)
context.bind("Super+MouseRight", click)
context.bind("Swipe3Left", swipe)

say(f"generation {GENERATION} holds {len(context.claims())} claims for {len(context.describe_bindings())} bindings")
say(f"generation {GENERATION} is running")
