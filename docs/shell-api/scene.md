# Scene, views and property bindings

The scene is the tree of nodes a script makes.

- Plain `SceneNode` objects, `View`s that draw a client or a workspace as a picture, and cameras are in it.
- `gnoblin.scene.root` is where a script mounts them.
- A node is placed with `position`, `rotation`, `scale` or one `transform`.
- A property of a node or a window can follow the properties it was computed from.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";
import { ArcLayout, SceneNode, View, Workspace } from "gnoblin-std";

const desktop = new Workspace("desktop", { layout: new ArcLayout({ radius: 1200, spacing: 0.35 }) });
gnoblin.workspaces.add(desktop);

const overview = new SceneNode({ id: "overview" });
const picture = new View({ size: { width: 700, height: 400 } });
picture.showWorkspace(desktop);
overview.add(picture);
gnoblin.scene.root?.add(overview);

const client = gnoblin.input?.primary?.focusedClient;
client?.properties?.bindDouble("opacity", () => (client.focused ? 1 : 0.7));
```

```python
from gnoblin import Gnoblin
from gnoblin.std import ArcLayout, GnoblinAugment, SceneNode, SceneSize, View, Workspace

gnoblin = Gnoblin.get()
std = GnoblinAugment.get()
desktop = Workspace("desktop", layout=ArcLayout(radius=1200, spacing=0.35))
std.workspaces.add(desktop)

overview = SceneNode(id="overview")
picture = View(size=SceneSize(width=700, height=400))
picture.show_workspace(desktop)
overview.add(picture)
if std.scene.root is not None:
    std.scene.root.add(overview)

client = gnoblin.input.primary.focused_client
if client is not None and client.properties is not None:
    window = client
    client.properties.bind_double("opacity", lambda: 1.0 if window.focused else 0.7)
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_std::{
    ArcLayout, ArcLayoutOptions, Scene, SceneNode, SceneNodeOptions, SceneSize, View, ViewOptions, Workspace,
    WorkspaceOptions, Workspaces,
};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let core = g.objects()?;
    let objects = g.objects()?.std()?;
    let model = Gnoblin::get(&core)?;
    let arc = ArcLayout::new_with(
        &objects,
        ArcLayoutOptions {
            radius: Some(1200.0),
            spacing: Some(0.35),
        },
    )?;
    let desktop = Workspace::new_with(
        &objects,
        "desktop",
        WorkspaceOptions {
            layout: Some(arc.as_layout()?),
            ..WorkspaceOptions::default()
        },
    )?;
    Workspaces::get(&objects)?.add(&desktop)?;

    let overview = SceneNode::new_with(
        &objects,
        SceneNodeOptions {
            id: Some("overview".into()),
            ..SceneNodeOptions::default()
        },
    )?;
    let picture = View::new_with(
        &objects,
        ViewOptions {
            size: Some(SceneSize::new(700.0, 400.0)),
            ..ViewOptions::default()
        },
    )?;
    picture.show_workspace(&desktop)?;
    overview.add(&picture.as_scene_node()?, None)?;
    if let Some(root) = Scene::get(&objects)?.root()? {
        root.add(&overview, None)?;
    }

    let input = model.input()?;
    let primary = input.primary()?;
    if let Some(client) = primary.focused_client()? {
        if let Some(properties) = client.properties()? {
            properties.bind_double("opacity", move || Ok(if client.focused()? { 1.0 } else { 0.7 }))?;
        }
    }
    Ok(())
}
```

:::

Put the file in `~/.config/gnoblin/shell.ts`, `~/.config/gnoblin/shell.py`, or a Rust shell library. The windows of `desktop` sit on an arc, the scene holds a small picture of the
workspace, and the focused window is drawn at full opacity while the others are dimmed.

## The scene

`gnoblin.scene` is the one scene. Its members:

| Member          | Type         | Meaning                                                                           |
| --------------- | ------------ | --------------------------------------------------------------------------------- |
| `root`          | `SceneNode?` | The root of the tree. Mount a node with `root.add(node)`. It cannot be destroyed. |
| `metresPerUnit` | number       | Metres in one scene unit. Default 1, which is logical units. The script keeps it. |
| `cameras`       | `Camera3D[]` | The cameras a script made.                                                        |
| `add(node)`     | method       | Mounts a node under the root and returns it.                                      |
| `find(name)`    | method       | The node with that name, or `undefined`.                                          |

`root` is `undefined` (`None` in Python and Rust) when the compositor has published no root node. Check it before use, or mount with `scene.add(node)`.

`root.children` lists the nodes a script made. Windows are reached through `gnoblin.clients` and workspaces through
`gnoblin.workspaces`: a workspace is a script object and not a tree node, so neither is in `children`.

## Place a node

A node made with `new SceneNode(options)` is in the tree under no parent until it is added to one. Every option is optional.

| Option or member     | Type                               | Meaning                                                                     |
| -------------------- | ---------------------------------- | --------------------------------------------------------------------------- |
| `id`                 | string                             | The node's stable id. It is also its `name`, and `node.id` answers with it. |
| `position`           | `{ x, y, z }`                      | Where the node is, in its parent's units.                                   |
| `rotation`           | `{ x, y, z }`                      | Angles about X, Y and Z in radians.                                         |
| `scale`              | `{ x, y, z }`                      | The scale along each axis.                                                  |
| `transform`          | `{ position, rotation, scale }`    | All three parts: `scale` is one number for every axis.                      |
| `visible`, `opacity` | boolean, number                    | As on a window.                                                             |
| `input`              | `"forward"`, `"capture"`, `"none"` | What a pointer ray does at the node. See [Input](#input).                   |
| `metresPerUnit`      | number                             | Metres in one unit of this node's space. Default 1.                         |

- A rotation, a position and a scale read back as a `Vec3` with `x`, `y` and `z`. Rotations turn about X, then Y, then Z.
- A write is checked before anything is written. A value that is not finite throws a `TypeError` that names the node and
  the member, as in `SceneNode#12.rotation`.

