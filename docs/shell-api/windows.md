# Windows

A window is a `Client`, one object per managed window. `gnoblin.clients` is the live collection of them. Read it, change a
window by writing its properties or calling its methods, and attach a listener to hear what happens to it.

::: tabs

```typescript
gnoblin.input.primary.bind("Super+t", () => {
    const terminal = gnoblin.clients.toList().find((client) => client.appId === "foot");
    terminal?.focus();
});
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()


def focus_terminal() -> None:
    terminal = next((client for client in gnoblin.clients.to_list() if client.app_id == "foot"), None)
    if terminal is not None:
        terminal.focus()


gnoblin.input.primary.bind("Super+t", focus_terminal)
```

```rust
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = Rc::new(ModelRoot::get(&g.objects()?)?);
    let shell = Rc::clone(&model);
    g.bind("Super+t", move || -> gnoblin::Result<()> {
        for client in shell.clients()?.to_list()? {
            if client.app_id()? == "foot" {
                client.focus()?;
                break;
            }
        }
        Ok(())
    })?;
    Ok(())
}
```

:::

Put the shell code in `~/.config/gnoblin/`: `shell.py` for Python, `shell.ts` for TypeScript, or a shell library for Rust,
which [Write a shell in Rust](/rust-shell) builds and runs. `Super+t` focuses the first terminal window it finds and does
nothing when there is none.

## Read windows

`gnoblin.clients` is a live set. It is not an array: its membership belongs to the compositor, so a script cannot add or
remove a window. The list form (`toList()` in TypeScript, `to_list()` in Python and Rust) returns the windows as an array.

Every read of one window returns the same `Client`, so you can keep it. A closed window keeps its last values and reads
`alive` as false.

The table uses TypeScript spellings. Python and Rust use snake_case for the same members (`at`, `get`, `to_list`, `where`,
`add_listener`).

| Member                  | Result                                                                                       |
| ----------------------- | -------------------------------------------------------------------------------------------- |
| `length`                | How many windows there are.                                                                  |
| `focused`               | The window that holds the keyboard focus, or `undefined`.                                    |
| `at(index)`             | The window at a position in list order, or `undefined`.                                      |
| `get(id)`               | The window whose `id` is that string, or `undefined`.                                        |
| `contains(client)`      | Whether the window is in the collection.                                                     |
| `toList()`              | An array of the windows, to use `find`, `filter`, `map` and `for...of` on.                   |
| `where(query)`          | A live selection of the windows whose properties equal the query. See [Find objects](find).  |
| `style`                 | The style every window starts from. See [window style](/js-style).                           |
| `addListener(listener)` | Attaches a `ClientCollectionListener` for every window. See [Events](/events/desktop-state). |

::: tabs

```typescript
const terminals = gnoblin.clients.toList().filter((client) => client.appId === "org.gnome.Console");
const titles = gnoblin.clients.toList().map((client) => client.title);
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()

terminals = [client for client in gnoblin.clients.to_list() if client.app_id == "org.gnome.Console"]
titles = [client.title for client in gnoblin.clients.to_list()]
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let mut terminals = Vec::new();
    let mut titles = Vec::new();
    for client in model.clients()?.to_list()? {
        titles.push(client.title()?);
        if client.app_id()? == "org.gnome.Console" {
            terminals.push(client);
        }
    }
    Ok(())
}
```

:::

## Properties

A read gives the state the compositor last published. A write asks the compositor to change it, and the next read shows
the result. Coordinates are logical pixels in the compositor's space, and an output further right starts at a larger `x`.

