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

- The extension is its own workspace, and nothing in the packaging builds it.
- `cargo xtask dist install` installs it only when a developer built it first (`just dist`).
- A package install therefore has none, and `GNOBLIN_XR=1` logs the one warning and starts without it.

If `GNOBLIN_XR=1` is set and no binary is found, the guardian logs one warning and the session starts without it.

The setting is an environment variable because the guardian starts before the shell runs its configuration and reads no
configuration file. A configuration key would take effect only after a restart of the guardian's child list, which
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

The stock shell's XR side is the `xr` module of `libgnoblin-std`, so a shell in any language gets it. It reacts to the seat
named `xr`. Until that seat appears the library only watches the seats, so a session with no XR program pays nothing for it.

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

A dialog (a window whose `parent_window` names another) gets a View of its own. That View is a child of its parent's View,
placed where the dialog sits on the parent and 2 pixels in front, so it moves, turns and scales with the parent. A dialog
whose parent has no View is placed on the ring like any window.

A client that is `secure` never gets a View, and one that turns secure loses it. The compositor enforces that on its
own: it masks a secure node in every View's picture.

A shell can keep other windows out of the room with an `accept` function. It runs every frame, so a window comes in when
it starts to say yes:

::: tabs

```typescript
import { Extensions } from "gnoblin-std/generated/objects";

Extensions.get().configure("xr", { room: true }, (client) => !client.appId.startsWith("org.keepassxc"));
```

```python
from gnoblin.objects.std import Extensions, XROptions

Extensions.get().configure("xr", XROptions(room=True), lambda client: not client.app_id.startswith("org.keepassxc"))
```

```rust
use gnoblin_std::objects::{Extensions, XrClient, XrOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let options = XrOptions { room: true, layout: None };
    Extensions::get(&objects)?.configure(
        "xr",
        &options,
        Some(|client: XrClient| Ok(!client.app_id.starts_with("org.keepassxc"))),
    )?;
    Ok(())
}
```

:::

A window's `accept` function is the one way to choose which windows appear: there is no declarative window-rule key for it.

### The ring

The panels sit on a ring round you. The ring is `SpatialLayout.arrangeRing` (`arrange_ring` in Python and Rust), a pure
function of the windows' sizes and the index of the focused one, so a window's place never depends on anything but those:

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
- **Hold.** A press of the trigger, the grip, the face button or the stick reaches the window 150 ms after it began,
  unless a shell binding claims it first ("Controller nodes"). A scroll starts 150 ms after the stick leaves the deadzone.
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

## Scripting XR

XR scripting is part of `gnoblin-std`, so a shell in TypeScript, Python or Rust uses the same classes, generated from the
library's catalog. They are objects over the tree's nodes, like a `Client`: the XR program is a separate process, and nothing
here loads into the shell. The [API reference](/reference) lists every member in each language.

### Is XR there?

`gnoblin.extensions.xr` is empty until the program has a session, which is while the seat named `xr` exists. A shell checks
for it, and a listener hears the session begin and end:

::: tabs

```typescript
import { Extensions, XRListener, type XRSession } from "gnoblin-std/generated/objects";

class Announce extends XRListener {
    onSessionStarted(session: XRSession): void {
        gnoblin.spawn(["notify-send", `Headset session ${session.id} started`]);
    }
    onSessionEnded(session: XRSession): void {
        gnoblin.spawn(["notify-send", `Headset session ${session.id} ended`]);
    }
}

const extensions = Extensions.get();
extensions.addListener(new Announce());
if (extensions.xr?.session) {
    // The session is alive, and the stock room is built.
}
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import Extensions, XRListener, XRSession

gnoblin = Gnoblin.get()


class Announce(XRListener):
    def on_session_started(self, session: XRSession) -> None:
        gnoblin.spawn(["notify-send", f"Headset session {session.id} started"])

    def on_session_ended(self, session: XRSession) -> None:
        gnoblin.spawn(["notify-send", f"Headset session {session.id} ended"])


extensions = Extensions.get()
extensions.add_listener(Announce())
xr = extensions.xr
if xr is not None and xr.session is not None:
    pass  # The session is alive, and the stock room is built.
```

