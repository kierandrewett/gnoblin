"""Example 25 in Python: the styles and transitions of libgnoblin-std, through the classes generated from its catalog.

    gnoblin ./shell.py

Gives every window rounded corners and two shadows, stages a transition that also changes the opacity of every window and a node of its
own, and aborts it: the library puts back the opacity, and the shell's rollback is told before ``abort`` answers. Every line starts with
``style:``, as the Rust shell of the same example spells it.
"""

from gnoblin import gnoblin
from gnoblin.objects.std import (
    ClientStyle,
    CornerRadius,
    SceneNode,
    Shadow,
    StyleProperty,
    Transition,
    TransitionOptions,
    TransitionValue,
)

style = ClientStyle.of_clients()
style.corner_radius = CornerRadius(all=12.0)
style.shadows = [Shadow(blur=8.0), Shadow(y=4.0, blur=20.0)]
radius = style.corner_radius.all
shadows = style.shadows
gnoblin.log(f"style: corner radius {radius}, {len(shadows)} shadows, the first blur {shadows[0].blur}")

probe = SceneNode(id="probe")
rolled = []
Transition.begin(TransitionOptions(duration=200.0), lambda: rolled.append(1))
Transition.set(probe.node, "x", TransitionValue(number=40.0))
style.opacity = 0.5
Transition.abort()
gnoblin.log(
    f"style: transition aborted, rollback ran {len(rolled)}, opacity is set {style.is_set(StyleProperty.OPACITY)}, x is {probe.x}"
)
