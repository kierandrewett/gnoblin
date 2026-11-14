# Session and runtime

`gnoblin.session` reads the session's state and asks it to log out, suspend, power off or lock. `gnoblin.runtime` asks the
shell to reload its configuration. `gnoblin.version()` names the build.

```ts
import { defineConfig } from "gnoblin";

defineConfig({
    keys: {
        "Super+l": ({ gnoblin }) => {
            gnoblin.session.lock();
        },
        "Super+Shift+r": ({ gnoblin }) => {
            gnoblin.runtime.reloadConfig();
        },
    },
});
```

Put the file in `~/.config/gnoblin/init.ts`. `Super+l` locks the session. `Super+Shift+r` reloads the configuration, as
`gnoblinctl config reload` does.

## Session

`gnoblin.session` talks to the compositor over a connection the shell holds for it. With no connection the status is
`unavailable` and every verb fails with the code `unavailable`.

| Member        | Returns                  | Meaning                                                               |
| ------------- | ------------------------ | --------------------------------------------------------------------- |
| `status()`    | `SessionStatus`          | The session as the compositor last published it. Read on each call.   |
| `logout()`    | `Operation<Accepted>`    | Ends the session once the compositor confirms.                        |
| `poweroff()`  | `Operation<Accepted>`    | Powers the machine off once the init system accepts. Does not prompt. |
| `reboot()`    | `Operation<Accepted>`    | Reboots, with the contract of `poweroff()`.                           |
| `suspend()`   | `Operation<Accepted>`    | Suspends to RAM. The session locks first.                             |
| `hibernate()` | `Operation<Accepted>`    | Hibernates. Fails with `unsupported` when the machine cannot.         |
| `lock()`      | `Operation<LockRequest>` | Asks the compositor to lock. Read `status()` to see the locked state. |

Each verb returns an `Operation`: see [Operations, errors and timers](errors). `logout`, `poweroff`, `reboot`, `suspend` and
`hibernate` succeed with `{ accepted: true }`. `lock` succeeds with `{ dispatched }`, which says the request went out and
does not prove the session is locked.

### Session status

`status()` returns these fields:

| Field           | Values                                       | Meaning                                                            |
| --------------- | -------------------------------------------- | ------------------------------------------------------------------ |
| `state`         | `running`, `unavailable`                     | `unavailable` means the shell has no connection to the compositor. |
| `lockAvailable` | boolean                                      | Whether the compositor has published a lock state yet.             |
| `revision`      | number                                       | How many lock changes the session published. Starts at 0.          |
| `lockState`     | `unlocked`, `covering`, `locked`, `failsafe` | Present only while `lockAvailable` is `true`.                      |

The lock states are explained in [Lifecycle events](/events/lifecycle#session). Only `locked` confirms the lock took.
Unavailable does not mean unlocked.

### When a power verb fails

A refused power verb fails with one of these codes, and the message names the D-Bus error:

- `denied`: logind or polkit refused.
- `unsupported`: the machine cannot do it, or logind has no such action.
- `busy`: logind is already running another power action.
- `unavailable`: logind is not on the system bus.
- `timed_out`: logind did not answer in time.

```ts
gnoblin.session.suspend().onComplete((value, error) => {
    if (error) {
        gnoblin.spawn(["notify-send", `Suspend failed: ${error.code}`]);
    }
});
```

## Reload

`gnoblin.runtime.reloadConfig()` asks for the reload `gnoblinctl config reload` asks for. It returns an
`Operation<ReloadResult>`. A success carries these fields:

| Field              | Meaning                                                                                                     |
| ------------------ | ----------------------------------------------------------------------------------------------------------- |
| `ok`               | Always `true` in a success.                                                                                 |
| `action`           | `"config reload"`.                                                                                          |
| `settingsRevision` | The revision the compositor committed.                                                                      |
| `warnings`         | The problems of the sections the reload dropped.                                                            |
| `notApplied`       | Settings accepted without effect, each a sentence. See [Lifecycle events](/events/lifecycle#configuration). |

`reloadConfig` throws while the configuration loads, because every reload would ask for another. When the document is
refused the operation fails with the code `config_rejected` and the reason, and the running configuration stays to hear
it. When the reload succeeds, the new configuration replaces the one that asked, so the asking configuration's callbacks do
not run.

## Version

`gnoblin.version()` returns a frozen record that names the build.

| Field                            | Meaning                                                    |
| -------------------------------- | ---------------------------------------------------------- |
| `gnoblin`, `gnome`, `mutter`     | The versions the build reports.                            |
| `js`                             | The version of the script runtime.                         |
| `api`                            | The API version of the shell interface.                    |
| `gitRemote`, `gitSha`, `buildId` | Where the build came from. A remote loses its credentials. |

A field the build does not know is `"unknown"`. The values come from a `version.ini` file, and the first file the shell
can read wins:

1. The file `GNOBLIN_VERSION_METADATA_FILE` names, when it is set.
2. `share/gnoblin/version.ini` under the installation prefix of the running executable.
3. `gnoblin/version.ini` in each directory of `XDG_DATA_DIRS`.