```rust
use std::rc::Rc;

use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{Extensions, XrListenerHandlers};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Rc::new(ModelRoot::get(&g.objects()?)?);
    let objects = g.std()?.objects()?;
    let extensions = Extensions::get(&objects)?;
    let (started, ended) = (Rc::clone(&model), Rc::clone(&model));
    let listener = XrListenerHandlers {
        on_session_started: Some(Box::new(move |session| {
            let text = format!("Headset session {} started", session.id()?);
            started.spawn(&["notify-send".to_owned(), text], None)?;
            Ok(())
        })),
        on_session_ended: Some(Box::new(move |session| {
            let text = format!("Headset session {} ended", session.id()?);
            ended.spawn(&["notify-send".to_owned(), text], None)?;
            Ok(())
        })),
    }
    .implement(&objects)?;
    extensions.add_listener(&listener)?;
    if let Some(xr) = extensions.xr()?
        && xr.session()?.is_some()
    {
        // The session is alive, and the stock room is built.
    }
    Ok(())
}
```

:::

A session object that a shell kept reads `alive` as false once the session ends, and a write to it fails. The stock room
ends with the session, so a handle on it fails as an object that is gone.

Two more calls tell a shell what is there:

- `require("xr")` fails with a message that says what to do when there is no XR extension. It succeeds when the session
  declared XR (`GNOBLIN_XR=1`) or when the program already has a session. A configuration that must also work without a
  headset checks `xr` instead.
- `list` names each extension with its status: `xr` is `ready` while there is a session and `unloaded` otherwise.

### The room, the head and the viewport

The stock shell builds the room when the session starts. To change what it builds, configure it before the session starts,
which a configuration can because it runs first. This one puts the ring 1.8 m away, moves with the right hand only, and leaves
`resize` out:

::: tabs

```typescript
import { Extensions, SpatialLayout } from "gnoblin-std/generated/objects";

const layout = new SpatialLayout({ radius: 1.8, triggers: { move: "Right.Grip+Drag", resize: "" } });
Extensions.get().configure("xr", { room: true, layout });
```

```python
from gnoblin.objects.std import Extensions, SpatialLayout, SpatialTriggers, XROptions

layout = SpatialLayout(
    radius=1.8,
    triggers=SpatialTriggers(move="Right.Grip+Drag", resize="", rotate="Trigger+Thumbstick+Drag"),
)
Extensions.get().configure("xr", XROptions(room=True, layout=layout))
```

```rust
use gnoblin_std::objects::{Extensions, SpatialLayout, SpatialLayoutOptions, SpatialTriggers, XrClient, XrOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let layout = SpatialLayout::new_with(
        &objects,
        SpatialLayoutOptions {
            radius: Some(1.8),
            triggers: Some(SpatialTriggers {
                r#move: "Right.Grip+Drag".to_owned(),
                resize: String::new(),
                ..SpatialTriggers::default()
            }),
            ..SpatialLayoutOptions::default()
        },
    )?;
    let options = XrOptions { room: true, layout: Some(layout) };
    // Every window gets a panel.
    Extensions::get(&objects)?.configure("xr", &options, Some(|_client: XrClient| Ok(true)))?;
    Ok(())
}
```

:::

`room: false` builds nothing and leaves the room to the shell. A trigger left empty leaves that binding out. A shell that
builds its own room hands it over with a viewport. The program reads the node named `room` and the camera named `head`, and
assigning the viewport names them:

::: tabs

```typescript
import { Extensions, XRCamera3D, XRRoom, XRViewport } from "gnoblin-std/generated/objects";

const session = Extensions.get().xr?.session;
if (session) {
    const head = new XRCamera3D({ tracking: session.head, near: 0.05, far: 100 });
    session.viewport = new XRViewport(new XRRoom(), head, { composition: "auto" });
}
```

```python
from gnoblin.objects.std import Extensions, XRCamera3D, XRRoom, XRViewport, XrComposition

xr = Extensions.get().xr
session = xr.session if xr is not None else None
if session is not None:
    head = XRCamera3D(tracking=session.head, near=0.05, far=100.0)
    session.viewport = XRViewport(XRRoom(), head, composition=XrComposition.AUTO)
```

