"""Example 36 in Python: a window the shell holds as a ``Client`` of the core, passed to the classes of libgnoblin-std that take one.

    gnoblin ./shell.py

Does what the Rust shell of the same example (``36-native-clients``) does and logs the same lines, which start with ``clients:``: when a
window appears it asks ``Decorations`` for the room of the window's frame (``inset_of``), opens the window menu on it (``WindowMenus.show``)
and closes the menu, and adds it to a workspace (``clients.add``). The library checks every handle against the core: text where a ``Client``
is taken, an object of another class and a window that has closed are each refused, and the log says how.
"""

from gnoblin import gnoblin as session
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import Decorations, WindowMenus, Workspace, Workspaces


def say(text):
    session.log(f"clients: {text}")


def refused(error):
    return f"{type(error).__name__}: {error}"


gnoblin = Gnoblin.get()
decorations = Decorations.get()
menus = WindowMenus.get()
menus.start()
main = Workspace("main")
Workspaces.get().add(main)


# The library puts a window on the workspace as it appears and tells the shell of it with the core's ``Client``: the same object the core's
# own collections give, which ``add`` and ``contains`` take as it is.
members = main.clients


@members.on("added")
def joined(client):
    members.add(client)
    say(f"{client.title}: it is in main: {str(members.contains(client)).lower()}")
    say(f"{client.title}: the library's client is the core's: {str(client is gnoblin.clients.get(client.id)).lower()}")


@gnoblin.clients.on("added")
def added(client, collection):
    inset = decorations.inset_of(client)
    say(f"{client.title}: the frame takes {inset.top:g} {inset.right:g} {inset.bottom:g} {inset.left:g}")
    menu = menus.show(client, 10, 10)
    say(f"{client.title}: the menu is for the window: {str(menu is not None and menu.client == client.id).lower()}")
    menus.close()
    try:
        decorations.inset_of(client.id)
    except TypeError as error:
        say(f"{client.title}: an id is refused: {refused(error)}")
    try:
        decorations.inset_of(gnoblin)
    except TypeError as error:
        say(f"{client.title}: another class is refused: {refused(error)}")


# A window that has closed is still a ``Client``, and its last values can be read; the library says it is gone.
@gnoblin.on("client_closed")
def closed(client):
    try:
        decorations.inset_of(client)
        say(f"{client.title}: a closed window was answered")
    except Exception as error:
        say(f"{client.title}: a closed window is refused: {refused(error)}")


say("running")
