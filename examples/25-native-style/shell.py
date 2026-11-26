"""Example 25 in Python: the styles and transitions of libgnoblin-std, through the classes generated from its catalog.

    gnoblin ./shell.py

Gives every window rounded corners and two shadows, stages a transition that also changes the opacity of every window and a node of its
own, and aborts it: the library puts back the opacity, and the shell's rollback is told before ``abort`` answers. Every line starts with
``style:``, as the Rust shell of the same example spells it.
"""

from gnoblin import gnoblin
from gnoblin.objects.std import (
    BlurFilter,
    Border,
    BrightnessFilter,
    ClientStyle,
    ContrastFilter,
    CornerRadius,
    SaturateFilter,
    SceneNode,
    ShaderFilter,
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

# The value classes: every one is built with its defaults and with values of its own, and the library hands back what it was given.
SHADER = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> { return c * p[0]; }"
style.borders = [Border(), Border(width=3, color="#ff0000", offset=-2, inset=True)]
style.shadows = [Shadow(), Shadow(x=2, y=4, blur=20, spread=3, color="#112233", inset=True)]
style.backdrop_filter = [
    BlurFilter(),
    BlurFilter(radius=20),
    SaturateFilter(),
    SaturateFilter(amount=1.5),
    BrightnessFilter(),
    BrightnessFilter(amount=0.5),
    ContrastFilter(),
    ContrastFilter(amount=1.25),
]
style.filter = [ShaderFilter(source=SHADER), ShaderFilter(source=SHADER, params=[0.5, 2])]


def describe(entry):
    kind = type(entry).__name__.removesuffix("Filter").lower()
    if isinstance(entry, ShaderFilter):
        return f"{kind} " + (",".join(f"{number:g}" for number in entry.params) or "-")
    return f"{kind} {(entry.radius if isinstance(entry, BlurFilter) else entry.amount):g}"


def flag(value):
    return str(value).lower()


gnoblin.log(
    "style: value classes borders "
    + " | ".join(f"{b.width:g} {b.color} {b.offset:g} {flag(b.inset)}" for b in style.borders)
)
gnoblin.log(
    "style: value classes shadows "
    + " | ".join(f"{s.x:g} {s.y:g} {s.blur:g} {s.spread:g} {s.color} {flag(s.inset)}" for s in style.shadows)
)
gnoblin.log("style: value classes backdrop filters " + " | ".join(describe(entry) for entry in style.backdrop_filter))
gnoblin.log("style: value classes filters " + " | ".join(describe(entry) for entry in style.filter))

# A bare number is the radius of every corner, and a filter may be written as a dict with one key, its kind.
style.corner_radius = 12
style.backdrop_filter = [{"blur": 24}, {"saturate": 1.35}]
style.filter = [{"shader": {"source": SHADER, "params": [2]}}]
gnoblin.log(
    f"style: shorthands corner {style.corner_radius.all:g}, "
    + "backdrop "
    + " | ".join(describe(entry) for entry in style.backdrop_filter)
    + ", filter "
    + " | ".join(describe(entry) for entry in style.filter)
)

probe = SceneNode(id="probe")
rolled = []
Transition.begin(TransitionOptions(duration=200.0), lambda: rolled.append(1))
Transition.set(probe.node, "x", TransitionValue(number=40.0))
style.opacity = 0.5
Transition.abort()
gnoblin.log(
    f"style: transition aborted, rollback ran {len(rolled)}, opacity is set {style.is_set(StyleProperty.OPACITY)}, x is {probe.x}"
)
