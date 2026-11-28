"""Example 39 in Python: why a call failed, and which call it was.

    gnoblin ./shell.py

Does what the Rust shell of the same example (``39-native-errors``) does and logs the same lines, which start with ``errors:``. A call the
library refuses raises a ``GnoblinError`` with a ``code`` (``already_exists``, ``not_found``) and a ``target``, the object and member of the
call (``Client#7.close``). A bad argument is not one: it raises a ``TypeError`` and has no code.
"""

from gnoblin import gnoblin as session
from gnoblin.errors import GnoblinError
from gnoblin import Gnoblin, InputContext


def say(text):
    session.log(f"errors: {text}")


def refused(error):
    if isinstance(error, GnoblinError):
        return f"code={error.code} target={error.target}"
    return f"not a GnoblinError: {error}"


gnoblin = Gnoblin.get()
input = gnoblin.input
if input is None:
    say("the compositor has no input")
else:
    # The first context takes the name; a second with the same name is refused.
    input.add_context(InputContext("errors"))
    try:
        input.add_context(InputContext("errors"))
        say("a taken name was accepted")
    except Exception as error:
        say(f"a taken name is refused: {refused(error)}")

    # A context with no name is a bad argument: a TypeError, which is not a GnoblinError and has no code.
    try:
        input.add_context(InputContext(""))
        say("an empty name was accepted")
    except Exception as error:
        say(
            f"a bad argument is refused: TypeError={str(isinstance(error, TypeError) and not isinstance(error, GnoblinError)).lower()}"
        )


# A window the shell has been told of is always told closed. The model reads the tree a batch at a time, so a window that opens and
# closes within a single batch is told of neither way.
@gnoblin.on("client_created")
def created(client, collection):
    say("a window appeared")


# A window that has closed is still a ``Client``; a call on it says it is gone.
@gnoblin.on("client_closed")
def closed(client):
    try:
        client.close()
        say(f"{client.title}: a closed window was answered")
    except Exception as error:
        say(f"{client.title}: a closed window is refused: {refused(error)}")


say("running")
