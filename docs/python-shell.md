# Python shell

Write the shell as a Python script. You install nothing from `pip` and run no build step: the shell runs the file as it is,
and reloads it when you save it or a module it imports.

```python
from gnoblin import gnoblin


@gnoblin.on_client_created
def place(window):
    window.x, window.y = 40, 40


@gnoblin.bind("<Super>Return")
def terminal():
    gnoblin.spawn("foot")
```

Run it with `gnoblin ./shell.py`. It reloads when you save it or a module it imports. A chord is each modifier in angle brackets,
then the key, as in `<Super>Return`; see [shortcuts](/guides/shortcuts) for the syntax.

## Run it

| To run the shell          | Do this                                                                               |
| ------------------------- | ------------------------------------------------------------------------------------- |
| Once, from a terminal     | `gnoblin ./shell.py`                                                                  |
| At every start            | Save it as `~/.config/gnoblin/shell.py`, then run `gnoblin`                           |
| By name                   | Save it as `~/.local/share/gnoblin/shells/tiler.py`, then run `gnoblin --shell tiler` |
| Chosen in a file          | Set `[shell] path = "~/dev/tiler/shell.py"` in `~/.config/gnoblin/config.toml`        |
| Chosen by the environment | `GNOBLIN_SHELL=~/dev/tiler/shell.py gnoblin`                                          |

`gnoblin --list-shells` shows which shell would run and why. [Start Gnoblin and choose a shell](starting-gnoblin.md)
lists every place Gnoblin looks and the errors it gives. Do not put both a `shell.py` and a `shell.ts` in the config
directory: two equal shells stop the start with a list of both.

## What you need

- **A system Python, 3.10 or newer**, with its shared library. On Debian and Ubuntu that is `libpython3.N` (the `python3`
  package pulls it in); on Fedora it is `python3-libs`. The runtime links the library of the Python it was built against, so a
  package of the runtime names that Python as a dependency. Nothing else is needed: no `pip` package, no `node`.
- **The runtime library**, `libgnoblin-runtime-python.so`, in `<libdir>/gnoblin/`. The shell loader opens it when it is asked
  to run a `.py` file, and never otherwise, so a session that runs no Python shell never maps the interpreter. Without it a
  `.py` shell is refused with the library's name and the directories searched. `GNOBLIN_RUNTIME_DIR` names a directory to
  look in first.
- **The package**, `gnoblin`, installed as data in `<prefix>/share/gnoblin/python/`. The runtime puts it on the `sys.path` of its
  own interpreter and nowhere else, so it is invisible to the Python you run in a terminal. `GNOBLIN_PYTHON_DIR` names a
  directory that holds it, to read it from instead (a source tree runs the `data/python` it was built from). Its generated
  modules (`gnoblin._generated` and `gnoblin.std`) are not read from there: the runtime library carries them, written from the
  schema and the ABI crates when it was built, and serves them to its interpreter ahead of the path. The copies `dist` installs
  beside the package, with the stubs, are for editors and checkers.

Libraries you installed for your Python (`requests`, `numpy`) import as usual.

## Write a shell

A shell is an ordinary script. It runs top to bottom once, and what it registers (a listener, a key, a timer) lasts until the
next reload replaces it.

### The `gnoblin` object

`from gnoblin import gnoblin` is the handle on the compositor. It is a thin spelling of the core: everything it does is one call
of the installed `libgnoblin-core`, and it holds no desktop policy.

| Use                                          | What it does                                                                      |
| -------------------------------------------- | --------------------------------------------------------------------------------- |
| `gnoblin.windows`                            | the windows in the tree, in stacking order (the first is the bottom)              |
| `gnoblin.root`                               | the root of the tree; `root.children()` lists what is in it                       |
| `gnoblin.spawn("foot", "-e", "htop")`        | starts a program (not through a shell), in a process group of its own             |
| `gnoblin.after(500, fn)`, `every(500, fn)`   | calls `fn()` once, or every 500 ms; both return a `Subscription`                  |
| `gnoblin.keep_state(key, data)`              | keeps bytes for the next generation of this shell; `kept_state(key)` reads them   |
| `gnoblin.log("text", "warn")`                | writes a line to the compositor's log (`error`, `warn`, `info`, `debug`, `trace`) |
| `gnoblin.post(fn)`                           | runs `fn()` on the shell thread; the one call that is safe from another thread    |
| `gnoblin.request_reload()`, `request_exit()` | asks the host to reload this shell, or to end the compositor                      |

### Listening

A named subscription is an attribute of `gnoblin` that registers a function. Used as a decorator, it returns your function
unchanged, so you can still call it:

```python
@gnoblin.on_client_created                  # no parameters: the function is the callback
def place(window): ...

@gnoblin.bind("<Super>q")                    # parameters first: the call returns the decorator
def close_focused():
    focused = next((w for w in gnoblin.windows if w.focused), None)
    if focused:
        focused.close()

sub = gnoblin.bind("<Super>d", launcher)     # parameters and the function: returns the Subscription
sub.unsubscribe()
```

