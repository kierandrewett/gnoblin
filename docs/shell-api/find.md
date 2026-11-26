# Find objects

Two calls select windows, workspaces and outputs. `find` returns an array of the objects that match right now.
`where` on a collection returns a live selection that follows the objects as they change.

::: tabs

```ts
gnoblin.input.primary.bind("Super+f", () => {
    const [notes] = gnoblin.find("Window", { appId: "org.gnome.Notes" });
    notes?.focus();
});
```

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()


def focus_notes(event):
    notes = gnoblin.find("Window", {"app_id": "org.gnome.Notes"})
    if notes:
        notes[0].focus()


gnoblin.input.primary.bind("Super+f", focus_notes)
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::objects::{ClientQuery, ModelRoot, OutputQuery};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let objects = g.objects()?;
    let model = ModelRoot::get(&objects)?;
    let Some(input) = model.input()?.and_then(|input| input.primary().ok().flatten()) else {
        return Ok(());
    };
    input.bind(
        "Super+f",
        Some(move |_event| {
            let query = ClientQuery {
                app_id: Some("org.gnome.Notes".to_owned()),
                ..Default::default()
            };
            if let Some(notes) = model.find_clients(Some(&query))?.first() {
                notes.focus()?;
            }
            Ok(true)
        }),
        None,
    )?;
    Ok(())
}
```

:::

`Super+f` focuses the Notes window when there is one.

## Find objects of a kind

`find(kind, filter?)` returns the live objects of one kind in list order:

- TypeScript and Python: `gnoblin.find`.
- Rust: the typed calls `find_clients` and `find_outputs`, as the examples show.

The objects are the same ones the collections hold, so a `Client` from `find` is the same `Client` you get from the
`clients` collection.

| Argument | Meaning                                                                                     |
| -------- | ------------------------------------------------------------------------------------------- |
| `kind`   | `"Window"`, `"Workspace"` or `"Output"`.                                                    |
| `filter` | Optional. An object of field values, or a function. Without it `find` returns every object. |

An unknown kind is an error that names the kinds that work.

### Filter by field values

An object filter keeps the objects whose fields equal every value in it. Each value is a string, a number or a boolean.
The field names are the same in every language, written in each language's own style: `appId` in TypeScript,
`app_id` in Python and Rust. Both spellings of the field name a single field.

::: tabs

```ts
const focusedNotes = gnoblin.find("Window", { appId: "org.gnome.Notes", focused: true });
const primary = gnoblin.find("Output", { primary: true })[0];
```

```python
focused_notes = gnoblin.find("Window", {"app_id": "org.gnome.Notes", "focused": True})
primary = gnoblin.find("Output", {"primary": True})[0]
```

```rust
let focused_notes = model.find_clients(Some(&ClientQuery {
    app_id: Some("org.gnome.Notes".to_owned()),
    focused: Some(true),
    ..Default::default()
}))?;
let primary = model.find_outputs(Some(&OutputQuery {
    primary: Some(true),
    ..Default::default()
}))?;
```

:::

The fields you can filter on:

| Kind        | Fields                                                                                                                                                                                    |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Window`    | `id`, `appId`, `title`, `alive`, `focused`, `fullscreen`, `secure`, `unresponsive`, `floating`, `maximised`, `minimised`, `tiled`, `visible`, `x`, `y`, `width`, `height`, `opacity`, `z` |
| `Workspace` | `id`, `name`, `alive`, `index`, `active`                                                                                                                                                  |
| `Output`    | The scalar properties of an output, such as `name`, `make`, `model`, `mode`, `scale`, `primary`, `enabled`, `width` and `height`.                                                         |

The table uses TypeScript spellings. Python and Rust spell the multi-word names in snake_case, such as `app_id`.

A field the kind does not have, or a value that is not a string, a number or a boolean, is an error that names
the fields that work.

### Filter with a function

A function receives each object and keeps it when the function returns a truthy value. Use it for anything but equality,
such as a title that contains a word:

::: tabs

```ts
const drafts = gnoblin.find("Window", (window) => window.title.includes("draft"));
```

```python
drafts = gnoblin.find("Window", lambda window: "draft" in window.title)
```

```rust skip="find_clients takes a typed ClientQuery, not a closure; a closure filter has no Rust equivalent in core.md"

```

:::

An error inside the function is an error of the `find` call.

## `where` on a collection

`where(query)` on a collection returns a live collection of the members whose properties equal the query. In TypeScript
and Python it is called on `gnoblin.clients`. It keeps following the objects: a window that later gets the matching
title joins it, and one that stops matching leaves. You can listen to it and style it like any collection.

::: tabs

```ts
import "gnoblin-std";

const terminals = gnoblin.clients.where({ appId: "foot" });
terminals.style.opacity = 0.9;
```

```python
from gnoblin.objects.model import ClientQuery, Gnoblin

gnoblin = Gnoblin.get()
terminals = gnoblin.clients.where(ClientQuery(app_id="foot"))
terminals.style.opacity = 0.9
```

```rust
let Some(clients) = model.clients()? else {
    return Ok(());
};
let terminals = clients.where_(
    &ClientQuery {
        app_id: Some("foot".to_owned()),
        ..Default::default()
    },
    None,
)?;
if let Some(style) = terminals.style()? {
    style.set_opacity(0.9)?;
}
```

:::

- A query value is compared for equality. To match one of several values, use the `any` field for that key:
  `{ anyAppId: ["foot", "kitty"] }` in TypeScript, `any_app_id=["foot", "kitty"]` in Python and
  `any_app_id: Some(vec!["foot".to_owned(), "kitty".to_owned()])` in Rust. An empty `any` list matches no member.
- The keys depend on the collection. Windows take `id`, `appId`, `title`, `focused`, `fullscreen`, `x`, `y`, `width`,
  `height`, `anyId`, `anyAppId` and `anyTitle`. Outputs take `id`, `name`, `make`, `model`, `mode`, `scale`, `primary`,
  `enabled`, `x`, `y`, `width`, `height`, `anyId` and `anyName`. Workspaces have no `where`: select them with `find`
  and the `Workspace` kind.
- Calling `where` on a selection narrows it further.
- A selection's `parent` is the collection it was made from, and its `query` is the query it was made with. On the
  top-level collections (`clients`, `outputs`) and on a list you create yourself, `parent` and `query` are none: `undefined`
  in TypeScript, `None` in Python and Rust.
- `where(query, priority)` ranks the selection's `style` among the other selections that style a window: the higher
  priority wins. See [window style](/js-style).

## Choosing between them

- Use `find` to act once on what exists now, as a key handler does.
- Use `where` to keep a rule true as windows come and go, such as a style for every terminal.
- Turn a collection into a plain array when you want a single object or an array of objects in the language you use,
  then pick from that array: [Windows](windows#read-windows).

Focus history, the windows in the order they were last focused, is in [Focus and activation](focus-activation#focus-history).
