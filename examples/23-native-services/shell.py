"""Example 23 in Python: the session services of libgnoblin-core.

    gnoblin ./shell.py

The classes of the ``gnoblin.model`` catalog are generated from the catalog the library serves, so ``gnoblin.objects.model`` has a class for
each of them. This shell does what the Rust shell of the same example (``23-native-services``) does, and logs the same lines:

- starts a command line and logs how it ended and what it printed;
- watches the session for 300 milliseconds of idleness, and logs when it is idle and when the user is back;
- begins a launch of an application that never opens a window, and logs that it timed out;
- asks for the session lock with ``<Super>e`` and logs how the request settled.
"""

import json

from gnoblin import gnoblin as session
from gnoblin.objects.model import Gnoblin, LaunchOptions, SpawnOptions, StreamMode


def say(text):
    session.log(f"services: {text}")


model = Gnoblin.get()
version = model.version()
say(f"gnoblin {version.gnoblin} speaks api {version.api}")

process = model.shell("printf hello; exit 3", SpawnOptions(stdout=StreamMode.CAPTURE))


@process.on("exited")
def ended(finished):
    out = finished.exit.stdout if finished.exit is not None and finished.exit.stdout is not None else ""
    say(f"the process ended with code {finished.exit_code} and printed {json.dumps(out)}")


launch = model.launches.begin("never", "org.example.Never", LaunchOptions(timeout_ms=100))


@launch.on("timed_out")
def late(launch):
    say(f"the launch {launch.token} timed out")


watch = model.idle.watch(300.0)
watch.on("idle", lambda *_: say("the session is idle"))
watch.on("resume", lambda *_: say("the session is active again"))


def lock():
    operation = model.session.lock()

    @operation.on("settled")
    def settled(*_):
        say(
            "the lock request settled, "
            + ("accepted" if operation.error == "" else f"refused with {operation.error_code}")
        )


session.bind("<Super>e", lock)
say("running")
