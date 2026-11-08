# Find objects

> **Status:** not available yet: `gnoblin.focus.history`, `gnoblin.layers.list`. The sections that use them describe the target, not current behaviour.

`gnoblin.find(kind, filter?)` returns the live objects of one kind that match a filter. One call works for every
kind that has a list, so you do not need a separate filter for each one.

```lua
local notes = gnoblin.find("Window", {app_id = "org.gnome.Notes", focused = true})
local primary = gnoblin.find("Output", {primary = true})[1]
```

| Argument | Meaning                                                                                              |
| -------- | ---------------------------------------------------------------------------------------------------- |
| `kind`   | The name of an object kind, as a string. The kinds are listed below.                                 |
| `filter` | Optional. A table of field values, or a predicate function. Without it, `find` returns every object. |

## Kinds

| Kind        | List that `find` filters    |
| ----------- | --------------------------- |
| `Window`    | `gnoblin.windows.list()`    |
| `Workspace` | `gnoblin.workspaces.list()` |
| `Output`    | `gnoblin.outputs.list()`    |

The result is an array of the same live handles that the list and the events give, in the order of the list. Compare
two handles with `==`.

## Table filter

A table filter keeps the objects whose fields equal every value in the table. Each key is the name of a field that
the kind declares, and each value is a string, a number or a boolean. For example, `{app_id = "notes", focused = true}`
keeps the focused windows with the application ID `notes`.

A table filter raises a Lua error, and the error names the fields that work, when:

- the kind does not declare a field of that name;
- a field is not a scalar, such as `Window.effects`;
- a value is not a string, a number or a boolean.

The fields of each kind are in the [type reference](/shell-api/types) and in `gnoblin.d.lua`.

## Predicate filter

A predicate is a function that receives each object and returns `true` to keep it. Use one for anything other than
equality, such as a title that contains a word:

```lua
local drafts = gnoblin.find("Window", function(window)
    return window.title:find("draft", 1, true) ~= nil
end)
```

An error inside the predicate is an error of the `find` call.

## Unknown kinds

`gnoblin.find` raises a Lua error for a kind that has no list. The error names the kinds that work.

`gnoblin.windows.list()` and `gnoblin.layers.list()` take no arguments. `gnoblin.focus.history(filter?)` keeps its
filter, because its result has an order and a limit. The socket and `gnoblinctl` methods keep their own filters.
