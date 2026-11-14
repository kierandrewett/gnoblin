# XR on the JS shell (P9)

Status: built and tested against a fake tree and, for the room, a real compositor. No headset, no OpenXR runtime and no
`gnoblin-xr` binary ran for it.

Owner-spec section 13 asks for a native `.so` extension. J7 replaces that with the out-of-process program of D34 and
D35, so this document says what each spec class becomes in that model. See `design/js-runtime.md` (J7, "P9 XR status"),
`design/xr-extension.md` and `docs/xr.md`.

## The model in four lines

- **The XR program** (`extensions/gnoblin-xr`) is an observer on the owner socket. It reads the `View` nodes under the
  node named `room`, submits layers, writes the pose of the camera named `head`, and makes the seat named `xr`.
- **The tree** is the only interface. Nothing is loaded into the shell, and the shell never calls the program.
- **The JS shell** decides policy: it builds the room, places the Views and moves them. The classes of
  `gnoblin/extensions/xr` are JS objects over tree nodes, like `Client`.
- **Presence** is the `xr` seat: it exists while the program has a session and the program destroys it when the session
  ends (`docs/xr.md`, "When the headset runtime goes").

## What is a node, a class and a signal

| Thing                           | Is                                                                                                     |
| ------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Session, head, room, a panel    | Tree nodes: the `xr` seat, the `camera` named `head`, the `node` named `room`, `view` nodes            |
| `XRSession`, `View`, `Camera3D` | JS classes, one node each. `alive` is false once the node is gone, and a write then throws             |
| A binding, the ring, a drag     | JS only. The state is in the script; the tree sees only the Views' own properties                      |
| A change the program makes      | A tree `set` (the head pose, `layer_budget`, a controller's values). The shell reads it once a frame   |
| Tree signals                    | None are new. A handler of the shell that stalls delays the next placement, never the program's writes |

## Spec class by class

"Provided" means the program or the compositor already publishes it. "JS" is policy in `data/js/gnoblin/xr.ts` and
`scene.ts`. "New" names the schema addition.

| Spec class or feature                  | Status                      | What it is here                                                                                                                                                               |
| -------------------------------------- | --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `View` (section 8)                     | Provided + JS               | The `view` class. A JS `View` sets `source` to a client and reparents nothing. A workspace is not a tree node, so it cannot be shown yet.                                     |
| `XRCamera3D`                           | Provided + JS               | The `camera` named `head`, `projection = "stereo"`. The program writes its pose and `layer_budget`. The class is a `Camera3D` that fixes the name and takes `near` in metres. |
| `XRViewport`                           | JS                          | The node named `room` and the camera named `head`. Assigning it to `session.viewport` names them and mounts the room. `composition: "projection"` sets each View's `camera`.  |
| Session                                | Provided + JS               | The `xr` seat. `alive` and `connected` follow it; `layerBudget` reads `head.layer_budget`; `session.input` is the seat's `InputContext`; `head.pose` reads the head camera.   |
| `XRExtension`                          | JS                          | `gnoblin.extensions.xr`: null with no session. `extensions.addListener` hears sessions begin and end; `extensions.configure("xr", ...)` shapes the stock room.                |
| `gnoblin/extensions/xr`                | JS + loader                 | A module that throws at load unless XR is declared (`GNOBLIN_XR=1`) or the seat exists. The classes live in `gnoblin/xr`, which is always embedded.                           |
| Room and ring                          | JS                          | `XRRoom` and `SpatialLayout`: the port of `xr.lua` (`arrangeRing`), radius and spacing in metres, one View per client, none for dialogs or secure clients.                    |
| Head-locked panel                      | Provided + JS               | A `View` under `head`. `room.pinToHead(client, { x, y, z })` in metres. The program draws it in view space.                                                                   |
| Controllers                            | **New: `controller` class** | See below. Today the program sends the right hand's aim ray as pointer events on the `xr` seat and publishes no pose or button value.                                         |
| `ViewMoveBinding`                      | JS over `controller`        | 6-DoF grab. Writes the View's `x y z rotation`; never the client. A moved View is pinned: the ring leaves it alone.                                                           |
| `ViewResizeBinding`                    | JS over `controller`        | Writes the View's uniform `scale` on every update, no window write. On release, if aspect or density asks, it writes the window's `width` and `height` once.                  |
| Ray-drag of a far panel                | JS over `controller`        | `ViewMoveBinding({ tracking: "ray" })`: the panel keeps its distance from the hand and its offset from the ray.                                                               |
| `ViewRotate`, `ViewScale`, `ThrowView` | Not built                   | Spec snippets 070 and 071. They would be JS over the same `controller` values.                                                                                                |

## Frames and units

- The room is in millimetres, as in `xr.lua`: `head.metres = 0.001`. A panel's width is `width * scale * metres` metres.
  Every option and result of the JS classes that is a size or a distance is in metres.
- A View's pose is in the program's placement frame: y up, the viewer at the origin looking down negative z, Euler angles
  in radians about X, then Y, then Z (`xr_scene::math::euler_degrees`). `room` is a translation.
- The program reads only the first component of a View's `scale`. A panel has one scale.

## New native work

Only the controllers need a schema addition. Nothing below is built.

1. **`controller` class** (`schema/tree/controller.toml`), base `node`. The aim ray's pose is `x y z rotation`, in the
   placement frame (the one `xr_scene::pick` casts in). Read-only properties: `hand`, `tracked`, `trigger` and `grip` (0
   to 1), `thumbstick_x`, `thumbstick_y`, `thumbstick_click`, `primary`, `secondary`.
2. **Who writes it.** The program, as an observer: one `controller` for each hand, written at most once a frame. The
   compositor must let an observer make that class and write those properties, and destroy the nodes when the
   connection closes (as it does for the seat).
3. **Arbitration.** The program sends grip as `BTN_RIGHT` to the window under the ray. A binding that owns Grip needs a
   shell-writable `controller.claimed` list that the program obeys.
4. **Optional: per-axis scale.** With it a resize preview could change the aspect; today it grows evenly.

The shell side of 1 to 3 is tested against `controller` nodes the tests make. Until the program writes them, a headset
session has no controllers and no drag starts.

## Acceptance (owner-spec section 21)

| Bullet | Where it is shown                                                                                                                           |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------- |
| XR1    | `crates/gnoblin-js/tests/xr.rs`: `xr1_the_classes_relate_as_the_spec_says` (XRCamera3D is a Camera3D, bindings are Bindings, session.input) |
| XR3    | `xr3_a_grip_drag_moves_the_view_and_changes_nothing_of_the_client`: the View moves, no `Set` reaches the window                             |
| XR4    | `xr4_a_resize_changes_the_views_size_on_every_update_...`: zero window writes while dragging, one transaction on release                    |
| XR5    | **Unverified.** The program's pose writer has one write in flight and a 250 ms timeout; no test stalls a shell while the program runs       |
| XR6    | `xr6_a_secure_client_never_gets_a_view...` (policy) and `view_secure_tests.rs` in `gnoblin-backend` (the compositor masks it)               |
| Room   | `crates/gnoblin/tests/js_shell.rs`: a real compositor, a real shell and an observer that makes the seat                                     |
