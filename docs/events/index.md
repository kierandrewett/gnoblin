# Events

A shell hears the compositor through listeners. A listener is an object with optional methods, one for each event it
wants. You attach it to the object the event is about, and the compositor calls the matching method when the event
happens. Each method receives the object first and the event's own values after it.

Each language writes a listener in its own way:

- **TypeScript:** a class that extends a listener class from `gnoblin-core`, attached with `addListener`.
- **Python:** a class that subclasses a listener class from `gnoblin.objects.model`, attached with `add_listener`.
- **Rust:** a struct of closures, built with `implement` and attached with `add_listener`, or a closure registered
  directly with an `on_*` method.

A listener that counts terminal bells, in each language:

::: tabs

```typescript
import { Session, SessionListener } from "gnoblin-core";
import { Toast } from "gnoblin-std";

class Bell extends SessionListener {
    onBell(_session: Session, count: number) {
        Toast.show(`Bell number ${count}`);
    }
}

gnoblin.session.addListener(new Bell());
```

```python
from gnoblin.objects.model import Gnoblin, SessionListener
from gnoblin.objects.std import Toast


class Bell(SessionListener):
    def on_bell(self, session, count):
        Toast.show(f"Bell number {count}")


Gnoblin.get().session.add_listener(Bell())
```

```rust
use std::time::Instant;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::Toast;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let Some(session) = ModelRoot::get(&objects)?.session()? else {
        return Ok(());
    };
    let toasts = objects.clone();
    let bell = session.on_bell(move |_session, count| {
        let started = Instant::now();
        let clock = move || Ok(started.elapsed().as_secs_f64() * 1000.0);
        let _ = Toast::show(
            &toasts,
            &format!("Bell number {count}"),
            Some(20.0),
            Some(400.0),
            Some(20.0),
            Some(clock),
        );
    })?;
    // Dropping the Listener stops the subscription, so the shell keeps it for as long as it runs.
    std::mem::forget(bell);
    Ok(())
}
```

:::

The Rust example uses `on_bell`, a closure registered on the session. It returns a `Listener`, which stops listening when
it is dropped, so the example keeps it. The Rust `SessionListenerHandlers` struct is the alternative when one listener
must answer several events at once.

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
import { Client, ClientCollection, ClientCollectionListener } from "gnoblin-core";

class Titles extends ClientCollectionListener {
    onAdded(client: Client, _collection: ClientCollection) {
        console.log(`window ${client.appId} opened`);
    }

    onTitleChanged(client: Client, title: string) {
        console.log(`window ${client.appId} is now ${title}`);
    }
}

gnoblin.clients.addListener(new Titles());
```

```python
from gnoblin.objects.model import ClientCollectionListener, Gnoblin


class Titles(ClientCollectionListener):
    def on_added(self, client, collection):
        print(f"window {client.app_id} opened")

    def on_title_changed(self, client, title):
        print(f"window {client.app_id} is now {title}")


Gnoblin.get().clients.add_listener(Titles())
```

```rust
use gnoblin_core::objects::{Client, ClientCollection, ClientCollectionListenerHandlers};
use gnoblin_core::Gnoblin;

fn watch(g: &Gnoblin) -> gnoblin_core::Result<()> {
    let objects = g.objects()?;
    let Some(clients) = g.clients()? else {
        return Ok(());
    };
    let titles = ClientCollectionListenerHandlers {
        on_added: Some(Box::new(move |client: Client, _collection: ClientCollection| {
            println!("window {} opened", client.app_id()?);
            Ok(())
        })),
        on_title_changed: Some(Box::new(move |client: Client, title: String| {
            println!("window {} is now {title}", client.app_id()?);
            Ok(())
        })),
        ..Default::default()
    }
    .implement(&objects)?;
    clients.add_listener(&titles)?;
    // The core holds the listener from here on.
    std::mem::forget(titles);
    Ok(())
}
```

:::

## Subscribe to one event

A listener is the right tool when one object answers several events. When a script needs one event only, each object
also has a method that subscribes to that single event:

::: tabs

```typescript
import { verbs } from "gnoblin-core/generated/verbs";

verbs.session.onBell((count) => {
    gnoblin.spawn(["notify-send", `Bell number ${count}`]);
});

gnoblin.onClientCreated((client) => {});
```

```python
from gnoblin import verbs
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()

verbs.session.on_bell(lambda session, count: gnoblin.spawn(["notify-send", f"Bell number {count}"]))
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_core::Gnoblin;

fn watch(g: &Gnoblin) -> gnoblin_core::Result<()> {
    let objects = g.objects()?;
    if let Some(session) = ModelRoot::get(&objects)?.session()? {
        let bell = session.on_bell(|_session, count| {
            println!("bell number {count}");
        })?;
        std::mem::forget(bell);
    }
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
