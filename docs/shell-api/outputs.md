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

A script reads these properties and writes none. It changes a display with `apply` and `mirror`, below.

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
continuously, at any mix of scales. There is no separate span mode; an output that is not mirrored or off is part of it.

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

The display backend decides what it can drive. Position, mirror and off need no modeset and work everywhere; changing a
mode or scale works on the virtual and nested backends, and a hardware (DRM) output refuses it with the backend's reason
until that backend can modeset.

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