| Property                    | Type                       | Meaning                                                                                         |
| --------------------------- | -------------------------- | ----------------------------------------------------------------------------------------------- |
| `id`                        | string                     | The window's identity, stable while it lives. What `get(id)` takes.                             |
| `alive`                     | boolean                    | `false` once the window closed.                                                                 |
| `appId`                     | string                     | The application id the client set. Empty when it set none.                                      |
| `title`                     | string                     | The window title.                                                                               |
| `focused`                   | boolean                    | Whether the window holds the keyboard focus. Read-only.                                         |
| `x`, `y`, `width`, `height` | number                     | The window geometry, shadow left out. A write moves or resizes it.                              |
| `bounds`                    | `Rect`                     | `{ x, y, width, height }` of the window geometry. A write sets all four.                        |
| `maximised`                 | boolean                    | Whether the window is maximised. A write maximises or restores it.                              |
| `minimised`                 | boolean                    | Whether the window is hidden by minimising. A write hides or shows it.                          |
| `fullscreen`                | boolean                    | Whether the window covers its output. A write enters or leaves it.                              |
| `floating`                  | boolean                    | A floating window is left out of its workspace's tiling. A write floats or tiles it.            |
| `floatingBounds`            | `Rect` or `undefined`      | The rectangle the window floats at. Writing `undefined` forgets it.                             |
| `tiled`                     | boolean                    | Whether the window sits in a tiled layout.                                                      |
| `visible`                   | boolean                    | Whether the window is drawn.                                                                    |
| `opacity`                   | number                     | 0 (invisible) to 1 (opaque). The compositor clamps other values.                                |
| `z`                         | number                     | The window's place in depth. Negative is in front of the camera.                                |
| `rotation`, `scale`         | `[x, y, z]` numbers        | Rotation about each axis in radians, and the scale along each axis.                             |
| `secure`                    | boolean                    | `true` keeps the window out of screen captures.                                                 |
| `unresponsive`              | boolean                    | The client left a liveness ping unanswered too long. Read-only.                                 |
| `workspace`                 | `Workspace` or `undefined` | The workspace the window is on. Read-only: use `moveToWorkspace`.                               |
| `output`                    | `Output` or `undefined`    | The output that shows the window's workspace. Read-only: use `moveToOutput`.                    |
| `parentWindow`              | `Client` or `undefined`    | The window this one is a dialog of.                                                             |
| `fromLauncher`              | boolean                    | The window opened on a token the session's launcher minted.                                     |
| `frameInset`                | `FrameInset`               | The room the frame takes round `bounds`: `top`, `right`, `bottom`, `left`. All 0 with no frame. |
| `style`, `computedStyle`    | styles                     | What this window overrides, and what it resolves to. See [window style](/js-style).             |

`parentWindow` and `fromLauncher` are explained in [Outputs](outputs#dialogs).

A value of the wrong type throws a `TypeError` that names the property. Any other failure throws a `GnoblinError` whose
`code` is one of these:

- `not_found`: the window is gone.
- `read_only`: the property cannot be written.
- `denied`: the compositor refused the change.

## Window geometry and capture

A window's `bounds` are its window geometry: the part of its surface the user sees as the window. The geometry is what
the client sets with `xdg_surface.set_window_geometry`, and the whole surface while it set none.

A client that draws a shadow around its window, or a title bar of its own outside its main surface, has that drawn, but
it is not the window. A maximised or fullscreen window fills its output with the geometry, and the shadow lies past the
screen's edge.

The size you write to `width` and `height` is the size the client is asked for, so a window asked for the size it
reports has that size back.

What is outside the geometry is still drawn, and is hit where the client's input region says so. The two ways to take a
picture of a window differ in what they hold:

- A capture of a window (screencopy, toplevel capture and a screen cast) is the pixels the output shows over the window
  geometry. A shadow outside it is not in the picture, and a window that overlaps it is.
- A `View` of a window draws the window's own surface cut to the geometry, with the window's subsurfaces and popups
  as they are drawn on the output.

## Methods

| Method                       | Does                                                                                        |
| ---------------------------- | ------------------------------------------------------------------------------------------- |
| `focus()`                    | Gives the window the keyboard focus.                                                        |
| `close()`                    | Asks the client to close. The window is `alive` until the compositor destroys it.           |
| `raise()`                    | Puts the window on top of its siblings. A fullscreen sibling stays on top.                  |
| `lower()`                    | Puts the window under its siblings.                                                         |
| `kill()`                     | Sends SIGKILL to the process that owns the window. See below.                               |
| `moveToWorkspace(workspace)` | Moves the window to a workspace and fits it to the work area. Returns `false` if it cannot. |
| `moveToOutput(output)`       | Moves the window to the workspace an output shows. Returns `false` if it cannot.            |
| `set(values, options)`       | Writes properties, over time when given a duration. See [Tweens and easing](tweens).        |
| `moveWithKeys()`             | Starts moving the window with the arrow keys. Returns `false` if it cannot.                 |
| `resizeWithKeys()`           | Starts resizing the window with the arrow keys. Returns `false` if it cannot.               |
| `addListener(listener)`      | Attaches a `ClientListener` to this window.                                                 |
| `removeListener(listener)`   | Detaches it.                                                                                |

`kill()` is the not-responding alert's Force quit. The compositor refuses, and does nothing, when it cannot name the
owning process, when the process changed since the window mapped, or when the owner is the compositor or init.

`moveToWorkspace(workspace)` returns `false`, and moves nothing, for these:

- a fullscreen window, a dialog or a dead window;
- a workspace outside `gnoblin.workspaces`;
- the workspace the window is on already.

`moveToOutput(output)` moves the window to the workspace the output shows. It returns `false` when the output is gone or
shows no workspace, and in each case above for that workspace.

The keyboard modes `moveWithKeys` and `resizeWithKeys` are described in [Window policy](/js-window-policy#move-and-resize-with-the-keyboard).

## Window requests

A client asks for a change of its window, and the compositor changes nothing until the shell answers. `WindowState` makes
the shell answer every kind of request, and a `RequestListener` may answer first. The request kinds and the `WindowState`
helpers that run the stock answers are in [Window requests](/typescript-shell#window-requests).
