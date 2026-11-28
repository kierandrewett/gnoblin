# Python shell

Write the shell as one Python script, `shell.py`. You install nothing from `pip` and run no build step: the shell runs the
file as it is, and reloads it when you save it or a module it imports.

A shell can be written in [TypeScript](typescript-shell.md), [Python](python-shell.md) or [Rust](rust-shell.md). The three
pages have the same sections, and the three languages reach the same objects: windows, workspaces, outputs, input and the
std library. Pick the language you like. The [shell API](/shell-api/) and the [API reference](/reference) show every member in
each of them.

The session runs this shell unless you pick another: see [Where the config lives](#where-the-config-lives).

## Create a shell

A `shell.py` is a program. It runs top to bottom, and each line reads or changes an object of the shell. This one is
short:

```python
from gnoblin import BindingEvent, ClientQuery, Gnoblin
from gnoblin.std import Decorations, MasterStackLayout, Workspace

gnoblin = Gnoblin.get()

Decorations.get().start()

code = Workspace("code", name="Code", layout=MasterStackLayout(ratio=0.6, gap=12.0))
gnoblin.workspaces.add(code)

gnoblin.clients.style.corner_radius = 12
gnoblin.clients.where(ClientQuery(app_id="firefox")).style.corner_radius = 18

seat = gnoblin.input.primary


def terminal(event: BindingEvent) -> None:
    gnoblin.spawn(["foot"])


def close_focused(event: BindingEvent) -> None:
    client = seat.focused_client
    if client is not None:
        client.close()


seat.bind("Super+Return", terminal)
seat.bind("Super+q", close_focused)
```

The file you write is the whole configuration. Nothing is layered under it, so a file with only the lines above has one
workspace, no volume keys and no launcher. Save it as `~/.config/gnoblin/shell.py` (`$XDG_CONFIG_HOME/gnoblin` when that is
set). To start from the full default instead, run this once:

```sh
gnoblinctl config init --language python
```

It writes a commented `shell.py` to `~/.config/gnoblin` (`$XDG_CONFIG_HOME/gnoblin` when that is set), the same desktop as the
TypeScript and Rust starters, and a `pyrightconfig.json` that points an editor at the stubs when they are installed:

```text
ok  shell.py written: /home/me/.config/gnoblin/shell.py
ok  pyrightconfig.json created: /home/me/.config/gnoblin/pyrightconfig.json (finds the stubs in /usr/share/gnoblin/types/python)

Next:
  1. Open /home/me/.config/gnoblin/shell.py in your editor.
  2. Check it with `gnoblinctl check /home/me/.config/gnoblin/shell.py`.
  3. Save it: a running shell reloads the file on save, and `gnoblinctl config reload` does it on request.
```

`config init` never replaces a config. It stops and says so when a shell file is already there, because two shell files in
one directory leave Gnoblin with no way to choose.

Any `shell.*` file stops it, such as a `shell.py`, a `shell.ts` or a `shell.so`. Add `--force` to write the starter over
`shell.py`. The [recipe](/recipes/complete-starter-config) shows the file in full.

Save the file. The shell reloads it after a short pause, and the windows you have open stay where they are. A key you
bind is live at once, and a key you delete stops working at that reload. `gnoblinctl check shell.py` checks the file before
you save it.

Python also has a thinner spelling of the compositor, `from gnoblin import gnoblin`, which works on the compositor's node
tree with decorators (`@gnoblin.bind("<Super>Return")`, `@gnoblin.on_client_created`). It is described under
[The `gnoblin` object](#the-gnoblin-object); the object classes above are the same ones TypeScript and Rust use.
Every class and member a script can use is in the [API reference](/reference).

## Run it

| To run the shell          | Do this                                                                               |
| ------------------------- | ------------------------------------------------------------------------------------- |
| Once, from a terminal     | `gnoblin ./shell.py`                                                                  |
| At every start            | Save it as `~/.config/gnoblin/shell.py`, then run `gnoblin`                           |
| By name                   | Save it as `~/.local/share/gnoblin/shells/tiler.py`, then run `gnoblin --shell tiler` |
| Chosen in a file          | Set `[shell] path = "~/dev/tiler/shell.py"` in `~/.config/gnoblin/config.toml`        |
| Chosen by the environment | `GNOBLIN_SHELL=~/dev/tiler/shell.py gnoblin`                                          |

`gnoblin --list-shells` shows which shell would run and why, and `gnoblin --watch ./shell.py` runs the shell and reloads it
when its files change. Do not put both a `shell.py` and a `shell.ts` in the config directory: two equal shells stop the
start with a list of both.

### Where the config lives

Gnoblin runs one shell, chosen once at start from seven places in order. For a Python shell the ones you use are:

1. **The command line:** `gnoblin ./shell.py` runs that file, and `gnoblin --shell tiler` runs the installed shell called
   `tiler`.
2. **`GNOBLIN_SHELL`:** a value with a `/` is a path, anything else is the name of an installed shell.
3. **`config.toml`:** `[shell] path = "~/dev/tiler/shell.py"` names a file, and `[shell] default = "tiler"` names an
   installed shell. Set one of them, not both.
4. **A file in the config directory:** `shell.py` in `~/.config/gnoblin` (or `$XDG_CONFIG_HOME/gnoblin`). `shell.ts` and
   `shell.so` are found the same way, for a [TypeScript](typescript-shell.md) and a [Rust](rust-shell.md) shell.
5. **Installed for your user:** a file such as `tiler.py`, a directory that holds a `shell.py`, or a launcher file
   `tiler.toml`, in `~/.local/share/gnoblin/shells/`.
6. **Installed for the system:** the same, in `gnoblin/shells/` under each `XDG_DATA_DIRS` directory.
7. **The packaged default:** `<prefix>/share/gnoblin/default-shell/shell.ts`, the stock config that a fresh install runs.

The first place that gives exactly one shell wins. [Start Gnoblin and choose a shell](starting-gnoblin.md) lists every place
Gnoblin looks and the errors it gives.

### What you need

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

- The typed classes (`gnoblin._generated`, `gnoblin.std` and the classes at the root of `gnoblin`) are written from the schema and the ABI
  crates when the runtime is built, so the package cannot describe a different catalog from the libraries it runs with.
- The rest is Rust in `gnoblin-runtime-python`: the loader that isolates each generation, the log routing, the descriptors of the
  typed classes, the `gnoblin` object, the exceptions and the subscriptions.

The shell runs inside the compositor's process, on a thread of its own. Everything you register runs on that shell thread,
one callback at a time.

## Reload while it runs

The shell reloads when you save the script or any module it imports from its own directory, and `gnoblinctl config reload`
or `gnoblinctl shell reload` does it on request. The runtime looks at those files a few times a second and asks the host,
which reloads through the loader every language shares:

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

## Types and editor support

### Check a file

```sh
gnoblinctl check ~/.config/gnoblin/shell.py
```

`check` runs the file in the dry-run shell, the shell's own library over an empty tree, the way the shell does. It reports a
syntax error, an import that does not resolve, a refused value or trigger and a top-level exception at the line you
wrote.

`--types` runs a TypeScript compiler and is for `.ts` files only. To check the types of a Python shell, point `pyright` or
`mypy` at the stubs below. See [Check and types](gnoblinctl.md#check-and-types).

### Editor support

An editor, `mypy` or `pyright` can check a shell without running it. The package is embedded in the runtime, so what is installed for
the checker is a stub-only package, `<prefix>/share/gnoblin/types/python/`:

- `gnoblin/py.typed`, which marks it;
- `gnoblin/__init__.pyi` and the stubs of the parts written in Rust (`_base`, `api`, `errors`, `events`, `host`);
- the stubs of the generated classes and of `gnoblin.std`, written by `cargo xtask dist` from the schema and the
  ABI crates;
- `_gnoblin_core.pyi`, for the built-in module.

Point the checker at `<prefix>/share/gnoblin/types/python`, or add it to `extraPaths` (`python.analysis.extraPaths` in
the Pylance and Pyright settings of an editor). `gnoblinctl config init --language python` writes a `pyrightconfig.json`
with that path, so Pyright and Pylance find the stubs for the files in your config directory.

The hand-written stubs live in `crates/gnoblin-runtime-python/stubs/`. A test (`tests/shells.rs`,
`the_stubs_of_the_package_agree_with_what_the_runtime_reflects`) holds them to what the runtime reflects of every public name.

`pyright` finds no error in any shell of `examples/` or in the golden shell (`tests/golden/shell.py`) against the installed stubs. The
test (`every_python_example_type_checks_clean_against_the_installed_stubs`) runs when `pyright` is on the path or `GNOBLIN_PYRIGHT`
names it, and fails on any error. A `GNOBLIN_PYRIGHT` that names nothing is an error too. The Rust workflow installs `pyright`, so
the check cannot go unrun in CI.

A property that holds an object reads as `Client | None` in a stub, as TypeScript reads `Client | undefined` and Rust `Option<Client>`.
The exception is a property the catalog marks `always`:

- The members of the root (`clients`, `outputs`, `focus`, `session`, `runtime`, `process`, `idle`, `launches`, `input`, `dialogs`,
  `authentication`, `lock`, `timers`, `compositor`) and `InputManager.primary` read as the object, so `gnoblin.input.primary.bind(...)`
  needs no check.
- A library that answered none for one of them would break its word, and the class raises a `GnoblinError` that names the property.

Every other object is checked before it is used: `client = seat.focused_client`, then `if client is not None`. A type checker does not
keep that narrowing inside a function the script defines, so give the object a name of its own in the check (`window = client`) and
use that name in the function.

A stub differs from the classes at run time in one way, on purpose:

- A class that derives from another (`Camera3D` from `SceneNode`) names it as its base, so a checker accepts it
  where the base is wanted. At run time the classes are flat, and a call takes any handle.

## Imports and libraries

A shell imports from these modules, and each is served from a library installed with Gnoblin:

| Module           | Library                | Gives you                                                                               |
| ---------------- | ---------------------- | --------------------------------------------------------------------------------------- |
| `gnoblin`        | `libgnoblin-core.so.1` | The `gnoblin` object, the core's classes (`Gnoblin`, `Client`, `Output`, ...), `verbs`. |
| `gnoblin.std`    | `libgnoblin-std.so.1`  | Workspaces, layouts, styles, bindings, transitions and the UI toolkit, as classes.      |
| `gnoblin.errors` | none                   | `GnoblinError` and its subclasses.                                                      |

`gnoblin.objects.model` and `gnoblin.objects.std` are the modules the classes are written in. `gnoblin` and `gnoblin.std` re-export
them, and those are the paths to import.

Importing `gnoblin.std` opens `libgnoblin-std.so.1` and raises `ImportError` where it is not
installed: `gnoblin-std is not installed (looked for libgnoblin-std.so.1 in ...)`.

That is the check a core-only install is meant to give. A shell on `gnoblin` alone is
complete: example 8 tiles windows with no workspace.

### The root of the core: `Gnoblin`

`Gnoblin.get()` returns the root of the core's model. It has the members the TypeScript library's `gnoblin` has for the core:

```python
from gnoblin import Gnoblin

gnoblin = Gnoblin.get()
gnoblin.timers.after(40, lambda: print("later"))   # `timers.now()` is the monotonic clock in milliseconds
print(gnoblin.version().api)                       # a method, not a property
print(gnoblin.motion(200))                         # 200.0, or 0.0 while the seat asks for reduced motion
print(gnoblin.compositor.user_shaders)             # what you wrote, else what the compositor read when it started
gnoblin.compositor.user_shaders = False            # compositor.user-shaders in the shell's settings document
gnoblin.compositor.reset("user_shaders")           # back to the default
windows = gnoblin.find("Window", {"app_id": "foot"})   # a dict of fields, a ClientQuery, or a function
```

The shell commits nothing itself. After the turn that wrote it, Gnoblin lays the settings the shell wrote over the configuration the
compositor holds and commits the result, so the next login has `compositor.user-shaders` (the setting of a device applies at once). The
rest of the configuration is not touched, and a shell that writes no setting changes nothing.

A setting stays in the configuration until a login writes another value. To go back to the default, write it
(`gnoblin.compositor.user_shaders = True`), or call `reset("user_shaders")` in the run that wrote it. A reload whose script does not
write a setting takes it back. A value that is not a boolean raises a `TypeError`.

Gnoblin keeps the settings only for the shell the stored configuration belongs to: a shell run by another path (`gnoblin --shell ./dev.py`)
while another shell is installed in the configuration directory commits nothing, and the log says so.

`find("Window", ...)` and `find("Output", ...)` raise a `TypeError` for a kind the core does not list (`"Workspace"` in a shell that loads
no std), naming the kinds that work, and for a field the filter lacks. `examples/35-native-root/shell.py` is this.

### The members std adds to the root

The core has no workspaces, displays, scene or gestures, so its `Gnoblin` has no member of those names. Importing `gnoblin.std`
sets them on the core's class. A shell that does not import it has none: the attribute is missing.

```python
import gnoblin.std                       # puts the members on `Gnoblin` and `OutputCollection`
from gnoblin import Gnoblin
from gnoblin.std import TransitionOptions, Workspace

gnoblin = Gnoblin.get()
main = Workspace("main")
gnoblin.workspaces.add(main)                     # `workspaces` is the list: `list`, `length`, `add`, `remove`, `clear`, `by_id`
gnoblin.workspaces.show(main)                    # `show_by_id("main")` takes the id; `area` is where windows are laid out
gnoblin.displays.replace(old, new)               # swaps a display of the shell's own for another as one validated change
gnoblin.outputs.profiles.add(profile)            # the hardware profiles; every output collection reports the desktop's list
gnoblin.request_transition(lambda: ..., TransitionOptions(duration=200.0))   # the same call as `Transition.request`
print(gnoblin.scene.root, gnoblin.gestures.swipe_threshold)
```

- Each member is read through the std library of the generation that is running, so after a reload it is the new generation's.
- The stubs declare the members on the core's classes, as the TypeScript declarations merge them. The stub of the core's classes
  extends a stub-only class for each, written by `gnoblin.std` from the same catalog, so a type checker finds
  `gnoblin.workspaces` without a further import.
- The lowered classes (`GnoblinAugment` and the others) stay, for a call made through the service.
- `examples/37-native-std-root/shell.py` is this, run by `crates/gnoblin/tests/modular/native_std_root.rs` in a real session beside the
  Rust and TypeScript shells of the same example.

### The object classes

The catalog of `libgnoblin-std` (an entry model, the focus policy, and the records and enums they take) is served through the object
protocol, and `gnoblin.std` has a class for each of its types. `gnoblin` is the same for the core's
`gnoblin.model`.

The classes are **generated** from the catalog the library serves (`gnoblin-sdkgen`, at build time and never committed; [the
generator](sdk-generator.md)) and embedded in the runtime library. A class added to the catalog is a class here with no change to the
runtime.

The library starts the stock `FocusPolicy` itself when a shell makes a workspace, a display or a window frame, so a click and a new window take
the keyboard with no line of the shell's. A shell with a focus policy of its own calls `FocusPolicy.get().stop()` first.

```python
from gnoblin import gnoblin
from gnoblin.std import EntryModel, FocusEvent, FocusEventKind, FocusMode, FocusPolicy, FocusSettings

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
- A member that takes a window takes the core's `gnoblin.Client`. `examples/36-native-clients/shell.py` shows these calls:
    - `decorations.inset_of(client)`
    - `menus.show(client, x, y)`
    - `workspace.clients.add(client)`
- An id as text, or an object of another class, is a `TypeError`. A window that has closed raises `NodeGoneError`.
- A window the workspace classes tell the shell of is the core's `Client` too: `members`, the `added` and `removed` events, and a layout's
  `children`, `focused` and `node`. It is the same object `Gnoblin.get().clients.get(id)` gives. A window the frame, menu, policy and shell
  classes tell the shell of is still its id as text.
- An **enum** is an `IntEnum` (a method also takes its name, `"follow-mouse"`), and a list a `list`.
- `on(event, handler)` returns a `Listener` with `stop()`; without a handler it is a decorator. It lasts until stopped or the
  generation ends, not until the object is collected.
- One library object is one Python object. Two reads of it, or two events that carry it, give the same object, so `is`, `==`, `in` and a
  dict key work: `Gnoblin.get().clients is Gnoblin.get().clients` is `True`.
- An **interface** (`gnoblin.std.<Name>` of kind `interface`) is a base class: a subclass defines the methods it wants to answer
  and passes an instance to the library; a method it does not define runs natively.
- `.destroy()` ends the object and its children whoever else holds them. A reference the script drops is released on the shell thread, so a
  thread of the script may drop one.
- A failure is the exception of `gnoblin` for its kind (`StaleError` for an object that has ended), a value of the wrong type is a
  `TypeError` that names it, and an exception a callback raises is logged with its traceback.
- `gnoblin/objects/*.pyi` are the stubs. A `tests/shells.rs` check holds them to what the runtime reflects. Importing the module opens
  `libgnoblin-std.so.1` and raises `ImportError` without it, as `gnoblin.std` does.

`examples/20-native-object/shell.py` is this, run by `crates/gnoblin/tests/modular/native_object.rs` in a real session beside the
Rust and TypeScript shells of the same example.

### Make an object with options

A constructor takes the properties its catalog names as positional arguments, then one keyword-only argument for every other writable
property. Each is `None` unless you give it, and a keyword that is `None` is not sent, so the property keeps the library's default.

```python
from gnoblin.std import Gaps, MasterStackLayout, OutputMatch, OutputProfile, ProfileSettings, Workspace

main = Workspace("main", name="Main", gaps=Gaps(inner=6.0, outer=12.0), layout=MasterStackLayout())
profile = OutputProfile(OutputMatch(connector="HDMI-A-1"), id="tv", settings=ProfileSettings(scale=2.0))
```

- The keywords go in the call that makes the object. A value the library refuses raises from the constructor with the library's text,
  and no object is left.
- A class with no positional property takes the keywords alone. The stub (`.pyi`) lists each keyword with its type.
- `examples/38-native-options/shell.py` is this, run by `crates/gnoblin/tests/modular/native_options.rs` in a real session
  beside the Rust and TypeScript shells of the same example.

### The functions of `gnoblin-std` in the generated classes

A bare function of the TypeScript `gnoblin-std` is a function of a class in `gnoblin.std`, with the same behaviour. The
Python spellings are `snake_case`.

| `gnoblin-std` (TypeScript) | `gnoblin.std`                                                                                                                  |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `openLauncher`             | `Launcher.open(spawn=None)`: one launcher lives at a time, and a second call focuses it and returns it                         |
| `toast`                    | `Toast.show(text, fade_in=None, hold=None, fade_out=None, clock=None)`: the card removes itself after 4420 ms by default       |
| `onSwipe`, `onPinch`       | `SwipeActionBinding()` and `PinchActionBinding()`, then `set_action(way, name)` or `set_function(way, fn)`, then `install()`   |
| `direction`                | `Gestures.direction(dx, dy, threshold=None)`: a `SwipeDirection` with the `way`, or `None` while the gesture has gone less far |
| `estimateText`             | `Ui.estimate_text(text, size)`: a `TextMetrics` of `width`, `height` and `ascent`, and no desktop is asked                     |
| `encodePaintList`          | `Ui.encode_paint_list(ops)`: `bytes` (the TypeScript function answered a list of numbers) for a canvas's `content`             |

`Ui.encode_paint_list` refuses the whole list when one operation cannot be encoded, with an error that names it (`draw op 2: ...`).

### The desktop library: `gnoblin.std`

`gnoblin.std` is the classes of the desktop above the core: workspaces and layouts, pointer bindings, transitions, styles and scene
nodes. They are **generated** from the catalog of `libgnoblin-std.so.1` (`gnoblin-sdkgen`, at build time and never committed;
[the generator](sdk-generator.md)), so there is a class for every type the library has and none it lacks.

- A record is a dataclass, and a constant group an `IntEnum` or `IntFlag`.
- Built-in layouts and bindings (`MasterStackLayout`, `GridLayout`, `MonocleLayout`, `DwindleLayout`, `StripLayout`, `ArcLayout`,
  `Carousel3dLayout`, `MoveBinding`, `ResizeBinding`, `SwapBinding`, `SplitResizeBinding`) take their options, defaults and actions from the library.
- A layout of your own is a subclass of `Layout` with `compute(context)`, as `examples/03-python-std/shell.py` shows.
- Failures are the exceptions of `gnoblin` (`RefusedError`, `StaleError`, ...), with the library's text.

```python
from gnoblin import gnoblin
from gnoblin.std import Gaps, MasterStackLayout, Workspace, Workspaces

main = Workspace("main", gaps=Gaps(inner=6.0, outer=12.0), layout=MasterStackLayout())
Workspaces.get().add(main)
gnoblin.log("a workspace of the master and stack layout")
```

## Use the objects

### The `gnoblin` object

`from gnoblin import gnoblin` is the handle on the compositor. It is a thin spelling of the core: everything it does is one call
of the installed `libgnoblin-core`, and it holds no desktop policy.

It also has every member of the root of the object model, as written from the schema: the object that
`Gnoblin.get()` returns. `gnoblin.input` and `gnoblin.clients` answer what the root answers, and so do the
rest of its properties and its methods. A member the object has of its own, such as `spawn`, is its own.

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

A short shell on this object:

```python
from gnoblin import gnoblin


@gnoblin.on_client_created
def place(window):
    window.x, window.y = 40, 40


@gnoblin.bind("<Super>Return")
def terminal():
    gnoblin.spawn("foot")
```

A chord on `gnoblin.bind` is each modifier in angle brackets, then the key, as in `<Super>Return`; see
[shortcuts](/guides/shortcuts) for the syntax. The seat's `input.bind("Super+Return", fn)` takes the `Super+Return`
spelling.

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

The compositor's objects are nodes of one tree. The classes you meet (`Window`, `Seat`, `Screen`, ...) are generated from
the schema, so they have exactly the properties, methods and signals the schema has.

A shell does not import them. It gets a node from the `gnoblin` object (`gnoblin.windows`, `gnoblin.root`) or from a callback.
A property is an attribute:

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
- `window.closed(fn)` listens to a signal of one window; `gnoblin.on_signal("window", "closed", fn)` to every window's.

### Verbs and signals by name

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

### Session, processes and timers

- `Gnoblin.get().session` logs out, suspends, powers off and locks.
- `Gnoblin.get().process` starts programs and reports how they end, and `Gnoblin.get().spawn([...])` starts one.
- `Gnoblin.get().timers` runs a function later (`after`), and a `SessionListener` or `RuntimeListener` hears the session and the configuration.
- `Gnoblin.get().idle.watch(ms, listener)` runs code when the session has had no input for a while, `authentication.add_listener(listener)`
  watches polkit authentication requests and may refuse one, and `launches.begin(token, application)` tracks the busy state between
  a launch and its window.

See [Session and runtime](/shell-api/session-runtime), [Processes](/shell-api/processes), [Idle watches](/shell-api/idle),
[Authentication prompts](/shell-api/authentication), [Launch feedback](/shell-api/launch-feedback) and [Events](/events/).

### Outputs, window policy and animation

With more than one output each shows its own workspace, and a window moves between outputs with `client.move_to_output(output)`.
See [Outputs](/shell-api/outputs), [Window policy](/js-window-policy) and [animation](/js-transitions): both pages show every
example in TypeScript, Python and Rust.

### XR

XR scripting is part of `gnoblin-std`, and the Python classes match the TypeScript ones. See
[XR mode](/xr#scripting-xr) for the room, the viewport and the bindings that move and resize a panel.

### The console

`gnoblinctl console` is a JavaScript prompt, so it has nothing to attach to in a Python shell. Read and call objects with
`gnoblinctl OBJECT list|get|watch|call`, and follow `print` and `logging` with `gnoblinctl log --follow`. See
[Console and inspector](/js-console) for what the other tools replace it with.

## Window requests

A client asks the compositor for a change of its window: fullscreen, minimise, maximise, raise, activate, move, resize.
The compositor changes nothing until the shell answers. `WindowState.get().start()` makes the shell answer every kind.

The answers are the same in every language: a maximised window fills the area the bars leave, a fullscreen window covers
the whole output, a minimised window is hidden and its focus handed on. [Window requests](typescript-shell.md#window-requests)
has the table.

A `RequestListener` with an `on_request(request)` method hears every request first. Call `request.handle()` to answer it
yourself: the built-in answer does not run, and no later listener hears it.

```python
from gnoblin import Gnoblin
from gnoblin.std import ClientRequest, ClientRequestKind, RequestListener, WindowState

gnoblin = Gnoblin.get()
state = WindowState.get()


class KeepOnTop(RequestListener):
    def on_request(self, request: ClientRequest) -> None:
        client = gnoblin.clients.get(request.client)
        if client is None:
            return
        if request.kind == ClientRequestKind.MINIMIZE and client.app_id == "org.example.Clock":
            request.handle()  # this window never minimises
        elif request.kind == ClientRequestKind.MAXIMIZE:
            request.handle()
            state.enter_fullscreen(client)  # answer a maximise as a fullscreen


state.add_listener(KeepOnTop())
state.start()
```

`add_listener(listener, client)` with a window as the second argument hears only that window's requests. Those listeners
are asked first, then the ones for every window, each in the order it was added. The first one to call `request.handle()`
wins. A listener that raises is reported and skipped.

`WindowState` is the policy the built-in answer calls. Its methods return whether they ran: `maximise`, `unmaximise`,
`enter_fullscreen`, `leave_fullscreen`, `minimise`, `unminimise`, `present`, `raise_` and `lower`.

## Troubleshooting

### A failed call

A failed call of the core is an exception: `GnoblinError` and its subclasses (`NodeGoneError`, `MemberError`, `RefusedError`,
`RetiredError`, ...). A value of the wrong kind is `TypeError`.

A call on an object of the library sets two attributes on the exception:

- `error.code` says why: `not_found`, `already_exists`, `read_only` and so on.
- `error.target` names the object and member, such as `Client#7.close`.
- Both are `None` on an error that did not come from such a call.

The message begins with the target. The subclass says the kind of failure and `code` says the reason, so catch `GnoblinError` and read `code`.

```python
from gnoblin.errors import GnoblinError

try:
    client.close()
except GnoblinError as error:
    if error.code == "not_found":
        gnoblin.log(f"{error.target} is gone")
```

Put the file in `~/.config/gnoblin/shell.py`. Closing a window that has already closed logs its target. A bad argument raises
`TypeError`, which has no code. `examples/39-native-errors/shell.py` shows all three. [Errors](shell-api/errors.md) lists the codes.

A message does not begin with the symbol of the ABI code (`E_REFUSED: `). A write to a read-only property is a `GnoblinError` with the code
`read_only`, and not a `TypeError`. [A shell that read the old failure](shell-api/errors.md#a-shell-that-read-the-old-failure) lists what changed.

A callback that raises does not stop the shell: the traceback goes to the log, the next event is delivered, and the core
counts the failure against the generation.

### Debugging

`print` and the `logging` module reach the log, with the component `python`, and `gnoblin.log("text", "warn")` writes at the level
you name (`error`, `warn`, `info`, `debug` or `trace`). Read it with `gnoblinctl log --follow` (see [Logging](/guides/logging)),
or run `gnoblin ./shell.py` in a terminal.

```python
import logging

from gnoblin import Client, Gnoblin

gnoblin = Gnoblin.get()


def trace(client: Client, focused: bool) -> None:
    logging.warning("%s focus is %s", client.app_id, focused)


gnoblin.clients.on("focus_changed", trace)
```

`GNOBLIN_LOG=debug` shows each generation's life.

### Threads

Everything you register runs on the shell thread, one callback at a time. A thread you start may log and call
`gnoblin.post(fn)`; any other call from it raises `GnoblinError` that says so. The interpreter's lock is released while the
shell waits for events, so your threads run meanwhile.

### A failed load

A `shell.py` that fails to load when the shell starts is a failed start: the loader's text names the file and line, and the
session runs without a shell, on the compositor's own window management, until the file is fixed. Nothing is lost by waiting
and nothing needs the session restarted:

- the guardian logs `the shell failed to start:` and the text, `gnoblinctl shell status` prints `state: failed` with the
  same text, and `gnoblin ./shell.py` shows it on your terminal;
- saving the file loads it, and `gnoblinctl shell reload` loads it on request, exiting 1 with the new text while it still fails.

A session with no shell file at all runs the packaged starter, which is a `shell.ts`: see [the TypeScript
shell](typescript-shell.md#with-no-shell-ts) for what it does.

If the shell stops, the compositor keeps running and answers every client: a window that maps is placed by the compositor's
own policy, and windows the shell had moved go back to the places the compositor gave them. Starting the session again runs the
file again.

## Examples

Small shells in `examples/` show the pieces, and a test in `crates/gnoblin-runtime-python/tests/shells.rs` runs each one
as it is:

- `examples/07-script-binding/shell.py`: a key binding written entirely in Python that moves the focused window.
- `examples/03-python-std/shell.py` (example 3) and `layouts.py`: a workspace with a layout of your own, a listener on the clients and a
  chord, and the built-in layouts. The tests hold their rectangles to the ones the native Rust shells produce.
- `examples/08-no-workspaces/shell.py` and `tiles.py`: a master-and-stack tiler of raw window nodes, in a shell file and a module it imports (edit
  `tiles.py` and the tiling changes without restarting).
- The same test builds example 11 (edit the shell and a module it imports; no listener twice; a broken edit keeps the shell) and
  example 17 (a core-only shell runs, and importing std says why it cannot).
