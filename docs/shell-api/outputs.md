# Outputs

An output is a connected display, real or virtual. Each output shows one workspace of its own, bars sit on one output, and
a window that loses its output moves to another or waits hidden for the next. `gnoblin.outputs` is the live collection of
`Output` objects.

```ts
import { defineConfig, toast } from "gnoblin";

defineConfig({
    workspaces: [{ id: "1" }, { id: "2" }, { id: "3" }],
    setup: (gnoblin) => {
        for (const output of gnoblin.outputs) {
            toast(`${output.name}: ${output.mode} showing ${output.workspace?.id ?? "nothing"}`);
        }
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. A message names each output, its mode and the workspace it shows.

## Read outputs

`gnoblin.outputs` is a live, read-only collection with one `Output` for each screen the compositor publishes. An output is
added when it is plugged in and removed when it goes. Every read of one output returns the same object, so you can keep it.
A removed output keeps its last values and reads `connected === false`.

`gnoblin.outputs.where({ primary: true })` selects live, as the other collections do, on `id`, `name`, `make`, `model`,
`mode`, `scale`, `primary`, `enabled`, `x`, `y`, `width` and `height`. [`gnoblin.find("Output", filter)`](find) returns an
array.

| Member                      | Meaning                                                                                                        |
| --------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `name`                      | The compositor's name for the output, such as `output-0`. A workspace set's entry names it.                    |
| `connector`                 | The connector the display is plugged into, such as `DP-1`. `defineConfig({ outputs })` and `mirrorOf` name it. |
| `identity`                  | The monitor's EDID identity, `MAN:product:serial`. Empty for a display that reports none.                      |
| `make`, `model`             | What the compositor knows of the display. Empty when it knows nothing.                                         |
| `mirrorOf`                  | The output this one shows a copy of, or `null`.                                                                |
| `mode`                      | The mode in use, `WxH@Hz`.                                                                                     |
| `scale`                     | The factor the output composes at. 2 is a HiDPI panel.                                                         |
| `transform`                 | The rotation and flip: `normal`, `90`, `180`, `270`, `flipped`, `flipped-90`, `flipped-180` or `flipped-270`.  |
| `x`, `y`, `width`, `height` | The output's rectangle in the compositor's coordinates. `bounds` is the same as one object.                    |
| `position`                  | `{ x, y }` of the output.                                                                                      |
| `primary`                   | Whether this is the primary output, the one the stock bars and `gnoblin.workspaces.area` are on.               |
| `enabled`                   | Whether the output is on.                                                                                      |
| `area`                      | What a layout tiles in on this output: its rectangle less the bars on it. `null` when the output is gone.      |
| `workspace`                 | The workspace the output shows, or `null`.                                                                     |
| `show(workspace, options?)` | Shows a workspace on this output. `options.duration` slides the windows.                                       |
| `apply(options)`            | Configures the display. See [Configure an output](#configure-an-output).                                       |
| `mirror(source)`            | Shows a copy of `source`, or stops with `null`.                                                                |
| `connected`, `alive`        | `false` once the display is gone.                                                                              |
| `addListener(listener)`     | Attaches an `OutputListener`.                                                                                  |

A script reads these properties and writes none. It changes a display with `apply` and `mirror`, below, or keeps a
setting for a monitor with an [`OutputProfile`](#hardware-profiles) in `gnoblin.outputs.profiles`.

## Configure an output

`output.apply({ ... })` asks the compositor to change one display. Every key is optional, and a key left out keeps its
value.

- The compositor checks the whole new arrangement before it touches hardware.
- It then drives the changed outputs. If the backend refuses a mode, the outputs already changed are put back and the
  last good arrangement stays.
- It keeps the result as that monitor's profile, so the same monitor on the same port gets it back after an unplug.

| Key         | Values                                                                           |
| ----------- | -------------------------------------------------------------------------------- |
| `mode`      | `"3440x1440@144"`, or `"preferred"` to keep what the backend drives.             |
| `scale`     | A number from 0.5 to 4.                                                          |
| `transform` | `"normal"`. The compositor draws no rotation, so the others are refused by name. |
| `position`  | `[x, y]` or `{ x, y }`: the top-left corner in the one desktop coordinate space. |
| `enabled`   | `false` switches the output off: it shows black and has no place in the desktop. |
| `mirrorOf`  | The connector, or an `Output`, to show an exact copy of. `null` stops mirroring. |

A value of the wrong shape throws before anything is sent, naming the key: `Output#output-1.apply.scale must be a number
from 0.5 to 4`.

The compositor answers a frame later. `apply` returns an `OutputApplication` with `done` and `error`, which is `null` on
success. An `OutputListener` hears the same answer in `onApplied(output, error)`. A refusal names the key, as in
`position: HDMI-1 and DP-1 would overlap`.

**Mode and scale.** Drive an output at another mode, composed at a fractional scale:

```ts
import { gnoblin } from "gnoblin";

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

