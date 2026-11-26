"""Example 37 in Python: the members the std library adds to the ``gnoblin`` root of the core, through the classes generated from the catalogs.

    gnoblin ./shell.py

Importing ``gnoblin.objects.std`` gives the core's ``Gnoblin`` the members ``workspaces``, ``displays``, ``scene``, ``gestures`` and
``request_transition``, its ``outputs`` the member ``profiles``, each input context ``active_display`` and each collection of windows
``style``. A shell that never imports it (example 35) has none of them. This shell:

- adds two workspaces to ``gnoblin.workspaces``, reads ``scene`` and ``gestures``, and runs a transition through
  ``gnoblin.request_transition``;
- sets the style of every window through ``gnoblin.clients.style``, and checks that a selection made with ``where``, and a selection of it,
  each have a style of their own, and that a list made with ``ClientCollection()`` has none;
- when the library has a display for an output: reads the area windows are laid out in, shows each workspace with ``show`` and
  ``show_by_id``, tries what the checks refuse (a workspace that is not in the list, an id that is not there), and reads the workspace the
  display under the pointer shows through ``gnoblin.input.primary.active_display``;
- adds an output profile to ``gnoblin.outputs.profiles``, adds a display of its own over it, swaps it for another with
  ``gnoblin.displays.replace``, and tries to replace a display of the library's, which is refused.

Every line starts with ``stdroot:``, as the Rust and TypeScript shells of the same example (``37-native-std-root``) spell it, so that one
test reads all three.
"""

from gnoblin import gnoblin as session
from gnoblin.objects.model import ClientCollection, ClientQuery, Gnoblin
from gnoblin.objects.std import Display, DisplaySelector, OutputMatch, OutputProfile, TransitionOptions, Workspace


def say(text):
    session.log(f"stdroot: {text}")


def flag(value):
    return str(value).lower()


gnoblin = Gnoblin.get()
workspaces = gnoblin.workspaces
displays = gnoblin.displays
profiles = gnoblin.outputs.profiles

main = Workspace("main")
side = Workspace("side")
ghost = Workspace("ghost")
workspaces.add(main)
workspaces.add(side)
say(f"workspaces hold {', '.join(workspace.id for workspace in workspaces.list)}")

say(f"scene metres per unit is 1: {flag(gnoblin.scene.metres_per_unit == 1)}")
say(f"gestures swipe threshold is positive: {flag(gnoblin.gestures.swipe_threshold > 0)}")

ran = []
gnoblin.request_transition(lambda: ran.append(True), TransitionOptions(duration=0.0), lambda: None)
say(f"requestTransition ran its work: {flag(bool(ran))}")
try:
    gnoblin.request_transition(lambda: None, TransitionOptions(duration=-1.0), lambda: None)
    say("requestTransition refused: no")
except Exception as error:
    say(f"requestTransition refused: {error}")

# The style of a collection of windows: every window for ``gnoblin.clients``, and a scope of its own for a selection and for a selection of
# it.
clients = gnoblin.clients
clients.style.opacity = 0.9
say(f"clients style opacity is 0.9: {flag(abs(clients.style.opacity - 0.9) < 1e-6)}")
foot = clients.where(ClientQuery(app_id="foot"))
say(f"a selection has its own style: {flag(foot.style.path != clients.style.path)}")
say(
    f"a selection of a selection has its own style: {flag(foot.where(ClientQuery(title='x')).style.path != foot.style.path)}"
)
try:
    say(f"refused list style: no {ClientCollection().style.path}")
except Exception as error:
    say(f"refused list style: {error}")


def ids():
    """The ids of the displays in the list, sorted and joined."""
    return ", ".join(sorted(display.id for display in displays.list))


def over(profile_id, name):
    """A display over the profile ``profile_id``, which has the id ``name``."""
    display = Display([DisplaySelector(profile=profiles.by_id(profile_id))])
    display.id = name
    return display


done = [False]


def on_changed():
    """The first time the library has a display for an output."""
    if done[0] or displays.length == 0:
        return
    done[0] = True
    area = workspaces.area
    say(f"the area is known: {flag(area is not None and area.width > 0 and area.height > 0)}")

    workspaces.show(side)
    say("show side: true")
    say(f"side is active: {flag(side.active)}")
    workspaces.show_by_id("main")
    say("show main by id: true")
    say(f"main is active: {flag(main.active)}")

    # ``find`` has the kind ``Workspace`` once std is loaded: by field values, and by a function.
    def workspace_ids(found):
        return ",".join(workspace.id for workspace in found)

    say(f"find Workspace by id: {workspace_ids(gnoblin.find('Workspace', {'id': 'side'}))}")
    say(f"find Workspace shown: {workspace_ids(gnoblin.find('Workspace', {'active': True}))}")
    say(
        f"find Workspace by a function: {workspace_ids(gnoblin.find('Workspace', lambda found: found.id.startswith('m')))}"
    )
    primary = gnoblin.input.primary if gnoblin.input is not None else None
    active = primary.active_display if primary is not None else None
    viewport = active.viewport if active is not None else None
    shown = viewport.content if viewport is not None else None
    say(f"the active display shows main: {flag(shown is not None and shown.id == 'main')}")
    try:
        workspaces.show(ghost)
        say("refused ghost: no")
    except Exception as error:
        say(f"refused ghost: {error}")
    try:
        workspaces.show_by_id("nope")
        say("refused nope: no")
    except Exception as error:
        say(f"refused nope: {error}")

    library = displays.list[0]
    say(f"displays hold {displays.length}")
    profile = OutputProfile(OutputMatch(connector=library.id.removeprefix("display-")))
    profiles.add(profile)
    say(f"profiles hold {profiles.length}")

    try:
        displays.replace(library, over(profile.id, "other"))
        say("refused library display: no")
    except Exception as error:
        say(f"refused library display: {error}")
    mine = over(profile.id, "mine")
    displays.add(mine)
    say(f"displays hold {ids()}")
    displays.replace(mine, over(profile.id, "next"))
    say(f"replaced mine with next: {ids()}")
    gone = displays.by_id("next")
    assert gone is not None, "the display that replaced mine is in the list"
    say(f"removed next: {flag(displays.remove(gone))}")
    say(f"after removing, displays hold {displays.length}")
    say(f"removed profile: {flag(profiles.remove(profile))}")
    say(f"profiles hold {profiles.length}")
    say("done")


displays.on("changed", on_changed)
say("running")
on_changed()
