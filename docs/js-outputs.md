# Outputs and desks in TypeScript

An output is a connected display, real or virtual. In the TypeScript shell each output shows one workspace of its own,
bars sit on one output, and a window that loses its output moves to another or waits hidden for the next. This page is
the TypeScript form of [Outputs](/shell-api/outputs), which describes the Lua runtime.

```ts
import { defineConfig, gnoblin } from "gnoblin";

defineConfig({
    workspaces: [{ id: "1" }, { id: "2" }, { id: "3" }],
    setup: (gnoblin) => {
        for (const output of gnoblin.outputs) {
            console.log(output.name, output.mode, output.workspace?.id);
        }
    },
});
```

## `gnoblin.outputs`

`gnoblin.outputs` is a live, read-only collection of `Output` objects, one for each screen node the compositor
publishes. An output is added when it is plugged in and removed when it goes. Every read of one output returns the same
object, so you can keep it. A removed output keeps its last values and reads `connected === false`.

| Member                      | Meaning                                                                              |
| --------------------------- | ------------------------------------------------------------------------------------ |
| `name`                      | The compositor's name for the output (`output-0`). A workspace set's entry names it. |
| `make`, `model`             | What the compositor knows of the display.                                            |
| `mode`                      | The mode in use, `WxH@Hz`.                                                           |
| `scale`, `transform`        | The scale the output composes at, and its transform.                                 |
| `x`, `y`, `width`, `height` | The output's rectangle in the compositor's coordinates, and `bounds` as one object.  |
| `primary`, `enabled`        | Whether it is the primary output, and whether it is on.                              |
| `area`                      | What a layout tiles in on this output: its rectangle less the bars on it.            |
| `workspace`                 | The workspace the output shows, or `null`.                                           |
| `show(workspace, options?)` | Shows a workspace on this output. `options.duration` slides the windows.             |
| `connected`, `alive`        | False once the display is gone.                                                      |
| `addListener(listener)`     | Attaches an `OutputListener`.                                                        |

A script reads these properties and writes none. `outputs.where({ primary: true })` selects live, as the other
collections do, on any of the keys above.

Changing an output's settings from a script is not available yet: the compositor has no operation for it, and the
`outputs` key of `defineConfig` is stored as written and not applied.

## Listening

```ts
import { OutputListener, gnoblin } from "gnoblin";

gnoblin.outputs.addListener(
    new (class extends OutputListener {
        onConnected(output) {
            gnoblin.spawn(["notify-send", `${output.name} plugged in`]);
        }
        onDisconnected(output) {
            console.log(output.name, "unplugged", output.width);
        }
        onModeChanged(output, mode) {
            console.log(output.name, "now", mode);
        }
    })(),
);
```

An `OutputListener` can define any of these:

- `onConnected`, `onDisconnected`
- `onModeChanged`, `onScaleChanged`, `onGeometryChanged`

A listener on the collection hears a connection only for an output plugged in while the shell runs. A disconnection is
reported after the output is gone, and its last values stay readable. Membership changes reach a collection listener
as additions and removals, as they do for clients.

## One workspace on one output

A `Workspace` is the script's own object, so it is on one output at a time. Each output starts on the first workspace no
output shows. `output.show(ws)` puts `ws` on that output. If another output shows it, that output takes the workspace
this one showed, so no workspace is on two outputs.

| Call                                | Acts on                                                                   |
| ----------------------------------- | ------------------------------------------------------------------------- |
| `output.show(ws)`                   | That output.                                                              |
| `gnoblin.show(ws)`, `ws.show()`     | The output under the pointer, or the primary output when no pointer says. |
| `Super+1` to `Super+9` (stock keys) | The same as `gnoblin.show`.                                               |
| `gnoblin.workspaces.show(ws)`       | The same as `gnoblin.show`.                                               |

`ws.active` is true while an output shows the workspace, so with two outputs two workspaces are active. `ws.output` is
the output that shows it. `client.output` is the output that shows the client's workspace.

A new window joins the workspace the output under the pointer shows, as in the Lua shell.

The Lua shell gives each output its own copy of every workspace, so two outputs can both show workspace 1. Here the
workspace is shared, which is why showing it on one output takes it from another.

`ShowOptions` takes `output` too: `gnoblin.show("2", { output })`.

Workspace sets (`gnoblin.workspaces.sets`) show each entry's workspace on its own output. An entry for an output that is
not connected is reported and shows on the primary output.

## Areas and bars

A layout computes in the area of the output that shows its workspace, in the compositor's coordinates: a workspace on an
output at `x = 1920` lays windows out at `x >= 1920`. `gnoblin.workspaces.area` is the primary output's area.

A `Bar` sits on the primary output unless you give it one:

```ts
import { Bar } from "gnoblin";

new Bar({ edge: "bottom", height: 24, output: gnoblin.outputs.at(1) });
```

A bar takes room only from windows on its own output, and `barInsets(output)` says how much. A bar hides while a
fullscreen window shows on the workspace its output shows, and the other outputs' bars stay. A bar whose output is
unplugged is hidden and takes no room.

Maximise and fullscreen use the output the window is on. A drag stays inside that output's area.

## Unplugging

When an output goes:

- **Another output remains.** The windows of the workspace it showed move to the workspace the remaining output shows,
  fitted to that output: a maximised window fills it, a snapped one tiles again, a floating one is shrunk to fit and
  moved inside it. The workspace the unplugged output showed is on no output until you show it.
- **No output remains.** The windows wait hidden, their workspace is not active, and the keyboard focus is dropped,
  because a window with no output cannot hold it. The next output to appear shows that workspace again, the windows
  come back, and the first of them takes the focus when nothing else holds it.

A script that runs with no output at all (a tree with no screen) still has a workspace on show; it just lays nothing out.

## Moving a window

```ts
client.moveToOutput(gnoblin.outputs.at(1)); // true when it moved
client.moveToWorkspace(gnoblin.workspaces.get("3")!);
```

`moveToOutput` is `moveToWorkspace` to the workspace the output shows. Both fit the window to the work area it lands in
and return `false`, moving nothing, for:

- a fullscreen window;
- a dialog (it moves only with its parent);
- a dead window, an output that is gone or shows nothing, or a workspace not in `gnoblin.workspaces`;
- the workspace the window is on already.

A window that held the focus and leaves the shown workspace hands the focus to a window that stays, or to nobody.

## Dialogs

A window the client made a dialog of another window (`xdg_toplevel.set_parent` or xdg-foreign) has a `parentWindow`. The
compositor writes it to the window's node as `parent_window`. The stock shell:

- puts the dialog on its parent's workspace and centres it in the parent when it opens;
- moves it by the same distance whenever the parent moves, whoever moved the parent;
- takes it along when the parent changes workspace or output, and never offers it to a tiling layout;
- places it as an ordinary window when its parent is gone.

## Windows from a launcher

`client.fromLauncher` is true when the window opened on an activation token that a privileged or trusted Wayland
connection (the session's launcher) asked for. The compositor records it when the window redeems such a token, and the
`focus.newWindows` setting then decides whether a launched window takes the focus: only `allow` lets it.

A program started with `gnoblin.spawn` is not marked. The shell's spawner does not mint a token, so the compositor has
nothing to attribute the window to. A launcher that runs as a Wayland client and mints a token for the program it starts
does mark it.

## Reloading

A reload hands the next generation what each output showed, by output name, and the generation puts each output back on
that workspace when the workspace still exists.

## Not done

- `Output.apply` and applying `outputs` from `defineConfig`: the compositor has no operation for it.
- A snapped window's remembered floating rectangle is not refitted when the window moves to another output.
