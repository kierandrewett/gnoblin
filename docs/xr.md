# XR mode

XR mode shows your Gnoblin windows in a headset. Each window is a flat panel hanging in a room round you. You look at
a panel and point at it with a controller to move the pointer over that window.

Gnoblin draws every panel itself, with the frame, the shadow and the effects you see on a monitor. A separate program,
`gnoblin-xr`, copies each panel into an OpenXR session and tells the OpenXR runtime where to put it. The runtime
(Monado or WiVRn) places the panels and shows them in the headset. The program never decides where a window goes.

## How XR mode starts

The `gnoblin-xr` extension starts the OpenXR session. When the session is up it creates a seat named `xr` on the owner
socket. That seat is the headset's pointer. The stock shell builds the room when this seat appears and removes it when
the seat goes. Nothing in your configuration needs to name the room.

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

Buttons and scroll are not wired yet: the extension sends pointer motion and leave only.

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