```rust
use gnoblin_std::objects::{
    Extensions, XrCamera3D, XrCamera3DOptions, XrComposition, XrRoom, XrViewport, XrViewportOptions,
};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let Some(xr) = Extensions::get(&objects)?.xr()? else {
        return Ok(());
    };
    let Some(session) = xr.session()? else {
        return Ok(());
    };
    let head = XrCamera3D::new_with(
        &objects,
        XrCamera3DOptions {
            tracking: session.head()?,
            near: Some(0.05),
            far: Some(100.0),
            ..XrCamera3DOptions::default()
        },
    )?;
    let room = XrRoom::new(&objects)?;
    let viewport = XrViewport::new_with(
        &objects,
        &room.as_scene_node()?,
        &head.as_camera3d()?,
        XrViewportOptions { composition: Some(XrComposition::Auto) },
    )?;
    session.set_viewport(&viewport)?;
    Ok(())
}
```

:::

`near` and `far` are in metres. The stock room is replaced by the new one. `composition: "projection"` draws each panel
through the head camera as a projection layer; `"layers"` and `"auto"` leave them as flat quads.

A client can also be shown in a panel that is fixed to your head, in metres in front of the eyes. The window keeps its panel
on the ring as well, and the panel goes with the window:

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core/generated/objects";
import { Extensions } from "gnoblin-std/generated/objects";

const room = Extensions.get().xr?.room;
const client = Gnoblin.get().clients?.focused;
if (room && client) {
    room.pinToHead(client, { x: 0.1, y: -0.2, z: -1 });
}
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import Extensions, HeadPanelOptions

xr = Extensions.get().xr
room = xr.room if xr is not None else None
client = Gnoblin.get().clients.focused
if room is not None and client is not None:
    room.pin_to_head(client, HeadPanelOptions(x=0.1, y=-0.2, z=-1.0))
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{Extensions, HeadPanelOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let Some(room) = Extensions::get(&objects)?.xr()?.map(|xr| xr.room()).transpose()?.flatten() else {
        return Ok(());
    };
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(client) = model.clients()?.focused()? else {
        return Ok(());
    };
    room.pin_to_head(&client, Some(&HeadPanelOptions { x: 0.1, y: -0.2, z: -1.0, scale: None }))?;
    Ok(())
}
```

:::

A head-locked panel is also the way to keep a screen in view: a virtual output attached to the viewer is not supported, so
`room.pinToHead(client)` (`pin_to_head` in Python and Rust) is how a layer-shell client stays in front of your eyes.

### Moving, resizing, turning and scaling a panel

The program writes one `controller` node for each hand (see "Controller nodes"), and the shell reads the nodes once a frame.
The room's `SpatialLayout` holds the bindings that act on the panel under the ray, and `layout.bindings` is their
collection:

| Id       | Class               | Default trigger           | What a drag does                                                               |
| -------- | ------------------- | ------------------------- | ------------------------------------------------------------------------------ |
| `move`   | `ViewMoveBinding`   | `Grip+Drag`               | The panel follows the hand (`tracking: "controller"`) or the ray (`"ray"`).    |
| `resize` | `ViewResizeBinding` | `Grip+Trigger+Drag`       | Previews the new size on the panel; asks the client for one size on release.   |
| `rotate` | `ViewRotateBinding` | `Trigger+Thumbstick+Drag` | Turns the panel about its own centre by what the hand turns.                   |
| `scale`  | `ViewScaleBinding`  | `Grip+Thumbstick+Drag`    | Adds the metres the hand pushes along its aim to the scale. Not in the layout. |

- `SpatialLayout`'s `triggers` takes `move`, `resize` and `rotate`. An empty trigger leaves the binding out.
- A `ViewScaleBinding` is added by the shell, with `minScale` and `maxScale` (0.2 and 3) as limits.
- `ViewResizeBinding` takes `edges`: `"quadrant"` (the corner nearest the ray) or one corner, such as `"top-left"`.
- Setting the room's `layout` to none takes it away: no panel is placed and no drag runs until a layout is set again.

This layout leaves `rotate` out, adds a scale drag, replaces the stock `move` with a ray drag and turns `resize` off:

::: tabs

```typescript
import { Extensions, SpatialLayout, ViewMoveBinding, ViewScaleBinding } from "gnoblin-std/generated/objects";

