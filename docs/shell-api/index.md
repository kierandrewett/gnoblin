# Shell runtime API

The runtime API is what a TypeScript shell sees: the `gnoblin` global and the classes you import from `"gnoblin"`. Use it
to read compositor state, place and style windows, handle input and events, and ask the session for things.

```ts
import { defineConfig } from "gnoblin";

defineConfig({
    setup: (gnoblin) => {
        for (const client of gnoblin.clients) {
            gnoblin.spawn(["notify-send", `${client.appId} is open`]);
        }
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. `setup` runs when the configuration loads, with the `gnoblin` object as its
argument. The same object is a global, so a listener or a key handler may use it too. To write your first config, start
with the [TypeScript shell](/typescript-shell). The [defineConfig reference](/typescript-config) lists the declarative keys.

## The `gnoblin` object

| Member                            | What it is                                                                  | Page                                      |
| --------------------------------- | --------------------------------------------------------------------------- | ----------------------------------------- |
| `clients`                         | The live collection of windows.                                             | [Windows](windows)                        |
| `workspaces`                      | The workspaces, and the sets that show several at once.                     | [Workspaces](workspaces)                  |
| `outputs`                         | The connected displays.                                                     | [Outputs](outputs)                        |
| `find(kind, filter?)`             | Selects windows, workspaces or outputs.                                     | [Find objects](find)                      |
| `focus`                           | The windows in the order they were focused.                                 | [Focus and activation](focus-activation)  |
| `input`                           | The seats and their devices, and bindings.                                  | [Input and seats](input)                  |
| `idle`                            | Watches for the session going idle, and holds that keep it awake.           | [Idle watches](idle)                      |
| `launches`                        | Launch feedback.                                                            | [Launch feedback](launch-feedback)        |
| `process`, `spawn`                | Programs the shell started.                                                 | [Processes](processes)                    |
| `session`, `runtime`, `version()` | The session, reloads and the build.                                         | [Session and runtime](session-runtime)    |
| `dialogs`                         | The compositor's consent and passphrase dialogs.                            | [Requests and dialogs](requests)          |
| `authentication`                  | Polkit authentication requests.                                             | [Authentication prompts](authentication)  |
| `timers`, `after`                 | Calls that run later, and the monotonic clock.                              | [Operations, errors and timers](errors)   |
| `motion(seconds)`, `gestures`     | Animation durations that obey reduced motion, and touchpad gesture helpers. | [Tweens and easing](tweens)               |
| `on`, `once`, `off`               | Compositor signals.                                                         | [Events](/events/)                        |
| `show(workspace, options?)`       | Shows a workspace.                                                          | [Workspaces](workspaces#show-a-workspace) |
| `extensions`                      | Optional extensions, such as XR.                                            | [XR mode](/xr#scripting-xr-in-typescript) |

## More pages

- [Canvas painting](canvas) and the [UI toolkit](/js-ui): draw the shell's own interface.
- [Keyboard shortcuts](keyboard-shortcuts) and [Input handlers](input-handlers): triggers, drags and gestures.
- [Window snapping](window-snapping): tile a window to an edge.
- [Shared types](types): records and small types that several pages use.
- [Window style](/js-style), [Window policy](/js-window-policy) and the [console](/js-console).

## Conventions

- Names are camelCase: `appId`, `exitCode`, `reloadConfig`.
- A read returns a live object or a plain value. A write to a live object asks the compositor, and the next read shows the
  result.
- A call that finishes later returns an `Operation`. A call with a bad argument throws at once. See
  [Operations, errors and timers](errors).
- Times are in the unit the name says: `timers.after` takes milliseconds and `after` takes seconds. Animation durations are
  seconds.
- Your code runs on the shell's single thread, between frames, under a time budget per call. Keep a handler short.
- Listeners and handlers that a configuration attached end with it, so a reload starts from your file again.

`gnoblin.version().api` is the API version of the build. The socket protocol for programs that are not scripts is in the
[compositor bridge](/bridge/). For command-line use, see [gnoblinctl](/cli/).

## Not in the TypeScript shell

These areas have no TypeScript API, because the compositor does not offer what they would need:

| Area                  | Why                                                                                                        |
| --------------------- | ---------------------------------------------------------------------------------------------------------- |
| Privacy state         | The compositor publishes none, and a shell does not reach the session bus that serves it.                  |
| Keyboard grab         | The tree carries no key events. A binding takes a chord, never the keyboard, so there is nothing to lease. |
| Text insertion        | There is no text-input target or grant to insert into.                                                     |
| Tablet-pad help       | The compositor sends no help request. A device lists a pad's name and capabilities and nothing more.       |
| Changing an output    | The compositor has no operation to apply a mode, scale or position.                                        |
| Node effects          | A window's blur and shadow are its [style](/js-style); there is no `effects` list on a node.               |
| Overrides of settings | A script lays no value over a committed setting. Change the file and reload.                               |
