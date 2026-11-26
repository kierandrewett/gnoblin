# Focus and activation

Focus policy decides which window gets the keyboard, and when it rises above the others. The shell keeps click to
focus, focuses a window that opens, and raises a window when you click it.

A file cannot change that policy. Focus mode, auto raise and the rest of the older `focus` settings have no object in the
API: see [Settings with no object](/js-settings). Read the order windows were focused with `gnoblin.focus.history()`, and the seat's current values with
`gnoblin.input.primary.settings`.

## Give a window the focus

`client.focus()` asks for the keyboard on a window. The compositor applies its own policy: it may leave the focus where it
was. To raise the window as well, call `client.raise()`.

```ts
const client = gnoblin.clients.toList().find((c) => c.appId === "foot");
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

With the compositor's default activation policy, it keeps a verified launch gesture after the source closes. Releasing its key or
mouse button does not revoke it. A later press, touch start, scroll or session lock does, and unlocking does not restore it.
The token expires 15 seconds after the initiating event and can authorize focus only once.