`place(parts)` is one write of the parts you name. `node.place({ rotation: { x: 0, y: 0.12, z: 0 } })` turns the node
and leaves everything else alone.

- `parts` also takes `uniformScale` for one number on every axis, and `orientation`, a quaternion `{ x, y, z, w }`
  that is stored as its Euler angles.
- All the parts are checked first, so one bad part writes nothing, and the writes of a frame go out in one transaction.

::: tabs

```typescript
import { SceneNode } from "gnoblin-std";

const room = new SceneNode({ id: "xr-room", position: { x: 0, y: 1.2, z: -2 } });
room.place({ rotation: { x: 0, y: Math.PI / 4, z: 0 }, uniformScale: 1 });
```

```python
from gnoblin.std import SceneNode, TransformParts, Vec3

room = SceneNode(id="xr-room", position=Vec3(x=0.0, y=1.2, z=-2.0))
room.place(TransformParts(rotation=Vec3(x=0.0, y=3.14159 / 4, z=0.0), uniform_scale=1.0))
```

```rust
use gnoblin_std::{SceneNode, SceneNodeOptions, TransformParts, Vec3};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;
    let room = SceneNode::new_with(
        &objects,
        SceneNodeOptions {
            id: Some("xr-room".into()),
            position: Some(Vec3 { x: 0.0, y: 1.2, z: -2.0 }),
            ..SceneNodeOptions::default()
        },
    )?;
    room.place(&TransformParts {
        rotation: Some(Vec3 { x: 0.0, y: std::f64::consts::FRAC_PI_4, z: 0.0 }),
        uniform_scale: Some(1.0),
        ..TransformParts::default()
    })?;
    Ok(())
}
```

:::

### Input

`input` says what a pointer does when it reaches the node:

| Value       | Meaning                                                                                             |
| ----------- | --------------------------------------------------------------------------------------------------- |
| `"forward"` | The node is hit and, for a `View` of a client, the pointer goes on into the client. The default.    |
| `"capture"` | The node is hit and the pointer stops there: the shell is sent its pointer events, the client none. |
| `"none"`    | The node is missed. What is under it takes the pointer, and its children are still tested.          |

