# XR mode

XR mode shows your Gnoblin windows in a headset. Each window is a flat panel hanging in a room round you. You look at
a panel and point at it with a controller to move the pointer over that window, click it and scroll it.

Gnoblin draws every panel itself, with the frame, the shadow and the effects you see on a monitor. A separate program,
`gnoblin-xr`, copies each panel into an OpenXR session and tells the OpenXR runtime where to put it. The runtime
(Monado or WiVRn) places the panels and shows them in the headset. The program never decides where a window goes.

## How XR mode starts

XR mode is off. The guardian starts `gnoblin-xr` only when you opt in:

1. Set `GNOBLIN_XR=1` in the environment the guardian is started with. Any other value, or no value, leaves it off, even
   when the program is installed.
2. Have `gnoblin-xr` installed. The guardian looks for it beside `gnoblin`, in the prefix's `libexec`, then on `PATH`.
   `cargo xtask dist install` installs it as `libexec/gnoblin-xr` when it was built (`just dist` builds it). To run a
   build that is not installed, set `GNOBLIN_XR_PROGRAM` to its path; it has no effect without `GNOBLIN_XR=1`.

Release packages do not ship `gnoblin-xr`:

- The extension is its own workspace, and nothing in the packaging (`pkgsmith`, `nix`, the CI release jobs) builds it.
- `cargo xtask dist install` installs it only when a developer built it first (`just dist`).
- A package install therefore has none, and `GNOBLIN_XR=1` logs the one warning and starts without it.

If `GNOBLIN_XR=1` is set and no binary is found, the guardian logs one warning and the session starts without it.

The setting is an environment variable because the guardian starts before the shell runs its configuration and reads no
configuration file. A `gnoblin.configure` key would take effect only after a restart of the guardian's child list, which
nothing does today.

### What the program gets

- **A Wayland connection with the `view` role.** The guardian makes a connected socket pair for each start. The program
  receives one end as `WAYLAND_SOCKET`, and the compositor adopts the other as a client that sees `wl_shm`,
  `ext_image_copy_capture_manager_v1` and `gnoblin_view_source_manager_v1` and no other global. The role belongs to the
  connection, so the program cannot ask for more by connecting to the session socket.
- **The owner socket.** The program connects to `GNOBLIN_IPC_SOCKET`, or `$XDG_RUNTIME_DIR/gnoblin/ipc.sock`, as every
  client of the session's user does. It attaches to the tree as an observer: it reads the snapshot and the changes beside the stock shell, which holds the tree, and it may only create the `xr` seat, destroy it and send pointer input. The seat goes when the program's connection closes.

### What the program does once it runs

`gnoblin-xr` starts the OpenXR session. When the session is up it creates a seat named `xr` on the owner socket.
That seat is the headset's pointer. The stock shell builds the room when this seat appears and removes it when
the seat goes. Nothing in your configuration needs to name the room.

### When the headset runtime goes

The OpenXR runtime is not part of Gnoblin, and it can be missing, refuse the session, lose it or restart under the
program. None of that stops `gnoblin-xr`, because a program that exits is restarted only five times:

- **No runtime, or a refused session.** The program logs the reason once and tries again after 250 ms. Each failed
  try doubles the wait, up to 4 s. The wait starts again from 250 ms after a session that stayed up for 30 s.
- **A lost session, or a runtime that asks the program to exit.** The session ends and a new one is tried in the same
  way. The program keeps its connection to the compositor throughout, so nothing in the tree has to be built again.
- **While there is no session.** The headset's pointer is gone, so the program destroys the `xr` seat it made and the
  stock shell takes the room down. The windows are untouched. The seat comes back, and the room with it, when the next
  session starts. A seat named `xr` that was already there when the program started is not the program's, so it is
  left alone.
- **A swapchain the runtime refuses.** The View loses its layer for that frame, the program destroys the swapchain and
  makes a new one on the next frame, and the picture is copied into it again.

The compositor is a separate process and never waits for any of this. The program exits only when the compositor closes
its socket (status 0), or when the socket fails (status 1, and the guardian restarts the program).

### When it stops

The guardian starts the program after the front and before the autostart applications, and does not wait for it to
report ready. It stops it before the front, after the applications.

If the program exits, the guardian restarts it:

- The wait before the first restart is 250 ms. It doubles for each restart in a row, up to 4 s.
- A program that stays up for a minute gets its budget back.
- After the fifth failed restart the guardian logs one warning and starts it no more:
  `[guardian] xr failed after 5 restarts; it is optional, so the session continues without it`.
- The session is not reported degraded and no other child is touched.
- A start that cannot be made (no compositor control socket, or the program cannot be executed) counts as a failed run
  of the same budget.

Logs are under the `xr` name: `[guardian] started xr (generation N, pid P)` and the program's own lines.

## What the stock shell does

The stock shell's XR side is `data/js/gnoblin/xr.ts` (the TypeScript shell; the Lua shell it replaces has the same
behaviour in `data/lua/gnoblin/shell/xr.lua`). It reacts to the seat named `xr`.

When the seat arrives the shell builds a node named `room` under `root`. The room holds:

- A camera named `head`, with `projection = "stereo"` and `metres = 0.001`. The room is in millimetres: 1 unit is
  0.001 m. A panel you put under `head` stays fixed to your head. The stock shell puts nothing there.
- One `View` node for each window, in the order the windows gained their View.

When the seat is destroyed the shell destroys the room. The Views go with it. The windows were never moved, so they are
still on their workspaces.

A room that already exists when the seat arrives is left alone. Gnoblin treats it as yours.

### A View for each window

Every window gets a `View` under the room when it maps, and loses it when the window is destroyed. The View has:

| Property             | Value                                                                |
| -------------------- | -------------------------------------------------------------------- |
| `source`             | The window. The extension draws this node.                           |
| `width` and `height` | The window's size in pixels. They follow the window when it resizes. |
| `visible`            | `false` while the window is minimised.                               |
| `input`              | `true`. A controller ray that crosses the panel reaches the window.  |
| `x`, `y`, `z`        | The panel's position in millimetres, set by the ring below.          |
| `rotation`           | Radians about each axis. Only the turn about `y` is used.            |
| `scale`              | Millimetres per window pixel, the same on all three axes.            |

A dialog has no View of its own. Its parent's View shows it, because the dialog is a child of the parent window.

A client that is `secure` never gets a View, and one that turns secure loses it. The compositor enforces that on its
own: it masks a secure node in every View's picture (`design/security-secure-content-in-views.md`).

A script can keep other windows out of the room with an `accept` function. It runs every frame, so a window comes in
when it starts to say yes:

```ts
gnoblin.extensions.configure("xr", {
    accept: (client) => !client.appId.startsWith("org.keepassxc"),
});
```

The Lua shell reads the window rule key `xr = false` for the same thing. The TypeScript rules do not have that key
(`data/js/gnoblin/rules.ts` refuses it by name), so the function is the one way.

### The ring

The panels sit on a ring round you. The ring is `arrangeRing` in `xr.ts`, a pure function of the windows' sizes and
the focus, so a window's place never depends on anything but those:

- The ring has a radius of 1.6 m, and the centre of each panel is 1.4 m above the room's floor. The extension reports
  poses in OpenXR's local space, whose origin is where your head starts, and the room's floor is 1.4 m below that
  origin, so the panels start at about eye height.
- A window of 800 pixels is 0.6 m tall on the ring. All windows use the same pixel size, so a larger window is a
  larger panel.
- Each panel faces you. Neighbouring panels are 3 degrees apart.
- The focused window is at the front, centre, 15% larger and 0.1 m nearer. Its neighbours in creation order stand to
  either side, wrapping round. When the focus moves, the ring turns.
- With no focused window, the first window is at the front and is not enlarged.
- Panels reach at most 150 degrees to each side. When wide windows would reach further, every panel shrinks by the same
  factor until they fit.
- The ring holds 12 panels. Further windows stack behind it: each one sits 0.25 m behind the ring panel at the same
  angle, and every further turn of 12 adds another 0.25 m. A focused window beyond the twelfth takes the ring's last
  place.
- A minimised window leaves the ring and the others close the gap.

The extension draws the Views in tree order and nothing depth-tests, so the shell keeps the room's children from
farthest to nearest: stacked panels first, then the ring, then the focused panel last.

The shell writes a pose only when it changes, so a window that does not change costs no writes.

