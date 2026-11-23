"""Example 38 in Python: the options of a constructor, through the classes generated from the catalogs.

    gnoblin ./shell.py

A constructor takes the properties its catalog names as positional arguments, and then keyword-only arguments: each keyword sets the
property of its name in the same call that makes the object, so a value the library refuses fails the call and leaves no object. This
shell makes:

- workspaces with no keywords, with every keyword (``name``, ``gaps``, ``layout``) and with one of them, and reads what each holds;
- output profiles with no keywords and with the keywords ``id`` and ``settings``, which the library sets as the object is made;
- a workspace and two profiles with a value the library refuses, and reads the library's words and what the lists hold afterwards.

Every line starts with ``ctoropts:``, as the Rust and TypeScript shells of the same example (``38-native-options``) spell it, so that
one test reads all three.
"""

from gnoblin import gnoblin as session
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import Gaps, MasterStackLayout, OutputMatch, OutputProfile, ProfileSettings, Workspace


def say(text):
    session.log(f"ctoropts: {text}")


def refused(work):
    """The words the library refuses ``work`` with, or ``no`` when it did not."""
    try:
        work()
        return "no"
    except Exception as error:
        return str(error)


def gaps_of(workspace):
    gaps = workspace.gaps
    return "none" if gaps is None else f"{gaps.inner:g}/{gaps.outer:g}"


def scale_of(profile):
    settings = profile.settings
    return "unset" if settings is None or settings.scale is None else f"{settings.scale:g}"


gnoblin = Gnoblin.get()
workspaces = gnoblin.workspaces
profiles = gnoblin.outputs.profiles

say("running")

# No keywords: the constructor of every shell so far.
plain = Workspace("plain")
say(f'plain workspace: id={plain.id} name="{plain.name}" gaps={gaps_of(plain)}')

# Every keyword.
main = Workspace("main", name="Main", gaps=Gaps(inner=6.0, outer=12.0), layout=MasterStackLayout())
say(
    f'main workspace: id={main.id} name="{main.name}" gaps={gaps_of(main)} layout={str(main.layout is not None).lower()}'
)

# One keyword: what is left out keeps the library's default.
half = Workspace("half", name="Half")
say(f'half workspace: id={half.id} name="{half.name}" gaps={gaps_of(half)}')

# A value the library refuses fails the call, and the next workspace of the id is made as if nothing had happened.
say(f"refused gaps: {refused(lambda: Workspace('bad', gaps=Gaps(inner=-1.0, outer=0.0)))}")
bad = Workspace("bad")
say(f'bad workspace after the refusal: id={bad.id} name="{bad.name}" gaps={gaps_of(bad)}')

workspaces.add(main)
workspaces.add(plain)
say(f"workspaces hold {', '.join(workspace.id for workspace in workspaces.list)}")

# The keywords of a class whose constructor sets only the match: the library sets the rest after it makes the object.
hdmi = OutputMatch(connector="HDMI-A-9")
unnamed = OutputProfile(hdmi)
say(f"plain profile: id={unnamed.id} scale={scale_of(unnamed)}")
mine = OutputProfile(hdmi, id="mine", settings=ProfileSettings(scale=2.0))
say(f"profile with options: id={mine.id} scale={scale_of(mine)}")
say(f"refused id: {refused(lambda: OutputProfile(hdmi, id=''))}")
say(f"refused scale: {refused(lambda: OutputProfile(hdmi, id='scaled', settings=ProfileSettings(scale=9.0)))}")

profiles.add(mine)
profiles.add(unnamed)
say(f"profiles hold {profiles.length}")
say("done")