**In the config.** `outputs` applies the same keys by connector when the config loads, and again when that display is
plugged in. An entry that does not check fails the config, naming `config.outputs.<connector>.<key>`; one the compositor
refuses is reported as a diagnostic naming `outputs.<connector>.<key>`.

```ts
export default defineConfig({
    outputs: {
        "DP-1": { mode: "3440x1440@144", scale: 1.25, position: [0, 0] },
        "HDMI-1": { mirrorOf: "DP-1" },
    },
});
```

### What each backend applies

The display backend decides what it can drive. Position, mirror and off need no modeset and work on every backend.
Changing a mode or scale works on the virtual (headless) backend and on a hardware (DRM) output. The nested backend, a
window on another desktop, refuses both by name.

On a hardware output:

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

The DRM path is proven against a fake kernel only. Check a mode change on your own display before relying on it, and
report a driver that refuses it.

The `wlr-output-management` protocol (`wlr-randr`, `kanshi`) and `gnoblinctl output call <id> apply` go through the same
engine as `apply`: the same checks, the same test before the first output is driven and the same rollback.

`output.apply` takes the whole configuration of one output:

- **Keys.** `x`, `y`, `scale`, `transform`, `primary`, `enabled` and `mode`.
- **Mode and primary.** An empty `mode` keeps the mode the output has. `primary` is accepted and not applied, because
  the layout derives the primary output.
- **Refusal.** The call completes with `mode-refused`, `scale-refused`, `position-refused`, `enabled-refused`,
  `transform-refused` or `refused`.

## Hardware profiles

An `OutputProfile` is the persistent setting of one monitor: which monitor it is for (`match`) and the mode, scale,
transform and place it should have. Put it in `gnoblin.outputs.profiles`. A profile is inert while no matching output is
connected; when one is, the profile applies as one request, and it applies again when the monitor comes back after an
unplug.

```ts
import { OutputProfile, gnoblin } from "gnoblin";

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
| `vrr`       | Accepted and reported as not applied: the output request has no variable refresh rate key yet.          |

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
import { Camera2D, Display, Viewport, Workspace, gnoblin } from "gnoblin";

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

- **One display per output.** `gnoblin.displays.add` throws `output-conflict`, naming both displays, when one of the
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
import { Camera2D, Camera3D, gnoblin } from "gnoblin";

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

- A `Camera2D` that is not the identity (`zoom` 1, `position` 0, `rotation` 0) is reported once per reload as not
  applied.
- A `Camera3D` is reported once per reload as `not applied: camera3d (needs a projected workspace)`. A flat-workspace
  perspective projection is renderer work.
- `viewport.clearColor` is kept; the compositor has no background colour behind a workspace to apply it to.

A layout can place windows in depth with `z`, `rotation` and `scale`; the camera does not add to that.

## Listen for plugging and unplugging

```ts
import { OutputListener, gnoblin } from "gnoblin";
import type { Output } from "gnoblin";

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

Workspace sets show each entry's workspace on its own output: see [Workspaces](workspaces#workspace-sets). An entry for an
output that is not connected is reported and shows on the primary output.

## Areas and bars

A layout computes in the area of the output that shows its workspace, in the compositor's coordinates: a workspace on an
output at `x = 1920` lays windows out at `x >= 1920`. `gnoblin.workspaces.area` is the primary output's area.

A `Bar` sits on the primary output unless you give it one:

```ts
import { Bar } from "gnoblin";

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
