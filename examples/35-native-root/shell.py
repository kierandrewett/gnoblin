"""Example 35 in Python: the members of the ``gnoblin`` root object of libgnoblin-core.

    gnoblin ./shell.py

The classes of the ``gnoblin.model`` catalog are generated from the catalog the library serves. This shell does what the Rust shell of the
same example (``35-native-root``) does, and logs the same lines:

- reads the build identity with ``version()``;
- sets a timer with ``timers.after`` and cancels another;
- lists the windows and the outputs with ``find``, and refuses a kind the core does not list and a field a window has no filter for, saying
  what works;
- when a window appears, reads ``motion(200)`` and ``compositor.user_shaders``.
"""

from gnoblin import gnoblin as session
from gnoblin.objects.model import Gnoblin


def say(text):
    session.log(f"root: {text}")


gnoblin = Gnoblin.get()
version = gnoblin.version()
say(f"gnoblin {version.gnoblin} speaks api {version.api}")

started = gnoblin.timers.now()
fired = gnoblin.timers.after(
    40, lambda: say(f"the timer fired after its time: {str(gnoblin.timers.now() - started >= 39).lower()}")
)
never = gnoblin.timers.after(10, lambda: say("a cancelled timer ran"))
never.cancel()
say(f"a cancelled timer is pending: {str(never.pending).lower()}")

try:
    gnoblin.find("Workspace")
except TypeError as error:
    say(f"an unknown kind is refused: {error}")
try:
    gnoblin.find("Window", {"no_such_field": 1})
except TypeError as error:
    say(f"an unknown field is refused: {error}")


@gnoblin.on("client_created")
def created(client, collection):
    windows = len(gnoblin.find("Window"))
    foot = len(gnoblin.find("Window", {"app_id": "foot"}))
    titled = len(gnoblin.find("Window", lambda found: found.title.startswith("Term")))
    outputs = len(gnoblin.find("Output"))
    say(
        f"created {client.title}: windows = {windows}, appId foot = {foot}, title starts with Term = {titled}, "
        f"outputs listed = {str(outputs > 0).lower()}, motion(200) = {gnoblin.motion(200):g}, "
        f"userShaders = {str(gnoblin.compositor.user_shaders).lower()}"
    )


say("running")