| Subscription                                                                                                               | The function is called with                                                   |
| -------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `on_client_created`, `on_client_destroyed`                                                                                 | the window                                                                    |
| `on_title_changed`, `on_app_id_changed`                                                                                    | the window and the new text                                                   |
| `on_focus_changed`                                                                                                         | the window that has the keyboard, or `None`                                   |
| `on_window_property_changed(name)`                                                                                         | the window and the new value                                                  |
| `on_window_request`                                                                                                        | the window and a `WindowRequest` (`maximize`, `close`, ...)                   |
| `on_node_created(class)`, `on_node_destroyed(class)`                                                                       | the node; `class` is a schema name such as `"screen"`                         |
| `on_class_property_changed(class, name)`, `on_property_changed(node, name)`                                                | the node and the new value                                                    |
| `on_signal(class, signal)`                                                                                                 | the node and the signal's fields                                              |
| `on_node_event(node)`                                                                                                      | a `NodeEvent` (a key, a pointer press, the wheel)                             |
| `on_shutdown`                                                                                                              | nothing; runs when this generation ends                                       |
| `bind(chord)`, `bind_events(chord)`, `bind_pointer(chord)`, `bind_gesture(kind, fingers)`, `claim_pointer_buttons(device)` | nothing, a `ChordEvent`, a `PointerDrag`, a `GestureEvent`, a `PointerButton` |

`gnoblin.off(fn)` removes every registration made for `fn`. You need it only to change your mind while the shell runs: a
reload removes every registration of the old script, so a script never undoes its own listeners.

Windows that were open before the shell started are not "created". List `gnoblin.windows` once at the top of the file and
then listen.

### Nodes

The compositor's objects are nodes of one tree, and the classes you meet (`Window`, `Seat`, `Screen`, ...) are generated from
the schema, so they have exactly the properties, methods and signals the schema has. A property is an attribute:

```python
window = gnoblin.windows[0]
print(window.title, window.app_id, window.focused)   # read-only
window.x, window.width = 100, 640                    # an int into a double property is a double
window.maximised = True
window.focus()                                       # a method of the schema
```

- A write is queued and leaves with the turn's other writes, in one transaction. A read sees the tree as the last batch
  left it.
- A node you hold is its id. Reading a window that has closed raises `NodeGoneError`; a node you kept across a reload keeps
  working, because the node does.
- `node.get("name")`, `node.set("name", value, seconds)` and `node.call("method", *args)` work by schema name for a member the
  generated class does not have yet.
- `Window.closed(fn)` listens to a signal of every window; `window.closed(fn)` to one window's.

### The compositor's own verbs and signals

What is not a node is the `gnoblin` namespace: the session, the idle watch, the outputs. `gnoblin.verbs` has one object for each,
generated from the schema like the node classes, with the parameters named and typed:

```python
from gnoblin import gnoblin, verbs

verbs.idle.inhibit("my-shell", on_answer=lambda result, error: gnoblin.log("held" if error is None else error, "info"))

def rang(count):                # the payload is the schema's field order
    gnoblin.log("the bell has rung %d time(s)" % count, "info")

verbs.session.on_bell(rang)
```

A verb sends the call and returns. `on_answer(result, error)` runs once on the shell thread when the compositor answers:

- `result` is the operation's value as JSON bytes, empty for a bare "done".
- `error` is the compositor's error code, or `None`.

A signal method returns the `Subscription` that removes it, and a reload removes what a generation registered. A call with the
wrong arguments raises `TypeError`. A session with no owner socket raises `GnoblinError` at the first call.

### Errors

A failed call of the core is an exception: `GnoblinError` and its subclasses (`NodeGoneError`, `MemberError`, `RefusedError`,
`RetiredError`, ...). A value of the wrong kind is `TypeError`.

A callback that raises does not stop the shell: the traceback goes to the log, the next event is delivered, and the core
counts the failure against the generation. `print` and the `logging` module reach the log too, with the component `python`.

### Threads

Everything you register runs on the shell thread, one callback at a time. A thread you start may log and call
`gnoblin.post(fn)`; any other call from it raises `GnoblinError` that says so. The interpreter's lock is released while the
shell waits for events, so your threads run meanwhile.

## Reload

The shell reloads when you save the script or any module it imports from its own directory, and `gnoblinctl config reload`
does it on request. The runtime looks at those files a few times a second and asks the host, which reloads through the loader
every language shares:

1. The new generation is **staged**: the script runs again, in a fresh namespace, beside the one that serves. Nothing it
   registers receives an event yet.
2. If it ran cleanly, the host **commits** it: the old generation's listeners, keys and timers are removed and the new ones
   take over in the same step. A window is neither destroyed nor made again, and a client's connection never notices.
3. If it raised, nothing changes: the old generation keeps serving with the modules it has, and the log says why.

What a reload does and does not do:

- **The script and every module beside it are read again from the source.** Nothing is served from a stale `sys.modules`
  entry or a `.pyc`: an edit that keeps the file's size and lands in the same second as the last one is still seen.
  Modules from `site-packages` are shared by every generation, as in any Python process.
