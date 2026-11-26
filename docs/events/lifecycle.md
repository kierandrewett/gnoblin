# Lifecycle events

These events report the session, the configuration, operations that finish later and the requests behind the
compositor's dialogs. Hear the first two with a `SessionListener` and a `RuntimeListener`, as [Events](/events/)
describes. Each method receives `gnoblin.session` or `gnoblin.runtime` first, then the values in the table.

## Session

Attach a `SessionListener` with `gnoblin.session.addListener(listener)`. Every method is optional, and each is also a
named subscription, `gnoblin.session.on("bell", handler)`, that returns a `Listener` whose `stop()` detaches it.

| Method                                         | Compositor signal                    | When                                                                |
| ---------------------------------------------- | ------------------------------------ | ------------------------------------------------------------------- |
| `onReady(session)`                             | `gnoblin.session.ready`              | The session can accept requests. Raised once.                       |
| `onBell(session, count)`                       | `gnoblin.session.bell`               | An application rang the bell. `count` is the bells since login.     |
| `onReloadRequested(session, generation)`       | `gnoblin.session.reload_requested`   | `gnoblinctl config reload` asked for a reload.                      |
| `onLockStateChanged(session, state, revision)` | `gnoblin.session.lock_state_changed` | The lock state changed. `revision` is the value `status()` reports. |

`count` lets a listener notice a bell it missed. Whether a bell flashes the screen or beeps is the compositor's
`visual_bell` and `audible_bell` setting, not the script's.

`onReloadRequested` is for information. The shell reloads on its own when it is asked, so a listener does not reload
anything. A script that wants to start a reload calls `gnoblin.runtime.reloadConfig()`, which
[Session and runtime](/shell-api/session-runtime#reload) describes.

`state` of `onLockStateChanged` is one of these:

| State      | Meaning                                                                       |
| ---------- | ----------------------------------------------------------------------------- |
| `unlocked` | The session is usable.                                                        |
| `covering` | A cover is going up or coming down.                                           |
| `locked`   | The cover is up on every output. Only this state confirms that the lock took. |
| `failsafe` | The cover is up and no lock screen can proceed.                               |

```ts
import { SessionListener } from "gnoblin-core";
import type { LockState, Session } from "gnoblin-core";
import { Toast } from "gnoblin-std";

class LockToast extends SessionListener {
    onLockStateChanged(_session: Session, state: LockState) {
        if (state === "locked") {
            Toast.show("Locked");
        }
    }
}

gnoblin.session.addListener(new LockToast());
```

## Configuration

Attach a `RuntimeListener` with `gnoblin.runtime.addListener(listener)`. Each method takes the runtime, then one record.

| Method                                   | Compositor signal              | When                                                      |
| ---------------------------------------- | ------------------------------ | --------------------------------------------------------- |
| `onConfigReloaded(runtime, reload)`      | `gnoblin.config.reloaded`      | A reload was accepted. The new configuration is in force. |
| `onConfigReloadFailed(runtime, failure)` | `gnoblin.config.reload_failed` | A reload was refused. The old configuration stays.        |

`reload` has `path`, `revision`, `warnings`, `notApplied`, `sequence` and `time`. `failure` has `path`, `error`, `sequence`
and `time`.

- `path` is the configuration file the reload read.
- `revision` is the number the compositor gave the committed document.
- `warnings` lists the problems of the sections the reload dropped, each a string. It is empty when every section
  validated.
- `notApplied` lists the settings accepted without effect, each a sentence naming the setting and the reason:
  `restart required` for a setting a process reads only when it starts, and `no effect` for one nothing reads.
- `error` is why a reload was refused: the first validation or evaluation failure.
- `sequence` counts accepted reloads in `onConfigReloaded` and refused reloads in `onConfigReloadFailed`. `time` is the
  shell's monotonic clock reading. Neither is a wall clock.

## Operations

| Event                         | Fields                                                     | When                                                            |
| ----------------------------- | ---------------------------------------------------------- | --------------------------------------------------------------- |
| `gnoblin.operation.completed` | `operationId`, `method`, `ok`, `error`, `sequence`, `time` | An operation finished: the compositor or the shell answered it. |

`ok` is `true` for a success. `error` is the failure code, empty on success. A script waits for one operation through
its own `Operation`: see [Operations, errors and timers](/shell-api/errors).

## Requests behind dialogs

| Event                      | Fields                                                       | When                                        |
| -------------------------- | ------------------------------------------------------------ | ------------------------------------------- |
| `gnoblin.request.offered`  | `request`, `deadline`, `slot`, `title`, `message`, `subject` | The compositor has a dialog to open.        |
| `gnoblin.request.finished` | `request`, `completion`, `reason`                            | The request ended. Raised once per request. |

`request` is the request's identity, the same in both events. `slot` names the kind of request. `subject` is text the
asking program chose and is not an identity to trust. The event never carries an answer.

To watch these requests with typed objects, and to refuse one, attach a listener:
[Requests and dialogs](/shell-api/requests) and [Authentication prompts](/shell-api/authentication).