### Stereo Views

A View is flat unless its `camera` names a camera whose `projection` is `stereo`. The compositor then draws the View's
source twice, as two eyes side by side in the View's picture: the left eye in the left half, and the right eye in the
right half (an odd width gives the extra column to the right eye).

The extension shows such a View as an OpenXR
projection layer instead of a quad, with the two eyes the camera states.

| Camera property                               | Meaning                                                                                                          |
| --------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `x`, `y`, `z`, `rotation`                     | The point between the eyes, in the source's own units, and the way it looks. The camera looks down negative `z`. |
| `ipd`                                         | The distance between the eyes, in metres. 0.063 by default.                                                      |
| `metres`                                      | Metres per unit, which turns `ipd` into units. 0.001 by default.                                                 |
| `fov`                                         | The vertical field of view in degrees, 60 by default. The horizontal field follows one eye's half of the View.   |
| `fov_left`, `fov_right`, `fov_up`, `fov_down` | An off-axis frustum in degrees, for the left eye. All four zero means the symmetric frustum from `fov`.          |
| `near`, `far`                                 | The clip distances, in units.                                                                                    |

- The left eye sits half of `ipd` to the camera's left and the right eye half to its right, along the camera's own x
  axis, so turning the camera turns the pair.
- `fov_left` and `fov_down` are negative for a symmetric view, as OpenXR states a field of view. The right eye uses the
  mirror image of the left eye's frustum, which is how a headset's two eyes differ: the nose side sees less.
- A frustum that is not one (`fov_left` not below `fov_right`, `fov_down` not below `fov_up`, an angle of 90 degrees or
  more), an `ipd` below zero or a `metres` of zero or less draws a blank picture, and the extension skips the View.
- The scene's y axis grows down and OpenXR's grows up, so the extension states the camera's position as `(x, -y, z)`
  metres and a turn about x or z reversed. The planner's `rig` module writes the rule down and tests it.

The eyes come from the tree, so the picture and the layer cannot disagree, and the same tree always gives the same
layer. The one exception is a View through the `head` camera, whose picture the extension pairs with the eyes it was
drawn from (see "Head tracking").

### Head tracking

A stereo View whose `camera` is the camera named `head` follows your head. The extension measures the head pose each
frame and writes it onto that camera, and the compositor draws the View again from the new pose.

- The pose goes into `x`, `y`, `z` and `rotation`, in one write, in the camera's units, with the scene's y axis flipped.
- Nothing is written while no View goes through the `head` camera.
- Writes are at most 60 a second, and the next one waits until the last has shown in the tree and every View through
  the camera has drawn from it, or 250 ms have passed.
- A pose measured more than 100 ms before the write is dropped. A pose with a NaN, an infinity or no rotation is never
  sent. A head that moved less than 0.1 mm and 0.005 degrees is not written again.
- When the runtime reports no head, nothing is written and the camera keeps its last pose. The log says
  `head tracking: untracked`.
- When the owner refuses a write, the extension logs the code once, waits a second and tries again. A `stale-base`
  refusal is retried on the next frame.
- When the camera is destroyed the writes stop, and a View whose camera is gone is a flat panel. A camera the shell
  makes again is followed from the next frame.

Only the program on the session owner's connection can write the head camera. It may write that one camera's pose and
`layer_budget` and nothing else, and the compositor refuses it `rate-limited` past 120 writes a second after a burst of 4. Any other node is refused `not-holder`, and a number that is not finite `not-finite`.

**Which eyes go with which picture.** A projection layer declares the eyes its picture was drawn from, and the runtime
reprojects it against your head when it shows it. The extension cannot read which pose the compositor drew from, so it
uses one rule:

- A picture first seen while a write is in flight is paired with that write's pose.
- Any other picture is paired with the camera as the tree holds it.

A picture drawn just before the write can be paired wrongly until the next picture, which the write itself causes. That
is at most one write, 1/60 s of head motion. No headset has shown how it looks.

### Layer budget

The runtime allows a fixed number of layers in one frame. The extension writes that number to the head camera's
read-only `layer_budget`. When more Views are ready than the budget allows, it keeps them in this order:

1. The View that shows a window with keyboard focus on any seat.
2. The Views nearest your head. A panel in the room is measured from your head, a panel under the `head` camera from
   the head itself.
