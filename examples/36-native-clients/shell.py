"""Example 36 in Python: a window the shell holds as a ``Client`` of the core, passed to the classes of libgnoblin-std that take one.

    gnoblin ./shell.py

Does what the Rust shell of the same example (``36-native-clients``) does and logs the same lines, which start with ``clients:``: when a
window appears it asks ``Decorations`` for the room of the window's frame (``inset_of``), opens the window menu on it (``WindowMenus.show``)
and closes the menu, and adds it to a workspace (``clients.add``). The library checks every handle against the core: text where a ``Client``
is taken, an object of another class and a window that has closed are each refused, and the log says how. It then reads the members the
library adds to a window, an output and the root once it is loaded: the window's ``workspace``, ``move_to_workspace``, ``style``,
``style_warnings``, ``move_with_keys``, ``output``, ``move_to_output`` and ``frame_inset``, the output's ``workspace``, ``area`` and
``show``, and the root's ``show``.
"""

from gnoblin import gnoblin as session
from gnoblin.objects.model import ClientQuery, Gnoblin
from gnoblin.objects.std import Decorations, LayoutRect, WindowMenus, Workspace, Workspaces


def say(text):
    session.log(f"clients: {text}")


def refused(error):
    return f"{type(error).__name__}: {error}"


gnoblin = Gnoblin.get()
decorations = Decorations.get()
menus = WindowMenus.get()
menus.start()
main = Workspace("main")
side = Workspace("side")
Workspaces.get().add(main)
Workspaces.get().add(side)


def flag(value):
    return str(value).lower()


def id_of(workspace):
    return workspace.id if workspace is not None else None


checked = []


def check_members(client):
    """The members the library adds to the core's ``Client`` and ``Output``, read once, for the first window."""
    checked.append(True)
    say(f"{client.title}: its workspace is main: {flag(id_of(client.workspace) == 'main')}")
    moved = client.move_to_workspace(side)
    say(f"{client.title}: moved to side: {flag(moved)}, now in side: {flag(id_of(client.workspace) == 'side')}")
    back = client.move_to_workspace(main)
    say(f"{client.title}: moved back: {flag(back)}, again refused: {flag(not client.move_to_workspace(main))}")
    client.style.opacity = 0.8
    say(f"{client.title}: its own style holds 0.8: {flag(abs(client.style.opacity - 0.8) < 1e-6)}")
    say(f"{client.title}: its style warnings are a list: {flag(isinstance(client.style_warnings, list))}")
    # A selection styles the windows it holds and no others, and a selection that can hold no window has no style.
    clients = gnoblin.clients

    def has_no_style(query):
        try:
            clients.where(query).style
            return False
        except Exception as error:
            return "holds no window" in str(error)

    say(f"{client.title}: an empty any list has no style: {flag(has_no_style(ClientQuery(any_app_id=[])))}")
    outside = has_no_style(ClientQuery(app_id="foot", any_app_id=["x"]))
    say(f"{client.title}: an app id outside its any list has no style: {flag(outside)}")
    matching = clients.where(ClientQuery(app_id="foot", any_app_id=["foot", "x"])).style
    matching.background = "#112233"
    clients.where(ClientQuery(app_id="nope"), 10.0).style.background = "#ffffff"
    clients.where(ClientQuery(app_id="foot")).where(ClientQuery(title="nope"), 20.0).style.background = "#ffffff"
    only = client.computed_style.background == matching.background
    say(f"{client.title}: only the matching selections style it: {flag(only)}")
    say(f"{client.title}: keyboard move before Repositioning starts: {flag(client.move_with_keys())}")
    output = gnoblin.outputs.at(0)
    say(f"{client.title}: the output shows main: {flag(id_of(output.workspace) == 'main')}")
    output.show(side)
    showed = id_of(output.workspace) == "side"
    output.show(main)
    say(f"{client.title}: the output showed side: {flag(showed)}")
    say(f"{client.title}: its output is the first: {flag(client.output is not None and client.output.id == output.id)}")
    say(f"{client.title}: a move to the output it is on is refused: {flag(not client.move_to_output(output))}")
    inset = client.frame_inset
    sides = [inset.top, inset.right, inset.bottom, inset.left]
    say(f"{client.title}: its frame inset has 4 sides: {flag(all(room >= 0 for room in sides))}")
    area = output.area
    inside = area is not None and 0 < area.width <= output.width
    say(f"{client.title}: the output's area is inside it: {flag(inside)}")
    gnoblin.show(side)
    shown = id_of(output.workspace) == "side"
    gnoblin.show(main)
    say(f"{client.title}: gnoblin.show showed side: {flag(shown)}")
    client.floating = True
    say(f"{client.title}: it floats once set: {flag(client.floating)}")
    client.floating_bounds = LayoutRect(x=10, y=20, width=300, height=200)
    kept = client.floating_bounds is not None and client.floating_bounds.width == 300
    say(f"{client.title}: it keeps its floating bounds: {flag(kept)}")
    client.floating_bounds = None
    client.floating = False
    say(f"{client.title}: it forgets them: {flag(client.floating_bounds is None and not client.floating)}")
    # ``set`` writes several properties in one transition; without options nothing animates.
    client.set({"floating": True})
    floated = client.floating
    client.set({"floating": False})
    say(f"{client.title}: set wrote floating: {flag(floated and not client.floating)}")
    try:
        client.set({"title": "x"})
        say(f"{client.title}: set refused title: no")
    except TypeError as error:
        say(f"{client.title}: set refused title: {refused(error)}")
    main.set({"name": "Primary"})
    named = main.name == "Primary"
    main.set({"name": ""})
    say(f"{client.title}: a workspace's set wrote its name: {flag(named)}")


# The library puts a window on the workspace as it appears and tells the shell of it with the core's ``Client``: the same object the core's
# own collections give, which ``add`` and ``contains`` take as it is.
members = main.clients


@members.on("added")
def joined(client):
    members.add(client)
    say(f"{client.title}: it is in main: {str(members.contains(client)).lower()}")
    say(f"{client.title}: the library's client is the core's: {str(client is gnoblin.clients.get(client.id)).lower()}")
    if not checked:
        check_members(client)


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
