# Launch feedback

`gnoblin.launches` tracks the busy state between asking for an application and its window, so a panel or launcher can show
a spinner while the program starts.

```ts
import { defineConfig } from "gnoblin";

defineConfig({
    keys: {
        "Super+e": ({ gnoblin }) => {
            gnoblin.launches.begin("terminal-1", "foot", {
                timeoutMs: 5000,
                onStarted: () => gnoblin.spawn(["notify-send", "The terminal is open"]),
                onTimedOut: () => gnoblin.spawn(["notify-send", "The terminal did not open"]),
            });
            gnoblin.spawn("foot");
        },
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. `Super+e` starts the `foot` terminal and tracks the launch: a notification
says whether its window appeared within five seconds.

## Begin a launch

`gnoblin.launches.begin(token, application, options?)` returns a `Launch` that is `pending` at once, with `revision` 0.
Anything outside these limits throws:

| Argument      | Accepted values                          | Default | Meaning                                                       |
| ------------- | ---------------------------------------- | ------- | ------------------------------------------------------------- |
| `token`       | non-empty string, at most 128 characters | none    | Names the launch. Beginning the same token again replaces it. |
| `application` | non-empty string, at most 512 characters | none    | The application id the window is matched against.             |
| `timeoutMs`   | integer from 100 to 10000                | `3000`  | How long the launch waits for a window.                       |

`options` also takes the handlers listed below.

A launch leaves `pending` once, for one of three states:

| State       | When                                                                                         |
| ----------- | -------------------------------------------------------------------------------------------- |
| `started`   | A window of the application appears, takes focus or names its application id.                |
| `timed_out` | The deadline passes first.                                                                   |
| `ended`     | `launch.finish()` is called, the token is begun again (`replaced`), or the registry is full. |

## Handlers

| Handler                     | Called when                                                                                                     |
| --------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `onChanged(launch, fields)` | A field of the launch changed. `fields` names them.                                                             |
| `onStarted(launch)`         | The launch became `started`.                                                                                    |
| `onTimedOut(launch)`        | The launch became `timed_out`.                                                                                  |
| `onClosed(launch, reason)`  | The launch left `pending`. It runs last. `reason` is the new state, `replaced`, or why the registry refused it. |

A `Launch` has these fields: `token`, `application`, `timeoutMs`, `startedAt` (Unix time in milliseconds), `state`, `alive`
(`true` while pending), `revision` (the number of changes) and `finish()`, which ends a pending launch. `finish` on a launch
that is not pending changes nothing.

## The registry

`gnoblin.launches.list()` returns the registry, oldest first. It holds at most 64 launches. A new one drops the oldest
finished one. With all 64 pending, the new launch comes back `ended`, with the reason in its `onClosed`.

A window matches by application id, compared in lowercase with a trailing `.desktop` removed. A window starts only the
oldest pending launch it matches, and a window that was already there when the launch began does not start it until it takes
focus.

This is the shell's own record. The compositor keeps no launch state, so another client does not see these launches, and a
`Launch` is not linked to the window or process that answered it.