const room = Extensions.get().xr?.room;
if (room) {
    const spatial = new SpatialLayout({ triggers: { rotate: "" } });
    const bindings = spatial.bindings;
    if (bindings) {
        bindings.add(new ViewScaleBinding({ minScale: 0.5, maxScale: 2 }));
        // The stock `move` holds the id, so remove it before adding a ray drag in its place.
        bindings.remove("move");
        bindings.add(new ViewMoveBinding({ id: "move", tracking: "ray", rotation: "keep" }));
        const resize = bindings.find("resize");
        if (resize) {
            resize.enabled = false;
        }
    }
    room.layout = spatial;
}
```

```python
from gnoblin.objects.std import (
    Extensions,
    SpatialLayout,
    SpatialTriggers,
    ViewMoveBinding,
    ViewRotation,
    ViewScaleBinding,
    ViewTracking,
)

xr = Extensions.get().xr
room = xr.room if xr is not None else None
if room is not None:
    spatial = SpatialLayout(triggers=SpatialTriggers(move="Grip+Drag", resize="Grip+Trigger+Drag", rotate=""))
    bindings = spatial.bindings
    if bindings is not None:
        bindings.add(ViewScaleBinding(min_scale=0.5, max_scale=2.0))
        # The stock `move` holds the id, so remove it before adding a ray drag in its place.
        bindings.remove("move")
        bindings.add(ViewMoveBinding(id="move", tracking=ViewTracking.RAY, rotation=ViewRotation.KEEP))
        resize = bindings.find("resize")
        if resize is not None:
            resize.enabled = False
    room.layout = spatial
```

```rust
use gnoblin_std::objects::{
    Extensions, SpatialLayout, SpatialLayoutOptions, SpatialTriggers, ViewMoveBinding, ViewMoveBindingOptions,
    ViewRotation, ViewScaleBinding, ViewScaleBindingOptions, ViewTracking,
};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let Some(room) = Extensions::get(&objects)?.xr()?.map(|xr| xr.room()).transpose()?.flatten() else {
        return Ok(());
    };
    let spatial = SpatialLayout::new_with(
        &objects,
        SpatialLayoutOptions {
            triggers: Some(SpatialTriggers { rotate: String::new(), ..SpatialTriggers::default() }),
            ..SpatialLayoutOptions::default()
        },
    )?;
    if let Some(bindings) = spatial.bindings()? {
        let scale = ViewScaleBinding::new_with(
            &objects,
            ViewScaleBindingOptions { min_scale: Some(0.5), max_scale: Some(2.0), ..ViewScaleBindingOptions::default() },
        )?;
        bindings.add(&scale.as_xr_drag_binding()?)?;
        // The stock `move` holds the id, so remove it before adding a ray drag in its place.
        bindings.remove("move")?;
        let ray = ViewMoveBinding::new_with(
            &objects,
            ViewMoveBindingOptions {
                id: Some("move".to_owned()),
                tracking: Some(ViewTracking::Ray),
                rotation: Some(ViewRotation::Keep),
                ..ViewMoveBindingOptions::default()
            },
        )?;
        bindings.add(&ray.as_xr_drag_binding()?)?;
        if let Some(resize) = bindings.find("resize")? {
            resize.set_enabled(false)?;
        }
    }
    room.set_layout(&spatial)?;
    Ok(())
}
```

:::

`bindings.add` refuses a binding whose id is taken, as every layout's collection does, so a shell removes the stock binding
first. A binding that matches the same trigger at the same priority as another is refused too. A binding's `trigger` can be
set again, and `parsed` reads it back in its one canonical spelling.

A cancelled drag, which an untracked hand or a hook that fails causes, puts the panel back where it was.

### Your own drag binding

`XRControllerDragBinding` is the base of the four classes above. A shell's own binding is one of them with a `handler`: an
object of the `XRDragHandler` interface whose hooks run once a frame while the trigger's buttons are down together. This one
nudges the panel sideways as far as the hand travels:

::: tabs

```typescript
import { Extensions, XRControllerDragBinding, XRDragHandler, type XRDragEvent } from "gnoblin-std/generated/objects";

