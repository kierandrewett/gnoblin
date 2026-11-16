# Outputs

An output is a connected display, real or virtual. Each output shows one workspace of its own, bars sit on one output, and
a window that loses its output moves to another or waits hidden for the next. `gnoblin.outputs` is the live collection of
`Output` objects.

```ts
import { FloatingLayout, Workspace, toast } from "gnoblin-std";

for (const id of ["1", "2", "3"]) {
    gnoblin.workspaces.add(new Workspace({ id, layout: new FloatingLayout() }));
}
for (const output of gnoblin.outputs) {
    toast(`${output.name}: ${output.mode} showing ${output.workspace?.id ?? "nothing"}`);
}
```

Put the file in `~/.config/gnoblin/shell.ts`. A message names each output, its mode and the workspace it shows.

## Read outputs

`gnoblin.outputs` is a live, read-only collection with one `Output` for each screen the compositor publishes. An output is
added when it is plugged in and removed when it goes. Every read of one output returns the same object, so you can keep it.
A removed output keeps its last values and reads `connected === false`.

`gnoblin.outputs.where({ primary: true })` selects live, as the other collections do, on `id`, `name`, `make`, `model`,
`mode`, `scale`, `primary`, `enabled`, `x`, `y`, `width` and `height`. [`gnoblin.find("Output", filter)`](find) returns an
array.

