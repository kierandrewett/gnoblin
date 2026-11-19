# Scene, views and property bindings

The scene is the tree of nodes a script makes.

- Plain `Node` objects, `View`s that draw a client or a workspace as a picture, and cameras are in it.
- `gnoblin.scene.root` is where a script mounts them.
- A node is placed with `position`, `rotation`, `scale` or one `transform`.
- A property of a node or a window can follow the properties it was computed from.

```ts
import { ArcLayout, Node, View, Workspace } from "gnoblin-std";

const desktop = new Workspace({ id: "desktop", layout: new ArcLayout({ radius: 1200, spacing: 0.35 }) });
gnoblin.workspaces.add(desktop);

const overview = new Node({ id: "overview" });
overview.add(new View({ content: desktop, size: { width: 700, height: 400 } }));
gnoblin.scene.root.add(overview);

const client = gnoblin.clients.focused;
client?.properties.bind("opacity", () => (client.focused ? 1 : 0.7));
```

Put the file in `~/.config/gnoblin/shell.ts`. The windows of `desktop` sit on an arc, the scene holds a small picture of the
workspace, and the focused window is drawn at full opacity while the others are dimmed.

## The scene

`gnoblin.scene` is the one scene. Its members:

| Member          | Type   | Meaning                                                                                |
| --------------- | ------ | -------------------------------------------------------------------------------------- |
| `root`          | `Node` | The root of the tree. `root.add(node)` mounts a node under it. It cannot be destroyed. |
| `metresPerUnit` | number | Metres in one scene unit. Default 1, which is logical units. The script keeps it.      |

`root.children` lists the nodes a script made. Windows are reached through `gnoblin.clients` and workspaces through
`gnoblin.workspaces`: a workspace is a script object and not a tree node, so neither is in `children`.

## Place a node

A node made with `new Node(options)` is in the tree under no parent until it is added to one. Every option is optional.

