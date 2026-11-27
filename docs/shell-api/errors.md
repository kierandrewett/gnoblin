# Operations, errors and timers

A call that finishes later returns an `Operation`. A call that cannot be made at all throws. A function you hand to the
shell, whether a listener, a handler or a timer, runs under a time budget and may fail without stopping the others.

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core/generated/objects";

const gnoblin = Gnoblin.get();

gnoblin.input.primary.bind("Super+Shift+q", () => {
    const operation = gnoblin.session.logout();
    operation.on("settled", () => {
        if (operation.error !== "") {
            gnoblin.spawn(["notify-send", `Logout refused: ${operation.errorCode}: ${operation.error}`]);
        }
    });
});
```

```python
from gnoblin.objects.model import BindingEvent, Gnoblin

gnoblin = Gnoblin.get()


def log_out(event: BindingEvent) -> None:
    operation = gnoblin.session.logout()

    @operation.on("settled")
    def settled(*_: object) -> None:
        if operation.error != "":
            gnoblin.spawn(["notify-send", f"Logout refused: {operation.error_code}: {operation.error}"])


gnoblin.input.primary.bind("Super+Shift+q", log_out)
```

```rust
use std::rc::Rc;

use gnoblin_core::objects::ModelRoot;
use gnoblin_core::{self as gnoblin, Gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = Rc::new(ModelRoot::get(&g.objects()?)?);
    let input = model.input()?;
    let session = model.session()?;
    let context = input.primary()?;
    context.bind(
        "Super+Shift+q",
        Some(move |_event| {
            let reporter = Rc::clone(&model);
            session.logout()?.catch(move |error, code| {
                let text = format!("Logout refused: {code}: {error}");
                reporter.spawn(&["notify-send".to_owned(), text], None)?;
                Ok(())
            })?;
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

The shell file is `~/.config/gnoblin/shell.ts` in TypeScript, `~/.config/gnoblin/shell.py` in Python, or a Rust shell library in
Rust, which [Write a shell in Rust](/rust-shell) builds and runs. `Super+Shift+q` logs out, and if the logout is refused it shows a
notification.

## Operations

An `Operation` settles once, with success or with a typed error. Its callbacks run as queued calls under the same time
budget as every listener: one that throws is reported, and the next callback still runs.

| Member                                    | Meaning                                                                                                                                  |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `status`                                  | `pending`, `succeeded` or `failed`.                                                                                                      |
| `error`                                   | The words of a failure. Empty otherwise.                                                                                                 |
| `errorCode`, `error_code`, `error_code()` | The code of a failure, from the table below: `errorCode` in TypeScript, `error_code` in Python, `error_code()` in Rust. Empty otherwise. |
| `then(fn)`                                | Calls `fn()` once, when the operation succeeds.                                                                                          |
| `catch(fn)`                               | Calls `fn(error, code)` once, when the operation fails.                                                                                  |
| `on("settled", fn)`                       | Calls `fn(operation)` once, when it settles either way. Returns a `Listener`.                                                            |
| `value`                                   | What it succeeded with: none until then, and none for most operations. A string, number, boolean, none or a list. `map` sets it.         |
| `map(fn)`                                 | A new operation: `fn(value)` runs when this one succeeds and its answer is the new `value`. A failure passes on; a throw is `internal`.  |
| `timeout(ms)`                             | A new operation that settles as this one does, or fails `timed_out` if this is pending after `ms` (0 to a day). It cancels nothing.      |
| `cancel()`                                | A pending operation fails `cancelled` and this returns `true`; `false` if it had settled. Work already sent still runs, unheard.         |

A call that has a result, such as `Process.wait()`, leaves it on the object that made the operation: read `process.exit`
when it settles (`then`). To stop waiting for an operation after a time, start a timer:

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core/generated/objects";

const gnoblin = Gnoblin.get();
const sleep = gnoblin.process.spawn(["sleep", "30"]);
sleep.wait().then(() => gnoblin.spawn(["notify-send", "sleep ended"]));
gnoblin.timers.after(5000, () => {
    if (sleep.alive) {
        gnoblin.spawn(["notify-send", "sleep took too long"]);
        sleep.kill();
    }
});
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()
sleep = gnoblin.process.spawn(["sleep", "30"])


def ended() -> None:
    gnoblin.spawn(["notify-send", "sleep ended"])


def too_slow() -> None:
    if sleep.alive:
        gnoblin.spawn(["notify-send", "sleep took too long"])
        sleep.kill()


sleep.wait().then(ended)
gnoblin.timers.after(5000.0, too_slow)
```

```rust
use std::rc::Rc;

use gnoblin_core::objects::ModelRoot;
use gnoblin_core::{self as gnoblin, Gnoblin};

fn notify(model: &ModelRoot, text: &str) -> gnoblin::Result<()> {
    model.spawn(&["notify-send".to_owned(), text.to_owned()], None)?;
    Ok(())
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = Rc::new(ModelRoot::get(&g.objects()?)?);
    let processes = model.process()?;
    let timers = model.timers()?;
    let sleep = Rc::new(processes.spawn(&["sleep".to_owned(), "30".to_owned()], None)?);
    let ended = Rc::clone(&model);
    sleep.wait()?.then(move || {
        if let Err(error) = notify(&ended, "sleep ended") {
            g.log(&format!("errors: cannot notify: {error}"));
        }
        Ok(())
    })?;
    let (late, watched) = (Rc::clone(&model), Rc::clone(&sleep));
    timers.after(5000.0, move || {
        if watched.alive()? {
            if let Err(error) = notify(&late, "sleep took too long") {
                g.log(&format!("errors: cannot notify: {error}"));
            }
            watched.kill(None)?;
        }
        Ok(())
    })?;
    Ok(())
}
```

:::

### Following, limiting and cancelling an operation

`timeout(ms)` and `map(fn)` each return a new operation that follows the one they were called on, so a call chains. `cancel()` stops
waiting for an operation.

The example waits for a program to end for 5000 milliseconds, and turns the answer into a text on the way. A failure, whether it is the
program's, the timeout's or the cancel's, reaches `catch` with its code. The value of a `map` is none, a boolean, a number, text or a
list of those: any other answer fails the new operation.

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core/generated/objects";

const gnoblin = Gnoblin.get();
const sleep = gnoblin.process.spawn(["sleep", "30"]);
const label = sleep
    .wait()
    .timeout(5000)
    .map(() => "sleep ended");
label.then(() => gnoblin.spawn(["notify-send", String(label.value)]));
label.catch((error, code) => gnoblin.spawn(["notify-send", `Gave up: ${code}: ${error}`]));
gnoblin.input.primary.bind("Super+Escape", () => {
    label.cancel();
});
```

```python
from gnoblin.objects.model import BindingEvent, Gnoblin

gnoblin = Gnoblin.get()
sleep = gnoblin.process.spawn(["sleep", "30"])
label = sleep.wait().timeout(5000.0).map(lambda _value: "sleep ended")


def ended() -> None:
    gnoblin.spawn(["notify-send", str(label.value)])


def gave_up(error: str, code: str) -> None:
    gnoblin.spawn(["notify-send", f"Gave up: {code}: {error}"])


def stop(event: BindingEvent) -> None:
    label.cancel()


label.then(ended)
label.catch(gave_up)
gnoblin.input.primary.bind("Super+Escape", stop)
```

```rust
use std::rc::Rc;

use gnoblin_core::objects::ModelRoot;
use gnoblin_core::{self as gnoblin, Gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let model = Rc::new(ModelRoot::get(&g.objects()?)?);
    let sleep = model.process()?.spawn(&["sleep".to_owned(), "30".to_owned()], None)?;
    let label = Rc::new(sleep.wait()?.timeout(5000.0)?.map(|_value| Ok("sleep ended".into()))?);
    let (ended, answer) = (Rc::clone(&model), Rc::clone(&label));
    label.then(move || {
        let text = format!("{:?}", answer.value()?);
        ended.spawn(&["notify-send".to_owned(), text], None)?;
        Ok(())
    })?;
    let gave_up = Rc::clone(&model);
    label.catch(move |error, code| {
        gave_up.spawn(&["notify-send".to_owned(), format!("Gave up: {code}: {error}")], None)?;
        Ok(())
    })?;
    let stop = Rc::clone(&label);
    model.input()?.primary()?.bind(
        "Super+Escape",
        Some(move |_event| {
            stop.cancel()?;
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

`timeout` does not cancel the operation it waits on: a request already sent still happens and its result is ignored. `cancel` is the
same, except for a request the library can drop, which it drops.

A bad argument is not an operation failure: the call throws a `TypeError` or a `RangeError` at once.

## Error codes

One set of codes covers every failure of the API, and the codes are snake_case.

- An `Operation` that fails carries an `error` and an error code: `errorCode` in TypeScript, `error_code` in Python and `error_code()` in Rust.
- A call that cannot be made throws a `GnoblinError` with a code from the same set.
- Its `target` names the object and member, such as `Client#7.x`, and its message begins with the target.

| Code               | Meaning                                                                    |
| ------------------ | -------------------------------------------------------------------------- |
| `invalid_argument` | An argument is invalid.                                                    |
| `not_found`        | The object or resource does not exist, or has ended.                       |
| `already_exists`   | The id, name or trigger is taken.                                          |
| `invalid_state`    | The object is not in a state that allows the call, such as a detached one. |
| `read_only`        | The property or collection cannot be written.                              |
| `unsupported`      | The compositor or the machine cannot perform the operation.                |
| `denied`           | Session policy, logind, polkit or the compositor rejected the operation.   |
| `busy`             | Something exclusive is already running or in use.                          |
| `cancelled`        | `cancel()` or the caller stopped the operation.                            |
| `timed_out`        | The operation reached its deadline.                                        |
| `unavailable`      | The compositor or a session service is not ready, or there is no seat.     |
| `internal`         | An unexpected error occurred, such as a throw inside `map`.                |
| `config_rejected`  | The configuration document refused a value, or a reload was refused.       |

A `TypeError` or a `RangeError` is the script's own mistake in a value, and it has no code.

A call the library refuses is a `GnoblinError`, so a script can branch on its code. The import differs by language:

- TypeScript imports `GnoblinError` from `gnoblin-core`.
- Python imports it from `gnoblin`.

Rust has no string code: a failed call returns an error that says what failed, and the example reports it as text:

::: tabs

```typescript
import { GnoblinError } from "gnoblin-core/generated/objects";
import { Workspace, Workspaces } from "gnoblin-std/generated/objects";

try {
    Workspaces.get().add(new Workspace("main"));
} catch (error) {
    if (error instanceof GnoblinError && error.code === "already_exists") {
        gnoblin.spawn(["notify-send", "There is a workspace with that id already"]);
    }
}
```

```python
from gnoblin import GnoblinError
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import Workspace, Workspaces

gnoblin = Gnoblin.get()

try:
    Workspaces.get().add(Workspace("main"))
except GnoblinError as error:
    if error.code == "already_exists":
        gnoblin.spawn(["notify-send", "There is a workspace with that id already"])
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{Workspace, WorkspaceOptions};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let objects = g.std()?.objects()?;
    let workspace = Workspace::new_with(&objects, "main", WorkspaceOptions::default())?;
    if let Err(fail) = model.workspaces()?.add(&workspace) {
        model.spawn(
            &["notify-send".to_owned(), format!("Cannot add the workspace: {fail}")],
            None,
        )?;
    }
    Ok(())
}
```

:::

The shell file is `~/.config/gnoblin/shell.ts` in TypeScript, `~/.config/gnoblin/shell.py` in Python, or a Rust shell library in
Rust. A second workspace with the id `main` shows a notification instead of stopping the file.

### Which failure carries which code

A call on an object of the library, such as `Client`, `InputContext` or a std widget, fails with the code that says why. The shell
sets it where it refuses the call.

| Failure                                                                 | Code                            |
| ----------------------------------------------------------------------- | ------------------------------- |
| A window or object that has closed, or a name the object does not have  | `not_found`                     |
| A second object with an id, name or trigger that is taken               | `already_exists`                |
| An object not added yet, or a transition that is not being staged       | `invalid_state`                 |
| A property or a collection that cannot be written                       | `read_only`                     |
| A member the library names and does not implement, or cannot animate    | `unsupported`                   |
| A request the compositor or the host refuses with no more specific word | `denied`                        |
| A single holder, or an object in use by a call that is running          | `busy`                          |
| No seat, or an owner that does not answer                               | `unavailable`                   |
| A fault of the shell itself, or a panic it caught                       | `internal`                      |
| A generation a reload has replaced                                      | `invalid_state`                 |
| A value of the wrong kind or out of range                               | none: `TypeError`, `RangeError` |

An object that holds state of its own is in use while a call about it runs. What a callback of that call may do depends on the
call:

- A call that changes the state, such as setting `Workspace.layout`: the callback cannot call the object back. It gets `busy`, and
  the call goes on.
- A call that only reads the state, or a property read: the callback may read the object's properties and call its other members
  that only read it.

For example, a layout's `findSplit` or `setSplit` may read the `Workspace` that runs it, and an entry's `run` may read the
`menu.client` of the `WindowMenu` that runs it.

The members the std library adds to `gnoblin` hold no state, so they are never in use. The callback of `gnoblin.requestTransition` may
read `gnoblin.workspaces` or `gnoblin.displays`.

`cancelled` comes from `Operation.cancel()`, `timed_out` from `Operation.timeout()` and from logind, and `config_rejected` from the
configuration. A call on an object does not raise them.

The `target` is `Class#id.member`, such as `Client#7.close`, where `id` is the object's `id` property. A class function, a
constructor or an object with no readable `id` gives `Class.member`, such as `InputContext.new`. The message begins with the
target.

### Member names in a refusal

A refusal names the member it refused, in the spelling of the language that made the call. The library words it with the schema's
name (`compositor_settings.user_shaders`), and each generated SDK writes the member as its own class spells it:

- TypeScript: `` `CompositorSettings.userShaders` must be a boolean, got text ``, with the camelCase member and the parameter or the
  record field the refusal names (`` `DwindleLayout.splitRatio`: `amount` ``).
- Python: `` `CompositorSettings.user_shaders` must be a boolean, got text ``, with the attribute the Python class has, so a member
  that is a keyword reads `from_`.
- Rust: the schema's name stays, as `Fail` carries it, and the compositor's log words it the same way.

The target of a failure (`Keyboard#1.repeatRate` in TypeScript, `Keyboard#1.repeat_rate` in Python and Rust) and the words after it
follow the same spelling. A name the generated class does not know is left as the library wrote it.

### A shell that read the old failure

Five things about a failed call of an object of the library read differently from before the codes. A shell written for the old reading changes as follows.

- `error.code` is the word, such as `"not_found"`. It was the ABI's integer, such as `-6`, on a plain `Error`. In TypeScript and Python, compare the code with a word after checking the failure is a `GnoblinError`. Rust has no string code.
- The message does not start with `E_TYPE: ` or another ABI symbol, as it did. A `TypeError` says what was wrong in the words of the library. A `GnoblinError` begins with its `target`.
- A write to a property or a collection that cannot be written is a `GnoblinError` with the code `read_only`, in TypeScript and in Python. It was a `TypeError`. A bad argument is still a `TypeError`.
- A timed write to a property that cannot animate is a `GnoblinError` with the code `unsupported`. It was a `TypeError`. A generated class has a timed setter only for a property that can animate, so only a direct call of the native module meets this.
- `Display.viewport` on a display that already shows a viewport is a `GnoblinError` with the code `busy`. It was a `TypeError`.

The ABI integer is not part of the script API. A shell that needs it reads `abi` on the `Error` a call of the native module throws, which a generated class never lets out.

## Timers

`gnoblin.timers.after(ms, fn)` runs `fn` once, `ms` milliseconds from now, at the first frame after that time. `ms` is from
0 to a day. It returns a `Timer`.

| Member                 | Meaning                                                              |
| ---------------------- | -------------------------------------------------------------------- |
| `timer.pending`        | `true` until the timer ran or was cancelled.                         |
| `timer.cancel()`       | Stops the timer. Nothing happens when it had already run.            |
| `gnoblin.timers.now()` | The monotonic clock in milliseconds. Only differences mean anything. |

- A timer that throws is reported, and the next one still runs.
- A reload cancels the timers of the old configuration, and they do not delay it.
- At most 1024 timers a shell made are pending at once. The 1025th `after` is refused with a `GnoblinError` of code `busy`, which says
  how many are pending; a timer that ran or was cancelled makes room.

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core/generated/objects";
import { Workspaces } from "gnoblin-std/generated/objects";

const gnoblin = Gnoblin.get();
const timer = gnoblin.timers.after(2000, () => Workspaces.get().showById("2"));
timer.cancel();
```

```python
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import Workspaces

gnoblin = Gnoblin.get()


def show_second() -> None:
    Workspaces.get().show_by_id("2")


timer = gnoblin.timers.after(2000.0, show_second)
timer.cancel()
```

```rust
use std::rc::Rc;

use gnoblin_core::objects::ModelRoot;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let timers = model.timers()?;
    let workspaces = Rc::new(model.workspaces()?);
    let timer = timers.after(2000.0, move || {
        workspaces.show_by_id("2", None)?;
        Ok(())
    })?;
    timer.cancel()?;
    Ok(())
}
```

:::

## Handler errors

A listener, a key handler, an event handler or a timer that throws is reported once per site, with the file, line and column of
the throw in your `.ts` source. The shell keeps the handler attached and goes on to the next one.

A call that runs past its time budget is interrupted, and the shell stages your config again from the files. After several
interruptions in a short time it stops doing that until a cool-down has passed or you reload on purpose. The compositor
keeps running throughout.

### Where a failure is reported

A failure of a handler, and the sentences the libraries have for you, are **notices**: warnings that say what failed or what was
not applied. The core says a notice once per site, and the shell keeps it in its diagnostics, which it writes to the log as a
warning.

- A site is the place a handler threw, so a handler that fails on every event is reported on the first and not again.
- A different message from the same place is not a new site.
- A failure that carries no stack is a site of its own text.

The handler here throws on every press:

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core/generated/objects";

const gnoblin = Gnoblin.get();

gnoblin.input.primary.bind("Super+Shift+q", () => {
    throw new Error("not wired up yet");
});
```

```python
from gnoblin.objects.model import BindingEvent, Gnoblin

gnoblin = Gnoblin.get()


def broken(event: BindingEvent) -> None:
    raise RuntimeError("not wired up yet")


gnoblin.input.primary.bind("Super+Shift+q", broken)
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_core::{self as gnoblin, Fail, Gnoblin};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let context = ModelRoot::get(&g.objects()?)?.input()?.primary()?;
    context.bind("Super+Shift+q", Some(move |_event| Err(Fail::err("not wired up yet"))))?;
    Ok(())
}
```

:::

Read the log with `gnoblinctl log --follow`. A TypeScript shell reports the first press as one warning that names the handler and
carries the stack in your `.ts` source, and the later presses add nothing.

A TypeScript shell also says what a library did not apply, once per reload, in the same place: `not applied: style.cornerRadius: ...`
for a style, `not applied: OutputProfile "BOE/Panel/SN42" vrr ...` for a profile, and a compositor's refusal of a profile's request as
`outputs.profiles.<connector>.<key>: <reason>`. A shell in another language does not take the notices, so the core writes the first
of each site to its own log as a warning.

### Time budgets

The budget counts the CPU time your code uses, not the time it takes on the clock, so a busy machine does not make a
well-behaved handler fail. The limits are:

- 1000 ms for a listener, handler or timer call.
- 5000 ms for each of the shell's own calls in a frame.
- 30000 ms for loading a config.

A call that is held up without computing is stopped when it has waited eight times its limit on the wall clock.

The message says which clock stopped the call, the limit, how much of each clock the call used and where in your code it
was:

```text
the script used 1003 ms of CPU time (1010 ms of wall time) against its 1000 ms CPU budget and was interrupted;
it was in at spin (shell.ts:12:5), called from at onFocus (shell.ts:20:3)
```

## See also

- [Events](/events/) for the listeners.
- [Compositor bridge](/bridge/connection) for the socket's own error codes.