class Nudge extends XRDragHandler {
    private start = 0;
    private hand = 0;

    onStart(event: XRDragEvent): boolean {
        if (!event.view || !event.controller.pose) {
            return false;
        }
        this.start = event.view.x;
        this.hand = event.controller.pose.position.x;
        return true;
    }

    onUpdate(event: XRDragEvent): void {
        const pose = event.controller.pose;
        if (event.view && pose) {
            event.view.x = this.start + pose.position.x - this.hand;
        }
    }

    onCancel(event: XRDragEvent): void {
        if (event.view) {
            event.view.x = this.start;
        }
    }
}

const bindings = Extensions.get().xr?.room?.layout?.bindings;
bindings?.add(new XRControllerDragBinding({ id: "nudge", trigger: "Primary+Drag", handler: new Nudge() }));
```

```python
from gnoblin.objects.std import Extensions, XRControllerDragBinding, XRDragEvent, XRDragHandler


class Nudge(XRDragHandler):
    start = 0.0
    hand = 0.0

    def on_start(self, event: XRDragEvent) -> bool:
        pose = event.controller.pose
        if event.view is None or pose is None:
            return False
        self.start = event.view.x
        self.hand = pose.position.x
        return True

    def on_update(self, event: XRDragEvent) -> None:
        pose = event.controller.pose
        if event.view is not None and pose is not None:
            event.view.x = self.start + pose.position.x - self.hand

    def on_cancel(self, event: XRDragEvent) -> None:
        if event.view is not None:
            event.view.x = self.start


xr = Extensions.get().xr
room = xr.room if xr is not None else None
layout = room.layout if room is not None else None
bindings = layout.bindings if layout is not None else None
if bindings is not None:
    bindings.add(XRControllerDragBinding(id="nudge", trigger="Primary+Drag", handler=Nudge()))
```

```rust
use std::cell::Cell;
use std::rc::Rc;