| Member                      | Meaning                                                                                                                                    |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `name`                      | The compositor's name for the output, such as `output-0`. Not a stable name: use `connector`.                                              |
| `connector`                 | The connector the display is plugged into, such as `DP-1`. An `OutputProfile` matches it.                                                  |
| `identity`                  | The monitor's EDID identity, `MAN:product:serial`. Empty for a display that reports none.                                                  |
| `make`, `model`             | What the compositor knows of the display. Empty when it knows nothing.                                                                     |
| `mirrorOf`                  | The output this one shows a copy of, or `null`.                                                                                            |
| `mode`                      | The mode in use, `WxH@Hz`.                                                                                                                 |
| `scale`                     | The factor the output composes at. 2 is a HiDPI panel.                                                                                     |
| `supersample`               | The factor the output is composed above its mode, 1 for none. See [Supersampling](#supersampling).                                         |
| `virtualSize`               | `{ width, height }`: the mode times `supersample`, the size of the output's screenshots.                                                   |
| `effectiveScale`            | `scale * supersample`: the scale clients are told.                                                                                         |
| `gpuMemory`                 | `{ target, scratch, staging, views, capture, total }` in bytes: what the compositor holds for the output. See [GPU memory](../gpu-memory). |
| `transform`                 | The rotation and flip: `normal`, `90`, `180`, `270`, `flipped`, `flipped-90`, `flipped-180` or `flipped-270`.                              |
| `x`, `y`, `width`, `height` | The output's rectangle in the compositor's coordinates. `bounds` is the same as one object.                                                |
| `position`                  | `{ x, y }` of the output.                                                                                                                  |
| `primary`                   | Whether this is the primary output, the one the stock bars and `gnoblin.workspaces.area` are on.                                           |
| `enabled`                   | Whether the output is on.                                                                                                                  |
| `area`                      | What a layout tiles in on this output: its rectangle less the bars on it. `null` when the output is gone.                                  |
| `workspace`                 | The workspace the output shows, or `null`.                                                                                                 |
| `show(workspace, options?)` | Shows a workspace on this output. `options.duration` slides the windows.                                                                   |
| `apply(options)`            | Configures the display. See [Configure an output](#configure-an-output).                                                                   |
| `mirror(source)`            | Shows a copy of `source`, or stops with `null`.                                                                                            |
| `connected`, `alive`        | `false` once the display is gone.                                                                                                          |
| `addListener(listener)`     | Attaches an `OutputListener`.                                                                                                              |

`gpuMemory` also carries four numbers about the whole session, the same on every output: `clients` (the bytes of every
client's live buffers), `budget` (the memory budget in bytes, 0 when it is off), `refused` (the requests refused for
passing a bound, a count) and `evicted` (the bytes given back from caches to make room). They are in
[GPU memory](../gpu-memory#the-budget).

A script reads these properties and writes none. It changes a display with `apply` and `mirror`, below, or keeps a
setting for a monitor with an [`OutputProfile`](#hardware-profiles) in `gnoblin.outputs.profiles`.

## Configure an output

`output.apply({ ... })` asks the compositor to change one display. Every key is optional, and a key left out keeps its
value.

- The compositor checks the whole new arrangement before it touches hardware.
- It then drives the changed outputs. If the backend refuses a mode, the outputs already changed are put back and the
  last good arrangement stays.
- It keeps the result as that monitor's profile, so the same monitor on the same port gets it back after an unplug.

| Key           | Values                                                                                            |
| ------------- | ------------------------------------------------------------------------------------------------- |
| `mode`        | `"3440x1440@144"`, or `"preferred"` to keep what the backend drives.                              |
| `scale`       | A number from 0.5 to 4.                                                                           |
| `supersample` | A number from 1 to 16: compose at this multiple of the mode. See [Supersampling](#supersampling). |
| `transform`   | `"normal"`. The compositor draws no rotation, so the others are refused by name.                  |
| `position`    | `[x, y]` or `{ x, y }`: the top-left corner in the one desktop coordinate space.                  |
| `enabled`     | `false` switches the output off: it shows black and has no place in the desktop.                  |
| `mirrorOf`    | The connector, or an `Output`, to show an exact copy of. `null` stops mirroring.                  |

A value of the wrong shape throws before anything is sent, naming the key: `Output#output-1.apply.scale must be a number
from 0.5 to 4`.

The compositor answers a frame later. `apply` returns an `OutputApplication` with `done` and `error`, which is `null` on
success. An `OutputListener` hears the same answer in `onApplied(output, error)`. A refusal names the key, as in
`position: HDMI-1 and DP-1 would overlap`.

**Mode and scale.** Drive an output at another mode, composed at a fractional scale:

```ts
import { gnoblin } from "gnoblin-core";

const app = gnoblin.outputs.where({ name: "output-1" }).at(0)!.apply({ mode: "2560x1440@60", scale: 1.25 });
```

**Position.** Put an output below another. Outputs share one coordinate space, so a window can straddle the seam:

```ts
gnoblin.outputs.at(1)!.apply({ position: [0, 1080] });
```

**Mirror.** Show the first output on the second, scaled to fit and letterboxed when the shape differs. The pair is one
monitor: the mirrored output has no place of its own, no workspace, and the pointer has one monitor for both.

```ts
gnoblin.outputs.at(1)!.mirror(gnoblin.outputs.at(0)!);
gnoblin.outputs.at(1)!.mirror(null); // stop
```

**Off.** Switch an output off. It keeps its node, so you can switch it back on:

```ts
gnoblin.outputs.at(1)!.apply({ enabled: false });
```

**Span.** The desktop already is the span: every output is a rectangle in one space and a window across the seam is drawn
continuously, at any mix of scales. By default each output shows a workspace of its own. A [`span` display](#displays-and-viewports)
shows one workspace over several outputs.

**In your file.** An `OutputProfile` applies the same keys by connector when the file loads, and again when that display is
plugged in: see [Hardware profiles](#hardware-profiles). One the compositor refuses is reported as a diagnostic naming
`outputs.profiles.<id>.<key>`.

### Supersampling

`supersample` renders an output at a multiple of its mode and averages the result down, like macOS's scaled resolutions.

- The compositor composes the output on a virtual surface of `mode * supersample` pixels, tells clients the scale
  `scale * supersample`, and shows the result at the real mode, averaged down.
- The logical size (`mode / scale`), window geometry and pointer do not change, so the desktop looks the same,
  anti-aliased and crisp, and screenshots and captures come out hi-res.
- Wayland clients draw `supersample` times the pixels. Xwayland's client is given the same factor, so an X11 window keeps
  its logical size and its toolkit is told to draw `supersample` times the pixels through `Gdk/WindowScalingFactor` and
  `Xft/DPI`. A window that is already open keeps the pixels it has until its client redraws or is reopened.

- **Supported factors.** 1, 2 and 4. Any other number from 1 to 16 is accepted when `mode * supersample` is a whole size
  and logs one notice, `supersample N is not recommended: use 1, 2 or 4`; the downsample is then bilinear. In a script the
  notice is the log line `output.apply` writes, and the same factor asked for again is not reported twice.
- **Refused by name.** A value that is not a number from 1 to 16 throws before anything is sent, naming the key. A factor
  that gives a fractional size (1.5 on a 1367 pixel wide mode) or a surface past the renderer's limit (4 on 3840x2160 is
  15360 pixels wide, over the 8192 limit) is refused by the compositor with the numbers, and the output keeps what it had.
- **What `wl_output` and `xdg_output` say.** The mode stays the real one, `scale` is the next whole number above
  `scale * supersample`, the preferred fractional scale is exact, and the logical size does not change.

```ts
const output = gnoblin.outputs.where({ name: "output-1" }).at(0)!;
output.apply({ supersample: 1 }); // the default: composed at the mode, nothing averaged
output.apply({ supersample: 2 }); // 1920x1080 composed at 3840x2160; output.virtualSize is { width: 3840, height: 2160 }
output.apply({ supersample: 4 }); // 1920x1080 composed at 7680x4320: a GPU with 8192 texels per edge is needed
```

From a terminal every key is optional and a key left out keeps its value, so a call names only what it changes:

```sh
gnoblinctl output call <id> apply supersample=1   # no supersampling
gnoblinctl output call <id> apply supersample=2   # twice the mode
gnoblinctl output call <id> apply supersample=4   # four times the mode
gnoblinctl output get <id> supersample            # what it is now
```

**What a screenshot looks like.** A screenshot or capture of a supersampled output is the composed frame, so it has
`supersample` times the pixels of the display on each axis, with the secure-content masks applied before anything is read:

| Factor | On a 1920x1080 mode | What it holds                                                                          |
| ------ | ------------------- | -------------------------------------------------------------------------------------- |
| 1      | 1920x1080           | The frame the display shows.                                                           |
| 2      | 3840x2160           | Four rendered pixels for each displayed one: text and edges are crisp.                 |
| 4      | 7680x4320           | Sixteen rendered pixels for each displayed one; refused where the limit is under 7680. |

`wlr-screencopy`, `ext-image-copy-capture`, the PipeWire export and `capture.screen` all advertise that size. The payload
of `capture.screen` takes `display` for the frame as the display shows it, at the mode, which is the hi-res frame
averaged down; use it when a consumer wants the picture the monitor shows rather than the sharpest one.

- **Software and GLES on a virtual output.** The hi-res frame is kept and read back as it is.
- **Nested.** The frame is averaged into the parent's buffer, which stays the size of the parent's window; a parent that
  rounded a fractional scale (no viewport) has a buffer that is not the mode, and the factor is refused by name.
- **Hardware (DRM).** The output composes on a persistent virtual surface and a box-filter pass fills the scan-out buffer.
  The DRM backend keeps no readable frame of an output, so there is no screenshot or capture of a hardware output at any
  factor; that limit is older than this setting and unchanged by it.
- **Vulkan.** The offscreen Vulkan renderer draws at any compositing scale into an image of any size the device allows, so
  it takes a factor the same way; the device's `maxImageDimension2D` is its limit and a factor past it is refused with
  the numbers.

### What each backend applies

The display backend decides what it can drive. Position, mirror and off need no modeset and work on every backend.
Changing a mode or scale works on the virtual (headless) backend and on a hardware (DRM) output. The nested backend, a
window on another desktop, refuses both by name, and takes a supersample factor (see [Supersampling](#supersampling)).

On a hardware output:

- **Off until you ask.** A mode change on a hardware output is off. It is refused with an error that names
  `GNOBLIN_DRM_MODESET`, and the output keeps its mode. Start the session with `GNOBLIN_DRM_MODESET=1` to allow it
  (`true`, `yes` and `on` also count; anything else, or no variable, leaves it off). A config file or a script cannot
  turn it on.
- **Mode.** The mode must be one the monitor lists, with the refresh rate the kernel computes for it (`2560x1440@59.95`
  and `2560x1440@60` are different modes). The screen goes black for the length of the modeset.
- **Refused.** A mode the monitor does not list, a mode the kernel refuses, and an output that is off are all refused.
  The output keeps its mode, its picture and its colour settings.
- **Several outputs.** A configuration of several outputs is tested on all of them before the first is driven.
- **Scale.** Changing only the scale commits nothing: the output composes at the new scale from its next frame and does
  not blank.
- **Kept across a mode change.** The night-light gamma table is written again after the modeset, and variable refresh
  and the other output controls are left as they were.
- **Not applied.** A transform other than `normal` is refused on every backend: no renderer draws a rotation.

Turn the mode change on, check it on your own display before relying on it, and report a driver that refuses it.

The `wlr-output-management` protocol (`wlr-randr`, `kanshi`) and `gnoblinctl output call <id> apply` go through the same
engine as `apply`: the same checks, the same test before the first output is driven and the same rollback.

`output.apply` takes the configuration of one output, and each key is optional:

- **Keys.** `x`, `y`, `scale`, `transform`, `enabled`, `mode` and `supersample`. A key left out keeps the value the
  output has, as a key left out of a shell's `screen.apply` does, so `apply supersample=2` changes only the factor and
  `apply scale=1` keeps the factor. A call with no key changes nothing.
- **Position.** `x` and `y` are one setting. Given only one, the other is the output's current coordinate.
- **Mode.** An empty `mode` keeps the mode the output has, as leaving it out does.
- **Strict.** A name that is not a key is refused, and so is a value of the wrong type or of a range the engine
  does not take (a `supersample` under 1, a `scale` outside 0.5 to 4). In each case nothing is applied.
- **Primary.** `primary` is not a key: the layout derives the primary output. A call that carries it is refused with
  an error that names `primary`, and nothing is applied. In a script, `output.apply({ primary: true })` throws a
  `TypeError` that names it.
- **Refusal.** The call completes with `mode-refused`, `scale-refused`, `position-refused`, `enabled-refused`,
  `transform-refused`, `supersample-refused` or `refused`.

## Hardware profiles

An `OutputProfile` is the persistent setting of one monitor: which monitor it is for (`match`) and the mode, scale,
transform and place it should have. Put it in `gnoblin.outputs.profiles`. A profile is inert while no matching output is
connected; when one is, the profile applies as one request, and it applies again when the monitor comes back after an
unplug.

```ts
import { gnoblin } from "gnoblin-core";
import { OutputProfile } from "gnoblin-std";

const main = new OutputProfile({
    match: { connector: "DP-1" },
    mode: { width: 3440, height: 1440, refreshRate: 144 },
    scale: 1.25,
    position: { x: 0, y: 0 },
});
const side = new OutputProfile({
    match: { connector: "HDMI-A-1" },
    mode: "preferred",
    position: { rightOf: main, align: "top" },
});
gnoblin.outputs.profiles.add(main);
gnoblin.outputs.profiles.add(side);
```

| Key         | Values                                                                                                  |
| ----------- | ------------------------------------------------------------------------------------------------------- |
| `id`        | The profile's name in the collection. Default: the `match` written out, such as `DP-1`.                 |
| `match`     | `connector`, `make`, `model` and `serial`, all that you give. `serial` is the third part of `identity`. |
| `mode`      | `"preferred"`, `"3440x1440@144"` or `{ width, height, refreshRate }`.                                   |
| `scale`     | A number from 0.5 to 4.                                                                                 |
| `transform` | `"normal"`: the others are refused, as for `output.apply`.                                              |
| `position`  | `{ x, y }`, or one of `rightOf`, `leftOf`, `above` and `below` naming another profile, with `align`.    |
| `enabled`   | `false` switches the output off.                                                                        |
| `vrr`       | Not applied yet: accepted and reported, because the output request has no variable refresh rate key.    |

- **By hardware.** `match: { make: "BOE", model: "Panel", serial: "SN42" }` finds a monitor whatever port it is on.
  `output.make`, `output.model` and `output.identity` show what a connected display reports.
- **Next to another.** A relative position places the output against the other profile's output.
    - The neighbour's own settings go first, and the position is sent once its answer is in, so a mode or scale change
      moves what is next to it.
    - `align` is `top`, `center` or `bottom` beside the neighbour, and `left`, `center` or `right` above or below it.
    - A neighbour that is not connected leaves the position unapplied, and a notice says so.
    - Placements that depend on themselves throw.
- **Refusal.** The compositor checks the whole arrangement, so an overlap, a gap or a mode the backend refuses leaves the
  last good one. The refusal is reported as a diagnostic naming `outputs.profiles.<id>.<key>`.
- **Changing and removing.** Every property but `match` can be assigned later, and the change applies at the next frame.
  Removing a profile stops the shell from applying it; the compositor keeps the arrangement it applied last.

## Displays and viewports

An `Output` is a connector. A `Display` is what the desktop presents on one or several outputs, under one `Viewport`
whose `content` is the workspace shown. `gnoblin.displays` is the live collection.

- The shell keeps one `single` display for every output that none of your displays presents, so each monitor is always
  in the collection.
- `gnoblin.input.primary.activeDisplay` is the display under the pointer.

```ts
import { gnoblin } from "gnoblin-core";
import { Camera2D, Display, Viewport, Workspace } from "gnoblin-std";

const desktop = new Workspace({ id: "desktop" });
const web = new Workspace({ id: "web" });
gnoblin.workspaces.add(desktop);
gnoblin.workspaces.add(web);

gnoblin.input.primary.bind("Super+2", () => {
    const display = gnoblin.input.primary.activeDisplay;
    if (display) display.viewport.content = web;
});
```

Assigning `viewport.content` shows the workspace on that display, as `gnoblin.workspaces.show` does. A workspace is on one
display at a time: assigning one that another display shows moves it, and that display takes the workspace this one
showed. Reading `content` gives what the display shows now. Inside a `gnoblin.requestTransition` callback the assignment
is part of the transition, and a failed callback puts it back.

`new Display({ outputs, mode, viewport })` takes `OutputProfile`s (persistent selectors: the display is parked while no
matching output is connected and keeps its workspace) or connected `Output`s:

| `mode`     | What it presents                                                                                         |
| ---------- | -------------------------------------------------------------------------------------------------------- |
| `"single"` | One output (the default).                                                                                |
| `"mirror"` | The first output's picture on the others, through the compositor's mirror (scaled to fit, letterboxed).  |
| `"span"`   | One workspace laid out over the bounding box of the outputs' work areas. A window may straddle the seam. |

```ts
const spanned = new Display({
    outputs: [main, side],
    mode: "span",
    viewport: new Viewport({ content: desktop, camera: new Camera2D() }),
});
gnoblin.displays.add(spanned);
```

- **One display per output.** `gnoblin.displays.add` throws a `GnoblinError` with code `busy`, naming both displays, when one of the
  outputs belongs to a display of yours already. `gnoblin.displays.replace(old, new)` swaps them as one change: it
  removes `old` and every other display of yours that shares an output with `new`, then adds `new` in `old`'s place.
- **Removing.** `gnoblin.displays.remove(display)` ends the mirror or span the display made; each of its outputs gets a
  display of the shell's again. A display of the shell's own cannot be removed.
- **A span's area.** The bounding box of unequal outputs has corners that no output shows; a window a layout puts there
  is not visible. Maximise and fullscreen use the output under the window.
- **A mirror** ends with the display. A reload whose config does not declare it does not undo it: apply
  `output.mirror(null)` for that.

## Cameras

A viewport looks at its content through a camera, `Camera2D` or `Camera3D`. Both are scene nodes, so a
`gnoblin.requestTransition` can animate their properties.

```ts
import { gnoblin } from "gnoblin-core";
import { Camera2D, Camera3D } from "gnoblin-std";

const flat = new Camera2D();
const deep = new Camera3D({ projection: "perspective", fov: 65, position: { x: 0, y: 0, z: 1600 } });
let threeD = false;
gnoblin.input.primary.bind("Super+Tab", () => {
    threeD = !threeD;
    gnoblin.requestTransition(
        () => {
            const display = gnoblin.input.primary.activeDisplay;
            if (display) display.viewport.camera = threeD ? deep : flat;
        },
        { duration: 750 },
    );
});
```

`Camera2D` has `position` (`{ x, y }`), `zoom` and `rotation` (radians); `Camera3D` has `position`, `projection`, `fov`,
`size` (orthographic), `near` and `far`.

**What is drawn.** Windows sit directly under the tree root and keep their own positions, so the shell has no
per-display root to transform. A camera on a viewport is kept and reads back, and nothing more is applied:

- A `Camera2D` that is not the identity (`zoom` 1, `position` 0, `rotation` 0) is not applied yet: it is reported once
  per reload as not applied.
- A `Camera3D` is not applied yet: it is reported once per reload as `not applied: camera3d (needs a projected
workspace)`. A flat-workspace perspective projection is renderer work.
- `viewport.clearColor` is not applied yet: it is kept, and the compositor has no background colour behind a workspace
  to apply it to.

A layout can place windows in depth with `z`, `rotation` and `scale`; the camera does not add to that.

## Listen for plugging and unplugging

```ts
import { OutputListener, gnoblin } from "gnoblin-core";
import type { Output } from "gnoblin-core";

class Hotplug extends OutputListener {
    onConnected(output: Output) {
        gnoblin.spawn(["notify-send", `${output.name} plugged in`]);
    }
    onDisconnected(output: Output) {
        gnoblin.spawn(["notify-send", `${output.name} unplugged`]);
    }
}

gnoblin.outputs.addListener(new Hotplug());
```

[Events](/events/desktop-state#listen-to-outputs) lists the methods an `OutputListener` can define.

A listener on the collection hears a connection only for
an output plugged in while the shell runs. A disconnection is reported after the output is gone, and its last values stay
readable. Membership changes reach a collection listener as additions and removals, as they do for windows.

## One workspace on one output

A `Workspace` is on one output at a time. Each output starts on the first workspace no output shows. `output.show(ws)` puts
`ws` on that output. If another output shows it, that output takes the workspace this one showed, so no workspace is on two
outputs.

| Call                                | Acts on                                                                   |
| ----------------------------------- | ------------------------------------------------------------------------- |
| `output.show(ws)`                   | That output.                                                              |
| `gnoblin.show(ws)`, `ws.show()`     | The output under the pointer, or the primary output when no pointer says. |
| `gnoblin.workspaces.show(ws)`       | The same as `gnoblin.show`.                                               |
| `Super+1` to `Super+9` (stock keys) | The same as `gnoblin.show`.                                               |

- `ws.active` is `true` while an output shows the workspace, so with two outputs two workspaces are active.
- `ws.output` is the output that shows it, and `client.output` is the output that shows the window's workspace.
- A new window joins the workspace the output under the pointer shows.
- `ShowOptions` takes `output` too: `gnoblin.show("2", { output })`.

Each display chooses its own workspace: set `display.viewport.content = workspace` for the display you mean (see
[Displays and viewports](#displays-and-viewports)).

## Areas and bars

A layout computes in the area of the output that shows its workspace, in the compositor's coordinates: a workspace on an
output at `x = 1920` lays windows out at `x >= 1920`. `gnoblin.workspaces.area` is the primary output's area.

A `Bar` sits on the primary output unless you give it one:

```ts
import { Bar } from "gnoblin-std";

new Bar({ edge: "bottom", height: 24, output: gnoblin.outputs.at(1) });
```

A bar takes room only from windows on its own output.

A bar hides while a fullscreen window shows on the workspace its output
shows, and the other outputs' bars stay. A bar whose output is unplugged is hidden and takes no room. Maximise and fullscreen
use the output the window is on, and a drag stays inside that output's area.

## Unplugging

When an output goes:

- **Another output remains.** The windows of the workspace it showed move to the workspace the remaining output shows, fitted
  to that output: a maximised window fills it, a snapped one tiles again, a floating one is shrunk to fit and moved inside
  it. The workspace the unplugged output showed is on no output until you show it.
- **No output remains.** The windows wait hidden, their workspace is not active, and the keyboard focus is dropped, because a
  window with no output cannot hold it. The next output to appear shows that workspace again, the windows come back, and the
  first of them takes the focus when nothing else holds it.
    - The windows stay in the tree while they wait, however long the wait is.
    - Each comes back as the same window: on the workspace it was on, snapped or maximised as it was.

A script that runs with no output at all still has a workspace on show; it just lays nothing out. A reload hands the next
configuration what each output showed, by output name, and each output goes back to that workspace when it still exists.

## Move a window between outputs

```ts
const [client] = gnoblin.find("Window", { focused: true });
client?.moveToOutput(gnoblin.outputs.at(1)!); // true when it moved
```

`moveToOutput` is `moveToWorkspace` to the workspace the output shows. Both fit the window to the work area it lands in and
return `false`, moving nothing, for:

- a fullscreen window;
- a dialog, which moves only with its parent;
- a dead window, an output that is gone or shows nothing, or a workspace that is not in `gnoblin.workspaces`;
- the workspace the window is on already.

A window that held the focus and leaves the shown workspace hands the focus to a window that stays, or to nobody. A snapped
window's remembered floating rectangle is not refitted for the new output.

## Dialogs

A window that its client made a dialog of another window (`xdg_toplevel.set_parent` or xdg-foreign) has a `parentWindow`.
The stock shell:

- puts the dialog on its parent's workspace and centres it in the parent when it opens;
- moves it by the same distance whenever the parent moves, whoever moved the parent;
- takes it along when the parent changes workspace or output, and never offers it to a tiling layout;
- places it as an ordinary window when its parent is gone.

## Windows from a launcher

`client.fromLauncher` is `true` when the window opened on an activation token that a privileged or trusted Wayland
connection, the session's launcher, asked for. The compositor records it when the window redeems such a token. The
`focus.newWindows` setting then decides whether a launched window takes the focus: only `allow` lets it.

A program started with `gnoblin.spawn` is not marked, because the shell's spawner does not mint a token and the compositor
has nothing to attribute the window to. A launcher that runs as a Wayland client and mints a token for the program it starts
does mark it.