- **No listener runs twice.** The core removes the old generation's registrations when it retires. Your script does not
  undo anything itself.
- **A program you start at the top level** keeps running through a reload that starts it again with the same arguments, and
  is not started a second time. A program a callback starts belongs to you and is never stopped.
- **State** you want to survive goes through `gnoblin.keep_state`. Globals of the old module are gone.
- **A failed reload reports the line.** The host's line names the file and line of the innermost frame that is yours, and the
  log has the whole traceback:

    ```text
    reload of /home/me/.config/gnoblin/shell.py failed, still running generation 1: ...: init failed (E_ERR):
    /home/me/.config/gnoblin/tiles.py:14: RuntimeError: tiles.py is not ready
    ```

    Fix the file and save: the next reload is the next generation. A module you add after a failure is noticed too.

- **A first load that fails** ends the shell with the same message; there is no generation to keep.

`GNOBLIN_LOG=debug` shows each generation's life. The runtime logs `python: FILE changed, asking the host to reload` when it
sees an edit.

## Type declarations

An editor, `mypy` or `pyright` can check a shell without running it. The package ships its declarations with it:

- `py.typed` and the inline annotations of the package;
- `gnoblin/__init__.pyi`, and the stubs of the generated classes;
- `_gnoblin_core.pyi`, for the built-in module.

Point the checker at `<prefix>/share/gnoblin/python`, or add it to `extraPaths`.

## The desktop library: `gnoblin.std`

`gnoblin.std` is the object model above the core: collections of clients, outputs and workspaces, layouts, pointer bindings,
transitions, styles and scene nodes. It is **generated** from the function table of `libgnoblin-std.so.1`
(`gnoblin-sdkgen`, at build time and never committed; [the generator](sdk-generator.md)), so it has every function of the table and none the library lacks.

`std.py` and its `std.pyi` are typed:

- A record is a dataclass, and a constant group an `IntEnum` or `IntFlag`.
- A handle is a class: `Collection`, `Workspace`, `Client`, `Transition` or `Registration`.
- Every call goes through one native function, `gnoblin._std.call`.

```python
from gnoblin import gnoblin
from gnoblin.std import DwindleLayout, EventKind, Layout, Placement, Workspace, clients

class Columns(Layout):                      # a layout of your own: the library calls compute() when the workspace changes
    def compute(self, cx):
        area = cx.inset_area()
        width = area.width / max(len(cx.children), 1)
        return [Placement.tile(c, area.x + i * width, area.y, width, area.height) for i, c in enumerate(cx.children)]

main = Workspace.create("main", layout=Columns(), gaps=(10.0, 20.0))
clients().on(EventKind.ADDED, lambda event: gnoblin.log(f"client {event.member} opened"))

other = Workspace.create("other", gaps=(6.0, 12.0))
dwindle = other.use(DwindleLayout(ratio=0.6))   # a built-in layout: the algorithm is the library's
dwindle.toggle_split()                          # the actions of a kind are methods, its options are properties
```

- **Built-in layouts and bindings** (`MasterStackLayout`, `GridLayout`, `MonocleLayout`, `DwindleLayout`, `StripLayout`,
  `ArcLayout`, `Carousel3dLayout`, `MoveBinding`, `ResizeBinding`, `SwapBinding`, `SplitResizeBinding`) are generated from the
  library's own table of kinds, so each option, its default, its enumeration and each action is the library's.
- **A layout of your own** is an object with `compute(cx)` that returns placements, and optionally `client_added`, `client_removed`,
  `area_changed`, `clients_swapped`, `revision`, `find_split`, `set_split` and `reset_split`. `Layout` is the generated base class.
- **Failures** are the exceptions of `gnoblin` (`RefusedError`, `StaleError`, ...), with the library's text.
- The library is opened by `import gnoblin.std`. Where it is not installed the import raises `ImportError`: `gnoblin-std is not
installed (looked for libgnoblin-std.so.1 in ...)`, which is the check a core-only install is meant to give. A shell on
  `gnoblin` alone is complete: example 8 tiles windows with no workspace.
- The only hand-written parts are `_std_base.py` (the native, handles, argument conversion) and `_std_sugar.py` (`len(collection)`,
  `client.window`, `Placement.tile`, `Workspace.create`, `Workspace.use`).

## Examples

Three small shells in `examples/` show the pieces, and a test in `crates/gnoblin-runtime-python/tests/shells.rs` runs each one
as it is:

- `examples/07-script-binding/shell.py`: a key binding written entirely in Python that moves the focused window.
- `examples/03-python-std/shell.py` (example 3) and `layouts.py`: a workspace with a layout of your own, a listener on the clients and a
  chord, and the built-in layouts. The tests hold their rectangles to the ones the native Rust shells produce.
- `examples/08-no-workspaces-py/`: a master-and-stack tiler of raw window nodes, in a shell file and a module it imports (edit
  `tiles.py` and the tiling changes without restarting).
- The same test builds example 11 (edit the shell and a module it imports; no listener twice; a broken edit keeps the shell) and
  example 17 (a core-only shell runs, and importing std says why it cannot).
