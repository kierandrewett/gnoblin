# Launch feedback

`gnoblin.launches` tracks the busy state between asking for an application and its window, so a panel or launcher can show
a spinner while the program starts.

::: tabs

```typescript
gnoblin.input.primary.bind("Super+e", () => {
    gnoblin.launches.begin("terminal-1", "foot", {
        timeoutMs: 5000,
        listener: {
            onStarted: () => gnoblin.spawn(["notify-send", "The terminal is open"]),
            onTimedOut: () => gnoblin.spawn(["notify-send", "The terminal did not open"]),
        },
    });
    gnoblin.spawn(["foot"]);
});
```

```python
from gnoblin.objects.model import BindingEvent, Gnoblin, Launch, LaunchListener, LaunchOptions

gnoblin = Gnoblin.get()


class Terminal(LaunchListener):
    def on_started(self, launch: Launch) -> None:
        gnoblin.spawn(["notify-send", "The terminal is open"])

    def on_timed_out(self, launch: Launch) -> None:
        gnoblin.spawn(["notify-send", "The terminal did not open"])


def open_terminal(event: BindingEvent) -> None:
    gnoblin.launches.begin("terminal-1", "foot", LaunchOptions(timeout_ms=5000, listener=Terminal()))
    gnoblin.spawn(["foot"])


gnoblin.input.primary.bind("Super+e", open_terminal)
```

```rust
use std::rc::Rc;

use gnoblin_core::objects::{LaunchListenerHandlers, LaunchOptions, ModelRoot};
use gnoblin_core::{self as gnoblin, Gnoblin};

fn notify_argv(text: &str) -> [String; 2] {
    ["notify-send".to_owned(), text.to_owned()]
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = Rc::new(ModelRoot::get(&objects)?);
    let (Some(input), Some(launches)) = (model.input()?, model.launches()?) else {
        return Ok(());
    };
    let Some(context) = input.primary()? else {
        return Ok(());
    };
    let (started, timed_out, opener) = (Rc::clone(&model), Rc::clone(&model), Rc::clone(&model));
    let listener = LaunchListenerHandlers {
        on_changed: None,
        on_started: Some(Box::new(move |_launch| {
            started.spawn(&notify_argv("The terminal is open"), None)?;
            Ok(())
        })),
        on_timed_out: Some(Box::new(move |_launch| {
            timed_out.spawn(&notify_argv("The terminal did not open"), None)?;
            Ok(())
        })),
        on_closed: None,
    }
    .implement(&objects)?;
    let options = LaunchOptions {
        timeout_ms: Some(5000),
        listener: Some(listener),
    };
    context.bind(
        "Super+e",
        Some(move |_event| {
            launches.begin("terminal-1", "foot", Some(&options))?;
            opener.spawn(&["foot".to_owned()], None)?;
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

Put the TypeScript in `~/.config/gnoblin/shell.ts` and the Python in `~/.config/gnoblin/shell.py`. The Rust is a shell
library, which [Write a shell in Rust](/rust-shell) builds and runs. `Super+e` starts the `foot` terminal and tracks the
launch: a notification says whether its window appeared within 5000 ms.

## Begin a launch

`gnoblin.launches.begin(token, application, options?)` returns a `Launch` that is `pending` at once, with `revision` 0.
Anything outside these limits throws:

| Argument      | Accepted values                          | Default | Meaning                                                       |
| ------------- | ---------------------------------------- | ------- | ------------------------------------------------------------- |
| `token`       | non-empty string, at most 128 characters | none    | Names the launch. Beginning the same token again replaces it. |
| `application` | non-empty string, at most 512 characters | none    | The application id the window is matched against.             |
| `timeoutMs`   | integer from 100 to 10000                | `3000`  | How long the launch waits for a window.                       |
| `listener`    | a `LaunchListener`                       | none    | Told of the launch's changes.                                 |

`options` also takes a `listener`, described below. An option it does not know throws.

A launch leaves `pending` once, for one of three states:

| State       | When                                                                                         |
| ----------- | -------------------------------------------------------------------------------------------- |
| `started`   | A window of the application appears, takes focus or names its application id.                |
| `timed_out` | The deadline passes first.                                                                   |
| `ended`     | `launch.finish()` is called, the token is begun again (`replaced`), or the registry is full. |

## The listener

A `LaunchListener` is a class with optional methods, or an object that has some of them.

| Method                      | Called when                                                                                                     |
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