use gnoblin_std::objects::{Extensions, XrDragBinding, XrDragBindingOptions, XrDragHandlerHandlers};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let Some(room) = Extensions::get(&objects)?.xr()?.map(|xr| xr.room()).transpose()?.flatten() else {
        return Ok(());
    };
    let Some(bindings) = room.layout()?.map(|layout| layout.bindings()).transpose()?.flatten() else {
        return Ok(());
    };
    // Where the panel and the hand were when the drag began.
    let start = Rc::new(Cell::new((0.0, 0.0)));
    let (begin, follow, back) = (Rc::clone(&start), Rc::clone(&start), Rc::clone(&start));
    let nudge = XrDragHandlerHandlers {
        on_start: Some(Box::new(move |event| {
            let (Some(view), Some(pose)) = (event.view, event.controller.pose()?) else {
                return Ok(false);
            };
            begin.set((view.x()?, pose.position.x));
            Ok(true)
        })),
        on_update: Some(Box::new(move |event| {
            if let (Some(view), Some(pose)) = (event.view, event.controller.pose()?) {
                let (panel, hand) = follow.get();
                view.set_x(panel + pose.position.x - hand)?;
            }
            Ok(())
        })),
        on_cancel: Some(Box::new(move |event| {
            if let Some(view) = event.view {
                view.set_x(back.get().0)?;
            }
            Ok(())
        })),
        ..Default::default()
    }
    .implement(&objects)?;
    let binding = XrDragBinding::new_with(
        &objects,
        XrDragBindingOptions {
            id: Some("nudge".to_owned()),
            trigger: Some("Primary+Drag".to_owned()),
            handler: Some(nudge),
            ..XrDragBindingOptions::default()
        },
    )?;
    bindings.add(&binding)?;
    Ok(())
}
```

:::

`onStart` (`on_start`) returns `false` to decline the drag; any other answer takes it. The event has:

- `controller`, its `ray`, and the `view` under the ray;
- `hit`, where the ray struck the view;
- `deltaTime` in seconds, and `unit`, the metres for each room unit.

A handler that needs more than its trigger's buttons defines `claims()`. A throw-and-glide binding is not built in: write it
as a handler like this one, with a physics stepper of your own.

### Controller nodes

The program is an observer. It makes one `controller` node for each hand the runtime reports as active, with every value
in one transaction, and destroys it when the runtime stops reporting the hand and when the session ends. It writes a
node at most once a frame, and at most about 90 times a second. The shell reads these properties:

| Property                                 | Meaning                                                                                           |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `x` `y` `z` `rotation`                   | The aim pose, in the room's node units (`metres` of the head camera), y up like Views.            |
| `hand`                                   | `left` or `right`.                                                                                |
| `tracked`                                | False while the aim pose cannot be located; the node keeps its last pose.                         |
| `trigger` `grip`                         | 0 to 1.                                                                                           |
| `thumbstick_x` `thumbstick_y`            | -1 to 1, y up.                                                                                    |
| `thumbstick_click` `primary` `secondary` | Buttons: A or X, B or Y (menu on controllers with no B).                                          |
| `claimed`                                | Written by the shell: the inputs `trigger`, `grip`, `thumbstick`, `primary`, `secondary` it owns. |

A claimed input is withheld from the window under the ray, and a press waits for a claim before the window sees it:

- The program holds every press of the trigger, the grip, the face button and the stick for 150 ms (`CLAIM_HOLD`). A
  claim that arrives in that time takes the press: the window gets no button event at all for it, not even a release.
- A press that nobody claims goes to the window when the 150 ms are over, at the ray the hand had when it went down. A
  click that was over inside the 150 ms arrives as a press and then a release, one frame apart, in that order. A flick of
  the stick that ends inside the 150 ms scrolls nothing.
- A shell that is slow, stopped or not running is the same as no claim: the press goes out at the deadline, and the head
  pose, the controller nodes and the layers are written on every frame meanwhile. The program never waits for the shell.
- So a click of these inputs lands 150 ms (up to one frame more) after the press, whether or not a binding uses the
  input. The shell's drag claims after `session.chordSettleMs` (at most 60, so the claim reaches the program inside the
  hold). A claim that comes later than the hold finds the press already in the window and releases it, as a claim did
  before the hold.
- A claimed grip is not `BTN_RIGHT`, a claimed trigger is not `BTN_LEFT` and a claimed thumbstick does not scroll.
- A button the window already holds when a late claim arrives is released.
- A press that began under a claim never reaches the window.
- The values are still written for the shell to read.

The pointer is the right hand's. The left hand's node is published and nothing reaches a window from it. The session tells
a shell when a controller comes and goes (`controller_added` and `controller_removed`).

A drag's bindings follow the ordinary lifecycle:

- `onStart` takes the drag, or declines it with `false`.
- `onUpdate` runs once a frame with that frame's pose.
- `onEnd` or `onCancel` ends the drag.

A trigger is the buttons held together, then `Drag`, with an optional `Left.` or `Right.` hand:

- The buttons are `Trigger`, `Grip`, `Thumbstick`, `Primary` and `Secondary`.
- A drag starts when exactly those buttons have been down together for 50 ms (`session.chordSettleMs`, at most 60). A
  `Grip` pressed an instant before a `Trigger` is not taken for the start of `Grip+Trigger`.

A panel that was moved or resized is pinned: the ring leaves it where it is until `room.unpin(view)`.

- **Resize preview.** The panel grows evenly by the corner under the ray, and the opposite corner stays still.
- **On release.** The client is asked for a new size once, and only if the size drawn changes the aspect by more than
  2% or leaves the client's picture under 85% of the pixels the new size shows (`pixelsPerMetre`, 1200).
- **Tiled workspaces.** A layout that owns the window's rectangle may place the window again.

### Claiming a controller's inputs

A drag takes its buttons from the window under the ray:

- The library claims the buttons of the binding's trigger on the controller that started the drag, as the drag starts.
- It gives them back when the drag ends, however it ends: the buttons are released, the hand is lost, the binding is
  removed or its hook fails, the session ends or the shell reloads.
- A `Grip+Drag` claims `grip`, and a `Grip+Trigger+Drag` claims `trigger` and `grip`.
- A handler that needs other inputs defines `claims()`.

A shell can claim inputs of its own on a controller. Each owner is a name you choose and has one claim, a later one of the
same owner replaces it, and the node holds the union of every owner's, so a claim made for a menu is not undone when a drag
ends:

::: tabs

```typescript
import { Extensions } from "gnoblin-std/generated/objects";

