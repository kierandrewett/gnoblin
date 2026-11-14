# Windows

A window is a `Client`, one object per managed window. `gnoblin.clients` is the live collection of them. Read it, change a
window by writing its properties or calling its methods, and attach a listener to hear what happens to it.

```ts
import { defineConfig } from "gnoblin";

defineConfig({
    keys: {
        "Super+t": ({ gnoblin }) => {
            const terminal = gnoblin.clients.find((client) => client.appId === "foot");
            terminal?.focus();
        },
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. `Super+t` focuses the first terminal window it finds and does nothing when
there is none.

## Read windows

`gnoblin.clients` is a live set. It is not an array: its membership belongs to the compositor, so a script cannot add or
remove a window. Every read of one window returns the same `Client`, so you can keep it. A closed window keeps its last
values and reads `alive === false`.

| Member                      | Result                                                                                      |
| --------------------------- | ------------------------------------------------------------------------------------------- |
| `length`                    | How many windows there are.                                                                 |
| `focused`                   | The window that holds the keyboard focus, or `null`.                                        |
| `at(index)`                 | The window at a position in list order, or `undefined`.                                     |
| `get(id)`                   | The window whose `id` is that string, or `undefined`.                                       |
| `find(predicate)`           | The first window the function accepts, or `undefined`.                                      |
| `filter(predicate)`         | An array of the windows the function accepts.                                               |
| `map(mapper)`               | An array of what the function returns for each window.                                      |
| `where(query)`              | A live selection of the windows whose properties equal the query. See [Find objects](find). |
| `for (const client of ...)` | Iterates the windows.                                                                       |
| `style`                     | The style every window starts from. See [window style](/js-style).                          |
| `addListener(listener)`     | Attaches a listener for every window. See [Events](/events/desktop-state).                  |

```ts
const terminals = gnoblin.clients.filter((client) => client.appId === "org.gnome.Console");
const titles = gnoblin.clients.map((client) => client.title);
```

## Properties

A read gives the state the compositor last published. A write asks the compositor to change it, and the next read shows
the result. Coordinates are logical pixels in the compositor's space, and an output further right starts at a larger `x`.

| Property                    | Type                | Meaning                                                                             |
| --------------------------- | ------------------- | ----------------------------------------------------------------------------------- |
| `id`                        | string              | The window's identity, stable while it lives. What `get(id)` takes.                 |
| `alive`                     | boolean             | `false` once the window closed.                                                     |
| `appId`                     | string              | The application id the client set. Empty when it set none.                          |
| `title`                     | string              | The window title.                                                                   |
| `focused`                   | boolean             | Whether the window holds the keyboard focus. Read-only.                             |
| `x`, `y`, `width`, `height` | number              | The window's content rectangle. A write moves or resizes the window.                |
| `bounds`                    | `Rect`              | `{ x, y, width, height }` of the content. A write sets all four.                    |
| `maximised`                 | boolean             | Whether the window is maximised. A write maximises or restores it.                  |
| `minimised`                 | boolean             | Whether the window is hidden by minimising. A write hides or shows it.              |
| `fullscreen`                | boolean             | Whether the window covers its output. A write enters or leaves it.                  |
| `floating`                  | boolean             | A floating window is left out of its workspace's tiling.                            |
| `tiled`                     | boolean             | Whether the window sits in a tiled layout.                                          |
| `visible`                   | boolean             | Whether the window is drawn.                                                        |
| `opacity`                   | number              | 0 (invisible) to 1 (opaque). The compositor clamps other values.                    |
| `z`                         | number              | The window's place in depth. Negative is in front of the camera.                    |
| `rotation`, `scale`         | `[x, y, z]` numbers | Rotation about each axis in radians, and the scale along each axis.                 |
| `secure`                    | boolean             | `true` keeps the window out of screen captures.                                     |
| `unresponsive`              | boolean             | The client left a liveness ping unanswered too long. Read-only.                     |
| `workspace`                 | `Workspace` or null | The workspace the window is on. Read-only: use `moveToWorkspace`.                   |
| `output`                    | `Output` or null    | The output that shows the window's workspace. Read-only.                            |
| `parentWindow`              | `Client` or null    | The window this one is a dialog of.                                                 |
| `fromLauncher`              | boolean             | The window opened on a token the session's launcher minted.                         |
| `frameInset`                | number              | Pixels a titlebar takes above `bounds`. 0 with no bar.                              |
| `style`, `computedStyle`    | styles              | What this window overrides, and what it resolves to. See [window style](/js-style). |

`parentWindow` and `fromLauncher` are explained in [Outputs](outputs#dialogs).

A value of the wrong type throws a `TypeError` that names the property. Any other failure throws a `GnoblinError` whose
`code` is one of these:

- `destroyed`: the window is gone.
- `read-only`: the property cannot be written.
- `refused`: the compositor refused the change.

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
| `moveWithKeys()`             | Starts moving the window with the arrow keys. Returns `false` if it cannot.                 |
| `resizeWithKeys()`           | Starts resizing the window with the arrow keys. Returns `false` if it cannot.               |
| `animate(values, options)`   | Moves properties to new values over time. See [Tweens and easing](tweens).                  |
| `addListener(listener)`      | Attaches a `ClientListener` to this window.                                                 |
| `removeListener(listener)`   | Detaches it.                                                                                |

`kill()` is the not-responding alert's Force quit. The compositor refuses, and does nothing, when it cannot name the
owning process, when the process changed since the window mapped, or when the owner is the compositor or init.

`moveToWorkspace` and `moveToOutput` return `false`, and move nothing, for these:

- a fullscreen window, a dialog or a dead window;
- a workspace outside `gnoblin.workspaces`;
- the workspace the window is on already.

The keyboard modes `moveWithKeys` and `resizeWithKeys` are described in [Window policy](/js-window-policy#move-and-resize-with-the-keyboard).

## Window requests

A client asks for a change of its window, and the compositor changes nothing until the shell answers. The stock shell
answers every kind of request, and a listener's `onRequest` may answer first. The request kinds and the `windowState`
helpers that run the stock answers are in [Window requests](/typescript-shell#window-requests).
