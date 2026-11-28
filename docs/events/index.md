# Events

A shell hears the compositor through listeners. A listener is an object with optional methods, one for each event it
wants. You attach it to the object the event is about, and the compositor calls the matching method when the event
happens. Each method receives the object first and the event's own values after it.

Each language writes a listener in its own way:

- **TypeScript:** a class that extends a listener class from `gnoblin-core`, attached with `addListener`.
- **Python:** a class that subclasses a listener class from `gnoblin`, attached with `add_listener`.
- **Rust:** a struct of closures, built with `implement` and attached with `add_listener`, or a closure registered
  directly with an `on_*` method.

A listener that counts terminal bells, in each language:

::: tabs

```typescript
import { Gnoblin, SessionListener } from "gnoblin-core";
import type { Session } from "gnoblin-core";
import { Toast } from "gnoblin-std";

class Bell extends SessionListener {
    onBell(_session: Session, count: number) {
        Toast.show(`Bell number ${count}`);
    }
}

Gnoblin.get().session.addListener(new Bell());
```

```python
from gnoblin import Gnoblin, SessionListener
from gnoblin.std import Toast


class Bell(SessionListener):
    def on_bell(self, session, count):
        Toast.show(f"Bell number {count}")


Gnoblin.get().session.add_listener(Bell())
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_std::Toast;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let session = Gnoblin::get(&g.objects()?)?.session()?;
    let toasts = g.objects()?.std()?;
    session
        .on_bell(move |_session, count| {
            let started = std::time::Instant::now();
            let _ = Toast::show(&toasts, &format!("Bell number {count}"), None, None, None, Some(move || Ok(started.elapsed().as_secs_f64() * 1000.0)));
        })?
        .keep();
    Ok(())
}
```

:::

The Rust example uses `on_bell`, a closure registered on the session. It returns a `Listener`, which stops listening when
it is dropped, so the example calls `keep()` to hold it for as long as the shell runs.

The Rust `SessionListenerHandlers` struct is the alternative when one listener must answer several events at once: it
has a field for every method, and `implement` turns it into a listener.

Put the TypeScript file in `~/.config/gnoblin/shell.ts`, the Python file in `~/.config/gnoblin/shell.py`, and the Rust
code in the `src/lib.rs` of a shell library. A reload removes every listener the old file attached, so the new file
attaches its own and nothing runs twice.

## The listener classes

Every object that emits events has a listener class. Attach one with `addListener` (TypeScript), `add_listener`
(Python) or `add_listener(&listener)` (Rust), and detach it with the matching `removeListener` or `remove_listener`.

| Listener                   | Attached to                            | It hears                                               |
| -------------------------- | -------------------------------------- | ------------------------------------------------------ |
| `ClientListener`           | a window                               | Its title, focus, geometry, pointer and close.         |
| `ClientCollectionListener` | a window collection, `gnoblin.clients` | Windows joining or leaving, and each member's changes. |
| `OutputListener`           | an output                              | Its connection, mode, scale and applied configuration. |
| `OutputCollectionListener` | the output collection                  | Outputs joining or leaving, and each member's changes. |
| `SessionListener`          | the session                            | Ready, bell, reload requested and lock state.          |
| `RuntimeListener`          | the runtime                            | A configuration reload that was accepted or refused.   |
| `ProcessListener`          | the process collection                 | A process that ended.                                  |

- Every method of a listener is optional. A listener defines only the ones it needs.
- Adding the same listener twice has no effect.
- Removing a listener that is not attached is ignored.
- A method that throws is reported, and the other listeners still run.

## A listener for windows

A `ClientCollectionListener` hears every window the collection holds, including the ones already open when it is
attached. Its `on_added` method runs for each one, and `on_title_changed` runs when a title changes:

::: tabs

```typescript
import { Gnoblin, ClientCollectionListener } from "gnoblin-core";
import type { Client, ClientCollection } from "gnoblin-core";

class Titles extends ClientCollectionListener {
    onAdded(client: Client, _collection: ClientCollection) {
        console.log(`window ${client.appId} opened`);
    }

    onTitleChanged(client: Client, title: string) {
        console.log(`window ${client.appId} is now ${title}`);
    }
}

Gnoblin.get().clients.addListener(new Titles());
```

```python
from gnoblin import ClientCollectionListener, Gnoblin


class Titles(ClientCollectionListener):
    def on_added(self, client, collection):
        print(f"window {client.app_id} opened")

    def on_title_changed(self, client, title):
        print(f"window {client.app_id} is now {title}")


Gnoblin.get().clients.add_listener(Titles())
```

```rust
use gnoblin_core::Gnoblin;

fn watch(g: &Gnoblin) -> gnoblin_core::Res<()> {
    let clients = Gnoblin::get(&g.objects()?)?.clients()?;
    clients
        .on_added(|client, _collection| {
            println!("window {} opened", client.app_id().unwrap_or_default());
        })?
        .keep();
    clients
        .on_title_changed(|client, title| {
            println!("window {} is now {title}", client.app_id().unwrap_or_default());
        })?
        .keep();
    Ok(())
}
```

:::

## Subscribe to one event

A listener is the right tool when one object answers several events. When a script needs one event only, each object
also has a method that subscribes to that single event:

::: tabs

```typescript
import { Gnoblin } from "gnoblin-core";

const gnoblin = Gnoblin.get();

gnoblin.session.on("bell", (_session, count) => {
    gnoblin.spawn(["notify-send", `Bell number ${count}`]);
});
```

```python
from gnoblin import Gnoblin

gnoblin = Gnoblin.get()


def on_bell(_session, count):
    gnoblin.spawn(["notify-send", f"Bell number {count}"])


gnoblin.session.on("bell", on_bell)
```

```rust
use gnoblin_core::Gnoblin;

fn watch(g: &Gnoblin) -> gnoblin_core::Res<()> {
    let objects = g.objects()?;
    let session = Gnoblin::get(&objects)?.session()?;
    session.on_bell(|_session, count| println!("Bell number {count}"))?.keep();
    Ok(())
}
```

:::

Each subscription call returns a handle that keeps it alive until it is stopped, or until a reload removes it. A reload
removes every subscription the old file made.

## Which events exist

- [Lifecycle events](lifecycle): the session, the configuration, finished operations and dialog requests.
- [Window, workspace and output events](desktop-state): what happens to the things on screen.

Seat chords and gestures have no listener. A shortcut is a binding that runs your function: see
[Keyboard shortcuts](/shell-api/keyboard-shortcuts).
