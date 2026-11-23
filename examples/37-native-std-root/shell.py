"""Example 37 in Python: the members the std library adds to the ``gnoblin`` root of the core, through the classes generated from the catalogs.

    gnoblin ./shell.py

Importing ``gnoblin.objects.std`` gives the core's ``Gnoblin`` the members ``workspaces``, ``displays``, ``scene``, ``gestures`` and
``request_transition``, and its ``outputs`` the member ``profiles``. A shell that never imports it (example 35) has none of them. This shell:

- adds two workspaces to ``gnoblin.workspaces``, reads ``scene`` and ``gestures``, and runs a transition through
  ``gnoblin.request_transition``;
- when the library has a display for an output: reads the area windows are laid out in, shows each workspace with ``show`` and
  ``show_by_id``, and tries what the checks refuse (a workspace that is not in the list, an id that is not there);
- adds an output profile to ``gnoblin.outputs.profiles``, adds a display of its own over it, swaps it for another with
  ``gnoblin.displays.replace``, and tries to replace a display of the library's, which is refused.

Every line starts with ``stdroot:``, as the Rust and TypeScript shells of the same example (``37-native-std-root``) spell it, so that one
test reads all three.
"""

from gnoblin import gnoblin as session
from gnoblin.objects.model import Gnoblin
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
    say(f"removed next: {flag(displays.remove(displays.by_id('next')))}")
    say(f"after removing, displays hold {displays.length}")
    say(f"removed profile: {flag(profiles.remove(profile))}")
    say(f"profiles hold {profiles.length}")
    say("done")


displays.on("changed", on_changed)
say("running")
on_changed()
