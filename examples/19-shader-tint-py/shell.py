"""Example 19 in Python: a custom shader on every window.

    gnoblin ./shell.py

A shader filter on the style of every window tints each of them: ``effect`` gets the window's colour and moves it a quarter of the way
towards a warm orange (``p[0]`` to ``p[2]`` are the colour and ``p[3]`` how far to go). The library writes it to each window's node as
one ``shader`` effect.

A shader is drawn only when the compositor's ``compositor.user-shaders`` setting is on, and only by a renderer that draws shaders. While
it is off the windows are drawn as they are, and the style warnings of a window say why. The compositor checks the source before it
accepts it: a source it refuses leaves the window's other effects as they were, and the compositor's log says where the source is
wrong. See docs/custom-shaders.md.
"""

from gnoblin import gnoblin
from gnoblin.std import StyleEntry, StyleKind, Style, clients

# ``p[0]`` to ``p[2]`` is the colour, ``p[3]`` the strength from 0 to 1. The window's colour is premultiplied, so the tint is scaled by
# its alpha.
TINT = """
fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {
    let tint = vec3<f32>(p[0], p[1], p[2]) * c.a;
    return vec4<f32>(mix(c.rgb, tint, p[3]), c.a);
}
"""

style = Style.scope(StyleKind.SCOPE_COLLECTION, clients().handle)
style.set_list(StyleKind.FILTER, [StyleEntry.of_shader(TINT, [1.0, 0.55, 0.2, 0.25])])
gnoblin.log("python shader tint shell is running")
