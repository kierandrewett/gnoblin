# Graphical Console

Use the graphical console to inspect and invoke supported session API methods.

The graphical console is available in a running Gnoblin session.

## Graphical console

With the built-in configuration, Alt+F2 opens the compositor’s Lua console.
Escape closes it. The input box sits below the output; Enter runs a command
and Shift+Enter inserts a newline. The console appears above application and
shell layers.

`gnoblin.console.toggle()` (Native API 1.77+) takes no arguments and queues a
visibility toggle.
It is unavailable while the session is locked. Bind it in a loaded Lua config:

```lua
gnoblin.configure {
    keybindings = {
        keyboard = {
            console = {
                binding = "<Alt>F2",
                callback = function() gnoblin.console.toggle() end,
            },
        },
    },
}
```

Run `gnoblinctl reload` after saving. The console evaluates Lua against the
active configuration runtime and sends reload requests through compositor
control.

Press Tab or choose **Complete** to list matching names from the active Lua
environment. Completion supports globals and dotted member names, such as
`gnoblin.windows.`. It does not evaluate the input or call Lua getters.

Returned tables and window records have expandable property previews. Previews
are read-only and bounded; cycles and omitted properties are marked. Expanding
a preview does not execute Lua code.

It keeps the last 200 submitted commands in
`$XDG_STATE_HOME/gnoblin/console-history.ini`, or
`~/.local/state/gnoblin/console-history.ini` when `XDG_STATE_HOME` is unset.
The history is diagnostic state, separate from the Lua configuration. Its file
is written asynchronously after a submitted command.
