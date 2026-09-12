# Developer console

Implementation checklist:

- [x] Add JavaScript evaluation, retained bindings and object inspection.
- [x] Add the top-edge console, keyboard input, history and completion.
- [x] Replace the Gnoblin Run dialog and Looking Glass entry points.
- [x] Verify keyboard focus, lock behaviour, stacking and rendered output.
- [x] Commit and push each completed part.

`Alt+F2` opens the console in the primary unlocked Gnoblin session. It is a
top-edge modal above the Shell chrome, with a multiline input, retained
bindings, top-level `await`, result history (`r(id)` and `$_`), completion,
syntax highlighting, inline object expansion and selectable stack traces.
The initial view contains only a prompt. Suggestions appear while typing;
Up/Down selects a suggestion and Tab or Enter accepts it. Enter otherwise
evaluates, Shift+Enter inserts a newline, and Ctrl+L clears the transcript. `Escape` closes
it, and a lock transition closes it before the stock unlock UI takes over.

The JavaScript context is intentionally compositor-local: it exposes `global`,
`Main`, GNOME Shell libraries and `windows()`, while the remote D-Bus control
surface remains separate. Build the Shell theme resource after changing the
console Sass file with:

```sh
ninja -C build/gnome-shell data/gnome-shell-theme.gresource
meson install -C build/gnome-shell --no-rebuild
```

Run the focused integration check with `just gnome-developer-console-verify`.

Select the JavaScript or Lua tab at the top to choose the language.
Each tab keeps its unfinished input. Both languages retain their own state across
switches. `:reset` resets both contexts and returns to JavaScript. Ctrl+L
clears output without resetting variables.

Lua supports expressions, chunks, multiple return values, `print()`, and
completion while typing. Globals persist between submissions; `local`
variables belong to the submitted chunk, as in a standard Lua interpreter.
For example:

```lua
counter = 40
counter + 2
local g = require('gnoblin'); g.load('scripts/probe.lua')
```

The Lua console uses the configuration runtime's restricted libraries, memory
and instruction limits, and `require('gnoblin')` API. Its initial settings
are loaded from the current config file. `g.load()` paths and local modules
resolve relative to that root. `g.set()` and changes to `g.config` affect only
this console's copy: they do not write files or apply live desktop settings.
Use JavaScript for direct `Main`, `global`, and Mutter object access.

Inspection expands inline and loads properties in pages of 100. JavaScript
shows own properties (including symbols and non-enumerable properties), arrays,
Map/Set entries, and separate `[[Prototype]]` branches. Accessors remain labelled
until you click a getter; its result or error then appears on that row. Invoking
a getter runs application code and may change state.

Lua results retain object references: expand tables, non-string keys, metatables,
and function source information and captured upvalues. Inspection reads raw
entries without invoking table metamethods. Circular references are marked in
both languages. Expanded contents are read when opened, so mutable values can
reflect changes since evaluation. Lua retains at most 4,096 inspection handles;
`:reset` releases them along with the Lua context.
