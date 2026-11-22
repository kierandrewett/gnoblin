"""Example 33 in Python: output profiles, displays and viewports of libgnoblin-std, through the classes generated from its catalog.

    gnoblin ./shell.py

The library keeps one ``single`` display for every output, so the list of displays says what each monitor shows. This shell:

- makes two workspaces and shows the second one on the display of the output, through the display's viewport;
- adds an output profile for that output (a scale), and hears the compositor take it (``applied``);
- adds a display of its own over the profile, which takes the output from the display of the library, and removes it again, which gives
  the output a display of the library's again;
- tries what the checks refuse: a scale out of range, a display with nothing to present, a second display over an output that has one.

Every line starts with ``displays:``, as the Rust shell of the same example (``33-native-displays``) spells it, so that one test reads all
three.
"""

from gnoblin import gnoblin
from gnoblin.objects.std import (
    Display,
    DisplayManager,
    DisplaySelector,
    OutputMatch,
    OutputProfile,
    ProfileSettings,
    Viewport,
    Workspace,
    Workspaces,
)


def say(text):
    gnoblin.log(f"displays: {text}")


workspaces = Workspaces.get()
one = Workspace("one")
two = Workspace("two")
workspaces.add(one)
workspaces.add(two)
manager = DisplayManager.get()


def listed():
    """The displays as ``id:mode:outputs:workspace`` words, sorted."""
    words = []
    for display in manager.displays:
        shown = display.viewport.content
        words.append(f"{display.id}:{display.mode.name.lower()}:{len(display.outputs)}:{shown.id if shown else 'none'}")
    return " ".join(sorted(words))


phase = [0]
held = {}


def on_changed():
    """The first time the library has a display for an output: show the second workspace on it and add the profile."""
    if phase[0] != 0 or not manager.displays:
        return
    phase[0] = 1
    say(f"listed {listed()}")
    display = manager.displays[0]
    display.viewport.content = two
    say(f"the display {display.id} shows {display.viewport.content.id}")
    connector = display.id.removeprefix("display-")
    profile = OutputProfile(OutputMatch(connector=connector))
    profile.settings = ProfileSettings(mode="preferred", scale=1.25)
    manager.add_profile(profile)
    held["profile"] = profile
    say(f"the profile {profile.id} is in the manager: {str(profile.listed).lower()}")


def on_applied(output, error):
    """The compositor answered the profile's request: use the profile in a display of this shell's own."""
    if phase[0] != 1:
        return
    phase[0] = 2
    say(f"the compositor answered the request: {'ok' if error == '' else error}")
    profile = held["profile"]
    settings = profile.settings
    say(f"the profile reads back scale {settings.scale} mode {settings.mode}")

    mine = Display([DisplaySelector(profile=profile)])
    mine.id = "main"
    viewport = Viewport()
    viewport.content = one
    mine.viewport = viewport
    manager.add_display(mine)
    say(f"listed {listed()}")

    rival = Display([DisplaySelector(profile=profile)])
    rival.id = "rival"
    try:
        manager.add_display(rival)
        say("refused overlap: no")
    except Exception as error:
        say(f"refused overlap: {error}")

    say(f"removed main: {str(manager.remove_display(mine)).lower()}")
    say(f"listed {listed()}")

    try:
        OutputProfile(OutputMatch(connector="HEADLESS-1")).settings = ProfileSettings(scale=9.0)
        say("refused scale: no")
    except Exception as error:
        say(f"refused scale: {error}")
    try:
        Display([])
        say("refused selectors: no")
    except Exception as error:
        say(f"refused selectors: {error}")
    say("done")


manager.on("changed", on_changed)
manager.on("applied", on_applied)
say("running")
on_changed()
