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
- **The package**, `gnoblin`, which needs nothing installed: the runtime library carries it, so the Python you run in a terminal
  never sees it.

Libraries you installed for your Python (`requests`, `numpy`) import as usual.

### How the package is served

Nothing of the package is installed as source or read from disk, and no directory or variable names it. A finder on the
`sys.meta_path` of the runtime's own interpreter serves it:

- The typed classes (`gnoblin._generated`, `gnoblin.std`, `gnoblin.objects`) are written from the schema and the ABI crates when the
  runtime is built, so the package cannot describe a different catalog from the libraries it runs with.
- The rest is Rust in `gnoblin-runtime-python`: the loader that isolates each generation, the log routing, the descriptors of the
  typed classes, the `gnoblin` object, the exceptions and the subscriptions.

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

An editor, `mypy` or `pyright` can check a shell without running it. The package is embedded in the runtime, so what is installed for
the checker is a stub-only package, `<prefix>/share/gnoblin/types/python/`:

- `gnoblin/py.typed`, which marks it;
- `gnoblin/__init__.pyi` and the stubs of the parts written in Rust (`_base`, `api`, `errors`, `events`);
- the stubs of the generated classes and of `gnoblin.std` and `gnoblin.objects`, written by `cargo xtask dist` from the schema and the
  ABI crates;
- `_gnoblin_core.pyi`, for the built-in module.

Point the checker at `<prefix>/share/gnoblin/types/python`, or add it to `extraPaths`. The hand-written stubs live in
`crates/gnoblin-runtime-python/stubs/`, and a test (`tests/shells.rs`, `the_stubs_of_the_package_agree_with_what_the_runtime_reflects`)
holds them to what the runtime reflects of every public name.

`pyright` finds no error in any shell of `examples/` or in the golden shell (`tests/golden/shell.py`) against the installed stubs. The
test (`every_python_example_type_checks_clean_against_the_installed_stubs`) runs when `pyright` is on the path or `GNOBLIN_PYRIGHT`
names it, and fails on any error. A `GNOBLIN_PYRIGHT` that names nothing is an error too.

A stub differs from the classes at run time in two ways, on purpose:

- A property or a result reads as `None` only where the catalog marks it `optional`.
- A class of `gnoblin.objects` that derives from another (`Camera3D` from `SceneNode`) names it as its base, so a checker accepts it
  where the base is wanted. At run time the classes are flat, and a call takes any handle.

## The desktop library: `gnoblin.std`

`gnoblin.std` is the object model above the core: collections of clients, outputs and workspaces, layouts, pointer bindings,
transitions, styles and scene nodes. It is **generated** from the function table of `libgnoblin-std.so.1`
(`gnoblin-sdkgen`, at build time and never committed; [the generator](sdk-generator.md)), so it has every function of the table and none the library lacks.

`std.py` and its `std.pyi` are typed:

- A record is a dataclass, and a constant group an `IntEnum` or `IntFlag`.
- A handle is a class: `Collection`, `Workspace`, `Client`, `Transition` or `Registration`.
- Every call goes through one native function, `_gnoblin_std.call`.

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
- The only hand-written parts are `gnoblin._std_base` (the native, handles, argument conversion) and `gnoblin._std_sugar`
  (`len(collection)`, `client.window`, `Placement.tile`, `Workspace.create`, `Workspace.use`). Both are Rust in
  `gnoblin-runtime-python`, like the rest of the package.

## The object classes: `gnoblin.objects.std`

The catalog of `libgnoblin-std` (an entry model, the focus policy, and the records and enums they take) is served through the object
protocol, and `gnoblin.objects.std` has a class for each of its types. `gnoblin.objects.model` is the same for the core's
`gnoblin.model`.

The classes are **generated** from the catalog the library serves (`gnoblin-sdkgen`, at build time and never committed; [the
generator](sdk-generator.md)) and embedded in the runtime library. A class added to the catalog is a class here with no change to the
runtime.

```python
from gnoblin import gnoblin
from gnoblin.objects.std import EntryModel, FocusEvent, FocusEventKind, FocusMode, FocusPolicy, FocusSettings

entry = EntryModel("héllo")                    # a handle: the library holds the object, the script holds a reference
entry.insert(", world")                        # a method changes it at once
entry.select_all()
print(entry.text, entry.selection)             # a property is read; a record comes back as a dataclass: TextRange(start=0, end=12)

@entry.on("changed")                           # a signal, told from the library's next turn and never inside the call that caused it
def heard():
    gnoblin.log("the entry changed")

entry.on("text_changed", lambda text: gnoblin.log(text))   # `<property>_changed` is an event of each property; `on` returns a listener

action = FocusPolicy.focus_decision(           # a function of a service: records and an enum go in, a record comes out
    FocusSettings(mode=FocusMode.SLOPPY), FocusEvent(kind=FocusEventKind.ENTER, held=False))
```

- A **record** is a `dataclass`. A field with a default starts at it and an optional one at `None`: `Border()` is one pixel wide and
  white.
- The record of a **family** has no class of its own. It is one of its variants, each a `dataclass`, and `Filter` is the union of them:
  `style.backdrop_filter = [BlurFilter(), SaturateFilter(amount=1.5)]`. A `BlurFilter` starts at a `radius` of 12 and a grade at an
  `amount` of 1. A call that takes a family refuses anything but an instance of a variant, with a `TypeError` that names them.
