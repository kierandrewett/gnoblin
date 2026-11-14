# XR on the JS shell (P9)

Status: the controllers are built in the program and the compositor and tested against a fake runtime and a fake owner;
the room and the shell's claims of a controller are tested against a real compositor. The program's withholding of a
claimed input, and the hold that gives the shell time to claim before the window sees a press, are proven against the
fake runtime only. No headset, no OpenXR runtime and no `gnoblin-xr` binary ran for it.

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

| Spec class or feature                  | Status                             | What it is here                                                                                                                                                               |
| -------------------------------------- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `View` (section 8)                     | Provided + JS                      | The `view` class. A JS `View` sets `source` to a client and reparents nothing. A workspace is not a tree node, so it cannot be shown yet.                                     |
| `XRCamera3D`                           | Provided + JS                      | The `camera` named `head`, `projection = "stereo"`. The program writes its pose and `layer_budget`. The class is a `Camera3D` that fixes the name and takes `near` in metres. |
| `XRViewport`                           | JS                                 | The node named `room` and the camera named `head`. Assigning it to `session.viewport` names them and mounts the room. `composition: "projection"` sets each View's `camera`.  |
| Session                                | Provided + JS                      | The `xr` seat. `alive` and `connected` follow it; `layerBudget` reads `head.layer_budget`; `session.input` is the seat's `InputContext`; `head.pose` reads the head camera.   |
| `XRExtension`                          | JS                                 | `gnoblin.extensions.xr`: null with no session. `extensions.addListener` hears sessions begin and end; `extensions.configure("xr", ...)` shapes the stock room.                |
| `gnoblin/extensions/xr`                | JS + loader                        | A module that throws at load unless XR is declared (`GNOBLIN_XR=1`) or the seat exists. The classes live in `gnoblin/xr`, which is always embedded.                           |
| Room and ring                          | JS                                 | `XRRoom` and `SpatialLayout`: the port of `xr.lua` (`arrangeRing`), radius and spacing in metres, one View per client, none for dialogs or secure clients.                    |
| Head-locked panel                      | Provided + JS                      | A `View` under `head`. `room.pinToHead(client, { x, y, z })` in metres. The program draws it in view space.                                                                   |
| Controllers                            | **New: `controller` class, built** | See "Native work". The program writes one node for each hand.                                                                                                                 |
| `ViewMoveBinding`                      | JS over `controller`               | 6-DoF grab. Writes the View's `x y z rotation`; never the client. A moved View is pinned: the ring leaves it alone. Claims its trigger's buttons for the drag.                |
| `ViewResizeBinding`                    | JS over `controller`               | Writes the View's uniform `scale` on every update, no window write. On release, if aspect or density asks, it writes the window's `width` and `height` once.                  |
| Ray-drag of a far panel                | JS over `controller`               | `ViewMoveBinding({ tracking: "ray" })`: the panel keeps its distance from the hand and its offset from the ray.                                                               |
| `ViewRotate`, `ViewScale`, `ThrowView` | Not built                          | Spec snippets 070 and 071. They would be JS over the same `controller` values.                                                                                                |

## Frames and units

- The room is in millimetres, as in `xr.lua`: `head.metres = 0.001`. A panel's width is `width * scale * metres` metres.
  Every option and result of the JS classes that is a size or a distance is in metres.
- A View's pose is in the program's placement frame: y up, the viewer at the origin looking down negative z, Euler angles
  in radians about X, then Y, then Z (`xr_scene::math::euler_degrees`). `room` is a translation.
- The program reads only the first component of a View's `scale`. A panel has one scale.

## Native work

Built (lane XrControllers). No headset was used; it is proven against the fake runtime and fake owners only.

1. **`controller` class** (`schema/tree/controller.toml`), base `node`. The aim ray's pose is `x y z rotation`, in the
   placement frame (the one `xr_scene::pick` casts in) and in the room's node units, y up, as a View's pose is. Read-only
   properties: `hand`, `tracked`, `trigger` and `grip` (0 to 1), `thumbstick_x`, `thumbstick_y`, `thumbstick_click`,
   `primary`, `secondary`.
2. **Who writes it.** The program, as an observer: one `controller` for each hand, made when the runtime reports the hand
   as active and destroyed when it does not. The compositor lets an observer make that class and write those properties
   on controllers it made, and destroys them when the connection closes (as it does for the seat).
