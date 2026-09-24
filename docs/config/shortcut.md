# gnoblin.shortcut

This is the legacy declaration form for adding or updating a named keyboard shortcut. Prefer [`gnoblin.configure.shortcuts`](/guides/shortcuts), which lets you edit named entries directly. An existing entry with the same `name` is merged: omitted fields stay unchanged. The [shortcuts guide](/guides/shortcuts) covers key names, conflicts and command behavior.

```lua
gnoblin.shortcut {
    name = "terminal",
    binding = "<Super>Return",
    command = {"ptyxis", "--new-window"},
}
```

| Field           | Values                                                 |
| --------------- | ------------------------------------------------------ |
| `name`          | Required nonempty string; letters, digits, `_` and `-` |
| `binding`       | GTK accelerator, such as `"<Super>Return"`             |
| `command`       | Argument list; no shell expansion                      |
| `capture_input` | Boolean; buffers popup typing; default `false`         |

A new shortcut needs both `binding` and `command`. An override can supply only its name and changed fields. Different names cannot claim the same binding. Limit: 256 command shortcuts.