const hand = Extensions.get().xr?.session?.controller("60");
hand?.claim("menu", ["thumbstick"]);
hand?.release("menu");
```

```python
from gnoblin.objects.std import Extensions

xr = Extensions.get().xr
session = xr.session if xr is not None else None
hand = session.controller("60") if session is not None else None
if hand is not None:
    hand.claim("menu", ["thumbstick"])
    hand.release("menu")
```

```rust
use gnoblin_std::objects::Extensions;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let Some(session) = Extensions::get(&objects)?.xr()?.map(|xr| xr.session()).transpose()?.flatten() else {
        return Ok(());
    };
    if let Some(hand) = session.controller("60")? {
        hand.claim("menu", &["thumbstick".to_owned()])?;
        hand.release("menu")?;
    }
    Ok(())
}
```

:::

The names are the five of the table above, in lower case. A list that is anything else, or an empty owner, fails with a
type error that names `Controller#<id>.claimed`, and a controller that is gone refuses the claim. A release of a gone
controller does nothing. `hand.claimed` reads the union.

When the shell's generation ends (a reload or an exit) the library gives back every claim it made, and the compositor gives
a shell's claims back when the shell goes (a crash, a kill), because the program would otherwise withhold the input for
ever.

## Testing without a headset

`scripts/xr-smoke.sh` runs the whole stack on an Agent37 instance with no GPU, no seat and no headset. It runs as root
and fakes nothing. The parts are:

- the real compositor on its headless backend, and the TypeScript shell with its built-in stock library;
- two solid-colour Wayland clients (`xr-smoke-client`);
- `gnoblin-xr`, started by the guardian with `GNOBLIN_XR=1`;
- Monado with the simulated driver, the null compositor and lavapipe;
- a private D-Bus session bus (`dbus-daemon`), because the guardian starts the portal backend when its binary is beside
  `gnoblin`, and the guardian reports the session ready, and so starts the extension, only after the portal has
  connected to a bus.

It prints `PASS` or `FAIL` for each assertion and exits non-zero if any failed.

```sh
cargo xtask stage && cargo build -p gnoblin    # `gnoblin` records libgnoblin-core.so.1; stage names the library beside it
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

- **Controllers are proven only against a fake runtime.** The two-hand OpenXR actions compile and the tests pass with
  scripted hands; no headset, and no Monado run of the new actions, has seen them. No headset was used.
    - The program's withholding of a claimed input, and the 150 ms hold that gives the shell time to claim a press before
      the window sees it, are checked against the fake runtime only. The tests step a scripted clock a frame at a time.
    - One real-compositor run of the stock shell (headless, `js_shell.rs`) made the grip's claim visible 106 to 125 ms
      after the squeeze, 25 ms of test polling included. A slow machine can still claim after the hold, and the window
      then sees the press first.
- **A panel has one scale.** The program reads the first component of a View's `scale`, so a resize previews the growth
  evenly and the aspect changes when the client's new size lands.
- **ThrowViewBinding.** There is no built-in throw-and-glide binding; write one as a drag handler of your own.
- **A stalled script.** The program writes the head pose and submits layers without waiting for the shell, and the
  shell reads the tree once a frame. No test stalls a shell while the program runs: that is unverified.

## Limits

- You need an OpenXR runtime such as Monado or WiVRn. WiVRn has not been tried.
- The extension needs a Vulkan device and `XR_KHR_vulkan_enable2`. Without them it runs a headless session, which plans
  and drives the pointer and shows nothing.
- No headset has shown it. Everything here has run only against Monado's null compositor, which shows nothing, a stub
  runtime, a fake owner, a fake tree and the real compositor on its headless backend. The XR classes of `gnoblin-std`
  ran against a fake tree, and the stock room and grab against the real compositor on its headless backend.
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
extension.