| Option or member     | Type                                       | Meaning                                                                                          |
| -------------------- | ------------------------------------------ | ------------------------------------------------------------------------------------------------ |
| `id`                 | string                                     | The node's stable id. It is also its `name`, and `node.id` answers with it.                      |
| `position`           | `{ x, y, z }` or `[x, y, z]`               | Where the node is, in its parent's units. Reads give a vector with `distanceTo`.                 |
| `rotation`           | `{ x, y, z }`, `[x, y, z]` or a quaternion | Angles about X, Y and Z in radians. A quaternion `{ x, y, z, w }` is stored as its Euler angles. |
| `scale`              | number, `{ x, y, z }` or `[x, y, z]`       | A number scales every axis.                                                                      |
| `transform`          | `{ position, rotation, scale }`            | Any of the three parts. A part left out is not written.                                          |
| `visible`, `opacity` | boolean, number                            | As on a window.                                                                                  |
| `input`              | `"forward"`, `"capture"`, `"none"`         | What a pointer ray does at the node. See [Input](#input).                                        |
| `metresPerUnit`      | number                                     | Metres in one unit of this node's space. Default 1.                                              |

- `rotation` and `scale` read back as three numbers that also answer to `x`, `y` and `z`.
- So `node.rotation.y` and `node.rotation[1]` are the same turn. Rotations turn about X, then Y, then Z.
- A write is checked before anything is written. A value that is not finite throws a `TypeError` that names the node and
  the member, as in `Node#12.rotation`.

`transform` is one write. `node.transform = { rotation: { x: 0, y: 0.12, z: 0 } }` changes the rotation and leaves the
position and the scale alone. All the parts are checked first, so one bad part writes nothing, and the writes of a frame
go out in one transaction.

```ts
const room = new Node({ id: "xr-room", position: { x: 0, y: 1.2, z: -2 }, scale: 1 });
room.transform = { rotation: { x: 0, y: Math.PI / 4, z: 0 } };
```

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
  hears `onPointerEnter`, `onPointerMotion`, `onPointerLeave`, `onPointerPress` and `onPointerRelease`, each with
  `{ node, x, y, button }` in the View's own plane. A script can use this to draw its own interaction over a client's
  picture.
- A node that is not a View of a client has nothing to forward to, so `"forward"` and `"capture"` both send the script
  its pointer events. A View with no `content` is the same.
- The handlers are plain members, set as in `panel.onPointerPress = (event) => { ... }`. A throw in one is reported
  like a throw in a listener.

```ts
const panel = new View({ content: client, size: { width: 400, height: 300 }, input: "capture" });
panel.onPointerPress = (event) => {
    if (event.x < 40 && event.y < 40) {
        client.close();
    } else {
        panel.input = "forward"; // from the next move, the client gets the pointer
    }
};
```

## Views

A `View` draws another node as a picture. Its `content` is a client or a workspace, and neither is moved or reparented.

| Option or member | Type                                | Meaning                                                         |
| ---------------- | ----------------------------------- | --------------------------------------------------------------- |
| `content`        | client, workspace or `null`         | What the View shows.                                            |
| `size`           | `{ width, height }`                 | The View's rectangle. The same numbers as `width` and `height`. |
| `contentFit`     | `"contain"`, `"cover"`, `"stretch"` | How the picture is fitted to `size`. Default `"stretch"`.       |
| `camera`         | `Camera3D` or `null`                | The camera the content is drawn through. `null` is flat.        |

```ts
const panel = new View({
    content: gnoblin.clients.focused,
    size: { width: 0.9, height: 0.55 },
    contentFit: "contain",
    input: "forward",
});
gnoblin.scene.root.add(panel);
```

A View of a client is that client's picture, drawn by the compositor at `size`. The compositor stretches it, so
`contentFit` is kept and read back for a client but only changes how a workspace is drawn.

A workspace is not a tree node, so the compositor has no node to draw for it. The View keeps a picture made of one child
View for each window on the workspace, each showing that window and placed where the window sits in the workspace's area.

- The area is the output that shows the workspace, or the smallest rectangle that holds its windows.
- `contentFit` scales the picture into `size`. `"contain"` keeps the aspect ratio and centres it. `"cover"` keeps it and
  fills `size`; what falls outside is not clipped. `"stretch"` scales each axis on its own.
- The picture follows the workspace. A window that opens, closes, moves or resizes changes it in the same frame.
- A window that its layout hides, such as the others of a monocle layout, is hidden in the picture, and so are minimised
  windows.
- `view.destroy()` removes the picture. Its child Views are internal and are not in `view.children`.

`client.committedSize` is the size the client last committed, as `{ width, height }`: what the compositor last published,
as against `width` and `height`, which are what the shell asked for.

## Place a window with a transform

- A window takes the same `position`, `rotation`, `scale` and `transform` as a node.
- A window's `position` is its `x`, `y` and its depth `z`.
- A transform that gives a position moves the window's rectangle, and a layout that places the window will place it again.
- The rotation and the scale are the window's own.

```ts
client.set({ opacity: 0.92, transform: { rotation: { x: 0, y: 0.12, z: 0 } } }, { duration: 250, easing: "ease-out" });
client.set({ opacity: 1, visible: true, transform: { rotation: [0, 0, 0], scale: 1 } });
```

`set` is one `requestTransition`. With options the compositor animates the change, and without them it is atomic and
immediate. See [Animation](/js-transitions).

## Spatial layouts

A layout's `compute` may return a `transform` for a node instead of, or as well as, `bounds`. `ArcLayout` does: it puts the
nodes on an arc around the viewer, the focused one at the front.

```ts
const arc = new ArcLayout({ radius: 1200, spacing: 0.35 });
```

- Node `i` is at the angle `(i - selected) * spacing`. `selected` is the focused node's index, or 0 when none is.
- Its position is `(sin(angle) * radius, 0, -cos(angle) * radius)`, and it is turned `-angle` about Y.
- `radius` and `spacing` are plain fields. Assigning `arc.radius` lays the workspace out again, and a transition that
  assigns them animates it.

What a flat workspace does with a `transform` placement:

- `position.x` and `position.y` move the window from the centre of its `bounds`. With no `bounds` the starting rectangle is
  the window's own size, centred in the workspace's area. The unit is pixels, with y down, and the result is rounded to whole
  pixels.
- `position.z` is the window's depth `z`. The rotation and the scale are the window's `rotation` and `scale`.
- A `z`, `rotation` or `scale` given on the placement itself wins over the transform's.

So an arc on a flat workspace shows the windows in a row, each offset across by `radius * sin(angle)`, with the depth and
the turn about Y that the compositor draws for a window placed in 3D.

The compositor's flat view has no eye position, so the arc is not projected as a ring around you. That needs a perspective
camera on the workspace, which is not built.

## Bind a property

`properties.bind(name, compute)` makes a property follow a function. It sets the property now, and sets it again when
anything the function read has changed. `bind(name, compute)` on the same object does the same.

```ts
client.properties.bind("opacity", () => Math.max(0.25, 1 - client.x / 2500));

const development = gnoblin.workspaces.get("development");
development?.properties.bind("name", () => (development.clients.length > 4 ? "busy" : "quiet"));
```

What it follows:

- The properties of windows and nodes that `compute` reads: `x`, `opacity`, `position`, `committedSize` and the rest.
- The membership of a collection that `compute` reads: `workspace.clients.length`, `gnoblin.clients`, a selection.

- A binding runs when one of those changed and at no other time: there is no timer and no polling.
- A frame runs each changed binding once.
- A binding's write can change what another binding read, so the frame repeats, up to 8 passes.
- A binding still changing after that is in a cycle. The shell reports it once, with the properties it reads.

`bind` returns a `PropertyBinding` with `property`, `alive` and `dispose()`.

- Disposing it keeps the last value.
- A binding also ends when its owner is destroyed or, for a workspace, removed from `gnoblin.workspaces`.
- A second binding of the same property replaces the first.
- A direct assignment to a bound property stands until a value that was read changes.

- `compute` must not write, spawn a program or wait.
- A throw in `compute`, or a property that does not exist or cannot be written, is reported once with the name of the
  binding. It does not throw at `bind`, and the other bindings are not affected. The binding goes on following what it read.
- A plain script field, such as `layout.radius`, is not tracked: nothing tells the shell when it changes.
- A binding that reads its own property sees the value it wrote last and is not run again by its own write.

## Compositor motion and frame callbacks

A shell that holds the tree controls two things the compositor does for each window. They are tree properties, written
over the owner socket by a shell of any language; the TypeScript library does not wrap them yet. A shell that writes
none of them sees the compositor as before.

**The compositor's own animations.** In a session that has the std library (a shell that imports `gnoblin-std`, or the stock
desktop), the compositor fades a window in when it maps, fades it out when it is closed, unmapped or hidden, and tweens a
change of its placement over 200 ms: the library sets those lengths as it opens. In a core-only session nothing sets them
and they are zero, so a window appears, moves and leaves at once and the shell animates its own windows. The seat's
`builtin_motion` (boolean, `true` by default) turns all three off in a session that has them: a window then appears, moves and leaves at once, and only the shell's own timed writes
play.

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

The members are tree properties, written over the owner socket by a shell of any language. The TypeScript library does not
wrap them yet. A shell that writes none of them sees the compositor as before.

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
- `contentFit` does not fit a client's picture; see [Views](#views).
