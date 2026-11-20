"""Example 24 in Python: the scene of libgnoblin-std, through the classes generated from its catalog.

    gnoblin ./shell.py

Makes a perspective camera and a view drawn through it, mounts both under the scene's root and logs the placement it reads back. Every line
starts with ``scene:``, as the Rust shell of the same example spells it.
"""

from gnoblin import gnoblin
from gnoblin.objects.std import Camera3d, Projection, Scene, SceneSize, Vec3, View

scene = Scene.get()
camera = Camera3d(id="cam", projection=Projection.PERSPECTIVE, fov=70.0, position=Vec3(x=1.0, y=2.0, z=3.0))
scene.add(camera)
view = View(id="panel", size=SceneSize(width=400.0, height=300.0), position=Vec3(x=0.5, y=0.2, z=-1.4), camera=camera)
scene.add(view)
at = view.position
gnoblin.log(
    f"scene: the view {view.id} is node {view.node} at {at.x} {at.y} {at.z}, the camera {camera.id} is node {camera.node} with fov {camera.fov}"
)
