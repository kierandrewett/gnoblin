"""Example 21 in Python: the object model of libgnoblin-core.

    gnoblin ./shell.py

The windows are not nodes this shell walks: ``gnoblin.clients`` is a live collection, and a window is an object with properties and
signals. The classes of the ``gnoblin.model`` catalog are generated from the catalog the library serves, so ``gnoblin.objects.model`` has a
class for each of them. This shell does what the Rust shell of the same example (``21-native-model``) does, and logs the same lines:

- reads the windows that are there and logs how many;
- makes a selection of the windows of one application (``where``) that follows its members, and logs each one that joins it;
- looks up a window that is not there, which answers ``None``, and one that is, which answers the window;
- listens to the titles of every window through the collection, and to the closing of every window through the ``gnoblin`` object.
"""

from gnoblin import gnoblin as session
from gnoblin import ClientQuery, Gnoblin


def say(text):
    session.log(f"model: {text}")


model = Gnoblin.get()
clients = model.clients
say(f"{clients.length} windows at the start")
# A lookup that finds nothing answers None, not an error.
say(f"absent lookups: at {clients.at(1000)!r}, get {clients.get('no-such-window')!r}")

# A selection nobody holds is freed with its listeners: the module keeps it for as long as the shell runs.
terminals = clients.where(ClientQuery(app_id="foot"))


@terminals.on("added")
def joined(client, collection):
    say(f"a terminal joined: {client.title}")
    say(f"the terminal is found by its id: {str(clients.get(client.id) is not None).lower()}")


@clients.on("title_changed")
def retitled(client, title):
    say(f"window {client.id} is now titled {title!r}")


@model.on("client_closed")
def closed(client):
    say(f'window {client.id} closed, last titled "{client.title}"')


say("running")
