# Session and runtime

`gnoblin.session` reads the session's state and asks it to log out, suspend, power off or lock. `gnoblin.runtime` asks the
shell to reload its configuration. `gnoblin.version()` names the build.

::: tabs

```typescript
const input = gnoblin.input.primary;
input.bind("Super+l", () => {
    gnoblin.session.lock();
});
input.bind("Super+Shift+r", () => {
    gnoblin.runtime.reloadConfig();
});
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()


def lock(event):
    gnoblin.session.lock()


def reload_config(event):
    gnoblin.runtime.reload_config()


input = gnoblin.input.primary
input.bind("Super+l", lock)
input.bind("Super+Shift+r", reload_config)
```

```rust
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = Rc::new(ModelRoot::get(&g.objects()?)?);
    let Some(input) = model.input()? else {
        return Ok(());
    };
    let Some(context) = input.primary()? else {
        return Ok(());
    };
    let locker = Rc::clone(&model);
    context.bind(
        "Super+l",
        Some(move |_event| {
            if let Some(session) = locker.session()? {
                session.lock()?;
            }
            Ok(true)
        }),
        None,
    )?;
    let reloader = Rc::clone(&model);
    context.bind(
        "Super+Shift+r",
        Some(move |_event| {
            if let Some(runtime) = reloader.runtime()? {
                runtime.reload_config()?;
            }
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

Put the TypeScript in `~/.config/gnoblin/shell.ts` and the Python in `~/.config/gnoblin/shell.py`. The Rust is a shell
library, which [Write a shell in Rust](/rust-shell) builds and runs. `Super+l` locks the session. `Super+Shift+r` reloads
the configuration, as `gnoblinctl config reload` does.

## Session

`gnoblin.session` talks to the compositor over a connection the shell holds for it. With no connection the status is
`unavailable` and every verb fails with the code `unavailable`.

| Member        | Returns         | Meaning                                                               |
| ------------- | --------------- | --------------------------------------------------------------------- |
| `status`      | `SessionStatus` | The session as the compositor last published it. Read on each access. |
| `logout()`    | `Operation`     | Ends the session once the compositor confirms.                        |
| `poweroff()`  | `Operation`     | Powers the machine off once the init system accepts. Does not prompt. |
| `reboot()`    | `Operation`     | Reboots, with the contract of `poweroff()`.                           |
| `suspend()`   | `Operation`     | Suspends to RAM. The session locks first.                             |
| `hibernate()` | `Operation`     | Hibernates. Fails with `unsupported` when the machine cannot.         |
| `lock()`      | `Operation`     | Asks the compositor to lock. Read `status` to see the locked state.   |

Each verb returns an `Operation`: see [Operations, errors and timers](errors). A success of `lock` says the request went out
and does not prove the session is locked.

### Session status

`status` holds these fields:

| Field           | Values                                       | Meaning                                                            |
| --------------- | -------------------------------------------- | ------------------------------------------------------------------ |
| `state`         | `running`, `unavailable`                     | `unavailable` means the shell has no connection to the compositor. |
| `lockAvailable` | boolean                                      | Whether the compositor has published a lock state yet.             |
| `revision`      | number                                       | How many lock changes the session published. Starts at 0.          |
| `lockState`     | `unlocked`, `covering`, `locked`, `failsafe` | Present only while `lockAvailable` is `true`.                      |

The lock states are explained in [Lifecycle events](/events/lifecycle#session). Only `locked` confirms the lock took.
Unavailable does not mean unlocked.

The tables use TypeScript camelCase spellings. The same members in the other languages are:

- Python: snake_case, such as `lock_available`, `last_reload` and `reload_config()`. `last_reload` is a property, and `status()` and `reload_config()` are methods.
- Rust: snake_case, with a method for each member. Every call returns a `Result`, such as `session.status()?` and `runtime.reload_config()?`.
- Fields of a Rust `SessionStatus` are `state`, `lock_available`, `revision` and `lock_state`; a `ReloadResult` has `settings_revision`, `warnings` and `not_applied`.

### When a power verb fails

A refused power verb fails with one of these codes, and the message names the D-Bus error:

- `denied`: logind or polkit refused.
- `unsupported`: the machine cannot do it, or logind has no such action.
- `busy`: logind is already running another power action.
- `unavailable`: logind is not on the system bus.
- `timed_out`: logind did not answer in time.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core/generated/objects";

gnoblin.session.suspend().catch((error, code) => {
    gnoblin.spawn(["notify-send", `Suspend failed: ${code}: ${error}`]);
});
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()


def notify_failure(error, code):
    gnoblin.spawn(["notify-send", f"Suspend failed: {code}: {error}"])


gnoblin.session.suspend().catch(notify_failure)
```

```rust
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = Rc::new(ModelRoot::get(&g.objects()?)?);
    let Some(session) = model.session()? else {
        return Ok(());
    };
    let notifier = Rc::clone(&model);
    session.suspend()?.catch(move |error, code| {
        notifier.spawn(&["notify-send".to_owned(), format!("Suspend failed: {code}: {error}")], None)?;
        Ok(())
    })?;
    Ok(())
}
```

:::

## Reload

`reloadConfig()` (`reload_config()` in Python and Rust) asks for the reload `gnoblinctl config reload` asks for. It returns an
`Operation`, and a success leaves a `ReloadResult` in `lastReload` (`last_reload` in Python and Rust) with these fields:

| Field              | Meaning                                                                                                     |
| ------------------ | ----------------------------------------------------------------------------------------------------------- |
| `settingsRevision` | The revision the compositor committed.                                                                      |
| `warnings`         | The problems of the sections the reload dropped.                                                            |
| `notApplied`       | Settings accepted without effect, each a sentence. See [Lifecycle events](/events/lifecycle#configuration). |

The reload is refused while the configuration loads, because every reload would ask for another. When the document is
refused the operation fails with the code `config_rejected` and the reason, and the running configuration stays to hear
it. When the reload succeeds, the new configuration replaces the one that asked, so the asking configuration's callbacks do
not run.

## Version

`version()` returns a frozen record that names the build. Call it as `gnoblin.version()` in TypeScript and Python, and as
`version()` on the Rust object, which returns a `Result`.

| Field                                                                                     | Meaning                                                    |
| ----------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| `gnoblin`, `gnome`, `mutter`                                                              | The versions the build reports.                            |
| `js`                                                                                      | The version of the script runtime.                         |
| `api`                                                                                     | The API version of the shell interface.                    |
| `gitRemote`, `gitSha`, `buildId` (`git_remote`, `git_sha`, `build_id` in Python and Rust) | Where the build came from. A remote loses its credentials. |

A field the build does not know is `"unknown"`. The values come from a `version.ini` file, and the first file the shell
can read wins:

1. The file `GNOBLIN_VERSION_METADATA_FILE` names, when it is set.
2. `share/gnoblin/version.ini` under the installation prefix of the running executable.
3. `gnoblin/version.ini` in each directory of `XDG_DATA_DIRS`.