- A write also takes `true` (`"forward"`) and `false` (`"none"`), and a read gives the string.
- The tree holds this as two properties: `input` (set for `"forward"` and `"capture"`) and `input_mode`
  (`"forward"` or `"capture"`). The compositor routes by them, so the mode is the same for every shell and for the XR
  program.
- `"forward"` on a `View` of a client gives the client the pointer in its own surface coordinates, wherever the View is
  placed, scaled or turned: the compositor maps the point of the View to the point of the client's picture under it. A
  point where the picture shows nothing of the client (the margin of a View larger than its client) reaches nobody.
  The script is sent nothing for the View.
- `"capture"` lands on the View itself. The client is not given the pointer (and keeps its keyboard focus), and the View
  has a `pointer` event with `{ edge, x, y, button }` in the View's own plane, where `edge` is `enter`, `motion`, `leave`,
  `press` or `release`. A script can use this to draw its own interaction over a client's picture.
- A node that is not a View of a client has nothing to forward to, so `"forward"` and `"capture"` both send the script
  its pointer events. A View with no `content` is the same.
- The handler is set with `panel.on("pointer", (event) => { ... })`. A throw in one is reported like a throw in a listener.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";
import { View } from "gnoblin-std";

const client = gnoblin.input?.primary?.focusedClient;
if (client) {
    const panel = new View({ size: { width: 400, height: 300 }, input: "capture" });
    panel.showClient(client);
    panel.on("pointer", (event) => {
        if (event.edge !== "press") return;
        if (event.x < 40 && event.y < 40) {
            client.close();
        } else {
            panel.input = "forward"; // from the next move, the client gets the pointer
        }
    });
    gnoblin.scene.add(panel);
}
```

```python
from gnoblin import Gnoblin
from gnoblin.std import GnoblinAugment, InputMode, PointerEdge, SceneSize, View

gnoblin = Gnoblin.get()
client = gnoblin.input.primary.focused_client
if client is not None:
    window = client
    panel = View(size=SceneSize(width=400, height=300), input=InputMode.CAPTURE)
    panel.show_client(window)

    def on_pointer(event) -> None:
        if event.edge != PointerEdge.PRESS:
            return
        if event.x < 40 and event.y < 40:
            window.close()
        else:
            panel.input = InputMode.FORWARD  # from the next move, the client gets the pointer

    panel.on("pointer", on_pointer)
    GnoblinAugment.get().scene.add(panel)
```

```rust
use std::rc::Rc;

use gnoblin_core::Gnoblin;
use gnoblin_std::{InputMode, PointerEdge, Scene, SceneSize, View, ViewOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let core = g.objects()?;
    let objects = g.objects()?.std()?;
    let model = Gnoblin::get(&core)?;
    let input = model.input()?;
    let primary = input.primary()?;
    let Some(client) = primary.focused_client()? else {
        return Ok(());
    };
    let panel = Rc::new(View::new_with(
        &objects,
        ViewOptions {
            size: Some(SceneSize::new(400.0, 300.0)),
            input: Some(InputMode::Capture),
            ..ViewOptions::default()
        },
    )?);
    panel.show_client(&client)?;
    let handle = Rc::clone(&panel);
    panel.on_pointer(move |event| {
        if !matches!(event.edge, PointerEdge::Press) {
            return;
        }
        if event.x < 40.0 && event.y < 40.0 {
            let _ = client.close();
        } else {
            // From the next move, the client gets the pointer.
            let _ = handle.set_input(InputMode::Forward);
        }
    })?;
    Scene::get(&objects)?.add(&panel.as_scene_node()?)?;
    Ok(())
}
```

:::

## Views

A `View` draws another node as a picture. `showClient(client)` or `showWorkspace(workspace)` says what it shows, and
neither is moved or reparented.

| Option or member   | Type                                | Meaning                                                                 |
| ------------------ | ----------------------------------- | ----------------------------------------------------------------------- |
| `showClient(c)`    | method                              | Shows a client.                                                         |
| `showWorkspace(w)` | method                              | Shows a workspace.                                                      |
| `size`             | `{ width, height }`                 | The View's rectangle. The same numbers as `width` and `height`.         |
| `fit`              | `"contain"`, `"cover"`, `"stretch"` | How the picture is fitted to `size`. Default `"stretch"`.               |
| `camera`           | `Camera3D`                          | The camera the content is drawn through. `clearCamera()` makes it flat. |
| `orphaned`         | boolean                             | `true` once what the View showed is gone.                               |

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";
import { View } from "gnoblin-std";

const panel = new View({ size: { width: 0.9, height: 0.55 }, fit: "contain", input: "forward" });
const focused = gnoblin.input?.primary?.focusedClient;
if (focused) {
    panel.showClient(focused);
}
gnoblin.scene.add(panel);
```

