"""Example 28 in Python: the input matcher of libgnoblin-core.

    gnoblin ./shell.py

The classes of the ``gnoblin.model`` catalog are generated from the catalog the library serves. This shell does what the Rust shell of the
same example (``28-native-input``) does, and logs the same lines:

- binds ``Super+a`` twice: a binding of priority 5 that declines the press (it answers ``False``) and a binding of priority 0 that handles
  it, so a press runs the first and then the second;
- binds ``Super+b`` with a binding that says which generation of the shell handles it, so a reload that hands the chord to the next
  generation is seen.
"""

from gnoblin import gnoblin as session
from gnoblin.objects.model import BindingOptions, Gnoblin

# Which generation of the shell this is. A reload test saves the file again with another number.
GENERATION = 1


def say(text):
    session.log(f"input: {text}")


context = Gnoblin.get().input.primary


def high(event):
    say("high declines Super+a")
    return False


def low(event):
    say("low handles Super+a")
    return True


def chord(event):
    say(f"generation {GENERATION} handles Super+b")
    return True


context.bind("Super+a", high, BindingOptions(id="high", priority=5.0))
context.bind("Super+a", low, BindingOptions(id="low"))
context.bind("Super+b", chord)

say(f"generation {GENERATION} holds {len(context.claims())} claims for {len(context.describe_bindings())} bindings")
say(f"generation {GENERATION} is running")
