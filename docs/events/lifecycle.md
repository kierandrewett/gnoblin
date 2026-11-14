# Lifecycle events

These signals report the session, the configuration, operations that finish later and the requests behind the
compositor's dialogs. Subscribe with `gnoblin.on` as [Events](/events/) describes. Each event also carries `name` and
`object`.

## Session

| Event                                | Fields              | When                                                                |
| ------------------------------------ | ------------------- | ------------------------------------------------------------------- |
| `gnoblin.session.ready`              | none                | The session can accept requests. Raised once.                       |
| `gnoblin.session.bell`               | `count`             | An application rang the bell. `count` is the bells since login.     |
| `gnoblin.session.reload_requested`   | `generation`        | `gnoblinctl config reload` asked for a reload.                      |
| `gnoblin.session.lock_state_changed` | `state`, `revision` | The lock state changed. `revision` is the value `status()` reports. |

`count` lets a handler notice a bell it missed. Whether a bell flashes the screen or beeps is the compositor's
`visual_bell` and `audible_bell` setting, not the script's.

`gnoblin.session.reload_requested` is for information. The shell reloads on its own when it is asked, so a handler does not
reload anything. A script that wants to start a reload calls `gnoblin.runtime.reloadConfig()`, which
[Session and runtime](/shell-api/session-runtime#reload) describes.

`state` of `gnoblin.session.lock_state_changed` is one of these:

| State      | Meaning                                                                       |
| ---------- | ----------------------------------------------------------------------------- |
| `unlocked` | The session is usable.                                                        |
| `covering` | A cover is going up or coming down.                                           |
| `locked`   | The cover is up on every output. Only this state confirms that the lock took. |
| `failsafe` | The cover is up and no lock screen can proceed.                               |

```ts
import { toast } from "gnoblin";

gnoblin.on("gnoblin.session.lock_state_changed", (event) => {
    if (event.state === "locked") {
        toast("Locked");
    }
});
```

## Configuration

| Event                          | Fields                                                           | When                                                      |
| ------------------------------ | ---------------------------------------------------------------- | --------------------------------------------------------- |
| `gnoblin.config.reloaded`      | `path`, `revision`, `warnings`, `notApplied`, `sequence`, `time` | A reload was accepted. The new configuration is in force. |
| `gnoblin.config.reload_failed` | `path`, `error`, `sequence`, `time`                              | A reload was refused. The old configuration stays.        |

- `path` is the configuration file the reload read.
- `revision` is the number the compositor gave the committed document.
- `warnings` lists the problems of the sections the reload dropped, each a string. It is empty when every section
  validated.
- `notApplied` lists the settings accepted without effect, each a sentence naming the setting and the reason:
  `restart required` for a setting a process reads only when it starts, and `no effect` for one nothing reads.
- `error` is why a reload was refused: the first validation or evaluation failure.
- `sequence` counts accepted reloads in `reloaded` and refused reloads in `reload_failed`. `time` is the shell's
  monotonic clock reading. Neither is a wall clock.

## Operations

| Event                         | Fields                                                     | When                                                            |
| ----------------------------- | ---------------------------------------------------------- | --------------------------------------------------------------- |
| `gnoblin.operation.completed` | `operationId`, `method`, `ok`, `error`, `sequence`, `time` | An operation finished: the compositor or the shell answered it. |

`ok` is `true` for a success. `error` is the failure code, empty on success. A script usually waits for one operation
through its own `Operation` instead: see [Operations, errors and timers](/shell-api/errors).

## Requests behind dialogs

| Event                      | Fields                                                       | When                                        |
| -------------------------- | ------------------------------------------------------------ | ------------------------------------------- |
| `gnoblin.request.offered`  | `request`, `deadline`, `slot`, `title`, `message`, `subject` | The compositor has a dialog to open.        |
| `gnoblin.request.finished` | `request`, `completion`, `reason`                            | The request ended. Raised once per request. |

`request` is the request's identity, the same in both events. `slot` names the kind of request. `subject` is text the
asking program chose and is not an identity to trust. The event never carries an answer.

To watch these requests with typed objects, and to refuse one, attach a listener instead:
[Requests and dialogs](/shell-api/requests) and [Authentication prompts](/shell-api/authentication).