```python
from gnoblin import Gnoblin
from gnoblin.std import ContentFit, GnoblinAugment, InputMode, SceneSize, View

gnoblin = Gnoblin.get()
panel = View(size=SceneSize(width=0.9, height=0.55), fit=ContentFit.CONTAIN, input=InputMode.FORWARD)
focused = gnoblin.input.primary.focused_client
if focused is not None:
    panel.show_client(focused)
GnoblinAugment.get().scene.add(panel)
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_std::{ContentFit, InputMode, Scene, SceneSize, View, ViewOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let core = g.objects()?;
    let objects = g.objects()?.std()?;
    let model = Gnoblin::get(&core)?;
    let panel = View::new_with(
        &objects,
        ViewOptions {
            size: Some(SceneSize::new(0.9, 0.55)),
            fit: Some(ContentFit::Contain),
            input: Some(InputMode::Forward),
            ..ViewOptions::default()
        },
    )?;
    let input = model.input()?;
    let primary = input.primary()?;
    if let Some(focused) = primary.focused_client()? {
        panel.show_client(&focused)?;
    }
    Scene::get(&objects)?.add(&panel.as_scene_node()?)?;
    Ok(())
}
```

:::

A View of a client is that client's picture, drawn by the compositor at `size`. The compositor stretches it, so `fit` is
kept and read back for a client but only changes how a workspace is drawn.

A workspace is not a tree node, so the compositor has no node to draw for it. The View keeps a picture made of one child
View for each window on the workspace, each showing that window and placed where the window sits in the workspace's area.

- The area is the output that shows the workspace, or the smallest rectangle that holds its windows.
- `fit` scales the picture into `size`. `"contain"` keeps the aspect ratio and centres it. `"cover"` keeps it and
  fills `size`; what falls outside is not clipped. `"stretch"` scales each axis on its own.
- The picture follows the workspace. A window that opens, closes, moves or resizes changes it in the same frame.
- A window that its layout hides, such as the others of a monocle layout, is hidden in the picture, and so are minimised
  windows.
- `view.destroy()` removes the picture. Its child Views are internal and are not in `view.children`.

`client.committedSize` is the size the client last committed, as `{ width, height }`: what the compositor last published,
as against `width` and `height`, which are what the shell asked for.

## Place a window with a transform

- A window has `position`, `rotation` and `scale` as lists of three numbers, as a node has them as vectors.
- A window's `position` is its `x`, `y` and its depth `z`.
- A position that is written moves the window's rectangle, and a layout that places the window will place it again.
- The rotation and the scale are the window's own.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";
import "gnoblin-std";

const client = gnoblin.input?.primary?.focusedClient;
if (client) {
    gnoblin.requestTransition(
        () => {
            client.opacity = 0.92;
            client.rotation = [0, 0.12, 0];
        },
        { duration: 250, easing: "ease-out" },
    );
}
```

```python
from gnoblin import Gnoblin
from gnoblin.std import Easing, GnoblinAugment, TransitionOptions