3. **Arbitration.** The shell-writable `controller.claimed` list names inputs (`trigger`, `grip`, `thumbstick`,
   `primary`, `secondary`); the program withholds a claimed trigger, grip or thumbstick from the window under the ray, and
   holds each press for `CLAIM_HOLD` first so that a claim still reaches a press in time (item 6).
   The shell side (lane JsXrClaims): a script writes the list through `XRController.claim(owner, names)`. The shell keeps
   one claim for each owner and each controller and writes the union, in the schema's order, only when it changes, so
   two owners do not clobber each other. The host converts a JS array of strings to the tree's `list<string>` value and
   refuses anything but the five lowercase names as `invalid-value`, which the module turns into a `TypeError` naming
   `Controller#<id>.claimed`. `XRRuntime` claims the buttons of a drag binding's trigger when the drag starts and
   releases them on `onEnd`, `onCancel`, a throw, the binding's removal, the session's end and `retire`. The compositor
   clears a controller's `claimed` when the shell's connection goes (`TreeQueue::reap_shell_nodes`), so a crash cannot
   leave an input withheld. An observer's write of `claimed` is refused `not-holder`.
4. **Choices the design left open.** `tracked` is false when the pose cannot be located and the node keeps its last pose.
   There are no controller signals (the design says none are new). Claim names are lowercase. Writes are thinned to
   about 90 a second, under the compositor's allowance of 120, which controllers have apart from the head's. Another
   observer may write a controller's pose (as any may write the head's) but not its other values.
5. **Not built: per-axis scale.** With it a resize preview could change the aspect; today it grows evenly.
6. **The claim comes before the press (lane XrGrip).** A drag claims after `chordSettleMs`, so the window used to get the
   grip's `BTN_RIGHT` for about 50 ms first. **Decision: option A, a hold in the program.** `xr_input::Arbiter` holds
   every press of the trigger, the grip, the face button and the stick for `CLAIM_HOLD` (150 ms of frame display time)
   and forwards it only if no claim came. A claim in that time discards the press and the window hears nothing of it, not
   even a release. With no claim the press goes out at the deadline at the ray the hand had when it went down; a press let
   go inside the hold is a press then a release a frame apart at the deadline; a stick flick shorter than the hold scrolls
   nothing. A press the window already holds when a late claim arrives is released, as before. The deadline is the only
   clock: a dead, slow or absent shell is the same as no claim, so nothing waits for JS, and pose writes and layer
   submission never see the hold. `chordSettleMs` is capped at 60 ms: settle plus the claim's trip must fit in the hold
   (the real stock shell on a headless compositor made a claim visible 106 to 125 ms after the press, with 25 ms of test
   polling in that, so 150 ms is not generous); a claim later than the hold leaks as it did. **Cost:** every click of these
   inputs lands 150 ms (to the next frame) late, bound to a binding or not. **Rejected:** (B) a standing `claimable` list
   and a per-press `pass` verdict, which would spare unbound inputs and taps, but needs a new shell-written property, a
   verdict protocol, a replay in the program and a rule for a shell that dies mid-verdict; (C) a per-input `claim_mode`
   is B with the flag on the input. B's standing list alone could later drop the hold from inputs no binding uses.

## Acceptance (owner-spec section 21)

| Bullet | Where it is shown                                                                                                                           |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------- |
| XR1    | `crates/gnoblin-js/tests/xr.rs`: `xr1_the_classes_relate_as_the_spec_says` (XRCamera3D is a Camera3D, bindings are Bindings, session.input) |
| XR3    | `xr3_a_grip_drag_moves_the_view_and_changes_nothing_of_the_client`: the View moves, no `Set` reaches the window                             |
| XR4    | `xr4_a_resize_changes_the_views_size_on_every_update_...`: zero window writes while dragging, one transaction on release                    |
| XR5    | **Unverified.** The program's pose writer has one write in flight and a 250 ms timeout; no test stalls a shell while the program runs       |
| XR6    | `xr6_a_secure_client_never_gets_a_view...` (policy) and `view_secure_tests.rs` in `gnoblin-backend` (the compositor masks it)               |
| Room   | `crates/gnoblin/tests/js_shell.rs`: a real compositor, a real shell and an observer that makes the seat                                     |
| Claims | `js_shell.rs`: an observer plays the program, the stock grab claims the grip on its `controller`, and the compositor clears it              |
| Hold   | `xr-input/tests/hands.rs`, `bridge/controller_nodes.rs`: claimed never reaches the window, unclaimed after `CLAIM_HOLD`; fake runtime only  |
