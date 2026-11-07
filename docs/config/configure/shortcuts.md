# gnoblin.configure.shortcuts

Bind a key chord to a command. Each shortcut is a named table. The name is the key, so it must be unique in the document.

```lua
gnoblin.configure {
    shortcuts = {
        terminal = {
            binding = "<Super>Return",
            command = {"ptyxis", "--new-window"},
        },
    },
}
```

Each shortcut has these fields:

| Field     | Accepted values                        | Meaning                                                                                                |
| --------- | -------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `binding` | One key chord, such as `<Super>Return` | Required. The chord that starts the command. See the [chord syntax](/guides/shortcuts#chord-syntax).   |
| `command` | Nonempty array of strings              | Required. The program and its arguments. The first entry names the program.                            |
| `trigger` | `"press"` or `"release"`               | Which key edge starts the command. Defaults to `"press"`.                                              |
| `enable`  | `true` or `false`                      | Whether the shortcut is bound. Defaults to `true`. Set `false` to keep it in the document but unbound. |

The command runs directly, with no shell, so `command` takes an argv rather than a string. See [command syntax](/guides/shortcuts#commands-and-shell-syntax).

A release trigger runs the command when the key is released:

```lua
gnoblin.configure {
    shortcuts = {
        launcher = {
            binding = "<Super>space",
            command = {"fuzzel"},
            trigger = "release",
        },
    },
}
```

Two shortcuts cannot share a chord. A duplicate chord, or a chord that does not parse, refuses the whole set.

A reload replaces the whole set at once: either every shortcut in the new document is bound, or none is and the previous set stays in place. The refusal is logged with the shortcut named.

Disable a shortcut without removing it:

```lua
gnoblin.configure {
    shortcuts = {
        terminal = {
            binding = "<Super>Return",
            command = {"ptyxis", "--new-window"},
            enable = false,
        },
    },
}
```

Built-in Mutter actions are not set here. Override them with [`gnoblin.configure.keybindings`](/config/configure/keybindings).

## Type definition

```lua
gnoblin.configure {
    shortcuts = {
        ["shortcut-name"] = {
            binding = string,
            command = {string, ...},
            trigger = "press" | "release"?,
            enable = boolean?,
        }, ...,
    },
}
```

The name must be 1 to 64 letters, digits, underscores or dashes. `?` marks an optional field; `|` separates alternatives.