gnoblin = Gnoblin.get()
client = gnoblin.input.primary.focused_client
if client is not None:
    window = client

    def change() -> None:
        window.opacity = 0.92
        window.rotation = [0.0, 0.12, 0.0]

    GnoblinAugment.get().request_transition(change, TransitionOptions(duration=250, easing=Easing.EASE_OUT))
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_std::{Easing, TransitionOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let input = model.input()?;
    let primary = input.primary()?;
    let Some(client) = primary.focused_client()? else {
        return Ok(());
    };
    let options = TransitionOptions {
        duration: 250.0,
        easing: Easing::EaseOut,
        delay: 0.0,
        autoplay: true,
    };
    model.request_transition(
        move || {
            client.set_opacity(0.92)?;
            client.set_rotation(&[0.0, 0.12, 0.0])?;
            Ok(())
        },
        Some(&options),
        Some(|| Ok(())),
    )?;
    Ok(())
}
```

:::

`requestTransition` animates the change. Plain writes are atomic and immediate. See [Animation](/js-transitions).

## Spatial layouts

A layout's `compute` may return more than `bounds` for a node: a depth, a turn, a scale and an offset. `ArcLayout` does:
it puts the nodes on an arc around the viewer, the focused one at the front.

::: tabs

```typescript
import { ArcLayout } from "gnoblin-std";

const arc = new ArcLayout({ radius: 1200, spacing: 0.35 });
```

```python
from gnoblin.std import ArcLayout

arc = ArcLayout(radius=1200, spacing=0.35)
```

```rust
use gnoblin_std::{ArcLayout, ArcLayoutOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;
    let _arc = ArcLayout::new_with(
        &objects,
        ArcLayoutOptions {
            radius: Some(1200.0),
            spacing: Some(0.35),
        },
    )?;
    Ok(())
}
```

:::

- Node `i` is at the angle `(i - selected) * spacing`. `selected` is the focused node's index, or 0 when none is.
- Its position is `(sin(angle) * radius, 0, -cos(angle) * radius)`, and it is turned `-angle` about Y.
- `radius` and `spacing` are plain fields. Assigning `arc.radius` lays the workspace out again, and a transition that
  assigns them animates it.

What a flat workspace does with a `transform` placement:

- `offsetX` and `offsetY` move the window from the centre of its `bounds`. With no `bounds` the starting rectangle is
  the window's own size, centred in the workspace's area. The unit is pixels, with y down, and the result is rounded to whole
  pixels.
- `z` is the window's depth. The rotation and the scale are the window's `rotation` and `scale`.

So an arc on a flat workspace shows the windows in a row, each offset across by `radius * sin(angle)`, with the depth and
the turn about Y that the compositor draws for a window placed in 3D.

The compositor's flat view has no eye position, so the arc is not projected as a ring around you. That needs a perspective
camera on the workspace, which is not built.

## Bind a property

`properties.bindDouble(name, compute)` makes a number property of a window follow a function. It sets the property now, and
sets it again when anything the function read has changed.

- A window's `properties` also has `bindBool`, `bindString` and `bindList` (`bind_bool`, `bind_string` and `bind_list` in Python and Rust).
- A scene node's `properties` has `bindNumber`, `bindFlag`, `bindText` and `bindNumbers` (`bind_number`, `bind_flag`, `bind_text` and `bind_numbers` in Python and Rust).
- `properties` is absent (`undefined`, `None`) when the object has no bindings, so check it first, as the examples do.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";

const client = gnoblin.input?.primary?.focusedClient;
client?.properties?.bindDouble("opacity", () => Math.max(0.25, 1 - client.x / 2500));
```

```python
from gnoblin import Gnoblin

gnoblin = Gnoblin.get()
client = gnoblin.input.primary.focused_client
if client is not None and client.properties is not None:
    window = client
    client.properties.bind_double("opacity", lambda: max(0.25, 1 - window.x / 2500))
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let input = model.input()?;
    let primary = input.primary()?;
    let Some(client) = primary.focused_client()? else {
        return Ok(());
    };
    let Some(properties) = client.properties()? else {
        return Ok(());
    };
    properties.bind_double("opacity", move || Ok((1.0_f64 - client.x()? / 2500.0).max(0.25)))?;
    Ok(())
}
```

:::

What it follows:

- The properties of windows and nodes that `compute` reads: `x`, `opacity`, `position`, `committedSize` and the rest.
- The membership of a collection that `compute` reads: `gnoblin.clients.length`, a selection.

