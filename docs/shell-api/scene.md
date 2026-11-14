# Scene, views and property bindings

The scene is the tree of nodes a script makes.

- Plain `Node` objects, `View`s that draw a client or a workspace as a picture, and cameras are in it.
- `gnoblin.scene.root` is where a script mounts them.
- A node is placed with `position`, `rotation`, `scale` or one `transform`.
- A property of a node or a window can follow the properties it was computed from.

```ts
import { ArcLayout, Node, View, Workspace } from "gnoblin";

const desktop = new Workspace({ id: "desktop", layout: new ArcLayout({ radius: 1200, spacing: 0.35 }) });
gnoblin.workspaces.add(desktop);

const overview = new Node({ id: "overview" });
overview.add(new View({ content: desktop, size: { width: 700, height: 400 } }));
gnoblin.scene.root.add(overview);

const client = gnoblin.clients.focused;
client?.properties.bind("opacity", () => (client.focused ? 1 : 0.7));
```

Put the file in `~/.config/gnoblin/init.ts`. The windows of `desktop` sit on an arc, the scene holds a small picture of the
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

`input` says what a pointer ray does when it reaches the node:

| Value       | Meaning                                                                       |
| ----------- | ----------------------------------------------------------------------------- |
| `"forward"` | The ray hits the node and, for a `View`, goes on to its content. The default. |
| `"capture"` | The ray hits the node and stops there.                                        |
| `"none"`    | The ray misses the node. Its children are still tested.                       |

- A write also takes `true` (`"forward"`) and `false` (`"none"`), and a read gives the string.
- The tree has one flag for this, `input`, which is true for `"forward"` and `"capture"`.
- The script keeps the difference between those two. Only code that reads `node.input` sees `"capture"`, as the XR
  extension's pointer picking does.

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

## Not built

- A perspective camera that projects a flat workspace's windows is not built, so `ArcLayout` is not seen as a ring.
- `contentFit` does not fit a client's picture; see [Views](#views).
- `"capture"` is not a state of the tree: it is kept by the script.