3. Plan order, to break a tie.

The Views that fit are still submitted in plan order, so dropping one never changes what covers what. A View with no
picture yet takes no budget. The log gets one line when the budget or the number dropped changes, such as
`layer budget 16: 2 views dropped`. The stock shell does not read `layer_budget`.

### Pointer and focus

A controller ray that crosses a panel reaches the window under it. The compositor delivers the pointer as input on the
window, not on the View, so the stock handlers see it like any other pointer: pressing a window focuses it and raises
it.

The shell does nothing else for the `xr` seat:

- The seat schema (`seat.toml`) has no property that says which window the ray is over, so the shell cannot follow the
  ray by itself.
- Whether the compositor sends `enter` events to the windows for the `xr` seat, which the hover focus modes would need,
  was not checked.

### Controller input

The extension reads the right-hand controller through one OpenXR action set, `gnoblin`, and sends what it reads to the
`xr` seat. The bindings are suggested for five interaction profiles; the runtime picks the one that matches the
controller in the hand.

| Input     | `khr/simple_controller` | `oculus/touch_controller`, `meta/touch_controller_plus`, `valve/index_controller` | `htc/vive_controller`           | Sent as                               |
| --------- | ----------------------- | --------------------------------------------------------------------------------- | ------------------------------- | ------------------------------------- |
| Aim       | `aim/pose`              | `aim/pose`                                                                        | `aim/pose`                      | The pointing ray                      |
| Select    | `select/click`          | `trigger/value`                                                                   | `trigger/value`                 | `BTN_LEFT` (`0x110`)                  |
| Secondary | `menu/click`            | `squeeze/value` or `b/click`                                                      | `squeeze/click` or `menu/click` | `BTN_RIGHT` (`0x111`)                 |
| Scroll    | none                    | `thumbstick`                                                                      | `trackpad`                      | Axis amounts, vertical and horizontal |

- **Buttons.** A trigger or grip presses at 75% pull and releases at 25%, so a half pull neither presses nor chatters.
  A click input reports 0 or 1.
- **Scroll.** A stick inside a 15% deadzone does nothing. Past it, the speed grows from zero to 400 axis units a second
  at full deflection. Push up to scroll up the page. A frame longer than 100 ms scrolls as 100 ms.
- **Grab.** A press goes to the window under the ray and holds the pointer on it, as a mouse button does. While any
  button is down, motion, scroll and the release go to that window. The ray does not leave it or enter another.
- **Drag.** The coordinates keep following the ray on the grabbed window's plane, so a drag past the window's edge
  reports pixels outside the window. The release of the last button ends the grab, and the pointer then leaves and
  enters as usual.
- **Press over nothing.** It goes nowhere, and neither does its release.
- **No controller: gaze.** When the runtime has no tracked aim pose for the right hand, the ray leaves the head and
  points straight ahead. The select and secondary inputs still work if anything is bound to them. The headless session
  has no input source at all: no ray, no buttons, no scroll.
- **A refused input** is logged with the owner's reason and skipped. The frame, and the next frames' input, carry on.

Not wired: the thumbstick click (`BTN_MIDDLE`), the A, X and Y buttons (`BTN_SIDE`, `BTN_EXTRA`), the left hand and
switching hands. On a Touch-style controller B is the secondary input, so it sends `BTN_RIGHT`, not `BTN_EXTRA`. The ray
comes from the right hand only.

## Scripting XR in TypeScript

The classes of the spec's `gnoblin/extensions/xr` are in the TypeScript shell. They are objects over the tree's nodes,
like a `Client`: the XR program is a separate process, and nothing here loads into the shell
(`design/js-xr.md` says which spec class is which node).

### Is XR there?

`gnoblin.extensions.xr` is `null` until the program has a session, which is while the seat named `xr` exists. A script
guards the use:

```ts
const xr = gnoblin.extensions.xr;
if (xr?.session) {
    // The session is alive, and the stock room is built.
}
```

A session object that a script kept reads `alive === false` once the session ends, and every write to it, to its room
or to a panel throws. A script that wants to follow the headset comes and goes listens for it:

```ts
gnoblin.extensions.addListener({
    onSessionStarted: (session) => {},
    onSessionEnded: (session) => {},
});
```

