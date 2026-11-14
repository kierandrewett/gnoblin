# Focus and activation

Focus policy decides which window gets the keyboard, and when it rises above the others. Set it in the `focus` key of
`defineConfig`; read the order windows were focused with `gnoblin.focus.history()`.

```ts
import { defineConfig } from "gnoblin";

defineConfig({
    focus: { mode: "sloppy", autoRaise: true, autoRaiseDelay: 300 },
});
```

Put the file in `~/.config/gnoblin/init.ts`. A window gets the focus when the pointer enters it, and rises 300 ms later.

## Focus settings

Every key is optional. A key you leave out is the compositor's own value for that setting.

| Key                   | Values                            | Default   | Effect                                                   |
| --------------------- | --------------------------------- | --------- | -------------------------------------------------------- |
| `mode`                | `click`, `sloppy`, `follow-mouse` | `click`   | How pointer movement changes the focus.                  |
| `newWindows`          | `allow`, `prevent`                | `prevent` | Whether a window that opens from a launcher takes focus. |
| `raiseOnClick`        | boolean                           | `true`    | A click on a window raises it.                           |
| `autoRaise`           | boolean                           | `false`   | The focused window rises by itself.                      |
| `autoRaiseDelay`      | milliseconds, 0 or more           | `500`     | How long after focus an auto raise waits.                |
| `changeOnPointerRest` | boolean                           | `false`   | Focus follows the pointer only once it stops moving.     |

The three modes:

- `click` focuses a window when you click it.
- `sloppy` focuses the window under the pointer as the pointer enters it, and keeps the focus when the pointer moves onto
  the desktop.
- `follow-mouse` is `sloppy`, and also clears the focus when the pointer leaves the focused window.

`newWindows` applies to a window that opened on an activation token a launcher minted (`client.fromLauncher`). A program
started with `gnoblin.spawn` is not marked, because the shell's spawner mints no token. With `prevent`, a new window opens
without taking the focus; with `allow` it takes it.

The settings are part of the config document, so a change takes effect on the next reload. [defineConfig reference](/typescript-config#focus)
lists how they combine with the stock config. The seat's current values are `gnoblin.input.primary.settings`.

There `focusMode` uses the compositor's own names: `click`, `hover` and `hover-strict`.
They stand for `click`, `sloppy` and `follow-mouse`.

## Give a window the focus

`client.focus()` asks for the keyboard on a window. The compositor applies its own policy: it may leave the focus where it
was. To raise the window as well, call `client.raise()`.

```ts
const client = gnoblin.clients.find((c) => c.appId === "foot");
client?.focus();
client?.raise();
```

A window that closes while focused hands the keyboard to its neighbour on the workspace: the next window, else the
previous one. A dialog hands it to its parent.

## Focus history

`gnoblin.focus.history(filter?)` returns the live windows, last focused first. Windows never seen focused follow in list
order, and minimised windows are included.

```ts
const [current, previous] = gnoblin.focus.history({ limit: 2 });
previous?.focus();
```

| Filter key    | Values                | Default | Effect                              |
| ------------- | --------------------- | ------- | ----------------------------------- |
| `workspaceId` | a workspace id        | none    | Only the windows of that workspace. |
| `monitorId`   | an output name        | none    | Only the windows of that output.    |
| `limit`       | integer from 1 to 256 | `50`    | How many windows to return.         |

An unknown filter key throws a `TypeError` that names the keys that work.

## Launching from a layer surface

A shell that runs as a separate Wayland client can start an application from a panel or popup with the standard
[XDG activation protocol](https://wayland.app/protocols/xdg-activation-v1).

Request the token from the connection that received the input, with that surface, seat and initiating press serial. Commit
the request before you dismiss or destroy the source surface, then pass the returned token to the application.

With `newWindows: "prevent"`, the compositor keeps a verified launch gesture after the source closes. Releasing its key or
mouse button does not revoke it. A later press, touch start, scroll or session lock does, and unlocking does not restore it.
The token expires 15 seconds after the initiating event and can authorize focus only once.

`newWindows: "allow"` keeps its permissive activation policy.
