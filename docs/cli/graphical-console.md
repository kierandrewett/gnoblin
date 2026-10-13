# Graphical Console

Use the graphical console to inspect and invoke supported session API methods.

The graphical console is available in a running Gnoblin session.

## Graphical console

With the built-in configuration, Alt+F2 opens the compositor's Lua console. Alt+F2 or
Escape closes it. It drops down from the top of the screen over every application and
shell layer, with the input line at the bottom.

Output grows upwards from the input line. Scroll it with the mouse wheel or Page Up and
Page Down. Enter runs a command and Shift+Enter inserts a newline.

While the console is open it is modal: keyboard shortcuts and keybindings do not run, and
every key goes to the console. Ctrl+Alt+F1 to F12 still switch to another virtual console.
Escape and Alt+F2 close it even if you have bound the console to another key.

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

Suggestions appear above the input as you type. Completion supports globals and dotted
member names, such as `gnoblin.windows.`. It does not evaluate the input or call Lua getters.
Press Tab to take the highlighted suggestion. Use Up and Down to move through the list; Enter
then takes the chosen suggestion, and otherwise runs what you typed. Escape hides the list.

A result that is a table or window record starts collapsed. Click its row, anywhere across
the width, to expand it. Expanded entries are read-only and bounded; a table with more entries
than are listed ends with a line such as `… 12 more`. Cycles are marked. Expanding does not
execute Lua code. Errors show the message alone, for example `attempt to index a nil value`.

It keeps the last 200 submitted commands in
`$XDG_STATE_HOME/gnoblin/console-history.ini`, or
`~/.local/state/gnoblin/console-history.ini` when `XDG_STATE_HOME` is unset.
The history is diagnostic state, separate from the Lua configuration. Its file
is written asynchronously after a submitted command.
