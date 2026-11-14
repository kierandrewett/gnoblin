# Find objects

Two calls select windows, workspaces and outputs. `gnoblin.find` returns an array of the objects that match right now.
`where` on a collection returns a live selection that follows the objects as they change.

```ts
import { defineConfig } from "gnoblin";

defineConfig({
    keys: {
        "Super+f": ({ gnoblin }) => {
            const [notes] = gnoblin.find("Window", { appId: "org.gnome.Notes" });
            notes?.focus();
        },
    },
});
```

`Super+f` focuses the Notes window when there is one.

## `gnoblin.find`

`gnoblin.find(kind, filter?)` returns the live objects of one kind in list order. The objects are the same ones the
collections hold, so a `Client` from `find` is the `Client` from `gnoblin.clients`.

| Argument | Meaning                                                                                     |
| -------- | ------------------------------------------------------------------------------------------- |
| `kind`   | `"Window"`, `"Workspace"` or `"Output"`.                                                    |
| `filter` | Optional. An object of field values, or a function. Without it `find` returns every object. |

An unknown kind throws a `TypeError` that names the kinds that work.

### Filter by field values

An object filter keeps the objects whose fields equal every value in it. Each value is a string, a number or a boolean.
`appId` and `app_id` name the same field.

```ts
const focusedNotes = gnoblin.find("Window", { appId: "org.gnome.Notes", focused: true });
const primary = gnoblin.find("Output", { primary: true })[0];
```

The fields you can filter on:

| Kind        | Fields                                                                                                                                                                                    |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Window`    | `id`, `appId`, `title`, `alive`, `focused`, `fullscreen`, `secure`, `unresponsive`, `floating`, `maximised`, `minimised`, `tiled`, `visible`, `x`, `y`, `width`, `height`, `opacity`, `z` |
| `Workspace` | `id`, `name`, `alive`, `index`, `active`                                                                                                                                                  |
| `Output`    | The scalar properties of an output, such as `name`, `make`, `model`, `mode`, `scale`, `primary`, `enabled`, `width` and `height`.                                                         |

A field the kind does not have, or a value that is not a string, a number or a boolean, throws a `TypeError` that names
the fields that work.

### Filter with a function

A function receives each object and keeps it when the function returns a truthy value. Use it for anything but equality,
such as a title that contains a word:

```ts
const drafts = gnoblin.find("Window", (window) => window.title.includes("draft"));
```

An error inside the function is an error of the `find` call.

## `where` on a collection

`gnoblin.clients.where(query)` returns a live collection of the members whose properties equal the query. It keeps
following the objects: a window that later gets the matching title joins it, and one that stops matching leaves. You can
listen to it and style it like any collection.

```ts
const terminals = gnoblin.clients.where({ appId: "foot" });
terminals.style.opacity = 0.9;
```

- A query value is compared with `===`. An array matches when the property equals any of its entries.
- The keys depend on the collection. Windows take `id`, `appId`, `title`, `focused`, `fullscreen`, `x`, `y`, `width` and
  `height`. Workspaces take `id`, `name`, `index` and `active`. Outputs take `id`, `name`, `make`, `model`, `mode`,
  `scale`, `primary`, `enabled`, `x`, `y`, `width` and `height`.
- `where(query, { priority })` ranks the selection's `style` among the other selections that style a window: the higher
  priority wins. See [window style](/js-style).

## Choosing between them

- Use `find` to act once on what exists now, as a key handler does.
- Use `where` to keep a rule true as windows come and go, such as a style for every terminal.
- Use the collection's own `find(predicate)` and `filter(predicate)` when you want a single object or an array in plain
  JavaScript: [Windows](windows#read-windows).

`gnoblin.focus.history(filter?)` returns the windows in the order they were last focused: [Focus and activation](focus-activation#focus-history).