Importing the module fails at load, with a message that says what to do, when there is no XR extension:

```ts
import { XRCamera3D, ViewMoveBinding } from "gnoblin/extensions/xr";
```

The import works when the session declared XR (`GNOBLIN_XR=1`) or when the program already has a session. A config that
must also work without a headset guards `gnoblin.extensions.xr` instead of importing.

### The room, the head and the viewport

The stock shell builds the room when the session starts. To change what it builds, configure it before the session
starts, which a config can because it runs first:

```ts
gnoblin.extensions.configure("xr", {
    room: true,
    layout: { radius: 1.8, triggers: { move: "Right.Grip+Drag", resize: null } },
});
```

`room: false` builds nothing and leaves the room to the script. A script that builds its own room hands it over with a
viewport; the program reads the node named `room` and the camera named `head`, and assigning the viewport names them:

```ts
const xr = gnoblin.extensions.xr;
if (xr?.session) {
    const head = new xr.Camera3D({ tracking: xr.session.head, near: 0.05, far: 100 });
    xr.session.viewport = new xr.Viewport({ content: new xr.Room(), camera: head, composition: "auto" });
}
```

`near` and `far` are in metres. The stock room is replaced by the new one. `composition: "projection"` draws each panel
through the head camera as a projection layer; `"layers"` and `"auto"` leave them as flat quads.

A client can also be shown in a panel that is fixed to your head, in metres in front of the eyes. The window keeps its
panel on the ring as well:

```ts
const panel = xr.room.pinToHead(client, { x: 0.1, y: -0.2, z: -1 });
```

### Moving and resizing a panel

The program does not send controller input to the shell as events. It will write one `controller` node for each
tracked hand (see "What is not built"), and the shell reads the nodes once a frame.

The bindings follow the ordinary lifecycle, and live in the room layout's `bindings`, a `BindingCollection`:

- `onStart` returns a state object, or `false` to decline.
- `onUpdate` runs once a frame with that frame's pose.
- `onEnd` or `onCancel` ends the drag.

| Binding             | Trigger             | What it writes                                                                                        |
| ------------------- | ------------------- | ----------------------------------------------------------------------------------------------------- |
| `ViewMoveBinding`   | `Grip+Drag`         | The View's position and rotation, never the client. `tracking: "ray"` follows where the ray points.   |
| `ViewResizeBinding` | `Grip+Trigger+Drag` | The View's scale on every update, and nothing to the client until release; then at most one new size. |

A trigger is the buttons held together, then `Drag`, with an optional `Left.` or `Right.` hand:

- The buttons are `Trigger`, `Grip`, `Thumbstick`, `Primary` and `Secondary`.
- A drag starts when exactly those buttons have been down together for 50 ms (`session.chordSettleMs`). A `Grip`
  pressed an instant before a `Trigger` is not taken for the start of `Grip+Trigger`.

A panel that was moved or resized is pinned: the ring leaves it where it is until `room.unpin(view)`.

- **Resize preview.** The panel grows evenly by the corner under the ray, and the opposite corner stays still.
- **On release.** The client is asked for a new size once, and only if the size drawn changes the aspect by more than
  2% or leaves the client's picture under 85% of the pixels the new size shows (`pixelsPerMetre`, 1200).
- **Tiled workspaces.** A layout that owns the window's rectangle may place the window again.

Turning a binding off and replacing it are the collection's calls:

```ts
const bindings = xr.room.layout.bindings;
bindings.remove("move");
bindings.add(new xr.ViewMoveBinding({ id: "move", trigger: "Right.Grip+Drag", tracking: "ray", rotation: "keep" }));
bindings.get("resize").enabled = false;
```

A binding of your own extends `XRControllerDragBinding` and is added the same way. `event.view` is the panel under the
ray when the drag began, `event.controller.pose` and `event.ray` are the hand, and `event.hit` is where the ray struck.

## Testing without a headset

`scripts/xr-smoke.sh` runs the whole stack on an Agent37 instance with no GPU, no seat and no headset. It runs as root
and fakes nothing. The parts are:

- the real compositor on its headless backend, and the stock Lua shell;
- two solid-colour Wayland clients (`xr-smoke-client`);
- `gnoblin-xr`, started by the guardian with `GNOBLIN_XR=1`;
- Monado with the simulated driver, the null compositor and lavapipe;
- a private D-Bus session bus (`dbus-daemon`), because the guardian starts the portal backend when its binary is beside
  `gnoblin`, and the guardian reports the session ready, and so starts the extension, only after the portal has
  connected to a bus.

It prints `PASS` or `FAIL` for each assertion and exits non-zero if any failed.

```sh
cargo build -p gnoblin
cargo build --manifest-path extensions/gnoblin-xr/Cargo.toml --bins
scripts/xr-smoke.sh            # installs the packages and builds Monado once when they are missing
```

The assertions:

- The `xr` seat exists, and the shell built a `room` with a `head` camera and one View for each window.
- The extension planned two layers and ended frames with two submitted layers, which Monado's null compositor accepted.
- The first picture uploaded for each View has the client's colour.
- The gaze ray gave the front client (the last window mapped, which holds the focus) a `wl_pointer.enter` on the `xr`
  seat, and every enter is at a point inside the window.
- Closing a client removes its layer, and the seat and the room go when the extension stops.

Logs, the tree dump and the pictures are left in `/tmp/xr-smoke`.

- `GNOBLIN_XR_DUMP_DIR` (test only): the extension writes the first picture uploaded for each View as
  `view-<node>-first.ppm`, and the most recent as `view-<node>-latest.ppm`. Nothing reads them back.
- `GNOBLIN_LOG=xr=debug`: the extension logs one line for every frame with the layers planned and submitted.
- `XR_SMOKE_HANDS=1`: gives the simulated headset a controller on each hand. Without it the pointing ray is the gaze.

## What is not built

- **The program does not publish controller nodes yet.** The bindings above read `controller` nodes
  (`design/js-xr.md`, "New native work"): one for each hand, with the aim pose and the trigger and grip values. No schema
  file declares the class, and the program and the compositor do not write it. Until they do, a headset session has no
  controllers in `session.controllers` and no drag starts. The shell side is proven against nodes made by a test.
- **The program still sends the grip as `BTN_RIGHT`.** A binding that owns the grip needs the program to stop sending
  it to the window under the ray. That arbitration is not designed.
- **A panel has one scale.** The program reads the first component of a View's `scale`, so a resize previews the growth
  evenly and the aspect changes when the client's new size lands.
- **A stalled script.** The program writes the head pose and submits layers without waiting for the shell, and the
  shell reads the tree once a frame. No test stalls a shell while the program runs: that is unverified.

## Limits

- You need an OpenXR runtime such as Monado or WiVRn. WiVRn has not been tried.
- The extension needs a Vulkan device and `XR_KHR_vulkan_enable2`. Without them it runs a headless session, which plans
  and drives the pointer and shows nothing.
- No headset has shown it. Everything here has run only against Monado's null compositor, which shows nothing, a stub
  runtime, a fake owner, a fake tree and the real compositor on its headless backend. The TypeScript XR classes ran only
  against a fake tree.
- Head tracking, the layer budget and the eyes a layer declares are proven only that way. Nothing says what a person
  sees, whether a layer swims, or how a real tracker's noise behaves.
- WiVRn, real controllers and a runtime that loses a session are untried.
- The pictures are `wl_shm` bytes copied through a staging buffer. There is no DMA-BUF path in the extension: the
  compositor imports a client's DMA-BUF and draws it, and a View of it is read back through the GPU, but the
  extension's capture asks for shared memory only.
- A stereo View follows the head only through the `head` camera, and only as well as the pairing rule above holds.
  A View whose picture changes in the same frame as the head pose can be paired with eyes one write (at most 1/60 s of
  head motion) off until the next picture. The tests do not measure how that looks.
- Windows from every workspace are in the room, not only the active one.
- The stock shell does not read `head.layer_budget`. The extension enforces the budget itself, whatever the shell does:
  the View that shows the focused window is the last to be dropped.
- A panel is flat. There is no depth, and a layout that makes windows intersect is not supported.

See [`extensions/gnoblin-xr`](https://github.com/kierandrewett/gnoblin/tree/main/extensions/gnoblin-xr) for the
extension and `design/xr-extension.md` for the design.
