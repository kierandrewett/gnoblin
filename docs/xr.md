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

If `GNOBLIN_XR=1` is set and no binary is found, the guardian logs one warning and the session starts without it.

The setting is an environment variable because the guardian starts before the Lua configuration runs and reads no
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

The stock shell's XR side is `data/lua/gnoblin/shell/xr.lua`. It reacts to the seat named `xr`.

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

A window rule can keep a window out of the room. The stock shell reads the rule key `xr`:

```lua
seat.data.window_rules = {
  { match = { app_id = "^org%.keepassxc" }, xr = false },
}
```

A window that matches a rule with `xr = false` has no View. When the rule stops matching, the View appears. Rules are
the stock shell's rules (`data/lua/gnoblin/shell/rules.lua`): the same `match` keys and the same re-evaluation when a
title or focus changes.

### The ring

The panels sit on a ring round you. The ring is `XrRing.arrange` in `xr.lua`, a pure function of the windows' sizes and
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
- The gaze ray gave a client a `wl_pointer.enter` on the `xr` seat.
- Closing a client removes its layer, and the seat and the room go when the extension stops.

Logs, the tree dump and the pictures are left in `/tmp/xr-smoke`.

- `GNOBLIN_XR_DUMP_DIR` (test only): the extension writes the first picture uploaded for each View as
  `view-<node>-first.ppm`, and the most recent as `view-<node>-latest.ppm`. Nothing reads them back.
- `GNOBLIN_LOG=xr=debug`: the extension logs one line for every frame with the layers planned and submitted.
- `XR_SMOKE_HANDS=1`: gives the simulated headset a controller on each hand. Without it the pointing ray is the gaze.

## Limits

- You need an OpenXR runtime such as Monado or WiVRn. WiVRn has not been tried.
- The extension needs a Vulkan device and `XR_KHR_vulkan_enable2`. Without them it runs a headless session, which plans
  and drives the pointer and shows nothing.
- It has been tested only against Monado's null compositor. No headset has shown it.
- The pictures are `wl_shm` bytes copied through a staging buffer. There is no DMA-BUF path yet.
- Windows from every workspace are in the room, not only the active one.
- The layer budget of the runtime is not read by the shell. A runtime that cannot show every panel drops the ones past
  its limit, head-locked panels first.
- A panel is flat. There is no depth, and a layout that makes windows intersect is not supported.

See [`extensions/gnoblin-xr`](https://github.com/kierandrewett/gnoblin/tree/main/extensions/gnoblin-xr) for the
extension and `design/xr-extension.md` for the design.