- A record is a detached value. One you read is a copy, so write it or its list back to change anything. The library checks it when it
  takes it, not when you build it.
- A member that takes a window takes the core's `gnoblin.objects.model.Client`. `examples/36-native-clients/shell.py` shows these calls:
    - `decorations.inset_of(client)`
    - `menus.show(client, x, y)`
    - `workspace.clients.add(client)`
- An id as text, or an object of another class, is a `TypeError`. A window that has closed raises `NodeGoneError`.
- A window the library tells the shell of is still its id as text. `Gnoblin.get().clients.get(id)` gives its `Client`.
- An **enum** is an `IntEnum` (a method also takes its name, `"follow-mouse"`), and a list a `list`.
- `on(event, handler)` returns a `Listener` with `stop()`; without a handler it is a decorator. It lasts until stopped or the
  generation ends, not until the object is collected.
- An **interface** (`gnoblin.objects.std.<Name>` of kind `interface`) is a base class: a subclass defines the methods it wants to answer
  and passes an instance to the library; a method it does not define runs natively.
- `.destroy()` ends the object and its children whoever else holds them. A reference the script drops is released on the shell thread, so a
  thread of the script may drop one.
- A failure is the exception of `gnoblin` for its kind (`StaleError` for an object that has ended), a value of the wrong type is a
  `TypeError` that names it, and an exception a callback raises is logged with its traceback.
- `gnoblin/objects/*.pyi` are the stubs. A `tests/shells.rs` check holds them to what the runtime reflects. Importing the module opens
  `libgnoblin-std.so.1` and raises `ImportError` without it, as `gnoblin.std` does.

`examples/20-native-object/shell.py` is this, run by `crates/gnoblin/tests/modular/native_object.rs` in a real session beside the
Rust and TypeScript shells of the same example.

### The root of the core: `gnoblin.objects.model`

`Gnoblin.get()` returns the root of the core's model. It has the members the TypeScript library's `gnoblin` has for the core:

```python
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()
gnoblin.timers.after(40, lambda: print("later"))   # `timers.now()` is the monotonic clock in milliseconds
print(gnoblin.version().api)                       # a method, not a property
print(gnoblin.motion(200))                         # 200.0, or 0.0 while the seat asks for reduced motion
print(gnoblin.compositor.user_shaders)             # what you wrote, else what the compositor read when it started
gnoblin.compositor.user_shaders = False            # compositor.user-shaders in the shell's settings document
gnoblin.compositor.reset("user_shaders")           # back to the default
windows = gnoblin.find("Window", {"app_id": "foot"})   # a dict of fields, a ClientQuery, or a function
```

A Python shell commits no configuration document of its own, so nothing commits the write: it stays in `gnoblin.input.document`, which only
the JavaScript runtime commits. To change the setting for the next login, set it in a `shell.ts` as [custom shaders](/custom-shaders) says.
A value that is not a boolean raises a `TypeError`.

`find("Window", ...)` and `find("Output", ...)` raise a `TypeError` for a kind the core does not list (`"Workspace"` in a shell that loads
no std), naming the kinds that work, and for a field the filter lacks. `examples/35-native-root/shell.py` is this.

### The functions of `gnoblin-std` in the generated classes

A bare function of the TypeScript `gnoblin-std` is a function of a class in `gnoblin.objects.std`, with the same behaviour. The
Python spellings are `snake_case`.

| `gnoblin-std` (TypeScript) | `gnoblin.objects.std`                                                                                                          |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `openLauncher`             | `Launcher.open(spawn=None)`: one launcher lives at a time, and a second call focuses it and returns it                         |
| `toast`                    | `Toast.show(text, fade_in=None, hold=None, fade_out=None, clock=None)`: the card removes itself after 4420 ms by default       |
| `onSwipe`, `onPinch`       | `SwipeActionBinding()` and `PinchActionBinding()`, then `set_action(way, name)` or `set_function(way, fn)`, then `install()`   |
| `direction`                | `Gestures.direction(dx, dy, threshold=None)`: a `SwipeDirection` with the `way`, or `None` while the gesture has gone less far |
| `estimateText`             | `Ui.estimate_text(text, size)`: a `TextMetrics` of `width`, `height` and `ascent`, and no desktop is asked                     |
| `encodePaintList`          | `Ui.encode_paint_list(ops)`: `bytes` (the TypeScript function answered a list of numbers) for a canvas's `content`             |

`Ui.encode_paint_list` refuses the whole list when one operation cannot be encoded, with an error that names it (`draw op 2: ...`).

## Examples

Three small shells in `examples/` show the pieces, and a test in `crates/gnoblin-runtime-python/tests/shells.rs` runs each one
as it is:

- `examples/07-script-binding/shell.py`: a key binding written entirely in Python that moves the focused window.
- `examples/03-python-std/shell.py` (example 3) and `layouts.py`: a workspace with a layout of your own, a listener on the clients and a
  chord, and the built-in layouts. The tests hold their rectangles to the ones the native Rust shells produce.
- `examples/08-no-workspaces/shell.py` and `tiles.py`: a master-and-stack tiler of raw window nodes, in a shell file and a module it imports (edit
  `tiles.py` and the tiling changes without restarting).
- The same test builds example 11 (edit the shell and a module it imports; no listener twice; a broken edit keeps the shell) and
  example 17 (a core-only shell runs, and importing std says why it cannot).