- A binding runs when one of those changed and at no other time: there is no timer and no polling.
- A frame runs each changed binding once.
- A binding's write can change what another binding read, so the frame repeats, up to 8 passes.
- A binding still changing after that is in a cycle. The shell reports it once, with the properties it reads.

Each bind call returns a `PropertyBinding` (a `NodeBinding` for a scene node) with `property`, `alive` and `dispose()`.
`properties.unbind(name)` ends the binding of a property, and `properties.dispose()` ends them all.

- Disposing it keeps the last value.
- A binding also ends when its owner is destroyed.
- A second binding of the same property replaces the first.
- A direct assignment to a bound property stands until a value that was read changes.

- `compute` must not write, spawn a program or wait.
- A throw in `compute`, or a property that does not exist or cannot be written, is reported once with the name of the
  binding. It does not throw at `bind`, and the other bindings are not affected. The binding goes on following what it read.
- A plain script field, such as `layout.radius`, is not tracked: nothing tells the shell when it changes.
- A binding that reads its own property sees the value it wrote last and is not run again by its own write.

## Compositor motion and frame callbacks

A shell that holds the tree controls two things the compositor does for each window. They are tree properties, written
over the owner socket by a shell of any language; no shell library wraps them yet. A shell that writes
none of them sees the compositor as before.

**The compositor's own animations.** In a session that has the std library (a shell that imports `gnoblin-std`, or the stock
desktop), the compositor fades a window in when it maps, fades it out when it is closed, unmapped or hidden, and tweens a
change of its placement over 200 ms: the library sets those lengths as it opens.

In a core-only session nothing sets them and they are zero, so a window appears, moves and leaves at once and the shell
animates its own windows. The seat's `builtin_motion` (boolean, `true` by default) turns all three off in a session that
has them: a window then appears, moves and leaves at once, and only the shell's own timed writes play.

- It is one switch for the session, not a property of each window. The fade in starts as the window's node is made,
  before a shell has had a frame to write anything on that node.
- It affects the changes that start after the write; a tween already running finishes.
- It takes effect only while a shell holds the tree. With no shell the compositor's own desktop animates as the std
  library set it, and the compositor sets `builtin_motion` back to `true` when the shell lets go.
- Reduced motion (`reduced_motion` on the seat) still turns every animation off.

**Frame callbacks.** A client draws its next frame when the compositor answers the frame callback it asked for. A
window's `frame_policy` sets when that happens:

| `frame_policy`     | The client's callback is answered                             |
| ------------------ | ------------------------------------------------------------- |
| `"auto"` (default) | After every frame the compositor draws that shows the window. |
| `"hold"`           | Never. The client waits and draws nothing new.                |
| `"throttle"`       | At most `frame_rate` times a second.                          |

- `frame_rate` is in hertz, above 0 and at most 1000, and 30 by default. It is read only while the policy is
  `"throttle"`.
- A held or throttled window keeps its last picture, and the compositor draws it as before.
- A shell sets a window it does not show (another workspace, a minimised window) to `"hold"`, and back to `"auto"`
  to wake it: the client is answered at the next frame.
- Any other policy text is refused `bad-value`, and a `frame_rate` outside the range is refused `out-of-range`.
- The policy is read only while a shell holds the tree. When the shell lets go the compositor sets `frame_policy`,
  `frame_rate` and `report_presented` back to their defaults, so a window is never left frozen with nobody to release it.

**Presentation.** Set a window's `report_presented` to `true` and each frame that draws it writes `presented`: the time
the frame reached the screen, in milliseconds on `CLOCK_MONOTONIC`, the clock `wp_presentation` names. It is the same
reading the window's client is sent in its presentation feedback. It is off by default because a property written on
every frame is a change in every batch.

## Place popups, subsurfaces and layer surfaces

A shell that holds the tree decides where a client's extra surfaces are, instead of the protocol. These are the popups
(menus, tooltips) and subsurfaces of a window, and the layer-shell surfaces (bars, docks, overlays).

The members are tree properties, written over the owner socket by a shell of any language. No shell library wraps them
yet. A shell that writes none of them sees the compositor as before.

The compositor makes a `surface` node for each:

| `role`       | Where the node is                    |
| ------------ | ------------------------------------ |
| `popup`      | A child of its window's node         |
| `subsurface` | A child of its parent surface's node |
| `layer`      | Unparented, like a window            |

| Property             | Type           | Default | What it does                                                         |
| -------------------- | -------------- | ------- | -------------------------------------------------------------------- |
| `placement_override` | `bool`         | `false` | The node's `x` and `y` (layer: and `width`, `height`) are the place. |
| `stack`              | `int`          | `0`     | Order among surfaces with one parent; negative is behind it.         |
| `visible`            | `bool`         | `true`  | `false` hides the surface and what hangs from it.                    |
| `reserve`            | `list<double>` | `[]`    | A layer surface's strip: top, right, bottom, left.                   |

A layer surface also publishes what its client asked for, read-only: `namespace`, `layer`, `anchor`, `margin`,
`requested_size` and `exclusive_zone`.

What is refused and when the overrides count:

- `placement_override` set to `true` on any node whose `role` is not `popup`, `subsurface` or `layer` is refused
  `not-owner`.
- A `reserve` that is not empty or four whole non-negative numbers is refused `bad-value`.
- A timed write (`seconds`) of a surface's `x`, `y`, `width` or `height` is refused `not-animated`. The compositor acts on
  the value the shell settles on.
- The overrides count only while a shell holds the tree and the session is unlocked. A locked session uses the protocol's
  placement and writes it to the node, so a shell writes its place again after the unlock.
- When the shell lets go, the compositor sets `placement_override`, `stack` and `reserve` back to their defaults and shows
  every surface again.

**Subsurfaces.** With `placement_override` the subsurface is drawn and picked at `x`, `y` in its parent's plane.

- Its client is not told: `wl_subsurface` has no event for its place.
- The client's own `set_position` keeps being applied as the protocol says, so writing `false` shows it where the client
  put it.
- A synchronized subsurface keeps showing the content its parent last applied. Only its place moves, so the shell's write
  never shows a commit early.

**Popups.** `x` and `y` are where the popup's surface starts in its parent's plane.

- The compositor runs the popup's positioner on that place, so the client's constraint adjustment still keeps it on the
  monitor. It writes the place the popup settled on back to `x` and `y`.
- A popup whose positioner is reactive is configured there, so what the client knows is what is drawn.
- `xdg_popup` allows no configure after the first for a popup that is not reactive. Such a popup is drawn and picked at the
  place without being told.
- A hidden popup that holds a grab is dismissed, because a popup nobody can see must not keep the keyboard and the pointer.
- A popup of a layer surface is a child of its layer surface's node. It is placed, restacked and hidden like any other popup.

**Layer surfaces.** The node's `x`, `y`, `width` and `height` start as the rectangle the protocol computed from the anchors,
margins and size.

- With `placement_override` the surface is configured at the width and height the shell wrote. A `0` keeps the protocol's
  size on that axis.
- `zwlr_layer_surface_v1.configure` is still sent, and the client still has the anchors and size it asked for.
- The rectangle is also the one the surface's popups are placed against and the one the session reports.
- `reserve` sets the strip the surface takes out of the work area. `[0, 0, 0, 0]` takes none, and an empty list keeps the
  strip the surface's own exclusive zone asks for.
- The compositor draws the surface's buffer at the rectangle, in its layer: `background` and `bottom` under every window,
  `top` and `overlay` above them. A popup of a layer surface is drawn above every layer.

**Restack and hide.** The pointer reaches what is drawn and nothing that is hidden.

- `stack` orders a popup or subsurface among the surfaces that hang from its parent.
- A negative number is behind the parent's own pixels and a positive one in front. Equal numbers keep the protocol's order.
- On a layer surface, `stack` orders it among the surfaces of its layer: a higher number is drawn and picked above a lower
  one. Setting `visible` to `false` hides a layer surface, and its client is not answered frame callbacks while it is hidden.

A surface-level input region override is not offered: the pointer still reaches a surface through the input region its
client set.

## Not built

- A perspective camera that projects a flat workspace's windows is not built, so `ArcLayout` is not seen as a ring.
- `fit` does not fit a client's picture; see [Views](#views).
